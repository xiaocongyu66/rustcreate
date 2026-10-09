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
use mcv_entity::{Health, LastHurt, MobId, MobKind, MobTicks, PhysBody, Yaw, spawn_mob};
use mcv_game::{Player, VoxelAccess, step_entity};
use mcv_platform::touch::TouchState;
use mcv_render::gpu::RenderChunk;
use mcv_render::{Camera, HudQuad, text};

pub const RENDER_DIST: i32 = 8;

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
    pub save_dir: std::path::PathBuf,
    render_chunks: Vec<RenderChunk>,
    spawned: bool,
    border_synced: HashMap<ChunkPos, u8>,
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
    spawn_cooldown: u32,
    pub player_xp: u32,
    /// 9 格快捷栏(vanilla Inventory 子集):放置消耗选中槽 Block 物品、
    /// 生存破坏掉落入栏、攻击武器 = 选中槽(方块按拳头)。
    pub hotbar: mcv_item::Hotbar,
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
            // 1.0 的情形，按源码收紧。
            let pos = h.pos;
            self.abort();
            self.delay = 5;
            return MineTick::Broken(pos);
        }
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

impl GameRuntime {
    pub fn new(
        seed: u64,
        uploader: mcv_render::gpu::MeshUploader,
        save_dir: std::path::PathBuf,
        mode: GameMode,
    ) -> Self {
        Self::assemble(
            seed,
            Box::new(CxxMesher::new(256 << 20, uploader)),
            save_dir,
            mode,
        )
    }

    /// 无头构造（集成测试/CI）：NullMesher 不触 GPU，其余接线与 [`new`](Self::new) 全同。
    pub fn new_headless(seed: u64, save_dir: std::path::PathBuf, mode: GameMode) -> Self {
        Self::assemble(seed, Box::new(NullMesher), save_dir, mode)
    }

