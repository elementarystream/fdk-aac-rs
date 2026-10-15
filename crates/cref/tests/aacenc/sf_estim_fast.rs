// Tests of the optimized helpers of fdk_aac_rs_aacenc::sf_estim against their straight ports
// (PORTING.md, "Optimization rules"); sf_estim.rs and qc_main_chain.rs test the functions
// that use them against the C++ reference.

use fdk_aac_rs_aacenc::sf_estim::{form_factor_sum, form_factor_sum_straight};
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL, MINVAL_DBL};

/// Every value below 2^22 and 2^20 values of every larger magnitude (both signs), each in
/// every lane of a group of eight (the NEON kernel takes groups of four, the x86 AVX2 kernels
/// eight (`x86`) or four (`x86_32`) lines); in the release profile 2^22 per magnitude.
#[test]
fn form_factor_sum_lanes() {
    let mut rng = Rng::new(0xf0f1);
    let per_bits = if cfg!(debug_assertions) { 1 << 16 } else { 1 << 22 };
    let check = |x: FixpDbl, rng: &mut Rng| {
        let lane = rng.range(0, 7) as usize;
        let mut g = [0; 8];
        g[lane] = x;
        let s = form_factor_sum_straight(&[x]);
        assert_eq!(form_factor_sum(&g), s, "x {x} lane {lane}");
    };
    for x in 0..(1 << 22) {
        check(x, &mut rng);
        check(-x, &mut rng);
    }
    for bits in 22..31 {
        for _ in 0..per_bits {
            let x = (1i32 << bits) | (rng.next_u32() as i32 & ((1 << bits) - 1));
            check(x, &mut rng);
            check(x.wrapping_neg(), &mut rng);
        }
    }
    for x in [MAXVAL_DBL, MAXVAL_DBL - 1, MINVAL_DBL, MINVAL_DBL + 1, 1 << 30, -(1 << 30)] {
        check(x, &mut rng);
    }
}

/// Random bands of any length (the remainder lines take the scalar code), with MINVAL_DBL
/// (whose fixp_abs() stays negative) and zeros.
#[test]
fn form_factor_sum_bands() {
    let mut rng = Rng::new(0xf0f2);
    for k in 0..200_000 {
        let n = rng.range(0, 70) as usize;
        let spec: Vec<FixpDbl> = (0..n)
            .map(|_| match rng.range(0, 12) {
                0 => MINVAL_DBL,
                1..=3 => 0,
                4..=6 => rng.i32() >> rng.range(0, 31),
                _ => rng.i32(),
            })
            .collect();
        assert_eq!(form_factor_sum(&spec), form_factor_sum_straight(&spec), "band {k}");
    }
}
