// Tests of the optimized hybrid band loop of DownmixPSQmfData (downmix_bands: on AArch64 the
// NEON kernel for the aarch64 variant, on x86 with AVX2 the AVX2 kernel for the x86 and x86_32
// variants) against the straight port (downmix_bands_straight: the C++ loop body for every
// band); ps_main.rs tests the whole PS encoder against C.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL, MINVAL_DBL};
use fdk_aac_rs_sbrenc::ps_main::{downmix_bands, downmix_bands_straight};

fn row(rng: &mut Rng, kind: i32) -> [FixpDbl; 71] {
    core::array::from_fn(|_| match kind {
        0 => rng.i32(),
        1 => rng.i32() >> rng.range(0, 31),
        2 => [0, 1, -1, MINVAL_DBL, MAXVAL_DBL, MINVAL_DBL + 1][rng.range(0, 5) as usize],
        _ => (rng.i32() >> rng.range(8, 31)) * rng.range(-3, 3),
    })
}

#[test]
fn downmix_bands_fast_vs_straight() {
    let mut rng = Rng::new(0x9d31);
    for it in 0..60_000 {
        let kind = rng.range(0, 4);
        let mut rows =
            [row(&mut rng, kind), row(&mut rng, kind), row(&mut rng, kind), row(&mut rng, kind)];
        // correlated (r = l, r = -l: no division), equal magnitudes, and rare extremes
        match rng.range(0, 5) {
            0 => {
                rows[2] = rows[0];
                rows[3] = rows[1];
            }
            1 => {
                rows[2] = rows[0].map(|x| x.wrapping_neg());
                rows[3] = rows[1].map(|x| x.wrapping_neg());
            }
            2 => {
                for k in 0..71 {
                    if rng.range(0, 3) == 0 {
                        rows[2][k] = rows[0][k].wrapping_add(rng.range(-2, 2));
                    }
                }
            }
            _ => {}
        }
        if rng.range(0, 20) == 0 {
            let (r, k) = (rng.range(0, 3) as usize, rng.range(0, 70) as usize);
            rows[r][k] = [MINVAL_DBL, MAXVAL_DBL, 0][rng.range(0, 2) as usize];
        }
        let dyn_qmf_scale = if rng.range(0, 9) == 0 { rng.range(0, 40) } else { rng.range(0, 4) };
        let refs = [&rows[0], &rows[1], &rows[2], &rows[3]];
        let (mut sr, mut si, mut fr, mut fi) = ([7; 71], [7; 71], [7; 71], [7; 71]);
        downmix_bands_straight(refs, &mut sr, &mut si, dyn_qmf_scale);
        downmix_bands(refs, &mut fr, &mut fi, dyn_qmf_scale);
        assert_eq!((sr, si), (fr, fi), "iteration {it} (kind {kind}, dynQmfScale {dyn_qmf_scale})");
    }
}
