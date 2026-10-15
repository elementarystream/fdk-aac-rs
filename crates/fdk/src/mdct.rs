// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/mdct.h, libFDK/src/mdct.cpp.

//! Port of `mdct.h`: forward MDCT and inverse modulated lapped transform with overlap-add.
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `mdct.cpp` apart from
//! `FDK_ASSERT`; the variants differ only through the primitives and the DCTs.
//!
//! # Memory model
//!
//! In C++, `mdct_t` holds a pointer to an overlap buffer that is owned by the caller
//! (`CAacDecoderStaticChannelInfo::pOverlapBuffer`, `OverlapBufferSize` values) and is passed to
//! `mdct_init()`. The same memory is used in two ways (`union overlap { freq; time; }`): the
//! spectral overlap of the last transform is stored at its end
//! (`overlap.freq + ov_size - tl/2 .. ov_size`), output samples that did not fit into the output
//! buffer are stored at its start (`overlap.time[0 .. ov_offset]`).
//!
//! In this port **[`MdctT`] owns the overlap buffer** ([`MdctT::overlap`]). [`mdct_init`]
//! resizes it to the requested size; like the C++ code it does not clear existing contents
//! (callers that clear `pOverlapBuffer` clear `overlap`). Code that uses `pOverlapBuffer`
//! directly (the low-delay filter bank of `block.cpp`) uses `overlap`.
//!
//! - `prev_wrs` points to a static window slope table: `Option<&'static [FixpWtp]>`.
//! - `pFacZir` points to the FAC zero input response in the ACELP memory of the USAC decoder; it
//!   is set by `CLpd_FAC_Acelp2Mdct()` and consumed (reset to `NULL`) by a later transform. The
//!   port stores a copy of the values ([`MdctT::p_fac_zir`]). This is exact only if the memory
//!   the C pointer refers to is not changed between setting and consuming it; the code that sets
//!   it has to check this.
//! - `pAsymOvlp` (asymmetric overlap) is never set by fdk-aac 2.0.3 (only `mdct_init` writes it,
//!   to `NULL`); the branch that uses it is ported with an owned `Option<Vec<FixpDbl>>`.
//!
//! [`imlt_block`] writes output samples either into `output` or, once `no_out_samples` samples
//! have been produced, into the overlap buffer, and it reads the previous spectral overlap either
//! from the overlap buffer (first window) or from the previous window of `spectrum`. This pointer
//! juggling is reproduced with (buffer, index) pairs.
//!
//! Deviation: indices outside of `output`, the overlap buffer or `spectrum`, for which the
//! reference has no defined result, read 0 and drop the write in the port (PORTING.md,
//! "Deviations from the reference").
//!
//! Where the reference has no defined result, [`mdct_block`] skips the iterations of the left
//! slope loop with indices outside of the buffers. Their values only reach indices that the
//! right slope overwrites, so `mdct_data` is the same as in C (deviation, counted by
//! [`mdct_block_dropped_accesses`]).

use crate::archdef::FixpWtp;
use crate::common_fix::*;
use crate::dct::{dct_iii, dct_iv, dst_iii, dst_iv};
use crate::machine_type::{IntPcm, SAMPLE_BITS};

/// Output additional headroom
pub const MDCT_OUT_HEADROOM: i32 = 2;

pub const PCM_OUT_BITS: i32 = DFRACT_BITS;
/// Must have the same values as DMXH_HEADROOM
pub const PCM_OUT_HEADROOM: i32 = 8;

pub const MDCT_OUTPUT_SCALE: i32 = -MDCT_OUT_HEADROOM + (DFRACT_BITS - PCM_OUT_BITS);
/// Refer to "Output word length" in ISO/IEC 14496-3:2008(E) 23.2.3.6
pub const MDCT_OUTPUT_GAIN: i32 = 16;

/// `IMDCT_SCALE(x, s)`: `SATURATE_RIGHT_SHIFT(x, s + MDCT_OUTPUT_SCALE, PCM_OUT_BITS)`
#[inline(always)]
pub const fn imdct_scale(x: FixpDbl, s: i32) -> FixpDbl {
    saturate_right_shift(x, s + MDCT_OUTPUT_SCALE, PCM_OUT_BITS)
}

/// `IMDCT_SCALE_DBL(x)`
#[inline(always)]
pub const fn imdct_scale_dbl(x: FixpDbl) -> FixpDbl {
    x
}

/// `IMDCT_SCALE_DBL_LSH1(x)`: `SATURATE_LEFT_SHIFT_ALT(x, 1, DFRACT_BITS)`
#[inline(always)]
pub const fn imdct_scale_dbl_lsh1(x: FixpDbl) -> FixpDbl {
    saturate_left_shift_alt(x, 1, DFRACT_BITS)
}

/// `MLT_FLAG_CURR_ALIAS_SYMMETRY`
pub const MLT_FLAG_CURR_ALIAS_SYMMETRY: i32 = 1;

/// `BLOCK_TYPE`
pub type BlockType = i32;
/// normal long block
pub const BLOCK_LONG: BlockType = 0;
/// long start block
pub const BLOCK_START: BlockType = 1;
/// 8 short blocks sequence
pub const BLOCK_SHORT: BlockType = 2;
/// long stop block
pub const BLOCK_STOP: BlockType = 3;

/// `WINDOW_SHAPE`
pub type WindowShape = i32;
pub const SHAPE_SINE: WindowShape = 0;
pub const SHAPE_KBD: WindowShape = 1;
pub const SHAPE_LOL: WindowShape = 2;

/// `mdct_t`: MDCT persistent data. See the [module documentation](self) for the memory model.
#[derive(Clone, Debug, Default)]
pub struct MdctT {
    /// Overlap memory (`overlap.freq` / `overlap.time`), owned by the MDCT.
    pub overlap: Vec<FixpDbl>,
    /// previous right window slope
    pub prev_wrs: Option<&'static [FixpWtp]>,
    /// previous transform length
    pub prev_tl: i32,
    /// previous right window offset
    pub prev_nr: i32,
    /// previous right window slope length
    pub prev_fr: i32,
    /// overlap time data fill level
    pub ov_offset: i32,
    /// Overlap buffer size in words
    pub ov_size: i32,
    /// `prevAliasSymmetry`
    pub prev_alias_symmetry: i32,
    /// `prevPrevAliasSymmetry`
    pub prev_prev_alias_symmetry: i32,
    /// `pFacZir`: the FAC ZIR values the pointer refers to (`None` for `NULL`).
    pub p_fac_zir: Option<Vec<FixpDbl>>,
    /// `pAsymOvlp`: asymmetric overlap (used for stereo LPD transition), `None` for `NULL`.
    pub p_asym_ovlp: Option<Vec<FixpDbl>>,
}

/// `mdct_init(hMdct, overlap, overlapBufferSize)`: initialize as valid MDCT handle.
///
/// The overlap buffer owned by `h_mdct` is resized to `overlap_buffer_size` values; existing
/// contents are kept (new values are zero), like the C++ code, which does not clear the buffer.
/// `prev_wrs` and `ov_offset` are not changed, as in C.
pub fn mdct_init(h_mdct: &mut MdctT, overlap_buffer_size: i32) {
    h_mdct.overlap.resize(overlap_buffer_size.max(0) as usize, 0);
    // FDKmemclear(overlap, overlapBufferSize*sizeof(FIXP_DBL));
    h_mdct.prev_fr = 0;
    h_mdct.prev_nr = 0;
    h_mdct.prev_tl = 0;
    h_mdct.ov_size = overlap_buffer_size;
    h_mdct.prev_alias_symmetry = 0;
    h_mdct.prev_prev_alias_symmetry = 0;
    h_mdct.p_fac_zir = None;
    h_mdct.p_asym_ovlp = None;
}

/// What [`mdct_block`] uses for a missing window slope (`NULL`, from `FDKgetWindowSlope` for a
/// length without a table), for which the reference has no defined result: the port reads zeros
/// (deviation). The longest slope of `mdct_block` has 512 entries (1024 lines).
static NULL_SLOPE: [FixpWtp; 512] = [FixpWtp { re: 0, im: 0 }; 512];

thread_local! {
    static DROPPED: core::cell::Cell<(u64, u64, u64)> = const { core::cell::Cell::new((0, 0, 0)) };
}

/// The accesses of [`mdct_block`] on this thread for which the port deviates from C:
/// `(dropped writes, skipped reads, missing window slopes)`; for tests.
pub fn mdct_block_dropped_accesses() -> (u64, u64, u64) {
    DROPPED.with(|c| c.get())
}

/// Adds to the counters of [`mdct_block_dropped_accesses`] (the carry of the thread pool,
/// [`crate::par`]).
pub fn add_mdct_block_dropped_accesses(n: (u64, u64, u64)) {
    DROPPED.with(|c| {
        let (w, r, s) = c.get();
        c.set((w.wrapping_add(n.0), r.wrapping_add(n.1), s.wrapping_add(n.2)));
    });
}

