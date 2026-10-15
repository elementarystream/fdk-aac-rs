// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/lpp_tran.cpp (optimized; no counterpart in the reference).

//! The band loop of `lppTransposer()` in low power mode, eight source bands at a time.
//!
//! The straight loop handles one source band (`loBand`) after the other: it copies the band's
//! values out of the filter states and the QMF slots, scales them, computes their
//! autocorrelation and the LPC coefficients, and filters them into the target bands of every
//! patch that contains the band. The QMF data are stored slot by slot with the bands
//! consecutive, so eight consecutive bands of a slot are one vector: here the values of eight
//! bands are kept time-major (`lb[t][lane]`), the scaling and the autocorrelation run in the
//! lanes, the LPC coefficients and the aliasing detection run band by band in the original
//! order, and the filtering of a patch writes the eight target bands of a slot at once.
//!
//! Exactness:
//! - Every lane computes exactly the operations of the straight loop for its band (the same
//!   shifts, wrapping sums in the same order, the same `fMultDiv2` products); the lanes past the
//!   last band compute on zeros and are not used.
//! - The LPC coefficients, the aliasing detection (`k1`, `degreeAlias`) and the bandwidth index
//!   of every patch are computed band by band in band order, by the code of the straight loop.
//! - The straight loop interleaves reads and writes of the QMF buffer: band `b` reads the slots
//!   of band `b` and writes the target bands `b + targetBandOffs` of its patches. The order of
//!   the writes of different patches and bands, and of the writes and later reads, only matters
//!   if a target band is a source band or two patches write the same band; [`run`] checks that
//!   every target band of the loop lies at or above `stop` (the end of the source bands) and
//!   below 64, and that the target ranges of the patches are disjoint, and otherwise leaves the
//!   loop to the straight code (as it does for any configuration whose buffers it cannot read or
//!   write in whole groups).

use super::*;
use fdk_aac_rs_fdk::clz::fixnormz_d;

/// The bands processed at a time.
const L: usize = 8;
/// The length of `lowBandReal[]`.
const N_LB: usize = MAX_COLS + MAX_OV_COLS + LPC_ORDER;

type Lanes = [i32; L];

/// `fMultDiv2(FIXP_DBL a, FIXP_DBL b)` (every variant)
#[inline(always)]
fn md2(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> 32) as i32
}

/// `getScalefactor()` of `lb[t0..t1]` in every lane.
#[inline(always)]
fn scalefactors(lb: &[Lanes], t0: usize, t1: usize) -> Lanes {
    let mut m = [0i32; L];
    for row in &lb[t0..t1] {
        for (m, &x) in m.iter_mut().zip(row) {
            *m |= x ^ (x >> (DFRACT_BITS - 1));
        }
    }
    m.map(|m| fix_max(0, fixnormz_d(m) - 1))
}

/// The left and right shift amounts of `scaleValues(.., s)` per lane: left by `min(s, 31)` for
/// `s > 0`, right by `min(-s, 31)` for `s < 0`, nothing for 0.
#[inline(always)]
fn scale_amounts(s: &Lanes) -> (Lanes, Lanes) {
    let left = s.map(|s| if s > 0 { fix_min(s, DFRACT_BITS - 1) } else { 0 });
    let right = s.map(|s| if s < 0 { (-(s as i64)).min(DFRACT_BITS as i64 - 1) as i32 } else { 0 });
    (left, right)
}

