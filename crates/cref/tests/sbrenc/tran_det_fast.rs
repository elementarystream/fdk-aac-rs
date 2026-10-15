// Tests of the optimized calculateThresholds of the SBR transient detector (calculate_thresholds:
// the loops over the bands inside) against its straight port (PORTING.md, "Optimization rules");
// tran_det.rs tests FDKsbrEnc_transientDetect, which uses the optimized one, against C.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL};
use fdk_aac_rs_sbrenc::tran_det::{calculate_thresholds, calculate_thresholds_straight};

fn energies(rng: &mut Rng) -> Vec<Vec<FixpDbl>> {
    let kind = rng.range(0, 3);
    (0..64)
        .map(|_| {
            (0..64)
                .map(|_| match kind {
                    0 => rng.i32() & MAXVAL_DBL,
                    1 => (rng.i32() & MAXVAL_DBL) >> rng.range(0, 31),
                    2 => [0, 1, MAXVAL_DBL, 255, 256, 257][rng.range(0, 5) as usize],
                    _ => rng.i32(),
                })
                .collect()
        })
        .collect()
}

/// Parameters as the encoder configures them (and around them), for which the straight code
/// stays inside its buffers.
struct Params {
    ybss: i32,
    ybwo: i32,
    no_cols: i32,
    no_rows: i32,
    tran_off: i32,
    add_prev: i32,
    start_band: i32,
}

fn params(rng: &mut Rng) -> Option<Params> {
    let ybss = rng.range(0, 1);
    let no_cols = [8, 15, 16, 24, 30, 32][rng.range(0, 5) as usize];
    let tran_off = rng.range(3, 12);
    let ybwo = if rng.range(0, 1) == 0 { tran_off << ybss } else { rng.range(0, 20) };
    let p = Params {
        ybss,
        ybwo,
        no_cols,
        no_rows: if rng.range(0, 3) == 0 { rng.range(0, 64) } else { 64 },
        tran_off,
        add_prev: rng.range(0, 1),
        start_band: if rng.range(0, 7) == 0 { rng.range(0, 10) } else { 0 },
    };
    // the straight code's energies_temp writes and reads, the energies rows and transients
    let start = (tran_off - 3) >> ybss;
    let end = ((no_cols + (ybwo << ybss)) - 1) >> ybss;
    let filled_hi = if ybss == 1 { 2 * end + 1 } else { end };
    let ok = start >= 0
        && filled_hi < 64
        && end < 64
        && tran_off + no_cols + 2 < 64
        && tran_off + p.add_prev + no_cols <= 48
        && (no_cols >> ybss) + tran_off <= 64
        && ybwo < 64;
    ok.then_some(p)
}

#[test]
fn calculate_thresholds_fast_vs_straight() {
    let mut rng = Rng::new(0x7d01);
    let mut n = 0;
    while n < 20_000 {
        let Some(p) = params(&mut rng) else { continue };
        n += 1;
        let e = energies(&mut rng);
        let rows: Vec<&[FixpDbl]> = e.iter().map(|r| &r[..]).collect();
        let scale = match rng.range(0, 3) {
            0 => [rng.range(0, 40), rng.range(0, 40)],
            _ => {
                let s = rng.range(0, 20);
                [s + rng.range(0, 3), s + rng.range(0, 3)]
            }
        };
        let th0: Vec<FixpDbl> = (0..64).map(|_| rng.i32() & MAXVAL_DBL).collect();
        let (mut ts, mut tf) = (th0.clone(), th0);
        calculate_thresholds_straight(
            &rows, &scale, &mut ts, p.ybwo, p.ybss, p.no_cols, p.no_rows, p.tran_off,
        );
        calculate_thresholds(
            &rows, &scale, &mut tf, p.ybwo, p.ybss, p.no_cols, p.no_rows, p.tran_off,
        );
        assert_eq!(ts, tf, "case {n}");
    }
}
