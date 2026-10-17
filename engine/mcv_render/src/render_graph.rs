//! 自研 RenderGraph（#222 批 A）——Bevy v0.14.2 `render_graph` 的实用子集。
//!
//! 旧版蓝图（bevy v0.14.2 crates/bevy_render/src/render_graph/，0.20 已被
//! ECS 调度取代、目录删除）三件套语义对照：
//! - `Node`（`run(&self, RenderGraphContext, RenderContext, &World)`）→
//!   本仓 [`PassNode`]：`execute(&mut self, &mut FrameCtx)` —— 节点保留
//!   `&mut self`（批次调度状态允许），帧上下面合并进单一 `FrameCtx`；
//! - `Edges`（`add_node_edge` 显式边 + InputSlot/OutputSlot 槽位连接）→
//!   本仓两条边源：节点 `deps()` 显式依赖 **∪** `reads()` 资源到全体
//!   `writes()` 者的自动边（Bevy 槽位配对的保守超集，读者必后于写者）；
//! - 拓序 runner（graph_runner.rs 按边拓扑执行）→ [`Plan`]：Kahn 拓序，
//!   并列按**注册序**破平（默认图世界→手持→HUD 即帧序）；
//! - `RenderGraphError`（节点/边/槽错）→ [`GraphError`]：缺边
//!   （依赖指向未注册节点 / 读到无人写的资源）与环一律在**编译面**
//!   （[`RenderGraph::compile`]，即 `Result`）报错，不进运行期。
//!
//! 0.14 的 sub_graph 多图嵌套、view Entity 抽取不在本批（三 pass 单图
//! 已覆盖 draw_frame；后续批按需扩）。运行面：节点沿注册名索引跳转，
//! [`Plan::run`] 只做序遍历——wgpu 指令序与旧直写路径逐条一致
//! （pass 体是原 draw_frame 三块逐字搬移），golden/离屏对拍零漂移。

use std::fmt;

use crate::camera::Camera;
use crate::gpu::{FrameStats, RenderChunk, Renderer, Scene};

/// 节点注册名（`'static`：节点图为静态装配，Bevy 用 interned label，同旨）。
pub type NodeName = &'static str;
/// 帧资源槽名（Bevy SlotInfo 的名字半边；本批只有两槽）。
pub type ResourceId = &'static str;

pub const RES_COLOR: ResourceId = "color";
pub const RES_DEPTH: ResourceId = "depth";

/// 编译面错误（缺边/环/重名）。`compile()` 返回 `Result`，错误**从不**
/// 延迟到运行期——默认图正常装配下四者皆不可触发。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// 同名节点重复注册。
    DuplicateNode { name: NodeName },
    /// 缺边：`deps()` 指向未注册节点（Bevy 语义：node edge 无目标）。
    MissingDependency { node: NodeName, dep: NodeName },
    /// 缺边：读到无任何节点写的资源（Bevy 语义：InputSlot 无 OutputSlot
    /// 可连，graph check 直接拒绝）。
    MissingProducer {
        node: NodeName,
        resource: ResourceId,
    },
    /// 环：拓扑后仍未消化的节点（按注册序列出）。
    Cycle { remaining: Vec<NodeName> },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::DuplicateNode { name } => {
                write!(f, "render-graph: 节点 {name:?} 重复注册")
            }
            GraphError::MissingDependency { node, dep } => {
                write!(
                    f,
                    "render-graph: 节点 {node:?} 依赖未注册节点 {dep:?}（缺边）"
                )
            }
            GraphError::MissingProducer { node, resource } => {
                write!(
                    f,
                    "render-graph: 节点 {node:?} 读资源 {resource:?} 无生产者（缺边）"
                )
            }
            GraphError::Cycle { remaining } => {
                write!(f, "render-graph: 节点环（未消化注册序）：{remaining:?}")
            }
        }
    }
}

impl std::error::Error for GraphError {}

/// 帧上下文：draw_frame 前奏产物（uniform/锥剔/相机派生量）+ 帧级借出。
/// 节点经各字段独立重借取用（fields 离散借，借用检查器逐路径放行）。
pub struct FrameCtx<'f, 's> {
    pub renderer: &'f mut Renderer,
    pub encoder: &'f mut wgpu::CommandEncoder,
    pub color: &'f wgpu::TextureView,
    pub depth: &'f wgpu::TextureView,
    pub scene: &'f Scene<'s>,
    /// frustum 剔后可见区块（slot=枚举序号不变式，见 gpu.rs 世界 pass）。
    pub visible: &'f [&'s RenderChunk],
    /// 结构性统计（#80）：节点体原位累加，帧尾回写 last_stats。
    pub stats: &'f mut FrameStats,
    /// 相机引用 + 派生量（CPU 纯函数，draw_frame 前奏算好，节点只读）。
    pub cam: &'f Camera,
    pub vp: glam::Mat4,
    pub eye: glam::Vec3,
    pub tick: u64,
}

