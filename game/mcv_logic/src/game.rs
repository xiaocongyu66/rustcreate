//! Game runtime: chunk streaming, input state, player physics, HUD.
//!
//! App-side glue between winit events, the world (terrain scheduler +
//! chunk map), and the renderer. Mesh upload lands when the C++ mesher
//! merges (M4); a no-op mesher keeps this compiling until then.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, BlockPos, ChunkHandle, ChunkPos, Stage};
use mcv_entity::combat;
use mcv_entity::defs::speed_m_s;
use mcv_entity::spawner;
use mcv_entity::{Mob, MobId};
use mcv_game::{step_entity, Entity, Player, VoxelAccess};
use mcv_platform::touch::TouchState;
use mcv_render::gpu::RenderChunk;
use mcv_render::{text, Camera, HudQuad};

pub const RENDER_DIST: i32 = 8;
pub const HOTBAR: [u16; 9] = [1, 2, 3, 4, 8, 6, 7, 5, 10];

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

/// Abstraction over the C++ mesher so the runtime wiring can land before
/// the mesher itself merges.
pub trait ChunkMesher: Send {
    /// `handles` = 3x3 neighbourhood, row-major (dz outer), center = [4].
    /// 返回引擎侧组装好的 [`RenderChunk`]（游戏层不触 wgpu，上传经
    /// [`mcv_render::gpu::MeshUploader`]）。
    fn build(&mut self, pos: ChunkPos, handles: &[Arc<ChunkHandle>; 9]) -> Option<RenderChunk>;
}

/// Real mesher: C++ greedy mesh via mcv_mesher + GPU upload.
pub struct CxxMesher {
    mesher: mcv_mesher::Mesher,
    uploader: mcv_render::gpu::MeshUploader,
}

impl CxxMesher {
    pub fn new(budget: u64, uploader: mcv_render::gpu::MeshUploader) -> Self {
        Self {
            mesher: mcv_mesher::Mesher::new(budget).expect("mesh pool"),
            uploader,
        }
    }
}

impl ChunkMesher for CxxMesher {
    fn build(&mut self, pos: ChunkPos, handles: &[Arc<ChunkHandle>; 9]) -> Option<RenderChunk> {
        // Copy the 9 neighbourhoods out (locks taken one at a time).
        let mut voxels = vec![0u16; 9 * 65536];
        let mut lights = vec![0xF0; 9 * 65536]; // sky=15 until light wires in
        for (i, h) in handles.iter().enumerate() {
            voxels[i * 65536..(i + 1) * 65536]
                .copy_from_slice(bytemuck::cast_slice(h.voxels.read().unwrap().as_slice()));
            lights[i * 65536..(i + 1) * 65536].copy_from_slice(&h.light.read().unwrap()[..]);
        }
        let slots: [Option<mcv_mesher::Slot>; 9] = std::array::from_fn(|i| {
            Some(mcv_mesher::Slot {
                voxels: &voxels[i * 65536..(i + 1) * 65536],
                light: &lights[i * 65536..(i + 1) * 65536],
            })
        });
        let opaque = self.mesher.build(&slots, mcv_mesher::MESH_OPAQUE).ok()?;
        let water = self.mesher.build(&slots, mcv_mesher::MESH_WATER).ok();
        // 裸字节交给引擎上传；水的索引缓冲复用 opaque 顶点缓冲（既有语义）。
        let origin = [16.0 * pos.x as f32, 0.0, 16.0 * pos.z as f32];
        Some(self.uploader.build_chunk(
            origin,
            opaque.vertex_data(),
            opaque.indices(),
            water.as_ref().map(|w| w.indices()),
        ))
    }
}

/// Voxel view over the loaded chunk map.
pub struct WorldView<'a> {
    pub chunks: &'a HashMap<ChunkPos, Arc<ChunkHandle>>,
}

impl VoxelAccess for WorldView<'_> {
    fn block(&self, p: BlockPos) -> BlockId {
        let Some(chunk) = self.chunks.get(&p.chunk()) else {
            return BlockId(1); // unloaded = solid stone (physics safety)
        };
        if chunk.stage() == Stage::Empty {
            return BlockId(1);
        }
        let [lx, ly, lz] = p.local();
        chunk.voxels.read().unwrap()[ly << 8 | lz << 4 | lx]
    }

    fn light(&self, _p: BlockPos) -> u8 {
        15 // M4 wires real light
    }

    fn chunk_loaded(&self, c: ChunkPos) -> bool {
        self.chunks.contains_key(&c)
    }
}

