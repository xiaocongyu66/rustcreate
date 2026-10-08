//! 衰减公式边界表:线性滚存,16 格截止。
use mcv_audio::{distance3, distance_attenuation, DEFAULT_ATTENUATION_RADIUS};

#[test]
fn attenuation_boundary_table() {
    let r = DEFAULT_ATTENUATION_RADIUS; // 16.0
                                        // (距离, 期望衰减) —— 全部取二进制精确值,可用同一容差。
    let table: [(f32, f32); 8] = [
        (0.0, 1.0),    // 同点:满增益
        (1.0, 0.9375), // 1/16
        (2.0, 0.875),
        (8.0, 0.5),      // 半径中点
        (12.0, 0.25),    // 3/4 处
        (15.5, 0.03125), // 紧贴截止
        (16.0, 0.0),     // 截止点:恰好 0(含)
        (100.0, 0.0),    // 超出:0
    ];
    for (d, want) in table {
        let got = distance_attenuation(d, r);
        assert!(
            (got - want).abs() <= 1e-6,
            "dist={d}: got {got}, want {want}"
        );
    }
    // 单调不增。
    let mut last = 1.0;
    for i in 0..=160 {
        let a = distance_attenuation(i as f32 / 10.0, r);
        assert!(a <= last + 1e-6, "非单调: {a} > {last}");
        assert!((0.0..=1.0).contains(&a));
        last = a;
    }
}

#[test]
fn attenuation_degenerate_inputs() {
    assert_eq!(distance_attenuation(-5.0, 16.0), 1.0, "负距离按同点处理");
    assert_eq!(distance_attenuation(f32::NAN, 16.0), 0.0, "NaN 不发声");
    assert_eq!(distance_attenuation(f32::INFINITY, 16.0), 0.0, "inf 不发声");
    assert_eq!(
        distance_attenuation(3.0, 0.0),
        0.0,
        "半径<=0 视为不发声(防除零)"
    );
    assert_eq!(distance_attenuation(3.0, -1.0), 0.0);
    assert_eq!(distance_attenuation(3.0, f32::NAN), 0.0);
}

#[test]
fn distance3_basic() {
    assert_eq!(distance3([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]), 0.0);
    assert_eq!(distance3([1.0, 2.0, 2.0], [0.0, 0.0, 0.0]), 3.0);
    // 恰好压在 16 格截止上 → 衰减 0。
    let d = distance3([16.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    assert_eq!(distance_attenuation(d, 16.0), 0.0);
}