/// `scaleValues(&lb[t0..t1], s)` with a scale factor per lane.
#[inline(always)]
fn scale_rows(lb: &mut [Lanes; N_LB], t0: usize, t1: usize, s: &Lanes) {
    let (left, right) = scale_amounts(s);
    #[cfg(target_arch = "aarch64")]
    // SAFETY: NEON is part of the AArch64 baseline.
    unsafe {
        neon::scale_rows(&mut lb[t0..t1], &left, &right)
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if fdk_aac_rs_fdk::simd_x86::avx2() {
        // SAFETY: the CPU has AVX2.
        unsafe { x86::scale_rows(&mut lb[t0..t1], &left, &right) }
    } else {
        scale_rows_portable(&mut lb[t0..t1], &left, &right)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    scale_rows_portable(&mut lb[t0..t1], &left, &right)
}

/// [`scale_rows`] without SIMD.
#[allow(dead_code)]
fn scale_rows_portable(rows: &mut [Lanes], left: &Lanes, right: &Lanes) {
    for row in rows {
        for l in 0..L {
            row[l] = (row[l] << left[l]) >> right[l];
        }
    }
}

/// The loop of `autoCorr2nd_real()` (`pairs` steps from `lb[2]`) in every lane: adds its terms
/// to `accu1`, `accu3`, `accu5`.
#[inline(always)]
fn auto_corr_pairs(lb: &[Lanes; N_LB], pairs: usize, ls: u32, acc: &mut [Lanes; 3]) {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: NEON is part of the AArch64 baseline.
    unsafe {
        neon::auto_corr_pairs(lb, pairs, ls, acc)
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if fdk_aac_rs_fdk::simd_x86::avx2() {
        // SAFETY: the CPU has AVX2.
        unsafe { x86::auto_corr_pairs(lb, pairs, ls, acc) }
    } else {
        auto_corr_pairs_portable(lb, pairs, ls, acc)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    auto_corr_pairs_portable(lb, pairs, ls, acc)
}

/// [`auto_corr_pairs`] without SIMD.
#[allow(dead_code)]
fn auto_corr_pairs_portable(lb: &[Lanes; N_LB], pairs: usize, ls: u32, acc: &mut [Lanes; 3]) {
    let [accu1, accu3, accu5] = acc;
    let mut p = 2usize;
    for _ in 0..pairs {
        let (a, b, c, d) = (&lb[p], &lb[p + 1], &lb[p + 2], &lb[p + 3]);
        for l in 0..L {
            accu1[l] = accu1[l].wrapping_add(md2(a[l], a[l]).wrapping_add(md2(b[l], b[l])) >> ls);
            accu3[l] = accu3[l].wrapping_add(md2(a[l], b[l]).wrapping_add(md2(b[l], c[l])) >> ls);
            accu5[l] = accu5[l].wrapping_add(md2(a[l], c[l]).wrapping_add(md2(b[l], d[l])) >> ls);
        }
        p += 2;
    }
}

/// `autoCorr2nd_real(&ac, lb + LPC_ORDER, len)` in every lane (see
/// `fdk_aac_rs_fdk::autocorr2nd`): the coefficients and the returned scaling.
fn auto_corr2nd_real_lanes(lb: &[Lanes; N_LB], len: i32) -> ([AcorrCoefs; L], Lanes) {
    let len_scale = f_max(32 - f_normz(len / 2), 1);
    let ls = len_scale as u32;
    let mut acc = [[0i32; L]; 3];
    for l in 0..L {
        let x = |t: usize| lb[t][l];
        acc[2][l] = md2(x(0), x(2)).wrapping_add(md2(x(1), x(3))) >> ls;
        acc[0][l] = md2(x(1), x(1)) >> ls;
        acc[1][l] = md2(x(1), x(2)) >> ls;
    }
    auto_corr_pairs(lb, ((len - 2) >> 1).max(0) as usize, ls, &mut acc);
    let [accu1, accu3, accu5] = acc;
    let mut ac = [AcorrCoefs::default(); L];
    let mut scaling = [0i32; L];
    // realBuf[k] is lb[k + LPC_ORDER]
    let rb = |k: i32, l: usize| lb[(k + LPC_ORDER as i32) as usize][l];
    for l in 0..L {
        let accu2 = (md2(rb(-2, l), rb(-2, l)) >> ls).wrapping_add(accu1[l]);
        let a1 = accu1[l].wrapping_add(md2(rb(len - 2, l), rb(len - 2, l)) >> ls);
        let accu4 = (md2(rb(-1, l), rb(-2, l)) >> ls).wrapping_add(accu3[l]);
        let a3 = accu3[l].wrapping_add(md2(rb(len - 1, l), rb(len - 2, l)) >> ls);
        let a5 = accu5[l];
        let m_scale = fixnormz_d(a1 | accu2 | f_abs(a3) | f_abs(accu4) | f_abs(a5)) - 1;
        scaling[l] = m_scale - 1 - len_scale;
        let sh = m_scale as u32;
        let ac = &mut ac[l];
        ac.r11r = a1.wrapping_shl(sh);
        ac.r22r = accu2.wrapping_shl(sh);
        ac.r01r = a3.wrapping_shl(sh);
        ac.r12r = accu4.wrapping_shl(sh);
        ac.r02r = a5.wrapping_shl(sh);
        ac.det = md2(ac.r11r, ac.r22r).wrapping_sub(md2(ac.r12r, ac.r12r));
        let m = if ac.det == 0 { 0 } else { (f_abs(ac.det) as u32).leading_zeros() as i32 - 1 };
        ac.det = ac.det.wrapping_shl(m as u32);
        ac.det_scale = m - 1;
    }
    (ac, scaling)
}

/// The LPC coefficients `alphar[0]`, `alphar[1]` of a band in low power mode: the code of
/// `lppTransposer()` from `resetLPCCoeffs` to the reset of the coefficients.
fn lp_alphas(ac: &AcorrCoefs, ac_det_scale: i32) -> [FixpSgl; 2] {
    // In case of determinant < 10^-38, resetLPCCoeffs=1 has to be enforced.
    let mut reset_lpc_coeffs = ac_det_scale > 126;
    let mut alphar: [FixpSgl; LPC_ORDER] = [0; LPC_ORDER];
    if ac.det != 0 {
        let abs_det = fixp_abs(ac.det);
        let tmp = (f_mult_div2(ac.r01r, ac.r12r) >> (LPC_SCALE_FACTOR - 1))
            .wrapping_sub(f_mult_div2(ac.r02r, ac.r11r) >> (LPC_SCALE_FACTOR - 1));
        let abs_tmp = fixp_abs(tmp);
        // Quick check: is first filter coeff >= 1(4)
        let mut scale = 0i32;
        let result = f_div_norm_fast(abs_tmp, abs_det, &mut scale);
        scale += ac.det_scale;
        if scale > 0 && result >= (MAXVAL_DBL >> scale) {
            reset_lpc_coeffs = true;
        } else {
            alphar[1] = fx_dbl2fx_sgl(scale_value_saturate(result, scale));
            if (tmp < 0) ^ (ac.det < 0) {
                alphar[1] = alphar[1].wrapping_neg();
            }
        }
    }
    if ac.r11r != 0 {
        // ac.r11r is always >=0
        let tmp = if ac.r01r >= 0 {
            (ac.r01r >> (LPC_SCALE_FACTOR + 1)).wrapping_add(f_mult_div2(alphar[1], ac.r12r))
        } else {
            // -((-ac.r01r) >> 3): ac.r01r == MINVAL_DBL negates to itself (wraps).
            (ac.r01r.wrapping_neg() >> (LPC_SCALE_FACTOR + 1))
                .wrapping_neg()
                .wrapping_add(f_mult_div2(alphar[1], ac.r12r))
        };
        let abs_tmp = fixp_abs(tmp);
        // Quick check: is first filter coeff >= 1(4)
        if abs_tmp >= (ac.r11r >> 1) {
            reset_lpc_coeffs = true;
        } else {
            let mut scale = 0i32;
            let result = f_div_norm_fast(abs_tmp, fixp_abs(ac.r11r), &mut scale);
            alphar[0] = fx_dbl2fx_sgl(scale_value_saturate(result, scale + 1));
            if (tmp > 0) ^ (ac.r11r < 0) {
                alphar[0] = alphar[0].wrapping_neg();
            }
        }
    }
    if reset_lpc_coeffs {
        alphar = [0; LPC_ORDER];
    }
    alphar
}

/// The shift amounts of [`filter_lanes`] per lane.
struct Shifts {
    dynscale: Lanes,
    rescale: Lanes,
    descale: Lanes,
    copy_descale: Lanes,
}

/// What a patch does with a lane of a group.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// the band is not in the patch (or past the bands of the loop)
    Skip,
    /// `bw <= 0`: the scaled source values
    Copy,
    /// the LPC filter with `a0r`, `a1r`
    Filter,
}

/// The parameters of [`run`] that the straight loop also uses.
pub(super) struct Params<'a> {
    pub h_lpp_trans: &'a SbrLppTrans,
    pub p_settings: &'a TransposerSettings,
    pub bw_vector: &'a [FixpDbl; MAX_NUM_PATCHES],
    pub start: i32,
    pub stop: i32,
    pub first_slot_offs: i32,
    pub time_step: i32,
    pub start_sample: i32,
    pub stop_sample: i32,
    pub com_low_band_scale: i32,
    pub ov_low_band_shift: i32,
    pub low_band_shift: i32,
    pub v_k_master0: i32,
}

/// The band loop of `lppTransposer()` in low power mode (`loBand` from `start` to `stop`), eight
/// bands at a time. Returns `false` without any effect if the configuration is outside what it
/// handles (see the module documentation); the caller then runs the straight loop.
pub(super) fn run(
    pr: &Params,
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    degree_alias: &mut [FixpDbl],
) -> bool {
    let ps = pr.p_settings;
    let (start, stop) = (pr.start, pr.stop);
    let n_cols = ps.n_cols as usize;
    let overlap = ps.overlap as usize;
    let fso = pr.first_slot_offs * pr.time_step;
    let ov_end = LPC_ORDER + overlap;
    let n_end = ov_end + n_cols;
    let len = (n_cols + overlap) as i32;
    if start < 0
        || start >= stop
        || stop > 64
        || overlap & 1 != 0
        || fso < 0
        || n_end > N_LB
        || ps.no_of_patches as usize > MAX_NUM_PATCHES
    {
        return false;
    }
    // copied values: the filter states, then pairs of slots and one slot for odd nCols
    let n_states = LPC_ORDER + fso as usize;
    let pairs = ((n_cols + overlap) as i32 - fso).max(0) as usize >> 1;
    let n_qmf = 2 * pairs + (n_cols & 1);
    if n_states + n_qmf > N_LB
        || fso as usize + n_qmf > qmf_buffer_real.len()
        || LPC_LEG_RE + (n_states - 1) * LPC_LEG_COLS + stop as usize > LPC_STATES_SIZE
        || qmf_buffer_real[fso as usize..fso as usize + n_qmf]
            .iter()
            .any(|s| s.len() < stop as usize)
    {
        return false;
    }
    let (s0, s1) = (pr.start_sample, pr.stop_sample);
    if s0 < 0
        || s1 as usize > qmf_buffer_real.len()
        || (s0 < s1 && qmf_buffer_real[s0 as usize..s1 as usize].iter().any(|s| s.len() < 64))
        || (s0 < s1 && (s1 + 2) as usize > N_LB)
    {
        return false;
    }
    // the target ranges of the patches: at or above stop, below 64, disjoint
    let n_patches = ps.no_of_patches as usize;
    let mut ranges = [(0i32, 0i32); MAX_NUM_PATCHES];
    for (p, r) in ranges.iter_mut().enumerate().take(n_patches) {
        let pp = &ps.patch_param[p];
        let lo_a = start.max(pp.source_start_band as i32);
        let lo_b = stop.min(pp.source_stop_band as i32);
        if lo_a < lo_b {
            let offs = pp.target_band_offs as i32;
            *r = (lo_a + offs, lo_b + offs);
            if r.0 < stop || r.1 > 64 {
                return false;
            }
        }
    }
    for p in 0..n_patches {
        for q in p + 1..n_patches {
            let (a, b) = (ranges[p], ranges[q]);
            if a.0 < a.1 && b.0 < b.1 && a.0 < b.1 && b.0 < a.1 {
                return false;
            }
        }
    }

    let mut bw_index = [0i32; MAX_NUM_PATCHES];
    let mut k1_below: FixpDbl = 0;
    let mut k1_below2: FixpDbl = 0;
    let lpc = &pr.h_lpp_trans.lpc;
    // the rows of lowBandReal that are read: up to n_end, and the slots of the filter
    let n_read = n_end.max(if s0 < s1 { s1 as usize + 2 } else { 0 });
    let n_copied = n_states + n_qmf;
    let mut lb = [[0i32; L]; N_LB];
    let mut g0 = start;
    while g0 < stop {
        let nl = (stop - g0).min(L as i32) as usize;
        let g = g0 as usize;
        // lowBandReal of the eight bands: the copied values, zero after them (as in the straight
        // loop) and in the lanes past the last band
        let row8 = |src: &[FixpDbl], o: usize| -> Lanes {
            if nl == L {
                src[o..o + L].try_into().unwrap()
            } else {
                let mut r = [0i32; L];
                for (r, &x) in r.iter_mut().zip(&src[o..o + nl]) {
                    *r = x;
                }
                r
            }
        };
        for (t, row) in lb.iter_mut().enumerate().take(n_states) {
            *row = row8(lpc, LPC_LEG_RE + t * LPC_LEG_COLS + g);
        }
        for (k, row) in lb[n_states..n_copied].iter_mut().enumerate() {
            *row = row8(qmf_buffer_real[fso as usize + k], g);
        }
        for row in lb.iter_mut().take(n_read).skip(n_copied) {
            *row = [0; L];
        }

        // Determine dynamic scaling value.
        let sf_ov = scalefactors(&lb, 0, ov_end);
        let sf_lb = scalefactors(&lb, ov_end, n_end);
        let mut dynamic_scale = [0i32; L];
        let mut clamp = [false; L];
        for l in 0..L {
            let ds = fix_min(DFRACT_BITS - 1 - LPC_SCALE_FACTOR, sf_ov[l] + pr.ov_low_band_shift);
            let ds = fix_min(ds, sf_lb[l] + pr.low_band_shift);
            clamp[l] = ds == 0;
            // one additional bit headroom to prevent -1.0
            dynamic_scale[l] = if ds == 0 { 0 } else { fix_max(0, ds - 1) };
        }
        if clamp.iter().any(|&c| c) {
            for row in &mut lb[..n_end] {
                for l in 0..L {
                    if clamp[l] {
                        row[l] = fix_max(row[l], 0x80000001u32 as i32);
                    }
                }
            }
        }
        // Scale temporal QMF buffer.
        scale_rows(&mut lb, 0, ov_end, &dynamic_scale.map(|d| d - pr.ov_low_band_shift));
        scale_rows(&mut lb, ov_end, n_end, &dynamic_scale.map(|d| d - pr.low_band_shift));

        let (ac, scaling) = auto_corr2nd_real_lanes(&lb, len);

        // band by band: LPC coefficients, aliasing detection, the patches' coefficients
        let mut mode = [[Mode::Skip; L]; MAX_NUM_PATCHES];
        let mut a0 = [[0 as FixpSgl; L]; MAX_NUM_PATCHES];
        let mut a1 = [[0 as FixpSgl; L]; MAX_NUM_PATCHES];
        for l in 0..nl {
            let lo_band = g0 + l as i32;
            let ac = &ac[l];
            let ds = dynamic_scale[l];
            // Examine dynamic of determinant in autocorrelation.
            let ac_det_scale = 2 * (scaling[l] + 2 * (pr.com_low_band_scale + ds)) + ac.det_scale;
            let alphar = lp_alphas(ac, ac_det_scale);

            // Aliasing detection
            let k1 = if ac.r11r == 0 {
                0
            } else if fixp_abs(ac.r01r) >= fixp_abs(ac.r11r) {
                if f_mult_div2(ac.r01r, ac.r11r) < 0 {
                    MAXVAL_DBL
                } else {
                    // Since this value is squared later, it must not ever become -1.0f.
                    MINVAL_DBL + 1
                }
            } else {
                let mut scale = 0i32;
                let result = f_div_norm_fast(fixp_abs(ac.r01r), fixp_abs(ac.r11r), &mut scale);
                let k1 = scale_value_saturate(result, scale);
                if !((ac.r01r < 0) ^ (ac.r11r < 0)) {
                    k1.wrapping_neg()
                } else {
                    k1
                }
            };
            if lo_band > 1 && lo_band < pr.v_k_master0 {
                // Check if the gain should be locked
                let deg = MAXVAL_DBL - f_pow2(k1_below);
                let lb_ = lo_band as usize;
                degree_alias[lb_] = 0;
                if (lo_band & 1) == 0 && k1 < 0 {
                    if k1_below < 0 {
                        // 2-Ch Aliasing Detection
                        degree_alias[lb_] = MAXVAL_DBL;
                        if k1_below2 > 0 {
                            // 3-Ch Aliasing Detection
                            degree_alias[lb_ - 1] = deg;
                        }
                    } else if k1_below2 > 0 {
                        // 3-Ch Aliasing Detection
                        degree_alias[lb_] = deg;
                    }
                }
                if (lo_band & 1) == 1 && k1 > 0 {
                    if k1_below > 0 {
                        // 2-CH Aliasing Detection
                        degree_alias[lb_] = MAXVAL_DBL;
                        if k1_below2 < 0 {
                            // 3-CH Aliasing Detection
                            degree_alias[lb_ - 1] = deg;
                        }
                    } else if k1_below2 < 0 {
                        // 3-CH Aliasing Detection
                        degree_alias[lb_] = deg;
                    }
                }
            }
            // remember k1 values of the 2 QMF channels below the current channel
            k1_below2 = k1_below;
            k1_below = k1;

            for patch in 0..n_patches {
                let pp = &ps.patch_param[patch];
                let hi_band = lo_band + pp.target_band_offs as i32;
                if lo_band < pp.source_start_band as i32 || lo_band >= pp.source_stop_band as i32 {
                    continue;
                }
                // bwIndex[patch] is already initialized with value from previous band inside
                // this patch
                while hi_band >= ps.bw_borders[bw_index[patch] as usize] as i32
                    && bw_index[patch] < MAX_NUM_PATCHES as i32 - 1
                {
                    bw_index[patch] += 1;
                }
                let bw = fx_dbl2fx_sgl(pr.bw_vector[bw_index[patch] as usize]);
                a0[patch][l] = fx_dbl2fx_sgl(f_mult(bw, alphar[0]));
                let bw = fx_dbl2fx_sgl(f_pow2(bw));
                a1[patch][l] = fx_dbl2fx_sgl(f_mult(bw, alphar[1]));
                mode[patch][l] = if bw <= 0 { Mode::Copy } else { Mode::Filter };
            }
        }

        // Filter Step 3 of every patch for the eight bands
        for patch in 0..n_patches {
            if mode[patch].iter().all(|&m| m == Mode::Skip) {
                continue;
            }
            let hb0 = (g0 + ps.patch_param[patch].target_band_offs as i32) as usize;
            filter_lanes(
                qmf_buffer_real,
                &lb,
                hb0,
                s0,
                s1,
                &mode[patch],
                &a0[patch],
                &a1[patch],
                &dynamic_scale,
            );
        }
        g0 += L as i32;
    }
    true
}

/// `calc_qmfBufferReal()` (`Mode::Filter`) or the copy of the scaled values (`Mode::Copy`, `bw
/// <= 0`) of the lanes of a group into the target bands `hb0 + lane` of the slots `s0..s1`.
#[allow(clippy::too_many_arguments)]
fn filter_lanes(
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    lb: &[Lanes; N_LB],
    hb0: usize,
    s0: i32,
    s1: i32,
    mode: &[Mode; L],
    a0: &[FixpSgl; L],
    a1: &[FixpSgl; L],
    dynamic_scale: &Lanes,
) {
    // calc_qmfBufferReal(): dynscale, rescale, descale of the filter; LPC_SCALE_FACTOR +
    // dynamicScale of the copy (all in 0..32: 0 <= dynamicScale <= 29)
    let dynscale = dynamic_scale.map(|d| fix_max(0, d - 1) + 1);
    let rescale = dynamic_scale.map(|d| -fix_min(0, d - 1) + 1);
    let mut descale = [0i32; L];
    let mut copy_descale = [0i32; L];
    for l in 0..L {
        descale[l] = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale[l] + rescale[l]);
        copy_descale[l] = fix_min(DFRACT_BITS - 1, LPC_SCALE_FACTOR + dynamic_scale[l]);
    }
    let sc = Shifts { dynscale, rescale, descale, copy_descale };
    let (i0, i1) = (s0.max(0) as usize, s1.max(0) as usize);
    #[cfg(target_arch = "aarch64")]
    if qmf_buffer_real[i0..i1].iter().all(|r| r.len() >= hb0 + L) {
        // SAFETY: NEON is part of the AArch64 baseline; every slot holds the 8 target bands.
        return unsafe { neon::filter(qmf_buffer_real, lb, hb0, i0, i1, mode, a0, a1, &sc) };
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if fdk_aac_rs_fdk::simd_x86::avx2()
        && qmf_buffer_real[i0..i1].iter().all(|r| r.len() >= hb0 + L)
    {
        // SAFETY: the CPU has AVX2; every slot holds the 8 target bands.
        return unsafe { x86::filter(qmf_buffer_real, lb, hb0, i0, i1, mode, a0, a1, &sc) };
    }
    let Shifts { dynscale, rescale, descale, copy_descale } = sc;
    for i in i0..i1 {
        // lowBandReal[LPC_ORDER + i - 2 .. LPC_ORDER + i]
        let (x2, x1, x0) = (&lb[i], &lb[i + 1], &lb[i + 2]);
        let out = &mut qmf_buffer_real[i][hb0..];
        for l in 0..L {
            match mode[l] {
                Mode::Skip => {}
                Mode::Copy => out[l] = x0[l] >> copy_descale[l],
                Mode::Filter => {
                    let accu = f_mult_div2(a1[l], x2[l]).wrapping_add(f_mult_div2(a0[l], x1[l]));
                    let accu = (x0[l] >> descale[l]).wrapping_add(accu >> dynscale[l]);
                    out[l] = saturate_left_shift(accu, rescale[l], DFRACT_BITS);
                }
            }
        }
    }
}

/// The lanes of `mode` equal to `m` as a mask (all ones or 0 per lane).
#[inline(always)]
fn mode_mask(mode: &[Mode; L], m: Mode) -> Lanes {
    mode.map(|x| if x == m { -1 } else { 0 })
}

/// NEON kernels (every variant: the operations are the same on all of them), four lanes at a
/// time:
/// - `fMultDiv2(FIXP_DBL, FIXP_DBL)` is the high word of the 64-bit product (`smull`, `uzp2`);
/// - `fMultDiv2(FIXP_SGL a, FIXP_DBL x)` = `(a * x) >> 16` is `sqdmulh(x, a << 15)`, which
///   cannot saturate (`a << 15 != INT_MIN`);
/// - `sshl` by a lane amount in `-31..=31` is the left shift (bits shifted out) or the arithmetic
///   right shift;
/// - `SATURATE_LEFT_SHIFT` is the formula of `arm/scale_arm.h`, which equals the generic one.
#[cfg(target_arch = "aarch64")]
mod neon {
    use super::{Lanes, Mode, Shifts, L, N_LB};
    use core::arch::aarch64::*;
    use fdk_aac_rs_fdk::common_fix::{FixpDbl, FixpSgl, MAXVAL_DBL};

    #[inline(always)]
    unsafe fn ld(p: &Lanes, h: usize) -> int32x4_t {
        vld1q_s32(p.as_ptr().add(4 * h))
    }

    #[inline(always)]
    unsafe fn st(p: &mut Lanes, h: usize, v: int32x4_t) {
        vst1q_s32(p.as_mut_ptr().add(4 * h), v)
    }

    /// `(a * b) >> 32` per lane.
    #[inline(always)]
    unsafe fn hi(a: int32x4_t, b: int32x4_t) -> int32x4_t {
        let lo = vreinterpretq_s32_s64(vmull_s32(vget_low_s32(a), vget_low_s32(b)));
        let hi = vreinterpretq_s32_s64(vmull_high_s32(a, b));
        vuzp2q_s32(lo, hi)
    }

    /// See `super::scale_rows`.
    pub(super) unsafe fn scale_rows(rows: &mut [Lanes], left: &Lanes, right: &Lanes) {
        for h in 0..2 {
            // one of the two amounts is 0: left - right is the signed sshl amount
            let sh = vsubq_s32(ld(left, h), ld(right, h));
            for row in rows.iter_mut() {
                st(row, h, vshlq_s32(ld(row, h), sh));
            }
        }
    }

    /// See `super::auto_corr_pairs`.
    pub(super) unsafe fn auto_corr_pairs(
        lb: &[Lanes; N_LB],
        pairs: usize,
        ls: u32,
        acc: &mut [Lanes; 3],
    ) {
        let sh = vdupq_n_s32(-(ls as i32));
        for h in 0..2 {
            let (mut a1, mut a3, mut a5) = (ld(&acc[0], h), ld(&acc[1], h), ld(&acc[2], h));
            let mut p = 2;
            for _ in 0..pairs {
                let (a, b) = (ld(&lb[p], h), ld(&lb[p + 1], h));
                let (c, d) = (ld(&lb[p + 2], h), ld(&lb[p + 3], h));
                a1 = vaddq_s32(a1, vshlq_s32(vaddq_s32(hi(a, a), hi(b, b)), sh));
                a3 = vaddq_s32(a3, vshlq_s32(vaddq_s32(hi(a, b), hi(b, c)), sh));
                a5 = vaddq_s32(a5, vshlq_s32(vaddq_s32(hi(a, c), hi(b, d)), sh));
                p += 2;
            }
            st(&mut acc[0], h, a1);
            st(&mut acc[1], h, a3);
            st(&mut acc[2], h, a5);
        }
    }

    /// See `super::filter_lanes`.
    ///
    /// # Safety
    /// Every slot `i0..i1` holds the bands `hb0..hb0 + 8`.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn filter(
        rows: &mut [&mut [FixpDbl]],
        lb: &[Lanes; N_LB],
        hb0: usize,
        i0: usize,
        i1: usize,
        mode: &[Mode; L],
        a0: &[FixpSgl; L],
        a1: &[FixpSgl; L],
        sc: &Shifts,
    ) {
        let (mf, mc) = (super::mode_mask(mode, Mode::Filter), super::mode_mask(mode, Mode::Copy));
        for h in 0..2 {
            let (mf, mc) = (vreinterpretq_u32_s32(ld(&mf, h)), vreinterpretq_u32_s32(ld(&mc, h)));
            if vmaxvq_u32(vorrq_u32(mf, mc)) == 0 {
                continue;
            }
            let a0v = vshlq_n_s32::<15>(vmovl_s16(vld1_s16(a0.as_ptr().add(4 * h))));
            let a1v = vshlq_n_s32::<15>(vmovl_s16(vld1_s16(a1.as_ptr().add(4 * h))));
            let de = vnegq_s32(ld(&sc.descale, h));
            let dy = vnegq_s32(ld(&sc.dynscale, h));
            let cd = vnegq_s32(ld(&sc.copy_descale, h));
            let rs = ld(&sc.rescale, h);
            let lim = vshlq_s32(vdupq_n_s32(MAXVAL_DBL), vnegq_s32(rs));
            let max = vdupq_n_s32(MAXVAL_DBL);
            for i in i0..i1 {
                let (x2, x1, x0) = (ld(&lb[i], h), ld(&lb[i + 1], h), ld(&lb[i + 2], h));
                let accu = vaddq_s32(vqdmulhq_s32(x2, a1v), vqdmulhq_s32(x1, a0v));
                let accu = vaddq_s32(vshlq_s32(x0, de), vshlq_s32(accu, dy));
                let sign = vshrq_n_s32::<31>(accu);
                let over = vcgtq_s32(veorq_s32(accu, sign), lim);
                let f = vbslq_s32(over, veorq_s32(sign, max), vshlq_s32(accu, rs));
                let c = vshlq_s32(x0, cd);
                let p = rows[i].as_mut_ptr().add(hb0 + 4 * h);
                vst1q_s32(p, vbslq_s32(mf, f, vbslq_s32(mc, c, vld1q_s32(p))));
            }
        }
    }
}

/// AVX2 kernels (every variant), eight lanes at a time:
/// - `fMultDiv2(FIXP_DBL, FIXP_DBL)` is the high word of the 64-bit products of `vpmuldq`;
/// - `fMultDiv2(FIXP_SGL a, FIXP_DBL x)` = `(a * x) >> 16` is `(x >> 16) * a + (((x & 0xFFFF) *
///   a) >> 16)` in 32-bit lanes (`|a (x >> 16)| <= 2^30`, `|a (x & 0xFFFF)| < 2^31`);
/// - the shifts by lane amounts in `0..=31` are `vpsllvd`/`vpsravd`;
/// - `SATURATE_LEFT_SHIFT` is `MAXVAL_DBL` above `MAXVAL_DBL >> s`, `MINVAL_DBL` below its
///   complement and the shifted value otherwise.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86 {
    use super::{Lanes, Mode, Shifts, L, N_LB};
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;
    use fdk_aac_rs_fdk::common_fix::{FixpDbl, FixpSgl, MAXVAL_DBL, MINVAL_DBL};

    #[inline(always)]
    unsafe fn ld(p: &Lanes) -> __m256i {
        _mm256_loadu_si256(p.as_ptr() as *const __m256i)
    }

    #[inline(always)]
    unsafe fn st(p: &mut Lanes, v: __m256i) {
        _mm256_storeu_si256(p.as_mut_ptr() as *mut __m256i, v)
    }

    /// `(a * b) >> 32` per lane.
    #[inline(always)]
    unsafe fn hi(a: __m256i, b: __m256i) -> __m256i {
        let even = _mm256_srli_epi64::<32>(_mm256_mul_epi32(a, b));
        let odd = _mm256_mul_epi32(_mm256_srli_epi64::<32>(a), _mm256_srli_epi64::<32>(b));
        _mm256_blend_epi32::<0b1010_1010>(even, odd)
    }

    /// `(a * x) >> 16` per lane (`a` a 16-bit value).
    #[inline(always)]
    unsafe fn mdiv2(a: __m256i, x: __m256i) -> __m256i {
        let h = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(x), a);
        let l = _mm256_mullo_epi32(_mm256_and_si256(x, _mm256_set1_epi32(0xFFFF)), a);
        _mm256_add_epi32(h, _mm256_srai_epi32::<16>(l))
    }

    /// See `super::scale_rows`.
    ///
    /// # Safety
    /// The CPU must have AVX2.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn scale_rows(rows: &mut [Lanes], left: &Lanes, right: &Lanes) {
        let (l, r) = (ld(left), ld(right));
        for row in rows.iter_mut() {
            st(row, _mm256_srav_epi32(_mm256_sllv_epi32(ld(row), l), r));
        }
    }

    /// See `super::auto_corr_pairs`.
    ///
    /// # Safety
    /// The CPU must have AVX2.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn auto_corr_pairs(
        lb: &[Lanes; N_LB],
        pairs: usize,
        ls: u32,
        acc: &mut [Lanes; 3],
    ) {
        let sh = _mm_cvtsi32_si128(ls as i32);
        let (mut a1, mut a3, mut a5) = (ld(&acc[0]), ld(&acc[1]), ld(&acc[2]));
        let mut p = 2;
        for _ in 0..pairs {
            let (a, b, c, d) = (ld(&lb[p]), ld(&lb[p + 1]), ld(&lb[p + 2]), ld(&lb[p + 3]));
            a1 = _mm256_add_epi32(a1, _mm256_sra_epi32(_mm256_add_epi32(hi(a, a), hi(b, b)), sh));
            a3 = _mm256_add_epi32(a3, _mm256_sra_epi32(_mm256_add_epi32(hi(a, b), hi(b, c)), sh));
            a5 = _mm256_add_epi32(a5, _mm256_sra_epi32(_mm256_add_epi32(hi(a, c), hi(b, d)), sh));
            p += 2;
        }
        st(&mut acc[0], a1);
        st(&mut acc[1], a3);
        st(&mut acc[2], a5);
    }

    /// See `super::filter_lanes`.
    ///
    /// # Safety
    /// The CPU must have AVX2; every slot `i0..i1` holds the bands `hb0..hb0 + 8`.
    #[allow(clippy::too_many_arguments)]
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn filter(
        rows: &mut [&mut [FixpDbl]],
        lb: &[Lanes; N_LB],
        hb0: usize,
        i0: usize,
        i1: usize,
        mode: &[Mode; L],
        a0: &[FixpSgl; L],
        a1: &[FixpSgl; L],
        sc: &Shifts,
    ) {
        let mf = ld(&super::mode_mask(mode, Mode::Filter));
        let mc = ld(&super::mode_mask(mode, Mode::Copy));
        let a0v = _mm256_cvtepi16_epi32(_mm_loadu_si128(a0.as_ptr() as *const __m128i));
        let a1v = _mm256_cvtepi16_epi32(_mm_loadu_si128(a1.as_ptr() as *const __m128i));
        let (de, dy) = (ld(&sc.descale), ld(&sc.dynscale));
        let (cd, rs) = (ld(&sc.copy_descale), ld(&sc.rescale));
        let max = _mm256_set1_epi32(MAXVAL_DBL);
        let lim = _mm256_srav_epi32(max, rs);
        let nlim = _mm256_xor_si256(lim, _mm256_set1_epi32(-1));
        for i in i0..i1 {
            let (x2, x1, x0) = (ld(&lb[i]), ld(&lb[i + 1]), ld(&lb[i + 2]));
            let accu = _mm256_add_epi32(mdiv2(a1v, x2), mdiv2(a0v, x1));
            let accu = _mm256_add_epi32(_mm256_srav_epi32(x0, de), _mm256_srav_epi32(accu, dy));
            let over = _mm256_cmpgt_epi32(accu, lim);
            let under = _mm256_cmpgt_epi32(nlim, accu);
            let f = _mm256_blendv_epi8(_mm256_sllv_epi32(accu, rs), max, over);
            let f = _mm256_blendv_epi8(f, _mm256_set1_epi32(MINVAL_DBL), under);
            let c = _mm256_srav_epi32(x0, cd);
            let p = rows[i].as_mut_ptr().add(hb0) as *mut __m256i;
            let v = _mm256_blendv_epi8(_mm256_blendv_epi8(_mm256_loadu_si256(p), c, mc), f, mf);
            _mm256_storeu_si256(p, v);
        }
    }
}
