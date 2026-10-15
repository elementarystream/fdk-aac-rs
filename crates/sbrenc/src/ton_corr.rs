// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/ton_corr.h, libSBRenc/src/ton_corr.cpp.

//! Port of `ton_corr.h` and `ton_corr.cpp`: the tonality correction parameter extraction
//! (tonality quotas, patching, and the calls of the inverse filtering, missing harmonics and
//! noise floor estimators).
//!
//! The quota and sign matrices (`quotaMatrix[i]`, `signMatrix[i]`: pointers to rows of 64
//! values of per channel allocations in C) are owned flat arrays, row `i` at `i * 64`.

use fdk_aac_rs_fdk::autocorr2nd::{auto_corr2nd_cplx, AcorrCoefs, LPC_ORDER};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

use crate::fram_gen::*;
use crate::invf_est::*;
use crate::mh_det::*;
use crate::nf_est::*;
use crate::sbr_def::*;
use crate::sbr_encoder::{SbrConfigData, SBR_SYNTAX_LOW_DELAY};
use crate::sbr_misc::schur_div_fast;

mod lanes;
#[allow(unused_imports)]
use lanes::Lanes;

/// `MAX_NUM_PATCHES`
pub const MAX_NUM_PATCHES: usize = 6;
/// `SCALE_NRGVEC`
pub const SCALE_NRGVEC: i32 = 4;

/// `BAND_V_SIZE`
const BAND_V_SIZE: usize = 32;
/// `NUM_V_COMBINE`: Must be a divisor of 64 and fulfill the ASSERTs below
const NUM_V_COMBINE: usize = 8;

/// `PATCH_PARAM`: parameter set for one single patch
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PatchParam {
    /// `sourceStartBand`: first band in lowbands where to take the samples from
    pub source_start_band: i32,
    /// `sourceStopBand`: first band in lowbands which is not included in the patch anymore
    pub source_stop_band: i32,
    /// `guardStartBand`: first band in highbands to be filled with zeros in order to reduce
    /// interferences between patches
    pub guard_start_band: i32,
    /// `targetStartBand`: first band in highbands to be filled with whitened lowband signal
    pub target_start_band: i32,
    /// `targetBandOffs`: difference between 'startTargetBand' and 'startSourceBand'
    pub target_band_offs: i32,
    /// `numBandsInPatch`: number of consecutive bands in this one patch
    pub num_bands_in_patch: i32,
}

/// `SBR_TON_CORR_EST` (`HANDLE_SBR_TON_CORR_EST` is `&mut SbrTonCorrEst`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbrTonCorrEst {
    /// `switchInverseFilt`: Flag to enable dynamic adaption of invf. detection
    pub switch_inverse_filt: i32,
    /// `noQmfChannels`
    pub no_qmf_channels: i32,
    /// `bufferLength`: Length of the r and i buffers.
    pub buffer_length: i32,
    /// `stepSize`: Stride for the lpc estimate.
    pub step_size: i32,
    /// `numberOfEstimates`: The total number of estiamtes, available in the quotaMatrix.
    pub number_of_estimates: i32,
    /// `numberOfEstimatesPerFrame`: The number of estimates per frame available in the
    /// quotaMatrix.
    pub number_of_estimates_per_frame: u32,
    /// `lpcLength`: Segment length used for second order LPC analysis.
    pub lpc_length: [i32; 2],
    /// `nextSample`: Where to start the LPC analysis of the current frame.
    pub next_sample: i32,
    /// `move`: How many estimates to move in the quotaMatrix, when buffering.
    pub move_: i32,
    /// `frameStartIndex`: The start index for the current frame in the r and i buffers.
    pub frame_start_index: i32,
    /// `startIndexMatrix`: The start index for the current frame in the quotaMatrix.
    pub start_index_matrix: i32,
    /// `frameStartIndexInvfEst`: The start index of the inverse filtering
    pub frame_start_index_invf_est: i32,
    /// `prevTransientFlag`: The transisent flag (from the transient detector) for the previous
    /// frame.
    pub prev_transient_flag: i32,
    /// `transientNextFrame`: Flag to indicate that the transient will show up in the next
    /// frame.
    pub transient_next_frame: i32,
    /// `transientPosOffset`: An offset value to match the transient pos as calculated by the
    /// transient detector with the actual position in the frame.
    pub transient_pos_offset: i32,

    /// `signMatrix[i]`: row `i` (64 values) of `Ram_Sbr_signMatrix`: the sign of each channel
    pub sign_matrix: [i32; MAX_NO_OF_ESTIMATES * 64],
    /// `quotaMatrix[i]`: row `i` (64 values) of `Ram_Sbr_quotaMatrix`: the quota values for
    /// all estimates, all channels
    pub quota_matrix: [FixpDbl; MAX_NO_OF_ESTIMATES * 64],
    /// `nrgVector`: the averaged energies for every QMF band
    pub nrg_vector: [FixpDbl; MAX_NO_OF_ESTIMATES],
    /// `nrgVectorFreq`: the averaged energies for every QMF channel
    pub nrg_vector_freq: [FixpDbl; 64],

    /// `indexVector`: Index vector poINTing to the correct lowband channel, when indexing a
    /// highband channel, -1 represents a guard band
    pub index_vector: [i8; 64],
    /// `patchParam`: new parameter set for patching
    pub patch_param: [PatchParam; MAX_NUM_PATCHES],
    /// `guard`: number of guardbands between every patch
    pub guard: i32,
    /// `shiftStartSb`: lowest subband of source range to be included in the patches
    pub shift_start_sb: i32,
    /// `noOfPatches`: number of patches
    pub no_of_patches: i32,

    /// `sbrMissingHarmonicsDetector`
    pub sbr_missing_harmonics_detector: SbrMissingHarmonicsDetector,
    /// `sbrNoiseFloorEstimate`
    pub sbr_noise_floor_estimate: SbrNoiseFloorEstimate,
    /// `sbrInvFilt`
    pub sbr_inv_filt: SbrInvFiltEst,
}

