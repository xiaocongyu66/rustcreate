//! 粒子引擎（M8a，任务 #57）：26.1 `net/minecraft/client/particle/**` 的
//! 引擎级移植。对照关系逐处标注 `file:line`（26.1 反编译树）。
//!
//! 数据布局：SoA 预分配池 + 空闲链。容量 [`MAX_PARTICLES`] =
//! 16384（`ParticleGroup.java:14`），写入语义对齐 Guava `EvictingQueue`
//! （`ParticleGroup.java:16`）：池满时挤掉**插入序最老**的粒子。
//!
//! 模拟（[`ParticleEngine::tick`]）为 20 Hz 固定步，与原版
//! `ParticleEngine.tick`（`ParticleEngine.java:73-97`）同节奏；渲染提取
//! （[`ParticleEngine::extract_vertices`]）在 draw 帧内做，逐粒子视锥点
//! 剔除（`QuadParticleGroup.java:35` `frustum.pointInFrustum`）+ 相机
//! billboard 展开（`SingleQuadParticle.java:51-96` extract +
//! `QuadParticleRenderState.java:129-137` renderRotatedQuad 顶点序）。
//!
//! 与原版的刻意差异（引擎层无 Level 访问）：
//! - 光照在 tick 时预算（原版 extract 时 `getLightCoords`，
//!   `Particle.java:184-187`）；粒子存活 4~40 tick，提前一 tick 无感。
//!   `spawn_crit` 出生即 `tick()` 一步（`CritParticle.java:44`），该步
//!   光照用全亮近似，下一真实 tick 被世界光照覆盖。
//! - 碰撞回调 [`ParticleWorld`] 由游戏层注入；形状方块（半砖/楼梯等）
//!   按满格碰撞近似（原版 `Entity.collideBoundingBox` 收集
//!   `getBlockCollisions` 任意 VoxelShape，`Entity.java:1138-1143`）。
//! - 随机源为引擎级确定性流（原版每粒子 `RandomSource.create()` 随机
//!   种子，`Particle.java:35`）；固定种子保证测试可复现。

use std::collections::VecDeque;

use glam::Vec3;

use crate::camera::Camera;
use crate::frustum::Frustum;

/// 粒子池上限（`ParticleGroup.java:14` MAX_PARTICLES = 16384）。
pub const MAX_PARTICLES: usize = 16384;
/// 单帧实际送入 GPU 的 quad 上限（可见粒子截断保护；vbuf 预分配 =
/// `MAX_DRAW_QUADS * 4 * 48 B ≈ 786 KiB`）。超出按插入序丢弃。
pub const MAX_DRAW_QUADS: usize = 4096;
/// particles 纹理数组边长：原版 particle PNG 常见 8×8/16×16，统一
/// nearest 放大到 16×16（整数倍 nearest 放大无损细节）。
pub const PARTICLE_PX: usize = 16;

/// particles 纹理数组的帧布局（`assets/minecraft/textures/particle/`
/// 原版 PNG：splash_0..3 / crit_0..11 / bubble）。
pub mod sprites {
    /// SplashParticle 帧（ParticleTypes.SPLASH → `SplashParticle.java`）。
    pub const SPLASH_BASE: u32 = 0;
    pub const SPLASH_COUNT: u32 = 4;
    /// CritParticle 帧（`CritParticle.java:118` sprite.get(random) 随机帧）。
    pub const CRIT_BASE: u32 = 4;
    pub const CRIT_COUNT: u32 = 12;
    /// BubbleParticle（`BubbleParticle.java`，落水第一波）。
    pub const BUBBLE: u32 = 16;
    pub const PARTICLE_LAYERS: u32 = 17;
}

/// splitmix64：单机客户端粒子随机（原版每粒子 `RandomSource.create()`，
/// `Particle.java:35`；此处集中为引擎级确定性流，测试可固定种子复现）。
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// [0, 1)。
    pub fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z >> 40) as u32) as f32 / (1u32 << 24) as f32
    }

    /// [0, 1)。
    pub fn next_f64(&mut self) -> f64 {
        self.next_f32() as f64
    }

    /// `[0, n)` 整数（n >= 1）。
    pub fn next_bounded(&mut self, n: u32) -> u32 {
        (self.next_f32() * n as f32) as u32
    }
}

/// 粒子世界回调：碰撞与光照由游戏层注入（引擎不依赖世界存储）。
/// 默认实现（[`NoWorld`]）= 无碰撞、全亮（原版缺区块时
/// `Particle.java:186` 返回 15728640 = sky 15 + block 15）。
pub trait ParticleWorld {
    /// 方块是否阻挡粒子（原版粒子碰撞收集 `getBlockCollisions`；
    /// 本引擎按"整立方固体"判定，形状方块按满格近似，见模块头）。
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool;
    /// (block_light, sky_light) 0..=15；缺区块回 (15, 15)
    /// （`Particle.java:184-187` hasChunkAt=false → 0xF000F0）。
    fn light_at(&self, _x: i32, _y: i32, _z: i32) -> (u8, u8) {
        (15, 15)
    }
    /// 所在格是否为水（BubbleParticle 离水即灭，`BubbleParticle.java:49-53`）。
    fn is_water(&self, _x: i32, _y: i32, _z: i32) -> bool {
        true
    }
    /// 所在格流体顶面高度（格内相对 0..=1；无水回 0）。WaterDrop 的
    /// 「没入方块/流体顶面即移除」判据（`WaterDropParticle.java:48-55`）
    /// 用；固体形状部分由 [`collide_voxel`] 的满格碰撞近似承担（模块头
    /// 差异项），这里只补流体面。
    fn fluid_top(&self, _x: i32, _y: i32, _z: i32) -> f64 {
        0.0
    }
}

/// 无世界实现（测试 / 无碰撞场景）。
pub struct NoWorld;

impl ParticleWorld for NoWorld {
    fn is_solid(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }
}

/// alpha 淡出曲线（`Particle.java:205-219` LifetimeAlpha 逐字段对应）。
#[derive(Clone, Copy, Debug)]
pub struct LifetimeAlpha {
    pub start_alpha: f32,
    pub end_alpha: f32,
    pub start_at_normalized_age: f32,
    pub end_at_normalized_age: f32,
}

impl LifetimeAlpha {
    /// `Particle.java:206` ALWAYS_OPAQUE。
    pub const ALWAYS_OPAQUE: Self = Self {
        start_alpha: 1.0,
        end_alpha: 1.0,
        start_at_normalized_age: 0.0,
        end_at_normalized_age: 1.0,
    };

    /// `Particle.java:212-219` currentAlphaForAge（inverseLerp + clampedLerp）。
    pub fn current_alpha(&self, age: u32, lifetime: u32, partial_tick: f32) -> f32 {
        if (self.start_alpha - self.end_alpha).abs() < f32::EPSILON {
            return self.start_alpha;
        }
        let t = (age as f32 + partial_tick) / lifetime.max(1) as f32;
        let span = self.end_at_normalized_age - self.start_at_normalized_age;
        let norm = ((t - self.start_at_normalized_age) / span.max(f32::EPSILON)).clamp(0.0, 1.0);
        self.start_alpha + (self.end_alpha - self.start_alpha) * norm
    }
}

/// billboard 朝向模式（`SingleQuadParticle.java:172-174` FacingCameraMode：
/// LOOKAT_XYZ = 整个相机旋转、LOOKAT_Y = 只取 yaw 分量）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Billboard {
    LookAtXyz,
    LookAtY,
}

/// tick 行为族（原版按 `ParticleRenderType` 分组到各 `ParticleGroup`
/// 子类，本引擎收敛为单池 + kind 分派；每族 tick 逐字段对照反编译）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `TerrainParticle`（crack 碎屑）等标准 `Particle.tick`
    /// （`Particle.java:90-112`）。
    Terrain,
    /// `CritParticle.tick`（`CritParticle.java:49-52`）：标准 tick 后
    /// gCol *= 0.96、bCol *= 0.9。
    Crit,
    /// `WaterDropParticle.tick`（`WaterDropParticle.java:41-64`）：
    /// `yd -= gravity`（非 0.04·gravity）、落地 50% 消散。
    WaterDrop,
    /// `BubbleParticle.tick`（`BubbleParticle.java:41-56`）：`yd += 0.002`
    /// 上浮、摩擦 0.85、离水即灭。
    Bubble,
}

