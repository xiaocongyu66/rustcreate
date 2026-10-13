//! #178 刀2b 行为快照锁：`tool::requirement_by_id` 与
//! `BlockId::is_replaceable` 切换前后逐 id 对拍（1171 全表）。
//!
//! `REQ_OLD`/`REPLACE_OLD` = 切换前手写实现（feat/data-registry@555fde1f 的
//! tool.rs 四档名单 + SNOW_SHOVEL / lib.rs 29 名手写名单+air+liquid）
//! 对全部 1171 注册行的逐位输出快照（Python 一次性导出，数组即事实登记）。
//! 新实现与其差异只许命中 `VANILLA_CORRECTIONS` 白名单（每条注明原版依据），
//! 白名单外的任何漂移 = 红。等价性铁律取证文件。
//!
//! 编码：0=无需工具；1=镐 T1；2=镐 T2；3=镐 T4；4=镐 T5；5=铲 T1；
//! 6=NEVER（gate=true 且无工具途径，仅新版可产出）。

use mcv_core::{BlockId, tool};

const NEVER: u8 = 6;

fn decode(r: Option<tool::Requirement>) -> u8 {
    match r {
        None => 0,
        Some(tool::Requirement::Never) => NEVER,
        Some(tool::Requirement::Tool { kind, tier }) => match (kind, tier) {
            (tool::ToolReq::Pickaxe, 1) => 1,
            (tool::ToolReq::Pickaxe, 2) => 2,
            (tool::ToolReq::Pickaxe, 4) => 3,
            (tool::ToolReq::Pickaxe, 5) => 4,
            (tool::ToolReq::Shovel, 1) => 5,
            // 新档位出现 = 生成表/tag 漂移，快照锁要求显式重审。
            (k, t) => panic!("快照外工具组合 {k:?} tier{t}"),
        },
    }
}

/// 原版修正白名单：方块名 → (旧码, 新码)。白名单外的 id 必须逐位相等。
/// 共 22 条，三族：
/// 1. 命令块族 5（command/chain/repeating/structure/jigsaw）：1 → 6。
///    原版 Blocks.java 属性 gate=true（旧名单没记错成员），但它们不在
///    任何 mineable tag ⇒ 工具侧 `Tool.isCorrectForDrops` 无规则匹配
///    →false（Tool.java:50-57），叠加 `Player.java:619` gate ⇒ 任何工具
///    （甚至下界合金镐）都不掉落。旧名单记「木镐即正确」是错。
/// 2. cobweb：0 → 6。原版 gate=true 且只有剪刀/剑的专用 Tool 组件能掉；
///    引擎无该组件（旧注释「宁缺勿错」），徒手从「可掉+30 档」修正为
///    「不可掉+100 档」（徒手原版是 12 s 磨且不掉，引擎旧行为偏易）。
/// 3. copper_bars / copper_chain 全氧化/打蜡 16 变体：0 → 1。
///    WeatheringCopperBlocks.create 的属性链带 requiresCorrectToolForDrops
///    （Blocks.java:1934/1944 lambda，∉needs_* ⇒ 任意镐级）；旧 #90
///    手写「最小可用集」漏收录，本次按实物补全。
static VANILLA_CORRECTIONS: &[(&str, u8, u8)] = &[
    ("chain_command_block", 1, NEVER),
    ("cobweb", 0, NEVER),
    ("command_block", 1, NEVER),
    ("copper_bars", 0, 1),
    ("copper_chain", 0, 1),
    ("exposed_copper_bars", 0, 1),
    ("exposed_copper_chain", 0, 1),
    ("jigsaw", 1, NEVER),
    ("oxidized_copper_bars", 0, 1),
    ("oxidized_copper_chain", 0, 1),
    ("repeating_command_block", 1, NEVER),
    ("structure_block", 1, NEVER),
    ("waxed_copper_bars", 0, 1),
    ("waxed_copper_chain", 0, 1),
    ("waxed_exposed_copper_bars", 0, 1),
    ("waxed_exposed_copper_chain", 0, 1),
    ("waxed_oxidized_copper_bars", 0, 1),
    ("waxed_oxidized_copper_chain", 0, 1),
    ("waxed_weathered_copper_bars", 0, 1),
    ("waxed_weathered_copper_chain", 0, 1),
    ("weathered_copper_bars", 0, 1),
    ("weathered_copper_chain", 0, 1),
];