#[derive(Default)]
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
}

pub struct GameRuntime {
    pub seed: u64,
    pub chunks: HashMap<ChunkPos, Arc<ChunkHandle>>,
    pub scheduler: mcv_worldgen::TerrainScheduler,
    pub player: Player,
    pub input: InputState,
    pub time_ticks: u64,
    pub mesher: Box<dyn ChunkMesher>,
    pub save_dir: std::path::PathBuf,
    render_chunks: Vec<RenderChunk>,
    spawned: bool,
    border_synced: HashMap<ChunkPos, u8>,
    pub mobs: Vec<Mob>,
    pub attack_ticker: f32,
    spawn_cooldown: u32,
    pub player_xp: u32,
    pub hotbar_slot: Option<mcv_item::ItemStack>,
    pub touch: TouchState,
    pub mode: GameMode,
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
    /// 死亡中（health 归零）：app 层画死亡界面，respawn() 复活。
    pub dead: bool,
    /// 离地时的 y（落地按 26.1 规则算摔落伤害：floor(高度−3)）。
    fall_y: Option<f32>,
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

impl GameRuntime {
    pub fn new(
        seed: u64,
        uploader: mcv_render::gpu::MeshUploader,
        save_dir: std::path::PathBuf,
        mode: GameMode,
    ) -> Self {
        Self {
            seed,
            chunks: HashMap::new(),
            scheduler: mcv_worldgen::TerrainScheduler::new(seed, mcv_core::world_worker_count()),
            player: Player::default(),
            input: InputState::default(),
            time_ticks: 6_000, // noon start
            mesher: Box::new(CxxMesher::new(256 << 20, uploader)),
            save_dir,
            render_chunks: Vec::new(),
            spawned: false,
            border_synced: HashMap::new(),
            mobs: Vec::new(),
            attack_ticker: 20.0, // ready
            spawn_cooldown: 0,
            player_xp: 0,
            hotbar_slot: Some(mcv_item::ItemStack::new(mcv_item::IRON_SWORD_INDEX, 1)),
            touch: TouchState::default(),
            mode,
            hardcore_death: false,
            render_dist: RENDER_DIST,
            sens: 1.0,
            cam_type: CameraType::default(),
            audio: mcv_audio::AudioManager::silent(mcv_audio::default_sounds_dir()),
            step_dist: 0.0,
            dead: false,
            fall_y: None,
        }
    }

    /// 玩家受伤（26.1 LivingEntity.hurt 简化）：无敌帧拒绝、击退、受伤音、
    /// 死亡置位。`from`=伤害来源（None = 环境伤害，不击退）。
    pub fn hurt_player(&mut self, amount: f32, from: Option<Vec3>) {
        let p = &mut self.player;
        if p.invulnerable > 0 || p.health <= 0.0 || self.mode == GameMode::Creative {
            return;
        }
        p.health -= amount;
        p.invulnerable = 10;
        if let Some(src) = from {
            let push = glam::Vec3::new(p.pos.x - src.x, 0.0, p.pos.z - src.z);
            let kb = mcv_entity::combat::knockback_velocity(p.vel, p.on_ground, 0.0, 0.5, push);
            p.vel = kb;
        }
        if p.health <= 0.0 {
            p.health = 0.0;
            self.dead = true;
            if self.mode == GameMode::Hardcore {
                self.hardcore_death = true;
            }
        }
        let pos = [p.pos.x, p.pos.y, p.pos.z];
        let id = [
            mcv_audio::SoundId::PlayerHurt1,
            mcv_audio::SoundId::PlayerHurt2,
        ][fast_rand() as usize & 1];
        self.audio.play_at(id, pos, pos, 1.0);
    }

