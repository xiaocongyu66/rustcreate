//! #209/#206 黑盒对拍 Rust 侧 dump 通道（JGH1 格式，与 java-harness
//! `Main -o` 同字节口径——python java_dupe.parse_jgh1 可直接读）。
//!
//! 门控：`RUST_DUMPOUT` 未设时 no-op（CI 例行跑零开销）；设为目录时
//! 输出 cave_audit 审计窗（seed 0x7EA20B1、chunks (2,-4)..(5,-1)、
//! y∈[-64,96)）逐 chunk 的 JGH1 bin，与 harness 官方 jar dump
//! （同窗同 seed）做 python 逐列矩阵对拍（含水层水面/首实心面/差异
//! 归因）。名字面 = mcv_core::BLOCKS[i].name 加 "minecraft:" 前缀
//! （状态属性差异如 water[level=0] 由对拍脚本侧归一化处理）。
//!
//! JGH1 布局（big-endian，java_dupe.py:377-395 实物）：
//! magic(4) + icnt(i32) + seed(i64) + cx(i32) + cz(i32) + ymin(i32)
//! + ymax(i32) + [u16 len + utf8 name]×icnt + u16 vox ×(160·16·16)。
//!
//! 体素序 = (y-ymin)*256 + z*16 + x（每层 z 行，行内 x）。

use mcv_core::{ChunkPos, WORLD_MIN_Y};
use mcv_worldgen::{TerrainBackend, generate_terrain_with};

// 官方对拍世界 = 132784305（0x7EA20B1，tools/fork-reanchor/jobs_all.txt
// 十进制同值实锤）。前刀常量多一个零（0x7EA200B1=2124546225，cave_audit
// 审计窗自家世界）→ dump 通道从未与官方同世界——矩阵跨世界失真（#210
// 作业发现；audit 行为锁锚值另属世界，此处不动）。
const SEED: u64 = 0x7EA20B1;
const Y_LO: i32 = WORLD_MIN_Y; // −64
const Y_HI: i32 = 96; // 审计窗上沿（与 cave_audit_stats 同口径）
const CX0: i32 = 2;
const CZ0: i32 = -4;
const N: i32 = 4;

#[test]
fn dump_for_java() {
    let Some(out_dir) = std::env::var_os("RUST_DUMPOUT") else {
        return; // 例行 CI：零开销 no-op
    };
    let out_dir = std::path::PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir).expect("mkdir dumpout");

    // 先扫一遍收集本窗出现过的 id（名字表按需建，与 java 侧同语义），
    // 再写文件——两遍走避免边写边扩表。
    let mut chunks: Vec<(i32, i32, Vec<u16>)> = Vec::new();
    let mut used = [false; 2048];
    for cz in 0..N {
        for cx in 0..N {
            let t = generate_terrain_with(
                TerrainBackend::Vanilla,
                SEED,
                ChunkPos::new(CX0 + cx, CZ0 + cz),
            )
            .expect("vanilla gen");
            let vox = t.voxels.as_u16_slice();
            let mut flat = vec![0u16; ((Y_HI - Y_LO) * 256) as usize];
            for z in 0..16usize {
                for x in 0..16usize {
                    for y in Y_LO..Y_HI {
                        let v = vox[mcv_core::vidx(x, y, z)];
                        let off = ((y - Y_LO) as usize) * 256 + z * 16 + x;
                        flat[off] = v;
                        used[usize::from(v)] = true;
                    }
                }
            }
            chunks.push((CX0 + cx, CZ0 + cz, flat));
        }
    }

    let mut ids: Vec<u16> = (0..2048u16).filter(|&i| used[usize::from(i)]).collect();
    // 名字字典序稳定（重跑同序 → 对拍侧按名 remap 稳定）。
    ids.sort_by_key(|&i| mcv_core::BLOCKS[i as usize].name);
    let names: Vec<String> = ids
        .iter()
        .map(|&i| format!("minecraft:{}", mcv_core::BLOCKS[i as usize].name))
        .collect();

    for (cx, cz, flat) in &chunks {
        let path = out_dir.join(format!("rust_seed{SEED}_c{cx}_{cz}.bin"));
        let mut b: Vec<u8> = Vec::with_capacity(32 + names.len() * 24 + flat.len() * 2);
        b.extend_from_slice(b"JGH1");
        b.extend_from_slice(&(names.len() as i32).to_be_bytes());
        b.extend_from_slice(&(SEED as i64).to_be_bytes());
        b.extend_from_slice(&cx.to_be_bytes());
        b.extend_from_slice(&cz.to_be_bytes());
        b.extend_from_slice(&Y_LO.to_be_bytes());
        b.extend_from_slice(&Y_HI.to_be_bytes());
        for nm in &names {
            b.extend_from_slice(&(nm.len() as u16).to_be_bytes());
            b.extend_from_slice(nm.as_bytes());
        }
        for v in flat {
            // 线性扫 ids（本窗 id 少，≤20）；宝贝化查表留给后续 wave。
            let mapped = ids.iter().position(|&x| x == *v).expect("id in table");
            b.extend_from_slice(&(mapped as u16).to_be_bytes());
        }
        std::fs::write(&path, &b).expect("write jgh1");
        println!("K-DUMP-4J {}", path.display());
    }
}