/// 渲染静态列（spawn 期写、提取期读；模拟列 tick 不碰这些字段）。
#[derive(Clone, Copy)]
struct QuadData {
    /// quad 半边长基准（`SingleQuadParticle.java:29,44`：0.1·(rand·0.5+0.5)·2）。
    quad_size: f32,
    /// 绕视轴滚动（`SingleQuadParticle.java:54-56` oRoll→roll 插值）。
    roll: f32,
    o_roll: f32,
    /// tint（TerrainParticle 固定 0.6，`TerrainParticle.java:39-41`）。
    r: f32,
    g: f32,
    b: f32,
    /// alpha 淡出（`Particle.java:205-219`）。
    alpha: LifetimeAlpha,
    /// tile 内 UV 矩形（TerrainParticle 为 sprite 的 1/4 随机小矩形，
    /// `TerrainParticle.java:51-52,62-79`）。
    u0: f32,
    u1: f32,
    v0: f32,
    v1: f32,
    /// 图集层 + 纹理组（0=方块图集 terrain 数组、1=particles 数组）。
    layer: u32,
    tex_set: u32,
    billboard: Billboard,
    /// 碰撞盒宽/高（`Particle.java:33-34,44` setSize(0.2,0.2) 初始）。
    bb_width: f32,
    bb_height: f32,
    /// crit 出生放大（`CritParticle.java:44-47`
    /// clamp((age+a)/lifetime·32, 0, 1)；Terrain 恒 1）。
    grow_in: bool,
    /// tick 期预算的光照（差异项见模块头）。
    block_light: u8,
    sky_light: u8,
}

impl QuadData {
    /// 新出生基准（SingleQuadParticle.java:21,26-30,44：r/g/b/alpha=1、
    /// quadSize = 0.1·(rand·0.5+0.5)·2.0、bb 0.2×0.2、LOOKAT_XYZ）。
    fn born(rng: &mut Rng) -> Self {
        Self {
            quad_size: 0.1 * (rng.next_f32() * 0.5 + 0.5) * 2.0,
            roll: 0.0,
            o_roll: 0.0,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            alpha: LifetimeAlpha::ALWAYS_OPAQUE,
            u0: 0.0,
            u1: 1.0,
            v0: 0.0,
            v1: 1.0,
            layer: 0,
            tex_set: 0,
            billboard: Billboard::LookAtXyz,
            bb_width: 0.2,
            bb_height: 0.2,
            grow_in: false,
            block_light: 15,
            sky_light: 15,
        }
    }
}

/// 渲染顶点（每 quad 4 顶点；stride 48 B，布局见 particle.wgsl PVertIn）。
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
    pub tex_set: u32,
    pub color: [f32; 4],
    pub light: f32,
}

const _: () = assert!(size_of::<ParticleVertex>() == 48);

/// terrain 光照曲线 CPU 侧（与 terrain.wgsl `light_curve` 同公式；
/// 26.1 lightmap.fsh:21-42 乘序：先 get_brightness v/(4-3v) 再乘 day）。
fn light_curve(sky: u8, block: u8, day: f32) -> f32 {
    let s = sky as f32 / 15.0;
    let b = block as f32 / 15.0;
    let sb = s / (4.0 - 3.0 * s) * day;
    let bb = b / (4.0 - 3.0 * b);
    0.08 + 0.92 * sb.max(bb)
}

/// SoA 粒子池 + 引擎。
pub struct ParticleEngine {
    // ---- SoA 模拟列（每 tick 扫描；位置/速度 f64 对齐原版 double）----
    x: Vec<f64>,
    y: Vec<f64>,
    z: Vec<f64>,
    xo: Vec<f64>,
    yo: Vec<f64>,
    zo: Vec<f64>,
    xd: Vec<f64>,
    yd: Vec<f64>,
    zd: Vec<f64>,
    age: Vec<u32>,
    lifetime: Vec<u32>,
    gravity: Vec<f32>,
    friction: Vec<f32>,
    /// bit0 on_ground、bit1 has_physics、bit2 stopped_by_collision、
    /// bit3 speed_up_when_y_motion_is_blocked（`Particle.java:29-40`）。
    flags: Vec<u8>,
    kind: Vec<Kind>,
    rend: Vec<QuadData>,
    // ---- 池结构：空闲链 + 活跃插入序（EvictingQueue 语义）----
    free: Vec<u16>,
    live: VecDeque<u16>,
    rng: Rng,
}

const F_ON_GROUND: u8 = 1;
const F_HAS_PHYSICS: u8 = 2;
const F_STOPPED: u8 = 4;
const F_SPEED_UP: u8 = 8;

impl ParticleEngine {
    /// 预分配 [`MAX_PARTICLES`] 槽（固定种子；原版随机种子，见模块头）。
    pub fn new() -> Self {
        Self::with_seed(0x5EED_BA5E)
    }

    pub fn with_seed(seed: u64) -> Self {
        let n = MAX_PARTICLES as u16;
        let zero = QuadData {
            quad_size: 0.1,
            roll: 0.0,
            o_roll: 0.0,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            alpha: LifetimeAlpha::ALWAYS_OPAQUE,
            u0: 0.0,
            u1: 1.0,
            v0: 0.0,
            v1: 1.0,
            layer: 0,
            tex_set: 0,
            billboard: Billboard::LookAtXyz,
            bb_width: 0.2,
            bb_height: 0.2,
            grow_in: false,
            block_light: 15,
            sky_light: 15,
        };
        Self {
            x: vec![0.0; n as usize],
            y: vec![0.0; n as usize],
            z: vec![0.0; n as usize],
            xo: vec![0.0; n as usize],
            yo: vec![0.0; n as usize],
            zo: vec![0.0; n as usize],
            xd: vec![0.0; n as usize],
            yd: vec![0.0; n as usize],
            zd: vec![0.0; n as usize],
            age: vec![0; n as usize],
            lifetime: vec![1; n as usize],
            gravity: vec![0.0; n as usize],
            friction: vec![0.98; n as usize],
            flags: vec![0; n as usize],
            kind: vec![Kind::Terrain; n as usize],
            rend: vec![zero; n as usize],
            free: (0..n).rev().collect(),
            live: VecDeque::with_capacity(n as usize),
            rng: Rng::new(seed),
        }
    }

    /// 活跃粒子数（`ParticleGroup.size`，ParticleGroup.java:57-59）。
    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// 分配槽：先空闲链，池满挤掉插入序最老（Guava `EvictingQueue.add`）。
    fn alloc(&mut self) -> usize {
        if let Some(i) = self.free.pop() {
            return i as usize;
        }
        // free 与 live 互补，live 非空时必可挤头。
        self.live.pop_front().map_or(0, |i| i as usize)
    }

