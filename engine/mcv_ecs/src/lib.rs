//! 极简 ECS(引擎层原语):实体生命周期 + sparse-set 组件存储 + 分表借用查询。
//!
//! 与 Bevy 的取舍(有意为之,不是没做到):
//! - **sparse set 而非 archetype 分表**:实体数百量级下 archetype 的迭代
//!   顺序优势不可感知,而组件集每次变动付一次迁移代价;sparse set 插入/
//!   删除 O(1) 且不迁移,迭代走密集索引同样紧凑。
//! - **系统 = 普通函数**,拿 World 的 `read`/`write` 表视图自己写循环——
//!   不做调度器、插件 trait、事件总线;出现真实消费方再加,接口形状已兼容。
//! - **零 `unsafe`**:组件表的独占借用靠每表 `RefCell` 运行时互斥——同一张
//!   表同时 `write` 两次直接 panic,语义等同 Bevy 的 query 冲突检查(区别
//!   只是编译期→运行期,系统本就单线程顺序执行)。
//!
//! 两级访问语义(性能与安全边界分明):
//! - **散点访问**走 [`World::get_ref`](带 `Ref` 守卫)或 [`World::write`]
//!   表视图:句柄经 generation 校验,悬挂句柄返回 `None`,绝不会读到
//!   复用槽位的新主人;
//! - **系统迭代**走 [`SparseSet::iter`]/[`SparseSet::for_each`]:密集表内部
//!   自洽(despawn 同步清表,表内元素必为活实体),句柄直接可用。
//!
//! ```
//! use mcv_ecs::World;
//! #[derive(Clone, Copy)]
//! struct Pos(f32);
//! #[derive(Clone, Copy)]
//! struct Hp(i32);
//!
//! let mut w = World::new();
//! let e = w.spawn();
//! w.insert(e, Pos(1.0));
//! w.insert(e, Hp(20));
//! let dead = w.spawn();
//! w.despawn(dead);
//! // 系统:所有 Pos 实体前进一步,读可选 Hp(悬挂句柄迭代内不会出现)。
//! {
//!     let (mut pos, hp) = (w.write::<Pos>(), w.read::<Hp>());
//!     pos.for_each(|e, p| {
//!         p.0 += hp.get(e).map_or(1.0, |_| 2.0);
//!     });
//! }
//! assert_eq!(w.get_ref::<Pos>(e).unwrap().0, 3.0);
//! assert!(w.get_ref::<Pos>(dead).is_none());
//! ```

use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;

/// 实体句柄:槽位下标 + 世代号。实体销毁后槽位可复用,但世代号 +1,
/// 旧句柄经 [`World::is_alive`]/[`World::get_ref`] 校验即失效。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Entity {
    idx: u32,
    // Rust 2024 起 `gen` 是保留关键字,字段/方法名避开(Bevy 同名迁移)。
    generation: u32,
}

impl Entity {
    /// 槽位下标(渲染/外部索引用途;校验存活仍须走 [`World::is_alive`])。
    #[inline]
    pub fn idx(&self) -> u32 {
        self.idx
    }
    /// 世代号。
    #[inline]
    pub fn generation(&self) -> u32 {
        self.generation
    }
}

/// 组件约束:`'static` 即可。泛型存储靠 TypeId 分表。
pub trait Component: 'static {}
impl<T: 'static> Component for T {}

/// 类型擦除的组件表视图:供 [`World::despawn`] 级联清理与统计。
trait Store: Any {
    /// 清除槽位上的组件(不存在则 no-op)。
    fn remove_at(&mut self, idx: u32);
    fn count(&self) -> usize;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// sparse-set 组件存储:`sparse[idx]` 按实体槽位直接寻址(插入/删除 O(1)),
/// `dense` 记录持有本组件的实体句柄(迭代紧凑,不受稀疏空洞影响)。
pub struct SparseSet<T> {
    sparse: Vec<Option<T>>,
    dense: Vec<Entity>,
}

impl<T: 'static> SparseSet<T> {
    fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
        }
    }
}

