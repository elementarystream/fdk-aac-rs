// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_filter.h, libSACenc/src/sacenc_filter.cpp.

//! Port of `sacenc_filter.h` and `sacenc_filter.cpp`: the DC filter of the input signals.
//!
//! # Variants
//!
//! The code is identical in the `aarch64` and `x86` builds; the results differ through
//! `fMult` and `fDivNormHighPrec` (`schur_div`; x86 division by zero gives the arm64 result).
//! The filter recursion can overflow; both builds wrap (scalar `add`/`sub`).

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::machine_type::IntPcm;

use crate::sacenc_lib::*;

/// `DC_FILTER_SF`
pub const DC_FILTER_SF: i32 = 1;

/// `DC_FILTER` (`struct T_DC_FILTER`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DcFilter {
    pub c_fdk: FixpDbl,
    pub state_fdk: FixpDbl,
}

/// `HANDLE_DC_FILTER`
pub type HandleDcFilter = Option<Box<DcFilter>>;

/// `fdk_sacenc_createDCFilter(hDCFilter)`
pub fn fdk_sacenc_create_dc_filter(h_dc_filter: Option<&mut HandleDcFilter>) -> FdkSacencError {
    match h_dc_filter {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = Some(Box::default());
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_initDCFilter(hDCFilter, sampleRate)`
pub fn fdk_sacenc_init_dc_filter(h_dc_filter: &mut DcFilter, sample_rate: u32) -> FdkSacencError {
    let mut s = 0;

    // Conversion for use of CalcInvLdData: e^x = 2^(x*log10(e)/log10(2) =
    // CalcInvLdData(x*log10(e)/log10(2)/64.0) 1.44269504089 = log10(e)/log10(2)
    // 0.5 = scale constant value with 1 Bits
    let mut exp_c = f_div_norm_high_prec(20, sample_rate as FixpDbl, &mut s);
    exp_c = f_mult_div2(fl2fxconst_dbl(-1.44269504089 * 0.5), exp_c) >> (LD_DATA_SHIFT - 1 - 1);

    if s < 0 {
        exp_c = exp_c.wrapping_shr(s.wrapping_neg() as u32);
    } else {
        exp_c = exp_c.wrapping_shl(s as u32);
    }

    exp_c = calc_inv_ld_data(exp_c);

    h_dc_filter.c_fdk = exp_c;
    h_dc_filter.state_fdk = 0;

    SACENC_OK
}

/// `fdk_sacenc_destroyDCFilter(hDCFilter)`
pub fn fdk_sacenc_destroy_dc_filter(h_dc_filter: Option<&mut HandleDcFilter>) -> FdkSacencError {
    if let Some(h) = h_dc_filter {
        *h = None;
    }
    SACENC_OK
}

/// `fdk_sacenc_applyDCFilter(hDCFilter, signalIn, signalOut, signalLength)` with
/// `signalIn == signalOut` (the encoder filters in place).
pub fn fdk_sacenc_apply_dc_filter_in_place(
    h_dc_filter: &mut DcFilter,
    signal: &mut [IntPcm],
    signal_length: i32,
) -> FdkSacencError {
    let c = h_dc_filter.c_fdk;
    let n = signal_length.max(1) as usize;

    // x[0] is read before any output is written; y[i - 1] is written after x[i] is read.
    let mut x0 = fx_pcm2fx_dbl(signal[0]) >> DC_FILTER_SF;
    let mut x1 = x0;
    let mut y1 = x0.wrapping_add(h_dc_filter.state_fdk);

    let mut i = 1usize;
    while i < n {
        x0 = fx_pcm2fx_dbl(signal[i]) >> DC_FILTER_SF;
        signal[i - 1] = fx_dbl2fx_pcm(y1);
        y1 = x0.wrapping_sub(x1).wrapping_add(f_mult(c, y1));
        x1 = x0;
        i += 1;
    }

    h_dc_filter.state_fdk = f_mult(c, y1).wrapping_sub(x1);
    signal[i - 1] = fx_dbl2fx_pcm(y1);

    SACENC_OK
}

/// `fdk_sacenc_applyDCFilter(hDCFilter, signalIn, signalOut, signalLength)`
pub fn fdk_sacenc_apply_dc_filter(
    h_dc_filter: &mut DcFilter,
    signal_in: &[IntPcm],
    signal_out: &mut [IntPcm],
    signal_length: i32,
) -> FdkSacencError {
    let n = signal_length.max(1) as usize;
    signal_out[..n].copy_from_slice(&signal_in[..n]);
    fdk_sacenc_apply_dc_filter_in_place(h_dc_filter, signal_out, signal_length)
}
