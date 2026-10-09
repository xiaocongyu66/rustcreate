//! 音效表解析与变体抽取:条目形态、乘子默认值、event/dictionary 展平、
//! 防环、加权随机的种子确定性(固定种子断言精确序列)。
//!
//! fixture(见 `common`)里 dev.mixed / dev.weighted / dev.dict 的变体权重都是
//! `[1, 3]`(顺序 `[dig/stone1, step/stone1]`),SplitMix64 种子 0..16 的抽取
//! 下标序列为 `[1,1,1,1,1,1,0,1,1,0,1,1,1,1,1,1]`(即 seed 6/9 抽到首个变体)。
mod common;

use common::SoundFixture;
use mcv_audio::{SoundLoader, SoundTable};

#[test]
fn entries_parse_with_defaults() {
    let fx = SoundFixture::new();
    let table = SoundTable::load(fx.dir()).expect("fixture 表必须可加载");
    assert!(!table.is_empty());

    // 裸字符串条目:全部默认乘子。
    let v = table.pick("dev.oneshot", 0).expect("事件应存在");
    assert_eq!(&*v.path, "dig/stone1");
    assert_eq!(v.volume, 1.0);
    assert_eq!(v.pitch, 1.0);
    assert_eq!(v.weight, 1);
    assert!(!v.stream);

    // 对象条目:乘子生效(seed 0 → 下标 1 的变体)。
    let v = table.pick("dev.mixed", 0).expect("事件应存在");
    assert_eq!(&*v.path, "step/stone1");
    assert_eq!(v.volume, 0.5);
    assert_eq!(v.pitch, 1.5);
    assert_eq!(v.weight, 3);
}

#[test]
fn event_refs_flatten_with_modifiers() {
    let fx = SoundFixture::new();
    let table = SoundTable::load(fx.dir()).unwrap();

    // type:"event" 别名:子事件变体按子事件自身乘子并入。
    let v = table.pick("dev.alias", 0).expect("别名应可播");
    assert_eq!(&*v.path, "step/stone1");
    assert_eq!(v.volume, 0.5);
    assert_eq!(v.pitch, 1.5);

    // 带乘子的引用:volume/pitch 乘到子变体上(0.25/2.0),与原版一致。
    // seed 0 → 下标 1:volume 0.25*0.5=0.125,pitch 2.0*1.5=3.0。
    let v = table.pick("dev.alias_mod", 0).expect("别名应可播");
    assert_eq!(&*v.path, "step/stone1");
    assert!((v.volume - 0.125).abs() < 1e-6);
    assert!((v.pitch - 3.0).abs() < 1e-6);
    // seed 6 → 下标 0:volume 0.25,pitch 2.0。
    let v = table.pick("dev.alias_mod", 6).expect("别名应可播");
    assert_eq!(&*v.path, "dig/stone1");
    assert!((v.volume - 0.25).abs() < 1e-6);
    assert!((v.pitch - 2.0).abs() < 1e-6);
}

#[test]
fn dictionary_flattens_by_weight() {
    let fx = SoundFixture::new();
    let table = SoundTable::load(fx.dir()).unwrap();

    // 字典 {dev.oneshot: 1, dev.other: 3} → 两键事件按权重展平为 [1, 3]。
    let v = table.pick("dev.dict", 0).expect("字典事件应可播");
    assert_eq!(&*v.path, "step/stone1");
    assert_eq!(v.weight, 3);
    let v = table.pick("dev.dict", 6).expect("字典事件应可播");
    assert_eq!(&*v.path, "dig/stone1");
    assert_eq!(v.weight, 1);
}

#[test]
fn bad_refs_and_cycles_are_skipped() {
    let fx = SoundFixture::new();
    let table = SoundTable::load(fx.dir()).unwrap();

    // 引用不存在事件的条目被跳过,同事件内的合法条目保留。
    let v = table.pick("dev.bad_ref", 0).expect("合法条目应保留");
    assert_eq!(&*v.path, "dig/stone1");

    // 引用环:两边都解析为空,抽取 no-op。
    assert!(table.pick("dev.cycle_a", 0).is_none());
    assert!(table.pick("dev.cycle_b", 0).is_none());

    // 表里根本没有的事件 → None。
    assert!(table.pick("dev.nope", 0).is_none());
}

#[test]
fn pick_sequence_is_deterministic_by_seed() {
    let fx = SoundFixture::new();
    let table = SoundTable::load(fx.dir()).unwrap();

    // 权重 [1,3]:SplitMix64 种子 0..16 的精确抽取序列(防实现漂移的回归锚)。
    let want = [1, 1, 1, 1, 1, 1, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1];
    for (seed, w) in want.iter().enumerate() {
        let v = table.pick("dev.weighted", seed as u64).unwrap();
        let got = if &*v.path == "dig/stone1" { 0 } else { 1 };
        assert_eq!(got, *w, "seed={seed}: 抽取序列漂移");
    }

    // 大样本分布趋近权重比(1:3 → 25%,容差 5%)。
    let mut hits0 = 0u32;
    for seed in 0..4096u64 {
        if &*table.pick("dev.weighted", seed).unwrap().path == "dig/stone1" {
            hits0 += 1;
        }
    }
    let ratio = f64::from(hits0) / 4096.0;
    assert!(
        (ratio - 0.25).abs() < 0.05,
        "加权分布失真: {ratio} (期望 ~0.25)"
    );
}

#[test]
fn zero_weights_fall_back_to_first() {
    let fx = SoundFixture::new();
    let table = SoundTable::load(fx.dir()).unwrap();
    // 全零权重:取首个,与种子无关。
    for seed in 0..8u64 {
        let v = table.pick("dev.zerow", seed).unwrap();
        assert_eq!(&*v.path, "dig/stone1");
        assert_eq!(v.weight, 0);
    }
}

#[test]
fn missing_sounds_json_yields_empty_table() {
    // 文件缺失:load 报 Err;SoundLoader 降级为空表,播放静默 no-op。
    let dir = std::env::temp_dir().join(format!("mcv-audio-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(SoundTable::load(&dir).is_err());
    let mut loader = SoundLoader::new(&dir);
    assert!(loader.table().is_empty());
    assert!(
        loader
            .play_ui("dev.anything", 1.0, std::time::Instant::now())
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
