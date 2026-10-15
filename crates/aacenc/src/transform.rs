// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/transform.h, libAACenc/src/transform.cpp.

//! Port of `transform.h`: MDCT of the AAC-LC/LD encoder and the analysis filter bank of AAC-ELD.
//!
//! The `aarch64` and `x86` translation units of `transform.cpp` are identical apart from
//! `FDK_ASSERT`; the variants differ only in the primitives and in `dct_IV`.
//!
//! # Window transitions
//!
//! [`mdct_block`] derives the left window slope of a transform from the right slope of the
//! previous one. Where the reference has no defined result, `mdct_block` skips the accesses
//! outside the buffers and reads a zero slope; see its documentation.

use crate::aac_enc_rom::{
    ELD_ANALYSIS_120, ELD_ANALYSIS_128, ELD_ANALYSIS_240, ELD_ANALYSIS_256, ELD_ANALYSIS_480,
    ELD_ANALYSIS_512,
};
use crate::psy_const::{LOL_WINDOW, LONG_WINDOW, SHORT_WINDOW, START_WINDOW, STOP_WINDOW};
use fdk_aac_rs_fdk::archdef::FixpWtb;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::dct::dct_iv;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::mdct::{mdct_block, MdctT};
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;

/// `WTS0`
pub const WTS0: i32 = 1;
/// `WTS1`
pub const WTS1: i32 = 0;
/// `WTS2`
pub const WTS2: i32 = -2;

/// `FDKaacEnc_Transform_Real()`: MDCT of time domain data.
///
/// - `p_time_data`: time domain input signal (`2 * frame_length` samples are read).
/// - `mdct_data`: frequency domain output data (`frame_length` values).
/// - `block_type`: `LONG_WINDOW`, `START_WINDOW`, `SHORT_WINDOW` or `STOP_WINDOW`.
/// - `window_shape`: `SINE_WINDOW`, `KBD_WINDOW` or `LOL_WINDOW` (low overlap, AAC-LD).
/// - `prev_window_shape`: set to `window_shape` on success.
/// - `frame_length`: length of the block (1024, 960, 512 or 480).
/// - `p_mdct_data_e`: exponent of the frequency domain output data.
///
/// Returns 0 in case of success, -1 in case of inconsistent parameters (an unknown block type,
/// or short windows with different exponents).
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_transform_real(
    p_time_data: &[IntPcm],
    mdct_data: &mut [FixpDbl],
    block_type: i32,
    window_shape: i32,
    prev_window_shape: &mut i32,
    mdct_pers: &mut MdctT,
    frame_length: i32,
    p_mdct_data_e: &mut i32,
    _filter_type: i32,
) -> i32 {
    let time_data = p_time_data;

    let num_spec: u32;
    let num_mdct_lines: u32;
    let fr: i32; // fr: right window slope length
    let mut mdct_data_e = [0i16; 8];

    if block_type == SHORT_WINDOW {
        num_spec = 8;
        num_mdct_lines = (frame_length >> 3) as u32;
    } else {
        num_spec = 1;
        num_mdct_lines = frame_length as u32;
    }

    let offset: u32 = if window_shape == LOL_WINDOW { ((frame_length * 3) >> 2) as u32 } else { 0 };
    match block_type {
        LONG_WINDOW | STOP_WINDOW => {
            fr = (frame_length as u32).wrapping_sub(offset) as i32;
        }
        // or StopStartSequence
        START_WINDOW | SHORT_WINDOW => {
            fr = frame_length >> 3;
        }
        _ => {
            fdk_assert!(false);
            return -1;
        }
    }

    mdct_block(
        mdct_pers,
        time_data,
        frame_length,
        mdct_data,
        num_spec as i32,
        num_mdct_lines as i32,
        fdk_get_window_slope(fr, window_shape),
        fr,
        &mut mdct_data_e,
    );

    if block_type == SHORT_WINDOW && !mdct_data_e.iter().all(|&e| e == mdct_data_e[0]) {
        return -1;
    }
    *prev_window_shape = window_shape;
    *p_mdct_data_e = mdct_data_e[0] as i32;

    0
}

