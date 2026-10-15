// Differential tests of fdk_aac_rs_fdk::clz against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_clz::*;
use fdk_aac_rs_fdk::clz::*;

#[test]
fn fixnormz_D_fixnorm_D() {
    let check = |x: i32| {
        assert_eq!(fixnormz_d(x), unsafe { cref_fixnormz_D(x) }, "fixnormz_D({x:#x})");
        assert_eq!(fixnorm_d(x), unsafe { cref_fixnorm_D(x) }, "fixnorm_D({x:#x})");
    };
    for i in 0..32 {
        check(1 << i);
        check((1u32 << i).wrapping_sub(1) as i32);
        check(-(1i64 << i) as i32);
        check(!((1u32 << i) as i32));
    }
    each_d(401, N, check);
}

#[test]
fn fixnormz_S_fixnorm_S() {
    // Exhaustive.
    each_s(402, 0, |x| {
        assert_eq!(fixnormz_s(x), unsafe { cref_fixnormz_S(x) }, "fixnormz_S({x})");
        assert_eq!(fixnorm_s(x), unsafe { cref_fixnorm_S(x) }, "fixnorm_S({x})");
    });
}