/// A missing window slope: [`NULL_SLOPE`] (counted).
#[cold]
#[inline(never)]
fn null_slope() -> &'static [FixpWtp] {
    DROPPED.with(|c| {
        let (w, r, n) = c.get();
        c.set((w, r, n + 1));
    });
    &NULL_SLOPE
}

/// The first index of the left slope loop of [`mdct_block`] for `nl < 0`, a left slope `fl`
/// longer than the transform `tl`.
///
/// The iteration `i` writes `mdctData[tl/2 + nl + i]` from `timeData[i + nl]` and
/// `timeData[tl - nl - i - 1]`. The port starts at the first `i` with `i >= -(tl/2 + nl)` and
/// `i >= -(t + nl)` (`t` = the offset of the block in `time_data`), so that every access lies
/// inside the buffers. A skipped iteration that writes inside `mdct_data` writes below
/// `tl/2 - t <= tl/2`, an index that the right slope loops overwrite (they cover `0..tl/2`), so
/// `mdctData[0..tl]` is the same as in C (deviation: the other writes are dropped).
#[cold]
#[inline(never)]
fn left_slope_start(tl: i32, nl: i32, t: isize) -> i32 {
    let writes = (-((tl / 2) + nl)).max(0);
    let reads = (-(t + nl as isize)).max(0) as i32;
    DROPPED.with(|c| {
        let (w, r, n) = c.get();
        c.set((w + writes as u64, r + reads as u64, n));
    });
    writes.max(reads)
}

/// `mdct_block(hMdct, pTimeData, noInSamples, mdctData, nSpec, tl, pRightWindowPart, fr,
/// pMdctData_e)`: forward MDCT of `n_spec` blocks of length `tl`.
///
/// The input block is in a form (A,B,C,D) where A,B,C and D are the respective 1/4th segments of
/// the block. The block is folded in the form (-D-Cr,A-Br), where the 'r' suffix denotes
/// flipping of the sequence, and windowed at the same time; the folded block is passed to the
/// DCT IV. The exponents of the `n_spec` spectra are written to `p_mdct_data_e`.
///
/// Returns the number of input samples processed.
///
/// The same as the straight port [`mdct_block_ref`]: the windowing loops run on slices where all
/// their accesses are inside the buffers (the products of two 16-bit values and the wrapping
/// sums are the same on every variant), which the compiler vectorizes.
pub fn mdct_block(
    h_mdct: &mut MdctT,
    time_data: &[IntPcm],
    no_in_samples: i32,
    mdct_data: &mut [FixpDbl],
    n_spec: i32,
    tl: i32,
    p_right_window_part: Option<&'static [FixpWtp]>,
    fr: i32,
    p_mdct_data_e: &mut [i16],
) -> i32 {
    mdct_block_g::<true>(
        h_mdct,
        time_data,
        no_in_samples,
        mdct_data,
        n_spec,
        tl,
        p_right_window_part,
        fr,
        p_mdct_data_e,
    )
}

/// `mdct_block(...)`: the straight port of [`mdct_block`].
pub fn mdct_block_ref(
    h_mdct: &mut MdctT,
    time_data: &[IntPcm],
    no_in_samples: i32,
    mdct_data: &mut [FixpDbl],
    n_spec: i32,
    tl: i32,
    p_right_window_part: Option<&'static [FixpWtp]>,
    fr: i32,
    p_mdct_data_e: &mut [i16],
) -> i32 {
    mdct_block_g::<false>(
        h_mdct,
        time_data,
        no_in_samples,
        mdct_data,
        n_spec,
        tl,
        p_right_window_part,
        fr,
        p_mdct_data_e,
    )
}

/// The index range `start .. start + n` if it is inside `0 .. len`.
#[inline(always)]
fn inside(start: isize, n: isize, len: usize) -> Option<core::ops::Range<usize>> {
    (start >= 0 && n >= 0 && start + n <= len as isize)
        .then_some(start as usize..(start + n) as usize)
}

