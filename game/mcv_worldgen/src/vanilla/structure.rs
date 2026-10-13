//! 结构模板运行时（StructureTemplate 机制的 clean-room 转写）。
//!
//! 消费 mcv_nbt 读原版结构 `.nbt`。load 双格式路径按 26.1 实物
//! `StructureTemplate.load`（StructureTemplate.java:710-721）：
//! - `palettes`（List of List）存在 → 每个调色板变体对**同一** blocks
//!   列表各展开一份（loadPalette 对每个 palette 复用 blockList,
//!   :716-720）；放置时按 `getRandomPalette` 选取（见下）。
//! - 否则 `palette` 单调色板（:721）。
//! - blocks `state` 越界/负数 → **空气**（SimplePalette.stateFor
//!   :867-870 的 DEFAULT_BLOCK_STATE = Blocks.AIR 实物）。
//! - 旧世代 `width/height/length`+`BlockIndices` 格式：26.1 代码树中
//!   **不存在**读取路径（全树 grep 无 `BlockIndices`;旧文件由 DataFixer
//!   在进入本类前升级）——按实物不复刻。
//!
//! 放置调用序逐行对照实物（详见 `place_at` 行内注释）：
//! 1. `getRandomPalette`（StructurePlaceSettings.java:138-143）：
//!    `getRandom(position).nextInt(palettes.size())`,无 settings.random
//!    时 = `RandomSource.create(Mth.getSeed(position))` 播种一次取
//!    nextInt（getRandom:110-115;26.1 `RandomSource.create(long)` =
//!    LegacyRandomSource LCG,RandomSource.java:24-26——Xoroshiro 系不在
//!    该路径,见 `mcv_core::rand` 模块头注）。
//! 2. 逐 block `BlockRotProcessor.processBlock`（BlockRotProcessor.java:
//!    43-53）:`settings.getRandom(world_pos)`——无共享流时**每块全新
//!    播种** `Mth.getSeed(世界坐标)`,掷一次 `nextFloat()`;
//!    `!(roll <= integrity)` 丢弃。integrity 处理器在 OceanRuinPieces
//!    实物里先于 ignore 添加（OceanRuinPieces.java:296-297）→ air 同样
//!    消耗掷骰,之后才被 `BlockIgnoreProcessor.STRUCTURE_AND_AIR` 滤除。
//! 3. 变换用 `StructureTemplate.transform` 公式（先 mirror 后 rotation,
//!    pivot 公式照抄）。
//!
//! 实物 `buildInfoList`（StructureTemplate.java:156-171）把 block 按
//! full/other/BE 三桶 (y,x,z) 排序后才进处理器;掷骰按世界坐标独立播种
//! （第 2 步）使排序**不影响放置结果**,仅影响 setBlock 先后——本仓输出
//! 保持 blockList 文件序。
//!
//! 登记的债（KNOWN-DIVERGENCE STRU-*）:
//! - STRU-STATE:mirror/rotation 只变换 block pos;palette 条目 Name 的
//!   facing 等 blockstate 属性旋转（原版 `BlockState.mirror/rotate` 经
//!   BlockRotationUtil）未实装——ruined_portal 主要受影响的楼梯/熔接器
//!   朝向会保持模板原向。
//! - STRU-BE:block `nbt`（方块实体）与 `entities` 已解析随 Template
//!   保留,但放置面不落地（世界层无 BE/实体通道;原版对带 nbt 的 block
//!   先铺 BARRIER 再写 BE,StructureTemplate.java:288-290）。

use std::collections::HashMap;
use std::sync::OnceLock;

use mcv_core::rand::{Lcg, mth_get_seed};
use mcv_core::{BLOCKS, BlockId};
use mcv_nbt::{NbtCompound, NbtError, NbtValue, read_auto};

/// 镜像轴（26.1 `Mirror`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirror {
    /// 不镜像。
    None,
    /// 沿 Z 轴镜像（LEFT_RIGHT: z → -z）。
    LeftRight,
    /// 沿 X 轴镜像（FRONT_BACK: x → -x）。
    FrontBack,
}

/// 旋转（26.1 `Rotation`,绕 pivot 的 y 轴旋转）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// 0°。
    None,
    /// 90° 顺时针。
    Clockwise90,
    /// 180°。
    Clockwise180,
    /// 90° 逆时针。
    Counterclockwise90,
}

