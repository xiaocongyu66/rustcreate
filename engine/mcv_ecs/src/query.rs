//! 自研 join 查询器（Bevy `WorldQuery`/`QueryState` 语义的稀疏表直读映射，
//! 任务板 #222 批 A）：多组件 join 的取数面——以**实体 id 集合为对齐键**，
//! 遍历最稀疏的一张表、逐实体查其余表，缺任一组件的实体直接跳过
//! （Bevy `QueryState::iter` 的 archetype 对齐语义在无 archetype 模型下的
//! 等价形态）。
//!
//! 与手写循环（`phys.for_each` + 他表 `get`/`get_mut` 缺则跳）的语义对齐点：
//! - **miss 跳过**：join 在构造期一次完成——[`Query2::ids`] 即交集实体序
//!   （锚表 dense 序），`iter`/`for_each*` 只看交集，循环体内不再判缺失；
//! - **访问冲突**：Bevy 靠编译面 read/write 位集拒绝同表 `&mut` 冲突；本仓
//!   照 lib.rs 既定取舍搬到**运行期**——同型 fetch 至少一侧可变即
//!   `debug_assert!` 拒绝（双 &mut/读写互斥），release 由每表 `RefCell`
//!   借用检查兜底（构造期必 panic，见下方 `should_panic` 测试）；
//! - **扩展位**：`With`/`Without`/`Changed` 组合过滤与 Builder 留待批次 B；
//!   本批只做双/三元 join + 互斥断言。不用宏、不用 `unsafe`、零外部依赖。
//!
//! ```
//! use mcv_ecs::{Query2, World};
//! #[derive(Clone, Copy)]
//! struct Pos(f32);
//! #[derive(Clone, Copy)]
//! struct Hp(i32);
//!
//! let mut w = World::new();
//! let a = w.spawn();
//! w.insert(a, Pos(1.0));
//! w.insert(a, Hp(20));
//! let b = w.spawn();
//! w.insert(b, Pos(2.0)); // 缺 Hp——join 时被跳过
//!
//! // 各表守卫经 World::read()/write() 取得（Ref/RefMut 自动归一为 Access）。
//! let q = Query2::new(w.read::<Pos>(), w.read::<Hp>());
//! let got: Vec<(u32, f32, i32)> = q.iter().map(|(e, p, h)| (e.idx(), p.0, h.0)).collect();
//! assert_eq!(got, vec![(a.idx(), 1.0, 20)]);
//! ```

use std::any::TypeId;
use std::cell::{Ref, RefMut};

use crate::{Component, Entity, SparseSet};

/// 查询内单表访问守卫：读（`World::read` 的 [`Ref`]）或写（`World::write`
/// 的 [`RefMut`]）二值形态。经 [`From`] 自动归一——[`Query2::new`]/[`Query3::new`]
/// 因此可任意组合读写（如 `Query2::new(w.write::<A>(), w.read::<B>())`）。
///
/// 同表冲突（双写、读写同表）的运行期互斥由每表 `RefCell` 借用检查承接：
/// 两个写守卫在构造期即 panic，语义等同 Bevy 的 query 冲突检查。
pub enum Access<'w, T: Component> {
    /// 共享只读（`World::read`）。
    Read(Ref<'w, SparseSet<T>>),
    /// 独占可变（`World::write`）。
    Write(RefMut<'w, SparseSet<T>>),
}

impl<'w, T: Component> From<Ref<'w, SparseSet<T>>> for Access<'w, T> {
    fn from(r: Ref<'w, SparseSet<T>>) -> Self {
        Self::Read(r)
    }
}

impl<'w, T: Component> From<RefMut<'w, SparseSet<T>>> for Access<'w, T> {
    fn from(r: RefMut<'w, SparseSet<T>>) -> Self {
        Self::Write(r)
    }
}