/// The windowing of one block of [`mdct_block`] on slices (see there), if all accesses are
/// inside `time_data` (from `t`), the block `md` of `tl` values and the window slopes; `false`
/// (nothing written) otherwise.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn mdct_window_fast(
    time_data: &[IntPcm],
    t: isize,
    md: &mut [FixpDbl],
    tl: i32,
    wls: &[FixpWtp],
    fl: i32,
    nl: i32,
    wrs: &[FixpWtp],
    fr: i32,
    nr: i32,
) -> bool {
    let (tl_, fl2, fr2) = (tl as isize, (fl / 2) as isize, (fr / 2) as isize);
    let (nl_, nr_) = (nl as isize, nr as isize);
    let h = tl_ / 2;
    let len = time_data.len();
    let sh = DFRACT_BITS - SAMPLE_BITS - 1;
    // The source and destination ranges of the four loops of the straight port.
    let (
        Some(d1),
        Some(s1),
        Some(d2),
        Some(s2a),
        Some(s2b),
        Some(d3),
        Some(s3),
        Some(d4),
        Some(s4a),
        Some(s4b),
    ) = (
        inside(h, nl_, md.len()),
        inside(t + tl_ - nl_, nl_, len),
        inside(h + nl_, fl2, md.len()),
        inside(t + nl_, fl2, len),
        inside(t + tl_ - nl_ - fl2, fl2, len),
        inside(h - nr_, nr_, md.len()),
        inside(t + tl_, nr_, len),
        inside(h - nr_ - fr2, fr2, md.len()),
        inside(t + tl_ + nr_, fr2, len),
        inside(t + 2 * tl_ - nr_ - fr2, fr2, len),
    )
    else {
        return false;
    };
    if nl < 0
        || nr < 0
        || fl2 < 0
        || fr2 < 0
        || wls.len() < fl2 as usize
        || wrs.len() < fr2 as usize
    {
        return false;
    }
    let (wls, wrs) = (&wls[..fl2 as usize], &wrs[..fr2 as usize]);
    // md[tl/2 + i] = -timeData[tl - i - 1] << 15, i < nl
    for (d, &x) in md[d1].iter_mut().zip(time_data[s1].iter().rev()) {
        *d = (x as FixpDbl).wrapping_neg() << sh;
    }
    // md[tl/2 + nl + i] = timeData[i + nl] * w.im - timeData[tl - nl - i - 1] * w.re
    for (((d, &a), &b), w) in
        md[d2].iter_mut().zip(&time_data[s2a]).zip(time_data[s2b].iter().rev()).zip(wls)
    {
        *d = f_mult_sub_div2(f_mult_div2(a, w.im), b, w.re);
    }
    // md[tl/2 - 1 - i] = -timeData[tl + i] << 15, i < nr
    for (d, &x) in md[d3].iter_mut().rev().zip(&time_data[s3]) {
        *d = (x as FixpDbl).wrapping_neg() << sh;
    }
    // md[tl/2 - nr - i - 1] = -(timeData[tl + nr + i] * w.re + timeData[2 tl - nr - i - 1] * w.im)
    for (((d, &a), &b), w) in
        md[d4].iter_mut().rev().zip(&time_data[s4a]).zip(time_data[s4b].iter().rev()).zip(wrs)
    {
        *d = f_mult_add_div2(f_mult_div2(a, w.re), b, w.im).wrapping_neg();
    }
    true
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn mdct_block_g<const FAST: bool>(
    h_mdct: &mut MdctT,
    time_data: &[IntPcm],
    no_in_samples: i32,
    mdct_data: &mut [FixpDbl],
    n_spec: i32,
    tl: i32,
    p_right_window_part: Option<&'static [FixpWtp]>,
    fr: i32,
    p_mdct_data_e: &mut [i16],
) -> i32 {
    // tl: transform length, fl: left window slope length, nl: left window slope offset,
    // fr: right window slope length, nr: right window slope offset.
    // See FDK_tools/doc/intern/mdct.tex for more detail.
    let wrs = p_right_window_part;

    // Detect FRprevious / FL mismatches and override parameters accordingly
    if h_mdct.prev_fr == 0 {
        // At start just initialize and pass parameters as they are
        h_mdct.prev_fr = fr;
        h_mdct.prev_wrs = wrs;
        h_mdct.prev_tl = tl;
    }

    // Derive NR
    let nr = (tl - fr) >> 1;

    // Skip input samples if tl is smaller than block size
    let mut t = ((no_in_samples - tl) >> 1) as isize;
    let tlu = tl as usize;

    // windowing
    for n in 0..n_spec.max(0) as usize {
        // MDCT scale:
        // + 1: fMultDiv2() in windowing.
        // + 1: Because of factor 1/2 in Princen-Bradley compliant windowed TDAC.
        let mut mdct_data_e: i32 = 1 + 1;

        let td = |i: i32| time_data[(t + i as isize) as usize];
        let md = &mut mdct_data[n * tlu..];

        // Derive left parameters
        let wls = match h_mdct.prev_wrs {
            Some(w) => w,
            None if h_mdct.prev_fr > 1 => null_slope(),
            None => &[],
        };
        let fl = h_mdct.prev_fr;
        let nl = (tl - fl) >> 1;

        let wrs_fast = match wrs {
            Some(w) => Some(w),
            None if fr > 1 => None,
            None => Some(&[][..]),
        };
        let done = FAST
            && nl >= 0
            && (h_mdct.prev_wrs.is_some() || h_mdct.prev_fr <= 1)
            && wrs_fast.is_some_and(|wrs_s| {
                let nr = (tl - fr) >> 1;
                let mlen = tlu.min(md.len());
                mdct_window_fast(time_data, t, &mut md[..mlen], tl, wls, fl, nl, wrs_s, fr, nr)
            });
        if !done {
            // Here we implement a simplified version of what happens after the this piece of code
            // (see the comments below). We implement the folding of A and B segments to (A-Br) but
            // A is zero, because in this part of the MDCT sequence the window coefficients with
            // which A must be multiplied are zero.
            for i in 0..nl {
                // SAMPLE_BITS (16) != DFRACT_BITS:
                // -(FIXP_DBL)timeData[tl - i - 1] << (DFRACT_BITS - SAMPLE_BITS - 1); /* 0(A)-Br */
                md[((tl / 2) + i) as usize] =
                    (td(tl - i - 1) as FixpDbl).wrapping_neg() << (DFRACT_BITS - SAMPLE_BITS - 1);
            }

            // Implements the folding and windowing of the left part of the sequence, that is
            // segments A and B. The A segment is multiplied by the respective left window
            // coefficient and placed in a temporary variable.
            //
            // tmp0 = fMultDiv2((FIXP_PCM)timeData[i+nl], pLeftWindowPart[i].v.im);
            //
            // After this the B segment taken in reverse order is multiplied by the left window and
            // subtracted from the previously derived temporary variable, so that finally we
            // implement the A-Br operation. This output is written to the right part of the MDCT
            // output : (-D-Cr,A-Br).
            //
            // mdctData[(tl/2)+i+nl] = fMultSubDiv2(tmp0, (FIXP_PCM)timeData[tl-nl-i-1],
            // pLeftWindowPart[i].v.re);//A*window-Br*window
            //
            // The (A-Br) data is written to the output buffer (mdctData) without being flipped.
            //
            // A left slope longer than the transform (nl < 0) starts the loop where both the write
            // and the read are inside the buffers (see `left_slope_start`).
            let i0 = if nl < 0 { left_slope_start(tl, nl, t) } else { 0 };
            for i in i0..fl / 2 {
                let w = wls[i as usize];
                let tmp0: FixpDbl = f_mult_div2(td(i + nl), w.im); // a*window
                md[((tl / 2) + i + nl) as usize] = f_mult_sub_div2(tmp0, td(tl - nl - i - 1), w.re);
                // A*window-Br*window
            }

            // Right window slope offset
            // Here we implement a simplified version of what happens after the this piece of code
            // (see the comments below). We implement the folding of C and D segments to (-D-Cr) but
            // D is zero, because in this part of the MDCT sequence the window coefficients with
            // which D must be multiplied are zero.
            for i in 0..nr {
                // -(FIXP_DBL)timeData[tl + i] << (DFRACT_BITS - SAMPLE_BITS - 1); /* -C flipped */
                md[((tl / 2) - 1 - i) as usize] =
                    (td(tl + i) as FixpDbl).wrapping_neg() << (DFRACT_BITS - SAMPLE_BITS - 1);
            }

            // Implements the folding and windowing of the right part of the sequence, that is,
            // segments C and D. The C segment is multiplied by the respective right window
            // coefficient and placed in a temporary variable.
            //
            // tmp1 = fMultDiv2((FIXP_PCM)timeData[tl+nr+i], pRightWindowPart[i].v.re);
            //
            // After this the D segment taken in reverse order is multiplied by the right window and
            // added from the previously derived temporary variable, so that we get (C+Dr)
            // operation. This output is negated to get (-C-Dr) and written to the left part of the
            // MDCT output while being reversed (flipped) at the same time, so that from (-C-Dr) we
            // get (-D-Cr)=> (-D-Cr,A-Br).
            //
            // mdctData[(tl/2)-nr-i-1] = -fMultAddDiv2(tmp1,
            // (FIXP_PCM)timeData[(tl*2)-nr-i-1], pRightWindowPart[i].v.im);
            let wrs_s = match wrs {
                Some(w) => w,
                None if fr > 1 => null_slope(),
                None => &[],
            };
            for i in 0..fr / 2 {
                let w = wrs_s[i as usize];
                let tmp1: FixpDbl = f_mult_div2(td(tl + nr + i), w.re); // C*window
                                                                        // -(C*window+Dr*window) and flip before placing -> -Cr - D
                md[((tl / 2) - nr - i - 1) as usize] =
                    f_mult_add_div2(tmp1, td((tl * 2) - nr - i - 1), w.im).wrapping_neg();
            }
        }

        // We pass the shortened folded data (-D-Cr,A-Br) to the MDCT function
        if FAST {
            dct_iv(md, tl, &mut mdct_data_e);
        } else {
            crate::dct::dct_iv_ref(md, tl, &mut mdct_data_e);
        }

        p_mdct_data_e[n] = mdct_data_e as i16;

        t += tl as isize;

        h_mdct.prev_wrs = wrs;
        h_mdct.prev_fr = fr;
        h_mdct.prev_tl = tl;
    }

    n_spec * tl
}

/// `imdct_gain(pGain_m, pGain_e, tl)`: add/multiply the 2/N transform gain and the MPEG-4 part 3
/// defined output gain (see [`MDCT_OUTPUT_GAIN`]) to the given mantissa factor and exponent.
pub fn imdct_gain(p_gain_m: &mut FixpDbl, p_gain_e: &mut i32, tl: i32) {
    let mut gain_m = *p_gain_m;
    let mut gain_e = *p_gain_e;

    gain_e += -MDCT_OUTPUT_GAIN - MDCT_OUT_HEADROOM + 1;
    if tl == 0 {
        // Dont regard the 2/N factor from the IDCT. It is compensated for somewhere else.
        *p_gain_e = gain_e;
        return;
    }

    let log2_tl = DFRACT_BITS - 1 - f_normz(tl);
    gain_e += -log2_tl;

    // Detect non-radix 2 transform length and add amplitude compensation factor which cannot be
    // included into the exponent above. The shift amount is negative for tl < 4 (undefined in
    // C, never used); the port takes it modulo 32 like `asr`/`sar`.
    match tl.wrapping_shr((log2_tl - 2) as u32) {
        0x7 => {
            // 10 ms, 1/tl = 1.0/(FDKpow(2.0, -log2_tl) * 0.53333333333333333333)
            if gain_m == 0 {
                gain_m = fl2fxconst_dbl(0.53333333333333333333_f32 as f64);
            } else {
                gain_m = f_mult(gain_m, fl2fxconst_dbl(0.53333333333333333333_f32 as f64));
            }
        }
        0x6 => {
            // 3/4 of radix 2, 1/tl = 1.0/(FDKpow(2.0, -log2_tl) * 2.0/3.0)
            // FL2FXCONST_DBL(2.0 / 3.0f): a double division.
            if gain_m == 0 {
                gain_m = fl2fxconst_dbl(2.0 / 3.0_f32 as f64);
            } else {
                gain_m = f_mult(gain_m, fl2fxconst_dbl(2.0 / 3.0_f32 as f64));
            }
        }
        0x5 => {
            // 0.8 of radix 2 (e.g. tl 160), 1/tl = 1.0/(FDKpow(2.0, -log2_tl) * 0.8/1.5)
            if gain_m == 0 {
                gain_m = fl2fxconst_dbl(0.53333333333333333333_f32 as f64);
            } else {
                gain_m = f_mult(gain_m, fl2fxconst_dbl(0.53333333333333333333_f32 as f64));
            }
        }
        0x4 => {
            // radix 2, nothing to do.
        }
        _ => {
            // unsupported
            fdk_assert!(false, "imdct_gain: unsupported transform length {tl}");
        }
    }

    *p_gain_m = gain_m;
    *p_gain_e = gain_e;
}

/// `imdct_drain(hMdct, output, nrSamplesRoom)`: drain buffered output samples into `output`.
/// Changes the MDCT state. Returns the number of drained samples.
pub fn imdct_drain(h_mdct: &mut MdctT, output: &mut [FixpDbl], nr_samples_room: i32) -> i32 {
    let mut buffered_samples = 0;

    if nr_samples_room > 0 {
        buffered_samples = h_mdct.ov_offset;

        fdk_assert!(buffered_samples <= nr_samples_room);

        if buffered_samples > 0 {
            let n = buffered_samples as usize;
            output[..n].copy_from_slice(&h_mdct.overlap[..n]);
            h_mdct.ov_offset = 0;
        }
    }
    buffered_samples
}

/// `imdct_copy_ov_and_nr(hMdct, pTimeData, nrSamples)`: copy overlap time domain data to
/// `time_data`. Does not change the MDCT state. Returns the number of copied samples (ov + nr).
pub fn imdct_copy_ov_and_nr(h_mdct: &MdctT, time_data: &mut [FixpDbl], nr_samples: i32) -> i32 {
    let mut nr_samples = nr_samples;
    let nt = f_min(h_mdct.ov_offset, nr_samples);
    nr_samples -= nt;
    let nf = f_min(h_mdct.prev_nr, nr_samples);
    let ntu = nt.max(0) as usize;
    time_data[..ntu].copy_from_slice(&h_mdct.overlap[..ntu]);
    let mut t = ntu;

    let mut p_ovl = h_mdct.ov_size as isize - 1;
    if h_mdct.prev_prev_alias_symmetry == 0 {
        for _ in 0..nf {
            let x = h_mdct.overlap[p_ovl as usize].wrapping_neg();
            p_ovl -= 1;
            time_data[t] = imdct_scale_dbl(x);
            t += 1;
        }
    } else {
        for _ in 0..nf {
            let x = h_mdct.overlap[p_ovl as usize];
            p_ovl -= 1;
            time_data[t] = imdct_scale_dbl(x);
            t += 1;
        }
    }

    nt + nf
}

/// `imdct_adapt_parameters(hMdct, pfl, pnl, tl, wls, noOutSamples)`: adapt MDCT parameters for
/// non-matching window slopes.
///
/// `pfl`: left overlap window side length, `pnl`: length of the left n part of the window,
/// `wls`: left side overlap window coefficients, `no_out_samples`: desired number of output
/// samples.
pub fn imdct_adapt_parameters(
    h_mdct: &mut MdctT,
    pfl: &mut i32,
    pnl: &mut i32,
    _tl: i32,
    wls: Option<&'static [FixpWtp]>,
    no_out_samples: i32,
) {
    let mut fl = *pfl;
    let mut nl = *pnl;
    let mut use_current = false;
    let mut use_previous = false;
    if h_mdct.prev_tl == 0 {
        h_mdct.prev_wrs = wls;
        h_mdct.prev_fr = fl;
        h_mdct.prev_nr = (no_out_samples - fl) >> 1;
        h_mdct.prev_tl = no_out_samples;
        h_mdct.ov_offset = 0;
        use_current = true;
    }

    let window_diff = (h_mdct.prev_fr - fl) >> 1;

    // check if the previous window slope can be adjusted to match the current window slope
    if h_mdct.prev_nr + window_diff > 0 {
        use_current = true;
    }
    // check if the current window slope can be adjusted to match the previous window slope
    if nl - window_diff > 0 {
        use_previous = true;
    }

    // if both is possible choose the larger of both window slope lengths
    if use_current && use_previous {
        if fl < h_mdct.prev_fr {
            use_current = false;
        }
    }
    // If the previous transform block is big enough, enlarge previous window overlap, if not,
    // then shrink current window overlap.
    if use_current {
        h_mdct.prev_nr += window_diff;
        h_mdct.prev_fr = fl;
        h_mdct.prev_wrs = wls;
    } else {
        nl -= window_diff;
        fl = h_mdct.prev_fr;
    }

    *pfl = fl;
    *pnl = nl;
}

/// Buffers a pointer of [`imlt_block`] can point into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Buf {
    /// `output`
    Out,
    /// `hMdct->overlap`
    Ovl,
    /// `spectrum`
    Spec,
}

