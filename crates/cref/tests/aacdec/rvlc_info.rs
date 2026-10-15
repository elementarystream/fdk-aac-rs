// Differential tests of fdk_aac_rs_aacdec::rvlc_info against the C++ reference.

use fdk_aac_rs_aacdec::rvlc_info::*;
use fdk_aac_rs_cref::aacdec_rvlc_info as c;

#[test]
fn constants() {
    let mut out = [0i32; 32];
    let n = unsafe { c::cref_rvlci_constants(out.as_mut_ptr()) } as usize;
    let r = [
        FWD as i32,
        BWD as i32,
        MAX_RVL,
        MIN_RVL,
        MAX_ALLOWED_DPCM_INDEX,
        TABLE_OFFSET,
        MAX_LEN_RVLC_CODE_WORD,
        MAX_LEN_RVLC_ESCAPE_WORD,
        DPCM_NOISE_NRG_BITS,
        SF_OFFSET,
        CONCEAL_MAX_INIT,
        CONCEAL_MIN_INIT,
        RVLC_MAX_SFB,
    ];
    assert_eq!(&out[..n], &r[..]);
}

/// `pRvlBitCnt_RVL` / `pBitstrIndxRvl_RVL` select the forward or backward fields (port only:
/// the C code assigns the addresses of these fields).
#[test]
fn rvl_selectors() {
    let mut r = CErRvlcInfo { length_of_rvlc_sf_fwd: 3, length_of_rvlc_sf_bwd: 5, ..Default::default() };
    r.bitstream_index_rvl_fwd = 7;
    r.bitstream_index_rvl_bwd = 9;
    assert_eq!(*r.rvl_bit_cnt_rvl(), 3);
    assert_eq!(*r.bitstr_indx_rvl_rvl(), 7);
    r.p_rvl_bit_cnt_rvl = BWD;
    r.p_bitstr_indx_rvl_rvl = BWD;
    *r.rvl_bit_cnt_rvl() -= 1;
    *r.bitstr_indx_rvl_rvl() += 1;
    assert_eq!((r.length_of_rvlc_sf_fwd, r.length_of_rvlc_sf_bwd), (3, 4));
    assert_eq!((r.bitstream_index_rvl_fwd, r.bitstream_index_rvl_bwd), (7, 10));
}
