//! #210 临时探针（对拍刀用后即删）：cell 外壳 wall-off vs raw 的逐层值。
use mcv_core::ChunkPos;
use mcv_worldgen::vanilla::cell_interp::CellShell;
use mcv_worldgen::vanilla::terrain::Orchestrator;

#[test]
fn cell_probe_values() {
    let seed: u64 = 0x7E_A20_0B1;
    let orch = Orchestrator::new(seed);
    let mut cell = CellShell::new(&orch);
    let mut out =
        String::from("== probe seed 0x7EA200B1 col world (34,-30) and corner (32,-32) ==\n");
    let mut sh = cell;
    for y in [
        -63i32, -56, -48, -40, -30, -14, -12, 0, 16, 28, 40, 43, 63, 80,
    ] {
        for (x, z) in [(32i32, -32i32), (34, -30)] {
            let corners = sh.cols4(x, z);
            let dv = CellShell::density_at(&corners, x, y, z);
            let cs = orch.column_state(x, z);
            let raw = orch.density(&cs, x, y, z);
            println!("P x={x} z={z} y={y} cell={dv:+.6} raw={raw:+.6}");
        }
    }
    // corner W 值直出（gy 0..17, W_TERRAIN）
    let corners = sh.cols4(32, -32);
    for gy in 0..17usize {
        let y = -64 + 8 * gy as i32;
        let c00 = &corners[0];
        println!(
            "W gy={gy} y={y} terr={:+.6} mask={:+.6} base={:+.6} ra={:+.6} rb={:+.6}",
            c00.corner_value(0, gy),
            c00.corner_value(1, gy),
            c00.corner_value(2, gy),
            c00.corner_value(3, gy),
            c00.corner_value(4, gy)
        );
    }
    panic!("CELL-PROBE-DUMP\n{out}");
}