/// A C++ pointer into one of the [`Buf`] buffers.
#[derive(Clone, Copy)]
struct Ptr {
    buf: Buf,
    idx: isize,
}

impl Ptr {
    #[inline(always)]
    fn new(buf: Buf, idx: isize) -> Self {
        Ptr { buf, idx }
    }
}

struct Bufs<'a> {
    out: &'a mut [FixpDbl],
    ovl: &'a mut [FixpDbl],
    spec: &'a mut [FixpDbl],
}

impl Bufs<'_> {
    #[inline(always)]
    fn slice(&self, b: Buf) -> &[FixpDbl] {
        match b {
            Buf::Out => self.out,
            Buf::Ovl => self.ovl,
            Buf::Spec => self.spec,
        }
    }

    /// `*p`; 0 outside of the buffer (see the module documentation).
    #[inline(always)]
    fn get(&self, p: Ptr) -> FixpDbl {
        usize::try_from(p.idx).ok().and_then(|i| self.slice(p.buf).get(i).copied()).unwrap_or(0)
    }

    /// `*p = v`; not written outside of the buffer (see the module documentation).
    #[inline(always)]
    fn set(&mut self, p: Ptr, v: FixpDbl) {
        let s = match p.buf {
            Buf::Out => &mut *self.out,
            Buf::Ovl => &mut *self.ovl,
            Buf::Spec => &mut *self.spec,
        };
        if let Some(d) = usize::try_from(p.idx).ok().and_then(|i| s.get_mut(i)) {
            *d = v;
        }
    }
}

/// The start and length of each buffer, taken once per operation: all raw pointers into a buffer
/// derive from the same borrow, so using two of them does not invalidate the other.
#[derive(Clone, Copy)]
struct RawBufs {
    out: (*mut FixpDbl, usize),
    ovl: (*mut FixpDbl, usize),
    spec: (*mut FixpDbl, usize),
}

impl RawBufs {
    /// The start and length of buffer `b`.
    #[inline(always)]
    fn get(&self, b: Buf) -> (*mut FixpDbl, usize) {
        match b {
            Buf::Out => self.out,
            Buf::Ovl => self.ovl,
            Buf::Spec => self.spec,
        }
    }
}

impl Bufs<'_> {
    /// The raw pointers of the three buffers (see [`RawBufs`]).
    #[inline(always)]
    fn raw(&mut self) -> RawBufs {
        RawBufs {
            out: (self.out.as_mut_ptr(), self.out.len()),
            ovl: (self.ovl.as_mut_ptr(), self.ovl.len()),
            spec: (self.spec.as_mut_ptr(), self.spec.len()),
        }
    }
}

/// The index range of `n` accesses from `p` in the direction `dir` (1 or -1), if all of them are
/// inside a buffer of `len` values.
#[inline(always)]
fn run_range(p: Ptr, dir: isize, n: usize, len: usize) -> Option<core::ops::Range<usize>> {
    let n = n as isize;
    let (lo, hi) = if dir > 0 { (p.idx, p.idx + n) } else { (p.idx - n + 1, p.idx + 1) };
    (lo >= 0 && hi <= len as isize).then_some(lo as usize..hi as usize)
}

/// Whether two runs (buffer, range) share a value.
#[inline(always)]
fn runs_overlap(a: (Buf, &core::ops::Range<usize>), b: (Buf, &core::ops::Range<usize>)) -> bool {
    a.0 == b.0 && a.1.start < b.1.end && b.1.start < a.1.end
}

/// `n` times `*dst = neg ? -*src : *src` with `src += sdir`, `dst += 1` (the pointers are not
/// updated). On slices if all accesses are inside the buffers and the source and destination
/// runs do not overlap, else element by element as in the straight port.
#[inline(always)]
fn copy_run(b: &mut Bufs, src: Ptr, sdir: isize, dst: Ptr, neg: bool, n: usize) {
    if n == 0 {
        return;
    }
    let raw = b.raw();
    let (sp, slen) = raw.get(src.buf);
    let (dp, dlen) = raw.get(dst.buf);
    if let (Some(sr), Some(dr)) = (run_range(src, sdir, n, slen), run_range(dst, 1, n, dlen)) {
        if !runs_overlap((src.buf, &sr), (dst.buf, &dr)) {
            // SAFETY: both ranges are inside their buffers and do not overlap; `b` is not used
            // while the slices live.
            let (s, d) = unsafe {
                (
                    core::slice::from_raw_parts(sp.add(sr.start) as *const FixpDbl, n),
                    core::slice::from_raw_parts_mut(dp.add(dr.start), n),
                )
            };
            match (sdir > 0, neg) {
                (true, false) => d.copy_from_slice(s),
                (true, true) => d.iter_mut().zip(s).for_each(|(d, &s)| *d = s.wrapping_neg()),
                (false, false) => d.iter_mut().zip(s.iter().rev()).for_each(|(d, &s)| *d = s),
                (false, true) => {
                    d.iter_mut().zip(s.iter().rev()).for_each(|(d, &s)| *d = s.wrapping_neg())
                }
            }
            return;
        }
    }
    for i in 0..n as isize {
        let x = b.get(Ptr::new(src.buf, src.idx + sdir * i));
        b.set(Ptr::new(dst.buf, dst.idx + i), if neg { x.wrapping_neg() } else { x });
    }
}

