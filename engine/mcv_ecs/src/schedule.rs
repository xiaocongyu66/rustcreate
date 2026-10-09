//! 系统调度(Godot `_physics_process`/`_process` 双阶段模型):
//! 固定步(默认 1/60,带追帧上限防死亡螺旋)+ 可变帧。系统在**启动期**
//! 注册、显式注册顺序执行,运行期只读(呼应 Godot ClassDB 的"注册在启动、
//! 运行期冻结")。每个阶段结束由调度器统一:排空 [`CommandQueue`] →
//! 翻转全部事件通道——阶段内看到的 World/事件都是稳定快照。
//!
//! 系统签名 [`System`] = `FnMut(&mut SysCtx)`:拿 World 表视图写循环、
//! 读写 [`Resources`](../resources/index.html)、`send` 事件、结构性变更走
//! [`Commands`](crate::CommandQueue)。系统间的宿主状态(配置、玩家快照等)
//! 走 [`Resources`]:宿主每步 `insert` 一份**所有权快照**即可——借来的东西
//! 进不了类型键表(`TypeId` 要求 `'static`),快照的所有权模型更简单,
//! 而 `Arc` 化数据本就是共享的,克隆只是计数。

use std::any::Any;
use std::collections::HashMap;

use crate::World;
use crate::commands::CommandQueue;
use crate::events::EventBus;
use crate::resources::Resources;

/// 固定步长(秒):60 Hz,与原版 tick 率一致。
pub const DEFAULT_FIXED_DT: f32 = 1.0 / 60.0;
/// 单帧最多追补的固定步数——卡顿后跳帧而非补作业(死亡螺旋防线)。
pub const MAX_CATCHUP_STEPS: u32 = 5;

/// 调度阶段:固定步(物理/玩法)与可变帧(表现/插值)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Fixed,
    Variable,
}

/// 一步系统上下文:五个 disjoint 字段,系统内按需各自借用(字段分离
/// 借用互不冲突,与 [`World::read`] 元组模式同理)。
pub struct SysCtx<'a> {
    pub world: &'a mut World,
    pub resources: &'a mut Resources,
    pub events: &'a mut EventBus,
    pub commands: &'a mut CommandQueue,
}

/// 已注册系统:名字(调试/断言用)+ 闭包。启动期注册,运行期只跑不改。
pub struct System {
    pub name: &'static str,
    run: Box<dyn for<'a> FnMut(&mut SysCtx<'a>)>,
}

impl System {
    pub fn new<F>(name: &'static str, f: F) -> Self
    where
        F: for<'a> FnMut(&mut SysCtx<'a>) + 'static,
    {
        Self {
            name,
            run: Box::new(f),
        }
    }
}

pub struct Schedule {
    fixed: Vec<System>,
    variable: Vec<System>,
    fixed_dt: f32,
    acc: f32,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            fixed: Vec::new(),
            variable: Vec::new(),
            fixed_dt: DEFAULT_FIXED_DT,
            acc: 0.0,
        }
    }

    pub fn with_fixed_dt(mut self, dt: f32) -> Self {
        assert!(dt > 0.0, "固定步长必须为正");
        self.fixed_dt = dt;
        self
    }

    /// 注册系统:同阶段内按注册顺序执行。
    pub fn add<F>(&mut self, stage: Stage, name: &'static str, f: F)
    where
        F: for<'a> FnMut(&mut SysCtx<'a>) + 'static,
    {
        let sys = System::new(name, f);
        match stage {
            Stage::Fixed => self.fixed.push(sys),
            Stage::Variable => self.variable.push(sys),
        }
    }

    /// 某阶段的已注册系统(调试/测试断言顺序)。
    pub fn systems(&self, stage: Stage) -> &[System] {
        match stage {
            Stage::Fixed => &self.fixed,
            Stage::Variable => &self.variable,
        }
    }

    /// 跑一个阶段:系统顺序执行 → 排空命令 → 翻转事件。
    pub fn run_stage(&mut self, stage: Stage, ctx: &mut SysCtx<'_>) {
        let systems = match stage {
            Stage::Fixed => &mut self.fixed,
            Stage::Variable => &mut self.variable,
        };
        for s in systems {
            (s.run)(ctx);
        }
        // 阶段边界不变式:结构性变更先生效,再换事件批次——阶段内系统
        // 看到的 World 与读侧事件都是稳定快照。
        ctx.commands.apply(&mut *ctx.world);
        ctx.events.rotate_all();
    }

    /// 完整一帧:固定步 `dt` 累加执行(≤ [`MAX_CATCHUP_STEPS`] 次),
    /// 然后一个可变帧阶段。返回本帧执行的固定步数(测试/断言用)。
    pub fn step(&mut self, dt: f32, ctx: &mut SysCtx<'_>) -> u32 {
        assert!(dt >= 0.0, "帧时长不得为负");
        self.acc += dt;
        let mut n = 0u32;
        while self.acc >= self.fixed_dt && n < MAX_CATCHUP_STEPS {
            self.acc -= self.fixed_dt;
            self.run_stage(Stage::Fixed, ctx);
            n += 1;
        }
        if n == MAX_CATCHUP_STEPS && self.acc >= self.fixed_dt {
            // 追不上就丢历史:保当前帧新鲜度,绝不补作业。
            self.acc = 0.0;
        }
        self.run_stage(Stage::Variable, ctx);
        n
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}