/// `FDKaacEnc_Transform_Real_Eld()`: ELD analysis filter bank of time domain data.
///
/// - `p_time_data`: time domain input signal (`2 * frame_length` samples are read).
/// - `mdct_data`: frequency domain output data (`frame_length` values).
/// - `block_type`: must be `LONG_WINDOW`.
/// - `prev_window_shape`: set to `window_shape` on success.
/// - `frame_length`: 512, 480, 256, 240, 128 or 120.
/// - `mdct_data_e`: exponent of the frequency domain output data (set to 2 before the frame
///   length is checked, as in C).
/// - `overlap_add_buffer`: overlap of the ELD filter bank (`3 * frame_length / 2` values).
///
/// Returns 0 in case of success, -1 for another block type or frame length.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_transform_real_eld(
    p_time_data: &[IntPcm],
    mdct_data: &mut [FixpDbl],
    block_type: i32,
    window_shape: i32,
    prev_window_shape: &mut i32,
    frame_length: i32,
    mdct_data_e: &mut i32,
    _filter_type: i32,
    overlap_add_buffer: &mut [FixpDbl],
) -> i32 {
    let time_data = p_time_data;

    // tl: transform length
    // fl: left window slope length
    // nl: left window slope offset
    // fr: right window slope length
    // nr: right window slope offset
    let p_window_eld: &[FixpWtb];
    let n = frame_length;
    let l = frame_length;

    if block_type != LONG_WINDOW {
        return -1;
    }

    // MDCT scale:
    // + 1: fMultDiv2() in windowing.
    // + 1: Because of factor 1/2 in Princen-Bradley compliant windowed TDAC.
    *mdct_data_e = 1 + 1;

    match frame_length {
        512 => p_window_eld = &ELD_ANALYSIS_512,
        480 => p_window_eld = &ELD_ANALYSIS_480,
        256 => {
            p_window_eld = &ELD_ANALYSIS_256;
            *mdct_data_e += 1;
        }
        240 => {
            p_window_eld = &ELD_ANALYSIS_240;
            *mdct_data_e += 1;
        }
        128 => {
            p_window_eld = &ELD_ANALYSIS_128;
            *mdct_data_e += 2;
        }
        120 => {
            p_window_eld = &ELD_ANALYSIS_120;
            *mdct_data_e += 2;
        }
        _ => {
            fdk_assert!(false);
            return -1;
        }
    }

    if !eld_window(time_data, l, n, p_window_eld, overlap_add_buffer, mdct_data) {
        eld_window_straight(time_data, l, n, p_window_eld, overlap_add_buffer, mdct_data);
    }
    dct_iv(mdct_data, frame_length, mdct_data_e);

    *prev_window_shape = window_shape;

    0
}