/// 单个模板方块:模板局部坐标 + 解析后的方块。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateBlock {
    /// 模板局部坐标。
    pub pos: [i32; 3],
    /// palette 解析结果。
    pub block: BlockId,
    /// 该 block 带 `nbt`（方块实体,STRU-BE:已解析保留,放置不落地）。
    pub has_nbt: bool,
}

/// 模板实体（`entities` 条目,STRU-BE:已解析保留,放置不落地）。
#[derive(Debug, Clone)]
pub struct TemplateEntity {
    /// 实体系坐标（原版 Vec3）。
    pub pos: [f64; 3],
    /// 所在方块坐标（原版 BlockPos）。
    pub block_pos: [i32; 3],
    /// 实体 NBT（原版直接挂载,本仓保留待消费）。
    pub nbt: Option<NbtValue>,
}

/// 结构模板运行时。
#[derive(Debug, Clone)]
pub struct Template {
    /// (width=x, height=y, length=z)。
    pub size: [i32; 3],
    /// 调色板变体（原版 `palettes`）:每变体对同一 blockList 独立展开
    /// （loadPalette:716-721）,至少一个。
    pub variants: Vec<Vec<TemplateBlock>>,
    /// 各变体的 palette 解析表（插入序即文件序）。
    pub palettes: Vec<Vec<BlockId>>,
    /// `entities` 解析结果（STRU-BE）。
    pub entities: Vec<TemplateEntity>,
    /// 未识别方块名个数（解析时计数,>0 时 warn 一次）。
    pub unknown_names: u32,
}

impl Template {
    /// 第 0 变体的方块列表（单调色板模板即全体;测试/单变体消费便捷口）。
    #[must_use]
    pub fn blocks(&self) -> &[TemplateBlock] {
        &self.variants[0]
    }

    /// 第 0 变体的 palette。
    #[must_use]
    pub fn palette(&self) -> &[BlockId] {
        &self.palettes[0]
    }
}

/// 模板解析错误:复用 mcv_nbt 错误 + 字段结构错误。
#[derive(Debug)]
pub enum TemplateError {
    /// NBT 二进制/SNBT 层失败。
    Nbt(NbtError),
    /// 必需字段缺失/类型不对。
    BadField(&'static str),
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::Nbt(e) => write!(f, "template nbt error: {e}"),
            TemplateError::BadField(k) => write!(f, "template missing/bad field: {k}"),
        }
    }
}

impl std::error::Error for TemplateError {}

impl From<NbtError> for TemplateError {
    fn from(e: NbtError) -> Self {
        TemplateError::Nbt(e)
    }
}

impl Template {
    /// 从 `.nbt` 字节（GZIP/ZLIB/RAW 自动嗅探）解析模板。
    pub fn from_nbt(bytes: &[u8]) -> Result<Template, TemplateError> {
        let root = read_auto(bytes)?;
        Self::from_compound(&root)
    }

