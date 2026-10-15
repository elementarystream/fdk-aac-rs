// Tests of the optimized FDK_interleave() overloads against the straight ports, which keep
// their differential tests against C (pcm_utils.rs; those run the optimized versions too).

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_pcmutils::pcm_utils::*;

/// Random dimensions (0 to 9 channels, lengths below, at and above the frame size, tile edges)
/// and slice sizes that are sometimes too small (the straight version then panics, and so must
/// the optimized one).
#[test]
fn fdk_interleave_fast_vs_straight() {
    let mut rng = Rng::new(0x1a7e_0001);
    for _ in 0..20000 {
        let ch = rng.next_u32() % 10;
        let fs = match rng.next_u32() % 4 {
            0 => [64, 120, 128, 1024, 2048][(rng.next_u32() % 5) as usize],
            _ => rng.next_u32() % 150,
        };
        let len = match rng.next_u32() % 3 {
            0 => fs,
            1 => rng.next_u32() % (fs + 1),
            _ => fs + rng.next_u32() % 70,
        };
        let full_in = (ch * fs + len) as usize;
        let full_out = (ch * len) as usize;
        let short = rng.next_u32() % 8 == 0;
        let n_in = if short && rng.next_u32() % 2 == 0 { full_in.saturating_sub(1 + rng.next_u32() as usize % 3) } else { full_in };
        let n_out = if short { full_out.saturating_sub(rng.next_u32() as usize % 3) } else { full_out + 8 };
        let in_d: Vec<i32> = (0..n_in).map(|_| rng.i32()).collect();
        let in_s: Vec<i16> = (0..n_in).map(|_| rng.i16()).collect();

        let run_d = |f: fn(&[i32], &mut [i32], u32, u32, u32)| {
            let mut out = vec![0x1234_5678i32; n_out];
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&in_d, &mut out, ch, fs, len))).is_ok();
            (ok, out)
        };
        let (a_ok, a) = run_d(fdk_interleave_straight);
        let (b_ok, b) = run_d(fdk_interleave);
        assert_eq!(a_ok, b_ok, "FDK_interleave(FIXP_DBL, LONG) {ch} {fs} {len} {n_in} {n_out}: panic");
        if a_ok {
            assert_eq!(a, b, "FDK_interleave(FIXP_DBL, LONG) {ch} {fs} {len}");
        }

        let run_s = |f: fn(&[i16], &mut [i16], u32, u32, u32)| {
            let mut out = vec![0x1234i16; n_out];
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&in_s, &mut out, ch, fs, len))).is_ok();
            (ok, out)
        };
        let (a_ok, a) = run_s(fdk_interleave_sgl_straight);
        let (b_ok, b) = run_s(fdk_interleave_sgl);
        assert_eq!(a_ok, b_ok, "FDK_interleave(FIXP_SGL, SHORT) {ch} {fs} {len} {n_in} {n_out}: panic");
        if a_ok {
            assert_eq!(a, b, "FDK_interleave(FIXP_SGL, SHORT) {ch} {fs} {len}");
        }
    }
}
