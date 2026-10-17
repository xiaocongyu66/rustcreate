//! 总线图:树拓扑 / 增益级联 / send 路由 / 位级等价 / 水下低通预留位。
//!
//! 大部分测试**不依赖素材**(直接合成 PCM 驱动 [`Mixer`],与 pitch.rs 同路);
//! 末尾一个走 `common` fixture 验证 `SoundLoader` 的落轨接线(需先
//! `bash ci/fetch-sounds.sh`,CI 的 test job 必跑)。混音等价断言全部是
//! **位级**(f32 逐样本相等)——总线全默认时必须与无总线实现逐位相同。
mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use common::SoundFixture;
use mcv_audio::{AudioCmd, BusId, BusMix, Mixer, SoundData, SoundLoader};

const OUT_RATE: u32 = 44100;
const BLOCK: usize = 1024;
const BLOCKS: usize = 4;
const FRAMES: usize = BLOCKS * BLOCK + 16;

/// 合成立体声 PCM(直流偏置避免 ±0.0 胶点;与混音器同率 → 整数步进,
/// 插值 frac=0 位确定:第 k 个解码样本恰等于 a[2k]/a[2k+1])。
fn synth() -> Arc<SoundData> {
    let mut samples = Vec::with_capacity(FRAMES * 2);
    for i in 0..FRAMES {
        let t = i as f32;
        // 双正弦异频,幅度留限幅余量。
        let l = (t * 0.031).sin() * 0.6 + 0.10;
        let r = (t * 0.017).sin() * 0.45 + 0.05;
        samples.push(l);
        samples.push(r);
    }
    Arc::new(SoundData {
        sample_rate: OUT_RATE,
        frames: FRAMES,
        samples,
    })
}

/// 直构起声部命令(落轨显式给定;voice.gain 固定 0.7)。
fn play(bus: BusId, sound: &Arc<SoundData>, gain: f32) -> AudioCmd {
    AudioCmd::Play {
        key: "synth".into(),
        sound: Arc::clone(sound),
        gain,
        pitch: 1.0,
        looping: false,
        loop_event: None,
        bus,
    }
}

/// 无总线混音器(旧行为本体)。
fn mixer_plain(master: f32) -> Mixer {
    let (_producer, consumer) = mcv_sync::Spsc::<AudioCmd>::new(16).split();
    Mixer::new(
        OUT_RATE,
        consumer,
        Arc::new(AtomicU32::new(master.to_bits())),
    )
}

/// 挂总线图混音器 + 共享 BusMix + 根 master 原子——三者经同一 Arc 连接
/// (与 AudioManager::open 的接线逐位同构;全默认:各轨 gain=1.0 / 无 send)。
fn mixer_bused(master: f32) -> (Mixer, Arc<BusMix>, Arc<AtomicU32>) {
    let master = Arc::new(AtomicU32::new(master.to_bits()));
    let buses = Arc::new(BusMix::new(Arc::clone(&master)));
    let (_producer, consumer) = mcv_sync::Spsc::<AudioCmd>::new(16).split();
    let m = Mixer::new(OUT_RATE, consumer, Arc::clone(&master)).with_buses(Arc::clone(&buses));
    (m, buses, master)
}

/// 混 `BLOCKS` 块,拼成一条输出序列。
fn mix_n(m: &mut Mixer) -> Vec<f32> {
    let mut buf = vec![0f32; BLOCK * 2];
    let mut acc = Vec::with_capacity(FRAMES * 2);
    // 覆盖 FRAMES = BLOCKS*BLOCK + 16 的完整采样窗（尾缝 16 帧；多混的
    // 静音段即缓冲自然填充：断言按 FRAMES 索引也不越界——CI run
    // 39052495900 三测「len 8192 index 8192」的根因即欠 16 帧尾缝）。
    for _ in 0..(BLOCKS + 2) {
        m.mix(&mut buf);
        acc.extend_from_slice(&buf);
    }
    acc
}

// ---- 树拓扑 ----

