# voxel-diff 机制自检:java(seed 1) vs java(seed 123), chunk 0,0, y[-64,320)

> 不同 seed 的两次 vanilla noise-fill,diff 应接近"不同世界"基线(~40-70% 不同),
> 证明 diff 机器与分带统计管线工作正常。真正的 Rust 对拍(同 seed)
> 走 CI 产物后再跑(本机禁 cargo)。

{
  "total": 98304,
  "equal": 84128,
  "same_rate": 0.855794,
  "first_diff": [
    7,
    -41,
    15,
    "minecraft:air",
    "minecraft:stone"
  ],
  "bands": {
    "deep(-32)": {
      "diff": 2014,
      "n": 10206,
      "rate": 0.1973
    },
    "deep(-64)": {
      "diff": 163,
      "n": 8355,
      "rate": 0.0195
    },
    "high(128)": {
      "diff": 0,
      "n": 8192,
      "rate": 0.0
    },
    "high(160)": {
      "diff": 0,
      "n": 8192,
      "rate": 0.0
    },
    "high(192)": {
      "diff": 0,
      "n": 8192,
      "rate": 0.0
    },
    "high(224)": {
      "diff": 0,
      "n": 8192,
      "rate": 0.0
    },
    "high(256)": {
      "diff": 0,
      "n": 8192,
      "rate": 0.0
    },
    "high(288)": {
      "diff": 0,
      "n": 8192,
      "rate": 0.0
    },
    "high(64)": {
      "diff": 68,
      "n": 8260,
      "rate": 0.0082
    },
    "high(96)": {
      "diff": 0,
      "n": 8192,
      "rate": 0.0
    },
    "mid(0)": {
      "diff": 3827,
      "n": 12019,
      "rate": 0.3184
    },
    "mid(32)": {
      "diff": 8104,
      "n": 16296,
      "rate": 0.4973
    }
  }
}