impl<T: 'static> Store for SparseSet<T> {
    fn remove_at(&mut self, idx: u32) {
        // edition 2024 let-chain。
        if let Some(slot) = self.sparse.get_mut(idx as usize)
            && slot.take().is_some()
        {
            let d = self
                .dense
                .iter()
                .position(|e| e.idx == idx)
                .expect("sparse/dense 不变式破坏:dense 含该槽位");
            self.dense.swap_remove(d);
        }
    }
    fn count(&self) -> usize {
        self.dense.len()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl<T: Component> SparseSet<T> {
    /// 按实体句柄取组件。系统迭代内使用(表内自洽,元素必为活实体);
    /// 散点访问优先 [`World::get_ref`](额外带代校验)。
    #[inline]
    pub fn get(&self, e: Entity) -> Option<&T> {
        self.sparse.get(e.idx as usize)?.as_ref()
    }

    /// 可变版本,约束同 [`SparseSet::get`]。经 [`World::write`] 拿到本视图后,
    /// 这就是"按句柄散点写"的唯一正道(RefCell 守卫全程持有,无悬垂风险)。
    #[inline]
    pub fn get_mut(&mut self, e: Entity) -> Option<&mut T> {
        self.sparse.get_mut(e.idx as usize)?.as_mut()
    }

    /// 密集只读迭代:只遍历持有本组件的实体,无空洞。
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.dense.iter().map(|&e| {
            (
                e,
                self.sparse[e.idx as usize]
                    .as_ref()
                    .expect("sparse/dense 不变式破坏"),
            )
        })
    }

    /// 密集可变迭代(回调式):字段分离借用,零 `unsafe`、零分配。
    /// 回调内可对**其他**表的视图随意读写;对本表再 `get_mut` 需换成
    /// 通过闭包参数 `v`(避免双重借用)。
    pub fn for_each<F: FnMut(Entity, &mut T)>(&mut self, mut f: F) {
        let Self { sparse, dense } = self;
        for &e in dense.iter() {
            if let Some(v) = sparse[e.idx as usize].as_mut() {
                f(e, v);
            }
        }
    }

    /// 持有本组件的实体数。
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    fn insert_at(&mut self, e: Entity, v: T) {
        let i = e.idx as usize;
        // 不能用 Vec::resize(要求 T: Clone,组件不保证)。
        while self.sparse.len() <= i {
            self.sparse.push(None);
        }
        if self.sparse[i].is_none() {
            self.dense.push(e);
        }
        self.sparse[i] = Some(v);
    }

    fn remove_take(&mut self, e: Entity) -> Option<T> {
        let slot = self.sparse.get_mut(e.idx as usize)?;
        let v = slot.take()?;
        let d = self
            .dense
            .iter()
            .position(|x| x.idx == e.idx)
            .expect("sparse/dense 不变式破坏");
        self.dense.swap_remove(d);
        Some(v)
    }
}

