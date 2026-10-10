//! GPU renderer: pipelines, bind groups, per-frame draw orchestration.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::celestial;
use crate::font;
use crate::frustum::Frustum;
use crate::gui::SpriteSheet;
use crate::particle_renderer::ParticleRenderer;
use crate::particles::ParticleEngine;
use crate::player_mesh::{self, PART_COUNT, PLAYER_STRIDE, PlayerVertex, SKIN_LAYERS};
use mcv_core::atlas;

pub const TERRAIN_STRIDE: usize = 24;
pub const HUD_STRIDE: usize = 28;

// 手持快照经 gpu 模块命名空间再暴露（game 层 `mcv_render::gpu::HandItem` 约定）。
pub use crate::hand::HandItem;
pub use crate::hand::HandRender;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FrameUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub cam_pos_time: [f32; 4],
    pub sun_dir_day: [f32; 4],
    pub fog_params: [f32; 4],
    /// 生物群系染色基色（26.1 ColorResolver 机制）：xyz = plains 草色
    /// （sRGB 0..1，来自 colormap/grass.png 温度×湿度查表），w = 染色开关
    /// （colormap 素材缺失时 0 → 不染色，不伪造颜色）。
    pub tint_grass: [f32; 4],
    /// xyz = plains 叶色（colormap/foliage.png 查表），w = 自由。
    pub tint_foliage: [f32; 4],
}

const _: () = assert!(size_of::<FrameUniforms>() == 144);

/// 水绘制分段（#80 合批）：批次内每个成员区块一段。半透明水必须保持
/// 逐区块远→近绘制序（混合次序不可变），故合批只合并索引缓冲，绘制仍
/// 逐段进行——`center` 与旧逐块排序键（chunk origin + (8,0,8)）逐值一致。
#[derive(Clone)]
pub struct WaterPart {
    /// [`RenderChunk::water_index_buf`] 内的索引区间。
    pub range: Range<u32>,
    /// 成员区块水面中心（origin + (8,0,8)），远→近排序键。
    pub center: [f32; 3],
}

/// 一个绘制条目：未合批时 = 单区块（origin 即区块原点、恰好 0/1 个
/// [`WaterPart`]）；#80 合批后 = 空间相邻、同材质 pass 的区块批次
/// （origin 为批次最小角、成员顶点已重定基到批次原点、water_parts 每
/// 成员一段）。缓冲仍只在建网格/重建批次时上传。
#[derive(Clone)]
pub struct RenderChunk {
    pub origin: [f32; 3],
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub opaque_range: Range<u32>,
    /// Water geometry is meshed into its own index buffer (separate pass);
    /// 合批后为批次全体成员水索引的拼接缓冲，分段见 [`RenderChunk::water_parts`]。
    pub water_index_buf: Option<wgpu::Buffer>,
    /// 水分段（每成员区块一段；无水 = 空表）。旧单值 `water_range` 是
    /// 「恰一段」的退化形态，合批后必须按成员拆段保持逐块混合序。
    pub water_parts: Vec<WaterPart>,
    pub aabb: (Vec3, Vec3),
}

/// 相邻区块合批（#80）的成员上限：区块数与合并字节双阈值。批次数上限
/// 决定 draw 收敛比（RENDER_DIST=8 → 289 块 / 8 ≈ 40 批），字节上限决定
/// 单块重网格触发整批重建的带宽放大上限。
const MAX_BATCH_MEMBERS: usize = 8;
const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

/// staging 环（#80 第二刀）：单槽容量与槽数。8 槽 × 2 MiB = 16 MiB 在途
/// 窗口，常规流式帧的网格上传（< 2 MiB）全部走环；单笔超槽容量或环耗尽
/// 时退一次性 mapped_at_creation 兜底（与改动前同机制，行为不变）。
const STAGING_SLOT_BYTES: u64 = 2 * 1024 * 1024;
const STAGING_SLOT_COUNT: usize = 8;

/// 异步上传统计（#80 第二刀可观测性）：主线程同步大拷贝消除的证据。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
    /// 经 staging 环提交的拷贝笔数。
    pub staged_uploads: u64,
    /// 经 staging 环提交的拷贝字节数（顶点+索引）。
    pub staged_bytes: u64,
    /// 环槽 map/unmap 次数（写权分帧回收的频率）。
    pub slot_maps: u64,
    /// 走一次性兜底（超槽容量或环耗尽）的笔数——恒为 0 说明环容量足够。
    pub fallback_uploads: u64,
}

/// staging 槽状态：Mapping（map_async 已发，等 poll 回调）→ Mapped（可写，
/// used = 写游标）→ InFlight（已写已 unmap、拷贝已提交，等
/// map_buffer_on_submit 回调）→ Mapped（写权回收，分帧循环）。
enum SlotState {
    Mapping,
    Mapped { used: u64 },
    InFlight,
}

struct StagingSlot {
    buf: wgpu::Buffer,
    state: SlotState,
}

/// 一笔待上传拷贝：目标缓冲 + 数据（flush 时写入 staging 并记 copy）。
struct PendingUpload {
    dst: wgpu::Buffer,
    bytes: Vec<u8>,
}

/// 合批结构性统计（#80 可观测性）：draw 收敛与重建带宽都从这里出数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchingStats {
    /// 池内已上传网格的区块数。
    pub meshed_chunks: usize,
    /// 绘制条目数（批次 + 不可合并单块）——opaque draw 数的上界。
    pub draw_entries: usize,
    /// 批次重建次数（新块并入/重网格/卸载都算一次）。
    pub batch_rebuilds: u64,
    /// 重建累计写出的顶点+索引字节数。
    pub rebuild_bytes: u64,
}

/// 池内单块网格的 CPU 字节（合批重建的数据源）：opaque 顶点 + opaque 局部
/// 索引 + 水顶点 + 水局部索引。索引一律保存区块局部（0 基）形态，进批次
/// 时按成员顶点基址平移。
struct PoolMesh {
    vbytes: Vec<u8>,
    opaque_vcount: u32,
    opaque_idx: Vec<u32>,
    water_vbytes: Vec<u8>,
    water_idx: Vec<u32>,
    /// origin 是否为 16 格网格点（区块原点 y 恒 0）。手工构造的测试原点
    /// （非 16 倍数）永远单块，不做重定基。
    mergeable: bool,
}

/// 网格上传器：游戏层只交出顶点/索引字节，拿回 [`RenderChunk`]。
///
/// 这是引擎把 wgpu 挡在游戏层之外的唯一入口——mesher 产出裸字节，本结构
/// 负责建 GPU 缓冲；游戏层因此不 `use wgpu`。水几何经
/// [`MeshUploader::build_chunk`] 的 `water` 参数上传：水顶点拼在 opaque
/// 顶点之后共用一个 vb、水索引独立成缓冲并整体偏移 opaque 顶点数（旧文档
/// 「只上传独立索引缓冲」描述的是 H1 修复前缺水顶点的行为，已纠正）。
///
/// #80 起本结构同时持有相邻区块合批账本：每块网格字节入池，空间相邻、
/// 材质 pass 相同的成员合并成批次条目（[`Self::entries`]），一次 draw 画
/// 多个区块。条目 origin = 批次最小角，成员顶点在建批次时重定基（加
/// chunk_origin − batch_origin，全为 16 的倍数，f32 精确）——着色器
/// `world = uniform.origin + pos` 逐位不变。
pub struct MeshUploader {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pool: std::collections::HashMap<(i32, i32), PoolMesh>,
    /// 批次成员表，与 `entries` 按下标平行；条目重建原位替换，仅整体清空
    /// 时才移除槽位（渲染端槽位分配依赖条目序稳定）。
    batch_members: Vec<Vec<(i32, i32)>>,
    entries: Vec<RenderChunk>,
    stats: BatchingStats,
    /// staging 环槽（MAP_WRITE|COPY_SRC，map_async 写入、map_buffer_on_submit
    /// 分帧回收写权）。
    staging: Vec<StagingSlot>,
    /// 映射回调回执（map_async/map_buffer_on_submit 完成的槽下标 + 是否
    /// 成功）。回调无法借用 self，经 Arc 信道送回 flush 消化。
    mapped_mailbox: std::sync::Arc<std::sync::Mutex<Vec<(usize, bool)>>>,
    /// 待 flush 的拷贝（目标缓冲 + 字节）；flush 记入 encoder 并 submit，
    /// 先于同帧任何 draw（单队列按提交序执行）。
    uploads: Vec<PendingUpload>,
    upload_stats: UploadStats,
}

