//! 结构模板运行时（StructureTemplate 机制的 clean-room 转写）。
//!
//! 消费 mcv_nbt 读原版结构 `.nbt`（26.1 data 树实物格式:根 compound
//! `size`(IntList 3)/`blocks`(List of `{pos: IntList3, state: Int, nbt?}`)/
//! `palette`(List of `{Name, Properties?}`)/`entities`/`DataVersion`；
//! 旧世代文件的 `palettes` 多调色板变体只取第 0 个,登记 STRU-1）。
//!
//! `place_at` 复刻 26.1 掷骰序实物:变换用 `StructureTemplate.transform`
//! （先 mirror 后 rotation,pivot 公式照抄）;air 不放置对应
//! `BlockIgnoreProcessor.STRUCTURE_AND_AIR`；integrity 对应
//! `BlockRotProcessor`:逐 block(模板序)`random.nextFloat() <= integrity`
//! 保留,否则丢弃。
//!
//! 登记的债:
//! - STRU-RNG:原版 `StructurePlaceSettings.getRandom(pos)` 在有 random 时
//!   共享单流、无 random 时按 `Mth.getSeed(pos)` 逐位播种;本刀用单流
//!   SplitMix64(seed),同 seed 两次放置逐字节一致(测试锁定),但与原版
//!   Xoroshiro128++ 不位同。
//! - STRU-STATE:mirror/rotation 只变换 block pos;palette 条目 Name 的
//!   facing 等 blockstate 属性旋转（原版 `BlockState.mirror/rotate` 经
//!   BlockRotationUtil）未实装——ruined_portal 主要受影响的楼梯/熔接器
//!   朝向会保持模板原向。
//! - STRU-BE:block `nbt`(方块实体)与 entities 读取后忽略。
//! - STRU-LOOKUP:Name→BlockId 线性查 `mcv_core::BLOCKS`;刀 7 统一换
//!   名称→索引表。

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
}

/// 结构模板运行时。
#[derive(Debug, Clone)]
pub struct Template {
    /// (width=x, height=y, length=z)。
    pub size: [i32; 3],
    /// 模板序方块列表（原版 blocks+palette 展开）。
    pub blocks: Vec<TemplateBlock>,
    /// palette 顺序解析表（插入序即文件序）。
    pub palette: Vec<BlockId>,
    /// 未识别方块名个数（解析时计数,>0 时 warn 一次）。
    pub unknown_names: u32,
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

