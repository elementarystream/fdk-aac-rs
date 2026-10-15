// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/sbr_dec.h, libSBRdec/src/sbr_dec.cpp.

//! Port of `sbr_dec.h` and `sbr_dec.cpp`: the SBR decoder core of one channel (QMF analysis,
//! transposition, envelope adjustment, parametric stereo and QMF synthesis).
//!
//! # QMF data
//!
//! `qmfDomainInCh`/`qmfDomainOutCh` are indices of the input/output channels of the QMF domain,
//! which every function takes explicitly (`&mut FdkQmfDomain`). The pointer arrays
//! `pLowBandReal`/`pLowBandImag` (`FIXP_DBL *[]`) are vectors of slot slices: the slots of the
//! QMF domain (64 values each, see `QmfDomainMem::slots_mut()`) followed, with the harmonic
//! transposer, by the rows of the HBE buffers. A `NULL` imaginary slot array (low power) is an
//! empty vector.
//!
//! The matrices `hQmfHBESlotsReal`/`Imag` and `codecQMFBufferReal`/`Imag`
//! (`fdkCallocMatrix2D_aligned(noCols, noChannels)`: one contiguous block of rows) are flat
//! vectors with the row length [`SbrDec::hbe_no_channels`]; copies with more values than a row
//! continue in the next row as in C. `tmp_memory` is the same buffer as `hQmfHBESlotsReal`.
//!
//! # Memory
//!
//! `createSbrDec()` allocates the HBE buffers and the transposer again on every call with the
//! harmonic transposer; the port replaces them by new zeroed ones, which is what the reference
//! uses afterwards.
//!
//! `SBRDEC_MAX_HB_FADE_FRAMES` is not defined in the reference (the high band fade-in code and
//! `highBandFadeCnt` are compiled out).

use crate::env_calc::*;
use crate::env_extr::*;
use crate::hbe::*;
use crate::lpp_tran::*;
use crate::psdec::*;
use crate::pvc_dec::*;
use crate::sbrdec_drc::*;
use crate::sbrdecoder::{SbrError, SBRDEC_MEM_ALLOC_FAILED, SBRDEC_OK};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_domain::*;

/// `SBRDEC_QMF_SKIP`
pub type SbrdecQmfSkip = i32;
/// `qmfSkipNothing`
pub const QMF_SKIP_NOTHING: SbrdecQmfSkip = 0;
/// `qmfSkipAnalysis`
pub const QMF_SKIP_ANALYSIS: SbrdecQmfSkip = 1 << 0;
/// `qmfSkipSynthesis`
pub const QMF_SKIP_SYNTHESIS: SbrdecQmfSkip = 1 << 1;

/// Number of values of a QMF slot (`(64)`).
const SLOT_LEN: usize = 64;

/// `SBR_DEC` (`HANDLE_SBR_DEC` is `&mut SbrDec`).
#[derive(Clone, Debug)]
pub struct SbrDec {
    /// `SbrCalculateEnvelope`
    pub sbr_calculate_envelope: SbrCalculateEnvelope,
    /// `LppTrans` (`LppTrans.pSettings` is the transposer settings of the element, passed
    /// separately)
    pub lpp_trans: SbrLppTrans,
    /// `PvcStaticData`
    pub pvc_static_data: PvcStaticData,
    /// `scale_ov`
    pub scale_ov: i16,
    /// `scale_lb`
    pub scale_lb: i16,
    /// `scale_hbe`
    pub scale_hbe: i16,
    /// `prev_frame_lSbr`
    pub prev_frame_l_sbr: i16,
    /// `prev_frame_hbeSbr`
    pub prev_frame_hbe_sbr: i16,
    /// `codecFrameSize`
    pub codec_frame_size: i32,
    /// `hHBE`
    pub h_hbe: Option<Box<HbeTransposer>>,
    /// `qmfDomainInCh`: index of the QMF domain input channel (`None` for `NULL`)
    pub qmf_domain_in_ch: Option<usize>,
    /// `qmfDomainOutCh`: index of the QMF domain output channel (`None` for `NULL`)
    pub qmf_domain_out_ch: Option<usize>,
    /// `sbrDrcChannel`
    pub sbr_drc_channel: SbrdecDrcChannel,
    /// `hQmfHBESlotsReal` (and `tmp_memory`): `noCols` rows of `hbe_no_channels` values
    pub hbe_slots_real: Vec<FixpDbl>,
    /// `hQmfHBESlotsImag`
    pub hbe_slots_imag: Vec<FixpDbl>,
    /// `codecQMFBufferReal`
    pub codec_qmf_buffer_real: Vec<FixpDbl>,
    /// `codecQMFBufferImag`
    pub codec_qmf_buffer_imag: Vec<FixpDbl>,
    /// Row length of the HBE buffers (`noChannels` of `createSbrDec()`).
    pub hbe_no_channels: usize,
    /// `savedStates`
    pub saved_states: u8,
    /// `applySbrProc_old`
    pub apply_sbr_proc_old: i32,
}

impl Default for SbrDec {
    /// The zeroed structure of `GetRam_SbrDecChannel()` (`FDKcalloc`).
    fn default() -> Self {
        SbrDec {
            sbr_calculate_envelope: SbrCalculateEnvelope::ZERO,
            lpp_trans: SbrLppTrans::ZERO,
            pvc_static_data: PvcStaticData::ZERO,
            scale_ov: 0,
            scale_lb: 0,
            scale_hbe: 0,
            prev_frame_l_sbr: 0,
            prev_frame_hbe_sbr: 0,
            codec_frame_size: 0,
            h_hbe: None,
            qmf_domain_in_ch: None,
            qmf_domain_out_ch: None,
            sbr_drc_channel: SbrdecDrcChannel::ZERO,
            hbe_slots_real: Vec::new(),
            hbe_slots_imag: Vec::new(),
            codec_qmf_buffer_real: Vec::new(),
            codec_qmf_buffer_imag: Vec::new(),
            hbe_no_channels: 0,
            saved_states: 0,
            apply_sbr_proc_old: 0,
        }
    }
}

/// `SBR_CHANNEL`
#[derive(Clone, Debug)]
pub struct SbrChannel {
    /// `frameData[(1) + 1]`
    pub frame_data: [SbrFrameData; 1 + 1],
    /// `prevFrameData`
    pub prev_frame_data: SbrPrevFrameData,
    /// `SbrDec`
    pub sbr_dec: SbrDec,
}

impl Default for SbrChannel {
    /// The zeroed structure of `GetRam_SbrDecChannel()` (`FDKcalloc`).
    fn default() -> Self {
        SbrChannel {
            frame_data: [SbrFrameData::ZERO; 2],
            prev_frame_data: SbrPrevFrameData::ZERO,
            sbr_dec: SbrDec::default(),
        }
    }
}

/// The QMF slot pointers of an input channel: `hQmfSlotsReal[0..n]` and `hQmfSlotsImag[0..n]`
/// (`NULL` for missing ones; the arrays hold at most `QMF_SLOTS_SIZE` pointers, the slots from
/// there on are `NULL`).
struct SlotPtrs {
    re: [QmfSlotPtr; QMF_SLOTS_SIZE],
    im: [QmfSlotPtr; QMF_SLOTS_SIZE],
    n: usize,
}

/// The [`SlotPtrs`] of an input channel for `n` slots.
fn domain_slot_ptrs(qd_in: &FdkQmfDomainIn, n: usize) -> SlotPtrs {
    let get = |a: &Option<Box<[QmfSlotPtr]>>, i: usize| {
        a.as_deref().and_then(|a| a.get(i).copied()).unwrap_or_default()
    };
    let mut p = SlotPtrs {
        re: [QmfSlotPtr::Null; QMF_SLOTS_SIZE],
        im: [QmfSlotPtr::Null; QMF_SLOTS_SIZE],
        n,
    };
    for i in 0..n.min(QMF_SLOTS_SIZE) {
        p.re[i] = get(&qd_in.h_qmf_slots_real, i);
        p.im[i] = get(&qd_in.h_qmf_slots_imag, i);
    }
    p
}

/// The slot slices of the pointers of [`domain_slot_ptrs`] appended to `re` and `im`, without
/// heap allocation (`QmfDomainMem::channel_slots_into`).
fn domain_slots<'a>(
    mem: &'a mut QmfDomainMem,
    ptrs: &SlotPtrs,
    re: &mut SlotList<'a>,
    im: &mut SlotList<'a>,
) {
    let k = ptrs.n.min(QMF_SLOTS_SIZE);
    mem.channel_slots_into(&ptrs.re[..k], &ptrs.im[..k], SLOT_LEN, re, im);
    for _ in k..ptrs.n {
        re.push(&mut []);
        im.push(&mut []);
    }
}

/// `FDKmemcpy(dst, src, n * sizeof(FIXP_DBL))` limited to the ends of both.
#[inline]
fn copy_vals(dst: &mut [FixpDbl], src: &[FixpDbl], n: usize) {
    let n = n.min(dst.len()).min(src.len());
    dst[..n].copy_from_slice(&src[..n]);
}