impl Default for SbrTonCorrEst {
    fn default() -> Self {
        SbrTonCorrEst {
            switch_inverse_filt: 0,
            no_qmf_channels: 0,
            buffer_length: 0,
            step_size: 0,
            number_of_estimates: 0,
            number_of_estimates_per_frame: 0,
            lpc_length: [0; 2],
            next_sample: 0,
            move_: 0,
            frame_start_index: 0,
            start_index_matrix: 0,
            frame_start_index_invf_est: 0,
            prev_transient_flag: 0,
            transient_next_frame: 0,
            transient_pos_offset: 0,
            sign_matrix: [0; MAX_NO_OF_ESTIMATES * 64],
            quota_matrix: [0; MAX_NO_OF_ESTIMATES * 64],
            nrg_vector: [0; MAX_NO_OF_ESTIMATES],
            nrg_vector_freq: [0; 64],
            index_vector: [0; 64],
            patch_param: [PatchParam::default(); MAX_NUM_PATCHES],
            guard: 0,
            shift_start_sb: 0,
            no_of_patches: 0,
            sbr_missing_harmonics_detector: SbrMissingHarmonicsDetector::default(),
            sbr_noise_floor_estimate: SbrNoiseFloorEstimate::default(),
            sbr_inv_filt: SbrInvFiltEst::default(),
        }
    }
}

/// `CountLeadingBits(x)` of a `fixp_abs()` result `x` as the reference builds compile it:
/// `fAbs()` negates with signed overflow, so clang takes `x` as not negative and drops the sign
/// fold of `fixnorm_D` (`clz(x) - 1`, 0 for 0). This is -1 instead of 0 for `MINVAL_DBL`.
#[inline(always)]
fn clb_of_abs(x: FixpDbl) -> i32 {
    if x == 0 {
        0
    } else {
        (x as u32).leading_zeros() as i32 - 1
    }
}

/// `FDKsbrEnc_CalculateTonalityQuotas(hTonCorr, sourceBufferReal, sourceBufferImag, usb,
/// qmfScale)`: the tonal to noise ratio for different frequency bands and time segments
/// (`A/B = q/(1-q)` in the quota matrix, scaled by RELAXATION; `nrgVector` scaled by 1/16,
/// `nrgVectorFreq` by 1/2). The QMF buffers are rows of (at least) 64 values.
///
/// The estimates of 4 (NEON) or 8 (AVX2, run time) bands at a time when the buffers allow it
/// (see `calculate_tonality_quotas_lanes`), otherwise
/// [`fdk_sbr_enc_calculate_tonality_quotas_straight`].
pub fn fdk_sbr_enc_calculate_tonality_quotas(
    h: &mut SbrTonCorrEst,
    source_buffer_real: &[&[FixpDbl]],
    source_buffer_imag: &[&[FixpDbl]],
    usb: i32,
    qmf_scale: i32,
) {
    #[cfg(target_arch = "aarch64")]
    if lanes_usable(h, source_buffer_real, source_buffer_imag, usb, 4) {
        tonality_prologue(h);
        // SAFETY: NEON is part of the AArch64 baseline.
        unsafe {
            calculate_tonality_quotas_lanes::<lanes::Neon>(
                h,
                source_buffer_real,
                source_buffer_imag,
                usb,
                qmf_scale,
            )
        };
        return;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if crate::simd::avx2() && lanes_usable(h, source_buffer_real, source_buffer_imag, usb, 8) {
        tonality_prologue(h);
        // SAFETY: the CPU supports AVX2.
        unsafe {
            calculate_tonality_quotas_avx2(
                h,
                source_buffer_real,
                source_buffer_imag,
                usb,
                qmf_scale,
            )
        };
        return;
    }
    fdk_sbr_enc_calculate_tonality_quotas_straight(
        h,
        source_buffer_real,
        source_buffer_imag,
        usb,
        qmf_scale,
    );
}

/// Whether [`calculate_tonality_quotas_lanes`] with `l` lanes computes the same as the straight
/// function: every estimate reads only slots of its own band (`nextSample >= LPC_ORDER`, at most
/// `BAND_V_SIZE` slots, block lengths that fit), and the rows of the slots hold the bands up to
/// `usb` rounded up to `l` (at most 64).
#[allow(dead_code)]
fn lanes_usable(
    h: &SbrTonCorrEst,
    re: &[&[FixpDbl]],
    im: &[&[FixpDbl]],
    usb: i32,
    l: usize,
) -> bool {
    let buff_len = h.buffer_length;
    let bands = (usb.max(0) as usize).div_ceil(l) * l;
    h.next_sample >= LPC_ORDER as i32
        && (0..=BAND_V_SIZE as i32).contains(&buff_len)
        && h.lpc_length.iter().all(|&b| (1..=buff_len).contains(&b))
        && h.step_size >= 1
        && (0..=64).contains(&usb)
        && bands <= 64
        && re.len() >= buff_len as usize
        && im.len() >= buff_len as usize
        && re[..buff_len as usize].iter().all(|row| row.len() >= bands)
        && im[..buff_len as usize].iter().all(|row| row.len() >= bands)
}

/// The buffering at the start of `FDKsbrEnc_CalculateTonalityQuotas()`: the quota and sign
/// matrices and the energy vector move by the estimates of a frame; the energies of the frame
/// are cleared.
fn tonality_prologue(h: &mut SbrTonCorrEst) {
    let start_index_matrix = h.start_index_matrix;
    let tot_no_est = h.number_of_estimates;
    let no_est_per_frame = h.number_of_estimates_per_frame as i32;
    let move_ = h.move_;
    let no_qmf_channels = h.no_qmf_channels.max(0) as usize; // Number of Bands

    // Buffering of the quotaMatrix and the quotaMatrixTransp.
    for i in 0..move_.max(0) as usize {
        let s = (i + no_est_per_frame as usize) * 64;
        h.quota_matrix.copy_within(s..s + no_qmf_channels, i * 64);
        h.sign_matrix.copy_within(s..s + no_qmf_channels, i * 64);
    }

    let ns = no_est_per_frame as usize;
    h.nrg_vector.copy_within(ns..ns + move_.max(0) as usize, 0);
    for v in h.nrg_vector[start_index_matrix as usize..tot_no_est as usize].iter_mut() {
        *v = 0;
    }
    h.nrg_vector_freq[..no_qmf_channels].fill(0);
}

/// [`fdk_sbr_enc_calculate_tonality_quotas`] with AVX2.
///
/// # Safety
///
/// The CPU must support AVX2; [`lanes_usable`] with 8 lanes holds.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn calculate_tonality_quotas_avx2(
    h: &mut SbrTonCorrEst,
    source_buffer_real: &[&[FixpDbl]],
    source_buffer_imag: &[&[FixpDbl]],
    usb: i32,
    qmf_scale: i32,
) {
    calculate_tonality_quotas_lanes::<lanes::Avx2>(
        h,
        source_buffer_real,
        source_buffer_imag,
        usb,
        qmf_scale,
    )
}