/// 单 pass 节点。`name` 是注册键；`deps` 显式边；`reads/writes` 是资源
/// 槽声明（编译面自动配对 + 文档化）。执行体 = 原 draw_frame 三块逐字。
pub trait PassNode {
    fn name(&self) -> NodeName;
    /// 显式前置节点名；全部必须已注册，缺者编译面 [`GraphError::MissingDependency`]。
    fn deps(&self) -> &'static [NodeName] {
        &[]
    }
    /// 声明消费的帧资源（自动边源：写到读）。
    fn reads(&self) -> &'static [ResourceId] {
        &[]
    }
    /// 声明产出的帧资源。
    fn writes(&self) -> &'static [ResourceId] {
        &[]
    }
    /// 节点执行。Ctx 硬绑定 [`FrameCtx`]（本 trait 不做泛型——本仓节点
    /// 图只服务帧三 pass；泛型化留给后续批真需求驱动）。
    fn execute(&mut self, ctx: &mut FrameCtx<'_, '_>);
}

/// 编译产物：拓序（注册序索引）+ 执行序名字表（诊断/测试面）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    order: Vec<usize>,
    /// 执行序节点名（与 `order` 同位对齐）。
    names: Vec<NodeName>,
}

impl Plan {
    /// 执行序节点名（仅供诊断/测试；运行走索引）。
    pub fn execution_order(&self) -> &[NodeName] {
        &self.names
    }
    /// 拓序 runner：按序执行节点。指令序 = 拓序，与 wgpu 编码共享同一
    /// encoder——位级输出由节点体逐字保真（gpu.rs 三 pass 原块）。
    pub fn run(&self, nodes: &mut [Box<dyn PassNode>], ctx: &mut FrameCtx<'_, '_>) {
        for &i in &self.order {
            nodes[i].execute(ctx);
        }
    }
}

/// 节点图：注册面 + 拓序编译面（缺边/环在此 `Result` 暴露）。
pub struct RenderGraph {
    /// 注册序节点表。帧运行需要整表借出（Renderer::draw_frame mem::take
    /// 拆自引用环），crate 内可见。
    pub(crate) nodes: Vec<Box<dyn PassNode>>,
    /// add_node 置脏；Renderer 帧首脏态重编译（帧手术的新图必过编译面）。
    dirty: bool,
}

impl Default for RenderGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderGraph {
    pub const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            dirty: false,
        }
    }

    /// 注册节点（注册序进并环检测与平度破序）。同名重复由编译面拒绝。
    pub fn add_node(&mut self, node: impl PassNode + 'static) {
        self.nodes.push(Box::new(node));
        self.dirty = true;
    }

    /// 注册序内既有节点（测试/诊断面）。
    pub fn node_names(&self) -> Vec<NodeName> {
        self.nodes.iter().map(|n| n.name()).collect()
    }

    /// add_node 是否迫使下帧重编译。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 编译面：重名 → 缺边（依赖/资源两侧）→ 拓序（环）。
    /// 边集 = 显式 deps ∪ 读→写者自动边（自边除外——节点内部次序即
    /// 语义，如 hand 同帧清深度续画颜色）。
    pub fn compile(&self) -> Result<Plan, GraphError> {
        let n = self.nodes.len();

        // 1) 重名检查（Bevy add_node 只在 debug 断言；我们进编译面 Result）。
        for (i, nd) in self.nodes.iter().enumerate() {
            if self.nodes[(i + 1)..].iter().any(|o| o.name() == nd.name()) {
                return Err(GraphError::DuplicateNode { name: nd.name() });
            }
        }

        // 2) 边集（u 前驱 → v 后继），去重防 Kahn 入度双计。
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut indegree = vec![0usize; n];
        let mut link = |u: usize, v: usize, edges: &mut Vec<Vec<usize>>| {
            if u != v && !edges[u].contains(&v) {
                edges[u].push(v);
                indegree[v] += 1;
            }
        };

        let index_of = |name: NodeName| self.nodes.iter().position(|nd| nd.name() == name);

        // 2a) 显式 deps 边 + 缺边检查。
        for (i, nd) in self.nodes.iter().enumerate() {
            for dep in nd.deps() {
                let target = index_of(*dep).ok_or(GraphError::MissingDependency {
                    node: nd.name(),
                    dep,
                })?;
                link(target, i, &mut edges);
            }
        }

        // 2b) 资源自动边：每读者连向全体写者（无写者 → 缺边 MissingProducer）。
        for (i, nd) in self.nodes.iter().enumerate() {
            for r in nd.reads() {
                let mut produced = false;
                for (p, producer) in self.nodes.iter().enumerate() {
                    if producer.writes().contains(r) {
                        produced = true;
                        link(p, i, &mut edges);
                    }
                }
                if !produced {
                    return Err(GraphError::MissingProducer {
                        node: nd.name(),
                        resource: r,
                    });
                }
            }
        }

        // 3) Kahn 拓序：每次扫描取**最小注册序**的就绪者（稳定破平——
        //    默认图若无边也按注册序执行，帧序=装配序，防静默重排）。
        let mut emitted = vec![false; n];
        let mut order = Vec::with_capacity(n);
        loop {
            let pick = (0..n).find(|&i| !emitted[i] && indegree[i] == 0);
            match pick {
                Some(i) => {
                    emitted[i] = true;
                    for &v in &edges[i] {
                        indegree[v] -= 1;
                    }
                    order.push(i);
                }
                None => break,
            }
        }
        if order.len() != n {
            let remaining = (0..n)
                .filter(|&i| !emitted[i])
                .map(|i| self.nodes[i].name())
                .collect();
            return Err(GraphError::Cycle { remaining });
        }

        Ok(Plan {
            names: order.iter().map(|&i| self.nodes[i].name()).collect(),
            order,
        })
    }
}