impl MeshUploader {
    /// device + queue：网格不再经创建期映射整块写入，而是 flush 时经
    /// staging 环 map_async 写入 + copy_buffer_to_buffer 提交——主线程
    /// 大拷贝与设备缓冲分配/解映射解耦（#80 第二刀）。
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let mailbox = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut staging = Vec::with_capacity(STAGING_SLOT_COUNT);
        for i in 0..STAGING_SLOT_COUNT {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh-staging"),
                size: STAGING_SLOT_BYTES,
                usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let mb = std::sync::Arc::clone(&mailbox);
            buf.map_async(wgpu::MapMode::Write, .., move |res| {
                let ok = res.is_ok();
                if let Err(e) = res {
                    log::error!("staging slot {i} initial map failed: {e}");
                }
                mb.lock().expect("staging mailbox").push((i, ok));
            });
            staging.push(StagingSlot {
                buf,
                state: SlotState::Mapping,
            });
        }
        Self {
            device,
            queue,
            pool: std::collections::HashMap::new(),
            batch_members: Vec::new(),
            entries: Vec::new(),
            stats: BatchingStats::default(),
            staging,
            mapped_mailbox: mailbox,
            uploads: Vec::new(),
            upload_stats: UploadStats::default(),
        }
    }

    /// 当前绘制条目表（批次 + 不可合并单块）。场景块表即此表——合批后
    /// 一次 draw 对应一个条目。
    pub fn entries(&self) -> &[RenderChunk] {
        &self.entries
    }

    /// 合批结构性统计（HUD/日志用）。
    pub fn batching_stats(&self) -> BatchingStats {
        BatchingStats {
            draw_entries: self.entries.len(),
            ..self.stats
        }
    }

    /// 异步上传统计（#80 可观测性）。
    pub fn upload_stats(&self) -> UploadStats {
        self.upload_stats
    }

    /// 每帧冲刷：把 build/unload 期间攒下的拷贝经 staging 环记入 encoder
    /// 并 submit。拷贝 submit 先于同帧 draw_frame 的 submit（单队列按提交
    /// 序执行）——draw 必见已上传数据，与旧「创建期映射写 + 下次 submit
    /// 内部拷贝」的可见时序逐帧一致。环槽写权经 map_buffer_on_submit 等
    /// 本 submit 执行完毕后才回到 Mapped（分帧回收）。
    pub fn flush(&mut self) {
        let in_flight = self
            .staging
            .iter()
            .any(|s| !matches!(s.state, SlotState::Mapped { used: 0 }));
        let mail_pending = !self
            .mapped_mailbox
            .lock()
            .expect("staging mailbox")
            .is_empty();
        if self.uploads.is_empty() && !in_flight && !mail_pending {
            return;
        }
        // 驱动映射回调（初始 map / 上一 submit 的写权回收）。
        let _ = self.device.poll(wgpu::PollType::Poll);
        for (i, ok) in self
            .mapped_mailbox
            .lock()
            .expect("staging mailbox")
            .drain(..)
        {
            self.staging[i].state = if ok {
                SlotState::Mapped { used: 0 }
            } else {
                // 映射失败：重新申请（state 回 Mapping，下轮 poll 重试）。
                let mb = std::sync::Arc::clone(&self.mapped_mailbox);
                self.staging[i]
                    .buf
                    .map_async(wgpu::MapMode::Write, .., move |res| {
                        mb.lock().expect("staging mailbox").push((i, res.is_ok()));
                    });
                SlotState::Mapping
            };
        }
        if self.uploads.is_empty() {
            return;
        }
        let ups = std::mem::take(&mut self.uploads);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mesh-upload"),
            });
        let mut used_slots: Vec<usize> = Vec::new();
        let mut n_staged = 0u64;
        let mut n_bytes = 0u64;
        let mut n_fallback = 0u64;
        for up in ups {
            let len = up.bytes.len() as u64;
            if len == 0 {
                // 空数据无拷贝：目标缓冲保持零初始化，几何为空永不引用
                // （opaque_range 空 / 无水分段）。
                continue;
            }
            // 找一个可写且有剩余容量的环槽；超槽容量或环耗尽 → 一次性
            // mapped_at_creation 兜底（改动前同机制）。
            let mut slot = None;
            for (i, st) in self.staging.iter().enumerate() {
                let SlotState::Mapped { used } = st.state else {
                    continue;
                };
                if used + len <= STAGING_SLOT_BYTES {
                    slot = Some(i);
                    break;
                }
            }
            match slot {
                Some(si) => {
                    let off = match self.staging[si].state {
                        SlotState::Mapped { used } => used,
                        _ => unreachable!("上方循环已过滤出 Mapped 槽"),
                    };
                    let dst = &self.staging[si];
                    let mut view = dst
                        .buf
                        .slice(..)
                        .get_mapped_range_mut()
                        .expect("slot mapped");
                    view.slice(off as usize..(off + len) as usize)
                        .copy_from_slice(&up.bytes);
                    drop(view);
                    encoder.copy_buffer_to_buffer(&dst.buf, off, &up.dst, 0, len);
                    if let SlotState::Mapped { used } = &mut self.staging[si].state {
                        *used = off + len;
                    }
                    if !used_slots.contains(&si) {
                        used_slots.push(si);
                    }
                    n_staged += 1;
                    n_bytes += len;
                }
                _ => {
                    // len > 槽容量或全槽在途：一次性兜底（submit 后随命令
                    // 缓冲释放，无写权需要回收）。
                    let tmp = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("mesh-staging-once"),
                        size: len.next_multiple_of(4),
                        usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: true,
                    });
                    {
                        let mut view = tmp
                            .slice(..)
                            .get_mapped_range_mut()
                            .expect("one-shot staged");
                        view.slice(0..len as usize).copy_from_slice(&up.bytes);
                    }
                    tmp.unmap();
                    encoder.copy_buffer_to_buffer(&tmp, 0, &up.dst, 0, len);
                    n_staged += 1;
                    n_bytes += len;
                    n_fallback += 1;
                }
            }
        }
        for si in used_slots {
            self.staging[si].buf.unmap();
            self.staging[si].state = SlotState::InFlight;
            let mb = std::sync::Arc::clone(&self.mapped_mailbox);
            // 写权回收必须挂在「本 submit 之后」：plain map_async 在提交前
            // 注册、只等更早的提交完成——拷贝尚未执行写权就回来会踩在途
            // 数据。map_buffer_on_submit 把映射推迟到本 submit 执行完毕
            // （wgpu 30 为此场景的原生 API），回调到站 → 下轮 flush 复用。
            encoder.map_buffer_on_submit(
                &self.staging[si].buf,
                wgpu::MapMode::Write,
                ..,
                move |res| {
                    mb.lock().expect("staging mailbox").push((si, res.is_ok()));
                },
            );
            self.upload_stats.slot_maps += 1;
        }
        self.queue.submit([encoder.finish()]);
        self.upload_stats.staged_uploads += n_staged;
        self.upload_stats.staged_bytes += n_bytes;
        self.upload_stats.fallback_uploads += n_fallback;
    }

    /// 区块卸载：释放该块网格字节并从所属批次移除成员；批空删批，否则
    /// 原位重建（剩余成员顶点重定基到新批次最小角）。
    pub fn unload(&mut self, pos: mcv_core::ChunkPos) {
        let key = (pos.x, pos.z);
        if self.pool.remove(&key).is_some() {
            self.stats.meshed_chunks -= 1;
        }
        let Some(bi) = self.batch_members.iter().position(|m| m.contains(&key)) else {
            return;
        };
        self.batch_members[bi].retain(|&k| k != key);
        if self.batch_members[bi].is_empty() {
            // 顺序保持的移除：后续条目槽位前移一次，与旧 per-chunk retain
            // 的槽位漂移同级。
            self.batch_members.remove(bi);
            self.entries.remove(bi);
        } else {
            self.rebuild_batch(bi);
        }
    }

    /// 建目标缓冲（数据为空，走 staging 异步拷贝填充）。
    /// 旧「创建期映射 + mapped slice 写入」路径已整体替换：创建期映射要求
    /// 上传线程同步整块写入并立即 unmap（每块 2-3 次分配/映射/大拷贝），
    /// staging 环把这三者从上传点解耦（#80 第二刀）。同样注意目标缓冲
    /// 不得用 queue.write_buffer 填充——GLES 拒绝对映射 buffer 的
    /// write_buffer 的教训保留在案，环内一律 map_async + copy。
    fn empty_buffer(&self, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh-dst"),
            // 映射视图要求长度是 4 的倍数且非空（wgpu 30 MapRangeError）。
            size: size.next_multiple_of(4).max(4),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// 由裸网格字节上传一个区块并入池（#80）：水顶点拼在 opaque 顶点之后
    /// 共用一个 vb、水索引整体偏移 opaque 顶点数——water draw 绑同一个
    /// vertex_buf。旧签名只收水索引、水顶点字节从未上传（`vertex_index(&[], wi)`），
    /// draw 把基址 0 的水索引绑到 opaque 顶点上：GLES 越界 `glDrawElements`
    /// 直接 INVALID_OPERATION 静默跳过、桌面越界读画垃圾埋进地形——
    /// 水从接线第一天起从未被真正绘制（2026-10-10 真机审计 H1）。
    ///
    /// 返回该区块当前的绘制条目（合批后 = 所属批次条目）。重网格时成员
    /// 资格不变，只重建所属批次；新块并入容量未满的邻接批次（固定方向序
    /// 保证确定性），否则新建单块批次。origin 非 16 格网格点（手工测试
    /// 原点）恒单块、不做重定基。
    pub fn build_chunk(
        &mut self,
        origin: [f32; 3],
        vbytes: &[u8],
        ibytes: &[u32],
        water: Option<(&[u8], &[u32])>,
    ) -> RenderChunk {
        let key = (
            (origin[0] / 16.0).round() as i32,
            (origin[2] / 16.0).round() as i32,
        );
        let mergeable = origin[0] == 16.0 * key.0 as f32
            && origin[2] == 16.0 * key.1 as f32
            && origin[1] == 0.0
            && vbytes.len().is_multiple_of(TERRAIN_STRIDE)
            && water.is_none_or(|(wv, _)| wv.len().is_multiple_of(TERRAIN_STRIDE));
        let old = self.pool.insert(
            key,
            PoolMesh {
                vbytes: vbytes.to_vec(),
                opaque_vcount: (vbytes.len() / TERRAIN_STRIDE) as u32,
                opaque_idx: ibytes.to_vec(),
                water_vbytes: water.map_or(Vec::new(), |(wv, _)| wv.to_vec()),
                water_idx: water.map_or(Vec::new(), |(_, wi)| wi.to_vec()),
                mergeable,
            },
        );
        if old.is_none() {
            self.stats.meshed_chunks += 1;
        }
        if let Some(bi) = self.batch_members.iter().position(|m| m.contains(&key)) {
            // 重网格：批次成员资格不变，只重建本批次（#80 稳定批次要求）。
            self.rebuild_batch(bi);
        } else {
            // 新块：按固定方向序找容量未满的邻接批次并入，否则新建单块批次。
            let mut joined = None;
            if mergeable {
                for (dx, dz) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let nk = (key.0 + dx, key.1 + dz);
                    let hit = self.batch_members.iter().position(|m| m.contains(&nk));
                    if let Some(bi) = hit
                        && self.batch_joinable(bi, key)
                    {
                        self.batch_members[bi].push(key);
                        joined = Some(bi);
                        break;
                    }
                }
            }
            let bi = match joined {
                Some(bi) => bi,
                None => {
                    let members = vec![key];
                    let (entry, bytes, uploads) = self.build_batch_entry(&members);
                    self.stats.rebuild_bytes += bytes;
                    self.uploads.extend(uploads);
                    self.batch_members.push(members);
                    self.entries.push(entry);
                    self.batch_members.len() - 1
                }
            };
            self.rebuild_batch(bi);
        }
        self.entry_of(key).expect("刚入池的区块必有批次")
    }

    /// 批次是否还能再吞下一个成员（成员数与合并字节双阈值）。
    fn batch_joinable(&self, bi: usize, key: (i32, i32)) -> bool {
        let Some(mesh) = self.pool.get(&key) else {
            return false;
        };
        if !mesh.mergeable {
            return false;
        }
        let members = &self.batch_members[bi];
        if members.len() >= MAX_BATCH_MEMBERS {
            return false;
        }
        let mesh_bytes = |m: &PoolMesh| {
            m.vbytes.len() + m.water_vbytes.len() + 4 * (m.opaque_idx.len() + m.water_idx.len())
        };
        let mut bytes = mesh_bytes(mesh);
        for k in members {
            bytes += self.pool.get(k).map_or(0, mesh_bytes);
        }
        bytes <= MAX_BATCH_BYTES
    }

    /// 键所属批次条目。
    fn entry_of(&self, key: (i32, i32)) -> Option<RenderChunk> {
        let bi = self.batch_members.iter().position(|m| m.contains(&key))?;
        self.entries.get(bi).cloned()
    }

    /// 原位重建第 bi 个批次（成员表不变，数据源为池内最新字节）。
    fn rebuild_batch(&mut self, bi: usize) {
        self.stats.batch_rebuilds += 1;
        let members = self.batch_members[bi].clone();
        let (entry, bytes, uploads) = self.build_batch_entry(&members);
        self.stats.rebuild_bytes += bytes;
        self.uploads.extend(uploads);
        self.entries[bi] = entry;
    }

    /// 由成员表组装批次条目：成员顶点重定基（pos += chunk_origin −
    /// batch_origin，全为 16 的倍数 → f32 精确，着色器世界坐标逐位不变）、
    /// 索引按成员顶点基址平移、水索引区间按成员拆段（保持逐块远→近混合序）。
    /// 返回 (条目, 逻辑字节数, 待 staging 拷贝表)。
    fn build_batch_entry(&self, members: &[(i32, i32)]) -> (RenderChunk, u64, Vec<PendingUpload>) {
        let min_cx = members.iter().map(|&(cx, _)| cx).min().expect("批次非空");
        let min_cz = members.iter().map(|&(_, cz)| cz).min().expect("批次非空");
        let batch_origin = [16.0 * min_cx as f32, 0.0, 16.0 * min_cz as f32];
        let mut vbytes: Vec<u8> = Vec::new();
        let mut opaque_idx: Vec<u32> = Vec::new();
        let mut water_idx: Vec<u32> = Vec::new();
        let mut water_parts: Vec<WaterPart> = Vec::new();
        let mut vbase: u32 = 0;
        let mut wibase: u32 = 0;
        let mut aabb: Option<(Vec3, Vec3)> = None;
        for &(cx, cz) in members {
            let mesh = self.pool.get(&(cx, cz)).expect("批次成员必在池中");
            let dx = ((cx - min_cx) * 16) as f32;
            let dz = ((cz - min_cz) * 16) as f32;
            let mut mv = mesh.vbytes.clone();
            rebase_positions(&mut mv, dx, dz);
            let mut wv = mesh.water_vbytes.clone();
            rebase_positions(&mut wv, dx, dz);
            for ix in &mesh.opaque_idx {
                opaque_idx.push(ix + vbase);
            }
            if !mesh.water_idx.is_empty() {
                let woff = vbase + mesh.opaque_vcount;
                for ix in &mesh.water_idx {
                    water_idx.push(ix + woff);
                }
                water_parts.push(WaterPart {
                    range: wibase..wibase + mesh.water_idx.len() as u32,
                    center: [16.0 * cx as f32 + 8.0, 0.0, 16.0 * cz as f32 + 8.0],
                });
                wibase += mesh.water_idx.len() as u32;
            }
            vbase += ((mv.len() + wv.len()) / TERRAIN_STRIDE) as u32;
            vbytes.extend_from_slice(&mv);
            vbytes.extend_from_slice(&wv);
            let c_origin = Vec3::new(16.0 * cx as f32, 0.0, 16.0 * cz as f32);
            let c_aabb = (
                c_origin,
                Vec3::new(c_origin.x + 16.0, 256.0, c_origin.z + 16.0),
            );
            aabb = Some(match aabb {
                None => c_aabb,
                Some((mn, mx)) => (mn.min(c_aabb.0), mx.max(c_aabb.1)),
            });
        }
        let entry_bytes = (vbytes.len() + 4 * (opaque_idx.len() + water_idx.len())) as u64;
        let has_water = !water_parts.is_empty();
        // 目标缓冲先建空壳，数据经 flush 的 staging 环拷入（#80 第二刀）：
        // 上传 submit 先于同帧 draw → draw 必见数据，时序与旧「创建期
        // 映射写 + 下次 submit 内部拷贝」逐帧一致。
        let mut uploads: Vec<PendingUpload> = Vec::new();
        // 拷贝长度必须是 COPY_BUFFER_ALIGNMENT(4) 的倍数：mesher 输出本就
        // 是 24B 整倍数，这里对畸形字节兜底补零（旧创建期映射路径无此
        // 约束，copy_buffer_to_buffer 校验更严）。
        if !vbytes.len().is_multiple_of(4) {
            vbytes.resize(vbytes.len().next_multiple_of(4), 0);
        }
        let vertex_buf = self.empty_buffer(vbytes.len() as u64, wgpu::BufferUsages::VERTEX);
        if !vbytes.is_empty() {
            uploads.push(PendingUpload {
                dst: vertex_buf.clone(),
                bytes: vbytes,
            });
        }
        let index_buf = self.empty_buffer(opaque_idx.len() as u64 * 4, wgpu::BufferUsages::INDEX);
        if !opaque_idx.is_empty() {
            uploads.push(PendingUpload {
                dst: index_buf.clone(),
                bytes: bytemuck::cast_slice(&opaque_idx).to_vec(),
            });
        }
        let water_index_buf = if has_water {
            let ib = self.empty_buffer(water_idx.len() as u64 * 4, wgpu::BufferUsages::INDEX);
            if !water_idx.is_empty() {
                uploads.push(PendingUpload {
                    dst: ib.clone(),
                    bytes: bytemuck::cast_slice(&water_idx).to_vec(),
                });
            }
            Some(ib)
        } else {
            None
        };
        let (aabb_min, aabb_max) = aabb.unwrap_or((Vec3::ZERO, Vec3::ZERO));
        (
            RenderChunk {
                origin: batch_origin,
                vertex_buf,
                index_buf,
                opaque_range: 0..opaque_idx.len() as u32,
                water_index_buf,
                water_parts,
                aabb: (aabb_min, aabb_max),
            },
            entry_bytes,
            uploads,
        )
    }
}