    /// 死亡界面「重生」：满状态回出生点上方。
    pub fn respawn(&mut self) {
        self.player.health = 20.0;
        self.player.hunger = 20.0;
        self.player.exhaustion = 0.0;
        self.player.invulnerable = 20;
        self.player.pos = Vec3::new(8.5, 200.0, 8.5);
        self.player.vel = Vec3::ZERO;
        self.player.flying = self.mode == GameMode::Creative;
        self.dead = false;
        self.fall_y = None;
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
                self.seed = meta.seed;
                self.time_ticks = meta.day_time;
                self.mode = GameMode::from_u8(meta.mode);
                if let Some(p) = meta.player {
                    self.player.pos = Vec3::new(p.x, p.y, p.z);
                    self.player.yaw = p.yaw;
                    self.player.pitch = p.pitch;
                    self.player.flying = p.flying;
                    self.player.sel_slot = p.sel_slot as usize;
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
            }),
        };
        let tmp = self.save_dir.join("level.meta.tmp");
        if std::fs::create_dir_all(&self.save_dir).is_ok()
            && std::fs::write(&tmp, meta.encode()).is_ok()
        {
            let _ = std::fs::rename(&tmp, self.save_dir.join("level.meta"));
        }
    }

    /// Persists chunks with the SAVE dirty bit (region files), clearing the
    /// bit. Called periodically and before unload.
    pub fn save_dirty(&mut self, only: Option<ChunkPos>) {
        let keys: Vec<ChunkPos> = match only {
            Some(p) => vec![p],
            None => self.chunks.keys().copied().collect(),
        };
        for pos in keys {
            let Some(handle) = self.chunks.get(&pos) else {
                continue;
            };
            if handle.dirty() & mcv_core::dirty::SAVE == 0 {
                continue;
            }
            if (handle.stage() as u8) < (Stage::TerrainReady as u8) {
                continue;
            }
            let (rx, rz) = mcv_save::chunk_region(pos.x, pos.z);
            let local = mcv_save::chunk_local(pos.x, pos.z);
            let mut region = match mcv_save::RegionFile::open(&self.save_dir, rx, rz) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("region open failed {rx},{rz}: {e}");
                    continue;
                }
            };
            let voxels = handle.voxels.read().unwrap();
            let ids = bytemuck::cast_slice(voxels.as_slice());
            if let Err(e) = region.save_chunk(local, ids) {
                log::error!("chunk save failed {pos:?}: {e}");
            } else {
                handle.clear_dirty(mcv_core::dirty::SAVE);
            }
        }
    }

    pub fn camera(&self, aspect: f32) -> Camera {
        let mut cam = Camera {
            pos: self.player.pos,
            yaw: self.player.yaw,
            pitch: self.player.pitch,
            fov_y: 1.25,
            aspect,
            near: 0.1,
            far: (self.render_dist * 16) as f32 * 1.6,
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
            if let Some((hit, _)) = dda_hit(&view, eye, d * sign, THIRD_PERSON_DIST + 0.5) {
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
    /// unload far ones.
    pub fn stream(&mut self) {
        let center = ChunkPos::new(
            (self.player.pos.x / 16.0).floor() as i32,
            (self.player.pos.z / 16.0).floor() as i32,
        );
        // unload (saving dirty chunks first)
        let far: Vec<ChunkPos> = self
            .chunks
            .keys()
            .filter(|c| {
                (c.x - center.x).abs() > self.render_dist + 2
                    || (c.z - center.z).abs() > self.render_dist + 2
            })
            .copied()
            .collect();
        for c in far {
            self.save_dirty(Some(c));
            self.chunks.remove(&c);
            self.render_chunks
                .retain(|r| r.origin[0] != 16.0 * c.x as f32 || r.origin[2] != 16.0 * c.z as f32);
        }
        // request in ring order; bounded per frame
        let mut budget = 4;
        'outer: for r in 0..=self.render_dist {
            for dx in -r..=r {
                for dz in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue; // ring only
                    }
                    let pos = ChunkPos::new(center.x + dx, center.z + dz);
                    if !self.chunks.contains_key(&pos) {
                        let handle = Arc::new(ChunkHandle::new(pos));
                        if self.try_load_saved(&handle) {
                            self.chunks.insert(pos, handle);
                        } else {
                            self.chunks.insert(pos, Arc::new(ChunkHandle::new(pos)));
                            self.scheduler.request(pos);
                        }
                        budget -= 1;
                        if budget == 0 {
                            break 'outer;
                        }
                    }
                }
            }
        }
        // drain completed terrain
        while let Ok(result) = self.scheduler.results().try_recv() {
            match result {
                mcv_worldgen::GenResult::Terrain(Ok(out)) => {
                    if let Some(handle) = self.chunks.get(&out.pos) {
                        mcv_worldgen::commit_terrain(handle, out);
                    }
                }
                mcv_worldgen::GenResult::Terrain(Err((pos, rc))) => {
                    log::error!("terrain gen failed at {pos:?}: {rc}");
                }
            }
        }

        // spawn drop: once the spawn chunk has terrain, place the player on
        // the surface (unless a saved position was loaded)
        if !self.spawned && self.player.pos == Vec3::ZERO {
            if let Some(handle) = self.chunks.get(&ChunkPos::new(0, 0)) {
                if (handle.stage() as u8) >= (Stage::TerrainReady as u8) {
                    let hm = handle.heightmap.read().unwrap();
                    let y = hm[(8 << 4) | 8];
                    self.player.pos = Vec3::new(8.5, f32::from(y) + 1.0, 8.5);
                    self.spawned = true;
                }
            }
        }
        // light init on newly-terrain-ready chunks (budgeted, main thread)
        let mut light_budget = 2;
        let keys: Vec<ChunkPos> = self.chunks.keys().copied().collect();
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
                let hm: Vec<u8> = hm_locked.to_vec();
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

        // border sync: mark pairs once both ends are LightLocalReady
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
                        marks.push((npos, Self::opposite_side(bit)));
                    }
                }
            }
            self.border_synced.insert(*pos, cur);
            for (npos, obit) in marks {
                let ns = self.border_synced.entry(npos).or_insert(0u8);
                *ns |= 1 << obit;
            }
        }

        // mesh chunks: 3x3 loaded, center lit, dirty or missing
        let mut remesh_budget = 2;
        let keys: Vec<ChunkPos> = self.chunks.keys().copied().collect();
        for pos in keys {
            if remesh_budget == 0 {
                break;
            }
            let handle = self.chunks[&pos].clone();
            if (handle.stage() as u8) < (Stage::LightLocalReady as u8) {
                continue;
            }
            if !self.neighbors_ready(pos) {
                continue;
            }
            let already = self
                .render_chunks
                .iter()
                .any(|r| r.origin[0] == 16.0 * pos.x as f32 && r.origin[2] == 16.0 * pos.z as f32);
            let dirty_mesh = handle.dirty() & mcv_core::dirty::MESH != 0;
            if already && !dirty_mesh {
                continue;
            }
            let mut handles: [Arc<ChunkHandle>; 9] = core::array::from_fn(|_| handle.clone());
            for dz in -1i32..=1 {
                for dx in -1i32..=1 {
                    let idx = ((dz + 1) * 3 + (dx + 1)) as usize;
                    handles[idx] = self.chunks[&ChunkPos::new(pos.x + dx, pos.z + dz)].clone();
                }
            }
            if let Some(rc) = self.mesher.build(pos, &handles) {
                let origin = rc.origin;
                self.render_chunks
                    .retain(|r| r.origin[0] != origin[0] || r.origin[2] != origin[2]);
                self.render_chunks.push(rc);
                handle.clear_dirty(mcv_core::dirty::MESH);
                remesh_budget -= 1;
            }
        }
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
        let mut ids = vec![0u16; 65536];
        if region
            .load_chunk(mcv_save::chunk_local(pos.x, pos.z), &mut ids)
            .is_err()
        {
            return false;
        }
        *handle.voxels.write().unwrap() = load_voxels(&ids);
        *handle.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&ids);
        handle.advance_to(Stage::TerrainReady);
        true
    }

    fn neighbors_ready(&self, pos: ChunkPos) -> bool {
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(h) = self.chunks.get(&ChunkPos::new(pos.x + dx, pos.z + dz)) {
                    if (h.stage() as u8) < (Stage::TerrainReady as u8) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }
        true
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
        // 摇杆 → 移动方向
        if let Some((dx, dy)) = self.touch.stick_direction() {
            // 摇杆向上推 = 前进
            self.input.forward = dy < -0.3;
            self.input.back = dy > 0.3;
            self.input.left = dx < -0.3;
            self.input.right = dx > 0.3;
            let len = (dx * dx + dy * dy).sqrt();
            self.input.sprint = len > 0.85;
        } else if self.touch.stick_vec == (0.0, 0.0) {
            self.input.forward = false;
            self.input.back = false;
            self.input.left = false;
            self.input.right = false;
            self.input.sprint = false;
        }
        if self.touch.jump_held {
            self.input.jump = true;
        }
        if self.touch.mine_held {
            self.input.mining = true;
            self.interact(false); // 挖掘 / 攻击（含跨帧冷却逻辑）
        }
    }

    pub fn fixed_step(&mut self, dt: f32) {
        self.apply_touch_input();
        if self.dead {
            // 死亡界面：尸体不响应输入，仅重力继续
            self.input = Default::default();
        }
        self.attack_ticker = (self.attack_ticker + dt).min(20.0);
        self.spawn_cooldown = self.spawn_cooldown.saturating_sub(1);

        // ---- natural spawning (budgeted every 20 ticks) ----
        if self.spawn_cooldown == 0 {
            self.spawn_cooldown = 20;
            self.try_natural_spawn();
        }

        // ---- mob AI + physics ----
        // WorldView 内联构造：字段分离借用（&self.chunks 与 &mut self.mobs/
        // player 无冲突），不持长命 view，中途才能调 &mut self 方法
        let player_pos = self.player.pos;
        let day = self.day_factor();
        let mut melee_hits: Vec<(Vec3, f32)> = Vec::new();
        for mob in self.mobs.iter_mut() {
            mob.invulnerable = mob.invulnerable.saturating_sub(1);
            let def = mob.def();
            let to_player = player_pos - mob.pos;
            let dist_sqr = to_player.length_squared();
            let speed = speed_m_s(def.speed_attr);
            let wish = if def.hostile && dist_sqr < def.follow_range * def.follow_range {
                // chase
                let dir = Vec3::new(to_player.x, 0.0, to_player.z).normalize_or_zero();
                mob.yaw = dir.z.atan2(dir.x);
                dir * speed
            } else {
                // wander: random direction changes on idle ticks
                if mob.idle_ticks % 120 == 0 && (fast_rand() & 3) == 0 {
                    mob.yaw = (mob.idle_ticks as f32 * 0.7) % std::f32::consts::TAU;
                }
                Vec3::new(mob.yaw.sin(), 0.0, -mob.yaw.cos()) * speed * 0.3
            };
            let melee = def.hostile && def.attack_damage > 0.0 && dist_sqr < 2.25;
            let input = mcv_game::StepInput {
                wish_dir: if melee { Vec3::ZERO } else { wish },
                jump: mob.on_ground && to_player.y > 1.0 && dist_sqr < 16.0,
                in_water: false,
                sneak: false,
            };
            let mut body = Entity {
                pos: mob.pos,
                vel: mob.vel,
                on_ground: mob.on_ground,
            };
            step_entity(
                &WorldView {
                    chunks: &self.chunks,
                },
                &mut body,
                def.half_size,
                &input,
            );
            mob.pos = body.pos;
            mob.vel = body.vel;
            mob.on_ground = body.on_ground;
            mob.idle_ticks += 1;
            // invulnerable 复用为近战冷却：命中后置 20 tick（1s，26.1 僵尸节奏）
            if melee && mob.invulnerable == 0 {
                mob.invulnerable = 20;
                melee_hits.push((mob.pos, def.attack_damage));
            }
        }
        self.mobs.retain(|m| m.health > 0.0);
        for (src, dmg) in melee_hits {
            self.hurt_player(dmg.max(1.0), Some(src));
        }

        // ---- 玩家物理（mcv_game::step，60 Hz 固定步）----
        {
            let f = self.camera(1.0).dir();
            let f = Vec3::new(f.x, 0.0, f.z)
                .try_normalize()
                .unwrap_or(Vec3::new(0.0, 0.0, -1.0));
            let r = f.cross(Vec3::Y);
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
            let wish_dir = wish.normalize_or_zero();
            let was_air = !self.player.on_ground;
            let fall_v = self.player.vel.y.min(0.0);
            let jumped_off = i.jump && self.player.on_ground;
            let before = self.player.pos;
            let in_water = self.in_water(&WorldView {
                chunks: &self.chunks,
            });
            // 空中累计最高点（MC fallDistance：上升不计，下落距离 = 最高点到落点）
            if !self.player.flying && !in_water {
                if self.player.on_ground {
                    self.fall_y = None;
                } else {
                    let y = self.player.pos.y;
                    self.fall_y = Some(self.fall_y.map_or(y, |f| f.max(y)));
                }
            } else {
                self.fall_y = None; // 飞行/游泳免疫摔落
            }
            let step_input = mcv_game::StepInput {
                wish_dir,
                jump: i.jump,
                in_water,
                sneak: i.sneak,
            };
            mcv_game::step(
                &WorldView {
                    chunks: &self.chunks,
                },
                &mut self.player,
                &step_input,
            );
            // ---- 行为音效：脚步 / 落地 ----
            let moved = (self.player.pos - before).length();
            self.step_dist += moved;
            if self.player.on_ground && was_air {
                if fall_v < -3.0 {
                    let p = self.player.pos;
                    self.audio.play_at(
                        mcv_audio::SoundId::LandFall,
                        [p.x, p.y, p.z],
                        [p.x, p.y, p.z],
                        0.5,
                    );
                }
                // 摔落伤害（MC: damage = floor(fallDistance - 3)）
                if let Some(top) = self.fall_y.take() {
                    let dmg = (top - self.player.pos.y - 3.0).floor().max(0.0);
                    if dmg > 0.0 {
                        self.hurt_player(dmg, None);
                    }
                }
            } else if self.player.on_ground {
                self.fall_y = None;
            }
            if self.player.on_ground && self.step_dist > 2.2 {
                self.step_dist = 0.0;
                let p = self.player.pos;
                let under = WorldView {
                    chunks: &self.chunks,
                }
                .block(BlockPos::new(p.x as i32, (p.y - 0.5) as i32, p.z as i32))
                .0;
                if let Some(sid) = step_sound(under) {
                    self.audio
                        .play_at(sid, [p.x, p.y, p.z], [p.x, p.y, p.z], 0.35);
                }
            }
            // ---- 生存统计（26.1 和平难度规则，粗化 exhaustion）----
            self.player.invulnerable = self.player.invulnerable.saturating_sub(1);
            if self.mode != GameMode::Creative {
                let p = &mut self.player;
                p.exhaustion += moved * if self.input.sprint { 0.02 } else { 0.01 };
                if jumped_off {
                    p.exhaustion += 0.2;
                }
                if p.exhaustion >= 4.0 {
                    p.exhaustion -= 4.0;
                    p.hunger = (p.hunger - 1.0).max(0.0);
                }
                // 饱和回血：hunger>17 每 4s 回 1 心（MC naturalRegeneration）
                if p.hunger > 17.0 && p.health > 0.0 && p.health < 20.0 {
                    p.health = (p.health + dt * 0.25).min(20.0);
                }
                // 饥饿掉血：hunger=0 掉至 10 为止（和平难度下限）
                if p.hunger <= 0.0 && p.health > 10.0 {
                    p.health = (p.health - dt * 0.25).max(10.0);
                }
            }
        }

        // ---- 虚空伤害（y < -10）：无视无敌帧的重击，死亡后传送回出生点上方 ----
        if self.player.pos.y < -10.0 {
            self.player.invulnerable = 0;
            self.hurt_player(40.0, None);
            if self.dead {
                self.player.pos = Vec3::new(8.5, 200.0, 8.5);
                self.player.vel = Vec3::ZERO;
                self.player.flying = self.mode == GameMode::Creative;
            }
        }
        let _ = day;
    }

    fn day_factor(&self) -> f32 {
        mcv_render::sun_state(self.time_ticks).1
    }

    /// NaturalSpawner-lite: a few random loaded chunks per budget window,
    /// 3 groups × up to 4 walk positions, cap + distance + light gates.
    fn try_natural_spawn(&mut self) {
        if self.chunks.is_empty() {
            return;
        }
        let monster_cap = spawner::SPAWN_CAPS
            .iter()
            .find(|(c, _, _, _)| *c == spawner::SpawnCategory::Monster)
            .map(|(_, cap, _, _)| *cap)
            .unwrap_or(0);
        let monsters = self.mobs.iter().filter(|m| m.def().hostile).count() as u32;
        if monsters >= monster_cap {
            return;
        }
        let center = (
            (self.player.pos.x / 16.0).floor() as i32,
            (self.player.pos.z / 16.0).floor() as i32,
        );
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
            let spawn_x = (fast_rand() % 16) as i32 + pos.x * 16;
            let spawn_z = (fast_rand() % 16) as i32 + pos.z * 16;
            let mut rng = spawn_rng();
            for _ in 0..spawner::GROUPS_PER_CHUNK {
                let mut p = glam::Vec3::new(spawn_x as f32, 0.0, spawn_z as f32);
                for _ in 0..spawner::ATTEMPTS_PER_GROUP {
                    p = spawner::group_walk(p, &mut rng);
                    let dist_sqr = (self.player.pos - p).length_squared();
                    if dist_sqr < spawner::MIN_PLAYER_DIST_SQR {
                        continue;
                    }
                    let block_pos = BlockPos::new(p.x as i32, 0, p.z as i32);
                    let cx = block_pos.chunk();
                    let Some(h) = self.chunks.get(&cx) else {
                        continue;
                    };
                    let [lx, _, lz] = block_pos.local();
                    let surface = h.heightmap.read().unwrap()[(lz << 4) | lx];
                    if surface == 0 || surface > 250 {
                        continue;
                    }
                    // find a dark spot near the surface column
                    let y = f32::from(surface) + 1.0;
                    let cell = BlockPos::new(p.x as i32, surface as i32, p.z as i32);
                    let [lxc, lyc, lzc] = cell.local();
                    let light_byte = h.light.read().unwrap()[lyc << 8 | lzc << 4 | lxc];
                    let sky = light_byte >> 4;
                    let blk = light_byte & 0xF;
                    let night = self.day_factor() < 0.4;
                    let dark_ok = if night {
                        spawner::light_allows_hostile(sky.min(4), blk, &mut rng)
                    } else {
                        spawner::light_allows_hostile(sky, blk, &mut rng) && surface > 130
                    };
                    if !dark_ok {
                        continue;
                    }
                    let id = if night || surface > 130 {
                        [MobId::ZOMBIE, MobId::SKELETON, MobId::CREEPER][(rng() as usize) % 3]
                    } else {
                        [MobId::COW, MobId::PIG, MobId::SHEEP][(rng() as usize) % 3]
                    };
                    p.y = y;
                    self.mobs.push(Mob::new(id, p));
                    break;
                }
            }
        }
    }

    /// 创造模式：挖掘无间隔。返回是否跳过冷却。
    pub fn instant_mine(&self) -> bool {
        self.mode == GameMode::Creative
    }

    /// Left-click attack: crosshair ray over mobs first, else mine block.
    pub fn attack(&mut self) {
        self.interact(false);
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
        };
        let out = combat::resolve_attack(&ctx);
        self.attack_ticker = 0.0;
        let mut best: Option<(usize, f32)> = None;
        for (i, mob) in self.mobs.iter().enumerate() {
            let to = mob.pos + glam::Vec3::new(0.0, 1.0, 0.0) - eye;
            let dist = to.length();
            if dist > 3.5 {
                continue;
            }
            let cos = to.normalize_or_zero().dot(dir);
            if cos > 0.92 && best.is_none_or(|(_, d)| dist < d) {
                best = Some((i, dist));
            }
        }
        if let Some((i, _)) = best {
            let mob = &mut self.mobs[i];
            let xp = mob.def().xp;
            if combat::apply_hurt(mob, out.damage, 0).is_some() && mob.health <= 0.0 {
                self.mobs.remove(i);
                self.player_xp += xp;
            }
        }
        if let Some(item) = &mut self.hotbar_slot {
            let _ = item.hurt(1, &mut || 0);
        }
    }

    fn hotbar_item(&self) -> Option<mcv_item::ItemStack> {
        self.hotbar_slot.clone()
    }

    #[allow(dead_code)] // wired into physics once mcv_game::step merges
    #[allow(dead_code)] // wired into physics once mcv_game::step merges
    fn in_water(&self, view: &WorldView) -> bool {
        let p = self.player.pos;
        let b = BlockPos::new(p.x as i32, (p.y + 0.5) as i32, p.z as i32);
        view.block(b).def().liquid
    }

    /// Mouse look.
    pub fn look(&mut self, dx: f64, dy: f64) {
        let k = 0.0025 * self.sens;
        self.player.yaw += dx as f32 * k;
        self.player.pitch = (self.player.pitch - dy as f32 * k).clamp(-1.55, 1.55);
    }

    /// Break / place at the crosshair. Uses a temporary inline DDA until the
    /// physics module merges; the voxel write path is final.
    pub fn interact(&mut self, place: bool) {
        let view = WorldView {
            chunks: &self.chunks,
        };
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let dir = self.camera(1.0).dir();
        let Some((hit, normal)) = dda_hit(&view, eye, dir, 5.0) else {
            return;
        };
        let target = if place {
            BlockPos::new(hit.x + normal[0], hit.y + normal[1], hit.z + normal[2])
        } else {
            hit
        };
        if place {
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
        }
        if let Some(handle) = self.chunks.get(&target.chunk()) {
            let [lx, ly, lz] = target.local();
            let new_id = BlockId(if place {
                HOTBAR[self.player.sel_slot % 9]
            } else {
                0
            });
            let old = handle.voxels.read().unwrap()[ly << 8 | lz << 4 | lx];
            // 破坏按原方块发声，放置按新方块发声（26.1 GameRenderer 行为音）
            let snd_vid = if place { new_id.0 } else { old.0 };
            handle.voxels.write().unwrap()[ly << 8 | lz << 4 | lx] = new_id;
            handle.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
            if old.0 != 0 || place {
                if let Some(sid) = dig_sound(snd_vid) {
                    let p = [
                        target.x as f32 + 0.5,
                        target.y as f32 + 0.5,
                        target.z as f32 + 0.5,
                    ];
                    self.audio.play_at(sid, p, [eye.x, eye.y, eye.z], 1.0);
                }
            }
        }
    }

    pub fn render_chunks(&self) -> &[RenderChunk] {
        &self.render_chunks
    }

    /// HUD：MC 26.1 风格（准星 / 快捷栏 / 心 / 饥饿，Gui.java 常数），
    /// `gui` 为 None 时整体回退旧程序化绘制；触屏摇杆程序化，
    /// `show_touch`（死亡界面等场景传 false 隐藏摇杆）。
    pub fn build_hud(
        &self,
        width: f32,
        height: f32,
        gui: Option<&mcv_render::gui::SpriteSheet>,
        show_touch: bool,
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
            for (i, &id) in HOTBAR.iter().enumerate() {
                if id != 0 {
                    quads.push(text::tile_icon(
                        mcv_core::BLOCKS[id as usize].tiles[2],
                        width * 0.5 - 88.0 * s + i as f32 * 20.0 * s,
                        height - 19.0 * s,
                        16.0 * s,
                    ));
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
            for (i, &id) in HOTBAR.iter().enumerate() {
                let x = x0 + i as f32 * slot;
                quads.push(text::rect(
                    x + 1.0,
                    y0 + 1.0,
                    slot - 2.0,
                    slot - 2.0,
                    [0.25, 0.25, 0.28, 0.8],
                ));
                if id != 0 {
                    quads.push(text::tile_icon(
                        mcv_core::BLOCKS[id as usize].tiles[2],
                        x + 5.0,
                        y0 + 5.0,
                        slot - 10.0,
                    ));
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

/// Global XOR-shift rand for spawn jitter (deterministic per sequence).
fn fast_rand() -> u32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0x243F6A8885A308D3);
    let v = C.fetch_add(0x9E3779B97F4A7C15, Ordering::Relaxed);
    let z = (v ^ (v >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    ((z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB) >> 33) as u32
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

fn load_voxels(ids: &[u16]) -> Box<[BlockId; 65536]> {
    debug_assert_eq!(ids.len(), 65536);
    bytemuck::cast_slice::<u16, BlockId>(ids)
        .to_vec()
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| panic!("wrong voxel slice length"))
}

/// Temporary inline Amanatides-Woo DDA; replaced by mcv_game::raycast when
/// the physics module merges.
fn dda_hit(
    view: &WorldView,
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
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
    for _ in 0..64 {
        let bp = BlockPos::new(pos.x, pos.y, pos.z);
        if view.block(bp).def().solid {
            return Some((bp, normal));
        }
        if t_max.x < t_max.y && t_max.x < t_max.z {
            if t_max.x > max_dist {
                return None;
            }
            pos.x += step.x;
            t_max.x += t_delta.x;
            normal = [-step.x, 0, 0];
        } else if t_max.y < t_max.z {
            if t_max.y > max_dist {
                return None;
            }
            pos.y += step.y;
            t_max.y += t_delta.y;
            normal = [0, -step.y, 0];
        } else {
            if t_max.z > max_dist {
                return None;
            }
            pos.z += step.z;
            t_max.z += t_delta.z;
            normal = [0, 0, -step.z];
        }
    }
    None
}

/// 挖掘/放置音效材质映射（BLOCKS 表序：0air 1stone 2dirt 3grass 4sand 5water
/// 6log 7leaves 8planks 9cobble 10bedrock 11snow_grass 12/13花）。
/// 26.1 素材库无 dig/dirt 组，泥土/草/沙共用 grass 音组（见 mcv_audio 注释）。
fn dig_sound(vid: u16) -> Option<mcv_audio::SoundId> {
    use mcv_audio::SoundId as S;
    match vid {
        1 | 9 | 10 => Some(S::DigStone),
        2 | 3 | 4 | 7 | 11 => Some(S::DigDirt),
        6 | 8 => Some(S::DigWood),
        _ => None,
    }
}

/// 脚步材质映射：草方块踩草地音，沙/石踩石头音，木板/原木踩木头音。
fn step_sound(vid: u16) -> Option<mcv_audio::SoundId> {
    use mcv_audio::SoundId as S;
    match vid {
        1 | 4 | 9 | 10 => Some(S::StepStone),
        2 | 3 | 7 | 11 => Some(S::StepGrass),
        6 | 8 => Some(S::StepWood),
        _ => None,
    }
}