    /// 单粒子出生（`Particle.java:42-62` 双构造并集）：默认 lifetime =
    /// `4/(rand·0.9+0.1)`；`with_velocity` 时按速度构造公式
    /// （`Particle.java:52-62`）把 aux 方向抖动 ±0.4 后归一化重定向。
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &mut self,
        kind: Kind,
        pos: [f64; 3],
        aux: [f64; 3],
        with_velocity: bool,
        rend: QuadData,
    ) {
        let i = self.alloc();
        self.x[i] = pos[0];
        self.y[i] = pos[1];
        self.z[i] = pos[2];
        self.xo[i] = pos[0];
        self.yo[i] = pos[1];
        self.zo[i] = pos[2];
        self.age[i] = 0;
        self.kind[i] = kind;
        self.flags[i] = F_HAS_PHYSICS;
        self.gravity[i] = 0.0;
        self.friction[i] = 0.98;
        let mut lifetime = (4.0 / (self.rng.next_f32() * 0.9 + 0.1)) as u32;
        if with_velocity {
            // Particle.java:52-62（rand 均为 f32 → cast f64 参与 double 运算）。
            let dx = aux[0] + (self.rng.next_f32() as f64 * 2.0 - 1.0) * 0.4;
            let dy = aux[1] + (self.rng.next_f32() as f64 * 2.0 - 1.0) * 0.4;
            let mut dz = aux[2] + (self.rng.next_f32() as f64 * 2.0 - 1.0) * 0.4;
            let speed = (self.rng.next_f32() as f64 + self.rng.next_f32() as f64 + 1.0) * 0.15;
            let dd = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-9);
            self.xd[i] = dx / dd * speed * 0.4;
            self.yd[i] = dy / dd * speed * 0.4 + 0.1;
            dz /= dd;
            self.zd[i] = dz * speed * 0.4;
        } else {
            self.xd[i] = aux[0];
            self.yd[i] = aux[1];
            self.zd[i] = aux[2];
        }
        if matches!(kind, Kind::WaterDrop | Kind::Bubble) {
            // WaterDropParticle.java:38 / BubbleParticle.java:37：
            // lifetime = 8/(rand·0.8+0.2)。
            lifetime = (8.0 / (self.rng.next_f32() * 0.8 + 0.2)) as u32;
        }
        self.lifetime[i] = lifetime.max(1);
        self.rend[i] = rend;
        self.live.push_back(i as u16);
    }

    /// TerrainParticle 的渲染参数（`TerrainParticle.java:25-54`）：
    /// gravity=1.0、r/g/b=0.6、quadSize 减半、UV = sprite 内 1/4 随机
    /// 小矩形（uo,vo ∈ [0,3]，矩形 [uo, uo+1]/4）。
    fn terrain_rend(&mut self, block_id: u16) -> QuadData {
        let mut q = QuadData::born(&mut self.rng);
        q.quad_size *= 0.5; // TerrainParticle.java:50
        q.r = 0.6;
        q.g = 0.6;
        q.b = 0.6;
        let uo = self.rng.next_f32() * 3.0;
        let vo = self.rng.next_f32() * 3.0;
        q.u0 = uo / 4.0;
        q.u1 = (uo + 1.0) / 4.0;
        q.v0 = vo / 4.0;
        q.v1 = (vo + 1.0) / 4.0;
        q.layer = terrain_layer(block_id);
        q.tex_set = 0;
        q
    }

    /// 方块破坏爆裂（26.1 `ClientLevel.addDestroyBlockEffect`，
    /// ClientLevel.java:942-973；服务端 `Level.destroyBlock` 的
    /// levelEvent 2001（Level.java:283）→ `LevelEventHandler.java:280-289`
    /// 驱动到这）。按 0.25 密度切网格（ClientLevel.java:951-953
    /// count = max(2, ceil(w/0.25))，满块即 4×4×4 = 64 粒），格心出生、
    /// 速度 = 格内相对偏移（ClientLevel.java:958-966）。
    ///
    /// `progress_stage`（0..=9）：原版碎屑粒子**不使用**挖掘进度
    /// （stage 只驱动 destroy_stage 裂纹 overlay 与独立 `block_crumb`
    /// 变体）；保留入参贴合派单签名，注释见
    /// [`Self::spawn_crumb`]。
    pub fn spawn_block_crack(&mut self, pos: [f64; 3], block_id: u16, progress_stage: u8) {
        self.spawn_block_crack_box(
            pos,
            block_id,
            progress_stage,
            [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        );
    }

    /// [`Self::spawn_block_crack`] 的形状盒变体：`box6` = 方块形状
    /// AABB（满块 `[0,0,0,1,1,1]`；半砖/楼梯由游戏层用
    /// `mcv_game::blockshapes::pick_boxes` 收窄，对齐原版
    /// `shape.forAllBoxes`，ClientLevel.java:946-971）。
    pub fn spawn_block_crack_box(
        &mut self,
        pos: [f64; 3],
        block_id: u16,
        progress_stage: u8,
        b: [f64; 6],
    ) {
        if block_id == 0 {
            return;
        }
        let w = (b[3] - b[0]).min(1.0);
        let h = (b[4] - b[1]).min(1.0);
        let d = (b[5] - b[2]).min(1.0);
        let cx = ((w / 0.25).ceil() as u32).max(2);
        let cy = ((h / 0.25).ceil() as u32).max(2);
        let cz = ((d / 0.25).ceil() as u32).max(2);
        for xx in 0..cx {
            for yy in 0..cy {
                for zz in 0..cz {
                    let rx = (xx as f64 + 0.5) / cx as f64;
                    let ry = (yy as f64 + 0.5) / cy as f64;
                    let rz = (zz as f64 + 0.5) / cz as f64;
                    self.spawn_terrain(
                        [
                            pos[0] + rx * w + b[0],
                            pos[1] + ry * h + b[1],
                            pos[2] + rz * d + b[2],
                        ],
                        [rx - 0.5, ry - 0.5, rz - 0.5],
                        block_id,
                    );
                }
            }
        }
        debug_assert!(progress_stage <= 9, "crack stage 0..=9");
    }

    /// TerrainParticle（Provider 路径，`TerrainParticle.java:141-155`）：
    /// 速度走 `Particle.java:52-62` 构造公式、gravity = 1.0
    /// （TerrainParticle.java:38）。
    pub fn spawn_terrain(&mut self, pos: [f64; 3], vel: [f64; 3], block_id: u16) {
        let q = self.terrain_rend(block_id);
        self.spawn(Kind::Terrain, pos, vel, true, q);
        self.set_last_gravity(1.0);
    }

    /// 最新出生粒子的重力（Java 构造器字段赋值的池化等价）。
    fn set_last_gravity(&mut self, g: f32) {
        if let Some(&i) = self.live.back() {
            self.gravity[i as usize] = g;
        }
    }

    /// 挖掘面碎屑（26.1 `ClientLevel.addBreakingBlockEffect`，
    /// ClientLevel.java:975-1013；`Minecraft.java:1619` 每挖 tick 调）：
    /// 形状盒内随机点（内缩 0.1）+ 沿命中面外偏 0.1、出生后
    /// `setPower(0.2).scale(0.6)`。
    ///
    /// `face` = 命中面法线序号：0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z
    /// （`mcv_logic::dda_hit` 返回的法线约定）。
    pub fn spawn_hit(&mut self, pos: [f64; 3], face: u8, block_id: u16, box6: [f64; 6]) {
        if block_id == 0 {
            return;
        }
        // ClientLevel.java:984-987：盒内随机点（内缩 0.2 + 0.1 起步）。
        let mut p = [
            pos[0] + self.rng.next_f64() * (box6[3] - box6[0] - 0.2) + 0.1 + box6[0],
            pos[1] + self.rng.next_f64() * (box6[4] - box6[1] - 0.2) + 0.1 + box6[1],
            pos[2] + self.rng.next_f64() * (box6[5] - box6[2] - 0.2) + 0.1 + box6[2],
        ];
        // ClientLevel.java:988-1007：命中面坐标替换为面外 0.1。
        match face {
            0 => p[0] = pos[0] + box6[3] + 0.1,
            1 => p[0] = pos[0] + box6[0] - 0.1,
            2 => p[1] = pos[1] + box6[4] + 0.1,
            3 => p[1] = pos[1] + box6[1] - 0.1,
            4 => p[2] = pos[2] + box6[5] + 0.1,
            _ => p[2] = pos[2] + box6[2] - 0.1,
        }
        let mut q = self.terrain_rend(block_id);
        // scale(0.6)（SingleQuadParticle.java:102-106 + Particle.java:77-80）。
        q.quad_size *= 0.6;
        q.bb_width = 0.2 * 0.6;
        q.bb_height = 0.2 * 0.6;
        // 原版（ClientLevel.java:1012）= TerrainParticle 速度构造（散布
        // 初速）+ setPower(0.2)。spawn 的 with_velocity 路径产出散布初速，
        // 出生后 apply_power 补 setPower。
        self.spawn(Kind::Terrain, p, [0.0, 0.0, 0.0], true, q);
        self.set_last_gravity(1.0);
        self.apply_power_last(0.2);
    }

    /// `setPower`（`Particle.java:64-69`）：xd·p、(yd−0.1)·p+0.1、zd·p；
    /// 作用于最新出生粒子（Java 为构造链调用，池化后落到 spawn 后补丁）。
    fn apply_power_last(&mut self, power: f64) {
        let Some(&i) = self.live.back() else { return };
        let i = i as usize;
        self.xd[i] *= power;
        self.yd[i] = (self.yd[i] - 0.1) * power + 0.1;
        self.zd[i] *= power;
    }

    /// 落水效果（26.1 `Entity.doWaterSplashEffect`，Entity.java:1594-1622）：
    /// 两波各 `1 + width·20` 个（玩家 width=0.6 → 13），x/z = ±width 随机、
    /// y = floor(y)+1；第一波 BUBBLE（vel = movement·0.2 ± rand·0.02）、
    /// 第二波 SPLASH/WaterDropParticle（初速 0、yd = rand·0.2+0.1、
    /// gravity 0.06）。
    pub fn spawn_water_splash(&mut self, x: f64, y: f64, z: f64, mv: [f64; 3], width: f64) {
        let count = (1.0 + width * 20.0) as u32;
        let yt = y.floor() + 1.0;
        for _ in 0..count {
            let xo = (self.rng.next_f64() * 2.0 - 1.0) * width;
            let zo = (self.rng.next_f64() * 2.0 - 1.0) * width;
            // BubbleParticle.java:26-38：size 0.02、quadSize·(rand·0.6+0.2)、
            // vel = aux·0.2 ± rand·0.02。
            let mut q = QuadData::born(&mut self.rng);
            q.bb_width = 0.02;
            q.bb_height = 0.02;
            q.quad_size *= self.rng.next_f32() * 0.6 + 0.2;
            q.layer = sprites::BUBBLE;
            q.tex_set = 1;
            let dx = mv[0] * 0.2 + (self.rng.next_f64() * 2.0 - 1.0) * 0.02;
            let dy = mv[1] * 0.2 + (self.rng.next_f64() * 2.0 - 1.0) * 0.02;
            let dz = mv[2] * 0.2 + (self.rng.next_f64() * 2.0 - 1.0) * 0.02;
            self.spawn(Kind::Bubble, [x + xo, yt, z + zo], [dx, dy, dz], false, q);
        }
        for _ in 0..count {
            let xo = (self.rng.next_f64() * 2.0 - 1.0) * width;
            let zo = (self.rng.next_f64() * 2.0 - 1.0) * width;
            // WaterDropParticle.java:29-39：构造速度 0（doWaterSplashEffect
            // 不把 movement 传给 SPLASH 粒子——Entity.java:1616-1620 的
            // xAux/…仅被 SplashParticle 的 ya==0 分支使用且 movement.y
            // 非 0 时跳过）、yd = rand·0.2+0.1、size 0.01、gravity 0.06。
            let mut q = QuadData::born(&mut self.rng);
            q.bb_width = 0.01;
            q.bb_height = 0.01;
            q.layer = self.rng.next_bounded(sprites::SPLASH_COUNT) + sprites::SPLASH_BASE;
            q.tex_set = 1;
            let yd = self.rng.next_f32() as f64 * 0.2 + 0.1;
            self.spawn(
                Kind::WaterDrop,
                [x + xo, yt, z + zo],
                [0.0, yd, 0.0],
                false,
                q,
            );
            let Some(&i) = self.live.back() else { continue };
            self.gravity[i as usize] = 0.06;
        }
    }

    /// 雨粒子（26.1 `ParticleTypes.RAIN` = **WaterDropParticle**，
    /// `ParticleResources.java:117`；纹理 rain.json → splash_0..3，与
    /// SPLASH 同图集；生成侧由天气层按 `WeatherEffectRenderer.
    /// tickRainParticles`（WeatherEffectRenderer.java:224-268）驱动）。
    /// 构造 = `WaterDropParticle.java:11-19`：aux 速度全 0（:258-266 传
    /// 0,0,0，`xd *= 0.3` 后仍 0）、`yd = rand·0.2+0.1`（出生先上跳）、
    /// size 0.01、gravity 0.06、lifetime = 8/(rand·0.8+0.2)。
    pub fn spawn_rain_drop(&mut self, x: f64, y: f64, z: f64) {
        let mut q = QuadData::born(&mut self.rng);
        q.bb_width = 0.01;
        q.bb_height = 0.01;
        q.layer = self.rng.next_bounded(sprites::SPLASH_COUNT) + sprites::SPLASH_BASE;
        q.tex_set = 1;
        let yd = self.rng.next_f32() as f64 * 0.2 + 0.1;
        self.spawn(Kind::WaterDrop, [x, y, z], [0.0, yd, 0.0], false, q);
        let Some(&i) = self.live.back() else { return };
        self.gravity[i as usize] = 0.06;
    }

    /// 溺水气泡（26.1 `broadcastEntityEvent(67)` → `makeDrownParticles`，
    /// LivingEntity.java:2087-2088/:2113-2123）：8 个 BUBBLE，出生偏移 =
    /// `random.triangle(0,1)` = nextDouble−nextDouble ∈ [-1,1]（
    /// RandomSource.java:59-61），速度 = 实体 movement（BubbleParticle
    /// 构造 ×0.2±rand·0.02，BubbleParticle.java:21-28，与 splash 波 1
    /// 同款构造）。
    pub fn spawn_drown_bubbles(&mut self, x: f64, y: f64, z: f64, mv: [f64; 3]) {
        for _ in 0..8 {
            let ox = self.rng.next_f64() - self.rng.next_f64();
            let oy = self.rng.next_f64() - self.rng.next_f64();
            let oz = self.rng.next_f64() - self.rng.next_f64();
            let mut q = QuadData::born(&mut self.rng);
            q.bb_width = 0.02;
            q.bb_height = 0.02;
            q.quad_size *= self.rng.next_f32() * 0.6 + 0.2;
            q.layer = sprites::BUBBLE;
            q.tex_set = 1;
            let dx = mv[0] * 0.2 + (self.rng.next_f64() * 2.0 - 1.0) * 0.02;
            let dy = mv[1] * 0.2 + (self.rng.next_f64() * 2.0 - 1.0) * 0.02;
            let dz = mv[2] * 0.2 + (self.rng.next_f64() * 2.0 - 1.0) * 0.02;
            self.spawn(
                Kind::Bubble,
                [x + ox, y + oy, z + oz],
                [dx, dy, dz],
                false,
                q,
            );
        }
    }

    /// 受击 crit（26.1 `CritParticle.Provider`，CritParticle.java:117-126；
    /// `Player.attack` 命中驱动）。`indicator=true` 走 DamageIndicator 变体
    /// （CritParticle.java:88-104：lifetime=20、ya+1 上飘）。
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_crit(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        xa: f64,
        ya: f64,
        za: f64,
        indicator: bool,
    ) {
        // CritParticle.java:26-47：friction 0.7、gravity 0.5、
        // vel·0.1 + aux·0.4、col 0.6..0.9、quadSize·0.75、hasPhysics=false。
        let mut q = QuadData::born(&mut self.rng);
        let col = self.rng.next_f32() * 0.3 + 0.6;
        q.r = col;
        q.g = col;
        q.b = col;
        q.quad_size *= 0.75;
        q.grow_in = true;
        q.tex_set = 1;
        q.layer = self.rng.next_bounded(sprites::CRIT_COUNT) + sprites::CRIT_BASE;
        let ay = if indicator { ya + 1.0 } else { ya };
        self.spawn(Kind::Crit, [x, y, z], [0.0, 0.0, 0.0], false, q);
        let Some(&i) = self.live.back() else { return };
        let i = i as usize;
        self.friction[i] = 0.7;
        self.gravity[i] = 0.5;
        self.flags[i] &= !F_HAS_PHYSICS;
        self.xd[i] = self.xd[i] * 0.1 + xa * 0.4;
        self.yd[i] = self.yd[i] * 0.1 + ay * 0.4;
        self.zd[i] = self.zd[i] * 0.1 + za * 0.4;
        if indicator {
            self.lifetime[i] = 20;
        } else {
            // CritParticle.java:42：max(6/(rand·0.8+0.6), 1)。
            self.lifetime[i] = ((6.0 / (self.rng.next_f32() * 0.8 + 0.6)) as u32).max(1);
        }
        // CritParticle.java:44：构造尾 tick() 一次（出生即走一步）。
        self.tick_one(i, &NoWorld);
    }

    /// 每 game tick 步进全部活跃粒子（`ParticleEngine.tick` →
    /// `ParticleGroup.tickParticles`，ParticleEngine.java:73-80 /
    /// ParticleGroup.java:26-39），到期回收回空闲链。
    pub fn tick(&mut self, world: &dyn ParticleWorld) {
        // live 顺序先取快照（tick_one 会借用 &mut self；VecDeque 元素是
        // 槽号，池满挤最老只发生在 spawn 路径，tick 期间顺序稳定）。
        let live: Vec<u16> = self.live.iter().copied().collect();
        let mut dead: Vec<u16> = Vec::new();
        for i in live {
            let i = i as usize;
            self.tick_one(i, world);
            if self.age[i] > self.lifetime[i] {
                dead.push(i as u16);
            }
        }
        if !dead.is_empty() {
            let dead: std::collections::HashSet<u16> = dead.into_iter().collect();
            self.live.retain(|i| !dead.contains(i));
            for i in dead {
                self.free.push(i);
            }
        }
    }

    /// 单粒子 tick（分派 kind；对照 Particle.java:90-112 及各族覆写）。
    fn tick_one(&mut self, i: usize, world: &dyn ParticleWorld) {
        // 旧位置（Particle.java:91-93）。
        self.xo[i] = self.x[i];
        self.yo[i] = self.y[i];
        self.zo[i] = self.z[i];

        match self.kind[i] {
            Kind::Terrain => self.tick_standard(i, world),
            Kind::Crit => {
                self.tick_standard(i, world);
                // CritParticle.java:49-52：gCol·0.96、bCol·0.9。
                let g = &mut self.rend[i];
                g.g *= 0.96;
                g.b *= 0.9;
            }
            Kind::WaterDrop => {
                // WaterDropParticle.java:41-64（lifetime-- <= 0 → remove；
                // 等价 age 递增越限，池回收判据统一 age > lifetime）。
                self.age[i] += 1;
                if self.age[i] > self.lifetime[i] {
                    return;
                }
                self.yd[i] -= self.gravity[i] as f64;
                self.move_particle(i, world);
                self.xd[i] *= 0.98;
                self.yd[i] *= 0.98;
                self.zd[i] *= 0.98;
                if self.flags_of(i, F_ON_GROUND) {
                    // WaterDropParticle.java:55-58：落地 50% 消散。
                    if self.rng.next_f32() < 0.5 {
                        self.lifetime[i] = 0; // 下一 tick 回收
                    }
                    self.xd[i] *= 0.7;
                    self.zd[i] *= 0.7;
                }
                // WaterDropParticle.java:48-55：本格形状顶面/流体顶面高于
                // 粒子 y → 移除（雨滴落到水面/方块面即灭，雨族的
                // 「落地判据」；固体面由 collide_voxel 满格近似提前截停）。
                let (cx, cy, cz) = (
                    self.x[i].floor() as i32,
                    self.y[i].floor() as i32,
                    self.z[i].floor() as i32,
                );
                let off = world.fluid_top(cx, cy, cz);
                if off > 0.0 && self.y[i] < cy as f64 + off {
                    self.lifetime[i] = 0;
                }
            }
            Kind::Bubble => {
                // BubbleParticle.java:41-56：yd += 0.002 上浮、摩擦 0.85。
                self.age[i] += 1;
                if self.age[i] > self.lifetime[i] {
                    return;
                }
                self.yd[i] += 0.002;
                self.move_particle(i, world);
                self.xd[i] *= 0.85;
                self.yd[i] *= 0.85;
                self.zd[i] *= 0.85;
                if !world.is_water(
                    self.x[i].floor() as i32,
                    self.y[i].floor() as i32,
                    self.z[i].floor() as i32,
                ) {
                    self.lifetime[i] = 0; // BubbleParticle.java:49-53 离水即灭
                }
            }
        }
        // tick 期预算光照（差异项见模块头）。
        let (bl, sl) = world.light_at(
            self.x[i].floor() as i32,
            self.y[i].floor() as i32,
            self.z[i].floor() as i32,
        );
        let r = &mut self.rend[i];
        r.block_light = bl;
        r.sky_light = sl;
    }

    /// `Particle.tick` 主体（Particle.java:94-111）：到期移除、
    /// 重力 `yd -= 0.04·gravity`、move 碰撞、摩擦 0.98、落地额外 0.7。
    fn tick_standard(&mut self, i: usize, world: &dyn ParticleWorld) {
        self.age[i] += 1;
        if self.age[i] > self.lifetime[i] {
            return; // age++ >= lifetime → remove（Particle.java:94）
        }
        self.yd[i] -= 0.04 * self.gravity[i] as f64;
        self.move_particle(i, world);
        if self.flags_of(i, F_SPEED_UP) && self.y[i] == self.yo[i] {
            self.xd[i] *= 1.1;
            self.zd[i] *= 1.1;
        }
        let f = self.friction[i] as f64;
        self.xd[i] *= f;
        self.yd[i] *= f;
        self.zd[i] *= f;
        if self.flags_of(i, F_ON_GROUND) {
            self.xd[i] *= 0.7;
            self.zd[i] *= 0.7;
        }
    }

    fn flags_of(&self, i: usize, bit: u8) -> bool {
        self.flags[i] & bit != 0
    }

    /// `Particle.move`（Particle.java:145-175）：体素 AABB 逐轴 clip。
    fn move_particle(&mut self, i: usize, world: &dyn ParticleWorld) {
        if self.flags_of(i, F_STOPPED) {
            return;
        }
        let (oxa, oya, oza) = (self.xd[i], self.yd[i], self.zd[i]);
        let (mut xa, mut ya, mut za) = (oxa, oya, oza);
        let w = self.rend[i].bb_width as f64;
        let h = self.rend[i].bb_height as f64;
        let speed2 = xa * xa + ya * ya + za * za;
        // Particle.java:150-155：hasPhysics 且速度 < 100² 才做碰撞。
        if self.flags_of(i, F_HAS_PHYSICS) && speed2 > 0.0 && speed2 < 100.0 * 100.0 {
            let bb = (
                self.x[i] - w * 0.5,
                self.y[i],
                self.z[i] - w * 0.5,
                self.x[i] + w * 0.5,
                self.y[i] + h,
                self.z[i] + w * 0.5,
            );
            let (cx, cy, cz) = collide_voxel(world, bb, xa, ya, za);
            xa = cx;
            ya = cy;
            za = cz;
        }
        if xa != 0.0 || ya != 0.0 || za != 0.0 {
            // 位置按移动量平移（盒心 x/z、底 y 语义随对称平移保持，
            // 无需重跑 setLocationFromBoundingbox）。
            self.x[i] += xa;
            self.y[i] += ya;
            self.z[i] += za;
        }
        // Particle.java:162-173：碰撞截停标记、落地判定、轴向速度清零。
        if oya.abs() >= 1.0e-5 && ya.abs() < 1.0e-5 {
            self.flags[i] |= F_STOPPED;
        }
        if oya != ya && oya < 0.0 {
            self.flags[i] |= F_ON_GROUND;
        } else {
            self.flags[i] &= !F_ON_GROUND;
        }
        if oxa != xa {
            self.xd[i] = 0.0;
        }
        if oza != za {
            self.zd[i] = 0.0;
        }
    }

    /// 渲染提取：视锥点剔除 + billboard 展开（追加到 `out`）。
    ///
    /// `partial_tick` ∈ [0,1)（帧内 tick 进度，原版 partialTickTime）。
    /// 上限 [`MAX_DRAW_QUADS`] 由调用方（gpu 侧）截断。
    pub fn extract_vertices(
        &self,
        cam: &Camera,
        day: f32,
        partial_tick: f32,
        out: &mut Vec<ParticleVertex>,
    ) {
        let frustum = Frustum::from_view_proj(&cam.view_proj());
        let view = cam.view();
        // LOOKAT_XYZ 基向量（SingleQuadParticle.java:173 rotation = 相机
        // 旋转）：view 3x3 行 = [xaxis; yaxis; zaxis]，glam 列主序取列转置。
        let right_v = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up_v = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        // LOOKAT_Y（SingleQuadParticle.java:174 只绕 Y）：
        // right = R_y(−yaw)·X = (cos yaw, 0, sin yaw)、up = Y。
        let (sy, cy) = cam.yaw.sin_cos();
        let right_y = Vec3::new(cy, 0.0, sy);
        let up_y = Vec3::Y;

        for &i in &self.live {
            let i = i as usize;
            // 插值位置（SingleQuadParticle.java:64-66）。
            let t = partial_tick as f64;
            let p = Vec3::new(
                lerp(t, self.xo[i], self.x[i]) as f32,
                lerp(t, self.yo[i], self.y[i]) as f32,
                lerp(t, self.zo[i], self.z[i]) as f32,
            );
            // QuadParticleGroup.java:35 frustum.pointInFrustum（点测试）。
            if !frustum.intersects_aabb(p, p) {
                continue;
            }
            let r = &self.rend[i];
            // quad 尺寸：Crit 出生放大（CritParticle.java:44-47）。
            let size = if r.grow_in {
                r.quad_size
                    * (((self.age[i] as f32 + partial_tick) / self.lifetime[i].max(1) as f32)
                        * 32.0)
                        .clamp(0.0, 1.0)
            } else {
                r.quad_size
            };
            // roll 插值（SingleQuadParticle.java:54-56）。
            let roll = lerp(t, r.o_roll as f64, r.roll as f64) as f32;
            let (mut rr, mut uu) = if r.billboard == Billboard::LookAtY {
                (right_y, up_y)
            } else {
                (right_v, up_v)
            };
            if roll != 0.0 {
                let (s, c) = roll.sin_cos();
                rr = rr * c + uu * s;
                uu = rr * -s + uu * c;
            }
            let alpha = r
                .alpha
                .current_alpha(self.age[i], self.lifetime[i], partial_tick);
            let light = light_curve(r.sky_light, r.block_light, day);
            let (u0, u1, v0, v1) = (r.u0, r.u1, r.v0, r.v1);
            // 顶点序与 UV 配对照抄 QuadParticleRenderState.java:130-137。
            let corners = [
                (p + (rr - uu) * size, [u1, v1]),
                (p + (rr + uu) * size, [u1, v0]),
                (p + (-rr + uu) * size, [u0, v0]),
                (p + (-rr - uu) * size, [u0, v1]),
            ];
            let color = [r.r, r.g, r.b, alpha];
            for (pos, uv) in corners {
                out.push(ParticleVertex {
                    pos: [pos.x, pos.y, pos.z],
                    uv,
                    layer: r.layer,
                    tex_set: r.tex_set,
                    color,
                    light,
                });
            }
        }
    }

    /// 活跃粒子数（HUD F3 调试用）。
    pub fn count(&self) -> usize {
        self.live.len()
    }

    /// 清空（`ParticleEngine.clearParticles`，ParticleEngine.java:145-150）。
    pub fn clear(&mut self) {
        self.live.clear();
        self.free = (0..MAX_PARTICLES as u16).rev().collect();
    }
}