impl<T: Component> Access<'_, T> {
    /// 表内持有本组件的实体数（dense 长度，O(1)）——锚表择稀疏用。
    fn len(&self) -> usize {
        match self {
            Access::Read(r) => r.len(),
            Access::Write(w) => w.len(),
        }
    }

    /// 实体是否持本组件（sparse 槽位非空，O(1)）——join 对齐查询用。
    fn has(&self, e: Entity) -> bool {
        match self {
            Access::Read(r) => r.get(e).is_some(),
            Access::Write(w) => w.get(e).is_some(),
        }
    }

    /// 共享取组件（读/写守卫都可借出 `&T`）。
    fn get(&self, e: Entity) -> Option<&T> {
        match self {
            Access::Read(r) => r.get(e),
            Access::Write(w) => w.get(e),
        }
    }

    /// 独占取组件。读形态守卫无此能力（程序员错——应经 `World::write`
    /// 构造），直接 panic 而非静默 `None`。
    fn get_mut(&mut self, e: Entity) -> Option<&mut T> {
        match self {
            Access::Write(w) => w.get_mut(e),
            Access::Read(_) => panic!("Access::get_mut 需要写形态守卫（经 World::write 构造）"),
        }
    }

    /// 表内实体序（dense 序——join 锚遍历与手写 `for_each` 循环同规）。
    /// 读/写两形态先统一到 `&SparseSet`（deref 强转），两臂共用同一迭代类型。
    fn iter_entities<'a>(&'a self) -> impl Iterator<Item = Entity> + use<'a, T> {
        let set: &SparseSet<T> = match self {
            Access::Read(r) => r,
            Access::Write(w) => w,
        };
        set.iter().map(|(e, _)| e)
    }

    /// 是否为独占守卫——同型双可变互斥断言的判定位。
    fn is_write(&self) -> bool {
        matches!(self, Access::Write(_))
    }
}

/// 同型 fetch 联合互斥检查：Bevy 里 `Query2<&mut A, &A>`（同表读写/双写）是
/// 编译期错误；本仓运行期把关——同型且任一侧可写即违约。双读同型合法
/// （两个 `&T` 共享，与 Bevy 一致）。
fn assert_disjoint(ta: TypeId, wa: bool, tb: TypeId, wb: bool, ctx: &str) {
    debug_assert!(
        ta != tb || !(wa || wb),
        "{ctx}:同型 fetch 且至少一侧可写——双 &mut/读写互斥被破坏\
         （Bevy query 冲突的运行期等价；release 由 RefCell 借用检查兜底必 panic）"
    );
}

/// 双 fetch 查询（Bevy `Query2<&A, &B>`）：构造期完成 join——锚=较稀疏表
/// （平局取 A），逐实体查他表，缺者落选。守卫经 [`Access`] 归一，读写任意
/// 组合；循环体内不再判缺失（miss 跳过在 join 时收口）。
pub struct Query2<'w, A: Component, B: Component> {
    a: Access<'w, A>,
    b: Access<'w, B>,
    /// join 对齐后的交集实体序（锚表 dense 序）——构造一次，逐帧循环复用。
    seq: Vec<Entity>,
}

impl<'w, A: Component, B: Component> Query2<'w, A, B> {
    /// 双 fetch 查询。各表守卫来自 `World::read::<T>()`（转入
    /// [`Access::Read`]）或 `World::write::<T>()`（转入 [`Access::Write`]）。
    ///
    /// # Panics
    /// 同型可变冲突（`A == B` 且任一侧 Write）规则见 [`assert_disjoint`]：
    /// debug 先炸断言给契约文案，release 由 `RefCell` 借用检查兜底必 panic。
    pub fn new(a: impl Into<Access<'w, A>>, b: impl Into<Access<'w, B>>) -> Self {
        let (a, b) = (a.into(), b.into());
        assert_disjoint(
            TypeId::of::<A>(),
            a.is_write(),
            TypeId::of::<B>(),
            b.is_write(),
            "Query2",
        );
        // 锚=较稀疏表（dense 短者，平局取 A）。
        let seq = if a.len() <= b.len() {
            a.iter_entities().filter(|e| b.has(*e)).collect()
        } else {
            b.iter_entities().filter(|e| a.has(*e)).collect()
        };
        Self { a, b, seq }
    }

