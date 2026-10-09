//! 类型化单例资源表(Godot autoload 的最小等价):一类型一实例,
//! 世界之外的全局状态(配置、玩家快照、计时器)住这里,不进组件表。

use std::any::{Any, TypeId};
use std::collections::HashMap;

pub struct Resources {
    map: HashMap<TypeId, Box<dyn Any>>,
}

impl Resources {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// 插入/替换单例,返回被替换的旧值(类型不符的旧值不可能存在)。
    pub fn insert<T: Any>(&mut self, v: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), Box::new(v))
            .and_then(|b| b.downcast::<T>().ok())
            .map(|b| *b)
    }

    /// 不存在则用 Default 补一份(已存在不动)。
    pub fn init_default<T: Any + Default>(&mut self) -> &mut T {
        self.map
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()));
        self.get_mut::<T>().expect("刚插入必存在")
    }

    pub fn get<T: Any>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
    }

    pub fn get_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.downcast_mut::<T>())
    }

    /// 移除并返回单例(无则 None)。
    pub fn take<T: Any>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast::<T>().ok())
            .map(|b| *b)
    }
}

impl Default for Resources {
    fn default() -> Self {
        Self::new()
    }
}