#[test]
fn tree_topology_is_master_over_three_tracks() {
    // 三轨完备性与声明序:master 的孩子恰好是 music/sfx/ambient。
    assert_eq!(
        BusId::TRACKS,
        [BusId::Music, BusId::Sfx, BusId::Ambient],
        "树拓扑单点真相被改动——须同步任务书与文档"
    );
    // 父链:三轨父恒为根;track_idx 与 TRACKS 下标互逆。
    for &t in &BusId::TRACKS {
        assert!(t.is_track(), "{} 必须是轨", t.name());
        assert_eq!(t.track_idx().map(|i| BusId::TRACKS[i]), Some(t));
        assert_eq!(t.parent(), Some(BusId::Master), "{} 的父必须为根", t.name());
    }
    // 根:汇流点,不挂声部、无父。
    assert!(!BusId::Master.is_track());
    assert!(BusId::Master.track_idx().is_none());
    assert_eq!(BusId::Master.parent(), None);
    // 全默认链增益恒 1.0(位级等价的支点)。
    let buses = BusMix::new(Arc::new(AtomicU32::new(1.0f32.to_bits())));
    for &t in &BusId::TRACKS {
        assert_eq!(buses.chain(t), 1.0, "{} 默认链必须为 1.0", t.name());
    }
    assert_eq!(buses.chain(BusId::Master), 1.0);
}

// ---- 增益级联 ----

#[test]
fn bus_gain_cascades_along_root_path() {
    let sound = synth();
    let (mut m, buses, _root) = mixer_bused(1.0);
    // music 0.5、ambient 0.25:落音乐轨的声部只乘 music 链,他轨增益无关。
    buses.set_gain(BusId::Music, 0.5);
    buses.set_gain(BusId::Ambient, 0.25);
    assert!(m.apply(play(BusId::Music, &sound, 0.7)));

    let out = mix_n(&mut m);
    // 期望照抄混音序:home = voice.gain × master;级联 = home × chain(落轨)。
    for k in 0..FRAMES {
        let l = sound.samples[k * 2];
        let r = sound.samples[k * 2 + 1];
        let home = 0.7f32 * 1.0;
        let g = home * buses.chain(BusId::Music);
        assert_eq!(out[k * 2], l * g, "L 帧 {k}:级联必须按公式精确生效");
        assert_eq!(out[k * 2 + 1], r * g, "R 帧 {k}:级联必须按公式精确生效");
    }
}

#[test]
fn bus_master_gain_writes_the_root_atomic() {
    // BusId::Master 的 set/gain 就是根原子读写,且混音器同一 Arc 可见
    // (单点写;若 BusMix 与 Mixer 的根 Arc 分离,下面的混音数值必红)。
    let sound = synth();
    let (mut m, buses, root) = mixer_bused(1.0);
    buses.set_gain(BusId::Master, 0.6);
    assert_eq!(buses.gain(BusId::Master), 0.6);
    assert_eq!(
        f32::from_bits(root.load(std::sync::atomic::Ordering::Relaxed)),
        0.6
    );
    // 端到端:混音端读到的根增益也变成 0.6(声明序 home = 0.7 × 0.6)。
    assert!(m.apply(play(BusId::Sfx, &sound, 0.7)));
    let out = mix_n(&mut m);
    for k in 0..FRAMES {
        let l = sound.samples[k * 2];
        let r = sound.samples[k * 2 + 1];
        let g = (0.7f32 * 0.6) * 1.0; // home × chain(Sfx)=1.0
        assert_eq!(out[k * 2], l * g, "L 帧 {k}:根增益必须被混音端看到");
        assert_eq!(out[k * 2 + 1], r * g, "R 帧 {k}:根增益必须被混音端看到");
    }
}

#[test]
fn non_finite_bus_config_is_rejected() {
    let (_m, buses, root) = mixer_bused(1.0);
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        buses.set_gain(BusId::Sfx, bad);
        assert_eq!(buses.gain(BusId::Sfx), 1.0, "{bad} 必须被拒(旧值保留)");
        buses.set_send(BusId::Music, BusId::Ambient, bad);
        assert_eq!(
            buses.send_gain(BusId::Music, BusId::Ambient),
            0.0,
            "{bad} 必须被拒"
        );
    }
    // 根同防线(坏值不覆盖既有 master)。
    buses.set_gain(BusId::Master, f32::NAN);
    assert_eq!(
        buses.gain(BusId::Master),
        f32::from_bits(root.load(std::sync::atomic::Ordering::Relaxed))
    );
}

// ---- send 路由 ----

