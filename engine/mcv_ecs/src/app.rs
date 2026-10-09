//! 装配壳 [`App`] 与 [`Plugin`](crate::Plugin):引擎给能力、游戏给规则,
//! App 是二者的组装点(World/Resources/Events/Schedule 的拥有者)。

use std::any::Any;

use crate::World;
use crate::commands::CommandQueue;
use crate::events::EventBus;
use crate::plugin::Plugin;
use crate::resources::Resources;
use crate::schedule::{Schedule, Stage, SysCtx};

/// ECS 应用:全部状态的家。宿主可以整帧驱动 [`App::update`],也可以
/// 拆出字段自建 [`SysCtx`] 单步驱动(宿主自带累加器时)。
pub struct App {
    pub world: World,
    pub resources: Resources,
    pub events: EventBus,
    pub commands: CommandQueue,
    pub schedule: Schedule,
}

impl App {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            resources: Resources::new(),
            events: EventBus::new(),
            commands: CommandQueue::new(),
            schedule: Schedule::new(),
        }
    }

    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        plugin.build(self);
        self
    }

    /// 注册系统(转发 schedule;链式装配顺手)。
    pub fn add_system<F>(&mut self, stage: Stage, name: &'static str, f: F) -> &mut Self
    where
        F: for<'a> FnMut(&mut SysCtx<'a>) + 'static,
    {
        self.schedule.add(stage, name, f);
        self
    }

    /// 单例资源:不存在则 Default 补一份。
    pub fn init_resource<T: Any + Default>(&mut self) -> &mut Self {
        self.resources.init_default::<T>();
        self
    }

    /// 整帧驱动(固定步累加 + 可变帧)。宿主自带累加器时拆字段自建
    /// [`SysCtx`] 单步驱动亦可(字段皆 pub,disjoint 借用互不冲突)。
    pub fn update(&mut self, dt: f32) -> u32 {
        let Self {
            world,
            resources,
            events,
            commands,
            schedule,
        } = self;
        let mut ctx = SysCtx {
            world,
            resources,
            events,
            commands,
        };
        schedule.step(dt, &mut ctx)
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