fdk_aac_rs_fdk::avx2_clone! {
/// The windowing of `FDKaacEnc_Transform_Real_Eld()` on slices, so that the
/// compiler vectorizes it; `false` (nothing done) if a buffer is too short.
///
/// Exactness: iteration `i` reads and writes only `oab[i]`, `oab[n/2 + i]` and
/// `oab[3n/2 - 1 - i]` (disjoint index sets for different `i`), reading each before it writes
/// it, so the iterations are independent; each computes the same expressions as
/// [`eld_window_straight`] (`fMult(SGL, SGL)` = `(a * b) << 1` wrapping, `fMultDiv2(SGL, SGL)` =
/// `a * b`, `fMultDiv2(DBL, SGL)` = `floor(a * b / 2^16)`; the same on every variant). On x86
/// the function is compiled a second time with AVX2 (`avx2_clone!`, selected at run time).
fn eld_window(
    time_data: &[IntPcm],
    l: i32,
    n: i32,
    w: &[FixpWtb],
    oab: &mut [FixpDbl],
    mdct_data: &mut [FixpDbl],
) -> bool {
    let (l, n) = (l as usize, n as usize);
    if n % 4 != 0
        || time_data.len() < l + n
        || w.len() < 3 * n
        || oab.len() < 3 * n / 2
        || mdct_data.len() < n
    {
        return false;
    }
    let (q, h) = (n / 4, n / 2);
    let fm = |a: IntPcm, b: FixpWtb| (a as FixpSgl as i32 * b as i32).wrapping_shl(1);
    let fmd2 = |a: IntPcm, b: FixpWtb| a as FixpSgl as i32 * b as i32;
    let fmd2_d = |a: FixpDbl, b: FixpWtb| ((a as i64 * b as i64) >> 16) as i32;
    let (oa, rest) = oab.split_at_mut(h);
    let (ob, rest) = rest.split_at_mut(h);
    let oc = &mut rest[..h];
    let (md_lo, md_hi) = mdct_data[..n].split_at_mut(h);
    // loop 1: i in 0..n/4
    {
        let ta = &time_data[l + h..l + h + q]; // td(l + 3n/4 - 1 - i), reversed
        let tc = &time_data[l + h + q..l + n]; // td(l + 3n/4 + i)
        let wb = &w[q..h]; // w(n/2 - 1 - i), reversed
        let wd = &w[h..h + q]; // w(n/2 + i)
        let we = &w[n + q..n + h]; // w(3n/2 - 1 - i), reversed
        let wf = &w[n + h..n + h + q]; // w(3n/2 + i)
        let wg = &w[2 * n..2 * n + q]; // w(2n + i)
        let wh = &w[2 * n + h..2 * n + h + q]; // w(5n/2 + i)
        let ocr = &mut oc[h - q..h]; // oab[3n/2 - 1 - i], reversed
        let mhr = &mut md_hi[h - q..h]; // mdct[n - 1 - i], reversed
        for i in 0..q {
            let r = q - 1 - i;
            let z0 = fm(ta[r], wb[r]).wrapping_add(fm(tc[i], wd[i]));
            let outval = fmd2(ta[r], we[r])
                .wrapping_add(fmd2(tc[i], wf[i]))
                .wrapping_add(fmd2_d(ob[i], wg[i]) >> 1);
            let old_a = oa[i];
            ob[i] = old_a;
            oa[i] = z0;
            md_lo[i] = old_a.wrapping_add(fmd2_d(ocr[r], wh[i]) >> 1);
            mhr[r] = outval;
            ocr[r] = outval;
        }
    }
    // loop 2: i in n/4..n/2 (index k = i - n/4)
    {
        let ta = &time_data[l + q..l + h]; // td(l + 3n/4 - 1 - i), reversed
        let tl = &time_data[l..l + q]; // td(l - n/4 + i)
        let wb = &w[..q]; // w(n/2 - 1 - i), reversed
        let we = &w[n..n + q]; // w(3n/2 - 1 - i), reversed
        let wd = &w[h + q..n]; // w(n/2 + i)
        let wg = &w[2 * n + q..2 * n + h]; // w(2n + i)
        let wh = &w[2 * n + h + q..3 * n]; // w(5n/2 + i)
        let (oa2, ob2) = (&mut oa[q..h], &mut ob[q..h]);
        let ocr = &mut oc[..q]; // oab[3n/2 - 1 - i], reversed
        let mlr = &mut md_lo[q..h];
        let mhr = &mut md_hi[..q]; // mdct[n - 1 - i], reversed
        for k in 0..q {
            let r = q - 1 - k;
            let z0 = fm(ta[r], wb[r]);
            let outval = fmd2(ta[r], we[r]).wrapping_add(fmd2_d(ob2[k], wg[k]) >> 1);
            let new_b = oa2[k].wrapping_add(fm(tl[k], wd[k]));
            ob2[k] = new_b;
            oa2[k] = z0;
            mlr[k] = new_b.wrapping_add(fmd2_d(ocr[r], wh[k]) >> 1);
            mhr[r] = outval;
            ocr[r] = outval;
        }
    }
    true
}
}