#[test]
fn send_routes_a_share_of_home_path_to_target_bus() {
    let sound = synth();
    let (mut m, buses, _root) = mixer_bused(1.0);
    buses.set_send(BusId::Music, BusId::Ambient, 0.25);
    buses.set_gain(BusId::Ambient, 0.5);
    assert!(m.apply(play(BusId::Music, &sound, 0.7)));

    let out = mix_n(&mut m);
    // 后置 send:家庭信号(voice.gain×master×chain(music))旁路一份,
    // 乘 send 增益 × 目标链(ambient);根不二次乘;其余轨不动。
    for k in 0..FRAMES {
        let l = sound.samples[k * 2];
        let r = sound.samples[k * 2 + 1];
        let home = 0.7f32 * 1.0 * 1.0; // × master × chain(Music)=1.0
        let g_send = (home * 0.25) * buses.chain(BusId::Ambient); // ×send ×0.5
        assert_eq!(
            out[k * 2],
            (l * home) + (l * g_send),
            "L 帧 {k}:send 路由公式"
        );
        assert_eq!(
            out[k * 2 + 1],
            (r * home) + (r * g_send),
            "R 帧 {k}:send 路由公式"
        );
    }
}

#[test]
fn send_zero_and_clear_disconnect_the_route() {
    let sound = synth();
    // 参照输出:未接 send 的同声部。
    let (mut ref_m, _b_ref, _r_ref) = mixer_bused(1.0);
    assert!(ref_m.apply(play(BusId::Music, &sound, 0.7)));
    let expected = mix_n(&mut ref_m);

    // send 接→由 0.0 撤线:必须回到与未接 send 逐位相同的输出。
    let (mut m, buses, _r) = mixer_bused(1.0);
    buses.set_send(BusId::Music, BusId::Ambient, 0.3);
    buses.set_send(BusId::Music, BusId::Ambient, 0.0);
    assert!(m.apply(play(BusId::Music, &sound, 0.7)));
    assert_eq!(mix_n(&mut m), expected, "send=0.0 必须等价未接");

    // 接 2 条 → clear_sends 整行撤:同上,位级复原。
    let (mut m, buses, _r) = mixer_bused(1.0);
    buses.set_send(BusId::Music, BusId::Ambient, 0.3);
    buses.set_send(BusId::Music, BusId::Sfx, 0.9);
    buses.clear_sends(BusId::Music);
    assert!(m.apply(play(BusId::Music, &sound, 0.7)));
    assert_eq!(mix_n(&mut m), expected, "clear_sends 必须等价未接");

    // 跨轨互不影响:clear(Music) 不伤 Sfx→Ambient。
    let (_m, buses, _r) = mixer_bused(1.0);
    buses.set_send(BusId::Sfx, BusId::Ambient, 0.5);
    buses.set_send(BusId::Music, BusId::Ambient, 0.5);
    buses.clear_sends(BusId::Music);
    assert_eq!(buses.send_gain(BusId::Sfx, BusId::Ambient), 0.5);
    assert_eq!(buses.send_gain(BusId::Music, BusId::Ambient), 0.0);
}

#[test]
fn send_guards_self_and_root_targets() {
    let buses = BusMix::new(Arc::new(AtomicU32::new(1.0f32.to_bits())));
    buses.set_send(BusId::Music, BusId::Music, 0.5);
    buses.set_send(BusId::Master, BusId::Ambient, 0.5);
    buses.set_send(BusId::Music, BusId::Master, 0.5);
    for from in &BusId::TRACKS {
        for to in &BusId::TRACKS {
            let wired = (*from, *to) == (BusId::Sfx, BusId::Ambient);
            assert_eq!(
                buses.send_gain(*from, *to),
                if wired { 0.5 } else { 0.0 },
                "{from:?}→{to:?} 只有 Sfx→Ambient 那条应接线成功"
            );
        }
    }
}

// ---- 位级等价(全默认 = 无总线) ----

#[test]
fn default_bus_graph_is_bitwise_identity_to_no_bus() {
    let sound = synth();
    for master in [1.0f32, 0.6] {
        // 旧行为:未挂总线,落 Master(即无路由声部)。
        let mut a = mixer_plain(master);
        assert!(a.apply(play(BusId::Master, &sound, 0.7)));
        // 新行为:全默认总线图,落 Sfx(默认轨)。
        let (mut b, buses, _r) = mixer_bused(master);
        assert!(b.apply(play(BusId::Sfx, &sound, 0.7)));
        assert_eq!(buses.gain(BusId::Music), 1.0);
        // 落 Sfx 且带默认总线 —— 必须与旧行为逐位相同。
        let out_a = mix_n(&mut a);
        let out_b = mix_n(&mut b);
        assert_eq!(out_a, out_b, "全默认总线(master={master})必须位级等价");

        // 未挂总线时 bus 字段一律旁路(级联不存在):
        let mut c = mixer_plain(master);
        assert!(c.apply(play(BusId::Music, &sound, 0.7)));
        assert_eq!(mix_n(&mut c), out_a, "无总线混音器不得理会 bus 字段");
    }
}

