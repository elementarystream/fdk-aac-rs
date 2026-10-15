// Tests of the optimized quantization functions of fdk_aac_rs_aacenc::quantize against their
// straight ports (PORTING.md, "Optimization rules"). The optimized functions are the public
// ones and are also tested against the C++ reference in quantize.rs.
//
// The per-line kernels are compared exhaustively where the input space is small (every quantized
// value with every gain in -300..300 and the gain extremes) and on random and extreme inputs
// otherwise; the band functions on random bands, with the quantized buffers pre-filled so that
// lines left unwritten are compared too.

use fdk_aac_rs_aacenc::quantize::testing;
use fdk_aac_rs_aacenc::quantize::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL, MINVAL_DBL};

fn gains() -> impl Iterator<Item = i32> {
    (-300..300).chain([
        i32::MIN,
        i32::MIN + 1,
        i32::MIN + 2,
        i32::MIN + 3,
        i32::MAX,
        i32::MAX - 1,
        i32::MAX - 2,
        i32::MAX - 3,
        -1000,
        1000,
        -4096,
        4095,
    ])
}

fn gain(rng: &mut Rng) -> i32 {
    match rng.range(0, 9) {
        0 => rng.i32(),
        1 => [i32::MIN, i32::MAX, i32::MIN + 1, -1000, 1000, 0][rng.range(0, 5) as usize],
        2 => rng.range(-400, 400),
        _ => rng.range(-120, 120),
    }
}

fn spec_value(rng: &mut Rng) -> FixpDbl {
    match rng.range(0, 9) {
        0 => [0, MINVAL_DBL, MAXVAL_DBL, 1, -1, MINVAL_DBL + 1, MAXVAL_DBL - 1, 0x4000_0000, 2, -2]
            [rng.range(0, 9) as usize],
        1..=4 => rng.i32() >> rng.range(0, 31),
        _ => rng.i32(),
    }
}

#[test]
fn quantize_line_kernel() {
    let mut rng = Rng::new(0x0f41);
    for g in gains() {
        for dz in 0..2 {
            for e in 0..32 {
                for x in [1i32 << e, (1i32 << e).wrapping_sub(1), (1i32 << e).wrapping_add(1)] {
                    for x in [x, x.wrapping_neg(), !x] {
                        let (s, f) = testing::quantize_line(g, x, dz);
                        assert_eq!(s, f, "quantize x {x} gain {g} dz {dz}");
                    }
                }
            }
            for _ in 0..2000 {
                let x = spec_value(&mut rng);
                let (s, f) = testing::quantize_line(g, x, dz);
                assert_eq!(s, f, "quantize x {x} gain {g} dz {dz}");
            }
        }
    }
    for _ in 0..2_000_000 {
        let (g, x, dz) = (gain(&mut rng), spec_value(&mut rng), rng.range(0, 1));
        let (s, f) = testing::quantize_line(g, x, dz);
        assert_eq!(s, f, "quantize x {x} gain {g} dz {dz}");
    }
}

#[test]
fn inv_quantize_line_kernel() {
    for g in gains() {
        for q in -MAX_QUANT..=MAX_QUANT {
            let (s, f) = testing::inv_quantize_line(g, q as i16);
            assert_eq!(s, f, "inverse quantization q {q} gain {g}");
        }
    }
}

#[test]
fn dist_line_kernel() {
    let mut rng = Rng::new(0x0f42);
    let edge = [
        0,
        1,
        -1,
        2,
        -2,
        MINVAL_DBL,
        MAXVAL_DBL,
        MINVAL_DBL + 1,
        0x4000_0000,
        -0x4000_0000,
        0x3FFF_FFFF,
        -0x3FFF_FFFF,
    ];
    for &a in &edge {
        for &b in &edge {
            let (s, f) = testing::dist_line(a, b);
            assert_eq!(s, f, "dist inv {a} spec {b}");
        }
    }
    for _ in 0..4_000_000 {
        let a = rng.i32();
        // spectral values near 2 * |a| (small differences), and unrelated ones
        let b = match rng.range(0, 3) {
            0 => rng.i32(),
            1 => a.wrapping_mul(2).wrapping_add(rng.range(-9, 9)),
            2 => a.wrapping_mul(-2).wrapping_add(rng.i32() >> rng.range(0, 31)),
            _ => [0, 1, -1, MINVAL_DBL][rng.range(0, 3) as usize],
        };
        let (s, f) = testing::dist_line(a, b);
        assert_eq!(s, f, "dist inv {a} spec {b}");
    }
}

#[test]
fn calc_sfb_dist_fast_vs_straight() {
    let mut rng = Rng::new(0x0f43);
    for k in 0..200_000 {
        // every length up to past the two-pass limit of 128 lines (masked last group)
        let n = if k % 4 == 0 { rng.range(0, 140) } else { rng.range(0, 72) };
        let spec: Vec<FixpDbl> = (0..n).map(|_| spec_value(&mut rng)).collect();
        let q0: Vec<i16> = (0..n).map(|_| rng.i16()).collect();
        let (g, dz) = (gain(&mut rng), rng.range(0, 1));
        let mut qs = q0.clone();
        let mut qf = q0.clone();
        let s = fdk_aac_enc_calc_sfb_dist_straight(&spec, &mut qs, n, g, dz);
        let f = fdk_aac_enc_calc_sfb_dist(&spec, &mut qf, n, g, dz);
        assert_eq!((s, &qs), (f, &qf), "calcSfbDist {k}: n {n} gain {g} dz {dz}");
        let mut qf = q0.clone();
        let f = testing::calc_sfb_dist_scalar(&spec, &mut qf, n, g, dz);
        assert_eq!((s, &qs), (f, &qf), "calcSfbDist scalar {k}: n {n} gain {g} dz {dz}");
    }
}