/// 顶点位置重定基：每 TERRAIN_STRIDE 字节的前 12 字节是 f32x3 pos，x/z 加
/// 批次内偏移（y 不动——区块原点 y 恒 0）。逐字节读写避开 Vec<u8> 的对齐
/// 假设；偏移为 16 的倍数、局部坐标为整数，f32 运算精确、世界坐标逐位
/// 与逐块绘制一致。
fn rebase_positions(v: &mut [u8], dx: f32, dz: f32) {
    if dx == 0.0 && dz == 0.0 {
        return;
    }
    for vtx in v.as_chunks_mut::<TERRAIN_STRIDE>().0 {
        let mut x = f32::from_le_bytes([vtx[0], vtx[1], vtx[2], vtx[3]]);
        let mut z = f32::from_le_bytes([vtx[8], vtx[9], vtx[10], vtx[11]]);
        x += dx;
        z += dz;
        vtx[0..4].copy_from_slice(&x.to_le_bytes());
        vtx[8..12].copy_from_slice(&z.to_le_bytes());
    }
}

/// One HUD rectangle (pixels, top-left origin). `tex` selects the source:
/// 0 = font atlas cell (glyph or the reserved solid-white cell 127),
/// 1 = terrain array layer, 2 = GUI sprite sheet (MC 素材).
pub struct HudQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// UV rect inside the source (0..1). For terrain tiles use 0..1.
    pub uv: [[f32; 2]; 2],
    pub color: [f32; 4],
    pub tex: u32,
    pub layer: u32,
    /// 绕 quad 中心的旋转角(弧度,splash 文字用);常规 quad 传 0。
    pub rot: f32,
}

/// 挖掘 overlay 的一面：暴露才画裂纹 quad；光照取相邻空气方块
/// （低 nibble=block、高 nibble=sky，与 mesher 面光照一致）。
#[derive(Clone, Copy, Debug)]
pub struct MineFace {
    pub exposed: bool,
    pub block_light: u8,
    pub sky_light: u8,
}

/// 选中/挖掘 overlay：`min` = 方块最小角世界坐标；`crack_stage` =
/// Some(0..=9) 时画裂纹层（CRACK_BASE+stage，原版 10 档
/// destroy_stage_0..9，MultiPlayerGameMode.java:551），描边始终画。
/// 面顺序与着色器 face_id 一致：+X,-X,+Y,-Y,+Z,-Z。
#[derive(Clone, Copy, Debug)]
pub struct MiningOverlay {
    pub min: [f32; 3],
    pub crack_stage: Option<u32>,
    pub faces: [MineFace; 6],
}

/// 裂纹 overlay 顶点：与 terrain 顶点逐字节同布局（TERRAIN_STRIDE）。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CrackVertex {
    pos: [f32; 3],
    uv: [u16; 2],
    layer: u16,
    block_light: u8,
    sky_light: u8,
    ao: u8,
    flags: u8,
    pad: [u8; 2],
}

const _: () = assert!(size_of::<CrackVertex>() == TERRAIN_STRIDE);

/// 面顶点模板（单位立方体局部坐标）与法线，索引 = face_id
/// （+X,-X,+Y,-Y,+Z,-Z，与 terrain.wgsl face_shade 表一致）。
const FACE_QUAD: [[[f32; 3]; 4]; 6] = [
    [
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ],
    [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0],
    ],
    [
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ],
    [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    ],
    [
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ],
    [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ],
];
const FACE_NORM: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];
/// uv 单位 = 1/4096 tile（同 mesher kUvPerBlock / terrain.wgsl 除数）；
/// 裂纹贴图为对称网格，角序方向不敏感，仅同步单位。
const FACE_UV: [[u16; 2]; 4] = [[0, 0], [0, 4096], [4096, 4096], [4096, 0]];

/// 生成暴露面的裂纹 quad（局部坐标沿法线外偏 0.003，深度只读时防
/// z-fighting）。layer = CRACK_BASE + stage（stage 0..=9 已由 game 层
/// 按原版公式钳好）。
fn build_crack_overlay(ov: &MiningOverlay) -> (Vec<CrackVertex>, Vec<u32>) {
    let stage = ov
        .crack_stage
        .unwrap_or(0)
        .min((atlas::CRACK_LAYERS - 1) as u32);
    let layer = (atlas::CRACK_BASE + stage as usize) as u16;
    let mut verts = Vec::with_capacity(24);
    let mut idx = Vec::with_capacity(36);
    for (f, face) in ov.faces.iter().enumerate() {
        if !face.exposed {
            continue;
        }
        let base = verts.len() as u32;
        for c in 0..4 {
            let p = &FACE_QUAD[f][c];
            let n = &FACE_NORM[f];
            verts.push(CrackVertex {
                pos: [
                    p[0] + n[0] * 0.003,
                    p[1] + n[1] * 0.003,
                    p[2] + n[2] * 0.003,
                ],
                uv: FACE_UV[c],
                layer,
                block_light: face.block_light,
                sky_light: face.sky_light,
                ao: 3,
                flags: f as u8,
                pad: [0; 2],
            });
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, idx)
}

/// 一个生物绘制实例：`kind` = [`mcv_render::mob_mesh::MobModelKind`] 序号，
/// `models` = 该实例的部位模型矩阵（来自 `mob_mesh::mob_model_matrices`，
/// 只取前 `MOB_PART_COUNTS[kind]` 个）。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MobInstance {
    pub kind: u32,
    pub models: [[[f32; 4]; 4]; crate::mob_mesh::MAX_MOB_PARTS],
}

pub struct Scene<'a> {
    pub camera: &'a Camera,
    pub time: f32,
    /// 天空亮度系数（day.json sky_light_factor；天气混合由调用方算——
    /// WeatherAttributes SKY_LIGHT_FACTOR 向夜底 0.24 混）。
    pub day_factor: f32,
    pub sun_dir: Vec3,
    /// 天气雾色 RGB 乘子（AtmosphericFogEnvironment.applyWeatherDarken:50-62；
    /// 晴 = [1,1,1]）乘到地平线色上（天空 + 地形雾同源）。
    pub fog_tint: [f32; 3],
    /// 天气雾密度乘子（雾距收缩 AtmosphericFogEnvironment.java:70-73 的
    /// exp 雾等价映射；晴 = 1.0）。
    pub fog_density_mult: f32,
    /// 月相序号 0..=7（MoonPhase 序，`celestial::moon_phase` 由游戏时间算出）。
    pub moon_phase: u32,
    /// Render target size in pixels (HUD coordinate space).
    pub width: f32,
    pub height: f32,
    pub chunks: &'a [RenderChunk],
    pub hud: &'a [HudQuad],
    /// 云：(资源, 设置)；None 或 enabled=false 不画（天空后、地形前）。
    pub cloud: Option<(&'a crate::Clouds, crate::CloudSettings)>,
    /// 玩家模型：(12 部位模型矩阵, 皮肤层 0=steve 1=alex)；第三人称时传入。
    pub player: Option<(&'a [glam::Mat4; PART_COUNT], u32)>,
    /// 生物实例（鸡/牛/羊/猪…）；None 或空 = 不画。
    pub mobs: Option<&'a [MobInstance]>,
    /// 挖掘裂纹 + 选中描边；None = 准星无目标。
    pub overlay: Option<MiningOverlay>,
    /// 眼睛在水中（Player.isEyeInFluid(WATER)）：帧雾切水下参数
    /// （26.1 水下视距骤减；GameRuntime::eye_under_water 喂入）。
    pub underwater: bool,
    /// 粒子引擎：(池, 帧内 tick 进度 partialTickTime 0..1)；None 不画
    /// （crack overlay 后、水前，26.1 translucent 序）。
    pub particles: Option<(&'a ParticleEngine, f32)>,
    /// 第一人称手持渲染（右臂 + 手持物 + 挥臂）：世界 pass 之后**清深度**
    /// 独立 pass（26.1 GameRenderer.java:724-729 clearDepth → renderItemInHand）。
    /// 第三人称/加载态/死亡态传 None 不画。
    pub hand: Option<crate::hand::HandRender>,
}

/// 帧绘制结构性统计（#80 可观测性）：合批前后 draw 数对比的代码级证据。
/// 读取经 [`Renderer::last_frame_stats`]；不进 HUD（渲染像素测试对 HUD
/// 区域敏感，仓内亦无 F3 通路）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// 视锥剔除后的绘制条目数（合批 = 批次数，非区块数）。
    pub visible_entries: u32,
    /// 本帧 opaque draw_indexed 次数。
    pub opaque_draws: u32,
    /// 本帧 water draw_indexed 次数（逐成员分段，与逐块绘制同粒度）。
    pub water_draws: u32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// 最近一帧的结构性统计（draw_frame 尾部写入）。
    last_stats: FrameStats,
    terrain_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    hud_pipeline: wgpu::RenderPipeline,
    frame_buf: wgpu::Buffer,
    origins_buf: wgpu::Buffer,
    sky_buf: wgpu::Buffer,
    hud_uniform: wgpu::Buffer,
    hud_vbuf: wgpu::Buffer,
    hud_ibuf: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    hud_bind: wgpu::BindGroup,
    sky_bind: wgpu::BindGroup,
    /// MC GUI 精灵表(资源根 textures/ 下原版精灵);None = 素材缺失
    /// (上层按素材红线显示加载失败提示，无程序化回退)。
    gui: Option<SpriteSheet>,
    player_pipeline: wgpu::RenderPipeline,
    player_bind_layout: wgpu::BindGroupLayout,
    player_uniform: wgpu::Buffer,
    player_vbuf: wgpu::Buffer,
    player_ibuf: wgpu::Buffer,
    player_bind: wgpu::BindGroup,
    player_sampler: wgpu::Sampler,
    /// 玩家索引切片（第一人称手持 pass 只画右臂切片用）。
    player_index_slices: [[Range<u32>; PART_COUNT]; SKIN_LAYERS as usize],
    skins_loaded: bool,
    /// 生物管线：复用 player 管线（同顶点格式/同 uniform 布局），只换贴图
    /// 数组 bind group + 独立顶点/索引缓冲。uniform 按 `MOB_MAX_INSTANCES`
    /// 个槽位动态偏移（256B 对齐）切分，每槽 view_proj + 部位矩阵。
    mob_uniform: wgpu::Buffer,
    mob_vbuf: wgpu::Buffer,
    mob_ibuf: wgpu::Buffer,
    mob_bind: wgpu::BindGroup,
    mob_index_ranges: [(u32, u32); crate::mob_mesh::MOB_KIND_COUNT],
    mobs_loaded: bool,
    crack_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    /// 裂纹 quad 顶点/索引（每帧覆写，最多 6 面 × 4 顶点 / 36 索引）。
    overlay_vbuf: wgpu::Buffer,
    overlay_ibuf: wgpu::Buffer,
    /// 描边 12 条棱 = 24 顶点，创建时一次性上传（单位立方体，origin 定位）。
    outline_vbuf: wgpu::Buffer,
    /// 设备纹理数组层数是否容得下 CRACK_BASE..（GLES 256 层钳制时为 false，
    /// 只画描边不画裂纹）。
    crack_layers_ok: bool,
    /// 生物群系染色基色（FrameUniforms 的 tint_grass/tint_foliage 初值，
    /// 创建期由 plains 基线算出；colormap 缺失时 w=0 禁用染色）。
    tint_grass: [f32; 4],
    tint_foliage: [f32; 4],
    /// 粒子渲染（独立小 draw；纹理/管线在 ParticleRenderer 内）。
    particles: ParticleRenderer,
    particle_bind: wgpu::BindGroup,
    /// 第一人称手持：物品图标小管线（顶点布局复用 player）+ 手持方块
    /// 立方体缓冲（每帧覆写，CrackVertex 布局直通 terrain 管线）。
    hand_icon_pipeline: wgpu::RenderPipeline,
    hand_uniform: wgpu::Buffer,
    hand_bind: wgpu::BindGroup,
    hand_terrain_vbuf: wgpu::Buffer,
    hand_terrain_ibuf: wgpu::Buffer,
    pub max_chunks: u32,
    pub max_hud_quads: u32,
}

/// origins_buf 末尾保留的 overlay/outline 专用 dynamic-offset 槽。
const OVERLAY_ORIGIN_PAD: u64 = 64;

/// 玩家管线 uniform:view_proj + 12 部位模型矩阵(mat4x4 align 16,无填充)。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PlayerUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub models: [[[f32; 4]; 4]; PART_COUNT],
}

