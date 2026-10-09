//! 延迟命令队列(Godot CallQueue 模式):系统迭代表视图期间 World 不能动
//! 生命周期(`despawn`/`insert` 需独占),结构性变更一律排队,阶段末由
//! 调度器统一 [`CommandQueue::apply`]。按提交顺序执行。

use crate::{Component, Entity, World};

type Cmd = Box<dyn FnOnce(&mut World)>;

pub struct CommandQueue {
    q: Vec<Cmd>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self { q: Vec::new() }
    }

    /// 任意延迟操作。闭包在阶段末拿到 `&mut World` 独占执行。
    pub fn push(&mut self, f: impl FnOnce(&mut World) + 'static) {
        self.q.push(Box::new(f));
    }

    /// 延迟建实体并以它装配组件:`spawn_with(|w, e| w.insert(e, Pos(..)))`。
    pub fn spawn_with(&mut self, f: impl FnOnce(&mut World, Entity) + 'static) {
        self.push(move |w| {
            let e = w.spawn();
            f(w, e);
        });
    }

    pub fn despawn(&mut self, e: Entity) {
        self.push(move |w| {
            w.despawn(e);
        });
    }

    /// 延迟插组件。**死实体上插会 panic**(与直接 `World::insert` 同语义):
    /// 同一批里先 despawn 再 insert 同一实体是逻辑错误。
    pub fn insert<T: Component>(&mut self, e: Entity, v: T) {
        self.push(move |w| w.insert(e, v));
    }

    pub fn remove<T: Component>(&mut self, e: Entity) {
        self.push(move |w| {
            let _ = w.remove::<T>(e);
        });
    }

    /// 排空执行(GameRuntime 之外的独立用法也可手动驱动)。
    pub fn apply(&mut self, world: &mut World) {
        for f in self.q.drain(..) {
            f(world);
        }
    }

    pub fn len(&self) -> usize {
        self.q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}
