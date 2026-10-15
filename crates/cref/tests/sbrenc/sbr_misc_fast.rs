// Tests of fdk_aac_rs_sbrenc::sbr_misc::schur_div_fast against schur_div
// (fdk_aac_rs_fdk::fixpoint_math, the straight port; PORTING.md, "Optimization rules").

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::fixpoint_math::schur_div;
use fdk_aac_rs_sbrenc::sbr_misc::schur_div_fast;

fn check(num: i32, denum: i32, count: i32) {
    assert_eq!(
        schur_div_fast(num, denum, count),
        schur_div(num, denum, count),
        "schur_div({num}, {denum}, {count})"
    );
}

/// Every count with random operands in the defined domain (0 <= num <= denum), at its edges
/// (num == denum, num >> 1 == denum >> 1, num 0 and 1), with the operands normalized as the
/// callers do, and outside the domain.
#[test]
fn schur_div_fast_vs_straight() {
    let mut rng = Rng::new(0x5c4d);
    for count in 1..=40 {
        for _ in 0..20_000 {
            let denum = match rng.range(0, 3) {
                0 => rng.i32().max(1),
                1 => (rng.next_u32() >> rng.range(0, 31)) as i32 & i32::MAX,
                2 => 0x4000_0000 | (rng.next_u32() as i32 & 0x3FFF_FFFF),
                _ => [1, 2, 3, i32::MAX, i32::MAX - 1, 0x4000_0000][rng.range(0, 5) as usize],
            };
            let num = match rng.range(0, 5) {
                0 => denum,
                1 => denum - (denum & 1),
                2 => [0, 1, 2, 3][rng.range(0, 3) as usize],
                3 => rng.i32(),
                _ => (rng.next_u32() % (denum as u32).max(1)) as i32,
            };
            let denum = if rng.range(0, 30) == 0 { rng.i32() } else { denum };
            check(num, denum, count);
        }
    }
}
