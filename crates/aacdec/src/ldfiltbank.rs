// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/ldfiltbank.h, libAACdec/src/ldfiltbank.cpp.

//! Port of `ldfiltbank.h`: the low delay filter bank (AAC-ELD).
//!
//! `PCM_DEC` is `FIXP_DBL` and `PCM_OUT_BITS` is `DFRACT_BITS` in the reference build, so the
//! output conversion of `multE2_DinvF_fdk()` is `SATURATE_LEFT_SHIFT(x, WTS + 1 - scale,
//! PCM_OUT_BITS)` (the `#else` branches of the C++ code). The `.ii` files of the `aarch64` and
//! `x86` builds differ only in the expansion of `SATURATE_LEFT_SHIFT` (`arm/scale_arm.h` versus
//! the generic `scale.h`), which [`saturate_left_shift`] implements per variant, and in the
//! shared header primitives (`dct_IV`).

use fdk_aac_rs_fdk::archdef::FixpWtb;
use fdk_aac_rs_fdk::common_fix::{
    f_add_saturate, f_mult, f_mult_div2, saturate_left_shift, scale_values_saturate, FixpDbl,
    DFRACT_BITS,
};
use fdk_aac_rs_fdk::dct::dct_iv;
use fdk_aac_rs_fdk::mdct::{imdct_gain, MDCT_OUT_HEADROOM, PCM_OUT_BITS};

use crate::aac_rom::{
    PcmDec, LOW_DELAY_SYNTHESIS_120, LOW_DELAY_SYNTHESIS_128, LOW_DELAY_SYNTHESIS_160,
    LOW_DELAY_SYNTHESIS_240, LOW_DELAY_SYNTHESIS_256, LOW_DELAY_SYNTHESIS_480,
    LOW_DELAY_SYNTHESIS_512, WTS0, WTS1, WTS2,
};

/// `LDFB_HEADROOM`
pub const LDFB_HEADROOM: i32 = 2;

mod fast;

/// `multE2_DinvF_fdk(output, x, fb, z, N)` (static in C): windowing and overlap-add of the low
/// delay filter bank. `z` is the filter state (`fs_buffer`, `3 * N / 2` values), `fb` the
/// window (`3 * N` values).
///
/// The optimized form of the port (module `fast`); [`mult_e2_dinv_f_fdk_straight`] is the
/// straight port.
pub fn mult_e2_dinv_f_fdk(
    output: &mut [PcmDec],
    x: &[FixpDbl],
    fb: &[FixpWtb],
    z: &mut [FixpDbl],
    n: i32,
) {
    let nu = n as usize;
    if n < 4
        || n % 4 != 0
        || output.len() < nu
        || x.len() < nu
        || fb.len() < 3 * nu
        || z.len() < nu * 3 / 2
    {
        return mult_e2_dinv_f_fdk_straight(output, x, fb, z, n);
    }
    fast::mult_e2(&mut output[..nu], &x[..nu], &fb[..3 * nu], &mut z[..nu * 3 / 2], nu);
}

