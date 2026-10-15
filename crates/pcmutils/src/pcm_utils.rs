// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libPCMutils/include/pcm_utils.h, libPCMutils/src/pcm_utils.cpp,
// libPCMutils/src/version.h.

//! Port of `pcm_utils.h`: functions that perform (de)interleaving combined with a format change,
//! and the library version of libPCMutils (`version.h`, shared by the limiter and the downmix).
//!
//! The C++ overloads of `FDK_interleave()` and `FDK_deinterleave()` get suffixes that name the
//! input and output sample types. The slices must hold the samples the C++ code reads and
//! writes; shorter slices panic.
//!
//! Both reference builds compile the same code (no `arm/` or `x86/` variant, no undefined
//! behavior).

use fdk_aac_rs_fdk::common_fix::{fx_dbl2fx_sgl, FixpDbl, FixpSgl};

/// `PCMUTIL_LIB_VL0`
pub const PCMUTIL_LIB_VL0: i32 = 3;
/// `PCMUTIL_LIB_VL1`
pub const PCMUTIL_LIB_VL1: i32 = 1;
/// `PCMUTIL_LIB_VL2`
pub const PCMUTIL_LIB_VL2: i32 = 0;
/// `PCMUTIL_LIB_TITLE`
pub const PCMUTIL_LIB_TITLE: &str = "PCM Utility Lib";
/// `PCMUTIL_LIB_BUILD_DATE`: `__DATE__` in C++, which the port cannot reproduce; the value of
/// `SUPPRESS_BUILD_DATE_INFO` is used (PORTING.md, "Deviations from the reference").
pub const PCMUTIL_LIB_BUILD_DATE: &str = "";
/// `PCMUTIL_LIB_BUILD_TIME`: `__TIME__` in C++, see [`PCMUTIL_LIB_BUILD_DATE`].
pub const PCMUTIL_LIB_BUILD_TIME: &str = "";

/// The sizes of an interleaving whose reads and writes all lie inside the slices (`None`
/// otherwise, where the straight functions run and panic as they did).
#[inline(always)]
fn interleave_fits(
    in_len: usize,
    out_len: usize,
    channels: u32,
    frame_size: u32,
    length: u32,
) -> Option<(usize, usize, usize)> {
    let (ch, fs, len) = (channels as usize, frame_size as usize, length as usize);
    let need_out = ch.checked_mul(len)?;
    let need_in = (ch.checked_sub(1)?).checked_mul(fs)?.checked_add(len)?;
    (need_out <= out_len && need_in <= in_len).then_some((ch, fs, len))
}

/// The interleaving loops on slices, specialized for one and two channels (`ch >= 1`; the slices
/// are large enough, see [`interleave_fits`]). Every output sample is written once, as in the
/// straight loops.
#[inline(always)]
fn interleave_slices<T: Copy, U>(
    p_in: &[T],
    p_out: &mut [U],
    ch: usize,
    fs: usize,
    len: usize,
    conv: impl Fn(T) -> U,
) {
    let out = &mut p_out[..len * ch];
    match ch {
        1 => {
            for (o, &x) in out.iter_mut().zip(&p_in[..len]) {
                *o = conv(x);
            }
        }
        2 => {
            let (a, b) = (&p_in[..len], &p_in[fs..fs + len]);
            for ((o, &x), &y) in out.chunks_exact_mut(2).zip(a).zip(b) {
                o[0] = conv(x);
                o[1] = conv(y);
            }
        }
        3 => interleave_frames::<T, U, 3>(p_in, out, fs, len, conv),
        4 => interleave_frames::<T, U, 4>(p_in, out, fs, len, conv),
        5 => interleave_frames::<T, U, 5>(p_in, out, fs, len, conv),
        6 => interleave_frames::<T, U, 6>(p_in, out, fs, len, conv),
        8 => interleave_frames::<T, U, 8>(p_in, out, fs, len, conv),
        _ => {
            // tiles of 64 samples, so that the strided stores of each channel stay in the cache
            const TILE: usize = 64;
            let mut t = 0;
            while t < len {
                let n = TILE.min(len - t);
                let tile = &mut out[t * ch..(t + n) * ch];
                for c in 0..ch {
                    let src = &p_in[c * fs + t..c * fs + t + n];
                    for (o, &x) in tile[c..].iter_mut().step_by(ch).zip(src) {
                        *o = conv(x);
                    }
                }
                t += n;
            }
        }
    }
}

/// The interleaving of `CH` channels frame by frame (`out` holds `len` frames).
#[inline(always)]
fn interleave_frames<T: Copy, U, const CH: usize>(
    p_in: &[T],
    out: &mut [U],
    fs: usize,
    len: usize,
    conv: impl Fn(T) -> U,
) {
    let srcs: [&[T]; CH] = core::array::from_fn(|c| &p_in[c * fs..c * fs + len]);
    for (s, o) in out.chunks_exact_mut(CH).enumerate() {
        for c in 0..CH {
            o[c] = conv(srcs[c][s]);
        }
    }
}

