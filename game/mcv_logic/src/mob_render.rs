//! mob 渲染拼装：从 mobs ECS World 收集 [`MobInstance`] 供 `Scene.mobs`。
//!
//! 主控接线（已落地）：
//! 1. mcv_app 的 in-game 场景装配处每帧调用 [`collect_mob_instances_in`]
//!    （视距裁剪 = 渲染距离），结果以 `Some(&instances[..])` 喂
//!    `scene.mobs`；第三参 `partial` = 60 Hz 步间残差（partialTick），
//!    怪渲染取 `pos_old→pos` 插值；
//! 2. 贴图上传由 `Renderer::new` 内建（assets 根存在即自动加载 9 层数组）；
//! 3. 动画面：全部怪经 `spawn_mob` 持有 `AnimState`（行走摆动账本，
//!    `mob_ai_system` 每 60 Hz 物理步推进 `advance_walk_anim`）——
//!    此前仅测试侧 passive 路线持有，游戏内恒零相位（僵尸腿不摆根因）。

use glam::Vec3;
use mcv_ecs::World;
// 走 crate 根再导出（MobInstance 是纯数据 POD，不含 wgpu 类型；
// 分层守卫：mcv_logic 不触碰 mcv_render::gpu 内部路径）。
use mcv_render::MobInstance;
use mcv_render::mob_mesh::{MAX_MOB_PARTS, MobModelKind, MobPose, mob_model_matrices};

/// 把 `mcv_entity::MobId` 映射到渲染侧模型种类（缺贴图变体时用 temperate；
/// 敌对四怪各自有专属贴图层）。水生怪带 `variant`（[`mcv_entity::AquaticVariant`]
/// 语义）：美西螈 0..4 → 五贴图种；热带鱼 packed variant → base×pattern
/// 十二型（TropicalFishPatternLayer 贴图展开见 mob_mesh::MOB_TEX_FILES 注）。
pub fn model_kind(id: mcv_entity::MobId, variant: u32) -> Option<MobModelKind> {
    let base = match id {
        mcv_entity::MobId::CHICKEN => MobModelKind::Chicken,
        mcv_entity::MobId::COW => MobModelKind::Cow,
        mcv_entity::MobId::SHEEP => MobModelKind::Sheep,
        mcv_entity::MobId::PIG => MobModelKind::Pig,
        mcv_entity::MobId::ZOMBIE => MobModelKind::Zombie,
        mcv_entity::MobId::SKELETON => MobModelKind::Skeleton,
        mcv_entity::MobId::CREEPER => MobModelKind::Creeper,
        mcv_entity::MobId::SPIDER => MobModelKind::Spider,
        mcv_entity::MobId::COD => MobModelKind::Cod,
        mcv_entity::MobId::SALMON => MobModelKind::Salmon,
        mcv_entity::MobId::AXOLOTL => {
            return Some(match variant {
                1 => MobModelKind::AxolotlWild,
                2 => MobModelKind::AxolotlGold,
                3 => MobModelKind::AxolotlCyan,
                4 => MobModelKind::AxolotlBlue,
                _ => MobModelKind::AxolotlLucy, // DEFAULT = LUCY（越界回缺省）
            });
        }
        mcv_entity::MobId::TROPICAL_FISH => {
            let (large, index) = mcv_entity::aquatic::tropical::texture_slot(variant);
            let root = if large {
                MobModelKind::TropicalLarge0
            } else {
                MobModelKind::TropicalSmall0
            };
            return MobModelKind::from_idx(root.idx() as u32 + u32::from(index));
        }
        _ => return None,
    };
    Some(base)
}

/// 收集全部生物实例（无剔除；量小，数百实体下开销可忽略）。
/// `partial` = 1.0：不插值，直接取最新固定步位置（测试侧便捷面）。
pub fn collect_mob_instances(world: &World) -> Vec<MobInstance> {
    collect_mob_instances_in(world, None, 1.0)
}