/// `multE2_DinvF_fdk()`, the straight port (the reference of [`mult_e2_dinv_f_fdk`]).
pub fn mult_e2_dinv_f_fdk_straight(
    output: &mut [PcmDec],
    x: &[FixpDbl],
    fb: &[FixpWtb],
    z: &mut [FixpDbl],
    n: i32,
) {
    let n = n as usize;

    /*  scale for FIXP_DBL -> PCM_DEC conversion:       */
    let scale: i32 = (DFRACT_BITS - PCM_OUT_BITS) - LDFB_HEADROOM + 3;

    for i in 0..n / 4 {
        let z2 = x[n / 2 + i];
        let z0 = f_add_saturate(z2, f_mult_div2(z[n / 2 + i], fb[2 * n + i]) >> (-WTS2 - 1));

        z[n / 2 + i] = f_add_saturate(
            x[n / 2 - 1 - i],
            f_mult_div2(z[n + i], fb[2 * n + n / 2 + i]) >> (-WTS2 - 1),
        );

        let tmp = f_mult_div2(z[n / 2 + i], fb[n + n / 2 - 1 - i])
            .wrapping_add(f_mult_div2(z[i], fb[n + n / 2 + i]));

        output[n * 3 / 4 - 1 - i] = saturate_left_shift(tmp, WTS1 + 1 - scale, PCM_OUT_BITS);

        z[i] = z0;
        z[n + i] = z2;
    }

    for i in n / 4..n / 2 {
        let z2 = x[n / 2 + i];
        let z0 = f_add_saturate(z2, f_mult_div2(z[n / 2 + i], fb[2 * n + i]) >> (-WTS2 - 1));

        z[n / 2 + i] = f_add_saturate(
            x[n / 2 - 1 - i],
            f_mult_div2(z[n + i], fb[2 * n + n / 2 + i]) >> (-WTS2 - 1),
        );

        let tmp0 = f_mult_div2(z[n / 2 + i], fb[n / 2 - 1 - i])
            .wrapping_add(f_mult_div2(z[i], fb[n / 2 + i]));
        let tmp1 = f_mult_div2(z[n / 2 + i], fb[n + n / 2 - 1 - i])
            .wrapping_add(f_mult_div2(z[i], fb[n + n / 2 + i]));

        output[i - n / 4] = saturate_left_shift(tmp0, WTS0 + 1 - scale, PCM_OUT_BITS);
        output[n * 3 / 4 - 1 - i] = saturate_left_shift(tmp1, WTS1 + 1 - scale, PCM_OUT_BITS);
        z[i] = z0;
        z[n + i] = z2;
    }

    /* Exchange quarter parts of x to bring them in the "right" order */
    for i in 0..n / 4 {
        let tmp0 = f_mult_div2(z[i], fb[n / 2 + i]);

        output[n * 3 / 4 + i] = saturate_left_shift(tmp0, WTS0 + 1 - scale, PCM_OUT_BITS);
    }
}

/// `InvMdctTransformLowDelay_fdk(mdctData, mdctData_e, output, fs_buffer, N)`: the inverse low
/// delay MDCT. `mdct_data` is the spectrum (`N` values, transformed in place), `output` receives
/// `N` samples, `fs_buffer` is the filter state (the channel's `pOverlapBuffer`). Returns 1.
///
/// Frame lengths other than 120, 128, 160, 240, 256, 480 and 512 use the 480 window, as in C
/// (`default:`). Buffers shorter than the lengths that the frame length implies leave
/// everything unchanged.
pub fn inv_mdct_transform_low_delay_fdk(
    mdct_data: &mut [FixpDbl],
    mdct_data_e: i32,
    output: &mut [PcmDec],
    fs_buffer: &mut [FixpDbl],
    n: i32,
) -> i32 {
    let mut gain: FixpDbl = 0;
    /* The LDFB_HEADROOM is compensated inside multE2_DinvF_fdk() below */
    let mut scale = mdct_data_e + MDCT_OUT_HEADROOM - LDFB_HEADROOM;

    /* Select LD window slope */
    let coef: &[FixpWtb] = match n {
        256 => &LOW_DELAY_SYNTHESIS_256,
        240 => &LOW_DELAY_SYNTHESIS_240,
        160 => &LOW_DELAY_SYNTHESIS_160,
        128 => &LOW_DELAY_SYNTHESIS_128,
        120 => &LOW_DELAY_SYNTHESIS_120,
        512 => &LOW_DELAY_SYNTHESIS_512,
        _ => &LOW_DELAY_SYNTHESIS_480,
    };

    let nu = n.max(0) as usize;
    if mdct_data.len() < nu
        || output.len() < nu
        || fs_buffer.len() < nu * 3 / 2
        || coef.len() < 3 * nu
        || nu < 4
    {
        return 1;
    }

    /*
      Apply exponent and 1/N factor.
      Note: "scale" is off by one because for LD_MDCT the window length is twice
      the window length of a regular MDCT. This is corrected inside
      multE2_DinvF_fdk(). Refer to ISO/IEC 14496-3:2009 page 277,
      chapter 4.6.20.2 "Low Delay Window".
    */
    imdct_gain(&mut gain, &mut scale, n);

    dct_iv(&mut mdct_data[..nu], n, &mut scale);

    if n == 256 || n == 240 || n == 160 {
        scale -= 1;
    } else if n == 128 || n == 120 {
        scale -= 2;
    }

    if gain != 0 {
        for v in mdct_data[..nu].iter_mut() {
            *v = f_mult(*v, gain);
        }
    }
    scale_values_saturate(&mut mdct_data[..nu], scale);

    /* Since all exponent and factors have been applied, current exponent is zero. */
    mult_e2_dinv_f_fdk(output, mdct_data, coef, fs_buffer, n);

    1
}
