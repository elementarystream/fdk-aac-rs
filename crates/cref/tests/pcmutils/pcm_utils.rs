// Differential tests of fdk_aac_rs_pcmutils::pcm_utils against the C++ reference.
//
// Every overload of FDK_interleave() and FDK_deinterleave() on random data with random channel
// counts, frame sizes and lengths (length < frameSize, = frameSize and > frameSize). The output
// buffers are prefilled with a pattern, so that writes beyond the expected range are detected.

use fdk_aac_rs_cref::pcmutils_pcm_utils as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_pcmutils::pcm_utils::*;

/// Random (channels, frameSize, length) with sizes for the input and the output.
fn dims(rng: &mut Rng) -> (u32, u32, u32) {
    let channels = rng.next_u32() % 10;
    let frame_size = match rng.next_u32() % 4 {
        0 => [120, 128, 240, 256, 480, 512, 960, 1024, 2048][(rng.next_u32() % 9) as usize],
        _ => rng.next_u32() % 70,
    };
    let length = match rng.next_u32() % 3 {
        0 => frame_size,
        1 => rng.next_u32() % (frame_size + 1),
        _ => frame_size + rng.next_u32() % 20,
    };
    (channels, frame_size, length)
}

#[test]
fn fdk_interleave_() {
    let mut rng = Rng::new(601);
    for _ in 0..3000 {
        let (ch, fs, len) = dims(&mut rng);
        // FDK_interleave reads pIn[sample + ch * frameSize] for sample < length.
        let n_in = (ch * fs + len) as usize;
        let n_out = (ch * len) as usize + 8;
        let in_d: Vec<i32> = (0..n_in).map(|_| rng.i32()).collect();
        let in_s: Vec<i16> = (0..n_in).map(|_| rng.i16()).collect();

        let mut r = vec![0x1234_5678i32; n_out];
        let mut cc = r.clone();
        fdk_interleave(&in_d, &mut r, ch, fs, len);
        unsafe { c::cref_pcm_utils_interleave_dbl_long(in_d.as_ptr(), cc.as_mut_ptr(), ch, fs, len) };
        assert_eq!(r, cc, "FDK_interleave(FIXP_DBL, LONG) {ch} {fs} {len}");

        let mut r = vec![0x1234i16; n_out];
        let mut cc = r.clone();
        fdk_interleave_dbl_to_sgl(&in_d, &mut r, ch, fs, len);
        unsafe { c::cref_pcm_utils_interleave_dbl_sgl(in_d.as_ptr(), cc.as_mut_ptr(), ch, fs, len) };
        assert_eq!(r, cc, "FDK_interleave(FIXP_DBL, SHORT) {ch} {fs} {len}");

        let mut r = vec![0x1234i16; n_out];
        let mut cc = r.clone();
        fdk_interleave_sgl(&in_s, &mut r, ch, fs, len);
        unsafe { c::cref_pcm_utils_interleave_sgl_sgl(in_s.as_ptr(), cc.as_mut_ptr(), ch, fs, len) };
        assert_eq!(r, cc, "FDK_interleave(FIXP_SGL, SHORT) {ch} {fs} {len}");
    }
}

#[test]
fn fdk_deinterleave_() {
    let mut rng = Rng::new(602);
    for _ in 0..3000 {
        let (ch, fs, len) = dims(&mut rng);
        // FDK_deinterleave reads pIn[ch + sample * channels] for sample < frameSize and writes
        // pOut[length * ch + sample].
        let n_in = (ch * fs) as usize;
        let n_out = if ch == 0 { 8 } else { (len * (ch - 1) + fs) as usize + 8 };
        let in_d: Vec<i32> = (0..n_in).map(|_| rng.i32()).collect();
        let in_s: Vec<i16> = (0..n_in).map(|_| rng.i16()).collect();

        let mut r = vec![0x1234i16; n_out];
        let mut cc = r.clone();
        fdk_deinterleave_long_to_sgl(&in_d, &mut r, ch, fs, len);
        unsafe { c::cref_pcm_utils_deinterleave_long_sgl(in_d.as_ptr(), cc.as_mut_ptr(), ch, fs, len) };
        assert_eq!(r, cc, "FDK_deinterleave(LONG, SHORT) {ch} {fs} {len}");

        let mut r = vec![0x1234_5678i32; n_out];
        let mut cc = r.clone();
        fdk_deinterleave(&in_d, &mut r, ch, fs, len);
        unsafe { c::cref_pcm_utils_deinterleave_long_long(in_d.as_ptr(), cc.as_mut_ptr(), ch, fs, len) };
        assert_eq!(r, cc, "FDK_deinterleave(LONG, LONG) {ch} {fs} {len}");

        let mut r = vec![0x1234i16; n_out];
        let mut cc = r.clone();
        fdk_deinterleave_sgl(&in_s, &mut r, ch, fs, len);
        unsafe { c::cref_pcm_utils_deinterleave_sgl_sgl(in_s.as_ptr(), cc.as_mut_ptr(), ch, fs, len) };
        assert_eq!(r, cc, "FDK_deinterleave(SHORT, SHORT) {ch} {fs} {len}");

        let mut r = vec![0x1234_5678i32; n_out];
        let mut cc = r.clone();
        fdk_deinterleave_sgl_to_long(&in_s, &mut r, ch, fs, len);
        unsafe { c::cref_pcm_utils_deinterleave_sgl_long(in_s.as_ptr(), cc.as_mut_ptr(), ch, fs, len) };
        assert_eq!(r, cc, "FDK_deinterleave(SHORT, LONG) {ch} {fs} {len}");
    }
}
