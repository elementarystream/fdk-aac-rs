// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/psy_main.h, libAACenc/src/psy_main.cpp.

//! Port of `psy_main.h`: the psychoacoustic model.
//!
//! # Memory model
//!
//! `PSY_ELEMENT::psyStatic[ch]` points into `PSY_INTERNAL::pStaticChannels[]` in C; the port
//! stores the indexes ([`UNASSIGNED`] for `NULL`). `FDKaacEnc_psyInit()` assigns the statics of an
//! element consecutively, so they form the contiguous slice
//! `p_static_channels[psy_static[0]..psy_static[0] + n]`. Where a channel pair has statics that
//! are not consecutive (see [`psy_main_stale_element_mappings`]), `FDKaacEnc_EncodeFrame()` moves
//! them next to each other for the call.
//!
//! `PSY_DYNAMIC` lives in the shared dynamic RAM of the encoder (`P_BUF_1`) in C, which it
//! shares with the quantizer's scratch buffers (bit look-up tables, threshold adjustment
//! buffers); here it is owned by [`PsyInternal`] and persists between calls. The encoder outputs
//! do not depend on its contents before a call (see [`fdk_aac_enc_psy_main`]).
//!
//! `PSY_DATA::mdctSpectrum` and the six arrays of `PSY_OUT_CHANNEL` that point into
//! `QC_OUT_CHANNEL` are the arrays of the element's [`QcOutChannel`]s, passed separately.

use crate::aacenc::*;
use crate::aacenc_pns::{PnsData, NO_NOISE_PNS};
use crate::aacenc_tns::{TnsData, TnsDataRaw, HIFILT, LOFILT};
use crate::band_nrg::*;
use crate::block_switch::*;
use crate::channel_map::{fdk_aac_enc_get_mono_stereo_mode, EL_MODE_MONO, EL_MODE_STEREO};
use crate::grp_data::fdk_aac_enc_group_short_data;
use crate::intensity::fdk_aac_enc_intensity_stereo_processing;
use crate::interface::*;
use crate::ms_stereo::fdk_aac_enc_ms_stereo_processing;
use crate::pns_func::*;
use crate::pre_echo_control::*;
use crate::psy_configuration::*;
use crate::psy_const::*;
use crate::psy_data::{PsyData, PsyStatic};
use crate::qc_data::{ChannelMapping, QcOutChannel};
use crate::spreading::fdk_aac_enc_spreading_max;
use crate::tns_func::*;
use crate::tonality::fdk_aac_enc_calculate_full_tonality;
use crate::transform::{fdk_aac_enc_transform_real, fdk_aac_enc_transform_real_eld};
use fdk_aac_rs_fdk::audio::{
    AudioObjectType, AC_SBR_PRESENT, AOT_ER_AAC_ELD, AOT_ER_AAC_LD, ID_LFE,
};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixpoint_math::{ld_data_vector, LD_DATA_SCALING};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::mdct::mdct_init;
use fdk_aac_rs_fdk::{Arch, ARCH};

mod pool;
pub use pool::fdk_aac_enc_psy_main_pool;
#[cfg(feature = "verify-poison")]
pub(crate) use pool::poison_psy_dynamic;
pub(crate) use pool::{psy_main_finish, psy_main_finish_cannot_fail, psy_main_start, PsyFrame};

/// `PSY_ELEMENT`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsyElement {
    /// `psyStatic[ch]`: indexes into [`PsyInternal::p_static_channels`]
    pub psy_static: [usize; 2],
}

/// `PSY_DYNAMIC` (layout identical to the C struct apart from the `mdctSpectrum` pointers of
/// [`PsyData`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsyDynamic {
    pub psy_data: [PsyData; 2],
    pub tns_data: [TnsData; 2],
    pub pns_data: [PnsData; 2],
}

/// `PSY_INTERNAL`
#[derive(Clone, Debug, Default)]
pub struct PsyInternal {
    /// LONG / SHORT
    pub psy_conf: [PsyConfiguration; 2],
    /// `psyElement[]` (`nElements` entries)
    pub psy_element: Vec<PsyElement>,
    /// `pStaticChannels[]` (`nChannels` entries)
    pub p_static_channels: Vec<PsyStatic>,
    pub psy_dynamic: Box<PsyDynamic>,
    pub granule_length: i32,
}

/// Blending to reduce gibbs artifacts: `FADE_OUT_LEN`
const FADE_OUT_LEN: usize = 6;
/// `fadeOutFactor`
///
/// `scripts/dump_table.py libAACenc/src/psy_main.cpp fadeOutFactor LONG`
pub static FADE_OUT_FACTOR: [FixpDbl; FADE_OUT_LEN] =
    [1840644096, 1533870080, 1227096064, 920322048, 613548032, 306774016];

/// The `MAX_GROUPED_SFB + 1` `INT`s that `FDKmemcpy` in `FDKaacEnc_psyMain` copies from
/// `psyConf->sfbOffset`, reproduced from the C layout: after the array come the following members
/// of the C struct (`filterbank`, then `sfbPcmQuantThreshold[]`).
fn psy_conf_sfb_offsets_61(conf: &PsyConfiguration, dst: &mut [i32; MAX_GROUPED_SFB + 1]) {
    const N: usize = MAX_SFB + 1;
    dst[..N].copy_from_slice(&conf.sfb_offset);
    dst[N] = conf.filterbank;
    dst[N + 1..].copy_from_slice(&conf.sfb_pcm_quant_threshold[..MAX_GROUPED_SFB - N]);
}

/// The two elements of a slice of (at least) two, mutably.
fn pair<T>(s: &mut [T]) -> (&mut T, &mut T) {
    let (a, b) = s.split_at_mut(1);
    (&mut a[0], &mut b[0])
}

/// `NULL` in [`PsyElement::psy_static`] and
/// [`PsyOutElement::psy_out_channel`](crate::interface::PsyOutElement::psy_out_channel): C
/// allocates the elements with `FDKcalloc`, and only `FDKaacEnc_psyInit()` assigns the pointers.
pub const UNASSIGNED: usize = usize::MAX;

/// The first of the `n` channels of an element mapping (`psyStatic[]` or `psyOutChannel[]`) if
/// they are assigned, below `len` and consecutive, as `FDKaacEnc_psyInit()` assigns them for the
/// current channel mode; `None` otherwise.
pub fn psy_element_channels(map: &[usize; 2], n: i32, len: usize) -> Option<usize> {
    let n = n.clamp(0, 2) as usize;
    (0..n).all(|ch| map[ch] < len && map[ch] == map[0] + ch).then_some(map[0])
}

/// The statics of the `n` channels of an element (`psyStatic[]`) if they are assigned (not
/// `NULL`), below `len` and, for a channel pair, two different ones; `None` otherwise (see
/// [`psy_main_stale_element_mappings`]). They need not be consecutive.
pub fn psy_element_statics(map: &[usize; 2], n: i32, len: usize) -> Option<[usize; 2]> {
    let n = n.clamp(0, 2) as usize;
    ((0..n).all(|ch| map[ch] < len) && (n < 2 || map[0] != map[1])).then_some(*map)
}