/// The window crossing loop of `imlt_block` over `fl2` values: `(x1, x0) =
/// cplxMultDiv2(*pCurr++, o, w[i])` with `o = -*pOvl--` (`NEG_OVL`) or `*pOvl--`,
/// `*pOut0++ = IMDCT_SCALE_DBL_LSH1(x0)`, `*pOut1-- = IMDCT_SCALE_DBL_LSH1(-x1)` (`NEG_X1`) or
/// `(x1)`. The pointers `[p_curr, p_ovl, p_out0, p_out1]` are advanced.
///
/// On slices (NEON for the `aarch64` variant on `aarch64` targets) if all accesses are inside the
/// buffers and the window, the two output runs do not overlap each other and none overlaps an
/// input run; else element by element as in the straight port.
#[inline(always)]
fn window_crossing_run<const NEG_OVL: bool, const NEG_X1: bool>(
    b: &mut Bufs,
    p: [&mut Ptr; 4],
    p_window: &[FixpWtp],
    fl2: usize,
) {
    let [p_curr, p_ovl, p_out0, p_out1] = p;
    let n = fl2;
    if n > 0 && p_window.len() >= n {
        let raw = b.raw();
        let (cp, clen) = raw.get(p_curr.buf);
        let (op, olen) = raw.get(p_ovl.buf);
        let (o0p, o0len) = raw.get(p_out0.buf);
        let (o1p, o1len) = raw.get(p_out1.buf);
        if let (Some(cr), Some(or), Some(o0r), Some(o1r)) = (
            run_range(*p_curr, 1, n, clen),
            run_range(*p_ovl, -1, n, olen),
            run_range(*p_out0, 1, n, o0len),
            run_range(*p_out1, -1, n, o1len),
        ) {
            let w0 = (p_out0.buf, &o0r);
            let w1 = (p_out1.buf, &o1r);
            let (r0, r1) = ((p_curr.buf, &cr), (p_ovl.buf, &or));
            if !runs_overlap(w0, w1)
                && !runs_overlap(w0, r0)
                && !runs_overlap(w0, r1)
                && !runs_overlap(w1, r0)
                && !runs_overlap(w1, r1)
            {
                // SAFETY: all ranges are inside their buffers, the output ranges do not overlap
                // each other or the input ranges; `b` is not used while the slices live.
                let (curr, ovl, out0, out1) = unsafe {
                    (
                        core::slice::from_raw_parts(cp.add(cr.start) as *const FixpDbl, n),
                        core::slice::from_raw_parts(op.add(or.start) as *const FixpDbl, n),
                        core::slice::from_raw_parts_mut(o0p.add(o0r.start), n),
                        core::slice::from_raw_parts_mut(o1p.add(o1r.start), n),
                    )
                };
                window_crossing_slices::<NEG_OVL, NEG_X1>(curr, ovl, out0, out1, &p_window[..n]);
                p_curr.idx += n as isize;
                p_ovl.idx -= n as isize;
                p_out0.idx += n as isize;
                p_out1.idx -= n as isize;
                return;
            }
        }
    }
    for i in 0..n {
        let w = p_window.get(i).copied().unwrap_or_default();
        let o = if NEG_OVL { b.get(*p_ovl).wrapping_neg() } else { b.get(*p_ovl) };
        let (x1, x0) = cplx_mult_div2(b.get(*p_curr), o, w.re, w.im);
        p_curr.idx += 1;
        p_ovl.idx -= 1;
        b.set(*p_out0, imdct_scale_dbl_lsh1(x0));
        b.set(*p_out1, imdct_scale_dbl_lsh1(if NEG_X1 { x1.wrapping_neg() } else { x1 }));
        p_out0.idx += 1;
        p_out1.idx -= 1;
    }
}

/// [`window_crossing_run`] on slices of `n` values: `ovl` and `out1` are accessed from their
/// end.
#[inline(always)]
fn window_crossing_slices<const NEG_OVL: bool, const NEG_X1: bool>(
    curr: &[FixpDbl],
    ovl: &[FixpDbl],
    out0: &mut [FixpDbl],
    out1: &mut [FixpDbl],
    w: &[FixpWtp],
) {
    let n = curr.len();
    let (ovl, out0, out1, w) = (&ovl[..n], &mut out0[..n], &mut out1[..n], &w[..n]);
    #[allow(unused_mut)]
    let mut i0 = 0;
    #[cfg(target_arch = "aarch64")]
    if ARCH_ARM {
        // SAFETY: all slices hold n values; the outputs do not overlap the inputs.
        i0 = unsafe {
            neon::window_crossing::<NEG_OVL, NEG_X1>(
                curr.as_ptr(),
                ovl.as_ptr(),
                out0.as_mut_ptr(),
                out1.as_mut_ptr(),
                w.as_ptr(),
                n,
            )
        };
    }
    if i0 == 0 {
        window_crossing_vec(curr, ovl, out0, out1, w, NEG_OVL, NEG_X1);
        return;
    }
    window_crossing_loop::<NEG_OVL, NEG_X1>(
        &curr[i0..],
        &ovl[..n - i0],
        &mut out0[i0..],
        &mut out1[..n - i0],
        &w[i0..],
    );
}

crate::avx2_clone! {
/// [`window_crossing_loop`] for the run-time signs (the variants other than `aarch64` and the
/// `aarch64` variant on other targets): AVX2 if available, see [`crate::simd`].
fn window_crossing_vec(
    curr: &[FixpDbl],
    ovl: &[FixpDbl],
    out0: &mut [FixpDbl],
    out1: &mut [FixpDbl],
    w: &[FixpWtp],
    neg_ovl: bool,
    neg_x1: bool,
) {
    match (neg_ovl, neg_x1) {
        (true, true) => window_crossing_loop::<true, true>(curr, ovl, out0, out1, w),
        (true, false) => window_crossing_loop::<true, false>(curr, ovl, out0, out1, w),
        (false, true) => window_crossing_loop::<false, true>(curr, ovl, out0, out1, w),
        (false, false) => window_crossing_loop::<false, false>(curr, ovl, out0, out1, w),
    }
}
}

/// The loop of [`window_crossing_slices`] over `curr.len()` values, written with iterators so
/// that the compiler vectorizes it (`ovl` and `out1` are accessed from their end).
#[inline(always)]
fn window_crossing_loop<const NEG_OVL: bool, const NEG_X1: bool>(
    curr: &[FixpDbl],
    ovl: &[FixpDbl],
    out0: &mut [FixpDbl],
    out1: &mut [FixpDbl],
    w: &[FixpWtp],
) {
    let n = curr.len();
    let (ovl, out0, out1, w) = (&ovl[..n], &mut out0[..n], &mut out1[..n], &w[..n]);
    for (((&c, &o), (d0, d1)), w) in
        curr.iter().zip(ovl.iter().rev()).zip(out0.iter_mut().zip(out1.iter_mut().rev())).zip(w)
    {
        let o = if NEG_OVL { o.wrapping_neg() } else { o };
        let (x1, x0) = cplx_mult_div2(c, o, w.re, w.im);
        *d0 = imdct_scale_dbl_lsh1(x0);
        *d1 = imdct_scale_dbl_lsh1(if NEG_X1 { x1.wrapping_neg() } else { x1 });
    }
}

crate::avx2_clone! {
/// `v[i] = fMult(v[i], gain)` (AVX2 if available, see [`crate::simd`]).
fn mult_gain(v: &mut [FixpDbl], gain: FixpDbl) {
    v.iter_mut().for_each(|x| *x = f_mult(*x, gain));
}
}