/// Row `row` of a flat matrix with row length `nch`, to the end of the buffer (copies with
/// more than `nch` values continue in the next row as in C).
#[inline]
fn flat_row(buf: &[FixpDbl], row: i32, nch: usize) -> &[FixpDbl] {
    if row < 0 {
        return &[];
    }
    buf.get(row as usize * nch..).unwrap_or(&[])
}

/// Mutable variant of [`flat_row`].
#[inline]
fn flat_row_mut(buf: &mut [FixpDbl], row: i32, nch: usize) -> &mut [FixpDbl] {
    if row < 0 {
        return &mut [];
    }
    buf.get_mut(row as usize * nch..).unwrap_or(&mut [])
}

/// `copyHarmonicSpectrum()`: copies the harmonic spectrum of the second patch of the 4:1
/// transposer into the higher patches.
pub fn copy_harmonic_spectrum(
    x_over_qmf: &[i32; MAX_NUM_PATCHES_HBE],
    qmf_real: &mut [&mut [FixpDbl]],
    qmf_imag: &mut [&mut [FixpDbl]],
    no_cols: i32,
    overlap: i32,
    keep_states_synced: KeepStatesSyncedMode,
) {
    let mut num_patches = 0;
    let mut slot_offset = 0;
    let mut base = overlap; // ppqmfReal = qmfReal + overlap

    if keep_states_synced == KEEP_STATES_SYNCED_NORMAL {
        slot_offset = no_cols - overlap - LPC_ORDER as i32;
    }

    if keep_states_synced == KEEP_STATES_SYNCED_OUTDIFF {
        base = 0;
    }

    for i in 1..MAX_NUM_PATCHES {
        if x_over_qmf[i] != 0 {
            num_patches += 1;
        }
    }

    // Reads and writes outside of the slots read 0 and are skipped.
    fn copy_band(slots: &mut [&mut [FixpDbl]], col: i32, dst: i32, src: i32) {
        if col < 0 || dst < 0 {
            return;
        }
        let Some(s) = slots.get_mut(col as usize) else {
            return;
        };
        let v = if src >= 0 { s.get(src as usize).copied().unwrap_or(0) } else { 0 };
        if let Some(d) = s.get_mut(dst as usize) {
            *d = v;
        }
    }

    for patch in (MAX_STRETCH_HBE - 1)..num_patches as usize {
        let mut patch_bands = x_over_qmf[patch + 1] - x_over_qmf[patch];
        let mut target = x_over_qmf[patch];
        let source_bands = x_over_qmf[MAX_STRETCH_HBE - 1] - x_over_qmf[MAX_STRETCH_HBE - 2];
        while patch_bands > 0 {
            let mut num_bands = source_bands;
            let mut start_band = x_over_qmf[MAX_STRETCH_HBE - 1] - 1;
            if target + num_bands >= x_over_qmf[patch + 1] {
                num_bands = x_over_qmf[patch + 1] - target;
            }
            if (((target + num_bands - 1) % 2) + ((x_over_qmf[MAX_STRETCH_HBE - 1] - 1) % 2)) % 2
                != 0
            {
                if num_bands == source_bands {
                    num_bands -= 1;
                } else {
                    start_band -= 1;
                }
            }
            let (col_start, col_end) = if keep_states_synced == KEEP_STATES_SYNCED_OUTDIFF {
                (slot_offset, overlap + LPC_ORDER as i32)
            } else {
                (slot_offset, no_cols)
            };
            for col in col_start..col_end {
                let mut i = 0;
                let mut band = num_bands;
                while band > 0 {
                    if (target + band - 1 < 64) && (target + band - 1 < x_over_qmf[patch + 1]) {
                        copy_band(qmf_real, base + col, target + band - 1, start_band - i);
                        copy_band(qmf_imag, base + col, target + band - 1, start_band - i);
                        i += 1;
                    }
                    band -= 1;
                }
            }
            // A patch without progress (numBands <= 0): the port stops.
            if num_bands <= 0 {
                break;
            }
            target += num_bands;
            patch_bands -= num_bands;
        }
    }
}

/// Two distinct elements of a slice, mutably.
fn pair_mut<T>(s: &mut [T], a: usize, b: usize) -> Option<(&mut T, &mut T)> {
    if a == b || a >= s.len() || b >= s.len() {
        return None;
    }
    if a < b {
        let (x, y) = s.split_at_mut(b);
        Some((&mut x[a], &mut y[0]))
    } else {
        let (x, y) = s.split_at_mut(a);
        Some((&mut y[0], &mut x[b]))
    }
}

/// The number of output values that the synthesis writes from `off` on (`no_col` slots of
/// `no_channels` values with the stride `stride`).
fn out_len(off: usize, no_col: i32, no_channels: i32, stride: i32) -> usize {
    let n = (no_col.max(0) as usize) * (no_channels.max(0) as usize);
    if n == 0 {
        0
    } else {
        off + (n - 1) * (stride.max(1) as usize) + 1
    }
}

