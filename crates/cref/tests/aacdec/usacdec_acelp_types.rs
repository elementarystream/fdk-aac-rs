// Differential tests of the types of fdk_aac_rs_aacdec::usacdec_acelp against the C++
// reference.

use fdk_aac_rs_aacdec::usacdec_acelp::*;
use fdk_aac_rs_cref::aacdec_usacdec_acelp_types as c;

#[test]
fn array_lengths() {
    let mut out = [0i32; 16];
    let n = unsafe { c::cref_acelpt_array_lengths(out.as_mut_ptr()) } as usize;
    let m = CAcelpStaticMem::default();
    let d = CAcelpChannelData::default();
    let r = [
        m.old_exc_mem.len(),
        m.old_syn_mem.len(),
        m.a.len(),
        d.t0.len(),
        d.t0_frac.len(),
        d.ltp_filtering_flag.len(),
        d.icb_index.len() * d.icb_index[0].len(),
        d.icb_index.len(),
        d.gains.len(),
    ];
    let r: Vec<i32> = r.iter().map(|&x| x as i32).collect();
    assert_eq!(&out[..n], &r[..]);
}