// ---- 水下低通效果位(预留) ----

#[test]
fn lowpass_bit_is_reserved_state_not_dsp() {
    let sound = synth();
    let (mut m_off, buses, _r) = mixer_bused(1.0);
    assert!(!buses.lowpass(BusId::Ambient), "默认必须 false");
    assert!(!buses.lowpass(BusId::Master), "根没有低通位");
    buses.set_lowpass(BusId::Ambient, true);
    assert!(buses.lowpass(BusId::Ambient));
    buses.set_lowpass(BusId::Master, true);
    assert!(!buses.lowpass(BusId::Master), "根写 no-op");

    // 预留位语义:置位前后混音输出逐位相同(DSP 未接;本波只存态)。
    assert!(m_off.apply(play(BusId::Ambient, &sound, 0.7)));
    let out_off = mix_n(&mut m_off);
    let (mut m_on, buses, _r) = mixer_bused(1.0);
    buses.set_lowpass(BusId::Ambient, true);
    assert!(m_on.apply(play(BusId::Ambient, &sound, 0.7)));
    assert_eq!(mix_n(&mut m_on), out_off, "低通位居位不得影响本波输出");
}

// ---- 接线:事件名分类落轨(loader)+ 静音模式配置面 ----

/// 仅三个事件,变体都来自真实树(common 已复制)。
const BUS_JSON: &str = r#"{
  "block.grass.step": {"sounds": ["dig/grass1"]},
  "music.menu": {"sounds": ["dig/wood1"]},
  "ambient.underwater.loop": {"sounds": ["dig/stone1"]}
}"#;

#[test]
fn loader_routes_events_to_buses_by_event_name() {
    let fx = SoundFixture::with_json(BUS_JSON);
    let mut loader = SoundLoader::new(fx.dir());
    let t = std::time::Instant::now();
    let dist = ([0.0f32; 3], [0.0f32; 3]);

    let cmd = loader
        .play_at("block.grass.step", dist.0, dist.1, 1.0, t)
        .expect("方块事件必须起命令");
    assert!(matches!(
        cmd,
        AudioCmd::Play {
            bus: BusId::Sfx,
            ..
        }
    ));

    let cmd = loader
        .play_at("music.menu", dist.0, dist.1, 1.0, t)
        .expect("音乐事件必须起命令");
    assert!(matches!(
        cmd,
        AudioCmd::Play {
            bus: BusId::Music,
            ..
        }
    ));

    let cmd = loader
        .loop_start("ambient.underwater.loop", 1.0, t)
        .expect("水环境循环必须起命令");
    assert!(matches!(
        cmd,
        AudioCmd::Play {
            bus: BusId::Ambient,
            ..
        }
    ));
}

#[test]
fn silent_manager_exposes_full_bus_config_surface() {
    // 静音模式(无设备)总线配置照常生效——配置面与设备面解耦。
    let audio = mcv_audio::AudioManager::silent(std::env::temp_dir());
    audio.set_bus_gain(BusId::Music, 0.25);
    assert_eq!(audio.bus_gain(BusId::Music), 0.25);
    audio.set_bus_lowpass(BusId::Ambient, true);
    assert!(audio.bus_lowpass(BusId::Ambient));
    audio.set_bus_send(BusId::Sfx, BusId::Music, 0.4);
    assert_eq!(audio.bus_gain(BusId::Music), 0.25);
    audio.clear_bus_sends(BusId::Sfx);
    // 根映射主音量:同一原子、单点写。
    audio.set_bus_gain(BusId::Master, 0.7);
    assert_eq!(audio.master_gain(), 0.7);
    assert_eq!(audio.bus_gain(BusId::Master), 0.7);
    // 共享句柄与委托方法同源。
    audio.buses().set_gain(BusId::Sfx, 0.9);
    assert_eq!(audio.bus_gain(BusId::Sfx), 0.9);
}

// 分类器单点真相(独立于素材,任何前缀都确定路由):
#[test]
fn event_classifier_rules() {
    for ev in ["block.stone.step", "entity.player.hurt", "ui.button.click"] {
        assert_eq!(BusId::for_event(ev), BusId::Sfx, "{ev} 兜底 sfx");
    }
    for ev in ["music.menu", "music.game.creative", "music_disc.cat"] {
        assert_eq!(BusId::for_event(ev), BusId::Music, "{ev} → music");
    }
    for ev in ["ambient.underwater.loop", "ambient.cave"] {
        assert_eq!(BusId::for_event(ev), BusId::Ambient, "{ev} → ambient");
    }
}