    fn assemble(
        seed: u64,
        mesher: Box<dyn ChunkMesher>,
        save_dir: std::path::PathBuf,
        mode: GameMode,
    ) -> Self {
        // 开局装备:铁剑(旧单格行为)。创造另发 8 格可放方块(开发期
        // 创造背包未做,给旧调试快捷栏的等价子集;水/基岩不可入栏)。
        let mut hotbar = mcv_item::Hotbar::empty();
        hotbar.slots[0] = mcv_item::ItemStack::new(mcv_item::IRON_SWORD_INDEX, 1);
        if mode == GameMode::Creative {
            for (i, item) in [
                mcv_item::STONE_ITEM,
                mcv_item::DIRT_ITEM,
                mcv_item::GRASS_ITEM,
                mcv_item::SAND_ITEM,
                mcv_item::COBBLESTONE,
                mcv_item::PLANKS,
                mcv_item::LOG,
                mcv_item::LEAVES_ITEM,
            ]
            .into_iter()
            .enumerate()
            {
                hotbar.slots[1 + i] = mcv_item::ItemStack::new(item, 64);
            }
        }
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
            save_dir,
            render_chunks: Vec::new(),
            spawned: false,
            border_synced: HashMap::new(),
            mobs_app: mcv_ecs::App::new(),
            // 攻击冷却 ticker，单位 tick（Player.attackStrengthTicker）；
            // 20 tick 起步 = 全武器满蓄力（attackSpeed≥1.0 → delay≤20 tick）。
            attack_ticker: 20.0, // ready
            food_tick_timer: 0,
            mine: MineMachine::default(),
            spawn_cooldown: 0,
            player_xp: 0,
            hotbar,
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
        };
        mcv_entity::register_mob_components(&mut rt.mobs_app.world);
        mcv_entity::register_drop_components(&mut rt.mobs_app.world);
        // 启动期注册、注册序即执行序(Godot ClassDB 原则)。
        rt.mobs_app
            .add_system(mcv_ecs::Stage::Fixed, "mob_ai", mob_ai_system);
        // 掉落物：物理/寿命系统 + 拾取/合并系统（同一 World，视图分区，
        // mob_ai 按 MobKind 过滤、掉落系统按 ItemDrop 过滤，互不触碰）。
        // TODO 渲染：mcv_render Scene 目前只有地形/HUD/玩家模型通路，怪物
        // 也未上屏——掉落物待通用实体渲染通路（billboard 物品图标或 1/4
        // 缩放方块）落地后再接，先保证物理+拾取语义完整。
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
        let p = &mut self.player;
        if p.health <= 0.0 || self.mode == GameMode::Creative {
            return;
        }
        let guard = p.invulnerable > 10;
        let Some(dmg) =
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
        p.health -= dmg;
        // 受伤 exhaustion 按 damage_type 数据取值（Player.java:761
        // causeFoodExhaustion(source.getFoodExhaustion())）：实体攻击
        // mob_attack/player_attack.json = 0.1，fall/out_of_world.json = 0.0；
        // 本入口 `from` 有值 ≙ 实体攻击。创造已在上方豁免（对应
        // Player.causeFoodExhaustion:1561-1567 的 abilities.invulnerable 门）。
        if from.is_some() {
            p.exhaustion = (p.exhaustion + 0.1).min(EXHAUSTION_MAX);
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
            if self.mode == GameMode::Hardcore {
                self.hardcore_death = true;
            }
            // 死亡掉落（26.1 Player.die → Inventory.dropAll，keepInventory
            // 默认 false）：快捷栏逐格生成 ItemDrop（拾取延迟 40 tick =
            // 2 s，LivingEntity.java:3398），与 mob 死亡掉落同一生成路径，
            // 再清栏。
            let at = p.pos + Vec3::Y * 0.9;
            let mut rng = spawn_rng();
            for s in &self.hotbar.slots {
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
        self.audio.play_event("entity.player.hurt", pos, pos, 1.0);
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
                    // v3 起存档带快捷栏;v1/v2 读为空——保留开局装备,
                    // 不能把 kit 擦成空栏。物品 id 越界(旧档)整槽跳过。
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
        if !self.spawned
            && self.player.pos == Vec3::ZERO
            && let Some(handle) = self.chunks.get(&ChunkPos::new(0, 0))
            && (handle.stage() as u8) >= (Stage::TerrainReady as u8)
        {
            let hm = handle.heightmap.read().unwrap();
            let y = hm[(8 << 4) | 8];
            self.player.pos = Vec3::new(8.5, f32::from(y) + 1.0, 8.5);
            self.spawned = true;
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
        // ---- 20 Hz tick 基建（原版 tick 语义的唯一换算点）----
        let n = accumulate_ticks(&mut self.tick_frac, dt);
        self.on_tick = n > 0;
        if self.on_tick {
            self.game_ticks += n;
            self.time_ticks += n; // 26.1 ServerClockManager 每 tick +1
        }
        self.apply_touch_input();
        if self.dead {
            // 死亡界面：尸体不响应输入，仅重力继续
            self.input = Default::default();
        }
        // 攻击冷却 ticker：tick 单位（26.1 Player.java:267 每 tick +1；消费侧
        // combat::attack_strength 的 delay = 20/attackSpeed tick，Player.java:
        // 1793-1795）。旧实现按秒累加又被当 tick 消费，铁剑满蓄力 12.5s（正确
        // 12.5 tick = 0.625s）。上限 20 tick（最慢武器 attackSpeed 1.0 → 蓄满）。
        if self.on_tick {
            self.attack_ticker = (self.attack_ticker + n as f32).min(20.0);
        }
        self.step_mining(dt);
        self.spawn_cooldown = self.spawn_cooldown.saturating_sub(1);

        // ---- natural spawning (budgeted every 20 ticks) ----
        if self.spawn_cooldown == 0 {
            self.spawn_cooldown = 20;
            self.try_natural_spawn();
        }

        // ---- mob AI + physics（ECS 调度：固定步驱动一次 Fixed 阶段）----
        // 系统上下文借自 mobs_app 各字段（disjoint 借用）；区块表克隆与
        // 玩家位姿每步 insert 进 Resources 快照（Arc 计数级克隆，体素数据
        // 共享）。近战命中走事件（信号语义）：run_stage 阶段末排空命令、
        // 翻转事件之后，GameRuntime 统一结算成玩家伤害。
        {
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
                ticks_step: n.min(u32::MAX as u64) as u32,
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
        }
        let hits: Vec<MobMeleeHit> = self.mobs_app.events.channel::<MobMeleeHit>().take();
        for h in hits {
            self.hurt_player(h.damage.max(1.0), Some(h.src));
        }
        // 掉落物拾取结算：入栏走 Hotbar::add（give 路径唯一），满栏剩余留地。
        mcv_entity::settle_pickups(
            &mut self.mobs_app.world,
            &mut self.mobs_app.events,
            &mut self.hotbar,
            self.player.sel_slot,
        );

        // ---- 玩家物理（mcv_game::step，60 Hz 固定步）----
        {
            let f = self.camera(1.0).dir();
            let f = Vec3::new(f.x, 0.0, f.z)
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
                // 冲刺提速 4.317→5.612 m/s（LivingEntity.java:156-158 +30%）；
                // 潜行在 step 内优先于冲刺（蹲下即退冲刺）。
                sprint: sprinting,
                gravity_scale: 1.0,
            };
            mcv_game::step(
                &WorldView {
                    chunks: &self.chunks,
                },
                &mut self.player,
                &step_input,
            );
            // ---- 行为音效：脚步 / 落地 ----
            let delta = self.player.pos - before;
            let moved = delta.length();
            // exhaustion 只认水平位移（ServerPlayer.checkMovementStatistics:1444
            // 用 sqrt(dx²+dz²)，垂直不计）。
            let moved_h = Vec3::new(delta.x, 0.0, delta.z).length();
            self.step_dist += moved;
            if self.player.on_ground && was_air {
                if fall_v < -3.0 {
                    let p = self.player.pos;
                    self.audio.play_event(
                        "entity.player.small_fall",
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
                if let Some(event) = step_event(under) {
                    self.audio
                        .play_event(event, [p.x, p.y, p.z], [p.x, p.y, p.z], 0.35);
                }
            }
            // ---- 生存统计（26.1 LivingEntity/ServerPlayer/FoodData）----
            // 无敌帧递减：tick 单位（ServerPlayer.java:576-577 每 tick −1），
            // 旧实现按 60 Hz 步递减使 20 tick i 帧只剩 0.33s。
            if self.on_tick {
                self.player.invulnerable = self.player.invulnerable.saturating_sub(1);
            }
            // 事件式 exhaustion 累加（原版在移动/跳跃事件即时加，非每 tick）：
            // 冲刺地面水平位移 0.1/m、走路/潜行 0.0/m 且只计水平分量
            // （ServerPlayer.checkMovementStatistics:1443-1456 +
            // FoodConstants.java:25-27）；跳跃 = 冲刺跳 0.2 / 普通跳 0.05
            // （ServerPlayer.jumpFromGround:1532-1540 +
            // FoodConstants.java:21-22，旧实现恒 0.2 高估普通跳）。
            if self.mode != GameMode::Creative {
                let p = &mut self.player;
                if p.on_ground {
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
            // tickTimer（和平封顶 10 为本仓既有登记偏差）。
            if self.on_tick && self.mode != GameMode::Creative {
                let p = &mut self.player;
                food_data_tick(
                    &mut p.exhaustion,
                    &mut p.saturation,
                    &mut p.hunger,
                    &mut p.health,
                    &mut self.food_tick_timer,
                );
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
    }

    fn day_factor(&self) -> f32 {
        mcv_render::day_factor(self.time_ticks)
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
        let monsters = self
            .mobs_app
            .world
            .read::<MobKind>()
            .iter()
            .filter(|(_, k)| k.0.def().hostile)
            .count() as u32;
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
                    spawn_mob(&mut self.mobs_app.world, id, p);
                    break;
                }
            }
        }
    }

    /// 左键按下入口（桌面鼠标/触摸按下边沿共用）：先攻准星下的 mob，
    /// 未命中则创造秒破、生存/极限进入进度挖掘 START。**按住**期间的连挖
    /// 不在这里——那是 `fixed_step` 每 tick 驱动的 `step_mining`
    /// （原版 Minecraft.continueAttack，Minecraft.java:1606-1628）。
    pub fn on_left_press(&mut self) {
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
                    // 死亡判定读回组件现值（语义同原 mob.health <= 0.0）。
                    if struck && hp.0 <= 0.0 {
                        // 带上 defs::MobKind 枚举（death_drops 按种类查 loot）。
                        slain = Some((target, def.xp, def.kind));
                    }
                }
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
        // random.break（缺事件时加载器自带节流 no-op）。
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
                self.audio.play_event(
                    "random.break",
                    [eye.x, eye.y, eye.z],
                    [eye.x, eye.y, eye.z],
                    1.0,
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

    /// 身体（脚上 0.5 格）在水中——物理步输入（游泳/浮沉）用。
    fn in_water(&self, view: &WorldView) -> bool {
        let p = self.player.pos;
        let b = BlockPos::new(p.x as i32, (p.y + 0.5) as i32, p.z as i32);
        view.block(b).def().liquid
    }

    /// 眼睛是否在水中（26.1 `Player.isEyeInFluid(WATER)`，Player.java:607：
    /// 水下挖掘惩罚按**眼位**判定，与物理用的脚位版 in_water 区分）。
    /// 接 per-tick 速率惩罚链（原 mcv_game::mining 惩罚实现的 live 路径版）。
    fn eye_in_water(&self, view: &WorldView) -> bool {
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let c = eye.floor().as_ivec3();
        view.block(BlockPos::new(c.x, c.y, c.z)).def().liquid
    }

    /// Mouse look.
    pub fn look(&mut self, dx: f64, dy: f64) {
        let k = 0.0025 * self.sens;
        self.player.yaw += dx as f32 * k;
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
        let (hit, _) = dda_hit(&view, eye, dir, 5.0)?;
        Some((hit, view.block(hit).0))
    }

    pub fn interact(&mut self, place: bool) {
        let view = WorldView {
            chunks: &self.chunks,
        };
        let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
        let dir = self.camera(1.0).dir();
        // 交互距离按模式取 26.1 block_interaction_range（生存 4.5 / 创造 5.0）。
        let Some((hit, normal)) = dda_hit(&view, eye, dir, block_interaction_reach(self.mode))
        else {
            return;
        };
        if !place {
            // 破坏入口：创造秒破走这里；生存由 step_mining 完成后调 destroy_block。
            self.destroy_block(hit);
            return;
        }
        let target = BlockPos::new(hit.x + normal[0], hit.y + normal[1], hit.z + normal[2]);
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
            let [lx, ly, lz] = target.local();
            handle.voxels.write().unwrap()[ly << 8 | lz << 4 | lx] = new_id;
            handle.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
            // 生存放置消耗一格（vanilla consumeItem）；创造不消耗。
            if self.mode != GameMode::Creative {
                self.hotbar.take_one(self.player.sel_slot);
            }
            // 放置按新方块发声（26.1 GameRenderer 行为音）
            if let Some(group) = block_group(new_id.0) {
                let p = [
                    target.x as f32 + 0.5,
                    target.y as f32 + 0.5,
                    target.z as f32 + 0.5,
                ];
                self.audio
                    .play_event(&format!("{group}.place"), p, [eye.x, eye.y, eye.z], 1.0);
            }
        }
    }

    /// 破坏目标方块：体素清零 + MESH/SAVE 脏 + 生存掉落（26.1：掉落需要
    /// 正确工具，`hasCorrectToolForDrops` 门控）+ break 音效。挖掘进度完成
    /// 与创造秒破共用。
    fn destroy_block(&mut self, target: BlockPos) {
        let Some(handle) = self.chunks.get(&target.chunk()) else {
            return;
        };
        let [lx, ly, lz] = target.local();
        let idx = ly << 8 | lz << 4 | lx;
        let old = handle.voxels.read().unwrap()[idx];
        if old.0 == 0 {
            return;
        }
        handle.voxels.write().unwrap()[idx] = BlockId(0);
        handle.mark_dirty(mcv_core::dirty::MESH | mcv_core::dirty::SAVE);
        // 生存掉落需正确工具（错误工具能磨掉但不掉东西）。创造秒破不留
        // 掉落物（26.1 give 进创造背包，此处背包未做 → 直接消失）。
        if self.mode != GameMode::Creative {
            // 每破坏一方块 exhaustion 0.005（Block.playerDestroy，
            // Block.java:478 causeFoodExhaustion(0.005F)；创造经
            // abilities.invulnerable 门豁免，Player.java:1561-1567）。
            self.player.exhaustion = (self.player.exhaustion + EXHAUSTION_MINE).min(EXHAUSTION_MAX);
            let held = self.held_stack();
            if mcv_item::mining::has_correct_tool(old, held.as_ref())
                && let Some(drop) = mcv_item::drop_for_block(old)
            {
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
                mcv_entity::spawn_item_drop(
                    &mut self.mobs_app.world,
                    c,
                    drop.item,
                    drop.count,
                    mcv_entity::PICKUP_DELAY,
                    &mut rng,
                );
            }
        }
        if let Some(group) = block_group(old.0) {
            let eye = self.player.pos + Vec3::new(0.0, mcv_game::Player::EYE, 0.0);
            let p = [
                target.x as f32 + 0.5,
                target.y as f32 + 0.5,
                target.z as f32 + 0.5,
            ];
            self.audio
                .play_event(&format!("{group}.break"), p, [eye.x, eye.y, eye.z], 1.0);
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
        let Some((hit, _)) = dda_hit(&view, eye, dir, block_interaction_reach(self.mode)) else {
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
            let hit = dda_hit(&view, eye, dir, reach).map(|(p, _)| p);
            if let Some(p) = self.mine.creative_tick(hit) {
                self.destroy_block(p);
            }
            return;
        }
        // 每 tick 重射线（原版客户端 hitResult 每 tick 重算；超出 reach 打不中
        // → None → continue_tick 内 ABORT，取代旧"中心距 >5.5 才中止"）。
        let hit = dda_hit(&view, eye, dir, reach)
            .map(|(p, _)| (p, view.block(p)))
            .filter(|(_, b)| b.0 != 0)
            .map(|(p, b)| MineHit {
                pos: p,
                per_tick: self.mine_per_tick(&view, b),
            });
        if let MineTick::Broken(p) = self.mine.continue_tick(hit) {
            self.destroy_block(p);
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
        let [lx, ly, lz] = p.local();
        let v = chunk.light.read().unwrap()[ly << 8 | lz << 4 | lx];
        (v & 0xF, v >> 4)
    }

    /// 挖掘/选中 overlay（渲染层数据）：挖掘中目标锁定状态机目标并按进度
    /// 给裂纹档位（progress×4 取整，0..3）；未挖掘时准星 DDA 目标只描边。
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
                let (hit, _) = dda_hit(&view, eye, dir, block_interaction_reach(self.mode))?;
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
            Some(((self.mine.progress * 4.0) as u32).min(3))
        } else {
            None
        };
        Some(mcv_render::gpu::MiningOverlay {
            min: [target.x as f32, target.y as f32, target.z as f32],
            crack_stage: stage,
            faces,
        })
    }

    pub fn render_chunks(&self) -> &[RenderChunk] {
        &self.render_chunks
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
                    match stack.def().kind {
                        mcv_item::ItemKind::Block(bid) => quads.push(text::tile_icon(
                            mcv_core::BLOCKS[bid.0 as usize].tiles[2],
                            ix,
                            iy,
                            16.0 * s,
                        )),
                        _ => quads.extend(g.sprite_full(
                            stack.def().name,
                            ix,
                            iy,
                            16.0 * s,
                            16.0 * s,
                            white,
                        )),
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
                        match stack.def().kind {
                            mcv_item::ItemKind::Block(bid) => quads.push(text::tile_icon(
                                mcv_core::BLOCKS[bid.0 as usize].tiles[2],
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
/// （追踪目标）。每固定步 `insert` 进 Resources——克隆只是加 Arc 计数，
/// 体素/光照数据仍与原表共享。
#[derive(Clone)]
struct MobServices {
    chunks: HashMap<ChunkPos, Arc<ChunkHandle>>,
    player_pos: Vec3,
    /// 本固定步跨过的 20 Hz tick 数（0/1 常态）。mob 侧一切按 tick 计时的
    /// 计数器都按它递减——原版 LivingEntity.tick 每 game tick 给非玩家实体
    /// 的 invulnerableTime −1（LivingEntity.java:452-453），绝不按 60 Hz 步
    /// 递减（旧实现 20 计数 = 0.33s，怪 3 击/秒、i 帧缩水 3 倍）。
    ticks_step: u32,
}

/// 近战命中事件（信号）：AI 系统发射，GameRuntime 在固定步末 drain 后结算
/// 玩家伤害——系统与玩家状态之间不共享可变借用。
#[derive(Clone, Copy)]
struct MobMeleeHit {
    src: Vec3,
    damage: f32,
}

/// mob AI + 物理固定步系统：一次取齐全部表视图跑 for_each；结构性变更
/// （死亡 despawn）走延迟命令，命中走事件。逻辑与原内联版一一对应。
fn mob_ai_system(ctx: &mut mcv_ecs::SysCtx) {
    let svc = ctx
        .resources
        .get::<MobServices>()
        .expect("mob_ai：MobServices 快照未注入 Resources");
    // disjoint 字段借用：表视图挂 world，命令/事件各自独立可写。
    let world: &mcv_ecs::World = ctx.world;
    let commands = &mut *ctx.commands;
    let events = &mut *ctx.events;
    let (mut phys, kind, mut ticks, mut yaw, health) = (
        world.write::<PhysBody>(),
        world.read::<MobKind>(),
        world.write::<MobTicks>(),
        world.write::<Yaw>(),
        world.read::<Health>(),
    );
    phys.for_each(|e, body| {
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
        let idle = tk.idle_ticks;
        let mut pos = body.pos;
        let to_player = svc.player_pos - pos;
        let dist_sqr = to_player.length_squared();
        let speed = speed_m_s(def.speed_attr);
        let wish = if def.hostile && dist_sqr < def.follow_range * def.follow_range {
            // chase
            let dir = Vec3::new(to_player.x, 0.0, to_player.z).normalize_or_zero();
            if let Some(y) = yaw.get_mut(e) {
                y.0 = dir.z.atan2(dir.x);
            }
            dir * speed
        } else {
            // wander: random direction changes on idle ticks
            if idle % 120 == 0
                && (fast_rand() & 3) == 0
                && let Some(y) = yaw.get_mut(e)
            {
                y.0 = (idle as f32 * 0.7) % std::f32::consts::TAU;
            }
            let heading = yaw.get(e).map_or(0.0, |y| y.0);
            Vec3::new(heading.sin(), 0.0, -heading.cos()) * speed * 0.3
        };
        let melee = def.hostile && def.attack_damage > 0.0 && dist_sqr < 2.25;
        let input = mcv_game::StepInput {
            wish_dir: if melee { Vec3::ZERO } else { wish },
            jump: body.on_ground && to_player.y > 1.0 && dist_sqr < 16.0,
            in_water: false,
            sneak: false,
            sprint: false,
            gravity_scale: 1.0,
        };
        // 独立表视图（各自 RefCell）：与 phys 的迭代借用互不冲突。
        let mut eng = body.body();
        step_entity(
            &WorldView {
                chunks: &svc.chunks,
            },
            &mut eng,
            def.half_size,
            &input,
        );
        body.set_body(&eng);
        pos = body.pos;
        tk.idle_ticks += 1;
        // invulnerable 复用为近战冷却：命中后置 20 tick（1s，26.1 僵尸节奏）
        if melee && tk.invulnerable == 0 {
            tk.invulnerable = 20;
            events.channel::<MobMeleeHit>().send(MobMeleeHit {
                src: pos,
                damage: def.attack_damage,
            });
        }
        // 死亡清理：Health<=0 → 排队 despawn（阶段末生效，等价原循环后
        // retain）+ 环境死亡掉落（非玩家击杀，spider_eye 不掉；玩家击杀
        // 在 try_attack 即时结算，不会走到这里）。
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
///    代价走 exhaustion+min(饱和,6)（FoodData.java:45-52）；
/// 3. 回血慢线：hunger≥18 且受伤，每 80 tick 回 1 HP，代价 exhaustion+6
///    （FoodData.java:53-59，FoodConstants.java:20 EXHAUSTION_HEAL=6.0——旧实现
///    回血零代价）；
/// 4. 饥饿掉血：hunger=0 每 80 tick 掉 1（难度封顶为既有登记偏差：一律按
///    和平封顶 10）。
pub fn food_data_tick(
    exhaustion: &mut f32,
    saturation: &mut f32,
    hunger: &mut f32,
    health: &mut f32,
    tick_timer: &mut u32,
) {
    if *exhaustion > 4.0 {
        *exhaustion -= 4.0;
        if *saturation > 0.0 {
            *saturation = (*saturation - 1.0).max(0.0);
        } else {
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
            if *health > 10.0 {
                *health -= 1.0;
            }
            *tick_timer = 0;
        }
    } else {
        *tick_timer = 0;
    }
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
    let (hit, normal) = dda_hit(view, eye, dir, max_dist)?;
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

/// 挖掘/放置音效材质 → 26.1 sounds.json 事件前缀（BLOCKS 表序：0air 1stone
/// 2dirt 3grass 4sand 5water 6log 7leaves 8planks 9cobble 10bedrock 11snow_grass
/// 12/13花）。调用点按动作拼 `.place` / `.break` 后缀；26.1 无 dig/dirt 组，
/// 泥土/草/沙共用 block.grass 音组。
fn block_group(vid: u16) -> Option<&'static str> {
    match vid {
        1 | 9 | 10 => Some("block.stone"),
        2 | 3 | 4 | 7 | 11 => Some("block.grass"),
        6 | 8 => Some("block.wood"),
        _ => None,
    }
}

/// 脚步材质 → 26.1 sounds.json 事件名：草方块踩草地音，沙/石踩石头音，
/// 木板/原木踩木头音。
fn step_event(vid: u16) -> Option<&'static str> {
    match vid {
        1 | 4 | 9 | 10 => Some("block.stone.step"),
        2 | 3 | 7 | 11 => Some("block.grass.step"),
        6 | 8 => Some("block.wood.step"),
        _ => None,
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