/// `sbr_dec()`: the SBR decoder core function for one channel.
///
/// `time_in` is `timeIn` (the low band time signal of this channel, stride 1); `time_out` is
/// the whole output buffer and `out_off`/`out_right_off` are the offsets of `timeOut` and
/// `timeOutRight` in it. `h_sbr_dec_right` is set with parametric stereo only. `p_coef` is the
/// PS scratch memory of the caller (`specificTo.mpeg.pCoef`), `p_settings` the transposer
/// settings of the element (`LppTrans.pSettings`).
///
/// Deviations: an input shorter than one frame is read as zero beyond its end, an output buffer
/// shorter than one frame is not written beyond its end; a channel without QMF domain channel
/// is not processed.
#[allow(clippy::too_many_arguments)]
pub fn sbr_dec(
    h_sbr_dec: &mut SbrDec,
    qd: &mut FdkQmfDomain,
    time_in: &[i32],
    time_out: &mut [i32],
    out_off: usize,
    h_sbr_dec_right: Option<&mut SbrDec>,
    out_right_off: usize,
    stride_out: i32,
    h_header_data: &mut SbrHeaderData,
    h_frame_data: &SbrFrameData,
    h_prev_frame_data: &mut SbrPrevFrameData,
    apply_processing: i32,
    mut h_ps_d: Option<&mut PsDec>,
    p_coef: &mut PsDecCoefficients,
    p_settings: &TransposerSettings,
    flags: u32,
    _codec_frame_size: i32,
    sbr_in_data_headroom: i32,
) {
    let (Some(ch_in), Some(ch_out)) = (h_sbr_dec.qmf_domain_in_ch, h_sbr_dec.qmf_domain_out_ch)
    else {
        return;
    };
    if ch_in >= QMF_DOMAIN_MAX_CHANNELS || ch_out >= QMF_DOMAIN_MAX_CHANNELS {
        return;
    }
    let use_lp = flags & SBRDEC_LOW_POWER != 0;
    let harmonic = flags & SBRDEC_USAC_HARMONICSBR != 0;

    // Number of QMF timeslots in the overlap buffer:
    let ov_len = p_settings.overlap as usize;
    // Number of QMF slots per frame
    let no_cols = h_header_data.number_time_slots as usize * h_header_data.time_step as usize;
    let n_dom = no_cols + ov_len;

    let SbrDec {
        sbr_calculate_envelope,
        lpp_trans,
        pvc_static_data,
        scale_ov,
        scale_lb,
        scale_hbe,
        prev_frame_l_sbr,
        prev_frame_hbe_sbr,
        h_hbe,
        sbr_drc_channel,
        hbe_slots_real,
        hbe_slots_imag,
        codec_qmf_buffer_real,
        codec_qmf_buffer_imag,
        hbe_no_channels,
        saved_states,
        apply_sbr_proc_old,
        ..
    } = h_sbr_dec;
    let hbe_nch = (*hbe_no_channels).max(1);

    // set pReal to where QMF analysis writes in case of legacy SBR
    let mut p_off = ov_len;

    if harmonic {
        // shift scale values according to buffer
        *scale_ov = *scale_lb;
        *scale_lb = *scale_hbe;

        // set pReal to where QMF analysis writes in case of HBE
        p_off += no_cols;
        if flags & SBRDEC_SKIP_QMF_ANA != 0 {
            // stereoCfgIndex3 with HBE
            let mut rr: Vec<&mut [FixpDbl]> = hbe_slots_real.chunks_mut(hbe_nch).collect();
            let mut ri: Vec<&mut [FixpDbl]> = hbe_slots_imag.chunks_mut(hbe_nch).collect();
            fdk_qmf_domain_qmf_data2_hbe(qd, ch_in, &mut rr, &mut ri);
        }
    }

    // low band codec signal subband filtering
    if flags & SBRDEC_SKIP_QMF_ANA != 0 && !harmonic {
        // stereoCfgIndex3 w/o HBE
        fdk_qmf_domain_work_buffer2_proc_channel(qd, ch_in);
    }

    let n_bands_synthesis = qd.global_conf.n_bands_synthesis as usize;
    let ptrs = domain_slot_ptrs(&qd.qmf_domain_in[ch_in], n_dom);
    let FdkQmfDomain { qmf_domain_in, qmf_domain_out, mem, .. } = qd;
    let dom_in = &mut qmf_domain_in[ch_in];

    // map QMF buffer to pointer array (Overlap + Frame)
    let (mut low_re, mut low_im) = (SlotList::new(), SlotList::new());
    domain_slots(mem, &ptrs, &mut low_re, &mut low_im);

    if harmonic {
        if flags & SBRDEC_SKIP_QMF_ANA == 0 {
            // We have to move old hbe frame data to lb area of buffer
            let nab = h_header_data.number_of_analysis_bands as usize;
            for i in 0..no_cols {
                copy_vals(low_re[ov_len + i], flat_row(hbe_slots_real, i as i32, hbe_nch), nab);
                copy_vals(low_im[ov_len + i], flat_row(hbe_slots_imag, i as i32, hbe_nch), nab);
            }
        }
        // in case of harmonic SBR and no HBE_LP map additional buffer for one more frame to
        // pointer array
        low_re.extend(hbe_slots_real.chunks_mut(hbe_nch).take(no_cols));
        low_im.extend(hbe_slots_imag.chunks_mut(hbe_nch).take(no_cols));
        // (fewer rows only without HBE buffers)
        while low_re.len() < n_dom + no_cols {
            low_re.push(&mut []);
        }
        while low_im.len() < n_dom + no_cols {
            low_im.push(&mut []);
        }
    }

    if flags & SBRDEC_SKIP_QMF_ANA == 0 {
        // qmfTemp: C_AALLOC_SCRATCH_START(qmfTemp, FIXP_DBL, 2 * (64)), written before read
        let mut qmf_temp = [0 as FixpDbl; 2 * 64];
        let needed = dom_in.fb.no_col.max(0) as usize * dom_in.fb.no_channels.max(0) as usize;
        let mut pad: Vec<i32>;
        let t_in: &[i32] = if time_in.len() < needed {
            pad = vec![0; needed];
            pad[..time_in.len()].copy_from_slice(time_in);
            &pad
        } else {
            time_in
        };
        let states = dom_in.p_ana_qmf_states.as_deref_mut().unwrap_or(&mut []);
        qmf_analysis_filtering::<i32, _>(
            &dom_in.fb,
            states,
            &mut low_re[p_off..],
            &mut low_im[p_off..],
            &mut dom_in.scaling,
            t_in,
            sbr_in_data_headroom,
            1,
            &mut qmf_temp,
        );
        pad = Vec::new();
        drop(pad);
    }

    // Clear upper half of spectrum
    if !(harmonic && h_frame_data.sbr_patching_mode == 0) {
        let n_analysis_bands = h_header_data.number_of_analysis_bands as usize;
        for slot in ov_len..no_cols + ov_len {
            if let Some(s) = low_re[slot].get_mut(n_analysis_bands..SLOT_LEN) {
                s.fill(0);
            }
            if !use_lp {
                if let Some(s) = low_im[slot].get_mut(n_analysis_bands..SLOT_LEN) {
                    s.fill(0);
                }
            }
        }
    }

    // Shift spectral data left to gain accuracy in transposer and adjustor.
    // Range was increased from lsb to no_channels because in some cases (e.g. USAC conf
    // eSbr_4_Pvc.mp4 and some HBE cases) it could be observed that the signal between lsb and
    // no_channels is used for the patching process.
    let max_val = max_subband_sample(
        &low_re[p_off..],
        if use_lp { None } else { Some(&low_im[p_off..]) },
        0,
        dom_in.fb.no_channels,
        0,
        no_cols as i32,
    );

    let mut reserve = fix_max(0, cnt_leading_zeros(max_val) - 1);
    reserve = fix_min(reserve, DFRACT_BITS - 1 - dom_in.scaling.lb_scale);

    // If all data is zero, lb_scale could become too large
    rescale_subband_samples(
        &mut low_re[p_off..],
        if use_lp { None } else { Some(&mut low_im[p_off..]) },
        0,
        dom_in.fb.no_channels,
        0,
        no_cols as i32,
        reserve,
    );

    dom_in.scaling.lb_scale += reserve;
    if harmonic {
        // actually this is our hbe_scale
        *scale_hbe = dom_in.scaling.lb_scale as i16;
        // the real lb_scale is stored in scale_lb from sbr
        dom_in.scaling.lb_scale = *scale_lb as i32;
    }

    // save low band scale, wavecoding or parametric stereo may modify it
    let save_lb_scale = dom_in.scaling.lb_scale;

    if apply_processing != 0 {
        let borders = h_frame_data.frame_info.borders;
        let n_env = (h_frame_data.frame_info.n_envelopes as usize).min(MAX_ENVELOPES);
        let last_slot_offs = borders[n_env] as i32 - h_header_data.number_time_slots as i32;
        // DEVIATION: not initialized in the reference (sbr_dec() only clears
        // lowSubband..highSubband in low power mode); for the other values the reference has no
        // defined result. The port reads 0.
        let mut degree_alias = [0 as FixpDbl; 64];
        let mut pvc_dynamic_data = PvcDynamicData::default();

        pvc_init_frame(
            pvc_static_data,
            &mut pvc_dynamic_data,
            if h_header_data.frame_error_flag != 0 { 0 } else { h_header_data.bs_info.pvc_mode },
            h_frame_data.ns,
            h_header_data.time_step as i32,
            h_header_data.freq_band_data.low_subband as i32,
            h_frame_data.frame_info.pvc_borders[0] as i32,
            &h_frame_data.pvc_id,
        );

        if h_header_data.frame_error_flag == 0 && h_header_data.bs_info.pvc_mode > 0 {
            pvc_decode_frame(
                pvc_static_data,
                &mut pvc_dynamic_data,
                &low_re,
                &low_im,
                ov_len as i32,
                15 - dom_in.scaling.ov_lb_scale, // SCALE2EXP
                15 - dom_in.scaling.lb_scale,    // SCALE2EXP
            );
        }
        pvc_end_frame(pvc_static_data, &mut pvc_dynamic_data);

        // The transposer will override most values in degreeAlias[]. The array needs to be
        // cleared at least from lowSubband to highSubband before.
        if use_lp {
            let lo = h_header_data.freq_band_data.low_subband as usize;
            let hi = h_header_data.freq_band_data.high_subband as usize;
            if let Some(s) = degree_alias.get_mut(lo..hi.max(lo).min(64)) {
                s.fill(0);
            }
        }

        // Inverse filtering of lowband and transposition into the SBR-frequency range
        {
            let keep_states_synced_mode = if harmonic && h_frame_data.sbr_patching_mode != 0 {
                KEEP_STATES_SYNCED_NORMAL
            } else {
                KEEP_STATES_SYNCED_OFF
            };

            if harmonic {
                let mut codec_off = p_off;
                if flags & SBRDEC_QUAD_RATE != 0 {
                    codec_off = codec_off.wrapping_sub(32);
                }

                let (hbe_cols, hbe_ch) = match h_hbe.as_deref() {
                    Some(h) => (h.no_cols, h.no_channels.max(0) as usize),
                    None => (0, 0),
                };

                if *saved_states == 0 && h_frame_data.sbr_patching_mode == 1 {
                    // copy saved states from previous frame to legacy SBR lpc filterstate buffer
                    for i in 0..LPC_ORDER + ov_len {
                        let row = no_cols as i32 - LPC_ORDER as i32 - ov_len as i32 + i as i32;
                        copy_vals(
                            lpp_trans.leg_re(i),
                            flat_row(codec_qmf_buffer_real, row, hbe_nch),
                            hbe_ch,
                        );
                        copy_vals(
                            lpp_trans.leg_im(i),
                            flat_row(codec_qmf_buffer_imag, row, hbe_nch),
                            hbe_ch,
                        );
                    }
                }

                // saving unmodified QMF states in case we are switching from legacy SBR to HBE
                for i in 0..hbe_cols.max(0) as usize {
                    if let Some(src) = low_re.get(ov_len + i) {
                        copy_vals(
                            flat_row_mut(codec_qmf_buffer_real, i as i32, hbe_nch),
                            src,
                            hbe_ch,
                        );
                    }
                    if let Some(src) = low_im.get(ov_len + i) {
                        copy_vals(
                            flat_row_mut(codec_qmf_buffer_imag, i as i32, hbe_nch),
                            src,
                            hbe_ch,
                        );
                    }
                }

                if let Some(h) = h_hbe.as_deref_mut() {
                    qmf_transposer_apply(
                        h,
                        &mut low_re,
                        &mut low_im,
                        codec_off,
                        no_cols as i32,
                        Some(lpp_trans.hbe_states_mut()),
                        h_frame_data.sbr_pitch_in_bins as i32,
                        *scale_lb as i32,
                        *scale_hbe as i32,
                        &mut dom_in.scaling.hb_scale,
                        h_header_data.time_step as i32,
                        borders[0] as i32,
                        ov_len as i32,
                        keep_states_synced_mode,
                    );

                    if flags & SBRDEC_QUAD_RATE != 0 {
                        let x_over_qmf = h.x_over_qmf;
                        copy_harmonic_spectrum(
                            &x_over_qmf,
                            &mut low_re,
                            &mut low_im,
                            no_cols as i32,
                            ov_len as i32,
                            keep_states_synced_mode,
                        );
                    }
                }
            }
        }

        if harmonic && h_frame_data.sbr_patching_mode == 0 {
            *prev_frame_l_sbr = 0;
            *prev_frame_hbe_sbr = 1;

            if let Some(h) = h_hbe.as_deref() {
                lpp_transposer_hbe(
                    lpp_trans,
                    p_settings,
                    h,
                    &mut dom_in.scaling,
                    &mut low_re,
                    &mut low_im,
                    h_header_data.time_step as i32,
                    borders[0] as i32,
                    last_slot_offs,
                    h_header_data.freq_band_data.n_invf_bands as i32,
                    &h_frame_data.sbr_invf_mode,
                    &h_prev_frame_data.sbr_invf_mode,
                );
            }
        } else {
            if harmonic {
                let hbe_cols = h_hbe.as_deref().map_or(0, |h| h.no_cols);
                for i in 0..LPC_ORDER + p_settings.overlap as usize {
                    // Store the unmodified qmf Slots values for upper part of spectrum (required
                    // for LPC filtering) required if next frame is a HBE frame
                    let idx = hbe_cols - LPC_ORDER as i32 + i as i32;
                    let (re, im) = if idx >= 0 {
                        (
                            low_re.get(idx as usize).map(|s| &**s).unwrap_or(&[]),
                            low_im.get(idx as usize).map(|s| &**s).unwrap_or(&[]),
                        )
                    } else {
                        (&[][..], &[][..])
                    };
                    copy_vals(lpp_trans.hbe_re(i), re, SLOT_LEN);
                    copy_vals(lpp_trans.hbe_im(i), im, SLOT_LEN);
                }
            }
            {
                *prev_frame_l_sbr = 1;
                *prev_frame_hbe_sbr = 0;
            }

            lpp_transposer(
                lpp_trans,
                p_settings,
                &mut dom_in.scaling,
                &mut low_re,
                &mut degree_alias, // only used if useLP = 1
                &mut low_im,
                use_lp,
                h_header_data.bs_info.sbr_preprocessing as i32,
                h_header_data.freq_band_data.v_k_master[0] as i32,
                h_header_data.time_step as i32,
                borders[0] as i32,
                last_slot_offs,
                h_header_data.freq_band_data.n_invf_bands as i32,
                &h_frame_data.sbr_invf_mode,
                &h_prev_frame_data.sbr_invf_mode,
            );
        }

        // Adjust envelope of current frame.
        if h_frame_data.sbr_patching_mode as i32 != sbr_calculate_envelope.sbr_patching_mode {
            let fbd = &mut h_header_data.freq_band_data;
            let freq_band_table_lo = fbd.freq_band_table_lo;
            let x_over = if harmonic && h_frame_data.sbr_patching_mode == 0 {
                getx_over_band_qmf_transposer(h_hbe.as_deref()).copied()
            } else {
                None
            };
            reset_limiter_bands(
                &mut fbd.limiter_band_table,
                &mut fbd.no_limiter_bands,
                &freq_band_table_lo,
                fbd.n_sfb[0] as i32,
                &p_settings.patch_param,
                p_settings.no_of_patches as i32,
                h_header_data.bs_data.limiter_bands as i32,
                h_frame_data.sbr_patching_mode,
                x_over.as_ref(),
                get41_sbr_qmf_transposer(h_hbe.as_deref()),
            );

            sbr_calculate_envelope.sbr_patching_mode = h_frame_data.sbr_patching_mode as i32;
        }

        calculate_sbr_envelope(
            &mut dom_in.scaling,
            sbr_calculate_envelope,
            h_header_data,
            h_frame_data,
            &pvc_dynamic_data,
            &mut low_re,
            &mut low_im,
            use_lp,
            &mut degree_alias,
            flags,
            (h_header_data.frame_error_flag != 0 || h_prev_frame_data.frame_error_flag != 0) as i32,
        );

        // Update hPrevFrameData (to be used in the next frame)
        for i in 0..(h_header_data.freq_band_data.n_invf_bands as usize).min(MAX_INVF_BANDS) {
            h_prev_frame_data.sbr_invf_mode[i] = h_frame_data.sbr_invf_mode[i];
        }
        h_prev_frame_data.coupling = h_frame_data.coupling;
        h_prev_frame_data.stop_pos = borders[n_env];
        h_prev_frame_data.amp_res = h_frame_data.amp_resolution_current_frame as u8;
        h_prev_frame_data.prev_sbr_pitch_in_bins = h_frame_data.sbr_pitch_in_bins;
        // could be done in extractFrameInfo_pvc() but hPrevFrameData is not available there
        h_prev_frame_data.prev_frame_info = h_frame_data.frame_info;
    } else {
        // rescale from lsb to nAnalysisBands in order to compensate scaling with hb_scale in
        // this area, done by synthesisFiltering

        // Reset hb_scale if no highband is present, because hb_scale is considered in the
        // QMF-synthesis
        dom_in.scaling.hb_scale = save_lb_scale;

        let rescale = dom_in.scaling.hb_scale - dom_in.scaling.ov_lb_scale;
        let lsb = qmf_domain_out[ch_out].fb.lsb;
        let length = dom_in.fb.no_channels - lsb;

        if rescale < 0 && length > 0 {
            let (lo, hi) = (lsb.max(0) as usize, (lsb + length).max(0) as usize);
            for i in 0..ov_len {
                if let Some(s) = low_re[i].get_mut(lo..hi.min(SLOT_LEN)) {
                    scale_values(s, rescale);
                }
                if !use_lp {
                    if let Some(s) = low_im[i].get_mut(lo..hi.min(SLOT_LEN)) {
                        scale_values(s, rescale);
                    }
                }
            }
        }
    }

    if !harmonic {
        let mut length = dom_in.fb.lsb;
        if flags & SBRDEC_SYNTAX_USAC != 0 {
            length = dom_in.fb.no_channels;
        }
        let length = length.max(0) as usize;

        // in case of legacy sbr saving of filter states here
        for i in 0..LPC_ORDER + ov_len {
            // Store the unmodified qmf Slots values (required for LPC filtering)
            let idx = (no_cols + i).wrapping_sub(LPC_ORDER);
            if let Some(src) = low_re.get(idx) {
                copy_vals(lpp_trans.leg_re(i), src, length);
            }
            if !use_lp {
                if let Some(src) = low_im.get(idx) {
                    copy_vals(lpp_trans.leg_im(i), src, length);
                }
            }
        }
    }

    // Synthesis subband filtering.

    if flags & SBRDEC_PS_DECODED == 0 {
        if flags & SBRDEC_SKIP_QMF_SYN == 0 {
            let mut out_scalefactor = -(8);
            let dom_out = &mut qmf_domain_out[ch_out];

            if let Some(ps) = h_ps_d.as_deref_mut() {
                ps.proc_frame_based = 1; // we here do frame based processing
            }

            sbr_decoder_drc_apply(
                Some(sbr_drc_channel),
                &mut low_re,
                if use_lp { &mut [] } else { &mut low_im },
                dom_out.fb.no_col,
                &mut out_scalefactor,
            );

            let states = dom_out.p_syn_qmf_states.as_deref_mut().unwrap_or(&mut []);
            qmf_change_out_scalefactor(&mut dom_out.fb, states, out_scalefactor);

            {
                let h_freq = &mut h_header_data.freq_band_data;
                let save_usb = dom_out.fb.usb;
                // qmfTemp: C_AALLOC_SCRATCH_START(qmfTemp, FIXP_DBL, 2 * QMF_MAX_SYNTHESIS_BANDS)
                let mut qmf_temp = [0 as FixpDbl; 2 * 64];
                if dom_out.fb.usb < h_freq.ov_high_subband as i32 {
                    // we need to patch usb for this frame as overlap may contain higher
                    // frequency range if headerchange occured; fb. usb is always limited to
                    // maximum fb.no_channels; In case of wrongly decoded headers it might be
                    // that ov_highSubband is higher than the number of synthesis channels
                    // (fb.no_channels), which is forbidden, therefore we need to limit
                    // ov_highSubband with fMin function to avoid not allowed usb in synthesis
                    // filterbank.
                    dom_out.fb.usb =
                        (h_freq.ov_high_subband as u32).min(dom_out.fb.no_channels as u32) as i32;
                }

                let needed =
                    out_len(out_off, dom_out.fb.no_col, dom_out.fb.no_channels, stride_out);
                let short = time_out.len() < needed;
                let mut pad: Vec<i32> = Vec::new();
                if short {
                    pad = vec![0; needed];
                    pad[..time_out.len()].copy_from_slice(time_out);
                }
                {
                    let out: &mut [i32] = if short { &mut pad } else { &mut *time_out };
                    let states = dom_out.p_syn_qmf_states.as_deref_mut().unwrap_or(&mut []);
                    qmf_synthesis_filtering::<i32, _>(
                        &dom_out.fb,
                        states,
                        &low_re,
                        &low_im,
                        &dom_in.scaling,
                        p_settings.overlap as i32,
                        &mut out[out_off..],
                        stride_out,
                        &mut qmf_temp,
                    );
                }
                if short {
                    let n = time_out.len();
                    time_out.copy_from_slice(&pad[..n]);
                }

                // restore saved value
                dom_out.fb.usb = save_usb;
                h_freq.ov_high_subband = save_usb as u8;
            }
        }
    } else if let (Some(h_ps_d), Some(right)) = (h_ps_d, h_sbr_dec_right) {
        // (flags & SBRDEC_PS_DECODED)
        let ch_out_r = right.qmf_domain_out_ch.unwrap_or(usize::MAX);
        if let Some((syn_l, syn_r)) = pair_mut(&mut qmf_domain_out[..], ch_out, ch_out_r) {
            // adapt scaling
            let sdiff = dom_in.scaling.lb_scale - reserve; // Scaling difference
            let mut scale_factor_high_band = sdiff - dom_in.scaling.hb_scale;
            let mut scale_factor_low_band_ov = sdiff - dom_in.scaling.ov_lb_scale;
            let mut scale_factor_low_band_no_ov = sdiff - dom_in.scaling.lb_scale;

            // Scale of low band overlapping QMF data
            scale_factor_low_band_ov =
                f_min(DFRACT_BITS - 1, f_max(-(DFRACT_BITS - 1), scale_factor_low_band_ov));
            // Scale of low band current QMF data
            scale_factor_low_band_no_ov =
                f_min(DFRACT_BITS - 1, f_max(-(DFRACT_BITS - 1), scale_factor_low_band_no_ov));
            // Scale of current high band
            scale_factor_high_band =
                f_min(DFRACT_BITS - 1, f_max(-(DFRACT_BITS - 1), scale_factor_high_band));

            if h_ps_d.proc_frame_based == 1 {
                // If we have switched from frame to slot based processing copy filter states
                // (procFrameBased will be unset later); copy filter states from left to right
                fdk_assert!(n_bands_synthesis <= QMF_MAX_SYNTHESIS_BANDS as usize);
                syn_r.fb.out_scalefactor = syn_l.fb.out_scalefactor;
                let n = 9 * n_bands_synthesis;
                if let (Some(src), Some(dst)) =
                    (syn_l.p_syn_qmf_states.as_deref(), syn_r.p_syn_qmf_states.as_deref_mut())
                {
                    let n = n.min(src.len()).min(dst.len());
                    dst[..n].copy_from_slice(&src[..n]);
                }
            }

            // Feed delaylines when parametric stereo is switched on.
            prepare_ps_processing(h_ps_d, &low_re, &low_im, scale_factor_low_band_ov);

            // use the same synthese qmf values for left and right channel
            syn_r.fb.no_col = syn_l.fb.no_col;
            syn_r.fb.lsb = syn_l.fb.lsb;
            syn_r.fb.usb = syn_l.fb.usb;

            let mut env = 0usize;

            {
                // pWorkBuffer: C_AALLOC_SCRATCH_START(pWorkBuffer, FIXP_DBL, 2 *
                // QMF_MAX_SYNTHESIS_BANDS); every value used is written first
                let mut p_work_buffer = [0 as FixpDbl; 2 * 64];
                let mut max_shift = 0;

                if sbr_drc_channel.enable != 0 {
                    if sbr_drc_channel.prev_fact_exp > max_shift {
                        max_shift = sbr_drc_channel.prev_fact_exp;
                    }
                    if sbr_drc_channel.curr_fact_exp > max_shift {
                        max_shift = sbr_drc_channel.curr_fact_exp;
                    }
                    if sbr_drc_channel.next_fact_exp > max_shift {
                        max_shift = sbr_drc_channel.next_fact_exp;
                    }
                }

                // copy DRC data to right channel (with PS both channels use the same DRC gains)
                right.sbr_drc_channel = *sbr_drc_channel;

                let out_scalefactor = max_shift - (8);
                let out_scalefactor_l = sbr_in_data_headroom + 1; // +1: psDiffScale! (MPEG-PS)
                let out_scalefactor_r = out_scalefactor_l;

                let nch = syn_l.fb.no_channels.clamp(0, 64) as usize;
                let stride = stride_out;
                let needed = out_len(out_off, syn_l.fb.no_col, syn_l.fb.no_channels, stride)
                    .max(out_len(out_right_off, syn_l.fb.no_col, syn_l.fb.no_channels, stride));
                let short = time_out.len() < needed;
                let mut pad: Vec<i32> = Vec::new();
                if short {
                    pad = vec![0; needed];
                    pad[..time_out.len()].copy_from_slice(time_out);
                }
                {
                    let out: &mut [i32] = if short { &mut pad } else { &mut *time_out };
                    // The right slot of FDKhybridSynthesisApply(), zeroed once
                    // instead of per slot. Exactness: it is only read after apply_ps_slot(), whose
                    // only write to it is FDKhybridSynthesisApply() of hybrid_synthesis[1]; that
                    // writes the indices 0..nr_bands (real) and 0..cplx_bands (imaginary) of a
                    // setup that does not change within the slot loop, so every slot overwrites
                    // the same indices and the others stay 0, as with a zeroed array per slot.
                    let mut ps_r_re = [0 as FixpDbl; 64];
                    let mut ps_r_im = [0 as FixpDbl; 64];

                    for i in 0..syn_l.fb.no_col.max(0) as usize {
                        // ----- no_col loop -----
                        // qmf timeslot of right channel: rQmfReal = pWorkBuffer, rQmfImag =
                        // pWorkBuffer + synQmf->no_channels
                        {
                            let ess = &h_ps_d.bs_data[(h_ps_d.process_slot as usize).min(1)]
                                .a_env_start_stop;
                            if ess.get(env).is_some_and(|&v| i == v as usize) {
                                init_slot_based_rotation(
                                    h_ps_d,
                                    p_coef,
                                    env as i32,
                                    h_header_data.freq_band_data.high_subband as i32,
                                );
                                env += 1;
                            }

                            if i + HYBRID_FILTER_DELAY < low_re.len()
                                && i + HYBRID_FILTER_DELAY < low_im.len()
                            {
                                let (r_re, r_im) = (&mut ps_r_re, &mut ps_r_im);
                                apply_ps_slot(
                                    h_ps_d,
                                    p_coef,
                                    &mut low_re[i..], // one timeslot of left/mono channel
                                    &mut low_im[i..], // one timeslot of left/mono channel
                                    &mut r_re[..],    // one timeslot or right channel
                                    &mut r_im[..],    // one timeslot or right channel
                                    scale_factor_low_band_no_ov,
                                    if i < p_settings.overlap as usize {
                                        scale_factor_low_band_ov
                                    } else {
                                        scale_factor_low_band_no_ov
                                    },
                                    scale_factor_high_band,
                                    syn_l.fb.lsb,
                                    syn_l.fb.usb,
                                );
                                // FDKhybridSynthesisApply() writes the right slot into
                                // pWorkBuffer: the 3 LF bands interleaved, then the 61 real
                                // values, then the 61 imaginary values (64 bands each; with
                                // fewer than 64 synthesis channels the imaginary slot overlaps
                                // the real one).
                                p_work_buffer[..3].copy_from_slice(&r_re[..3]);
                                p_work_buffer[nch..nch + 3].copy_from_slice(&r_im[..3]);
                                p_work_buffer[3..64].copy_from_slice(&r_re[3..64]);
                                p_work_buffer[nch + 3..nch + 64].copy_from_slice(&r_im[3..64]);
                            }
                        }

                        sbr_decoder_drc_apply_slot_shared(
                            // right channel
                            Some(&mut right.sbr_drc_channel),
                            &mut p_work_buffer,
                            nch,
                            i as i32,
                            syn_r.fb.no_col,
                            max_shift,
                        );

                        sbr_decoder_drc_apply_slot(
                            // left channel
                            Some(sbr_drc_channel),
                            low_re[i],
                            Some(&mut *low_im[i]),
                            i as i32,
                            syn_l.fb.no_col,
                            max_shift,
                        );

                        if flags & SBRDEC_SKIP_QMF_SYN == 0 {
                            let states_l = syn_l.p_syn_qmf_states.as_deref_mut().unwrap_or(&mut []);
                            qmf_change_out_scalefactor(&mut syn_l.fb, states_l, out_scalefactor);
                            let states_r = syn_r.p_syn_qmf_states.as_deref_mut().unwrap_or(&mut []);
                            qmf_change_out_scalefactor(&mut syn_r.fb, states_r, out_scalefactor);

                            // The input slot of the right channel is the work buffer itself:
                            // the inverse modulation reads every value before it overwrites it,
                            // a copy of the input is equivalent.
                            let r_in: [FixpDbl; 2 * 64] = p_work_buffer;
                            let off = out_right_off + i * nch * stride.max(0) as usize;
                            qmf_synthesis_filtering_slot::<i32>(
                                &syn_r.fb,
                                states_r,
                                &r_in[..nch],
                                &r_in[nch..2 * nch],
                                out_scalefactor_l,
                                out_scalefactor_l,
                                &mut out[off..],
                                stride,
                                &mut p_work_buffer,
                            );
                            let off = out_off + i * nch * stride.max(0) as usize;
                            qmf_synthesis_filtering_slot::<i32>(
                                &syn_l.fb,
                                states_l,
                                low_re[i],
                                low_im[i],
                                out_scalefactor_r,
                                out_scalefactor_r,
                                &mut out[off..],
                                stride,
                                &mut p_work_buffer,
                            );
                        }
                    } // no_col loop  i
                }
                if short {
                    let n = time_out.len();
                    time_out.copy_from_slice(&pad[..n]);
                }
            }
        }
    }

    sbr_decoder_drc_update_channel(Some(sbr_drc_channel));

    // Update overlap buffer. Even bands above usb are copied to avoid outdated spectral data in
    // case the stop frequency raises.
    if flags & SBRDEC_SKIP_QMF_SYN == 0 {
        fdk_qmf_domain_save_overlap(qd, ch_in, 0);
        fdk_assert!(qd.qmf_domain_in[ch_in].scaling.ov_lb_scale == save_lb_scale);
    }

    *saved_states = 0;

    // Save current frame status
    h_prev_frame_data.frame_error_flag = h_header_data.frame_error_flag;
    *apply_sbr_proc_old = apply_processing;
}

