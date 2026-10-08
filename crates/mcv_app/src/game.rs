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
use mcv_render::gpu::RenderChunk;
use mcv_render::{text, Camera, HudQuad};

pub const RENDER_DIST: i32 = 8;
pub const HOTBAR: [u8; 9] = [1, 2, 3, 4, 8, 6, 7, 5, 10];

/// GPU mesh produced by a [`ChunkMesher`].
pub struct MeshGpu {
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub opaque_range: std::ops::Range<u32>,
    /// Separate water index buffer + range (water is meshed per pass).
    pub water: Option<(wgpu::Buffer, std::ops::Range<u32>)>,
}

/// Abstraction over the C++ mesher so the runtime wiring can land before
/// the mesher itself merges.
pub trait ChunkMesher: Send {
    /// `handles` = 3x3 neighbourhood, row-major (dz outer), center = [4].
    fn build(&mut self, handles: &[Arc<ChunkHandle>; 9]) -> Option<MeshGpu>;
}

/// Real mesher: C++ greedy mesh via mcv_mesher + GPU upload.
pub struct CxxMesher {
    mesher: mcv_mesher::Mesher,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl CxxMesher {
    pub fn new(budget: u64, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            mesher: mcv_mesher::Mesher::new(budget).expect("mesh pool"),
            device,
            queue,
        }
    }
}

impl ChunkMesher for CxxMesher {
    fn build(&mut self, handles: &[Arc<ChunkHandle>; 9]) -> Option<MeshGpu> {
        // Copy the 9 neighbourhoods out (locks taken one at a time).
        let mut voxels = vec![0u8; 9 * 65536];
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
        let to_gpu =
            |buf: &mcv_ffi::CxxMeshBuffer| -> (wgpu::Buffer, wgpu::Buffer, std::ops::Range<u32>) {
                let vb = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: (buf.vertex_data().len() as u64).max(1),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: true,
                });
                self.queue.write_buffer(&vb, 0, buf.vertex_data());
                vb.unmap();
                let ib = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: (buf.indices().len() as u64 * 4).max(4),
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: true,
                });
                self.queue
                    .write_buffer(&ib, 0, bytemuck::cast_slice(buf.indices()));
                ib.unmap();
                let n = buf.indices().len() as u32;
                (vb, ib, 0..n)
            };
        let (vertex_buf, index_buf, opaque_range) = to_gpu(&opaque);
        let water = water.map(|w| {
            let (vb, ib, range) = to_gpu(&w);
            (vb, ib, range)
        });
        Some(MeshGpu {
            vertex_buf,
            index_buf,
            opaque_range,
            water: water.map(|(_, ib, r)| (ib, r)),
        })
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
    renderer: mcv_render::Renderer,
    render_chunks: Vec<RenderChunk>,
    spawned: bool,
    border_synced: HashMap<ChunkPos, u8>,
    pub mobs: Vec<Mob>,
    pub attack_ticker: f32,
    spawn_cooldown: u32,
    pub player_xp: u32,
    pub hotbar_slot: Option<mcv_item::ItemStack>,
}

impl GameRuntime {
    pub fn new(
        seed: u64,
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        save_dir: std::path::PathBuf,
    ) -> Self {
        let renderer = mcv_render::Renderer::new(device.clone(), queue.clone(), color_format, None);
        Self {
            seed,
            chunks: HashMap::new(),
            scheduler: mcv_worldgen::TerrainScheduler::new(seed, mcv_core::world_worker_count()),
            player: Player::default(),
            input: InputState::default(),
            time_ticks: 6_000, // noon start
            mesher: Box::new(CxxMesher::new(256 << 20, device.clone(), queue.clone())),
            save_dir,
            renderer,
            render_chunks: Vec::new(),
            spawned: false,
            border_synced: HashMap::new(),
            mobs: Vec::new(),
            attack_ticker: 20.0, // ready
            spawn_cooldown: 0,
            player_xp: 0,
            hotbar_slot: Some(mcv_item::ItemStack::new(mcv_item::IRON_SWORD_INDEX, 1)),
        }
    }

