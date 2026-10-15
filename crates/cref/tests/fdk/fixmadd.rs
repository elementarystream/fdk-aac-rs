// Differential tests of fdk_aac_rs_fdk::fixmadd against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_fixmadd::*;
use fdk_aac_rs_fdk::fixmadd::*;

#[test]
fn fixmadd_DD() {
    each_ddd(201, N, |x, a, b| {
        assert_eq!(fixmadddiv2_dd(x, a, b), unsafe { cref_fixmadddiv2_DD(x, a, b) }, "{x} {a} {b}");
        assert_eq!(fixmsubdiv2_dd(x, a, b), unsafe { cref_fixmsubdiv2_DD(x, a, b) });
        assert_eq!(fixmadddiv2_bit_exact_dd(x, a, b), unsafe {
            cref_fixmadddiv2BitExact_DD(x, a, b)
        });
        assert_eq!(fixmsubdiv2_bit_exact_dd(x, a, b), unsafe {
            cref_fixmsubdiv2BitExact_DD(x, a, b)
        });
        assert_eq!(fixmadd_dd(x, a, b), unsafe { cref_fixmadd_DD(x, a, b) });
        assert_eq!(fixmsub_dd(x, a, b), unsafe { cref_fixmsub_DD(x, a, b) });
    });
}

#[test]
fn fixmadd_SD_DS() {
    each_dsd(202, N, |x, a, b| {
        assert_eq!(fixmadddiv2_sd(x, a, b), unsafe { cref_fixmadddiv2_SD(x, a, b) });
        assert_eq!(fixmadddiv2_ds(x, b, a), unsafe { cref_fixmadddiv2_DS(x, b, a) });
        assert_eq!(fixmsubdiv2_sd(x, a, b), unsafe { cref_fixmsubdiv2_SD(x, a, b) });
        assert_eq!(fixmsubdiv2_ds(x, b, a), unsafe { cref_fixmsubdiv2_DS(x, b, a) });
        assert_eq!(fixmadddiv2_bit_exact_sd(x, a, b), unsafe {
            cref_fixmadddiv2BitExact_SD(x, a, b)
        });
        assert_eq!(fixmadddiv2_bit_exact_ds(x, b, a), unsafe {
            cref_fixmadddiv2BitExact_DS(x, b, a)
        });
        assert_eq!(fixmsubdiv2_bit_exact_sd(x, a, b), unsafe {
            cref_fixmsubdiv2BitExact_SD(x, a, b)
        });
        assert_eq!(fixmsubdiv2_bit_exact_ds(x, b, a), unsafe {
            cref_fixmsubdiv2BitExact_DS(x, b, a)
        });
        assert_eq!(fixmadd_sd(x, a, b), unsafe { cref_fixmadd_SD(x, a, b) });
        assert_eq!(fixmadd_ds(x, b, a), unsafe { cref_fixmadd_DS(x, b, a) });
        assert_eq!(fixmsub_sd(x, a, b), unsafe { cref_fixmsub_SD(x, a, b) });
        assert_eq!(fixmsub_ds(x, b, a), unsafe { cref_fixmsub_DS(x, b, a) });
    });
}

#[test]
fn fixmadd_SS() {
    each_dss(203, N, |x, a, b| {
        assert_eq!(fixmadddiv2_ss(x, a, b), unsafe { cref_fixmadddiv2_SS(x, a, b) });
        assert_eq!(fixmsubdiv2_ss(x, a, b), unsafe { cref_fixmsubdiv2_SS(x, a, b) });
        assert_eq!(fixmadd_ss(x, a, b), unsafe { cref_fixmadd_SS(x, a, b) });
        assert_eq!(fixmsub_ss(x, a, b), unsafe { cref_fixmsub_SS(x, a, b) });
    });
}

#[test]
fn fixpadd_D_S() {
    each_dd(204, N, |x, a| {
        assert_eq!(fixpadddiv2_d(x, a), unsafe { cref_fixpadddiv2_D(x, a) });
        assert_eq!(fixpadd_d(x, a), unsafe { cref_fixpadd_D(x, a) });
    });
    each_sd(205, N, |a, x| {
        assert_eq!(fixpadddiv2_s(x, a), unsafe { cref_fixpadddiv2_S(x, a) });
        assert_eq!(fixpadd_s(x, a), unsafe { cref_fixpadd_S(x, a) });
    });
}
