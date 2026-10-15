// Tests of the optimized helpers of fdk_aac_rs_sbrdec::transcendent against the straight
// port: f_div_norm_fast against fdk_aac_rs_fdk's f_div_norm (which is compared with C++ in the
// fdk tests), on random, edge-biased and normalized inputs.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::f_div_norm;
use fdk_aac_rs_sbrdec::transcendent::f_div_norm_fast;

fn check(num: i32, den: i32) {
    let (mut ea, mut eb) = (0x55, 0x55);
    let a = f_div_norm_fast(num, den, &mut ea);
    let b = f_div_norm(num, den, &mut eb);
    assert_eq!((a, ea), (b, eb), "num {num:#x} den {den:#x}");
}

#[test]
fn f_div_norm_fast_matches_straight() {
    let edges = [
        0,
        1,
        2,
        3,
        -1,
        -2,
        i32::MAX,
        i32::MAX - 1,
        i32::MIN,
        i32::MIN + 1,
        0x4000_0000,
        0x3FFF_FFFF,
        0x4000_0001,
        0x2000_0000,
        0x7FFF_0000,
        0x0000_FFFF,
        0x1_0000,
    ];
    for &a in &edges {
        for &b in &edges {
            check(a, b);
        }
    }
    let mut rng = Rng::new(0x0306);
    for _ in 0..4_000_000 {
        check(rng.i32(), rng.i32());
        // the domain of the library: num >= 0, den > 0
        let num = (rng.next_u32() >> 1) as i32 >> (rng.next_u32() % 32);
        let den = ((rng.next_u32() >> 1) as i32 >> (rng.next_u32() % 32)).max(1);
        check(num, den);
        // equal and neighbouring mantissas (quotients at the top of the range)
        let m = (rng.next_u32() >> 1) as i32 | 1;
        check(m, m);
        check(m - 1, m);
        check(m, m.saturating_add(1));
        check(m >> (rng.next_u32() % 31), m);
    }
}