/// The estimate loop of `FDKsbrEnc_CalculateTonalityQuotas()` for `V::N` bands at a time, one
/// band per lane (after [`tonality_prologue`]).
///
/// Exactness: each lane holds the slots of its band, as the straight function's band buffer
/// does (the same values: a copy of the band's column of the QMF rows), and goes through the
/// estimates in the same order with the same operations per lane: the scale factor
/// (`getScalefactor()` of the real and imaginary segment), the in-place scaling (which the next
/// estimate's overlapping slots see, as in the straight function), and `autoCorr2nd_cplx()`
/// ([`lanes::acorr`]). The bands' results are then processed by the same scalar code
/// ([`quota_and_energy`]), estimate by estimate instead of band by band: every band writes its
/// own entries of the quota and sign matrices and of `nrgVectorFreq`, and adds to
/// `nrgVector[timeIndex]` with wrap-around, which does not depend on the order. Lanes past `usb`
/// are computed and discarded.
///
/// # Safety
///
/// The vector instructions of `V` must be available; [`lanes_usable`] with `V::N` lanes holds.
#[inline(always)]
unsafe fn calculate_tonality_quotas_lanes<V: lanes::Lanes>(
    h: &mut SbrTonCorrEst,
    source_buffer_real: &[&[FixpDbl]],
    source_buffer_imag: &[&[FixpDbl]],
    usb: i32,
    qmf_scale: i32,
) {
    let buff_len = h.buffer_length as usize;
    let step_size = h.step_size;
    let p_block_length = h.lpc_length;
    let mut re = [V::splat(0); BAND_V_SIZE];
    let mut im = [V::splat(0); BAND_V_SIZE];
    // the lanes of the autocorrelation results and the scale factors (initialized once: every
    // estimate stores the V::N lanes it reads)
    let mut ac_lanes = [[0i32; 16]; 12];
    let mut sc_l = [0i32; 16];
    let mut r0 = 0usize;
    while r0 < usb as usize {
        for i in 0..buff_len {
            re[i] = V::load(source_buffer_real[i].as_ptr().add(r0));
            im[i] = V::load(source_buffer_imag[i].as_ptr().add(r0));
        }
        let mut k = h.next_sample;
        let mut time_index = h.start_index_matrix;
        let mut block_length = p_block_length[0];
        while k <= h.buffer_length - block_length {
            let s = k as usize - LPC_ORDER;
            let n = LPC_ORDER + block_length as usize;
            let (seg_re, seg_im) = (&mut re[s..s + n], &mut im[s..s + n]);
            // fixMax(0, fixMin(getScalefactor(re), getScalefactor(im)) - 1), per lane
            let sc = V::scalefactor_pair(seg_re, seg_im);
            for x in seg_re.iter_mut().chain(seg_im.iter_mut()) {
                *x = x.shl_lanes(sc);
            }
            let res = lanes::acorr::<V>(seg_re, seg_im, block_length);
            for (dst, v) in ac_lanes.iter_mut().zip(res.values.iter()) {
                v.store(dst.as_mut_ptr());
            }
            sc.store(sc_l.as_mut_ptr());
            for lane in 0..V::N.min(usb as usize - r0) {
                let g = |j: usize| ac_lanes[j][lane];
                let ac = AcorrCoefs {
                    r00r: g(0),
                    r11r: g(1),
                    r22r: g(2),
                    r01r: g(3),
                    r02r: g(4),
                    r12r: g(5),
                    r01i: g(6),
                    r02i: g(7),
                    r12i: g(8),
                    det: g(9),
                    det_scale: g(10),
                };
                // autoCorrScaling = (scale << 1) + autoCorr2nd_cplx()
                let auto_corr_scaling = (sc_l[lane] << 1) + g(11);
                quota_and_energy(
                    h,
                    &ac,
                    auto_corr_scaling,
                    (r0 + lane) as i32,
                    time_index,
                    qmf_scale,
                );
            }
            block_length = p_block_length[1];
            k += step_size;
            time_index += 1;
        }
        r0 += V::N;
    }
}

