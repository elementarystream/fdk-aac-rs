// Differential tests of fdk_aac_rs_sacdec::sac_dec_errorcodes against the C++ reference.

use fdk_aac_rs_cref::sacdec_sac_dec_errorcodes as c;
use fdk_aac_rs_sacdec::sac_dec_errorcodes::*;

#[test]
fn sac_dec_errorcodes_values() {
    let rs = [
        __MPS_ERROR_START,
        MPS_OK,
        MPS_NOTOK,
        MPS_OUTOFMEMORY,
        MPS_INVALID_HANDLE,
        MPS_INVALID_PARAMETER,
        MPS_UNSUPPORTED_HRTFMODEL,
        MPS_UNSUPPORTED_HRTFFREQ,
        MPS_UNSUPPORTED_UPMIX_TYPE,
        MPS_UNSUPPORTED_FORMAT,
        MPS_OUTPUT_BUFFER_TOO_SMALL,
        MPS_INVALID_PARAMETERBANDS,
        MPS_INVALID_TREECONFIG,
        MPS_INVALID_HRTFSET,
        MPS_INVALID_TTT,
        MPS_INVALID_BOXIDX,
        MPS_INVALID_SETIDX,
        MPS_INVALID_QUANTMODE,
        MPS_UNEQUAL_SSC,
        MPS_UNSUPPORTED_CONFIG,
        MPS_PARSE_ERROR,
        MPS_INVALID_TEMPSHAPE,
        MPS_WRONG_PARAMETERSETS,
        MPS_WRONG_PARAMETERBANDS,
        MPS_WRONG_TREECONFIG,
        MPS_WRONG_BLINDCONFIG,
        MPS_WRONG_OTT,
        MPS_WRONG_QUANTMODE,
        MPS_RESDEC_ERROR,
        MPS_APPLY_M2_ERROR,
        __MPS_ERROR_END,
    ];
    let mut out = [0i32; 64];
    let n = unsafe { c::cref_sacdec_error_codes(out.as_mut_ptr()) } as usize;
    assert_eq!(&out[..n], &rs[..]);
}
