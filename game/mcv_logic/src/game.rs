//! Game runtime: chunk streaming, input state, player physics, HUD.
//!
//! App-side glue between winit events, the world (terrain scheduler +
//! chunk map), and the renderer. Mesh upload lands when the C++ mesher
//! merges (M4); a no-op mesher keeps this compiling until then.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkHandle, ChunkPos, ChunkStatus, Stage};
use mcv_entity::combat;
use mcv_entity::defs::speed_m_s;
use mcv_entity::spawner;
use mcv_entity::{
    AiAction, Health, ItemDrop, LastHurt, MobArrow, MobBrain, MobId, MobIntent, MobKind, MobPath,
    MobTicks, PhysBody, PickupReq, Yaw, spawn_mob,
};
use mcv_game::{Player, VoxelAccess, step_entity};
use mcv_platform::touch::TouchState;
use mcv_render::gpu::RenderChunk;
use mcv_render::{Camera, HudQuad, text};

use crate::ticket::{self, TicketTable, TicketType};

pub const RENDER_DIST: i32 = 8;

/// 世界竖直下界（f32 视图；唯一定义源是 [`mcv_core::WORLD_MIN_Y`]，
/// `Level.getMinY()` 语义，Entity.checkBelowWorld 的参照常数——v6：−64，
/// 本处仅 re-export 给物理/虚空伤害等 f32 域消费）。
pub const WORLD_MIN_Y: f32 = mcv_core::WORLD_MIN_Y as f32;

/// 游戏模式（存档 meta.mode 字段值对应）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameMode {
    Survival = 0,
    Creative = 1,
    Hardcore = 2,
}

impl GameMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Creative,
            2 => Self::Hardcore,
            _ => Self::Survival,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Survival => "生存",
            Self::Creative => "创造",
            Self::Hardcore => "极限",
        }
    }
}

/// 网格侧结构性统计（#80 可观测性）：draw 收敛 + 异步上传带宽，
/// app 层 ~1 Hz 打一行日志（不进 HUD 像素）。
#[derive(Clone, Copy, Debug, Default)]
pub struct MeshStats {
    /// 已上传网格的区块数。
    pub meshed_chunks: usize,
    /// 绘制条目数（批次 + 不可合并单块）。
    pub draw_entries: usize,
    /// 批次重建次数。
    pub batch_rebuilds: u64,
    /// 重建累计字节数。
    pub rebuild_bytes: u64,
    /// 经 staging 环提交的拷贝笔数。
    pub staged_uploads: u64,
    /// 经 staging 环提交的字节数。
    pub staged_bytes: u64,
    /// 一次性兜底笔数（超槽容量/环耗尽）。
    pub fallback_uploads: u64,
    /// GPU 侧驻留网格字节总量（#115 刀 5a；含批次共享缓冲的分配对齐）。
    pub resident_bytes: u64,
    /// CPU 侧留存网格字节（#80 无 LRU 池，计入内存预算面）。
    pub cpu_pool_bytes: u64,
}

/// Abstraction over the C++ mesher so the runtime wiring can land before
/// the mesher itself merges.
pub trait ChunkMesher: Send {
    /// `handles` = 3x3 neighbourhood, row-major (dz outer), center = [4].
    /// 返回引擎侧组装好的 [`RenderChunk`]（游戏层不触 wgpu，上传经
    /// [`mcv_render::gpu::MeshUploader`]）。#80 起网格字节由实现入池并
    /// 参与相邻区块合批，绘制条目表由实现自持（[`Self::entries`]）。
    fn build(&mut self, pos: ChunkPos, handles: &[Arc<ChunkHandle>; 9]) -> Option<RenderChunk>;

    /// 区块卸载：释放该块网格与批次成员资格（#80 合批记账，默认 no-op）。
    fn unload(&mut self, _pos: ChunkPos) {}

    /// 当前绘制条目表（#80 合批后 = 批次条目；无头实现恒空表）。
    fn entries(&self) -> &[RenderChunk] {
        &[]
    }

    /// 每帧冲刷异步上传（#80 第二刀）：staging 拷贝在此 submit，先于同帧
    /// draw（stream 末尾调用，draw_frame 之前）。默认 no-op。
    fn flush(&mut self) {}

    /// 合批 + 异步上传统计（默认零值）。
    fn mesh_stats(&self) -> MeshStats {
        MeshStats::default()
    }
}

/// E3 快照核：3×3 邻域拷出为裸字节（voxels u16 + light u8，布局
/// `mcv_core::vidx`，行主序 dz 外层）。网格路径上唯一碰区块 RwLock 的点
/// ——必须在主线程同步段内串行完成（拷贝不可被编辑打断 → 无快照撕裂），
/// worker 只见字节。同步路径（[`RustMesher::build`]）与 worker 派发
/// （[`GameRuntime::build_chunk_mesh`]）共用同一装配，保证两路快照与
/// `mcv_mesher::neighborhood_slots` 的输入逐字节同源。
fn snapshot_neighborhood(handles: &[Arc<ChunkHandle>; 9]) -> (Vec<u16>, Vec<u8>) {
    let vol = mcv_core::CHUNK_VOL;
    let mut voxels = vec![0u16; 9 * vol];
    let mut lights = vec![0xF0; 9 * vol]; // sky=15 until light wires in
    for (i, h) in handles.iter().enumerate() {
        voxels[i * vol..(i + 1) * vol]
            .copy_from_slice(bytemuck::cast_slice(h.voxels.read().unwrap().as_slice()));
        lights[i * vol..(i + 1) * vol].copy_from_slice(&h.light.read().unwrap()[..]);
    }
    (voxels, lights)
}

/// Real mesher: pure-Rust greedy mesh via mcv_mesher + GPU upload.
/// E3 起双路分叉（[`ChunkMesher`] trait 与各实现签名零改动）：
/// - **worker 路**（`results: Some`）：[`Self::build`] 从不被调用，派发走
///   `GameRuntime::mesh_sched`（快照在 `build_chunk_mesh`），回传在
///   [`Self::flush`] 消化——每帧 stream 末尾必达，无结果饿死；
/// - **同步路**（`results: None`，`MCV_MESHER_WORKERS=0` 回滚开关）：与
///   E3 前逐字节同款——`build` 内快照+计算+上传一体。
pub struct RustMesher {
    mesher: mcv_mesher::Mesher,
    uploader: mcv_render::gpu::MeshUploader,
    /// E3 回传端（None = 同步路径，不建池）。
    results: Option<mcv_mesher::MeshResults>,
}

impl RustMesher {
    /// E3 装配：`workers > 0` → 并行（返回调度器提交端给
    /// [`GameRuntime`]）；`workers == 0`（`MCV_MESHER_WORKERS=0` 回滚
    /// 开关）→ 同步路径，返回 None。
    pub fn new_with_workers(
        uploader: mcv_render::gpu::MeshUploader,
        workers: usize,
    ) -> (Self, Option<mcv_mesher::MeshScheduler>) {
        if workers == 0 {
            return (
                Self {
                    mesher: mcv_mesher::Mesher::new(),
                    uploader,
                    results: None,
                },
                None,
            );
        }
        let (sched, results) = mcv_mesher::MeshScheduler::new(workers);
        (
            Self {
                mesher: mcv_mesher::Mesher::new(),
                uploader,
                results: Some(results),
            },
            Some(sched),
        )
    }
}

impl ChunkMesher for RustMesher {
    fn build(&mut self, pos: ChunkPos, handles: &[Arc<ChunkHandle>; 9]) -> Option<RenderChunk> {
        // 同步路径（回滚开关）：快照核 + 直呼 mesher（E3 前同款时序）。
        let (voxels, lights) = snapshot_neighborhood(handles);
        let slots = mcv_mesher::neighborhood_slots(&voxels, &lights);
        let opaque = self.mesher.build(&slots, mcv_mesher::MESH_OPAQUE).ok()?;
        let water = self.mesher.build(&slots, mcv_mesher::MESH_WATER).ok();
        // 裸字节交给引擎上传；水顶点+索引都传（build_chunk 拼接共用 vb 并
        // 偏移索引——只传索引会让水索引绑到 opaque 顶点上，GLES 静默跳 draw）。
        let origin = [16.0 * pos.x as f32, 0.0, 16.0 * pos.z as f32];
        Some(self.uploader.build_chunk(
            origin,
            opaque.vertex_data(),
            opaque.indices(),
            water.as_ref().map(|w| (w.vertex_data(), w.indices())),
        ))
    }

    fn unload(&mut self, pos: ChunkPos) {
        // E3：卸载记账同步回收在途产物（僵尸锁）——提交地板抬到当前
        // 派发序，其后回传的该块产物在 flush 处拒收，防网格条目复活到
        // 已卸载键（tests/mesh_parallel.rs 僵尸锁）。
        if let Some(results) = self.results.as_mut() {
            results.kill(pos);
        }
        self.uploader.unload(pos);
    }

    fn entries(&self) -> &[RenderChunk] {
        self.uploader.entries()
    }

    fn flush(&mut self) {
        // E3 回传段：worker 产物在此消化——主线程 uploader.build_chunk 进
        // staging（提交地板过滤僵尸/陈旧产物），随后 uploader.flush 把本帧
        // 全部 staging 拷贝 submit（先于同帧 draw 提交，draw 必见数据）。
        // 每帧 stream 末尾必达，无结果饿死；消化成本落 E0 flush 子段窗。
        if let Some(results) = self.results.as_mut() {
            for done in results.drain_committed() {
                // 产物缺失 = worker 侧 build 报错（kind 为编译期常量，
                // BAD_ARG 不可达；若真发生与同步路失败同级别响亮记账），
                // 不炸主线程：记 error 跳过该笔（该块网格缺失，MESH 位
                // 已在派发时清除——下一笔编辑/重网格恢复）。
                let Some(opaque) = done.opaque else {
                    log::error!(
                        "mesh worker产物缺失 {:?}（worker 侧报错，跳过提交）",
                        done.pos
                    );
                    continue;
                };
                let rc = self.uploader.build_chunk(
                    done.origin,
                    &opaque.vertex,
                    &opaque.index,
                    done.water.as_ref().map(|w| (&w.vertex[..], &w.index[..])),
                );
                // 真机取证埋点：worker 回传提交带区块坐标——与派发日志
                // 对账可观测回传延迟（帧数差）。
                log::debug!(
                    "mesh worker commit {:?} idx={}",
                    done.pos,
                    rc.opaque_range.end
                );
            }
        }
        self.uploader.flush();
    }

    fn mesh_stats(&self) -> MeshStats {
        let b = self.uploader.batching_stats();
        let u = self.uploader.upload_stats();
        MeshStats {
            meshed_chunks: b.meshed_chunks,
            draw_entries: b.draw_entries,
            batch_rebuilds: b.batch_rebuilds,
            rebuild_bytes: b.rebuild_bytes,
            staged_uploads: u.staged_uploads,
            staged_bytes: u.staged_bytes,
            fallback_uploads: u.fallback_uploads,
            resident_bytes: b.resident_bytes,
            cpu_pool_bytes: b.cpu_pool_bytes,
        }
    }
}

/// Voxel view over the loaded chunk map.
pub struct WorldView<'a> {
    pub chunks: &'a HashMap<ChunkPos, Arc<ChunkHandle>>,
}

impl VoxelAccess for WorldView<'_> {
    /// 未加载 / 体素未就位（Empty）→ **空气**，与 26.1 一致：越界列
    /// `Level.getBlockState` 返回 VOID_AIR（Level.java:361-363），从无
    /// 「未加载=实心石」代理。旧石安全垫是真机「隐形墙」（撞上看不见
    /// 的实心边界）与「看得见却穿透」错位（石面高度≠地形）的共同根因。
    /// 玩家物理安全不再靠假方块，而靠模拟区不变量：`fixed_step` 步进
    /// 前 [`GameRuntime::sim_safe_radius`] 门、步末 [`GameRuntime::clamp_to_sim_area`]
    /// 钳回已就位区——玩家 AABB 查询的列恒 ≥TerrainReady（恒真体素）。
    /// 区外查询（mob 远景射线、缺块粒子等）得空气=原版该区不 tick 的
    /// 等价近似（ServerLevel.java:419），不再是假石头。
    fn block(&self, p: BlockPos) -> BlockId {
        // y 出界 = 虚空空气（coords 审计 P2：界外索引会绕回同列另一端 →
        // 显示≠真实的幽灵方块/隐形地板；原版越界一律 VOID_AIR，
        // Level.java:361-363）。v6：界门换绝对 [WORLD_MIN_Y, WORLD_MAX_Y)。
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&p.y) {
            return BlockId(0);
        }
        let Some(chunk) = self.chunks.get(&p.chunk()) else {
            return BlockId(0); // unloaded = void air, as 26.1 Level.java:361-363
        };
        if chunk.stage() == Stage::Empty {
            return BlockId(0); // terrain not committed yet — same treatment
        }
        let [lx, _, lz] = p.local();
        chunk.voxels.read().unwrap()[mcv_core::vidx(lx, p.y, lz)]
    }

    /// 真实光照（原「恒 15」M4 死桩已清）：返回 sky/block 较亮者的
    /// 0..15 亮度，与 [`Self::chunk_loaded`] 无关——缺块给满亮天空，
    /// 对齐 Particle.java:184-187 hasChunkAt=false → 0xF 天光语义。
    fn light(&self, p: BlockPos) -> u8 {
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&p.y) {
            return 15; // 虚空：满亮（同下缺块分支）
        }
        let c = p.chunk();
        match self.chunks.get(&c) {
            Some(chunk) if chunk.stage() != Stage::Empty => {
                let [lx, _, lz] = p.local();
                let v = chunk.light.read().unwrap()[mcv_core::vidx(lx, p.y, lz)];
                (v & 0xF).max(v >> 4)
            }
            _ => 15,
        }
    }

    fn chunk_loaded(&self, c: ChunkPos) -> bool {
        self.chunks.contains_key(&c)
    }
}

/// 粒子世界适配器（[`mcv_render::particles::ParticleWorld`]）：只借
/// chunks，让 fixed_step 能在 `&mut self.particles` 的同时喂世界回调。
/// 缺区块语义：碰撞=空气（不碰撞，原版无 chunk 粒子不做碰撞、按
/// removeIfNoChunk 消亡）、光=满亮（Particle.java:184-187
/// hasChunkAt=false → 0xF000F0）。
struct ParticleRt<'a> {
    chunks: &'a HashMap<ChunkPos, Arc<ChunkHandle>>,
}

impl mcv_render::particles::ParticleWorld for ParticleRt<'_> {
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        let view = WorldView {
            chunks: self.chunks,
        };
        view.block(BlockPos::new(x, y, z)).def().solid
    }

    fn light_at(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let p = BlockPos::new(x, y, z);
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&y) {
            return (15, 15); // 虚空满亮（y 出界不绕回，coords P2）
        }
        match self.chunks.get(&p.chunk()) {
            Some(c) if c.stage() != Stage::Empty => {
                let [lx, _, lz] = p.local();
                let v = c.light.read().unwrap()[mcv_core::vidx(lx, y, lz)];
                (v & 0xF, v >> 4)
            }
            _ => (15, 15),
        }
    }

    fn is_water(&self, x: i32, y: i32, z: i32) -> bool {
        let view = WorldView {
            chunks: self.chunks,
        };
        view.block(BlockPos::new(x, y, z)).def().liquid
    }

    /// 水体顶面高度（WaterDrop「没入流体面即灭」判据，WaterDropParticle.
    /// java:48-55）：本仓无半流体（流动水按满格存）→ 满高 1.0。
    fn fluid_top(&self, x: i32, y: i32, z: i32) -> f64 {
        let view = WorldView {
            chunks: self.chunks,
        };
        let def = view.block(BlockPos::new(x, y, z)).def();
        if def.liquid && def.name == "water" {
            1.0
        } else {
            0.0
        }
    }
}

/// 边号 → 相邻区块方向：0=+X 1=-X 2=+Z 3=-Z（与 mcv_light 的边编码一致）。
fn side_delta(side: u8) -> (i32, i32) {
    match side {
        0 => (1, 0),
        1 => (-1, 0),
        2 => (0, 1),
        _ => (0, -1),
    }
}

/// C1 光照接线：place/destroy 共用的方块编辑入口。26.1 参照
/// `Level.setBlock` → `LevelLightEngine.checkBlock` → `LightEngine.
/// runLightUpdates`（先 decrease 后 increase 两阶段，LightEngine.java:
/// 147-148）；调用方必须已经写好体素并标好 MESH/SAVE 脏。步骤：
/// 1. 重算 heightmap——用 worldgen 唯一生产者 [`mcv_worldgen::
///    recompute_heightmap`]（对生成期方块与 C++ terrain pass 3 逐列一致，
///    并推广到玩家放置的透明方块；光照不读 heightmap，见
///    `mcv_light::init` 文档——它是出生/刷怪用的地表高）。
/// 2. 区块光照已初始化（≥LightLocalReady）时调 [`mcv_light::update_block`]
///    做增量重光照；未初始化时直接返回，主循环稍后的 `init` 会全量覆盖。
/// 3. 光发生变化的边界交给 [`sync_light_edges`] 跨区块派发，光变块的
///    MESH 脏在派发路径内标好（网格顶点烘焙光照字节）。
///
/// `queue` = 跨区块边派发队列（调用方持有：游戏路径传
/// [`GameRuntime::pending_light_edges`] 跨帧消化，测试路径就地
/// [`sync_light_edges`] 排空）。本函数只做块内增量重光照与双向任务
/// 入队，**不再同步跑边同步**——旧实现整链跑在 fixed_step 的编辑调用
/// 栈内，贴边一次编辑最坏 512 步 × 每步双 BFS 冻结主线程数百 ms
/// （审计 A1/A3；原版 light check 排队按 tick 预算执行、未完必重排，
/// LevelLightEngine.runLightUpdates）。
fn relight_block_edit(
    chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>,
    target: BlockPos,
    old_id: u16,
    new_id: u16,
    queue: &mut Vec<(ChunkPos, u8)>,
) {
    let cpos = target.chunk();
    let Some(handle) = chunks.get(&cpos) else {
        return;
    };
    {
        let ids: Vec<u16> = bytemuck::cast_slice(handle.voxels.read().unwrap().as_slice()).to_vec();
        *handle.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&ids);
    }
    if (handle.stage() as u8) < (Stage::LightLocalReady as u8) {
        return;
    }
    let [lx, _, lz] = target.local();
    let mut seeds = Vec::new();
    let mask = {
        let vg = handle.voxels.read().unwrap();
        let mut lg = handle.light.write().unwrap();
        let hg = handle.heightmap.read().unwrap();
        let voxels: &[u16] = bytemuck::cast_slice(vg.as_slice());
        let mut view = mcv_light::LightChunk {
            voxels,
            light: &mut lg[..],
            heightmap: &hg[..],
        };
        // v6：update_block 的 y 为绝对世界 y（x/z 为区块内局部）。
        mcv_light::update_block(
            &mut view, lx as u32, target.y, lz as u32, old_id, new_id, &mut seeds,
        )
    };
    // BorderSeed 是格级差分记录；跨区块协议以整条边快照
    // （extract_edge/apply_edge）为单位，脏面掩码已足够驱动派发。
    let _ = seeds;
    // 双向派发：mask 只驱动「推」（本块边值 → 邻块）。挖开贴边遮光壁时
    // 本侧字节可能不变（暗格 0→0），邻区光进不来（update_block 文档 :552-
    // 556 明言拉方向由调用方补）——故对每条脏边再排一条反向同步，把邻块
    // 现值回喂本块。队列是 LIFO：每边先排 pull 后排 push，pop 序为
    // push→pull，保证 pull 读到邻块被推之后的现值。同键去重由
    // sync_light_edges 的 in-flight 集合兜底（pull 与 push 的 REMOVE 回报
    // 常生成同一任务，审计 A1）。
    for side in 0..4u8 {
        if mask & (1 << side) != 0 {
            let (dx, dz) = side_delta(side);
            let npos = ChunkPos::new(cpos.x + dx, cpos.z + dz);
            let pull = (npos, GameRuntime::opposite_side(side));
            let push = (cpos, side);
            if !queue.contains(&pull) {
                queue.push(pull);
            }
            if !queue.contains(&push) {
                queue.push(push);
            }
        }
    }
}

/// 跨区块光照边派发队列（C1：原 border_synced 只做记账，光从不真正过界）。
/// 每步取源区块 `side` 的整条边快照，先 REMOVE（撤回邻块不再被该边证成的
/// 光）再 ADD（吸收该边新增的光）——等价 vanilla 跨 section 的
/// decrease→increase 两阶段（LightEngine.java:147-148，由 setBlock 的
/// checkNode 驱动）。接收块光变 → 标 MESH 脏 + 其另三面新脏回队级联。
/// REMOVE 时 `apply_edge` 若撤改了本侧边值会回报刚同步的边（对
/// propagateDecrease 幸存格按 stored 现值重播种的跨块等价，
/// BlockLightEngine.java:103-105），级联把本侧现值回喂编辑块；ADD 不回报。
/// 回报仅在边字节严格变化时触发，光级别有限，级联单调收敛；步数上限只是
/// 防御性兜底——**预算耗尽不丢任务**：剩余留在 `queue` 里由调用方
/// （`GameRuntime::pending_light_edges`）跨帧继续消化（审计 A3；原版
/// LevelLightEngine 未完必重排、从不丢弃）。
///
/// 同键去重（审计 A1）：relight 排的 pull 与 push 的 REMOVE 回报常生成
/// 同一 `(ChunkPos, side)` 任务；in-flight 集合让每条边每轮至多处理一次
/// （对齐原版区块任务按距离合并去重，ChunkHolder/LevelLightEngine）。
fn sync_light_edges(chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>, queue: &mut Vec<(ChunkPos, u8)>) {
    let mut inflight: HashSet<(ChunkPos, u8)> = queue.iter().copied().collect();
    let mut steps = 0usize;
    while let Some((pos, side)) = queue.pop() {
        inflight.remove(&(pos, side));
        if steps >= 512 {
            // 当前出队任务也放回队列头部一并留待下帧（旧实现连它一起丢）。
            queue.push((pos, side));
            inflight.insert((pos, side));
            log::warn!(
                "light edge sync budget exhausted, {} edge task(s) deferred to next frame",
                queue.len()
            );
            break;
        }
        steps += 1;
        let (dx, dz) = side_delta(side);
        let npos = ChunkPos::new(pos.x + dx, pos.z + dz);
        // 真机取证埋点：每条边同步任务（REMOVE+ADD 两相一体）。静止期持续
        // 刷屏即光照边同步乒乓/级联不收敛实证（512 步兜底会伴随 warn）。
        log::debug!("light edge {pos:?} side{side} -> {npos:?}");
        let (Some(from), Some(to)) = (chunks.get(&pos), chunks.get(&npos)) else {
            continue;
        };
        // 129-L/L4（129-C §1 漏洞 A）：from 侧未完成本地布光（读盘块
        // TerrainReady game.rs:2316 / 生成块 commit 后等 init 席位
        // :2082-2106，光数组全零）时，extract_edge 抽出全零边，REMOVE 相
        // 按 `s > n+1` 把已亮块边界格清零成黑带（待邻块 init 才自愈）。
        // 原版跨 section decrease 波携带编辑前波前等级，从不拿
        // 「未初始化=0」当波前（BlockLightEngine.java:91-105）。与 to 侧
        // 门同语义跳过：两端就绪后由 border_synced 成对标记 + 成对同步
        //（:2121-2157）双向补收。
        if (from.stage() as u8) < (Stage::LightLocalReady as u8) {
            continue;
        }
        if (to.stage() as u8) < (Stage::LightLocalReady as u8) {
            // 未初始化邻块：其 init 会本地播种，就绪时由 border_synced 的
            // 成对同步补收这条边。
            continue;
        }
        let obit = GameRuntime::opposite_side(side);
        let edge = {
            let vg = from.voxels.read().unwrap();
            let mut lg = from.light.write().unwrap();
            let hg = from.heightmap.read().unwrap();
            let voxels: &[u16] = bytemuck::cast_slice(vg.as_slice());
            let view = mcv_light::LightChunk {
                voxels,
                light: &mut lg[..],
                heightmap: &hg[..],
            };
            mcv_light::extract_edge(&view, side)
        };
        // REMOVE(1) 先、ADD(0) 后：边变暗时撤无据光、变亮时喂新光；
        // 混合变化两轮都收敛到"以该边为界的正当亮度"。
        for op in [1u8, 0u8] {
            let dirty = {
                let vg = to.voxels.read().unwrap();
                let mut lg = to.light.write().unwrap();
                let hg = to.heightmap.read().unwrap();
                let voxels: &[u16] = bytemuck::cast_slice(vg.as_slice());
                let mut view = mcv_light::LightChunk {
                    voxels,
                    light: &mut lg[..],
                    heightmap: &hg[..],
                };
                mcv_light::apply_edge(&mut view, &edge, obit, op)
            };
            if dirty != 0 {
                to.mark_dirty(mcv_core::dirty::MESH);
                for bit in 0..4u8 {
                    if dirty & (1 << bit) != 0 && inflight.insert((npos, bit)) {
                        queue.push((npos, bit));
                    }
                }
            }
        }
    }
}

/// 游戏阶段（26.1 `LevelLoadTracker.ClientState` 三态收束版）。
///
/// 本仓单进程一体（无客户端/服务端之分），`WaitingForServer` 与
/// `WaitingForPlayerChunk` 合并为 [`GamePhase::Loading`]；`ClientLevelReady`
/// 对应 [`GameRuntime::load_ready_at`] 记账 + 关屏延迟
/// （`isLevelReady`，LevelLoadTracker.java:66-68）。
///
/// 26.1 进入世界即 `setScreen(LevelLoadingScreen)`（Minecraft.java
/// `doWorldLoad` :2080-2081），出生点区块未就绪前画面停在本阶段：
/// 文本 + 进度条 + 区块状态网格（LevelLoadingScreen.java:91-120），期间
/// 玩家输入被挡（screen 非 null 时 `KeyboardInput` 不驱动移动）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GamePhase {
    /// 进入世界加载：世界生成/光照/网格照常流式跑，玩家无输入。
    #[default]
    Loading,
    /// 出生点邻域就绪，正常游玩。
    Playing,
}

/// 本固定步聚合后的输入意图（键盘 + 触屏摇杆/按钮合并，见
/// `apply_touch_input` 与 app 层键盘接线）。
pub struct InputState {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
    pub mining: bool,
    pub placing: bool,
    /// 移动模拟量（0..=1）：键盘恒 1.0；触屏摇杆为偏移幅度（死区重标定，
    /// `TouchState::stick_analog`）。原版 getInputVector 只在输入模长 >1 时
    /// 归一化（Entity.java:1677），亚单位输入自然产生亚单位速度。
    /// Default 为 1.0（手写）：derive 会给 0，让一切移动静止。
    pub analog: f32,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            forward: false,
            back: false,
            left: false,
            right: false,
            jump: false,
            sneak: false,
            sprint: false,
            mining: false,
            placing: false,
            analog: 1.0,
        }
    }
}

/// E0 帧预算分段计时（engine-roadmap.md §3-E0，观测先行）：一帧内各预算
/// 段的 wall-clock 耗时（微秒）。段序 = 帧内执行序（蓝图 §2 的预算模型）：
/// fixed_step（内含 ECS 阶段单列）→ stream（init/edge/io/remesh 子段）→
/// flush → draw。纯计时记账：`Instant` 排尺 + u64 累加，不改任何执行
/// 顺序、调度或预算数值。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameTiming {
    /// fixed_step 总窗口（一帧内全部固定步累计；含下方 ecs 子段）。
    pub fixed_step_us: u64,
    /// 其中 ECS Fixed 阶段单列：两次整表快照 insert（`MobServices`/
    /// `DropWorld` 的 chunks 克隆）+ `schedule.run_stage(Fixed)`。
    pub ecs_us: u64,
    /// stream() 总窗口。
    pub stream_us: u64,
    /// stream 子段：光照 init（≤2 块/帧，主线程双通道 BFS）。
    pub init_us: u64,
    /// stream 子段：光照边同步（帧头消化段 + 成对标记后同步，两窗合并）。
    pub edge_us: u64,
    /// stream 子段：region IO（卸载落盘/积压门/脏块写回/请求环读盘，
    /// 多窗合并）。
    pub io_us: u64,
    /// stream 子段：remesh（编辑源块优先通道 + 近优先预算流）。
    pub remesh_us: u64,
    /// MeshUploader staging flush（stream 末尾 submit；独立列）。
    pub flush_us: u64,
    /// draw 段：HUD/Scene 装配 + draw_frame 提交（app 层归账）。
    pub draw_us: u64,
}

impl FrameTiming {
    /// 段名表（render-stats 日志字段名，序 = 帧内执行序；E0 验收面）。
    pub const SEGMENTS: [&'static str; 9] = [
        "fixed_step",
        "ecs",
        "stream",
        "init",
        "edge",
        "io",
        "remesh",
        "flush",
        "draw",
    ];

    /// 逐段饱和累加（跨帧累计：单调不减、不回绕）。
    pub fn saturating_add(self, rhs: Self) -> Self {
        Self {
            fixed_step_us: self.fixed_step_us.saturating_add(rhs.fixed_step_us),
            ecs_us: self.ecs_us.saturating_add(rhs.ecs_us),
            stream_us: self.stream_us.saturating_add(rhs.stream_us),
            init_us: self.init_us.saturating_add(rhs.init_us),
            edge_us: self.edge_us.saturating_add(rhs.edge_us),
            io_us: self.io_us.saturating_add(rhs.io_us),
            remesh_us: self.remesh_us.saturating_add(rhs.remesh_us),
            flush_us: self.flush_us.saturating_add(rhs.flush_us),
            draw_us: self.draw_us.saturating_add(rhs.draw_us),
        }
    }

    /// 逐段取较大值（1Hz 汇总的单帧峰值归并）。
    fn max_zip(self, rhs: Self) -> Self {
        Self {
            fixed_step_us: self.fixed_step_us.max(rhs.fixed_step_us),
            ecs_us: self.ecs_us.max(rhs.ecs_us),
            stream_us: self.stream_us.max(rhs.stream_us),
            init_us: self.init_us.max(rhs.init_us),
            edge_us: self.edge_us.max(rhs.edge_us),
            io_us: self.io_us.max(rhs.io_us),
            remesh_us: self.remesh_us.max(rhs.remesh_us),
            flush_us: self.flush_us.max(rhs.flush_us),
            draw_us: self.draw_us.max(rhs.draw_us),
        }
    }
}

/// E0：1Hz 汇总累计器——自上次取走以来逐帧分段值的和（求均值）与单帧
/// 峰值。app 层每渲染帧末 [`GameRuntime::end_frame_timing`] 归并一帧；
/// 1Hz 日志端 [`GameRuntime::take_frame_timing_stats`] 取走并清零。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameTimingStats {
    /// 已归并的渲染帧数。
    pub frames: u64,
    /// 帧分段值之和（均值 = sum / frames）。
    pub sum: FrameTiming,
    /// 单帧峰值（逐段 max）。
    pub max: FrameTiming,
}

impl FrameTimingStats {
    /// 归并一帧的分段值（帧界记账）。
    pub fn record(&mut self, frame: FrameTiming) {
        self.frames = self.frames.saturating_add(1);
        self.sum = self.sum.saturating_add(frame);
        self.max = self.max.max_zip(frame);
    }

    /// 逐段均值（微秒，整数除法；frames=0 时全 0）。
    pub fn mean_us(&self) -> FrameTiming {
        if self.frames == 0 {
            return FrameTiming::default();
        }
        let n = self.frames;
        FrameTiming {
            fixed_step_us: self.sum.fixed_step_us / n,
            ecs_us: self.sum.ecs_us / n,
            stream_us: self.sum.stream_us / n,
            init_us: self.sum.init_us / n,
            edge_us: self.sum.edge_us / n,
            io_us: self.sum.io_us / n,
            remesh_us: self.sum.remesh_us / n,
            flush_us: self.sum.flush_us / n,
            draw_us: self.sum.draw_us / n,
        }
    }
}

pub struct GameRuntime {
    pub seed: u64,
    pub chunks: HashMap<ChunkPos, Arc<ChunkHandle>>,
    pub scheduler: mcv_worldgen::TerrainScheduler,
    pub player: Player,
    pub input: InputState,
    pub time_ticks: u64,
    /// 60 Hz 固定步 → 20 Hz 原版 tick 的小数累加器（原版每 tick +1，
    /// 帧率耦合的 `(dt*20) as u64` 截断在 60 fps 下恒为 0，时间冻结）。
    tick_frac: f64,
    /// 全局 20 Hz tick 计数（与 time_ticks 分开：time_ticks 可被睡觉/指令
    /// 跳转，game_ticks 单调只增，做系统节流用）。
    pub game_ticks: u64,
    /// 本固定步是否跨过至少一个 tick 边界——按原版 tick 计时的计时器
    /// （无敌帧、攻击冷却、AI 节奏）必须在此为真时 +1，不许按 60 Hz 步计。
    pub on_tick: bool,
    pub mesher: Box<dyn ChunkMesher>,
    /// E3 网格计算调度器提交端（None = 同步路径，`MCV_MESHER_WORKERS=0`
    /// 回滚开关）。回传端在 RustMesher 内（flush 消化）；派发端在此供
    /// remesh 预算流投递快照字节（hi/lo 通道选择在 [`Self::build_chunk_mesh`]）。
    mesh_sched: Option<mcv_mesher::MeshScheduler>,
    /// E4 独立 IO 通道（None = 同步路，`MCV_IO_ASYNC=0` 回滚开关/无头）。
    /// 提交与回执消化都在主线程；region open/read/write 全部移出主线程。
    io: Option<IoChannel>,
    /// 在途读盘记账：pos → (投递帧, seq)。复用「Empty 在途 300 帧驻留」
    /// 防乒乓机制（与 req_frames 并列的 or 条件），读回填走
    /// advance_to(TerrainReady) 既有接线。
    io_frames: HashMap<ChunkPos, (u64, u64)>,
    /// 在途写记账：pos → 最新快照 seq（同块新快照接管后，旧回执按 seq 丢弃）。
    io_writes: HashMap<ChunkPos, u64>,
    /// io seq 单调源（投递与完成回执的配对键）。
    io_seq: u64,
    pub save_dir: std::path::PathBuf,
    /// 已建网格区块的记账（#80 起绘制条目表由 mesher 实现自持，游戏层只
    /// 记「哪块已建网格」）。O(1) 判「这块是否已有网格」替代逐帧对全表做
    /// 线性扫（旧实现每帧 O(loaded×meshed) 次浮点比对），并消除按 origin
    /// 浮点匹配的精度隐患。
    meshed: HashSet<ChunkPos>,
    spawned: bool,
    border_synced: HashMap<ChunkPos, u8>,
    /// stream() 帧计数（在途请求驻留计时基准；stream 是帧驱动非 tick 驱动）。
    stream_frame: u64,
    /// 已发出 worker 请求、尚未收到结果的区块 → 发出时的 `stream_frame`。
    /// 在途区块不卸载（原版 PLAYER_LOADING ticket 释放要等 entity-ticking
    /// future 完成，DistanceManager.java:87-104）；超时 300 帧（≈5s）视为
    /// 失败可弃。
    req_frames: HashMap<ChunkPos, u64>,
    /// 票表（刀 3a：原版 TicketStorage + DistanceManager 裁剪镜像，见
    /// [`crate::ticket`]）：玩家加载/模拟票存储与级别派生。卸载候选与
    /// status 推进授权均由级别驱动（原版卸载迟滞 = 票级别语义，
    /// ChunkMap.java:376-381）。
    pub(crate) tickets: TicketTable,
    /// 卸载复活缓存（原版 ChunkMap.pendingUnloads，ChunkMap.java:388-392）：
    /// 越环区块先从活动 `chunks` 摘除（GPU 网格条目同步释放），数据留在
    /// 此处；重进请求环时原位复活、不重 IO 不重生成；每帧限量落盘，
    /// 超容量（64）从最旧端强制落盘后丢弃。
    pending_unloads: Vec<(ChunkPos, Arc<ChunkHandle>)>,
    /// 跨区块光照边派发持久队列（审计 A1/A3/F2）：编辑路径只入队，
    /// stream() 开头按步预算跨帧消化，未完不丢弃（原版 LevelLightEngine
    /// 重排语义）。fixed_step 编辑栈内不再跑边同步。
    pending_light_edges: Vec<(ChunkPos, u8)>,
    /// 编辑源区块当帧优先重建网格（审计 A2/F3）：place/destroy 置位，
    /// remesh 循环最先消费（预算内第一位），消除「已挖开仍画实心」的
    /// 玩家正前方陈旧窗口；级联标脏的邻块仍走近优先预算流。
    mesh_priority: Option<ChunkPos>,
    /// mesher.build 连续失败计数（审计 F6 退避）：首败 log::error（真机
    /// 取证坐标），后续同块重复失败降为 debug 不再刷屏；成功即清账。
    /// 无头 NullMesher 恒失败 = 预期路径，只报一次。
    mesh_fail: HashMap<ChunkPos, u32>,
    /// 运行时 mob 集合（ECS App：World + 调度 + 事件总线；组件见
    /// mcv_entity::components，装配走 spawn_mob，行为走 `mob_ai` 系统）。
    pub mobs_app: mcv_ecs::App,
    pub attack_ticker: f32,
    /// FoodData.tickTimer（26.1 FoodData.java:17）：回血快线 10 tick、慢线与
    /// 饥饿掉血 80 tick 的共享节拍（每 on_tick 走一格）。
    food_tick_timer: u32,
    /// 挖掘状态机（26.1 MultiPlayerGameMode 的 destroyBlockPos/destroyProgress/
    /// destroyDelay 三件套）：生存/极限走 START→CONTINUE→ABORT，创造走按住
    /// 连秒破冷却；仅 `on_tick` 为真的固定步推进（原版每 tick 一次 continue）。
    mine: MineMachine,
    /// 挥臂动画（26.1 LivingEntity.swinging/swingTime/attackAnim，:212,2149-2158）：
    /// 攻击按下沿与挖掘中的每个 tick 触发（restart 半程规则 :1995），渲染层
    /// 经 `swing_progress` 取 0..1 驱动第一人称手臂/手持物摆动。
    swinging: bool,
    /// 挥臂计时（tick 制，0..=SWING_TICKS；连续制推进按 dt*20 折算帧间平滑）。
    swing_time: f32,
    /// 行走 bob 状态（26.1 ClientAvatarState walkDist/walkDistO/bob/bobO，
    /// ClientAvatarState.java:76-92）：bobView 的相位源与幅度源，每 on_tick
    /// 推进一次、渲染帧经 [`GameRuntime::hand_bob`] 插值。
    walk_dist: f32,
    walk_dist_o: f32,
    bob: f32,
    bob_o: f32,
    /// 上一 tick 边界的脚底位置（水平位移计量用，LocalPlayer.java:989 的
    /// `Mth.length(deltaX, deltaZ)`）。
    bob_prev_pos: Vec3,
    spawn_cooldown: u32,
    pub player_xp: u32,
    /// 9 格快捷栏(vanilla Inventory 子集):放置消耗选中槽 Block 物品、
    /// 生存破坏掉落入栏、攻击武器 = 选中槽(方块按拳头)。
    pub hotbar: mcv_item::Hotbar,
    pub touch: TouchState,
    pub mode: GameMode,
    /// 难度（26.1 Difficulty.java:8-13；伤害缩放见
    /// [`difficulty::scale_entity_damage`]，和平清怪/不索敌在 AI 系统门）。
    pub difficulty: crate::difficulty::Difficulty,
    /// 天气状态机（ServerLevel.advanceWeatherCycle；[`weather::Weather`]）。
    pub weather: crate::weather::Weather,
    /// 玩家活动状态效果账本（26.1 LivingEntity.activeEffects；机制层
    /// [`mcv_entity::effect::EffectBook`]，每 on_tick 推进一次。施加方
    /// （药水/信标）待上游域，机制与行为对拍已就位）。
    pub effects: mcv_entity::EffectBook,
    /// M8a 粒子池（挖掘碎屑/破坏爆裂/溅水/暴击）：fixed_step 里 on_tick
    /// 推进，app.rs 每帧经 Scene.particles 交 mcv_render 绘制。
    pub particles: mcv_render::particles::ParticleEngine,
    /// 弓蓄力账本（Some=按住蓄力中的 tick 数；状态机 [`mcv_item::bow`]）。
    bow_hold: Option<u32>,
    /// 进食账本（Some=按住进食中；状态机 [`mcv_item::food::step_eating`]，
    /// 启动沿在 interact 的食物分支、松开/换手即取消）。
    eat_hold: Option<mcv_item::food::Eating>,
    /// 熔炉屏会话（Some=(会话炉方块位, FurnaceScreen)；interact 命中
    /// furnace 开/关，fixed_step on_tick 驱动烧炼，见
    /// [`GameRuntime::toggle_furnace_screen`]）。单会话：本仓无方块实体
    /// 存储，炉内容不可持久（关屏即归还，守恒纪律审计 #52）。
    pub furnace: Option<(BlockPos, crate::ui::FurnaceScreen)>,
    /// 右键重触发延迟（tick，原版 `Minecraft.rightClickDelay = 4`；完食后
    /// 按住右键由此节流连吃，空手/非食物不受影响）。
    eat_cooldown: u32,
    /// 极限模式死亡后置位：app 层负责删档并回主菜单。
    pub hardcore_death: bool,
    /// 渲染距离（区块），设置界面可调。
    pub render_dist: i32,
    /// 鼠标/触摸灵敏度倍率。
    pub sens: f32,
    /// 视角：F5 循环 第一→第三后→第三前（MC 26.1 GameRenderer 顺序）。
    pub cam_type: CameraType,
    /// 行为音效播放器（静音回退由 app 装配，见 set_audio）。
    pub audio: mcv_audio::AudioManager,
    /// 脚步触发：自上次音效以来水平移动距离（格）。
    step_dist: f32,
    /// 步距门阈值（26.1 `Entity.nextStep`，Entity.java:246 初值 1.0；触发后
    /// 置 `(int)moveDist + 1`，:1188-1190）。
    next_step: f32,
    /// 死亡中（health 归零）：app 层画死亡界面，respawn() 复活。
    pub dead: bool,
    /// 离地时的 y（落地按 26.1 规则算摔落伤害：floor(高度−3)）。
    fall_y: Option<f32>,
    /// 上一步 wasTouchingWater（入水沿检测：溅落音 + 摔落豁免，
    /// Entity.java:1570-1574）。
    was_in_water: bool,
    /// 当前阶段：进入世界先 [`GamePhase::Loading`]，出生点邻域就绪后转
    /// [`GamePhase::Playing`]（26.1 `LevelLoadTracker` 状态机）。
    pub phase: GamePhase,
    /// 就绪后的关屏延迟，单位 tick（26.1 `LevelLoadTracker` 构造参数
    /// `closeDelayMs`，新世界 500ms、其余 0，Minecraft.java `doWorldLoad`
    /// :2083；500ms / 50ms-per-tick = 10 tick）。
    load_close_delay_ticks: u32,
    /// 出生点邻域首次全部就绪的 game_ticks（26.1 `ClientLevelReady(readyAt)`
    /// ，LevelLoadTracker.java:107）。
    load_ready_at: Option<u64>,
    /// 加载等待截止 tick（26.1 `CLIENT_WAIT_TIMEOUT_MS` = 30s =
    /// 600 tick，LevelLoadTracker.java:26；超时放玩家进场，:152-156）。
    load_deadline_tick: u64,
    /// 显示用平滑进度：每 tick 向目标值 lerp 0.2
    /// （LevelLoadingScreen.java:84 `smoothedProgress += (target−cur)×0.2`）。
    smoothed_progress: f32,
    /// 上一固定步 jump 键状态（双击切飞行的按下沿检测）。
    jump_held_prev: bool,
    /// 双击切飞行的窗口余量，**tick** 单位（26.1 LocalPlayer.jumpTriggerTime，
    /// LocalPlayer.java:835-836 窗口 7 tick = 350ms；Player.aiStep:443-445
    /// 每 tick −1）。首按沿置 7，窗口内再按沿 → 切换 abilities.flying。
    flight_jump_trigger: f32,
    /// 空气供给（Entity.java:2739-2741 getMaxAirSupply = 300；消耗/恢复/
    /// 溺水伤害结算见 [`air_supply_tick`]，LivingEntity.java:417-439）。
    air_supply: i32,
    /// 水下视野计时 waterVisionTime，tick 0..=600（LocalPlayer.java:155 字段、
    /// :858-864 更新：眼在水中 +1/tick（旁观者 +10，本仓无旁观者）、出水
    /// −10/tick）。getWaterVision（:1150-1163）换算成 0..1 视野系数：
    /// 驱动水下雾 end = 96×max(0.25, wv)（WaterFogEnvironment.java:19-21）
    /// 与雾色归一提亮（FogRenderer.java:148-166）。瞬态客户端态，不入存档。
    water_vision_time: u32,
    /// 游泳姿态（Entity shared flag bit4，Entity.java:2657-2659/:2669-2671；状态机
    /// [`swimming_tick`]，Entity.java:1558-1564 + Player.java:1410-1416）。
    /// 原版 Pose.SWIMMING 由此驱动（Player.java:342-361）；第三人称
    /// prone 模型接线在 mcv_app（遗留清单，见报告）。
    swimming: bool,
    /// 玩家剩余燃烧 tick（26.1 Entity.remainingFireTicks，Entity.java:534-544
    /// 服务端分支：>0 时每 20 tick 1 点 on_fire 伤害并每 tick −1；点燃入口
    /// lavaIgnite/fireIgnite，:607-640）。创造不死但同样挂燃烧账
    /// （hurt_player 创造豁免在伤害侧，燃烧账照记——与原版一致）。
    pub fire_ticks: i32,
    /// E0 帧预算分段计时：本帧各段累计（微秒）。fixed_step/stream 内部在
    /// 段边界累积；draw 段由 app 层归账（app.rs 唯一渲染帧的装配点）；
    /// 帧界由 [`Self::end_frame_timing`] 归并。纯记账，不参与任何逻辑。
    pub timing_frame: FrameTiming,
    /// E0：1Hz 汇总累计器（均值/峰值），日志端经
    /// [`Self::take_frame_timing_stats`] 取走。
    timing_stats: FrameTimingStats,
}

/// 视角模式（26.1 CameraType 子集；固定第一人称俯仰不变）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CameraType {
    #[default]
    FirstPerson,
    ThirdPersonBack,
    ThirdPersonFront,
}

/// 第三人称摄像机距离上限（MC options.cameraDistance 默认 norm）。
const THIRD_PERSON_DIST: f32 = 4.0;

/// 物品图标三级解析（#150 同源函数）：Block 物品 → 方块图集（渲染端取
/// `BLOCKS[].tiles[2]` 侧面 tile），材料物品 → GUI 精灵表图标名，空堆 →
/// Empty。三处消费（HUD [`GameRuntime::build_hud`] 槽内容、第一人称手持
/// [`GameRuntime::hand_item`]、掉落物 billboard [`GameRuntime::item_drops`]
/// ）共用本函数——图标名/方块 id 的解析只此一份，uv 逻辑不漂移
/// （26.1 Gui.renderSlot 与 ItemRenderer 同走 itemModelResolver 的语义位）。
pub fn item_icon(stack: &mcv_item::ItemStack) -> mcv_render::gpu::HandItem {
    if stack.is_empty() {
        return mcv_render::gpu::HandItem::Empty;
    }
    match stack.def().kind {
        mcv_item::ItemKind::Block(bid) => mcv_render::gpu::HandItem::Block(bid.id()),
        _ => mcv_render::gpu::HandItem::Sprite(stack.def().name),
    }
}

/// 方块交互距离（26.1 `Attributes.BLOCK_INTERACTION_RANGE` 基值 4.5，
/// `Attributes.java:22-23`；创造 +0.5 加法修饰 = 5.0，`ServerPlayer.java:215-216`
/// `CREATIVE_BLOCK_INTERACTION_RANGE_MODIFIER` ADD_VALUE）。挖掘/放置/选中
/// 射线一律传本函数，替换此前散落的硬编码 5.0。服务端 START/STOP 另有
/// 1.0 容差（`ServerPlayerGameMode.java:153`）且按住期间不查距离——那是
/// 联网防作弊复核，单机一体无客户端上报语义，按住期间按原版客户端行为
/// 每 tick 以基值重射线（打不中即 ABORT）。
pub fn block_interaction_reach(mode: GameMode) -> f32 {
    if mode == GameMode::Creative {
        mcv_game::raycast::REACH + 0.5
    } else {
        mcv_game::raycast::REACH
    }
}

/// 生存挖掘单 tick 继续的输入：准星射线命中的非空气方块 + **该 tick 现算**
/// 的每 tick 进度速率。原版每 tick 重算 `getDestroyProgress`
/// （`ServerPlayerGameMode.tick()` :107-130 → `Player.getDestroySpeed`
/// Player.java:586-614），空中/入水当 tick 即变速，不许起手缓存速率。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MineHit {
    pub pos: BlockPos,
    pub per_tick: f32,
}

/// [`MineMachine::continue_tick`] 的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MineTick {
    Idle,
    /// 本 tick 挖穿（调用方负责写空气 + 掉落 + 音效）。
    Broken(BlockPos),
}

/// 挖掘状态机（26.1 客户端 `MultiPlayerGameMode` 的 destroyBlockPos/
/// destroyProgress/destroyDelay 三件套，:79,308-311）。纯逻辑、无世界访问：
/// 射线与 per-tick 速率由 `GameRuntime::step_mining` 每 tick 现算喂入，
/// 且只由 `GameRuntime::on_tick` 门驱动——原版按住期间是**每 tick** 一次
/// `continueDestroyBlock`（`Minecraft.continueAttack`，Minecraft.java:
/// 1606-1628），60 Hz 固定步按 dt 连加会让进度偏 3×，故按 tick 离散推进。
#[derive(Clone, Copy, Debug, Default)]
pub struct MineMachine {
    /// START 目标（原版 destroyBlockPos）；None = 未在挖。
    pub pos: Option<BlockPos>,
    /// 最近一 tick 使用的速率（overlay/调试展示；判定不依赖缓存值）。
    pub per_tick: f32,
    /// 累积进度（原版 destroyProgress；≥1 破坏）。
    pub progress: f32,
    /// 破坏后冷却（原版 destroyDelay，5 tick；仅正常挖穿与创造连破置位）。
    pub delay: u32,
    /// 续挖 tick 计数（原版 destroyTicks，MultiPlayerGameMode.java:78）：START
    /// 清零（:196）、同目标续挖每 tick 递增（:268）；挖掘敲击音按
    /// `destroyTicks % 4 == 0` 判定（:251-263），判定在递增前。
    pub destroy_ticks: u32,
}

impl MineMachine {
    /// START_DESTROY_BLOCK（生存分支，`MultiPlayerGameMode.java:147-205`）：
    /// 换目标即隐式 ABORT 旧 + 进度清零重算。返回 Some(pos) = insta-mine
    /// （per ≥ 1 起手即破，`ServerPlayerGameMode.java:210-212`）；秒破**不**
    /// 置冷却——原版 destroyDelay=5 只在正常挖穿（:282）与创造（:167/:232）
    /// 置位，起手秒破（花类）不置。
    pub fn start(&mut self, pos: BlockPos, per_tick: f32) -> Option<BlockPos> {
        self.pos = Some(pos);
        self.per_tick = per_tick;
        // START 清零敲击计数（MultiPlayerGameMode.java:196 destroyTicks = 0.0F）。
        self.destroy_ticks = 0;
        // START 当 tick 计入 1 份进度：服务端进度公式为 per×(ticksSpent+1)
        // （ServerPlayerGameMode.incrementDestroyProgress :132-142）。
        self.progress = per_tick;
        if per_tick >= 1.0 {
            self.abort();
            return Some(pos);
        }
        None
    }

    /// ABORT（26.1 stopDestroyBlock → 发 ABORT_DESTROY_BLOCK，
    /// `MultiPlayerGameMode.java:207-222`；服务端 ABORT 分支只清状态、不
    /// 破坏，`ServerPlayerGameMode.java:239-249`）：进度作废。delay 不清
    /// （原版同款——冷却是节奏计数，留给后续 CONTINUE 自行衰减）。
    pub fn abort(&mut self) {
        self.pos = None;
        self.per_tick = 0.0;
        self.progress = 0.0;
    }

    /// CONTINUE_DESTROY_BLOCK 生存分支，每 tick 一次
    /// （`MultiPlayerGameMode.java:224-286`）。`hit` 为 None = 准星射线打不
    /// 中非空方块（移出 reach/移开/目标被破坏变空）——原版此时走
    /// stopDestroyBlock = ABORT（Minecraft.java:1624-1626；或 continue 内
    /// 目标变空气 isDestroying=false，:244-248）。
    pub fn continue_tick(&mut self, hit: Option<MineHit>) -> MineTick {
        if self.delay > 0 {
            // destroyDelay 先减且本 tick 不推进（:226-228）：正常挖穿后置 5，
            // 第 6 个 tick 才对新目标 START —— "5 tick 冷却再开下一块"。
            self.delay -= 1;
            return MineTick::Idle;
        }
        let Some(h) = hit else {
            self.abort();
            return MineTick::Idle;
        };
        if self.pos != Some(h.pos) {
            // 换目标（含冷却减尽后对新目标的自动重启——continue 落到
            // startDestroyBlock，:285-286）：ABORT 旧进度 + START 新目标。
            self.abort();
            if let Some(p) = self.start(h.pos, h.per_tick) {
                return MineTick::Broken(p);
            }
            return MineTick::Idle;
        }
        // 同目标续挖：速率用本 tick 现算值（空中/水下随条件实时变化）。
        self.per_tick = h.per_tick;
        self.progress += h.per_tick;
        if self.progress >= 1.0 {
            // 原版阈值是 `>= 1.0F`（:274），旧实现的 `> 1.0` 会漏掉恰好
            // 1.0 的情形，按源码收紧。挖穿清敲击计数（:281 destroyTicks = 0.0F）。
            let pos = h.pos;
            self.abort();
            self.delay = 5;
            self.destroy_ticks = 0;
            return MineTick::Broken(pos);
        }
        // 同目标续挖每 tick 递增敲击计数（MultiPlayerGameMode.java:268
        // destroyTicks++；敲击音判定在调用点用递增前值，同源 :251-263）。
        self.destroy_ticks += 1;
        MineTick::Idle
    }

    /// 创造按住每 tick（`MultiPlayerGameMode.java:230-242`）：冷却减尽后
    /// 秒破准星方块并再置 destroyDelay=5。返回 Some(pos) = 本 tick 破坏。
    /// 按下瞬间的首破由调用方（on_left_press）负责并置初始 delay（:157-167）。
    pub fn creative_tick(&mut self, hit: Option<BlockPos>) -> Option<BlockPos> {
        if self.delay > 0 {
            self.delay -= 1;
            return None;
        }
        let pos = hit?;
        self.delay = 5;
        Some(pos)
    }
}

/// 挥臂动画一轮时长（tick，26.1 `SwingAnimation.DEFAULT.duration`，
/// SwingAnimation.java:12 —— 默认 WHACK 6 tick）。
const SWING_TICKS: f32 = 6.0;
/// 挥臂重启半程阈值（26.1 LivingEntity.swing :1995 `swingTime >= duration/2`
/// 时允许重置——挖掘长按每 tick 都 swing，实际挥臂周期 = 3 tick）。
const SWING_RESTART_TICKS: f32 = SWING_TICKS * 0.5;

impl GameRuntime {
    /// 挥臂触发（26.1 LivingEntity.swing :1994-1996 的 restart 半程规则）：
    /// 攻击按下沿与挖掘中的每 tick 都可调用，半程前重触发无效（不加速）。
    fn swing(&mut self) {
        if !self.swinging || self.swing_time >= SWING_RESTART_TICKS {
            self.swinging = true;
            self.swing_time = 0.0;
        }
    }
}

/// 60 Hz 固定步 → 20 Hz 原版 tick 累加：返回本步跨过的 tick 数（0 或 1
/// 为常态），小数留在 `frac`。`dt ≥ 0.2 s` 的 burst（卡顿/后台回归）封顶
/// 4 tick，防级联。原版逻辑全部按 tick 计时（20 tick/s），任何按 60 Hz
/// 步数或按秒累加的 tick 计时器都慢/快 3~20 倍。
fn accumulate_ticks(frac: &mut f64, dt: f32) -> u64 {
    *frac += (dt * 20.0) as f64;
    let n = *frac as u64;
    if n > 0 {
        *frac -= n as f64;
    }
    n.min(4)
}

/// 无 GPU 空网格器（无头测试/CI 用）：不产出渲染网格。
struct NullMesher;
impl ChunkMesher for NullMesher {
    fn build(&mut self, _pos: ChunkPos, _handles: &[Arc<ChunkHandle>; 9]) -> Option<RenderChunk> {
        None
    }
}

// ---- E4 独立单 IO 线程（存/读盘异步化，docs/engine-roadmap.md §3 E4）----
//
// 原版同构：IO 走独立 executor 不入节流器——存盘编码
// `CompletableFuture.supplyAsync(data::write, Util.backgroundExecutor())`
// （ChunkMap.java:773），解析同（:566、:924）；写失败报告
// reportChunkSaveFailure（:775-777）。本仓对应物 = 专属 std::thread（非
// TaskPool——IO 与生成争 worker 会互饿），mpsc 命令通道（命令低频）。
// 帧预算（读 4/写 2+积压/脏 2）保留为**投递预算**，不再是执行预算；
// E0 的 io 子段计时自动覆盖新路径（量的是快照+投递成本）。

/// 主线程 → IO 线程命令。
enum IoCommand {
    /// 两段式写：主线程已快照的 (voxels u16, heightmap) 投递写盘。
    /// 载荷 Box 化（~197 KiB 变体尺寸差，clippy::large_enum_variant）。
    Write {
        pos: ChunkPos,
        seq: u64,
        ids: Box<[u16]>,
        hm: Box<[i16; 256]>,
    },
    /// 读盘请求（结果经完成通道回主线程走 advance_to(TerrainReady) 接线）。
    Load { pos: ChunkPos, seq: u64 },
}

/// IO 线程 → 主线程完成回执。
enum IoDone {
    /// 写完成：`ok=false` = 写失败（回执端保脏位重试；报告路径对应原版
    /// reportChunkSaveFailure，ChunkMap.java:775-777）。
    Written { pos: ChunkPos, seq: u64, ok: bool },
    /// 读完成：`None` = 未存档/读失败（与同步 try_load_saved 同语义 →
    /// 回退生成器）。载荷 Box 化（同上）。
    Loaded {
        pos: ChunkPos,
        seq: u64,
        found: Option<Box<(Vec<u16>, [i16; 256])>>,
    },
}

/// 在途投递高水位（`save_dirty(None)` 全量扫描/常规预算路的停投线）：
/// 64 × ~197 KiB 快照 ≈ 12.6 MiB 上界（对齐复活缓存 64 硬顶量级）。
const IO_HIGH_WATER: usize = 64;

/// 单区块 region 写盘（IO 线程侧）。错误文案与同步路 save_handle_io
/// 两处 log::error 逐字同源（region open / chunk save）。
fn write_chunk_to_disk(
    dir: &std::path::Path,
    pos: ChunkPos,
    ids: &[u16],
    hm: &[i16; 256],
) -> std::io::Result<()> {
    let (rx, rz) = mcv_save::chunk_region(pos.x, pos.z);
    let local = mcv_save::chunk_local(pos.x, pos.z);
    let mut region = match mcv_save::RegionFile::open(dir, rx, rz) {
        Ok(r) => r,
        Err(e) => {
            log::error!("region open failed {rx},{rz}: {e}");
            return Err(e);
        }
    };
    if let Err(e) = region.save_chunk(local, ids, hm) {
        log::error!("chunk save failed {pos:?}: {e}");
        return Err(e);
    }
    Ok(())
}

/// 单区块 region 读盘（IO 线程侧）。与同步 try_load_saved 同语义同静默：
/// region 文件不存在/打开失败/记录缺失或版本不符（v1/v2）→ None → 回退
/// 生成器。exists() 前置避免 open 在 miss 上创建空 region 文件。
fn load_chunk_from_disk(dir: &std::path::Path, pos: ChunkPos) -> Option<(Vec<u16>, [i16; 256])> {
    let (rx, rz) = mcv_save::chunk_region(pos.x, pos.z);
    let path = dir.join(format!("region/r.{rx}.{rz}.mcrv"));
    if !path.exists() {
        return None;
    }
    let mut region = mcv_save::RegionFile::open(dir, rx, rz).ok()?;
    let mut ids = vec![0u16; mcv_core::CHUNK_VOL];
    let mut hm = [mcv_core::WORLD_MIN_Y as i16; 256];
    region
        .load_chunk(mcv_save::chunk_local(pos.x, pos.z), &mut ids, &mut hm)
        .ok()?;
    Some((ids, hm))
}

/// 单命令执行（IO 线程侧；panic 由 [`spawn_io_worker`] 隔离成失败回执）。
fn run_io_command(dir: &std::path::Path, cmd: &IoCommand) -> IoDone {
    match cmd {
        IoCommand::Write { pos, seq, ids, hm } => IoDone::Written {
            pos: *pos,
            seq: *seq,
            ok: write_chunk_to_disk(dir, *pos, ids, hm).is_ok(),
        },
        IoCommand::Load { pos, seq } => IoDone::Loaded {
            pos: *pos,
            seq: *seq,
            found: load_chunk_from_disk(dir, *pos).map(Box::new),
        },
    }
}

/// 失败回执合成（命令 panic 时；保脏位/回退生成的语义不因线程侧异常失效）。
fn failed_io_done(cmd: &IoCommand) -> IoDone {
    match cmd {
        IoCommand::Write { pos, seq, .. } => IoDone::Written {
            pos: *pos,
            seq: *seq,
            ok: false,
        },
        IoCommand::Load { pos, seq } => IoDone::Loaded {
            pos: *pos,
            seq: *seq,
            found: None,
        },
    }
}

/// IO 线程主循环：逐命令执行并回执；任务 panic 隔离（镜像 TaskPool
/// run_isolated——worker 存活，命令以失败回执收场）；主线程撤发送端后
/// 通道清空断开，循环自然退出。
fn spawn_io_worker(
    save_dir: std::path::PathBuf,
    rx: std::sync::mpsc::Receiver<IoCommand>,
    tx: std::sync::mpsc::Sender<IoDone>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("mcv-io".into())
        .spawn(move || {
            while let Ok(cmd) = rx.recv() {
                let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_io_command(&save_dir, &cmd)
                }))
                .unwrap_or_else(|_| {
                    log::error!("[io] command panicked; worker survives, failure reported");
                    failed_io_done(&cmd)
                });
                if tx.send(done).is_err() {
                    break; // 主线程已弃收（退出），不再产回执
                }
            }
        })
        .expect("spawn mcv-io thread")
}

/// E4 IO 通道：主线程持有（提交 + 回执消化都在主线程）。`outstanding`
/// 主线程侧计数即可（投递 +1、消化 -1），无需与 worker 共享。
struct IoChannel {
    tx: Option<std::sync::mpsc::Sender<IoCommand>>,
    rx: std::sync::mpsc::Receiver<IoDone>,
    outstanding: usize,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl IoChannel {
    fn new(save_dir: std::path::PathBuf) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = spawn_io_worker(save_dir, rx, done_tx);
        Self {
            tx: Some(tx),
            rx: done_rx,
            outstanding: 0,
            worker: Some(worker),
        }
    }

    fn post(&mut self, cmd: IoCommand) {
        if self.tx.as_ref().is_some_and(|tx| tx.send(cmd).is_ok()) {
            self.outstanding += 1;
        }
    }

    fn try_recv(&mut self) -> Option<IoDone> {
        match self.rx.try_recv() {
            Ok(done) => {
                self.outstanding = self.outstanding.saturating_sub(1);
                Some(done)
            }
            Err(_) => None,
        }
    }

    fn recv_timeout(&mut self, t: std::time::Duration) -> Option<IoDone> {
        match self.rx.recv_timeout(t) {
            Ok(done) => {
                self.outstanding = self.outstanding.saturating_sub(1);
                Some(done)
            }
            Err(_) => None,
        }
    }

    fn outstanding(&self) -> usize {
        self.outstanding
    }
}

impl Drop for IoChannel {
    fn drop(&mut self) {
        // 先撤发送端（worker 收完通道余量后退出），再 join—— Drop 不 join
        // 会detach：退出期在途写盘命令执行完才收线程，保证尽量多落盘。
        self.tx.take();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl GameRuntime {
    pub fn new(
        seed: u64,
        uploader: mcv_render::gpu::MeshUploader,
        save_dir: std::path::PathBuf,
        mode: GameMode,
    ) -> Self {
        // E3：worker 数在构造时点解析一次（主线程；env 不入后台线程）。
        // MCV_MESHER_WORKERS=0 → 同步路径（回滚开关，对齐 MCV_TERRAIN_BACKEND
        // 先例），>0 → 快照-计算-回传三段式并行。
        let (mesher, mesh_sched) =
            RustMesher::new_with_workers(uploader, mcv_mesher::mesh_worker_count());
        // E4：IO 异步开关同点解析一次（MCV_IO_ASYNC=0 → 同步路回滚）。
        let io_async = Self::io_async_enabled();
        Self::assemble(seed, Box::new(mesher), save_dir, mode, mesh_sched, io_async)
    }

    /// 无头构造（集成测试/CI）：NullMesher 不触 GPU，其余接线与 [`new`](Self::new) 全同。
    pub fn new_headless(seed: u64, save_dir: std::path::PathBuf, mode: GameMode) -> Self {
        Self::assemble(seed, Box::new(NullMesher), save_dir, mode, None, false)
    }

    /// E3 测试装配（headless + 真实网格调度器）：NullMesher 不触 GPU、
    /// 不接回传端，派发产物由测试持有的 [`mcv_mesher::MeshResults`] 取用
    /// （派发记账/hi 路由/快照字节的游戏层断言面，tests/mesh_parallel.rs）。
    /// 生产装配走 [`new`](Self::new)（回传端在 RustMesher 内 flush 消化）。
    #[doc(hidden)]
    pub fn new_headless_with_mesh(
        seed: u64,
        save_dir: std::path::PathBuf,
        mode: GameMode,
        workers: usize,
    ) -> (Self, mcv_mesher::MeshResults) {
        assert!(workers > 0, "workers=0 是同步路径，无调度器可返回");
        let (sched, results) = mcv_mesher::MeshScheduler::new(workers);
        (
            Self::assemble(
                seed,
                Box::new(NullMesher),
                save_dir,
                mode,
                Some(sched),
                false,
            ),
            results,
        )
    }

    /// E4 测试装配（headless + 显式 IO 异步开关）：`io_async = true` 走独立
    /// IO 线程（两段式落盘 + 读盘投递），`false` = 同步路（`MCV_IO_ASYNC=0`
    /// 回滚开关同款语义）。生产装配走 [`new`](Self::new)（env 解析一次）。
    #[doc(hidden)]
    pub fn new_headless_with_io(
        seed: u64,
        save_dir: std::path::PathBuf,
        mode: GameMode,
        io_async: bool,
    ) -> Self {
        Self::assemble(seed, Box::new(NullMesher), save_dir, mode, None, io_async)
    }

    /// `MCV_IO_ASYNC` 语义（E4 回滚开关，对齐 `MCV_MESHER_WORKERS` /
    /// `MCV_TERRAIN_BACKEND` 先例）：未设或非 `"0"` → 异步（独立 IO 线程）；
    /// `"0"` → 同步路（E3 前行为）。纯函数核，env 解析见 [`io_async_enabled`]。
    pub fn io_async_enabled_from(v: Option<&str>) -> bool {
        v != Some("0")
    }

    /// 生产解析：env 在构造时点读一次（主线程；后台线程不反复读 env，
    /// 同 `TerrainScheduler::new` 的后端固定纪律）。
    pub fn io_async_enabled() -> bool {
        Self::io_async_enabled_from(std::env::var("MCV_IO_ASYNC").ok().as_deref())
    }

    fn assemble(
        seed: u64,
        mesher: Box<dyn ChunkMesher>,
        save_dir: std::path::PathBuf,
        mode: GameMode,
        mesh_sched: Option<mcv_mesher::MeshScheduler>,
        io_async: bool,
    ) -> Self {
        // 新档背包全模式为空（26.1 Inventory.java:56 items 初始化为
        // 36×ItemStack.EMPTY，:62-65 构造只挂 player/equipment 引用、从不
        // 发放物品；生存/创造皆然）。旧实现无条件发铁剑（旧单格行为）+
        // 创造另发 8 格调试方块（开发期创造背包未做的替代品，分页 UI 已
        // 实现故理由消失）——均无源码依据且被真机投诉，删除。
        let hotbar = mcv_item::Hotbar::empty();
        let mut rt = Self {
            seed,
            chunks: HashMap::new(),
            scheduler: mcv_worldgen::TerrainScheduler::new(seed, mcv_core::world_worker_count()),
            player: Player::default(),
            input: InputState::default(),
            // 新世界从 tick 0 = 黎明起步（26.1 ClockInstance.totalTicks
            // 默认 0，ServerClockManager.java:149；day.json wake_up_from_sleep
            // 标记 0。旧实现 6000「noon start」在新相位下是正午开局，无源码
            // 依据——旧相位 bug 时代的补偿，相位修正后按原版语义归零）。
            time_ticks: 0,
            tick_frac: 0.0,
            game_ticks: 0,
            on_tick: false,
            mesher,
            mesh_sched,
            io: if io_async {
                Some(IoChannel::new(save_dir.clone()))
            } else {
                None
            },
            io_frames: HashMap::new(),
            io_writes: HashMap::new(),
            io_seq: 0,
            save_dir,
            meshed: HashSet::new(),
            spawned: false,
            border_synced: HashMap::new(),
            stream_frame: 0,
            req_frames: HashMap::new(),
            tickets: TicketTable::new(),
            pending_unloads: Vec::new(),
            pending_light_edges: Vec::new(),
            mesh_priority: None,
            mesh_fail: HashMap::new(),
            mobs_app: mcv_ecs::App::new(),
            // 攻击冷却 ticker，单位 tick（Player.attackStrengthTicker）；
            // 20 tick 起步 = 全武器满蓄力（attackSpeed≥1.0 → delay≤20 tick）。
            attack_ticker: 20.0, // ready
            food_tick_timer: 0,
            mine: MineMachine::default(),
            swinging: false,
            swing_time: 0.0,
            walk_dist: 0.0,
            walk_dist_o: 0.0,
            bob: 0.0,
            bob_o: 0.0,
            bob_prev_pos: Vec3::ZERO,
            spawn_cooldown: 0,
            player_xp: 0,
            hotbar,
            touch: TouchState::default(),
            mode,
            difficulty: crate::difficulty::Difficulty::Normal,
            weather: crate::weather::Weather::new(),
            effects: mcv_entity::EffectBook::default(),
            particles: mcv_render::particles::ParticleEngine::new(),
            bow_hold: None,
            eat_hold: None,
            furnace: None,
            eat_cooldown: 0,
            hardcore_death: false,
            render_dist: RENDER_DIST,
            sens: 1.0,
            cam_type: CameraType::default(),
            audio: mcv_audio::AudioManager::silent(mcv_audio::default_sounds_dir()),
            step_dist: 0.0,
            next_step: 1.0, // Entity.java:246
            dead: false,
            fall_y: None,
            // 构造即加载态（26.1 doWorldLoad 先 setScreen(LevelLoadingScreen)
            // 再起服务器，Minecraft.java:2079-2081）；closeDelay 仅新世界
            // 500ms（:2083 `new LevelLoadTracker(newWorld ? 500L : 0L)`），
            // 由 begin_load 按入口覆写；等待截止 = 30s（600 tick）。
            phase: GamePhase::Loading,
            load_close_delay_ticks: 0,
            load_ready_at: None,
            load_deadline_tick: 600,
            smoothed_progress: 0.0,
            jump_held_prev: false,
            flight_jump_trigger: 0.0,
            air_supply: MAX_AIR_SUPPLY,
            water_vision_time: 0,
            swimming: false,
            was_in_water: false,
            fire_ticks: 0,
            // E0 分段计时（纯观测，构造期全零）。
            timing_frame: FrameTiming::default(),
            timing_stats: FrameTimingStats::default(),
        };
        // 行走 bob 的位移计量基准 = 出生点（首个 tick 只计量构造后的真实位移，
        // 防把 ZERO→出生点的一次性差值灌进 walkDist）。
        rt.bob_prev_pos = rt.player.pos;
        mcv_entity::register_mob_components(&mut rt.mobs_app.world);
        mcv_entity::register_drop_components(&mut rt.mobs_app.world);
        // 被动组件（WanderState/AnimState）也要注册：mob 渲染收集器
        // （mob_render::collect_mob_instances_in）read 全部五类组件，未注册
        // 类型 read 会 panic。游戏内被动怪经 spawn_mob 装配（无这两组件，
        // 视图为空表不 panic）——注册仅为收集器的读面兜底。
        mcv_entity::register_passive_components(&mut rt.mobs_app.world);
        // 启动期注册、注册序即执行序(Godot ClassDB 原则)。
        rt.mobs_app
            .add_system(mcv_ecs::Stage::Fixed, "mob_ai", mob_ai_system);
        rt.mobs_app
            .add_system(mcv_ecs::Stage::Fixed, "mob_arrows", arrow_system);
        // 掉落物：物理/寿命系统 + 拾取/合并系统（同一 World，视图分区，
        // mob_ai 按 MobKind 过滤、掉落系统按 ItemDrop 过滤，互不触碰）。
        // 渲染（#150 通用物品渲染通路）：mcv_render::item_render 的
        // billboard 批已接——GameRuntime::item_drops 每帧只读快照（图标
        // 解析/光照/相位）喂 Scene.drops，物理+拾取语义不变。
        rt.mobs_app.add_system(
            mcv_ecs::Stage::Fixed,
            "item_physics",
            mcv_entity::item_physics_system,
        );
        rt.mobs_app.add_system(
            mcv_ecs::Stage::Fixed,
            "item_merge",
            mcv_entity::item_merge_system,
        );
        rt.mobs_app.add_system(
            mcv_ecs::Stage::Fixed,
            "item_pickup",
            mcv_entity::item_pickup_system,
        );
        rt
    }

    /// 玩家受伤（26.1 LivingEntity.hurtServer 简化）：无敌帧门、击退、受伤音、
    /// 死亡置位。`from`=伤害来源（None = 环境伤害，不击退）。
    ///
    /// 无敌帧为 **tick** 单位：结算后置 `invulnerable = 20`（=1s，
    /// LivingEntity.java:1206 `invulnerableTime = 20`；ServerPlayer.java:576-577
    /// 每 tick −1，本仓在 fixed_step 的 on_tick 里递减）。差值门与 mob 路径
    /// 同构（combat::invulnerable_gate:50-60 ← LivingEntity.java:1196-1206）：
    /// invulnerable > 10 tick 时，伤害 ≤ lastHurt 整段忽略；更强伤害只扣
    /// `伤害 − lastHurt` 且不重置无敌帧。注：26.1 该门只依赖 invulnerableTime
    /// 与 lastHurt（外加伤害类型的 BYPASSES_COOLDOWN 标签，本仓无此类伤害源，
    /// 不建模）；`lastHurtByMobTimestamp`/`lastHurtMobTimestamp` 只用于仇恨
    /// 记录（LivingEntity.java:241-244），不构成本门的一部分——按源码实况实现。
    pub fn hurt_player(&mut self, amount: f32, from: Option<Vec3>) {
        // 受伤 exhaustion 按 damage_type 数据取值（Player.java:761
        // causeFoodExhaustion(source.getFoodExhaustion())）：实体攻击
        // mob_attack/player_attack.json = 0.1；创造豁免同原版
        // abilities.invulnerable（Player.causeFoodExhaustion:1561-1567）。
        self.hurt_ex(amount, from, if from.is_some() { 0.1 } else { 0.0 }, false);
    }

    /// 火系伤害统一入口（damage_type effects=burning 家族：lava/in_fire/
    /// on_fire，均 ∈ IS_FIRE 标签）：现役防火效果 → 整段免伤
    /// （LivingEntity.hurtServer:1163-1165 `source.is(IS_FIRE) &&
    /// hasEffect(FIRE_RESISTANCE) → return false`）。
    fn hurt_fire(&mut self, amount: f32, food_exhaustion: f32) {
        if self.effects.has(mcv_entity::Kind::FireResistance) {
            return;
        }
        self.hurt_ex(amount, None, food_exhaustion, false);
    }

    /// hurt_player 的参数化内核：`food_exhaustion` = damage_type.json 的
    /// exhaustion 值（lava/in_fire = 0.1、on_fire/in_wall/out_of_world/starve/
    /// drown = 0.0）；`bypass_creative` = 伤害类型 ∈ bypasses_invulnerability
    /// 标签（仅 out_of_world/fell_out_of_world，tags/damage_type/
    /// bypasses_invulnerability.json：values = [out_of_world, generic_kill]）
    /// ——创造免疫走 Entity.isInvulnerableToBase:2955-2960 的
    /// `invulnerable && !BYPASSES_INVULNERABILITY` 门，穿标签的伤害照常结算。
    fn hurt_ex(
        &mut self,
        amount: f32,
        from: Option<Vec3>,
        food_exhaustion: f32,
        bypass_creative: bool,
    ) {
        // 难度缩放（Player.hurtServer:692-706）：仅 scalesWithDifficulty 伤
        // 害源（DamageSource.java:92-97 = LivingEntity 造成且非玩家 → 本仓
        // mob 近战/箭/爆炸，`from` 有值）参与；和平归 0 直接免伤结算。
        let amount = if from.is_some() {
            crate::difficulty::scale_entity_damage(amount, self.difficulty)
        } else {
            amount
        };
        if amount <= 0.0 {
            return;
        }
        let p = &mut self.player;
        if p.health <= 0.0 || (self.mode == GameMode::Creative && !bypass_creative) {
            return;
        }
        let guard = p.invulnerable > 10;
        let Some(mut dmg) =
            combat::invulnerable_gate(p.invulnerable.max(0) as u32, p.last_hurt, amount)
        else {
            return;
        };
        p.last_hurt = amount;
        // 差值分支（LivingEntity.java:1201-1202）无敌帧不重置；全额分支才置 20
        // （LivingEntity.java:1206）。
        if !guard {
            p.invulnerable = 20;
        }
        // 吸收盾先扣（LivingEntity.actuallyHurt:1933-1939：dmg = max(dmg−absorb,0)，
        // absorb −= 吸收量；:3335 clamp 0..maxAbsorption——本仓 maxAbsorption
        // 由吸收效果修饰给出，见 effects bundle）。伤害为 0 时不作结算音外副作用。
        let absorbed = dmg.min(p.absorption);
        dmg -= absorbed;
        p.absorption = (p.absorption - absorbed).max(0.0);
        p.health -= dmg;
        // 受伤 exhaustion 按 damage_type 数据取值（Player.java:761
        // causeFoodExhaustion(source.getFoodExhaustion())）。
        if food_exhaustion > 0.0 {
            p.exhaustion = (p.exhaustion + food_exhaustion).min(EXHAUSTION_MAX);
        }
        if let Some(src) = from {
            let push = glam::Vec3::new(p.pos.x - src.x, 0.0, p.pos.z - src.z);
            // 受击击退力度 0.4（LivingEntity.java:1238 `knockback(0.4F, xd, zd)`，
            // 旧实现 0.5）；冲刺命中 +0.5 是攻击方机制（Player.java:987），不在此。
            let kb = mcv_entity::combat::knockback_velocity(p.vel, p.on_ground, 0.0, 0.4, push);
            p.vel = kb;
        }
        if p.health <= 0.0 {
            p.health = 0.0;
            self.dead = true;
            // clearFire（26.1 Player.die:554-555 死亡即熄灭）。
            self.fire_ticks = 0;
            if self.mode == GameMode::Hardcore {
                self.hardcore_death = true;
            }
            // 死亡掉落（26.1 Player.die → Inventory.dropAll，keepInventory
            // 默认 false）：全 36 格（快捷栏 9 + 主背包 27）逐格生成
            // ItemDrop（拾取延迟 40 tick = 2 s，LivingEntity.java:3398），
            // 与 mob 死亡掉落同一生成路径，再清栏。旧实现只掉快捷栏 9 格，
            // main 27 格死亡保留 = 变相 keepInventory。
            // KNOWN-DIVERGENCE：原版 Player.die 还会掉合成光标手持堆
            // （AbstractContainerMenu carried）；本工程光标堆存活在 mcv_app
            // 的 UI 会话层（CraftScreen.cursor，app.rs 持有），Game 状态层
            // 不可达，暂不掉落。
            let at = p.pos + Vec3::Y * 0.9;
            let mut rng = spawn_rng();
            for s in self.hotbar.slots.iter().chain(self.hotbar.main.iter()) {
                if !s.is_empty() {
                    mcv_entity::spawn_item_drop(
                        &mut self.mobs_app.world,
                        at,
                        s.item,
                        s.count,
                        mcv_entity::DEATH_PICKUP_DELAY,
                        &mut rng,
                    );
                }
            }
            self.hotbar = mcv_item::Hotbar::empty();
        }
        let pos = [p.pos.x, p.pos.y, p.pos.z];
        // 26.1 sounds.json 事件:变体随机交给音效表按权重抽取。
        // 消费点 LivingEntity.java:2019 playSound(hurtSound, …)。
        self.audio
            .play_event(mcv_audio::events::PLAYER_HURT, pos, pos, 1.0);
    }

    /// 死亡界面「重生」：满状态回出生点上方（FoodData 全新实例 = 满食 +
    /// 饱和 5.0 + exhaustion 0，FoodConstants.java:6 / FoodData.java:15；
    /// 无敌帧 0——26.1 只有 hurt 全额分支才置 invulnerableTime=20
    /// （LivingEntity.java:1206），restoreFrom 不传送无敌帧，旧实现 20 tick
    /// 重生保护期无源码依据）。
    pub fn respawn(&mut self) {
        self.player.health = 20.0;
        self.player.hunger = 20.0;
        self.player.saturation = 5.0;
        self.player.exhaustion = 0.0;
        self.player.last_hurt = 0.0;
        self.food_tick_timer = 0;
        self.player.invulnerable = 0;
        // 效果与吸收盾不随复活保留（26.1 死亡即清 activeEffects）。
        self.effects.clear();
        self.player.absorption = 0.0;
        self.player.vel = Vec3::ZERO;
        self.player.flying = self.mode == GameMode::Creative;
        self.dead = false;
        self.fall_y = None;
        self.air_supply = MAX_AIR_SUPPLY;
        self.swimming = false;
        // 燃烧不随复活保留（26.1 死亡 clearFire，Player.die:554）。
        self.fire_ticks = 0;
        // 26.1 重生与首次进入同走加载画面：handleRespawn →
        // startWaitingForNewLevel（ClientPacketListener.java:1259、:1280），
        // closeDelay 用默认 0（重生不走 `new LevelLoadTracker(500)` 那条
        // 新世界路径，Minecraft.java:2083）。
        self.begin_load(false);
        // 复活点位改走出生点投放路径（stream() 里按 heightmap 落地）：
        // 旧实现固定 (8.5, 200, 8.5) 自由落体，若死亡点远离出生点、区块
        // 已卸载（unloaded 按实心石代理）或地形顶面远低于 200，落地即
        // floor(高度差−3) 摔死循环。pos=ZERO + spawned=false 即复用首次
        // 进世界的投放（game.rs stream 出生投放块）。
        self.player.pos = Vec3::ZERO;
        self.spawned = false;
    }

    /// 进入/重进加载态（26.1 `Minecraft.doWorldLoad` :2079-2081
    /// `setScreen(new LevelLoadingScreen(loadTracker, …))` + 重生路径
    /// `ClientPacketListener.startWaitingForNewLevel` :1630-1643）。
    ///
    /// `new_world` = true 时关屏延迟 500ms（Minecraft.java:2083
    /// `LevelLoadTracker(newWorld ? 500L : 0L)` → 10 tick），存档载入/
    /// 重生为 0。等待截止 30s（`CLIENT_WAIT_TIMEOUT_MS`，LevelLoadTracker
    /// .java:26）超时放行，防区块流式卡死时永久黑屏。
    pub fn begin_load(&mut self, new_world: bool) {
        self.phase = GamePhase::Loading;
        self.load_close_delay_ticks = if new_world { 10 } else { 0 };
        self.load_ready_at = None;
        self.load_deadline_tick = self.game_ticks + 600;
        self.smoothed_progress = 0.0;
    }

    /// 加载进度统计半径（出生点区块四周）。
    ///
    /// 原版玩家区块批 = `EXPECTED_PLAYER_CHUNKS = Mth.square(7)` = 7×7
    /// （LevelLoadProgressTracker.java:15），即半径 3；上限再与渲染距离取
    /// min——stream 只在 render_dist 环内请求区块，半径超过它就永远等不齐
    /// （设置界面下限 4，正常运行恒取 3，此 min 仅为兜底）。
    fn load_radius(&self) -> i32 {
        self.render_dist.min(3)
    }

    // ---- 流式分层：模拟区 / 请求环 / 卸载环（fix/stream-collision；
    // 刀 3 起全部由票级别派生承载，见 [`crate::ticket`]）----
    //
    // 原版三层距离（审计 stream-arch §1）：加载 ticket 半径 = viewDistance
    // （DistanceManager.java:43, :319-321），模拟半径独立字段默认 10
    // （:48），玩家所在区块恒持 PLAYER_SIMULATION ticket（addPlayer
    // :110-117、ChunkMap.move :1071-1096 随区块迁移），实体仅在模拟环内
    // tick（ServerLevel.java:419），客户端收发环另有 vd+2 缓冲
    // （ChunkTrackingView.java:71-80），FULL 之外还有生成晕圈
    // （ChunkLevel.java:14-15）。本仓单进程一体，分层落地为：
    //   请求环 = 模拟半径 sim_dist = rd+1（含原版 +1 支撑环）= 票发放
    //   半径 view_dist（refresh_player_tickets），
    //   可见网格半径 ≤ sim_dist（remesh 环过滤，杜绝「看得见却穿透」），
    //   卸载 = 票级别 > MAX_LEVEL（ChunkMap.java:376-381），级别场边界
    //   = view_dist + RADIUS + 2 = rd+4 ≡ 旧自研 unload_dist（迟滞带 3 环
    //   ≥ 2，消除边界翻动；旧值 rd+2 与请求环仅隔 1 环，是「动一下就换
    //   一批块」的直接来源），
    //   玩家物理恒走真体素（sim_safe_radius 门 + clamp_to_sim_area 钳制）。
    // 模拟环 ≫ 一步最远位移：钳制保证玩家与最近未就位区块恒隔 ≥1 整区块
    // （16 格），而单个固定步（1/60 s）最大位移（疾跑 5.6 m/s、创造冲刺
    // 飞行 20.2 m/s、击退初速叠加 ≈0.4 格）不足其 1/40。

    /// 模拟/请求半径：调度器目标环内（Chebyshev ≤ 本值）区块最终全部
    /// TerrainReady+，且可见网格只会建到本环内（物理安全区 ≥ 可见区）。
    pub fn sim_dist(&self) -> i32 {
        self.render_dist + 1
    }

    /// 卸载阈值：Chebyshev 距离 > 本值的区块退出活动表。比请求环多 3 环
    /// 迟滞（要求 ≥2），原版等效余量 = 发书缓冲 vd+2 + FULL 生成晕。
    pub fn unload_dist(&self) -> i32 {
        self.sim_dist() + 3
    }

    /// 玩家票维护（刀 3a 接线）：每帧从玩家锚点全量重建——
    /// - PLAYER_SIMULATION 单锚点（玩家块一张，级别 max(0, 31 − sim_dist)；
    ///   DistanceManager.java:110-117 addPlayer / :132-134）；
    /// - PLAYER_LOADING 方环（Chebyshev ≤ view_dist 的每块一张、级别 31；
    ///   DistanceManager.java:276,:319-321）。
    ///
    /// view_dist 取 `sim_dist()` = rd+1（视距语义不变——本仓请求环含 +1
    /// 支撑环，票发放半径对齐它后，级别传播给出的加载晕圈恰到 rd+4 ≡
    /// 旧 `unload_dist`，卸载候选集逐块等价，见 [`crate::ticket`] 模块注释）。
    ///
    /// 债：原版为增量 tracker（addPlayer/removePlayer/updateViewDistance +
    /// queueLevels 传播队列），本仓每帧重建 + 解析求 min——传播不动点等价
    /// （ChunkTracker.java:23-27,:66），增量性不复制。
    fn refresh_player_tickets(&mut self, center: ChunkPos) {
        let vd = self.sim_dist();
        let sim_level = (ticket::ENTITY_TICKING_LEVEL - vd).max(0) as u8;
        self.tickets.clear();
        self.tickets
            .add_ticket(center, TicketType::PlayerSimulation, sim_level);
        for dx in -vd..=vd {
            for dz in -vd..=vd {
                self.tickets.add_ticket(
                    ChunkPos::new(center.x + dx, center.z + dz),
                    TicketType::PlayerLoading,
                    ticket::ENTITY_TICKING_LEVEL as u8,
                );
            }
        }
    }

    /// 模拟安全半径：以 `pc` 为中心最大 r ≥ 0，使 Chebyshev ≤r 的环全部
    /// **status ≥ Full**（原版实体物理门 = FULL 级 chunk：ChunkLevel.java
    /// :35-43 fullStatus，level ≤ 33 → FULL；Full 证据链 = worldgen commit
    /// 加 3×3 邻域光照齐，见 [`Self::advance_status_by_level`]）；玩家本块
    /// 未就位返回 **-1**（物理门：整步冻结，26.1 玩家实体在区块就绪前
    /// 不参与物理）。返回值同时导出钳制矩形（[`Self::clamp_to_sim_area`]），
    /// 玩家 AABB 查询的列恒真体素。
    ///
    /// r = min(票环派生值, stage 证据环)（刀 4a）：
    /// - 票环派生（O(1)）：玩家块为 sim 单锚点 + view_dist 方环加载票，
    ///   级别场闭式 level(d) = (31 − sim_dist) + d（两票种在级别场恰好
    ///   相切——环内 sim 锚点传播 ≤ 31 自持、环外与加载票 +1/环重合，
    ///   见 [`Self::refresh_player_tickets`] 与 [`crate::ticket`] 模块
    ///   注释）。Full 授权域 level ≤ 33（ChunkLevel.java:17-19
    ///   generationStatus）⟺ d ≤ sim_dist + (FULL − ENTITY_TICKING)。
    ///   授权域外的块 status 永远推不到 Full（推进器不授权），环扫描
    ///   必然自止于此——上界由票级别场解析给出，一查即得（sim_dist
    ///   ≤ 31 时精确；更大视距该值只是放宽的上界，证据门照常兜底）。
    /// - stage 证据环：status=Full 是否真已达成（体素+光照证据）只有
    ///   流式管线知道，级别场无法作证——逐环早退扫描保留（派单书
    ///   允许式）。相对刀 3 旧扫描（stage≥TerrainReady + unload_dist
    ///   上界）的差异面：已 TerrainReady 但 status 未推进/未授权的
    ///   过渡态，新值更小——收紧方向安全（钳制更早生效），对拍见
    ///   `sim_safe_ticket_predicate_is_sound_subset_of_legacy_scan`。
    pub fn sim_safe_radius(&self, pc: ChunkPos) -> i32 {
        let ready = |c: ChunkPos| {
            self.chunks
                .get(&c)
                .is_some_and(|h| (h.status() as u8) >= (ChunkStatus::Full as u8))
        };
        if !ready(pc) {
            return -1;
        }
        let max_r = self.sim_dist() + (ticket::FULL_LEVEL - ticket::ENTITY_TICKING_LEVEL);
        let mut r = 0;
        while r < max_r {
            let n = r + 1;
            // 只扫新入环的周界（内环上一轮已验证全就位）；周界累计
            // O(max_r²)，旧形状每轮重扫整个正方形是 O(max_r³)。
            let mut ring_ok = true;
            'ring: for dx in -n..=n {
                for dz in -n..=n {
                    if dx.abs() != n && dz.abs() != n {
                        continue;
                    }
                    if !ready(ChunkPos::new(pc.x + dx, pc.z + dz)) {
                        ring_ok = false;
                        break 'ring;
                    }
                }
            }
            if !ring_ok {
                break;
            }
            r = n;
        }
        r
    }

    /// 步末位置钳制：把玩家钉回票覆盖的已就位矩形（刀 4b 票环派生）。
    /// 矩形半宽 m = max(min(r_safe, cap) − 1, 0)：
    /// - cap = sim_dist + (FULL − ENTITY_TICKING) = 票级别场 Full 授权域
    ///   半径（级别场闭式 level(d) = (31 − sim_dist) + d ≤ 33，与
    ///   [`Self::sim_safe_radius`] 的票环上界同源——4a 的 r_safe 已含此
    ///   cap，此处 min 是防线纵深：任何未来调用方传入未收敛的就位环也
    ///   越不出票覆盖域，玩家票环恒在（DistanceManager.java:110-117
    ///   PLAYER_SIMULATION 单锚点 + :276,:319-321 加载方环随区块迁移，
    ///   ChunkMap.move :1071-1096）。
    /// - r_safe 侧来自 4a 证据环（status≥Full，ChunkLevel.java:35-43
    ///   FULL 级）——票授权 × 证据齐备二者取小，稳态下与票域一致收敛。
    ///
    /// `r ≥ 1` 时矩形区块再外扩一环（步进中原点位移 + AABB 半宽的查询
    /// 触达）仍整体落在已就位环 ≤r 内，pad = 玩家半宽 + 余量即可；
    /// `r = 0`（仅本块就位）时 pad 收到 1.0 格，触达不出本块。
    /// 被钳轴向速度清零（贴原版：移动在已加载区边缘自然停止——玩家区块
    /// 恒有 ticket、物理从不越界，而非撞假石头或穿进虚空）。
    fn clamp_to_sim_area(&mut self, pc: ChunkPos, r_safe: i32) {
        debug_assert!(r_safe >= 0);
        let cap = self.sim_dist() + (ticket::FULL_LEVEL - ticket::ENTITY_TICKING_LEVEL);
        let m = (r_safe.min(cap) - 1).max(0);
        let pad = if r_safe == 0 {
            1.0
        } else {
            mcv_game::Player::HALF[0] + 0.02
        };
        let lo_x = (pc.x - m) as f32 * 16.0 + pad;
        let hi_x = (pc.x + m + 1) as f32 * 16.0 - pad;
        let lo_z = (pc.z - m) as f32 * 16.0 + pad;
        let hi_z = (pc.z + m + 1) as f32 * 16.0 - pad;
        let p = &mut self.player;
        let nx = p.pos.x.clamp(lo_x, hi_x);
        if nx != p.pos.x {
            p.pos.x = nx;
            p.vel.x = 0.0;
        }
        let nz = p.pos.z.clamp(lo_z, hi_z);
        if nz != p.pos.z {
            p.pos.z = nz;
            p.vel.z = 0.0;
        }
        // 不变量兜底（debug）：钳制后玩家所在区块必须仍满足新 ready 判据
        // （status ≥ Full，与 sim_safe_radius 谓词同步——隐形墙回归前哨）。
        debug_assert!(
            self.chunks
                .get(&ChunkPos::new(
                    (p.pos.x / 16.0).floor() as i32,
                    (p.pos.z / 16.0).floor() as i32
                ))
                .is_some_and(|h| (h.status() as u8) >= (ChunkStatus::Full as u8)),
            "clamp_to_sim_area 后玩家仍在未就位区块：pos={} r_safe={}",
            p.pos,
            r_safe
        );
    }

    /// 出生搜索窗（区块 (0,0) ± [`Self::load_radius`]）是否全部
    /// TerrainReady。投放等齐窗再搜：只看 (0,0) 时邻块往往还在生成，
    /// 海景种子下搜索窗退化为单块全洋面、直接走兜底把玩家扔在海上；
    /// 窗就绪（TerrainReady）严格先于加载门放行（门要求同半径
    /// Uploaded，见 loading_progress_parts），故不推迟进世界。中心恒
    /// (0,0)：投放期间玩家仍在哨兵位 ZERO（26.1 出生区块即采样器选定
    /// 的 spawnChunk，MinecraftServer.java:489）。
    fn spawn_window_ready(&self) -> bool {
        let r = self.load_radius();
        (-r..=r).all(|dx| {
            (-r..=r).all(|dz| {
                self.chunks
                    .get(&ChunkPos::new(dx, dz))
                    .is_some_and(|h| (h.stage() as u8) >= (Stage::TerrainReady as u8))
            })
        })
    }

    /// 出生投放窗（世界出生区块 (0,0) 为中心、半径 [`Self::load_radius`]，
    /// 与 [`Self::spawn_window_ready`] 同源同半径）的加载门就绪统计
    /// （刀 3b 解耦点①）：`(已就绪数, 总数)`。就绪 = **status ≥ Surface**
    /// （worldgen commit / 读盘完成——世界生成完成度，原版 LevelLoadTracker
    /// 等服务器侧区块就绪的映射；status 由 advance_status_by_level 按票
    /// 级别派生维护）。网格/上传（stage Uploaded）**另判**——见
    /// [`Self::loading_progress_parts`]（显示指标，不再挡进世界）。
    ///
    /// 窗口锚点（原版取证 LevelLoadTracker/PrepareSpawnTask）：26.1 在
    /// spawn finder 定位完成后才以出生位为中心发玩家区块批次——
    /// PrepareSpawnTask.java:135-143 `addTicketAndLoadWithRadius(
    /// TicketType.PLAYER_SPAWN, spawnChunk, 3)` + `updateFocus(spawnChunk)`
    /// （setInitialSpawn :491 首标 focus 同点），门批次锚在**出生点**上，
    /// 而非玩家实时位。本仓投放落点可合法偏离 (0,0)（setInitialSpawn
    /// :504-521 区块螺旋先到先得；vanilla density 内核换代后种子
    /// 20261010 出生区块全海洋、落点在 (1,0)）：门若跟随已投放的玩家位
    /// 会漂进请求环外（(4,z) 列从未生成），49 格永不齐 → 加载门死锁
    /// （run 38164228251 load_gate 红的根因）。故出生入口加载（新世界/
    /// 重生重投放）恒锚世界出生区块 (0,0)：`spawned` 仅由投放路径置位
    /// （load_meta 恢复存档位故意保持 false，见其注释），pos==ZERO 为
    /// 待投放哨兵（此时玩家位恰在 (0,0)，口径不变）。存档位恢复
    /// （pos≠ZERO 且 !spawned，无投放）不属出生入口，仍锚玩家实时位
    /// （原版 join 后批次围绕玩家，PlayerChunkSender）。登记裁剪债：
    /// 原版 LOAD_PLAYER_CHUNKS 批次锚**最终**出生位（finder 结果，可距
    /// 采样 spawnChunk 至 5 区块），本仓恒锚 (0,0)——投放落点保证落在
    /// 搜索窗（半径 3 = 门半径）内、其自身区块恒被门窗口覆盖，进世界后
    /// 流式按玩家位续载（sim 环 rd+1 ⊇ 落点邻域），仅门开时点差数块。
    pub fn load_gate_parts(&self) -> (usize, usize) {
        let r = self.load_radius();
        let center = if self.spawned || self.player.pos == Vec3::ZERO {
            ChunkPos::new(0, 0)
        } else {
            ChunkPos::new(
                (self.player.pos.x / 16.0).floor() as i32,
                (self.player.pos.z / 16.0).floor() as i32,
            )
        };
        let mut ready = 0usize;
        let total = ((2 * r + 1) * (2 * r + 1)) as usize;
        for dx in -r..=r {
            for dz in -r..=r {
                if self
                    .chunks
                    .get(&ChunkPos::new(center.x + dx, center.z + dz))
                    .is_some_and(|h| (h.status() as u8) >= (mcv_core::ChunkStatus::Surface as u8))
                {
                    ready += 1;
                }
            }
        }
        (ready, total)
    }

    /// 加载统计：`(已就绪数, 总数)`。就绪 = 状态机走到 [`Stage::Uploaded`]
    /// （网格已建并上传 GPU；真实网格器建网格成功处推进该状态，无头路径
    /// 由测试手工推进——见 mobs_runtime.rs:44 同款用法）。
    ///
    /// 刀 3b 解耦点①后本统计是**网格侧显示指标**（加载画面的进度百分比、
    /// 超时补载测试的就绪判据），不再是加载门谓词——门改读
    /// [`Self::load_gate_parts`]（status ≥ Surface）。进度语义对应 26.1
    /// `LevelLoadProgressTracker` 的 currentChunks/totalChunks 分段分数
    /// （:62-70），本仓无服务端权重段（PREPARE_SERVER_WEIGHT/
    /// LOAD_PLAYER_CHUNKS 是服务器启动编排，单进程一体不存在），直接用
    /// 就绪区块占比。
    pub fn loading_progress_parts(&self) -> (usize, usize) {
        let r = self.load_radius();
        let center = ChunkPos::new(
            (self.player.pos.x / 16.0).floor() as i32,
            (self.player.pos.z / 16.0).floor() as i32,
        );
        let mut ready = 0usize;
        let total = ((2 * r + 1) * (2 * r + 1)) as usize;
        for dx in -r..=r {
            for dz in -r..=r {
                if self
                    .chunks
                    .get(&ChunkPos::new(center.x + dx, center.z + dz))
                    .is_some_and(|h| (h.stage() as u8) >= (Stage::Uploaded as u8))
                {
                    ready += 1;
                }
            }
        }
        (ready, total)
    }

    /// 原始加载进度（钳制 0..1；就绪数/总数，对应 26.1 serverProgress）。
    pub fn loading_progress(&self) -> f32 {
        let (ready, total) = self.loading_progress_parts();
        if total == 0 {
            return 0.0;
        }
        (ready as f32 / total as f32).clamp(0.0, 1.0)
    }

    /// 显示用平滑进度（LevelLoadingScreen.java:84 每 tick lerp 0.2）。
    pub fn loading_progress_smoothed(&self) -> f32 {
        self.smoothed_progress
    }

    /// 在途生成请求数（pending_generate 集合大小）。对应原版
    /// ThrottlingChunkTaskDispatcher.chunkPositionsInExecution 的规模
    /// （:13；getDebugStatus :39-46 暴露同类运行时状态）。
    pub fn inflight_generation_count(&self) -> usize {
        self.req_frames.len()
    }

    /// 卸载复活缓存积压数（原版 ChunkMap.pendingUnloads / unloadQueue
    /// 的规模；hasWork/积压门 ChunkMap.java:465-498 消费同类状态）。
    pub fn pending_unload_count(&self) -> usize {
        self.pending_unloads.len()
    }

    /// 加载画面的区块状态网格数据（26.1 `ChunkLoadStatusView` 等价）：
    /// `(dx, dz, Option<Stage as u8>)` 列表，视野半径 7 = 26.1
    /// `chunkStatusViewRadius = max(5, 3) + RADIUS_AROUND_FULL_CHUNK + 1`
    /// （Minecraft.java `doWorldLoad`；RADIUS_AROUND_FULL_CHUNK=1，
    /// ChunkLevel.java:13-14——FULL 步仅继承 LIGHT 对 INITIALIZE_LIGHT 的
    /// 半径 1 需求，ChunkPyramid.java:36-39）。未加载格返回 None。
    pub fn loading_grid(&self) -> (i32, Vec<(i32, i32, Option<u8>)>) {
        // max(5, 3) + RADIUS_AROUND_FULL_CHUNK + 1 = 5 + 1 + 1 = 7。
        let radius = 5 + 1 + 1;
        let center = ChunkPos::new(
            (self.player.pos.x / 16.0).floor() as i32,
            (self.player.pos.z / 16.0).floor() as i32,
        );
        let mut cells = Vec::with_capacity(((2 * radius + 1) * (2 * radius + 1)) as usize);
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                let stage = self
                    .chunks
                    .get(&ChunkPos::new(center.x + dx, center.z + dz))
                    .map(|h| h.stage() as u8);
                cells.push((dx, dz, stage));
            }
        }
        (radius, cells)
    }

    /// 加载态门：输入清零、进度平滑、就绪判定与阶段转移。每固定步调用
    /// （就绪判定 ≤49 次 stage 读，开销可忽略；平滑按 on_tick 20Hz——
    /// 原版 LevelLoadingScreen.tick 走 20 TPS 游戏拍）。
    fn update_load_gate(&mut self) {
        // 进度平滑（LevelLoadingScreen.java:84，每 tick lerp 0.2）。
        if self.on_tick {
            let target = self.loading_progress();
            self.smoothed_progress =
                (self.smoothed_progress + (target - self.smoothed_progress) * 0.2).clamp(0.0, 1.0);
        }
        // 邻域首次全就绪 → 记账 ClientLevelReady(readyAt)
        // （LevelLoadTracker.java:107、WaitingForPlayerChunk.tick :117-120）。
        // 刀 3b 解耦点①：门谓词从 stage==Uploaded 改 status ≥ Surface
        // （世界生成完成度；原版等服务器侧 FULL/区块就绪，网格上传是
        // 客户端侧后续流水不挡进世界）。Spawn 态语义归出生门（出生投放
        // 块仍以出生列及邻域 TerrainReady 为门，现状映射）。
        let (ready, total) = self.load_gate_parts();
        let all_ready = ready >= total;
        if all_ready && self.load_ready_at.is_none() {
            self.load_ready_at = Some(self.game_ticks);
        }
        // isLevelReady（LevelLoadTracker.java:66-68）= 已记账且
        // now ≥ readyAt + closeDelay；或 30s 超时放行（:152-156 的
        // "Timed out … letting the player into the world anyway"）。
        let delayed_ok = self
            .load_ready_at
            .is_some_and(|t| self.game_ticks >= t + self.load_close_delay_ticks as u64);
        let timeout = self.game_ticks >= self.load_deadline_tick;
        if delayed_ok || timeout {
            self.phase = GamePhase::Playing;
            self.load_ready_at = None;
            // 行走 bob 基准重置：加载期的出生投放位移不进 walkDist
            // （原版玩家 join 后才开始 tick 计量，本仓加载期也在 tick）。
            self.walk_dist = 0.0;
            self.walk_dist_o = 0.0;
            self.bob = 0.0;
            self.bob_o = 0.0;
            self.bob_prev_pos = self.player.pos;
        }
    }

    /// 装配真实音频后端（app 层 open 成功后注入；失败保持 silent 降级）。
    pub fn set_audio(&mut self, audio: mcv_audio::AudioManager) {
        self.audio = audio;
    }

    /// F5 循环视角：第一 → 第三后 → 第三前（26.1 顺序）。
    pub fn cycle_camera(&mut self) {
        self.cam_type = match self.cam_type {
            CameraType::FirstPerson => CameraType::ThirdPersonBack,
            CameraType::ThirdPersonBack => CameraType::ThirdPersonFront,
            CameraType::ThirdPersonFront => CameraType::FirstPerson,
        };
    }

    /// Loads player state + world time from level.meta (if present).
    pub fn load_meta(&mut self) {
        let path = self.save_dir.join("level.meta");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        match mcv_save::LevelMeta::decode(&bytes) {
            Ok(meta) => {
                if meta.seed != self.seed {
                    self.seed = meta.seed;
                    // 调度器在 assemble 时按构造 seed 建池，而 on_world_pick
                    // （app.rs）传入的是当前时钟、并非存档 seed；此处不改写
                    // 的话，重进旧世界后所有新请求区块都会用错 seed 生成，
                    // 与存档地形接不上（边界断崖、按存档坐标落位悬空）。
                    // 重建即丢弃旧池中在途结果：load_meta 只在进世界装配期
                    // 调用，stream 尚未请求过区块，无丢失。
                    self.scheduler = mcv_worldgen::TerrainScheduler::new(
                        self.seed,
                        mcv_core::world_worker_count(),
                    );
                }
                self.time_ticks = meta.day_time;
                self.mode = GameMode::from_u8(meta.mode);
                // v6:天气随档恢复（26.1 ServerLevel.java:273 构造期
                // prepareWeather :687-694 的移植：raining → rainLevel=1.0、
                // thundering → thunderLevel=1.0；v1–v5 旧档无天气块 →
                // WeatherData 无档默认全 0/假（WeatherData.java:28-29）=
                // 长晴，不是开局下雨）。
                self.weather = crate::weather::Weather::restore(
                    meta.weather.clear_weather_time,
                    meta.weather.rain_time,
                    meta.weather.thunder_time,
                    meta.weather.raining,
                    meta.weather.thundering,
                );
                if let Some(p) = meta.player {
                    // 存档位置恢复即不走出生投放（stream 投放块以
                    // pos==ZERO 为待投放哨兵）。spawned 故意保持 false：
                    // 若在这里置 true，开局即退出的存档（meta 里 pos 还是
                    // ZERO）下次进入会永久冻结在哨兵位、永不投放——ZERO
                    // 位置交由投放路径处理才是全状态正确的。
                    self.player.pos = Vec3::new(p.x, p.y, p.z);
                    self.player.yaw = p.yaw;
                    self.player.pitch = p.pitch;
                    self.player.flying = p.flying;
                    self.player.sel_slot = p.sel_slot as usize;
                    // v5 生存数值（v1–v4 解码器已给开局默认值）：退出重进/
                    // 死亡重生不丢饥饿饱和账——不持久化则进食白吃。
                    self.player.health = p.health;
                    self.player.hunger = p.hunger;
                    self.player.saturation = p.saturation;
                    self.player.exhaustion = p.exhaustion;
                    self.air_supply = p.air_supply;
                    self.difficulty = crate::difficulty::Difficulty::by_id(p.difficulty);
                    // v3 起存档带快捷栏;v1/v2 读为空——新档 assemble 不再
                    // 发放（空栏即原版态），空存档热栏保持空即可。物品 id
                    // 越界(旧档)整槽跳过。
                    if !p.hotbar.is_empty() || !p.main.is_empty() {
                        let mut h = mcv_item::Hotbar::empty();
                        let mk = |(item, count, damage): (u16, u8, u16)| {
                            (count > 0 && (item as usize) < mcv_item::ITEMS.len()).then(|| {
                                mcv_item::ItemStack {
                                    item,
                                    count,
                                    damage,
                                    enchants: Vec::new(),
                                }
                            })
                        };
                        for (k, st) in p.hotbar.into_iter().take(9).enumerate() {
                            if let Some(st) = mk(st) {
                                h.slots[k] = st;
                            }
                        }
                        // v4 主背包 27 格;v3 档读为空(保持原行为)。
                        for (k, st) in p.main.into_iter().take(27).enumerate() {
                            if let Some(st) = mk(st) {
                                h.main[k] = st;
                            }
                        }
                        self.hotbar = h;
                    }
                }
                log::info!(
                    "loaded world meta: seed={} time={}",
                    self.seed,
                    self.time_ticks
                );
            }
            Err(e) => log::warn!("level.meta unreadable, fresh world: {e}"),
        }
    }

    /// Writes level.meta (player + time). Call on exit and periodically.
    pub fn save_meta(&self) {
        let meta = mcv_save::LevelMeta {
            seed: self.seed,
            name: "world".into(),
            day_time: self.time_ticks,
            mode: self.mode as u8,
            player: Some(mcv_save::PlayerMeta {
                x: self.player.pos.x,
                y: self.player.pos.y,
                z: self.player.pos.z,
                yaw: self.player.yaw,
                pitch: self.player.pitch,
                flying: self.player.flying,
                sel_slot: self.player.sel_slot as u8,
                // 9 槽全量导出,槽序即下标(count 0 = 空格)。
                hotbar: self
                    .hotbar
                    .slots
                    .iter()
                    .map(|s| (s.item, s.count, s.damage))
                    .collect(),
                // v4:主背包 27 格同样全量导出。
                main: self
                    .hotbar
                    .main
                    .iter()
                    .map(|s| (s.item, s.count, s.damage))
                    .collect(),
                // v5:生存数值（health/hunger/saturation/exhaustion/air/
                // difficulty）随档——进食成果跨会话成立。
                health: self.player.health,
                hunger: self.player.hunger,
                saturation: self.player.saturation,
                exhaustion: self.player.exhaustion,
                air_supply: self.air_supply,
                difficulty: self.difficulty.id(),
            }),
            // v6:天气五字段随档（26.1 WeatherData SavedData 落盘等价，
            // WeatherData.java:11-15 字段序；advanceWeatherCycle 每 tick
            // 把计时器写回 WeatherData，ServerLevel.java:734-738，任意
            // 时刻快照即合法存档态）。
            weather: {
                let (clear_weather_time, rain_time, thunder_time, raining, thundering) =
                    self.weather.weather_state();
                mcv_save::WeatherMeta {
                    clear_weather_time,
                    rain_time,
                    thunder_time,
                    raining,
                    thundering,
                }
            },
        };
        let tmp = self.save_dir.join("level.meta.tmp");
        if std::fs::create_dir_all(&self.save_dir).is_ok()
            && std::fs::write(&tmp, meta.encode()).is_ok()
        {
            let _ = std::fs::rename(&tmp, self.save_dir.join("level.meta"));
        }
    }

    /// Persists chunks with the SAVE dirty bit (region files), clearing the
    /// bit. Called periodically and at exit.
    ///
    /// 帧内常态写回不在这里：stream() 每帧 ≤2 块走 FIFO 落盘队列（写回
    /// 预算，审计 §5.3），30s 全量调用通常只剩扫描。`None` = 全量兜底（退出
    /// 安全需接 [`Self::flush_save_blocking`]，含 pending_unloads 复活缓存
    /// 清空——这些块已离开活动表，退出前必须落盘）。`Some(pos)` = 单块即时
    /// 存盘（未在册则跳过）。
    ///
    /// E4：异步路（`io` 在册）时本方法退化为**投递扫描**——锁内快照 +
    /// 投递独立 IO 线程，不执行 region IO；同步路（`MCV_IO_ASYNC=0` 回滚
    /// 开关/无头）保持 E3 前的同步写盘逐字不变。
    pub fn save_dirty(&mut self, only: Option<ChunkPos>) {
        if only.is_none() {
            let pend = std::mem::take(&mut self.pending_unloads);
            for (_, h) in pend {
                if h.dirty() & mcv_core::dirty::SAVE != 0
                    && (h.stage() as u8) >= (Stage::TerrainReady as u8)
                {
                    self.save_handle(&h, false);
                }
            }
        }
        let keys: Vec<ChunkPos> = match only {
            Some(p) => vec![p],
            None => self.chunks.keys().copied().collect(),
        };
        for pos in keys {
            let Some(handle) = self.chunks.get(&pos).cloned() else {
                continue;
            };
            if handle.dirty() & mcv_core::dirty::SAVE == 0 {
                continue;
            }
            if (handle.stage() as u8) < (Stage::TerrainReady as u8) {
                continue;
            }
            self.save_handle(&handle, false);
        }
    }

    /// 单区块落盘分叉：异步（E4 两段式投递，见 [`Self::post_save`]）或同步
    /// （`MCV_IO_ASYNC=0` 回滚开关/无头，[`Self::save_handle_io`] 原样）。
    /// `force` = 强制路径（复活缓存积压门/64 硬顶溢出）绕过在途高水位门。
    fn save_handle(&mut self, handle: &Arc<ChunkHandle>, force: bool) -> bool {
        if self.io.is_some() {
            self.post_save(handle, force)
        } else {
            self.save_handle_io(handle)
        }
    }

    /// E4 两段式写·投递段（原版锚 ChunkMap.java:773
    /// `CompletableFuture.supplyAsync(data::write, Util.backgroundExecutor())`
    /// 的同构：编码/写盘在后台，拷贝快照在主线程）：锁内快照
    /// voxels+heightmap（~197 KiB）→ 投递 IO 线程 → 完成回执在
    /// [`Self::process_io_done`] 消化。
    ///
    /// 脏位纪律（保住「失败保脏位」，game.rs 既有语义）：**投递即清
    /// SAVE 位**——在途窗内的编辑再次置位 → 下轮以新快照重写（盘上内容
    /// = 最后编辑）；完成回执失败 → 回执端重置脏位重试。同块已有在途
    /// 快照则跳过（最新快照覆盖语义由下轮投递承担，不重复投递）。
    /// 返回 true = 已快照并投递（预算记账）。
    fn post_save(&mut self, handle: &Arc<ChunkHandle>, force: bool) -> bool {
        let pos = handle.pos;
        if handle.dirty() & mcv_core::dirty::SAVE == 0
            || (handle.stage() as u8) < (Stage::TerrainReady as u8)
            || self.io_writes.contains_key(&pos)
            || (!force && self.io_outstanding() >= IO_HIGH_WATER)
        {
            return false;
        }
        // 主线程锁内快照（唯一碰 RwLock 的点，拷贝 ~50µs 量级）。
        let ids: Box<[u16]> =
            bytemuck::cast_slice(handle.voxels.read().unwrap().as_slice()).to_vec();
        let hm: Box<[i16; 256]> = Box::new(**handle.heightmap.read().unwrap());
        let seq = self.io_seq.wrapping_add(1);
        self.io_seq = seq;
        self.io_writes.insert(pos, seq);
        if let Some(io) = self.io.as_mut() {
            io.post(IoCommand::Write { pos, seq, ids, hm });
        }
        handle.clear_dirty(mcv_core::dirty::SAVE);
        true
    }

    /// E4：IO 完成回执消化（stream 每帧、[`Self::flush_save_blocking`] 退出
    /// 排空共用）。读回填走 `advance_to(TerrainReady)` 既有接线；写失败保
    /// 脏位；过期回执（卸载超时后重投的旧读/同块更新快照已接管的旧写）
    /// 按 seq 丢弃。
    fn process_io_done(&mut self, done: IoDone) {
        match done {
            IoDone::Written { pos, seq, ok } => {
                // 同块更晚快照在途（seq 已更新）→ 旧回执丢弃：盘序无关
                // （per-position region 槽位，最新快照覆盖），失败保脏位由
                // 最新在途回执负责。
                if self.io_writes.get(&pos) == Some(&seq) {
                    self.io_writes.remove(&pos);
                }
                if !ok && !self.io_writes.contains_key(&pos) {
                    // 写失败：保脏位（下帧队头重试并 log::error，与同步路
                    // save_handle_io 同文案）。手柄可能在册或在复活缓存。
                    let h = self.chunks.get(&pos).cloned().or_else(|| {
                        self.pending_unloads
                            .iter()
                            .find(|(p, _)| *p == pos)
                            .map(|(_, h)| h.clone())
                    });
                    if let Some(h) = h {
                        h.mark_dirty(mcv_core::dirty::SAVE);
                    }
                }
            }
            IoDone::Loaded { pos, seq, found } => {
                // 过期回执（超时卸载后重投的旧读）——丢弃，不惊动新一轮记账。
                let current = self.io_frames.get(&pos).map(|(_, s)| *s);
                if current != Some(seq) {
                    return;
                }
                self.io_frames.remove(&pos);
                match found {
                    Some(data) => {
                        let (ids, hm) = *data;
                        if let Some(h) = self.chunks.get(&pos) {
                            *h.voxels.write().unwrap() = load_voxels(&ids);
                            **h.heightmap.write().unwrap() = hm;
                            h.advance_to(Stage::TerrainReady);
                        }
                        // 手柄已超时卸载 → 数据随回执丢弃，下轮重投读。
                    }
                    None => {
                        // 未存档/读失败 → 回退生成器（与同步 try_load_saved
                        // 同语义）。手柄已在册（Empty），直接发 worker 生成
                        // 请求并计入节流记账（本帧 pops 相应收缩）。
                        let miss = self
                            .chunks
                            .get(&pos)
                            .is_some_and(|h| h.stage() == Stage::Empty);
                        if miss {
                            self.scheduler.request(pos);
                            self.req_frames.insert(pos, self.stream_frame);
                        }
                    }
                }
            }
        }
    }

    /// IO 在途记账观测面（测试取证；非调度依据）：`(在途写, 在途读)`。
    /// 在途写 = 已投递快照未收到完成回执；在途读 = 已投递读请求未回填。
    #[doc(hidden)]
    pub fn io_inflight(&self) -> (usize, usize) {
        (self.io_writes.len(), self.io_frames.len())
    }

    /// 测试钩子：登记一笔在途读记账（等价请求环投递后的驻留标记），
    /// 供防乒乓驻留断言在不依赖真实 IO 时序的情况下驱动。
    #[doc(hidden)]
    pub fn io_debug_mark_loading(&mut self, pos: ChunkPos) {
        let seq = self.io_seq.wrapping_add(1);
        self.io_seq = seq;
        self.io_frames.insert(pos, (self.stream_frame, seq));
    }

    /// 测试钩子：只消化 IO 完成回执（等价 stream 帧头的 drain 半段，不跑
    /// 环扫描/写回预算）——写路径语义测试用它驱动回执而不引入预算流竞争。
    #[doc(hidden)]
    pub fn io_debug_drain(&mut self) {
        while let Some(done) = self.io.as_mut().and_then(|io| io.try_recv()) {
            self.process_io_done(done);
        }
    }

    /// 退出安全落盘：全量投递扫描（[`Self::save_dirty(None)`]）+ **阻塞排空
    /// 在途 IO**。仅退出/挂起/关窗路径调用（quit_to_menu、suspended、
    /// CloseRequested）——周期 30s 存盘保持纯投递（不掉帧）。写持续失败时
    /// 有界自旋（截稿 5s）后放弃并 log::error（保脏位语义在退出场景失效
    /// 与同步路「一次重试后丢弃」同形）。
    pub fn flush_save_blocking(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            self.save_dirty(None);
            // 排空在途：回执总数 = 在途投递数，recv_timeout 有界等待。
            while self.io_outstanding() > 0 && std::time::Instant::now() < deadline {
                let Some(done) = self
                    .io
                    .as_mut()
                    .and_then(|io| io.recv_timeout(std::time::Duration::from_millis(100)))
                else {
                    continue;
                };
                self.process_io_done(done);
            }
            let pending = self.chunks.values().any(|h| {
                h.dirty() & mcv_core::dirty::SAVE != 0
                    && (h.stage() as u8) >= (Stage::TerrainReady as u8)
            }) || self.pending_unloads.iter().any(|(_, h)| {
                h.dirty() & mcv_core::dirty::SAVE != 0
                    && (h.stage() as u8) >= (Stage::TerrainReady as u8)
            });
            if !pending {
                return;
            }
            if std::time::Instant::now() >= deadline {
                log::error!("flush_save_blocking deadline: SAVE-dirty chunks remain");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// 在途投递总数（含未消化回执）；高水位门数据源（内存有界）。
    fn io_outstanding(&self) -> usize {
        self.io.as_ref().map_or(0, IoChannel::outstanding)
    }

    /// 单区块 region 写盘（save_dirty 全量/预算流/pending_unloads 落盘
    /// 共用）。成功清 SAVE 脏并返回 true；IO 失败保留脏位（下帧/下轮
    /// 重试）并返回 false。
    fn save_handle_io(&mut self, handle: &ChunkHandle) -> bool {
        let pos = handle.pos;
        if handle.dirty() & mcv_core::dirty::SAVE == 0
            || (handle.stage() as u8) < (Stage::TerrainReady as u8)
        {
            return false;
        }
        let (rx, rz) = mcv_save::chunk_region(pos.x, pos.z);
        let local = mcv_save::chunk_local(pos.x, pos.z);
        let mut region = match mcv_save::RegionFile::open(&self.save_dir, rx, rz) {
            Ok(r) => r,
            Err(e) => {
                log::error!("region open failed {rx},{rz}: {e}");
                return false;
            }
        };
        let voxels = handle.voxels.read().unwrap();
        let ids = bytemuck::cast_slice(voxels.as_slice());
        let hm = handle.heightmap.read().unwrap();
        if let Err(e) = region.save_chunk(local, ids, &hm) {
            log::error!("chunk save failed {pos:?}: {e}");
            return false;
        }
        handle.clear_dirty(mcv_core::dirty::SAVE);
        true
    }

    /// 天气渲染参数（app 层 Scene 装配用）：`(day_factor 混合值, 雾色 RGB
    /// 乘子, 环境雾层端点 (start, end))`。`day` 传
    /// `mcv_render::sun_state(t).1`；雾端点基线 = 属性默认 0/1024
    /// （EnvironmentAttributes.java:18-24），不再吃 camera.far。
    pub fn weather_visual(&self, day: f32) -> (f32, [f32; 3], (f32, f32)) {
        (
            self.weather.sky_light_factor(day),
            self.weather.fog_tint(),
            self.weather.fog_env_endpoints(),
        )
    }

    pub fn camera(&self, aspect: f32) -> Camera {
        // 行走 bob：bobView 矩阵乘进投影侧（GameRenderer.java:684-691 世界 +
        // :360-362 手 pass 同源一份；本仓统一在 Camera::view_proj 里乘）。
        let (bob, bid) = self.hand_bob();
        let mut cam = Camera {
            pos: self.player.pos,
            yaw: self.player.yaw,
            pitch: self.player.pitch,
            fov_y: 1.25,
            aspect,
            near: 0.1,
            far: (self.render_dist * 16) as f32 * 1.6,
            bob: mcv_render::view_bob_matrix(bob, bid),
        };
        if self.cam_type != CameraType::FirstPerson {
            // 视眼 = pos + EYE_HEIGHT；第三人称沿视线平移，遇方块拉近
            //（MC GameRenderer 的 camera-clip 行为简化为 DDA 钳距）。
            let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
            let d = cam.dir();
            let sign = if self.cam_type == CameraType::ThirdPersonFront {
                1.0
            } else {
                -1.0
            };
            let mut dist = THIRD_PERSON_DIST;
            let view = WorldView {
                chunks: &self.chunks,
            };
            // 相机遮挡按碰撞形状（原版相机 clip 与拾取不同源：火把不挡相机）。
            if let Some((hit, _)) = dda_hit(
                &view,
                eye,
                d * sign,
                THIRD_PERSON_DIST + 0.5,
                mcv_game::blockshapes::RayTarget::Collide,
            ) {
                let t = (Vec3::new(hit.x as f32 + 0.5, hit.y as f32 + 0.5, hit.z as f32 + 0.5)
                    - eye)
                    .dot(d * sign);
                dist = dist.min((t - 0.3).max(0.5));
            }
            cam.pos = self.player.pos + d * (sign * dist);
        }
        cam
    }

    /// Request missing chunks in a spiral around the player (a few per call),
    /// unload far ones. 每帧一次（帧驱动流式调度）。
    ///
    /// 刀 3b 票驱动重写（预算迁原版语义，机制对照见各段注释）：
    /// - 请求 = 玩家票任务 pop，受在途生成集合 ≤4 节流
    ///   （ThrottlingChunkTaskDispatcher.java:13-14,:29-31；上限 4 =
    ///   DistanceManager.java:55）；worker TaskPool 不设帧预算（原版
    ///   worldgen/light dispatcher 无节流，ChunkMap.java:195-196）。
    /// - 卸载 = 票级别 > MAX_LEVEL（ChunkMap.java:376-381，迟滞=票级别
    ///   语义），每帧 ≤4 时间片（ChunkMap.java:477-498）+ 复活缓存
    ///   （ChunkMap.java:385-392）；积压门 48（原版 2000 按本仓规模缩减，
    ///   ChunkMap.java:489-495——登记裁剪债）。
    /// - 在途（Empty）请求 300 帧（≈5s）驻留不撤回——**与原版差异=债**：
    ///   原版无此机制（ticket 释放等 future，DistanceManager.java:87-104
    ///   是最近似对应物）；本仓防「请求→出环撤回→回环重请求→再生成」
    ///   乒乓的实测有效机制，保留。
    /// - 加载门 = status ≥ Surface（世界生成完成度），网格/上传另判
    ///   （解耦点①，见 load_gate_parts）；出生投放门维持出生列及邻域
    ///   TerrainReady 现状映射（Spawn 态语义归出生门）。
    /// - IO 预算化：读盘 ≤4/帧、卸载写盘常态 ≤2/帧（积压超门追加）、
    ///   脏块写回 ≤2/帧（app.rs 30s 全量存盘退化为兜底扫描）。E4 起
    ///   这些预算是**投递**预算：region open/read/write 在独立 IO 线程
    ///   执行（原版同构，ChunkMap.java:773/:566/:924），完成回执在下一帧
    ///   头消化；`MCV_IO_ASYNC=0` 回落同步执行路。
    pub fn stream(&mut self) {
        // E0：stream 总窗口起点（纯记账，见 [`FrameTiming`]；子段窗口互不
        // 重叠且均落在本窗内——init/edge/io/remesh/flush 之和 ≤ stream）。
        let stream_t0 = std::time::Instant::now();
        let center = ChunkPos::new(
            (self.player.pos.x / 16.0).floor() as i32,
            (self.player.pos.z / 16.0).floor() as i32,
        );
        self.stream_frame += 1;

        // 票表维护（刀 3a）：玩家票随 center 迁移全量重建，卸载与 status
        // 推进在本帧的派生都以它为输入。
        self.refresh_player_tickets(center);

        // 光照边派发队列消化（审计 A3/F2）：编辑路径只入队，这里按步预算
        // 跨帧处理，未完保留（原版 LevelLightEngine 重排语义）。放在最前，
        // 让标脏的 MESH 当帧进入下面的 remesh 预算流。
        if !self.pending_light_edges.is_empty() {
            // E0：edge 子段（帧头消化段，512 步预算）。
            let edge_t0 = std::time::Instant::now();
            let mut q = std::mem::take(&mut self.pending_light_edges);
            sync_light_edges(&self.chunks, &mut q);
            self.pending_light_edges = q;
            self.timing_frame.edge_us += edge_t0.elapsed().as_micros() as u64;
        }

        // ---- unload：票级别 > MAX_LEVEL（卸载迟滞=票级别语义）、每帧 ≤4、
        // pending_unloads 复活缓存 ----
        // 刀 3a：候选集从自研距离阈值改由票级别派生——`!is_loaded(level)`，
        // 原版 updateChunkScheduling 的 `!ChunkLevel.isLoaded(level)` → toDrop
        // （ChunkMap.java:376-381）。票发放半径 = view_dist = rd+1 时级别场
        // 给出的边界 = Chebyshev rd+4 ≡ 旧 `unload_dist()`，候选集逐块等价
        // （等价锁见 ticket.rs::loading_spread_unload_boundary_matches_legacy_
        // distance_rule）。
        let levels = self.tickets.level_map(self.chunks.keys().copied());
        let mut far: Vec<ChunkPos> = self
            .chunks
            .keys()
            .filter(|c| {
                let c = **c;
                !ticket::is_loaded(levels.get(&c).copied().unwrap_or(ticket::NOT_LOADED) as i32)
            })
            .copied()
            .collect();
        // 近端优先淘汰（同帧多候选时先处理离玩家近的，远期滞留短）。
        far.sort_by_key(|c| (c.x - center.x).abs().max((c.z - center.z).abs()));
        let mut evicted = 0usize;
        for c in far {
            if evicted >= 4 {
                break; // 时间片（原版 unloadQueue 按 tick 排空，ChunkMap.java:492）
            }
            let handle = self.chunks[&c].clone();
            if handle.stage() == Stage::Empty {
                // 在途请求驻留：worker 未回且发出 <5s（300 帧）不卸载，
                // 杜绝「请求→出环撤回→回环重请求→再生成」乒乓（症状 5）。
                // E4：在途记账区分生成（req_frames）/读盘（io_frames）两类，
                // 任一未超时都驻留。
                let expired = self
                    .req_frames
                    .get(&c)
                    .is_none_or(|f| self.stream_frame - f >= 300)
                    && self
                        .io_frames
                        .get(&c)
                        .is_none_or(|(f, _)| self.stream_frame - f >= 300);
                if !expired {
                    continue;
                }
                self.req_frames.remove(&c);
                self.io_frames.remove(&c);
                self.chunks.remove(&c);
                self.border_synced.remove(&c);
                self.mesh_fail.remove(&c);
                evicted += 1;
                continue;
            }
            self.chunks.remove(&c);
            self.border_synced.remove(&c);
            self.mesh_fail.remove(&c);
            // 网格记账同步回收：mesher 实现释放该块的网格字节与批次成员
            // 资格（GPU 缓冲随条目重建/清批 drop），meshed 集合删键保证重进
            // 视野时会重建网格（保持两者严格同步）。
            self.meshed.remove(&c);
            self.mesher.unload(c);
            // 数据不立即落盘/丢弃：进复活缓存，存盘按帧预算推进（原版
            // saveChunksEagerly 时间片）；重进请求环时原位复活。
            let entry = (c, handle);
            if !self.pending_unloads.iter().any(|(p, _)| *p == c) {
                self.pending_unloads.push(entry);
            }
            evicted += 1;
        }
        // 复活缓存落盘：常态每帧把最前 ≤2 块脏数据写盘；积压门与硬顶见下
        // （超限从最旧端强制落盘后出队，有界内存）。
        let mut pend = std::mem::take(&mut self.pending_unloads);
        // E0：io 子段（卸载落盘 + 积压/溢出门）窗口起点。
        let io_t0 = std::time::Instant::now();
        // 卸载队列积压门（刀 3b，原版 ChunkMap.java:489-495）：unloadQueue
        // 积压 ≥2000 时忽略 haveTime 强制排空 size−2000。本仓对应物 =
        // pending_unloads 超门即从最旧端强制落盘后出队（无 SAVE 位的条目
        // 本就无数据可丢，直接出队），保证队列有界且只进不滞。原版 2000 按
        // 本仓规模缩减（活动集 ≤ 数百块、复活缓存容量 64）取 48 = 容量
        // 3/4——**登记为裁剪债**（比例缩减，机制等价：时间片让位于积压）。
        // 常态落盘时间片 2/帧之外，积压期追加「len − GATE」个落盘名额
        // （同一「强制排空」在脏数据侧的体现）；64 硬顶保留为最终兜底。
        const UNLOAD_BACKLOG_GATE: usize = 48;
        let save_budget = 2usize + pend.len().saturating_sub(UNLOAD_BACKLOG_GATE);
        let mut saved = 0usize;
        for (_, h) in pend.iter_mut() {
            if saved >= save_budget {
                break;
            }
            if h.dirty() & mcv_core::dirty::SAVE != 0
                && (h.stage() as u8) >= (Stage::TerrainReady as u8)
                && self.save_handle(h, false)
            {
                saved += 1;
            }
        }
        while pend.len() > UNLOAD_BACKLOG_GATE {
            let (pos, h) = pend.remove(0);
            if h.dirty() & mcv_core::dirty::SAVE != 0
                && (h.stage() as u8) >= (Stage::TerrainReady as u8)
            {
                // 强制路径：绕过在途高水位门（投递后条目照常出队——写盘
                // 由 IO 线程收尾，不因出队丢失）。
                self.save_handle(&h, true);
            }
            log::debug!("pending_unloads backlog gate, flushed {pos:?}");
        }
        while pend.len() > 64 {
            let (pos, h) = pend.remove(0);
            if h.dirty() & mcv_core::dirty::SAVE != 0
                && (h.stage() as u8) >= (Stage::TerrainReady as u8)
            {
                self.save_handle(&h, true);
                log::debug!("pending_unloads overflow, flushed {pos:?}");
            }
        }
        self.pending_unloads = pend;
        self.timing_frame.io_us += io_t0.elapsed().as_micros() as u64;

        // 脏块写回（审计 §5.3：app.rs 30s 全量存盘是周期性掉帧源）：
        // 每帧 ≤2 块，30s 全量兜底通常只剩扫描。SAVE 位只在编辑与新生成
        // 时置位（commit_terrain / mark_dirty，写盘成功即清），脏集本身
        // 就是待落盘差量——按 (z,x) 稳定序从队头消耗（FIFO 落盘队列，
        // 成功清脏自动前进）。不用轮转游标：脏集变小时游标取模会回卷，
        // 加载洪流期对同一批 2 块逐帧重复 open+write（原版
        // saveChunksEagerly 也只扫 dirty 集限时间片，ChunkMap.java:500-513）。
        // IO 失败保脏位，下帧队头重试并 log::error。E4：≤2 保留为**投递**
        // 预算（锁内快照 + mpsc 投递），region 写盘在独立 IO 线程执行。
        let mut dirty_keys: Vec<ChunkPos> = self
            .chunks
            .iter()
            .filter(|(_, h)| {
                h.dirty() & mcv_core::dirty::SAVE != 0
                    && (h.stage() as u8) >= (Stage::TerrainReady as u8)
            })
            .map(|(p, _)| *p)
            .collect();
        if !dirty_keys.is_empty() {
            // E0：io 子段（脏块写回 ≤2）窗口起点。
            let io_t0 = std::time::Instant::now();
            dirty_keys.sort_unstable_by_key(|p| (p.z, p.x));
            for pos in dirty_keys.into_iter().take(2) {
                let handle = self.chunks[&pos].clone();
                self.save_handle(&handle, false);
            }
            self.timing_frame.io_us += io_t0.elapsed().as_micros() as u64;
        }

        // E4：IO 完成回执消化（读盘回填 advance_to(TerrainReady) / miss 转
        // 生成请求 / 写盘清脏或保脏）。置于请求段之前：读完成的 miss 在本帧
        // 直接发生成请求（req_frames 计入下方节流），读/写在途记账先行解除，
        // 本帧的环扫描与投递预算才看到最新状态。
        while let Some(done) = self.io.as_mut().and_then(|io| io.try_recv()) {
            self.process_io_done(done);
        }

        // drain completed terrain（结果到达即解除在途驻留记账）。置于请求
        // 段之前：先释放完成任务的在途名额，本帧的 pop 才能把名额补给新
        // 任务（原版 release→popTasks 的衔接；帧驱动下同帧先收后发）。
        while let Ok(result) = self.scheduler.results().try_recv() {
            match result {
                mcv_worldgen::GenResult::Terrain(Ok(out)) => {
                    self.req_frames.remove(&out.pos);
                    if let Some(handle) = self.chunks.get(&out.pos) {
                        mcv_worldgen::commit_terrain(handle, out);
                    }
                }
                mcv_worldgen::GenResult::Terrain(Err((pos, rc))) => {
                    self.req_frames.remove(&pos);
                    log::error!("terrain gen failed at {pos:?}: {rc}");
                }
            }
        }

        // request in ring order; ticket-driven pop（刀 3b）。
        // 多请求一圈支撑环（sim_dist = render_dist+1，原版 view distance
        // +1 边界块）：可见最外环的网格需要 3×3 邻域在册，只请求到
        // render_dist 时外环永远凑不齐邻居、永远建不了网格——真机上渲染
        // 边缘呈永久残缺带，玩家移动时残带随视野推进逐块翻新（症状：往右
        // 动一点就换一批块）。
        //
        // 票驱动节流（原版 ThrottlingChunkTaskDispatcher）：在途生成集合
        // （pending_generate = req_frames 键集）达到 MAX_CHUNKS_IN_EXECUTION
        // 前不再 pop 新玩家票任务（ThrottlingChunkTaskDispatcher.java:29-31
        // popTasks；集合本身 :13；上限 4 = DistanceManager.java:55 构造参
        // 数）。环序遍历 = 优先级队列按距离出队的等价序（近优先）。三条
        // 填充路径的预算切分（对照原版）：
        // - 复活：即时、无预算（原版 updateChunkScheduling 里 pendingUnloads
        //   命中即原位复活，ChunkMap.java:385-388）；
        // - 读盘：保留 4/帧时间片（原版 IO 走独立 executor 不入节流器；
        //   本仓同步 IO 在主线程，无预算会尖刺——自研债）。IO 片耗尽的
        //   miss 不得绕过读盘直发生成（存档会被生成结果覆盖）；
        // - 生成：受在途名额约束；worker TaskPool 本身不设帧预算（原版
        //   worldgen/light dispatcher 无节流，ChunkMap.java:195-196）。
        let mut pops = ticket::MAX_CHUNKS_IN_EXECUTION.saturating_sub(self.req_frames.len());
        let mut io_budget = 4usize;
        for r in 0..=self.sim_dist() {
            for dx in -r..=r {
                for dz in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue; // ring only
                    }
                    let pos = ChunkPos::new(center.x + dx, center.z + dz);
                    if self.chunks.contains_key(&pos) {
                        continue;
                    }
                    // 复活优先（原版 pendingUnloads 命中，ChunkMap.java:388-
                    // 392）：数据还在手里就不重开 IO、不重发 worker。
                    if let Some(i) = self.pending_unloads.iter().position(|(p, _)| *p == pos) {
                        let (_, h) = self.pending_unloads.remove(i);
                        self.chunks.insert(pos, h);
                        continue;
                    }
                    let handle = Arc::new(ChunkHandle::new(pos));
                    if io_budget > 0 {
                        if let Some(io) = self.io.as_mut() {
                            // E4 读异步化（≤4/帧保留为**投递**预算，不再执行）：
                            // Empty 手柄先在册 + io_frames 记账（复用 300 帧
                            // 驻留防乒乓），读盘在独立 IO 线程执行，结果经
                            // 完成通道回主线程走 advance_to(TerrainReady) 既有
                            // 接线（原版解析同走后台 executor，
                            // ChunkMap.java:566/:924）。miss 的生成请求由回执
                            // 端补发（本帧 pops 节流照算）。
                            let io_t0 = std::time::Instant::now();
                            io_budget -= 1;
                            let seq = self.io_seq.wrapping_add(1);
                            self.io_seq = seq;
                            self.io_frames.insert(pos, (self.stream_frame, seq));
                            io.post(IoCommand::Load { pos, seq });
                            self.timing_frame.io_us += io_t0.elapsed().as_micros() as u64;
                            self.chunks.insert(pos, handle);
                            continue;
                        }
                        // E0：io 子段（请求环读盘 ≤4）窗口。（同步路 =
                        // MCV_IO_ASYNC=0 回滚开关/无头）
                        let io_t0 = std::time::Instant::now();
                        let loaded = self.try_load_saved(&handle);
                        self.timing_frame.io_us += io_t0.elapsed().as_micros() as u64;
                        if loaded {
                            io_budget -= 1;
                            self.chunks.insert(pos, handle);
                            continue;
                        }
                    } else {
                        // IO 时间片耗尽：本帧跳过该 miss（未验存档前不得
                        // 直发生成），留待下帧。
                        continue;
                    }
                    if pops == 0 {
                        // 在途满：停发新票任务（节流器核心语义），继续扫
                        // 复活/读盘命中——二者不受节流。
                        continue;
                    }
                    self.chunks.insert(pos, handle);
                    self.scheduler.request(pos);
                    self.req_frames.insert(pos, self.stream_frame);
                    pops -= 1;
                }
            }
        }

        // 出生投放（26.1 setInitialSpawn + PlayerSpawnFinder 等价，判定见
        // find_spawn_slot / spawn_column_feet_y 注释）：待投放哨兵 = pos
        // 为 ZERO 且 !spawned（respawn():766-767 双复位，重进也走这里；
        // load_meta 恢复的存档位置 pos≠ZERO，不进本分支）。
        //
        // 触发时机：出生区块 TerrainReady **且** 搜索窗（±load_radius）
        // 全部 TerrainReady，或 30s 加载超时已转 Playing（用已就绪子集
        // 兜底，不无限冻结）。旧实现只等 (0,0)：海景种子下邻块尚未就绪
        // 时搜索窗退化为单块全洋面，玩家被扔在海底；窗就绪严格先于加载
        // 门放行（门要求同半径 Uploaded ⊇ TerrainReady），不推迟进世界。
        if !self.spawned
            && self.player.pos == Vec3::ZERO
            && let Some(handle) = self.chunks.get(&ChunkPos::new(0, 0))
            && (handle.stage() as u8) >= (Stage::TerrainReady as u8)
            && (self.spawn_window_ready() || self.phase == GamePhase::Playing)
        {
            // 列搜索（MinecraftServer.setInitialSpawn:504-521 区块螺旋 +
            // getSpawnPosInChunk 全列扫描）：拒绝水列与脚/头无空间列。
            // 旧实现直接取 (8,8) 列 hm+1 落点，海面列会把玩家放进海底
            // 沙地上（真机「复活在沙子底下、满屏沙」根因）。
            let slot = find_spawn_slot(&self.chunks, self.load_radius()).unwrap_or_else(|| {
                // 全窗无合法列（窗口全海洋）：26.1 fixupSpawnHeight
                // （PlayerSpawnFinder.java:89-104）兜底——建议列 (8,8)
                // 最高实体/流体顶 +1，海面即落在水面之上，不再埋入。
                let voxels = handle.voxels.read().unwrap();
                Vec3::new(8.5, fixup_spawn_feet_y(&voxels[..], 8, 8) as f32, 8.5)
            });
            self.player.pos = slot;
            self.spawned = true;
        }
        // light init on newly-terrain-ready chunks (budgeted, main thread)。
        // 候选按「离玩家近优先」排序：HashMap 迭代序随每次插入/删除漂移，
        // 每帧 2 格预算会随机落到世界任何角落——真机上表现为每帧零散点亮
        // 不同区块（地下/地上无规律闪现）。排序后布光以玩家为圆心成波推进，
        // 加载门控关心的出生邻域最先就绪。
        let mut light_budget = 2;
        let keys = self.sorted_keys(center);
        // E0：init 子段（光照 init ≤2，主线程双通道 BFS）窗口起点。
        let init_t0 = std::time::Instant::now();
        for pos in &keys {
            if light_budget == 0 {
                break;
            }
            let handle = self.chunks[pos].clone();
            if handle.stage() != Stage::TerrainReady {
                continue;
            }
            {
                let voxels_locked = handle.voxels.read().unwrap();
                let mut light_locked = handle.light.write().unwrap();
                let hm_locked = handle.heightmap.read().unwrap();
                let voxels: Vec<u16> = bytemuck::cast_slice(voxels_locked.as_slice()).to_vec();
                let hm: Vec<i16> = hm_locked.to_vec();
                let mut view = mcv_light::LightChunk {
                    voxels: &voxels,
                    light: &mut light_locked[..],
                    heightmap: &hm,
                };
                mcv_light::init(&mut view);
            }
            handle.advance_to(Stage::LightLocalReady);
            light_budget -= 1;
        }
        self.timing_frame.init_us += init_t0.elapsed().as_micros() as u64;

        // status 阶梯推进（刀 3a）：由票级别派生驱动，替换刀 2 的「同点同
        // 条件」接线（worldgen commit→Surface / 光照 init→Light）。本帧的
        // commit（drain）、读盘（request loop）、布光（上面的 init）产出的
        // 证据都在这里统一消费——授权门 = 票级别 target（原版
        // runAllUpdates → updateHighestAllowedStatus，DistanceManager.java:
        // 75-84；ChunkLevel.generationStatus，ChunkLevel.java:17-19），证据
        // 门 = 各 pass 完成的 stage 位。语义等价：请求环 ⊆ level ≤ 33 授权
        // 域，各块推进时机仍为同帧；跨块边同步不另立 status 档（stage 侧
        // border_synced/Lit 记账），债见 ChunkStatus 裁剪映射。
        let status_levels = self.tickets.level_map(self.chunks.keys().copied());
        self.advance_status_by_level(&status_levels);

        // border sync: mark pairs once both ends are LightLocalReady。
        // C1：新成对（两端都已本地布光）时真正交换边快照——init 只做块内
        // 播种，跨界的火把/阴影在此对齐（双向各发一条边，REMOVE+ADD 两相）。
        // E0：edge 子段（成对边标记 + 同步）窗口起点。
        let edge_t0 = std::time::Instant::now();
        let mut pair_edges: Vec<(ChunkPos, u8)> = Vec::new();
        for pos in &keys {
            let handle = self.chunks[pos].clone();
            if handle.stage() != Stage::LightLocalReady {
                continue;
            }
            let mut cur = *self.border_synced.entry(*pos).or_insert(0u8);
            let mut marks: Vec<(ChunkPos, u8)> = Vec::new();
            for (bit, (dx, dz)) in [(0u8, (1i32, 0i32)), (1, (-1, 0)), (2, (0, 1)), (3, (0, -1))] {
                if cur & (1 << bit) == 0 {
                    let npos = ChunkPos::new(pos.x + dx, pos.z + dz);
                    let ready = self
                        .chunks
                        .get(&npos)
                        .is_some_and(|n| (n.stage() as u8) >= (Stage::LightLocalReady as u8));
                    if ready {
                        cur |= 1 << bit;
                        let obit = Self::opposite_side(bit);
                        marks.push((npos, obit));
                        pair_edges.push((*pos, bit));
                        pair_edges.push((npos, obit));
                        // 真机取证埋点：成对标记应每对仅一次——静止期重复
                        // 出现同一对即记账被反复重置（乒乓实证）。
                        log::debug!("border pair {pos:?} bit{bit} <-> {npos:?}");
                    }
                }
            }
            self.border_synced.insert(*pos, cur);
            for (npos, obit) in marks {
                let ns = self.border_synced.entry(npos).or_insert(0u8);
                *ns |= 1 << obit;
            }
        }
        sync_light_edges(&self.chunks, &mut pair_edges);
        self.timing_frame.edge_us += edge_t0.elapsed().as_micros() as u64;
        // 成对边任务同样「预算耗尽不丢」（审计 A3 对 pair 路径的收口）：
        // border_synced 已先行记账，残留若随手 Vec 丢弃即永久暗缝——并入
        // 持久队列，下一帧 stream 开头的消化段续跑。
        if !pair_edges.is_empty() {
            for e in pair_edges {
                if !self.pending_light_edges.contains(&e) {
                    self.pending_light_edges.push(e);
                }
            }
        }

        // mesh chunks: 3x3 loaded, center lit, dirty or missing。
        // 同样按玩家近优先排序（理由同 light init）：网格以玩家脚下的区块
        // 最先建成，远处补齐——不再出现「眼前的块没网格、远处的块先上屏」。
        // 预算（审计 §4.3/A2）：初始装载（门控期）放宽 8/帧，进世界后
        // 2/帧。可见网格半径 ≤ sim_dist（物理安全区 ≥ 可见区，杜绝
        // 「看得见却穿透」；待卸滞留环不建网格不占预算）。
        // E0：remesh 子段（编辑源块优先通道 + 近优先预算流）窗口起点。
        let remesh_t0 = std::time::Instant::now();
        let mut remesh_budget: i32 = if self.phase == GamePhase::Loading {
            8
        } else {
            2
        };
        // 编辑源块当帧优先（审计 A2/F3）：级联标脏的邻块仍走近优先预算流。
        // E3：mesh_priority 走 hi 通道（worker 池优先队列跳过排队 lo），
        // 预算流走 lo——到达上传的先后由完成序决定，hi 保证编辑源块不排在
        // 旧预算任务之后（tests/mesh_parallel.rs 顺序锁）。
        if let Some(p) = self.mesh_priority.take()
            && self.build_chunk_mesh(p, true)
        {
            remesh_budget = remesh_budget.saturating_sub(1);
        }
        for pos in self.sorted_keys(center) {
            if remesh_budget == 0 {
                break;
            }
            if (pos.x - center.x).abs().max((pos.z - center.z).abs()) > self.sim_dist() {
                continue;
            }
            if self.build_chunk_mesh(pos, false) {
                remesh_budget -= 1;
            }
        }
        self.timing_frame.remesh_us += remesh_t0.elapsed().as_micros() as u64;
        // 异步上传冲刷（#80 第二刀）：本帧 build/unload 攒下的网格拷贝在
        // 此记入 staging 并 submit——先于同帧 draw_frame 的 submit（单队列
        // 按提交序执行），draw 必见数据；状态机 Uploaded 推进时序不变。
        // E3：worker 回传的产物也在此消化（RustMesher::flush 内先
        // drain_committed → uploader.build_chunk，再 staging submit）——
        // 回传≠上传：派发预算（上）由主线程执行，回传只搬字节。
        // E0：flush 子段（staging submit，独立列）+ stream 总窗口收账。
        let flush_t0 = std::time::Instant::now();
        self.mesher.flush();
        self.timing_frame.flush_us += flush_t0.elapsed().as_micros() as u64;
        self.timing_frame.stream_us += stream_t0.elapsed().as_micros() as u64;
    }

    /// 单个区块的建网格+记账（stream remesh 流水与编辑源块优先通道共用）。
    /// 返回 true = 本帧实际建了一块（消耗一格预算）。门槛：中心
    /// ≥LightLocalReady、3×3 邻域就绪（[`Self::neighbors_ready`]）、且
    /// （未建过 或 MESH 脏）。
    ///
    /// E3 双路分叉（蓝图快照-计算-回传三段式）：
    /// - **worker 路**（默认）：门槛通过后主线程快照 3×3（唯一碰 RwLock
    ///   的点，拷完即发），worker 跑 `Mesher::build`（opaque+water，~10ms
    ///   纯计算移出主线程，2 块/帧预算的实际帧成本 ~20ms → 快照拷贝 +
    ///   结果入池）；回传字节由 RustMesher::flush 经 uploader.build_chunk
    ///   进 staging。派发即记账（meshed/stage/MESH 脏位）：快照与脏位清除
    ///   在同一主线程瞬时——此前的编辑已在快照里，此后的编辑重新置位
    ///   MESH 触发下轮重建（同块在途产物由提交地板过滤，陈旧不覆盖），
    ///   无丢失更新。`hi` 选通道（编辑源块 hi / 预算流 lo）。
    /// - **同步路**（`MCV_MESHER_WORKERS=0` 回滚开关）：与 E3 前逐字节
    ///   同款，快照+计算+上传一体。
    fn build_chunk_mesh(&mut self, pos: ChunkPos, hi: bool) -> bool {
        let Some(handle) = self.chunks.get(&pos).cloned() else {
            return false;
        };
        if (handle.stage() as u8) < (Stage::LightLocalReady as u8) {
            return false;
        }
        if !self.neighbors_ready(pos) {
            return false;
        }
        let already = self.meshed.contains(&pos);
        let dirty_mesh = handle.dirty() & mcv_core::dirty::MESH != 0;
        if already && !dirty_mesh {
            return false;
        }
        let mut handles: [Arc<ChunkHandle>; 9] = core::array::from_fn(|_| handle.clone());
        for dz in -1i32..=1 {
            for dx in -1i32..=1 {
                let idx = ((dz + 1) * 3 + (dx + 1)) as usize;
                handles[idx] = self.chunks[&ChunkPos::new(pos.x + dx, pos.z + dz)].clone();
            }
        }
        // E3 worker 路：快照-计算-回传三段式（默认）。
        if let Some(sched) = self.mesh_sched.as_ref() {
            // 背压门：在途（排队+执行）≥ 高水位时本帧不投递（不耗预算，
            // 下帧预算流自动重试）——worker 饥饿时快照/产物字节（~5 MiB/笔
            // 双端）不得无界堆积；峰值在途 ≈ workers+2 笔。
            if sched.saturated() {
                return false;
            }
            let (voxels, lights) = snapshot_neighborhood(&handles);
            sched.submit(
                mcv_mesher::MeshJob {
                    pos,
                    origin: [16.0 * pos.x as f32, 0.0, 16.0 * pos.z as f32],
                    voxels,
                    lights,
                },
                hi,
            );
            // 派发即记账：快照与脏位清除在同一主线程瞬时（见函数 doc）。
            // meshed 记账同步：记新。
            self.meshed.insert(pos);
            handle.clear_dirty(mcv_core::dirty::MESH);
            // 网格派发即推进状态机终点 Uploaded（GPU 字节经回传异步到位，
            // 差 1-2 帧；加载门谓词是 status ≥ Surface——刀 3b 解耦点①，
            // 不读 stage，加载语义不变；loading_progress_parts 的显示统计
            // 相应提前 1-2 帧，非行为面）。无头 NullMesher + 同步路径不进
            // 本分支，记账仍由下方同步路承担。
            handle.advance_to(Stage::Uploaded);
            // 真机取证埋点（logcat -s RustMcv）：派发事件带区块坐标——
            // 与 flush 的 "mesh worker commit" 对账可观测回传延迟。
            log::debug!("mesh dispatch {} {pos:?}", if hi { "hi" } else { "lo" });
            return true;
        }
        // 同步路径（回滚开关）：E3 前逐字节同款。
        let Some(rc) = self.mesher.build(pos, &handles) else {
            // 失败分支不得静默（审计 B2/F6）：首建失败 = 该块不在
            // render_chunks——不可见却按真实体素碰撞（另一种「看不见却
            // 实心」）；重网格失败 = 旧网格无限期陈旧。MESH 脏保留，
            // 下帧预算流自动重试。计数退避：首败 error（真机取证坐标），
            // 重复失败降 debug 不刷屏——无头 NullMesher 恒失败属预期，
            // 全测试期只报一行。
            let n = self.mesh_fail.entry(pos).or_insert(0);
            *n += 1;
            if *n == 1 {
                log::error!("mesh build failed for {pos:?} (kept dirty, will retry)");
            } else {
                log::debug!("mesh build still failing for {pos:?} (n={n})");
            }
            return false;
        };
        self.mesh_fail.remove(&pos);
        let idx_count = rc.opaque_range.end;
        // #80 起网格字节已在 mesher 实现内入池并合批（条目表由实现自持、
        // 原位重建不受重网格影响），游戏层只做 meshed 记账；rc 仅取索引量
        // 入日志（合批后为所属批次总量，仅取证粒度差异，非行为变化），
        // 条目值随后自然弃置。
        // meshed 记账同步：记新。
        self.meshed.insert(pos);
        handle.clear_dirty(mcv_core::dirty::MESH);
        // 网格已建且经 MeshUploader 上传 GPU → 状态机终点
        // Uploaded（chunk.rs 实际路径 Empty→TerrainReady→LightLocalReady→
        // Uploaded；Lit/MeshReady 为预留死态，见 chunk.rs 文档。加载画面
        // 「就绪」判定依赖 Uploaded）。无头 NullMesher 不产出网格、不
        // 推进，由测试手工 advance_to。
        handle.advance_to(Stage::Uploaded);
        // Uploaded/MeshReady 是渲染记账，不占 status 档（stage 侧）——
        // status 无对应推进点（ChunkStatus 裁剪映射）。
        // 真机取证埋点（logcat -s RustMcv）：网格入队/上传事件带
        // 区块坐标与索引量——静止期反复出现即重网格循环实证。
        log::debug!("mesh upload {pos:?} idx={idx_count}");
        true
    }

    /// 在册区块按「玩家环距（Chebyshev）近优先」排序的键表，环内按
    /// (dx,dz) 字典序破平——同一输入恒得同一顺序。布光/成对边标记/建网格
    /// 三条预算流水共用：哈希表迭代序随插入删除逐帧漂移，按它花钱会让
    /// 每帧预算散落到任意区块（真机症状：每帧零散加载不同块、明暗碎片
    /// 闪烁）；排序后三条流水都以玩家为中心的同心波推进。
    fn sorted_keys(&self, center: ChunkPos) -> Vec<ChunkPos> {
        let mut keys: Vec<ChunkPos> = self.chunks.keys().copied().collect();
        keys.sort_by_key(|p| {
            let dx = p.x - center.x;
            let dz = p.z - center.z;
            (dx.abs().max(dz.abs()), dx, dz)
        });
        keys
    }

    pub(crate) fn opposite_side(bit: u8) -> u8 {
        match bit {
            0 => 1,
            1 => 0,
            2 => 3,
            _ => 2,
        }
    }

    /// Tries to load a chunk from its region file (skip if absent/corrupt).
    fn try_load_saved(&self, handle: &Arc<ChunkHandle>) -> bool {
        let pos = handle.pos;
        let (rx, rz) = mcv_save::chunk_region(pos.x, pos.z);
        let region_path = self.save_dir.join(format!("region/r.{rx}.{rz}.mcrv"));
        if !region_path.exists() {
            return false;
        }
        let mut region = match mcv_save::RegionFile::open(&self.save_dir, rx, rz) {
            Ok(r) => r,
            Err(_) => return false,
        };
        let mut ids = vec![0u16; mcv_core::CHUNK_VOL];
        let mut hm = [mcv_core::WORLD_MIN_Y as i16; 256];
        if region
            .load_chunk(mcv_save::chunk_local(pos.x, pos.z), &mut ids, &mut hm)
            .is_err()
        {
            return false; // 缺记录/版本不符（v1/v2）→ 回退生成器
        }
        *handle.voxels.write().unwrap() = load_voxels(&ids);
        // v3 起 heightmap 随记录落盘（原版做法：Heightmap.java:138
        // getRawData 直存 NBT，读盘不重算）；直接采纳盘上值。
        **handle.heightmap.write().unwrap() = hm;
        handle.advance_to(Stage::TerrainReady);
        true
    }

    /// 建网格门槛：3×3 邻域全部在册且都已完成本地布光（≥LightLocalReady）。
    /// 旧门槛只要求邻域 TerrainReady——网格顶点把邻块边界面光烘焙为字节，
    /// 未布光邻块的光数组是全 0，交界处会烘出一圈黑缝，等邻块布光+边同步
    /// 后再触发整块重建（先黑后亮的闪烁 + 一倍无效网格工作量）。中心块的
    /// stage 由调用方（stream 的 remesh 循环）单独检查。
    fn neighbors_ready(&self, pos: ChunkPos) -> bool {
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(h) = self.chunks.get(&ChunkPos::new(pos.x + dx, pos.z + dz)) {
                    if (h.stage() as u8) < (Stage::LightLocalReady as u8) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }
        true
    }

    /// status 阶梯推进：票级别派生驱动（刀 3a）。授权门 = 票级别允许的最
    /// 高 status（[`ticket::generation_status`]，ChunkLevel.generationStatus
    /// 镜像）；证据门 = 本仓各 pass 完成的 stage 位：
    /// - stage ≥ TerrainReady（worldgen commit / 读盘）→ `Surface`；
    /// - stage ≥ LightLocalReady（块内布光）→ `Light`；
    /// - `Full`：级别 ≤ 33 且 3×3 邻域 ≥ LightLocalReady——FULL 步的
    ///   accumulated dependencies 半径 1 = 邻域 LIGHT（ChunkPyramid.java:
    ///   36-39；与网格门槛 [`Self::neighbors_ready`] 同谓词）。FULL 蕴含的
    ///   跨块边一致性在本仓由 stage 侧 border_synced/Lit 记账（债，见
    ///   ChunkStatus 裁剪映射）。`Spawn` 不接线（语义归出生门，本仓出生门
    ///   = 出生列及邻域 TerrainReady 现状映射）。
    ///
    /// 级别 > MAX_LEVEL 的区块不授权任何推进（同帧已入卸载候选）。
    fn advance_status_by_level(&self, levels: &HashMap<ChunkPos, u8>) {
        for (pos, handle) in &self.chunks {
            let Some(target) = ticket::generation_status(
                levels.get(pos).copied().unwrap_or(ticket::NOT_LOADED) as i32,
            ) else {
                continue;
            };
            let target = target as u8;
            let stage = handle.stage() as u8;
            if target >= ChunkStatus::Surface as u8 && stage >= Stage::TerrainReady as u8 {
                handle.advance_status(ChunkStatus::Surface);
            }
            if target >= ChunkStatus::Light as u8 && stage >= Stage::LightLocalReady as u8 {
                handle.advance_status(ChunkStatus::Light);
            }
            if target == ChunkStatus::Full as u8
                && stage >= Stage::LightLocalReady as u8
                && self.neighbors_ready(*pos)
            {
                handle.advance_status(ChunkStatus::Full);
            }
        }
    }

    /// 触控效果注入：视角、槽位、放置、移动方向、跳跃、挖掘。
    /// 仅在收到过触摸事件后生效，桌面键盘行为不受影响。
    fn apply_touch_input(&mut self) {
        if !self.touch.enabled {
            return;
        }
        let fx = self.touch.consume();
        self.look(fx.look.0 as f64 * 0.4, fx.look.1 as f64 * 0.4);
        if let Some(slot) = fx.slot {
            self.player.sel_slot = slot;
        }
        if fx.place {
            self.interact(true);
        }
        // 放置键沿（与 jump 同一双向纪律）：按住置真、松开沿置假——进食
        // 的按住推进依赖 input.placing 持续为真，仅按沿触发会让进食下一拍
        // 即被取消；触摸启用后镜像，不覆盖桌面鼠标按下态（同 mine 注记）。
        if self.touch.place_held {
            self.input.placing = true;
        } else if fx.place_released {
            self.input.placing = false;
        }
        // 摇杆 → 移动方向
        if let Some((dx, dy)) = self.touch.stick_direction() {
            // 摇杆向上推 = 前进
            self.input.forward = dy < -0.3;
            self.input.back = dy > 0.3;
            self.input.left = dx < -0.3;
            self.input.right = dx > 0.3;
            // 冲刺门 = 摇杆物理偏移 >85% 半径（stick_vec 是像素、上限
            // STICK_R）。旧实现把 stick_direction 归一化后的单位向量再求模
            // （恒 1.0）与 0.85 比较 → 死区外任何轻推都恒冲刺。
            self.input.sprint = self.touch.stick_sprint();
            // 移动量随偏移幅度缩放（Bedrock 式模拟摇杆；键盘恒 1.0）。
            self.input.analog = self.touch.stick_analog();
        } else if self.touch.stick_vec == (0.0, 0.0) {
            self.input.forward = false;
            self.input.back = false;
            self.input.left = false;
            self.input.right = false;
            self.input.sprint = false;
            // 松杆恢复键盘全速比例（否则残留上一杆的亚单位幅度）。
            self.input.analog = 1.0;
        }
        // 跳跃键沿镜像：只写 true 会让触摸松开后 jump 永远悬真（无键盘来
        // 清零）→ 落地自动连跳/飞行中永久上升；无条件镜像又会在「触摸启用
        // 过的桌面」压掉键盘按住态。按沿双向：按住置真，松开沿置假。
        if self.touch.jump_held {
            self.input.jump = true;
        } else if fx.jump_released {
            self.input.jump = false;
        }
        // 触摸挖掘：按下 = 攻击/开始挖，松开 = STOP 补判（与桌面鼠标同一入口）。
        // 仅在触摸启用后镜像，避免清掉桌面鼠标按下的 mining 状态。
        if self.touch.enabled {
            let held = self.touch.mine_held;
            let was = self.input.mining;
            self.input.mining = held;
            if held && !was {
                self.on_left_press();
            } else if !held && was {
                self.on_left_release();
            }
        }
    }

    pub fn fixed_step(&mut self, dt: f32) {
        // E0 帧预算分段计时：fixed_step 总窗口起点（纯记账，见
        // [`FrameTiming`]；一帧 n 步在帧窗内逐笔累加）。
        let step_t0 = std::time::Instant::now();
        // ---- 20 Hz tick 基建（原版 tick 语义的唯一换算点）----
        let n = accumulate_ticks(&mut self.tick_frac, dt);
        self.on_tick = n > 0;
        if self.on_tick {
            self.game_ticks += n;
            self.time_ticks += n; // 26.1 ServerClockManager 每 tick +1
            // 天气推进（ServerLevel.advanceWeatherCycle:694-755）。
            self.weather.tick(&mut spawn_rng());
            // 熔炉会话推进（26.1 TickingBlockEntity 每 tick 一次；单人开屏
            // 服务端不停——Screen 非 null 不停服）。n>1 补齐跨步亏欠。
            if let Some((_, f)) = &mut self.furnace {
                f.tick(n as u32);
            }
            // 粒子推进（26.1 ParticleEngine.tick 每 game tick 一次；
            // 世界回调仅借 chunks，与 &mut particles 字段互斥无冲突）。
            let pw = ParticleRt {
                chunks: &self.chunks,
            };
            self.particles.tick(&pw);
            // 雨粒子：tick 后按天气强度在世界里补原版 RAIN 粒子
            // （原版同为 tick 尾客户端生成，LevelRenderer.java:1164）。
            self.spawn_rain_particles();
            // ---- 生长 tick（#146：树苗→树 + 作物 age；ServerLevel.tickChunk
            // 的 random tick 驱动，唯一调用点）----
            self.growth_tick();
            // ---- 行走 bob 状态（26.1 LocalPlayer.aiStep → updateBob 链）----
            // LocalPlayer.java:989 walkDist += Mth.length(Δx,Δz)·0.6（每 tick）；
            // AbstractClientPlayer.java:84-87 tBob = min(0.1, |Δv|水平)，
            // 门 = onGround && !dying && !swimming（本仓无游泳态，登记
            // KNOWN-DIVERGENCE；rideTick resetBob :70-73 无骑乘）；
            // ClientAvatarState.java:76-78 bobO=bob、bob += (tBob−bob)·0.4。
            // 只在游玩态累计：加载期出生投放的位移不进 walkDist（原版玩家
            // join 后才 tick 计量，本仓加载期也在 tick——转入 Playing 时
            // 由 update_load_gate 重置基准）。
            if self.phase == GamePhase::Playing {
                self.walk_dist_o = self.walk_dist;
                self.bob_o = self.bob;
                let moved = self.player.pos - self.bob_prev_pos;
                self.walk_dist += (moved.x * moved.x + moved.z * moved.z).sqrt() * 0.6;
                self.bob_prev_pos = self.player.pos;
                let t_bob = if self.player.on_ground && !self.dead {
                    (self.player.vel.x * self.player.vel.x + self.player.vel.z * self.player.vel.z)
                        .sqrt()
                        / 20.0
                } else {
                    0.0
                };
                // vel 是 m/s，原版 deltaMovement 是格/tick：/20 换算（满速走
                // 4.317/20 = 0.216 ≥ 0.1 → 封顶）。
                self.bob += (t_bob.min(0.1) - self.bob) * 0.4;
            }
        }
        if self.phase == GamePhase::Loading {
            // 加载态 = 26.1 LevelLoadingScreen 盖在游戏上（Screen 非 null）：
            // 移动/跳跃等输入不生效、触屏摇杆不接入（apply_touch_input 仅
            // Playing 接线）；触摸事件队列仍要排空，防止积压的放置/挖掘
            // 按下效果在进场瞬间连发。
            self.update_load_gate();
            let _ = self.touch.consume();
            self.input = Default::default();
        } else {
            self.apply_touch_input();
        }
        if self.dead {
            // 死亡界面：尸体不响应输入，仅重力继续
            self.input = Default::default();
        }
        // ---- 双击跳 = 切换创造飞行（26.1 LocalPlayer.aiStep:827-848）----
        // abilities.mayfly 门 → 本仓映射为创造模式；按下沿 + 7 tick 窗口
        // （jumpTriggerTime，LocalPlayer.java:835-836；Player.aiStep:443-445
        // 每 tick −1，这里在 on_tick 递减）。起飞瞬间在地面则同款
        // jumpFromGround（LocalPlayer.java:839-841）。加载/死亡态输入已清，
        // 无按下沿，天然被门住。
        if self.on_tick {
            self.flight_jump_trigger = (self.flight_jump_trigger - n as f32).max(0.0);
        }
        let jump_down = self.input.jump;
        if jump_down && !self.jump_held_prev && self.mode == GameMode::Creative {
            if self.flight_jump_trigger > 0.0 {
                self.player.flying = !self.player.flying;
                self.flight_jump_trigger = 0.0;
                if self.player.flying && self.player.on_ground {
                    self.player.vel.y = mcv_game::consts::JUMP_SPEED;
                }
            } else {
                self.flight_jump_trigger = 7.0;
            }
        }
        self.jump_held_prev = jump_down;
        // 攻击冷却 ticker：tick 单位（26.1 Player.java:267 每 tick +1；消费侧
        // combat::attack_strength 的 delay = 20/attackSpeed tick，Player.java:
        // 1793-1795）。旧实现按秒累加又被当 tick 消费，铁剑满蓄力 12.5s（正确
        // 12.5 tick = 0.625s）。上限 20 tick（最慢武器 attackSpeed 1.0 → 蓄满）。
        if self.on_tick {
            self.attack_ticker = (self.attack_ticker + n as f32).min(20.0);
        }
        // 挥臂计时（LivingEntity.aiStep :2149-2158 的连续制等价）：60 Hz 步
        // 按 dt 折算 tick 平滑推进（渲染帧间不跳变）；满一轮归零停摆。
        if self.swinging {
            self.swing_time += dt * 20.0;
            if self.swing_time >= SWING_TICKS {
                self.swinging = false;
                self.swing_time = 0.0;
            }
        }
        self.step_mining(dt);

        // ---- natural spawning (budgeted every 20 ticks) ----
        // 预算按 20 Hz tick 走，不按 60 Hz 步——旧写法每 20 步 = 1/3 s 刷一
        // 轮，节奏 3× 过快（审计 C-1；轮次密度 vs 原版每 tick 一轮为既有
        // KNOWN-DIVERGENCE M-3）。
        if self.on_tick {
            self.spawn_cooldown = self.spawn_cooldown.saturating_sub(1);
            if self.spawn_cooldown == 0 {
                self.spawn_cooldown = 20;
                self.try_natural_spawn();
            }
        }

        // ---- mob AI + physics（ECS 调度：固定步驱动一次 Fixed 阶段）----
        // 系统上下文借自 mobs_app 各字段（disjoint 借用）；区块表克隆与
        // 玩家位姿每步 insert 进 Resources 快照（Arc 计数级克隆，体素数据
        // 共享）。近战命中走事件（信号语义）：run_stage 阶段末排空命令、
        // 翻转事件之后，GameRuntime 统一结算成玩家伤害。
        {
            // E0：ECS Fixed 阶段单列（含两次整表快照 insert + run_stage）。
            let ecs_t0 = std::time::Instant::now();
            let mcv_ecs::App {
                world,
                resources,
                events,
                commands,
                schedule,
            } = &mut self.mobs_app;
            resources.insert(MobServices {
                chunks: self.chunks.clone(),
                player_pos: self.player.pos,
                on_tick: self.on_tick,
                game_ticks: self.game_ticks,
                monsters_burn: monsters_burn(self.time_ticks),
                // 天气修正（WeatherAttributes SKY_LIGHT_LEVEL 混合）。
                sky_darken: self.weather.sky_darken(sky_darken(self.time_ticks)),
                ticks_step: n.min(u32::MAX as u64) as u32,
                difficulty: self.difficulty.id(),
                creative: self.mode == GameMode::Creative,
                // 实体门输入：模拟票级别视图快照（DistanceManager.java:136-138）。
                tickets: self.tickets.clone(),
            });
            // 掉落物系统同快照（Arc 计数级克隆）+ 玩家位姿（拾取判定）。
            resources.insert(mcv_entity::DropWorld {
                chunks: self.chunks.clone(),
                player_pos: self.player.pos,
            });
            let mut ctx = mcv_ecs::SysCtx {
                world,
                resources,
                events,
                commands,
            };
            schedule.run_stage(mcv_ecs::Stage::Fixed, &mut ctx);
            self.timing_frame.ecs_us += ecs_t0.elapsed().as_micros() as u64;
        }
        let hits: Vec<MobMeleeHit> = self.mobs_app.events.channel::<MobMeleeHit>().take();
        for h in hits {
            self.hurt_player(h.damage.max(1.0), Some(h.src));
            // 受击暴击十字（DamageIndicator 变体，CritParticle.java:26-47
            // 经 CombatTracker 触发；胸口高度 +1.0，纯指示无初速）。
            self.particles.spawn_crit(
                h.src.x as f64,
                (h.src.y + 1.0) as f64,
                h.src.z as f64,
                0.0,
                0.0,
                0.0,
                true,
            );
        }
        let arrows: Vec<MobArrowHit> = self.mobs_app.events.channel::<MobArrowHit>().take();
        for h in arrows {
            self.hurt_player(h.damage.max(1.0), Some(h.src));
        }
        self.settle_player_arrow_hits();
        let blasts: Vec<MobExplosionHit> = self.mobs_app.events.channel::<MobExplosionHit>().take();
        for b in blasts {
            // ExplosionDamageCalculator 不在反编译树（NOTES-mobs.md:94，无法
            // 逐值核对）：exposure=1.0（未做方块遮挡逐点采样 →
            // KNOWN-DIVERGENCE），距离衰减曲线用 ai::explosion_damage
            // （((p²+p)/2)·7·2R+1，26.1 已知曲线，ai.rs:77-84）。
            let pc = self.player.pos + Vec3::new(0.0, 1.0, 0.0);
            let dmg = mcv_entity::ai::explosion_damage((pc - b.center).length(), b.radius, 1.0);
            if dmg > 0.0 {
                self.hurt_player(dmg, Some(b.center));
            }
        }
        // 掉落物拾取结算：入栏走 Hotbar::add（give 路径唯一），满栏剩余留地。
        // 拾取确认音（26.1 客户端 take 实体包处理 ClientPacketListener.java:
        // 1002-1013：ITEM_PICKUP、volume 0.2、pitch (rand−rand)×1.4 + 2.0，
        // 位置 = 被拾实体）。结算内部会 despawn 满收实体——先经读侧
        // `read()` 非消费快照候选（含实体句柄/位姿/没收前 count；同 tick 已
        // 被合并/到期的实体在此处 get_ref 即 None，与 settle 的跳过门一致），
        // 结算后按「收走即响」过滤：满收 despawn → get_ref None；部分入栏 →
        // count 变小（对应原版部分拾取也发 take 包）。
        let pickup_reqs: Vec<PickupReq> = self
            .mobs_app
            .events
            .channel::<PickupReq>()
            .read()
            .copied()
            .collect();
        let mut pickup_hits: Vec<(mcv_ecs::Entity, [f32; 3], u8)> = Vec::new();
        for r in pickup_reqs {
            let Some(d) = self.mobs_app.world.get_ref::<ItemDrop>(r.e).map(|d| *d) else {
                continue;
            };
            if d.count == 0 {
                continue;
            }
            let Some(b) = self.mobs_app.world.get_ref::<PhysBody>(r.e).map(|b| *b) else {
                continue;
            };
            pickup_hits.push((r.e, [b.pos.x, b.pos.y, b.pos.z], d.count));
        }
        mcv_entity::settle_pickups(
            &mut self.mobs_app.world,
            &mut self.mobs_app.events,
            &mut self.hotbar,
            self.player.sel_slot,
        );
        for (e, pos, pre_count) in pickup_hits {
            let collected = self
                .mobs_app
                .world
                .get_ref::<ItemDrop>(e)
                .is_none_or(|d| d.count < pre_count);
            if !collected {
                continue;
            }
            let r1 = (fast_rand() % 1024) as f32 / 1024.0;
            let r2 = (fast_rand() % 1024) as f32 / 1024.0;
            let listener = [self.player.pos.x, self.player.pos.y, self.player.pos.z];
            self.audio.play_event_pitched(
                mcv_audio::events::ITEM_PICKUP,
                pos,
                listener,
                0.2,
                2.0 + (r1 - r2) * 1.4,
            );
        }

        // ---- 玩家物理（mcv_game::step，60 Hz 固定步）----
        // 出生投放前暂停（pos=ZERO 是「待投放」哨兵，26.1 里玩家实体在
        // 区块就绪前不存在于客户端）：ZERO 坐在实体柱里会被去穿透乱推，
        // 投放路径（stream 出生投放块）会整体覆写 pos，期间步进纯浪费。
        let awaiting_spawn = !self.spawned && self.player.pos == Vec3::ZERO;
        // ---- 模拟区门（fix/stream-collision：玩家永不走入未加载区）----
        // 原版不变量：玩家区块恒持 PLAYER_SIMULATION ticket（DistanceManager
        // .java:110-117，ChunkMap.move :1071-1096 随区块迁移），物理恒跑在
        // 已加载真体素上；界外列查询得 VOID_AIR（Level.java:361-363）而非
        // 石头。本仓生成异步，用「门 + 钳制」实现同一不变量：玩家所在
        // 区块 status 未达 Full（刀 4a 谓词重接：FULL 级证据链 =
        // worldgen commit + 邻域光照齐，ChunkLevel.java:35-43；超时放行
        // 前沿/重生后 IO 未回）则整步冻结（原版：玩家实体在区块就绪前
        // 不参与物理），就绪后步末再钳回已就位矩形（clamp_to_sim_area，
        // 见流式分层节注释）。
        let pc = ChunkPos::new(
            (self.player.pos.x / 16.0).floor() as i32,
            (self.player.pos.z / 16.0).floor() as i32,
        );
        // 投放等待期不必算安全半径（本块必然未就位，且省掉生成高峰期的
        // 每步环扫描）。
        let r_safe = if awaiting_spawn {
            -1
        } else {
            self.sim_safe_radius(pc)
        };
        if !awaiting_spawn && r_safe >= 0 {
            let look = self.camera(1.0).dir();
            let f = Vec3::new(look.x, 0.0, look.z)
                .try_normalize()
                .unwrap_or(Vec3::new(0.0, 0.0, -1.0));
            let r = f.cross(Vec3::Y);
            // 冲刺饥饿门（26.1 LocalPlayer.java:1133 → Player
            // .hasEnoughFoodToDoExhaustiveManoeuvres = food>6 || mayfly，
            // Player.java:1569-1571；SPRINT_LEVEL=6，FoodConstants.java:12）：
            // 饥饿 ≤6 自动退出冲刺（不提速、不计冲刺 exhaustion）。
            let sprinting = self.input.sprint && (self.player.hunger > 6.0 || self.player.flying);
            let i = &self.input;
            let mut wish = Vec3::ZERO;
            if i.forward {
                wish += f;
            }
            if i.back {
                wish -= f;
            }
            if i.right {
                wish += r;
            }
            if i.left {
                wish -= r;
            }
            // 模拟量随输入幅度缩放（触屏摇杆 0..1，键盘 1.0）；>1 的键盘
            // 斜向合成由 step 内按原版 getInputVector 规则归一化
            // （Entity.java:1677：模长 >1 才归一化）。
            // 速度效果修饰（急速 +0.2×(amp+1)、迟缓 −0.15×(amp+1)，
            // MobEffects.java:15-28 ADD_MULTIPLIED_TOTAL）。KNOWN-DIVERGENCE：
            // physics.rs（冻结域）对 |wish|>1 归一化 → 键盘满输入下急速的
            // 放大被钳回 1；迟缓与亚单位模拟量输入正确。根治需 StepInput
            // 速度倍率缝（physics 域解冻后补）。
            let wish_dir = wish * self.input.analog * self.effects.bundle().move_speed_mult as f32;
            let was_air = !self.player.on_ground;
            let jumped_off = i.jump && self.player.on_ground;
            let before = self.player.pos;
            let in_water = self.in_water(&WorldView {
                chunks: &self.chunks,
            });
            // 入水沿：原版 Entity.updateFluidInteraction（Entity.java:1570
            // -1574）——wasTouchingWater 假→真瞬间 resetFallDistance +
            // doWaterSplashEffect。摔落豁免由下方 fall_y 清空承担。水花按
            // doWaterSplashEffect（Entity.java:1596-1622）：音量 = speed =
            // min(1, sqrt(vx²·0.2+vy²+vz²·0.2)·0.2)（自身控制 volumeModifier
            // =0.2，:1597-1601）；speed<0.25 → entity.player.splash，否则
            // high_speed 分档（:1602-1607 + Player.java:382-390）；音高 =
            // 1.0+(rand−rand)·0.4（:1608，同型随机式 :1408）。event 缺素材时
            // 静默 no-op。
            let entered_water = in_water && !self.was_in_water;
            if entered_water {
                let v = self.player.vel;
                let speed = (v.x * v.x * 0.2 + v.y * v.y + v.z * v.z * 0.2)
                    .sqrt()
                    .min(1.0)
                    * 0.2;
                let event = if speed < 0.25 {
                    mcv_audio::events::PLAYER_SPLASH
                } else {
                    mcv_audio::events::PLAYER_SPLASH_HIGH_SPEED
                };
                let r1 = (fast_rand() % 1024) as f32 / 1024.0;
                let r2 = (fast_rand() % 1024) as f32 / 1024.0;
                let p = self.player.pos;
                self.audio.play_event_pitched(
                    event,
                    [p.x, p.y + 0.5, p.z],
                    [p.x, p.y, p.z],
                    speed,
                    1.0 + (r1 - r2) * 0.4,
                );
                // 溅水两波粒子（Entity.doWaterSplashEffect Entity.java:1594-1622：
                // 波 1+width·20 粒，width=0.6）。
                self.particles.spawn_water_splash(
                    p.x as f64,
                    (p.y + 0.5) as f64,
                    p.z as f64,
                    [
                        self.player.vel.x as f64,
                        self.player.vel.y as f64,
                        self.player.vel.z as f64,
                    ],
                    0.6,
                );
            }
            self.was_in_water = in_water;
            // 攀附中免摔落账（原版 handleOnClimbable 每拍 resetFallDistance，
            // LivingEntity.java:2644）——谓词与 physics::step 同源
            // on_climbable（贴面即达，见 LADDER_PROBE 注）。
            let on_ladder = !self.player.flying
                && mcv_game::physics::on_climbable(
                    &WorldView {
                        chunks: &self.chunks,
                    },
                    &mcv_game::Aabb::from_player(self.player.pos),
                );
            // 空中累计最高点（MC fallDistance：上升不计，下落距离 = 最高点到落点）
            if !self.player.flying && !in_water && !on_ladder {
                if self.player.on_ground {
                    self.fall_y = None;
                } else {
                    let y = self.player.pos.y;
                    self.fall_y = Some(self.fall_y.map_or(y, |f| f.max(y)));
                }
            } else {
                self.fall_y = None; // 飞行/游泳/攀附免疫摔落
            }
            let step_input = mcv_game::StepInput {
                wish_dir,
                jump: i.jump,
                in_water,
                sneak: i.sneak,
                // 冲刺提速 4.317→5.612 m/s（LivingEntity.java:156-158 +30%）；
                // 潜行在 step 内优先于冲刺（蹲下即退冲刺）。
                sprint: sprinting,
                // 冲刺跳的水平增补沿 **yaw 朝向**（LivingEntity.java:2349-2351
                // 只用 yaw，与俯仰无关）；冲刺游泳的竖直转向用俯仰分量
                // （Player.java:1383-1392）。水平=f、竖直=look.y 的合成。
                look_dir: Vec3::new(f.x, look.y, f.z),
                gravity_scale: 1.0,
            };
            mcv_game::step(
                &WorldView {
                    chunks: &self.chunks,
                },
                &mut self.player,
                &step_input,
            );
            // 跳跃提升的跳跃初速加成（LivingEntity.getJumpBoostPower:2340：
            // +0.1 块/tick ×(amp+1) = +2.0×(amp+1) m/s，叠在 JUMP_STRENGTH
            // 0.42 之上）。physics.rs（冻结域）内部置 JUMP_SPEED → 落地跳后
            // 补加；vel.y>0 门防顶头跳（天花板钳零）被再抬升。水中无
            // jumpFromGround（LivingEntity aiStep 流体分支）。
            if jumped_off
                && !in_water
                && let Some(a) = self.effects.amplifier(mcv_entity::Kind::JumpBoost)
                && self.player.vel.y > 0.0
            {
                self.player.vel.y += 2.0 * (f32::from(a) + 1.0);
            }
            // 步末钳制：一步最大位移（≤0.4 格）≪ 钳制余量（≥1 整区块），
            // 常态不可达；只有生成/IO 掉队时才把玩家钉在已就位区边缘
            // （替代旧「隐形石墙」）。
            self.clamp_to_sim_area(pc, r_safe);
            // ---- 行为音效：脚步 / 落地 ----
            let delta = self.player.pos - before;
            // exhaustion 只认水平位移（ServerPlayer.checkMovementStatistics:1444
            // 用 sqrt(dx²+dz²)，垂直不计）。
            let moved_h = Vec3::new(delta.x, 0.0, delta.z).length();
            // 步距累计：原版 moveDist 只加水平位移 × 0.6（Entity.java:812-813
            // horizontalMovedDistance = horizontalDistance() * 0.6F）。
            self.step_dist += moved_h * 0.6;
            if self.player.on_ground && was_air {
                // 摔落伤害（MC: damage = floor(fallDistance - 3)）；安全坠落
                // 距离 = 基础 3 + 跳跃提升 +1×(amp+1)（MobEffects.java:48-52
                // SAFE_FALL_DISTANCE ADD_VALUE；LivingEntity.java:1824
                // fallPower − SAFE_FALL_DISTANCE）。
                if let Some(top) = self.fall_y.take() {
                    let safe = 3.0 + self.effects.bundle().safe_fall_add as f32;
                    let dmg = (top - self.player.pos.y - safe).floor().max(0.0);
                    if dmg > 0.0 {
                        // 摔落伤害音（LivingEntity.java:1770-1772：落地伤害>0
                        // 才播 getFallDamageSound，1.0/1.0；分档 :1644
                        // dmg>4 → big_fall 否则 small_fall）。
                        let p = self.player.pos;
                        let event = if dmg > 4.0 {
                            mcv_audio::events::PLAYER_FALL_BIG
                        } else {
                            mcv_audio::events::PLAYER_FALL_SMALL
                        };
                        self.audio
                            .play_event(event, [p.x, p.y, p.z], [p.x, p.y, p.z], 1.0);
                        // 方块摔落音（LivingEntity.playBlockFallSound :1831-1840：
                        // 脚下 y−0.2 取非空方块，fall 音 volume×0.5、pitch×0.75）。
                        let under = WorldView {
                            chunks: &self.chunks,
                        }
                        .block(BlockPos::new(
                            p.x.floor() as i32,
                            (p.y - 0.2).floor() as i32,
                            p.z.floor() as i32,
                        ))
                        .0;
                        if under != 0 {
                            let st = sound_type_of(under);
                            self.audio.play_event_pitched(
                                st.fall,
                                [p.x, p.y, p.z],
                                [p.x, p.y, p.z],
                                st.volume * 0.5,
                                st.pitch * 0.75,
                            );
                        }
                        self.hurt_player(dmg, None);
                    }
                }
            } else if self.player.on_ground {
                self.fall_y = None;
            }
            // 走路脚步：步距门（Entity.java:820 moveDist > nextStep）触发后
            // 阈值 = (int)moveDist + 1（:1188-1190）；支撑格非空才发声（同
            // :820 !supportingState.isAir()）。on_ground 门为本仓姿态位
            // （原版无显式落地判定，支撑格判定近似）。
            if self.player.on_ground
                && let Some(ns) = step_gate(self.step_dist, self.next_step)
            {
                self.next_step = ns;
                let p = self.player.pos;
                let under = WorldView {
                    chunks: &self.chunks,
                }
                // floor 取整（coords 审计 P3：`as i32` 向零截断在负坐标
                // 取错方块；全仓其余取整点均为 floor）。
                .block(BlockPos::new(
                    p.x.floor() as i32,
                    (p.y - 0.5).floor() as i32,
                    p.z.floor() as i32,
                ))
                .0;
                if under != 0 {
                    let st = sound_type_of(under);
                    // 步音 volume = SoundType.volume × 0.15（Entity.java:1391
                    // getVolume() * 0.15F）；音高 = SoundType.pitch × [0.8,1.2)
                    // 随机（派单规格；26.1 同型随机式 Entity.java:1408 /
                    // LocalPlayer.java:954）。
                    let jitter = 0.8 + (fast_rand() % 1024) as f32 / 1024.0 * 0.4;
                    self.audio.play_event_pitched(
                        st.step,
                        [p.x, p.y, p.z],
                        [p.x, p.y, p.z],
                        st.volume * 0.15,
                        st.pitch * jitter,
                    );
                }
            }
            // ---- 生存统计（26.1 LivingEntity/ServerPlayer/FoodData）----
            // 无敌帧递减：tick 单位（ServerPlayer.java:576-577 每 tick −1），
            // 旧实现按 60 Hz 步递减使 20 tick i 帧只剩 0.33s。
            if self.on_tick {
                self.player.invulnerable = self.player.invulnerable.saturating_sub(1);
            }
            // 事件式 exhaustion 累加（原版在移动/跳跃事件即时加，非每 tick）：
            // 水中 0.01/m——游泳/眼下水按 3D 距离、水面行进按水平距离
            // （ServerPlayer.checkMovementStatistics:1422-1439 +
            // FoodConstants.java:28 EXHAUSTION_SWIM），优先序 isSwimming >
            // eyeInFluid(WATER) > inWater > onGround 与原版 else-if 链一致
            // （self.swimming 为上一拍值，姿态位本拍尾才翻转，1 tick 滞后）。
            // 地面冲刺 0.1/m、走路/潜行 0.0/m 且只计水平分量
            // （checkMovementStatistics:1443-1456 + FoodConstants.java:25-27，
            // 旧实现水中零消耗为登记差异，本提交消解）；跳跃 = 冲刺跳 0.2 /
            // 普通跳 0.05（ServerPlayer.jumpFromGround:1532-1540 +
            // FoodConstants.java:21-22，旧实现恒 0.2 高估普通跳）。
            if self.mode != GameMode::Creative {
                // 眼位水样（eyeInFluid(WATER) 分支判据，:1428）。
                let eyes_in_water = {
                    let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
                    let ec = eye.floor().as_ivec3();
                    let view = WorldView {
                        chunks: &self.chunks,
                    };
                    let d = view.block(BlockPos::new(ec.x, ec.y, ec.z)).def();
                    d.liquid && d.name == "water"
                };
                let p = &mut self.player;
                let moved_3d = delta.length();
                let water_cost = if self.swimming || eyes_in_water {
                    0.01 * moved_3d
                } else if in_water {
                    0.01 * moved_h
                } else {
                    0.0
                };
                if water_cost > 0.0 {
                    p.exhaustion = (p.exhaustion + water_cost).min(EXHAUSTION_MAX);
                } else if p.on_ground {
                    p.exhaustion =
                        (p.exhaustion + move_exhaustion(sprinting, moved_h)).min(EXHAUSTION_MAX);
                }
                if jumped_off {
                    p.exhaustion =
                        (p.exhaustion + if sprinting { 0.2 } else { 0.05 }).min(EXHAUSTION_MAX);
                }
            }
            // FoodData.tick 每 game tick 一次（26.1 FoodData.java:32-72）：
            // exhaustion>4 先扣 1 饱和、饱和耗尽才扣饥饿；回血/饥饿掉血走
            // tickTimer（和平封顶 10 为本仓既有登记偏差）。饿死拍命中时由
            // 调用方走完整 hurt 管线（FoodData.java:64 hurtServer(starve)，
            // i 帧门/死亡结算/受伤音照常；exhaustion 按 starve.json=0.0）。
            if self.on_tick && self.mode != GameMode::Creative {
                let p = &mut self.player;
                let starve = food_data_tick(
                    &mut p.exhaustion,
                    &mut p.saturation,
                    &mut p.hunger,
                    &mut p.health,
                    &mut self.food_tick_timer,
                    self.difficulty,
                );
                if starve {
                    self.hurt_player(1.0, None);
                }
            }
            // ---- 游泳姿态 + 空气/溺水（每 game tick，20 Hz）----
            if self.on_tick {
                // 眼位流体（原版 isEyeInFluid(**WATER**)，LivingEntity.java:417
                // 只认水不认岩浆；eye_in_water 是挖掘惩罚用的“任意流体”版，
                // 语义不同不能复用）。
                let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
                let ec = eye.floor().as_ivec3();
                let feet_pos = BlockPos::new(
                    self.player.pos.x.floor() as i32,
                    self.player.pos.y.floor() as i32,
                    self.player.pos.z.floor() as i32,
                );
                // 借用隔离：view 的不可变借用只在取数块内存活（旧写法 view
                // 活满全段，与下方 hurt_*/particles 可变借用冲突 = E0502，
                // 前任提交从未绿过 CI）。BlockId::def() 返回 &'static
                // BlockDef，格定义可安全带出借用域。
                let (eyes_water, feet_water, in_lava, fire_dmg, eye_suffocate) = {
                    let view = WorldView {
                        chunks: &self.chunks,
                    };
                    let eye_def = view.block(BlockPos::new(ec.x, ec.y, ec.z)).def();
                    let eyes_water = eye_def.liquid && eye_def.name == "water";
                    let feet_def = view.block(feet_pos).def();
                    let feet_water = feet_def.liquid && feet_def.name == "water";
                    // ---- 方块接触伤害（26.1 InsideBlockEffectApplier：
                    // 与实体 AABB 重叠的每格触发 entityInside）----
                    // 岩浆（LavaFluid.entityInside:119-123）：CLEAR_FREEZE +
                    // LAVA_IGNITE（Entity.lavaIgnite:607-611 点燃 15s）+
                    // Entity::lavaHurt:613-624 → lava() 4.0F/tick（i 帧节流成
                    // 4.0/s）。判据按 AABB 与岩浆格任一重叠近似为「脚部或眼部
                    // 格是岩浆」（本引擎单点采样脚/眼，1.8 m 身高横跨 ≤3 格，
                    // 差异登记 KNOWN-DIVERGENCE）。
                    let in_lava = (feet_def.liquid && feet_def.name == "lava")
                        || (eye_def.liquid && eye_def.name == "lava");
                    // 火焰方块（BaseFireBlock.entityInside:131-137）：FIRE_IGNITE
                    // （fireIgnite:139-155 → igniteForSeconds(8)=160 tick，只增
                    // 不减）+ in_fire() fireDamage/tick（i 帧节流）。数值：
                    // FireBlock 构造 1.0F（FireBlock.java Vineflower 反编译失败，
                    // 按 vanilla 常量；SoulFireBlock.java:22 = 2.0F 实读确认）。
                    // 判据同岩浆：脚/眼格任一是火（KNOWN-DIVERGENCE 单点采样）。
                    let fire_dmg = if feet_def.name == "soul_fire" || eye_def.name == "soul_fire" {
                        Some(2.0)
                    } else if feet_def.name == "fire" || eye_def.name == "fire" {
                        Some(1.0)
                    } else {
                        None
                    };
                    // 窒息（26.1 LivingEntity.baseTick:405-406 isInWall → inWall()
                    // 1.0F/tick，i 帧门自然节流 ~1/s；Entity.isInWall:2164-2182 =
                    // 眼位 0.8×width 窄盒（1e-6 高 → 仅眼位所在 y 层）与
                    // suffocating 方块求交；suffocating 默认判据 =
                    // blocksMotion && 满碰撞立方（BlockBehaviour.java:1004）→
                    // 本仓按 solid 全立方近似）。
                    let eye_suffocate = {
                        let hw = mcv_game::Player::HALF[0] * 0.8;
                        let mut hit = false;
                        for bx in (eye.x - hw).floor() as i32..=(eye.x + hw).floor() as i32 {
                            for bz in (eye.z - hw).floor() as i32..=(eye.z + hw).floor() as i32 {
                                let d = view.block(BlockPos::new(bx, ec.y, bz)).def();
                                hit |= d.solid
                                    && !d.liquid
                                    && mcv_core::Shape::from_u8(d.shape) == mcv_core::Shape::Cube;
                            }
                        }
                        hit
                    };
                    (eyes_water, feet_water, in_lava, fire_dmg, eye_suffocate)
                };
                // 姿态位（Pose.SWIMMING 的驱动源，Entity.java:1558-1564 +
                // Player.java:1410-1416；第三人称 prone 模型接线遗留）。
                self.swimming = swimming_tick(
                    self.swimming,
                    sprinting,
                    in_water,
                    eyes_water && in_water,
                    feet_water,
                    self.player.flying,
                );
                // 空气供给（LivingEntity.java:417-439）：创造 invulnerable
                // 免溺（:422-423），旁观者本仓无。
                let drown = air_supply_tick(
                    &mut self.air_supply,
                    eyes_water,
                    self.mode != GameMode::Creative,
                );
                // waterVisionTime 计时（LocalPlayer.java:858-864）：眼在水中
                // +1/tick、否则 −10/tick，clamp 0..=600。
                self.water_vision_time = if eyes_water {
                    (self.water_vision_time + 1).min(600)
                } else {
                    self.water_vision_time.saturating_sub(10)
                };
                if drown {
                    // broadcastEntityEvent(67) → makeDrownParticles：8 个
                    // BUBBLE（LivingEntity.java:2087-2088/:2113-2123）。
                    // 出生点 = 实体坐标（getY() = 脚底，:2121-2122，偏移
                    // triangle ±1 自行散布全身，无 +0.5 抬升）。
                    let p = self.player.pos;
                    let v = [
                        self.player.vel.x as f64,
                        self.player.vel.y as f64,
                        self.player.vel.z as f64,
                    ];
                    self.particles
                        .spawn_drown_bubbles(p.x as f64, p.y as f64, p.z as f64, v);
                    // damageSources().drown() 2.0F（LivingEntity.java:428）；
                    // 环境伤害不进难度缩放（DamageSource.java:92-97 判据
                    // = 实体伤害，hurt_player 的 from=None 分支同语义）。
                    self.hurt_player(2.0, None);
                }
                if in_lava {
                    // lavaIgnite：igniteForSeconds(15) = 300 tick（只增不减，
                    // igniteForTicks:634-640 `remainingFireTicks < n` 门）。
                    self.fire_ticks = self.fire_ticks.max(15 * 20);
                    // lavaHurt：lava() 4.0F（Entity.java:613-624，含
                    // GENERIC_BURN 音，音效接 event 表后再挂）。
                    self.hurt_fire(4.0, 0.1);
                }
                // 燃烧结算（Entity.baseTick:534-544）：remainingFireTicks>0
                // 且每 20 tick 边界且**不在岩浆**（岩浆侧 lavaHurt 每 tick
                // 独立结算）→ on_fire() 1.0F；随后每 tick −1。
                if self.fire_ticks > 0 {
                    if self.fire_ticks % 20 == 0 && !in_lava {
                        self.hurt_fire(1.0, 0.0);
                    }
                    self.fire_ticks -= 1;
                }
                if let Some(dmg) = fire_dmg {
                    self.fire_ticks = self.fire_ticks.max(8 * 20);
                    self.hurt_fire(dmg, 0.1);
                }
                if eye_suffocate {
                    self.hurt_ex(1.0, None, 0.0, false);
                }
            }
            // ---- 状态效果 tick（26.1 MobEffectInstance.tickServer:223-240，
            // 随 entityTick 每 game tick 一次；与 FoodData.tick 同拍）----
            // 回血/进食/exhaustion/吸收盾就地结算（保持逐效果顺序语义）；
            // 伤害（毒/凋零/瞬间伤害）出队后走 hurt_player 完整管线
            // （无敌帧/吸收先扣/难度——魔法无来源不缩放，与原版一致）。
            // 生命上限加值来自 health_boost（MobEffects.java:78-82）。
            let harms = if self.on_tick {
                let game_ticks = self.game_ticks.min(i32::MAX as u64) as i32;
                let max_hp = 20.0 + self.effects.bundle().max_health_add as f32;
                self.effects.tick(
                    game_ticks,
                    &mut self.player.health,
                    max_hp,
                    &mut self.player.absorption,
                    // FoodData 三元组裸引用（FoodData.java:19-22 的唯一触碰面）。
                    Some((
                        &mut self.player.hunger,
                        &mut self.player.saturation,
                        &mut self.player.exhaustion,
                    )),
                )
            } else {
                Vec::new()
            };
            for h in harms {
                self.hurt_player(h.amount, None);
            }
        }

        // ---- 弓（BowItem.releaseUsing:28-43 蓄力放箭；状态机 mcv_item::bow）----
        // 按住右键蓄力（每 game tick +1）、松开结算；pow<0.1 取消（:38）。
        let has_bow =
            self.hotbar.selected(self.player.sel_slot).def().kind == mcv_item::ItemKind::Bow;
        if let Some(rel) = mcv_item::bow::step_charge(
            self.input.placing,
            self.on_tick,
            has_bow,
            &mut self.bow_hold,
        ) {
            // 弹药门（BowItem.java:30-34 getProjectile 空 → 不放）；创造豁免。
            let arrow_id = mcv_item::item_by_name("arrow");
            let has_ammo = self.mode == GameMode::Creative
                || arrow_id.is_some_and(|id| {
                    self.hotbar
                        .all_slots()
                        .iter()
                        .any(|s| !s.is_empty() && s.item == id)
                });
            if has_ammo {
                if self.mode != GameMode::Creative
                    && let Some(id) = arrow_id
                {
                    // 消耗一支箭（vanilla draw() shrink；就近找格扣 1）。
                    for i in 0..mcv_item::HOTBAR_SLOTS + mcv_item::MAIN_SLOTS {
                        let s = self.hotbar.slot_mut(i);
                        if !s.is_empty() && s.item == id {
                            s.count -= 1;
                            if s.count == 0 {
                                *s = mcv_item::ItemStack::empty();
                            }
                            break;
                        }
                    }
                }
                // shootFromRotation：初速 pow×3.0（BowItem.java:41），
                // pow==1 → 暴击 flag（:41 第 5 参）。
                let dir = self.camera(1.0).dir();
                let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
                let e = self.mobs_app.world.spawn();
                self.mobs_app.world.insert(
                    e,
                    MobArrow {
                        pos: eye + dir * 0.5,
                        vel: dir * mcv_item::bow::release_velocity(rel.power),
                        ttl_ticks: 1200,
                        base_damage: 2.0, // AbstractArrow.java:622 默认。
                        crit: rel.power >= 1.0,
                        player_owned: true,
                    },
                );
                // 放箭音（BowItem.releaseUsing BowItem.java:46-55：ARROW_SHOOT
                // = "entity.arrow.shoot"（SoundEvents.java:111），volume 1.0、
                // pitch = 1.0/(rand×0.4 + 1.2) + pow×0.5；rand 取本仓 fast_rand
                // 1024 档近似，同溅落音 jitter 惯例）。
                let r = (fast_rand() % 1024) as f32 / 1024.0;
                let pitch = 1.0 / (r * 0.4 + 1.2) + rel.power * 0.5;
                self.audio.play_event_pitched(
                    mcv_audio::events::ARROW_SHOOT,
                    [eye.x, eye.y, eye.z],
                    [eye.x, eye.y, eye.z],
                    1.0,
                    pitch,
                );
            }
        }

        // ---- 进食（Consumable 26.1）：按住右键推进、松开/换手取消、
        // 吃满 eat_ticks 结算（结算体 = finish_eating，20Hz tick 语义）----
        if self.on_tick && self.eat_cooldown > 0 {
            self.eat_cooldown -= 1;
        }
        // 按住右键的重触发（原版 Minecraft.handleKeybinds：rightClickDelay
        // 到 0 且 use 键仍按住 → startUseItem；try_begin_eating 内部自带
        // 工作台拦截与可吃门，非食物空转零成本）。
        if self.on_tick && self.input.placing && self.eat_cooldown == 0 && self.eat_hold.is_none() {
            self.try_begin_eating();
        }
        {
            let held = {
                let s = self.hotbar.selected(self.player.sel_slot);
                (!s.is_empty()).then_some(s.item)
            };
            let food = held.and_then(mcv_item::food::food_properties);
            let can_eat = food.is_some_and(|f| self.can_eat_now(f));
            match mcv_item::food::step_eating(
                self.input.placing,
                self.on_tick,
                held,
                food,
                can_eat,
                &mut self.eat_hold,
            ) {
                Some(done) => self.finish_eating(done),
                None => self.eat_chew_tick(food),
            }
        }

        // ---- 虚空（26.1 Entity.checkBelowWorld:579-583 y < getMinY()−64 →
        // onBelowWorld；LivingEntity.onBelowWorld:2142-2144 → fellOutOfWorld
        // **4.0F/tick 走正常 hurtServer 管线**（i 帧门节流成 0.5s/跳，不清
        // 无敌帧）；类型 out_of_world ∈ bypasses_invulnerability
        // （tags/damage_type/bypasses_invulnerability.json）→ 创造不豁免
        // （Entity.isInvulnerableToBase:2955-2960 穿标签），照常死。
        // 旧实现 y<−10 清无敌帧打 40 秒杀且创造无限坠落（软锁），两处均无
        // 源码依据。minY 取世界常数（体素布局 y ∈ 0..CHUNK_SY）。
        if self.player.pos.y < WORLD_MIN_Y - 64.0 {
            self.hurt_ex(4.0, None, 0.0, true);
            if self.dead {
                self.player.pos = Vec3::new(8.5, 200.0, 8.5);
                self.player.vel = Vec3::ZERO;
                self.player.flying = self.mode == GameMode::Creative;
            }
        }
        // E0：fixed_step 总窗口收账（纯记账）。
        self.timing_frame.fixed_step_us += step_t0.elapsed().as_micros() as u64;
    }

    /// 玩家箭命中 mob 结算（`arrow_system` 事件 → 伤害/击退/击杀掉落）。
    /// 伤害路径与 try_attack 相同（combat::apply_hurt + HurtByTarget 输入 +
    /// knockback 0.4 + die → XP/loot）；不重构 try_attack（并行接线纪律），
    /// 此处独立成方法。
    fn settle_player_arrow_hits(&mut self) {
        let hits: Vec<PlayerArrowHitMob> =
            self.mobs_app.events.channel::<PlayerArrowHitMob>().take();
        for h in hits {
            let Some(def) = ({
                let world = &self.mobs_app.world;
                let kind = world.read::<MobKind>();
                kind.get(h.target).map(|k| *k.0.def())
            }) else {
                continue;
            };
            let mut slain = false;
            {
                let world = &mut self.mobs_app.world;
                let mut health = world.write::<Health>();
                let mut ticks = world.write::<MobTicks>();
                let mut last_hurt = world.write::<LastHurt>();
                if let (Some(hp), Some(tk), Some(lh)) = (
                    health.get_mut(h.target),
                    ticks.get_mut(h.target),
                    last_hurt.get_mut(h.target),
                ) {
                    let hurt = combat::apply_hurt(
                        &mut hp.0,
                        &mut tk.invulnerable,
                        &mut lh.0,
                        def.armor,
                        h.damage as f32,
                        0,
                    );
                    if hurt.is_some() {
                        tk.hurt_flag = true; // 受击索敌（HurtByTargetGoal）。
                        slain = hp.0 <= 0.0;
                    }
                }
            }
            // 击退（LivingEntity.java:1238，victim 0.4，同近战）。
            if !slain && let Some(b) = self.mobs_app.world.write::<PhysBody>().get_mut(h.target) {
                let dir = Vec3::new(b.pos.x - h.src.x, 0.0, b.pos.z - h.src.z);
                b.vel = combat::knockback_velocity(b.vel, b.on_ground, 0.0, 0.4, dir);
            }
            if slain {
                let pos = self
                    .mobs_app
                    .world
                    .get_ref::<PhysBody>(h.target)
                    .map(|b| b.pos + Vec3::Y * 0.5)
                    .unwrap_or(h.src);
                self.mobs_app.world.despawn(h.target);
                self.player_xp += def.xp;
                let mut rng = spawn_rng();
                for ev in mcv_entity::death_drops(def.kind, true, pos, &mut rng) {
                    if let Some(item) = mcv_item::item_by_name(ev.item) {
                        mcv_entity::spawn_item_drop(
                            &mut self.mobs_app.world,
                            pos,
                            item,
                            ev.count.min(u8::MAX as u32) as u8,
                            mcv_entity::PICKUP_DELAY,
                            &mut rng,
                        );
                    }
                }
            }
        }
    }

    /// 生长 tick（#146，fixed_step 每 on_tick 调用一次）：树苗→树 +
    /// 作物 age 推进。决策面全部在 [`crate::growth`]（纯函数，引注见该
    /// 模块头注）；本方法只做三件事——ticking 域圈定、扫描、结算写入。
    ///
    /// - **ticking 域**：玩家块 Chebyshev ±2（5×5）且 `stage ≥
    ///   LightLocalReady`（自然刷新同款就绪门，`try_natural_spawn` 先例）。
    ///   原版 random tick 跑在模拟距内全部 ticking 块（ServerLevel.java:
    ///   497-523 遍历 shouldTick 块）→ 本仓取交互半径子集，登记
    ///   KNOWN-DIVERGENCE GROWTH-1 ④。push 序 = dx 升序、dz 升序 =
    ///   (x, z) 字典序（`growth::scan_tick` 流消耗序契约面）。
    /// - **结算**：CropGrow/SaplingAdvance = 单格写（age/stage 变 id 不变 →
    ///   光照与高度图不受影响，只标 MESH|SAVE）；GrowTree = 苗位清 AIR →
    ///   `place_tree`（#135）投影（替换门 [`crate::growth::tree_write_gate`]
    ///   同 `vanilla/tree_pass.rs::ChunkClipper`；跨界裁剪同 TREES-2 ④）→
    ///   逐格 `relight_block_edit`（树影投下后下方作物光照门才能如实收敛）
    ///   + 触达块 MESH|SAVE。
    fn growth_tick(&mut self) {
        if self.phase != GamePhase::Playing {
            return;
        }
        let pc = ChunkPos::new(
            (self.player.pos.x / 16.0).floor() as i32,
            (self.player.pos.z / 16.0).floor() as i32,
        );
        let mut chunks = Vec::with_capacity(25);
        for dx in -2..=2 {
            for dz in -2..=2 {
                let p = ChunkPos::new(pc.x + dx, pc.z + dz);
                let ready = self
                    .chunks
                    .get(&p)
                    .is_some_and(|h| (h.stage() as u8) >= (Stage::LightLocalReady as u8));
                if ready {
                    chunks.push(p);
                }
            }
        }
        if chunks.is_empty() {
            return;
        }
        let view = WorldView {
            chunks: &self.chunks,
        };
        // 原始双通道光照（树苗门 darken 只压天光，LevelLightEngine.java:
        // 146-151；缺块/界外 = 满亮天空，与 WorldView::light 同语义）。
        let light = |p: BlockPos| -> crate::growth::LightCell {
            if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&p.y) {
                return crate::growth::LightCell { sky: 15, block: 0 };
            }
            match self.chunks.get(&p.chunk()) {
                Some(h) if h.stage() != Stage::Empty => {
                    let [lx, _, lz] = p.local();
                    let v = h.light.read().unwrap()[mcv_core::vidx(lx, p.y, lz)];
                    crate::growth::LightCell {
                        sky: v >> 4,
                        block: v & 0xF,
                    }
                }
                _ => crate::growth::LightCell { sky: 15, block: 0 },
            }
        };
        let darken = self.weather.sky_darken(sky_darken(self.time_ticks));
        let decisions = crate::growth::scan_tick(
            &chunks,
            self.seed,
            self.game_ticks,
            darken,
            &|p| view.block(p),
            &light,
        );
        for (pos, g) in decisions {
            match g {
                crate::growth::Growth::None => {}
                crate::growth::Growth::CropGrow { new_age } => {
                    if let Some(h) = self.chunks.get(&pos.chunk()) {
                        let [lx, _, lz] = pos.local();
                        let idx = mcv_core::vidx(lx, pos.y, lz);
                        let mut v = h.voxels.write().unwrap();
                        v[idx] = crate::growth::with_crop_age(v[idx], new_age);
                        drop(v);
                        h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
                    }
                }
                crate::growth::Growth::SaplingAdvance => {
                    if let Some(h) = self.chunks.get(&pos.chunk()) {
                        let [lx, _, lz] = pos.local();
                        let idx = mcv_core::vidx(lx, pos.y, lz);
                        let mut v = h.voxels.write().unwrap();
                        v[idx] = crate::growth::with_sapling_stage(v[idx], 1);
                        drop(v);
                        h.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
                    }
                }
                crate::growth::Growth::GrowTree(kind) => {
                    self.grow_tree_at(pos, kind);
                }
            }
        }
    }

    /// 苗位长树（SaplingBlock.java:55 → TreeGrower.growTree 的本仓结构面）：
    /// 先清苗位（TreeGrower.java:169 `setBlock(pos, emptyBlock, 260)`），
    /// 再 `place_tree` 投影（树基 = 苗原位），逐格增量重光照。
    fn grow_tree_at(&mut self, pos: BlockPos, kind: mcv_worldgen::trees::TreeKind) {
        let base_chunk = pos.chunk();
        let Some(handle) = self.chunks.get(&base_chunk) else {
            return;
        };
        let [lx, _, lz] = pos.local();
        let base_idx = mcv_core::vidx(lx, pos.y, lz);
        // 清苗（growTree 先清位再 place；本仓 place_tree 恒成功 → 无回植
        // 分支，TreeGrower.java:171-175 的失败恢复不适用）。
        let old = handle.voxels.read().unwrap()[base_idx];
        handle.voxels.write().unwrap()[base_idx] = BlockId(0);
        handle.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
        let mut edits: Vec<(BlockPos, u16, u16)> = vec![(pos, old.0, 0)];
        // 树投影写手：跨界裁剪同 TREES-2 ④（苗所属块 16×16 柱内才落格），
        // 替换门 = growth::tree_write_gate（ChunkClipper 同表）。
        struct RuntimeSetter<'a> {
            chunks: &'a HashMap<ChunkPos, Arc<ChunkHandle>>,
            base: ChunkPos,
            edits: &'a mut Vec<(BlockPos, u16, u16)>,
        }
        impl mcv_worldgen::trees::BlockSetter for RuntimeSetter<'_> {
            fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
                let p = BlockPos::new(x, y, z);
                if p.chunk() != self.base
                    || !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&y)
                {
                    return;
                }
                let Some(slot) = crate::growth::tree_slot_of(id) else {
                    return;
                };
                let h = self.chunks.get(&self.base).expect("base chunk held");
                let [lx, _, lz] = p.local();
                let idx = mcv_core::vidx(lx, y, lz);
                let cur = h.voxels.read().unwrap()[idx];
                if !crate::growth::tree_write_gate(slot, cur) {
                    return;
                }
                h.voxels.write().unwrap()[idx] = BlockId(id);
                self.edits.push((p, cur.0, id));
            }
        }
        {
            let mut setter = RuntimeSetter {
                chunks: &self.chunks,
                base: base_chunk,
                edits: &mut edits,
            };
            let mut rng = mcv_worldgen::trees::TreesRng::new(crate::growth::tree_stream_seed(
                self.seed,
                self.game_ticks,
                pos,
            ));
            mcv_worldgen::trees::place_tree(&mut rng, pos.x, pos.y, pos.z, kind, &mut setter);
        }
        // 逐格增量重光照 + 高度图（26.1 setBlock → checkBlock 同位；树影
        // 决定下方作物后续的亮度门，不能只标脏）。源块当帧优先重建网格。
        for (p, o, n) in &edits {
            relight_block_edit(&self.chunks, *p, *o, *n, &mut self.pending_light_edges);
        }
        self.mesh_priority = Some(base_chunk);
    }

    /// NaturalSpawner-lite（预算密度为既有 KNOWN-DIVERGENCE M-3）：每 20
    /// tick 抽 2 个已点亮区块，每块 3 组 × ≤4 步游走。审计接线点：
    /// - 候选 y 在 [0, surface] 均匀随机（NaturalSpawner.java:343-349
    ///   getRandomPosWithin `randomBetweenInclusive(minY, surface+1)`——
    ///   洞穴是主怪源，旧版只刷地表 y；审计 N-2）。
    /// - 亮度三段门 `spawner::is_dark_enough`（Monster.java:78-91）：gate1
    ///   **原始天光** vs rand(32)；gate2 方块光 > 0 拒（overworld limit=0）；
    ///   gate3 `max(sky − skyDarken, block)` ≤ rand(8)（LevelReader.java:167
    ///   -175 + Level.java:736）——旧版 `sky.min(4)` 三道门共用 + gate3 恒
    ///   拒的通道错误已修（审计 M-2）。
    /// - 被动走独立通道（Animal.java:111-120）：脚下草地 + `raw(pos,0) > 8`
    ///   （与敌对暗度门互不复用）+ CREATURE cap 10——旧白天分支恒要求
    ///   surface>130 的死分支已删（审计 N-4）。
    /// - 组内成批（NaturalSpawner.java:197-200 `min + next(1+max−min)`）：
    ///   组类型首只固定，逐只复检；组大小数值在生物群系 spawner JSON（未
    ///   随反编译提取）→ 用库默认 1..=4，KNOWN-DIVERGENCE（审计 N-3）。
    /// - 落地 y = 候选格本身（`mob.snapTo(xx, yStart, zz)`，:211；旧版
    ///   surface+1 悬空 1 格，审计 M-4）。
    /// - 种类池补 spider（审计 M-5；权重 JSON 未提取 → 均匀，
    ///   KNOWN-DIVERGENCE）。
    fn try_natural_spawn(&mut self) {
        if self.chunks.is_empty() {
            return;
        }
        let cap_of = |cat| -> u32 {
            spawner::SPAWN_CAPS
                .iter()
                .find(|(c, ..)| *c == cat)
                .map(|(_, cap, ..)| *cap)
                .unwrap_or(0)
        };
        let (mut monsters, mut animals) = (0u32, 0u32);
        {
            let kinds = self.mobs_app.world.read::<MobKind>();
            for (_, k) in kinds.iter() {
                if k.0.def().hostile {
                    monsters += 1;
                } else {
                    animals += 1;
                }
            }
        }
        let monster_cap = cap_of(spawner::SpawnCategory::Monster);
        let animal_cap = cap_of(spawner::SpawnCategory::Creature);
        if monsters >= monster_cap && animals >= animal_cap {
            return;
        }
        let center = (
            (self.player.pos.x / 16.0).floor() as i32,
            (self.player.pos.z / 16.0).floor() as i32,
        );
        // 和平不刷怪（Monster.checkMonsterSpawnRules 的
        // `getDifficulty() != PEACEFUL`，Monster.java:112）；雷暴把天空亮度
        // 压暗再判（Level.getRawBrightness 的 darken 参数经
        // Weather.sky_darken：雨/雷暴 11/12 级暗化，
        // Monster.isDarkEnoughToSpawn :87 用压暗后的 getBrightness 对比
        // random.nextInt(32)）。
        let cfg = spawner::SpawnConfig {
            peaceful: self.difficulty.is_peaceful(),
            ..Default::default()
        };
        let darken = self.weather.sky_darken(sky_darken(self.time_ticks));
        for _ in 0..2 {
            // random loaded chunk within spawn range
            let dx = (fast_rand() % (2 * spawner::SPAWN_RANGE_CHUNKS as u32 + 1)) as i32
                - spawner::SPAWN_RANGE_CHUNKS;
            let dz = (fast_rand() % (2 * spawner::SPAWN_RANGE_CHUNKS as u32 + 1)) as i32
                - spawner::SPAWN_RANGE_CHUNKS;
            let pos = ChunkPos::new(center.0 + dx, center.1 + dz);
            let Some(handle) = self.chunks.get(&pos) else {
                continue;
            };
            if (handle.stage() as u8) < (Stage::LightLocalReady as u8) {
                continue;
            }
            let start_x = (fast_rand() % 16) as i32 + pos.x * 16;
            let start_z = (fast_rand() % 16) as i32 + pos.z * 16;
            // 组起点柱面的地表 y（yStart 上限）——原版 getRandomPosWithin 用
            // WORLD_SURFACE+1（NaturalSpawner.java:347），此处取起始柱面。
            let start_surface = self.surface_at(start_x, start_z);
            if start_surface == 0 || start_surface > 250 {
                continue;
            }
            let mut rng = spawn_rng();
            for _ in 0..spawner::GROUPS_PER_CHUNK {
                // yStart 组内固定、每步只抖动 x/z（NaturalSpawner.java:169/180
                // -182：`int yStart = start.getY()` 于组前，walk 只改 x/z）——
                // y 均匀 [0, surface]（:343-349 randomBetweenInclusive），多数
                // 步落不到实心面被拒（原版同款低成功率 = 洞穴/地下主怪源）。
                let y_start = (rng() % (start_surface as u32 + 1)) as i32;
                let mut p = glam::Vec3::new(start_x as f32, y_start as f32, start_z as f32);
                // 组内类型首次成功后固定（NaturalSpawner.java:191-200）。
                let mut kind: Option<MobId> = None;
                let mut want = 0usize;
                let mut got = 0usize;
                for _ in 0..spawner::ATTEMPTS_PER_GROUP {
                    if want != 0 && got >= want {
                        break;
                    }
                    p = spawner::group_walk(p, &mut rng);
                    let gx = p.x.floor() as i32;
                    let gz = p.z.floor() as i32;
                    let cell = BlockPos::new(gx, y_start, gz);
                    let Some(h) = self.chunks.get(&cell.chunk()) else {
                        continue;
                    };
                    if (h.stage() as u8) < (Stage::LightLocalReady as u8) {
                        continue;
                    }
                    let center = glam::Vec3::new(gx as f32 + 0.5, y_start as f32, gz as f32 + 0.5);
                    let dist_sqr = (self.player.pos - center).length_squared();
                    // isRightDistanceToPlayerAndSpawnPoint（NaturalSpawner.java
                    // :223-237）：24 内 / 128 外拒（出生点 24 内拒 = respawn
                    // 系统未接线，TODO）。
                    if dist_sqr <= spawner::MIN_PLAYER_DIST_SQR
                        || dist_sqr > spawner::MAX_SPAWN_DIST * spawner::MAX_SPAWN_DIST
                    {
                        continue;
                    }
                    // ON_GROUND：本格 + 头顶非碰撞、下方实心（SpawnPlacements
                    // .ON_GROUND，isValidEmptySpawnBlock 近似）。不中即拒该步
                    // （原版不在任意 yStart 上扫描，低命中率是设计）。
                    if !self.standable(cell) {
                        continue;
                    }
                    let (sky, blk) = chunk_light(&self.chunks, cell);
                    // 敌对通道：三段暗度门（gate1 原始天光，gate3 扣 skyDarken）。
                    let raw = spawner::raw_brightness(sky, blk, darken);
                    if monsters < monster_cap
                        && spawner::is_dark_enough(sky, blk, raw, &cfg, &mut rng)
                    {
                        if kind.is_none() {
                            kind = Some(
                                [
                                    MobId::ZOMBIE,
                                    MobId::SKELETON,
                                    MobId::CREEPER,
                                    MobId::SPIDER,
                                ][(rng() as usize) % 4],
                            );
                            want = cfg.group_min
                                + (rng() as usize) % (cfg.group_max - cfg.group_min + 1);
                        }
                        spawn_mob(&mut self.mobs_app.world, kind.unwrap(), center);
                        monsters += 1;
                        got += 1;
                        continue;
                    }
                    // 被动通道（Animal.java:111-120）：地表草地（tag
                    // ANIMALS_SPAWNABLE_ON 的代表成员 grass_block：本引擎
                    // id 3，含雪覆形态 id 11；tag JSON 未提取）+ 亮处 raw>8
                    // （不扣 skyDarken：原版 getRawBrightness(pos, 0)）。
                    let below = BlockPos::new(cell.x, cell.y - 1, cell.z);
                    let grass = matches!(self.block_at(below).0, 3 | 11);
                    if kind.is_none()
                        && animals < animal_cap
                        && grass
                        && spawner::raw_brightness(sky, blk, 0) > spawner::ANIMAL_MIN_RAW_BRIGHTNESS
                    {
                        kind = Some([MobId::COW, MobId::PIG, MobId::SHEEP][(rng() as usize) % 3]);
                        want =
                            cfg.group_min + (rng() as usize) % (cfg.group_max - cfg.group_min + 1);
                        spawn_mob(&mut self.mobs_app.world, kind.unwrap(), center);
                        animals += 1;
                        got += 1;
                    }
                }
            }
        }
    }

    /// 柱面地表 y（heightmap：首空格 y，mcv_worldgen 约定）；缺区块返回 0。
    fn surface_at(&self, x: i32, z: i32) -> i32 {
        let Some(h) = self.chunks.get(&BlockPos::new(x, 0, z).chunk()) else {
            return 0;
        };
        let [lx, _, lz] = BlockPos::new(x, 0, z).local();
        i32::from(h.heightmap.read().unwrap()[(lz << 4) | lx])
    }

    /// 方块查询（未加载 = 实心石，生成侧取保守值）。
    fn block_at(&self, p: BlockPos) -> BlockId {
        WorldView {
            chunks: &self.chunks,
        }
        .block(p)
    }

    /// ON_GROUND 可站立判定：下方实心 + 本格与头顶空气（2 格净空，
    /// SpawnPlacements ON_GROUND 语义的 AABB 近似）。
    fn standable(&self, p: BlockPos) -> bool {
        let below = self.block_at(BlockPos::new(p.x, p.y - 1, p.z)).def().solid;
        let here = !self.block_at(p).def().solid;
        let above = !self.block_at(BlockPos::new(p.x, p.y + 1, p.z)).def().solid;
        below && here && above
    }

    /// 左键按下入口（桌面鼠标/触摸按下边沿共用）：先攻准星下的 mob，
    /// 未命中则创造秒破、生存/极限进入进度挖掘 START。**按住**期间的连挖
    /// 不在这里——那是 `fixed_step` 每 tick 驱动的 `step_mining`
    /// （原版 Minecraft.continueAttack，Minecraft.java:1606-1628）。
    pub fn on_left_press(&mut self) {
        // 加载态屏蔽（26.1：LevelLoadingScreen 活动时 MouseHandler 不派发
        // 攻击，continueAttack 只在 screen==null 时走）。
        if self.phase == GamePhase::Loading {
            return;
        }
        // 攻击沿必挥臂（26.1 startAttack → Minecraft.java :1669 `this.player
        // .swing(InteractionHand.MAIN_HAND)`；打实体/挖方块/对空挥统一在这里）。
        self.swing();
        if self.try_attack() {
            return;
        }
        if self.mode == GameMode::Creative {
            // 创造按下 = 立即秒破 + destroyDelay=5（MultiPlayerGameMode:157-167），
            // 后续按住连破由 step_mining 的 creative_tick 接管。
            self.interact(false);
            self.mine.delay = 5;
            return;
        }
        self.start_mining();
    }

    /// 准星射线选 mob 并攻击（26.1 攻击判定先于挖掘），返回是否被攻击消费。
    fn try_attack(&mut self) -> bool {
        // mob hit: nearest mob within reach along view dir
        let dir = self.camera(1.0).dir();
        let eye = self.player.pos + glam::Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let weapon = self.hotbar_item();
        let ctx = combat::AttackContext {
            attacker_pos_eye: eye,
            weapon,
            cooldown_ticker: self.attack_ticker,
            fall_distance: 0.0,
            on_ground: self.player.on_ground,
            in_water: false,
            sprinting: false,
            // 力量/虚弱加值与急迫/挖掘疲劳攻速乘子（MobEffects.java:41-45/
            // :71-75/:29-40；加值在冷却缩放前并入基础伤害，Player.attack:945-950）。
            attack_damage_bonus: self.effects.bundle().attack_damage_add as f32,
            attack_speed_mult: self.effects.bundle().attack_speed_mult as f32,
        };
        let out = combat::resolve_attack(&ctx);
        // 准星射线选目标（旧实现 3.5m + cos>0.92 锥形近似、无遮挡，可隔墙
        // 打怪）：实体攻击距离 = DEFAULT_ENTITY_INTERACTION_RANGE 3.0
        // （Player.java:133）；射线先对世界求方块命中距离，实体 AABB 命中
        // 必须在方块命中之前（方块 clip 与实体射线取近的 hit pick 语义）。
        let dir = dir.normalize_or_zero();
        let view = WorldView {
            chunks: &self.chunks,
        };
        let block_t = block_hit_t(&view, eye, dir, ENTITY_ATTACK_RANGE);
        let mut targets: Vec<(Vec3, [f32; 3])> = Vec::new();
        let mut ents: Vec<mcv_ecs::Entity> = Vec::new();
        {
            let bodies = self.mobs_app.world.read::<PhysBody>();
            let kinds = self.mobs_app.world.read::<MobKind>();
            for (e, body) in bodies.iter() {
                if let Some(k) = kinds.get(e) {
                    ents.push(e);
                    targets.push((body.pos, k.0.def().half_size));
                }
            }
        }
        let best = pick_attack_target(eye, dir, &targets, block_t).map(|(i, t)| (ents[i], t));
        let mut slain = None;
        let mut struck = false;
        if let Some((target, _)) = best {
            {
                let (kind, mut health, mut ticks, mut last_hurt) = (
                    self.mobs_app.world.read::<MobKind>(),
                    self.mobs_app.world.write::<Health>(),
                    self.mobs_app.world.write::<MobTicks>(),
                    self.mobs_app.world.write::<LastHurt>(),
                );
                if let (Some(&MobKind(id)), Some(hp), Some(tk), Some(lh)) = (
                    kind.get(target),
                    health.get_mut(target),
                    ticks.get_mut(target),
                    last_hurt.get_mut(target),
                ) {
                    let def = *id.def();
                    let hurt = combat::apply_hurt(
                        &mut hp.0,
                        &mut tk.invulnerable,
                        &mut lh.0,
                        def.armor,
                        out.damage,
                        0,
                    );
                    struck = hurt.is_some();
                    if struck {
                        // 受击索敌输入（HurtByTargetGoal 等价，无需视线）。
                        tk.hurt_flag = true;
                    }
                    // 死亡判定读回组件现值（语义同原 mob.health <= 0.0）。
                    if struck && hp.0 <= 0.0 {
                        // 带上 defs::MobKind 枚举（death_drops 按种类查 loot）。
                        slain = Some((target, def.xp, def.kind));
                    }
                }
            }
            // mob 受击击退（LivingEntity.java:1238 `knockback(0.4F, ...)`
            // victim 侧；combat::knockback_velocity 公式已核实但运行时从未
            // 调用——combat 审计"玩家命中 mob 无击退"）。
            if struck
                && slain.is_none()
                && let Some(b) = self.mobs_app.world.write::<PhysBody>().get_mut(target)
            {
                let dir = Vec3::new(b.pos.x - eye.x, 0.0, b.pos.z - eye.z);
                b.vel = combat::knockback_velocity(b.vel, b.on_ground, 0.0, 0.4, dir);
            }
            if let Some((e, xp, kind)) = slain {
                // 击杀掉落（26.1 LivingEntity.die → loot）：despawn 前取位姿。
                let pos = self
                    .mobs_app
                    .world
                    .get_ref::<PhysBody>(e)
                    .map(|b| b.pos + Vec3::Y * 0.5)
                    .unwrap_or(self.player.pos);
                self.mobs_app.world.despawn(e);
                self.player_xp += xp;
                let mut rng = spawn_rng();
                for ev in mcv_entity::death_drops(kind, true, pos, &mut rng) {
                    if let Some(item) = mcv_item::item_by_name(ev.item) {
                        mcv_entity::spawn_item_drop(
                            &mut self.mobs_app.world,
                            pos,
                            item,
                            ev.count.min(u8::MAX as u32) as u8,
                            mcv_entity::PICKUP_DELAY,
                            &mut rng,
                        );
                    }
                }
            }
        }
        // 攻击实体即消费这次点击（26.1 左键先打实体），顺带中断进度挖掘。
        let hit_entity = best.is_some();
        if hit_entity {
            // 冷却只在攻击实体时清零（Player.attack:959 `this.onAttack()` →
            // resetOnlyAttackStrengthTicker，Player.java:1816-1824）；对空挥/打方块
            // 不清（旧实现进函数即清）。
            self.attack_ticker = 0.0;
            self.cancel_mining();
        }
        // 耐久只在命中时消耗（26.1 useOnEnemy 语义）；破损清槽并播放
        // 耐久破碎音（缺事件时加载器自带节流 no-op）。
        if struck {
            // 命中 exhaustion 0.1（Player.java:996 causeFoodExhaustion(0.1F)，
            // player_attack.json 同口径）；创造豁免（abilities.invulnerable 门）。
            if self.mode != GameMode::Creative {
                self.player.exhaustion = (self.player.exhaustion + 0.1).min(EXHAUSTION_MAX);
            }
            let broke = self
                .hotbar
                .selected_mut(self.player.sel_slot)
                .hurt(1, &mut || 0);
            if broke {
                self.hotbar.slots[self.player.sel_slot % 9] = mcv_item::ItemStack::empty();
                // 26.1 耐久破碎音（LivingEntity.breakItem :1412-1420：BREAK_SOUND
                // 默认 ITEM_BREAK（DataComponents.java:423 / SoundEvents.java:897
                // "entity.item.break"），volume 0.8、pitch 0.8 + random×0.4）。
                // 旧接线写 1.20 前的 "random.break"——26.1 sounds.json 已无此
                // 事件，实为静默死音，本波按原版事件名/音量/音高对齐。
                let r = (fast_rand() % 1024) as f32 / 1024.0;
                self.audio.play_event_pitched(
                    mcv_audio::events::ITEM_BREAK,
                    [eye.x, eye.y, eye.z],
                    [eye.x, eye.y, eye.z],
                    0.8,
                    0.8 + r * 0.4,
                );
            }
        }
        hit_entity
    }

    /// 攻击武器：选中槽的非方块物品（26.1：方块不参战，按空手算）。
    fn hotbar_item(&self) -> Option<mcv_item::ItemStack> {
        let s = self.hotbar.selected(self.player.sel_slot);
        if !s.is_empty() && !matches!(s.def().kind, mcv_item::ItemKind::Block(_)) {
            Some(s.clone())
        } else {
            None
        }
    }

    /// 身体在水中——原版 `Entity.wasTouchingWater` 语义（Entity.java:1566
    /// -1580：`fluidInteraction.isInFluid(WATER)`，AABB 与水块**任一重叠**
    /// 即触水，非「浸水比例」也非脚下一格）。游泳/浮沉/摔落豁免共用。
    fn in_water(&self, view: &WorldView) -> bool {
        let p = self.player.pos;
        let hx = mcv_game::Player::HALF[0];
        let top = p.y + 2.0 * mcv_game::Player::HALF[1];
        for by in p.y.floor() as i32..=top.floor() as i32 {
            for bx in (p.x - hx).floor() as i32..=(p.x + hx).floor() as i32 {
                for bz in (p.z - hx).floor() as i32..=(p.z + hx).floor() as i32 {
                    if view.block(BlockPos::new(bx, by, bz)).def().liquid {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// 眼睛是否在水中（26.1 `Player.isEyeInFluid(WATER)`，Player.java:607：
    /// 水下挖掘惩罚按**眼位**判定，与物理用的触水版 in_water 区分）。
    /// 接 per-tick 速率惩罚链（原 mcv_game::mining 惩罚实现的 live 路径版）。
    fn eye_in_water(&self, view: &WorldView) -> bool {
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let c = eye.floor().as_ivec3();
        view.block(BlockPos::new(c.x, c.y, c.z)).def().liquid
    }

    /// 眼在水中（渲染侧查询：水下雾/视距，`Scene::underwater`）。
    pub fn eye_under_water(&self) -> bool {
        self.eye_in_water(&WorldView {
            chunks: &self.chunks,
        })
    }

    /// 水下视野系数 waterVision 0..1（LocalPlayer.getWaterVision，:1150-1163
    /// 原式：`a×0.6 + b×0.4`，a = clamp(t/100)、b = t<100 ? 0 :
    /// clamp((t−100)/500)，t = waterVisionTime；满 600 → 1.0）。非水上方
    /// 原版恒 0（:1151-1153 早退），此处调用方只在 underwater 分支消费。
    pub fn water_vision(&self) -> f32 {
        let t = self.water_vision_time as f32;
        if t >= 600.0 {
            return 1.0;
        }
        let a = (t / 100.0).clamp(0.0, 1.0);
        let b = if t < 100.0 {
            0.0
        } else {
            ((t - 100.0) / 500.0).clamp(0.0, 1.0)
        };
        a * 0.6 + b * 0.399_999_98
    }

    /// 空气供给 0..=300（原版气泡 HUD 消费；HUD 接线登记遗留，见报告）。
    pub fn air_supply(&self) -> i32 {
        self.air_supply
    }

    /// 游泳姿态（26.1 `Pose.SWIMMING` 驱动源，Player.java:342-361 +
    /// :1662-1664；第三人称 prone 模型/气泡 HUD 消费）。
    pub fn is_swimming(&self) -> bool {
        self.swimming
    }

    /// 雨粒子生成（26.1 `WeatherEffectRenderer.tickRainParticles`，
    /// WeatherEffectRenderer.java:224-268）：按 `count = 0.225·(2r+1)²·
    /// rainLevel²`（:232）在相机四周 `weatherRadius`=10（Options.java:
    /// 178-184 默认）随机列，查 MOTION_BLOCKING heightmap（本仓 chunk
    /// heightmap 即首空 y，与 `getHeightmapPos().getY()` 同语义，但
    /// **不计流体**是 worldgen 基线差异，下方手动补流体面）、相机高度
    /// ±10 过滤（:239-240）、方块/流体顶面出生（:247-254）。
    /// **空中雨层**（真机报障②）：相机高出地表 >10 的列，地表雨滴全被
    /// :238-240 滤掉，但原版雨幕列 `extractRenderState`（:78-110）仍以
    /// `y0 = max(camY−10, terrain)、y1 = max(camY+10, terrain)` 生成——
    /// 带随相机、飞行照样下。本仓按同一带在 [camY−10, camY+10] 内补
    /// RAIN 粒子（KNOWN-DIVERGENCE：原版幕帘是 rain.png 列渲染
    /// :112-162，本仓粒子引擎无列路径，以粒子近似同一视觉带；地表/地下
    /// 行为与原版逐句一致）。
    /// 生物群系降水门（:241/:282-288）本仓无 biome 系统 → 恒 RAIN；
    /// 岩浆/岩浆块/营火→SMOKE 分支（:255-257）同理未接（KNOWN-DIVERGENCE，
    /// 登记报告）。
    fn spawn_rain_particles(&mut self) {
        let rain_level = self.weather.rain_level();
        if rain_level <= 0.0 {
            return;
        }
        let radius = RAIN_PARTICLE_RADIUS;
        let count = rain_particle_count(rain_level, radius);
        let p = self.player.pos;
        // 相机 = 眼位（原版 `BlockPos.containing(camera.position())`，
        // :228；第一人称相机即在眼睛，y = 脚底 + EYE）。
        let cam = BlockPos::new(
            p.x.floor() as i32,
            (p.y + mcv_game::Player::EYE).floor() as i32,
            p.z.floor() as i32,
        );
        let span = (2 * radius + 1) as u32;
        for _ in 0..count {
            let x = cam.x + (fast_rand() % span) as i32 - radius;
            let z = cam.z + (fast_rand() % span) as i32 - radius;
            let col = BlockPos::new(x, 0, z);
            if !self.chunks.contains_key(&col.chunk()) {
                continue; // 原版 hasChunk 门（getPrecipitationAt :283-285）
            }
            let mut top = self.surface_at(x, z);
            if top < cam.y - 10 {
                // 幕帘带分支：相机高出地表 >10（extractRenderState :91-92
                // 的 y0 钳位成立）→ 雨滴带 [camY−10, camY+10]（原版幕帘
                // y0/y1 全钳 camY±10）。全空列（哨兵 = WORLD_MIN_Y）原版
                // 幕帘无 minY 门（:238 只管粒子），此处同为幕帘带 → 放行。
                let ry = fast_rand() as f64 / u32::MAX as f64;
                let rx = fast_rand() as f64 / u32::MAX as f64;
                let rz = fast_rand() as f64 / u32::MAX as f64;
                self.particles.spawn_rain_drop(
                    x as f64 + rx,
                    (cam.y - 10) as f64 + ry * 20.0,
                    z as f64 + rz,
                );
                continue;
            }
            if top <= mcv_core::WORLD_MIN_Y {
                continue; // 原版 heightmapPos.getY() > minY 门（:238；v6：
                // 全空列哨兵 = WORLD_MIN_Y，绝对域下即拒）
            }
            // heightmap 不计流体（recompute_heightmap 基线）→ 上溯流体面
            // （原版 particleY = max(blockTop, fluidTop)，:252-254）。
            while top < mcv_core::WORLD_MAX_Y - 1
                && self.block_at(BlockPos::new(x, top, z)).def().liquid
            {
                top += 1;
            }
            if top > cam.y + 10 {
                continue; // :239-240 上界（地表高出相机 → 室内/悬顶无雨）
            }
            let rx = fast_rand() as f64 / u32::MAX as f64;
            let rz = fast_rand() as f64 / u32::MAX as f64;
            self.particles
                .spawn_rain_drop(x as f64 + rx, top as f64, z as f64 + rz);
        }
    }

    /// 固定步余量（partialTickTime 0..1，26.1 Minecraft.getFrameTime 语义）：
    /// 渲染层做粒子/实体帧间插值用。
    pub fn tick_frac(&self) -> f32 {
        self.tick_frac as f32
    }

    /// Mouse look.
    ///
    /// 加载态不生效：26.1 LevelLoadingScreen 是活动 Screen，MouseHandler
    /// 只在 mouseGrabbed 且 screen 为 null 时转向玩家。
    pub fn look(&mut self, dx: f64, dy: f64) {
        if self.phase == GamePhase::Loading {
            return;
        }
        let k = 0.0025 * self.sens;
        self.player.yaw += dx as f32 * k;
        // 折回 (−π, π]（审计 coords P4：yaw 无界累积使 f32 三角函数精度
        // 随游玩时长退化；sin/cos 消费端取值不变）。
        self.player.yaw = (self.player.yaw + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        self.player.pitch = (self.player.pitch - dy as f32 * k).clamp(-1.55, 1.55);
    }

    /// Break / place at the crosshair. Uses a temporary inline DDA until the
    /// physics module merges; the voxel write path is final.
    /// 视线 5 格命中的方块(位置, 方块 id)——壳层拦截工作台等交互方块用。
    pub fn look_block(&self) -> Option<(BlockPos, u16)> {
        let view = WorldView {
            chunks: &self.chunks,
        };
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let dir = self.camera(1.0).dir();
        let (hit, _) = dda_hit(&view, eye, dir, 5.0, mcv_game::blockshapes::RayTarget::Pick)?;
        Some((hit, view.block(hit).0))
    }

    pub fn interact(&mut self, place: bool) {
        // 加载态屏蔽（同 on_left_press：Screen 非 null 时不派发交互）。
        if self.phase == GamePhase::Loading {
            return;
        }
        // 食物右键 = 启动进食（26.1 Item.use → Consumable.startConsuming，
        // Item.java:189-192）。进食不依赖视线命中（方块 use 优先的拦截——
        // 本仓唯一可交互方块是工作台——已由 app 层在进入本函数前完成）；
        // 空手/非食物返回 false，走原放置路径、行为零变化。
        if place && self.try_begin_eating() {
            return;
        }
        let view = WorldView {
            chunks: &self.chunks,
        };
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let dir = self.camera(1.0).dir();
        // 交互距离按模式取 26.1 block_interaction_range（生存 4.5 / 创造 5.0）；
        // 命中判据 = 拾取形状（火把/花草可命中，见 blockshapes）。
        let Some((hit, normal)) = dda_hit(
            &view,
            eye,
            dir,
            block_interaction_reach(self.mode),
            mcv_game::blockshapes::RayTarget::Pick,
        ) else {
            return;
        };
        if !place {
            // 破坏入口：创造秒破走这里；生存由 step_mining 完成后调 destroy_block。
            self.destroy_block(hit);
            return;
        }
        // 熔炉方块 use（26.1 startUseItem 方块 use 优先 → AbstractFurnaceBlock
        // useWithoutItem 开 FurnaceMenu）：命中 furnace → 开/关熔炉屏，不落
        // 放置（手持方块右键炉 = 开炉，与工作台 app 层拦截同先例）。
        let hit_id = view.block(hit).0;
        if hit_id == self.furnace_block() {
            self.toggle_furnace_screen(hit);
            return;
        }
        let target = BlockPos::new(hit.x + normal[0], hit.y + normal[1], hit.z + normal[2]);
        // y 出界拒绝（coords 审计 P2：界外索引绕回同列另一端——显示≠
        // 真实）；原版 build 高度界外不可放置/破坏。v6：界门走绝对域。
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&target.y) {
            return;
        }
        // 目标格可替换门（26.1 `BlockPlaceContext.canPlace`
        // BlockPlaceContext.java:55-57 → `BlockState.canBeReplaced`
        // BlockBehaviour.java:270-272/819-829：空气 ∥ Properties.replaceable
        // ——雪层/植被/火/水岩浆族；判据表实现见 `BlockDef::is_replaceable`）。
        // 原版 canPlace 失败即整个 useOn 中止——不放方块、不消耗手持
        // （ItemStack.useOn → place 返回 FAIL 路径 shrink 不到），故本门置于
        // 体素写与 take_one 之前：实心格点右键 = 无事发生。
        if !self.block_at(target).def().is_replaceable() {
            return;
        }
        // 放置物 = 选中槽 Block 物品；非方块物品/空槽右键无事发生
        // （26.1 交互仅方块实现，其余走未实现的 useItem）。
        let place_id = {
            let s = self.hotbar.selected(self.player.sel_slot);
            match s.def().kind {
                mcv_item::ItemKind::Block(bid) if !s.is_empty() => Some(bid),
                _ => None,
            }
        };
        let Some(new_id) = place_id else {
            return;
        };
        // reject placement that would intersect the player AABB
        let p = &self.player;
        let (pmin, pmax) = player_aabb(&p.pos);
        let cmin = Vec3::new(target.x as f32, target.y as f32, target.z as f32);
        let cmax = cmin + Vec3::ONE;
        let overlap = cmin.x < pmax.x
            && cmax.x > pmin.x
            && cmin.y < pmax.y
            && cmax.y > pmin.y
            && cmin.z < pmax.z
            && cmax.z > pmin.z;
        if overlap {
            return;
        }
        if let Some(handle) = self.chunks.get(&target.chunk()) {
            let [lx, _, lz] = target.local();
            let idx = mcv_core::vidx(lx, target.y, lz);
            let old_id = handle.voxels.read().unwrap()[idx];
            // 形状状态位（半砖上下/楼梯朝向）写入体素高 nibble，
            // 网格与碰撞按 mcv_core::BlockId::state 读取。
            let st = placement_state(
                mcv_core::shape::shape(new_id.0),
                normal,
                self.input.sneak,
                self.player.yaw,
            );
            handle.voxels.write().unwrap()[idx] = new_id.with_state(st);
            handle.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
            // C1：写体素后立刻增量重光照 + heightmap 维护 + 跨区块边派发
            //（26.1 setBlock → LevelLightEngine.checkBlock 的对应位）。
            // 边派发只入队（stream 跨帧消化，审计 A1/F2），编辑源块进
            // 当帧优先重建通道（审计 A2/F3）。
            relight_block_edit(
                &self.chunks,
                target,
                old_id.0,
                new_id.0,
                &mut self.pending_light_edges,
            );
            self.mesh_priority = Some(target.chunk());
            // 生存放置消耗一格（vanilla consumeItem）；创造不消耗。
            if self.mode != GameMode::Creative {
                self.hotbar.take_one(self.player.sel_slot);
            }
            // 放置按新方块发声（26.1 BlockItem.placeBlock → BlockItem.java:87：
            // placeSound，volume=(st.volume+1)/2、pitch=st.pitch×0.8）。
            if new_id.0 != 0 {
                let st = sound_type_of(new_id.0);
                let p = [
                    target.x as f32 + 0.5,
                    target.y as f32 + 0.5,
                    target.z as f32 + 0.5,
                ];
                self.audio.play_event_pitched(
                    st.place,
                    p,
                    [eye.x, eye.y, eye.z],
                    (st.volume + 1.0) / 2.0,
                    st.pitch * 0.8,
                );
            }
        }
    }

    // ---- 进食（26.1 Consumable；属性表/状态机见 mcv_item::food）----

    /// 工作台方块 id（app 层同款按注册名查；查不到回退已知值 252）。
    fn crafting_table(&self) -> u16 {
        mcv_core::BLOCKS
            .iter()
            .position(|b| b.name == "crafting_table")
            .unwrap_or(252) as u16
    }

    /// 熔炉方块 id（同款按注册名查；查不到回退 blocks_gen 实号 417）。
    fn furnace_block(&self) -> u16 {
        mcv_core::BLOCKS
            .iter()
            .position(|b| b.name == "furnace")
            .unwrap_or(417) as u16
    }

    /// 熔炉屏开/关（26.1 startUseItem 方块 use 优先 → AbstractFurnaceBlock
    /// useWithoutItem 打开 FurnaceMenu）。已开会话 = 关闭归还；未开 = 开新屏。
    /// 单会话：对第二台炉右键先归还第一台的内容（本仓炉内容不可持久，
    /// 见 [`GameRuntime::close_furnace_session`] 债注）。
    fn toggle_furnace_screen(&mut self, at: BlockPos) {
        if self.furnace.is_some() {
            self.close_furnace_session();
            return;
        }
        self.furnace = Some((at, crate::ui::FurnaceScreen::new()));
    }

    /// 关熔炉屏（vanilla `removed` 语义 + 守恒纪律）：光标先背包后掉落
    /// （ACM:588-596 dropOrPlaceInInventory）；容器槽（输入/燃料/产物）原版
    /// 留在方块实体（removed 不清炉内容）——本仓无方块实体存储、存档面无
    /// 炉内容落点，持久 = 重载静默吞物，故关屏即取回：走 CM:96-99
    /// clearContainer 同款世界掉落通道（审计 #52 禁静默吞物）。债登记
    /// HANDOFF-FURNACE.md（方块实体波接持久化后改回原版语义）。
    fn close_furnace_session(&mut self) {
        let Some((_, f)) = self.furnace.take() else {
            return;
        };
        let (cursor, drops) = f.close_to_world();
        let mut spills: Vec<mcv_item::ItemStack> = Vec::new();
        if let Some(st) = cursor
            && let Some(rest) = self.hotbar.add(self.player.sel_slot, st)
        {
            spills.push(rest);
        }
        spills.extend(drops);
        if !spills.is_empty() {
            crate::ui::drop_container_items(self, &spills);
        }
    }

    /// 熔炉产物取出→背包：入栏走 Hotbar::add 既有拾取通道（game.rs
    /// 「give 路径唯一」），满栏余量走 [`crate::ui::drop_container_items`]
    /// 世界掉落。返回是否取到。
    pub fn furnace_take_result(&mut self) -> bool {
        let Some(st) = self.furnace.as_mut().and_then(|(_, f)| f.take_result()) else {
            return false;
        };
        if let Some(rest) = self.hotbar.add(self.player.sel_slot, st) {
            crate::ui::drop_container_items(self, &[rest]);
        }
        true
    }

    /// `Player.canEat`（Player.java:1581-1582）的本仓并入式：
    /// invulnerable（创造）|| canAlwaysEat || needsFood（hunger < 20）。
    fn can_eat_now(&self, food: &mcv_item::food::FoodProperties) -> bool {
        self.mode == GameMode::Creative
            || mcv_item::food::can_eat(self.player.hunger, food.can_always_eat)
    }

    /// 右键 use 入口的食物分支（`Item.use` Item.java:189-192 →
    /// `Consumable.startConsuming` Consumable.java:64-77）：选中槽是食物且
    /// 可吃 → 落启动账、返回 true（右键不再走放置）；否则 false 行为不变。
    /// 按住期间的推进在 fixed_step（每 on_tick +1），松开即取消。
    fn try_begin_eating(&mut self) -> bool {
        if self.dead || self.eat_cooldown > 0 {
            return false;
        }
        // 方块 use 优先（26.1 startUseItem 先试 block use）：工作台/熔炉在档。
        if self
            .look_block()
            .is_some_and(|(_, id)| id == self.crafting_table() || id == self.furnace_block())
        {
            return false;
        }
        let selected = self.hotbar.selected(self.player.sel_slot);
        if selected.is_empty() {
            return false;
        }
        let item = selected.item;
        let Some(food) = mcv_item::food::food_properties(item) else {
            return false;
        };
        if !self.can_eat_now(food) {
            return false;
        }
        self.eat_hold = Some(mcv_item::food::Eating { item, ticks: 0 });
        true
    }

    /// 进食咀嚼拍（step_eating 未完食的每个固定步调用；仅 on_tick 节拍发
    /// 声）。26.1 节拍门 `shouldEmitParticlesAndSounds`（Consumable.java:
    /// 119-124）：`已吃 > (int)(total×0.21875)` 且 `remaining % 4 == 0`——
    /// `remaining` 用**本拍自增前**的已吃数（本仓 s.ticks 已含本拍，故
    /// used = ticks−1，与原版 onUseTick 先播后减的序一致）。音量/音高与完
    /// 食拍同式：GENERIC_EAT（Consumable.java:42 默认 sound）音量
    /// nextBoolean ? 0.5 : 1.0（:105/:109 EAT 分支）、音高
    /// triangle(1.0, 0.2)（:106/:110；RandomSource.java:63-65）；3D 于实体
    /// 足底（LivingEntity.playSound → Player.java:397-398）。调用链：
    /// LivingEntity.updateUsingItem :3408 → ItemStack.onUseTick
    /// （ItemStack.java:1094-1098）→ Consumable.emitParticlesAndSounds
    /// :104-117。本仓无 DRINK 动画食物（mcv_item 食物表无饮食物），饮音
    /// 分支不接（音量 0.5 / 音高 randomBetween(0.9,1.0) :107-108）。
    fn eat_chew_tick(&mut self, food: Option<&mcv_item::food::FoodProperties>) {
        if !self.on_tick {
            return;
        }
        let Some(e) = self.eat_hold.as_ref() else {
            return;
        };
        let total = food.map_or(u32::from(mcv_item::food::DEFAULT_EAT_TICKS), |f| {
            f.consume_ticks()
        });
        let used = e.ticks.saturating_sub(1);
        let remaining = total.saturating_sub(used);
        let wait = (total as f32 * 0.21875) as u32;
        if remaining % 4 != 0 || used <= wait {
            return;
        }
        let (vol, pitch) = eat_volume_pitch();
        let p = self.player.pos;
        self.audio.play_event_pitched(
            mcv_audio::events::GENERIC_EAT,
            [p.x, p.y, p.z],
            [p.x, p.y, p.z],
            vol,
            pitch,
        );
    }

    /// 完食结算（`FoodProperties.onConsume` FoodProperties.java:40-49 +
    /// `Consumable.onConsume` Consumable.java:78-94）：
    /// FoodData.eat（nutrition 回饥饿、饱和走公式与溢出钳）→ 效果概率掷 →
    /// 物品 −1（创造豁免，vanilla hasInfiniteMaterials）→ 右键重触发延迟。
    fn finish_eating(&mut self, item: u16) {
        let Some(food) = mcv_item::food::food_properties(item) else {
            return;
        };
        // 完食拍发声（Consumable.onConsume :77-79 → emitParticlesAndSounds
        // :104-117：GENERIC_EAT、音量/音高同咀嚼拍）。
        let (vol, pitch) = eat_volume_pitch();
        let p = self.player.pos;
        self.audio.play_event_pitched(
            mcv_audio::events::GENERIC_EAT,
            [p.x, p.y, p.z],
            [p.x, p.y, p.z],
            vol,
            pitch,
        );
        // FoodData.eat(int, float)（FoodData.java:24-26；溢出规则 :19-22）。
        mcv_item::food::eat(
            &mut self.player.hunger,
            &mut self.player.saturation,
            i32::from(food.nutrition),
            food.saturation_modifier,
        );
        // ApplyStatusEffectsConsumeEffect：概率掷（腐肉 0.8 / 蜘蛛眼 1.0）。
        // spawn_rng 输出 31 位（game.rs fast_rand 链），归一到 [0,1)。
        if let Some(eff) = food.effect {
            let roll = (spawn_rng()() as f32) / 2147483648.0;
            if mcv_item::food::effect_fires(&eff, roll) {
                let kind = match eff.kind {
                    mcv_item::food::FoodEffectKind::Hunger => mcv_entity::Kind::Hunger,
                    mcv_item::food::FoodEffectKind::Poison => mcv_entity::Kind::Poison,
                };
                self.effects
                    .apply_simple(kind, eff.duration_ticks, eff.amplifier);
            }
        }
        if self.mode != GameMode::Creative {
            self.hotbar.take_one(self.player.sel_slot);
        }
        // 按住右键的连吃节奏 = 原版 rightClickDelay（4 tick）。
        self.eat_cooldown = mcv_item::food::RIGHT_CLICK_DELAY_TICKS;
    }

    /// 进食进度 0..=1（HUD 进度条备用通路；进度画面与第一人称 EAT 抖动
    /// 动画待 HUD/手代理落地后接线，登记 TODO 不做）。
    pub fn eat_progress(&self) -> Option<f32> {
        let eating = self.eat_hold.as_ref()?;
        let total = mcv_item::food::food_properties(eating.item)
            .map_or(u32::from(mcv_item::food::DEFAULT_EAT_TICKS), |f| {
                f.consume_ticks()
            });
        Some((eating.ticks as f32 / total.max(1) as f32).min(1.0))
    }

    /// 破坏目标方块：体素清零 + MESH/SAVE 脏 + 生存掉落/耐久/exhaustion
    /// （26.1 destroyBlock 三段拆门：掉落与记账走 `hasCorrectToolForDrops`，
    /// 耐久走 `mineBlock` 独立段）+ break 音效。挖掘进度完成与创造秒破共用。
    fn destroy_block(&mut self, target: BlockPos) {
        // y 出界拒绝（同 interact 放置；防界外索引绕回删到同列另一端）。
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&target.y) {
            return;
        }
        let Some(handle) = self.chunks.get(&target.chunk()) else {
            return;
        };
        let [lx, _, lz] = target.local();
        let idx = mcv_core::vidx(lx, target.y, lz);
        let old = handle.voxels.read().unwrap()[idx];
        if old.0 == 0 {
            return;
        }
        handle.voxels.write().unwrap()[idx] = BlockId(0);
        handle.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
        // 熔炉会话兜底（vanilla 炉毁经 BaseContainerBlockEntity.preRemoveSide
        // Effects/Containers.dropContents 落内容）：会话炉方块被毁即关屏归还，
        // 禁会话悬空成吞物黑洞（守恒硬约束，审计 #52）。
        if self.furnace.as_ref().is_some_and(|(pos, _)| *pos == target) {
            self.close_furnace_session();
        }
        // C1：与放置同一接线——增量重光照（removal 波 + 边界派发）+
        // heightmap 维护（26.1 destroy → checkBlock）。边派发入队延后，
        // 编辑源块当帧优先重建（审计 A1/A2/F2/F3）。
        relight_block_edit(
            &self.chunks,
            target,
            old.0,
            0,
            &mut self.pending_light_edges,
        );
        self.mesh_priority = Some(target.chunk());
        // 破坏爆裂碎屑（ClientLevel.addDestroyBlockEffect:942-973：满块
        // 0.25 密度 4×4×4=64 粒，取被破坏方块图集层的 1/4 随机小矩形）。
        self.particles.spawn_block_crack(
            [target.x as f64, target.y as f64, target.z as f64],
            old.0,
            0,
        );
        // 生存掉落需正确工具（错误工具能磨掉但不掉东西）。创造秒破不留
        // 掉落物（26.1 give 进创造背包，此处背包未做 → 直接消失）。
        if self.mode != GameMode::Creative {
            let held = self.held_stack();
            // 掉落门 hasCorrectToolForDrops（ServerPlayerGameMode.java:295
            // canDestroy）：门内才走 playerDestroy——exhaustion 0.005 与
            // 掉落同门记账（Block.java:469-479 causeFoodExhaustion(0.005F)），
            // 错误工具磨掉方块两者皆无；创造经 abilities.invulnerable 门豁免
            //（Player.java:1561-1567）。
            if mcv_item::mining::has_correct_tool(old, held.as_ref()) {
                self.player.exhaustion =
                    (self.player.exhaustion + EXHAUSTION_MINE).min(EXHAUSTION_MAX);
                // 生成点：方块中心 ±0.25 随机三轴、y 再 −0.125（26.1
                // Block.popResource，Block.java:410-418）；pickup_delay 走
                // 默认 10 tick（Block.java:436-444，非 0 贴手）。
                let mut rng = spawn_rng();
                let j = |rng: &mut dyn FnMut() -> u32| (rng() as f32 / u32::MAX as f32 - 0.5) * 0.5;
                let c = Vec3::new(
                    target.x as f32 + 0.5 + j(&mut rng),
                    target.y as f32 + 0.5 + j(&mut rng) - 0.125,
                    target.z as f32 + 0.5 + j(&mut rng),
                );
                for drop in mcv_item::drops_for_block(old, &mut rng) {
                    mcv_entity::spawn_item_drop(
                        &mut self.mobs_app.world,
                        c,
                        drop.item,
                        drop.count,
                        mcv_entity::PICKUP_DELAY,
                        &mut rng,
                    );
                }
                // 作物收获（#146，26.1 loot_table/blocks/wheat.json 直译：
                // age=7 → 1 麦粒 + 0..3 种子、age<7 → 1 种子）。物品名 → id
                // 走 mcv_item::item_by_name——ITEMS（#144 在扩）现无
                // wheat/wheat_seeds → 解析 None 即不掉，待 #144 合入自动
                // 接通（KNOWN-DIVERGENCE GROWTH-1 ②；届时勿在 DROP_TABLE
                // 重复登记小麦行，防双重掉落）。
                for (name, count) in crate::growth::harvest_drops(old, &mut rng) {
                    if let Some(item_id) = mcv_item::item_by_name(name) {
                        mcv_entity::spawn_item_drop(
                            &mut self.mobs_app.world,
                            c,
                            item_id,
                            count,
                            mcv_entity::PICKUP_DELAY,
                            &mut rng,
                        );
                    }
                }
            }
            // 耐久段（ServerPlayerGameMode.java:296 itemStack.mineBlock →
            // Item.java:257-268）：带 Tool 组件的手持物（镐/斧/锹 1、剑 2）对
            // destroySpeed != 0 的方块每次成功挖掘扣 1，与掉落门**解耦**——
            // 木镐挖钻石矿不掉落但照样耗；硬度 0（花草）不扣。耐久耗尽即销毁
            // （ItemStack.java:466-468 applyDamage → shrink(1)），清槽 +
            // 耐久破碎音同攻击段（LivingEntity.breakItem :1412-1420：
            // ITEM_BREAK、volume 0.8、pitch 0.8 + random×0.4；旧接线
            // "random.break" 是 26.1 已删事件名，本波对齐）。
            let cost = mcv_item::mining::mine_durability_cost(old, held.as_ref());
            if cost > 0 {
                let broke = self
                    .hotbar
                    .selected_mut(self.player.sel_slot)
                    .hurt(cost, &mut || 0);
                if broke {
                    self.hotbar.slots[self.player.sel_slot % 9] = mcv_item::ItemStack::empty();
                    let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
                    let r = (fast_rand() % 1024) as f32 / 1024.0;
                    self.audio.play_event_pitched(
                        mcv_audio::events::ITEM_BREAK,
                        [eye.x, eye.y, eye.z],
                        [eye.x, eye.y, eye.z],
                        0.8,
                        0.8 + r * 0.4,
                    );
                }
            }
        }
        // 破坏音（26.1 客户端 levelEvent 2001 → LevelEventHandler.java:280-285：
        // breakSound，volume=(st.volume+1)/2、pitch=st.pitch×0.8）。
        if old.0 != 0 {
            let st = sound_type_of(old.0);
            let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
            let p = [
                target.x as f32 + 0.5,
                target.y as f32 + 0.5,
                target.z as f32 + 0.5,
            ];
            self.audio.play_event_pitched(
                st.dig,
                p,
                [eye.x, eye.y, eye.z],
                (st.volume + 1.0) / 2.0,
                st.pitch * 0.8,
            );
        }
    }

    /// 选中槽物品（空槽 = None，挖掘按徒手算）。
    fn held_stack(&self) -> Option<mcv_item::ItemStack> {
        let s = self.hotbar.selected(self.player.sel_slot);
        (!s.is_empty()).then(|| s.clone())
    }

    /// 本 tick 条件下的 per-tick 挖掘速率（26.1 每 tick 重算：
    /// `ServerPlayerGameMode.tick()` :107-130 → `Player#getDestroySpeed`
    /// Player.java:586-614——空中 ÷5（:611-612）、眼在水中 ×0.2（:607-608））。
    fn mine_per_tick(&self, view: &WorldView, block: BlockId) -> f32 {
        mcv_item::mining::progress_per_tick_env(
            block,
            self.held_stack().as_ref(),
            self.player.on_ground,
            self.eye_in_water(view),
        )
    }

    /// 生存/极限 START（26.1 START_DESTROY_BLOCK，MultiPlayerGameMode:147-205）：
    /// 起手射线按交互距离（生存 4.5），速率含当前空中/水下惩罚，
    /// 首 tick 进度即计入，≥1 走 "insta mine" 秒破；不可破坏方块（进度 0）
    /// 直接无事。
    fn start_mining(&mut self) {
        let view = WorldView {
            chunks: &self.chunks,
        };
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let dir = self.camera(1.0).dir();
        let Some((hit, _)) = dda_hit(
            &view,
            eye,
            dir,
            block_interaction_reach(self.mode),
            mcv_game::blockshapes::RayTarget::Pick,
        ) else {
            return;
        };
        let block = view.block(hit);
        if block.0 == 0 {
            return;
        }
        let per = self.mine_per_tick(&view, block);
        if per <= 0.0 {
            return; // 不可破坏（基岩）
        }
        if let Some(p) = self.mine.start(hit, per) {
            self.destroy_block(p);
        }
    }

    /// 松开左键 = 原版 stopDestroyBlock：发 ABORT_DESTROY_BLOCK、进度**作废**
    /// （MultiPlayerGameMode.java:207-222；服务端 ABORT 分支只清状态不破坏，
    /// ServerPlayerGameMode.java:239-249）。旧"perTick×(tick+1) ≥ 0.7 补判
    /// 破坏"删除——0.7 阈值只存在于服务端复核**客户端完成上报**的 STOP 包
    /// （ServerPlayerGameMode.java:216-236），玩家中途主动松手从不破坏；本
    /// 引擎无客户端上报，主动松手一律作废。
    pub fn on_left_release(&mut self) {
        self.mine.abort();
    }

    /// 兼容入口：攻击实体等旁路取消进度挖掘（语义 = ABORT，见 on_left_release）。
    fn cancel_mining(&mut self) {
        self.mine.abort();
    }

    /// 原版 Minecraft.continueAttack（Minecraft.java:1606-1628）：左键**按住
    /// 状态**驱动，每 tick 重射线一次——不是按下边沿一次性。`on_tick` 门 =
    /// 每 20 Hz tick 推进一次（60 Hz 按 dt 连加会偏 3×）。行为：
    /// 生存 = 同目标续挖（速率每 tick 现算）/ 换目标 ABORT+START / 挖穿后
    /// 5-tick 冷却自动开下一目标；创造 = 冷却减尽后每 tick 秒破准星目标。
    fn step_mining(&mut self, _dt: f32) {
        if !self.input.mining {
            // 松开边沿已在 on_left_release 走 ABORT，这里是防御路径
            // （死亡清输入等）。
            self.mine.abort();
            return;
        }
        if !self.on_tick {
            // 本固定步未跨 tick 边界：原版一个 tick 只 continue 一次。
            return;
        }
        let view = WorldView {
            chunks: &self.chunks,
        };
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let dir = self.camera(1.0).dir();
        let reach = block_interaction_reach(self.mode);
        if self.mode == GameMode::Creative {
            let hit = dda_hit(
                &view,
                eye,
                dir,
                reach,
                mcv_game::blockshapes::RayTarget::Pick,
            )
            .map(|(p, _)| p);
            // 按住连破的每 tick 挥臂（continueAttack 对 destroying 的每 tick
            // swing；半程重启规则保证周期 3 tick）。
            if hit.is_some() {
                self.swing();
            }
            if let Some(p) = self.mine.creative_tick(hit) {
                self.destroy_block(p);
            }
            return;
        }
        // 每 tick 重射线（原版客户端 hitResult 每 tick 重算；超出 reach 打不中
        // → None → continue_tick 内 ABORT，取代旧"中心距 >5.5 才中止"）。
        let ray = dda_hit(
            &view,
            eye,
            dir,
            reach,
            mcv_game::blockshapes::RayTarget::Pick,
        );
        // 保留命中面（面号 0..5 = +X,-X,+Y,-Y,+Z,-Z）：未挖穿的每个 tick
        // 在命中面撒碎屑（Minecraft.java:1619 → addBreakingBlockEffect）。
        let mut face = 0u8;
        let target = ray.map(|(p, n)| {
            face = match (n[0], n[1], n[2]) {
                (1, ..) => 0,
                (-1, ..) => 1,
                (_, 1, _) => 2,
                (_, -1, _) => 3,
                (_, _, 1) => 4,
                _ => 5,
            };
            p
        });
        let hit = target
            .map(|p| (p, view.block(p)))
            .filter(|(_, b)| b.0 != 0)
            .map(|(p, b)| MineHit {
                pos: p,
                per_tick: self.mine_per_tick(&view, b),
            });
        // 挖掘长按挥臂（continueAttack :1628 挖中方块即 swing；半程重启规则
        // 令挥臂周期 = duration/2 = 3 tick，与原版挖掘节奏一致）。
        // 碎屑方块 id 先行取出：Idle 分支后续要用，不能让 `view`（&self.chunks
        // 的不可变借用）横跨下面的 `self.swing()`（E0502）。
        let debris = target.map(|p| (p, view.block(p).0));
        if hit.is_some() {
            self.swing();
        }
        // 挖掘中敲击音（26.1 客户端 continueDestroyBlock，MultiPlayerGameMode
        // .java:251-263）：同目标续挖且 `destroyTicks % 4 == 0` 的 tick 播
        // hit 音（volume=(st.volume+1)/8、pitch=st.pitch×0.5）。计数在 START
        // 清零（:196）、续挖每 tick 递增（:268），判定用递增前值——本调用在
        // continue_tick 之前，与原版「先判后增」同序。创造秒破无敲击音（:155-170）。
        if self.mine.pos == target
            && self.mine.destroy_ticks.is_multiple_of(4)
            && let Some((p, b)) = debris
        {
            let st = sound_type_of(b);
            self.audio.play_event_pitched(
                st.hit,
                [p.x as f32 + 0.5, p.y as f32 + 0.5, p.z as f32 + 0.5],
                [eye.x, eye.y, eye.z],
                (st.volume + 1.0) / 8.0,
                st.pitch * 0.5,
            );
        }
        match self.mine.continue_tick(hit) {
            MineTick::Broken(p) => self.destroy_block(p),
            MineTick::Idle => {
                if let Some((p, b)) = debris {
                    self.particles.spawn_hit(
                        [p.x as f64, p.y as f64, p.z as f64],
                        face,
                        b,
                        [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    );
                }
            }
        }
    }

    /// 真实面光照：读 ChunkHandle.light（低 nibble=block 高 nibble=sky，
    /// 与 mesher 同一约定）。缺区块按 0（对齐 mesher 的 missing=0）。
    fn light_at(&self, p: BlockPos) -> (u8, u8) {
        let Some(chunk) = self.chunks.get(&p.chunk()) else {
            return (0, 0);
        };
        if chunk.stage() == Stage::Empty {
            return (0, 0);
        }
        if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&p.y) {
            return (0, 0); // 界外按缺格保守全黑（缺块分支同值）
        }
        let [lx, _, lz] = p.local();
        let v = chunk.light.read().unwrap()[mcv_core::vidx(lx, p.y, lz)];
        (v & 0xF, v >> 4)
    }

    /// 第一人称挥臂进度 0..=1（26.1 attackAnim = swingTime/duration，
    /// LivingEntity.java:2158；静止 = 0）。渲染层取值驱动手持摆动。
    pub fn swing_progress(&self) -> f32 {
        if self.swinging {
            (self.swing_time / SWING_TICKS).min(1.0)
        } else {
            0.0
        }
    }

    /// 行走 bob 渲染参数（渲染帧插值）：`(bob, backwardsInterpolatedWalkDistance)`。
    /// 26.1 ClientAvatarState.java:85-87 `getInterpolatedBob`
    /// （lerp(bobO, bob, partialTicks)）与 :89-92
    /// `getBackwardsInterpolatedWalkDistance`（bID = −(walkDist + Δ·t)——
    /// 注意 walkDist 本身不做 o→cur 插值，原版直接取当前值加外推差）。
    pub fn hand_bob(&self) -> (f32, f32) {
        let f = self.tick_frac as f32; // tick_frac 内部 f64（20 Hz 累加器）
        let bob = self.bob_o + (self.bob - self.bob_o) * f;
        let wd = self.walk_dist + (self.walk_dist - self.walk_dist_o) * f;
        (bob, -wd)
    }

    /// 第一人称手持物（渲染层数据）：选中槽 Block 物品 = 缩小方块（取
    /// `BLOCKS[].tiles`）、其余物品 = GUI 精灵图标 quad、空槽 = 只有手臂。
    pub fn hand_item(&self) -> mcv_render::gpu::HandItem {
        item_icon(self.hotbar.selected(self.player.sel_slot))
    }

    /// 掉落物渲染快照（#150 刀 1）：mobs ECS World 里的 ItemDrop 集合 →
    /// 渲染端 billboard 批（[`mcv_entity::collect_item_drop_snaps`] 只读
    /// 收集 + 同源图标解析 [`item_icon`] + 所在格光照）。视距裁剪 = 渲染
    /// 距离 ×16（mob 收集同口径）；年龄换 tick 叠加 partialTick——26.1
    /// ItemEntityRenderState.ageInTicks 的 bob/spin 相位同源。
    pub fn item_drops(&self) -> Vec<mcv_render::item_render::ItemDropDraw> {
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let snaps = mcv_entity::collect_item_drop_snaps(
            &self.mobs_app.world,
            Some((eye, self.render_dist as f32 * 16.0)),
        );
        snaps
            .into_iter()
            .map(|s| {
                // 光照取掉落物盒中心所在格（feet + 0.125；缺区块按 0 全黑，
                // light_at 同 mesher 的 missing=0 口径）。
                let (b, k) = self.light_at(BlockPos::new(
                    s.pos.x.floor() as i32,
                    (s.pos.y + 0.125).floor() as i32,
                    s.pos.z.floor() as i32,
                ));
                mcv_render::item_render::ItemDropDraw {
                    pos: s.pos.to_array(),
                    age_ticks: s.age_steps as f32 / mcv_entity::STEPS_PER_TICK as f32
                        + self.tick_frac as f32,
                    phase: s.phase,
                    icon: item_icon(&mcv_item::ItemStack::new(s.item, s.count)),
                    light: [b, k],
                }
            })
            .collect()
    }

    /// 挖掘/选中 overlay（渲染层数据）：挖掘中目标锁定状态机目标并按进度
    /// 给裂纹档位（原版 10 档：`(int)(destroyProgress * 10)`，
    /// MultiPlayerGameMode.java:551）；未挖掘时准星 DDA 目标只描边。
    /// 面暴露 = 邻格空气；面光照取邻格（与 mesher 面光照同规则）。
    pub fn mining_overlay(&self) -> Option<mcv_render::gpu::MiningOverlay> {
        let view = WorldView {
            chunks: &self.chunks,
        };
        let mining = self.mine.pos.is_some();
        let target = match self.mine.pos {
            Some(p) => p,
            None => {
                let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
                let dir = self.camera(1.0).dir();
                let (hit, _) = dda_hit(
                    &view,
                    eye,
                    dir,
                    block_interaction_reach(self.mode),
                    mcv_game::blockshapes::RayTarget::Pick,
                )?;
                hit
            }
        };
        if view.block(target).0 == 0 {
            return None;
        }
        const NORMALS: [[i32; 3]; 6] = [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ];
        let mut faces = [mcv_render::gpu::MineFace {
            exposed: false,
            block_light: 0,
            sky_light: 0,
        }; 6];
        for (i, n) in NORMALS.iter().enumerate() {
            let nb = BlockPos::new(target.x + n[0], target.y + n[1], target.z + n[2]);
            if view.block(nb).0 == 0 {
                let (b, s) = self.light_at(nb);
                faces[i] = mcv_render::gpu::MineFace {
                    exposed: true,
                    block_light: b,
                    sky_light: s,
                };
            }
        }
        let stage = if mining {
            // 原版 10 档映射（MultiPlayerGameMode.java:551）。
            Some(((self.mine.progress * 10.0).clamp(0.0, 9.0)) as u32)
        } else {
            None
        };
        Some(mcv_render::gpu::MiningOverlay {
            min: [target.x as f32, target.y as f32, target.z as f32],
            crack_stage: stage,
            faces,
        })
    }

    /// 当前绘制条目表（#80：合批批次条目，由 mesher 实现自持）。
    pub fn render_chunks(&self) -> &[RenderChunk] {
        self.mesher.entries()
    }

    /// 网格侧结构性统计（#80 可观测性：draw 收敛 + 异步上传带宽）。
    pub fn mesh_stats(&self) -> MeshStats {
        self.mesher.mesh_stats()
    }

    /// E0 帧界归并：把本帧分段窗口（[`Self::timing_frame`]）并入 1Hz
    /// 累计器并清零帧窗。app 层每渲染帧末调用一次（draw 段须先行归账）；
    /// 纯计时记账，零行为变化。
    pub fn end_frame_timing(&mut self) {
        let frame = std::mem::take(&mut self.timing_frame);
        self.timing_stats.record(frame);
    }

    /// E0：取走自上次取走以来累计的 1Hz 汇总（均值/峰值在日志端派生），
    /// 累计器清零。
    pub fn take_frame_timing_stats(&mut self) -> FrameTimingStats {
        std::mem::take(&mut self.timing_stats)
    }

    /// HUD：MC 26.1 风格（准星 / 快捷栏 / 心 / 饥饿，Gui.java 常数），
    /// `gui` 为 None 时整体回退旧程序化绘制；触屏摇杆程序化，
    /// `show_touch`（死亡界面等场景传 false 隐藏摇杆）。
    /// `show_hotbar` = false 时不画快捷栏（合成/创造界面自带 36 格面板，
    /// 避免底部快捷栏与面板内快捷栏重复）。
    pub fn build_hud(
        &self,
        width: f32,
        height: f32,
        gui: Option<&mcv_render::gui::SpriteSheet>,
        show_touch: bool,
        show_hotbar: bool,
    ) -> Vec<HudQuad> {
        let mut quads = Vec::new();
        let s = mcv_render::gui_scale(height);
        let white = [1.0, 1.0, 1.0, 1.0];
        let sel = self.player.sel_slot % 9;
        if let Some(g) = gui {
            // 准星：15x15 居中（MC crosshair.png）
            if let Some(q) = g.sprite_full(
                "crosshair",
                (width - 15.0 * s) * 0.5,
                (height - 15.0 * s) * 0.5,
                15.0 * s,
                15.0 * s,
                [1.0, 1.0, 1.0, 0.85],
            ) {
                quads.push(q);
            }
            if show_hotbar {
                // 快捷栏：hotbar.png 182x22，选中框 24x23（外扩 1px）
                quads.extend(g.sprite_full(
                    "hotbar",
                    width * 0.5 - 91.0 * s,
                    height - 22.0 * s,
                    182.0 * s,
                    22.0 * s,
                    white,
                ));
                quads.extend(g.sprite_full(
                    "hotbar_sel",
                    width * 0.5 - 92.0 * s + sel as f32 * 20.0 * s,
                    height - 23.0 * s,
                    24.0 * s,
                    23.0 * s,
                    white,
                ));
                // 槽内容（26.1 Gui.renderSlot）：Block 物品取方块图集侧面 tile，
                // 其余物品取 GUI 精灵表图标；count>1 右下角计数；损伤工具画耐久条。
                for (i, stack) in self.hotbar.slots.iter().enumerate() {
                    if stack.is_empty() {
                        continue;
                    }
                    let ix = width * 0.5 - 88.0 * s + i as f32 * 20.0 * s;
                    let iy = height - 19.0 * s;
                    // 图标解析同源（#150）：item_icon 三级分支，与手持/掉落
                    // 物同一函数（26.1 Gui.renderSlot 的 Block→方块 tile、
                    // 其余→item sprite 分支）。
                    match item_icon(stack) {
                        mcv_render::gpu::HandItem::Block(bid) => quads.push(text::tile_icon(
                            mcv_core::BLOCKS[bid as usize].tiles[2],
                            ix,
                            iy,
                            16.0 * s,
                        )),
                        mcv_render::gpu::HandItem::Sprite(name) => {
                            quads.extend(g.sprite_full(name, ix, iy, 16.0 * s, 16.0 * s, white))
                        }
                        mcv_render::gpu::HandItem::Empty => {}
                    }
                    if stack.count > 1 {
                        let t = stack.count.to_string();
                        let tw = text::text_width(&t, s);
                        quads.extend(text::text_quads(
                            &t,
                            ix + 18.0 * s - tw,
                            iy + 11.0 * s,
                            s,
                            white,
                        ));
                    }
                    // renderSlot 耐久条：黑底 13x1 + 绿→红渐变前景，位于图标下沿。
                    if stack.damage > 0 {
                        let max = stack.max_damage().max(1) as f32;
                        let f = 1.0 - stack.damage as f32 / max;
                        let bar = (13.0 - stack.damage as f32 * 13.0 / max).max(0.0) * s;
                        quads.push(text::rect(
                            ix + s,
                            iy + 12.0 * s,
                            13.0 * s,
                            s,
                            [0.0, 0.0, 0.0, 1.0],
                        ));
                        quads.push(text::rect(
                            ix + s,
                            iy + 12.0 * s,
                            bar,
                            s,
                            [f * 0.392, f, 0.0, 1.0],
                        ));
                    }
                }
            }
            // 心（左上）与饥饿（右上镜像）：Gui.renderHealth/renderFood 规则，
            // 整心/半心/空槽三态，右侧先耗尽；无敌帧期间闪烁。
            let x_left = width * 0.5 - 91.0 * s;
            let x_right = width * 0.5 + 91.0 * s;
            let y_base = height - 39.0 * s;
            let blink = self.player.invulnerable > 0 && (self.player.invulnerable / 2) % 2 == 0;
            let bar_a = if blink { 0.4 } else { 1.0 };
            let tint = [1.0, 1.0, 1.0, bar_a];
            let health = self.player.health.max(0.0);
            let food = self.player.hunger.max(0.0);
            for i in 0..10 {
                let hx = x_left + i as f32 * 8.0 * s;
                let fx = x_right - i as f32 * 8.0 * s - 9.0 * s;
                quads.extend(g.sprite_full("heart_container", hx, y_base, 9.0 * s, 9.0 * s, tint));
                if health >= (i as f32 + 1.0) * 2.0 {
                    quads.extend(g.sprite_full("heart_full", hx, y_base, 9.0 * s, 9.0 * s, tint));
                } else if health >= i as f32 * 2.0 + 1.0 {
                    quads.extend(g.sprite_full("heart_half", hx, y_base, 9.0 * s, 9.0 * s, tint));
                }
                quads.extend(g.sprite_full("food_empty", fx, y_base, 9.0 * s, 9.0 * s, tint));
                if food >= (i as f32 + 1.0) * 2.0 {
                    quads.extend(g.sprite_full("food_full", fx, y_base, 9.0 * s, 9.0 * s, tint));
                } else if food >= i as f32 * 2.0 + 1.0 {
                    quads.extend(g.sprite_full("food_half", fx, y_base, 9.0 * s, 9.0 * s, tint));
                }
            }
            // 空气泡（26.1 Gui.extractAirBubbles:884-927）：眼下在水或
            // air<满值（300）才显示；行位 = 心/饥饿行上一行（yLineAir =
            // yLineBase − 10，:790 vehicleHearts==0 分支）；右缘镜像
            // x = xRight − (i−1)·8 − 9（:905，i 从 1 起）。三态映射：
            // 满 = ceil((air−2)·10/300)（:926 getCurrentAirSupplyBubble
            // offset −2）、爆裂位 = ceil(air·10/300)（offset 0，仅水下且
            // 满≠爆裂位，:898/:908-911）、空 = 10 − ceil((air+delay)·10/300)，
            // delay = air≠0 且水下 ? 1 : 0（:921-923）。简化不建模：爆裂帧
            // 时长 2（AIR_BUBBLE_POPPING_DURATION:129，客户端瞬时态）与空泡
            // 随机抖动（:912 tickCount%2）、pop 音（playAirBubblePoppedSound
            // :929——BUBBLE_POP 事件未进音效表）。
            let under_water = {
                let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
                let ec = eye.floor().as_ivec3();
                let view = WorldView {
                    chunks: &self.chunks,
                };
                let d = view.block(BlockPos::new(ec.x, ec.y, ec.z)).def();
                d.liquid && d.name == "water"
            };
            let air = self.air_supply.clamp(0, MAX_AIR_SUPPLY);
            if under_water || air < MAX_AIR_SUPPLY {
                // 桶数换算对齐原版 Mth.ceil((air+offset)*10/max)（Gui.java:926
                // getCurrentAirSupplyBubble；CI rustc 无 i32::div_ceil，且原版
                // 本就是浮点 ceil——air+offset ≥ −2·10 = −20，f32 距离内精确）。
                let bubbles = |offset: i32| {
                    (((air + offset) * 10) as f32 / MAX_AIR_SUPPLY as f32).ceil() as i32
                };
                let full = bubbles(-2);
                let popping = bubbles(0);
                let empty = 10 - bubbles(if air != 0 && under_water { 1 } else { 0 });
                let y_air = y_base - 10.0 * s;
                for b in 1..=10i32 {
                    let bx = x_right - (b - 1) as f32 * 8.0 * s - 9.0 * s;
                    if b <= full {
                        quads.extend(g.sprite_full("air", bx, y_air, 9.0 * s, 9.0 * s, tint));
                    } else if full != popping && b == popping && under_water {
                        quads.extend(g.sprite_full(
                            "air_bursting",
                            bx,
                            y_air,
                            9.0 * s,
                            9.0 * s,
                            tint,
                        ));
                    } else if b > 10 - empty {
                        quads.extend(g.sprite_full("air_empty", bx, y_air, 9.0 * s, 9.0 * s, tint));
                    }
                }
            }
        } else {
            // 回退：旧程序化准星 + 快捷栏
            let (cx, cy) = (width * 0.5 - 1.0, height * 0.5 - 8.0);
            quads.push(text::rect(cx, cy, 2.0, 16.0, [1.0, 1.0, 1.0, 0.75]));
            quads.push(text::rect(
                width * 0.5 - 8.0,
                height * 0.5 - 1.0,
                16.0,
                2.0,
                [1.0, 1.0, 1.0, 0.75],
            ));
            if show_hotbar {
                let slot = 40.0;
                let total = slot * 9.0;
                let x0 = width * 0.5 - total * 0.5;
                let y0 = height - slot - 8.0;
                quads.push(text::rect(
                    x0 - 2.0,
                    y0 - 2.0,
                    total + 4.0,
                    slot + 4.0,
                    [0.1, 0.1, 0.1, 0.6],
                ));
                for (i, stack) in self.hotbar.slots.iter().enumerate() {
                    let x = x0 + i as f32 * slot;
                    quads.push(text::rect(
                        x + 1.0,
                        y0 + 1.0,
                        slot - 2.0,
                        slot - 2.0,
                        [0.25, 0.25, 0.28, 0.8],
                    ));
                    if !stack.is_empty() {
                        // 图标解析同源（#150）：与 sprite 路径同一 item_icon；
                        // 无图集时 Sprite 分支只画通用色块（无程序化贴图）。
                        match item_icon(stack) {
                            mcv_render::gpu::HandItem::Block(bid) => quads.push(text::tile_icon(
                                mcv_core::BLOCKS[bid as usize].tiles[2],
                                x + 5.0,
                                y0 + 5.0,
                                slot - 10.0,
                            )),
                            // 无图集时非方块物品只画通用色块。
                            _ => quads.push(text::rect(
                                x + (slot - 20.0) * 0.5,
                                y0 + (slot - 20.0) * 0.5,
                                20.0,
                                20.0,
                                [0.55, 0.5, 0.42, 0.95],
                            )),
                        }
                        if stack.count > 1 {
                            let t = stack.count.to_string();
                            let tw = text::text_width(&t, 1.0);
                            quads.extend(text::text_quads(
                                &t,
                                x + slot - 3.0 - tw,
                                y0 + slot - 12.0,
                                1.0,
                                white,
                            ));
                        }
                    }
                    if i == sel {
                        quads.push(text::rect(
                            x - 1.0,
                            y0 - 1.0,
                            slot + 2.0,
                            2.0,
                            [1.0, 1.0, 1.0, 0.9],
                        ));
                    }
                }
            }
        }
        // 触屏控件（仅在收到过触摸事件后显示；死亡界面隐藏）
        if self.touch.enabled && show_touch {
            use mcv_platform::touch::{BTN_R, STICK_R};
            let stick_c = TouchState::stick_center(width, height);
            // 摇杆底盘 + 滑块
            quads.push(text::rect(
                stick_c.0 - STICK_R,
                stick_c.1 - STICK_R,
                STICK_R * 2.0,
                STICK_R * 2.0,
                [1.0, 1.0, 1.0, 0.10],
            ));
            let (ox, oy) = self.touch.stick_vec;
            quads.push(text::rect(
                stick_c.0 + ox - STICK_R * 0.4,
                stick_c.1 + oy - STICK_R * 0.4,
                STICK_R * 0.8,
                STICK_R * 0.8,
                [1.0, 1.0, 1.0, 0.35],
            ));
            // 动作按钮（跳 / 挖 / 放）
            let jump_c = TouchState::jump_center(width, height);
            let mine_c = TouchState::mine_center(width, height);
            let place_c = TouchState::place_center(width, height);
            let alpha = 0.18;
            quads.push(text::rect(
                jump_c.0 - BTN_R,
                jump_c.1 - BTN_R,
                BTN_R * 2.0,
                BTN_R * 2.0,
                [
                    0.4,
                    0.9,
                    0.4,
                    alpha + if self.touch.jump_held { 0.15 } else { 0.0 },
                ],
            ));
            quads.push(text::rect(
                mine_c.0 - BTN_R,
                mine_c.1 - BTN_R,
                BTN_R * 2.0,
                BTN_R * 2.0,
                [
                    0.9,
                    0.5,
                    0.3,
                    alpha + if self.touch.mine_held { 0.15 } else { 0.0 },
                ],
            ));
            quads.push(text::rect(
                place_c.0 - BTN_R,
                place_c.1 - BTN_R,
                BTN_R * 2.0,
                BTN_R * 2.0,
                [0.4, 0.6, 0.9, alpha],
            ));
            // 按钮符号（程序化）
            // 跳跃: 上箭头杆
            quads.push(text::rect(
                jump_c.0 - 3.0,
                jump_c.1 - 12.0,
                6.0,
                22.0,
                [1.0, 1.0, 1.0, 0.8],
            ));
            quads.push(text::rect(
                jump_c.0 - 10.0,
                jump_c.1 - 6.0,
                8.0,
                6.0,
                [1.0, 1.0, 1.0, 0.8],
            ));
            quads.push(text::rect(
                jump_c.0 + 2.0,
                jump_c.1 - 6.0,
                8.0,
                6.0,
                [1.0, 1.0, 1.0, 0.8],
            ));
            // 挖掘: 竖柄 + 斜头（近似镐）
            quads.push(text::rect(
                mine_c.0 - 2.0,
                mine_c.1 - 14.0,
                5.0,
                26.0,
                [1.0, 1.0, 1.0, 0.8],
            ));
            quads.push(text::rect(
                mine_c.0 - 14.0,
                mine_c.1 - 16.0,
                28.0,
                5.0,
                [1.0, 1.0, 1.0, 0.8],
            ));
            // 放置: 加号
            quads.push(text::rect(
                place_c.0 - 3.0,
                place_c.1 - 13.0,
                6.0,
                26.0,
                [1.0, 1.0, 1.0, 0.8],
            ));
            quads.push(text::rect(
                place_c.0 - 13.0,
                place_c.1 - 3.0,
                26.0,
                6.0,
                [1.0, 1.0, 1.0, 0.8],
            ));
        }

        // debug line
        let p = &self.player.pos;
        let line = format!(
            "XYZ {:.1}/{:.1}/{:.1}  chunks {}  t{}",
            p.x,
            p.y,
            p.z,
            self.chunks.len(),
            self.time_ticks
        );
        quads.extend(text::text_quads(&line, 8.0, 8.0, 1.5, [1.0, 1.0, 1.0, 0.9]));
        quads
    }
}

/// mob AI 系统的单步只读快照：区块表克隆（物理步进的地形）+ 玩家位姿
/// （追踪目标）+ **20 Hz tick 上下文**——ECS 系统拿不到 GameRuntime，
/// tick 语义（20 tick/s）必须经 Resources 传入（审计 C-1 的接线面）。
/// 每固定步 `insert` 进 Resources——克隆只是加 Arc 计数，体素/光照数据
/// 仍与原表共享。
#[derive(Clone)]
pub struct MobServices {
    pub chunks: HashMap<ChunkPos, Arc<ChunkHandle>>,
    pub player_pos: Vec3,
    /// 本固定步是否跨过 20 Hz tick 边界（[`GameRuntime::on_tick`] 快照）：
    /// 所有 tick 语义计时器（noActionTime / 燃烧 / AI 决策 /
    /// 箭矢推进）只在此为真时 +1，60 Hz 步只跑物理。
    pub on_tick: bool,
    /// 本固定步跨过的 20 Hz tick 数（0/1 常态）。受击无敌帧按它递减——
    /// 原版 LivingEntity.tick 每 game tick 给非玩家实体 invulnerableTime
    /// −1（LivingEntity.java:452-453），绝不按 60 Hz 步递减（旧实现
    /// 20 计数 = 0.33s，怪 3 击/秒、i 帧缩水 3 倍）；burst 步 n>1 时
    /// 比 on_tick 单次递减更忠实。
    pub ticks_step: u32,
    /// 单调 20 Hz 计数快照（[`GameRuntime::game_ticks`]）。
    pub game_ticks: u64,
    /// 白天（EnvironmentAttributes.MONSTERS_BURN，Timelines.java:157）——
    /// 亡灵白天直晒燃烧的判据。
    pub monsters_burn: bool,
    /// 当前 skyDarken = 15 − SKY_LIGHT_LEVEL（Level.java:736，关键帧
    /// Timelines.java:80-85；天气混合 WeatherAttributes.java:13/:26）——
    /// magic light / 亮度门读天光前必须扣减（LevelReader.java:163-170）。
    pub sky_darken: u8,
    /// 难度 id（Difficulty.java:28-30 快照）：和平清怪/不索敌见
    /// Mob.java:656 与 LivingEntity.java:928；skeleton 射速/散布见
    /// AbstractSkeleton.java:51-54,170；箭 base 噪声见
    /// AbstractArrow.java:718-720。
    pub difficulty: u8,
    /// 玩家创造态快照：创造玩家不被索敌（LivingEntity.java:928
    /// canBeSeenAsEnemy 的本仓等价门，审计 N-5）。
    pub creative: bool,
    /// 票表快照（[`TicketTable`]）：实体门按实体所在块查
    /// `level_of_type(PlayerSimulation)` ≤ ENTITY_TICKING——原版
    /// `inEntityTickingRange`（DistanceManager.java:136-138）的消费侧
    /// 镜像。票表每帧从玩家锚点全量重建（refresh_player_tickets），此处
    /// 克隆为固定步快照（表恒小，与 chunks 的 Arc 克隆同量级）。
    pub tickets: TicketTable,
}

/// 近战命中事件（信号）：AI 系统发射，GameRuntime 在固定步末 drain 后结算
/// 玩家伤害——系统与玩家状态之间不共享可变借用。
#[derive(Clone, Copy)]
pub struct MobMeleeHit {
    pub src: Vec3,
    pub damage: f32,
}

/// 箭矢命中玩家事件（结算同近战：走 hurt_player）。
#[derive(Clone, Copy)]
pub struct MobArrowHit {
    pub src: Vec3,
    pub damage: f32,
}

/// 玩家箭命中 mob 事件（arrow_system 发射；结算走
/// [`GameRuntime::settle_player_arrow_hits`]，同 try_attack 的伤害路径）。
#[derive(Clone, Copy)]
pub struct PlayerArrowHitMob {
    pub target: mcv_ecs::Entity,
    /// 命中点（击退方向基准，AbstractArrow.doKnockback :514-516 近似）。
    pub src: Vec3,
    /// 已含速度缩放/暴击的点数（AbstractArrow.java:421-437）。
    pub damage: u32,
}

/// creeper 引爆事件：AI 发信号，GameRuntime 按距离衰减结算玩家伤害
/// 并移除本体（爆炸会改方块 → 等爆炸系统完整实现后补，TODO）。
#[derive(Clone, Copy)]
pub struct MobExplosionHit {
    pub center: Vec3,
    pub radius: f32,
}

/// 区块光照表读取 `(sky, block)`（打包字节：高 4 位天光、低 4 位方块光；
/// 与 [`GameRuntime::light_at`] 的 (block, sky) 返回序相反，mob 域统一
/// (sky, block) 以对齐 Java LightLayer.SKY/BLOCK 顺序）。
/// 未加载/未点亮列按全黑 (0,0)——燃烧/刷怪侧取保守值。
fn chunk_light(chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>, p: BlockPos) -> (u8, u8) {
    let Some(h) = chunks.get(&p.chunk()) else {
        return (0, 0);
    };
    if !(mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).contains(&p.y) {
        return (0, 0); // 界外全黑（未点亮列保守值同分支）
    }
    let [lx, _, lz] = p.local();
    let b = h.light.read().unwrap()[mcv_core::vidx(lx, p.y, lz)];
    (b >> 4, b & 0xF)
}

/// 视线（26.1 Sensing.hasLineOfSight 等价）：怪眼 → 玩家眼的射线被固体
/// 方块截断即不可见（TargetGoal.java:64 的 unseen 账本输入；审计 C-2 的
/// LOS 接线）。玩家本体不在体素网格，只判方块遮挡。
fn has_los(view: &WorldView, from: Vec3, to: Vec3) -> bool {
    let d = to - from;
    let dist = d.length();
    dist <= 1e-4 || mcv_game::raycast(view, from, d, dist).is_none()
}

/// mob AI + 物理固定步系统：AI 决策/计时按 20 Hz tick（`on_tick` 门，
/// Brain = Goal 系统等价，审计 C-2 接线），物理步进 60 Hz 沿上一 tick 的
/// [`MobIntent`]。结构性变更走延迟命令，命中走事件。
pub fn mob_ai_system(ctx: &mut mcv_ecs::SysCtx) {
    let svc = ctx
        .resources
        .get::<MobServices>()
        .expect("mob_ai：MobServices 快照未注入 Resources");
    // disjoint 字段借用：表视图挂 world，命令/事件各自独立可写。
    let world: &mcv_ecs::World = ctx.world;
    let commands = &mut *ctx.commands;
    let events = &mut *ctx.events;
    let (mut phys, kind, mut ticks, mut yaw, mut health, mut brains, mut intents, mut paths) = (
        world.write::<PhysBody>(),
        world.read::<MobKind>(),
        world.write::<MobTicks>(),
        world.write::<Yaw>(),
        world.write::<Health>(),
        world.write::<MobBrain>(),
        world.write::<MobIntent>(),
        world.write::<MobPath>(),
    );
    let view = WorldView {
        chunks: &svc.chunks,
    };
    let p_eye = svc.player_pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
    let mut rng = || fast_rand();
    phys.for_each(|e, body| {
        // 实体 tick 双门（26.1 ServerLevel.java:418-419 逐条镜像）：
        // 1) 模拟环：`inEntityTickingRange`（DistanceManager.java:136-138）
        //    = PLAYER_SIMULATION 票级别视图 ≤ ENTITY_TICKING（谓词收口到
        //    ticket::is_entity_ticking，实体环 = 距玩家锚点 Chebyshev
        //    ≤ sim_dist）——模拟距离外实体完全不 tick（不 AI、不位移，
        //    invulnerableTime 递减也在 tick 内，LivingEntity.java:452-453），
        //    且**不删除不卸载**（冻结≠消失，回环即恢复）；
        // 2) 已载：所在区块 stage ≥ TerrainReady（entityManager
        //    .canPositionTick 的证据位等价）——世界视图未载改空气，环内
        //    未就绪块上的怪会隐形石面坠落（旧实现症状）。
        // 环外/未载仍跑 checkDespawn（ServerLevel.java:417-419：despawn
        // 检查在环门**之前**，Mob.java:655-679——和平 discard、>128² 立即
        // 移除、<32² 清 noActionTime；环外不跑 serverAiStep，noActionTime
        // 不增，Mob.java:683 在 tick 内）。
        let own = ChunkPos::new(
            (body.pos.x / 16.0).floor() as i32,
            (body.pos.z / 16.0).floor() as i32,
        );
        let sim_ticking = ticket::is_entity_ticking(
            svc.tickets.level_of_type(TicketType::PlayerSimulation, own) as i32,
        );
        let loaded = svc
            .chunks
            .get(&own)
            .is_some_and(|h| (h.stage() as u8) >= (Stage::TerrainReady as u8));
        if !sim_ticking || !loaded {
            if svc.on_tick {
                let Some(&MobKind(id)) = kind.get(e) else {
                    return;
                };
                let def = *id.def();
                // 和平清怪在 checkDespawn（Mob.java:656-658），环外照跑。
                if svc.difficulty == 0 && def.hostile {
                    commands.despawn(e);
                    return;
                }
                if let Some(tk) = ticks.get_mut(e) {
                    let d2 = (svc.player_pos - body.pos).length_squared();
                    if d2 < (spawner::NO_DESPAWN_DIST * spawner::NO_DESPAWN_DIST) as f32 {
                        tk.idle_ticks = 0; // Mob.java:672-673 近距清账
                    } else if spawner::should_despawn(
                        d2,
                        tk.idle_ticks,
                        spawner::DESPAWN_DIST,
                        spawner::NO_DESPAWN_DIST,
                        &mut rng,
                    ) {
                        commands.despawn(e);
                    }
                }
            }
            return;
        }
        let tk = match ticks.get_mut(e) {
            Some(t) => t,
            None => return,
        };
        // i 帧/近战冷却按 game tick 递减（LivingEntity.java:452-453 每 tick
        // −1；60 Hz 步递减会让 20 tick i 帧只剩 0.33s，见 MobServices 注释）。
        tk.invulnerable = tk.invulnerable.saturating_sub(svc.ticks_step);
        let Some(&MobKind(id)) = kind.get(e) else {
            return;
        };
        let def = *id.def();
        let (Some(tk), Some(brain), Some(intent)) =
            (ticks.get_mut(e), brains.get_mut(e), intents.get_mut(e))
        else {
            return;
        };
        let mut pos = body.pos;
        let to_player = svc.player_pos - pos;
        let dist_sqr = to_player.length_squared();
        let speed = speed_m_s(def.speed_attr);
        // 眼位 ≈ 身高×0.85（原版 getEyeY 的实体尺寸近似）。
        let eye = pos + Vec3::new(0.0, def.half_size[1] * 1.7, 0.0);

        if svc.on_tick {
            // ---- 20 Hz tick 语义：计时器只在这里推进（审计 C-1）----
            //（无敌帧递减在循环头部按 ticks_step 处理，见上。）
            // 和平清怪（Mob.checkDespawn:656-658）：PEACEFUL 且非
            // isAllowedInPeaceful → 立即 discard（审计 N-5 接线）。
            if svc.difficulty == 0 && def.hostile {
                commands.despawn(e);
                return;
            }
            // 燃烧：Entity.java:534-544——remainingFireTicks%20==0 时 1 点
            // 伤害（每秒 1 点；"每 tick 1 伤害"系派单口误，以源码为准）。
            if tk.fire_ticks > 0 {
                if tk.fire_ticks % 20 == 0
                    && let Some(h) = health.get_mut(e)
                {
                    h.0 -= 1.0;
                }
                tk.fire_ticks -= 1;
            }
            // magic light（getLightLevelDependentMagicValue，LevelReader.java
            // :113-117 曲线式）：raw = getMaxLocalRawBrightness(眼位) =
            // max(sky − skyDarken, block)（LevelReader.java:163-170 +
            // Level.java:736）。白天直晒 darken=0；夜直晒 raw=15−11=4 →
            // br≈0.09 < 0.5——旧版误用原始 max(sky, block)，夜间直晒也会
            // 错触发 Monster.java:49-54 的 +2 加速与黄昏窗口燃烧概率虚高。
            let (sky, blk) = chunk_light(
                &svc.chunks,
                BlockPos::new(
                    eye.x.floor() as i32,
                    eye.y.floor() as i32,
                    eye.z.floor() as i32,
                ),
            );
            let br = spawner::magic_light(spawner::raw_brightness(sky, blk, svc.sky_darken));
            // noActionTime：Mob.java:683 每 tick +1，Monster.java:51-54 亮处
            // 额外 +2；<32² 清零（Mob.java:673）。
            tk.idle_ticks += spawner::no_action_inc(def.hostile, br);
            if dist_sqr < (spawner::NO_DESPAWN_DIST * spawner::NO_DESPAWN_DIST) as f32 {
                tk.idle_ticks = 0;
            }
            // 消散（Mob.java:655-678，审计 C-3）：>128² 立即移除；
            // idle>600 且 >32² 时每 tick 1/800 随机移除。
            if spawner::should_despawn(
                dist_sqr,
                tk.idle_ticks,
                spawner::DESPAWN_DIST,
                spawner::NO_DESPAWN_DIST,
                &mut rng,
            ) {
                commands.despawn(e);
            } else {
                // ---- AI tick（mcv_entity::ai::Brain，此前是死代码）----
                let los = def.hostile && has_los(&view, eye, p_eye);
                // 障碍判定：想走走不动（速度远低于预期）→ 跳跃兜底
                // （A* 寻路未实现 N-1 的最小替代）。
                let flat_speed = (body.vel.x * body.vel.x + body.vel.z * body.vel.z).sqrt();
                let blocked = intent.wish.length_squared() > 1e-4
                    && body.on_ground
                    && flat_speed < speed * 0.25;
                let hurt = tk.hurt_flag;
                tk.hurt_flag = false;
                // 和平/创造豁免（LivingEntity.java:928 canBeSeenAsEnemy +
                // 审计 N-5）：创造玩家/和平难度不被索敌。
                let targetable = def.hostile && !svc.creative && svc.difficulty != 0;
                // ---- A* 路径账本（审计 N-1）：GroundPathNavigation.moveTo +
                // MeleeAttackGoal.java:100-124 重算节奏。寻路失败 → 航点空，
                // Brain 走直线（旧行为兜底，等价 moveTo false 的 +15 惩罚）。
                let mut waypoint = None;
                if targetable && let Some(path) = paths.get_mut(e) {
                    path.recalc_cd = path.recalc_cd.saturating_sub(1);
                    let moved = (svc.player_pos - path.pathed_target).length_squared() >= 1.0;
                    if path.recalc_cd == 0 && (moved || path.idx >= path.nodes.len()) {
                        let hp = health.get(e).map(|h| h.0).unwrap_or(def.health);
                        let params = mcv_entity::pathfinding::PathParams::ground(
                            def.follow_range,
                            def.half_size[1] * 2.0,
                            mcv_entity::pathfinding::max_fall_distance(
                                true,
                                hp,
                                def.health,
                                svc.difficulty,
                            ),
                        );
                        let tgt = BlockPos::new(
                            svc.player_pos.x.floor() as i32,
                            svc.player_pos.y.floor() as i32,
                            svc.player_pos.z.floor() as i32,
                        );
                        path.pathed_target = svc.player_pos;
                        match mcv_entity::pathfinding::find_path(&view, pos, tgt, &params) {
                            Some(found) => {
                                path.nodes = found.nodes;
                                path.idx = 0;
                                // MeleeAttackGoal.java:111-117：4+rand(7)，
                                // dist²>256 +5、>1024 +10。
                                path.recalc_cd = 4
                                    + rng() % 7
                                    + if dist_sqr > 1024.0 {
                                        10
                                    } else if dist_sqr > 256.0 {
                                        5
                                    } else {
                                        0
                                    };
                            }
                            None => {
                                path.nodes.clear(); // moveTo false → +15（:121-123）。
                                path.recalc_cd = 4 + rng() % 7 + 15;
                            }
                        }
                    }
                    // 航点推进：水平 0.5² 内且高差 <1.5 视作到达（vanilla
                    // tick 判据近似）。
                    while path.idx < path.nodes.len() {
                        let w = path.nodes[path.idx];
                        let dx = pos.x - (w.x as f32 + 0.5);
                        let dz = pos.z - (w.z as f32 + 0.5);
                        if dx * dx + dz * dz < 0.25 && (pos.y - w.y as f32).abs() < 1.5 {
                            path.idx += 1;
                        } else {
                            waypoint =
                                Some(Vec3::new(w.x as f32 + 0.5, w.y as f32, w.z as f32 + 0.5));
                            break;
                        }
                    }
                    if path.idx >= path.nodes.len() {
                        waypoint = Some(svc.player_pos); // 路径走完 → 直奔玩家。
                    }
                }
                let p = mcv_entity::Percept {
                    pos,
                    // 玩家=唯一可索敌实体；和平/创造豁免（N-5）。
                    target: targetable.then_some(svc.player_pos),
                    target_half_width: mcv_game::Player::HALF[0],
                    los,
                    br,
                    day: svc.monsters_burn,
                    // canSeeSky(眼) 的代理：眼位格天光=15。
                    sky_exposed: sky == 15,
                    // 流体/头部装备/避猫狼/藏身点：世界尚无对应系统 → 保守值。
                    in_water: false,
                    head_armor: false,
                    has_bow: mcv_entity::ai_table(def.kind).default_bow,
                    hurt,
                    blocked,
                    avoid: None,
                    shelter: None,
                    difficulty: svc.difficulty,
                    waypoint,
                };
                let acts = brain.0.tick(&def, &p, &mut rng);
                let mut next = MobIntent::IDLE;
                for a in &acts {
                    match a {
                        AiAction::Walk {
                            dir,
                            speed_mult,
                            jump,
                        } => {
                            next.wish = *dir * (speed * speed_mult);
                            next.jump = *jump;
                            // A* 跳跃节点（y+1 台阶）：航点高于脚底 → 起跳。
                            if waypoint.is_some_and(|w| w.y > pos.y + 0.5) {
                                next.jump = true;
                            }
                        }
                        AiAction::Look(t) => {
                            if let Some(y) = yaw.get_mut(e) {
                                let d = *t - pos;
                                y.0 = d.x.atan2(-d.z);
                            }
                        }
                        AiAction::MeleeHit { damage } => {
                            // 20 tick 节拍由 Brain.attack_cd 保证（MeleeAttack
                            // Goal.java:136，audit M-12 的字段拆分随之消解）。
                            events.channel::<MobMeleeHit>().send(MobMeleeHit {
                                src: pos,
                                damage: *damage,
                            });
                        }
                        AiAction::Shoot {
                            dir,
                            speed: sp,
                            spread,
                            base_damage,
                        } => {
                            // 散布近似：按 inaccuracy×0.01 扰动方向（原版
                            // randomTriangularSpread 映射未导出 → 近似）。
                            let jitter =
                                |r: u32| (r as f32 / u32::MAX as f32 - 0.5) * spread * 0.01;
                            let mut d = *dir;
                            d.x += jitter(rng());
                            d.y += jitter(rng());
                            d.z += jitter(rng());
                            let v = d.normalize_or_zero() * *sp;
                            let src = eye;
                            // setBaseDamageFromMob（AbstractArrow.java:718-720）：
                            // base = power×2.0（Brain 已算）+ triangle(难度×0.11,
                            // 0.57425) 难度噪声。
                            let dmg = *base_damage
                                + mcv_entity::arrow::triangle(
                                    svc.difficulty.min(3) as f32 * 0.11,
                                    0.57425,
                                    &mut rng,
                                );
                            commands.spawn_with(move |w, ar| {
                                w.insert(
                                    ar,
                                    MobArrow {
                                        pos: src,
                                        vel: v,
                                        ttl_ticks: 400,
                                        base_damage: dmg,
                                        crit: false, // 怪箭无暴击 flag。
                                        player_owned: false,
                                    },
                                );
                            });
                        }
                        AiAction::Swelling { .. } => {
                            // 嘶嘶/膨胀动画 flag：渲染侧消费 fuse（TODO）。
                        }
                        AiAction::Detonate { radius } => {
                            // Creeper.java:144-149 引爆后本体移除；对玩家的
                            // 距离衰减伤害在 fixed_step 结算。
                            events.channel::<MobExplosionHit>().send(MobExplosionHit {
                                center: pos,
                                radius: *radius,
                            });
                            commands.despawn(e);
                        }
                        AiAction::SetOnFire { ticks: t } => {
                            // Mob.java:494 igniteForSeconds(8) → Entity.java:630-632
                            // floor(8×20)=160 tick。
                            tk.fire_ticks = *t;
                        }
                        AiAction::Nothing => {}
                    }
                }
                *intent = next;
            }
        }

        // ---- 60 Hz 物理：沿上一 tick 的移动意图步进（tick 门只管决策）----
        let input = mcv_game::StepInput {
            wish_dir: intent.wish,
            jump: intent.jump && body.on_ground,
            in_water: false,
            sneak: false,
            sprint: false,
            // 生物无冲刺跳增补（sprint=false 使增补分支不可达），视线置零。
            look_dir: Vec3::ZERO,
            gravity_scale: 1.0,
        };
        // 独立表视图（各自 RefCell）：与 phys 的迭代借用互不冲突。
        let mut eng = body.body();
        step_entity(&view, &mut eng, def.half_size, &input);
        body.set_body(&eng);
        pos = body.pos;
        // 死亡清理：Health<=0 → 排队 despawn（阶段末生效，等价原循环后
        // retain）+ 环境死亡掉落（非玩家击杀，spider_eye 不掉；玩家击杀
        // 在 try_attack 即时结算，不会走到这里）。近战冷却/命中已由
        // Brain.attack_cd 在 tick 门内结算（MeleeAttackGoal.java:21,136）。
        if health.get(e).is_some_and(|h| h.0 <= 0.0) {
            let mut rng = spawn_rng();
            let drops: Vec<(u16, u8)> = mcv_entity::death_drops(def.kind, false, pos, &mut rng)
                .into_iter()
                .filter_map(|ev| {
                    mcv_item::item_by_name(ev.item).map(|i| (i, ev.count.min(u8::MAX as u32) as u8))
                })
                .collect();
            if !drops.is_empty() {
                let at = pos + Vec3::Y * 0.5;
                commands.push(move |w| {
                    let mut r = spawn_rng();
                    for (item, count) in drops {
                        mcv_entity::spawn_item_drop(
                            w,
                            at,
                            item,
                            count,
                            mcv_entity::PICKUP_DELAY,
                            &mut r,
                        );
                    }
                });
            }
            commands.despawn(e);
        }
    });
}

/// 简化箭矢系统（AbstractArrow 近似；完整投射物系统 TODO）：仅 `on_tick`
/// 推进（20 Hz tick 语义），重力 0.05/tick²、惯量 0.99/tick
/// （INERTIA 0.99/tick AbstractArrow.java:59,263；重力 0.05/tick²
/// AbstractArrow.java:339-340）；每 tick 拆 8 子步（0.2 格/步）防高速
/// 隧穿。撞固体方块即移除（原版 onHitBlock → setInGround(true)，
/// AbstractArrow.java:542；残留杆渲染不做）/ 命中玩家或 mob AABB /
/// ttl 耗尽 → 移除。**伤害曲线**（AbstractArrow.java:421-431）：
/// `ceil(命中瞬间速度模长 × baseDamage)`——随弹道重力/空气衰减，远射
/// 伤害降；暴击箭（满蓄力弓，BowItem.java:41）再 `+ rand(d/2+2)`
/// （AbstractArrow.java:434-437）。
pub fn arrow_system(ctx: &mut mcv_ecs::SysCtx) {
    let svc = ctx
        .resources
        .get::<MobServices>()
        .expect("mob_arrows：MobServices 快照未注入 Resources");
    if !svc.on_tick {
        return;
    }
    let world: &mcv_ecs::World = ctx.world;
    let commands = &mut *ctx.commands;
    let events = &mut *ctx.events;
    let view = WorldView {
        chunks: &svc.chunks,
    };
    let mut rng = || fast_rand();
    // mob 表只读视图（玩家箭命中判定用）：与 MobArrow 写视图分表不冲突，
    // 提升到循环外避免每子步重建。
    let bodies = world.read::<PhysBody>();
    let kinds = world.read::<MobKind>();
    let mut arrows = world.write::<MobArrow>();
    arrows.for_each(|e, a| {
        // 实体 tick 双门（同 mob_ai：ServerLevel.java:419 对 entityTickList
        // 全部实体生效——AbstractArrow 也不例外）：模拟环外/所在区块未载
        // 的箭矢滞空冻结（不推进、不命中、寿命不耗），无 checkDespawn
        // 分支（箭非 Mob）。
        let own = ChunkPos::new(
            (a.pos.x / 16.0).floor() as i32,
            (a.pos.z / 16.0).floor() as i32,
        );
        let sim_ticking = ticket::is_entity_ticking(
            svc.tickets.level_of_type(TicketType::PlayerSimulation, own) as i32,
        );
        let loaded = svc
            .chunks
            .get(&own)
            .is_some_and(|h| (h.stage() as u8) >= (Stage::TerrainReady as u8));
        if !sim_ticking || !loaded {
            return;
        }
        a.vel.y -= 0.05; // 重力 0.05/tick²
        let step = a.vel / 8.0;
        let mut dead = false;
        for _ in 0..8 {
            a.pos += step;
            let bp = BlockPos::new(
                a.pos.x.floor() as i32,
                a.pos.y.floor() as i32,
                a.pos.z.floor() as i32,
            );
            if view.block(bp).def().solid {
                dead = true; // 入地/撞墙（onHitBlock setInGround，AbstractArrow.java:542）
                break;
            }
            // 命中量按命中瞬间速度（AbstractArrow.java:421-431）+ 暴击。
            let mut dmg = mcv_entity::arrow::hit_damage(a.vel, a.base_damage);
            if a.crit {
                dmg = mcv_entity::arrow::crit_bonus(dmg, &mut rng);
            }
            let h = mcv_game::Player::HALF;
            let p = svc.player_pos;
            // 玩家 AABB（HALF=[0.3,0.9,0.3]，脚底 p.y → 头顶 +2h）；玩家
            // 自己射的箭不打自己（player_owned 跳过玩家判定）。
            if !a.player_owned
                && (a.pos.x - p.x).abs() < h[0]
                && (a.pos.z - p.z).abs() < h[2]
                && (a.pos.y - (p.y + h[1])).abs() < h[1]
            {
                events.channel::<MobArrowHit>().send(MobArrowHit {
                    src: a.pos,
                    damage: dmg as f32,
                });
                dead = true;
                break;
            }
            // 玩家箭 → mob AABB（LivingEntity.getBoundingBox 相交近似）。
            if a.player_owned {
                for (me, body) in bodies.iter() {
                    let Some(mk) = kinds.get(me) else {
                        continue;
                    };
                    let hs = mk.0.def().half_size;
                    if (a.pos.x - body.pos.x).abs() < hs[0]
                        && (a.pos.z - body.pos.z).abs() < hs[2]
                        && (a.pos.y - (body.pos.y + hs[1])).abs() < hs[1]
                    {
                        events
                            .channel::<PlayerArrowHitMob>()
                            .send(PlayerArrowHitMob {
                                target: me,
                                src: a.pos,
                                damage: dmg,
                            });
                        dead = true;
                        break;
                    }
                }
            }
            if dead {
                break;
            }
        }
        a.vel *= 0.99; // INERTIA（AbstractArrow.java:59）
        a.ttl_ticks = a.ttl_ticks.saturating_sub(1);
        if dead || a.ttl_ticks == 0 || a.pos.y < -8.0 {
            commands.despawn(e);
        }
    });
}

/// 白天判定 = EnvironmentAttributes.MONSTERS_BURN（26.1 环境属性，替代
/// 此前 `day_factor<0.4` 的离散近似）：Timelines.java:157 OR 修饰轨，
/// t%24000 ∈ [23460,24000)∪[0,12542) 为真（白天）。亡灵燃烧判据
/// （Mob.java:499-505 isSunBurnTick）以它为前提。
fn monsters_burn(time_ticks: u64) -> bool {
    let t = time_ticks % 24_000;
    !(12_542..23_460).contains(&t)
}

/// skyDarken = 15 − SKY_LIGHT_LEVEL（Level.java:736）：主世界时间线
/// SKY_LIGHT_LEVEL = 15 × mult，mult 关键帧 (133,1.0)(11867,1.0)
/// (13670,4/15)(22330,4/15)（Timelines.java:81-84）→ darken 白天 0、
/// 夜 11，关键帧间线性插值（修饰轨 easing 曲线未随反编译导出 → 线性近似，
/// KNOWN-DIVERGENCE）。雷暴 skyDarken=10（Monster.java:87）：无天气系统。
fn sky_darken(time_ticks: u64) -> u8 {
    let t = time_ticks % 24_000;
    // 关键帧 133 在跨日处：把 [0,133) 折回上一周期尾段。
    let t = (if t < 133 { t + 24_000 } else { t }) as f32;
    let mult = if t <= 11_867.0 {
        1.0
    } else if t < 13_670.0 {
        1.0 + (4.0 / 15.0 - 1.0) * (t - 11_867.0) / (13_670.0 - 11_867.0)
    } else if t <= 22_330.0 {
        4.0 / 15.0
    } else {
        4.0 / 15.0 + (1.0 - 4.0 / 15.0) * (t - 22_330.0) / (24_133.0 - 22_330.0)
    };
    ((15.0 * (1.0 - mult)).round() as i32).clamp(0, 11) as u8
}

/// Global XOR-shift rand for spawn jitter (deterministic per sequence).
fn fast_rand() -> u32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0x243F6A8885A308D3);
    let v = C.fetch_add(0x9E3779B97F4A7C15, Ordering::Relaxed);
    let z = (v ^ (v >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    ((z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB) >> 33) as u32
}

/// 进食音量/音高（Consumable.java:105-110 EAT 分支：音量
/// `nextBoolean ? 0.5 : 1.0`；音高 `triangle(1.0, 0.2)` =
/// 1.0 + (r1−r2)×0.2，RandomSource.java:63-65。随机源取本仓 fast_rand
/// 1024 档近似，同溅落音 jitter 惯例）。
fn eat_volume_pitch() -> (f32, f32) {
    let vol = if fast_rand().is_multiple_of(2) {
        0.5
    } else {
        1.0
    };
    let r1 = (fast_rand() % 1024) as f32 / 1024.0;
    let r2 = (fast_rand() % 1024) as f32 / 1024.0;
    (vol, 1.0 + (r1 - r2) * 0.2)
}

fn spawn_rng() -> impl FnMut() -> u32 {
    let mut s = u64::from(fast_rand()) | 1;
    move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
    }
}

fn player_aabb(pos: &Vec3) -> (Vec3, Vec3) {
    let h = Player::HALF;
    (
        Vec3::new(pos.x - h[0], pos.y, pos.z - h[2]),
        Vec3::new(pos.x + h[0], pos.y + h[1] * 2.0, pos.z + h[2]),
    )
}

// ---- 出生投放（26.1 setInitialSpawn / PlayerSpawnFinder 等价）----
//
// 原版世界出生点选定链（MinecraftServer.java:480-521 setInitialSpawn）：
// 建议点 = 出生区块第 (8,8) 列地表；随后按区块螺旋（±5，Mth.square(11)）
// 逐块调 `getSpawnPosInChunk` 全列扫描，首个合法列即出生点；全窗无合法
// 列才保留建议点并按 fixupSpawnHeight（PlayerSpawnFinder.java:89-104）
// 校正。列合法性 = getOverworldRespawnPos（PlayerSpawnFinder.java
// :148-176：海面列拒绝 + 满顶面地面）+ noCollisionNoLiquid（:80
// /:106-108，玩家盒 0.6×1.8 = 脚、头两格无碰撞且不在流体里）。
// 本组函数以「已 TerrainReady 的搜索窗」复刻同一机制；搜索半径取
// load_radius（3）而非原版 5：窗即加载门窗（见 stream 投放块注释），
// 之外区块未就绪不可判。

/// 单列投放判定：合法返回脚底 y（= 地面方块顶），不合法返回 None。
///
/// 入参 `hm` 为本仓 heightmap（recompute_heightmap / terrain.cpp pass 3：
/// 最高非「空气/流体/damp0」方块 y + 1），故地面方块 y = hm-1；跳过集
/// 与原版对照：流体不计（OCEAN_FLOOR 语义，lib.rs recompute 注释），
/// 花/火把/玻璃等 damp0 不计——原版逐格向下扫的 isFaceFull(:170) 由
/// 下面的地面形状判定 + 脚头空间判定共同承载。
fn spawn_column_feet_y(voxels: &[BlockId], hm: &[i16], lx: usize, lz: usize) -> Option<i32> {
    let gy = i32::from(hm[(lz << 4) | lx]) - 1;
    // gy<WORLD_MIN_Y：全空列（heightmap 空列哨兵 = WORLD_MIN_Y，gy=-65，
    // 地面判定同拒）；gy>WORLD_MAX_Y−3：gy+2 头位越出世界顶。两者皆无
    // 脚头空间可言（v6：heightmap i16 绝对 y，无 u8 环绕）。
    if !(mcv_core::WORLD_MIN_Y..=(mcv_core::WORLD_MAX_Y - 3)).contains(&gy) {
        return None;
    }
    // 掩状态 nibble（半砖/楼梯朝向），按基础方块查表。
    let def = |y: i32| -> Option<&'static mcv_core::BlockDef> {
        mcv_core::BLOCKS.get(voxels[mcv_core::vidx(lx, y, lz)].id() as usize)
    };
    // 地面须满顶面实体（isFaceFull(shape, UP)，:170）：台阶/楼梯/十字
    // 作地面会令玩家盒悬空或嵌盒，弃列。
    if !def(gy).is_some_and(|d| d.solid && d.shape == mcv_core::shape::Shape::Cube as u8) {
        return None;
    }
    // 海面列拒绝（:156-159 `surface <= topY && surface > ocean_floor` 的
    // 等价谓词）：地面上方出现流体即海/湖床列。生成期流体只会成片压在
    // 实心地面上（terrain.cpp:308-309 水格生成、:318 水不参与雕刻），
    // 玩家又不可放置流体（创造栏水不入栏，assemble 注释），故向上扫到
    // 首个实体即可停——海底洞穴顶板这类「水上有盖」列同样被拒（原版
    // 向下扫遇流体即 break，:166）。
    let mut y = gy + 1;
    while y < mcv_core::WORLD_MAX_Y {
        match def(y) {
            Some(d) if d.liquid => return None,
            Some(d) if d.solid => break,
            _ => y += 1,
        }
    }
    // 脚 + 头两格 passable（noCollisionNoLiquid :106-108）：非实体且非
    // 流体；花/火把等无碰撞装饰放行（26.1 花列同样可作为出生列）。
    // 未注册 id 按实体处理（拒），与 OPACITY 的 unwrap_or(15) 同保守。
    let passable = |y: i32| def(y).is_some_and(|d| !d.solid && !d.liquid);
    (passable(gy + 1) && passable(gy + 2)).then_some(gy + 1)
}

/// 出生点列搜索：出生区块 (0,0) 起 ±radius 区块螺旋（转向式与
/// MinecraftServer.java:513-517 同式），先到先得；未就绪区块跳过（调用
/// 方保证窗就绪或已超时兜底）。出生区块先试建议列 (8,8)（:498 建议点
/// 语义，保持本仓 (8.5,·,8.5) 出生约定——虚空兜底与 respawn 旧落点均
/// 引用该列）；其余列 x 外 z 内 = getSpawnPosInChunk
/// （PlayerSpawnFinder.java:183-190）列序。
fn find_spawn_slot(chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>, radius: i32) -> Option<Vec3> {
    let (mut ox, mut oz) = (0i32, 0i32);
    let (mut dx, mut dz) = (0i32, -1i32);
    for _ in 0..((2 * radius + 1) * (2 * radius + 1)) {
        if (-radius..=radius).contains(&ox)
            && (-radius..=radius).contains(&oz)
            && let Some(h) = chunks.get(&ChunkPos::new(ox, oz))
            && (h.stage() as u8) >= (Stage::TerrainReady as u8)
        {
            let voxels = h.voxels.read().unwrap();
            let hm = h.heightmap.read().unwrap();
            if ox == 0
                && oz == 0
                && let Some(y) = spawn_column_feet_y(&voxels[..], &hm[..], 8, 8)
            {
                return Some(Vec3::new(8.5, y as f32, 8.5));
            }
            for lx in 0..16usize {
                for lz in 0..16usize {
                    if let Some(y) = spawn_column_feet_y(&voxels[..], &hm[..], lx, lz) {
                        return Some(Vec3::new(
                            (ox * 16) as f32 + lx as f32 + 0.5,
                            y as f32,
                            (oz * 16) as f32 + lz as f32 + 0.5,
                        ));
                    }
                }
            }
        }
        // 螺旋转向（MinecraftServer.java:513-517 逐字）：到拐角换向，
        // 否则沿当前方向推进一格。
        if ox == oz || (ox < 0 && ox == -oz) || (ox > 0 && ox == 1 - oz) {
            let old = dx;
            dx = -dz;
            dz = old;
        }
        ox += dx;
        oz += dz;
    }
    None
}

/// fixupSpawnHeight（PlayerSpawnFinder.java:89-104）收敛等价：从建议点
/// 向上爬过所有「有碰撞或流体」的阻挡、再落回顶面，净效果 = 列内最高
/// 实体/流体方块顶 +1（花/火把无碰撞不阻拦，同原版 noCollision）。空列
/// （生成数据不存在）回落世界底 1。仅作全窗无合法列的兜底：全海洋窗口
/// 落在水面之上（掉落游泳），不再埋入沙底。
fn fixup_spawn_feet_y(voxels: &[BlockId], lx: usize, lz: usize) -> i32 {
    for y in (mcv_core::WORLD_MIN_Y..mcv_core::WORLD_MAX_Y).rev() {
        if mcv_core::BLOCKS
            .get(voxels[mcv_core::vidx(lx, y, lz)].id() as usize)
            .is_some_and(|d| d.solid || d.liquid)
        {
            return y + 1;
        }
    }
    mcv_core::WORLD_MIN_Y + 1 // 空列回落世界底+1（旧 `1` 的绝对 y 对应位）
}

fn load_voxels(ids: &[u16]) -> Box<[BlockId; mcv_core::CHUNK_VOL]> {
    debug_assert_eq!(ids.len(), mcv_core::CHUNK_VOL);
    bytemuck::cast_slice::<u16, BlockId>(ids)
        .to_vec()
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| panic!("wrong voxel slice length"))
}

/// 放置时按形状计算状态 nibble（写进体素 bit12-15，规则见 mcv_core）。
/// `normal` = 命中面外法线（+Y 表示点了顶面，= 26.1 getClickedFace）。
///
/// 朝向（C1）：26.1 `StairBlock.getStateForPlacement`（StairBlock.java:101-102）
/// 写 `FACING = context.getHorizontalDirection()` = `player.getDirection()`
/// （Entity.java:3367 `Direction.fromYRot`，即玩家**视线同向**）；此前实现
/// 取视线反方向属错误，已按源码修正。引擎相机约定视线水平分量 =
/// `(sin yaw, -cos yaw)`（camera.rs `Camera::dir`），四向编码
/// 0=+Z 1=-Z 2=+X 3=-X 与网格器 `emit_stairs`/`mcv_core::BlockId::state`
/// 同一约定；楼梯几何为"踏步（整高半）位于 FACING 朝向侧"
/// （StairBlock.java:37-38：facing=NORTH → 上半占 -Z 半格）。
///
/// 半区（C2）：26.1 `StairBlock.java:103-105` 与 `SlabBlock.java:77-80`
/// 同一规则——点底面（DOWN）→ 上半；点顶面（UP）→ 下半；水平面按
/// 点击点在面内 y>0.5 → 上半。我方 DDA 无格内点击点坐标（输入信息缺失，
/// KNOWN-DIVERGENCE），水平面近似为下半，与半砖一致。
/// `sneak` 翻转上下为自加行为（KNOWN-DIVERGENCE：26.1 半砖/楼梯放置均
/// 不看潜行，SlabBlock/StairBlock 源码无 isSecondaryUseActive 分支）。
/// 放置状态 nibble（半砖 bit0；楼梯 bit0-1 朝向 + bit2 上半）。
/// 朝向=视线水平同向（26.1 `StairBlock.java:102` `getHorizontalDirection`，
/// `UseOnContext.java:70`/`DirectionalPlaceContext.java:59`）。半区判据为
/// 近似：原版按格内点击点 Y 定上下（`SlabBlock.java:78`、`StairBlock.java:
/// 104`），DDA 仅有进入面法线 → 顶/底面按法线、侧面恒下半，且原版无潜行
/// 切换（`!= sneak` 为引擎自设便捷位，KNOWN-DIVERGENCE）。
fn placement_state(shape: mcv_core::Shape, normal: [i32; 3], sneak: bool, yaw: f32) -> u8 {
    match shape {
        mcv_core::Shape::Slab => u8::from((normal[1] == -1) != sneak),
        mcv_core::Shape::Stairs => {
            // 视线水平分量 (sin yaw, -cos yaw)，同向归四向（C1）。
            let (fx, fz) = (yaw.sin(), -yaw.cos());
            let facing = if fz.abs() >= fx.abs() {
                if fz > 0.0 { 0 } else { 1 } // +Z / -Z
            } else if fx > 0.0 {
                2 // +X
            } else {
                3 // -X
            };
            // bit2=上半：点底面(-Y)→上半，点顶面(+Y)→下半（C2，
            // 与半砖 normal[1]==-1 分支同向，StairBlock.java:104）。
            facing | (u8::from((normal[1] == -1) != sneak) << 2)
        }
        _ => 0,
    }
}

/// 网格步进射线检测：按形状的命中判据（`mcv_game::blockshapes`）——
/// `mode=Pick` 用拾取形状（原版拾取与碰撞无关：火把/花草可命中、
/// 半砖/楼梯按状态盒、空气/水穿透，BaseTorchBlock.java:16 等）；
/// `mode=Collide` 用碰撞形状（第三人称相机遮挡：原版相机 VISUAL 形状
/// 默认即碰撞形状，Camera.java:297 + BlockBehaviour.java:345-347，
/// 栅栏 visual 差异见 blockshapes::RayTarget::Collide 注释）。
/// 全立方固体的命中/法线与旧整格 DDA 逐位一致。
fn dda_hit(
    view: &WorldView,
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
    mode: mcv_game::blockshapes::RayTarget,
) -> Option<(BlockPos, [i32; 3])> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let mut pos = origin.floor().as_ivec3();
    let step = dir.signum().as_ivec3();
    let mut t_max = Vec3::new(
        if dir.x > 0.0 {
            (pos.x as f32 + 1.0 - origin.x) / dir.x
        } else if dir.x < 0.0 {
            (origin.x - pos.x as f32) / -dir.x
        } else {
            f32::INFINITY
        },
        if dir.y > 0.0 {
            (pos.y as f32 + 1.0 - origin.y) / dir.y
        } else if dir.y < 0.0 {
            (origin.y - pos.y as f32) / -dir.y
        } else {
            f32::INFINITY
        },
        if dir.z > 0.0 {
            (pos.z as f32 + 1.0 - origin.z) / dir.z
        } else if dir.z < 0.0 {
            (origin.z - pos.z as f32) / -dir.z
        } else {
            f32::INFINITY
        },
    );
    let t_delta = Vec3::new(1.0 / dir.x.abs(), 1.0 / dir.y.abs(), 1.0 / dir.z.abs());
    let mut normal = [0i32; 3];
    // 射线进入当前格的参数（起始格 = 0）。
    let mut t0 = 0.0f32;
    for _ in 0..64 {
        let bp = BlockPos::new(pos.x, pos.y, pos.z);
        // 本格区间 [t0, 离开本格的 t]（不超出 max_dist）。
        let t1 = t_max.x.min(t_max.y).min(t_max.z).min(max_dist);
        if let Some((th, n)) =
            mcv_game::blockshapes::hit_in_cell(view, bp, origin, dir, t0, t1, mode, normal)
            && th <= max_dist
        {
            return Some((bp, n));
        }
        if t_max.x < t_max.y && t_max.x < t_max.z {
            if t_max.x > max_dist {
                return None;
            }
            t0 = t_max.x;
            pos.x += step.x;
            t_max.x += t_delta.x;
            normal = [-step.x, 0, 0];
        } else if t_max.y < t_max.z {
            if t_max.y > max_dist {
                return None;
            }
            t0 = t_max.y;
            pos.y += step.y;
            t_max.y += t_delta.y;
            normal = [0, -step.y, 0];
        } else {
            if t_max.z > max_dist {
                return None;
            }
            t0 = t_max.z;
            pos.z += step.z;
            t_max.z += t_delta.z;
            normal = [0, 0, -step.z];
        }
    }
    None
}

/// 实体攻击距离（26.1 `Player.DEFAULT_ENTITY_INTERACTION_RANGE = 3.0`，
/// Player.java:133；Attributes.ENTITY_INTERACTION_RANGE 以此为基值）。
pub const ENTITY_ATTACK_RANGE: f32 = 3.0;

/// exhaustion 上限（FoodData.addExhaustion:100-101 `min(x + amount, 40)`）。
pub const EXHAUSTION_MAX: f32 = 40.0;

/// 每破坏一方块的 exhaustion（Block.playerDestroy，Block.java:470-479
/// `causeFoodExhaustion(0.005F)`；FoodConstants.java:23 EXHAUSTION_MINE）。
pub const EXHAUSTION_MINE: f32 = 0.005;

/// 地面水平位移的 exhaustion（26.1 ServerPlayer.checkMovementStatistics:1443-1456
/// 连同 FoodConstants.java:25-27）：冲刺 **0.1/m**、走路/潜行 **0.0/m**，
/// 且只计水平分量。水中 0.01/m（FoodConstants.java:28 EXHAUSTION_SWIM）
/// 未接线，登记为已知差异。
pub fn move_exhaustion(sprinting: bool, horizontal_m: f32) -> f32 {
    if sprinting { 0.1 * horizontal_m } else { 0.0 }
}

/// FoodData.tick 的一拍（26.1 FoodData.java:32-72），由调用方每 20 Hz tick
/// 调一次（非 60 Hz 固定步）：
/// 1. exhaustion **>4**（FoodData.java:35 严格大于，非 >=4）扣 4，先扣 1 点
///    饱和度、饱和见底才扣饥饿；
/// 2. 回血快线：饱和>0 且 hunger≥20 且受伤，每 10 tick 回 min(饱和,6)/6 HP，
///    代价走 exhaustion+min(饱和,6)（FoodData.java:45-52；naturalRegen 游
///    戏规则默认 true，本仓无 gamerules 设施按默认建模）；
/// 3. 回血慢线：hunger≥18 且受伤，每 80 tick 回 1 HP，代价 exhaustion+6
///    （FoodData.java:53-59，FoodConstants.java:20 EXHAUSTION_HEAL=6.0——旧实现
///    回血零代价）；
/// 4. 饥饿掉血：hunger=0 每 80 tick 一拍，难度封顶门 `health>10 || HARD ||
///    (health>1 && NORMAL)`（FoodData.java:63）。命中时返回 true，**由调用
///    方走完整受伤管线**（原版 `player.hurtServer(…starve(), 1.0F)`，
///    FoodData.java:64；旧实现就地 `health -= 1.0` 绕过 i 帧门/死亡结算，
///    已消解）。starve.json exhaustion = 0.0。
pub fn food_data_tick(
    exhaustion: &mut f32,
    saturation: &mut f32,
    hunger: &mut f32,
    health: &mut f32,
    tick_timer: &mut u32,
    difficulty: crate::difficulty::Difficulty,
) -> bool {
    if *exhaustion > 4.0 {
        *exhaustion -= 4.0;
        if *saturation > 0.0 {
            *saturation = (*saturation - 1.0).max(0.0);
        } else if !difficulty.is_peaceful() {
            // 和平只耗饱和不扣饥饿（FoodData.java:39 `else if (difficulty != PEACEFUL)`）。
            *hunger = (*hunger - 1.0).max(0.0);
        }
    }
    let hurt = *health > 0.0 && *health < 20.0;
    if *saturation > 0.0 && hurt && *hunger >= 20.0 {
        *tick_timer += 1;
        if *tick_timer >= 10 {
            let spent = (*saturation).min(6.0);
            *health = (*health + spent / 6.0).min(20.0);
            *exhaustion = (*exhaustion + spent).min(EXHAUSTION_MAX);
            *tick_timer = 0;
        }
    } else if *hunger >= 18.0 && hurt {
        *tick_timer += 1;
        if *tick_timer >= 80 {
            *health = (*health + 1.0).min(20.0);
            *exhaustion = (*exhaustion + 6.0).min(EXHAUSTION_MAX);
            *tick_timer = 0;
        }
    } else if *hunger <= 0.0 {
        *tick_timer += 1;
        if *tick_timer >= 80 {
            // 封顶表（FoodData.java:63 `health > 10 || HARD || (health > 1 && NORMAL)`，
            // 和平/简单只掉到 10）——旧「一律封顶 10」登记偏差已消解。
            let starve = crate::difficulty::starve_can_hurt(*health, difficulty);
            *tick_timer = 0;
            return starve; // 伤害由调用方走 hurt 管线（FoodData.java:64）。
        }
    } else {
        *tick_timer = 0;
    }
    false
}

/// 满气（26.1 `Entity.getMaxAirSupply` = **300**，Entity.java:2739-2741；
/// 即派单「水下 air 供给 300 tick」的原版出处）。
pub const MAX_AIR_SUPPLY: i32 = 300;

/// 每 game tick 的空气供给结算（26.1 `LivingEntity.baseTick` 水门分支，
/// LivingEntity.java:417-439；`shouldTakeDrowningDamage` :487-489；
/// decrease/increaseAirSupply :565-579）。返回 true = 本 tick 触发溺水
/// 伤害（原版 `damageSources().drown(), 2.0F`，:428）。
///
/// - 眼下水 + 可溺水：供给 −1/tick（`decreaseAirSupply`，OXYGEN_BONUS
///   属性默认 0 → 无跳过概率，:565-575）；`air ≤ −20` 时伤害并把供给
///   清回 0（:425-429）→ 满气后第 320 tick 首伤、之后每 20 tick 一伤。
/// - 眼下水 + 不可溺水（创造 `abilities.invulnerable`，:422-423）：
///   不减、不伤、**也不回**（原版水下回气只有水下呼吸药水分支 :430-432）。
/// - 眼未下水：+4/tick 恢复到上限（`increaseAirSupply` :577-579）。
pub fn air_supply_tick(air: &mut i32, eyes_in_water: bool, can_drown: bool) -> bool {
    if eyes_in_water {
        if !can_drown {
            return false;
        }
        *air -= 1;
        if *air <= -20 {
            // LivingEntity.java:425-429（清 0 + broadcastEvent(67) + hurt 2.0）。
            *air = 0;
            return true;
        }
        false
    } else if *air < MAX_AIR_SUPPLY {
        *air = (*air + 4).min(MAX_AIR_SUPPLY);
        false
    } else {
        false
    }
}

/// 游泳姿态状态机（26.1 `Entity.updateSwimming`，Entity.java:1558-1564；
/// 创造飞行恒关 `Player.updateSwimming`，Player.java:1410-1416）。
/// 原版判据（`isPassenger` 恒假，本仓无骑乘）：
/// - 维持：`sprinting && isInWater`（身体触水即可，不必没顶）；
/// - 起步：`sprinting && isUnderWater`（=wasEyeInWater && isInWater，
///   Entity.java:1536-1538）`&& 脚下格流体为水`（blockPosition 的
///   FluidState is WATER，:1562-1563）。
///
/// `sprinting` 传带饥饿门的冲刺态（LocalPlayer.java:1133 → Player.java
/// :1569-1571，GameRuntime 侧已算）。
pub fn swimming_tick(
    prev: bool,
    sprinting: bool,
    in_water: bool,
    under_water: bool,
    feet_water: bool,
    flying: bool,
) -> bool {
    if flying {
        return false;
    }
    if prev {
        sprinting && in_water
    } else {
        sprinting && under_water && feet_water
    }
}

/// 雨粒子半径（26.1 客户端设置 `weatherRadius` 默认 **10**、范围 3..10，
/// Options.java:178-184；调用点 LevelRenderer.java:1164。本仓无该设置
/// 界面，直取默认值——设置接线登记遗留）。
pub const RAIN_PARTICLE_RADIUS: i32 = 10;

/// 雨粒子每 tick 生成数（`WeatherEffectRenderer.tickRainParticles`，
/// WeatherEffectRenderer.java:232：`count = (int)(0.225·(2r+1)²·rainLevel²)`；
/// ParticleStatus 减半分支 :231-232 本仓无粒子质量设置，恒 FULL）。
pub fn rain_particle_count(rain_level: f32, radius: i32) -> u32 {
    if rain_level <= 0.0 {
        return 0;
    }
    let diameter = 2 * radius + 1;
    let area = (diameter * diameter) as f32;
    (0.225 * area * rain_level * rain_level) as u32
}

/// 射线 vs AABB（slab 法）：返回原点到入射点的距离（原点在盒内取 0）。
/// `dir` 必须已归一化；盒在射线背后或不相交返回 None。
pub fn ray_aabb_t(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut t_min = 0.0f32;
    let mut t_max = f32::INFINITY;
    for (o, d, lo, hi) in [
        (origin.x, dir.x, min.x, max.x),
        (origin.y, dir.y, min.y, max.y),
        (origin.z, dir.z, min.z, max.z),
    ] {
        if d.abs() < 1e-9 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let mut t0 = (lo - o) * inv;
        let mut t1 = (hi - o) * inv;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        t_min = t_min.max(t0);
        t_max = t_max.min(t1);
        if t_min > t_max {
            return None;
        }
    }
    Some(t_min)
}

/// 视线第一个方块命中的**距离**：经模块内私有 DDA `dda_hit`（仅 solid 遮挡，
/// 水/花不挡刀）给出命中格与入射面法线，再按入射面求精确入射距离 t。
/// 给出命中格与入射面法线，再按入射面求精确入射距离 t。
pub fn block_hit_t(view: &WorldView, eye: Vec3, dir: Vec3, max_dist: f32) -> Option<f32> {
    let dir = dir.normalize_or_zero();
    // 攻击遮挡判据 = 碰撞形状：水/花草/火把无碰撞盒 → 不挡刀，与
    // 本函数"仅 solid 遮挡"语义一致（Collide 系 blockshapes 形状盒）。
    let (hit, normal) = dda_hit(
        view,
        eye,
        dir,
        max_dist,
        mcv_game::blockshapes::RayTarget::Collide,
    )?;
    let mut t = 0.0f32;
    // dda_hit 的 normal = -step：正向步进从 min 面入射（平面 = 格坐标），
    // 负向从 max 面入射（平面 = 格坐标 + 1）；起点即命中的退化情形法线全 0
    // （眼在方块内），t 保持 0。
    let axes = [
        (normal[0], hit.x as f32, eye.x, dir.x),
        (normal[1], hit.y as f32, eye.y, dir.y),
        (normal[2], hit.z as f32, eye.z, dir.z),
    ];
    for (n, base, o, d) in axes {
        if n == 0 {
            continue;
        }
        let plane = base + if n > 0 { 1.0 } else { 0.0 };
        t = (plane - o) / d;
    }
    if t.is_finite() && (0.0..=max_dist).contains(&t) {
        Some(t)
    } else {
        None
    }
}

/// 实体攻击目标选取（try_attack 的等价纯函数，供集成测试）：`mobs` =
/// （脚底中心，[`mcv_entity::defs::MobDef::half_size`]），AABB 与 step_entity
/// 同型（高 = 2·hy）。入选条件 = ray-AABB 命中距离 ≤ [`ENTITY_ATTACK_RANGE`]
/// **且** 严格小于方块命中距离 `block_t`（None = 视线无方块），取最近者，
/// 返回 (下标, 命中距离)。
pub fn pick_attack_target(
    eye: Vec3,
    dir: Vec3,
    mobs: &[(Vec3, [f32; 3])],
    block_t: Option<f32>,
) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    for (i, (pos, half)) in mobs.iter().enumerate() {
        let min = Vec3::new(pos.x - half[0], pos.y, pos.z - half[2]);
        let max = Vec3::new(pos.x + half[0], pos.y + 2.0 * half[1], pos.z + half[2]);
        let Some(t) = ray_aabb_t(eye, dir, min, max) else {
            continue;
        };
        if t > ENTITY_ATTACK_RANGE {
            continue;
        }
        // 方块遮挡：实体入射点不比方块命中点近 → 挡墙落空。
        if block_t.is_some_and(|bt| t >= bt) {
            continue;
        }
        if best.is_none_or(|(_, d)| t < d) {
            best = Some((i, t));
        }
    }
    best
}

/// 方块 id → 原版 SoundType：千块表注册名 → mcv_audio 全表二分
/// （26.1 Blocks.java `.sound(SoundType.X)` 归属；未登记名/越界 id 回落 STONE，
/// BlockBehaviour.java:973 `Properties` 默认）。
fn sound_type_of(vid: u16) -> &'static mcv_audio::SoundType {
    let i = (vid & mcv_core::ID_MASK) as usize;
    let name = mcv_core::BLOCKS.get(i).map_or("no_such_block", |d| d.name);
    mcv_audio::sound_type_for_block(name)
}

/// 原版步距门纯函数（Entity.java:820 触发门 `moveDist > nextStep` +
/// :1188-1190 阈值更新 `nextStep = (int)moveDist + 1`；moveDist 由调用侧按
/// 水平位移 × 0.6 累计，:812-813）。返回 `Some(新阈值)` = 本 tick 触发一步；
/// `None` = 未过门（等值不触发，严格 `>`）。
pub fn step_gate(step_dist: f32, next_step: f32) -> Option<f32> {
    (step_dist > next_step).then(|| step_dist.trunc().max(0.0) + 1.0)
}

#[cfg(test)]
mod placement_tests {
    use super::placement_state;
    use mcv_core::Shape;

    #[test]
    fn slab_top_bit() {
        // 点顶面（法线 +Y）→ 下半砖；点底面 → 上半砖；潜行翻转。
        assert_eq!(placement_state(Shape::Slab, [0, 1, 0], false, 0.0), 0);
        assert_eq!(placement_state(Shape::Slab, [0, -1, 0], false, 0.0), 1);
        assert_eq!(placement_state(Shape::Slab, [0, 1, 0], true, 0.0), 1);
        assert_eq!(placement_state(Shape::Slab, [1, 0, 0], false, 0.0), 0);
    }

    /// 三段贯通之一（placement→nibble）：26.1 StairBlock.java:101-102
    /// FACING=视线同向 + 103-105 DOWN→TOP/UP→BOTTOM。nibble 编码
    /// 0=+Z 1=-Z 2=+X 3=-X，几何（踏步半盒在朝向侧）断言见
    /// game/mcv_mesher/tests/mesh.rs::stairs_facing_and_top_flip——
    /// 玩家面向 -Z（yaw=0）放置 → nibble facing=1 → 几何踏步占 -Z 半格。
    #[test]
    fn stairs_facing_follows_vanilla() {
        // yaw=0 视线 (sin0, -cos0)=(0,-1) 即 -Z → facing=1；
        // 点顶面(+Y 法线)→下半（bit2=0）。
        assert_eq!(placement_state(Shape::Stairs, [0, 1, 0], false, 0.0), 1);
        // yaw=π/2 视线 (+1,0) 即 +X → facing=2。
        assert_eq!(
            placement_state(Shape::Stairs, [0, 1, 0], false, std::f32::consts::FRAC_PI_2),
            2
        );
        // yaw=π 视线 (0,+1) 即 +Z → facing=0。
        assert_eq!(
            placement_state(Shape::Stairs, [0, 1, 0], false, std::f32::consts::PI) & 3,
            0
        );
        // 点底面（法线 -Y）→ 上半（bit2=1），与半砖规则同向。
        assert_eq!(
            placement_state(Shape::Stairs, [0, -1, 0], false, 0.0),
            1 | 4
        );
        // 水平面：DDA 无格内点击点，近似下半（KNOWN-DIVERGENCE，同半砖）。
        assert_eq!(placement_state(Shape::Stairs, [0, 0, 1], false, 0.0) & 4, 0);
        assert_eq!(placement_state(Shape::Stairs, [1, 0, 0], false, 0.0) & 4, 0);
        // 潜行 XOR 翻转上下（KNOWN-DIVERGENCE：原版无潜行逻辑，保留现状）。
        assert_eq!(placement_state(Shape::Stairs, [0, 1, 0], true, 0.0), 1 | 4);
    }

    #[test]
    fn other_shapes_zero() {
        for s in [Shape::Cube, Shape::Cross, Shape::Torch, Shape::Fence] {
            assert_eq!(placement_state(s, [0, -1, 0], true, 1.23), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STONE: u16 = 1;
    /// 千块表 id 1009 = "torch"（26.1 发光 14；damp=0，见 mcv_core::OPACITY）。
    const TORCH: u16 = 1009;

    /// #150 图标解析同源锁：三处消费（HUD/手持/掉落物）共用 item_icon，
    /// 方块物品 → Block(方块 id 透传)、材料物品 → Sprite(物品名)、空堆 →
    /// Empty。hand_item/build_hud 改走本函数后，uv/名解析不许分叉。
    #[test]
    fn item_icon_resolves_block_material_and_empty() {
        use mcv_render::gpu::HandItem;
        // 圆石 = 方块物品（ItemKind::Block(BlockId(9))，registry 序 27）。
        assert_eq!(
            item_icon(&mcv_item::ItemStack::new(mcv_item::COBBLESTONE, 1)),
            HandItem::Block(9)
        );
        // 木棍 = 材料物品 → 精灵名（GUI 表条目名 = mcv_item::ItemDef.name）。
        assert_eq!(
            item_icon(&mcv_item::ItemStack::new(mcv_item::STICK, 1)),
            HandItem::Sprite("stick")
        );
        // 空堆 → Empty（手持只画手臂、HUD 槽跳过、掉落物不画）。
        assert_eq!(item_icon(&mcv_item::ItemStack::empty()), HandItem::Empty);
        // 手持物与图标解析同源：hand_item 是 item_icon(选中槽) 的直通。
        // （GameRuntime 无头构造在本 mod 不可达——同源关系由
        // tests/runtime.rs 的快照测试与该 pub fn 的单行实现锁住。）
    }

    // v6：绝对 y 域，索引公式统一走 mcv_core::vidx（旧 y<<8|z<<4|x）。
    fn lidx(x: usize, y: usize, z: usize) -> usize {
        mcv_core::vidx(x, y as i32, z)
    }

    /// 无头拼装一个已完成本地布光的区块（镜像主循环 init 阶段的效果；
    /// GameRuntime::new 需要 GPU MeshUploader，无头测试走 relight_block_edit
    /// 这条纯数据接缝——place/destroy 两条游戏路径调用的就是它）。
    fn lit_chunk(x: i32, z: i32, floor_top: usize) -> Arc<ChunkHandle> {
        let handle = Arc::new(ChunkHandle::new(ChunkPos::new(x, z)));
        {
            let mut vg = handle.voxels.write().unwrap();
            for y in 0..=floor_top {
                for zz in 0..16usize {
                    for xx in 0..16usize {
                        vg[lidx(xx, y, zz)] = BlockId(STONE);
                    }
                }
            }
        }
        let hm = mcv_worldgen::recompute_heightmap(bytemuck::cast_slice(
            handle.voxels.read().unwrap().as_slice(),
        ));
        *handle.heightmap.write().unwrap() = hm;
        {
            let vg = handle.voxels.read().unwrap();
            let mut lg = handle.light.write().unwrap();
            let hg = handle.heightmap.read().unwrap();
            let voxels: &[u16] = bytemuck::cast_slice(vg.as_slice());
            let mut view = mcv_light::LightChunk {
                voxels,
                light: &mut lg[..],
                heightmap: &hg[..],
            };
            mcv_light::init(&mut view);
        }
        handle.advance_to(Stage::LightLocalReady);
        handle
    }

    fn blk(h: &ChunkHandle, x: usize, y: usize, z: usize) -> u8 {
        h.light.read().unwrap()[lidx(x, y, z)] & 0xF
    }

    /// 模拟游戏路径：调用方写体素 → relight_block_edit 接线（同 interact/
    /// destroy_block 的调用序）。游戏路径边派发延后到 stream 消化；测试
    /// 断言的是收敛终态，这里就地排空队列（同一收敛，仅时序折叠）。
    fn edit(chunks: &HashMap<ChunkPos, Arc<ChunkHandle>>, at: BlockPos, old: u16, new: u16) {
        let h = &chunks[&at.chunk()];
        let [lx, _, lz] = at.local();
        h.voxels.write().unwrap()[mcv_core::vidx(lx, at.y, lz)] = BlockId(new);
        let mut queue = Vec::new();
        relight_block_edit(chunks, at, old, new, &mut queue);
        sync_light_edges(chunks, &mut queue);
        assert!(
            queue.is_empty(),
            "测试排空后不应残留边任务（预算未耗尽前提）"
        );
    }

    #[test]
    fn place_torch_then_dig_updates_block_light() {
        let mut chunks = HashMap::new();
        chunks.insert(ChunkPos::new(0, 0), lit_chunk(0, 0, 20));
        let h = &chunks[&ChunkPos::new(0, 0)];
        let torch_at = BlockPos::new(8, 21, 8);
        assert_eq!(blk(h, 9, 21, 8), 0, "放火把前邻格无方块光");
        // 放火把：邻格 = 14 - max(1, damp(空气)=0) = 13（六向同规，
        // LightEngine.java:77-79）。
        edit(&chunks, torch_at, 0, TORCH);
        assert_eq!(blk(h, 8, 21, 8), 14, "火把格自发光 14");
        assert_eq!(blk(h, 9, 21, 8), 13, "放火把后邻格方块光升高");
        // 挖掉：removal 波清空后不得留残光。
        edit(&chunks, torch_at, TORCH, 0);
        assert_eq!(blk(h, 9, 21, 8), 0, "挖掉后邻格方块光回落");
        assert_eq!(blk(h, 8, 21, 8), 0, "挖掉后火把格熄灭");
    }

    #[test]
    fn torch_light_crosses_chunk_border() {
        let mut chunks = HashMap::new();
        chunks.insert(ChunkPos::new(0, 0), lit_chunk(0, 0, 20));
        chunks.insert(ChunkPos::new(1, 0), lit_chunk(1, 0, 20));
        let a = &chunks[&ChunkPos::new(0, 0)];
        let b = &chunks[&ChunkPos::new(1, 0)];
        // 火把贴着 chunk(0,0) 的 +X 边界（x=15）。
        let torch_at = BlockPos::new(15, 21, 8);
        assert_eq!(blk(b, 0, 21, 8), 0, "同步前邻块跨界格无光");
        edit(&chunks, torch_at, 0, TORCH);
        assert_eq!(blk(a, 15, 21, 8), 14, "火把格自发光");
        assert_eq!(blk(b, 0, 21, 8), 13, "边同步：光跨过区块边界");
        assert_eq!(blk(b, 1, 21, 8), 12, "边同步后邻块内部继续衰减");
        edit(&chunks, torch_at, TORCH, 0);
        assert_eq!(blk(b, 0, 21, 8), 0, "挖掉后跨界光被 REMOVE 撤回");
        assert_eq!(blk(b, 1, 21, 8), 0, "撤回不留残光");
    }

    #[test]
    fn block_edit_maintains_heightmap() {
        let mut chunks = HashMap::new();
        chunks.insert(ChunkPos::new(0, 0), lit_chunk(0, 0, 20));
        let h = &chunks[&ChunkPos::new(0, 0)];
        let col = (8usize << 4) | 8usize;
        assert_eq!(h.heightmap.read().unwrap()[col], 21, "初始地表高=20+1");
        edit(&chunks, BlockPos::new(8, 30, 8), 0, STONE);
        assert_eq!(
            h.heightmap.read().unwrap()[col],
            31,
            "放置后 heightmap 抬升"
        );
        edit(&chunks, BlockPos::new(8, 30, 8), STONE, 0);
        assert_eq!(
            h.heightmap.read().unwrap()[col],
            21,
            "挖掉后 heightmap 回落"
        );
    }

    // ---- 加载态（26.1 LevelLoadingScreen / LevelLoadTracker）----

    /// 无头运行时 + 唯一临时存档目录（并行安全，模式照抄 tests/runtime.rs）。
    fn headless_rt(tag: &str) -> GameRuntime {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (tag, std::process::id(), std::time::SystemTime::now()).hash(&mut h);
        let dir = std::env::temp_dir().join(format!("mcv-load-{}-{:x}", tag, h.finish()));
        GameRuntime::new_headless(20261010, dir, GameMode::Survival)
    }

    // ---- E0 帧预算分段计时（engine-roadmap.md §3-E0：观测先行）----

    /// E0 验收面（纯函数侧）：分段结构体覆盖全部段名（段序 = 帧内执行序，
    /// 蓝图 §2）、Debug 形态携带段名（日志汇总结构的可序列化面）、数值
    /// 非负且逐段饱和累加单调可累计（不回绕）；1Hz 汇总器 record/mean
    /// 语义（帧数、和、单帧峰值、均值 = sum/frames）。
    #[test]
    fn frame_timing_segments_named_and_accumulable() {
        assert_eq!(
            FrameTiming::SEGMENTS,
            [
                "fixed_step",
                "ecs",
                "stream",
                "init",
                "edge",
                "io",
                "remesh",
                "flush",
                "draw"
            ]
        );
        let dbg = format!("{:?}", FrameTiming::default());
        for name in FrameTiming::SEGMENTS {
            assert!(dbg.contains(name), "Debug 缺段名 {name}: {dbg}");
        }
        // 逐段饱和累加：单调不减、不回绕（u64 域数值非负由类型保证）。
        let a = FrameTiming {
            fixed_step_us: 5,
            stream_us: 7,
            ..Default::default()
        };
        let b = a.saturating_add(FrameTiming {
            fixed_step_us: u64::MAX,
            stream_us: 1,
            ..Default::default()
        });
        assert!(b.fixed_step_us >= a.fixed_step_us);
        assert!(b.stream_us >= a.stream_us);
        // 1Hz 汇总：record 归并帧；峰值逐段取 max；均值 = sum/frames。
        let mut stats = FrameTimingStats::default();
        stats.record(FrameTiming {
            stream_us: 100,
            init_us: 10,
            ..Default::default()
        });
        stats.record(FrameTiming {
            stream_us: 300,
            edge_us: 40,
            ..Default::default()
        });
        assert_eq!(stats.frames, 2);
        assert_eq!(stats.sum.stream_us, 400);
        assert_eq!(stats.max.stream_us, 300);
        assert_eq!(stats.max.init_us, 10);
        assert_eq!(stats.max.edge_us, 40);
        assert_eq!(stats.mean_us().stream_us, 200);
        assert_eq!(stats.mean_us().edge_us, 20);
        // 零帧均值恒零（无除零）。
        assert_eq!(
            FrameTimingStats::default().mean_us(),
            FrameTiming::default()
        );
    }

    /// E0 验收面（接线侧）：headless 帧循环里分段计时可观测——
    /// fixed_step/stream 各段被记账、子段 ⊆ 总段（同一 Instant 排尺的
    /// 窗口包含关系）、帧界归并后 1Hz 取走即清零。
    #[test]
    fn frame_timing_measured_in_headless_frames() {
        let mut rt = headless_rt("e0-timing");
        for _ in 0..5 {
            rt.fixed_step(1.0 / 60.0);
        }
        rt.stream();
        // 子段 ⊆ 总段：ecs ⊆ fixed_step；stream 五子段之和 ⊆ stream。
        assert!(rt.timing_frame.ecs_us <= rt.timing_frame.fixed_step_us);
        let sub = rt.timing_frame.init_us
            + rt.timing_frame.edge_us
            + rt.timing_frame.io_us
            + rt.timing_frame.remesh_us
            + rt.timing_frame.flush_us;
        assert!(sub <= rt.timing_frame.stream_us);
        // 帧界归并 → 1Hz 取走（累计器清零）。
        rt.end_frame_timing();
        let stats = rt.take_frame_timing_stats();
        assert_eq!(stats.frames, 1);
        assert!(stats.sum.fixed_step_us > 0, "fixed_step 段必须被记账");
        assert!(stats.sum.stream_us > 0, "stream 段必须被记账");
        let again = rt.take_frame_timing_stats();
        assert_eq!(again.frames, 0);
        assert_eq!(again.sum, FrameTiming::default());
    }

    /// 把玩家摆到地表并把出生点邻域（半径 3，49 块 = 26.1
    /// EXPECTED_PLAYER_CHUNKS = Mth.square(7)，LevelLoadProgressTracker
    /// .java:15）全部顶到 Uploaded。
    fn fill_neighborhood(rt: &mut GameRuntime) {
        rt.player.pos = Vec3::new(8.5, 71.0, 8.5);
        rt.player.vel = Vec3::ZERO;
        for dx in -3i32..=3 {
            for dz in -3i32..=3 {
                let pos = ChunkPos::new(dx, dz);
                rt.chunks
                    .entry(pos)
                    .or_insert_with(|| lit_chunk(dx, dz, 69));
                rt.chunks[&pos].advance_to(Stage::Uploaded);
                // 刀 3b 门谓词解耦（stage==Uploaded → status ≥ Surface）后，
                // 手工 fixture 必须双记账位同步置位（生产管线中 advance_
                // status_by_level 每 stream 帧按票级别授权推进；本 fixture
                // 绕过 stream，需镜像真实终态：级别 ≤33 邻域齐 → Full）。
                rt.chunks[&pos].advance_status(ChunkStatus::Full);
            }
        }
    }

    // ---- 刀 4a：sim_safe_radius 谓词重接（status≥Full + 票环上界）----

    /// 旧实现复刻（刀 3 及更早：谓词 = stage≥TerrainReady、上界 =
    /// unload_dist、全方环重扫形状）——仅供对拍测试引用。
    fn legacy_sim_safe_scan(rt: &GameRuntime, pc: ChunkPos) -> i32 {
        let ready = |c: ChunkPos| {
            rt.chunks
                .get(&c)
                .is_some_and(|h| (h.stage() as u8) >= (Stage::TerrainReady as u8))
        };
        if !ready(pc) {
            return -1;
        }
        let max_r = rt.unload_dist();
        let mut r = 0;
        while r < max_r {
            let n = r + 1;
            let all =
                (-n..=n).all(|dx| (-n..=n).all(|dz| ready(ChunkPos::new(pc.x + dx, pc.z + dz))));
            if !all {
                break;
            }
            r = n;
        }
        r
    }

    /// 确定性 LCG（对拍可复现，不引 rand 依赖）。
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// 新旧 sim_safe 对拍（随机区块位形）。合法性（soundness）：新 ≤ 旧
    /// ——新 ready 集（status≥Full）是旧证据集（stage≥TerrainReady）的
    /// 子集（生产不变量：Full 推进要求 stage ≥ LightLocalReady），且票环
    /// 上界 sim_dist+2 比旧 unload_dist 紧一环——新实现永不比旧实现
    /// 「多声称」就位环（无新增真空触达）。new < old 时差异必须落入两个
    /// 显式差异面之一（无未解释缝隙）：谓词收紧面 = 环 ≤ old 内存在
    /// 「stage 已就绪但 status < Full」的块（worldgen 完成、证据链未齐/
    /// 未授权的过渡态）；票环上界面 = old > sim_dist + (FULL −
    /// ENTITY_TICKING)，即旧扫描可触及 level=34 晕圈环（仅授权 Light），
    /// 票 Full 授权域不含它。
    #[test]
    fn sim_safe_ticket_predicate_is_sound_subset_of_legacy_scan() {
        for seed in [1u64, 0xDEAD_BEEF, 2026_1011, 777] {
            for &rd in &[2i32, 4, 8] {
                let mut rt = headless_rt("safecmp");
                rt.render_dist = rd;
                let pc = ChunkPos::new(13, -5);
                let mut rng = Lcg(seed);
                let reach = rd + 5; // 覆盖旧上界（rd+4）外一环
                for dx in -reach..=reach {
                    for dz in -reach..=reach {
                        if rng.below(100) < 30 {
                            continue; // 30% 缺席
                        }
                        let pos = ChunkPos::new(pc.x + dx, pc.z + dz);
                        let h = Arc::new(ChunkHandle::new(pos));
                        match rng.below(100) {
                            0..=59 => {}
                            60..=89 => h.advance_to(Stage::TerrainReady),
                            _ => h.advance_to(Stage::LightLocalReady),
                        }
                        // status 生成保持生产 plausible：Full 只落在
                        // LightLocalReady 之上（advance_status_by_level 的
                        // 证据门），避免测试自造生产不可达位形。
                        match rng.below(100) {
                            0..=39 => {}
                            40..=69 => h.advance_status(ChunkStatus::Surface),
                            70..=89 => h.advance_status(ChunkStatus::Light),
                            _ => {
                                if (h.stage() as u8) >= (Stage::LightLocalReady as u8) {
                                    h.advance_status(ChunkStatus::Full);
                                }
                            }
                        }
                        rt.chunks.insert(pos, h);
                    }
                }
                let new_r = rt.sim_safe_radius(pc);
                let old_r = legacy_sim_safe_scan(&rt, pc);
                assert!(
                    new_r <= old_r,
                    "seed={seed} rd={rd}: 新 {new_r} > 旧 {old_r}，新实现不得超出旧证据集"
                );
                if new_r < old_r {
                    let cap = rt.sim_dist() + (ticket::FULL_LEVEL - ticket::ENTITY_TICKING_LEVEL);
                    let cap_face = old_r > cap;
                    let mut tightening_witness = false;
                    'find: for dx in -old_r..=old_r {
                        for dz in -old_r..=old_r {
                            if let Some(h) = rt.chunks.get(&ChunkPos::new(pc.x + dx, pc.z + dz))
                                && (h.stage() as u8) >= (Stage::TerrainReady as u8)
                                && (h.status() as u8) < (ChunkStatus::Full as u8)
                            {
                                tightening_witness = true;
                                break 'find;
                            }
                        }
                    }
                    assert!(
                        cap_face || tightening_witness,
                        "seed={seed} rd={rd}: 缝隙 new={new_r} old={old_r} 不属任何登记差异面"
                    );
                }
            }
        }
    }

    /// -1 整步冻结语义（刀 4a 保留的计划锁死项）：本块缺席或体素已备但
    /// status 未推进 → -1；fixed_step 在 -1 上整步冻结（不位移、不坠）。
    /// 谓词收紧面（TerrainReady 不再放行）单独断言——证据链未齐就是未就绪。
    #[test]
    fn sim_safe_freeze_on_missing_or_unproven_own_chunk() {
        let mut rt = headless_rt("freeze4a");
        rt.phase = GamePhase::Playing;
        rt.player.pos = Vec3::new(8.5, 70.0, 8.5);
        rt.player.vel = Vec3::ZERO;
        let pc = ChunkPos::new(0, 0);
        assert_eq!(rt.sim_safe_radius(pc), -1, "区块缺席必须 -1");
        let h = Arc::new(ChunkHandle::new(pc));
        h.advance_to(Stage::TerrainReady);
        rt.chunks.insert(pc, h);
        assert_eq!(
            rt.sim_safe_radius(pc),
            -1,
            "TerrainReady 但 status 未推进：证据链未齐，谓词收紧后仍 -1"
        );
        rt.input.forward = true;
        let before = rt.player.pos;
        for _ in 0..60 {
            rt.fixed_step(1.0 / 60.0);
        }
        assert_eq!(
            rt.player.pos, before,
            "-1 必须整步冻结（隐形墙/坠虚空前哨）"
        );
        rt.chunks[&pc].advance_status(ChunkStatus::Full);
        assert_eq!(
            rt.sim_safe_radius(pc),
            0,
            "本块 Full 后解冻，至少本块可动（邻环缺席截断）"
        );
    }

    /// 刀 4b：钳制矩形由票覆盖环派生。cap = sim_dist + (FULL −
    /// ENTITY_TICKING)（票级别场 Full 授权域半径，与 sim_safe_radius 上界
    /// 同源）；m = max(min(r_safe, cap) − 1, 0)——票授权 × 证据环取小。
    /// 边界：r_safe=1 → 矩形=本块（pad=半宽+余量）、r_safe=0 → pad 收
    /// 1.0、cap 绑定（r_safe 超票域被压回）、被钳轴清速/余轴保速、钳后
    /// 玩家块满足新 ready 判据（status≥Full 前哨，fixture 全 Full 兜住）。
    #[test]
    fn clamp_rect_derives_from_ticket_full_domain() {
        let mut rt = headless_rt("clamp4b");
        rt.render_dist = 4; // vd=5 → cap = 5 + 2 = 7
        for cx in -9..=9 {
            let pos = ChunkPos::new(cx, 0);
            let h = Arc::new(ChunkHandle::new(pos));
            h.advance_to(Stage::TerrainReady);
            h.advance_status(ChunkStatus::Full);
            rt.chunks.insert(pos, h);
        }
        let pad = mcv_game::Player::HALF[0] + 0.02;
        let pc = ChunkPos::new(0, 0);

        // r_safe=1 → m=0：矩形=本块，x 钳回 16−pad、vel.x 清零；z 未触
        // 钳位（8.5 ∈ [pad, 16−pad]），vel.z 保留。
        rt.player.pos = Vec3::new(15.9, 70.0, 8.5);
        rt.player.vel = Vec3::new(9.0, 0.0, 3.0);
        rt.clamp_to_sim_area(pc, 1);
        assert_eq!(rt.player.pos.x, 16.0 - pad, "r_safe=1 右边界 = 16−pad");
        assert_eq!(rt.player.vel.x, 0.0, "被钳轴清速");
        assert_eq!(rt.player.vel.z, 3.0, "未钳轴保速");
        assert_eq!(rt.player.pos.y, 70.0, "钳制不动 y");

        // 左边界同理：lo = 0+pad。
        rt.player.pos = Vec3::new(0.1, 70.0, 8.5);
        rt.player.vel = Vec3::new(-9.0, 0.0, 0.0);
        rt.clamp_to_sim_area(pc, 1);
        assert_eq!(rt.player.pos.x, pad, "r_safe=1 左边界 = pad");

        // r_safe=2 → m=1：矩形=pc±1，右界 = 32−pad。
        rt.player.pos = Vec3::new(31.9, 70.0, 8.5);
        rt.clamp_to_sim_area(pc, 2);
        assert_eq!(rt.player.pos.x, 32.0 - pad, "r_safe=2 → 矩形 pc±1");

        // r_safe=0：pad 收 1.0 格（仅本块就位，触达不出本块）。
        rt.player.pos = Vec3::new(15.9, 70.0, 8.5);
        rt.clamp_to_sim_area(pc, 0);
        assert_eq!(rt.player.pos.x, 15.0, "r_safe=0 → pad=1.0");

        // cap 绑定：r_safe=100 超出票 Full 授权域（cap=7）→ m=6，
        // 与 r_safe=7 的矩形一致（票环派生防线纵深）。
        rt.player.pos = Vec3::new(150.0, 70.0, 8.5);
        rt.clamp_to_sim_area(pc, 100);
        let capped_hi = rt.player.pos.x;
        assert_eq!(capped_hi, 112.0 - pad, "cap=7 → m=6 → 右界 112−pad");
        rt.player.pos = Vec3::new(150.0, 70.0, 8.5);
        rt.clamp_to_sim_area(pc, 7);
        assert_eq!(
            rt.player.pos.x, capped_hi,
            "r_safe 超票域被压回 cap，与 r_safe=cap 同矩形"
        );
        // 钳后玩家块 = (6,0)（111.68/16=6.98）：fixture 已 Full，debug
        // 前哨（钳后 status≥Full）在此放行。
        assert_eq!(
            (rt.player.pos.x / 16.0).floor() as i32,
            6,
            "钳位落点在票覆盖域内"
        );
    }

    // ---- 模拟距离实体门（26.1 ServerLevel.java:418-419）----
    // 原版语义：实体仅当所在区块在 PLAYER_SIMULATION 票级别视图 ≤
    // ENTITY_TICKING（DistanceManager.java:136-138，环 = 距玩家锚点
    // Chebyshev ≤ sim_dist）才 tick；环外 = 冻结（不 AI、不位移、计时器
    // 不走——invulnerableTime 递减也在 tick 内，LivingEntity.java:452-453），
    // **不删除不卸载**（用户语：「超出就会暂停」）；checkDespawn 在环门外
    // 照跑（ServerLevel.java:417-419 + Mob.java:655-679）。旧门只看
    // 「自身区块 stage ≥ TerrainReady」（已载代理）——迟滞带（sim_dist <
    // d ≤ sim_dist+3，仍在载）里的怪照常 tick，与原版不符。

    /// rd=4 → sim_dist=5：实体环 = Chebyshev ≤5；迟滞带 d∈(5,8]（仍在
    /// 载）；刷怪冷却关死，断言只面向手工放置的实体。`spawned=true` 关死
    /// 出生投放（防后台 worker 生成 (0,0) 后投放挪动玩家锚点——本组测试
    /// 锚点必须恒定：环内/环外由玩家块对怪块的 Chebyshev 距离决定）。
    fn sim_rt(tag: &str) -> GameRuntime {
        let mut rt = headless_rt(tag);
        rt.render_dist = 4;
        rt.spawn_cooldown = u32::MAX;
        rt.spawned = true;
        rt
    }

    /// 环内（d=3 ≤ 5）：动物照常 tick——noActionTime 按 20 Hz 递增
    /// （Mob.java:683 serverAiStep，本仓 mob_ai 的 idle 账）。
    #[test]
    fn mob_inside_simulation_ring_ticks() {
        let mut rt = sim_rt("sim-in");
        rt.chunks.insert(ChunkPos::new(3, 0), lit_chunk(3, 0, 20));
        let cow = spawn_mob(
            &mut rt.mobs_app.world,
            MobId::COW,
            Vec3::new(3.0 * 16.0 + 8.5, 25.0, 8.5),
        );
        rt.stream(); // 票表锚 = 玩家块 (0,0)（pos=ZERO 待投放哨兵）
        rt.fixed_step(1.0 / 20.0);
        let ticks = rt.mobs_app.world.read::<MobTicks>();
        let idle = ticks.get(cow).expect("环内动物在册").idle_ticks;
        assert!(
            idle >= 1,
            "环内动物每 20 Hz tick 推进 noActionTime（≥1），实得 {idle}"
        );
    }

    /// 环外迟滞带（d=7 > 5；块仍在载且已亮）：冻结——不位移、idle 不增、
    /// **不删除**（暂停不消失；距玩家 112 格 > 32 且 idle=0，checkDespawn
    /// 也不应移除它）。
    #[test]
    fn mob_outside_simulation_ring_frozen_but_kept() {
        let mut rt = sim_rt("sim-out");
        rt.chunks.insert(ChunkPos::new(7, 0), lit_chunk(7, 0, 20));
        let cow = spawn_mob(
            &mut rt.mobs_app.world,
            MobId::COW,
            Vec3::new(7.0 * 16.0 + 8.5, 25.0, 8.5),
        );
        rt.stream();
        let before = *rt
            .mobs_app
            .world
            .read::<PhysBody>()
            .get(cow)
            .expect("放置后在册");
        for _ in 0..3 {
            rt.fixed_step(1.0 / 20.0);
        }
        let bodies = rt.mobs_app.world.read::<PhysBody>();
        let ticks = rt.mobs_app.world.read::<MobTicks>();
        let body = bodies.get(cow).expect("环外实体不得被删除（冻结≠卸载）");
        assert_eq!(body.pos, before.pos, "环外实体完全不 tick（位移零）");
        assert_eq!(body.vel, before.vel, "环外实体速度不动");
        assert_eq!(
            ticks.get(cow).expect("组件保留").idle_ticks,
            0,
            "环外实体计时器冻结（noActionTime 不增）"
        );
    }

    /// 冻结是暂时的：玩家锚点向怪迁移后（怪块落入环）恢复 tick，且冻结
    /// 期间实体一直活着。
    #[test]
    fn mob_resumes_ticking_after_anchor_moves_closer() {
        let mut rt = sim_rt("sim-resume");
        rt.chunks.insert(ChunkPos::new(7, 0), lit_chunk(7, 0, 20));
        let cow = spawn_mob(
            &mut rt.mobs_app.world,
            MobId::COW,
            Vec3::new(7.0 * 16.0 + 8.5, 25.0, 8.5),
        );
        rt.stream();
        for _ in 0..3 {
            rt.fixed_step(1.0 / 20.0);
        }
        {
            let ticks = rt.mobs_app.world.read::<MobTicks>();
            assert_eq!(
                ticks.get(cow).map(|t| t.idle_ticks),
                Some(0),
                "前置：冻结期 idle 不增"
            );
        }
        // 锚点 (0,0) → (4,0)：怪块 (7,0) 落入环（d=3 ≤ 5，sim 级别
        // (31−5)+3=29 ≤ 31）。pos≠ZERO 不触投放哨兵；玩家块 (4,0) 缺席，
        // 玩家物理整步冻结（锚点稳定）。
        rt.player.pos = Vec3::new(4.0 * 16.0 + 8.5, 70.0, 8.5);
        rt.stream(); // 票表随锚点全量重建（stream 每帧语义）
        rt.fixed_step(1.0 / 20.0);
        let ticks = rt.mobs_app.world.read::<MobTicks>();
        let idle = ticks.get(cow).expect("恢复后仍在册").idle_ticks;
        assert!(idle >= 1, "回环后恢复 tick，实得 {idle}");
    }

    /// 环门外唯一逻辑 = checkDespawn（ServerLevel.java:417-419 的 despawn
    /// 检查在环门之前）：环外冻结论不豁免 >128 格立即移除（Mob.java:664-666）。
    /// 此为行为锁——两个相位都必须绿。
    #[test]
    fn frozen_mob_beyond_despawn_distance_still_despawns() {
        let mut rt = sim_rt("sim-despawn");
        rt.chunks.insert(ChunkPos::new(8, 0), lit_chunk(8, 0, 20));
        let cow = spawn_mob(
            &mut rt.mobs_app.world,
            MobId::COW,
            Vec3::new(8.0 * 16.0 + 8.5, 25.0, 8.5),
        ); // 距玩家 (8.5,0,8.5) ≥128 格（dx=128，dy=25）
        rt.stream();
        rt.fixed_step(1.0 / 20.0);
        assert!(
            rt.mobs_app.world.read::<PhysBody>().get(cow).is_none(),
            ">128 格的冻结实体仍被 checkDespawn 立即移除"
        );
    }

    /// 箭矢同门（AbstractArrow 亦在 entityTickList，ServerLevel.java:419
    /// 对全部实体生效）：环外箭矢滞空冻结——不推进、寿命不耗；环内照常飞。
    #[test]
    fn arrow_outside_simulation_ring_frozen() {
        let mut rt = sim_rt("sim-arrow");
        rt.chunks.insert(ChunkPos::new(7, 0), lit_chunk(7, 0, 20));
        rt.chunks.insert(ChunkPos::new(3, 0), lit_chunk(3, 0, 20));
        let (far, near) = {
            let world = &mut rt.mobs_app.world;
            let far = world.spawn();
            world.insert(
                far,
                MobArrow {
                    pos: Vec3::new(7.0 * 16.0 + 8.5, 25.0, 8.5),
                    vel: Vec3::new(-10.0, 0.0, 0.0),
                    ttl_ticks: 200,
                    base_damage: 2.0,
                    crit: false,
                    player_owned: true,
                },
            );
            let near = world.spawn();
            world.insert(
                near,
                MobArrow {
                    pos: Vec3::new(3.0 * 16.0 + 8.5, 25.0, 8.5),
                    vel: Vec3::new(-10.0, 0.0, 0.0),
                    ttl_ticks: 200,
                    base_damage: 2.0,
                    crit: false,
                    player_owned: true,
                },
            );
            (far, near)
        };
        rt.stream();
        rt.fixed_step(1.0 / 20.0);
        let arrows = rt.mobs_app.world.read::<MobArrow>();
        let far_a = arrows.get(far).expect("环外箭矢在册");
        assert_eq!(
            far_a.pos,
            Vec3::new(7.0 * 16.0 + 8.5, 25.0, 8.5),
            "环外箭矢滞空冻结（不推进）"
        );
        assert_eq!(far_a.ttl_ticks, 200, "环外箭矢寿命不耗");
        let near_a = arrows.get(near).expect("环内箭矢在册");
        assert!(
            near_a.pos.x < 3.0 * 16.0 + 8.5,
            "环内箭矢照常推进（沿 −x 速度）"
        );
    }

    // ---- 触摸挖掘端到端（任务板 #93 优先项回归锁）----
    // 走 app 层同款入口：runtime.touch 点亮 + press_mine → fixed_step 固定步
    // 内 apply_touch_input 镜像 input.mining 并触发 on_left_press →
    // step_mining 每 20 Hz tick 续挖 → 硬度到 → 方块破坏。

    /// 往 rt 写一个目标方块（直接写体素；无头测试不建网格，不触 GPU）。
    fn put_block(rt: &mut GameRuntime, at: BlockPos, id: u16) {
        let handle = rt.chunks.get(&at.chunk()).expect("目标区块在册");
        let [lx, _, lz] = at.local();
        handle.voxels.write().unwrap()[mcv_core::vidx(lx, at.y, lz)] = BlockId(id);
    }

    fn voxel(rt: &GameRuntime, at: BlockPos) -> u16 {
        let handle = rt.chunks.get(&at.chunk()).expect("目标区块在册");
        let [lx, _, lz] = at.local();
        handle.voxels.read().unwrap()[mcv_core::vidx(lx, at.y, lz)].0
    }

    #[test]
    fn touch_mine_held_breaks_blocks_end_to_end() {
        let mut rt = headless_rt("touchmine");
        fill_neighborhood(&mut rt);
        rt.player.pos = Vec3::new(8.5, 70.0, 8.5);
        rt.player.vel = Vec3::ZERO;
        // 徒手基准（清掉开局铁剑）：泥土硬度 0.5、不需工具 → 30 档 =
        // 15 tick（0.75 s）破一块。
        rt.hotbar.slots[0] = mcv_item::ItemStack::empty();
        let dirt = mcv_core::BLOCKS
            .iter()
            .position(|b| b.name == "dirt")
            .expect("注册表含 dirt") as u16;
        // 目标：正前方 2 格、眼高（pitch=0 视线 -Z，y=71.62 落在 y=71 格）。
        let first = BlockPos::new(8, 71, 6);
        let second = BlockPos::new(8, 71, 5);
        put_block(&mut rt, first, dirt);
        put_block(&mut rt, second, dirt);

        // app 层入口：任意触摸事件点亮（enabled）+ 挖按钮按下沿。
        rt.touch.enabled = true;
        rt.touch.press_mine();
        assert_eq!(
            rt.phase,
            GamePhase::Loading,
            "构造即加载态：挖掘按下沿在转游玩前的步不派发"
        );

        let mut saw_progress = false;
        let mut first_broken = false;
        let mut second_broken = false;
        for _ in 0..240 {
            rt.fixed_step(1.0 / 60.0);
            // 挖掘中（目标锁定 + 挥臂推进）即接线生效。
            if rt.mine.pos == Some(first) && rt.swing_progress() > 0.0 {
                saw_progress = true;
            }
            if voxel(&rt, first) == 0 {
                first_broken = true;
                // 破坏后 5-tick 冷却内不开下一目标（原版 destroyDelay）。
                if voxel(&rt, second) == 0 {
                    second_broken = true;
                    break;
                }
            }
        }
        assert!(
            saw_progress,
            "触摸长按必须连进挖掘状态机：目标锁定 + 挥臂进度推进（按下边沿只触发一次 on_left_press，按住续挖走 fixed_step 的 step_mining）"
        );
        assert!(first_broken, "硬度累加到阈值后目标方块必须被破坏");
        assert!(
            second_broken,
            "按住不松必须自动开下一目标（continue_tick 的 5-tick 冷却后对新目标 START）"
        );

        // 松开沿：input.mining 清零、状态机不再被带起。
        rt.touch.release_mine();
        rt.fixed_step(1.0 / 60.0);
        assert!(!rt.input.mining, "松开挖掘按钮必须镜像清 input.mining");
        assert!(
            rt.mine.pos.is_none(),
            "松开后挖掘状态机保持作废（进度不补判）"
        );
    }

    /// 攻击按下沿（无目标方块）同样挥臂：挥臂动画接线的第二触发点。
    #[test]
    fn attack_press_swing_starts_even_without_block() {
        let mut rt = headless_rt("swing");
        fill_neighborhood(&mut rt);
        rt.player.pos = Vec3::new(8.5, 70.0, 8.5);
        rt.player.vel = Vec3::ZERO;
        rt.touch.enabled = true;
        rt.touch.press_mine();
        rt.fixed_step(1.0 / 60.0); // 转游玩步（不派发）
        assert_eq!(rt.swing_progress(), 0.0, "加载态不派发攻击沿");
        rt.fixed_step(1.0 / 60.0); // 攻击沿
        assert!(
            rt.swing_progress() > 0.0,
            "对空按下沿必挥臂（26.1 startAttack → player.swing）"
        );
        let mid = rt.swing_progress();
        rt.fixed_step(1.0 / 60.0);
        assert!(
            rt.swing_progress() > mid,
            "挥臂按 tick 制平滑推进（渲染帧间不跳变）"
        );
        // 满一轮（6 tick）后停摆归零。
        for _ in 0..40 {
            rt.fixed_step(1.0 / 60.0);
        }
        assert_eq!(rt.swing_progress(), 0.0, "一轮挥完停摆归零");
    }

    #[test]
    fn walking_bob_state_accumulates_and_standing_stays_zero() {
        // 26.1 行走 bob 链（审计 #4「走路不晃」直接根因）：
        // LocalPlayer.java:989 walkDist += len(Δx,Δz)·0.6（每 tick）；
        // AbstractClientPlayer.java:84-87 tBob = min(0.1, |Δv|水平)，门 =
        // onGround；ClientAvatarState.java:77 bob += (tBob−bob)·0.4。
        let mut rt = playing_rt("bob-walk");
        rt.player.pos = Vec3::new(8.5, 70.0, 8.5);
        rt.player.vel = Vec3::new(4.317, 0.0, 0.0); // 满速走 m/s
        rt.player.on_ground = true;
        let (b0, bid0) = rt.hand_bob();
        assert_eq!((b0, bid0), (0.0, 0.0), "未走时 bob 幅度与相位都为 0");
        // 走一 tick：位移 0.216 格进 walk_dist、速度幅度进 bob。
        rt.player.pos.x += 4.317 / 20.0;
        rt.fixed_step(1.0 / 20.0);
        // 半 tick 的渲染插值余量（整 tick 后 tick_frac=0，hand_bob 读 o 值）。
        rt.fixed_step(1.0 / 60.0);
        let (b, bid) = rt.hand_bob();
        assert!(
            b > 0.0,
            "行走必须积累 bob 幅度（0.4 插值收敛，首 tick = 0.1·0.4 = 0.04），got {b}"
        );
        assert!(
            bid < 0.0,
            "walkDist 必须随位移增长且 bID 取负（got {bid}）——bID 恒 0 即 bobView 无相位源"
        );
        // 停步：门关（tBob=0）后幅度按 0.6/tick 收敛回 0。
        rt.player.vel = Vec3::ZERO;
        for _ in 0..40 {
            rt.fixed_step(1.0 / 20.0);
        }
        let (b_end, _) = rt.hand_bob();
        assert!(
            b_end < 1e-4,
            "静止 40 tick 后 bob 幅度必须收敛归零（got {b_end}）"
        );
    }

    #[test]
    fn loading_state_blocks_input_and_transition_until_ready() {
        let mut rt = headless_rt("gate");
        // 构造即加载态（26.1 doWorldLoad 先 setScreen(LevelLoadingScreen)）。
        assert_eq!(rt.phase, GamePhase::Loading);
        rt.player.pos = Vec3::new(8.5, 71.0, 8.5);
        // 只有中心块就绪：49 块邻域未齐 → 不放玩家。
        rt.chunks
            .entry(ChunkPos::new(0, 0))
            .or_insert_with(|| lit_chunk(0, 0, 69))
            .advance_to(Stage::Uploaded);
        rt.input.forward = true;
        rt.input.jump = true;
        rt.input.mining = true;
        for _ in 0..30 {
            rt.fixed_step(1.0 / 60.0);
        }
        assert_eq!(rt.phase, GamePhase::Loading, "邻域未就绪不放玩家");
        assert!(
            !rt.input.forward && !rt.input.jump && !rt.input.mining,
            "加载态输入被门清零（26.1 Screen 非 null 时移动输入不生效）"
        );
        assert!(
            rt.mine.pos.is_none() && rt.mine.progress == 0.0,
            "挖掘状态机不得被输入带起"
        );
    }

    #[test]
    fn loading_transfers_next_step_when_neighborhood_instant_ready() {
        let mut rt = headless_rt("instant");
        fill_neighborhood(&mut rt);
        // 全部区块瞬间就绪：下一固定步即转游玩（closeDelay 默认 0）。
        rt.fixed_step(1.0 / 60.0);
        assert_eq!(rt.phase, GamePhase::Playing);
        // 游玩态输入恢复生效：前向输入驱动位移。
        rt.input.forward = true;
        let before = rt.player.pos;
        rt.fixed_step(1.0 / 60.0);
        assert_ne!(rt.player.pos, before, "转游玩后输入生效");
    }

    #[test]
    fn loading_progress_monotonic_and_clamped() {
        let mut rt = headless_rt("progress");
        rt.player.pos = Vec3::new(8.5, 71.0, 8.5);
        let mut last = rt.loading_progress();
        assert_eq!(last, 0.0, "无就绪区块进度 0");
        let mut cells: Vec<(i32, i32)> = {
            let mut v = Vec::new();
            for dx in -3i32..=3 {
                for dz in -3i32..=3 {
                    v.push((dx, dz));
                }
            }
            v
        };
        // 打乱填充顺序，验证乱序完成下进度单调不减。
        for i in (0..cells.len()).rev() {
            let j = (i * 7 + 13) % (i + 1);
            cells.swap(i, j);
        }
        for (dx, dz) in cells {
            let pos = ChunkPos::new(dx, dz);
            rt.chunks
                .entry(pos)
                .or_insert_with(|| lit_chunk(dx, dz, 69))
                .advance_to(Stage::Uploaded);
            // 刀 3b 门谓词解耦后 fixture 双记账位同步置位（见
            // fill_neighborhood 注释）：关屏延迟段走门放行，需 status。
            rt.chunks[&pos].advance_status(ChunkStatus::Full);
            let p = rt.loading_progress();
            assert!((0.0..=1.0).contains(&p), "进度钳制 0..1：{p}（{dx},{dz}）");
            assert!(p >= last, "进度单调不减：{p} < {last}");
            last = p;
        }
        assert_eq!(last, 1.0, "邻域全就绪进度 1");
        // 平滑进度（LevelLoadingScreen.java:84 每 tick lerp 0.2）：重进
        // 加载态（新世界关屏延迟 10 tick 留出观察窗），观察期内平滑值
        // 向 1 单调爬升且钳制。
        rt.begin_load(true);
        let mut last_s = rt.loading_progress_smoothed();
        for _ in 0..30 {
            rt.fixed_step(1.0 / 60.0);
            let s = rt.loading_progress_smoothed();
            assert!((0.0..=1.0).contains(&s), "平滑进度钳制 0..1：{s}");
            assert!(s >= last_s, "平滑进度单调不减：{s} < {last_s}");
            last_s = s;
        }
        assert!(last_s > 0.0, "观察期内平滑进度应向 1 爬升：{last_s}");
        assert_eq!(rt.phase, GamePhase::Playing, "关屏延迟走满转游玩");
    }

    #[test]
    fn loading_close_delay_holds_new_world_half_second() {
        let mut rt = headless_rt("delay");
        fill_neighborhood(&mut rt);
        // 新世界关屏延迟 500ms = 10 tick（Minecraft.java:2083
        // `new LevelLoadTracker(newWorld ? 500L : 0L)`）；60Hz 下每 3 步
        // 1 tick，第 30 步 game_ticks 才到 10。前 27 步（game_ticks≤9）
        // 必须保持加载态。
        rt.begin_load(true);
        for _ in 0..27 {
            rt.fixed_step(1.0 / 60.0);
            assert_eq!(rt.phase, GamePhase::Loading, "关屏延迟期内保持加载态");
        }
        // 延迟走满（game_ticks 达 ready_at+10）即转游玩。
        for _ in 0..6 {
            rt.fixed_step(1.0 / 60.0);
            if rt.phase == GamePhase::Playing {
                return;
            }
        }
        panic!("关屏延迟走满后未转游玩态");
    }

    #[test]
    fn loading_timeout_lets_player_in() {
        let mut rt = headless_rt("timeout");
        rt.player.pos = Vec3::new(8.5, 71.0, 8.5);
        // 30s 等待截止（LevelLoadTracker.java:26 CLIENT_WAIT_TIMEOUT_MS，
        // :152-156 超时放行）。
        rt.game_ticks = 601;
        rt.fixed_step(1.0 / 60.0);
        assert_eq!(rt.phase, GamePhase::Playing, "超时放玩家进场");
    }

    #[test]
    fn respawn_reenters_loading_and_drops_at_surface() {
        let mut rt = headless_rt("respawn");
        rt.player.pos = Vec3::new(8.5, 71.0, 8.5);
        rt.hurt_player(100.0, None);
        assert!(rt.dead);
        rt.respawn();
        assert!(!rt.dead);
        assert_eq!(
            rt.phase,
            GamePhase::Loading,
            "重生重进加载态（26.1 handleRespawn → startWaitingForNewLevel）"
        );
        assert_eq!(rt.player.pos, Vec3::ZERO, "复活点位待出生点投放");
        assert!(
            !rt.spawned,
            "respawn 双复位（pos=ZERO + spawned=false）后重走投放"
        );
        // 搜索窗（±3）全部地形就绪 → stream 投放到地表（脚踩 heightmap
        // 顶 = gy+1，26.1 pos.above()，不再从 y=200 自由落体、也不再有
        // hm+1 的悬空一格）。
        for dx in -3i32..=3 {
            for dz in -3i32..=3 {
                rt.chunks
                    .entry(ChunkPos::new(dx, dz))
                    .or_insert_with(|| lit_chunk(dx, dz, 69));
            }
        }
        rt.stream();
        let surface =
            f32::from(rt.chunks[&ChunkPos::new(0, 0)].heightmap.read().unwrap()[(8 << 4) | 8]);
        assert_eq!(
            rt.player.pos,
            Vec3::new(8.5, surface, 8.5),
            "复活落点 = 出生点地表（脚位 = heightmap 顶）"
        );
        assert!(rt.spawned, "投放完成置位");
        // 邻域就绪后放行。
        fill_neighborhood(&mut rt);
        rt.fixed_step(1.0 / 60.0);
        assert_eq!(rt.phase, GamePhase::Playing);
    }

    // 出生投放专项测试见 spawn_tests.rs（本 mod 子模块，标准嵌套路径
    // src/game/tests/，共用无头装配 headless_rt / lit_chunk）。
    mod spawn_tests;
    // 天气存档往返/旧档回退/雨粒子分层测试见 weather_tests.rs（同上路径）。
    mod weather_tests;
    // ---- 进食整链（26.1 Consumable；板载 #94）----

    use mcv_item::ItemStack as ItemSt;

    /// 转 Playing 态并把玩家摆在地表（共用 fill_neighborhood 装配）。
    fn playing_rt(tag: &str) -> GameRuntime {
        let mut rt = headless_rt(tag);
        fill_neighborhood(&mut rt);
        rt.fixed_step(1.0 / 20.0);
        assert_eq!(rt.phase, GamePhase::Playing, "前置：邻域齐备已转游玩");
        rt
    }

    /// 按住右键吃满 33 tick（启动沿 + 32 tick 推进 + 1 步结算余量）。
    fn hold_eat(rt: &mut GameRuntime, ticks: usize) {
        rt.input.placing = true;
        rt.interact(true);
        for _ in 0..ticks {
            rt.fixed_step(1.0 / 20.0);
        }
    }

    #[test]
    fn eat_rotten_flesh_restores_hunger_and_consumes_stack() {
        let mut rt = playing_rt("eat-bread-chain");
        // 面包数值面在 mcv_item::food 测试（登记不造）；此处用可得腐肉
        // 走整链：nutrition 4 → hunger 15 + 4 = 19。
        rt.hotbar = mcv_item::Hotbar::empty();
        rt.hotbar.slots[0] = ItemSt::new(mcv_item::ROTTEN_FLESH, 3);
        rt.player.hunger = 15.0;
        rt.player.saturation = 0.0;
        hold_eat(&mut rt, 35);
        assert_eq!(rt.player.hunger, 19.0, "完食回饥饿（FoodData.eat）");
        // 饱和 = 4×0.1×2 = 0.8（FoodConstants.java:30-32）。
        assert!((rt.player.saturation - 0.8).abs() < 1e-4);
        assert_eq!(rt.hotbar.slots[0].count, 2, "生存消耗一格（stack.consume）");
        assert!(rt.eat_hold.is_none(), "完食清账");
        assert_eq!(rt.eat_progress(), None);
    }

    #[test]
    fn eat_spider_eye_applies_poison_effect() {
        // 蜘蛛眼 chance 1.0 → 效果可确定性断言（腐肉 0.8 概率面在
        // mcv_item::food::effect_fires 纯函数测试覆盖）。
        let mut rt = playing_rt("eat-eye");
        rt.hotbar = mcv_item::Hotbar::empty();
        rt.hotbar.slots[0] = ItemSt::new(mcv_item::SPIDER_EYE, 1);
        rt.player.hunger = 10.0;
        hold_eat(&mut rt, 35);
        assert_eq!(rt.player.hunger, 12.0, "nutrition 2 回饥饿");
        assert!(rt.effects.has(mcv_entity::Kind::Poison), "完食挂 poison");
        // 施加值 100（Consumables.java:62-64）来自 food 表（mcv_item 测试锁
        // 死）；在账剩余 = 100 − 3：apply 落在第 32 步，第 33–35 步 effects
        // .tick 各扣 1。
        assert_eq!(
            rt.effects.get(mcv_entity::Kind::Poison).map(|a| a.duration),
            Some(97),
            "剩余时长按 tick 递减（MobEffectInstance.advance）"
        );
        assert!(rt.hotbar.slots[0].is_empty(), "最后一件吃完槽清空");
    }

    #[test]
    fn releasing_mid_eat_cancels_and_keeps_stack() {
        let mut rt = playing_rt("eat-cancel");
        rt.hotbar = mcv_item::Hotbar::empty();
        rt.hotbar.slots[0] = ItemSt::new(mcv_item::ROTTEN_FLESH, 3);
        rt.player.hunger = 15.0;
        rt.input.placing = true;
        rt.interact(true);
        assert!(rt.eat_hold.is_some(), "按下沿启动进食");
        assert!(rt.eat_progress().is_some(), "进度通路备用");
        for _ in 0..10 {
            rt.fixed_step(1.0 / 20.0);
        }
        // 松手 = releaseUsing 取消：不吃、不扣、进度不保留。
        rt.input.placing = false;
        rt.fixed_step(1.0 / 20.0);
        assert!(rt.eat_hold.is_none());
        assert_eq!(rt.player.hunger, 15.0, "取消不结算");
        assert_eq!(rt.hotbar.slots[0].count, 3, "取消不扣物品");
        // 重新按住 = 从头吃（重启启动沿，进度归零）。
        rt.input.placing = true;
        rt.interact(true);
        assert_eq!(rt.eat_hold.map(|e| e.ticks), Some(0), "重新起步 tick 0");
        rt.fixed_step(1.0 / 20.0);
        assert_eq!(rt.eat_hold.map(|e| e.ticks), Some(1));
    }

    #[test]
    fn full_hunger_refuses_food_and_keeps_stack() {
        // 满饥饿 nutrition 浪费规则：needsFood 门拒吃（Player.java:1581-1582）。
        let mut rt = playing_rt("eat-full");
        rt.hotbar = mcv_item::Hotbar::empty();
        rt.hotbar.slots[0] = ItemSt::new(mcv_item::ROTTEN_FLESH, 3);
        rt.player.hunger = 20.0;
        rt.input.placing = true;
        rt.interact(true);
        assert!(rt.eat_hold.is_none(), "满饥饿拒吃");
        assert_eq!(rt.hotbar.slots[0].count, 3);
        // 按住也不经 fixed_step 起吃（auto-repeat 同被门拦）。
        for _ in 0..10 {
            rt.fixed_step(1.0 / 20.0);
        }
        assert_eq!(rt.player.hunger, 20.0);
        assert_eq!(rt.hotbar.slots[0].count, 3);
    }

    #[test]
    fn chained_holding_eats_until_cap_twenty() {
        // 连续按住：连吃节奏 = rightClickDelay 4 tick + 32 tick 进食，
        // 到 20 上限即被 needsFood 门拦下，剩余腐肉不扣。
        let mut rt = playing_rt("eat-chain");
        rt.hotbar = mcv_item::Hotbar::empty();
        rt.hotbar.slots[0] = ItemSt::new(mcv_item::ROTTEN_FLESH, 16);
        rt.player.hunger = 1.0;
        rt.player.saturation = 0.0;
        hold_eat(&mut rt, 0);
        // hunger 1 → 5 次腐肉到 20；每次 32 + 4 = 36 tick，给足 5×37 步。
        for _ in 0..(5 * 37 + 34) {
            rt.fixed_step(1.0 / 20.0);
        }
        assert_eq!(rt.player.hunger, 20.0, "钳 0..20（FoodData.java:20）");
        assert!(rt.eat_hold.is_none(), "满饥饿连吃中止");
        // 5 块下肚（1+4×5=20 恰好），第 6 次起不吃 → 剩 11。
        assert_eq!(rt.hotbar.slots[0].count, 11);
    }

    #[test]
    fn creative_eats_without_consuming_stack() {
        // 创造 invulnerable 免 needsFood 门（vanilla canEat），hasInfinite
        // 材料不扣（ Consumable.onConsume stack.consume 豁免）。
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            "eat-creative",
            std::process::id(),
            std::time::SystemTime::now(),
        )
            .hash(&mut h);
        let dir = std::env::temp_dir().join(format!("mcv-eat-creative-{:x}", h.finish()));
        let mut rt = GameRuntime::new_headless(20261010, dir, GameMode::Creative);
        fill_neighborhood(&mut rt);
        rt.fixed_step(1.0 / 20.0);
        rt.hotbar = mcv_item::Hotbar::empty();
        rt.hotbar.slots[0] = ItemSt::new(mcv_item::ROTTEN_FLESH, 5);
        rt.player.hunger = 20.0;
        rt.input.placing = true;
        rt.interact(true);
        assert!(rt.eat_hold.is_some(), "创造 invulnerable 可吃");
        for _ in 0..35 {
            rt.fixed_step(1.0 / 20.0);
        }
        assert_eq!(rt.hotbar.slots[0].count, 5, "创造不扣物品");
        assert_eq!(rt.player.hunger, 20.0, "满值钳制");
    }

    #[test]
    fn survival_stats_survive_save_load_roundtrip() {
        // v5 存档：hunger/saturation/exhaustion/health/air/difficulty 退出
        // 重进不失忆——不持久化则进食白吃（板载 #94 硬绑定条）。
        let mut rt = headless_rt("stats-save");
        rt.player.health = 13.5;
        rt.player.hunger = 17.0;
        rt.player.saturation = 4.25;
        rt.player.exhaustion = 39.75;
        let dir = rt.save_dir.clone();
        rt.save_meta();
        let mut rt2 = GameRuntime::new_headless(20261010, dir, GameMode::Survival);
        rt2.load_meta();
        assert_eq!(rt2.player.health, 13.5);
        assert_eq!(rt2.player.hunger, 17.0);
        assert_eq!(rt2.player.saturation, 4.25);
        assert_eq!(rt2.player.exhaustion, 39.75);
        assert_eq!(rt2.air_supply(), MAX_AIR_SUPPLY, "未下水满气");
        assert_eq!(rt2.difficulty, crate::difficulty::Difficulty::Normal);
    }

    #[test]
    fn survival_stats_and_difficulty_restore_from_meta() {
        // 难度与空气也随档：难度掉血曲线、溺水账不因重进重置。
        let mut rt = headless_rt("stats-hard");
        rt.difficulty = crate::difficulty::Difficulty::Hard;
        rt.player.hunger = 6.0;
        rt.player.health = 9.0;
        rt.air_supply = 120;
        let dir = rt.save_dir.clone();
        rt.save_meta();
        let mut rt2 = GameRuntime::new_headless(20261010, dir, GameMode::Survival);
        rt2.load_meta();
        assert_eq!(rt2.difficulty, crate::difficulty::Difficulty::Hard);
        assert_eq!(rt2.player.hunger, 6.0);
        assert_eq!(rt2.player.health, 9.0);
        assert_eq!(rt2.air_supply(), 120);
    }

    // ---- 交互波 0：B1 基岩豁免 / B3 放置可替换校验 ----

    /// 旧表 id：基岩（hardness=inf）、水、圆石（手持方块物品 = COBBLESTONE）。
    const BEDROCK: u16 = 10;
    const WATER: u16 = 5;

    /// 把脚下方格 (8,69,8) 写成指定方块（lit_chunk 装配的石柱地表）。
    fn set_under_player(rt: &mut GameRuntime, id: u16) {
        let h = &rt.chunks[&ChunkPos::new(0, 0)];
        h.voxels.write().unwrap()[lidx(8, 69, 8)] = BlockId(id);
    }

    /// 俯视脚下：pitch 取鼠标 clamp 下限 −1.55（≈88.8°，camera 约定
    /// `clamp(-1.55, 1.55)`；−π/2 会被裁回，−1.0 弧度只有 57° 会斜打邻列）。
    /// 视线近乎垂直：眼 (8.5,71.62,8.5) 下探 1.62 格水平漂移仅 ~0.03，
    /// 必中 (8,69,8) 顶面。
    fn look_down(rt: &mut GameRuntime) {
        rt.player.pitch = -1.55;
        rt.player.yaw = 0.0;
    }

    /// B1 基岩豁免（26.1 `strength(-1)` Blocks.java:193-196 →
    /// getDestroyProgress 恒 0，BlockBehaviour.java:355-359）：生存按住左键
    /// 20 秒，进度恒 0、基岩纹丝不动；创造按下秒破
    /// （`abilities.instabuild` 先于硬度判定，ServerPlayerGameMode.java:172-175）。
    #[test]
    fn survival_cannot_mine_bedrock_creative_can() {
        let mut rt = playing_rt("b1-bedrock");
        // 脚下方格写成基岩（放置链路由 placement_requires_replaceable_target
        // 覆盖，此处直接落体素，聚焦挖掘侧）。
        look_down(&mut rt);
        set_under_player(&mut rt, BEDROCK);
        // 生存按住挖 400 tick：per=0 无任何进度、方块不掉。长按 CONTINUE
        // 换目标时状态机会登记 pos——原版同款：continueDestroyBlock 对
        // getDestroyProgress=0 的方块照样进入 destroy 状态、仅进度恒 0
        // （ServerPlayerGameMode.java:205-217），故锁进度恒 0 + 方块完好，
        // 不锁 pos。
        rt.on_left_press();
        rt.input.mining = true;
        for _ in 0..400 {
            rt.fixed_step(1.0 / 20.0);
        }
        assert_eq!(rt.mine.progress, 0.0, "无限硬度不得推进进度");
        assert_eq!(rt.mine.per_tick, 0.0);
        assert_eq!(
            rt.chunks[&ChunkPos::new(0, 0)].voxels.read().unwrap()[lidx(8, 69, 8)],
            BlockId(BEDROCK),
            "生存 400 tick 后基岩仍在"
        );
        // 创造按下 = 无视硬度秒破（instabuild 门，无 destroyProgress 参与）。
        rt.input.mining = false;
        rt.mode = GameMode::Creative;
        rt.on_left_press();
        assert_eq!(
            rt.chunks[&ChunkPos::new(0, 0)].voxels.read().unwrap()[lidx(8, 69, 8)],
            BlockId(0),
            "创造必须能破基岩（26.1 实况：instabuild 先于硬度判定）"
        );
    }

    /// B3 放置可替换校验（26.1 `BlockPlaceContext.canPlace` :55-57 →
    /// `canBeReplaced` = 空气 ∥ `Properties.replaceable()`）：目标格实心
    /// 拒放（不写体素、不扣手持）；空气成功；水（replaceable+liquid）
    /// 被替换。判据表实现 = `BlockDef::is_replaceable`（名单锁定测试见
    /// mcv_core::lib 的 is_replaceable_matches_vanilla_property）。
    #[test]
    fn placement_requires_replaceable_target() {
        let mut rt = playing_rt("b3-replace");
        rt.hotbar = mcv_item::Hotbar::empty();
        rt.hotbar.slots[0] = ItemSt::new(mcv_item::COBBLESTONE, 2);
        // 玩家悬空到 y=72.5（不推进 tick、不落位）：脚部 AABB y∈[72.5,74.3]
        // 与目标格 [70,71]/[71,72] 全不相交，聚焦可替换门本身；眼
        // 74.12 → 石柱顶面 70.0 距离 4.12 < 生存 reach 4.5。
        rt.player.pos = Vec3::new(8.5, 72.5, 8.5);
        rt.player.vel = Vec3::ZERO;
        look_down(&mut rt);
        let v = |rt: &GameRuntime, x: usize, y: usize, z: usize| -> u16 {
            rt.chunks[&ChunkPos::new(0, 0)].voxels.read().unwrap()[lidx(x, y, z)].id()
        };
        let set = |rt: &mut GameRuntime, x: usize, y: usize, z: usize, id: u16| {
            rt.chunks[&ChunkPos::new(0, 0)].voxels.write().unwrap()[lidx(x, y, z)] = BlockId(id);
        };
        // ① 空气目标：俯视命中 (8,69,8) 石顶 → 目标格 (8,70,8) 空气，
        //    成功放置并消耗（回归基线）。
        rt.interact(true);
        assert_eq!(v(&rt, 8, 70, 8), 9, "空气格应放得下方块");
        assert_eq!(rt.hotbar.slots[0].count, 1, "成功放置消耗一格");
        // ② 实心目标：(8,71,8) 手工灌石头，俯视命中圆石 (8,70,8) 顶面 →
        //    目标格 = 石头实心 → 拒放：不写体素、不扣物品
        //    （26.1 canPlace=false → useOn 中止）。
        set(&mut rt, 8, 71, 8, STONE);
        rt.interact(true);
        assert_eq!(v(&rt, 8, 71, 8), STONE, "实心格不可替换，不得改写");
        assert_eq!(v(&rt, 8, 70, 8), 9, "圆石格未被波及");
        assert_eq!(rt.hotbar.slots[0].count, 1, "拒放不消耗手持（不发放置）");
        // ③ 水目标：实心石头换成水（replaceable+liquid）→ 同一条射线，
        //    目标格 = 水格 → 可替换，放置成功替换水并消耗。
        set(&mut rt, 8, 71, 8, WATER);
        rt.hotbar.slots[0].count = 2;
        rt.interact(true);
        assert_eq!(
            v(&rt, 8, 71, 8),
            9,
            "水可替换：放置替换水（26.1 water .replaceable() Blocks.java:202）"
        );
        assert_eq!(rt.hotbar.slots[0].count, 1, "替换水成功消耗一格");
    }

    /// B3 玩家碰撞门回归锁：目标格与玩家 AABB 相交（脚下站立格）时拒放
    /// 不消耗——旧行为保持，且现在先被可替换门前置拦截亦同结果。
    #[test]
    fn placement_into_player_cell_not_consumed() {
        let mut rt = playing_rt("b3-self");
        rt.hotbar = mcv_item::Hotbar::empty();
        rt.hotbar.slots[0] = ItemSt::new(mcv_item::COBBLESTONE, 2);
        // 显式落位到石柱顶面 y=70.0（fill_neighborhood 悬在 71.0，单步
        // fixed_step 不足以跨 tick 落位——悬浮时脚部 AABB 与目标格 [70,71]
        // 恰好边界相切（`cmax.y > pmin.y` 为 71>71 false），AABB 门反而
        // 放行；站上后 [70,71.8] ∩ [70,71] 严格相交才是真实占格语义）。
        rt.player.pos = Vec3::new(8.5, 70.0, 8.5);
        rt.player.vel = Vec3::ZERO;
        // 俯视命中 (8,69,8) 顶面 → 目标格 = (8,70,8) = 玩家脚部所在格。
        look_down(&mut rt);
        rt.interact(true);
        assert_eq!(
            rt.chunks[&ChunkPos::new(0, 0)].voxels.read().unwrap()[lidx(8, 70, 8)],
            BlockId(0),
            "玩家占格不可放置"
        );
        assert_eq!(rt.hotbar.slots[0].count, 2, "拒放不消耗");
    }

    /// 129-L/L0（129-C §1 漏洞 A——「编辑后黑带」复现）：from 端未完成本地
    /// 布光（TerrainReady，读盘块 game.rs:2316 / 生成块等 init 席位
    /// :2082-2106，光数组全零）的 pull 任务，extract_edge 抽出全零边，
    /// REMOVE 相按 `s > n+1` 把已亮块边界格清零成黑带（原版跨 section
    /// decrease 携带编辑前波前等级，BlockLightEngine.java:91-105，从不拿
    /// 「未初始化=0」当波前）。修复判据：sync_light_edges 对 from 补
    /// ≥LightLocalReady 门（对齐 to 侧 :424 门语义），未就绪跳过、就绪后
    /// 由 border_synced 成对同步（:2121-2157）双向补收。
    #[test]
    fn edit_pull_from_unlit_neighbour_never_retracts_lit_chunk() {
        // 构造 C：已布光，+X 边界列含水柱（C 报场景：玩家在水边编辑）。
        fn lit_ocean_edge() -> Arc<ChunkHandle> {
            let h = Arc::new(ChunkHandle::new(ChunkPos::new(0, 0)));
            {
                let mut vg = h.voxels.write().unwrap();
                for y in 0..=20usize {
                    for zz in 0..16usize {
                        for xx in 0..16usize {
                            vg[lidx(xx, y, zz)] = BlockId(STONE);
                        }
                    }
                }
                for y in 21..=24usize {
                    for zz in 0..16usize {
                        for xx in 12..16usize {
                            vg[lidx(xx, y, zz)] = BlockId(5); // water
                        }
                    }
                }
            }
            let hm = mcv_worldgen::recompute_heightmap(bytemuck::cast_slice(
                h.voxels.read().unwrap().as_slice(),
            ));
            *h.heightmap.write().unwrap() = hm;
            {
                let vg = h.voxels.read().unwrap();
                let mut lg = h.light.write().unwrap();
                let hg = h.heightmap.read().unwrap();
                let voxels: &[u16] = bytemuck::cast_slice(vg.as_slice());
                let mut view = mcv_light::LightChunk {
                    voxels,
                    light: &mut lg[..],
                    heightmap: &hg[..],
                };
                mcv_light::init(&mut view);
            }
            h.advance_to(Stage::LightLocalReady);
            h
        }

        // 两组世界仅差一处：邻块 N 是否在册。同一编辑（水中放石）后 C 的
        // 光照必须逐字节一致——pull(from=N) 不得引入任何额外变化。
        // 组 A（C 报时序）：N 在册但停 TerrainReady（体素已到、光未 init，
        // 等 init 席位 game.rs:2082-2106）；pull 抽到全零边。
        let mut with_n = HashMap::new();
        with_n.insert(ChunkPos::new(0, 0), lit_ocean_edge());
        let n = Arc::new(ChunkHandle::new(ChunkPos::new(1, 0)));
        {
            let mut vg = n.voxels.write().unwrap();
            for y in 0..=20usize {
                for zz in 0..16usize {
                    for xx in 0..16usize {
                        vg[lidx(xx, y, zz)] = BlockId(STONE);
                    }
                }
            }
        }
        n.advance_to(Stage::TerrainReady);
        with_n.insert(ChunkPos::new(1, 0), n);
        // 组 B（基线）：N 缺席，pull 无从抽边 = 纯本地重光照终态
        //（审计草案「N 缺席前」口径）。N 就绪后由成对同步补收真实边值，
        // 本地重光照终态即正当终态。
        let mut without_n = HashMap::new();
        without_n.insert(ChunkPos::new(0, 0), lit_ocean_edge());

        // 前置：两组边界水柱一致且已亮（浅水 sky≥5，129-C §6）。
        for (tag, m) in [("with_n", &with_n), ("without_n", &without_n)] {
            let c = &m[&ChunkPos::new(0, 0)];
            assert!(
                c.light.read().unwrap()[lidx(15, 22, 8)] >> 4 >= 5,
                "{tag}: 前置边界水柱应已亮"
            );
        }

        // 同一编辑：C 贴边格水中放石 → relight 派发 pull+push。
        edit(&with_n, BlockPos::new(15, 22, 8), 5, STONE);
        edit(&without_n, BlockPos::new(15, 22, 8), 5, STONE);

        let lit = |m: &HashMap<ChunkPos, Arc<ChunkHandle>>| {
            m[&ChunkPos::new(0, 0)].light.read().unwrap().to_vec()
        };
        assert_eq!(
            lit(&with_n),
            lit(&without_n),
            "from 未 init 的 pull 任务改写了已亮块的光（黑带）；\
             N 在册(组A) 与 N 缺席(组B) 的 C 光照应逐字节一致"
        );
    }
}

#[cfg(test)]
mod tick_tests {
    use super::accumulate_ticks;

    #[test]
    fn sixty_hertz_steps_map_to_twenty_hertz_ticks() {
        let d = 1.0f32 / 60.0;
        let mut frac = 0.0f64;
        assert_eq!(accumulate_ticks(&mut frac, d), 0);
        assert_eq!(accumulate_ticks(&mut frac, d), 0);
        assert_eq!(
            accumulate_ticks(&mut frac, d),
            1,
            "每 3 个固定步 = 1 原版 tick"
        );
        // 一秒（60 步）恰好 20 tick——(dt*20) as u64 截断版恒 0 的回归锁。
        let mut sec = 0.0f64;
        let total: u64 = (0..60).map(|_| accumulate_ticks(&mut sec, d)).sum();
        assert_eq!(total, 20, "60 fps 下每秒必须走满 20 tick");
        // 帧率不敏感：240 fps（每步 1/240 s）同样 20 tick/s。
        let mut fast = 0.0f64;
        let total: u64 = (0..240)
            .map(|_| accumulate_ticks(&mut fast, 1.0 / 240.0))
            .sum();
        assert_eq!(total, 20);
        // burst 封顶 4（卡顿/后台回归防级联）。
        let mut burst = 0.0f64;
        assert_eq!(accumulate_ticks(&mut burst, 10.0), 4);
    }
}