/// `FDKsbrEnc_CalculateTonalityQuotas()`: straight port (one band at a time), kept as the
/// reference of [`fdk_sbr_enc_calculate_tonality_quotas`].
#[doc(hidden)]
pub fn fdk_sbr_enc_calculate_tonality_quotas_straight(
    h: &mut SbrTonCorrEst,
    source_buffer_real: &[&[FixpDbl]],
    source_buffer_imag: &[&[FixpDbl]],
    usb: i32,
    qmf_scale: i32,
) {
    let start_index_matrix = h.start_index_matrix;
    let buff_len = h.buffer_length; // Number of Slots
    let step_size = h.step_size;
    let p_block_length = h.lpc_length;

    let mut ac = AcorrCoefs::default();
    let mut real_buf_ref = [0 as FixpDbl; 2 * BAND_V_SIZE * NUM_V_COMBINE];
    // realBuf = realBufRef + real_off, imagBuf = realBuf + BAND_V_SIZE * NUM_V_COMBINE
    let mut real_off = 0usize;
    const IMAG: usize = BAND_V_SIZE * NUM_V_COMBINE;

    fdk_assert!(buff_len as usize <= BAND_V_SIZE);

    tonality_prologue(h);

    // Calculate the quotas for the current time steps.
    for r in 0..usb.max(0) {
        let mut k = h.next_sample; // startSample
        let mut time_index = start_index_matrix;
        // Copy as many as possible Band across all Slots at once
        if real_off != 0 {
            real_off -= BAND_V_SIZE;
        } else {
            real_off += BAND_V_SIZE * (NUM_V_COMBINE - 1);
            for i in 0..buff_len.max(0) as usize {
                // the bands r..r + NUM_V_COMBINE of the slot inside both rows: the same copies
                // without the per-value range tests (ptr = real_off + i - v * BAND_V_SIZE)
                let b = r as usize..r as usize + NUM_V_COMBINE;
                if let (true, Some(re), Some(im)) = (
                    i < BAND_V_SIZE,
                    source_buffer_real[i].get(b.clone()),
                    source_buffer_imag[i].get(b),
                ) {
                    let (dre, dim) = real_buf_ref.split_at_mut(IMAG);
                    for v in 0..NUM_V_COMBINE {
                        dre[real_off + i - v * BAND_V_SIZE] = re[v];
                        dim[real_off + i - v * BAND_V_SIZE] = im[v];
                    }
                    continue;
                }
                let mut ptr = real_off + i;
                for v in 0..NUM_V_COMBINE {
                    let b = r as usize + v;
                    real_buf_ref[ptr] = source_buffer_real[i].get(b).copied().unwrap_or(0);
                    real_buf_ref[ptr + IMAG] = source_buffer_imag[i].get(b).copied().unwrap_or(0);
                    ptr = ptr.wrapping_sub(BAND_V_SIZE);
                }
            }
        }
        let mut block_length = p_block_length[0];

        while k <= buff_len - block_length {
            let s = real_off + k as usize - LPC_ORDER;
            let n = LPC_ORDER + block_length as usize;
            let mut auto_corr_scaling = fix_min(
                get_scalefactor(&real_buf_ref[s..s + n]),
                get_scalefactor(&real_buf_ref[IMAG + s..IMAG + s + n]),
            );
            auto_corr_scaling = fix_max(0, auto_corr_scaling - 1);

            scale_values(&mut real_buf_ref[s..s + n], auto_corr_scaling);
            scale_values(&mut real_buf_ref[IMAG + s..IMAG + s + n], auto_corr_scaling);

            auto_corr_scaling <<= 1; // consider qmf buffer scaling twice
            let (re, im) = real_buf_ref.split_at(IMAG);
            auto_corr_scaling += auto_corr2nd_cplx(&mut ac, &re[s..], &im[s..], block_length);

            quota_and_energy(h, &ac, auto_corr_scaling, r, time_index, qmf_scale);

            block_length = p_block_length[1];
            k += step_size;
            time_index += 1;
        }
    }
}

/// The quota, sign and energies of band `r` at estimate `time_index` from its autocorrelation
/// (the end of the estimate loop of `FDKsbrEnc_CalculateTonalityQuotas()`).
#[inline(always)]
fn quota_and_energy(
    h: &mut SbrTonCorrEst,
    ac: &AcorrCoefs,
    auto_corr_scaling: i32,
    r: i32,
    time_index: i32,
    qmf_scale: i32,
) {
    {
        {
            let mut alphar = [0 as FixpDbl; 2];
            let mut alphai = [0 as FixpDbl; 2];
            let fac;
            if ac.det == 0 {
                alphar[1] = 0;
                alphai[1] = 0;

                alphar[0] = ac.r01r >> 2;
                alphai[0] = ac.r01i >> 2;

                fac = f_mult_div2(ac.r00r, ac.r11r) >> 1;
            } else {
                alphar[1] = (f_mult_div2(ac.r01r, ac.r12r) >> 1)
                    .wrapping_sub(f_mult_div2(ac.r01i, ac.r12i) >> 1)
                    .wrapping_sub(f_mult_div2(ac.r02r, ac.r11r) >> 1);
                alphai[1] = (f_mult_div2(ac.r01i, ac.r12r) >> 1)
                    .wrapping_add(f_mult_div2(ac.r01r, ac.r12i) >> 1)
                    .wrapping_sub(f_mult_div2(ac.r02i, ac.r11r) >> 1);

                alphar[0] = f_mult_div2(ac.r01r, ac.det)
                    .wrapping_shr((ac.det_scale + 1) as u32)
                    .wrapping_add(f_mult(alphar[1], ac.r12r))
                    .wrapping_add(f_mult(alphai[1], ac.r12i));
                alphai[0] = f_mult_div2(ac.r01i, ac.det)
                    .wrapping_shr((ac.det_scale + 1) as u32)
                    .wrapping_add(f_mult(alphai[1], ac.r12r))
                    .wrapping_sub(f_mult(alphar[1], ac.r12i));

                // C: `>> (ac->det_scale + 1)`; det_scale can be -1 or below for full-scale input (a
                // negative shift amount), which both builds take modulo 32.
                fac = f_mult_div2(ac.r00r, f_mult(ac.det, ac.r11r))
                    .wrapping_shr((ac.det_scale + 1) as u32);
            }

            let ti = time_index as usize * 64 + r as usize;
            if fac == 0 {
                h.quota_matrix[ti] = 0;
                h.sign_matrix[ti] = 0;
            } else {
                // quotaMatrix is scaled with the factor RELAXATION
                // parse RELAXATION in fractional part and shift factor: 1/(1/0.524288 *
                // 2^RELAXATION_SHIFT)
                let mut num = f_mult_div2(alphar[0], ac.r01r)
                    .wrapping_add(f_mult_div2(alphai[0], ac.r01i))
                    .wrapping_sub(f_mult_div2(alphar[1], f_mult(ac.r02r, ac.r11r)))
                    .wrapping_sub(f_mult_div2(alphai[1], f_mult(ac.r02i, ac.r11r)));
                num = fixp_abs(num);

                let mut denom = (fac >> 1)
                    .wrapping_add(f_mult_div2(fac, RELAXATION_FRACT) >> RELAXATION_SHIFT)
                    .wrapping_sub(num);
                denom = fixp_abs(denom);

                num = f_mult(num, RELAXATION_FRACT);

                let num_shift = count_leading_bits(num) - 2;
                num = scale_value(num, num_shift);

                // CountLeadingBits(denom) of the fixp_abs() result as both reference builds
                // compile it: without the sign fold (arm64 `clz`, x86 `bsr` of denom itself), so
                // -1 for MINVAL_DBL, and the register shift then takes the amount modulo 32.
                let denom_shift = clb_of_abs(denom);
                denom = denom.wrapping_shl(denom_shift as u32);

                if num > 0 && denom != 0 {
                    let mut common_shift =
                        fix_min(num_shift - denom_shift + RELAXATION_SHIFT, DFRACT_BITS - 1);
                    if common_shift < 0 {
                        common_shift = -common_shift;
                        let tmp = schur_div_fast(num, denom, 16);
                        common_shift = fix_min(common_shift, count_leading_bits(tmp));
                        h.quota_matrix[ti] = tmp << common_shift;
                    } else {
                        h.quota_matrix[ti] = schur_div_fast(num, denom, 16) >> common_shift;
                    }
                } else {
                    h.quota_matrix[ti] = 0;
                }

                let sign = if ac.r11r != 0 {
                    if (ac.r01r >= 0 && ac.r11r >= 0) || (ac.r01r < 0 && ac.r11r < 0) {
                        1
                    } else {
                        -1
                    }
                } else {
                    1
                };

                let r2 = if sign < 0 {
                    r // (INT) pow(-1, band);
                } else {
                    r + 1 // (INT) pow(-1, band+1);
                };
                h.sign_matrix[ti] = 1 - 2 * (r2 & 0x1);
            }

            // C: (ac->r00r) >> fixMin(DFRACT_BITS - 1, (2 * qmfScale + autoCorrScaling +
            // SCALE_NRGVEC)); the amount is negative for a negative qmfScale (an input of
            // full-scale QMF values), which both builds shift modulo 32.
            let sh = fix_min(DFRACT_BITS - 1, 2 * qmf_scale + auto_corr_scaling + SCALE_NRGVEC);
            h.nrg_vector[time_index as usize] =
                h.nrg_vector[time_index as usize].wrapping_add(ac.r00r.wrapping_shr(sh as u32));
            // pNrgVectorFreq[r] finally has to be divided by noEstPerFrame, replaced division
            // by shifting with one
            h.nrg_vector_freq[r as usize] =
                h.nrg_vector_freq[r as usize].wrapping_add(ac.r00r.wrapping_shr(sh as u32));
        }
    }
}