/// `createSbrDec()`: creates (initializes) the SBR decoder of one channel.
#[allow(clippy::too_many_arguments)]
pub fn create_sbr_dec(
    h_sbr_channel: &mut SbrChannel,
    h_header_data: &mut SbrHeaderData,
    p_settings: &mut TransposerSettings,
    _downsample_fac: i32,
    _qmf_flags: u32,
    flags: u32,
    overlap: i32,
    chan: i32,
    codec_frame_size: i32,
) -> SbrError {
    let time_slots = h_header_data.number_time_slots as i32; // Number of SBR slots per frame
    let no_cols = time_slots * h_header_data.time_step as i32; // Number of QMF slots per frame
    let hs = &mut h_sbr_channel.sbr_dec;

    hs.scale_hbe = 15;
    hs.scale_lb = 15;
    hs.scale_ov = 15;

    hs.prev_frame_l_sbr = 0;
    hs.prev_frame_hbe_sbr = 0;

    hs.codec_frame_size = codec_frame_size;

    // create envelope calculator
    let err = create_sbr_envelope_calc(&mut hs.sbr_calculate_envelope, h_header_data, chan, flags);
    if err != SBRDEC_OK {
        return err;
    }

    init_sbr_prev_frame_data(&mut h_sbr_channel.prev_frame_data, time_slots);

    // create transposer
    let fbd = &h_header_data.freq_band_data;
    let err = create_lpp_transposer(
        &mut hs.lpp_trans,
        p_settings,
        fbd.low_subband as i32,
        &fbd.v_k_master,
        fbd.num_master as i32,
        fbd.high_subband as i32,
        time_slots,
        no_cols,
        &fbd.freq_band_table_noise,
        fbd.n_nfb as i32,
        h_header_data.sbr_proc_smpl_rate,
        chan,
        overlap,
    );
    if err != SBRDEC_OK {
        return err;
    }

    if flags & SBRDEC_USAC_HARMONICSBR != 0 {
        let b_sbr41 = if flags & SBRDEC_QUAD_RATE != 0 { 1 } else { 0 };
        // 32 for (32:64 and 24:64) and 16 for 16:64
        let no_channels = QMF_SYNTH_CHANNELS / ((b_sbr41 + 1) * 2);

        // fdkCallocMatrix2D_aligned() returns NULL for an empty matrix
        if no_cols <= 0 {
            return SBRDEC_MEM_ALLOC_FAILED;
        }
        let n = no_cols as usize * no_channels as usize;

        // shared memory between hbeLightTimeDelayBuffer and hQmfHBESlotsReal if
        // SBRDEC_HBE_ENABLE (tmp_memory)
        hs.hbe_slots_real = vec![0; n];
        hs.hbe_slots_imag = vec![0; n];
        // buffers containing unmodified qmf data; required when switching from legacy SBR to
        // HBE; buffer can be used as LPCFilterstates buffer because legacy SBR needs exactly
        // these values for LPC filtering
        hs.codec_qmf_buffer_real = vec![0; n];
        hs.codec_qmf_buffer_imag = vec![0; n];
        hs.hbe_no_channels = no_channels as usize;

        let err = qmf_transposer_create(&mut hs.h_hbe, codec_frame_size, 0, b_sbr41);
        if err != SBRDEC_OK {
            return err;
        }
    }

    SBRDEC_OK
}