/// `FDK_interleave(const FIXP_DBL *pIn, LONG *pOut, channels, frameSize, length)`: interleaves
/// `length` samples of each of the `channels` channels of `p_in` (stored `frame_size` samples
/// apart).
///
/// Optimized: loops on slices (specialized for one and two channels) where all accesses are
/// inside the slices, [`fdk_interleave_straight`] otherwise.
pub fn fdk_interleave(
    p_in: &[FixpDbl],
    p_out: &mut [i32],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    if channels == 0 || length == 0 {
        return;
    }
    match interleave_fits(p_in.len(), p_out.len(), channels, frame_size, length) {
        Some((ch, fs, len)) => interleave_slices(p_in, p_out, ch, fs, len, |x| x),
        None => fdk_interleave_straight(p_in, p_out, channels, frame_size, length),
    }
}

/// `FDK_interleave(const FIXP_DBL *pIn, LONG *pOut, ...)` as ported (see [`fdk_interleave`]).
pub fn fdk_interleave_straight(
    p_in: &[FixpDbl],
    p_out: &mut [i32],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    let (channels, frame_size) = (channels as usize, frame_size as usize);
    let mut o = 0;
    for sample in 0..length as usize {
        for ch in 0..channels {
            p_out[o] = p_in[sample + ch * frame_size];
            o += 1;
        }
    }
}

/// `FDK_interleave(const FIXP_DBL *pIn, SHORT *pOut, channels, frameSize, length)`: as
/// [`fdk_interleave`], converted with `FX_DBL2FX_SGL()` (the upper 16 bits).
pub fn fdk_interleave_dbl_to_sgl(
    p_in: &[FixpDbl],
    p_out: &mut [i16],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    let (channels, frame_size) = (channels as usize, frame_size as usize);
    let mut o = 0;
    for sample in 0..length as usize {
        for ch in 0..channels {
            p_out[o] = fx_dbl2fx_sgl(p_in[sample + ch * frame_size]);
            o += 1;
        }
    }
}

/// `FDK_interleave(const FIXP_SGL *pIn, SHORT *pOut, channels, frameSize, length)`: as
/// [`fdk_interleave`] for 16-bit samples.
///
/// Optimized as [`fdk_interleave`]; [`fdk_interleave_sgl_straight`] is the port.
pub fn fdk_interleave_sgl(
    p_in: &[FixpSgl],
    p_out: &mut [i16],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    if channels == 0 || length == 0 {
        return;
    }
    match interleave_fits(p_in.len(), p_out.len(), channels, frame_size, length) {
        Some((ch, fs, len)) => interleave_slices(p_in, p_out, ch, fs, len, |x| x),
        None => fdk_interleave_sgl_straight(p_in, p_out, channels, frame_size, length),
    }
}

/// `FDK_interleave(const FIXP_SGL *pIn, SHORT *pOut, ...)` as ported (see
/// [`fdk_interleave_sgl`]).
pub fn fdk_interleave_sgl_straight(
    p_in: &[FixpSgl],
    p_out: &mut [i16],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    let (channels, frame_size) = (channels as usize, frame_size as usize);
    let mut o = 0;
    for sample in 0..length as usize {
        for ch in 0..channels {
            p_out[o] = p_in[sample + ch * frame_size];
            o += 1;
        }
    }
}

/// `FDK_deinterleave(const LONG *pIn, SHORT *pOut, channels, frameSize, length)`:
/// deinterleaves `frame_size` samples of each of the `channels` channels of `p_in`; channel `ch`
/// is written to `p_out[length * ch..]`. The samples are converted with `(SHORT)(x >> 16)`.
pub fn fdk_deinterleave_long_to_sgl(
    p_in: &[i32],
    p_out: &mut [i16],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    let (channels, length) = (channels as usize, length as usize);
    for ch in 0..channels {
        for sample in 0..frame_size as usize {
            p_out[length * ch + sample] = (p_in[ch + sample * channels] >> 16) as i16;
        }
    }
}

/// `FDK_deinterleave(const LONG *pIn, LONG *pOut, channels, frameSize, length)`: as
/// [`fdk_deinterleave_long_to_sgl`] without conversion.
pub fn fdk_deinterleave(
    p_in: &[i32],
    p_out: &mut [i32],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    let (channels, length) = (channels as usize, length as usize);
    for ch in 0..channels {
        for sample in 0..frame_size as usize {
            p_out[length * ch + sample] = p_in[ch + sample * channels];
        }
    }
}

/// `FDK_deinterleave(const SHORT *pIn, SHORT *pOut, channels, frameSize, length)`: as
/// [`fdk_deinterleave`] for 16-bit samples.
pub fn fdk_deinterleave_sgl(
    p_in: &[i16],
    p_out: &mut [i16],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    let (channels, length) = (channels as usize, length as usize);
    for ch in 0..channels {
        for sample in 0..frame_size as usize {
            p_out[length * ch + sample] = p_in[ch + sample * channels];
        }
    }
}

/// `FDK_deinterleave(const SHORT *pIn, LONG *pOut, channels, frameSize, length)`: as
/// [`fdk_deinterleave`], converted with `(LONG)x << 16`.
pub fn fdk_deinterleave_sgl_to_long(
    p_in: &[i16],
    p_out: &mut [i32],
    channels: u32,
    frame_size: u32,
    length: u32,
) {
    let (channels, length) = (channels as usize, length as usize);
    for ch in 0..channels {
        for sample in 0..frame_size as usize {
            p_out[length * ch + sample] = (p_in[ch + sample * channels] as i32) << 16;
        }
    }
}