impl Default for ParticleEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// TerrainParticle 贴图层：方块 def 的 +X tile（side 语义；原版
/// particle material ≈ 模型 particle 贴图，多数方块 = 侧面）。
fn terrain_layer(block_id: u16) -> u32 {
    mcv_core::BlockId(block_id).def().tiles[0] as u32
}

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + (b - a) * t
}

/// 体素 AABB 逐轴 clip（`Entity.collideWithShapes` Entity.java:1168-1183；
/// 轴步序 `Direction.axisStepOrder` Direction.java:379-381：
/// `|x| < |z| ? YZX : YXZ`；截断阈值 `Shapes.collide` Shapes.java:233-235）。
fn collide_voxel(
    world: &dyn ParticleWorld,
    bb: (f64, f64, f64, f64, f64, f64),
    dx: f64,
    dy: f64,
    dz: f64,
) -> (f64, f64, f64) {
    let mut out = [dx, dy, dz];
    let order: [usize; 3] = if dx.abs() < dz.abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    };
    let mut moved = bb;
    for &axis in &order {
        let d = out[axis];
        if d == 0.0 {
            continue;
        }
        let clipped = clip_axis(world, moved, axis, d);
        out[axis] = clipped;
        match axis {
            0 => {
                moved.0 += clipped;
                moved.3 += clipped;
            }
            1 => {
                moved.1 += clipped;
                moved.4 += clipped;
            }
            _ => {
                moved.2 += clipped;
                moved.5 += clipped;
            }
        }
    }
    (out[0], out[1], out[2])
}

