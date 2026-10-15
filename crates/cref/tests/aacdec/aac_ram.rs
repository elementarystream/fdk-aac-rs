// Differential tests of fdk_aac_rs_aacdec::aac_ram against the C++ reference.
//
// The sizes that the decoder uses (GetRequiredMem of the buffers whose size does not depend on
// the C layout of a structure), the constants, and the initial contents: every C allocation is
// zero-filled, and so is every value the port returns (checked through the Debug output: every
// number in it is 0, every Option is None).

use fdk_aac_rs_aacdec::aac_ram::*;
use fdk_aac_rs_cref::aacdec_aac_ram as c;

#[test]
fn constants() {
    let mut out = [0i32; 16];
    let n = unsafe { c::cref_ram_constants(out.as_mut_ptr()) } as usize;
    let r = [
        MAX_SYNCHS,
        SAMPL_FREQS,
        TIME_DATA_FLUSH_SIZE,
        get_required_mem_overlap_buffer() as i32,
        get_required_mem_time_data_flush() as i32,
        get_required_mem_work_buffer_core2() as i32,
        get_required_mem_work_buffer_core1() as i32,
        get_required_mem_work_buffer_core5() as i32,
    ];
    assert_eq!(&out[..n], &r[..]);
}

/// Every number in the Debug output of `v` is 0 and nothing is `Some`/`true` (field names may
/// contain digits; they are part of identifier tokens).
fn assert_zero<T: std::fmt::Debug>(what: &str, v: &T) {
    let s = format!("{v:?}");
    for tok in s.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')) {
        if tok.is_empty() {
            continue;
        }
        if tok.chars().all(|ch| ch.is_ascii_digit()) {
            assert_eq!(tok, "0", "{what}: nonzero value");
        }
        assert_ne!(tok, "Some", "{what}: Some");
        assert_ne!(tok, "true", "{what}: true");
    }
}

#[test]
fn zero_filled() {
    let mut out = [0i32; 16];
    let n = unsafe { c::cref_ram_zeroed(out.as_mut_ptr()) } as usize;
    assert_eq!(&out[..n], &[1; 12][..], "the C allocations are zero-filled");

    let s = get_aac_decoder_static_channel_info(0);
    assert_zero("GetAacDecoderStaticChannelInfo", &s);
    let mut s = Some(s);
    free_aac_decoder_static_channel_info(&mut s);
    assert!(s.is_none());
    let ci = get_aac_decoder_channel_info(7);
    assert_zero("GetAacDecoderChannelInfo", &ci);
    let ov = get_overlap_buffer(0);
    assert_eq!(ov.len(), 768);
    assert_zero("GetOverlapBuffer", &ov);
    assert_zero("GetDrcInfo", &get_drc_info(0));
    assert_zero("GetCpePersistentData", &get_cpe_persistent_data(0));
    assert_zero("GetCplxPredictionData", &get_cplx_prediction_data(0));
    let tf = get_time_data_flush(0);
    assert_eq!(tf.len(), TIME_DATA_FLUSH_SIZE as usize);
    assert_zero("GetTimeDataFlush", &tf);
    let w2 = get_work_buffer_core2(0);
    assert_eq!(w2.len(), 8 * 1024);
    assert_zero("GetWorkBufferCore2", &w2);
    assert_zero("GetWorkBufferCore6", &get_work_buffer_core6(0));
    assert_zero("GetWorkBufferCore1", &get_work_buffer_core1(0));
    let w5 = get_work_buffer_core5(0);
    assert_eq!(w5.len(), 8 * 4096 * 2);
    assert_zero("GetWorkBufferCore5", &w5);
}
