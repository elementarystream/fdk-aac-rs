// Differential tests of fdk_aac_rs_aacdec::usacdec_ace_ltp against the C++ reference.

use crate::usacdec_lpd_util::rand_dbl_vec;
use fdk_aac_rs_aacdec::usacdec_ace_ltp::*;
use fdk_aac_rs_cref::aacdec_usacdec_ace_ltp as c;
use fdk_aac_rs_cref::testutil::Rng;

/// The excitation buffer of `CLpd_AcelpDecode()`: `PIT_MAX_MAX + L_INTERPOL + L_DIV + 1`.
const EXC_BUF: usize = 411 + 17 + 256 + 1;

#[test]
fn pred_lt4_random() {
    let mut rng = Rng::new(0x12AC_0001);
    for it in 0..40_000 {
        let buf = rand_dbl_vec(&mut rng, EXC_BUF);
        // exc = exc_buf + PIT_MAX_MAX + L_INTERPOL + i_subfr
        let off = 428 + 64 * (rng.next_u32() % 4) as usize;
        let t0 = match it % 4 {
            0 => rng.range(16, 411),
            1 => rng.range(16, 80),
            2 => [16, 17, 34, 64, 65, 231, 410, 411][(rng.next_u32() % 8) as usize],
            _ => rng.range(16, 411),
        };
        let frac = (rng.next_u32() % 4) as i32;
        let mut r = buf.clone();
        let mut cc = buf;
        pred_lt4(&mut r, off, t0, frac);
        unsafe { c::cref_ltp_pred_lt4(cc.as_mut_ptr(), off as i32, t0, frac) };
        assert_eq!(r, cc, "T0 {t0} frac {frac} off {off}");
    }
}

#[test]
fn pred_lt4_postfilter_random() {
    let mut rng = Rng::new(0x12AC_0002);
    for _ in 0..40_000 {
        let buf = rand_dbl_vec(&mut rng, EXC_BUF);
        let off = 428 + 64 * (rng.next_u32() % 4) as usize;
        let mut r = buf.clone();
        let mut cc = buf;
        pred_lt4_postfilter(&mut r, off);
        unsafe { c::cref_ltp_pred_lt4_postfilter(cc.as_mut_ptr(), off as i32) };
        assert_eq!(r, cc, "off {off}");
    }
}