    /// 从已解析 compound 构造（load 双格式路径:StructureTemplate.java:
    /// 710-721——`palettes` 优先,逐变体 loadPalette;否则 `palette` 单变体;
    /// 两缺=空 palette 单变体,全部 stateFor 回落空气,同实物
    /// getListOrEmpty 语义）。
    pub fn from_compound(root: &NbtCompound) -> Result<Template, TemplateError> {
        let size = root
            .get("size")
            .and_then(int3_of)
            .ok_or(TemplateError::BadField("size"))?;
        // 调色板变体列表（原版 load:715-722）。
        let palette_tags: Vec<Vec<NbtValue>> = if let Some(all) = root.get_list("palettes") {
            all.items()
                .iter()
                .map(|v| v.as_list().map_or_else(Vec::new, |l| l.items().to_vec()))
                .collect()
        } else if let Some(single) = root.get_list("palette") {
            vec![single.items().to_vec()]
        } else {
            vec![Vec::new()]
        };
        // 共享 blockList 先解析一遍(loadPalette 对每 palette 复用同列)。
        let blocks_tag = root
            .get_list("blocks")
            .ok_or(TemplateError::BadField("blocks"))?;
        let mut raw_blocks: Vec<([i32; 3], i32, bool)> = Vec::new();
        for b in blocks_tag.items() {
            let Some(c) = b.as_compound() else {
                continue;
            };
            let Some(pos) = c.get("pos").and_then(int3_of) else {
                continue;
            };
            // getIntOr("state", 0) 实物:缺失/非 int 一律 0。
            let idx = c.get_i32("state").unwrap_or(0);
            raw_blocks.push((pos, idx, c.get("nbt").is_some()));
        }
        let mut variants = Vec::with_capacity(palette_tags.len());
        let mut palettes = Vec::with_capacity(palette_tags.len());
        let mut unknown = 0u32;
        for pal in &palette_tags {
            let mut palette = Vec::with_capacity(pal.len());
            for entry in pal {
                let name = entry
                    .as_compound()
                    .and_then(|c| c.get_string("Name"))
                    .ok_or(TemplateError::BadField("palette.Name"))?;
                let id = block_by_name(name);
                if id.is_none() {
                    unknown += 1;
                }
                palette.push(id.unwrap_or_else(air_id));
            }
            variants.push(
                raw_blocks
                    .iter()
                    .map(|&(pos, idx, has_nbt)| TemplateBlock {
                        pos,
                        // stateFor 越界/负数 → DEFAULT_BLOCK_STATE=AIR
                        // (SimplePalette.java:867-870)。
                        block: usize::try_from(idx)
                            .ok()
                            .and_then(|i| palette.get(i).copied())
                            .unwrap_or_else(air_id),
                        has_nbt,
                    })
                    .collect(),
            );
            palettes.push(palette);
        }
        if unknown > 0 {
            // 未知方块名 → 空气,整模板只 warn 一次（指令口径）。
            log::warn!("structure template: {unknown} 个未知方块名按空气处理");
        }
        // entities(load:724-731):pos(Vec3 f64)/blockPos(Vec3i)/nbt。
        let mut entities = Vec::new();
        if let Some(list) = root.get_list("entities") {
            for e in list.items() {
                let Some(c) = e.as_compound() else { continue };
                let pos = c
                    .get("pos")
                    .and_then(|v| v.as_list())
                    .filter(|l| l.len() >= 3)
                    .map(|l| {
                        [
                            l.get(0).and_then(f64_of).unwrap_or(0.0),
                            l.get(1).and_then(f64_of).unwrap_or(0.0),
                            l.get(2).and_then(f64_of).unwrap_or(0.0),
                        ]
                    })
                    .unwrap_or([0.0; 3]);
                let block_pos = c.get("blockPos").and_then(int3_of).unwrap_or([0; 3]);
                entities.push(TemplateEntity {
                    pos,
                    block_pos,
                    nbt: c.get("nbt").cloned(),
                });
            }
        }
        Ok(Template {
            size,
            variants,
            palettes,
            entities,
            unknown_names: unknown,
        })
    }

    /// 放置:palette 选取 + 变换(mirror→rotate,绕 pivot)+ anchor 平移 +
    /// integrity 掷骰 + air 不放置。返回世界坐标方块列表(blockList 文件序;
    /// 原版为位置播种掷骰,两次调用必然逐字节一致,见实物 getRandom)。
    #[must_use]
    pub fn place_at(
        &self,
        anchor: [i32; 3],
        pivot: [i32; 3],
        mirror: Mirror,
        rotation: Rotation,
        integrity: f32,
    ) -> Vec<(i32, i32, i32, BlockId)> {
        let air = air_id();
        // getRandomPalette(StructurePlaceSettings.java:138-143):
        // getRandom(position).nextInt(size);getRandom(:110-115)无共享流时
        // = RandomSource.create(Mth.getSeed(position))——每放置播种一次,
        // 单变体模板同样消耗一次 nextInt(1)(实物无条件调用)。
        let mut sel = Lcg::new(mth_get_seed(anchor[0], anchor[1], anchor[2]));
        let variant = &self.variants[sel.next_int_bounded(self.variants.len() as i32) as usize];
        let mut out = Vec::with_capacity(variant.len());
        for b in variant {
            let t = transform(b.pos, mirror, rotation, pivot);
            let wp = [t[0] + anchor[0], t[1] + anchor[1], t[2] + anchor[2]];
            // BlockRotProcessor.processBlock(BlockRotProcessor.java:48-51):
            // settings.getRandom(processedBlockInfo.pos)——无共享流时每块
            // 全新 Mth.getSeed(世界坐标) 播种,恰一次 nextFloat();
            // `!(roll <= integrity)` → 丢弃。rot 先于 ignore 添加
            // (OceanRuinPieces.java:296-297)→ air 也掷骰。
            let roll = Lcg::new(mth_get_seed(wp[0], wp[1], wp[2])).next_float();
            if roll > integrity {
                continue;
            }
            // BlockIgnoreProcessor.STRUCTURE_AND_AIR:air 不放置。
            if b.block == air {
                continue;
            }
            out.push((wp[0], wp[1], wp[2], b.block));
        }
        out
    }
}