const _: () = assert!(size_of::<PlayerUniforms>() == 832);

/// player_uniform 的手持 pass 右臂槽偏移（槽 1，256B 动态偏移对齐）。
/// 槽 0 = 世界 pass 玩家本体，槽 1 = 第一人称手持右臂，两者同一 submit
/// 内不争写（queue.write_buffer 无法插入两个 render pass 之间）。
const PLAYER_HAND_SLOT_OFF: u64 = (size_of::<PlayerUniforms>() as u64).next_multiple_of(256);

/// 单 quad 索引（手持物品图标路径；与方块展开首 quad 的角序一致）。
const QUAD_INDICES: [u32; 6] = [0, 1, 2, 0, 2, 3];

fn terrain_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: TERRAIN_STRIDE as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint16x2,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint16,
                offset: 16,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x2,
                offset: 18,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x2,
                offset: 20,
                shader_location: 4,
            },
        ],
    }
}

fn hud_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: HUD_STRIDE as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Unorm8x4,
                offset: 16,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32x2,
                offset: 20,
                shader_location: 3,
            },
        ],
    }
}

fn player_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: PLAYER_STRIDE as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Unorm8x2,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32x2,
                offset: 16,
                shader_location: 2,
            },
        ],
    }
}

const _: () = assert!(size_of::<PlayerVertex>() == PLAYER_STRIDE);

/// 生成方块图集的 texture_2d_array 绑定声明 + 按 layer 区间选数组的采样
/// 函数（terrain.wgsl / hud.wgsl 的 @@...@@ 占位由 gpu.rs 烘入，与创建的
/// 数组 counts 同源，永不错位）。
///
/// 单数组（上限 ≥ atlas::LAYERS）时生成的就是一次 textureSample——与拆分前逐字节
/// 同语义，`min(layer, N-1)` 对合法层号是恒等（仅防御 mesher 异常层号）。
/// 多数组（GLES 保底 256）时按区间 if 链：
/// - 分支条件是 `@interpolate(flat)` 层号，逐图元一致，textureSample 的
///   隐式导数在各图元内仍是良定义的；
/// - naga 30.0.1 已不对 fragment 阶段强制 uniform 控制流
///   （naga-30.0.1 src/valid/analyzer.rs:23
///   DISABLE_UNIFORMITY_REQ_FOR_FRAGMENT_STAGE = true），if 链内
///   textureSample 可过 create_shader_module 校验；
/// - 每分支 min() 把局部层号钉死在本数组内——wgpu/naga GLES 后端对数组
///   层不做任何钳制（naga-30.0.1 src/back/glsl/writer.rs:2635-2639 裸拼
///   layer 分量；GLSL ES 3.0 §8.8 越界层结果未定义，Mali 实测常返回
///   透明黑 → fs_terrain alpha<0.5 discard → 「雾色平色面」），故越界
///   在这里从源头杜绝。
fn atlas_arrays_wgsl(counts: &[usize], bindings: &[u32], sampler: &str, fn_name: &str) -> String {
    assert_eq!(counts.len(), bindings.len());
    assert!(!counts.is_empty(), "图集至少要有一个数组");
    let mut s = String::new();
    // 占位标记写在 `// ` 注释行内，replace 只换标记本身——首行前补换行，
    // 让残留的 `// ` 孤立成空注释行，生成的声明才不会被注释掉。
    s.push('\n');
    for (i, &b) in bindings.iter().enumerate() {
        s.push_str(&format!(
            "@group(0) @binding({b}) var terrain_tex{i}: texture_2d_array<f32>;\n"
        ));
    }
    s.push_str(&format!(
        "fn {fn_name}(uv: vec2<f32>, layer: u32) -> vec4<f32> {{\n"
    ));
    let mut acc = 0usize;
    for (i, &cnt) in counts.iter().enumerate() {
        let last = i + 1 == counts.len();
        let local = if acc == 0 {
            format!("min(layer, {}u)", cnt - 1)
        } else {
            format!("min(layer - {}u, {}u)", acc, cnt - 1)
        };
        if last {
            // WGSL 规定 else 后只能是复合块或 if 语句，裸 return 不合法。
            if i > 0 {
                s.push_str(" {\n");
            }
            s.push_str(&format!(
                "    return textureSample(terrain_tex{i}, {sampler}, uv, {local});\n"
            ));
            if i > 0 {
                s.push_str("    }\n");
            }
        } else {
            s.push_str(&format!(
                "    if (layer < {}u) {{\n        return textureSample(terrain_tex{i}, {sampler}, uv, {local});\n    }} else ",
                acc + cnt
            ));
        }
        acc += cnt;
    }
    s.push_str("}\n");
    s
}