    /// 从已解析 compound 构造。
    pub fn from_compound(root: &NbtCompound) -> Result<Template, TemplateError> {
        let size = root
            .get("size")
            .and_then(int3_of)
            .ok_or(TemplateError::BadField("size"))?;
        // `palettes` 多调色板变体只取第 0 个（STRU-1）。
        let palette_tag: &[NbtValue] = if let Some(l) = root.get_list("palette") {
            l.items()
        } else if let Some(all) = root.get_list("palettes") {
            all.get(0)
                .and_then(NbtValue::as_list)
                .map_or(&[][..], |l| l.items())
        } else {
            &[][..]
        };
        let mut palette = Vec::with_capacity(palette_tag.len());
        let mut unknown = 0u32;
        for entry in palette_tag {
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
        let blocks_tag = root
            .get_list("blocks")
            .ok_or(TemplateError::BadField("blocks"))?;
        let mut blocks = Vec::new();
        for b in blocks_tag.items() {
            let Some(c) = b.as_compound() else {
                continue;
            };
            let Some(pos) = c.get("pos").and_then(int3_of) else {
                continue;
            };
            let idx = c.get_i32("state").unwrap_or(0);
            // 越界索引 → 空气（原版 palette.stateFor 缺省回落 first状态,
            // 保守取空气更安全;偏差随 STRU-1 登记）。
            let block = usize::try_from(idx)
                .ok()
                .and_then(|i| palette.get(i).copied())
                .unwrap_or_else(air_id);
            blocks.push(TemplateBlock { pos, block });
        }
        if unknown > 0 {
            // 未知方块名 → 空气,整模板只 warn 一次（指令口径）。
            log::warn!("structure template: {unknown} 个未知方块名按空气处理");
        }
        Ok(Template {
            size,
            blocks,
            palette,
            unknown_names: unknown,
        })
    }

    /// 放置:变换(mirror→rotate,绕 pivot)+ anchor 平移 + integrity 掷骰 +
    /// air 不放置。返回世界坐标方块列表(模板序,integrity 掷骰同 seed 确定)。
    #[must_use]
    pub fn place_at(
        &self,
        anchor: [i32; 3],
        pivot: [i32; 3],
        mirror: Mirror,
        rotation: Rotation,
        integrity: f32,
        seed: u64,
    ) -> Vec<(i32, i32, i32, BlockId)> {
        let air = air_id();
        let mut rng = PlaceRng::new(seed);
        let mut out = Vec::with_capacity(self.blocks.len());
        for b in &self.blocks {
            // BlockRotProcessor 掷骰序实物:processBlockInfos 阶段模板序
            // 逐 block 一掷(air 也消耗掷骰——ignore 在其后的处理器链),
            // `nextFloat() <= integrity` 保留。
            let roll = rng.next_f32();
            // BlockIgnoreProcessor.STRUCTURE_AND_AIR 语义:air 不放置。
            if b.block == air || roll > integrity {
                continue;
            }
            let t = transform(b.pos, mirror, rotation, pivot);
            out.push((
                t[0] + anchor[0],
                t[1] + anchor[1],
                t[2] + anchor[2],
                b.block,
            ));
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

/// Name → BlockId。剥 `minecraft:` 前缀,线性查注册表(STRU-LOOKUP)。
#[must_use]
pub fn block_by_name(name: &str) -> Option<BlockId> {
    let bare = name.strip_prefix("minecraft:").unwrap_or(name);
    // 带属性串(如 "stone[slab=bottom]")按基名解析——属性旋转未实装
    // (STRU-STATE),palette Name 实物也不带方括号属性。
    let base = bare.split('[').next().unwrap_or(bare);
    BLOCKS
        .iter()
        .position(|d| d.name == base)
        .map(|i| BlockId(i as u16))
}

fn air_id() -> BlockId {
    block_by_name("air").unwrap_or(BlockId(0))
}

/// 放置掷骰流:SplitMix64(与 lake_pass LakeRng 同族先例,见 STRU-RNG)。
struct PlaceRng {
    s: u64,
}

impl PlaceRng {
    const fn new(seed: u64) -> Self {
        Self { s: seed }
    }

    fn next_u64(&mut self) -> u64 {
        let mut z = self
            .s
            .wrapping_add(0x9E37_79B9_7F4A_7C15)
            .wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        z = (z ^ (z >> 27)).wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        self.s = z ^ (z >> 31);
        self.s
    }

    /// [0,1) 单精度(原版 nextFloat 同精度面)。
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }
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

    #[test]
    fn synthetic_template_roundtrip_and_place() {
        // 不依赖资产的自构造模板:2x1x2,palette [stone, air]。
        let mut root = NbtCompound::new();
        root.insert(
            "size",
            NbtValue::List(mcv_nbt::NbtList::from_vec(vec![
                NbtValue::Int(2),
                NbtValue::Int(1),
                NbtValue::Int(2),
            ])),
        );
        let mut pal = mcv_nbt::NbtList::new();
        for n in ["minecraft:stone", "minecraft:air"] {
            let mut e = NbtCompound::new();
            e.insert("Name", NbtValue::String(n.into()));
            pal.push(NbtValue::Compound(e)).unwrap();
        }
        root.insert("palette", NbtValue::List(pal));
        let mut blocks = mcv_nbt::NbtList::new();
        for (i, p) in [[0, 0, 0], [1, 0, 0], [0, 0, 1], [1, 0, 1]]
            .iter()
            .enumerate()
        {
            let mut b = NbtCompound::new();
            b.insert(
                "pos",
                NbtValue::List(mcv_nbt::NbtList::from_vec(
                    p.iter().map(|v| NbtValue::Int(*v)).collect(),
                )),
            );
            // 第 4 格放 air(palette 1),验证不放置。
            b.insert("state", NbtValue::Int(if i == 3 { 1 } else { 0 }));
            blocks.push(NbtValue::Compound(b)).unwrap();
        }
        root.insert("blocks", NbtValue::List(blocks));

        let t = Template::from_compound(&root).unwrap();
        assert_eq!(t.size, [2, 1, 2]);
        assert_eq!(t.blocks.len(), 4);
        assert!(
            t.palette
                .iter()
                .all(|b| *b == block_by_name("stone").unwrap() || *b == air_id())
        );

        // 同 seed 两次一致;air 不放置。
        let a = t.place_at(
            [10, 20, 30],
            [0, 0, 0],
            Mirror::None,
            Rotation::None,
            1.0,
            42,
        );
        let b = t.place_at(
            [10, 20, 30],
            [0, 0, 0],
            Mirror::None,
            Rotation::None,
            1.0,
            42,
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 3, "air 不放置");
        // integrity=-0.5 → 数学保证全丢（roll ≥ 0 > -0.5）。
        assert!(
            t.place_at([0, 0, 0], [0, 0, 0], Mirror::None, Rotation::None, -0.5, 7)
                .is_empty()
        );

        // 90° 旋转:局部包围盒 x/z 互换律——用沿 x 轴的 3x1x1 直条。
        let mut root2 = NbtCompound::new();
        root2.insert(
            "size",
            NbtValue::List(mcv_nbt::NbtList::from_vec(vec![
                NbtValue::Int(3),
                NbtValue::Int(1),
                NbtValue::Int(1),
            ])),
        );
        let mut pal2 = mcv_nbt::NbtList::new();
        let mut pe = NbtCompound::new();
        pe.insert("Name", NbtValue::String("minecraft:stone".into()));
        pal2.push(NbtValue::Compound(pe)).unwrap();
        root2.insert("palette", NbtValue::List(pal2));
        let mut blks2 = mcv_nbt::NbtList::new();
        for x in 0..3 {
            let mut b = NbtCompound::new();
            b.insert(
                "pos",
                NbtValue::List(mcv_nbt::NbtList::from_vec(vec![
                    NbtValue::Int(x),
                    NbtValue::Int(0),
                    NbtValue::Int(0),
                ])),
            );
            b.insert("state", NbtValue::Int(0));
            blks2.push(NbtValue::Compound(b)).unwrap();
        }
        root2.insert("blocks", NbtValue::List(blks2));
        let t2 = Template::from_compound(&root2).unwrap();
        let plain = t2.place_at([0, 0, 0], [0, 0, 0], Mirror::None, Rotation::None, 1.0, 1);
        let rot = t2.place_at(
            [0, 0, 0],
            [0, 0, 0],
            Mirror::None,
            Rotation::Clockwise90,
            1.0,
            1,
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
}