    /// 交集实体序（锚表 dense 序；swap_remove 语义下非按 idx 排序，
    /// 与手写 `for_each` 循环同规）。
    pub fn ids(&self) -> &[Entity] {
        &self.seq
    }

    /// 交集实体数。
    pub fn len(&self) -> usize {
        self.seq.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seq.is_empty()
    }

    /// 共享取一对组件（散点访问；缺任一 `None`）。
    pub fn get(&self, e: Entity) -> Option<(&A, &B)> {
        Some((self.a.get(e)?, self.b.get(e)?))
    }

    /// 交集只读迭代：`(Entity, &A, &B)`——缺任一组件的实体已被 join 跳过。
    /// `use` 精确捕获（edition 2024 RPIT 全量捕获会把 'w 也圈进来，
    /// 屏蔽调用侧在迭代器存活期外继续用 World 的正常路径）。
    pub fn iter<'s>(&'s self) -> impl Iterator<Item = (Entity, &'s A, &'s B)> + use<'s, A, B> {
        self.seq
            .iter()
            .copied()
            .filter_map(move |e| self.get(e).map(|(va, vb)| (e, va, vb)))
    }

    /// 双可变散点取（两笔 `&mut` 分属不同表——互斥由构造期
    /// [`assert_disjoint`] 把关）。任一表为读形态守卫时 panic（见
    /// [`Access::get_mut`]）。
    pub fn get_mut(&mut self, e: Entity) -> Option<(&mut A, &mut B)> {
        let va = self.a.get_mut(e)?;
        let vb = self.b.get_mut(e)?;
        Some((va, vb))
    }

    /// 闭包式双可变步进：交集实体逐个回调 `(Entity, &mut A, &mut B)`。
    /// 回调内可对查询外的**其他**表视图随意读写；结构性变更走
    /// [`crate::CommandQueue`]（帧末生效，不破坏本次迭代）。
    pub fn for_each_mut<F: FnMut(Entity, &mut A, &mut B)>(&mut self, mut f: F) {
        let Query2 { a, b, seq } = self;
        for &e in seq.iter() {
            // 构造后表结构不变（变更走延迟命令），双 Some 恒成立——
            // 仍按缺口跳过，防御未来维护。
            if let (Some(va), Some(vb)) = (a.get_mut(e), b.get_mut(e)) {
                f(e, va, vb);
            }
        }
    }

    /// 一写一读步进：`FnMut(Entity, &mut A, &B)`——第二表是共享快照
    /// （读/写守卫都可借出 `&B`）。Bevy `Query2<&mut A, &B>` 形态的直译。
    pub fn for_each_1w1r<F: FnMut(Entity, &mut A, &B)>(&mut self, mut f: F) {
        let Query2 { a, b, seq } = self;
        for &e in seq.iter() {
            if let Some(va) = a.get_mut(e) {
                let Some(vb) = b.get(e) else { continue };
                f(e, va, vb);
            }
        }
    }
}

/// 三 fetch 查询（Bevy `Query3<&A, &B, &C>`）：join 语义同 [`Query2`]，
/// 锚=三表中最稀疏者（平局取靠前者）。
pub struct Query3<'w, A: Component, B: Component, C: Component> {
    a: Access<'w, A>,
    b: Access<'w, B>,
    c: Access<'w, C>,
    seq: Vec<Entity>,
}

