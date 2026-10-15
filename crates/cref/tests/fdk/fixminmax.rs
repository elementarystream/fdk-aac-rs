// Differential tests of fdk_aac_rs_fdk::fixminmax against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_fixminmax::*;
use fdk_aac_rs_fdk::fixminmax::*;

#[test]
fn fixmin_fixmax() {
    each_dd(501, N, |a, b| {
        assert_eq!(fixmin_d(a, b), unsafe { cref_fixmin_D(a, b) });
        assert_eq!(fixmax_d(a, b), unsafe { cref_fixmax_D(a, b) });
        assert_eq!(fixmin(a, b), unsafe { cref_fixmin_D(a, b) });
        assert_eq!(fixmax(a, b), unsafe { cref_fixmax_D(a, b) });
        assert_eq!(fixmin_i(a, b), unsafe { cref_fixmin_I(a, b) });
        assert_eq!(fixmax_i(a, b), unsafe { cref_fixmax_I(a, b) });
        let (ua, ub) = (a as u32, b as u32);
        assert_eq!(fixmin_ui(ua, ub), unsafe { cref_fixmin_UI(ua, ub) });
        assert_eq!(fixmax_ui(ua, ub), unsafe { cref_fixmax_UI(ua, ub) });
    });
    each_ss(502, N, |a, b| {
        assert_eq!(fixmin_s(a, b), unsafe { cref_fixmin_S(a, b) });
        assert_eq!(fixmax_s(a, b), unsafe { cref_fixmax_S(a, b) });
    });
}