/// 沿轴扫掠盒覆盖格逐块 clip（collectColliders 的体素版）。
fn clip_axis(
    world: &dyn ParticleWorld,
    bb: (f64, f64, f64, f64, f64, f64),
    axis: usize,
    d: f64,
) -> f64 {
    let (minx, miny, minz, maxx, maxy, maxz) = bb;
    // 扫掠盒 = 当前盒沿本轴朝 d 方向扩展（其余轴不扩）。
    let (mut lo, mut hi) = match axis {
        0 => (minx, maxx),
        1 => (miny, maxy),
        _ => (minz, maxz),
    };
    if d < 0.0 {
        lo += d;
    } else {
        hi += d;
    }
    let gx0 = (if axis == 0 { lo } else { minx }).floor() as i32;
    let gx1 = (if axis == 0 { hi } else { maxx }).ceil() as i32;
    let gy0 = (if axis == 1 { lo } else { miny }).floor() as i32;
    let gy1 = (if axis == 1 { hi } else { maxy }).ceil() as i32;
    let gz0 = (if axis == 2 { lo } else { minz }).floor() as i32;
    let gz1 = (if axis == 2 { hi } else { maxz }).ceil() as i32;
    let mut dist = d;
    for bx in gx0..gx1 {
        for by in gy0..gy1 {
            for bz in gz0..gz1 {
                if !world.is_solid(bx, by, bz) {
                    continue;
                }
                dist = clip_cube(bb, (bx as f64, by as f64, bz as f64), axis, dist);
                if dist.abs() < 1.0e-7 {
                    return 0.0;
                }
            }
        }
    }
    dist
}