/// `FDKsbrEnc_TonCorrParamExtr(hTonCorr, infVec, noiseLevels, missingHarmonicFlag,
/// missingHarmonicsIndex, envelopeCompensation, frameInfo, transientInfo, freqBandTable,
/// nSfb, xposType, sbrSyntaxFlags)`: the parameters required in the decoder to obtain the
/// correct tonal to noise ratio after SBR (inverse filtering levels, noise levels and
/// additional sines).
///
/// `missing_harmonics_index` is the full `addHarmonic[MAX_FREQ_COEFFS]` vector of the envelope
/// data and `env_data_tail` the image of the members that follow it (see [`crate::mh_det`]).
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_ton_corr_param_extr(
    h: &mut SbrTonCorrEst,
    inf_vec: &mut [InvfMode],
    noise_levels: &mut [FixpDbl],
    missing_harmonic_flag: &mut i32,
    missing_harmonics_index: &mut [u8],
    envelope_compensation: &mut [u8],
    frame_info: &SbrFrameInfo,
    transient_info: &[u8],
    freq_band_table: &[u8],
    n_sfb: i32,
    xpos_type: XposMode,
    sbr_syntax_flags: u32,
    env_data_tail: &[u8; ENV_DATA_TAIL_BYTES],
) {
    let transient_flag = transient_info[1] as i32; // Flag indicating if a transient is present in the current frame.
    let transient_pos = transient_info[0] as i32; // Position of the transient.

    // Determine if this is a frame where a transient starts...
    // The detection of noise-floor, missing harmonics and invf_est, is not in sync for the
    // non-buf-opt decoder such as AAC. Hence we need to keep track on the transient in the
    // present frame as well as in the next.
    let mut transient_frame = 0;
    if h.transient_next_frame != 0 {
        // The transient was detected in the previous frame, but is actually
        transient_frame = 1;
        h.transient_next_frame = 0;

        if transient_flag != 0
            && transient_pos + h.transient_pos_offset
                >= frame_info.borders[frame_info.n_envelopes as usize]
        {
            h.transient_next_frame = 1;
        }
    } else if transient_flag != 0 {
        if transient_pos + h.transient_pos_offset
            < frame_info.borders[frame_info.n_envelopes as usize]
        {
            transient_frame = 1;
            h.transient_next_frame = 0;
        } else {
            h.transient_next_frame = 1;
        }
    }
    let transient_frame_invf_est = transient_frame;

    // Estimate the required invese filtereing level.
    if h.switch_inverse_filt != 0 {
        fdk_sbr_enc_qmf_inverse_filtering_detector(
            &mut h.sbr_inv_filt,
            &h.quota_matrix,
            &h.nrg_vector,
            &h.index_vector,
            h.frame_start_index_invf_est,
            h.number_of_estimates_per_frame as i32 + h.frame_start_index_invf_est,
            transient_frame_invf_est,
            inf_vec,
        );
    }

    // Detect what tones will be missing.
    if xpos_type == XPOS_LC {
        fdk_sbr_enc_sbr_missing_harmonics_detector_qmf(
            &mut h.sbr_missing_harmonics_detector,
            &h.quota_matrix,
            &h.sign_matrix,
            &h.index_vector,
            frame_info,
            transient_info,
            missing_harmonic_flag,
            missing_harmonics_index,
            freq_band_table,
            n_sfb,
            envelope_compensation,
            &h.nrg_vector_freq,
            env_data_tail,
        );
    } else {
        *missing_harmonic_flag = 0;
        missing_harmonics_index[..n_sfb.max(0) as usize].fill(0);
    }

    // Noise floor estimation
    let inf_vec_ptr = h.sbr_inv_filt.prev_invf_mode;

    fdk_sbr_enc_sbr_noise_floor_estimate_qmf(
        &mut h.sbr_noise_floor_estimate,
        frame_info,
        noise_levels,
        &h.quota_matrix,
        &h.index_vector,
        *missing_harmonic_flag,
        h.frame_start_index,
        h.number_of_estimates_per_frame,
        transient_frame,
        &inf_vec_ptr,
        sbr_syntax_flags,
    );

    // Store the invfVec data for the next frame...
    h.sbr_inv_filt.prev_invf_mode[..(h.sbr_inv_filt.no_detector_bands.max(0) as usize)]
        .copy_from_slice(&inf_vec[..(h.sbr_inv_filt.no_detector_bands.max(0) as usize)]);
}

