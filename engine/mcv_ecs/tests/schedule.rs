//! 调度/事件/命令/资源/插件五件套的行为测试:固定步累加与追帧上限、
//! 注册顺序、事件阶段边界翻转、结构性变更延迟到阶段末、单步服务注入、
//! 插件装配。

use mcv_ecs::{App, DEFAULT_FIXED_DT, MAX_CATCHUP_STEPS, Plugin, Stage};

#[derive(Clone, Copy, PartialEq, Debug)]
struct Pos(f32);

#[derive(Default)]
struct Ticks(u64);

#[derive(Default)]
struct Order(Vec<&'static str>);

const DT: f32 = DEFAULT_FIXED_DT;

#[test]
fn fixed_stage_accumulates_and_catchup_is_bounded() {
    let mut app = App::new();
    // 步长取 1s:累加全在二进制下精确,断言无浮点边界。
    app.schedule = mcv_ecs::Schedule::new().with_fixed_dt(1.0);
    app.init_resource::<Ticks>();
    app.add_system(Stage::Fixed, "tick", |ctx: &mut mcv_ecs::SysCtx| {
        ctx.resources.get_mut::<Ticks>().unwrap().0 += 1;
    });
    // 2.5 个固定步 → 执行 2,余额滚入下一帧。
    assert_eq!(app.update(2.5), 2);
    assert_eq!(app.resources.get::<Ticks>().unwrap().0, 2);
    assert_eq!(app.update(0.5), 1);
    assert_eq!(app.resources.get::<Ticks>().unwrap().0, 3);
    // 卡顿追帧上限:再大的 dt 也只跑 MAX_CATCHUP 步,且不欠债。
    assert_eq!(app.update(100.0), MAX_CATCHUP_STEPS);
    let after = app.resources.get::<Ticks>().unwrap().0;
    assert_eq!(app.update(0.6), 0, "100s 的历史债务必须被丢弃");
    assert_eq!(app.resources.get::<Ticks>().unwrap().0, after);
}

#[test]
fn systems_run_in_registration_order_fixed_before_variable() {
    let mut app = App::new();
    app.init_resource::<Order>();
    for (name, stage) in [
        ("var", Stage::Variable),
        ("fix_a", Stage::Fixed),
        ("fix_b", Stage::Fixed),
    ] {
        app.add_system(stage, name, move |ctx: &mut mcv_ecs::SysCtx| {
            ctx.resources.get_mut::<Order>().unwrap().0.push(name);
        });
    }
    app.update(DT);
    assert_eq!(
        app.resources.get::<Order>().unwrap().0,
        vec!["fix_a", "fix_b", "var"]
    );
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Boom(u32);

#[test]
fn events_are_visible_next_stage_and_expire() {
    let mut app = App::new();
    app.init_resource::<Vec<Boom>>();
    // 每个固定步发一颗雷;可变帧收集当帧可见批次。
    app.add_system(Stage::Fixed, "send", |ctx: &mut mcv_ecs::SysCtx| {
        ctx.events.channel::<Boom>().send(Boom(1));
    });
    app.add_system(Stage::Variable, "collect", |ctx: &mut mcv_ecs::SysCtx| {
        let seen: Vec<Boom> = ctx.events.channel::<Boom>().read().copied().collect();
        *ctx.resources.get_mut::<Vec<Boom>>().unwrap() = seen;
    });
    app.update(DT); // 1 固定步 → 事件翻到读侧,可变帧恰好看到 1 颗
    assert_eq!(*app.resources.get::<Vec<Boom>>().unwrap(), vec![Boom(1)]);
    app.update(0.0); // 无固定步:上一批已过期,读侧为空
    assert!(app.resources.get::<Vec<Boom>>().unwrap().is_empty());
}

#[test]
fn deferred_spawn_and_despawn_apply_at_stage_end() {
    let mut app = App::new();
    app.world.register::<Pos>();
    // 形态复刻 mob AI:for_each 迭代期间只排队 despawn,绝不触碰 World。
    app.add_system(Stage::Fixed, "life_cycle", |ctx: &mut mcv_ecs::SysCtx| {
        if ctx.world.is_empty() {
            ctx.commands.spawn_with(|w, e| w.insert(e, Pos(1.0)));
            return;
        }
        let mut phys = ctx.world.write::<Pos>();
        phys.for_each(|e, p| {
            p.0 += 1.0;
            if p.0 >= 2.0 {
                ctx.commands.despawn(e);
            }
        });
    });
    app.update(DT); // spawn 命令在阶段末生效
    assert_eq!(app.world.len(), 1);
    assert_eq!(app.world.component_count::<Pos>(), 1);
    app.update(DT); // Pos=2.0 → 排队 despawn → 阶段末生效
    assert_eq!(app.world.len(), 0);
    assert_eq!(app.world.component_count::<Pos>(), 0);
}

#[test]
fn host_state_snapshot_reaches_systems() {
    let mut app = App::new();
    // 整组替换 schedule 会清掉已注册系统:先换步长,再注册。
    app.schedule = mcv_ecs::Schedule::new().with_fixed_dt(1.0);
    app.resources.insert(2.0f32); // 单例资源:配置
    app.resources.insert(Vec::<u32>::new()); // 系统侧收集器
    app.add_system(Stage::Fixed, "use_snapshot", |ctx: &mut mcv_ecs::SysCtx| {
        let gain = *ctx.resources.get::<f32>().unwrap();
        ctx.resources
            .get_mut::<Vec<u32>>()
            .unwrap()
            .push(gain as u32);
    });
    assert_eq!(app.update(2.5), 2);
    assert_eq!(*app.resources.get::<Vec<u32>>().unwrap(), vec![2, 2]);
}

struct PingPlugin;

impl Plugin for PingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Ticks>();
        app.add_system(Stage::Fixed, "ping", |ctx: &mut mcv_ecs::SysCtx| {
            ctx.resources.get_mut::<Ticks>().unwrap().0 += 10;
        });
    }
}

#[test]
fn plugin_assembles_and_plain_plugin_runs_once() {
    let mut app = App::new();
    app.add_plugin(PingPlugin);
    app.add_plugin(mcv_ecs::plain_plugin(|app| {
        app.init_resource::<Order>();
    }));
    app.update(DT);
    assert_eq!(app.resources.get::<Ticks>().unwrap().0, 10);
    assert!(app.resources.get::<Order>().is_some());
}