impl Renderer {
    /// 单数组图集（桌面/Vulkan 主路径）。
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        assets_dir: Option<&std::path::Path>,
    ) -> Self {
        Self::with_atlas_layer_cap(device, queue, color_format, assets_dir, None)
    }

    /// `atlas_layer_cap = Some(n)`：把单数组层数上限强制为
    /// min(n, 设备上限)（测试钩子——CI lavapipe 上限 3907，用它模拟
    /// GLES 256 层设备走多数组路径）。
    pub fn with_atlas_layer_cap(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        assets_dir: Option<&std::path::Path>,
        atlas_layer_cap: Option<u32>,
    ) -> Self {
        let max_chunks: u32 = 1024;
        let max_hud_quads: u32 = 4096;

        // ---- AssetManager（M8c 资源管线统一）--------------------------------
        // 全部素材 IO 经 mcv_assets（缓存去重 + 缺素材硬错误登记，错误含
        // 完整路径）；构造尾部的 summary_report 一次性汇总上报缺失清单，
        // 显示层降级仍按素材红线（missing 标记/透明占位，绝无程序化伪造）。
        let assets_mgr = assets_dir.map(mcv_assets::AssetManager::new);

        // ---- terrain texture arrays ------------------------------------
        // 真实官方贴图 827 层 + missing 哨兵 + 裂纹 10 层 = atlas::LAYERS。
        // 设备 max_texture_array_layers < LAYERS（GLES 规范下限 256）时按
        // atlas::split_layer_counts 拆成 N 个 texture_2d_array，
        // terrain.wgsl/hud.wgsl 由 gpu.rs 展开 layer 区间 if 链选数组；
        // 上限 ≥ LAYERS（桌面/Vulkan）保持单数组零分支零回归。
        let device_max = device.limits().max_texture_array_layers;
        let per = atlas_layer_cap.unwrap_or(device_max).min(device_max).max(1) as usize;
        let counts = atlas::split_layer_counts(per);
        let n_layers: usize = counts.iter().sum();
        // 图集容量取证（2026-10-10 真机 Mali「平色面」定位）：error 级常显，
        // 与 app.rs 的 adapter 侧 `atlas-cap` 行配对。真机 logcat 必见此行：
        // truncated=false 且 arrays=[838] 说明设备没被卡，崩坏另有根因。
        log::error!(
            "atlas-cap: device max_texture_array_layers={} max_texture_dimension_2d={} atlas::LAYERS={} arrays={:?} created={} truncated={} crack_layers_ok={}",
            device_max,
            device.limits().max_texture_dimension_2d,
            atlas::LAYERS,
            counts,
            n_layers,
            n_layers < atlas::LAYERS,
            n_layers > atlas::CRACK_BASE,
        );
        // 图集源贴图 → 838 层 payload：rebuild() 语义入口（AssetManager 接管，
        // 源文件缺失清单可经 atlas::missing_source_files / atlas_missing 查询）。
        let payload = match &assets_mgr {
            Some(am) => am.rebuild_atlas_payload(),
            None => atlas::generate_payload(),
        };
        let mip0_layer_bytes = atlas::TILE_PX * atlas::TILE_PX * 4;
        let mip1_layer_bytes = (atlas::TILE_PX / 2) * (atlas::TILE_PX / 2) * 4;
        assert_eq!(
            payload.len(),
            atlas::LAYERS * (mip0_layer_bytes + mip1_layer_bytes),
            "atlas payload 布局与拆分假设不符"
        );
        let mip0_total = atlas::LAYERS * mip0_layer_bytes;
        let mut terrain_views: Vec<wgpu::TextureView> = Vec::with_capacity(counts.len());
        let mut layer_base = 0usize;
        for (i, &cnt) in counts.iter().enumerate() {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("terrain-array{i}")),
                size: wgpu::Extent3d {
                    width: atlas::TILE_PX as u32,
                    height: atlas::TILE_PX as u32,
                    depth_or_array_layers: cnt as u32,
                },
                mip_level_count: atlas::MIP_LEVELS,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &payload[layer_base * mip0_layer_bytes..(layer_base + cnt) * mip0_layer_bytes],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((atlas::TILE_PX * 4) as u32),
                    rows_per_image: Some(atlas::TILE_PX as u32),
                },
                wgpu::Extent3d {
                    width: atlas::TILE_PX as u32,
                    height: atlas::TILE_PX as u32,
                    depth_or_array_layers: cnt as u32,
                },
            );
            // mip1 必须单独上传：payload 尾部是 box 下采样结果（层序同 mip0）。
            // 采样器 mipmap_filter=Nearest 会把 LOD≥0.5 直接舍入到 mip1，
            // 漏传则采到未定义内容（lavapipe 清零 → 裂纹 discard、远景发黑）。
            let mip1_off = mip0_total + layer_base * mip1_layer_bytes;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 1,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &payload[mip1_off..mip1_off + cnt * mip1_layer_bytes],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((atlas::TILE_PX / 2) as u32 * 4),
                    rows_per_image: Some((atlas::TILE_PX / 2) as u32),
                },
                wgpu::Extent3d {
                    width: (atlas::TILE_PX / 2) as u32,
                    height: (atlas::TILE_PX / 2) as u32,
                    depth_or_array_layers: cnt as u32,
                },
            );
            terrain_views.push(tex.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }));
            layer_base += cnt;
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            ..Default::default()
        });

        // ---- 生物群系染色（26.1 BlockColors / GrassColor 等价）-----------
        // ① LUT：tile 层号 → tint 类别（vec4<u32>(kind, r, g, b)，std140
        //   stride 16）。草/叶族乘 FrameUniforms 基色，云杉/白桦常量色烤进
        //   LUT；裂纹/哨兵层 kind=0 不染色。
        let tint_lut_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tint-lut"),
            size: (atlas::LAYERS * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&tint_lut_buf, 0, &mcv_core::tint::tint_lut_bytes());
        // ② plains 基线草/叶色：colormap PNG（原版 GrassColorReloadListener
        //   同款 256x256 查表图）。缺失 → log::error + 关闭染色（草地按
        //   原版灰度贴图原样显示，不伪造颜色）。
        let load_colormap = |file: &str| -> Option<Vec<u8>> {
            let am = assets_mgr.as_ref()?;
            let bytes = am.colormap(file).ok()?;
            mcv_core::tint::decode_colormap(&bytes)
        };
        let (tint_grass, tint_foliage) =
            match (load_colormap("grass.png"), load_colormap("foliage.png")) {
                (Some(g), Some(f)) => {
                    let (g, f) = mcv_core::tint::world_grass_foliage_color(&g, &f)
                        .expect("colormap 已按 256x256 校验");
                    ([g[0], g[1], g[2], 1.0], [f[0], f[1], f[2], 1.0])
                }
                _ => {
                    log::error!(
                        "colormap 素材缺失（textures/colormap/{{grass,foliage}}.png）\
——生物群系染色禁用，草/树叶按灰度贴图原样显示（不伪造颜色）"
                    );
                    ([1.0, 1.0, 1.0, 0.0], [1.0, 1.0, 1.0, 0.0])
                }
            };

        // ---- celestial texture array（太阳 + 8 月相）--------------------
        // 原版 26.1 日月为贴图 quad（SkyRenderer.java:125-127/:149-157），
        // 素材 environment/celestial/{sun.png, moon/<phase>.png}。素材缺失
        // → 素材红线（2026-10）：删除程序化天体圆盘回退，上传全透明纹理、
        // log::error，天空保持无天体——绝不画假太阳/假月亮。
        let celestial_payload = assets_mgr.as_ref().and_then(celestial::load_payload_via);
        if celestial_payload.is_none() {
            log::error!(
                "天体贴图缺失：textures/environment/celestial/{{sun.png,moon/*.png}}\
——天空将没有太阳与月亮（无程序化回退，请检查部署的 assets/）"
            );
        }
        let celestial_data = celestial_payload
            .unwrap_or_else(|| vec![0u8; celestial::CELESTIAL_LAYERS * 32 * 32 * 4]);
        let celestial_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("celestial-array"),
            size: wgpu::Extent3d {
                width: celestial::CELESTIAL_PX as u32,
                height: celestial::CELESTIAL_PX as u32,
                depth_or_array_layers: celestial::CELESTIAL_LAYERS as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &celestial_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &celestial_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((celestial::CELESTIAL_PX * 4) as u32),
                rows_per_image: Some(celestial::CELESTIAL_PX as u32),
            },
            wgpu::Extent3d {
                width: celestial::CELESTIAL_PX as u32,
                height: celestial::CELESTIAL_PX as u32,
                depth_or_array_layers: celestial::CELESTIAL_LAYERS as u32,
            },
        );
        let celestial_view = celestial_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let celestial_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("celestial-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // ---- font texture ---------------------------------------------
        // 原版字体贴图（textures/font/ascii.png + BitmapProvider 度量）。
        // 缺失/解码失败 → 素材红线（2026-10）：不再回退程序化 8x8 字体，
        // log::error 并以全透明纹理占位（HUD 文字整体不上屏）。
        let (font_data, font_widths) = match font::load_atlas(assets_dir) {
            Ok(v) => v,
            Err(e) => {
                log::error!("{e}——HUD 文字将不上屏");
                (vec![0u8; font::TEX_W * font::TEX_H * 4], [0u8; 256])
            }
        };
        font::install_widths(font_widths);
        let font_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("font"),
            size: wgpu::Extent3d {
                width: font::TEX_W as u32,
                height: font::TEX_H as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        {
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("font-staging"),
                contents: &font_data,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some((font::TEX_W * 4) as u32),
                        rows_per_image: Some(font::TEX_H as u32),
                    },
                },
                font_tex.as_image_copy(),
                wgpu::Extent3d {
                    width: font::TEX_W as u32,
                    height: font::TEX_H as u32,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([enc.finish()]);
        }
        let font_view = font_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // ---- GUI 精灵表(资源根 textures/ 下原版精灵)-------------------
        // 缺素材时建 1x1 占位纹理，gui 字段为 None → 上层按素材红线显示
        // 加载失败提示（无程序化面板回退）。
        let gui = assets_mgr.as_ref().and_then(SpriteSheet::load_via);
        let (gui_rgba, gui_w, gui_h) = match &gui {
            Some(s) => (s.rgba.clone(), s.w, s.h),
            None => (vec![0u8; 4], 1, 1),
        };
        let gui_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui-sprites"),
            size: wgpu::Extent3d {
                width: gui_w,
                height: gui_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        {
            let bpr = gui_w as usize * 4;
            // COPY_BUFFER_ALIGN = 256:staging 行需补齐到 256 字节
            let padded_bpr = bpr.div_ceil(256) * 256;
            let mut padded = vec![0u8; padded_bpr * gui_h as usize];
            for row in 0..gui_h as usize {
                let s = row * bpr..(row + 1) * bpr;
                let d = row * padded_bpr..row * padded_bpr + bpr;
                padded[d].copy_from_slice(&gui_rgba[s]);
            }
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gui-staging"),
                contents: &padded,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bpr as u32),
                        rows_per_image: Some(gui_h),
                    },
                },
                gui_tex.as_image_copy(),
                wgpu::Extent3d {
                    width: gui_w,
                    height: gui_h,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([enc.finish()]);
        }
        let gui_view = gui_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let hud_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hud-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // ---- bind layouts ---------------------------------------------
        // 追加图集数组槽（binding 5..）：仅当设备上限 < atlas::LAYERS 拆
        // 多数组时存在（4 固定给 tint LUT，见 frame_layout_entries 尾部）。
        let atlas_d2_array = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let mut frame_layout_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            },
            atlas_d2_array(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // 生物群系染色 LUT（层号 → tint 类别，mcv_core::tint）。
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        for i in 1..terrain_views.len() {
            frame_layout_entries.push(atlas_d2_array(4 + i as u32));
        }
        let frame_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame-layout"),
            entries: &frame_layout_entries,
        });
        let sky_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 天体纹理数组（太阳 + 8 月相）：原版日月为贴图 quad，
                // fs_sky 解析投影采样（SkyRenderer.java:125-157）。
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let mut hud_layout_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            atlas_d2_array(3),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ];
        // 追加图集数组槽（binding 6..8）：物品图标与 terrain 同规则选数组。
        for i in 1..terrain_views.len() {
            hud_layout_entries.push(atlas_d2_array(5 + i as u32));
        }
        let hud_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud-layout"),
            entries: &hud_layout_entries,
        });

        // unifont CJK 位图图集（text.rs 经 OnceLock 用同一份 cjk.f16 生成 quad）
        let unifont_view = {
            let mk_white = || {
                let t = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("unifont-placeholder"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    t.as_image_copy(),
                    &[255, 255, 255, 255],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                t.create_view(&wgpu::TextureViewDescriptor::default())
            };
            match crate::text::unifont_shared() {
                None => Some(mk_white()),
                Some(u) => {
                    let (rgba, w, h) = u.atlas_rgba();
                    let max = device.limits().max_texture_dimension_2d;
                    if w as u32 > max || h as u32 > max {
                        log::warn!("unifont atlas {w}x{h} > device max {max}, CJK disabled");
                        Some(mk_white())
                    } else {
                        let t = device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("unifont-atlas"),
                            size: wgpu::Extent3d {
                                width: w as u32,
                                height: h as u32,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });
                        queue.write_texture(
                            t.as_image_copy(),
                            rgba,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some((w * 4) as u32),
                                rows_per_image: Some(h as u32),
                            },
                            wgpu::Extent3d {
                                width: w as u32,
                                height: h as u32,
                                depth_or_array_layers: 1,
                            },
                        );
                        Some(t.create_view(&wgpu::TextureViewDescriptor::default()))
                    }
                }
            }
        };

        // ---- buffers ---------------------------------------------------
        let frame_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame-uniforms"),
            size: size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let origins_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-origins"),
            // 末尾多留 2 槽：overlay 的 origin（dynamic offset =
            // max_chunks * 256）与手持方块 origin（(max_chunks+1) * 256，
            // wgpu 建缓冲零初始化 = (0,0,0)，方块顶点已是世界坐标直通），
            // 复用 terrain/water 的绑定组布局。
            size: (max_chunks as u64 + 2) * 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky-uniforms"),
            size: 176, // inv_view_proj 64 + 3 vec4
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud-uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud-vbuf"),
            size: (max_hud_quads as u64) * 4 * HUD_STRIDE as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_ibuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud-ibuf"),
            size: (max_hud_quads as u64) * 6 * 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ---- particles -------------------------------------------------
        // 粒子渲染器（原版 textures/particle 贴图）；frame uniform 复用
        // frame_buf，方块图集/sampler 复用 terrain 首数组（多数组拆分设备上
        // 碎屑仅数组 0 的层有真贴图，其余层采样钳在末层——粒子为装饰可接受，
        // 真机层数 ≥838 单数组无此问题）。
        let particles = ParticleRenderer::new(&device, &queue, color_format, assets_dir);
        let particle_bind =
            particles.build_bind_group(&device, &frame_buf, &terrain_views[0], &sampler);

        // ---- bind groups ----------------------------------------------
        // terrain 图集数组槽：binding 2 = terrain_views[0]，追加数组 5/6/7
        // （binding 4 固定给生物群系染色 LUT——tint_lut 在 terrain.wgsl 是
        // 静态声明，不能随数组数漂移）。
        let mut frame_bind_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &origins_buf,
                    offset: 0,
                    size: std::num::NonZeroU64::new(16),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&terrain_views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: tint_lut_buf.as_entire_binding(),
            },
        ];
        for (i, view) in terrain_views.iter().enumerate().skip(1) {
            frame_bind_entries.push(wgpu::BindGroupEntry {
                binding: 4 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let frame_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame-bind"),
            layout: &frame_bind_layout,
            entries: &frame_bind_entries,
        });
        let sky_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-bind"),
            layout: &sky_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sky_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&celestial_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&celestial_sampler),
                },
            ],
        });
        // hud 图集数组槽：binding 3 = terrain_views[0]，追加数组 6/7/8。
        let mut hud_bind_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: hud_uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&font_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&hud_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&terrain_views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&gui_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(
                    unifont_view.as_ref().expect("unifont view"),
                ),
            },
        ];
        for (i, view) in terrain_views.iter().enumerate().skip(1) {
            hud_bind_entries.push(wgpu::BindGroupEntry {
                binding: 5 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let hud_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hud-bind"),
            layout: &hud_bind_layout,
            entries: &hud_bind_entries,
        });

        // ---- pipelines -------------------------------------------------
        // 图集数组声明+采样函数按设备层数烘进 shader（与 counts 同源，见
        // atlas_arrays_wgsl）；terrain 图集绑定在 group0 的 2(+5/6/7)
        // （4 固定给 tint LUT），hud 在 3(+6/7/8)。
        let terrain_arrays = atlas_arrays_wgsl(
            &counts,
            &[2, 5, 6, 7][..counts.len()],
            "terrain_samp",
            "sample_terrain",
        );
        let hud_arrays = atlas_arrays_wgsl(
            &counts,
            &[3, 6, 7, 8][..counts.len()],
            "hud_samp",
            "sample_terrain_icon",
        );
        let terrain_src =
            include_str!("../assets/terrain.wgsl").replace("@@TERRAIN_ARRAYS@@", &terrain_arrays);
        let hud_src =
            include_str!("../assets/hud.wgsl").replace("@@HUD_TERRAIN_ARRAYS@@", &hud_arrays);
        // 占位替换必须生效，否则 shader 里 sample_terrain 无定义（naga 报错
        // 难定位到模板层），在这里直接把错抛出来。
        assert!(!terrain_src.contains("@@"), "terrain.wgsl 占位未替换");
        assert!(!hud_src.contains("@@"), "hud.wgsl 占位未替换");
        let frame_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(terrain_src.into()),
        });
        let sky_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/sky.wgsl").into()),
        });
        let hud_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud"),
            source: wgpu::ShaderSource::Wgsl(hud_src.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world-layout"),
            bind_group_layouts: &[Some(&frame_bind_layout)],
            immediate_size: 0,
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky-pipeline-layout"),
            bind_group_layouts: &[Some(&sky_bind_layout)],
            immediate_size: 0,
        });
        let hud_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud-pipeline-layout"),
            bind_group_layouts: &[Some(&hud_bind_layout)],
            immediate_size: 0,
        });

        let depth_stencil = Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        let depth_read_only = Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_mod,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[Some(terrain_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frame_mod,
                entry_point: Some("fs_terrain"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let water_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_mod,
                entry_point: Some("vs_water"),
                compilation_options: Default::default(),
                buffers: &[Some(terrain_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frame_mod,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: depth_read_only.clone(),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // 裂纹：复用 vs_terrain 与 terrain 顶点布局，片元 alpha 混合；
        // 顶点 CPU 侧外偏 0.003，深度只读不写。
        let crack_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mining-crack"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_mod,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[Some(terrain_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frame_mod,
                entry_point: Some("fs_crack"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: depth_read_only.clone(),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // 描边：LineList，pos-only 顶点；12 条棱 24 顶点创建时传一次。
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("block-outline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_mod,
                entry_point: Some("vs_outline"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frame_mod,
                entry_point: Some("fs_outline"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: depth_read_only,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let overlay_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("overlay-vbuf"),
            size: 6 * 4 * TERRAIN_STRIDE as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let overlay_ibuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("overlay-ibuf"),
            size: 6 * 6 * 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 单位立方体 12 条棱（LineList），外扩 0.002 防与方块面 z-fighting。
        let outline_vbuf = {
            let e = 0.002f32;
            let (lo, hi) = (-e, 1.0 + e);
            // 8 角索引位打包：bit0=x bit1=y bit2=z
            let corners = |i: u32| {
                [
                    if i & 1 == 0 { lo } else { hi },
                    if i & 2 == 0 { lo } else { hi },
                    if i & 4 == 0 { lo } else { hi },
                ]
            };
            let edges: [[u32; 2]; 12] = [
                [0, 1],
                [2, 3],
                [4, 5],
                [6, 7], // X 向
                [0, 2],
                [1, 3],
                [4, 6],
                [5, 7], // Y 向
                [0, 4],
                [1, 5],
                [2, 6],
                [3, 7], // Z 向
            ];
            let mut verts = Vec::with_capacity(24);
            for [a, b] in edges {
                verts.extend_from_slice(&corners(a));
                verts.extend_from_slice(&corners(b));
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("outline-vbuf"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            })
        };
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState {
                module: &sky_mod,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &sky_mod,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let hud_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud"),
            layout: Some(&hud_layout),
            vertex: wgpu::VertexState {
                module: &hud_mod,
                entry_point: Some("vs_hud"),
                compilation_options: Default::default(),
                buffers: &[Some(hud_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &hud_mod,
                entry_point: Some("fs_hud"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        // ---- player pipeline -------------------------------------------
        let player_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("player"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/player.wgsl").into()),
        });
        let player_bind_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("player-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            // mob 管线复用本 layout，按 MOB_MAX_INSTANCES 个
                            // 256B 对齐槽位动态偏移切分同一 uniform 条带；
                            // 玩家本体恒用偏移 0。
                            has_dynamic_offset: true,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            // nearest 采样不要求 filterable float(sRGB array 在
                            // GLES 上也不满足 filterable 要求)
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                        count: None,
                    },
                ],
            });
        let player_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("player-pipeline-layout"),
            bind_group_layouts: &[Some(&player_bind_layout)],
            immediate_size: 0,
        });
        let player_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("player"),
            layout: Some(&player_layout),
            vertex: wgpu::VertexState {
                module: &player_mod,
                entry_point: Some("vs_player"),
                compilation_options: Default::default(),
                buffers: &[Some(player_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &player_mod,
                entry_point: Some("fs_player"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None, // cutout:shader 内 alpha discard
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                // player_mesh 的角点序经镜像变换(行列式 -1)后从盒外看为 CW,
                // 与 wgpu 默认 CCW 前置相反;若剔背面会只剩内壁。剔正面又会
                // 在 overlay discard 处透出内背壁。两难之下不剔(每盒 ≤24 三角,
                // 代价可忽略),由深度测试取胜者。
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let mesh = player_mesh::build_player_mesh();
        let player_index_slices = mesh.slices.clone();
        let player_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("player-uniforms"),
            // 槽 0 = 世界 pass 玩家本体；槽 1（PLAYER_HAND_SLOT_OFF）=
            // 第一人称手持右臂（独立槽位，手持 pass 与世界 pass 同一次
            // submit 内不争写同一 uniform）。
            size: PLAYER_HAND_SLOT_OFF + size_of::<PlayerUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let player_vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("player-vbuf"),
            contents: bytemuck::cast_slice(&mesh.verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let player_ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("player-ibuf"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let player_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("player-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        // 皮肤未加载前的 1x1x2 全透明占位(draw_player 亦以 skins_loaded 短路)。
        let placeholder = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("player-skin-placeholder"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: SKIN_LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let player_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("player-bind-placeholder"),
            layout: &player_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    // 带动态偏移的 uniform 绑定必须显式给 size（玩家恒占
                    // 偏移 0 的一个 PlayerUniforms 槽位）。
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &player_uniform,
                        offset: 0,
                        size: std::num::NonZeroU64::new(size_of::<PlayerUniforms>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&placeholder.create_view(
                        &wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            ..Default::default()
                        },
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&player_sampler),
                },
            ],
        });

        // ---- first-person hand icon pipeline ----------------------------
        // 世界 pass 之后清深度的独立小 pass 用（26.1 renderItemInHand 时机）。
        // 顶点布局复用 player（pos f32x3 + uv unorm8x2 + meta u32x2）。
        let hand_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hand-icon"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../assets/hand.wgsl").into()),
        });
        let hand_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hand-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let hand_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hand-pipeline-layout"),
            bind_group_layouts: &[Some(&hand_bind_layout)],
            immediate_size: 0,
        });
        let hand_icon_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hand-icon"),
            layout: Some(&hand_layout),
            vertex: wgpu::VertexState {
                module: &hand_mod,
                entry_point: Some("vs_hand_icon"),
                compilation_options: Default::default(),
                buffers: &[Some(player_vertex_layout())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &hand_mod,
                entry_point: Some("fs_hand_icon"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None, // cutout：shader 内 alpha discard
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                // 单 quad 双面（挥臂转向后不凭角序赌朝向）。
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let hand_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand-uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hand_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hand-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        // 图标绑定组在构造期捕获 GUI 精灵表视图（图集运行期不重建，
        // 与 hud_bind 同生命周期约定），hand_uniform 每帧在 draw_hand 写入。
        let hand_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand-bind"),
            layout: &hand_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: hand_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&gui_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&hand_sampler),
                },
            ],
        });
        // 手持方块立方体：CrackVertex 布局（terrain 管线直通），每帧覆写。
        let hand_terrain_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand-terrain-vbuf"),
            size: (TERRAIN_STRIDE * 24) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hand_terrain_ibuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand-terrain-ibuf"),
            size: (4 * 36) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ---- mob pipeline：复用 player 管线，只换贴图数组 + 独立缓冲 ------
        let mob = crate::mob_mesh::build_mob_mesh();
        let mob_ranges = mob.slices.map(|r| (r.start, r.end));
        // uniform 槽位步长：PlayerUniforms(832B) 向上对齐 256（动态偏移对齐）。
        let mob_stride = size_of::<PlayerUniforms>().next_multiple_of(256) as u64;
        let mob_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mob-uniforms"),
            size: mob_stride * crate::mob_mesh::MOB_MAX_INSTANCES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mob_vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mob-vbuf"),
            contents: bytemuck::cast_slice(&mob.verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mob_ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mob-ibuf"),
            contents: bytemuck::cast_slice(&mob.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        // 原版 mob 贴图（entity/{chicken,cow,sheep,pig}/...）：素材缺失时
        // mobs_loaded=false 短路不画，绝不程序化伪造。
        let mob_payload = assets_mgr
            .as_ref()
            .and_then(crate::mob_mesh::load_mob_payload_via);
        let mobs_loaded = mob_payload.is_some();
        let mob_tex_data = mob_payload.unwrap_or_else(|| {
            vec![
                0u8;
                crate::mob_mesh::MOB_TEX_LAYERS
                    * crate::mob_mesh::MOB_TEX_PX
                    * crate::mob_mesh::MOB_TEX_PX
                    * 4
            ]
        });
        let mob_px = crate::mob_mesh::MOB_TEX_PX as u32;
        let mob_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mob-textures"),
            size: wgpu::Extent3d {
                width: mob_px,
                height: mob_px,
                depth_or_array_layers: crate::mob_mesh::MOB_TEX_LAYERS as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            mob_tex.as_image_copy(),
            &mob_tex_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mob_px * 4), // 256B 对齐，无需 padding
                rows_per_image: Some(mob_px),
            },
            wgpu::Extent3d {
                width: mob_px,
                height: mob_px,
                depth_or_array_layers: crate::mob_mesh::MOB_TEX_LAYERS as u32,
            },
        );
        let mob_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mob-bind"),
            layout: &player_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &mob_uniform,
                        offset: 0,
                        size: std::num::NonZeroU64::new(size_of::<PlayerUniforms>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&mob_tex.create_view(
                        &wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            ..Default::default()
                        },
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&player_sampler),
                },
            ],
        });

        // 一次性汇总上报缺素材（硬错误诊断；显示层已按素材红线降级）。
        if let Some(am) = &assets_mgr {
            am.summary_report();
        }

        Self {
            device,
            queue,
            player_pipeline,
            player_bind_layout,
            player_uniform,
            player_vbuf,
            player_ibuf,
            player_bind,
            player_sampler,
            player_index_slices,
            skins_loaded: false,
            mob_uniform,
            mob_vbuf,
            mob_ibuf,
            mob_bind,
            mob_index_ranges: mob_ranges,
            mobs_loaded,
            terrain_pipeline,
            water_pipeline,
            sky_pipeline,
            hud_pipeline,
            frame_buf,
            origins_buf,
            sky_buf,
            hud_uniform,
            hud_vbuf,
            hud_ibuf,
            frame_bind,
            hud_bind,
            sky_bind,
            gui,
            crack_pipeline,
            outline_pipeline,
            overlay_vbuf,
            overlay_ibuf,
            outline_vbuf,
            crack_layers_ok: n_layers > atlas::CRACK_BASE,
            tint_grass,
            tint_foliage,
            particles,
            particle_bind,
            hand_icon_pipeline,
            hand_uniform,
            hand_bind,
            hand_terrain_vbuf,
            hand_terrain_ibuf,
            max_chunks,
            max_hud_quads,
            last_stats: FrameStats::default(),
        }
    }

    /// MC GUI 精灵表;None 表示资源根未带精灵，上层按素材红线显示
    /// 加载失败提示（无程序化回退）。
    pub fn gui(&self) -> Option<&SpriteSheet> {
        self.gui.as_ref()
    }

    /// （M8c）图集热替换薄接口——**留桩，已注明**：
    /// `rebuild()` 语义的 CPU 半边已由
    /// [`mcv_assets::AssetManager::rebuild_atlas_payload`] 提供（源贴图 →
    /// 838 层 payload，GLES 拆分仍走 [`atlas::split_layer_counts`] +
    /// [`atlas::remap_layer`]）；本函数转发之并返回新 payload，供调用方
    /// 在资源包变更后取新数据。**GPU 侧纹理换绑未接**：terrain 纹理数组
    /// 与 frame_bind 绑定布局一起重建属渲染线程重构范围（在排队），故当前
    /// 取到新 payload 后仍需整体重建 Renderer 才会生效。
    pub fn rebuild_atlas_payload(assets: &mcv_assets::AssetManager) -> Vec<u8> {
        assets.rebuild_atlas_payload()
    }

    /// 上传 steve/alex 皮肤为 64x64x2 texture_2d_array(layer 0=steve,
    /// 1=alex,与 PlayerVertex.meta.x 约定一致)。可在任意时刻调用(重建 bind group)。
    pub fn load_skins(&mut self, steve_png: &[u8], alex_png: &[u8]) -> Result<(), String> {
        let decode = |png: &[u8], name: &str| -> Result<Vec<u8>, String> {
            let img = image::load_from_memory(png)
                .map_err(|e| format!("{name} png: {e}"))?
                .to_rgba8();
            if img.width() != 64 || img.height() != 64 {
                return Err(format!(
                    "{name}: 需要 64x64 皮肤,得到 {}x{}",
                    img.width(),
                    img.height()
                ));
            }
            Ok(img.into_raw())
        };
        let mut data = decode(steve_png, "steve")?;
        data.extend_from_slice(&decode(alex_png, "alex")?);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("player-skins"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: SKIN_LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            tex.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64 * 4), // 256B 对齐,无需 padding
                rows_per_image: Some(64),
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: SKIN_LAYERS,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        self.player_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("player-bind"),
            layout: &self.player_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    // 同上：动态偏移绑定要求显式 size。
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.player_uniform,
                        offset: 0,
                        size: std::num::NonZeroU64::new(size_of::<PlayerUniforms>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.player_sampler),
                },
            ],
        });
        self.skins_loaded = true;
        Ok(())
    }

    /// 皮肤是否已就绪(app.rs 可用 `player_mesh::update_walk_animation` 等
    /// 计算姿态,不必在皮肤缺失时白白计算矩阵)。
    pub fn has_skins(&self) -> bool {
        self.skins_loaded
    }

    /// 在(带 Depth24Plus 深度附件的)world pass 内绘制一次玩家。几何同时含
    /// steve/alex 两套顶点(meta.x 选皮肤层),`skin_layer` 选对应索引区间;
    /// `part_models` 来自 `player_mesh::model_matrices(&pose)`。
    /// 皮肤未加载时为 no-op。
    pub fn draw_player(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        view_proj: [[f32; 4]; 4],
        part_models: &[glam::Mat4; PART_COUNT],
        skin_layer: u32,
    ) {
        if !self.skins_loaded || skin_layer >= SKIN_LAYERS {
            return;
        }
        let mut models = [[[0f32; 4]; 4]; PART_COUNT];
        for (m, dst) in part_models.iter().zip(models.iter_mut()) {
            *dst = m.to_cols_array_2d();
        }
        let u = PlayerUniforms { view_proj, models };
        self.queue
            .write_buffer(&self.player_uniform, 0, bytemuck::bytes_of(&u));
        pass.set_pipeline(&self.player_pipeline);
        // layout 声明动态偏移（mob 管线共用），玩家恒用槽位 0。
        pass.set_bind_group(0, &self.player_bind, &[0]);
        pass.set_vertex_buffer(0, self.player_vbuf.slice(..));
        pass.set_index_buffer(self.player_ibuf.slice(..), wgpu::IndexFormat::Uint32);
        // 整款一次 draw:shader 按 meta.y 逐顶点取模型矩阵、meta.x 取皮肤层。
        pass.draw_indexed(0..(PART_COUNT * player_mesh::PART_INDEXES) as u32, 0, 0..1);
    }

    /// 在 world pass 内绘制一批生物。复用 player 管线与采样器，贴图数组
    /// 为 entity/{chicken,cow,sheep,pig,zombie,skeleton,creeper,spider}/...
    /// 八种原版素材（pad 到 64x64x9）。
    /// uniform 按 256B 对齐的槽位动态偏移切分，每实例一次 write_buffer +
    /// 一次 draw_indexed。素材缺失时 no-op。
    pub fn draw_mobs(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        view_proj: [[f32; 4]; 4],
        instances: &[MobInstance],
    ) {
        if !self.mobs_loaded || instances.is_empty() {
            return;
        }
        let stride = size_of::<PlayerUniforms>().next_multiple_of(256) as u32;
        for (i, inst) in instances
            .iter()
            .enumerate()
            .take(crate::mob_mesh::MOB_MAX_INSTANCES)
        {
            let mut u = PlayerUniforms {
                view_proj,
                models: [[[0.0; 4]; 4]; PART_COUNT],
            };
            // MobInstance.models 已是 to_cols_array_2d 的列数组形（由
            // mob_render 拼装），这里逐矩阵直拷即可。
            for (m, dst) in inst.models.iter().zip(u.models.iter_mut()) {
                *dst = *m;
            }
            self.queue.write_buffer(
                &self.mob_uniform,
                (i as u64) * (stride as u64),
                bytemuck::bytes_of(&u),
            );
        }
        pass.set_pipeline(&self.player_pipeline);
        pass.set_vertex_buffer(0, self.mob_vbuf.slice(..));
        pass.set_index_buffer(self.mob_ibuf.slice(..), wgpu::IndexFormat::Uint32);
        for (i, inst) in instances
            .iter()
            .enumerate()
            .take(crate::mob_mesh::MOB_MAX_INSTANCES)
        {
            let kind = inst.kind as usize;
            if kind >= self.mob_index_ranges.len() {
                continue;
            }
            pass.set_bind_group(0, &self.mob_bind, &[i as u32 * stride]);
            let (start, end) = self.mob_index_ranges[kind];
            pass.draw_indexed(start..end, 0, 0..1);
        }
    }

    /// Renders one frame into `target` (color view + matching depth view).
    /// Chunks beyond max_chunks are ignored (frustum-culled first).
    pub fn draw_frame(
        &mut self,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        scene: &Scene,
    ) {
        // 结构性统计（#80 可观测性）：draw 数下降的代码级取证，
        // [`Renderer::last_frame_stats`] 读取。
        let mut stats = FrameStats::default();
        let cam = scene.camera;
        let eye = cam.pos + glam::Vec3::new(0.0, crate::EYE_HEIGHT, 0.0);
        let vp = cam.view_proj();

        let uniforms = FrameUniforms {
            view_proj: vp.to_cols_array_2d(),
            cam_pos_time: [eye.x, eye.y, eye.z, scene.time],
            sun_dir_day: [
                scene.sun_dir.x,
                scene.sun_dir.y,
                scene.sun_dir.z,
                scene.day_factor,
            ],
            // 雾：常规 = 基线 0.006 × 天气乘子（雨雾距收缩的 exp 映射，
            // AtmosphericFogEnvironment.java:70-73）；水下 = 高密度短视距
            // 覆盖（26.1 水下能见度骤减；fog_factor = exp2(-dist·x)，
            // x=0.05 → 20 m 处透过 0.5）。
            fog_params: if scene.underwater {
                [0.05, 0.0, 32.0, 0.0]
            } else {
                [0.006 * scene.fog_density_mult, 0.0, cam.far * 0.95, 0.0]
            },
            tint_grass: self.tint_grass,
            tint_foliage: self.tint_foliage,
        };
        self.queue
            .write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&uniforms));

        let inv_vp = vp.inverse();
        let mut sky_u = [0f32; 44];
        sky_u[..16].copy_from_slice(&inv_vp.to_cols_array());
        let eye_arr = [eye.x, eye.y, eye.z, scene.time];
        sky_u[16..20].copy_from_slice(&eye_arr);
        let sun_arr = [
            scene.sun_dir.x,
            scene.sun_dir.y,
            scene.sun_dir.z,
            scene.day_factor,
        ];
        sky_u[20..24].copy_from_slice(&sun_arr);
        // 地平线基色 × 天气雾色乘子（雨/雷变灰变暗，天空 + 地形雾同源）。
        let hor = [
            0.62 * scene.fog_tint[0],
            0.76 * scene.fog_tint[1],
            0.95 * scene.fog_tint[2],
            0.0,
        ];
        sky_u[24..28].copy_from_slice(&hor);
        // 天体参数：x = 月相序（MoonPhase 序 0..7）、y = 自由（旧「素材缺失
        // → 程序化圆盘回退」开关已随红线删除）。
        sky_u[28..32].copy_from_slice(&[scene.moon_phase as f32, 0.0, 0.0, 0.0]);
        self.queue
            .write_buffer(&self.sky_buf, 0, bytemuck::cast_slice(&sky_u));

        // frustum cull + slot assignment
        // 不变式：origins buffer 按 visible 序**连续打包**（slot=枚举序号），
        // 所有 draw 循环必须以 enumerate 序号取 origin。曾因存 scene.chunks
        // 原始索引而剔除任一区块后其后全部区块画错位置（真机「动一下山没了」）。
        let frustum = Frustum::from_view_proj(&vp);
        let mut visible: Vec<&RenderChunk> = Vec::with_capacity(64);
        for (i, rc) in scene.chunks.iter().enumerate() {
            if (i as u32) >= self.max_chunks {
                break;
            }
            let (min, max) = rc.aabb;
            if frustum.intersects_aabb(min, max) {
                visible.push(rc);
            }
        }
        stats.visible_entries = visible.len() as u32;
        if !visible.is_empty() {
            let mut origins = Vec::with_capacity(visible.len() * 64);
            for rc in &visible {
                origins.extend_from_slice(&[rc.origin[0], rc.origin[1], rc.origin[2], 0.0]);
                origins.extend_from_slice(&[0.0f32; 60]); // pad slot to 256 B
            }
            self.queue
                .write_buffer(&self.origins_buf, 0, bytemuck::cast_slice(&origins));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // sky first: writes no depth, terrain overdraws it
            pass.set_pipeline(&self.sky_pipeline);
            pass.set_bind_group(0, &self.sky_bind, &[]);
            pass.draw(0..3, 0..1);

            // clouds: sky 之后、不透明之前（26.1 RenderPassOrder）。
            // 云顶点是**相机相对**坐标（offset=(−xInCell, bottomY−eyeY, −zInCell)，
            // CloudRenderer.java:151/198——26.1 相机相对管线里 ModelViewMat 不含
            // 平移）。地形走世界空间 vp，云必须换无平移视图（eye 置原点），
            // 否则平移二次叠加把整片云推出视锥（离屏测试 diff=0 的根因）。
            if let Some((clouds, settings)) = scene.cloud {
                let vp_rel = cam.proj() * glam::Mat4::look_at_rh(Vec3::ZERO, cam.dir(), Vec3::Y);
                clouds.draw(
                    &mut pass,
                    &vp_rel.to_cols_array_2d(),
                    eye,
                    scene.time,
                    settings,
                );
            }

            // opaque
            pass.set_pipeline(&self.terrain_pipeline);
            for (slot, rc) in visible.iter().enumerate() {
                let off = slot as u32 * 256;
                pass.set_bind_group(0, &self.frame_bind, &[off]);
                pass.set_vertex_buffer(0, rc.vertex_buf.slice(..));
                pass.set_index_buffer(rc.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                if !rc.opaque_range.is_empty() {
                    pass.draw_indexed(rc.opaque_range.clone(), 0, 0..1);
                    stats.opaque_draws += 1;
                }
            }

            // player: 不透明地形后、水前（entity 在 translucent 之前渲染）
            if let Some((models, skin)) = scene.player {
                self.draw_player(&mut pass, vp.to_cols_array_2d(), models, skin);
            }

            // mobs: 同序（entity 透不透明阶段，水前）
            if let Some(mobs) = scene.mobs {
                self.draw_mobs(&mut pass, vp.to_cols_array_2d(), mobs);
            }

            // 挖掘裂纹 + 选中描边：不透明后、水前（26.1 translucent 序）。
            if let Some(ov) = scene.overlay {
                let overlay_off = self.max_chunks * 256;
                let mut slot = [0.0f32; OVERLAY_ORIGIN_PAD as usize];
                slot[..3].copy_from_slice(&ov.min);
                self.queue.write_buffer(
                    &self.origins_buf,
                    overlay_off as u64,
                    bytemuck::cast_slice(&slot),
                );
                if ov.crack_stage.is_some() && self.crack_layers_ok {
                    let (verts, idx) = build_crack_overlay(&ov);
                    if !idx.is_empty() {
                        self.queue.write_buffer(
                            &self.overlay_vbuf,
                            0,
                            bytemuck::cast_slice(&verts),
                        );
                        self.queue
                            .write_buffer(&self.overlay_ibuf, 0, bytemuck::cast_slice(&idx));
                        pass.set_pipeline(&self.crack_pipeline);
                        pass.set_bind_group(0, &self.frame_bind, &[overlay_off]);
                        pass.set_vertex_buffer(
                            0,
                            self.overlay_vbuf
                                .slice(..(verts.len() * TERRAIN_STRIDE) as u64),
                        );
                        pass.set_index_buffer(
                            self.overlay_ibuf.slice(..(idx.len() * 4) as u64),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
                    }
                }
                pass.set_pipeline(&self.outline_pipeline);
                pass.set_bind_group(0, &self.frame_bind, &[overlay_off]);
                pass.set_vertex_buffer(0, self.outline_vbuf.slice(..));
                pass.draw(0..24, 0..1);
            }

            // 粒子：裂纹 overlay 后、水前（26.1 translucent 序；
            // ParticleEngine.tick 是 20Hz 固定步，提取/绘制在渲染帧）。
            if let Some((engine, partial_tick)) = scene.particles {
                self.particles.draw(
                    &mut pass,
                    &self.particle_bind,
                    crate::particle_renderer::DrawCtx {
                        engine,
                        cam,
                        day: scene.day_factor,
                        partial_tick,
                    },
                    &self.queue,
                );
            }

            // water: far to near（#80 合批：批次条目的 water_parts 每成员区块
            // 一段，仍逐段远→近绘制——半透明混合序与逐块绘制逐值同键
            // （WaterPart.center = 成员 origin+(8,0,8)），只是索引缓冲换批次
            // 拼接缓冲、origin uniform 用批次槽位（重定基后世界坐标逐位不变）。
            pass.set_pipeline(&self.water_pipeline);
            let mut water: Vec<(f32, u32, usize)> = Vec::new();
            for (slot, rc) in visible.iter().enumerate() {
                if rc.water_index_buf.is_none() {
                    continue;
                }
                for (pi, part) in rc.water_parts.iter().enumerate() {
                    if part.range.is_empty() {
                        continue;
                    }
                    let c = Vec3::from(part.center);
                    water.push((c.distance_squared(eye), slot as u32, pi));
                }
            }
            water.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, slot, pi) in &water {
                let rc = &visible[*slot as usize];
                let part = &rc.water_parts[*pi];
                pass.set_bind_group(0, &self.frame_bind, &[*slot * 256]);
                pass.set_vertex_buffer(0, rc.vertex_buf.slice(..));
                pass.set_index_buffer(
                    rc.water_index_buf.as_ref().unwrap().slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(part.range.clone(), 0, 0..1);
                stats.water_draws += 1;
            }
        }

        // 第一人称手持 pass：世界之后、HUD 之前，同一次 submit 内**清深度**
        // 独立 pass（26.1 GameRenderer.java:724-729：clearDepthTexture →
        // renderItemInHand）。手持不被世界深度裁剪，而 HUD 又画在手持之上
        // （原版 gui 渲染序在 hand 之后）。
        if let Some(hand) = scene.hand {
            self.draw_hand(&mut encoder, color, depth, cam, hand);
        }

        // HUD pass
        if !scene.hud.is_empty() {
            // 防溢出:菜单铺贴 quad 数量超预期时截断并告警,而不是 panic
            let hud: &[HudQuad] = if scene.hud.len() as u32 > self.max_hud_quads {
                log::warn!(
                    "hud: {} quads > max {}, truncating",
                    scene.hud.len(),
                    self.max_hud_quads
                );
                &scene.hud[..self.max_hud_quads as usize]
            } else {
                scene.hud
            };
            let (verts, indices) = flatten_hud(hud);
            self.queue
                .write_buffer(&self.hud_vbuf, 0, bytemuck::cast_slice(&verts));
            self.queue
                .write_buffer(&self.hud_ibuf, 0, bytemuck::cast_slice(&indices));
            let wh = [scene.width, scene.height, scene.time, 0.0];
            self.queue
                .write_buffer(&self.hud_uniform, 0, bytemuck::cast_slice(&wh));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.hud_pipeline);
            pass.set_bind_group(0, &self.hud_bind, &[]);
            pass.set_vertex_buffer(0, self.hud_vbuf.slice(..(verts.len() * HUD_STRIDE) as u64));
            pass.set_index_buffer(
                self.hud_ibuf.slice(..(indices.len() * 4) as u64),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..(indices.len() as u32), 0, 0..1);
        }

        self.last_stats = stats;
        self.queue.submit([encoder.finish()]);
    }

    /// 第一人称手持 pass：右臂盒体（player 管线右臂切片 + 独立 uniform 槽）、
    /// 手持方块缩小立方体（terrain 管线直通）或物品图标 quad（hand 管线）。
    ///
    /// 在 draw_frame 的世界 pass 与 HUD pass 之间被调用，自起清深度的
    /// 独立 render pass（26.1 GameRenderer.java:724-729）。
    fn draw_hand(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        cam: &Camera,
        hand: crate::hand::HandRender,
    ) {
        let swing = hand.swing.clamp(0.0, 1.0);
        // (a) 手臂：独立 uniform 槽（偏移 1024，256B 对齐）写挥臂矩阵——
        //     手持 pass 与世界 pass 同一次 submit，必须与世界玩家本体（槽 0）
        //     争写隔离；槽位 4 = 右臂切片（顶点 meta.y=4）。
        let mut models = [glam::Mat4::IDENTITY; PART_COUNT];
        models[crate::player_mesh::P_R_ARM] = crate::hand::hand_arm_matrix(cam, swing);
        if self.skins_loaded {
            let mut mu = PlayerUniforms {
                view_proj: cam.view_proj().to_cols_array_2d(),
                models: [[[0f32; 4]; 4]; PART_COUNT],
            };
            for (m, dst) in models.iter().zip(mu.models.iter_mut()) {
                *dst = m.to_cols_array_2d();
            }
            self.queue.write_buffer(
                &self.player_uniform,
                PLAYER_HAND_SLOT_OFF,
                bytemuck::bytes_of(&mu),
            );
        }

        // (b) 手持方块立方体顶点（世界空间，origin 槽置零直通 terrain）。
        let mut cube: Option<(Vec<CrackVertex>, Vec<u32>)> = None;
        if let HandItem::Block(bid) = hand.item
            && let Some(def) = mcv_core::BLOCKS.get(bid as usize)
        {
            let rot = crate::hand::hand_block_rotation(cam, swing);
            let center = crate::hand::hand_block_center(swing);
            let mut verts = Vec::with_capacity(24);
            let mut idx = Vec::with_capacity(36);
            let (r, u, b) = crate::hand::camera_basis(cam);
            let basis = glam::Mat4::from_cols(
                r.extend(0.0),
                u.extend(0.0),
                b.extend(0.0),
                glam::Vec3::ZERO.extend(1.0),
            );
            let eye = cam.pos + glam::Vec3::new(0.0, crate::EYE_HEIGHT, 0.0);
            for (f, face) in FACE_QUAD.iter().enumerate() {
                let base = verts.len() as u32;
                for (c, p) in face.iter().enumerate() {
                    // 单位立方体角 → 立方局部（−0.5..0.5）×缩放 → 旋转 →
                    // 视空间中心 → 世界。
                    let local =
                        (glam::Vec3::from(*p) - glam::Vec3::splat(0.5)) * crate::hand::BLOCK_SCALE;
                    let view = (rot * local.extend(1.0)).truncate() + center;
                    let world = eye + basis.transform_point3(view);
                    verts.push(CrackVertex {
                        pos: world.to_array(),
                        uv: FACE_UV[c],
                        layer: def.tiles[f],
                        block_light: 15,
                        sky_light: 15,
                        ao: 3,
                        flags: f as u8,
                        pad: [0; 2],
                    });
                }
                idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            cube = Some((verts, idx));
        }

        // (c) 物品图标 quad（GUI 精灵表；素材缺失降级只画手臂）。
        let mut icon: Option<[PlayerVertex; 4]> = None;
        if let HandItem::Sprite(name) = hand.item
            && let Some(sheet) = self.gui.as_ref()
            && let Some((uv0, uv1)) = sheet.sprite_uv(name)
        {
            let (pts, mut uvs) = crate::hand::hand_icon_quad(cam, swing);
            let map = |uv: [f32; 2], (uv0, uv1): ([f32; 2], [f32; 2])| {
                if (uv[0], uv[1]) == (0.0, 0.0) {
                    uv0
                } else if (uv[0], uv[1]) == (1.0, 0.0) {
                    [uv1[0], uv0[1]]
                } else if (uv[0], uv[1]) == (1.0, 1.0) {
                    uv1
                } else {
                    [uv0[0], uv1[1]]
                }
            };
            for uv in uvs.iter_mut() {
                *uv = map(*uv, (uv0, uv1));
            }
            // hand.rs 产出的是**视空间**角点，与手持方块同路：经相机正交基
            // + 眼位转世界坐标（vs_hand_icon 的 uniform 只带世界 view_proj）。
            let (r, u, b) = crate::hand::camera_basis(cam);
            let basis = glam::Mat4::from_cols(
                r.extend(0.0),
                u.extend(0.0),
                b.extend(0.0),
                glam::Vec3::ZERO.extend(1.0),
            );
            let eye = cam.pos + glam::Vec3::new(0.0, crate::EYE_HEIGHT, 0.0);
            icon = Some(std::array::from_fn(|k| PlayerVertex {
                pos: (eye + basis.transform_point3(pts[k])).to_array(),
                uv: [(uvs[k][0] * 255.0) as u8, (uvs[k][1] * 255.0) as u8],
                _pad: [0; 2],
                meta: [0u32, 0u32],
            }));
        }

        if cube.is_none() && icon.is_none() && !self.skins_loaded {
            return;
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hand"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // 臂（皮肤切片 0=steve；挥臂矩阵已写 player_uniform 手持槽）。
            // 26.1 renderArmWithItem 只在 **itemStack.isEmpty()** 时接线
            // renderPlayerArm（:449）——手持方块/物品时臂不画，否则自定臂盒
            // 会把更远的持物条带深度裁掉（CI 实测 sprite 图标仅余 17 px）。
            if self.skins_loaded && matches!(hand.item, HandItem::Empty) {
                let slice = self.player_index_slices[0][crate::player_mesh::P_R_ARM].clone();
                pass.set_pipeline(&self.player_pipeline);
                pass.set_bind_group(0, &self.player_bind, &[PLAYER_HAND_SLOT_OFF as u32]);
                pass.set_vertex_buffer(0, self.player_vbuf.slice(..));
                pass.set_index_buffer(self.player_ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(slice, 0, 0..1);
            }

            // 手持方块（专用 origin 槽置零 + terrain 管线；不透明语义写深度）。
            if let Some((verts, idx)) = cube {
                self.queue
                    .write_buffer(&self.hand_terrain_vbuf, 0, bytemuck::cast_slice(&verts));
                self.queue
                    .write_buffer(&self.hand_terrain_ibuf, 0, bytemuck::cast_slice(&idx));
                // 专用 origin 槽（overlay 槽归世界 pass 的裂纹/描边，同一次
                // submit 内争写互踩）。
                let off = (self.max_chunks + 1) * 256;
                self.queue.write_buffer(
                    &self.origins_buf,
                    off as wgpu::BufferAddress,
                    bytemuck::cast_slice(&[0.0f32; 4]),
                );
                pass.set_pipeline(&self.terrain_pipeline);
                pass.set_bind_group(0, &self.frame_bind, &[off]);
                pass.set_vertex_buffer(0, self.hand_terrain_vbuf.slice(..));
                pass.set_index_buffer(self.hand_terrain_ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..36, 0, 0..1);
            }

            // 物品图标（不写深度；uniform 每帧喂世界 view_proj；quad 索引
            // 单独写——sprite 路径没有方块 36 索引可复用）。
            if let Some(verts) = icon {
                self.queue
                    .write_buffer(&self.hand_terrain_vbuf, 0, bytemuck::bytes_of(&verts));
                self.queue.write_buffer(
                    &self.hand_uniform,
                    0,
                    bytemuck::bytes_of(&cam.view_proj().to_cols_array_2d()),
                );
                self.queue.write_buffer(
                    &self.hand_terrain_ibuf,
                    0,
                    bytemuck::bytes_of(&QUAD_INDICES),
                );
                pass.set_pipeline(&self.hand_icon_pipeline);
                pass.set_bind_group(0, &self.hand_bind, &[]);
                pass.set_vertex_buffer(
                    0,
                    self.hand_terrain_vbuf.slice(..(4 * PLAYER_STRIDE) as u64),
                );
                pass.set_index_buffer(
                    self.hand_terrain_ibuf
                        .slice(..QUAD_INDICES.len() as u64 * 4),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..QUAD_INDICES.len() as u32, 0, 0..1);
            }
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// 最近一帧的结构性统计（#80）：opaque/water draw 数与可见条目数。
    pub fn last_frame_stats(&self) -> FrameStats {
        self.last_stats
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HudVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
    src: [u32; 2],
}

fn flatten_hud(quads: &[HudQuad]) -> (Vec<HudVertex>, Vec<u32>) {
    let mut verts = Vec::with_capacity(quads.len() * 4);
    let mut indices = Vec::with_capacity(quads.len() * 6);
    for q in quads {
        let base = verts.len() as u32;
        let c = [
            (q.color[0] * 255.0) as u8,
            (q.color[1] * 255.0) as u8,
            (q.color[2] * 255.0) as u8,
            (q.color[3] * 255.0) as u8,
        ];
        // rot != 0 时绕 quad 中心旋转(splash 文字);常规 quad 走直线分支
        let (cos, sin) = if q.rot == 0.0 {
            (1.0, 0.0)
        } else {
            (q.rot.cos(), q.rot.sin())
        };
        let hw = q.w * 0.5;
        let hh = q.h * 0.5;
        for (ox, oy, uv) in [
            (-hw, -hh, q.uv[0]),
            (hw, -hh, [q.uv[1][0], q.uv[0][1]]),
            (hw, hh, q.uv[1]),
            (-hw, hh, [q.uv[0][0], q.uv[1][1]]),
        ] {
            verts.push(HudVertex {
                pos: [
                    q.x + hw + ox * cos - oy * sin,
                    q.y + hh + ox * sin + oy * cos,
                ],
                uv,
                color: c,
                src: [q.tex, q.layer],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, indices)
}