/// `findClosestEntry(goalSb, v_k_master, numMaster, direction)`: the closest match in the
/// master frequency table.
fn find_closest_entry(goal_sb: i32, v_k_master: &[u8], num_master: i32, direction: i32) -> i32 {
    if goal_sb <= v_k_master[0] as i32 {
        return v_k_master[0] as i32;
    }

    if goal_sb >= v_k_master[num_master as usize] as i32 {
        return v_k_master[num_master as usize] as i32;
    }

    let mut index;
    if direction != 0 {
        index = 0usize;
        while (v_k_master[index] as i32) < goal_sb {
            index += 1;
        }
    } else {
        index = num_master as usize;
        while v_k_master[index] as i32 > goal_sb {
            index -= 1;
        }
    }

    v_k_master[index] as i32
}

/// `resetPatch(hTonCorr, xposctrl, highBandStartSb, v_k_master, numMaster, fs, noChannels)`:
/// returns 0 if OK.
fn reset_patch(
    h: &mut SbrTonCorrEst,
    xposctrl: i32,
    high_band_start_sb: i32,
    v_k_master: &[u8],
    num_master: i32,
    fs: i32,
    no_channels: i32,
) -> i32 {
    let sb_guard = h.guard;
    let mut lsb = v_k_master[0] as i32; // Lowest subband related to the synthesis filterbank
    let usb = v_k_master[num_master as usize] as i32; // Stop subband related to the synthesis filterbank
    let mut xover_offset = high_band_start_sb - v_k_master[0] as i32; // Calculate distance in subbands between k0 and kx

    // Initialize the patching parameter
    if xposctrl == 1 {
        lsb += xover_offset;
        xover_offset = 0;
    }

    let mut goal_sb = (2 * no_channels * 16000 + (fs >> 1)) / fs; // 16 kHz band
    goal_sb = find_closest_entry(goal_sb, v_k_master, num_master, 1); // Adapt region to master-table

    // First patch
    let mut source_start_band = h.shift_start_sb + xover_offset;
    let mut target_stop_band = lsb + xover_offset;

    // even (odd) numbered channel must be patched to even (odd) numbered channel
    let mut patch: i32 = 0;
    while target_stop_band < usb {
        // To many patches
        if patch >= MAX_NUM_PATCHES as i32 {
            return 1; // Number of patches to high
        }

        let p = patch as usize;
        h.patch_param[p].guard_start_band = target_stop_band;
        target_stop_band += sb_guard;
        h.patch_param[p].target_start_band = target_stop_band;

        let mut num_bands_in_patch = goal_sb - target_stop_band; // get the desired range of the patch

        if num_bands_in_patch >= lsb - source_start_band {
            // desired number bands are not available -> patch whole source range
            let mut patch_distance = target_stop_band - source_start_band; // get the targetOffset
            patch_distance &= !1; // rounding off odd numbers and make all even
            num_bands_in_patch = lsb - (target_stop_band - patch_distance);
            num_bands_in_patch = find_closest_entry(
                target_stop_band + num_bands_in_patch,
                v_k_master,
                num_master,
                0,
            ) - target_stop_band; // Adapt region to master-table
        }

        // desired number bands are available -> get the minimal even patching distance
        let mut patch_distance = num_bands_in_patch + target_stop_band - lsb; // get minimal distance
        patch_distance = (patch_distance + 1) & !1; // rounding up odd numbers and make all even

        if num_bands_in_patch <= 0 {
            patch -= 1;
        } else {
            h.patch_param[p].source_start_band = target_stop_band - patch_distance;
            h.patch_param[p].target_band_offs = patch_distance;
            h.patch_param[p].num_bands_in_patch = num_bands_in_patch;
            h.patch_param[p].source_stop_band =
                h.patch_param[p].source_start_band + num_bands_in_patch;

            target_stop_band += h.patch_param[p].num_bands_in_patch;
        }

        // All patches but first
        source_start_band = h.shift_start_sb;

        // Check if we are close to goalSb
        if (target_stop_band - goal_sb).abs() < 3 {
            goal_sb = usb;
        }

        patch += 1;
    }

    patch -= 1;

    // if highest patch contains less than three subband: skip it
    if patch > 0 && h.patch_param[patch as usize].num_bands_in_patch < 3 {
        patch -= 1;
    }

    h.no_of_patches = patch + 1;

    // Assign the index-vector, so we know where to look for the high-band. -1 represents a
    // guard-band.
    for k in 0..h.patch_param[0].guard_start_band.max(0) as usize {
        h.index_vector[k] = k as i8;
    }

    for i in 0..h.no_of_patches as usize {
        let source_start = h.patch_param[i].source_start_band;
        let target_start = h.patch_param[i].target_start_band;
        let number_of_bands = h.patch_param[i].num_bands_in_patch;
        let start_guard_band = h.patch_param[i].guard_start_band;

        for k in 0..(target_start - start_guard_band).max(0) {
            h.index_vector[(start_guard_band + k) as usize] = -1;
        }

        for k in 0..number_of_bands.max(0) {
            h.index_vector[(target_start + k) as usize] = (source_start + k) as i8;
        }
    }

    0
}

