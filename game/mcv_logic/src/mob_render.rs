//! mob 渲染拼装：从 mobs ECS World 收集 [`MobInstance`] 供 `Scene.mobs`。
//!
//! 主控接线（已落地）：
//! 1. mcv_app 的 in-game 场景装配处每帧调用 [`collect_mob_instances_in`]（视距
//!    裁剪 = 渲染距离），结果以 `Some(&instances[..])` 喂 `scene.mobs`；
//! 2. 贴图上传由 `Renderer::new` 内建（assets 根存在即自动加载 9 层数组）；
//! 3. 依赖面：被收集的 World 须注册被动组件（WanderState/AnimState——
//!    read 未注册类型会 panic），GameRuntime 构造时统一注册。敌对怪
//!    不持有这两组件，姿态走零相位静止路径，不 panic。

use glam::Vec3;
use mcv_ecs::World;
// 走 crate 根再导出（MobInstance 是纯数据 POD，不含 wgpu 类型；
// 分层守卫：mcv_logic 不触碰 mcv_render::gpu 内部路径）。
use mcv_render::MobInstance;
use mcv_render::mob_mesh::{MAX_MOB_PARTS, MobModelKind, MobPose, mob_model_matrices};

/// 把 `mcv_entity::MobId` 映射到渲染侧模型种类（缺贴图变体时用 temperate；
/// 敌对四怪各自有专属贴图层）。
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
        _ => None,
    }
}

/// 收集全部生物实例（无剔除；量小，数百实体下开销可忽略）。
pub fn collect_mob_instances(world: &World) -> Vec<MobInstance> {
    collect_mob_instances_in(world, None)
}

/// 带视距裁剪的收集：`center` = 观察眼位，`radius` = 视距（格）。
pub fn collect_mob_instances_in(world: &World, view: Option<(Vec3, f32)>) -> Vec<MobInstance> {
    let kinds = world.read::<mcv_entity::MobKind>();
    let bodies = world.read::<mcv_entity::PhysBody>();
    let yaws = world.read::<mcv_entity::Yaw>();
    let wanders = world.read::<mcv_entity::WanderState>();
    let anims = world.read::<mcv_entity::AnimState>();

    let mut out = Vec::new();
    for (e, kind) in kinds.iter() {
        // kind.0 = MobId（Copy）；模型种类映射在游戏层中转，见 model_kind。
        let Some(kind) = model_kind(kind.0) else {
            continue;
        };
        let Some(body) = bodies.get(e) else {
            continue;
        };
        if let Some((eye, radius)) = view {
            let d = (body.pos - eye).length();
            if d > radius {
                continue;
            }
        }
        let yaw = yaws.get(e).map(|y| y.0).unwrap_or(0.0);
        let (phase, amount, wing) = match (anims.get(e), wanders.get(e)) {
            (Some(a), Some(w)) => (
                a.phase,
                a.amount,
                ((a.wing_phase.sin() + 1.0) * a.wing_speed, w.eat_ticks),
            ),
            _ => (0.0, 0.0, (0.0, 0)),
        };
        let pose = MobPose {
            pos: body.pos,
            yaw,
            phase,
            amount,
            head_pitch: 0.0,
            wing_angle: wing.0,
            head_drop: anims.get(e).map_or(0.0, |a| a.head_drop(wing.1)),
            head_eat_angle: anims.get(e).map_or(0.0, |a| a.head_eat_angle(wing.1)),
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
        // 被动四怪 + 敌对四怪全部有模型映射（自然生成通道只出这八种，
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
        ] {
            assert!(model_kind(id).is_some(), "{id:?} 应有模型");
        }
    }
}