/// 组件世界:实体生命周期 + 按类型分表存储。单线程用途(RefCell 非 Sync)。
pub struct World {
    /// 每槽位当前世代(despawn 时 +1,作废全部旧句柄)。
    gens: Vec<u32>,
    /// 槽位占用位。
    occupied: Vec<bool>,
    /// 活实体密集表(句柄含世代,直接切片暴露)。
    alive: Vec<Entity>,
    /// 组件表。值直接是 RefCell(不经 Rc 中转——视图守卫的生命周期必须
    /// 挂在 `&self` 上才能从 read/write 返回,Rc deref 会断在局部作用域)。
    stores: HashMap<TypeId, RefCell<Box<dyn Store>>>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            gens: Vec::new(),
            occupied: Vec::new(),
            alive: Vec::new(),
            stores: HashMap::new(),
        }
    }

    /// 新建实体(优先复用空闲槽位)。
    pub fn spawn(&mut self) -> Entity {
        let idx = match self.occupied.iter().position(|&o| !o) {
            Some(i) => i,
            None => {
                self.occupied.push(false);
                self.gens.push(0);
                self.occupied.len() - 1
            }
        };
        self.occupied[idx] = true;
        let e = Entity {
            idx: idx as u32,
            generation: self.gens[idx],
        };
        self.alive.push(e);
        e
    }

    /// 销毁实体:移出活表、世代 +1(旧句柄作废)、级联清各组件表
    /// (组件在此 Drop)。返回是否真的销毁了(死句柄 false)。
    pub fn despawn(&mut self, e: Entity) -> bool {
        if !self.is_alive(e) {
            return false;
        }
        self.gens[e.idx as usize] += 1;
        self.occupied[e.idx as usize] = false;
        let a = self
            .alive
            .iter()
            .position(|x| *x == e)
            .expect("alive 表破坏");
        self.alive.swap_remove(a);
        for store in self.stores.values_mut() {
            store.borrow_mut().remove_at(e.idx);
        }
        true
    }

    /// 句柄是否指向活实体(代校验)。
    #[inline]
    pub fn is_alive(&self, e: Entity) -> bool {
        self.occupied
            .get(e.idx as usize)
            .is_some_and(|&o| o && self.gens[e.idx as usize] == e.generation)
    }

    /// 全部活实体(含世代)。
    pub fn entities(&self) -> &[Entity] {
        &self.alive
    }

    pub fn len(&self) -> usize {
        self.alive.len()
    }

    pub fn is_empty(&self) -> bool {
        self.alive.is_empty()
    }

    /// 插入组件(已有则替换,旧值 Drop)。
    pub fn insert<T: Component>(&mut self, e: Entity, v: T) {
        assert!(self.is_alive(e), "向死/无效实体插入组件");
        self.ensure_store::<T>();
        let store = &self.stores[&TypeId::of::<T>()];
        store
            .borrow_mut()
            .as_any_mut()
            .downcast_mut::<SparseSet<T>>()
            .expect("TypeId→表类型映射被破坏")
            .insert_at(e, v);
    }

    /// 移除并返回组件(代校验;无组件/死句柄 None)。
    pub fn remove<T: Component>(&mut self, e: Entity) -> Option<T> {
        if !self.is_alive(e) {
            return None;
        }
        self.stores
            .get(&TypeId::of::<T>())?
            .borrow_mut()
            .as_any_mut()
            .downcast_mut::<SparseSet<T>>()?
            .remove_take(e)
    }

    /// 散点只读(代校验),返回 `Ref` 守卫。要求可变散点写请走
    /// [`World::write`] 表视图 + [`SparseSet::get_mut`]。
    pub fn get_ref<T: Component>(&self, e: Entity) -> Option<Ref<'_, T>> {
        if !self.is_alive(e) {
            return None;
        }
        let set = Ref::filter_map(self.stores.get(&TypeId::of::<T>())?.borrow(), |s| {
            s.as_any().downcast_ref::<SparseSet<T>>()
        })
        .ok()?;
        Ref::filter_map(set, |s| s.get(e)).ok()
    }

    /// 预注册组件表(可选)。`insert` 会自动注册;但若某类型可能**从未被
    /// 插入**就要被 `read`/`write`(如零怪时系统仍要取 `MobKind` 视图),
    /// 必须在建 World 后显式注册一次,否则 `read`/`write` panic。
    pub fn register<T: Component>(&mut self) {
        self.ensure_store::<T>();
    }

    /// 整表只读视图。同一张表的冲突借用(如两次 `write`)panic——运行时
    /// 互斥,等同 Bevy 的 query 冲突;不同表可自由并行取(借用都是 `&self`,
    /// `(w.write::<A>(), w.read::<B>())` 元组模式合法)。
    /// 视图持有期间禁止再调 `&mut self` 的 World 方法(despawn 级联清表
    /// 需要独占);系统函数请成对使用:先取全部所需表视图跑循环,结束后
    /// 再动 World 生命周期。未注册类型 panic(见 [`World::register`])。
    pub fn read<T: Component>(&self) -> Ref<'_, SparseSet<T>> {
        // Ref 不实现 Debug,expect 不可用;downcast 失败是内部不变式破坏。
        Ref::filter_map(self.table::<T>().borrow(), |s| {
            s.as_any().downcast_ref::<SparseSet<T>>()
        })
        .unwrap_or_else(|_| panic!("TypeId→表类型映射被破坏"))
    }

    /// 整表可变视图,约束同 [`World::read`]。
    pub fn write<T: Component>(&self) -> RefMut<'_, SparseSet<T>> {
        RefMut::filter_map(self.table::<T>().borrow_mut(), |s| {
            s.as_any_mut().downcast_mut::<SparseSet<T>>()
        })
        .unwrap_or_else(|_| panic!("TypeId→表类型映射被破坏"))
    }

    fn table<T: Component>(&self) -> &RefCell<Box<dyn Store>> {
        self.stores
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("组件类型未注册:先 register::<T>() 或 insert"))
    }

    /// 该组件类型的全表实体数(调试/测试)。
    pub fn component_count<T: Component>(&self) -> usize {
        self.stores
            .get(&TypeId::of::<T>())
            .map_or(0, |s| s.borrow().count())
    }

    fn ensure_store<T: Component>(&mut self) {
        self.stores
            .entry(TypeId::of::<T>())
            .or_insert_with(|| RefCell::new(Box::new(SparseSet::<T>::new()) as Box<dyn Store>));
    }
}
