// Differential tests of fdk_aac_rs_aacdec::aacdec_drc_types against the C++ reference.

use fdk_aac_rs_aacdec::aacdec_drc_types::*;
use fdk_aac_rs_cref::aacdec_aacdec_drc_types as c;

#[test]
fn constants() {
    let mut out = [0i32; 16];
    let n = unsafe { c::cref_drct_constants(out.as_mut_ptr()) } as usize;
    let r = [
        MAX_DRC_THREADS,
        MAX_DRC_BANDS,
        UNKNOWN_PAYLOAD,
        MPEG_DRC_EXT_DATA,
        DVB_DRC_ANC_DATA,
        DISABLED_PARAMETER_HANDLING,
        ENABLED_PARAMETER_HANDLING,
        DRC_PRESENTATION_MODE_1,
        DRC_PRESENTATION_MODE_2,
    ];
    assert_eq!(&out[..n], &r[..]);
}

#[test]
fn array_lengths() {
    let mut out = [0i32; 16];
    let n = unsafe { c::cref_drct_array_lengths(out.as_mut_ptr()) } as usize;
    let d = CDrcChannelData::default();
    let i = CDrcInfo::default();
    let r = [d.band_top.len() as i32, d.drc_value.len() as i32, i.drc_payload_position.len() as i32];
    assert_eq!(&out[..n], &r[..]);
}