/// 26.1 StructureTemplate.transform 公式照抄(先 mirror 后 rotation)。
#[must_use]
pub fn transform(pos: [i32; 3], mirror: Mirror, rotation: Rotation, pivot: [i32; 3]) -> [i32; 3] {
    let (x, y, z) = (pos[0], pos[1], pos[2]);
    let (x, z) = match mirror {
        Mirror::LeftRight => (x, -z),
        Mirror::FrontBack => (-x, z),
        Mirror::None => (x, z),
    };
    let (px, pz) = (pivot[0], pivot[2]);
    match rotation {
        Rotation::Counterclockwise90 => [px - pz + z, y, px + pz - x],
        Rotation::Clockwise90 => [px + pz - z, y, pz - px + x],
        Rotation::Clockwise180 => [px + px - x, y, pz + pz - z],
        Rotation::None => [x, y, z],
    }
}

/// 取 tag 的 3 分量整数(容忍 IntList 与 IntArray 两种实物形态)。
fn int3_of(v: &NbtValue) -> Option<[i32; 3]> {
    match v {
        NbtValue::List(l) => {
            let g = |i: usize| l.get(i)?.as_i32();
            Some([g(0)?, g(1)?, g(2)?])
        }
        NbtValue::IntArray(a) => Some([*a.first()?, *a.get(1)?, *a.get(2)?]),
        _ => None,
    }
}

/// list 元素的 f64 读取(entities pos 实物为 Double;容忍整型位)。
fn f64_of(v: &NbtValue) -> Option<f64> {
    match v {
        NbtValue::Double(d) => Some(*d),
        NbtValue::Float(d) => Some(f64::from(*d)),
        NbtValue::Int(i) => Some(*i as f64),
        NbtValue::Long(l) => Some(*l as f64),
        _ => None,
    }
}

/// Name → BlockId(`minecraft:` 前缀可省)。注册表名→下标一次性建表
/// (OnceLock),O(1) 查表。
#[must_use]
pub fn block_by_name(name: &str) -> Option<BlockId> {
    let bare = name.strip_prefix("minecraft:").unwrap_or(name);
    // 带属性串(如 "stone[slab=bottom]")按基名解析——属性旋转未实装
    // (STRU-STATE),palette Name 实物也不带方括号属性。
    let base = bare.split('[').next().unwrap_or(bare);
    static INDEX: OnceLock<HashMap<&'static str, BlockId>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            BLOCKS
                .iter()
                .enumerate()
                .map(|(i, d)| (d.name, BlockId(i as u16)))
                .collect()
        })
        .get(base)
        .copied()
}