/// `scaleValuesSaturate(pSpec, tl, scale)`: NEON on `aarch64` targets (the function is the same
/// on every variant).
#[inline(always)]
fn scale_spectrum(v: &mut [FixpDbl], scalefactor: i32) {
    #[cfg(target_arch = "aarch64")]
    {
        neon::scale_values_saturate(v, scalefactor)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        scale_values_saturate(v, scalefactor)
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) mod neon;
#[cfg(test)]
mod tests;

/// `imlt_block(hMdct, output, spectrum, scalefactor, nSpec, noOutSamples, tl, wls, fl, wrs, fr,
/// gain, flags)`: perform several inverse MLT transforms (frequency domain to time domain).
///
/// This implements the inverse modulated lapped transform, a generalized version of the inverse
/// MDCT. Setting none of the MLT_*_ALIAS_FLAG flags computes the IMDCT, setting all of them
/// computes the IMDST. Other combinations of these flags compute type III transforms used by the
/// RSVD60 multichannel tool for transitions between MDCT/MDST.
///
/// * `output`: output time domain signal.
/// * `spectrum`: the `n_spec` input spectra of length `tl` (modified in place).
/// * `scalefactor`: exponents of the input spectra.
/// * `wls`/`fl`: left side overlap window coefficients and length.
/// * `wrs`/`fr`: right side overlap window coefficients and length of all individual IMLTs.
/// * `gain`: factor to apply to the output samples (if != 0).
/// * `flags`: [`MLT_FLAG_CURR_ALIAS_SYMMETRY`].
///
/// Returns the number of output samples written to `output`.
///
/// The same as the straight port [`imlt_block_ref`]: the element loops run on slices where all
/// their accesses are inside the buffers and the written values do not overlap the read ones
/// (otherwise element by element as in the straight port, see `copy_run` and
/// `window_crossing_run`), the window crossing in NEON on `aarch64` targets for the `aarch64`
/// variant, and the scaling of the spectrum in NEON on `aarch64` targets.
///
/// If we pass the data block (A,B,C,D,E,F) to the FORWARD MDCT it will produce two outputs. The
/// first one will be over the (A,B,C,D) part =>(-D-Cr,A-Br) and the second one will be over the
/// (C,D,E,F) part => (-F-Er,C-Dr), since there is a overlap between consequtive passes of the
/// algorithm. This overlap is over the (C,D) segments. The two outputs will be given
/// sequentially to the DCT IV algorithm. At the INVERSE MDCT side we get two consecutive
/// outputs from the IDCT IV algorithm, namely the same blocks: (-D-Cr,A-Br) and (-F-Er,C-Dr).
/// The first of them lands in the Overlap buffer and the second is in the working one, which,
/// one algorithm pass later will substitute the one residing in the overlap register. The IMDCT
/// algorithm has to produce the C and D segments from the two buffers. In order to do this we
/// take the left part of the overlap buffer(-D-Cr,A-Br), namely (-D-Cr) and add it
/// appropriately to the right part of the working buffer (-F-Er,C-Dr), namely (C-Dr), so that
/// we get first the C segment and later the D segment. We do this in the following way: From the
/// right part of the working buffer(C-Dr) we subtract the flipped left part of the overlap
/// buffer(-D-Cr):
///
/// Result = (C-Dr) - flipped(-D-Cr) = C -Dr + Dr + C = 2C
///
/// We divide by two and get the C segment. What we did is adding the right part of the first
/// frame to the left part of the second one. While applying these operation we multiply the
/// respective segments with the appropriate window functions.
///
/// In order to get the D segment we do the following: From the negated second part of the
/// working buffer(C-Dr) we subtract the flipped first part of the overlap buffer (-D-Cr):
///
/// Result= - (C -Dr) - flipped(-D-Cr)= -C +Dr +Dr +C = 2Dr.
///
/// After dividing by two and flipping we get the D segment.What we did is adding the right part
/// of the first frame to the left part of the second one. While applying these operation we
/// multiply the respective segments with the appropriate window functions.
///
/// Once we have obtained the C and D segments the overlap buffer is emptied and the current
/// buffer is sent in it, so that the E and F segments are available for decoding in the next
/// algorithm pass.
pub fn imlt_block(
    h_mdct: &mut MdctT,
    output: &mut [FixpDbl],
    spectrum: &mut [FixpDbl],
    scalefactor: &[i16],
    n_spec: i32,
    no_out_samples: i32,
    tl: i32,
    wls: Option<&'static [FixpWtp]>,
    fl: i32,
    wrs: Option<&'static [FixpWtp]>,
    fr: i32,
    gain: FixpDbl,
    flags: i32,
) -> i32 {
    let mut fl = fl;
    let mut gain = gain;
    let mut nr_samples = 0;
    let mut transform_gain_e = 0;
    let curr_alias_symmetry = flags & MLT_FLAG_CURR_ALIAS_SYMMETRY;

    // Derive NR and NL
    let nr = (tl - fr) >> 1;
    let mut nl = (tl - fl) >> 1;

    // Include 2/N IMDCT gain into gain factor and exponent.
    imdct_gain(&mut gain, &mut transform_gain_e, tl);

    // Detect FRprevious / FL mismatches and override parameters accordingly
    if h_mdct.prev_fr != fl {
        imdct_adapt_parameters(h_mdct, &mut fl, &mut nl, tl, wls, no_out_samples);
    }

    // The overlap buffer is taken out of the handle while the other fields are updated.
    let mut ovl_buf = core::mem::take(&mut h_mdct.overlap);
    let ov_size = h_mdct.ov_size as isize;
    let mut b = Bufs { out: output, ovl: &mut ovl_buf, spec: spectrum };

    let mut p_ovl = Ptr::new(Buf::Ovl, ov_size - 1);
    let mut p_out0 = Ptr::new(Buf::Out, 0);

    if no_out_samples > nr_samples {
        // Purge buffered output.
        let n = h_mdct.ov_offset.max(0) as usize;
        copy_run(&mut b, Ptr::new(Buf::Ovl, 0), 1, p_out0, false, n);
        p_out0.idx += n as isize;
        nr_samples = h_mdct.ov_offset;
        h_mdct.ov_offset = 0;
    }

    let tlu = tl as isize;
    for w in 0..n_spec {
        // Detect FRprevious / FL mismatches and override parameters accordingly
        if h_mdct.prev_fr != fl {
            imdct_adapt_parameters(h_mdct, &mut fl, &mut nl, tl, wls, no_out_samples);
        }

        let mut spec_shift_scale = transform_gain_e;

        // Setup window pointers
        let p_window = h_mdct.prev_wrs.unwrap_or(&[]);

        // Current spectrum
        let spec_off = w as isize * tlu;
        {
            let p_spec = &mut b.spec[spec_off as usize..];

            // DCT IV of current spectrum.
            if curr_alias_symmetry == 0 {
                if h_mdct.prev_alias_symmetry == 0 {
                    dct_iv(p_spec, tl, &mut spec_shift_scale);
                } else {
                    let mut tmp = [0 as FixpDbl; 1024];
                    dct_iii(p_spec, &mut tmp, tl, &mut spec_shift_scale);
                }
            } else {
                if h_mdct.prev_alias_symmetry == 0 {
                    let mut tmp = [0 as FixpDbl; 1024];
                    dst_iii(p_spec, &mut tmp, tl, &mut spec_shift_scale);
                } else {
                    dst_iv(p_spec, tl, &mut spec_shift_scale);
                }
            }

            // Optional scaling of time domain - no yet windowed - of current spectrum
            // and de-scale current spectrum signal (time domain, no yet windowed)
            if gain != 0 {
                mult_gain(&mut p_spec[..tl as usize], gain);
            }

            {
                let loc_scale =
                    fixmin_i(scalefactor[w as usize] as i32 + spec_shift_scale, DFRACT_BITS - 1);
                scale_spectrum(&mut p_spec[..tl as usize], loc_scale);
            }
        }

        if no_out_samples <= nr_samples {
            // Divert output first half to overlap buffer if we already got enough output
            // samples.
            p_out0 = Ptr::new(Buf::Ovl, h_mdct.ov_offset as isize);
            h_mdct.ov_offset += h_mdct.prev_nr + fl / 2;
        } else {
            // Account output samples
            nr_samples += h_mdct.prev_nr + fl / 2;
        }

        // NR output samples 0 .. NR. -overlap[TL/2..TL/2-NR]
        if h_mdct.p_fac_zir.is_some() && (h_mdct.prev_nr == fl / 2) {
            // In the case of ACELP -> TCX20 -> FD short add FAC ZIR on nr signal part
            let zir = h_mdct.p_fac_zir.take().unwrap_or_default();
            for i in 0..h_mdct.prev_nr.max(0) as usize {
                let x = b.get(p_ovl).wrapping_neg();
                p_ovl.idx -= 1;
                b.set(p_out0, f_add_saturate(x, imdct_scale_dbl(zir.get(i).copied().unwrap_or(0))));
                p_out0.idx += 1;
            }
            // hMdct->pFacZir = NULL (taken above)
        } else {
            // Here we implement a simplified version of what happens after the this piece of
            // code (see the comments below). We implement the folding of C and D segments from
            // (-D-Cr) but D is zero, because in this part of the MDCT sequence the window
            // coefficients with which D must be multiplied are zero. "pOut0" writes
            // sequentially the C block from left to right.
            // IMDCT_SCALE_DBL is the identity.
            let n = h_mdct.prev_nr.max(0) as usize;
            copy_run(&mut b, p_ovl, -1, p_out0, h_mdct.prev_prev_alias_symmetry == 0, n);
            p_ovl.idx -= n as isize;
            p_out0.idx += n as isize;
        }

        let mut p_out1;
        if no_out_samples <= nr_samples {
            // Divert output second half to overlap buffer if we already got enough output
            // samples.
            p_out1 = Ptr::new(Buf::Ovl, (h_mdct.ov_offset + fl / 2 - 1) as isize);
            h_mdct.ov_offset += fl / 2 + nl;
        } else {
            p_out1 = Ptr::new(p_out0.buf, p_out0.idx + (fl - 1) as isize);
            nr_samples += fl / 2 + nl;
        }

        // output samples before window crossing point NR .. TL/2.
        //   -overlap[TL/2-NR..TL/2-NR-FL/2] + current[NR..TL/2]
        // output samples after window crossing point TL/2 .. TL/2+FL/2.
        //   -overlap[0..FL/2] - current[TL/2..FL/2]
        let mut p_curr = Ptr::new(Buf::Spec, spec_off + tlu - (fl / 2) as isize);
        let fl2 = (fl / 2).max(0) as usize;
        let win = |i: usize| p_window.get(i).copied().unwrap_or_default();

        if h_mdct.prev_prev_alias_symmetry == 0 {
            if h_mdct.prev_alias_symmetry == 0 {
                if h_mdct.p_asym_ovlp.is_none() {
                    window_crossing_run::<true, true>(
                        &mut b,
                        [&mut p_curr, &mut p_ovl, &mut p_out0, &mut p_out1],
                        p_window,
                        fl2,
                    );
                } else {
                    let asym = h_mdct.p_asym_ovlp.take().unwrap_or_default();
                    let mut p_asym_ovl = (fl / 2 - 1) as isize;
                    for i in 0..fl2 {
                        let w = win(i);
                        let curr = b.get(p_curr);
                        let asym_v = usize::try_from(p_asym_ovl)
                            .ok()
                            .and_then(|k| asym.get(k).copied())
                            .unwrap_or(0);
                        let x1 = f_mult_div2(curr, w.re)
                            .wrapping_neg()
                            .wrapping_add(f_mult_div2(asym_v, w.im));
                        let x0 =
                            f_mult_div2(curr, w.im).wrapping_sub(f_mult_div2(b.get(p_ovl), w.re));
                        p_curr.idx += 1;
                        p_ovl.idx -= 1;
                        p_asym_ovl -= 1;
                        b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                        p_out0.idx += 1;
                        b.set(p_out1, imdct_scale_dbl_lsh1(x1));
                        p_out1.idx -= 1;
                    }
                    // hMdct->pAsymOvlp = NULL (taken above)
                }
            } else {
                // prevAliasingSymmetry == 1
                window_crossing_run::<true, false>(
                    &mut b,
                    [&mut p_curr, &mut p_ovl, &mut p_out0, &mut p_out1],
                    p_window,
                    fl2,
                );
            }
        } else {
            // prevPrevAliasingSymmetry == 1
            let ptrs = [&mut p_curr, &mut p_ovl, &mut p_out0, &mut p_out1];
            if h_mdct.prev_alias_symmetry == 0 {
                window_crossing_run::<false, true>(&mut b, ptrs, p_window, fl2);
            } else {
                // prevAliasingSymmetry == 1
                window_crossing_run::<false, false>(&mut b, ptrs, p_window, fl2);
            }
        }

        if let Some(zir) = h_mdct.p_fac_zir.take() {
            // add FAC ZIR of previous ACELP -> mdct transition
            let p_out = Ptr::new(p_out0.buf, p_out0.idx - (fl / 2) as isize);
            fdk_assert!(fl / 2 <= 128);
            for i in 0..fl2 {
                let q = Ptr::new(p_out.buf, p_out.idx + i as isize);
                let v = f_add_saturate(b.get(q), imdct_scale_dbl(zir.get(i).copied().unwrap_or(0)));
                b.set(q, v);
            }
            // hMdct->pFacZir = NULL (taken above)
        }
        p_out0.idx += ((fl / 2) + nl) as isize;

        // NL output samples TL/2+FL/2..TL. - current[FL/2..0]
        p_out1.idx += ((fl / 2) + 1) as isize;
        p_curr = Ptr::new(Buf::Spec, spec_off + tlu - (fl / 2) as isize - 1);
        // Here we implement a simplified version of what happens above the this piece of code
        // (see the comments above). We implement the folding of C and D segments from (C-Dr)
        // but C is zero, because in this part of the MDCT sequence the window coefficients with
        // which C must be multiplied are zero. "pOut1" writes sequentially the D block from left
        // to right.
        // IMDCT_SCALE_DBL is the identity.
        let n = nl.max(0) as usize;
        copy_run(&mut b, p_curr, -1, p_out1, h_mdct.prev_alias_symmetry == 0, n);

        // Set overlap source pointer for next window pOvl = pSpec + tl/2 - 1;
        p_ovl = Ptr::new(Buf::Spec, spec_off + (tl / 2) as isize - 1);

        // Previous window values.
        h_mdct.prev_nr = nr;
        h_mdct.prev_fr = fr;
        h_mdct.prev_tl = tl;
        h_mdct.prev_wrs = wrs;

        // Previous aliasing symmetry
        h_mdct.prev_prev_alias_symmetry = h_mdct.prev_alias_symmetry;
        h_mdct.prev_alias_symmetry = curr_alias_symmetry;
    }

    // Save overlap
    let half = (tl / 2).max(0) as isize;
    let dst = ov_size - half;
    let src = (n_spec as isize - 1) * tlu;
    copy_run(&mut b, Ptr::new(Buf::Spec, src), 1, Ptr::new(Buf::Ovl, dst), false, half as usize);

    h_mdct.overlap = ovl_buf;

    nr_samples
}

/// `imlt_block(hMdct, output, spectrum, scalefactor, nSpec, noOutSamples, tl, wls, fl, wrs, fr,
/// gain, flags)`: the straight port of [`imlt_block`], with the same arguments and results.
pub fn imlt_block_ref(
    h_mdct: &mut MdctT,
    output: &mut [FixpDbl],
    spectrum: &mut [FixpDbl],
    scalefactor: &[i16],
    n_spec: i32,
    no_out_samples: i32,
    tl: i32,
    wls: Option<&'static [FixpWtp]>,
    fl: i32,
    wrs: Option<&'static [FixpWtp]>,
    fr: i32,
    gain: FixpDbl,
    flags: i32,
) -> i32 {
    let mut fl = fl;
    let mut gain = gain;
    let mut nr_samples = 0;
    let mut transform_gain_e = 0;
    let curr_alias_symmetry = flags & MLT_FLAG_CURR_ALIAS_SYMMETRY;

    // Derive NR and NL
    let nr = (tl - fr) >> 1;
    let mut nl = (tl - fl) >> 1;

    // Include 2/N IMDCT gain into gain factor and exponent.
    imdct_gain(&mut gain, &mut transform_gain_e, tl);

    // Detect FRprevious / FL mismatches and override parameters accordingly
    if h_mdct.prev_fr != fl {
        imdct_adapt_parameters(h_mdct, &mut fl, &mut nl, tl, wls, no_out_samples);
    }

    // The overlap buffer is taken out of the handle while the other fields are updated.
    let mut ovl_buf = core::mem::take(&mut h_mdct.overlap);
    let ov_size = h_mdct.ov_size as isize;
    let mut b = Bufs { out: output, ovl: &mut ovl_buf, spec: spectrum };

    let mut p_ovl = Ptr::new(Buf::Ovl, ov_size - 1);
    let mut p_out0 = Ptr::new(Buf::Out, 0);

    if no_out_samples > nr_samples {
        // Purge buffered output.
        for i in 0..h_mdct.ov_offset.max(0) as isize {
            let v = b.get(Ptr::new(Buf::Ovl, i));
            b.set(p_out0, v);
            p_out0.idx += 1;
        }
        nr_samples = h_mdct.ov_offset;
        h_mdct.ov_offset = 0;
    }

    let tlu = tl as isize;
    for w in 0..n_spec {
        // Detect FRprevious / FL mismatches and override parameters accordingly
        if h_mdct.prev_fr != fl {
            imdct_adapt_parameters(h_mdct, &mut fl, &mut nl, tl, wls, no_out_samples);
        }

        let mut spec_shift_scale = transform_gain_e;

        // Setup window pointers
        let p_window = h_mdct.prev_wrs.unwrap_or(&[]);

        // Current spectrum
        let spec_off = w as isize * tlu;
        {
            let p_spec = &mut b.spec[spec_off as usize..];

            // DCT IV of current spectrum.
            if curr_alias_symmetry == 0 {
                if h_mdct.prev_alias_symmetry == 0 {
                    crate::dct::dct_iv_ref(p_spec, tl, &mut spec_shift_scale);
                } else {
                    let mut tmp = [0 as FixpDbl; 1024];
                    crate::dct::dct_iii_ref(p_spec, &mut tmp, tl, &mut spec_shift_scale);
                }
            } else {
                if h_mdct.prev_alias_symmetry == 0 {
                    let mut tmp = [0 as FixpDbl; 1024];
                    crate::dct::dst_iii_ref(p_spec, &mut tmp, tl, &mut spec_shift_scale);
                } else {
                    crate::dct::dst_iv_ref(p_spec, tl, &mut spec_shift_scale);
                }
            }

            // Optional scaling of time domain - no yet windowed - of current spectrum
            // and de-scale current spectrum signal (time domain, no yet windowed)
            if gain != 0 {
                mult_gain(&mut p_spec[..tl as usize], gain);
            }

            {
                let loc_scale =
                    fixmin_i(scalefactor[w as usize] as i32 + spec_shift_scale, DFRACT_BITS - 1);
                scale_values_saturate(&mut p_spec[..tl as usize], loc_scale);
            }
        }

        if no_out_samples <= nr_samples {
            // Divert output first half to overlap buffer if we already got enough output
            // samples.
            p_out0 = Ptr::new(Buf::Ovl, h_mdct.ov_offset as isize);
            h_mdct.ov_offset += h_mdct.prev_nr + fl / 2;
        } else {
            // Account output samples
            nr_samples += h_mdct.prev_nr + fl / 2;
        }

        // NR output samples 0 .. NR. -overlap[TL/2..TL/2-NR]
        if h_mdct.p_fac_zir.is_some() && (h_mdct.prev_nr == fl / 2) {
            // In the case of ACELP -> TCX20 -> FD short add FAC ZIR on nr signal part
            let zir = h_mdct.p_fac_zir.take().unwrap_or_default();
            for i in 0..h_mdct.prev_nr.max(0) as usize {
                let x = b.get(p_ovl).wrapping_neg();
                p_ovl.idx -= 1;
                b.set(p_out0, f_add_saturate(x, imdct_scale_dbl(zir.get(i).copied().unwrap_or(0))));
                p_out0.idx += 1;
            }
            // hMdct->pFacZir = NULL (taken above)
        } else {
            // Here we implement a simplified version of what happens after the this piece of
            // code (see the comments below). We implement the folding of C and D segments from
            // (-D-Cr) but D is zero, because in this part of the MDCT sequence the window
            // coefficients with which D must be multiplied are zero. "pOut0" writes
            // sequentially the C block from left to right.
            if h_mdct.prev_prev_alias_symmetry == 0 {
                for _ in 0..h_mdct.prev_nr {
                    let x = b.get(p_ovl).wrapping_neg();
                    p_ovl.idx -= 1;
                    b.set(p_out0, imdct_scale_dbl(x));
                    p_out0.idx += 1;
                }
            } else {
                for _ in 0..h_mdct.prev_nr {
                    let x = b.get(p_ovl);
                    p_ovl.idx -= 1;
                    b.set(p_out0, imdct_scale_dbl(x));
                    p_out0.idx += 1;
                }
            }
        }

        let mut p_out1;
        if no_out_samples <= nr_samples {
            // Divert output second half to overlap buffer if we already got enough output
            // samples.
            p_out1 = Ptr::new(Buf::Ovl, (h_mdct.ov_offset + fl / 2 - 1) as isize);
            h_mdct.ov_offset += fl / 2 + nl;
        } else {
            p_out1 = Ptr::new(p_out0.buf, p_out0.idx + (fl - 1) as isize);
            nr_samples += fl / 2 + nl;
        }

        // output samples before window crossing point NR .. TL/2.
        //   -overlap[TL/2-NR..TL/2-NR-FL/2] + current[NR..TL/2]
        // output samples after window crossing point TL/2 .. TL/2+FL/2.
        //   -overlap[0..FL/2] - current[TL/2..FL/2]
        let mut p_curr = Ptr::new(Buf::Spec, spec_off + tlu - (fl / 2) as isize);
        let fl2 = (fl / 2).max(0) as usize;
        let win = |i: usize| p_window.get(i).copied().unwrap_or_default();

        if h_mdct.prev_prev_alias_symmetry == 0 {
            if h_mdct.prev_alias_symmetry == 0 {
                if h_mdct.p_asym_ovlp.is_none() {
                    for i in 0..fl2 {
                        let w = win(i);
                        let (x1, x0) =
                            cplx_mult_div2(b.get(p_curr), b.get(p_ovl).wrapping_neg(), w.re, w.im);
                        p_curr.idx += 1;
                        p_ovl.idx -= 1;
                        b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                        b.set(p_out1, imdct_scale_dbl_lsh1(x1.wrapping_neg()));
                        p_out0.idx += 1;
                        p_out1.idx -= 1;
                    }
                } else {
                    let asym = h_mdct.p_asym_ovlp.take().unwrap_or_default();
                    let mut p_asym_ovl = (fl / 2 - 1) as isize;
                    for i in 0..fl2 {
                        let w = win(i);
                        let curr = b.get(p_curr);
                        let asym_v = usize::try_from(p_asym_ovl)
                            .ok()
                            .and_then(|k| asym.get(k).copied())
                            .unwrap_or(0);
                        let x1 = f_mult_div2(curr, w.re)
                            .wrapping_neg()
                            .wrapping_add(f_mult_div2(asym_v, w.im));
                        let x0 =
                            f_mult_div2(curr, w.im).wrapping_sub(f_mult_div2(b.get(p_ovl), w.re));
                        p_curr.idx += 1;
                        p_ovl.idx -= 1;
                        p_asym_ovl -= 1;
                        b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                        p_out0.idx += 1;
                        b.set(p_out1, imdct_scale_dbl_lsh1(x1));
                        p_out1.idx -= 1;
                    }
                    // hMdct->pAsymOvlp = NULL (taken above)
                }
            } else {
                // prevAliasingSymmetry == 1
                for i in 0..fl2 {
                    let w = win(i);
                    let (x1, x0) =
                        cplx_mult_div2(b.get(p_curr), b.get(p_ovl).wrapping_neg(), w.re, w.im);
                    p_curr.idx += 1;
                    p_ovl.idx -= 1;
                    b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                    b.set(p_out1, imdct_scale_dbl_lsh1(x1));
                    p_out0.idx += 1;
                    p_out1.idx -= 1;
                }
            }
        } else {
            // prevPrevAliasingSymmetry == 1
            if h_mdct.prev_alias_symmetry == 0 {
                for i in 0..fl2 {
                    let w = win(i);
                    let (x1, x0) = cplx_mult_div2(b.get(p_curr), b.get(p_ovl), w.re, w.im);
                    p_curr.idx += 1;
                    p_ovl.idx -= 1;
                    b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                    b.set(p_out1, imdct_scale_dbl_lsh1(x1.wrapping_neg()));
                    p_out0.idx += 1;
                    p_out1.idx -= 1;
                }
            } else {
                // prevAliasingSymmetry == 1
                for i in 0..fl2 {
                    let w = win(i);
                    let (x1, x0) = cplx_mult_div2(b.get(p_curr), b.get(p_ovl), w.re, w.im);
                    p_curr.idx += 1;
                    p_ovl.idx -= 1;
                    b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                    b.set(p_out1, imdct_scale_dbl_lsh1(x1));
                    p_out0.idx += 1;
                    p_out1.idx -= 1;
                }
            }
        }

        if let Some(zir) = h_mdct.p_fac_zir.take() {
            // add FAC ZIR of previous ACELP -> mdct transition
            let p_out = Ptr::new(p_out0.buf, p_out0.idx - (fl / 2) as isize);
            fdk_assert!(fl / 2 <= 128);
            for i in 0..fl2 {
                let q = Ptr::new(p_out.buf, p_out.idx + i as isize);
                let v = f_add_saturate(b.get(q), imdct_scale_dbl(zir.get(i).copied().unwrap_or(0)));
                b.set(q, v);
            }
            // hMdct->pFacZir = NULL (taken above)
        }
        p_out0.idx += ((fl / 2) + nl) as isize;

        // NL output samples TL/2+FL/2..TL. - current[FL/2..0]
        p_out1.idx += ((fl / 2) + 1) as isize;
        p_curr = Ptr::new(Buf::Spec, spec_off + tlu - (fl / 2) as isize - 1);
        // Here we implement a simplified version of what happens above the this piece of code
        // (see the comments above). We implement the folding of C and D segments from (C-Dr)
        // but C is zero, because in this part of the MDCT sequence the window coefficients with
        // which C must be multiplied are zero. "pOut1" writes sequentially the D block from left
        // to right.
        if h_mdct.prev_alias_symmetry == 0 {
            for _ in 0..nl {
                let x = b.get(p_curr).wrapping_neg();
                p_curr.idx -= 1;
                b.set(p_out1, imdct_scale_dbl(x));
                p_out1.idx += 1;
            }
        } else {
            for _ in 0..nl {
                let x = b.get(p_curr);
                p_curr.idx -= 1;
                b.set(p_out1, imdct_scale_dbl(x));
                p_out1.idx += 1;
            }
        }

        // Set overlap source pointer for next window pOvl = pSpec + tl/2 - 1;
        p_ovl = Ptr::new(Buf::Spec, spec_off + (tl / 2) as isize - 1);

        // Previous window values.
        h_mdct.prev_nr = nr;
        h_mdct.prev_fr = fr;
        h_mdct.prev_tl = tl;
        h_mdct.prev_wrs = wrs;

        // Previous aliasing symmetry
        h_mdct.prev_prev_alias_symmetry = h_mdct.prev_alias_symmetry;
        h_mdct.prev_alias_symmetry = curr_alias_symmetry;
    }

    // Save overlap
    let half = (tl / 2).max(0) as isize;
    let dst = ov_size - half;
    let src = (n_spec as isize - 1) * tlu;
    for i in 0..half {
        let v = b.get(Ptr::new(Buf::Spec, src + i));
        b.set(Ptr::new(Buf::Ovl, dst + i), v);
    }

    h_mdct.overlap = ovl_buf;

    nr_samples
}