/// The windowing of `FDKaacEnc_Transform_Real_Eld()`: straight port, kept as the reference of
/// [`eld_window`].
#[doc(hidden)]
pub fn eld_window_straight(
    time_data: &[IntPcm],
    l: i32,
    n: i32,
    p_window_eld: &[FixpWtb],
    overlap_add_buffer: &mut [FixpDbl],
    mdct_data: &mut [FixpDbl],
) {
    let td = |i: i32| time_data[i as usize] as FixpSgl;
    let w = |i: i32| p_window_eld[i as usize];
    let oab = overlap_add_buffer;

    for i in 0..n / 4 {
        let z0: FixpDbl = (f_mult(td(l + n * 3 / 4 - 1 - i), w(n / 2 - 1 - i)) << (WTS0 - 1))
            .wrapping_add(f_mult(td(l + n * 3 / 4 + i), w(n / 2 + i)) << (WTS0 - 1));

        let mut outval: FixpDbl =
            f_mult_div2(td(l + n * 3 / 4 - 1 - i), w(n + n / 2 - 1 - i)) >> (-WTS1);
        outval =
            outval.wrapping_add(f_mult_div2(td(l + n * 3 / 4 + i), w(n + n / 2 + i)) >> (-WTS1));
        outval = outval
            .wrapping_add(f_mult_div2(oab[(n / 2 + i) as usize], w(2 * n + i)) >> (-WTS2 - 1));

        oab[(n / 2 + i) as usize] = oab[i as usize];

        oab[i as usize] = z0;
        mdct_data[i as usize] = oab[(n / 2 + i) as usize].wrapping_add(
            f_mult_div2(oab[(n + n / 2 - 1 - i) as usize], w(2 * n + n / 2 + i)) >> (-WTS2 - 1),
        );

        mdct_data[(n - 1 - i) as usize] = outval;
        oab[(n + n / 2 - 1 - i) as usize] = outval;
    }

    for i in n / 4..n / 2 {
        let z0: FixpDbl = f_mult(td(l + n * 3 / 4 - 1 - i), w(n / 2 - 1 - i)) << (WTS0 - 1);

        let mut outval: FixpDbl =
            f_mult_div2(td(l + n * 3 / 4 - 1 - i), w(n + n / 2 - 1 - i)) >> (-WTS1);
        outval = outval
            .wrapping_add(f_mult_div2(oab[(n / 2 + i) as usize], w(2 * n + i)) >> (-WTS2 - 1));

        oab[(n / 2 + i) as usize] =
            oab[i as usize].wrapping_add(f_mult(td(l - n / 4 + i), w(n / 2 + i)) << (WTS0 - 1));

        oab[i as usize] = z0;
        mdct_data[i as usize] = oab[(n / 2 + i) as usize].wrapping_add(
            f_mult_div2(oab[(n + n / 2 - 1 - i) as usize], w(2 * n + n / 2 + i)) >> (-WTS2 - 1),
        );

        mdct_data[(n - 1 - i) as usize] = outval;
        oab[(n + n / 2 - 1 - i) as usize] = outval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slice windowing of the ELD analysis against the straight port, for every frame
    /// length, with random windows, input and overlap buffers (full scale included).
    #[test]
    fn eld_window_equals_straight_port() {
        let mut seed = 0x0e1d_0a5e_1234_5678u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        for k in 0..3000 {
            let n = [512, 480, 256, 240, 128, 120][k % 6];
            let l = n;
            let extreme = k % 4 == 0;
            let mut v16 = |len: usize| -> Vec<i16> {
                (0..len)
                    .map(|_| {
                        if extreme {
                            [i16::MIN, i16::MAX, -1][(next() % 3) as usize]
                        } else {
                            next() as i16
                        }
                    })
                    .collect()
            };
            let time = v16(2 * n);
            let w = v16(3 * n);
            let oab0: Vec<FixpDbl> = (0..3 * n / 2)
                .map(|_| {
                    if extreme {
                        [i32::MIN, i32::MAX][(next() % 2) as usize]
                    } else {
                        next() as i32
                    }
                })
                .collect();
            let (mut oa, mut ob) = (oab0.clone(), oab0.clone());
            let (mut ma, mut mb) = (vec![7; n], vec![7; n]);
            assert!(eld_window(&time, l as i32, n as i32, &w, &mut oa, &mut ma));
            eld_window_straight(&time, l as i32, n as i32, &w, &mut ob, &mut mb);
            assert_eq!((&oa, &ma), (&ob, &mb), "ELD window n {n}");
        }
    }
}