    pub fn renderer(&mut self) -> &mut mcv_render::Renderer {
        &mut self.renderer
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
            let bytes = unsafe { std::slice::from_raw_parts(voxels.as_ptr().cast(), 65536) };
            if let Err(e) = region.save_chunk(local, bytes) {
                log::error!("chunk save failed {pos:?}: {e}");
            } else {
                handle.clear_dirty(mcv_core::dirty::SAVE);
            }
        }
    }

    pub fn camera(&self, aspect: f32) -> Camera {
        Camera {
            pos: self.player.pos,
            yaw: self.player.yaw,
            pitch: self.player.pitch,
            fov_y: 1.25,
            aspect,
            near: 0.1,
            far: (RENDER_DIST * 16) as f32 * 1.6,
        }
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
                (c.x - center.x).abs() > RENDER_DIST + 2 || (c.z - center.z).abs() > RENDER_DIST + 2
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
        'outer: for r in 0..=RENDER_DIST {
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
                let voxels: Vec<u8> = bytemuck::cast_slice(voxels_locked.as_slice()).to_vec();
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
            if let Some(mesh) = self.mesher.build(&handles) {
                let origin = [16.0 * pos.x as f32, 0.0, 16.0 * pos.z as f32];
                let (water_index_buf, water_range) = match mesh.water {
                    Some((ib, r)) => (Some(ib), r),
                    None => (None, 0..0),
                };
                let rc = RenderChunk {
                    origin,
                    vertex_buf: mesh.vertex_buf,
                    index_buf: mesh.index_buf,
                    opaque_range: mesh.opaque_range,
                    water_index_buf,
                    water_range,
                    aabb: (
                        Vec3::new(origin[0], 0.0, origin[2]),
                        Vec3::new(origin[0] + 16.0, 256.0, origin[2] + 16.0),
                    ),
                };
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
        let mut bytes = vec![0u8; 65536];
        if region
            .load_chunk(mcv_save::chunk_local(pos.x, pos.z), &mut bytes)
            .is_err()
        {
            return false;
        }
        *handle.voxels.write().unwrap() = load_voxels(&bytes);
        *handle.heightmap.write().unwrap() = mcv_worldgen::recompute_heightmap(&bytes);
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

    pub fn fixed_step(&mut self, dt: f32) {
        self.attack_ticker = (self.attack_ticker + dt).min(20.0);
        self.spawn_cooldown = self.spawn_cooldown.saturating_sub(1);

        // ---- natural spawning (budgeted every 20 ticks) ----
        if self.spawn_cooldown == 0 {
            self.spawn_cooldown = 20;
            self.try_natural_spawn();
        }

        // ---- mob AI + physics ----
        let view = WorldView {
            chunks: &self.chunks,
        };
        let player_pos = self.player.pos;
        let day = self.day_factor();
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
            step_entity(&view, &mut body, def.half_size, &input);
            mob.pos = body.pos;
            mob.vel = body.vel;
            mob.on_ground = body.on_ground;
            mob.idle_ticks += 1;
            if melee && mob.invulnerable == 0 {
                // monster melee lands in player damage routing (M7 wiring)
            }
        }
        self.mobs.retain(|m| m.health > 0.0);
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
        self.player.yaw += dx as f32 * 0.0025;
        self.player.pitch = (self.player.pitch - dy as f32 * 0.0025).clamp(-1.55, 1.55);
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
            handle.voxels.write().unwrap()[ly << 8 | lz << 4 | lx] = new_id;
            handle.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
        }
    }

    pub fn render_chunks(&self) -> &[RenderChunk] {
        &self.render_chunks
    }

    /// HUD: crosshair + hotbar + debug line.
    pub fn build_hud(&self, width: f32, height: f32) -> Vec<HudQuad> {
        let mut quads = Vec::new();
        // crosshair
        let (cx, cy) = (width * 0.5 - 1.0, height * 0.5 - 8.0);
        quads.push(text::rect(cx, cy, 2.0, 16.0, [1.0, 1.0, 1.0, 0.75]));
        quads.push(text::rect(
            width * 0.5 - 8.0,
            height * 0.5 - 1.0,
            16.0,
            2.0,
            [1.0, 1.0, 1.0, 0.75],
        ));
        // hotbar
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
            if i == self.player.sel_slot % 9 {
                quads.push(text::rect(
                    x - 1.0,
                    y0 - 1.0,
                    slot + 2.0,
                    2.0,
                    [1.0, 1.0, 1.0, 0.9],
                ));
            }
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

fn load_voxels(bytes: &[u8]) -> Box<[BlockId; 65536]> {
    let mut out = Box::new([BlockId(0); 65536]);
    for (i, &b) in bytes.iter().enumerate() {
        out[i] = BlockId(b);
    }
    out
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