// ============================================================================
// 默认图：现有 draw_frame 三 pass（Bevy 0.14 对照：MasterList→MainPass→
// HUD 的节点链同构）。节点体在 gpu.rs `record_world_pass`/`draw_hand`/
// `record_hud_pass`（原直写路径逐字搬移）。
// ============================================================================

/// 世界 pass：天穹→云→不透明地形→实体→裂纹描边→粒子→水（近→远水序）。
pub(crate) struct WorldPassNode;
/// 第一人称手持 pass：清深度独立 pass（26.1 GameRenderer.java:724-729）。
pub(crate) struct HandPassNode;
/// HUD pass：HUD quad 批（世界帧尾，颜色 Load 续画）。
pub(crate) struct HudPassNode;

impl PassNode for WorldPassNode {
    fn name(&self) -> NodeName {
        "world"
    }
    fn writes(&self) -> &'static [ResourceId] {
        &[RES_COLOR, RES_DEPTH]
    }
    fn execute(&mut self, ctx: &mut FrameCtx<'_, '_>) {
        // 字段离散重借：renderer/encoder/stats 可变，其余只读/拷贝——
        // 借用检查按 `ctx.<field>` 路径放行互斥成员并行借用。
        let r = &mut *ctx.renderer;
        let enc = &mut *ctx.encoder;
        let stats = &mut *ctx.stats;
        r.record_world_pass(
            enc,
            ctx.color,
            ctx.depth,
            ctx.scene,
            ctx.visible,
            stats,
            ctx.cam,
            ctx.vp,
            ctx.eye,
            ctx.tick,
        );
    }
}

impl PassNode for HandPassNode {
    fn name(&self) -> NodeName {
        "hand"
    }
    fn deps(&self) -> &'static [NodeName] {
        &["world"]
    }
    fn reads(&self) -> &'static [ResourceId] {
        &[RES_COLOR]
    }
    fn writes(&self) -> &'static [ResourceId] {
        &[RES_COLOR, RES_DEPTH]
    }
    fn execute(&mut self, ctx: &mut FrameCtx<'_, '_>) {
        // 场景无手持件 → 本节点整帧空转（原 draw_frame 的 if-let 前哨）。
        let Some(hand) = ctx.scene.hand else {
            return;
        };
        let r = &mut *ctx.renderer;
        let enc = &mut *ctx.encoder;
        r.draw_hand(enc, ctx.color, ctx.depth, ctx.cam, hand);
    }
}

impl PassNode for HudPassNode {
    fn name(&self) -> NodeName {
        "hud"
    }
    fn deps(&self) -> &'static [NodeName] {
        &["hand"]
    }
    fn reads(&self) -> &'static [ResourceId] {
        &[RES_COLOR]
    }
    fn writes(&self) -> &'static [ResourceId] {
        &[RES_COLOR]
    }
    fn execute(&mut self, ctx: &mut FrameCtx<'_, '_>) {
        let r = &mut *ctx.renderer;
        let enc = &mut *ctx.encoder;
        r.record_hud_pass(enc, ctx.color, ctx.scene);
    }
}

