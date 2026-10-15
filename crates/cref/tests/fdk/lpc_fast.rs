// Tests of the optimized `clpc_synthesis_lattice` against the straight port kept next to
// it (`clpc_synthesis_lattice_straight`), on random and extreme inputs: every order, both
// directions, other increments (which take the straight port), values at the saturation
// boundaries of SATURATE_LEFT_SHIFT_ALT (±2^30, ±2^31) and unstable filters that saturate
// often. No C code is involved, so the test runs with every FDK_AAC_RS_ARCH variant.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::lpc::*;

const EDGES: [i32; 12] = [
    i32::MIN,
    i32::MIN + 1,
    -0x4000_0001,
    -0x4000_0000,
    -0x3FFF_FFFF,
    -1,
    0,
    1,
    0x3FFF_FFFF,
    0x4000_0000,
    0x4000_0001,
    i32::MAX,
];

fn value(rng: &mut Rng, scale: u32) -> i32 {
    match rng.next_u32() % 16 {
        0 => EDGES[rng.range(0, EDGES.len() as i32 - 1) as usize],
        1 => rng.i32(),
        2 => EDGES[rng.range(0, EDGES.len() as i32 - 1) as usize].wrapping_add(rng.range(-3, 3)),
        _ => rng.i32() >> scale,
    }
}

fn exponent(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 8 {
        0 => rng.range(-40, 40),
        1 => rng.range(28, 34),
        _ => rng.range(-3, 3),
    }
}

#[test]
fn synthesis_lattice_fast_matches_straight() {
    let mut rng = Rng::new(0x7a77);
    for iter in 0..60000 {
        let order = rng.range(1, LPC_MAX_ORDER as i32);
        // realistic coefficients (|k| < 1, stable) or anything (saturating)
        let cscale = if iter % 3 == 0 { 0 } else { 1 + rng.next_u32() % 4 };
        let coeff: Vec<i32> = (0..order).map(|_| value(&mut rng, cscale)).collect();
        let sscale = rng.next_u32() % 12;
        let mut st_a: Vec<i32> = (0..order).map(|_| value(&mut rng, sscale)).collect();
        let mut st_b = st_a.clone();
        for call in 0..3 {
            let size = rng.range(1, 70);
            let inc = match rng.next_u32() % 16 {
                0 => 0,
                1 => 2,
                n if n % 2 == 0 => 1,
                _ => -1,
            };
            let (e, e_out) = (exponent(&mut rng), exponent(&mut rng));
            let len = if inc == 2 { 2 * size as usize } else { size as usize };
            let mut sig_a: Vec<i32> = (0..len).map(|_| value(&mut rng, sscale)).collect();
            let mut sig_b = sig_a.clone();
            clpc_synthesis_lattice(&mut sig_a, size, e, e_out, inc, &coeff, order, &mut st_a);
            clpc_synthesis_lattice_straight(
                &mut sig_b, size, e, e_out, inc, &coeff, order, &mut st_b,
            );
            let what =
                format!("iter {iter} call {call} order {order} e {e} e_out {e_out} inc {inc}");
            assert_eq!(sig_a, sig_b, "signal, {what}");
            assert_eq!(st_a, st_b, "state, {what}");
        }
    }
}

/// Longer runs with the TNS coefficients of the library (`FDK_AAC_DEC_TNS_COEFF3/4` are the
/// sine quantizer values; here the same kind of values) and full-scale spectra.
#[test]
fn synthesis_lattice_fast_matches_straight_long() {
    let mut rng = Rng::new(0x7a78);
    for iter in 0..2000 {
        let order = [1, 2, 7, 8, 12, 15, 20, 24][iter % 8];
        let coeff: Vec<i32> = (0..order).map(|_| (rng.i32() >> 1) + (rng.i32() >> 2)).collect();
        let mut st_a = vec![0; order as usize];
        let mut st_b = st_a.clone();
        let size = rng.range(1, 1024);
        let shift = rng.next_u32() % 4;
        let mut sig_a: Vec<i32> = (0..size).map(|_| rng.i32() >> shift).collect();
        let mut sig_b = sig_a.clone();
        let inc = if iter % 2 == 0 { 1 } else { -1 };
        clpc_synthesis_lattice(&mut sig_a, size, 0, 0, inc, &coeff, order, &mut st_a);
        clpc_synthesis_lattice_straight(&mut sig_b, size, 0, 0, inc, &coeff, order, &mut st_b);
        assert_eq!(sig_a, sig_b, "signal, iter {iter} order {order}");
        assert_eq!(st_a, st_b, "state, iter {iter} order {order}");
    }
}