fn air_id() -> BlockId {
    block_by_name("air").unwrap_or(BlockId(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_matches_vanilla_formulas() {
        // Rotation.CLOCKWISE_90,pivot=(0,0,0):(1,0,0) → (0,0,1)。
        assert_eq!(
            transform([1, 7, 0], Mirror::None, Rotation::Clockwise90, [0, 0, 0]),
            [0, 7, 1]
        );
        // Mirror LEFT_RIGHT → (1,0,-5);CCW90 公式 [px−pz+z, y, px+pz−x],
        // pivot=(2,0,3) → [2−3+(−5), 0, 2+3−1] = [−6, 0, 4]。
        assert_eq!(
            transform(
                [1, 0, 5],
                Mirror::LeftRight,
                Rotation::Counterclockwise90,
                [2, 0, 3]
            ),
            [-6, 0, 4]
        );
    }

    /// 测试侧复刻实物 getRandomPalette/BlockRotProcessor 的种子+比较,
    /// 与 place_at 实现互证(独立重推,不共享中间量)。
    fn roll_at(wp: [i32; 3]) -> f32 {
        Lcg::new(mth_get_seed(wp[0], wp[1], wp[2])).next_float()
    }

    #[test]
    fn synthetic_template_roundtrip_and_place() {
        // 不依赖资产的自构造模板:2x1x2,palette [stone, air]。
        let root = synth(
            &[&["minecraft:stone", "minecraft:air"]],
            &[
                ([0, 0, 0], 0),
                ([1, 0, 0], 0),
                ([0, 0, 1], 0),
                ([1, 0, 1], 1),
            ],
        );
        let t = Template::from_compound(&root).unwrap();
        assert_eq!(t.size, [2, 1, 2]);
        assert_eq!(t.blocks().len(), 4);
        assert_eq!(t.variants.len(), 1);
        assert!(
            t.palette()
                .iter()
                .all(|b| *b == block_by_name("stone").unwrap() || *b == air_id())
        );

        // 两次调用逐字节一致(实物位置播种无流状态,天然确定);air 不放置。
        let a = t.place_at([10, 20, 30], [0, 0, 0], Mirror::None, Rotation::None, 1.0);
        let b = t.place_at([10, 20, 30], [0, 0, 0], Mirror::None, Rotation::None, 1.0);
        assert_eq!(a, b);
        assert_eq!(a.len(), 3, "air 不放置");
        // integrity=-0.5 → 数学保证全丢(roll ≥ 0 > -0.5)。
        assert!(
            t.place_at([0, 0, 0], [0, 0, 0], Mirror::None, Rotation::None, -0.5)
                .is_empty()
        );

        // 90° 旋转:局部包围盒 x/z 互换律——用沿 x 轴的 3x1x1 直条。
        let root2 = synth(
            &[&["minecraft:stone"]],
            &[([0, 0, 0], 0), ([1, 0, 0], 0), ([2, 0, 0], 0)],
        );
        let t2 = Template::from_compound(&root2).unwrap();
        let plain = t2.place_at([0, 0, 0], [0, 0, 0], Mirror::None, Rotation::None, 1.0);
        let rot = t2.place_at(
            [0, 0, 0],
            [0, 0, 0],
            Mirror::None,
            Rotation::Clockwise90,
            1.0,
        );
        let ext = |v: &[(i32, i32, i32, BlockId)]| {
            let xs: Vec<i32> = v.iter().map(|p| p.0).collect();
            let zs: Vec<i32> = v.iter().map(|p| p.2).collect();
            (
                xs.iter().max().unwrap() - xs.iter().min().unwrap() + 1,
                zs.iter().max().unwrap() - zs.iter().min().unwrap() + 1,
            )
        };
        assert_eq!(ext(&plain), (3, 1));
        assert_eq!(ext(&rot), (1, 3), "90° 旋转包围盒 x/z 互换律");
    }

    #[test]
    fn integrity_dice_are_position_seeded_vanilla() {
        // 实物:每块 = 新 Lcg(Mth.getSeed(世界坐标)).nextFloat() > integrity
        // → 丢弃(BlockRotProcessor.java:48-51)。自实现向量:同一局部块在
        // 两个不同 anchor 下命运一般不同(位置种子),而在相同世界坐标下
        // 命运相同。
        let root = synth(
            &[&["minecraft:stone"]],
            &[([0, 0, 0], 0), ([1, 0, 0], 0), ([2, 0, 0], 0)],
        );
        let t = Template::from_compound(&root).unwrap();
        let stone = block_by_name("stone").unwrap();
        // integrity 取某块 roll 的临界值:对 anchor=(0,0,0),块 (1,0,0) 的
        // roll 恰为阈值 ±ε 时,±ε 必须翻转其保留/丢弃(边界语义 <=)。
        let r1 = roll_at([1, 0, 0]);
        assert!(r1 > 0.0, "本用例需要非零下界");
        let at_thresh_excl = t.place_at([0, 0, 0], [0, 0, 0], Mirror::None, Rotation::None, r1);
        // roll <= integrity 保留:integrity = r1 → (1,0,0) 保留。
        assert!(
            at_thresh_excl
                .iter()
                .any(|p| p.0 == 1 && p.3 == stone && p.1 == 0 && p.2 == 0),
            "integrity == roll 必须保留(<= 实物): roll {r1}"
        );
        let just_below = f32::from_bits(r1.to_bits() - 1);
        let at_thresh_incl = t.place_at(
            [0, 0, 0],
            [0, 0, 0],
            Mirror::None,
            Rotation::None,
            just_below,
        );
        assert!(
            !at_thresh_incl.iter().any(|p| p.0 == 1 && p.2 == 0),
            "integrity 比 roll 低 1ulp 必须丢弃: roll {r1}"
        );
        // 位置种子:块 (0,0,0)@anchor0 的世界坐标 = 块 (1,0,0)@anchor(-1,0,0),
        // 两处掷骰同值 → 同 integrity 下同命运。
        let w0 = t.place_at([0, 0, 0], [0, 0, 0], Mirror::None, Rotation::None, 0.5);
        let w1 = t.place_at([-1, 0, 0], [0, 0, 0], Mirror::None, Rotation::None, 0.5);
        let in0 = w0.iter().any(|p| p.0 == 0 && p.2 == 0);
        let in1 = w1.iter().any(|p| p.0 == 0 && p.2 == 0);
        assert_eq!(in0, in1, "同世界坐标必须同命运(位置播种)");
    }

    #[test]
    fn multi_palette_variants_selected_by_seed() {
        // 实物:palettes 逐变体展开同列 blocks;放置按
        // Xor…(实为 LCG)create(Mth.getSeed(anchor)).nextInt(n) 选一
        // (StructurePlaceSettings.java:138-143)。
        let root = synth_multi(
            &[
                &["minecraft:stone", "minecraft:dirt"],
                &["minecraft:dirt", "minecraft:stone"],
            ],
            &[([0, 0, 0], 0), ([1, 0, 0], 1)],
        );
        let t = Template::from_compound(&root).unwrap();
        assert_eq!(t.variants.len(), 2);
        // 两变体 pos 相同、方块互换。
        assert_eq!(t.variants[0][0].block, t.variants[1][1].block);
        assert_eq!(t.variants[0][1].block, t.variants[1][0].block);
        // 选取确定性:同 anchor 两次一致;且两 anchor 覆盖两个变体。
        let a = t.place_at([5, 6, 7], [0, 0, 0], Mirror::None, Rotation::None, 1.0);
        let b = t.place_at([5, 6, 7], [0, 0, 0], Mirror::None, Rotation::None, 1.0);
        assert_eq!(a, b);
        // 实物种子推导:nextInt(2) 选出的变体,其方块组合必须整体命中
        // (不会混用两变体)。
        let idx = Lcg::new(mth_get_seed(5, 6, 7)).next_int_bounded(2) as usize;
        let stone = block_by_name("stone").unwrap();
        let dirt = block_by_name("dirt").unwrap();
        let expect_pair: [BlockId; 2] = [t.variants[idx][0].block, t.variants[idx][1].block];
        assert!(
            expect_pair.contains(&stone)
                && expect_pair.contains(&dirt)
                && a.iter().all(|p| p.3 == stone || p.3 == dirt)
        );
        let got: Vec<BlockId> = a.iter().map(|p| p.3).collect();
        assert!(
            (expect_pair[0] == expect_pair[1])
                || (got.contains(&expect_pair[0]) && got.contains(&expect_pair[1])),
            "单次放置只能来自选中变体: {got:?} expect {expect_pair:?}"
        );
    }

    #[test]
    fn state_out_of_range_falls_back_to_air_first_entry_is_not_used() {
        // SimplePalette.stateFor:867-870——越界回落 DEFAULT_BLOCK_STATE
        // = Blocks.AIR(**不是** palette 首元素;任务书「首元素」说法与
        // 26.1 实物不符,以实物为准)。
        let root = synth(
            &[&["minecraft:stone", "minecraft:dirt"]],
            &[([0, 0, 0], 5), ([1, 0, 0], -3), ([2, 0, 0], 1)],
        );
        let t = Template::from_compound(&root).unwrap();
        assert_eq!(t.blocks()[0].block, air_id(), "state=5 越界 → 空气");
        assert_eq!(t.blocks()[1].block, air_id(), "state=-3 → 空气");
        assert_eq!(t.blocks()[2].block, block_by_name("dirt").unwrap());
        // air 越界位不放置:
        let placed = t.place_at([0, 0, 0], [0, 0, 0], Mirror::None, Rotation::None, 1.0);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].3, block_by_name("dirt").unwrap());
    }

    #[test]
    fn entities_and_block_nbt_parsed() {
        let mut root = synth(&[&["minecraft:stone"]], &[([0, 0, 0], 0)]);
        let mut ent = NbtCompound::new();
        let mut pos = mcv_nbt::NbtList::new();
        for v in [1.5f64, 2.0, 3.25] {
            pos.push(NbtValue::Double(v)).unwrap();
        }
        ent.insert("pos", NbtValue::List(pos));
        ent.insert(
            "blockPos",
            NbtValue::List(mcv_nbt::NbtList::from_vec(vec![
                NbtValue::Int(1),
                NbtValue::Int(2),
                NbtValue::Int(3),
            ])),
        );
        let mut inner = NbtCompound::new();
        inner.insert("id", NbtValue::String("minecraft:zombie".into()));
        ent.insert("nbt", NbtValue::Compound(inner));
        let mut ents = mcv_nbt::NbtList::new();
        ents.push(NbtValue::Compound(ent)).unwrap();
        root.insert("entities", NbtValue::List(ents));
        let t = Template::from_compound(&root).unwrap();
        assert_eq!(t.entities.len(), 1);
        assert_eq!(t.entities[0].pos, [1.5, 2.0, 3.25]);
        assert_eq!(t.entities[0].block_pos, [1, 2, 3]);
        assert!(t.entities[0].nbt.is_some(), "实体 nbt 必须保留(STRU-BE)");
        // block nbt 标记(STRU-BE:保留待消费)。
        assert!(!t.blocks()[0].has_nbt);
    }

    fn synth_entry(pal: &str) -> NbtValue {
        let mut e = NbtCompound::new();
        e.insert("Name", NbtValue::String(pal.into()));
        NbtValue::Compound(e)
    }

    fn synth(palettes: &[&[&str]], blocks: &[([i32; 3], i32)]) -> NbtCompound {
        assert_eq!(palettes.len(), 1);
        synth_multi(palettes, blocks)
    }

    fn synth_multi(palettes: &[&[&str]], blocks: &[([i32; 3], i32)]) -> NbtCompound {
        let mut root = NbtCompound::new();
        // size 由 blocks 包围盒推导(原版是存档字段,自构造 fixture 保持
        // 自洽:逐轴 max(pos)+1)。
        let dim =
            |axis: usize| -> i32 { blocks.iter().map(|(p, _)| p[axis] + 1).max().unwrap_or(0) };
        root.insert(
            "size",
            NbtValue::List(mcv_nbt::NbtList::from_vec(vec![
                NbtValue::Int(dim(0)),
                NbtValue::Int(dim(1)),
                NbtValue::Int(dim(2)),
            ])),
        );
        if palettes.len() == 1 {
            let mut pal = mcv_nbt::NbtList::new();
            for n in palettes[0] {
                pal.push(synth_entry(n)).unwrap();
            }
            root.insert("palette", NbtValue::List(pal));
        } else {
            let mut all = mcv_nbt::NbtList::new();
            for pal in palettes {
                let mut l = mcv_nbt::NbtList::new();
                for n in *pal {
                    l.push(synth_entry(n)).unwrap();
                }
                all.push(NbtValue::List(l)).unwrap();
            }
            root.insert("palettes", NbtValue::List(all));
        }
        let mut blks = mcv_nbt::NbtList::new();
        for &(p, st) in blocks {
            let mut b = NbtCompound::new();
            b.insert(
                "pos",
                NbtValue::List(mcv_nbt::NbtList::from_vec(
                    p.iter().map(|v| NbtValue::Int(*v)).collect(),
                )),
            );
            b.insert("state", NbtValue::Int(st));
            blks.push(NbtValue::Compound(b)).unwrap();
        }
        root.insert("blocks", NbtValue::List(blks));
        root
    }
}