/// 满格块的 `AABB.clip{X,Y,Z}Collide` 语义（轴 0=x 1=y 2=z；另两轴
/// 区间严格重叠才裁剪）。
fn clip_cube(
    bb: (f64, f64, f64, f64, f64, f64),
    cube: (f64, f64, f64),
    axis: usize,
    d: f64,
) -> f64 {
    let (minx, miny, minz, maxx, maxy, maxz) = bb;
    let (bx, by, bz) = cube;
    match axis {
        0 => {
            if maxy > by && miny < by + 1.0 && maxz > bz && minz < bz + 1.0 {
                if d > 0.0 && maxx <= bx {
                    (bx - maxx).min(d)
                } else if d < 0.0 && minx >= bx + 1.0 {
                    (bx + 1.0 - minx).max(d)
                } else {
                    d
                }
            } else {
                d
            }
        }
        1 => {
            if maxx > bx && minx < bx + 1.0 && maxz > bz && minz < bz + 1.0 {
                if d > 0.0 && maxy <= by {
                    (by - maxy).min(d)
                } else if d < 0.0 && miny >= by + 1.0 {
                    (by + 1.0 - miny).max(d)
                } else {
                    d
                }
            } else {
                d
            }
        }
        _ => {
            if maxx > bx && minx < bx + 1.0 && maxy > by && miny < by + 1.0 {
                if d > 0.0 && maxz <= bz {
                    (bz - maxz).min(d)
                } else if d < 0.0 && minz >= bz + 1.0 {
                    (bz + 1.0 - minz).max(d)
                } else {
                    d
                }
            } else {
                d
            }
        }
    }
}