thread_local! {
    static STALE_ELEMENT_MAPPINGS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn psy_main_stale_element_mappings() -> u64 {
    STALE_ELEMENT_MAPPINGS.with(|c| c.get())
}

/// Adds to the counter of [`psy_main_stale_element_mappings`] (the carry of the thread pool).
pub(crate) fn add_psy_main_stale_element_mappings(n: u64) {
    STALE_ELEMENT_MAPPINGS.with(|c| c.set(c.get().wrapping_add(n)));
}

#[cold]
#[inline(never)]
pub(crate) fn stale_element_mapping() {
    STALE_ELEMENT_MAPPINGS.with(|c| c.set(c.get() + 1));
}

/// `FDKaacEnc_PsyNew(phpsy, nElements, nChannels, dynamic_RAM)`: allocates memory for the
/// psychoacoustic model (`nElements` elements, `nChannels` static channel states with their
/// input buffers, and the dynamic data).
pub fn fdk_aac_enc_psy_new(
    phpsy: &mut Option<Box<PsyInternal>>,
    n_elements: i32,
    n_channels: i32,
) -> AacEncoderError {
    // FDKcalloc: psyStatic[] is NULL until FDKaacEnc_psyInit()
    let h_psy = PsyInternal {
        psy_element: vec![PsyElement { psy_static: [UNASSIGNED; 2] }; n_elements.max(0) as usize],
        p_static_channels: vec![PsyStatic::default(); n_channels.max(0) as usize],
        ..Default::default()
    };
    *phpsy = Some(Box::new(h_psy));
    AAC_ENC_OK
}

/// `FDKaacEnc_PsyOutNew(phpsyOut, nElements, nChannels, nSubFrames, dynamic_RAM)`: allocates
/// the `PSY_OUT` structures of `nSubFrames` sub frames.
pub fn fdk_aac_enc_psy_out_new(
    phpsy_out: &mut [Option<Box<PsyOut>>],
    n_elements: i32,
    n_channels: i32,
    n_sub_frames: i32,
) -> AacEncoderError {
    for n in 0..n_sub_frames.max(0) as usize {
        phpsy_out[n] = Some(Box::new(PsyOut {
            // FDKcalloc: psyOutChannel[] is NULL until FDKaacEnc_psyInit()
            psy_out_element: vec![
                PsyOutElement {
                    psy_out_channel: [UNASSIGNED; 2],
                    ..Default::default()
                };
                n_elements.max(0) as usize
            ],
            p_psy_out_channels: vec![PsyOutChannel::default(); n_channels.max(0) as usize],
            // FDKcalloc: the array pointers are NULL until FDKaacEnc_EncodeFrame()
            p_array_qc: vec![UNASSIGNED; n_channels.max(0) as usize],
        }));
    }
    AAC_ENC_OK
}

/// `FDKaacEnc_psyInitStates(hPsy, psyStatic, audioObjectType)`
pub fn fdk_aac_enc_psy_init_states(
    psy_static: &mut PsyStatic,
    audio_object_type: AudioObjectType,
) -> AacEncoderError {
    // init input buffer
    psy_static.psy_input_buffer[..MAX_INPUT_BUFFER_SIZE].fill(0);

    fdk_aac_enc_init_block_switching(
        &mut psy_static.block_switching_control,
        is_low_delay(audio_object_type),
    );

    AAC_ENC_OK
}

/// `FDKaacEnc_psyInit(hPsy, phpsyOut, nSubFrames, nMaxChannels, audioObjectType, cm)`
pub fn fdk_aac_enc_psy_init(
    h_psy: &mut PsyInternal,
    phpsy_out: &mut [Option<Box<PsyOut>>],
    n_sub_frames: i32,
    n_max_channels: i32,
    audio_object_type: AudioObjectType,
    cm: &ChannelMapping,
) -> AacEncoderError {
    let error_status = AAC_ENC_OK;
    let mut ch_inc: usize = 0;
    let mut reset_channels: usize = 3;

    if (n_max_channels > 2) && (cm.n_channels == 2) {
        ch_inc = 1;
        fdk_aac_enc_psy_init_states(&mut h_psy.p_static_channels[0], audio_object_type);
    }

    if n_max_channels == 2 {
        reset_channels = 0;
    }

    for i in 0..cm.n_elements as usize {
        for ch in 0..cm.el_info[i].n_channels_in_el as usize {
            h_psy.psy_element[i].psy_static[ch] = ch_inc;
            let psy_static = &mut h_psy.p_static_channels[ch_inc];
            if cm.el_info[i].el_type != ID_LFE {
                if ch_inc >= reset_channels {
                    fdk_aac_enc_psy_init_states(psy_static, audio_object_type);
                }
                mdct_init(&mut psy_static.mdct_pers, 0);
                psy_static.is_lfe = 0;
            } else {
                psy_static.is_lfe = 1;
            }
            ch_inc += 1;
        }
    }

    for n in 0..n_sub_frames.max(0) as usize {
        let Some(psy_out) = phpsy_out[n].as_deref_mut() else { continue };
        ch_inc = 0;
        for i in 0..cm.n_elements as usize {
            for ch in 0..cm.el_info[i].n_channels_in_el as usize {
                psy_out.psy_out_element[i].psy_out_channel[ch] = ch_inc;
                ch_inc += 1;
            }
        }
    }

    error_status
}

/// `FDKaacEnc_psyMainInit(hPsy, audioObjectType, cm, sampleRate, granuleLength, bitRate,
/// tnsMask, bandwidth, usePns, useIS, useMS, syntaxFlags, initFlags)`: initializes the
/// psychoacoustic model.
pub fn fdk_aac_enc_psy_main_init(
    h_psy: &mut PsyInternal,
    audio_object_type: AudioObjectType,
    cm: &ChannelMapping,
    sample_rate: i32,
    granule_length: i32,
    bit_rate: i32,
    tns_mask: i32,
    bandwidth: i32,
    use_pns: i32,
    use_is: i32,
    use_ms: i32,
    syntax_flags: u32,
    init_flags: u32,
) -> AacEncoderError {
    let mut error_status: AacEncoderError;
    let channels_eff = cm.n_channels_eff;

    // Where the element mapping has no defined reference result (see
    // psy_main_stale_element_mappings), the port refuses the initialization before it changes any
    // state (deviation).
    let n_static = h_psy.p_static_channels.len();
    for i in 0..cm.n_elements.max(0) as usize {
        let map = h_psy.psy_element.get(i).map_or([UNASSIGNED; 2], |e| e.psy_static);
        if psy_element_statics(&map, cm.el_info[i].n_channels_in_el, n_static).is_none() {
            stale_element_mapping();
            return AAC_ENC_UNKNOWN;
        }
    }

    // ... and map to tnsChannels
    let tns_channels = match fdk_aac_enc_get_mono_stereo_mode(cm.enc_mode) {
        EL_MODE_MONO => 1,
        EL_MODE_STEREO => 2,
        _ => 0,
    };

    let filter_bank: FbType = match audio_object_type {
        AOT_ER_AAC_LD => FB_LD,
        AOT_ER_AAC_ELD => FB_ELD,
        _ => FB_LC,
    };

    h_psy.granule_length = granule_length;

    // Division by channelsEff (not 0 here; the port gives 0 for it).
    let div = |a: i32, b: i32| if b != 0 { a.wrapping_div(b) } else { 0 };

    error_status = fdk_aac_enc_init_psy_configuration(
        div(bit_rate, channels_eff),
        sample_rate,
        bandwidth,
        LONG_WINDOW,
        h_psy.granule_length,
        use_is,
        use_ms,
        &mut h_psy.psy_conf[0],
        filter_bank,
    );
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    // C passes tC = &pC->tnsConf; the function does not read pC->tnsConf, so a copy is exact.
    let mut tns_conf = h_psy.psy_conf[0].tns_conf;
    error_status = fdk_aac_enc_init_tns_configuration(
        div(bit_rate.wrapping_mul(tns_channels), channels_eff),
        sample_rate,
        tns_channels,
        LONG_WINDOW,
        h_psy.granule_length,
        is_low_delay(audio_object_type),
        if (syntax_flags & AC_SBR_PRESENT) != 0 { 1 } else { 0 },
        &mut tns_conf,
        &h_psy.psy_conf[0],
        tns_mask & 2,
        tns_mask & 8,
    );
    h_psy.psy_conf[0].tns_conf = tns_conf;
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    if granule_length > 512 {
        error_status = fdk_aac_enc_init_psy_configuration(
            div(bit_rate, channels_eff),
            sample_rate,
            bandwidth,
            SHORT_WINDOW,
            h_psy.granule_length,
            use_is,
            use_ms,
            &mut h_psy.psy_conf[1],
            filter_bank,
        );
        if error_status != AAC_ENC_OK {
            return error_status;
        }

        let mut tns_conf = h_psy.psy_conf[1].tns_conf;
        error_status = fdk_aac_enc_init_tns_configuration(
            div(bit_rate.wrapping_mul(tns_channels), channels_eff),
            sample_rate,
            tns_channels,
            SHORT_WINDOW,
            h_psy.granule_length,
            is_low_delay(audio_object_type),
            if (syntax_flags & AC_SBR_PRESENT) != 0 { 1 } else { 0 },
            &mut tns_conf,
            &h_psy.psy_conf[1],
            tns_mask & 1,
            tns_mask & 4,
        );
        h_psy.psy_conf[1].tns_conf = tns_conf;
        if error_status != AAC_ENC_OK {
            return error_status;
        }
    }

    for i in 0..cm.n_elements as usize {
        for ch in 0..cm.el_info[i].n_channels_in_el as usize {
            // checked at the start (psy_element_statics)
            let idx = h_psy.psy_element[i].psy_static[ch];
            let psy_static = &mut h_psy.p_static_channels[idx];
            if init_flags != 0 {
                // reset states
                fdk_aac_enc_psy_init_states(psy_static, audio_object_type);
            }

            fdk_aac_enc_init_pre_echo_control(
                &mut psy_static.sfb_thresholdnm1,
                &mut psy_static.calc_pre_echo,
                h_psy.psy_conf[0].sfb_cnt,
                &h_psy.psy_conf[0].sfb_pcm_quant_threshold,
                &mut psy_static.mdct_scalenm1,
            );
        }
    }

    {
        let conf = &mut h_psy.psy_conf[0];
        let sfb_offset = conf.sfb_offset;
        let is_lc = (conf.filterbank == FB_LC) as i32;
        error_status = fdk_aac_enc_init_pns_configuration(
            &mut conf.pns_conf,
            div(bit_rate, channels_eff),
            sample_rate,
            use_pns,
            conf.sfb_cnt,
            &sfb_offset,
            cm.el_info[0].n_channels_in_el,
            is_lc,
        );
    }
    if error_status != AAC_ENC_OK {
        return error_status;
    }

    if granule_length > 512 {
        let conf = &mut h_psy.psy_conf[1];
        let sfb_offset = conf.sfb_offset;
        let is_lc = (conf.filterbank == FB_LC) as i32;
        // Note: C passes cm->elInfo[1].nChannelsInEl, also for a single element.
        error_status = fdk_aac_enc_init_pns_configuration(
            &mut conf.pns_conf,
            div(bit_rate, channels_eff),
            sample_rate,
            use_pns,
            conf.sfb_cnt,
            &sfb_offset,
            cm.el_info[1].n_channels_in_el,
            is_lc,
        );
        if error_status != AAC_ENC_OK {
            return error_status;
        }
    }

    error_status
}

/// `spectrum[line] <<= finalShift` for `line < lowpassLine` in `FDKaacEnc_psyMain`.
///
/// `finalShift` is negative in rare cases (short blocks with spectral values of at least 2^30,
/// where `nrgShift--` gives -1; `FDK_ASSERT(finalShift >= 0)` is compiled out), which is
/// undefined behavior. Both reference builds vectorize the loop, and the vector shift
/// instructions treat a negative amount differently from the scalar remainder loop:
///
/// - AArch64: the first `lowpassLine & ~3` values use `ushl.4s` with the amount in the signed
///   low byte, which shifts right logically for negative amounts; the remaining values use
///   `lsl` (amount modulo 32).
/// - x86-64: for `lowpassLine >= 8`, the first `lowpassLine & ~7` values use `pslld` with the
///   zero-extended 32-bit amount, which gives 0 for amounts above 31; the remaining values use
///   `shll %cl` (amount modulo 32).
/// - i686 (`x86_32`, no SSE): every value uses `shll %cl` (amount modulo 32).
fn shift_spectrum_left(spectrum: &mut [FixpDbl], lowpass_line: i32, final_shift: i32) {
    let n = lowpass_line.max(0) as usize;
    if (0..32).contains(&final_shift) {
        for v in spectrum[..n].iter_mut() {
            *v <<= final_shift;
        }
        return;
    }
    let vec_len = match ARCH {
        Arch::Aarch64 | Arch::Generic => n & !3,
        Arch::X86 => {
            if n >= 8 {
                n & !7
            } else {
                0
            }
        }
        // ref-i686 has no SSE: the loop is scalar (`shll %cl` unrolled four times).
        Arch::X86_32 => 0,
    };
    for (i, v) in spectrum[..n].iter_mut().enumerate() {
        if i < vec_len {
            *v = match ARCH {
                Arch::Aarch64 | Arch::Generic => {
                    // ushl.4s: signed low byte of the amount, negative = logical right shift.
                    let s = final_shift as i8 as i32;
                    if s >= 0 {
                        if s >= 32 {
                            0
                        } else {
                            ((*v as u32) << s) as i32
                        }
                    } else if -s >= 32 {
                        0
                    } else {
                        ((*v as u32) >> -s) as i32
                    }
                }
                // pslld with a count above 31 gives 0.
                Arch::X86 => 0,
                Arch::X86_32 => v.wrapping_shl(final_shift as u32),
            };
        } else {
            *v = v.wrapping_shl(final_shift as u32);
        }
    }
}

thread_local! {
    static NEGATIVE_LOWPASS_CLEARS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn psy_main_negative_lowpass_clears() -> u64 {
    NEGATIVE_LOWPASS_CLEARS.with(|c| c.get())
}

/// Adds to the counter of [`psy_main_negative_lowpass_clears`] (the carry of the thread pool).
pub(crate) fn add_psy_main_negative_lowpass_clears(n: u64) {
    NEGATIVE_LOWPASS_CLEARS.with(|c| c.set(c.get().wrapping_add(n)));
}

#[cold]
#[inline(never)]
fn negative_lowpass_clear() {
    NEGATIVE_LOWPASS_CLEARS.with(|c| c.set(c.get() + 1));
}

type SpectrumHook = Option<fn(&mut [FixpDbl])>;

thread_local! {
    static NEGATIVE_FINAL_SHIFTS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
    static SPECTRUM_HOOK: core::cell::Cell<SpectrumHook> = const { core::cell::Cell::new(None) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn psy_main_negative_final_shifts() -> u64 {
    NEGATIVE_FINAL_SHIFTS.with(|c| c.get())
}

/// Adds to the counter of [`psy_main_negative_final_shifts`] (the carry of the thread pool).
pub(crate) fn add_psy_main_negative_final_shifts(n: u64) {
    NEGATIVE_FINAL_SHIFTS.with(|c| c.set(c.get().wrapping_add(n)));
}

#[cold]
#[inline(never)]
fn negative_final_shift() {
    NEGATIVE_FINAL_SHIFTS.with(|c| c.set(c.get() + 1));
}

/// Test hook: `hook` is applied to the spectrum of every channel (the
/// `frameLength` values of all windows) right after the transform in [`fdk_aac_enc_psy_main`].
/// The transform of 16-bit PCM does not produce the spectral values that make `finalShift`
/// negative; the differential test produces them with this hook and the same operation in a
/// copy of the reference's psy_main.cpp. `None` (the default) disables it.
#[doc(hidden)]
pub fn psy_main_set_spectrum_hook(hook: Option<fn(&mut [FixpDbl])>) {
    SPECTRUM_HOOK.with(|c| c.set(hook));
}

/// The hook of [`psy_main_set_spectrum_hook`] (the carry of the thread pool).
pub(crate) fn psy_main_spectrum_hook() -> Option<fn(&mut [FixpDbl])> {
    SPECTRUM_HOOK.with(|c| c.get())
}

thread_local! {
    static STALE_SHORT_BLOCKS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn psy_main_stale_short_blocks() -> u64 {
    STALE_SHORT_BLOCKS.with(|c| c.get())
}

/// Adds to the counter of [`psy_main_stale_short_blocks`] (the carry of the thread pool).
pub(crate) fn add_psy_main_stale_short_blocks(n: u64) {
    STALE_SHORT_BLOCKS.with(|c| c.set(c.get().wrapping_add(n)));
}

thread_local! {
    static MIXED_BLOCK_PAIRS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn psy_main_mixed_block_pairs() -> u64 {
    MIXED_BLOCK_PAIRS.with(|c| c.get())
}

/// Adds to the counter of [`psy_main_mixed_block_pairs`] (the carry of the thread pool).
pub(crate) fn add_psy_main_mixed_block_pairs(n: u64) {
    MIXED_BLOCK_PAIRS.with(|c| c.set(c.get().wrapping_add(n)));
}

thread_local! {
    static UNDEFINED_SPECTRUM_READS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn psy_main_undefined_spectrum_reads() -> u64 {
    UNDEFINED_SPECTRUM_READS.with(|c| c.get())
}

/// Adds to the counter of [`psy_main_undefined_spectrum_reads`] (the carry of the thread pool).
pub(crate) fn add_psy_main_undefined_spectrum_reads(n: u64) {
    UNDEFINED_SPECTRUM_READS.with(|c| c.set(c.get().wrapping_add(n)));
}

thread_local! {
    static LFE_FLAG_PAIRS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn psy_main_lfe_flag_pairs() -> u64 {
    LFE_FLAG_PAIRS.with(|c| c.get())
}

/// Adds to the counter of [`psy_main_lfe_flag_pairs`] (the carry of the thread pool).
pub(crate) fn add_psy_main_lfe_flag_pairs(n: u64) {
    LFE_FLAG_PAIRS.with(|c| c.set(c.get().wrapping_add(n)));
}

/// `FDKaacEnc_psyMain(channels, psyElement, psyDynamic, psyConf, psyOutElement, pInput,
/// inputBufSize, chIdx, totalChannels)`: psychoacoustic model of one element.
///
/// This function assumes that enough input data is in the modulo buffer.
///
/// Port parameters: `psy_static` are the `channels` static states of the element
/// (`psyElement->psyStatic[]`); `psy_out_channel` and `qc_out_channel` are the `channels`
/// channels of the element (`psyOutElement->psyOutChannel[]` and the `QC_OUT_CHANNEL`s that hold
/// the arrays `PSY_OUT_CHANNEL` points to, including the spectrum `psyData[ch]->mdctSpectrum`).
///
/// The encoder outputs do not depend on the contents of `PSY_DYNAMIC` before the call.
pub fn fdk_aac_enc_psy_main(
    channels: i32,
    psy_static: &mut [PsyStatic],
    psy_dynamic: &mut PsyDynamic,
    psy_conf: &mut [PsyConfiguration; 2],
    psy_out_element: &mut PsyOutElement,
    psy_out_channel: &mut [PsyOutChannel],
    qc_out_channel: &mut [QcOutChannel],
    p_input: &[IntPcm],
    input_buf_size: u32,
    ch_idx: &[i32],
    total_channels: i32,
) -> AacEncoderError {
    let _ = total_channels;
    let common_window: i32 = 1;
    let mut max_sfb_per_group = [0i32; 2];
    let mut mdct_spectrum_e: i32 = 0;
    let n_ch = channels as usize;

    let mut sfb_tonality = [[0 as FixpSgl; MAX_SFB_LONG]; 2];

    let mut zero_spec = TRUE; // means all spectral lines are zero

    let mut this_conf = [0usize; 2]; // hThisPsyConf: 0 = long, 1 = short
    let mut window_length = [0i32; 2];
    let mut n_windows = [0i32; 2];
    let mut max_sfb = [0i32; 2];
    let mut is_short_window = [0i32; 2];

    // number of incoming time samples to be processed
    let n_time_samples = psy_conf[0].granule_length;

    let block_switching_offset: i32 = match psy_conf[0].filterbank {
        FB_LC => n_time_samples + (9 * n_time_samples / (2 * TRANS_FAC)),
        FB_LD | FB_ELD => n_time_samples,
        _ => return AAC_ENC_UNSUPPORTED_FILTERBANK,
    };

    // Where the reference has no defined result, the port returns AAC_ENC_UNKNOWN before it
    // changes any state (deviation; see also `psy_main_negative_lowpass_clears`).
    {
        let long = &psy_conf[0];
        let short = &psy_conf[1];
        if !(0..=n_time_samples).contains(&long.lowpass_line)
            || !(0..=n_time_samples).contains(&long.lowpass_line_lfe)
            || (long.filterbank == FB_LC
                && !(0..=n_time_samples / TRANS_FAC).contains(&short.lowpass_line))
        {
            return AAC_ENC_UNKNOWN;
        }
    }

    let psy_data = &mut psy_dynamic.psy_data;
    let tns_data = &mut psy_dynamic.tns_data;
    let pns_data = &mut psy_dynamic.pns_data;

    // psyData[ch]->mdctSpectrum = psyOutChannel[ch]->mdctSpectrum: qc_out_channel[ch].mdct_spectrum

    let input_offset = |ch: usize, off: i32| -> usize {
        (ch_idx[ch] as i64 * input_buf_size as i64 + off as i64) as usize
    };

    // block switching
    if psy_conf[0].filterbank != FB_ELD {
        let mut p_time_signal = [[0 as IntPcm; 1024]; 2];
        let n = n_time_samples as usize;
        for ch in 0..n_ch {
            // copy input data and use for block switching
            let src = input_offset(ch, 0);
            p_time_signal[ch][..n].copy_from_slice(&p_input[src..src + n]);
        }
        if n_ch == 2 {
            // both channels at once (the same results as one call per channel)
            let (l, r) = pair(psy_static);
            fdk_aac_enc_block_switching_pair(
                [&mut l.block_switching_control, &mut r.block_switching_control],
                n_time_samples,
                [l.is_lfe, r.is_lfe],
                [&p_time_signal[0], &p_time_signal[1]],
            );
        }
        for ch in 0..n_ch {
            let st = &mut psy_static[ch];
            if n_ch != 2 {
                fdk_aac_enc_block_switching(
                    &mut st.block_switching_control,
                    n_time_samples,
                    st.is_lfe,
                    &p_time_signal[ch],
                );
            }

            // fill up internal input buffer, to 2xframelength samples
            let bso = block_switching_offset as usize;
            let len = (2 * n_time_samples - block_switching_offset) as usize;
            st.psy_input_buffer[bso..bso + len].copy_from_slice(&p_time_signal[ch][..len]);
        }

        // synch left and right block type
        let err = if channels > 1 {
            let (l, r) = pair(psy_static);
            fdk_aac_enc_sync_block_switching(
                &mut l.block_switching_control,
                Some(&mut r.block_switching_control),
                channels,
                common_window,
            )
        } else {
            fdk_aac_enc_sync_block_switching(
                &mut psy_static[0].block_switching_control,
                None,
                channels,
                common_window,
            )
        };

        if err != 0 {
            return AAC_ENC_UNSUPPORTED_AOT; // mixed up LC and LD
        }
    } else {
        for ch in 0..n_ch {
            // copy input data and use for block switching
            let src = input_offset(ch, 0);
            let n = n_time_samples as usize;
            let bso = block_switching_offset as usize;
            psy_static[ch].psy_input_buffer[bso..bso + n].copy_from_slice(&p_input[src..src + n]);
        }
    }

    for ch in 0..n_ch {
        is_short_window[ch] =
            (psy_static[ch].block_switching_control.last_window_sequence == SHORT_WINDOW) as i32;
    }
    if (0..n_ch).any(|ch| is_short_window[ch] != 0) && psy_conf[1].granule_length != n_time_samples
    {
        STALE_SHORT_BLOCKS.with(|c| c.set(c.get() + 1));
    }
    // Where the reference has no defined result (deviation, see psy_main_mixed_block_pairs), the
    // port refuses the frame.
    if n_ch == 2 && is_short_window[0] != is_short_window[1] {
        MIXED_BLOCK_PAIRS.with(|c| c.set(c.get() + 1));
        return AAC_ENC_UNKNOWN;
    }

    // set parameters according to window length
    for ch in 0..n_ch {
        if is_short_window[ch] != 0 {
            this_conf[ch] = 1;
            window_length[ch] = psy_conf[0].granule_length / TRANS_FAC;
            n_windows[ch] = TRANS_FAC;
            max_sfb[ch] = MAX_SFB_SHORT as i32;
        } else {
            this_conf[ch] = 0;
            window_length[ch] = psy_conf[0].granule_length;
            n_windows[ch] = 1;
            max_sfb[ch] = MAX_GROUPED_SFB as i32;
        }
        // pSfbMaxScaleSpec[ch], pSfbEnergy[ch], ... point to .Short[0] or .Long, both at index
        // 0 of the flat unions; window w is at w * maxSfb[ch].
    }

    // Transform and get mdctScaling for all channels and windows.
    for ch in 0..n_ch {
        let conf = &psy_conf[this_conf[ch]];
        let pd = &mut psy_data[ch];
        let st = &mut psy_static[ch];
        let spec = &mut qc_out_channel[ch].mdct_spectrum;

        // update number of active bands
        if st.is_lfe != 0 {
            pd.sfb_active = conf.sfb_active_lfe;
            pd.lowpass_line = conf.lowpass_line_lfe;
        } else {
            pd.sfb_active = conf.sfb_active;
            pd.lowpass_line = conf.lowpass_line;
        }

        if conf.filterbank == FB_ELD {
            let bsc = &mut st.block_switching_control;
            if fdk_aac_enc_transform_real_eld(
                &st.psy_input_buffer,
                spec,
                bsc.last_window_sequence,
                bsc.window_shape,
                &mut bsc.last_window_shape,
                n_time_samples,
                &mut mdct_spectrum_e,
                conf.filterbank,
                &mut st.overlap_add_buffer,
            ) != 0
            {
                return AAC_ENC_UNSUPPORTED_FILTERBANK;
            }
        } else {
            let bsc = &mut st.block_switching_control;
            if fdk_aac_enc_transform_real(
                &st.psy_input_buffer,
                spec,
                bsc.last_window_sequence,
                bsc.window_shape,
                &mut bsc.last_window_shape,
                &mut st.mdct_pers,
                n_time_samples,
                &mut mdct_spectrum_e,
                conf.filterbank,
            ) != 0
            {
                return AAC_ENC_UNSUPPORTED_FILTERBANK;
            }
        }
        if let Some(hook) = SPECTRUM_HOOK.with(|c| c.get()) {
            hook(&mut spec[..n_time_samples.max(0) as usize]);
        }

        for w in 0..n_windows[ch] {
            let w_offset = w * window_length[ch];

            // Low pass / highest sfb
            let start = (pd.lowpass_line + w_offset) as usize;
            if pd.lowpass_line > window_length[ch] {
                negative_lowpass_clear();
            }
            let len = (window_length[ch] - pd.lowpass_line).max(0) as usize;
            spec[start..start + len].fill(0);

            if (psy_conf[0].filterbank != FB_LC) && (pd.lowpass_line >= FADE_OUT_LEN as i32) {
                // Do blending to reduce gibbs artifacts
                for i in 0..FADE_OUT_LEN {
                    let k = (pd.lowpass_line + w_offset - FADE_OUT_LEN as i32 + i as i32) as usize;
                    spec[k] = f_mult(spec[k], FADE_OUT_FACTOR[i]);
                }
            }

            // Check for zero spectrum. These loops will usually terminate very, very early.
            let mut line = 0;
            while (line < pd.lowpass_line) && (zero_spec == TRUE) {
                if spec[(line + w_offset) as usize] != 0 {
                    zero_spec = FALSE;
                    break;
                }
                line += 1;
            }
        } // w loop

        pd.mdct_scale = mdct_spectrum_e;

        // rotate internal time samples
        let nts = n_time_samples as usize;
        st.psy_input_buffer.copy_within(nts..2 * nts, 0);

        // ... and get remaining samples from input buffer
        let src = input_offset(ch, 2 * n_time_samples - block_switching_offset);
        let len = (block_switching_offset - n_time_samples) as usize;
        st.psy_input_buffer[nts..nts + len].copy_from_slice(&p_input[src..src + len]);
    } // ch

    // Do some rescaling to get maximum possible accuracy for energies
    if zero_spec == FALSE {
        // Calc possible spectrum leftshift for each sfb (1 means: 1 bit left shift is possible
        // without overflow)
        let mut min_spec_shift: i32 = MAX_SHIFT_DBL;
        let mut nrg_shift: i32 = MAX_SHIFT_DBL;
        let mut final_shift: i32;
        let mut curr_nrg: FixpDbl;
        let mut max_nrg: FixpDbl = 0;

        for ch in 0..n_ch {
            let conf = &psy_conf[this_conf[ch]];
            let pd = &mut psy_data[ch];
            for w in 0..n_windows[ch] {
                let w_offset = (w * window_length[ch]) as usize;
                let m = (w * max_sfb[ch]) as usize;
                fdk_aac_enc_calc_sfb_max_scale_spec(
                    &qc_out_channel[ch].mdct_spectrum[w_offset..],
                    &conf.sfb_offset,
                    &mut pd.sfb_max_scale_spec[m..],
                    pd.sfb_active,
                );

                for sfb in 0..pd.sfb_active as usize {
                    min_spec_shift = fix_min(min_spec_shift, pd.sfb_max_scale_spec[m + sfb]);
                }
            }
        }

        // Calc possible energy leftshift for each sfb (1 means: 1 bit left shift is possible
        // without overflow)
        for ch in 0..n_ch {
            let conf = &psy_conf[this_conf[ch]];
            let pd = &mut psy_data[ch];
            for w in 0..n_windows[ch] {
                let w_offset = (w * window_length[ch]) as usize;
                let m = (w * max_sfb[ch]) as usize;
                curr_nrg = fdk_aac_enc_check_band_energy_optim(
                    &qc_out_channel[ch].mdct_spectrum[w_offset..],
                    &pd.sfb_max_scale_spec[m..],
                    &conf.sfb_offset,
                    pd.sfb_active,
                    &mut pd.sfb_energy[m..],
                    &mut pd.sfb_energy_ld_data[m..],
                    min_spec_shift - 4,
                );

                max_nrg = fix_max(max_nrg, curr_nrg);
            }
        }

        if max_nrg != 0 {
            nrg_shift = (count_leading_bits(max_nrg) >> 1) + (min_spec_shift - 4);
        }

        // 2check: Hasn't this decision to be made for both channels?
        // For short windows 1 additional bit headroom is necessary to prevent overflows when
        // summing up energies in FDKaacEnc_groupShortData()
        if is_short_window[0] != 0 {
            nrg_shift -= 1;
        }

        // both spectrum and energies mustn't overflow
        final_shift = fix_min(min_spec_shift, nrg_shift);

        // do not shift more than 3 bits more to the left than signal without blockfloating
        // point would be to avoid overflow of scaled PCM quantization thresholds
        if final_shift > psy_data[0].mdct_scale + 3 {
            final_shift = psy_data[0].mdct_scale + 3;
        }

        fdk_assert!(final_shift >= 0); // right shift is not allowed
        if final_shift < 0 {
            negative_final_shift();
        }

        // correct sfbEnergy and sfbEnergyLdData with new finalShift
        // FIXP_DBL ldShift = finalShift * FL2FXCONST_DBL(2.0 / 64);
        let ld_shift: FixpDbl = final_shift.wrapping_mul(fl2fxconst_dbl(2.0 / 64.0));
        for ch in 0..n_ch {
            let pd = &mut psy_data[ch];
            let max_sfb_ch = max_sfb[ch] as usize;
            let mut w_max_sfb_ch: usize = 0;
            for _w in 0..n_windows[ch] {
                for sfb in 0..pd.sfb_active as usize {
                    let k = w_max_sfb_ch + sfb;
                    let mut scale = fix_max(0, pd.sfb_max_scale_spec[k] - 4);
                    scale = fix_min((scale - final_shift) << 1, DFRACT_BITS - 1);
                    if scale >= 0 {
                        pd.sfb_energy[k] >>= scale;
                    } else {
                        // -scale <= 8 here; the left shift of the energy can overflow (scalar
                        // `lsl` in both builds).
                        pd.sfb_energy[k] = pd.sfb_energy[k].wrapping_shl((-scale) as u32);
                    }
                    pd.sfb_threshold[k] = f_mult(pd.sfb_energy[k], C_RATIO);
                    pd.sfb_energy_ld_data[k] = pd.sfb_energy_ld_data[k].wrapping_add(ld_shift);
                }
                w_max_sfb_ch += max_sfb_ch;
            }
        }

        if final_shift != 0 {
            for ch in 0..n_ch {
                let pd = &mut psy_data[ch];
                let w_len = window_length[ch];
                let lowpass_line = pd.lowpass_line;
                let mut w_offset: i32 = 0;
                let mdct_spectrum = &mut qc_out_channel[ch].mdct_spectrum;
                for w in 0..n_windows[ch] {
                    let spectrum = &mut mdct_spectrum[w_offset as usize..];
                    shift_spectrum_left(spectrum, lowpass_line, final_shift);
                    w_offset += w_len;

                    // update sfbMaxScaleSpec
                    let m = (w * max_sfb[ch]) as usize;
                    for sfb in 0..pd.sfb_active as usize {
                        pd.sfb_max_scale_spec[m + sfb] -= final_shift;
                    }
                }
                // update mdctScale
                pd.mdct_scale -= final_shift;
            }
        }
    } else {
        // all spectral lines are zero
        for ch in 0..n_ch {
            let pd = &mut psy_data[ch];
            // otherwise mdctScale would be for example 7 and PCM quantization thresholds would
            // be shifted 14 bits to the right causing some of them to become 0 (which causes
            // problems later)
            pd.mdct_scale = 0;
            // clear sfbMaxScaleSpec
            for w in 0..n_windows[ch] {
                let m = (w * max_sfb[ch]) as usize;
                for sfb in 0..pd.sfb_active as usize {
                    pd.sfb_max_scale_spec[m + sfb] = 0;
                    pd.sfb_energy[m + sfb] = 0;
                    pd.sfb_energy_ld_data[m + sfb] = fl2fxconst_dbl(-1.0);
                    pd.sfb_threshold[m + sfb] = 0;
                }
            }
        }
    }

    // Advance psychoacoustics: Tonality and TNS
    if (channels >= 1) && (psy_static[0].is_lfe != 0) {
        *tns_data[0].data_raw.tns_active_mut(TnsDataRaw::LONG_SUB_BLOCK_INFO, HIFILT) = 0;
        *tns_data[0].data_raw.tns_active_mut(TnsDataRaw::LONG_SUB_BLOCK_INFO, LOFILT) = 0;
    } else {
        for ch in 0..n_ch {
            if is_short_window[ch] == 0 {
                let conf = &psy_conf[this_conf[ch]];
                let pd = &psy_data[ch];
                // tonality
                fdk_aac_enc_calculate_full_tonality(
                    &qc_out_channel[ch].mdct_spectrum,
                    &pd.sfb_max_scale_spec,
                    &pd.sfb_energy_ld_data,
                    &mut sfb_tonality[ch],
                    pd.sfb_active,
                    &conf.sfb_offset,
                    conf.pns_conf.use_pns,
                );
            }
        } // ch

        if psy_conf[0].tns_conf.tns_active != 0 || psy_conf[1].tns_conf.tns_active != 0 {
            let mut tns_active = [0i32; TRANS_FAC as usize];
            let mut nrg_scaling = [0i32; 2];
            let mut tns_spec_shift: i32 = 0;

            for ch in 0..n_ch {
                let conf = &psy_conf[this_conf[ch]];
                for w in 0..n_windows[ch] {
                    let w_offset = (w * window_length[ch]) as usize;
                    // TNS
                    fdk_aac_enc_tns_detect(
                        &mut tns_data[ch],
                        &conf.tns_conf,
                        &mut psy_out_channel[ch].tns_info,
                        conf.sfb_cnt,
                        &qc_out_channel[ch].mdct_spectrum[w_offset..],
                        w,
                        psy_static[ch].block_switching_control.last_window_sequence,
                    );
                }
            }

            if channels == 2 {
                let (td0, td1) = pair(&mut tns_data[..]);
                let (po0, po1) = pair(psy_out_channel);
                fdk_aac_enc_tns_sync(
                    td1,
                    td0,
                    &mut po1.tns_info,
                    &po0.tns_info,
                    psy_static[1].block_switching_control.last_window_sequence,
                    psy_static[0].block_switching_control.last_window_sequence,
                    &psy_conf[this_conf[1]].tns_conf,
                );
            }

            if channels >= 1 {
                // all checks for TNS do only work for common windows (which is always set)
                fdk_assert!(1 == common_window);
                let last = n_ch - 1;
                for w in 0..n_windows[0] as usize {
                    let o = if is_short_window[0] != 0 {
                        TnsDataRaw::short_sub_block_info(w)
                    } else {
                        TnsDataRaw::LONG_SUB_BLOCK_INFO
                    };
                    tns_active[w] = (tns_data[0].data_raw.tns_active(o, HIFILT) != 0
                        || tns_data[0].data_raw.tns_active(o, LOFILT) != 0
                        || tns_data[last].data_raw.tns_active(o, HIFILT) != 0
                        || tns_data[last].data_raw.tns_active(o, LOFILT) != 0)
                        as i32;
                }
            }

            for ch in 0..n_ch {
                if tns_active[0] != 0 && is_short_window[ch] == 0 {
                    // Scale down spectrum if tns is active in one of the two channels with same
                    // lastWindowSequence
                    // first part of threshold calculation; it's not necessary to update
                    // sfbMaxScaleSpec
                    let shift = 1;
                    let conf = &psy_conf[this_conf[ch]];
                    let pd = &mut psy_data[ch];
                    let spec = &mut qc_out_channel[ch].mdct_spectrum;
                    for sfb in 0..conf.lowpass_line.max(0) as usize {
                        spec[sfb] >>= shift;
                    }

                    // update thresholds
                    for sfb in 0..pd.sfb_active as usize {
                        pd.sfb_threshold[sfb] >>= 2 * shift;
                    }

                    pd.mdct_scale += shift; // update mdctScale

                    // calc sfbEnergies after tnsEncode again !
                }
            }

            for ch in 0..n_ch {
                let conf = &psy_conf[this_conf[ch]];
                let pd = &mut psy_data[ch];
                for w in 0..n_windows[ch] {
                    let w_offset = (w * window_length[ch]) as usize;
                    fdk_aac_enc_tns_encode(
                        &psy_out_channel[ch].tns_info,
                        &tns_data[ch],
                        conf.sfb_cnt,
                        &conf.tns_conf,
                        // hThisPsyConf[ch]->lowpassLine: filter stops before that line !
                        conf.sfb_offset[pd.sfb_active as usize],
                        &mut qc_out_channel[ch].mdct_spectrum[w_offset..],
                        w,
                        psy_static[ch].block_switching_control.last_window_sequence,
                    );

                    if tns_active[w as usize] != 0 {
                        // Calc sfb-bandwise mdct-energies for left and right channel again,
                        // if tns active in current channel or in one channel with same
                        // lastWindowSequence left and right
                        let m = (w * max_sfb[ch]) as usize;
                        fdk_aac_enc_calc_sfb_max_scale_spec(
                            &qc_out_channel[ch].mdct_spectrum[w_offset..],
                            &conf.sfb_offset,
                            &mut pd.sfb_max_scale_spec[m..],
                            pd.sfb_active,
                        );
                    }
                }
            }

            for ch in 0..n_ch {
                let conf = &psy_conf[this_conf[ch]];
                let pd = &mut psy_data[ch];
                for w in 0..n_windows[ch] {
                    if tns_active[w as usize] != 0 {
                        if is_short_window[ch] != 0 {
                            let m = (w * max_sfb[ch]) as usize;
                            fdk_aac_enc_calc_band_energy_optim_short(
                                &qc_out_channel[ch].mdct_spectrum
                                    [(w * window_length[ch]) as usize..],
                                &pd.sfb_max_scale_spec[m..],
                                &conf.sfb_offset,
                                pd.sfb_active,
                                &mut pd.sfb_energy[m..],
                            );
                        } else {
                            // with tns, energy calculation can overflow; -> scaling
                            nrg_scaling[ch] = fdk_aac_enc_calc_band_energy_optim_long(
                                &qc_out_channel[ch].mdct_spectrum,
                                &pd.sfb_max_scale_spec,
                                &conf.sfb_offset,
                                pd.sfb_active,
                                &mut pd.sfb_energy,
                                &mut pd.sfb_energy_ld_data,
                            );
                            // nrgScaling is set only if nrg would have an overflow
                            tns_spec_shift = fix_max(tns_spec_shift, nrg_scaling[ch]);
                        }
                    } // if tnsActive
                }
            } // end channel loop

            // adapt scaling to prevent nrg overflow, only for long blocks
            for ch in 0..n_ch {
                if (tns_spec_shift != 0) && is_short_window[ch] == 0 {
                    let conf = &psy_conf[this_conf[ch]];
                    let pd = &mut psy_data[ch];
                    // scale down spectrum, nrg's and thresholds, if there was an overflow in
                    // sfbNrg calculation after tns
                    let spec = &mut qc_out_channel[ch].mdct_spectrum;
                    for line in 0..conf.lowpass_line.max(0) as usize {
                        spec[line] >>= tns_spec_shift;
                    }
                    let scale = (tns_spec_shift - nrg_scaling[ch]) << 1;
                    for sfb in 0..pd.sfb_active as usize {
                        // pSfbEnergyLdData[ch][sfb] -= scale * FL2FXCONST_DBL(1.0 /
                        // LD_DATA_SCALING);
                        pd.sfb_energy_ld_data[sfb] = pd.sfb_energy_ld_data[sfb].wrapping_sub(
                            scale.wrapping_mul(fl2fxconst_dbl(1.0 / LD_DATA_SCALING as f64)),
                        );
                        pd.sfb_energy[sfb] >>= scale;
                        pd.sfb_threshold[sfb] >>= tns_spec_shift << 1;
                    }
                    // update mdctScale; not necessary to update sfbMaxScaleSpec
                    pd.mdct_scale += tns_spec_shift;
                }
            } // end channel loop
        }
        // TNS active
        else {
            // In case of disable TNS, reset its dynamic data. Some of its elements is required
            // in PNS detection below.
            tns_data[0] = TnsData::default();
            tns_data[1] = TnsData::default();
        }
    } // !isLFE

    // Advance thresholds
    for ch in 0..n_ch {
        let conf = &psy_conf[this_conf[ch]];
        let pd = &mut psy_data[ch];
        let st = &mut psy_static[ch];
        let headroom: i32;

        let clip_energy: FixpDbl;
        let mut energy_shift = pd.mdct_scale * 2;
        let clip_nrg_shift = energy_shift - THR_SHIFTBITS;
        if is_short_window[ch] != 0 {
            headroom = 6;
        } else {
            headroom = 0;
        }

        if clip_nrg_shift >= 0 {
            // clipEnergy >> clipNrgShift: the amount can exceed 31 (undefined); both builds
            // use a scalar shift by register (amount modulo 32).
            clip_energy = conf.clip_energy.wrapping_shr(clip_nrg_shift as u32);
        } else if clip_nrg_shift >= -headroom {
            clip_energy = conf.clip_energy.wrapping_shl((-clip_nrg_shift) as u32);
        } else {
            clip_energy = MAXVAL_DBL;
        }

        for w in 0..n_windows[ch] {
            let m = (w * max_sfb[ch]) as usize;
            // limit threshold to avoid clipping
            for i in 0..pd.sfb_active as usize {
                pd.sfb_threshold[m + i] = fix_min(pd.sfb_threshold[m + i], clip_energy);
            }

            // spreading
            fdk_aac_enc_spreading_max(
                pd.sfb_active,
                &conf.sfb_mask_low_factor,
                &conf.sfb_mask_high_factor,
                &mut pd.sfb_threshold[m..],
            );

            // PCM quantization threshold
            energy_shift += PCM_QUANT_THR_SCALE;
            if energy_shift >= 0 {
                energy_shift = fix_min(DFRACT_BITS - 1, energy_shift);
                for i in 0..pd.sfb_active as usize {
                    pd.sfb_threshold[m + i] = fix_max(
                        pd.sfb_threshold[m + i] >> THR_SHIFTBITS,
                        conf.sfb_pcm_quant_threshold[i] >> energy_shift,
                    );
                }
            } else {
                energy_shift = fix_min(DFRACT_BITS - 1, -energy_shift);
                for i in 0..pd.sfb_active as usize {
                    pd.sfb_threshold[m + i] = fix_max(
                        pd.sfb_threshold[m + i] >> THR_SHIFTBITS,
                        conf.sfb_pcm_quant_threshold[i].wrapping_shl(energy_shift as u32),
                    );
                }
            }

            if st.is_lfe == 0 {
                // preecho control
                if st.block_switching_control.last_window_sequence == STOP_WINDOW {
                    // prevent FDKaacEnc_PreEchoControl from comparing stop thresholds with
                    // short thresholds
                    for i in 0..pd.sfb_active as usize {
                        st.sfb_thresholdnm1[i] = MAXVAL_DBL;
                    }

                    st.mdct_scalenm1 = 0;
                    st.calc_pre_echo = 0;
                }

                fdk_aac_enc_pre_echo_control(
                    &mut st.sfb_thresholdnm1,
                    st.calc_pre_echo,
                    pd.sfb_active,
                    conf.max_allowed_increase_factor,
                    conf.min_remaining_threshold_factor,
                    &mut pd.sfb_threshold[m..],
                    pd.mdct_scale,
                    &mut st.mdct_scalenm1,
                );

                st.calc_pre_echo = 1;

                if st.block_switching_control.last_window_sequence == START_WINDOW {
                    // prevent FDKaacEnc_PreEchoControl in next frame to compare start
                    // thresholds with short thresholds
                    for i in 0..pd.sfb_active as usize {
                        st.sfb_thresholdnm1[i] = MAXVAL_DBL;
                    }

                    st.mdct_scalenm1 = 0;
                    st.calc_pre_echo = 0;
                }
            }

            // spread energy to avoid hole detection
            let n = pd.sfb_active as usize;
            let (energy, spread) = (&pd.sfb_energy, &mut pd.sfb_spread_energy);
            spread[m..m + n].copy_from_slice(&energy[m..m + n]);

            fdk_aac_enc_spreading_max(
                pd.sfb_active,
                &conf.sfb_mask_low_factor_spr_en,
                &conf.sfb_mask_high_factor_spr_en,
                &mut pd.sfb_spread_energy[m..],
            );
        }
    }

    // Calc bandwise energies for mid and side channel. Do it only if 2 channels exist
    if channels == 2 {
        let conf1 = &psy_conf[this_conf[1]];
        let last_ws1 = psy_static[1].block_switching_control.last_window_sequence;
        let (pd0, pd1) = pair(&mut psy_data[..]);
        for w in 0..n_windows[1] {
            let w_offset = (w * window_length[1]) as usize;
            let m0 = (w * max_sfb[0]) as usize;
            let m1 = (w * max_sfb[1]) as usize;
            fdk_aac_enc_calc_band_nrg_ms_opt(
                &qc_out_channel[0].mdct_spectrum[w_offset..],
                &qc_out_channel[1].mdct_spectrum[w_offset..],
                &pd0.sfb_max_scale_spec[m0..],
                &pd1.sfb_max_scale_spec[m1..],
                &conf1.sfb_offset,
                pd0.sfb_active,
                &mut pd0.sfb_energy_ms[m0..],
                &mut pd1.sfb_energy_ms[m1..],
                (last_ws1 != SHORT_WINDOW) as i32,
                &mut pd0.sfb_energy_ms_ld_data,
                &mut pd1.sfb_energy_ms_ld_data,
            );
        }
    }

    // group short data (maxSfb[ch] for short blocks is determined here)
    for ch in 0..n_ch {
        let pd = &mut psy_data[ch];
        let poc = &mut psy_out_channel[ch];
        let qoc = &mut qc_out_channel[ch];
        if is_short_window[ch] != 0 {
            let conf_short = &psy_conf[1];
            let bsc = &psy_static[ch].block_switching_control;
            let no_sfb = bsc.no_of_groups * conf_short.sfb_cnt;
            // At this point, energies and thresholds are copied/regrouped from the ".Short"
            // to the ".Long" arrays
            fdk_aac_enc_group_short_data(
                &mut qoc.mdct_spectrum,
                &mut pd.sfb_threshold,
                &mut pd.sfb_energy,
                &mut pd.sfb_energy_ms,
                &mut pd.sfb_spread_energy,
                conf_short.sfb_cnt,
                pd.sfb_active,
                &conf_short.sfb_offset,
                &conf_short.sfb_min_snr_ld_data,
                &mut pd.grouped_sfb_offset,
                &mut max_sfb_per_group[ch],
                &mut qoc.sfb_min_snr_ld_data,
                bsc.no_of_groups,
                &bsc.group_len,
                psy_conf[1].granule_length,
            );

            let n = pd.sfb_active as usize;
            let step = conf_short.sfb_cnt.max(1) as usize;

            // calculate ldData arrays (short values are in .Long-arrays after
            // FDKaacEnc_groupShortData)
            let mut sfb_grp: usize = 0;
            while (sfb_grp as i32) < no_sfb {
                ld_data_vector(
                    &pd.sfb_energy[sfb_grp..sfb_grp + n],
                    &mut qoc.sfb_energy_ld_data[sfb_grp..],
                );
                sfb_grp += step;
            }

            // calc sfbThrld and set Values smaller 2^-31 to 2^-33
            sfb_grp = 0;
            while (sfb_grp as i32) < no_sfb {
                ld_data_vector(
                    &pd.sfb_threshold[sfb_grp..sfb_grp + n],
                    &mut qoc.sfb_threshold_ld_data[sfb_grp..],
                );
                for sfb in 0..n {
                    qoc.sfb_threshold_ld_data[sfb_grp + sfb] = fix_max(
                        qoc.sfb_threshold_ld_data[sfb_grp + sfb],
                        fl2fxconst_dbl(-0.515625),
                    );
                }
                sfb_grp += step;
            }

            if channels == 2 {
                sfb_grp = 0;
                while (sfb_grp as i32) < no_sfb {
                    let src = pd.sfb_energy_ms;
                    ld_data_vector(
                        &src[sfb_grp..sfb_grp + n],
                        &mut pd.sfb_energy_ms_ld_data[sfb_grp..],
                    );
                    sfb_grp += step;
                }
            }

            poc.sfb_offsets = pd.grouped_sfb_offset;
        } else {
            let conf_long = &psy_conf[0];
            // maxSfb[ch] for long blocks
            let mut sfb = pd.sfb_active - 1;
            while sfb >= 0 {
                let mut line = conf_long.sfb_offset[(sfb + 1) as usize] - 1;
                while line >= conf_long.sfb_offset[sfb as usize] {
                    if qoc.mdct_spectrum[line as usize] != 0 {
                        break;
                    }
                    line -= 1;
                }
                if line > conf_long.sfb_offset[sfb as usize] {
                    break;
                }
                sfb -= 1;
            }
            max_sfb_per_group[ch] = sfb + 1;
            max_sfb_per_group[ch] = fix_max(fix_min(5, pd.sfb_active), max_sfb_per_group[ch]);

            let n = pd.sfb_active as usize;
            // sfbNrgLdData is calculated in FDKaacEnc_advancePsychLong, copy in psyOut
            // structure
            qoc.sfb_energy_ld_data[..n].copy_from_slice(&pd.sfb_energy_ld_data[..n]);

            // FDKmemcpy(psyOutChannel[ch]->sfbOffsets, hPsyConfLong->sfbOffset,
            //           (MAX_GROUPED_SFB + 1) * sizeof(INT));
            // copies 61 values from sfbOffset[MAX_SFB + 1] (52 values): the last 9 come from
            // the members that follow it in PSY_CONFIGURATION (filterbank,
            // sfbPcmQuantThreshold[0..8]).
            psy_conf_sfb_offsets_61(conf_long, &mut poc.sfb_offsets);

            // sfbMinSnrLdData modified in adjust threshold, copy necessary
            qoc.sfb_min_snr_ld_data[..n].copy_from_slice(&conf_long.sfb_min_snr_ld_data[..n]);

            // sfbEnergyMSLdData ist already calculated in FDKaacEnc_CalcBandNrgMSOpt; only in
            // long case

            // calc sfbThrld and set Values smaller 2^-31 to 2^-33
            ld_data_vector(&pd.sfb_threshold[..n], &mut qoc.sfb_threshold_ld_data);
            for i in 0..n {
                qoc.sfb_threshold_ld_data[i] =
                    fix_max(qoc.sfb_threshold_ld_data[i], fl2fxconst_dbl(-0.515625));
            }
        }
    }

    // Intensity parameter intialization.
    for ch in 0..n_ch {
        psy_out_channel[ch].is_book = [0; MAX_GROUPED_SFB];
        psy_out_channel[ch].is_scale = [0; MAX_GROUPED_SFB];
    }

    for ch in 0..n_ch {
        let win = if is_short_window[ch] != 0 { 1 } else { 0 };
        if psy_static[ch].is_lfe == 0 {
            let pd = &mut psy_data[ch];
            let poc = &mut psy_out_channel[ch];
            let qoc = &mut qc_out_channel[ch];
            let tns_order = poc.tns_info.order[0][0];
            let raw = &tns_data[ch].data_raw;
            let tns_prediction_gain = raw.prediction_gain(TnsDataRaw::LONG_SUB_BLOCK_INFO, HIFILT);
            let tns_active = raw.tns_active(TnsDataRaw::LONG_SUB_BLOCK_INFO, HIFILT);
            let sfb_offset = psy_conf[win].sfb_offset;
            let tonality_reads = crate::noisedet::noise_detect_tonality_reads();
            // PNS Decision
            fdk_aac_enc_pns_detect(
                &psy_conf[0].pns_conf,
                &mut pns_data[ch],
                psy_static[ch].block_switching_control.last_window_sequence,
                pd.sfb_active,
                max_sfb_per_group[ch], // count of Sfb which are not zero.
                &qoc.sfb_threshold_ld_data,
                &sfb_offset,
                &qoc.mdct_spectrum,
                &pd.sfb_max_scale_spec,
                &sfb_tonality[ch],
                tns_order,
                tns_prediction_gain,
                tns_active,
                &qoc.sfb_energy_ld_data,
                &mut poc.noise_nrg,
            );
            // Counted where the reference has no defined result (deviation, see
            // psy_main_lfe_flag_pairs).
            if psy_static[0].is_lfe != 0
                && crate::noisedet::noise_detect_tonality_reads() != tonality_reads
            {
                add_psy_main_lfe_flag_pairs(1);
            }
        } // !isLFE
    } // ch

    // stereo Processing
    if channels == 2 {
        psy_out_element.tools_info.ms_digest = MS_NONE;
        psy_out_element.common_window = common_window;
        if psy_out_element.common_window != 0 {
            let m = fix_max(max_sfb_per_group[0], max_sfb_per_group[1]);
            max_sfb_per_group[0] = m;
            max_sfb_per_group[1] = m;
        }
        let allow_is = (psy_conf[0].allow_is != 0 && psy_out_element.common_window != 0) as i32;
        let short = psy_static[0].block_switching_control.last_window_sequence == SHORT_WINDOW;
        let (is_sfb_cnt, is_sfb_per_group) = if !short {
            (psy_conf[0].sfb_cnt, psy_conf[0].sfb_cnt)
        } else {
            (
                psy_static[0].block_switching_control.no_of_groups * psy_conf[1].sfb_cnt,
                psy_conf[1].sfb_cnt,
            )
        };
        // sfbOffset of IntensityStereoProcessing: psyConf[0].sfbOffset for long blocks,
        // psyData[0]->groupedSfbOffset for short blocks.
        // (a copy on the stack, as in pool.rs: no allocation per frame)
        let is_sfb_offset_buf: [i32; MAX_GROUPED_SFB + 1];
        let is_sfb_offset: &[i32] = if !short {
            &psy_conf[0].sfb_offset
        } else {
            is_sfb_offset_buf = psy_data[0].grouped_sfb_offset;
            &is_sfb_offset_buf
        };

        if !short {
            // PNS preprocessing depending on ms processing: PNS not in Short Window!
            let (pd0, pd1) = pair(&mut psy_data[..]);
            let (qc0, qc1) = pair(qc_out_channel);
            let (pns0, pns1) = pair(&mut pns_data[..]);
            fdk_aac_enc_pre_process_pns_channel_pair(
                pd0.sfb_active,
                &pd0.sfb_energy,
                &pd1.sfb_energy,
                &qc0.sfb_energy_ld_data,
                &qc1.sfb_energy_ld_data,
                &pd0.sfb_energy_ms,
                &psy_conf[0].pns_conf,
                pns0,
                pns1,
            );
        }

        {
            let (pd0, pd1) = pair(&mut psy_data[..]);
            let (qc0, qc1) = pair(qc_out_channel);
            let (pns0, pns1) = pair(&mut pns_data[..]);
            let po1 = &mut psy_out_channel[1];
            fdk_aac_enc_intensity_stereo_processing(
                &pd0.sfb_energy,
                &mut pd1.sfb_energy,
                &mut qc0.mdct_spectrum,
                &mut qc1.mdct_spectrum,
                &pd0.sfb_threshold,
                &mut pd1.sfb_threshold,
                &mut qc1.sfb_threshold_ld_data,
                &pd0.sfb_spread_energy,
                &mut pd1.sfb_spread_energy,
                &qc0.sfb_energy_ld_data,
                &mut qc1.sfb_energy_ld_data,
                &mut psy_out_element.tools_info.ms_digest,
                &mut psy_out_element.tools_info.ms_mask,
                is_sfb_cnt,
                is_sfb_per_group,
                max_sfb_per_group[0],
                is_sfb_offset,
                allow_is,
                &mut po1.is_book,
                &mut po1.is_scale,
                [Some(pns0), Some(pns1)],
            );
        }

        // For short blocks it's OK to pass the ".Long" arrays here. They contain grouped short
        // data since FDKaacEnc_groupShortData()
        let sfb_offsets0 = psy_out_channel[0].sfb_offsets;
        let is_book1 = psy_out_channel[1].is_book;
        let (allow_ms, ms_sfb_cnt, ms_sfb_per_group) = if !short {
            (psy_conf[0].allow_ms, psy_data[0].sfb_active, psy_data[0].sfb_active)
        } else {
            (psy_conf[1].allow_ms, is_sfb_cnt, psy_conf[1].sfb_cnt)
        };
        fdk_aac_enc_ms_stereo_processing(
            &mut psy_data[..2],
            &mut qc_out_channel[..2],
            Some(&is_book1),
            &mut psy_out_element.tools_info.ms_digest,
            &mut psy_out_element.tools_info.ms_mask,
            allow_ms,
            ms_sfb_cnt,
            ms_sfb_per_group,
            max_sfb_per_group[0],
            &sfb_offsets0,
        );

        if !short {
            // PNS postprocessing
            let (pns0, pns1) = pair(&mut pns_data[..]);
            fdk_aac_enc_post_process_pns_channel_pair(
                psy_data[0].sfb_active,
                &psy_conf[0].pns_conf,
                pns0,
                pns1,
                &mut psy_out_element.tools_info.ms_mask,
                &mut psy_out_element.tools_info.ms_digest,
            );
        }
    } // (channels == 2)

    // PNS Coding
    for ch in 0..n_ch {
        let pd = &mut psy_data[ch];
        if psy_static[ch].is_lfe != 0 {
            // no PNS coding
            for sfb in 0..pd.sfb_active as usize {
                psy_out_channel[ch].noise_nrg[sfb] = NO_NOISE_PNS;
            }
        } else {
            let conf = &mut psy_conf[this_conf[ch]];
            fdk_aac_enc_code_pns_channel(
                pd.sfb_active,
                &conf.pns_conf,
                &pns_data[ch].pns_flag,
                &pd.sfb_energy_ld_data,
                // this is the energy that will be written to the bitstream
                &mut psy_out_channel[ch].noise_nrg,
                &mut qc_out_channel[ch].sfb_threshold_ld_data,
            );
        }
    }

    // build output
    for ch in 0..n_ch {
        let pd = &psy_data[ch];
        let poc = &mut psy_out_channel[ch];
        let bsc = &psy_static[ch].block_switching_control;
        poc.max_sfb_per_group = max_sfb_per_group[ch];
        poc.mdct_scale = pd.mdct_scale;
        if is_short_window[ch] == 0 {
            poc.sfb_cnt = psy_conf[0].sfb_active;
            poc.sfb_per_group = psy_conf[0].sfb_active;
            poc.last_window_sequence = bsc.last_window_sequence;
            poc.window_shape = bsc.window_shape;
        } else {
            let sfb_cnt = bsc.no_of_groups * psy_conf[1].sfb_cnt;

            poc.sfb_cnt = sfb_cnt;
            poc.sfb_per_group = psy_conf[1].sfb_cnt;
            poc.last_window_sequence = SHORT_WINDOW;
            poc.window_shape = SINE_WINDOW;
        }
        // generate grouping mask
        let mut mask: i32 = 0;
        for grp in 0..bsc.no_of_groups as usize {
            mask <<= 1;
            for _j in 1..bsc.group_len[grp] {
                mask = (mask << 1) | 1;
            }
        }
        poc.grouping_mask = mask;

        // build interface
        poc.group_len.copy_from_slice(&bsc.group_len[..MAX_NO_OF_GROUPS]);
        let qoc = &mut qc_out_channel[ch];
        qoc.sfb_energy.copy_from_slice(&pd.sfb_energy[..MAX_GROUPED_SFB]);
        qoc.sfb_spread_energy.copy_from_slice(&pd.sfb_spread_energy[..MAX_GROUPED_SFB]);
    }

    AAC_ENC_OK
}

/// `FDKaacEnc_PsyClose(phPsyInternal, phPsyOut)`: frees the psychoacoustic model.
pub fn fdk_aac_enc_psy_close(
    ph_psy_internal: Option<&mut Option<Box<PsyInternal>>>,
    ph_psy_out: Option<&mut [Option<Box<PsyOut>>]>,
) {
    if let Some(p) = ph_psy_internal {
        *p = None;
    }
    if let Some(p) = ph_psy_out {
        for o in p.iter_mut().take(1) {
            *o = None;
        }
    }
}