/// Bands with lines past `MAX_QUANT` at every position
/// (calc_sfb_quant_energy_and_dist_fast_vs_straight covers -32768).
#[test]
fn calc_sfb_dist_fast_vs_straight_extremes() {
    let mut rng = Rng::new(0x0f44);
    for k in 0..100_000 {
        let n = rng.range(1, 16);
        let spec: Vec<FixpDbl> = (0..n)
            .map(|_| match rng.range(0, 3) {
                0 => MINVAL_DBL + rng.range(0, 3),
                1 => MAXVAL_DBL - rng.range(0, 3),
                _ => spec_value(&mut rng),
            })
            .collect();
        let q0: Vec<i16> = (0..n).map(|_| rng.i16()).collect();
        let g = rng.range(-200, 40);
        let dz = rng.range(0, 1);
        let mut qs = q0.clone();
        let mut qf = q0.clone();
        let s = fdk_aac_enc_calc_sfb_dist_straight(&spec, &mut qs, n, g, dz);
        let f = fdk_aac_enc_calc_sfb_dist(&spec, &mut qf, n, g, dz);
        assert_eq!((s, &qs), (f, &qf), "calcSfbDist extremes {k}: n {n} gain {g} dz {dz}");
        let mut qf = q0.clone();
        let f = testing::calc_sfb_dist_scalar(&spec, &mut qf, n, g, dz);
        assert_eq!((s, &qs), (f, &qf), "calcSfbDist scalar extremes {k}: n {n} gain {g}");
    }
}

#[test]
fn calc_sfb_quant_energy_and_dist_fast_vs_straight() {
    let mut rng = Rng::new(0x0f45);
    for k in 0..300_000 {
        let n = rng.range(0, 72);
        let spec: Vec<FixpDbl> = (0..n).map(|_| spec_value(&mut rng)).collect();
        let wide = rng.range(0, 3) == 0;
        let q: Vec<i16> = (0..n)
            .map(|_| match rng.range(0, 19) {
                0 if wide => rng.i16(),
                1 if wide => i16::MIN,
                2..=5 => 0,
                _ => (rng.range(-MAX_QUANT, MAX_QUANT) >> rng.range(0, 13)) as i16,
            })
            .collect();
        let g = gain(&mut rng);
        let (mut es, mut ds, mut ef, mut df) = (0x1234, 0x5678, 0x1234, 0x5678);
        fdk_aac_enc_calc_sfb_quant_energy_and_dist_straight(&spec, &q, n, g, &mut es, &mut ds);
        fdk_aac_enc_calc_sfb_quant_energy_and_dist(&spec, &q, n, g, &mut ef, &mut df);
        assert_eq!((es, ds), (ef, df), "calcSfbQuantEnergyAndDist {k}: n {n} gain {g}");
        let f = testing::calc_sfb_quant_energy_and_dist_scalar(&spec, &q, n, g);
        assert_eq!((es, ds), f, "calcSfbQuantEnergyAndDist scalar {k}: n {n} gain {g}");
    }
}

#[test]
fn quantize_spectrum_fast_vs_straight() {
    let mut rng = Rng::new(0x0f46);
    for k in 0..20_000 {
        let groups = rng.range(1, 4);
        let per = rng.range(1, 60 / groups);
        let sfb_cnt = groups * per;
        let max_sfb = rng.range(0, per);
        let mut offsets = vec![0i32; 61];
        for i in 1..61 {
            // band widths of any size (the encoder's are multiples of 4)
            let w = if k % 2 == 0 { 4 * rng.range(0, 6) } else { rng.range(0, 23) };
            offsets[i] = (offsets[i - 1] + w).min(1024);
        }
        let spec: Vec<FixpDbl> = (0..1024).map(|_| spec_value(&mut rng)).collect();
        let scf: Vec<i32> = (0..60)
            .map(|_| if rng.range(0, 9) == 0 { rng.i32() } else { rng.range(-60, 60) })
            .collect();
        let (g, dz) = (gain(&mut rng), rng.range(0, 1));
        let q0: Vec<i16> = (0..1024).map(|_| rng.i16()).collect();
        let mut qs = q0.clone();
        let mut qf = q0.clone();
        fdk_aac_enc_quantize_spectrum_straight(
            sfb_cnt, max_sfb, per, &offsets, &spec, g, &scf, &mut qs, dz,
        );
        fdk_aac_enc_quantize_spectrum(sfb_cnt, max_sfb, per, &offsets, &spec, g, &scf, &mut qf, dz);
        assert_eq!(qs, qf, "QuantizeSpectrum {k}");
    }
}

#[test]
fn quantize_lines_scalar_vs_straight() {
    let mut rng = Rng::new(0x0f47);
    for k in 0..100_000 {
        let n = rng.range(0, 72) as usize;
        let spec: Vec<FixpDbl> = (0..n).map(|_| spec_value(&mut rng)).collect();
        let (g, dz) = (gain(&mut rng), rng.range(0, 1));
        let s: Vec<i16> = spec.iter().map(|&x| testing::quantize_line(g, x, dz).0).collect();
        let mut f = vec![0x5555i16; n];
        testing::quantize_lines_scalar(g, &spec, &mut f, dz);
        assert_eq!(s, f, "quantizeLines scalar {k}: gain {g} dz {dz}");
    }
}