// ---- 单测 ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 简单地板世界：y < 8 全 solid。
    struct Floor;
    impl ParticleWorld for Floor {
        fn is_solid(&self, _x: i32, y: i32, _z: i32) -> bool {
            y < 8
        }
        fn light_at(&self, _x: i32, _y: i32, _z: i32) -> (u8, u8) {
            (0, 15)
        }
        fn is_water(&self, _x: i32, _y: i32, _z: i32) -> bool {
            false
        }
    }

    fn cam_at(pos: Vec3, yaw: f32) -> Camera {
        Camera {
            pos,
            yaw,
            pitch: 0.0,
            fov_y: 1.2,
            aspect: 1.6,
            near: 0.1,
            far: 256.0,
        }
    }

    #[test]
    fn pool_spawn_recycle_and_eviction() {
        let mut e = ParticleEngine::with_seed(1);
        // 满块破坏爆裂一次 = 64 粒（4×4×4，ClientLevel.java:951-953）。
        e.spawn_block_crack([0.0, 8.0, 0.0], mcv_core::tiles::STONE, 0);
        assert_eq!(e.len(), 64, "full-cube crack spawns 4*4*4");
        // 打满池：上限不越界（EvictingQueue 挤最老）。
        for _ in 0..MAX_PARTICLES {
            e.spawn_terrain([0.0, 9.0, 0.0], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        }
        assert_eq!(e.len(), MAX_PARTICLES, "pool capped at 16384");
        // 再挤 10 个：仍在上限内。
        for _ in 0..10 {
            e.spawn_terrain([0.0, 9.0, 0.0], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        }
        assert_eq!(e.len(), MAX_PARTICLES, "eviction keeps cap");
        // 到期回收（lifetime ≤ 40 tick）。
        let world = NoWorld;
        for _ in 0..64 {
            e.tick(&world);
        }
        assert!(e.is_empty(), "expired particles are recycled");
        // 空闲链无泄漏：回收后可重新用满池。
        for _ in 0..MAX_PARTICLES {
            e.spawn_terrain([0.0, 9.0, 0.0], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        }
        assert_eq!(e.len(), MAX_PARTICLES);
    }

    #[test]
    fn crack_uv_rect_inside_tile_quarter_width() {
        // TerrainParticle.java:51-52,62-79：uo,vo = rand·3，矩形
        // [uo, uo+1]/4 ⊆ [0,1]，宽高 = tile 的 1/4。
        let mut e = ParticleEngine::with_seed(7);
        e.spawn_block_crack([0.0, 8.0, 0.0], mcv_core::tiles::STONE, 0);
        // eye = pos + EYE_HEIGHT(1.62) → 相机给 7.5，块体 y 8..9 全入视锥。
        let cam = cam_at(Vec3::new(0.5, 7.5, 4.0), 0.0);
        let mut out = Vec::new();
        e.extract_vertices(&cam, 1.0, 0.0, &mut out);
        assert_eq!(out.len(), 64 * 4, "4 verts per particle");
        let mut min_u = f32::MAX;
        let mut max_u = f32::MIN;
        let mut min_v = f32::MAX;
        let mut max_v = f32::MIN;
        for quad in out.chunks(4) {
            assert_eq!(quad[0].layer, quad[3].layer, "flat layer per quad");
            for v in quad {
                assert!((0.0..=1.0).contains(&v.uv[0]), "u inside tile: {v:?}");
                assert!((0.0..=1.0).contains(&v.uv[1]), "v inside tile: {v:?}");
                min_u = min_u.min(v.uv[0]);
                max_u = max_u.max(v.uv[0]);
                min_v = min_v.min(v.uv[1]);
                max_v = max_v.max(v.uv[1]);
            }
            // 顶点序（QuadParticleRenderState.java:130-137）：
            // v0=(u1,v1) v1=(u1,v0) v2=(u0,v0) v3=(u0,v1)。
            let w = quad[0].uv[0] - quad[3].uv[0];
            assert!(
                (w - 0.25).abs() < 1e-5,
                "crack UV width = 1/4 tile, got {w}"
            );
            let h = quad[0].uv[1] - quad[1].uv[1];
            assert!(
                (h - 0.25).abs() < 1e-5,
                "crack UV height = 1/4 tile, got {h}"
            );
        }
        assert!(max_u <= 1.0 && max_v <= 1.0, "rects never leave tile");
        assert!(min_u < max_u && min_v < max_v, "offsets vary randomly");
    }

    #[test]
    fn gravity_friction_and_ground_collision() {
        let mut e = ParticleEngine::with_seed(3);
        e.spawn_terrain([0.5, 8.5, 0.5], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        let i = *e.live.front().unwrap() as usize;
        let world = Floor;
        // 出生带散布初速（TerrainParticle 速度构造，Particle.java:52-62），
        // 重力 0.04·gravity(1) 逐 tick 施加（Particle.java:97）。
        e.tick(&world);
        assert!(
            (e.y[i] - e.yo[i]).abs() > 1e-4,
            "particle moves on first tick"
        );
        // 落在 y=8（盒底贴地）且 onGround。
        for _ in 0..60 {
            e.tick(&world);
        }
        assert!(
            (e.y[i] - 8.0).abs() < 0.05,
            "rests on ground y=8, got {}",
            e.y[i]
        );
        assert!(e.flags[i] & F_ON_GROUND != 0, "on_ground set");
        // 到期回收（Particle.java:49 lifetime ≤ 40）。
        for _ in 0..64 {
            e.tick(&world);
        }
        assert!(e.is_empty(), "expired particles are recycled");
    }

    #[test]
    fn tick_kinds_diverge() {
        // Terrain 重力 0.04·gravity(1)（Particle.java:97）；WaterDrop 直落
        // gravity(0.06)（WaterDropParticle.java:44）。重力字段与 kind 分派
        // 回归锁（位移方向受散布初速影响，不比符号）。
        let mut et = ParticleEngine::with_seed(9);
        et.spawn_terrain([0.5, 8.5, 0.5], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        let it = *et.live.front().unwrap() as usize;
        let mut ed = ParticleEngine::with_seed(9);
        ed.spawn_water_splash(0.5, 8.5, 0.5, [0.0, 0.0, 0.0], 0.6);
        let drop = *ed.live.back().unwrap() as usize;
        let bubble = *ed.live.front().unwrap() as usize;
        assert_eq!(et.kind[it], Kind::Terrain);
        assert_eq!(ed.kind[drop], Kind::WaterDrop);
        assert_eq!(ed.kind[bubble], Kind::Bubble);
        assert!(
            (et.gravity[it] - 1.0).abs() < 1e-6,
            "TerrainParticle gravity=1"
        );
        assert!(
            (ed.gravity[drop] - 0.06).abs() < 1e-6,
            "WaterDrop gravity=0.06"
        );
        assert!(ed.gravity[bubble].abs() < 1e-6, "Bubble gravity=0");
    }

    #[test]
    fn lifetime_alpha_curve() {
        let a = LifetimeAlpha {
            start_alpha: 1.0,
            end_alpha: 0.0,
            start_at_normalized_age: 0.75,
            end_at_normalized_age: 1.0,
        };
        assert!((a.current_alpha(0, 100, 0.0) - 1.0).abs() < 1e-6);
        assert!(
            (a.current_alpha(50, 100, 0.0) - 1.0).abs() < 1e-6,
            "before fade window"
        );
        let mid = a.current_alpha(87, 100, 0.0);
        assert!((0.3..0.7).contains(&mid), "mid fade ≈ 0.5: {mid}");
        assert!((a.current_alpha(100, 100, 0.0) - 0.0).abs() < 1e-6);
        assert!(
            (LifetimeAlpha::ALWAYS_OPAQUE.current_alpha(17, 30, 0.5) - 1.0).abs() < 1e-6,
            "opaque stays opaque"
        );
    }

    #[test]
    fn billboard_vertices_face_camera() {
        let mut e = ParticleEngine::with_seed(11);
        e.spawn_terrain([0.0, 9.0, -3.0], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        // LOOKAT_XYZ（TerrainParticle 默认，SingleQuadParticle.java:47-49）：
        // yaw=0 看 -Z，quad 展开在 x/y、z 恒等于粒子 z。
        let cam = cam_at(Vec3::new(0.0, 7.5, 0.0), 0.0);
        let mut out = Vec::new();
        e.extract_vertices(&cam, 1.0, 0.0, &mut out);
        assert_eq!(out.len(), 4);
        for v in &out {
            assert!(
                (v.pos[2] - (-3.0)).abs() < 1e-4,
                "quad spans camera plane: {v:?}"
            );
        }
        let span_x = span(&out, 0);
        let span_y = span(&out, 1);
        assert!(
            span_x > 0.02 && span_y > 0.02,
            "quad opens: {span_x} {span_y}"
        );
        // 相机 yaw=90°（看 +X）：quad 展开在 z/y、x 恒定（billboard 跟转）。
        // 粒子换到视线内 (3, 9, 0)。
        e.clear();
        e.spawn_terrain([3.0, 9.0, 0.0], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        let cam2 = cam_at(Vec3::new(0.0, 7.5, 0.0), std::f32::consts::FRAC_PI_2);
        let mut out2 = Vec::new();
        e.extract_vertices(&cam2, 1.0, 0.0, &mut out2);
        for v in &out2 {
            assert!(
                (v.pos[0] - 3.0).abs() < 1e-4,
                "quad follows yaw (flat in x): {v:?}"
            );
        }
        assert!(span(&out2, 2) > 0.02, "quad opens along z after yaw");
    }

    fn span(verts: &[ParticleVertex], axis: usize) -> f32 {
        let lo = verts.iter().map(|v| v.pos[axis]).fold(f32::MAX, f32::min);
        let hi = verts.iter().map(|v| v.pos[axis]).fold(f32::MIN, f32::max);
        hi - lo
    }

    #[test]
    fn spawn_hit_offsets_away_from_face_and_powers_up() {
        // ClientLevel.java:988-1007：命中面外偏 0.1。
        let mut e = ParticleEngine::with_seed(5);
        e.spawn_hit(
            [0.0, 8.0, 0.0],
            2,
            mcv_core::tiles::STONE,
            [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        );
        let i = *e.live.back().unwrap() as usize;
        // +Y 面 → y = 1 + 0.1 = 1.1（相对 pos）。
        assert!(
            (e.y[i] - 9.1).abs() < 1e-6,
            "UP face offsets +0.1: {}",
            e.y[i]
        );
        assert!(
            e.x[i].abs() < 0.9 && (e.z[i]).abs() < 0.9,
            "in-box random x/z"
        );
        // setPower(0.2)（Particle.java:64-69）：yd = (yd−0.1)·0.2+0.1。
        assert!(
            e.yd[i] > 0.0 && e.yd[i] < 0.2,
            "power-scaled upward pop: {}",
            e.yd[i]
        );
        // scale(0.6)：碰撞盒 0.12。
        assert!((e.rend[i].bb_width - 0.12).abs() < 1e-6);
    }

    #[test]
    fn splash_spawns_bubble_and_drop_waves() {
        // Entity.java:1607-1620：两波各 1 + width·20 = 13（width 0.6）。
        let mut e = ParticleEngine::with_seed(13);
        e.spawn_water_splash(0.5, 8.5, 0.5, [0.1, -0.3, 0.0], 0.6);
        assert_eq!(e.len(), 26, "13 bubbles + 13 splashes");
        let bubble = *e.live.front().unwrap() as usize;
        let drop = *e.live.back().unwrap() as usize;
        assert_eq!(e.kind[bubble], Kind::Bubble);
        assert_eq!(e.kind[drop], Kind::WaterDrop);
        assert!(
            e.rend[drop].layer < sprites::SPLASH_BASE + sprites::SPLASH_COUNT,
            "splash random frame 0..3, got {}",
            e.rend[drop].layer
        );
        assert!(
            (e.gravity[drop] - 0.06).abs() < 1e-6,
            "WaterDrop gravity 0.06"
        );
        // 气泡初速含 movement·0.2 分量（Entity.java:1612）。
        assert!(e.xd[bubble].abs() < 0.1 && e.yd[bubble].abs() < 0.1);
    }

    #[test]
    fn crit_ignores_physics_and_grows_in() {
        let mut e = ParticleEngine::with_seed(17);
        e.spawn_crit(0.0, 9.0, 0.0, 0.3, 0.4, 0.0, false);
        let i = *e.live.back().unwrap() as usize;
        assert!(
            e.flags[i] & F_HAS_PHYSICS == 0,
            "CritParticle hasPhysics=false"
        );
        assert!(e.rend[i].grow_in, "crit quad grows in");
        assert!((e.gravity[i] - 0.5).abs() < 1e-6 && (e.friction[i] - 0.7).abs() < 1e-6);
        // 非指示器 lifetime ∈ [1, 8]（6/(rand·0.8+0.6)，rand=0 → 10？6/0.6=10）。
        assert!(
            e.lifetime[i] >= 1 && e.lifetime[i] <= 10,
            "crit lifetime {}",
            e.lifetime[i]
        );
        // 出生即 tick 一步（CritParticle.java:44）：age 已为 1。
        assert_eq!(e.age[i], 1, "constructor tick()");
    }

    #[test]
    fn frustum_culls_offscreen_particles() {
        let mut e = ParticleEngine::with_seed(19);
        e.spawn_terrain([0.0, 9.0, -3.0], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        e.spawn_terrain([0.0, 9.0, 30.0], [0.0, 0.0, 0.0], mcv_core::tiles::STONE);
        // eye = pos + 1.62 → 相机给 7.5，两粒均在其视锥几何内；后方粒子
        // (z=+30) 被点剔除（QuadParticleGroup.java:35）。
        let cam = cam_at(Vec3::new(0.0, 7.5, 0.0), 0.0);
        let mut out = Vec::new();
        e.extract_vertices(&cam, 1.0, 0.0, &mut out);
        assert_eq!(
            out.len(),
            4,
            "behind-camera particle culled (QuadParticleGroup.java:35)"
        );
    }

    #[test]
    fn rain_drop_ctor_matches_water_drop() {
        // spawn_rain_drop = WaterDropParticle 构造全集（WaterDropParticle.
        // java:11-19；aux 速度 = 0，WeatherEffectRenderer.java:258-266）：
        // gravity 0.06、初速上跳 0.1..0.3、lifetime = 8/(rand·0.8+0.2)
        // ∈ [8,40]、纹理 splash_0..3（assets particles/rain.json）。
        let mut e = ParticleEngine::with_seed(21);
        e.spawn_rain_drop(0.5, 8.0, 0.5);
        let i = *e.live.back().unwrap() as usize;
        assert_eq!(
            e.kind[i],
            Kind::WaterDrop,
            "RAIN 粒子 = WaterDropParticle（ParticleResources.java:117）"
        );
        assert!((e.gravity[i] - 0.06).abs() < 1e-6, "gravity 0.06（:17）");
        assert!(
            e.yd[i] >= 0.1 && e.yd[i] < 0.3,
            "yd = rand·0.2+0.1 上跳（:14），got {}",
            e.yd[i]
        );
        assert_eq!(e.xd[i], 0.0);
        assert_eq!(e.zd[i], 0.0);
        assert!(
            e.lifetime[i] >= 8 && e.lifetime[i] <= 40,
            "lifetime 8/(rand·0.8+0.2)（:18）, got {}",
            e.lifetime[i]
        );
        assert_eq!(e.rend[i].tex_set, 1);
        assert!(
            e.rend[i].layer >= sprites::SPLASH_BASE
                && e.rend[i].layer < sprites::SPLASH_BASE + sprites::SPLASH_COUNT,
            "雨纹理 = splash_0..3（rain.json 与 splash.json 同图集）"
        );
        // 首 tick 上跳（yd ≥ 0.1 > gravity 0.06）。
        let y0 = e.y[i];
        e.tick(&NoWorld);
        assert!(e.y[i] > y0, "首 tick 上跳");
    }

    #[test]
    fn rain_drop_dies_at_fluid_surface() {
        // WaterDropParticle.java:48-55：y < 本格形状/流体顶面 → remove
        // （雨滴落水即灭的判据；固体面由 collide_voxel 满格截停承担）。
        struct WaterCell;
        impl ParticleWorld for WaterCell {
            fn is_solid(&self, _x: i32, _y: i32, _z: i32) -> bool {
                false
            }
            fn fluid_top(&self, _x: i32, y: i32, _z: i32) -> f64 {
                if y == 8 { 1.0 } else { 0.0 }
            }
        }
        let mut e = ParticleEngine::with_seed(23);
        e.spawn_rain_drop(0.5, 8.4, 0.5); // y 8.4 < 8+1.0 → 首 tick 判死
        e.tick(&WaterCell);
        assert!(e.is_empty(), "没入流体顶面即移除（:48-55）");
        // 无水世界不触发该判据（fluid_top 默认 0）。
        let mut e2 = ParticleEngine::with_seed(23);
        e2.spawn_rain_drop(0.5, 8.4, 0.5);
        e2.tick(&NoWorld);
        assert_eq!(e2.len(), 1, "无流体面不误杀");
    }

    #[test]
    fn drown_bubbles_are_8_and_die_outside_water() {
        // makeDrownParticles（LivingEntity.java:2113-2123）：8 个 BUBBLE、
        // 偏移 triangle(0,1) ∈ [-1,1]；离水即灭（BubbleParticle.java:43-45）。
        struct Air;
        impl ParticleWorld for Air {
            fn is_solid(&self, _x: i32, _y: i32, _z: i32) -> bool {
                false
            }
            fn is_water(&self, _x: i32, _y: i32, _z: i32) -> bool {
                false
            }
        }
        let mut e = ParticleEngine::with_seed(11);
        e.spawn_drown_bubbles(0.5, 8.5, 0.5, [0.4, -0.2, 0.0]);
        assert_eq!(e.len(), 8, "8 BUBBLE（LivingEntity.java:2116）");
        let i = *e.live.front().unwrap() as usize;
        assert_eq!(e.kind[i], Kind::Bubble);
        // triangle 偏移域。
        for &slot in &e.live {
            let i = slot as usize;
            for (v, c) in [(e.x[i], 0.5), (e.y[i], 8.5), (e.z[i], 0.5)] {
                assert!((v - c).abs() <= 1.0 + 1e-9, "triangle offset ∈ [-1,1]");
            }
        }
        // 水中存活（NoWorld is_water 默认 true）、空气中下一 tick 全灭。
        let mut ew = ParticleEngine::with_seed(11);
        ew.spawn_drown_bubbles(0.5, 8.5, 0.5, [0.0, 0.0, 0.0]);
        ew.tick(&NoWorld);
        assert_eq!(ew.len(), 8, "水中存活");
        e.tick(&Air);
        assert!(e.is_empty(), "离水即灭（BubbleParticle.java:43-45）");
    }
}