/// 带视距裁剪的收集：`center` = 观察眼位，`radius` = 视距（格）。
///
/// `partial` = partialTick ∈ [0,1]（26.1 LivingEntity 渲染位置
/// `lerp(xOld, x, partialTick)` 等价）：渲染帧取 `pos_old→pos` 的插值，
/// 消除固定步边界的位置跳变（此前直接消费 `body.pos` → 渲染帧率 ≠ 步率
/// 时怪瞬移/顿挫，即 #59 Scene.mobs 快照缺插值余量）。调用方把喂步
/// 累加器余量归一化后续入（app.rs `step_accum × 60`）。
pub fn collect_mob_instances_in(
    world: &World,
    view: Option<(Vec3, f32)>,
    partial: f32,
) -> Vec<MobInstance> {
    let kinds = world.read::<mcv_entity::MobKind>();
    let bodies = world.read::<mcv_entity::PhysBody>();
    let yaws = world.read::<mcv_entity::Yaw>();
    let wanders = world.read::<mcv_entity::WanderState>();
    let anims = world.read::<mcv_entity::AnimState>();
    // 水生变体/水位账本（AquaticMode 实体持有；其余实体读空表走缺省）。
    let variants = world.read::<mcv_entity::AquaticVariant>();
    let waters = world.read::<mcv_entity::WaterState>();
    let t = partial.clamp(0.0, 1.0);

    let mut out = Vec::new();
    for (e, kind) in kinds.iter() {
        // kind.0 = MobId（Copy）；模型种类映射在游戏层中转，见 model_kind。
        // 变体优先存水生账本的 packed 值（非水生读 0）。
        let variant = variants.get(e).map_or(0, |v| v.0);
        let Some(kind) = model_kind(kind.0, variant) else {
            continue;
        };
        let Some(body) = bodies.get(e) else {
            continue;
        };
        // 视距裁剪按插值后的渲染位置（观察眼剔除的语义对象是"看到什么"）。
        let render_pos = body.pos_old + (body.pos - body.pos_old) * t;
        if let Some((eye, radius)) = view {
            let d = (render_pos - eye).length();
            if d > radius {
                continue;
            }
        }
        let yaw = yaws.get(e).map(|y| y.0).unwrap_or(0.0);
        // AnimState 与 WanderState 独立读取（此前按元组 match，敌对怪有
        // AnimState 无 WanderState → 整组落零相位兜底——账本推进被消费端
        // 丢弃的第二个根因；spawn_mob 统一装配后两类怪都必须吃同一条）。
        let (phase, amount, wing_angle) = match anims.get(e) {
            Some(a) => (a.phase, a.amount, (a.wing_phase.sin() + 1.0) * a.wing_speed),
            None => (0.0, 0.0, 0.0),
        };
        let eat_ticks = wanders.get(e).map_or(0, |w| w.eat_ticks);
        // 水生面：水位/尾摆相位（WaterState.age_ticks，20 tick/s 摆源）+
        // 鲑鱼尺寸缩放（Salmon.getSalmonScale 0.5/1.0/1.5）。非水生实体
        // 无 WaterState 读 → in_water 缺省 false（陆行怪语义一致）。
        let water = waters.get(e);
        let pose = MobPose {
            pos: render_pos,
            yaw,
            phase,
            amount,
            head_pitch: 0.0,
            wing_angle,
            head_drop: anims.get(e).map_or(0.0, |a| a.head_drop(eat_ticks)),
            head_eat_angle: anims.get(e).map_or(0.0, |a| a.head_eat_angle(eat_ticks)),
            in_water: water.is_some_and(|w| w.in_water),
            swim_phase: water.map_or(0.0, |w| w.age_ticks as f32),
            scale: if kind == MobModelKind::Salmon {
                mcv_entity::aquatic::salmon::scale(variant)
            } else {
                1.0
            },
        };
        let mats = mob_model_matrices(kind, &pose);
        let mut models = [[[0.0f32; 4]; 4]; MAX_MOB_PARTS];
        for (m, dst) in mats.iter().zip(models.iter_mut()) {
            *dst = m.to_cols_array_2d();
        }
        out.push(MobInstance {
            kind: kind.idx() as u32,
            models,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_kind_covers_all_spawnable() {
        // 被动四怪 + 敌对四怪 + 水生四族全部有模型映射（自然生成通道，
        // 见 GameRuntime 的 spawner 分支）。
        for id in [
            mcv_entity::MobId::CHICKEN,
            mcv_entity::MobId::COW,
            mcv_entity::MobId::SHEEP,
            mcv_entity::MobId::PIG,
            mcv_entity::MobId::ZOMBIE,
            mcv_entity::MobId::SKELETON,
            mcv_entity::MobId::CREEPER,
            mcv_entity::MobId::SPIDER,
            mcv_entity::MobId::COD,
            mcv_entity::MobId::SALMON,
            mcv_entity::MobId::TROPICAL_FISH,
            mcv_entity::MobId::AXOLOTL,
        ] {
            assert!(model_kind(id, 0).is_some(), "{id:?} 应有模型");
        }
    }

    #[test]
    fn aquatic_variant_kinds_disjoint() {
        // 美西螈五变体 → 互异种类；热带鱼 12 型边界都不越过 MOB_KIND_COUNT。
        let mut kinds = std::collections::HashSet::new();
        for v in 0..5u32 {
            let k = model_kind(mcv_entity::MobId::AXOLOTL, v).unwrap();
            assert!(k.idx() < mcv_render::mob_mesh::MOB_KIND_COUNT);
            kinds.insert(k.idx());
        }
        assert_eq!(kinds.len(), 5);
        // 热带鱼：small/large × index 0..5 → 12 型（packed 侧由
        // mcv_entity::aquatic::tropical 保真，这里锁边界）。
        for idx in 0..6u8 {
            let packed = mcv_entity::aquatic::tropical::pack(idx, false, 0, 0);
            let k = model_kind(mcv_entity::MobId::TROPICAL_FISH, packed).unwrap();
            assert_eq!(k.idx(), MobModelKind::TropicalSmall0.idx() + idx as usize);
            let packed = mcv_entity::aquatic::tropical::pack(idx, true, 0, 0);
            let k = model_kind(mcv_entity::MobId::TROPICAL_FISH, packed).unwrap();
            assert_eq!(k.idx(), MobModelKind::TropicalLarge0.idx() + idx as usize);
        }
    }
}