impl<'w, A: Component, B: Component, C: Component> Query3<'w, A, B, C> {
    /// 三 fetch 查询，读写任意组合（各表守卫经 [`Access`] 归一）。
    /// 典型形态：`Query3::new(w.write::<A>(), w.write::<B>(), w.read::<C>())`
    /// ——两写一读（热点系统高频组合）。
    ///
    /// # Panics
    /// 同型可变冲突（任两表同型且至少一侧 Write）规则见 [`assert_disjoint`]。
    pub fn new(
        a: impl Into<Access<'w, A>>,
        b: impl Into<Access<'w, B>>,
        c: impl Into<Access<'w, C>>,
    ) -> Self {
        let (a, b, c) = (a.into(), b.into(), c.into());
        assert_disjoint(
            TypeId::of::<A>(),
            a.is_write(),
            TypeId::of::<B>(),
            b.is_write(),
            "Query3(A,B)",
        );
        assert_disjoint(
            TypeId::of::<A>(),
            a.is_write(),
            TypeId::of::<C>(),
            c.is_write(),
            "Query3(A,C)",
        );
        assert_disjoint(
            TypeId::of::<B>(),
            b.is_write(),
            TypeId::of::<C>(),
            c.is_write(),
            "Query3(B,C)",
        );
        // 锚=最稀疏表（平局取靠前者），逐实体验三表齐活。三分支各自以
        // 具体锚表迭代（守卫类型互异，无法统一锚变量——见 [`Access`]）。
        let mut seq = Vec::with_capacity(a.len().min(b.len()).min(c.len()));
        if a.len() <= b.len() && a.len() <= c.len() {
            seq.extend(a.iter_entities().filter(|e| b.has(*e) && c.has(*e)));
        } else if b.len() <= c.len() {
            seq.extend(b.iter_entities().filter(|e| a.has(*e) && c.has(*e)));
        } else {
            seq.extend(c.iter_entities().filter(|e| a.has(*e) && b.has(*e)));
        }
        Self { a, b, c, seq }
    }

    /// 交集实体序。
    pub fn ids(&self) -> &[Entity] {
        &self.seq
    }

    /// 交集实体数。
    pub fn len(&self) -> usize {
        self.seq.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seq.is_empty()
    }

    /// 共享取三元组（散点访问；缺任一 `None`）。
    pub fn get(&self, e: Entity) -> Option<(&A, &B, &C)> {
        Some((self.a.get(e)?, self.b.get(e)?, self.c.get(e)?))
    }