/// 切换前 `tool::requirement`（555fde1f 手写名单）逐 id 输出。
#[rustfmt::skip]
static REQ_OLD: [u8; 1171] = [
    0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 1, 0, 4, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 1, 1, 1,
    1, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1,
    0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0,
    0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1,
    0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1,
    0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 0, 1, 1, 1, 1, 1,
    1, 1, 0, 0, 1, 0, 0, 0, 0, 2, 2, 0, 2, 0, 0, 2,
    0, 2, 0, 2, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 4, 2, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0, 0,
    1, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0,
    1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 2, 3, 3, 3,
    2, 2, 3, 1, 1, 1, 1, 0, 3, 3, 1, 1, 1, 1, 0, 1,
    0, 0, 0, 0, 0, 1, 1, 3, 3, 1, 0, 0, 0, 0, 1, 1,
    1, 1, 1, 0, 2, 2, 0, 2, 0, 2, 0, 0, 2, 0, 2, 2,
    2, 2, 2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 0, 0, 0, 0, 0, 3, 3, 0, 1, 1, 1, 1, 0,
    0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0,
    0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0,
    0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 2, 1, 0, 2, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    2, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1,
    0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0,
    1, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1,
    0, 0, 0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 0, 0, 4, 1,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 4, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 1,
    0, 0, 0, 0, 2, 2, 0, 2, 0, 2, 0, 0, 2, 0, 2, 2,
    2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,
    0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1,
    1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0,
    0, 1, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 2, 3,
    2, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 1, 1, 1, 1, 0,
    1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 3, 0, 0,
    0, 0, 0, 1, 0, 1, 1, 1, 1, 0, 4, 0, 0, 1, 1, 1,
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 0, 5, 5, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 1, 1,
    1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 2, 0, 2, 2, 0, 2, 0, 0, 2, 0, 2, 2, 2, 2, 2,
    2, 0, 2, 0, 2, 0, 0, 2, 0, 2, 2, 2, 2, 2, 2, 2,
    2, 0, 2, 0, 2, 0, 0, 2, 0, 2, 2, 2, 2, 2, 2, 2,
    0, 2, 0, 2, 0, 0, 2, 0, 2, 2, 2, 2, 2, 2, 2, 0,
    2, 0, 2, 0, 0, 2, 0, 2, 2, 2, 2, 2, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0,
    0, 0, 0,];

/// 切换前 `is_replaceable`（air ∥ liquid ∥ 29 名手写名单）逐 id 输出。
#[rustfmt::skip]
static REPLACE_OLD: [bool; 1171] = [
    true, false, false, false, false, true, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, true, false, false, false, false, false, true, false, false, false, false,
    false, false, false, false, false, false, false, false, true, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    true, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, true, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, true, true,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, true, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, true,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, true, true, false,
    true, false, false, true, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, true, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, true, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, true, true, true, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, true, false, false, true, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    true, false, false, false, false, false, true, true, true, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, true, true, false,
    false, false, false, false, false, false, false, false, false, false, true, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false, false, false, false, false, false,
    false, false, false, false, false, false, false, false, false,];

/// requirement 全表逐位锁 + 白名单显式登记核对。
#[test]
fn requirement_snapshot_all_1171() {
    let corr: std::collections::HashMap<&str, (u8, u8)> = VANILLA_CORRECTIONS
        .iter()
        .map(|(n, o, c)| (*n, (*o, *c)))
        .collect();
    let mut seen = 0;
    for (i, b) in mcv_core::BLOCKS.iter().enumerate() {
        let old = REQ_OLD[i];
        let got = decode(mcv_core::tool::requirement_by_id(BlockId(i as u16)));
        if let Some((wo, wc)) = corr.get(b.name) {
            assert_eq!((old, got), (*wo, *wc), "{} 修正集成员漂移", b.name);
            seen += 1;
        } else {
            assert_eq!(
                got, old,
                "{} 白名单外行为漂移（旧 {old} → 新 {got}）",
                b.name
            );
        }
    }
    assert_eq!(seen, VANILLA_CORRECTIONS.len(), "白名单有方块改名/失踪");
}

/// is_replaceable 全表逐位锁（tag 切换 = 零行为差异，无白名单）。
#[test]
fn replaceable_snapshot_all_1171() {
    for (i, b) in mcv_core::BLOCKS.iter().enumerate() {
        assert_eq!(
            BlockId(i as u16).is_replaceable(),
            REPLACE_OLD[i],
            "{} 可替换性漂移",
            b.name
        );
    }
}
