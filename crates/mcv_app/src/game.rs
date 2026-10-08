//! Game runtime: chunk streaming, input state, player physics, HUD.
//!
//! App-side glue between winit events, the world (terrain scheduler +
//! chunk map), and the renderer. Mesh upload lands when the C++ mesher
//! merges (M4); a no-op mesher keeps this compiling until then.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mcv_core::{BlockId, ChunkHandle, ChunkPos, BlockPos, Stage};
use mcv_game::{Player, VoxelAccess};
use mcv_render::gpu::RenderChunk;
use mcv_render::{text, Camera, HudQuad};

pub const RENDER_DIST: i32 = 8;
pub const HOTBAR: [u8; 9] = [1, 2, 3, 4, 8, 6, 7, 5, 10];

/// GPU mesh produced by a [`ChunkMesher`].
pub struct MeshGpu {
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub opaque_range: std::ops::Range<u32>,
    pub water_range: std::ops::Range<u32>,
}

/// Abstraction over the C++ mesher so the runtime wiring can land before
/// the mesher itself merges.
pub trait ChunkMesher: Send {
    fn build(&mut self, center: &Arc<ChunkHandle>) -> Option<MeshGpu>;
}

/// Placeholder until the C++ mesher merges (renders nothing).
pub struct NoopMesher;
impl ChunkMesher for NoopMesher {
    fn build(&mut self, _center: &Arc<ChunkHandle>) -> Option<MeshGpu> {
        None
    }
}

/// Voxel view over the loaded chunk map.
pub struct WorldView<'a> {
    pub chunks: &'a HashMap<ChunkPos, Arc<ChunkHandle>>,
}

impl VoxelAccess for WorldView<'_> {
    fn block(&self, p: BlockPos) -> BlockId {
        let chunk = self.chunks.get(&p.chunk())?;
        let stage = chunk.stage();
        if stage == Stage::Empty {
            return BlockId(1); // treat unloaded as solid stone for physics safety
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
    pub mesher: Box<dyn ChunkMesher>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    render_chunks: Vec<RenderChunk>,
    stream_cursor: u32,
}

impl GameRuntime {
    pub fn new(seed: u64, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            seed,
            chunks: HashMap::new(),
            scheduler: mcv_worldgen::TerrainScheduler::new(seed, mcv_core::world_worker_count()),
            player: Player::default(),
            input: InputState::default(),
            time_ticks: 6_000, // noon start
            mesher: Box::new(NoopMesher),
            device,
            queue,
            render_chunks: Vec::new(),
            stream_cursor: 0,
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
        let center = self.player.pos.chunk();
        // unload
        let far: Vec<ChunkPos> = self
            .chunks
            .keys()
            .filter(|c| {
                (c.x - center.x).abs() > RENDER_DIST + 2 || (c.z - center.z).abs() > RENDER_DIST + 2
            })
            .copied()
            .collect();
        for c in far {
            self.chunks.remove(&c);
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
                        self.chunks.insert(pos, Arc::new(ChunkHandle::new(pos)));
                        self.scheduler.request(pos);
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
        // mesh chunks that are ready (center + 8 neighbours loaded)
        let mut remesh_budget = 2;
        let keys: Vec<ChunkPos> = self.chunks.keys().copied().collect();
        for pos in keys {
            if remesh_budget == 0 {
                break;
            }
            let handle = self.chunks[&pos].clone();
            if handle.stage() < Stage::TerrainReady {
                continue;
            }
            if !self.neighbors_ready(pos) {
                continue;
            }
            // TODO(m4): gate on Lit stage; for now mesh right after terrain
            if let Some(mesh) = self.mesher.build(&handle) {
                let origin = [16.0 * pos.x as f32, 0.0, 16.0 * pos.z as f32];
                let rc = RenderChunk {
                    origin,
                    vertex_buf: mesh.vertex_buf,
                    index_buf: mesh.index_buf,
                    opaque_range: mesh.opaque_range,
                    water_range: mesh.water_range,
                    aabb: (
                        Vec3::new(origin[0], 0.0, origin[2]),
                        Vec3::new(origin[0] + 16.0, 256.0, origin[2] + 16.0),
                    ),
                };
                self.render_chunks.retain(|r| r.origin[0] != origin[0] || r.origin[2] != origin[2]);
                self.render_chunks.push(rc);
                remesh_budget -= 1;
            }
        }
        let _ = self.stream_cursor;
    }

    fn neighbors_ready(&self, pos: ChunkPos) -> bool {
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(h) = self.chunks.get(&ChunkPos::new(pos.x + dx, pos.z + dz)) {
                    if h.stage() < Stage::TerrainReady {
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
        // Physics steps in via mcv_game once the physics module merges;
        // until then only look/stream are live.
        let _ = dt;
    }

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
            let new_id = BlockId(if place { HOTBAR[self.player.sel_slot % 9] } else { 0 });
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
        quads.push(text::rect(width * 0.5 - 8.0, height * 0.5 - 1.0, 16.0, 2.0, [1.0, 1.0, 1.0, 0.75]));
        // hotbar
        let slot = 40.0;
        let total = slot * 9.0;
        let x0 = width * 0.5 - total * 0.5;
        let y0 = height - slot - 8.0;
        quads.push(text::rect(x0 - 2.0, y0 - 2.0, total + 4.0, slot + 4.0, [0.1, 0.1, 0.1, 0.6]));
        for (i, &id) in HOTBAR.iter().enumerate() {
            let x = x0 + i as f32 * slot;
            quads.push(text::rect(x + 1.0, y0 + 1.0, slot - 2.0, slot - 2.0, [0.25, 0.25, 0.28, 0.8]));
            if id != 0 {
                quads.push(text::tile_icon(
                    mcv_core::BLOCKS[id as usize].tiles[2],
                    x + 5.0,
                    y0 + 5.0,
                    slot - 10.0,
                ));
            }
            if i == self.player.sel_slot % 9 {
                quads.push(text::rect(x - 1.0, y0 - 1.0, slot + 2.0, 2.0, [1.0, 1.0, 1.0, 0.9]));
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

fn player_aabb(pos: &Vec3) -> (Vec3, Vec3) {
    let h = Player::HALF;
    (
        Vec3::new(pos.x - h[0], pos.y, pos.z - h[2]),
        Vec3::new(pos.x + h[0], pos.y + h[1] * 2.0, pos.z + h[2]),
    )
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
    let t_max = Vec3::new(
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
    let t_delta = Vec3::new(
        1.0 / dir.x.abs(),
        1.0 / dir.y.abs(),
        1.0 / dir.z.abs(),
    );
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