    /// 交集只读迭代：`(Entity, &A, &B, &C)`。捕获界同 [`Query2::iter`]。
    pub fn iter<'s>(&'s self) -> impl Iterator<Item = (Entity, &'s A, &'s B, &'s C)> + use<'s, A, B, C>
    {
        self.seq
            .iter()
            .copied()
            .filter_map(move |e| self.get(e).map(|(va, vb, vc)| (e, va, vb, vc)))
    }

    /// 三可变散点取（三表互异由构造期断言把关；任一表为读守卫则 panic）。
    pub fn get_mut(&mut self, e: Entity) -> Option<(&mut A, &mut B, &mut C)> {
        let va = self.a.get_mut(e)?;
        let vb = self.b.get_mut(e)?;
        let vc = self.c.get_mut(e)?;
        Some((va, vb, vc))
    }

    /// 闭包式三可变步进：`FnMut(Entity, &mut A, &mut B, &mut C)`。
    pub fn for_each_3w<F: FnMut(Entity, &mut A, &mut B, &mut C)>(&mut self, mut f: F) {
        let Query3 { a, b, c, seq } = self;
        for &e in seq.iter() {
            if let (Some(va), Some(vb), Some(vc)) = (a.get_mut(e), b.get_mut(e), c.get_mut(e)) {
                f(e, va, vb, vc);
            }
        }
    }

    /// 两写一读步进：`FnMut(Entity, &mut A, &mut B, &C)`——第三表是共享
    /// 快照（读/写守卫都可借出 `&C`）。热点系统「双写表×查定义表」的
    /// 标准入口（如被动怪：写 PhysBody×WanderState，读 MobKind）。
    pub fn for_each_2w1r<F: FnMut(Entity, &mut A, &mut B, &C)>(&mut self, mut f: F) {
        let Query3 { a, b, c, seq } = self;
        for &e in seq.iter() {
            if let (Some(va), Some(vb)) = (a.get_mut(e), b.get_mut(e)) {
                let Some(vc) = c.get(e) else { continue };
                f(e, va, vb, vc);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 等价测试：Query2/Query3 输出与手写 join 循环（同锚表规则）逐实体全等
//（实体序 + 值）；miss 跳过；双 &mut/读写互斥的运行期拒绝。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entity, World};

    #[derive(Clone, Copy, PartialEq, Debug)]
    struct Pos(f32);
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct Vel(f32);
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct Kind(u8);

    /// 造一个含穿插实体（部分缺组件、含 swap_remove 历史）的 world，
    /// 使各表 dense 序彼此错开——锚表规则在全序下可考。
    /// 成员（es[2] 在记录后会被 despawn）：0: P+V+K  1: 仅 P  2: V+K
    ///（已亡）  3: P+K  4: P+V  5: 空壳。
    fn woven_world() -> (World, Vec<Entity>) {
        let mut w = World::new();
        w.register::<Kind>(); // 显式注册面（未 insert 的表也要可取守卫）
        let es: Vec<_> = (0..6).map(|_| w.spawn()).collect();
        w.insert(es[0], Pos(0.0));
        w.insert(es[0], Vel(0.0));
        w.insert(es[0], Kind(0));
        w.insert(es[1], Pos(1.0));
        w.insert(es[2], Vel(2.0));
        w.insert(es[2], Kind(0));
        w.insert(es[3], Pos(3.0));
        w.insert(es[3], Kind(1));
        w.insert(es[4], Pos(4.0));
        w.insert(es[4], Vel(4.0));
        // 制造 swap_remove：销毁 2 号，V 表 dense [0,2,4] → [0,4]，
        // 与 P 表 [0,1,3,4] 错开——join 锚将落在 Vel 表。
        assert!(w.despawn(es[2]));
        (w, es)
    }

    /// 手写双表 join 基准：与查询同一锚表规则（较稀疏 dense 表、平局取 A）。
    fn hand_join2(w: &World) -> Vec<(Entity, f32, f32)> {
        let (pos, vel) = (w.read::<Pos>(), w.read::<Vel>());
        let mut out = Vec::new();
        if pos.len() <= vel.len() {
            for (e, p) in pos.iter() {
                if let Some(v) = vel.get(e) {
                    out.push((e, p.0, v.0));
                }
            }
        } else {
            for (e, v) in vel.iter() {
                if let Some(p) = pos.get(e) {
                    out.push((e, p.0, v.0));
                }
            }
        }
        out
    }

    #[test]
    fn query2_equals_hand_join() {
        let (w, es) = woven_world();
        let q = Query2::<Pos, Vel>::new(w.read::<Pos>(), w.read::<Vel>());
        let expected: Vec<_> = hand_join2(&w);
        let got: Vec<(Entity, f32, f32)> = q.iter().map(|(e, p, v)| (e, p.0, v.0)).collect();
        assert_eq!(got, expected, "Query2 输出必须与手写 join 循环全等（序+值）");
        assert_eq!(q.len(), 2); // 0/4 持 P+V（2 已亡）
        assert_eq!(q.ids().len(), q.len());
        // 锚=Vel 表（dense 更短）：交集序 = [es0, es4]。
        assert_eq!(q.ids(), &[es[0], es[4]]);
    }

    #[test]
    fn query2_miss_skipped_and_get_scatter() {
        let (w, es) = woven_world();
        let q = Query2::<Pos, Vel>::new(w.read::<Pos>(), w.read::<Vel>());
        // 散点 join：仅 Pos 的实体缺 Vel → None。
        assert!(q.get(es[0]).is_some());
        assert!(q.get(es[1]).is_none(), "仅 Pos 的实体必须被 join 拒之门外");
        assert!(q.get(es[3]).is_none());
    }

    #[test]
    fn query2_for_each_mut_writes_both_tables() {
        let (w, es) = woven_world();
        let mut q = Query2::<Pos, Vel>::new(w.write::<Pos>(), w.write::<Vel>());
        let mut n = 0;
        q.for_each_mut(|e, p, v| {
            p.0 += 100.0;
            v.0 += if e.idx() == es[0].idx() { 1.0 } else { 2.0 };
            n += 1;
        });
        assert_eq!(n, 2);
        drop(q);
        let (pos, vel) = (w.read::<Pos>(), w.read::<Vel>());
        assert_eq!(pos.get(es[0]).unwrap().0, 100.0);
        assert_eq!(vel.get(es[0]).unwrap().0, 1.0);
        assert_eq!(pos.get(es[4]).unwrap().0, 104.0);
        assert_eq!(vel.get(es[4]).unwrap().0, 6.0);
        // 仅 Pos 表实体（es[1]/es[3]）不得被触碰。
        assert_eq!(pos.get(es[1]).unwrap().0, 1.0);
        assert_eq!(pos.get(es[3]).unwrap().0, 3.0);
    }

    #[test]
    fn query2_get_mut_pairs_disjoint_write() {
        let (w, es) = woven_world();
        let mut q = Query2::<Pos, Vel>::new(w.write::<Pos>(), w.write::<Vel>());
        let (p, v) = q.get_mut(es[4]).unwrap();
        p.0 = 7.0;
        v.0 = 9.0;
        drop(q);
        assert_eq!(w.read::<Pos>().get(es[4]).unwrap().0, 7.0);
        assert_eq!(w.read::<Vel>().get(es[4]).unwrap().0, 9.0);
    }

    #[test]
    fn query2_mixed_read_write_guard() {
        let (w, es) = woven_world();
        // 写 Pos、读 Vel 的混合形态（Bevy `Query2<&mut Pos, &Vel>`）。
        let mut q = Query2::new(w.write::<Pos>(), w.read::<Vel>());
        q.for_each_1w1r(|_e, p, v| p.0 = v.0 + 10.0); // Vel 对齐写入 Pos
        drop(q);
        let (pos, vel) = (w.read::<Pos>(), w.read::<Vel>());
        // 仅持 Vel 的实体（es0/es4）被对齐加 10；缺组件成员不受扰。
        assert_eq!(pos.get(es[0]).unwrap().0, 10.0);
        assert_eq!(pos.get(es[4]).unwrap().0, 14.0);
        assert_eq!(pos.get(es[1]).unwrap().0, 1.0);
        for (e, v) in vel.iter() {
            assert_eq!(pos.get(e).unwrap().0, v.0 + 10.0);
        }
    }

    #[test]
    #[should_panic(expected = "Access::get_mut 需要写形态守卫")]
    fn query2_get_mut_on_read_guard_panics() {
        let (w, es) = woven_world();
        let mut q = Query2::new(w.read::<Pos>(), w.read::<Vel>());
        let _ = q.get_mut(es[0]); // 读守卫无从可变——必须显式炸出而非静默
    }

    #[test]
    #[should_panic(expected = "already")]
    fn query2_same_type_dual_write_rejected() {
        // 同型双写：双 &mut 互斥的运行期拒绝。debug 下先经断言给契约文案，
        // release 由 RefCell 借用检查兜底——两种剖面都是必 panic。
        let mut w = World::new();
        let e = w.spawn();
        w.insert(e, Pos(1.0));
        let _q = Query2::<Pos, Pos>::new(w.write::<Pos>(), w.write::<Pos>());
    }

    #[test]
    #[should_panic(expected = "already")]
    fn query2_same_type_read_write_rejected() {
        // 同型读写（Bevy Query2<&mut A, &A> 冲突）同样运行期拒。
        let mut w = World::new();
        let e = w.spawn();
        w.insert(e, Pos(1.0));
        let _q = Query2::<Pos, Pos>::new(w.write::<Pos>(), w.read::<Pos>());
    }

    #[test]
    fn query2_same_type_both_read_is_fine() {
        let (w, _es) = woven_world();
        // 两个共享守卫同表：与 Bevy Query2<&A, &A> 一致，合法。
        let q = Query2::<Pos, Pos>::new(w.read::<Pos>(), w.read::<Pos>());
        assert!(!q.is_empty());
    }

    /// 手写三表 join 基准（锚=三表最稀疏、平局取靠前者）。
    fn hand_join3(w: &World) -> Vec<(Entity, f32, f32, u8)> {
        let (pos, vel, kind) = (w.read::<Pos>(), w.read::<Vel>(), w.read::<Kind>());
        let mut out = Vec::new();
        let alen = pos.len();
        let blen = vel.len();
        let clen = kind.len();
        if alen <= blen && alen <= clen {
            for (e, p) in pos.iter() {
                if let (Some(v), Some(k)) = (vel.get(e), kind.get(e)) {
                    out.push((e, p.0, v.0, k.0));
                }
            }
        } else if blen <= clen {
            for (e, v) in vel.iter() {
                if let (Some(p), Some(k)) = (pos.get(e), kind.get(e)) {
                    out.push((e, p.0, v.0, k.0));
                }
            }
        } else {
            for (e, k) in kind.iter() {
                if let (Some(p), Some(v)) = (pos.get(e), vel.get(e)) {
                    out.push((e, p.0, v.0, k.0));
                }
            }
        }
        out
    }

    #[test]
    fn query3_equals_hand_join() {
        let (w, es) = woven_world();
        let q = Query3::<Pos, Vel, Kind>::new(w.read::<Pos>(), w.read::<Vel>(), w.read::<Kind>());
        let expected = hand_join3(&w);
        let got: Vec<_> = q.iter().map(|(e, p, v, k)| (e, p.0, v.0, k.0)).collect();
        assert_eq!(got, expected, "Query3 输出必须与手写 join 循环全等（序+值）");
        assert_eq!(q.len(), 1); // 仅 0 号齐三件（2 已亡）
        let (p, v, k) = q.get(es[0]).unwrap();
        assert_eq!((p.0, v.0, k.0), (0.0, 0.0, 0));
    }

    #[test]
    fn query3_2w1r_steps_dual_write_table_loop() {
        // 被动怪形态：写 Pos、写 Vel、读 Kind——2W1R 步进。
        let (w, es) = woven_world();
        let mut q = Query3::new(w.write::<Pos>(), w.write::<Vel>(), w.read::<Kind>());
        let mut seen: Vec<u8> = Vec::new();
        q.for_each_2w1r(|e, p, v, k| {
            seen.push(k.0);
            p.0 = 50.0 + k.0 as f32 + e.idx() as f32;
            v.0 = 25.0;
        });
        assert_eq!(seen, vec![0]);
        drop(q);
        assert_eq!(w.read::<Pos>().get(es[0]).unwrap().0, 50.0);
        assert_eq!(w.read::<Vel>().get(es[0]).unwrap().0, 25.0);
    }

    #[test]
    fn query3_get_mut_triple_write() {
        let (w, es) = woven_world();
        let mut q = Query3::new(w.write::<Pos>(), w.write::<Vel>(), w.write::<Kind>());
        let (p, v, k) = q.get_mut(es[0]).unwrap();
        p.0 = 1.5;
        v.0 = 2.5;
        k.0 = 3;
        drop(q);
        assert_eq!(w.read::<Pos>().get(es[0]).unwrap().0, 1.5);
        assert_eq!(w.read::<Vel>().get(es[0]).unwrap().0, 2.5);
        assert_eq!(w.read::<Kind>().get(es[0]).unwrap().0, 3);
    }

    // 查询是构造期快照：after despawn 的新查询看到的交集收窄（旧查询的
    // 守卫占用已从编译面挡住构造期的 &mut World——这里只验时序语义）。
    #[test]
    fn query_reflects_structure_at_construction() {
        let (mut w, es) = woven_world();
        let q_before = Query2::<Pos, Vel>::new(w.read::<Pos>(), w.read::<Vel>());
        assert_eq!(q_before.len(), 2);
        drop(q_before);
        assert!(w.despawn(es[4]));
        let q_after = Query2::<Pos, Vel>::new(w.read::<Pos>(), w.read::<Vel>());
        assert_eq!(q_after.len(), 1, "despawn 后交集必须收窄");
    }
}