/// `FDKsbrEnc_CreateTonCorrParamExtr(hTonCorr, chan)`: clears the module (its matrices are
/// owned). Returns 0.
pub fn fdk_sbr_enc_create_ton_corr_param_extr(h: &mut SbrTonCorrEst, chan: i32) -> i32 {
    *h = SbrTonCorrEst::default();
    if fdk_sbr_enc_create_sbr_missing_harmonics_detector(
        &mut h.sbr_missing_harmonics_detector,
        chan,
    ) != 0
    {
        return -1;
    }
    0
}

/// `FDKsbrEnc_InitTonCorrParamExtr(frameSize, hTonCorr, sbrCfg, timeSlots, xposCtrl,
/// ana_max_level, noiseBands, noiseFloorOffset, useSpeechConfig)`: returns 0 if OK.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_init_ton_corr_param_extr(
    frame_size: i32,
    h: &mut SbrTonCorrEst,
    sbr_cfg: &SbrConfigData,
    time_slots: i32,
    xpos_ctrl: i32,
    ana_max_level: i32,
    noise_bands: i32,
    noise_floor_offset: i32,
    use_speech_config: u32,
) -> i32 {
    let n_cols = sbr_cfg.no_qmf_slots;
    let fs = sbr_cfg.sample_freq;
    let no_qmf_channels = sbr_cfg.no_qmf_bands;

    let high_band_start_sb = sbr_cfg.freq_band_table_lo[0] as i32;
    let v_k_master = &sbr_cfg.v_k_master;
    let num_master = sbr_cfg.num_master;

    let n_sfb = sbr_cfg.n_sfb;

    // Reset the patching and allocate memory for the quota matrix. Assuming parameters for the
    // LPC analysis.
    if sbr_cfg.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        match time_slots {
            NUMBER_TIME_SLOTS_1920 => {
                h.lpc_length[0] = 8 - LPC_ORDER as i32;
                h.lpc_length[1] = 7 - LPC_ORDER as i32;
                h.number_of_estimates = NO_OF_ESTIMATES_LD;
                h.number_of_estimates_per_frame = 2; // sbrCfg->noQmfSlots / 7
                h.frame_start_index_invf_est = 0;
                h.transient_pos_offset = FRAME_MIDDLE_SLOT_512LD;
            }
            NUMBER_TIME_SLOTS_2048 => {
                h.lpc_length[0] = 8 - LPC_ORDER as i32;
                h.lpc_length[1] = 8 - LPC_ORDER as i32;
                h.number_of_estimates = NO_OF_ESTIMATES_LD;
                h.number_of_estimates_per_frame = 2; // sbrCfg->noQmfSlots / 8
                h.frame_start_index_invf_est = 0;
                h.transient_pos_offset = FRAME_MIDDLE_SLOT_512LD;
            }
            _ => {}
        }
    } else {
        match time_slots {
            NUMBER_TIME_SLOTS_2048 => {
                h.lpc_length[0] = 16 - LPC_ORDER as i32; // blockLength[0]
                h.lpc_length[1] = 16 - LPC_ORDER as i32; // blockLength[0]
                h.number_of_estimates = NO_OF_ESTIMATES_LC;
                h.number_of_estimates_per_frame = (sbr_cfg.no_qmf_slots / 16) as u32;
                h.frame_start_index_invf_est = 0;
                h.transient_pos_offset = FRAME_MIDDLE_SLOT_2048;
            }
            NUMBER_TIME_SLOTS_1920 => {
                h.lpc_length[0] = 15 - LPC_ORDER as i32; // blockLength[0]
                h.lpc_length[1] = 15 - LPC_ORDER as i32; // blockLength[0]
                h.number_of_estimates = NO_OF_ESTIMATES_LC;
                h.number_of_estimates_per_frame = (sbr_cfg.no_qmf_slots / 15) as u32;
                h.frame_start_index_invf_est = 0;
                h.transient_pos_offset = FRAME_MIDDLE_SLOT_1920;
            }
            _ => return -1,
        }
    }

    h.buffer_length = n_cols;
    h.step_size = h.lpc_length[0] + LPC_ORDER as i32; // stepSize[0] implicitly 0.

    h.next_sample = LPC_ORDER as i32; // firstSample

    h.move_ = h.number_of_estimates - h.number_of_estimates_per_frame as i32; // Number of estimates to move when buffering.
    if h.move_ < 0 {
        return -1;
    }
    h.start_index_matrix = h.number_of_estimates - h.number_of_estimates_per_frame as i32; // Where to store the latest estimations in the tonality Matrix.
    h.frame_start_index = 0; // Where in the tonality matrix the current frame (to be sent to the decoder) starts.
    h.prev_transient_flag = 0;
    h.transient_next_frame = 0;

    h.no_qmf_channels = no_qmf_channels;

    let nq = (no_qmf_channels.max(0) as usize).min(64);
    for i in 0..(h.number_of_estimates.max(0) as usize).min(MAX_NO_OF_ESTIMATES) {
        h.quota_matrix[i * 64..i * 64 + nq].fill(0);
        h.sign_matrix[i * 64..i * 64 + nq].fill(0);
    }

    // Reset the patch.
    h.guard = 0;
    h.shift_start_sb = 1;

    if reset_patch(h, xpos_ctrl, high_band_start_sb, v_k_master, num_master, fs, no_qmf_channels)
        != 0
    {
        return 1;
    }

    if fdk_sbr_enc_init_sbr_noise_floor_estimate(
        &mut h.sbr_noise_floor_estimate,
        ana_max_level,
        &sbr_cfg.freq_band_table_lo,
        n_sfb[LO],
        noise_bands,
        noise_floor_offset,
        time_slots,
        use_speech_config,
    ) != 0
    {
        return 1;
    }

    let fbt = h.sbr_noise_floor_estimate.freq_band_table_qmf;
    if fdk_sbr_enc_init_inv_filt_detector(
        &mut h.sbr_inv_filt,
        &fbt,
        h.sbr_noise_floor_estimate.no_noise_bands,
        use_speech_config,
    ) != 0
    {
        return 1;
    }

    if fdk_sbr_enc_init_sbr_missing_harmonics_detector(
        &mut h.sbr_missing_harmonics_detector,
        fs,
        frame_size,
        n_sfb[HI],
        no_qmf_channels,
        h.number_of_estimates,
        h.move_,
        h.number_of_estimates_per_frame as i32,
        sbr_cfg.sbr_syntax_flags,
    ) != 0
    {
        return 1;
    }

    0
}