/// 默认装配：world → hand → hud（自动边 world→hand/hud、hand→hud 与
/// 显式 deps 同向，拓序稳定即注册序）。
pub(crate) fn default_frame_graph() -> RenderGraph {
    let mut g = RenderGraph::new();
    g.add_node(WorldPassNode);
    g.add_node(HandPassNode);
    g.add_node(HudPassNode);
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 纯编译面桩：execute 永不调用（图运行需 wgpu 设备，属 CI 离屏
    /// golden 测试域；本文件只锁编译语义）。
    struct Stub {
        name: NodeName,
        deps: &'static [NodeName],
        reads: &'static [ResourceId],
        writes: &'static [ResourceId],
    }
    impl Stub {
        fn n(name: NodeName) -> Box<Self> {
            Box::new(Stub {
                name,
                deps: &[],
                reads: &[],
                writes: &[],
            })
        }
    }
    impl PassNode for Stub {
        fn name(&self) -> NodeName {
            self.name
        }
        fn deps(&self) -> &'static [NodeName] {
            self.deps
        }
        fn reads(&self) -> &'static [ResourceId] {
            self.reads
        }
        fn writes(&self) -> &'static [ResourceId] {
            self.writes
        }
        fn execute(&mut self, _ctx: &mut FrameCtx<'_, '_>) {
            panic!("编译面测试不应触达执行体");
        }
    }

    fn graph(nodes: Vec<Box<dyn PassNode>>) -> RenderGraph {
        let mut g = RenderGraph::new();
        g.nodes = nodes;
        g
    }

    #[test]
    fn default_graph_order_is_frame_order() {
        let g = default_frame_graph();
        let plan = g.compile().expect("默认图必须可编译");
        assert_eq!(plan.execution_order(), &["world", "hand", "hud"][..]);
    }

    #[test]
    fn missing_dependency_is_compile_error() {
        let mut g = RenderGraph::new();
        g.nodes.push(Box::new(Stub {
            name: "solo",
            deps: &["ghost"],
            reads: &[],
            writes: &[],
        }));
        assert_eq!(
            g.compile(),
            Err(GraphError::MissingDependency {
                node: "solo",
                dep: "ghost",
            })
        );
    }

    #[test]
    fn missing_producer_is_compile_error() {
        let g = graph(vec![Box::new(Stub {
            name: "reader",
            deps: &[],
            reads: &["bloom"],
            writes: &[],
        })]);
        assert_eq!(
            g.compile(),
            Err(GraphError::MissingProducer {
                node: "reader",
                resource: "bloom",
            })
        );
    }

    #[test]
    fn cycle_is_compile_error_with_remaining_nodes() {
        let g = graph(vec![
            Box::new(Stub {
                name: "a",
                deps: &["b"],
                reads: &[],
                writes: &[],
            }),
            Box::new(Stub {
                name: "b",
                deps: &["a"],
                reads: &[],
                writes: &[],
            }),
        ]);
        assert_eq!(
            g.compile(),
            Err(GraphError::Cycle {
                remaining: vec!["a", "b"],
            })
        );
    }

    #[test]
    fn duplicate_node_is_compile_error() {
        let g = graph(vec![Stub::n("dup"), Stub::n("dup")]);
        assert_eq!(g.compile(), Err(GraphError::DuplicateNode { name: "dup" }));
    }

    #[test]
    fn independent_nodes_keep_registration_order() {
        // 反向注册（hud 在前）+ 无边：拓序=注册序，帧序不被静默重排。
        let g = graph(vec![Stub::n("z"), Stub::n("a")]);
        let plan = g.compile().expect("无边图必编译");
        assert_eq!(plan.execution_order(), &["z", "a"][..]);
    }

    #[test]
    fn resource_edges_order_writer_before_reader() {
        // 只注册资源面（无显式 deps）：写者 world 必排在读者 hud 前。
        let g = graph(vec![
            Box::new(Stub {
                name: "hud",
                deps: &[],
                reads: &[RES_COLOR],
                writes: &[RES_COLOR],
            }),
            Box::new(Stub {
                name: "world",
                deps: &[],
                reads: &[],
                writes: &[RES_COLOR, RES_DEPTH],
            }),
        ]);
        let plan = g.compile().expect("资源边图必编译");
        assert_eq!(plan.execution_order(), &["world", "hud"][..]);
    }
}