/// `deleteSbrDec()`: frees the buffers of the SBR decoder of one channel.
pub fn delete_sbr_dec(h_sbr_channel: &mut SbrChannel) -> i32 {
    let hs = &mut h_sbr_channel.sbr_dec;

    delete_sbr_envelope_calc(&mut hs.sbr_calculate_envelope);

    hs.hbe_slots_real = Vec::new();
    hs.hbe_slots_imag = Vec::new();
    qmf_transposer_close(&mut hs.h_hbe);
    hs.codec_qmf_buffer_real = Vec::new();
    hs.codec_qmf_buffer_imag = Vec::new();

    0
}

/// `resetSbrDec()`: resets the SBR decoder of one channel after a header change.
/// `h_frame_data` is `pSbrChannel[ch]->frameData` of the caller, i.e. always `frameData[0]`.
#[allow(clippy::too_many_arguments)]
pub fn reset_sbr_dec(
    h_sbr_dec: &mut SbrDec,
    qd: &mut FdkQmfDomain,
    h_header_data: &mut SbrHeaderData,
    h_prev_frame_data: &SbrPrevFrameData,
    _downsample_fac: i32,
    flags: u32,
    h_frame_data: &SbrFrameData,
    p_settings: &mut TransposerSettings,
) -> SbrError {
    let (Some(ch_in), Some(ch_out)) = (h_sbr_dec.qmf_domain_in_ch, h_sbr_dec.qmf_domain_out_ch)
    else {
        fdk_assert!(false);
        return SBRDEC_OK;
    };
    if ch_in >= QMF_DOMAIN_MAX_CHANNELS || ch_out >= QMF_DOMAIN_MAX_CHANNELS {
        return SBRDEC_OK;
    }
    let use_lp = flags & SBRDEC_LOW_POWER != 0;
    let overlap = p_settings.overlap as i32;

    let mut old_lsb = qd.qmf_domain_in[ch_in].fb.lsb;
    let mut old_usb = qd.qmf_domain_in[ch_in].fb.usb;
    let mut new_lsb = h_header_data.freq_band_data.low_subband as i32;
    // int new_usb = hHeaderData->freqBandData.highSubband;

    // in case the previous frame was not active in terms of SBR processing, the full band from 0
    // to no_channels was rescaled and not overwritten. Thats why the scaling factor lb_scale can
    // be seen as assigned to all bands from 0 to no_channels in the previous frame. The same
    // states for the current frame if the current frame is not active in terms of SBR
    // processing
    let apply_sbr_proc = h_header_data.sync_state == SBR_ACTIVE
        || (h_header_data.frame_error_flag == 0 && h_header_data.sync_state == SBR_HEADER);
    let apply_sbr_proc_old = h_sbr_dec.apply_sbr_proc_old;

    if !apply_sbr_proc {
        new_lsb = qd.qmf_domain_in[ch_in].fb.no_channels;
    }
    if apply_sbr_proc_old == 0 {
        old_lsb = qd.qmf_domain_in[ch_in].fb.no_channels;
        old_usb = old_lsb;
    }

    reset_sbr_envelope_calc(&mut h_sbr_dec.sbr_calculate_envelope);

    // Change lsb and usb
    // Synthesis
    {
        let out = &mut qd.qmf_domain_out[ch_out].fb;
        out.lsb = fix_min(out.no_channels, h_header_data.freq_band_data.low_subband as i32);
        out.usb = fix_min(out.no_channels, h_header_data.freq_band_data.high_subband as i32);
    }
    // Analysis
    {
        let (lsb, usb) = (qd.qmf_domain_out[ch_out].fb.lsb, qd.qmf_domain_out[ch_out].fb.usb);
        qd.qmf_domain_in[ch_in].fb.lsb = lsb;
        qd.qmf_domain_in[ch_in].fb.usb = usb;
    }

    // The following initialization of spectral data in the overlap buffer is required for
    // dynamic x-over or a change of the start-freq for 2 reasons:
    // 1. If the lowband gets _wider_, unadjusted data would remain
    // 2. If the lowband becomes _smaller_, the highest bands of the old lowband must be cleared
    //    because the whitening would be affected
    let mut start_band = old_lsb;
    let mut stop_band = new_lsb;
    let start_slot = f_max(
        0,
        h_header_data.time_step as i32
            * (h_prev_frame_data.stop_pos as i32 - h_header_data.number_time_slots as i32),
    );
    let mut size = f_max(0, stop_band - start_band);

    let SbrDec {
        lpp_trans,
        h_hbe,
        hbe_slots_real,
        hbe_slots_imag,
        codec_qmf_buffer_real,
        codec_qmf_buffer_imag,
        hbe_no_channels,
        saved_states,
        scale_lb,
        scale_hbe,
        sbr_calculate_envelope,
        ..
    } = h_sbr_dec;
    let hbe_nch = (*hbe_no_channels).max(1);

    // OverlapBufferReal/Imag: the slots of the input channel (overlap and frame)
    let n_slots = QMF_SLOTS_SIZE;
    let ptrs = domain_slot_ptrs(&qd.qmf_domain_in[ch_in], n_slots);
    {
        let FdkQmfDomain { qmf_domain_in, mem, .. } = &mut *qd;
        let dom_in = &mut qmf_domain_in[ch_in];
        let (mut ov_re, mut ov_im) = (SlotList::new(), SlotList::new());
        domain_slots(mem, &ptrs, &mut ov_re, &mut ov_im);

        let clear = |s: &mut [FixpDbl], start: i32, size: i32| {
            if start >= 0 && size > 0 {
                let (a, b) = (start as usize, (start + size) as usize);
                if let Some(v) = s.get_mut(a..b.min(s.len()).max(a)) {
                    v.fill(0);
                }
            }
        };
        let scale_range = |s: &mut [FixpDbl], start: i32, len: i32, sf: i32| {
            if start >= 0 && len > 0 {
                let (a, b) = (start as usize, (start + len) as usize);
                if let Some(v) = s.get_mut(a..b.min(s.len()).max(a)) {
                    scale_values(v, sf);
                }
            }
        };

        // in case of USAC we don't want to zero out the memory, as this can lead to holes in
        // the spectrum; fix shall only be applied for USAC not for MPEG-4 SBR, in this case
        // setting zero remains
        if flags & SBRDEC_SYNTAX_USAC == 0 {
            // keep already adjusted data in the x-over-area
            for l in start_slot..overlap {
                clear(ov_re[l as usize], start_band, size);
                if !use_lp {
                    clear(ov_im[l as usize], start_band, size);
                }
            }

            // reset LPC filter states
            start_band = fix_min(old_lsb, new_lsb);
            stop_band = fix_max(old_lsb, new_lsb);
            size = fix_max(0, stop_band - start_band);

            clear(lpp_trans.leg_re(0), start_band, size);
            clear(lpp_trans.leg_re(1), start_band, size);
            if !use_lp {
                clear(lpp_trans.leg_im(0), start_band, size);
                clear(lpp_trans.leg_im(1), start_band, size);
            }
        }

        if start_slot != 0 {
            // Rescale already processed spectral data between old and new x-over frequency.
            // This must be done because of the separate scalefactors for lowband and highband.
            let mut source_exp;
            let target_exp;
            let target_lsb;
            let target_usb;

            // We have four relevant transitions to cover (see the C documentation).
            if new_lsb > old_lsb {
                // case 1 and 3
                source_exp = 15 - dom_in.scaling.ov_hb_scale; // SCALE2EXP
                target_exp = 15 - dom_in.scaling.ov_lb_scale;

                start_band = old_lsb;

                if new_lsb >= old_usb {
                    // case 1
                    stop_band = old_usb;
                } else {
                    // case 3
                    stop_band = new_lsb;
                }

                target_lsb = 0;
                target_usb = old_lsb;
            } else {
                // case 2 and 4
                source_exp = 15 - dom_in.scaling.ov_lb_scale;
                target_exp = 15 - dom_in.scaling.ov_hb_scale;

                start_band = new_lsb;
                stop_band = old_lsb;

                target_lsb = old_lsb;
                target_usb = old_usb;
            }

            let max_val = max_subband_sample(
                &ov_re,
                if use_lp { None } else { Some(&ov_im) },
                start_band,
                stop_band,
                0,
                start_slot,
            );

            let mut reserve = if max_val != 0 { cnt_leading_zeros(max_val) - 1 } else { 0 };
            reserve = fix_min(reserve, DFRACT_BITS - 1 - (15 - source_exp)); // EXP2SCALE

            // process only if x-over-area is not dominant after rescale; otherwise I'm not sure
            // if all buffers are scaled correctly;
            if target_exp - (source_exp - reserve) >= 0 {
                rescale_subband_samples(
                    &mut ov_re,
                    if use_lp { None } else { Some(&mut ov_im) },
                    start_band,
                    stop_band,
                    0,
                    start_slot,
                    reserve,
                );
                source_exp -= reserve;
            }

            let mut delta_exp = target_exp - source_exp;

            if delta_exp < 0 {
                // x-over-area is dominant
                start_band = target_lsb;
                stop_band = target_usb;
                delta_exp = -delta_exp;

                if new_lsb > old_lsb {
                    // The lowband has to be rescaled
                    dom_in.scaling.ov_lb_scale = 15 - source_exp; // EXP2SCALE
                } else {
                    // The highband has to be rescaled
                    dom_in.scaling.ov_hb_scale = 15 - source_exp;
                }
            }

            fdk_assert!(start_band <= stop_band);

            for l in 0..start_slot as usize {
                scale_range(ov_re[l], start_band, stop_band - start_band, -delta_exp);
                if !use_lp {
                    scale_range(ov_im[l], start_band, stop_band - start_band, -delta_exp);
                }
            }
        } // startSlot != 0
    }

    // Initialize transposer and limiter
    {
        let fbd = &h_header_data.freq_band_data;
        let sbr_error = reset_lpp_transposer(
            p_settings,
            fbd.low_subband,
            &fbd.v_k_master,
            fbd.num_master,
            &fbd.freq_band_table_noise,
            fbd.n_nfb,
            fbd.high_subband,
            h_header_data.sbr_proc_smpl_rate,
        );
        if sbr_error != SBRDEC_OK {
            return sbr_error;
        }
    }

    *saved_states = 0;

    if flags & SBRDEC_USAC_HARMONICSBR != 0 && apply_sbr_proc {
        let fbd = &h_header_data.freq_band_data;
        let sbr_error = qmf_transposer_re_init(
            h_hbe.as_deref_mut(),
            [&fbd.freq_band_table_lo[..], &fbd.freq_band_table_hi[..]],
            fbd.n_sfb,
        );
        if sbr_error != SBRDEC_OK {
            return sbr_error;
        }

        let (hbe_cols, hbe_ch) = match h_hbe.as_deref() {
            Some(h) => (h.no_cols, h.no_channels.max(0) as usize),
            None => (0, 0),
        };

        // copy saved states from previous frame to legacy SBR lpc filterstate buffer
        for i in 0..LPC_ORDER + overlap as usize {
            let row = hbe_cols - LPC_ORDER as i32 - overlap + i as i32;
            copy_vals(lpp_trans.leg_re(i), flat_row(codec_qmf_buffer_real, row, hbe_nch), hbe_ch);
            copy_vals(lpp_trans.leg_im(i), flat_row(codec_qmf_buffer_imag, row, hbe_nch), hbe_ch);
        }
        *saved_states = 1;

        {
            let n_lpc = overlap as usize + LPC_ORDER;
            let hc = hbe_cols.max(0) as usize;
            let quad = flags & SBRDEC_QUAD_RATE != 0;
            let patching0 = h_frame_data.sbr_patching_mode == 0;
            let hb_scale = &mut qd.qmf_domain_in[ch_in].scaling.hb_scale;
            let lpc = &mut lpp_trans.lpc;
            let (lpc_lo, lpc_hbe_im) = lpc.split_at_mut(LPC_HBE_IM);
            let lpc_hbe_re = &mut lpc_lo[LPC_HBE_RE..];

            // map QMF buffer to pointer array (Overlap + Frame)
            let mut low_re: Vec<&mut [FixpDbl]> =
                lpc_hbe_re.chunks_mut(LPC_HBE_COLS).take(n_lpc).collect();
            let mut low_im: Vec<&mut [FixpDbl]> =
                lpc_hbe_im.chunks_mut(LPC_HBE_COLS).take(n_lpc).collect();
            // map QMF buffer to pointer array (Overlap + Frame)
            low_re.extend(codec_qmf_buffer_real.chunks_mut(hbe_nch).take(hc));
            low_im.extend(codec_qmf_buffer_imag.chunks_mut(hbe_nch).take(hc));
            while low_re.len() < n_lpc + hc {
                low_re.push(&mut []);
            }
            while low_im.len() < n_lpc + hc {
                low_im.push(&mut []);
            }

            if let Some(h) = h_hbe.as_deref_mut() {
                if quad {
                    if patching0 {
                        let x_over_qmf = h.x_over_qmf;
                        // in case of harmonic SBR and no HBE_LP map additional buffer for one
                        // more frame to pointer array
                        low_re.extend(hbe_slots_real.chunks_mut(hbe_nch).take(hc / 2));
                        low_im.extend(hbe_slots_imag.chunks_mut(hbe_nch).take(hc / 2));

                        qmf_transposer_apply(
                            h,
                            &mut low_re,
                            &mut low_im,
                            overlap as usize + hc / 2 + LPC_ORDER,
                            hbe_cols,
                            None,
                            h_prev_frame_data.prev_sbr_pitch_in_bins as i32,
                            *scale_lb as i32,
                            *scale_hbe as i32,
                            hb_scale,
                            h_header_data.time_step as i32,
                            h_frame_data.frame_info.borders[0] as i32,
                            overlap,
                            KEEP_STATES_SYNCED_OUTDIFF,
                        );

                        copy_harmonic_spectrum(
                            &x_over_qmf,
                            &mut low_re,
                            &mut low_im,
                            hbe_cols,
                            overlap,
                            KEEP_STATES_SYNCED_OUTDIFF,
                        );
                    }
                } else {
                    // in case of harmonic SBR and no HBE_LP map additional buffer for one more
                    // frame to pointer array
                    low_re.extend(hbe_slots_real.chunks_mut(hbe_nch).take(hc));
                    low_im.extend(hbe_slots_imag.chunks_mut(hbe_nch).take(hc));

                    if patching0 {
                        qmf_transposer_apply(
                            h,
                            &mut low_re,
                            &mut low_im,
                            overlap as usize + LPC_ORDER,
                            hbe_cols,
                            None,
                            0, // not required for keeping states updated in this frame
                            *scale_lb as i32,
                            *scale_lb as i32,
                            hb_scale,
                            h_header_data.time_step as i32,
                            h_frame_data.frame_info.borders[0] as i32,
                            overlap,
                            KEEP_STATES_SYNCED_NOOUT,
                        );
                    }

                    qmf_transposer_apply(
                        h,
                        &mut low_re,
                        &mut low_im,
                        overlap as usize + hc + LPC_ORDER,
                        hbe_cols,
                        None,
                        h_prev_frame_data.prev_sbr_pitch_in_bins as i32,
                        *scale_lb as i32,
                        *scale_hbe as i32,
                        hb_scale,
                        h_header_data.time_step as i32,
                        h_frame_data.frame_info.borders[0] as i32,
                        overlap,
                        KEEP_STATES_SYNCED_OUTDIFF,
                    );
                }
            }
        }

        if h_frame_data.sbr_patching_mode == 0 {
            let ptrs = domain_slot_ptrs(&qd.qmf_domain_in[ch_in], n_slots);
            let (mut ov_re, mut ov_im) = (SlotList::new(), SlotList::new());
            domain_slots(&mut qd.mem, &ptrs, &mut ov_re, &mut ov_im);
            for i in start_slot.max(0)..overlap {
                // Store the unmodified qmf Slots values for upper part of spectrum (required for
                // LPC filtering) required if next frame is a HBE frame
                let iu = i as usize;
                if let Some(d) = ov_re.get_mut(iu) {
                    copy_vals(d, lpp_trans.hbe_re(iu + LPC_ORDER), SLOT_LEN);
                }
                if let Some(d) = ov_im.get_mut(iu) {
                    copy_vals(d, lpp_trans.hbe_im(iu + LPC_ORDER), SLOT_LEN);
                }
            }
            let hbe_cols = h_hbe.as_deref().map_or(0, |h| h.no_cols);
            for i in start_slot.max(0)..overlap {
                // Store the unmodified qmf Slots values for upper part of spectrum (required for
                // LPC filtering) required if next frame is a HBE frame
                let iu = i as usize;
                let row = hbe_cols - overlap + i;
                if let Some(d) = ov_re.get_mut(iu) {
                    copy_vals(
                        d,
                        flat_row(codec_qmf_buffer_real, row, hbe_nch),
                        new_lsb.max(0) as usize,
                    );
                }
                if let Some(d) = ov_im.get_mut(iu) {
                    copy_vals(
                        d,
                        flat_row(codec_qmf_buffer_imag, row, hbe_nch),
                        new_lsb.max(0) as usize,
                    );
                }
            }
        }
    }

    {
        let ptrs = domain_slot_ptrs(&qd.qmf_domain_in[ch_in], n_slots);
        let FdkQmfDomain { qmf_domain_in, mem, .. } = &mut *qd;
        let dom_in = &mut qmf_domain_in[ch_in];
        let (mut ov_re, mut ov_im) = (SlotList::new(), SlotList::new());
        domain_slots(mem, &ptrs, &mut ov_re, &mut ov_im);

        let mut adapt_lb = 0;
        let mut diff = 0;
        let mut new_scale = dom_in.scaling.ov_lb_scale;

        if dom_in.scaling.ov_lb_scale != dom_in.scaling.lb_scale && start_slot != 0 {
            // we need to adapt spectrum to have equal scale factor, always larger than zero
            diff = (15 - dom_in.scaling.ov_lb_scale) - (15 - dom_in.scaling.lb_scale); // SCALE2EXP

            if diff > 0 {
                adapt_lb = 1;
                diff = -diff;
                new_scale = dom_in.scaling.ov_lb_scale;
            }

            stop_band = new_lsb;
        }

        if h_frame_data.sbr_patching_mode == 1 {
            // scale states from LegSBR filterstates buffer
            for i in 0..overlap as usize + LPC_ORDER {
                let s = lpp_trans.leg_re(i);
                let n = (new_lsb.max(0) as usize).min(s.len());
                scale_values(&mut s[..n], diff);
                if !use_lp {
                    let s = lpp_trans.leg_im(i);
                    let n = (new_lsb.max(0) as usize).min(s.len());
                    scale_values(&mut s[..n], diff);
                }
            }

            if flags & SBRDEC_SYNTAX_USAC != 0 {
                // get missing states between old and new x_over from LegSBR filterstates
                // buffer; in case of legacy SBR we leave these values zeroed out
                let n = f_max(new_lsb - old_lsb, 0) as usize;
                if old_lsb >= 0 {
                    let o = old_lsb as usize;
                    for i in start_slot.max(0)..overlap {
                        let iu = i as usize;
                        if let Some(d) = ov_re.get_mut(iu).and_then(|d| d.get_mut(o..)) {
                            copy_vals(
                                d,
                                lpp_trans.leg_re(LPC_ORDER + iu).get(o..).unwrap_or(&[]),
                                n,
                            );
                        }
                        if !use_lp {
                            if let Some(d) = ov_im.get_mut(iu).and_then(|d| d.get_mut(o..)) {
                                copy_vals(
                                    d,
                                    lpp_trans.leg_im(LPC_ORDER + iu).get(o..).unwrap_or(&[]),
                                    n,
                                );
                            }
                        }
                    }
                }
            }
            if new_lsb > old_lsb {
                stop_band = old_lsb;
            }
        }
        if adapt_lb == 1 && stop_band > start_band {
            for l in start_slot.max(0)..overlap {
                let lu = l as usize;
                if start_band >= 0 {
                    let (a, b) = (start_band as usize, stop_band as usize);
                    if let Some(s) =
                        ov_re.get_mut(lu).and_then(|s| s.get_mut(a..b.min(s.len()).max(a)))
                    {
                        scale_values(s, diff);
                    }
                    if !use_lp {
                        if let Some(s) =
                            ov_im.get_mut(lu).and_then(|s| s.get_mut(a..b.min(s.len()).max(a)))
                        {
                            scale_values(s, diff);
                        }
                    }
                }
            }
        }
        dom_in.scaling.ov_lb_scale = new_scale;
    }

    let fbd = &mut h_header_data.freq_band_data;
    let freq_band_table_lo = fbd.freq_band_table_lo;
    let sbr_error = reset_limiter_bands(
        &mut fbd.limiter_band_table,
        &mut fbd.no_limiter_bands,
        &freq_band_table_lo,
        fbd.n_sfb[0] as i32,
        &p_settings.patch_param,
        p_settings.no_of_patches as i32,
        h_header_data.bs_data.limiter_bands as i32,
        h_frame_data.sbr_patching_mode,
        getx_over_band_qmf_transposer(h_hbe.as_deref()),
        get41_sbr_qmf_transposer(h_hbe.as_deref()),
    );

    sbr_calculate_envelope.sbr_patching_mode = h_frame_data.sbr_patching_mode as i32;

    sbr_error
}