/// `FDKsbrEnc_ResetTonCorrParamExtr(hTonCorr, xposctrl, highBandStartSb, v_k_master,
/// numMaster, fs, freqBandTable, nSfb, noQmfChannels)`: returns 0 if OK. `freq_band_table_lo`
/// is `freqBandTable[LO]`.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_reset_ton_corr_param_extr(
    h: &mut SbrTonCorrEst,
    xposctrl: i32,
    high_band_start_sb: i32,
    v_k_master: &[u8],
    num_master: i32,
    fs: i32,
    freq_band_table_lo: &[u8],
    n_sfb: &[i32; 2],
    no_qmf_channels: i32,
) -> i32 {
    // Reset the patch.
    h.guard = 0;
    h.shift_start_sb = 1;

    if reset_patch(h, xposctrl, high_band_start_sb, v_k_master, num_master, fs, no_qmf_channels)
        != 0
    {
        return 1;
    }

    // Reset the noise floor estimate.
    if fdk_sbr_enc_reset_sbr_noise_floor_estimate(
        &mut h.sbr_noise_floor_estimate,
        freq_band_table_lo,
        n_sfb[LO],
    ) != 0
    {
        return 1;
    }

    // Reset the inveerse filtereing detector.
    let fbt = h.sbr_noise_floor_estimate.freq_band_table_qmf;
    if fdk_sbr_enc_reset_inv_filt_detector(
        &mut h.sbr_inv_filt,
        &fbt,
        h.sbr_noise_floor_estimate.no_noise_bands,
    ) != 0
    {
        return 1;
    }
    // Reset the missing harmonics detector.
    if fdk_sbr_enc_reset_sbr_missing_harmonics_detector(
        &mut h.sbr_missing_harmonics_detector,
        n_sfb[HI],
    ) != 0
    {
        return 1;
    }

    0
}

/// `FDKsbrEnc_DeleteTonCorrParamExtr(hTonCorr)`: nothing to free (owned memory).
pub fn fdk_sbr_enc_delete_ton_corr_param_extr(_h: &mut SbrTonCorrEst) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tonality quotas with the lanes against the straight function (whole state compared):
    /// the four configurations of the encoder, random `usb` and `qmfScale`, QMF values of every
    /// magnitude, full scale (`MINVAL_DBL`, `MAXVAL_DBL`) and zero runs, random previous state.
    #[test]
    fn calculate_tonality_quotas_equals_straight_port() {
        let mut seed = 0x70a1_17e5_0123_4567u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        // (lpcLength, estimates, estimates per frame, slots)
        let configs: [([i32; 2], i32, u32, i32); 5] = [
            ([6, 5], NO_OF_ESTIMATES_LD, 2, 15),
            ([6, 6], NO_OF_ESTIMATES_LD, 2, 16),
            ([14, 14], NO_OF_ESTIMATES_LC, 2, 32),
            ([13, 13], NO_OF_ESTIMATES_LC, 2, 30),
            ([6, 6], NO_OF_ESTIMATES_LD, 1, 8),
        ];
        for k in 0..3000 {
            let (lpc, est, per_frame, slots) = configs[k % configs.len()];
            let mut h = SbrTonCorrEst {
                no_qmf_channels: 64,
                buffer_length: slots,
                step_size: lpc[0] + LPC_ORDER as i32,
                number_of_estimates: est,
                number_of_estimates_per_frame: per_frame,
                lpc_length: lpc,
                next_sample: LPC_ORDER as i32,
                move_: est - per_frame as i32,
                start_index_matrix: est - per_frame as i32,
                ..SbrTonCorrEst::default()
            };
            for x in h.quota_matrix.iter_mut() {
                *x = next() as i32;
            }
            for x in h.sign_matrix.iter_mut() {
                *x = next() as i32 % 3;
            }
            for x in h.nrg_vector.iter_mut() {
                *x = next() as i32;
            }
            let mode = next() % 6;
            let mut v = || match mode {
                0 => [i32::MIN, i32::MAX, 0, -1, 1][(next() % 5) as usize],
                1 => next() as i32,
                2 => 0,
                _ => (next() as i32) >> (next() % 32),
            };
            let re: Vec<Vec<i32>> = (0..32).map(|_| (0..64).map(|_| v()).collect()).collect();
            let im: Vec<Vec<i32>> = (0..32).map(|_| (0..64).map(|_| v()).collect()).collect();
            let rr: Vec<&[i32]> = re.iter().map(|r| &r[..]).collect();
            let ii: Vec<&[i32]> = im.iter().map(|r| &r[..]).collect();
            let usb = if k % 4 == 0 { 64 } else { (next() % 65) as i32 };
            let qmf_scale = (next() % 24) as i32 - 2;
            let mut h2 = h;
            fdk_sbr_enc_calculate_tonality_quotas(&mut h, &rr, &ii, usb, qmf_scale);
            fdk_sbr_enc_calculate_tonality_quotas_straight(&mut h2, &rr, &ii, usb, qmf_scale);
            assert!(
                h == h2,
                "CalculateTonalityQuotas config {} usb {usb} qmfScale {qmf_scale}",
                k % 5
            );
        }
    }
}
