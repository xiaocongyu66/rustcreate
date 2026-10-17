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
/// 敌对七怪各自有专属贴图层）。
pub const fn model_kind(id: mcv_entity::MobId) -> Option<MobModelKind> {
    match id {
        mcv_entity::MobId::CHICKEN => Some(MobModelKind::Chicken),
        mcv_entity::MobId::COW => Some(MobModelKind::Cow),
        mcv_entity::MobId::SHEEP => Some(MobModelKind::Sheep),
        mcv_entity::MobId::PIG => Some(MobModelKind::Pig),
        mcv_entity::MobId::ZOMBIE => Some(MobModelKind::Zombie),
        mcv_entity::MobId::SKELETON => Some(MobModelKind::Skeleton),
        mcv_entity::MobId::CREEPER => Some(MobModelKind::Creeper),
        mcv_entity::MobId::SPIDER => Some(MobModelKind::Spider),
        mcv_entity::MobId::BLAZE => Some(MobModelKind::Blaze),
        mcv_entity::MobId::ENDERMAN => Some(MobModelKind::EnderMan),
        mcv_entity::MobId::CREAKING => Some(MobModelKind::Creaking),
    }
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
    let t = partial.clamp(0.0, 1.0);

    let mut out = Vec::new();
    for (e, kind) in kinds.iter() {
        // kind.0 = MobId（Copy）；模型种类映射在游戏层中转，见 model_kind。
        let Some(kind) = model_kind(kind.0) else {
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
        let pose = MobPose {
            pos: render_pos,
            yaw,
            phase,
            amount,
            head_pitch: 0.0,
            wing_angle,
            head_drop: anims.get(e).map_or(0.0, |a| a.head_drop(eat_ticks)),
            head_eat_angle: anims.get(e).map_or(0.0, |a| a.head_eat_angle(eat_ticks)),
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
        // 被动四怪 + 敌对七怪全部有模型映射（自然生成通道全部 11 种，
        // 见 GameRuntime 的 spawner 分支；M7d 烈焰人/末影人/嘎吱怪入表，
        // 群系/结构门在 spawner 侧，缺门不缺模型）。
        for id in [
            mcv_entity::MobId::CHICKEN,
            mcv_entity::MobId::COW,
            mcv_entity::MobId::SHEEP,
            mcv_entity::MobId::PIG,
            mcv_entity::MobId::ZOMBIE,
            mcv_entity::MobId::SKELETON,
            mcv_entity::MobId::CREEPER,
            mcv_entity::MobId::SPIDER,
            mcv_entity::MobId::BLAZE,
            mcv_entity::MobId::ENDERMAN,
            mcv_entity::MobId::CREAKING,
        ] {
            assert!(model_kind(id).is_some(), "{id:?} 应有模型");
        }
        assert_eq!(mcv_entity::MobId::CHICKEN.def().name, "chicken");
        assert_eq!(mcv_entity::MobId::BLAZE.def().name, "blaze");
        assert_eq!(mcv_entity::MobId::ENDERMAN.def().name, "enderman");
        assert_eq!(mcv_entity::MobId::CREAKING.def().name, "creaking");
    }
}
