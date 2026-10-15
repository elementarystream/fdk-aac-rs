// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/include/sbrdecoder.h, libSBRdec/src/sbrdecoder.cpp.

//! Port of `sbrdecoder.h` and `sbrdecoder.cpp`: the API of the SBR decoder.
//!
//! # Notes for the glue (the AAC decoder)
//!
//! * `HANDLE_SBRDECODER` is `Box<SbrDecoderInstance>`; [`sbr_decoder_open`] creates it in an
//!   `Option` (`*pSelf`), [`sbr_decoder_close`] drops it. The functions take the instance as
//!   `&mut SbrDecoderInstance`: the `self == NULL` checks of the C code are not reachable.
//! * The QMF domain (`pQmfDomain`, owned by the AAC decoder) is not stored in the instance:
//!   every function that uses it ([`sbr_decoder_init_element`], [`sbr_decoder_header`],
//!   [`sbr_decoder_apply`]) takes it as `&mut FdkQmfDomain`. The SBR channels refer to its
//!   channels by index (`SbrDec::qmf_domain_in_ch`/`qmf_domain_out_ch`).
//! * [`sbr_decoder_apply`]: `input` is the core output (one block of `codecFrameSize` samples
//!   per channel), `time_data` the output buffer (`timeDataSize` is passed as in C and checked
//!   against the frame size; the slice must hold the output). For the second call of
//!   `aacDecoder_DecodeFrame()` (MPEG Surround with `SBR_SKIP_QMF`, `input == timeData` in C)
//!   the input is not read: pass an empty `input` slice.
//! * [`sbr_decoder_parse`]: `pDrmBsBuffer`/`drmBsBufferSize` are the slice `drm_bs_buffer`
//!   (only used for DRM, at least 512 bytes in the reference). `acElFlags` is a slice indexed by
//!   `elementIndex`.
//! * [`sbr_decoder_get_lib_info`]: `build_date`/`build_time` are empty strings (the reference
//!   uses `__DATE__`/`__TIME__`).
//! * The PS scratch memory (`pPsScratch`, `struct PS_DEC_COEFFICIENTS`) is a local of
//!   `sbrDecoder_DecodeElement()`.

use crate::env_dec::decode_sbr_data;
use crate::env_extr::*;
use crate::lpp_tran::TransposerSettings;
use crate::psbitdec::decode_ps;
use crate::psdec::*;
use crate::sbr_dec::*;
use crate::sbr_ram::*;
use crate::sbrdec_drc::*;
use crate::sbrdec_freq_sca::reset_freq_band_tables;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::crc::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::qmf::{QMF_FLAG_CLDFB, QMF_FLAG_KEEP_STATES, QMF_FLAG_LP, QMF_FLAG_MPSLDFB};
use fdk_aac_rs_fdk::qmf_domain::*;
use fdk_aac_rs_fdk::syslib_channel_map_descr::*;

/// `SBR_ERROR`
pub type SbrError = i32;
/// `SBRDEC_OK`: all fine.
pub const SBRDEC_OK: SbrError = 0;
/// `SBRDEC_INVALID_ARGUMENT`
pub const SBRDEC_INVALID_ARGUMENT: SbrError = 1;
/// `SBRDEC_CREATE_ERROR`
pub const SBRDEC_CREATE_ERROR: SbrError = 2;
/// `SBRDEC_NOT_INITIALIZED`
pub const SBRDEC_NOT_INITIALIZED: SbrError = 3;
/// `SBRDEC_MEM_ALLOC_FAILED`: memory allocation failed.
pub const SBRDEC_MEM_ALLOC_FAILED: SbrError = 4;
/// `SBRDEC_PARSE_ERROR`
pub const SBRDEC_PARSE_ERROR: SbrError = 5;
/// `SBRDEC_UNSUPPORTED_CONFIG`
pub const SBRDEC_UNSUPPORTED_CONFIG: SbrError = 6;
/// `SBRDEC_SET_PARAM_FAIL`
pub const SBRDEC_SET_PARAM_FAIL: SbrError = 7;
/// `SBRDEC_OUTPUT_BUFFER_TOO_SMALL`
pub const SBRDEC_OUTPUT_BUFFER_TOO_SMALL: SbrError = 8;

/// `SBRDEC_PARAM`
pub type SbrdecParam = i32;
/// `SBR_SYSTEM_BITSTREAM_DELAY`: switch to enable an additional SBR bitstream delay of one
/// frame.
pub const SBR_SYSTEM_BITSTREAM_DELAY: SbrdecParam = 0;
/// `SBR_QMF_MODE`: set QMF mode, either complex or low power.
pub const SBR_QMF_MODE: SbrdecParam = 1;
/// `SBR_LD_QMF_TIME_ALIGN`: set QMF type, either LD-MPS or CLDFB (ELD only).
pub const SBR_LD_QMF_TIME_ALIGN: SbrdecParam = 2;
/// `SBR_FLUSH_DATA`: flush the decoder with the next process call.
pub const SBR_FLUSH_DATA: SbrdecParam = 3;
/// `SBR_CLEAR_HISTORY`: clear all internal states (delay lines, QMF states, ...).
pub const SBR_CLEAR_HISTORY: SbrdecParam = 4;
/// `SBR_BS_INTERRUPTION`: signal bit stream interruption. Value is ignored.
pub const SBR_BS_INTERRUPTION: SbrdecParam = 5;
/// `SBR_SKIP_QMF`: enable skipping of QMF step: 1 skip analysis, 2 skip synthesis.
pub const SBR_SKIP_QMF: SbrdecParam = 6;

/// `(8)`: maximum number of SBR elements.
pub const SBRDEC_MAX_ELEMENTS: usize = 8;
/// `(1)`: maximum number of additional delay frames (`(1) + 1` slots).
pub const SBRDEC_MAX_DELAY_FRAMES: usize = 1;

/// `SBRDECODER_LIB_VL0`
const SBRDECODER_LIB_VL0: i32 = 3;
/// `SBRDECODER_LIB_VL1`
const SBRDECODER_LIB_VL1: i32 = 1;
/// `SBRDECODER_LIB_VL2`
const SBRDECODER_LIB_VL2: i32 = 0;
/// `SBRDECODER_LIB_TITLE`
const SBRDECODER_LIB_TITLE: &str = "SBR Decoder";
/// `SBRDECODER_LIB_BUILD_DATE`: `__DATE__` in the reference (deviation: empty).
const SBRDECODER_LIB_BUILD_DATE: &str = "";
/// `SBRDECODER_LIB_BUILD_TIME`: `__TIME__` in the reference (deviation: empty).
const SBRDECODER_LIB_BUILD_TIME: &str = "";

/// `SBR_DECODER_ELEMENT`
#[derive(Clone, Debug)]
pub struct SbrDecoderElement {
    /// `pSbrChannel[SBRDEC_MAX_CH_PER_ELEMENT]`
    pub p_sbr_channel: [Option<Box<SbrChannel>>; SBRDEC_MAX_CH_PER_ELEMENT],
    /// `transposerSettings`: common transport settings for each individual channel of an
    /// element
    pub transposer_settings: TransposerSettings,
    /// `elementID`: element ID set during initialization
    pub element_id: Mp4ElementId,
    /// `nChannels`: number of elements output channels (=2 in case of PS)
    pub n_channels: i32,
    /// `frameErrorFlag`: frame error status for every slot in the delay line
    pub frame_error_flag: [u8; SBRDEC_MAX_DELAY_FRAMES + 1],
    /// `useFrameSlot`: index which defines which slot will be decoded/filled next
    pub use_frame_slot: u8,
    /// `useHeaderSlot`: link between header and frame data
    pub use_header_slot: [u8; SBRDEC_MAX_DELAY_FRAMES + 1],
}

impl Default for SbrDecoderElement {
    /// The zeroed structure of `GetRam_SbrDecElement()` (`FDKcalloc`).
    fn default() -> Self {
        SbrDecoderElement {
            p_sbr_channel: [None, None],
            transposer_settings: TransposerSettings::ZERO,
            element_id: 0,
            n_channels: 0,
            frame_error_flag: [0; SBRDEC_MAX_DELAY_FRAMES + 1],
            use_frame_slot: 0,
            use_header_slot: [0; SBRDEC_MAX_DELAY_FRAMES + 1],
        }
    }
}

/// `struct SBR_DECODER_INSTANCE` (`HANDLE_SBRDECODER` is `Box<SbrDecoderInstance>`).
#[derive(Clone, Debug)]
pub struct SbrDecoderInstance {
    /// `pSbrElement[(8)]`
    pub p_sbr_element: [Option<Box<SbrDecoderElement>>; SBRDEC_MAX_ELEMENTS],
    /// `sbrHeader[(8)][(1) + 1]`: SBR header for each individual channel of an element
    pub sbr_header: [[SbrHeaderData; SBRDEC_MAX_DELAY_FRAMES + 1]; SBRDEC_MAX_ELEMENTS],
    /// `hParametricStereoDec`
    pub h_parametric_stereo_dec: Option<Box<PsDec>>,
    /// `coreCodec`: AOT of core codec
    pub core_codec: AudioObjectType,
    /// `numSbrElements`
    pub num_sbr_elements: i32,
    /// `numSbrChannels`
    pub num_sbr_channels: i32,
    /// `sampleRateIn`: SBR decoder input sampling rate
    pub sample_rate_in: i32,
    /// `sampleRateOut`: sampling rate of the SBR decoder output audio samples
    pub sample_rate_out: i32,
    /// `codecFrameSize`
    pub codec_frame_size: u16,
    /// `synDownsampleFac`
    pub syn_downsample_fac: u8,
    /// `downscaleFactor`
    pub downscale_factor: i32,
    /// `numDelayFrames`: the current number of additional delay frames used for processing
    pub num_delay_frames: u8,
    /// `harmonicSBR`
    pub harmonic_sbr: u8,
    /// `numFlushedFrames`: the number of frames which are flushed consecutively
    pub num_flushed_frames: u8,
    /// `flags`
    pub flags: u32,
    /// `sbrInDataHeadroom`: headroom of the SBR input time signal to prevent clipping
    pub sbr_in_data_headroom: i32,
    /// Not in C: the scratch memory of the per channel parallelism of [`sbr_decoder_apply_pool`]
    /// (allocated on its first use).
    pub par_scratch: Option<Box<SbrParScratch>>,
}

/// Not in C: the scratch memory of the second channel of a stereo element when the two channels
/// of the element are decoded at the same time ([`sbr_decoder_apply_pool`]).
#[derive(Clone, Debug, Default)]
pub struct SbrParScratch {
    /// The QMF domain of the second channel: its input and output channel and its overlap
    /// buffer (moved in from the decoder's QMF domain for the duration of the call), and work
    /// buffer sections of its own.
    qd: FdkQmfDomain,
    /// The output buffer of the second channel.
    out: Vec<i32>,
}

/// Not in C: the per channel parallelism of [`sbr_decoder_apply_pool`].
#[derive(Clone, Copy, Debug)]
pub struct SbrPar<'a> {
    /// The thread pool (at least two threads).
    pub pool: &'a fdk_aac_rs_fdk::par::ThreadPool,
    /// Test builds: fill the work buffer sections of both channels with a pattern before the
    /// channels are decoded (if a channel read QMF data an earlier channel or frame left, the
    /// output would change). 0: off; otherwise the seed of the pattern.
    pub poison: u64,
    /// Test builds (negative control of the poisoning): fill the overlap buffer of the second
    /// channel with a pattern before it is decoded (changes the output whenever the parallel
    /// path runs).
    pub poison_overlap: bool,
}

/// The number of stereo SBR elements whose channels [`sbr_decoder_apply_pool`] decoded at the
/// same time (all threads of the process; for tests).
static PAR_ELEMENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The number of stereo SBR elements whose channels were decoded at the same time in this
/// process (see [`sbr_decoder_apply_pool`]).
pub fn par_elements() -> u64 {
    PAR_ELEMENTS.load(std::sync::atomic::Ordering::Relaxed)
}

impl Default for SbrDecoderInstance {
    /// The zeroed structure of `GetRam_SbrDecoder()` (`FDKcalloc`).
    fn default() -> Self {
        SbrDecoderInstance {
            p_sbr_element: Default::default(),
            sbr_header: [[SbrHeaderData::ZERO; SBRDEC_MAX_DELAY_FRAMES + 1]; SBRDEC_MAX_ELEMENTS],
            h_parametric_stereo_dec: None,
            core_codec: 0,
            num_sbr_elements: 0,
            num_sbr_channels: 0,
            sample_rate_in: 0,
            sample_rate_out: 0,
            codec_frame_size: 0,
            syn_downsample_fac: 0,
            downscale_factor: 0,
            num_delay_frames: 0,
            harmonic_sbr: 0,
            num_flushed_frames: 0,
            flags: 0,
            sbr_in_data_headroom: 0,
            par_scratch: None,
        }
    }
}

/// `setFrameErrorFlag()`
fn set_frame_error_flag(p_sbr_element: Option<&mut SbrDecoderElement>, value: u8) {
    if let Some(el) = p_sbr_element {
        match value {
            FRAME_ERROR_ALLSLOTS => {
                el.frame_error_flag = [FRAME_ERROR; SBRDEC_MAX_DELAY_FRAMES + 1];
            }
            _ => {
                let slot = (el.use_frame_slot as usize).min(SBRDEC_MAX_DELAY_FRAMES);
                el.frame_error_flag[slot] = value;
            }
        }
    }
}

/// `getHeaderSlot()`: a free header slot.
fn get_header_slot(current_slot: u8, hdr_slot_usage: &[u8; SBRDEC_MAX_DELAY_FRAMES + 1]) -> u8 {
    let mut occupied: u32 = 0;
    let mut slot = hdr_slot_usage[(current_slot as usize).min(SBRDEC_MAX_DELAY_FRAMES)];

    fdk_assert!((1) + 1 < 32);

    for s in 0..SBRDEC_MAX_DELAY_FRAMES + 1 {
        if hdr_slot_usage[s] == slot && s != slot as usize {
            occupied = 1;
            break;
        }
    }

    if occupied != 0 {
        occupied = 0;

        for s in 0..SBRDEC_MAX_DELAY_FRAMES + 1 {
            occupied |= 1u32.wrapping_shl(hdr_slot_usage[s] as u32);
        }
        for s in 0..SBRDEC_MAX_DELAY_FRAMES + 1 {
            if occupied & 0x1 == 0 {
                slot = s as u8;
                break;
            }
            occupied >>= 1;
        }
    }

    slot
}

/// `copySbrHeader()`: copies the whole header (the frequency band table pointers point into
/// the header itself, see [`FreqBandData`]).
fn copy_sbr_header(h_dst: &mut SbrHeaderData, h_src: &SbrHeaderData) {
    *h_dst = *h_src;
}

/// `compareSbrHeader()`: 0 if the headers are equal.
fn compare_sbr_header(h_hdr1: &SbrHeaderData, h_hdr2: &SbrHeaderData) -> i32 {
    let mut result = 0;
    let neq = |a: &[u8], b: &[u8]| (a != b) as i32;

    // compare basic data
    result |= (h_hdr1.sync_state != h_hdr2.sync_state) as i32;
    result |= (h_hdr1.status != h_hdr2.status) as i32;
    result |= (h_hdr1.frame_error_flag != h_hdr2.frame_error_flag) as i32;
    result |= (h_hdr1.number_time_slots != h_hdr2.number_time_slots) as i32;
    result |= (h_hdr1.number_of_analysis_bands != h_hdr2.number_of_analysis_bands) as i32;
    result |= (h_hdr1.time_step != h_hdr2.time_step) as i32;
    result |= (h_hdr1.sbr_proc_smpl_rate != h_hdr2.sbr_proc_smpl_rate) as i32;

    // compare bitstream data (FDKmemcmp: only zero/non-zero matters to the callers)
    result |= neq(pod_bytes(&h_hdr1.bs_data), pod_bytes(&h_hdr2.bs_data));
    result |= neq(pod_bytes(&h_hdr1.bs_dflt), pod_bytes(&h_hdr2.bs_dflt));
    result |= neq(pod_bytes(&h_hdr1.bs_info), pod_bytes(&h_hdr2.bs_info));

    // compare frequency band data
    let (f1, f2) = (&h_hdr1.freq_band_data, &h_hdr2.freq_band_data);
    const N: usize = 8 + MAX_NUM_LIMITERS_HDR + 1;
    result |= neq(&pod_bytes(f1)[..N], &pod_bytes(f2)[..N]);
    result |= neq(&f1.freq_band_table_lo, &f2.freq_band_table_lo);
    result |= neq(&f1.freq_band_table_hi, &f2.freq_band_table_hi);
    result |= neq(&f1.freq_band_table_noise, &f2.freq_band_table_noise);
    result |= neq(&f1.v_k_master, &f2.v_k_master);

    result
}

/// `MAX_NUM_LIMITERS` (for the size of the compared part of `FREQ_BAND_DATA`).
const MAX_NUM_LIMITERS_HDR: usize = crate::lpp_tran::MAX_NUM_LIMITERS;

/// `sbrDecoder_ResetElement()`: resets the SBR decoder of one element.
#[allow(clippy::too_many_arguments)]
fn sbr_decoder_reset_element(
    self_: &mut SbrDecoderInstance,
    qd: &mut FdkQmfDomain,
    sample_rate_in: i32,
    mut sample_rate_out: i32,
    samples_per_frame: i32,
    _element_id: Mp4ElementId,
    element_index: usize,
    overlap: i32,
) -> SbrError {
    let mut sbr_error = SBRDEC_OK;
    let mut qmf_flags: u32 = 0;

    // USAC: assuming theoretical case 8 kHz output sample rate with 4:1 SBR
    let sbr_min_sample_rate_in = if is_usac(self_.core_codec) { 2000 } else { 6400 };

    'bail: {
        // Check in/out samplerates
        if sample_rate_in < sbr_min_sample_rate_in || sample_rate_in > (96000) {
            sbr_error = SBRDEC_UNSUPPORTED_CONFIG;
            break 'bail;
        }

        if sample_rate_out > (96000) {
            sbr_error = SBRDEC_UNSUPPORTED_CONFIG;
            break 'bail;
        }

        // Set QMF mode flags
        if self_.flags & SBRDEC_LOW_POWER != 0 {
            qmf_flags |= QMF_FLAG_LP;
        }

        if self_.core_codec == AOT_ER_AAC_ELD {
            if self_.flags & SBRDEC_LD_MPS_QMF != 0 {
                qmf_flags |= QMF_FLAG_MPSLDFB;
            } else {
                qmf_flags |= QMF_FLAG_CLDFB;
            }
        }

        // Set downsampling factor for synthesis filter bank
        if sample_rate_out == 0 {
            // no single rate mode
            // In case of implicit signalling, assume dual rate SBR
            sample_rate_out = sample_rate_in << 1;
        }

        let syn_downsample_fac;
        if sample_rate_in == sample_rate_out {
            syn_downsample_fac = 2;
            self_.flags |= SBRDEC_DOWNSAMPLE;
        } else {
            syn_downsample_fac = 1;
            self_.flags &= !SBRDEC_DOWNSAMPLE;
        }

        self_.syn_downsample_fac = syn_downsample_fac as u8;
        self_.sample_rate_out = sample_rate_out;

        {
            for i in 0..SBRDEC_MAX_DELAY_FRAMES + 1 {
                let h_sbr_header = &mut self_.sbr_header[element_index][i];
                let set_dflt = (h_sbr_header.sync_state == SBR_NOT_INITIALIZED
                    || self_.flags & SBRDEC_FORCE_RESET != 0) as i32;

                // init a default header such that we can at least do upsampling later
                sbr_error = init_header_data(
                    h_sbr_header,
                    sample_rate_in,
                    sample_rate_out,
                    self_.downscale_factor,
                    samples_per_frame,
                    self_.flags,
                    set_dflt,
                );

                // Set synchState to UPSAMPLING in case it already is initialized
                h_sbr_header.sync_state = if h_sbr_header.sync_state > UPSAMPLING {
                    UPSAMPLING
                } else {
                    h_sbr_header.sync_state
                };
            }
        }

        if sbr_error != SBRDEC_OK {
            break 'bail;
        }

        if qd.global_conf.qmf_domain_explicit_config == 0 {
            let h0 = &self_.sbr_header[element_index][0];
            let gc = &mut qd.global_conf;
            gc.flags_requested |= qmf_flags;
            gc.n_bands_analysis_requested = h0.number_of_analysis_bands;
            gc.n_bands_synthesis_requested = if syn_downsample_fac == 1 { 64 } else { 32 }; // may be overwritten by MPS
            gc.n_bands_synthesis_requested = (gc.n_bands_synthesis_requested as i32)
                .checked_div(self_.downscale_factor)
                .unwrap_or(0) as u16;
            gc.n_qmf_time_slots_requested = h0.number_time_slots.wrapping_mul(h0.time_step);
            gc.n_qmf_ov_time_slots_requested = overlap as u8;
            gc.n_qmf_proc_bands_requested = 64; // always 64
            gc.n_qmf_proc_channels_requested = 1; // may be overwritten by MPS
        }

        // Init SBR channels going to be assigned to a SBR element
        {
            let SbrDecoderInstance { p_sbr_element, sbr_header, flags, codec_frame_size, .. } =
                &mut *self_;
            let Some(el) = p_sbr_element[element_index].as_deref_mut() else {
                break 'bail;
            };
            for ch in 0..el.n_channels.clamp(0, SBRDEC_MAX_CH_PER_ELEMENT as i32) as usize {
                let header_index = get_header_slot(el.use_frame_slot, &el.use_header_slot) as usize;

                // and create sbrDec
                let Some(chan) = el.p_sbr_channel[ch].as_deref_mut() else {
                    continue;
                };
                sbr_error = create_sbr_dec(
                    chan,
                    &mut sbr_header[element_index][header_index.min(SBRDEC_MAX_DELAY_FRAMES)],
                    &mut el.transposer_settings,
                    syn_downsample_fac,
                    qmf_flags,
                    *flags,
                    overlap,
                    ch as i32,
                    *codec_frame_size as i32,
                );
                if sbr_error != SBRDEC_OK {
                    break 'bail;
                }
            }
        }

        // FDKmemclear(sbr_OverlapBuffer, sizeof(sbr_OverlapBuffer));

        if self_.num_sbr_elements == 1 {
            match self_.core_codec {
                AOT_AAC_LC | AOT_SBR | AOT_PS | AOT_ER_AAC_SCAL | AOT_DRM_AAC
                | AOT_DRM_SURROUND => {
                    if create_ps_dec(&mut self_.h_parametric_stereo_dec, samples_per_frame) != 0 {
                        sbr_error = SBRDEC_CREATE_ERROR;
                        break 'bail;
                    }
                }
                _ => {}
            }
        }

        // Init frame delay slot handling
        if let Some(el) = self_.p_sbr_element[element_index].as_deref_mut() {
            el.use_frame_slot = 0;
            for i in 0..SBRDEC_MAX_DELAY_FRAMES + 1 {
                el.use_header_slot[i] = i as u8;
            }
        }
    }

    sbr_error
}

/// `sbrDecoder_AssignQmfChannels2SbrChannels()`: assigns the QMF domain channels to the SBR
/// channels.
fn sbr_decoder_assign_qmf_channels2_sbr_channels(self_: &mut SbrDecoderInstance) {
    let mut abs_ch_offset = 0usize;
    for el in 0..(self_.num_sbr_elements.clamp(0, SBRDEC_MAX_ELEMENTS as i32) as usize) {
        if let Some(e) = self_.p_sbr_element[el].as_deref_mut() {
            let n = e.n_channels.clamp(0, SBRDEC_MAX_CH_PER_ELEMENT as i32) as usize;
            for ch in 0..n {
                fdk_assert!(abs_ch_offset + ch < QMF_DOMAIN_MAX_CHANNELS);
                if let Some(c) = e.p_sbr_channel[ch].as_deref_mut() {
                    c.sbr_dec.qmf_domain_in_ch = Some(abs_ch_offset + ch);
                    c.sbr_dec.qmf_domain_out_ch = Some(abs_ch_offset + ch);
                }
            }
            abs_ch_offset += e.n_channels.max(0) as usize;
        }
    }
}

/// `sbrDecoder_Open()`: creates an SBR decoder instance (the QMF domain is passed to the
/// functions that use it, see the module documentation).
pub fn sbr_decoder_open(p_self: &mut Option<Box<SbrDecoderInstance>>) -> SbrError {
    // Get memory for this instance
    let mut self_ = Box::<SbrDecoderInstance>::default();

    self_.num_delay_frames = 1; // set to the max value by default

    // Initialize header sync state
    for el in self_.sbr_header.iter_mut() {
        for h in el.iter_mut() {
            h.sync_state = SBR_NOT_INITIALIZED;
        }
    }

    *p_self = Some(self_);

    SBRDEC_OK
}

/// `sbrDecoder_isCoreCodecValid()`: 1 if SBR can be processed with the core codec.
fn sbr_decoder_is_core_codec_valid(core_codec: AudioObjectType) -> bool {
    matches!(
        core_codec,
        AOT_AAC_LC
            | AOT_SBR
            | AOT_PS
            | AOT_ER_AAC_SCAL
            | AOT_ER_AAC_ELD
            | AOT_DRM_AAC
            | AOT_DRM_SURROUND
            | AOT_USAC
    )
}

/// `sbrDecoder_DestroyElement()`
fn sbr_decoder_destroy_element(self_: &mut SbrDecoderInstance, element_index: usize) {
    if let Some(mut el) = self_.p_sbr_element[element_index].take() {
        for ch in 0..SBRDEC_MAX_CH_PER_ELEMENT {
            if let Some(mut c) = el.p_sbr_channel[ch].take() {
                delete_sbr_dec(&mut c);
                self_.num_sbr_channels -= 1;
            }
        }
        self_.num_sbr_elements -= 1;
    }
}

/// `sbrDecoder_InitElement()`: initializes the SBR decoder for one element.
#[allow(clippy::too_many_arguments)]
pub fn sbr_decoder_init_element(
    self_: &mut SbrDecoderInstance,
    qd: &mut FdkQmfDomain,
    sample_rate_in: i32,
    sample_rate_out: i32,
    samples_per_frame: i32,
    core_codec: AudioObjectType,
    element_id: Mp4ElementId,
    element_index: i32,
    harmonic_sbr: u8,
    _stereo_config_index: u8,
    config_mode: u8,
    config_changed: &mut u8,
    downscale_factor: i32,
) -> SbrError {
    let mut sbr_error = SBRDEC_OK;
    let mut ch_cnt = 0;

    let n_sbr_elements_start = self_.num_sbr_elements;
    let n_sbr_channels_start = self_.num_sbr_channels;

    'bail: {
        // Check core codec AOT
        if !sbr_decoder_is_core_codec_valid(core_codec) || element_index >= (8) || element_index < 0
        {
            sbr_error = SBRDEC_UNSUPPORTED_CONFIG;
            break 'bail;
        }
        let ei = element_index as usize;

        if element_id != ID_SCE && element_id != ID_CPE && element_id != ID_LFE {
            sbr_error = SBRDEC_UNSUPPORTED_CONFIG;
            break 'bail;
        }

        if self_.sample_rate_in == sample_rate_in
            && self_.codec_frame_size as i32 == samples_per_frame
            && self_.core_codec == core_codec
            && self_.p_sbr_element[ei].as_ref().is_some_and(|e| e.element_id == element_id)
            && self_.flags & SBRDEC_FORCE_RESET == 0
            && (if sample_rate_out == 0 { true } else { self_.sample_rate_out == sample_rate_out })
            // The value 2 signalizes that harmonicSBR shall be ignored in the config change
            // detection
            && (if harmonic_sbr == 2 { true } else { self_.harmonic_sbr == harmonic_sbr })
        {
            // Nothing to do
            return SBRDEC_OK;
        } else if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
            *config_changed = 1;
        }

        // reaching this point the SBR-decoder gets (re-)configured

        // The flags field is used for all elements!
        // Keep the global flags. They will be reset after decoding.
        self_.flags &= SBRDEC_FORCE_RESET | SBRDEC_FLUSH;
        self_.flags |= if downscale_factor > 1 { SBRDEC_ELD_DOWNSCALE } else { 0 };
        self_.flags |= if core_codec == AOT_ER_AAC_ELD { SBRDEC_ELD_GRID } else { 0 };
        self_.flags |= if core_codec == AOT_ER_AAC_SCAL { SBRDEC_SYNTAX_SCAL } else { 0 };
        self_.flags |=
            if core_codec == AOT_DRM_AAC { SBRDEC_SYNTAX_SCAL | SBRDEC_SYNTAX_DRM } else { 0 };
        self_.flags |=
            if core_codec == AOT_DRM_SURROUND { SBRDEC_SYNTAX_SCAL | SBRDEC_SYNTAX_DRM } else { 0 };
        self_.flags |= if core_codec == AOT_USAC { SBRDEC_SYNTAX_USAC } else { 0 };
        // Robustness: Take integer division rounding into consideration. E.g. 22050 Hz with 4:1
        // SBR => 5512 Hz core sampling rate.
        self_.flags |= if sample_rate_in == sample_rate_out / 4 { SBRDEC_QUAD_RATE } else { 0 };
        self_.flags |= if harmonic_sbr == 1 { SBRDEC_USAC_HARMONICSBR } else { 0 };

        if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
            return SBRDEC_OK;
        }

        self_.sample_rate_in = sample_rate_in;
        self_.codec_frame_size = samples_per_frame as u16;
        self_.core_codec = core_codec;
        self_.harmonic_sbr = harmonic_sbr;
        self_.downscale_factor = downscale_factor;

        // Init SBR elements
        {
            if self_.p_sbr_element[ei].is_none() {
                self_.p_sbr_element[ei] = Some(Box::default());
                self_.num_sbr_elements += 1;
            } else {
                self_.num_sbr_channels -=
                    self_.p_sbr_element[ei].as_ref().map_or(0, |e| e.n_channels);
            }

            // Determine amount of channels for this element
            let mut el_channels = match element_id {
                ID_NONE | ID_CPE => 2,
                ID_LFE | ID_SCE => 1,
                _ => 0,
            };

            // Handle case of Parametric Stereo
            if ei == 0 && element_id == ID_SCE {
                match core_codec {
                    AOT_AAC_LC | AOT_SBR | AOT_PS | AOT_ER_AAC_SCAL | AOT_DRM_AAC
                    | AOT_DRM_SURROUND => {
                        el_channels = 2;
                    }
                    _ => {}
                }
            }

            let cur_channels = self_.p_sbr_element[ei].as_ref().map_or(0, |e| e.n_channels);
            // Sanity check to avoid memory leaks
            if el_channels < cur_channels || (self_.num_sbr_channels + el_channels) > (8) + (1) {
                self_.num_sbr_channels += cur_channels;
                sbr_error = SBRDEC_PARSE_ERROR;
                break 'bail;
            }

            // Save element ID for sanity checks and to have a fallback for concealment.
            let SbrDecoderInstance { p_sbr_element, num_sbr_channels, .. } = &mut *self_;
            let Some(el) = p_sbr_element[ei].as_deref_mut() else {
                break 'bail;
            };
            el.element_id = element_id;
            el.n_channels = el_channels;

            for ch in 0..el_channels as usize {
                if el.p_sbr_channel[ch].is_none() {
                    el.p_sbr_channel[ch] = Some(Box::default());
                }
                *num_sbr_channels += 1;

                if let Some(c) = el.p_sbr_channel[ch].as_deref_mut() {
                    sbr_decoder_drc_init_channel(Some(&mut c.sbr_dec.sbr_drc_channel));
                }

                ch_cnt += 1;
            }
        }
        let _ = ch_cnt;

        if qd.global_conf.qmf_domain_explicit_config == 0 {
            qd.global_conf.n_input_channels_requested = self_.num_sbr_channels as u8;
            qd.global_conf.n_output_channels_requested =
                f_max(self_.num_sbr_channels, qd.global_conf.n_output_channels_requested as i32)
                    as u8;
        }

        // Make sure each SBR channel has one QMF channel assigned even if numSbrChannels or
        // element set-up has changed.
        sbr_decoder_assign_qmf_channels2_sbr_channels(self_);

        // clear error flags for all delay slots
        if let Some(el) = self_.p_sbr_element[ei].as_deref_mut() {
            el.frame_error_flag = [0; SBRDEC_MAX_DELAY_FRAMES + 1];
        }

        {
            let overlap = if core_codec == AOT_ER_AAC_ELD {
                0
            } else if self_.flags & SBRDEC_QUAD_RATE != 0 {
                3 * 4
            } else {
                3 * 2
            };
            // Initialize this instance
            sbr_error = sbr_decoder_reset_element(
                self_,
                qd,
                sample_rate_in,
                sample_rate_out,
                samples_per_frame,
                element_id,
                ei,
                overlap,
            );
        }
    }

    if sbr_error != SBRDEC_OK {
        if n_sbr_elements_start < self_.num_sbr_elements
            || n_sbr_channels_start < self_.num_sbr_channels
        {
            // Free the memory allocated for this element
            if (0..8).contains(&element_index) {
                sbr_decoder_destroy_element(self_, element_index as usize);
            }
        } else if (0..8).contains(&element_index) {
            // Set error flag to trigger concealment
            set_frame_error_flag(
                self_.p_sbr_element[element_index as usize].as_deref_mut(),
                FRAME_ERROR,
            );
        }
    }

    sbr_error
}

/// `sbrDecoder_FreeMem()`: frees the configuration dependent memory.
pub fn sbr_decoder_free_mem(self_: Option<&mut SbrDecoderInstance>) -> SbrError {
    if let Some(s) = self_ {
        for i in 0..SBRDEC_MAX_ELEMENTS {
            sbr_decoder_destroy_element(s, i);
        }
        for el in s.sbr_header.iter_mut() {
            for h in el.iter_mut() {
                h.sync_state = SBR_NOT_INITIALIZED;
            }
        }
    }
    SBRDEC_OK
}

/// `sbrDecoder_HeaderUpdate()`: applies a decoded SBR header to one element.
fn sbr_decoder_header_update(
    flags: u32,
    h_sbr_header: &mut SbrHeaderData,
    header_status: SbrHeaderStatus,
) -> SbrError {
    // change of control data, reset decoder
    let error_status = reset_freq_band_tables(h_sbr_header, flags);

    if error_status == SBRDEC_OK {
        if h_sbr_header.sync_state == UPSAMPLING && header_status != HEADER_RESET {
            // As the default header would limit the frequency range, lowSubband and highSubband
            // must be patched.
            h_sbr_header.freq_band_data.low_subband = h_sbr_header.number_of_analysis_bands;
            h_sbr_header.freq_band_data.high_subband = h_sbr_header.number_of_analysis_bands;
        }

        // Trigger a reset before processing this slot
        h_sbr_header.status |= SBRDEC_HDR_STAT_RESET;
    }

    error_status
}

/// `sbrDecoder_Header()`: reads an SBR header of the configuration (ASC) and initializes the
/// element.
#[allow(clippy::too_many_arguments)]
pub fn sbr_decoder_header(
    self_: &mut SbrDecoderInstance,
    qd: &mut FdkQmfDomain,
    h_bs: &mut FdkBitStream,
    sample_rate_in: i32,
    sample_rate_out: i32,
    samples_per_frame: i32,
    core_codec: AudioObjectType,
    element_id: Mp4ElementId,
    element_index: i32,
    harmonic_sbr: u8,
    stereo_config_index: u8,
    config_mode: u8,
    config_changed: &mut u8,
    downscale_factor: i32,
) -> i32 {
    let mut sbr_error;
    // flags should not be changed in AC_CM_DET_CFG_CHANGE - mode after parsing
    let mut flags_saved: u32 = 0;

    if !(0..8).contains(&element_index) {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }
    let ei = element_index as usize;

    if !sbr_decoder_is_core_codec_valid(core_codec) {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
        flags_saved = self_.flags; // store
    }

    'bail: {
        sbr_error = sbr_decoder_init_element(
            self_,
            qd,
            sample_rate_in,
            sample_rate_out,
            samples_per_frame,
            core_codec,
            element_id,
            element_index,
            harmonic_sbr,
            stereo_config_index,
            config_mode,
            config_changed,
            downscale_factor,
        );

        if sbr_error != SBRDEC_OK || element_id == ID_LFE {
            break 'bail;
        }

        let header_index = if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
            None
        } else {
            // (None: no element, no defined result in C)
            self_.p_sbr_element[ei]
                .as_deref()
                .map(|el| get_header_slot(el.use_frame_slot, &el.use_header_slot) as usize)
        };

        let header_status = sbr_get_header_data(
            header_index.map(|h| &mut self_.sbr_header[ei][h.min(SBRDEC_MAX_DELAY_FRAMES)]),
            h_bs,
            self_.flags,
            0,
            config_mode,
        );

        if core_codec == AOT_USAC {
            if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
                self_.flags = flags_saved; // restore
            }
            return sbr_error;
        }

        if config_mode & AC_CM_ALLOC_MEM != 0 {
            let flags = self_.flags;
            let SbrDecoderInstance { p_sbr_element, sbr_header, .. } = &mut *self_;
            // Sanity check
            if let Some(el) = p_sbr_element[ei].as_deref() {
                if (element_id == ID_CPE && el.n_channels != 2)
                    || (element_id != ID_CPE && el.n_channels != 1)
                {
                    return SBRDEC_UNSUPPORTED_CONFIG;
                }
                if header_status == HEADER_RESET {
                    if let Some(hi) = header_index {
                        let h_sbr_header = &mut sbr_header[ei][hi.min(SBRDEC_MAX_DELAY_FRAMES)];
                        sbr_error = sbr_decoder_header_update(flags, h_sbr_header, header_status);

                        if sbr_error == SBRDEC_OK {
                            h_sbr_header.sync_state = SBR_HEADER;
                            h_sbr_header.status |= SBRDEC_HDR_STAT_UPDATE;
                        } else {
                            h_sbr_header.sync_state = SBR_NOT_INITIALIZED;
                            h_sbr_header.status = HEADER_ERROR as u8;
                        }
                    }
                }
            }
        }
    }

    if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
        self_.flags = flags_saved; // restore
    }

    sbr_error
}

/// `sbrDecoder_SetParam()`
pub fn sbr_decoder_set_param(
    self_: Option<&mut SbrDecoderInstance>,
    param: SbrdecParam,
    value: i32,
) -> SbrError {
    let mut error_status = SBRDEC_OK;

    // configure the subsystems
    match param {
        SBR_SYSTEM_BITSTREAM_DELAY => {
            if !(0..=1).contains(&value) {
                error_status = SBRDEC_SET_PARAM_FAIL;
            } else if let Some(s) = self_ {
                s.num_delay_frames = value as u8;
            } else {
                error_status = SBRDEC_NOT_INITIALIZED;
            }
        }
        SBR_QMF_MODE => match self_ {
            None => error_status = SBRDEC_NOT_INITIALIZED,
            Some(s) => {
                if value == 1 {
                    s.flags |= SBRDEC_LOW_POWER;
                } else {
                    s.flags &= !SBRDEC_LOW_POWER;
                }
            }
        },
        SBR_LD_QMF_TIME_ALIGN => match self_ {
            None => error_status = SBRDEC_NOT_INITIALIZED,
            Some(s) => {
                if value == 1 {
                    s.flags |= SBRDEC_LD_MPS_QMF;
                } else {
                    s.flags &= !SBRDEC_LD_MPS_QMF;
                }
            }
        },
        SBR_FLUSH_DATA => {
            if value != 0 {
                match self_ {
                    None => error_status = SBRDEC_NOT_INITIALIZED,
                    Some(s) => s.flags |= SBRDEC_FLUSH,
                }
            }
        }
        SBR_CLEAR_HISTORY => {
            if value != 0 {
                match self_ {
                    None => error_status = SBRDEC_NOT_INITIALIZED,
                    Some(s) => s.flags |= SBRDEC_FORCE_RESET,
                }
            }
        }
        SBR_BS_INTERRUPTION => match self_ {
            None => error_status = SBRDEC_NOT_INITIALIZED,
            Some(s) => {
                // Loop over SBR elements
                for element_index in 0..(s.num_sbr_elements.clamp(0, 8) as usize) {
                    if let Some(el) = s.p_sbr_element[element_index].as_deref() {
                        let header_index =
                            get_header_slot(el.use_frame_slot, &el.use_header_slot) as usize;
                        let h_sbr_header = &mut s.sbr_header[element_index]
                            [header_index.min(SBRDEC_MAX_DELAY_FRAMES)];

                        // Set sync state UPSAMPLING for the corresponding slot. This switches off
                        // bitstream parsing until a new header arrives.
                        if h_sbr_header.sync_state != SBR_NOT_INITIALIZED {
                            h_sbr_header.sync_state = UPSAMPLING;
                            h_sbr_header.status |= SBRDEC_HDR_STAT_UPDATE;
                        }
                    }
                }
            }
        },
        SBR_SKIP_QMF => match self_ {
            None => error_status = SBRDEC_NOT_INITIALIZED,
            Some(s) => {
                if value == 1 {
                    s.flags |= SBRDEC_SKIP_QMF_ANA;
                } else {
                    s.flags &= !SBRDEC_SKIP_QMF_ANA;
                }
                if value == 2 {
                    s.flags |= SBRDEC_SKIP_QMF_SYN;
                } else {
                    s.flags &= !SBRDEC_SKIP_QMF_SYN;
                }
            }
        },
        _ => {
            error_status = SBRDEC_SET_PARAM_FAIL;
        }
    }

    error_status
}

/// `sbrDecoder_drcGetChannel()`: the DRC data of SBR channel `channel`.
fn sbr_decoder_drc_get_channel(
    self_: &mut SbrDecoderInstance,
    channel: i32,
) -> Option<&mut SbrdecDrcChannel> {
    let mut el_chan_idx: i32 = 0;
    let mut num_ch = 0;
    let mut element_index: i32 = 0;

    while (element_index as usize) < SBRDEC_MAX_ELEMENTS && num_ch <= channel {
        let Some(el) = self_.p_sbr_element[element_index as usize].as_deref() else {
            el_chan_idx = 0;
            break;
        };
        el_chan_idx = 0;

        // Determine amount of channels for this element
        let mut el_channels = match el.element_id {
            ID_CPE => 2,
            ID_LFE | ID_SCE => 1,
            _ => 0,
        };

        // Limit with actual allocated element channels
        el_channels = f_min(el_channels, el.n_channels);

        let mut c = 0;
        while c < el_channels && num_ch <= channel {
            if el.p_sbr_channel.get(el_chan_idx as usize).is_some_and(|p| p.is_some()) {
                num_ch += 1;
                el_chan_idx += 1;
            }
            c += 1;
        }
        element_index += 1;
    }
    // The C loop ends with elementIndex incremented also when it stops at a NULL element
    // ("break" before the increment): replicate the index arithmetic.
    let element_index = element_index - 1;
    let el_chan_idx = el_chan_idx - 1;

    if el_chan_idx < 0 || element_index < 0 {
        return None;
    }

    self_.p_sbr_element[element_index as usize]
        .as_deref_mut()
        .and_then(|el| el.p_sbr_channel.get_mut(el_chan_idx as usize))
        .and_then(|c| c.as_deref_mut())
        .map(|c| &mut c.sbr_dec.sbr_drc_channel)
}

/// `sbrDecoder_drcFeedChannel()`: feeds the DRC gains of one channel. `p_next_fact_mag` is
/// `None` for `NULL`.
#[allow(clippy::too_many_arguments)]
pub fn sbr_decoder_drc_feed_channel(
    self_: &mut SbrDecoderInstance,
    ch: i32,
    num_bands: u32,
    p_next_fact_mag: Option<&[FixpDbl]>,
    next_fact_exp: i32,
    drc_interpolation_scheme: i16,
    win_sequence: u8,
    p_band_top: &[u16],
) -> SbrError {
    let mut is_valid_data = false;

    let Some(next_fact_mag) = p_next_fact_mag else {
        return SBRDEC_SET_PARAM_FAIL;
    };
    if ch > (8) {
        return SBRDEC_SET_PARAM_FAIL;
    }

    // Search for gain values different to 1.0f
    for band in 0..num_bands as usize {
        let m = next_fact_mag.get(band).copied().unwrap_or(0);
        if !((m == fl2fxconst_dbl(0.5) && next_fact_exp == 1)
            || (m == MAXVAL_DBL && next_fact_exp == 0))
        {
            is_valid_data = true;
            break;
        }
    }

    // Find the right SBR channel
    if let Some(d) = sbr_decoder_drc_get_channel(self_, ch) {
        if d.enable != 0 || is_valid_data {
            // Activate processing only with real and valid data
            d.enable = 1;
            d.num_bands_next = num_bands;

            d.win_sequence_next = win_sequence;
            d.drc_interpolation_scheme_next = drc_interpolation_scheme;
            d.next_fact_exp = next_fact_exp;

            // numBands <= SBRDEC_MAX_DRC_BANDS from the AAC decoder (deviation: further bands
            // are ignored)
            for i in 0..(num_bands as usize).min(SBRDEC_MAX_DRC_BANDS) {
                d.band_top_next[i] = p_band_top.get(i).copied().unwrap_or(0);
                d.next_fact_mag[i] = next_fact_mag.get(i).copied().unwrap_or(0);
            }
        }
    }

    SBRDEC_OK
}

/// `sbrDecoder_drcDisable()`
pub fn sbr_decoder_drc_disable(self_: &mut SbrDecoderInstance, ch: i32) {
    if ch > (8) || self_.num_sbr_elements == 0 || self_.num_sbr_channels == 0 {
        return;
    }

    // Find the right SBR channel
    if let Some(d) = sbr_decoder_drc_get_channel(self_, ch) {
        sbr_decoder_drc_init_channel(Some(d));
    }
}

/// `sbrDecoder_Parse()`: parses the SBR payload of one element. `drm_bs_buffer` is
/// `pDrmBsBuffer` (`drmBsBufferSize` is its length), `ac_el_flags` is `acElFlags[]`.
///
/// Quirk (reproduced): for DRM, `*count` is reduced by the difference between the start
/// position in the reversed payload and the valid bits of the original bitstream.
#[allow(clippy::too_many_arguments)]
pub fn sbr_decoder_parse(
    self_: &mut SbrDecoderInstance,
    h_bs_original: &mut FdkBitStream,
    drm_bs_buffer: &mut [u8],
    count: &mut i32,
    mut bs_pay_len: i32,
    crc_flag: i32,
    prev_element: Mp4ElementId,
    element_index: i32,
    ac_flags: u32,
    ac_el_flags: &[u32],
) -> SbrError {
    let mut error_status = SBRDEC_OK;
    let mut header_status = HEADER_NOT_PRESENT;
    let start_pos: i32;
    let mut crc_info = FdkCrcInfo::default();
    let mut crc_reg = 0;
    let mut sbr_crc: u16 = 0;
    let mut crc_start_value: u32 = 0;
    let mut bs_bwd = FdkBitStream::default();
    let mut use_bwd = false;
    // The reversed DRM payload: a copy of pDrmBsBuffer as the buffer of the bitstream
    let mut drm_local: Vec<u8> = Vec::new();

    let valid = (0..8).contains(&element_index);
    let ei = element_index.clamp(0, 7) as usize;
    let el_flags = ac_el_flags.get(ei).copied().unwrap_or(0);
    let f_global_independency_flag = ac_flags & AC_INDEP;
    let bs_pvc = el_flags & AC_EL_USAC_PVC;
    let bs_inter_tes = el_flags & AC_EL_USAC_ITES;
    let mut f_do_decode_sbr_data = true;
    let mut align_bits = 0;
    let mut last_hdr_slot = 0usize;
    let mut this_hdr_slot = 0usize;
    let mut have_element = false;
    let mut have_header = false;

    if *count <= 0 {
        if valid {
            set_frame_error_flag(self_.p_sbr_element[ei].as_deref_mut(), FRAME_ERROR);
        }
        return SBRDEC_OK;
    }

    'bail: {
        // Reverse bits of DRM SBR payload
        if self_.flags & SBRDEC_SYNTAX_DRM != 0 && *count > 0 {
            fdk_assert!(drm_bs_buffer.len() >= (512));
            let mut data_bits = *count;

            if data_bits > (512) * 8 {
                // do not flip more data than needed
                data_bits = (512) * 8;
            }

            let mut data_bytes = (data_bits + 7) >> 3;

            let j = fdk_get_valid_bits(h_bs_original) as i32;
            if j != 8 {
                fdk_push_bi_directional(h_bs_original, j - 8);
            }

            drm_local = vec![0u8; 512];
            for (k, b) in drm_local.iter_mut().enumerate() {
                *b = drm_bs_buffer.get(k).copied().unwrap_or(0);
            }
            let mut j = 0usize;
            while data_bytes > 0 {
                let mut buffer: u8 = 0x00;
                let tmp_byte = fdk_read_bits(h_bs_original, 8) as u8;
                for i in 0..4 {
                    let shift = 2 * i + 1;
                    buffer |= (tmp_byte & (0x08 >> i)) << shift;
                    buffer |= (tmp_byte & (0x10 << i)) >> shift;
                }
                drm_local[j] = buffer;
                if let Some(b) = drm_bs_buffer.get_mut(j) {
                    *b = buffer;
                }
                j += 1;
                fdk_push_back(h_bs_original, 16);
                data_bytes -= 1;
            }

            fdk_init_bit_stream(
                &mut bs_bwd,
                &BitMem::from_slice(&drm_local[..]),
                512,
                data_bits as u32,
                BS_READER,
            );

            // Use reversed data
            use_bwd = true;
            bs_pay_len = *count;
        }

        let h_bs: &mut FdkBitStream = if use_bwd { &mut bs_bwd } else { &mut *h_bs_original };

        // Remember start position of  SBR element
        start_pos = fdk_get_valid_bits(h_bs) as i32;

        // SBR sanity checks
        if !valid || self_.p_sbr_element[ei].is_none() {
            error_status = SBRDEC_NOT_INITIALIZED;
            break 'bail;
        }
        have_element = true;

        let num_delay_frames = self_.num_delay_frames;
        let core_codec = self_.core_codec;
        let SbrDecoderInstance {
            p_sbr_element, sbr_header, h_parametric_stereo_dec, flags, ..
        } = &mut *self_;
        let Some(h_sbr_element) = p_sbr_element[ei].as_deref_mut() else {
            break 'bail;
        };

        let last_slot = if h_sbr_element.use_frame_slot > 0 {
            h_sbr_element.use_frame_slot as usize - 1
        } else {
            num_delay_frames as usize
        };
        last_hdr_slot =
            h_sbr_element.use_header_slot[last_slot.min(SBRDEC_MAX_DELAY_FRAMES)] as usize;

        // Get a free header slot not used by frames not processed yet.
        this_hdr_slot =
            get_header_slot(h_sbr_element.use_frame_slot, &h_sbr_element.use_header_slot) as usize;
        let (this_s, last_s) = (
            this_hdr_slot.min(SBRDEC_MAX_DELAY_FRAMES),
            last_hdr_slot.min(SBRDEC_MAX_DELAY_FRAMES),
        );
        have_header = true;

        let stereo = h_sbr_element.element_id == ID_CPE;
        let ufs = (h_sbr_element.use_frame_slot as usize).min(SBRDEC_MAX_DELAY_FRAMES);
        let n_channels = h_sbr_element.n_channels;
        let overlap = h_sbr_element.transposer_settings.overlap as i32;
        let element_id = h_sbr_element.element_id;
        let [c0, c1] = &mut h_sbr_element.p_sbr_channel;
        let (Some(c0), c1) = (c0.as_deref_mut(), c1.as_deref_mut()) else {
            // (no channel: no defined result in C)
            break 'bail;
        };
        let c1 = if stereo { c1 } else { None };
        if stereo && c1.is_none() {
            break 'bail;
        }

        // store frameData; new parsed frameData possibly corrupted
        let frame_data_left_copy = c0.frame_data[ufs];
        let frame_data_right_copy = c1.as_deref().map(|c| c.frame_data[ufs]);

        // reset PS flag; will be set after PS was found
        *flags &= !SBRDEC_PS_DECODED;

        if sbr_header[ei][this_s].status & SBRDEC_HDR_STAT_UPDATE != 0 {
            // Got a new header from extern (e.g. from an ASC)
            header_status = HEADER_OK;
            sbr_header[ei][this_s].status &= !SBRDEC_HDR_STAT_UPDATE;
        } else if this_s != last_s {
            // Copy the last header into this slot otherwise the header compare will trigger
            // more HEADER_RESETs than needed.
            let src = sbr_header[ei][last_s];
            copy_sbr_header(&mut sbr_header[ei][this_s], &src);
        }
        let h_sbr_header = &mut sbr_header[ei][this_s];

        // Check if bit stream data is valid and matches the element context
        if (prev_element != ID_SCE && prev_element != ID_CPE) || prev_element != element_id {
            // In case of LFE we also land here, since there is no LFE SBR element (do upsampling
            // only)
            f_do_decode_sbr_data = false;
        }

        if f_do_decode_sbr_data && (fdk_get_valid_bits(h_bs) as i32) <= 0 {
            f_do_decode_sbr_data = false;
        }

        // SBR CRC-check
        if f_do_decode_sbr_data && crc_flag != 0 {
            let (crc_poly, crc_len) = match core_codec {
                AOT_DRM_AAC | AOT_DRM_SURROUND => {
                    crc_start_value = 0x000000ff;
                    (0x001d, 8)
                }
                _ => {
                    crc_start_value = 0x00000000;
                    (0x0633, 10)
                }
            };
            sbr_crc = fdk_read_bits(h_bs, crc_len) as u16;
            // Setup CRC decoder
            fdk_crc_init(&mut crc_info, crc_poly, crc_start_value, crc_len);
            // Start CRC region
            crc_reg = fdk_crc_start_reg(&mut crc_info, h_bs, 0);
        }

        // Read in the header data and issue a reset if change occured
        if f_do_decode_sbr_data {
            let mut sbr_header_present;

            if *flags & (SBRDEC_SYNTAX_RSVD50 | SBRDEC_SYNTAX_USAC) != 0 {
                // (newSbrInfo: every field is written before it is compared)
                let mut new_sbr_info = SbrHeaderDataBsInfo::ZERO;
                let sbr_info_present;

                if bs_inter_tes != 0 {
                    *flags |= SBRDEC_USAC_ITES;
                } else {
                    *flags &= !SBRDEC_USAC_ITES;
                }

                if f_global_independency_flag != 0 {
                    *flags |= SBRDEC_USAC_INDEP;
                    sbr_info_present = true;
                    sbr_header_present = true;
                } else {
                    *flags &= !SBRDEC_USAC_INDEP;
                    sbr_info_present = fdk_read_bit(h_bs) != 0;
                    if sbr_info_present {
                        sbr_header_present = fdk_read_bit(h_bs) != 0;
                    } else {
                        sbr_header_present = false;
                    }
                }

                if sbr_info_present {
                    new_sbr_info.amp_resolution = fdk_read_bit(h_bs) as u8;
                    new_sbr_info.xover_band = fdk_read_bits(h_bs, 4) as u8;
                    new_sbr_info.sbr_preprocessing = fdk_read_bit(h_bs) as u8;
                    if bs_pvc != 0 {
                        new_sbr_info.pvc_mode = fdk_read_bits(h_bs, 2) as u8;
                        // bs_pvc_mode: 0 -> no PVC, 1 -> PVC mode 1, 2 -> PVC mode 2, 3 -> reserved
                        if new_sbr_info.pvc_mode > 2 {
                            header_status = HEADER_ERROR;
                        }
                        if stereo && new_sbr_info.pvc_mode > 0 {
                            // bs_pvc is always transmitted but pvc_mode is set to zero in case
                            // of stereo SBR. The config might be wrong but we cannot tell for
                            // sure.
                            new_sbr_info.pvc_mode = 0;
                        }
                    } else {
                        new_sbr_info.pvc_mode = 0;
                    }
                    if header_status != HEADER_ERROR {
                        if pod_bytes(&h_sbr_header.bs_info) != pod_bytes(&new_sbr_info) {
                            // in case of ampResolution and preprocessing change no full reset
                            // required; HEADER reset would trigger HBE transposer reset which
                            // breaks eSbr_3_Eaa.mp4
                            if h_sbr_header.bs_info.pvc_mode != new_sbr_info.pvc_mode
                                || h_sbr_header.bs_info.xover_band != new_sbr_info.xover_band
                            {
                                header_status = HEADER_RESET;
                            } else {
                                header_status = HEADER_OK;
                            }

                            h_sbr_header.bs_info = new_sbr_info;
                        } else {
                            header_status = HEADER_OK;
                        }
                    }
                }
                if header_status == HEADER_ERROR {
                    // Corrupt SBR info data, do not decode and switch to UPSAMPLING
                    h_sbr_header.sync_state = if h_sbr_header.sync_state > UPSAMPLING {
                        UPSAMPLING
                    } else {
                        h_sbr_header.sync_state
                    };
                    f_do_decode_sbr_data = false;
                    sbr_header_present = false;
                }

                if sbr_header_present && f_do_decode_sbr_data {
                    let use_dflt_header = fdk_read_bit(h_bs) != 0;

                    if use_dflt_header {
                        sbr_header_present = false;
                        if pod_bytes(&h_sbr_header.bs_data) != pod_bytes(&h_sbr_header.bs_dflt)
                            || h_sbr_header.sync_state != SBR_ACTIVE
                        {
                            h_sbr_header.bs_data = h_sbr_header.bs_dflt;
                            header_status = HEADER_RESET;
                        }
                    }
                }
            } else {
                sbr_header_present = fdk_read_bit(h_bs) != 0;
            }

            if sbr_header_present {
                header_status = sbr_get_header_data(Some(&mut *h_sbr_header), h_bs, *flags, 1, 0);
            }

            if header_status == HEADER_RESET {
                error_status = sbr_decoder_header_update(*flags, h_sbr_header, header_status);

                if error_status == SBRDEC_OK {
                    h_sbr_header.sync_state = SBR_HEADER;
                } else {
                    h_sbr_header.sync_state = SBR_NOT_INITIALIZED;
                    header_status = HEADER_ERROR;
                }
            }

            if error_status != SBRDEC_OK {
                f_do_decode_sbr_data = false;
            }
        } // if (fDoDecodeSbrData)

        // read frame data
        if h_sbr_header.sync_state >= SBR_HEADER && f_do_decode_sbr_data {
            // read the SBR element data
            if !stereo {
                if let Some(ps) = h_parametric_stereo_dec.as_deref_mut() {
                    // update slot index for PS bitstream parsing
                    ps.bs_last_slot = ps.bs_read_slot;
                    ps.bs_read_slot = ufs as u8;
                }
            }
            let SbrChannel { frame_data, prev_frame_data, sbr_dec } = c0;
            let sbr_frame_ok = sbr_get_channel_element(
                h_sbr_header,
                &mut frame_data[ufs],
                c1.map(|c| &mut c.frame_data[ufs]),
                prev_frame_data,
                sbr_dec.pvc_static_data.pvc_mode_last,
                h_bs,
                if stereo { None } else { h_parametric_stereo_dec.as_deref_mut() },
                *flags,
                overlap,
            );

            if sbr_frame_ok == 0 {
                f_do_decode_sbr_data = false;
            } else {
                let val_bits = if bs_pay_len > 0 {
                    bs_pay_len - (start_pos - fdk_get_valid_bits(h_bs) as i32)
                } else {
                    fdk_get_valid_bits(h_bs) as i32
                };

                // sanity check of remaining bits
                if val_bits < 0 {
                    f_do_decode_sbr_data = false;
                } else {
                    match core_codec {
                        AOT_SBR | AOT_PS | AOT_AAC_LC => {
                            // This sanity check is only meaningful with General Audio
                            // bitstreams
                            align_bits = val_bits & 0x7;

                            if val_bits > align_bits {
                                f_do_decode_sbr_data = false;
                            }
                        }
                        _ => {
                            // No sanity check available
                        }
                    }
                }
            }
        } else {
            // The returned bit count will not be the actual payload size since we did not parse
            // the frame data. Return an error so that the caller can react respectively.
            error_status = SBRDEC_PARSE_ERROR;
        }

        if crc_flag != 0 && h_sbr_header.sync_state >= SBR_HEADER && f_do_decode_sbr_data {
            fdk_push_for(h_bs, align_bits as u32);
            fdk_crc_end_reg(&mut crc_info, h_bs, crc_reg); // End CRC region
            fdk_push_back(h_bs, align_bits as u32);
            // Check CRC
            if (fdk_crc_get_crc(&crc_info) as u32 ^ crc_start_value) != sbr_crc as u32 {
                f_do_decode_sbr_data = false;
                if header_status != HEADER_NOT_PRESENT {
                    header_status = HEADER_ERROR;
                    h_sbr_header.sync_state = SBR_NOT_INITIALIZED;
                }
            }
        }

        if !f_do_decode_sbr_data {
            // Set error flag for this slot to trigger concealment
            set_frame_error_flag(p_sbr_element[ei].as_deref_mut(), FRAME_ERROR);
            // restore old frameData for concealment
            if let Some(el) = p_sbr_element[ei].as_deref_mut() {
                if let Some(c) = el.p_sbr_channel[0].as_deref_mut() {
                    c.frame_data[ufs] = frame_data_left_copy;
                }
                if let (true, Some(copy)) = (stereo, frame_data_right_copy) {
                    if let Some(c) = el.p_sbr_channel[1].as_deref_mut() {
                        c.frame_data[ufs] = copy;
                    }
                }
            }
            error_status = SBRDEC_PARSE_ERROR;
        } else {
            // Everything seems to be ok so clear the error flag
            set_frame_error_flag(p_sbr_element[ei].as_deref_mut(), FRAME_OK);
        }

        if !stereo {
            // Turn coupling off explicitely to avoid access to absent right frame data that
            // might occur with corrupt bitstreams.
            if let Some(c) =
                p_sbr_element[ei].as_deref_mut().and_then(|el| el.p_sbr_channel[0].as_deref_mut())
            {
                c.frame_data[ufs].coupling = COUPLING_OFF;
            }
        }
        let _ = n_channels;
    }

    // bail:
    if error_status != SBRDEC_NOT_INITIALIZED {
        let mut use_old_hdr = (header_status == HEADER_NOT_PRESENT
            || header_status == HEADER_ERROR
            || (header_status == HEADER_RESET && error_status == SBRDEC_PARSE_ERROR))
            as i32;

        if use_old_hdr == 0 && this_hdr_slot != last_hdr_slot && have_header {
            let (t, l) = (
                this_hdr_slot.min(SBRDEC_MAX_DELAY_FRAMES),
                last_hdr_slot.min(SBRDEC_MAX_DELAY_FRAMES),
            );
            use_old_hdr |= (compare_sbr_header(&self_.sbr_header[ei][t], &self_.sbr_header[ei][l])
                == 0) as i32;
        }

        if have_element {
            let num_delay_frames = self_.num_delay_frames as usize;
            if let Some(h_sbr_element) = self_.p_sbr_element[ei].as_deref_mut() {
                let ufs = (h_sbr_element.use_frame_slot as usize).min(SBRDEC_MAX_DELAY_FRAMES);
                if use_old_hdr != 0 {
                    // Use the old header for this frame
                    h_sbr_element.use_header_slot[ufs] = last_hdr_slot as u8;
                } else {
                    // Use the new header for this frame
                    h_sbr_element.use_header_slot[ufs] = this_hdr_slot as u8;
                }

                // Move frame pointer to the next slot which is up to be decoded/applied next
                h_sbr_element.use_frame_slot =
                    ((h_sbr_element.use_frame_slot as usize + 1) % (num_delay_frames + 1)) as u8;
            }
        }
    }

    // (hBs = hBsOriginal for DRM)
    *count -= start_pos - fdk_get_valid_bits(h_bs_original) as i32;

    drop(drm_local);
    error_status
}

/// `sbrDecoder_DecodeElement()`: renders one SBR element into the time domain signal.
#[allow(clippy::too_many_arguments)]
fn sbr_decoder_decode_element(
    self_: &mut SbrDecoderInstance,
    qd: &mut FdkQmfDomain,
    input: &[i32],
    time_data: &mut [i32],
    time_data_size: i32,
    map_descr: &FdkChannelMapDescr,
    map_idx: i32,
    channel_index: i32,
    element_index: usize,
    num_in_channels: i32,
    num_out_channels: &mut i32,
    ps_possible: i32,
    par: Option<SbrPar<'_>>,
) -> SbrError {
    let mut error_status = SBRDEC_OK;
    let codec_frame_size = self_.codec_frame_size as i32;

    let Some(el) = self_.p_sbr_element[element_index].as_deref() else {
        return SBRDEC_NOT_INITIALIZED;
    };
    let stereo = el.element_id == ID_CPE;
    let num_element_channels = el.n_channels; // Number of channels of the current SBR element

    if self_.flags & SBRDEC_FLUSH != 0 {
        if self_.num_flushed_frames > self_.num_delay_frames {
            // No valid SBR payload available, hence switch to upsampling (in all headers)
            for hdr_idx in 0..SBRDEC_MAX_DELAY_FRAMES + 1 {
                if self_.sbr_header[element_index][hdr_idx].sync_state > UPSAMPLING {
                    self_.sbr_header[element_index][hdr_idx].sync_state = UPSAMPLING;
                }
            }
        } else {
            // Move frame pointer to the next slot which is up to be decoded/applied next
            let n = self_.num_delay_frames as usize + 1;
            if let Some(el) = self_.p_sbr_element[element_index].as_deref_mut() {
                el.use_frame_slot = ((el.use_frame_slot as usize + 1) % n) as u8;
            }
            // Update header and frame data pointer because they have already been set
        }
    }

    let SbrDecoderInstance {
        p_sbr_element,
        sbr_header,
        h_parametric_stereo_dec,
        flags,
        sample_rate_in,
        sample_rate_out,
        downscale_factor,
        syn_downsample_fac,
        sbr_in_data_headroom,
        par_scratch,
        ..
    } = &mut *self_;
    let Some(h_sbr_element) = p_sbr_element[element_index].as_deref_mut() else {
        return SBRDEC_NOT_INITIALIZED;
    };
    let ufs = (h_sbr_element.use_frame_slot as usize).min(SBRDEC_MAX_DELAY_FRAMES);
    let hdr_slot = (h_sbr_element.use_header_slot[ufs] as usize).min(SBRDEC_MAX_DELAY_FRAMES);
    let h_sbr_header = &mut sbr_header[element_index][hdr_slot];

    // Update the header error flag
    h_sbr_header.frame_error_flag = h_sbr_element.frame_error_flag[ufs];

    let SbrDecoderElement { p_sbr_channel, transposer_settings, .. } = h_sbr_element;
    let [c0, c1] = p_sbr_channel;

    // Prepare filterbank for upsampling if no valid bit stream data is available.
    if h_sbr_header.sync_state == SBR_NOT_INITIALIZED {
        error_status = init_header_data(
            h_sbr_header,
            *sample_rate_in,
            *sample_rate_out,
            *downscale_factor,
            codec_frame_size,
            *flags,
            1, // SET_DEFAULT_HDR
        );

        if error_status != SBRDEC_OK {
            return error_status;
        }

        h_sbr_header.sync_state = UPSAMPLING;

        error_status = sbr_decoder_header_update(*flags, h_sbr_header, HEADER_NOT_PRESENT);

        if error_status != SBRDEC_OK {
            h_sbr_header.sync_state = SBR_NOT_INITIALIZED;
            return error_status;
        }
    }

    // reset
    if h_sbr_header.status & SBRDEC_HDR_STAT_RESET != 0 {
        let apply_sbr_proc = h_sbr_header.sync_state == SBR_ACTIVE
            || (h_sbr_header.frame_error_flag == 0 && h_sbr_header.sync_state == SBR_HEADER);
        for ch in 0..num_element_channels.clamp(0, 2) as usize {
            let c = if ch == 0 { c0.as_deref_mut() } else { c1.as_deref_mut() };
            let Some(c) = c else {
                continue;
            };
            let SbrChannel { frame_data, prev_frame_data, sbr_dec: sd } = c;
            // pSbrChannel[ch]->frameData: the first frame data slot
            let error_status_tmp = reset_sbr_dec(
                sd,
                qd,
                h_sbr_header,
                prev_frame_data,
                *syn_downsample_fac as i32,
                *flags,
                &frame_data[0],
                transposer_settings,
            );
            if error_status_tmp != SBRDEC_OK {
                h_sbr_header.sync_state = UPSAMPLING;
            }
        }
        if apply_sbr_proc {
            h_sbr_header.status &= !SBRDEC_HDR_STAT_RESET;
        }
    }

    let Some(c0) = c0.as_deref_mut() else {
        return SBRDEC_NOT_INITIALIZED;
    };
    let mut c1 = c1.as_deref_mut();
    if stereo && c1.is_none() {
        return SBRDEC_NOT_INITIALIZED;
    }

    // decoding
    if h_sbr_header.sync_state == SBR_ACTIVE
        || (h_sbr_header.sync_state == SBR_HEADER && h_sbr_header.frame_error_flag == 0)
    {
        error_status = SBRDEC_OK;

        let (fl, pl) = (&mut c0.frame_data[ufs], &mut c0.prev_frame_data);
        match (stereo, c1.as_deref_mut()) {
            (true, Some(r)) => {
                decode_sbr_data(
                    h_sbr_header,
                    fl,
                    pl,
                    Some(&mut r.frame_data[ufs]),
                    Some(&mut r.prev_frame_data),
                );
            }
            _ => decode_sbr_data(h_sbr_header, fl, pl, None, None),
        }

        // Now we have a full parameter set and can do parameter based concealment instead of
        // plain upsampling.
        h_sbr_header.sync_state = SBR_ACTIVE;
    }

    if time_data_size
        < (h_sbr_header.number_time_slots as i32
            * h_sbr_header.time_step as i32
            * qd.global_conf.n_bands_synthesis as i32
            * if ps_possible != 0 { f_max(2, num_in_channels) } else { num_in_channels })
    {
        return SBRDEC_OUTPUT_BUFFER_TOO_SMALL;
    }

    let stride_out;
    {
        *flags &= !SBRDEC_PS_DECODED;
        // C_ALLOC_SCRATCH_START(pPsScratch, struct PS_DEC_COEFFICIENTS, 1)
        let mut p_ps_scratch = PsDecCoefficients::ZERO;

        // decode PS data if available
        if ps_possible != 0 && h_sbr_header.sync_state == SBR_ACTIVE {
            if let Some(h_ps_d) = h_parametric_stereo_dec.as_deref_mut() {
                // define which frame delay line slot to process
                h_ps_d.process_slot = ufs as u8;

                let apply_ps = decode_ps(h_ps_d, h_sbr_header.frame_error_flag, &mut p_ps_scratch);
                *flags |= if apply_ps != 0 { SBRDEC_PS_DECODED } else { 0 };
            }
        }

        let offset0 =
            fdk_ch_map_descr_get_map_value(map_descr, channel_index as u8, map_idx as u32) as usize;
        let offset0_block = offset0 * codec_frame_size.max(0) as usize;
        let mut offset1 = 255usize;
        let mut offset1_block = 0usize;
        if stereo || ps_possible != 0 {
            // the value of offset1 only matters if the condition is true, however if it is not
            // true channelIndex+1 may exceed the channel map resutling in an error, though the
            // value of offset1 is actually meaningless. This is prevented here.
            offset1 = fdk_ch_map_descr_get_map_value(
                map_descr,
                (channel_index + 1) as u8,
                map_idx as u32,
            ) as usize;
            offset1_block = offset1 * codec_frame_size.max(0) as usize;
        }

        // Set strides for reading and writing
        stride_out = if ps_possible != 0 {
            if num_in_channels < 2 {
                2
            } else {
                num_in_channels
            }
        } else {
            num_in_channels
        };

        let ps_decoded = *flags & SBRDEC_PS_DECODED != 0;
        let in0 = input.get(offset0_block..).unwrap_or(&[]);

        // Not in C: both channels of a stereo element at the same time (see
        // sbr_dec_stereo_pool()).
        let split = match (par, c1.as_deref_mut()) {
            (Some(par), Some(r)) if stereo => sbr_dec_stereo_pool(
                par,
                par_scratch,
                c0,
                r,
                ufs,
                qd,
                in0,
                input.get(offset1_block..).unwrap_or(&[]),
                time_data,
                offset0,
                offset1,
                stride_out,
                h_sbr_header,
                h_parametric_stereo_dec.as_deref_mut(),
                &mut p_ps_scratch,
                transposer_settings,
                *flags,
                codec_frame_size,
                *sbr_in_data_headroom,
            ),
            _ => false,
        };

        // use same buffers for left and right channel and apply PS per timeslot
        // Process left channel
        if !split {
            let SbrChannel { frame_data, prev_frame_data, sbr_dec: sd } = &mut *c0;
            crate::sbr_dec::sbr_dec(
                sd,
                qd,
                in0,
                time_data,
                offset0,
                if ps_decoded { c1.as_deref_mut().map(|c| &mut c.sbr_dec) } else { None },
                offset1,
                stride_out,
                h_sbr_header,
                &frame_data[ufs],
                prev_frame_data,
                (h_sbr_header.sync_state == SBR_ACTIVE) as i32,
                h_parametric_stereo_dec.as_deref_mut(),
                &mut p_ps_scratch,
                transposer_settings,
                *flags,
                codec_frame_size,
                *sbr_in_data_headroom,
            );
        }

        if stereo && !split {
            // Process right channel
            if let Some(c1) = c1 {
                let in1 = input.get(offset1_block..).unwrap_or(&[]);
                let SbrChannel { frame_data, prev_frame_data, sbr_dec: sd } = c1;
                crate::sbr_dec::sbr_dec(
                    sd,
                    qd,
                    in1,
                    time_data,
                    offset1,
                    None,
                    0,
                    stride_out,
                    h_sbr_header,
                    &frame_data[ufs],
                    prev_frame_data,
                    (h_sbr_header.sync_state == SBR_ACTIVE) as i32,
                    None,
                    &mut p_ps_scratch,
                    transposer_settings,
                    *flags,
                    codec_frame_size,
                    *sbr_in_data_headroom,
                );
            }
        }
    }

    if let Some(h_ps_d) = h_parametric_stereo_dec.as_deref_mut() {
        // save PS status for next run
        h_ps_d.ps_decoded_prv = if *flags & SBRDEC_PS_DECODED != 0 { 1 } else { 0 };
    }

    if ps_possible != 0 && *flags & SBRDEC_SKIP_QMF_SYN == 0 {
        fdk_assert!(stride_out > 1);
        if *flags & SBRDEC_PS_DECODED == 0 {
            // A decoder which is able to decode PS has to produce a stereo output even if no PS
            // data is available. So copy left channel to right channel.
            let in_ch = qd.qmf_domain_in[0].fb.no_channels;
            // (guarded: zero input channels give 0)
            let copy_frame_size = (codec_frame_size * qd.qmf_domain_out[0].fb.no_channels)
                .checked_div(in_ch)
                .unwrap_or(0);
            fdk_assert!(stride_out == 2);

            let mut p = 0usize;
            for _ in 0..(copy_frame_size >> 1).max(0) {
                for _ in 0..2 {
                    let tmp = time_data.get(p).copied().unwrap_or(0);
                    p += 1;
                    if let Some(d) = time_data.get_mut(p) {
                        *d = tmp;
                    }
                    p += 1;
                }
            }
        }

        *num_out_channels = 2; // Output minimum two channels when PS is enabled.
    }

    error_status
}

/// One channel of [`sbr_dec_stereo_pool`].
struct StereoTask<'a> {
    c: &'a mut SbrChannel,
    qd: &'a mut FdkQmfDomain,
    time_in: &'a [i32],
    out: &'a mut [i32],
    out_off: usize,
    hdr: &'a mut SbrHeaderData,
    ps_d: Option<&'a mut PsDec>,
    ps: &'a mut PsDecCoefficients,
}

/// Fills buffers with a pattern derived from `seed` (buffer poisoning of test builds).
fn poison_buffers<'a>(bufs: impl Iterator<Item = &'a mut [FixpDbl]>, seed: u64) {
    let mut x = seed | 1;
    for b in bufs {
        for v in b.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *v = x as FixpDbl;
        }
    }
}

/// Not in C: the two `sbr_dec()` calls of a stereo element of `sbrDecoder_DecodeElement()`
/// (left, then right) on two threads. Returns `false` without any effect if the element does
/// not qualify; the caller then makes the two calls.
///
/// Why the result is that of the two calls in order:
///
/// - Each call reads and writes its own channel (`SBR_CHANNEL`: the SBR decoder state, the
///   frame data, the previous frame data), its own input and output channel of the QMF domain
///   (filter bank, states, scaling, slot pointers) and its own overlap buffer. The left channel
///   takes the parametric stereo decoder (unused without PS: `SBRDEC_PS_DECODED` is not set for
///   a stereo element) and the PS scratch memory (unused without PS); both read the transposer
///   settings and the input.
/// - The QMF work buffer (`globalConf.pWorkBuffer`) is shared in C: both channels use the same
///   slots (`nQmfProcChannels` is 1 for SBR without MPEG Surround). The second channel gets work
///   buffer sections of its own ([`SbrParScratch`]), at the same offsets. That is exact if no
///   call reads QMF data it did not write itself in the same call: the analysis writes the
///   lower bands of every slot of the frame and `sbr_dec()` clears the upper ones before
///   anything reads them, the transposer and the envelope adjuster read and write within these
///   slots, and the overlap slots are the channel's own buffer. Checked by poisoning both work
///   buffers before every element (`poison`). Elements that pass QMF data through the work
///   buffer to or from MPEG Surround (`SBRDEC_SKIP_QMF_ANA`, `SBRDEC_SKIP_QMF_SYN`) are not
///   split.
/// - The header (`hHeaderData`, shared): the calls write `ovHighSubband` (the usb of the
///   channel's synthesis filter bank, after its synthesis; the right channel reads it in its
///   envelope adjuster and synthesis) and, when the patching mode of a channel changes, the
///   limiter band tables (`resetLimiterBands()`). An element is split only if the left
///   channel's write of `ovHighSubband` does not change it and no channel resets the limiter
///   bands; the right channel works on a copy of the header, whose `ovHighSubband` (the last
///   write) is copied back.
/// - The output is interleaved (`stride_out`): the left channel writes the samples at
///   `offset0 + k * stride_out`, the right one those at `offset1 + k * stride_out`. The right
///   channel writes into a copy of the output buffer, whose samples of its residue class are
///   copied back.
#[allow(clippy::too_many_arguments)]
fn sbr_dec_stereo_pool(
    par: SbrPar<'_>,
    scratch: &mut Option<Box<SbrParScratch>>,
    l: &mut SbrChannel,
    r: &mut SbrChannel,
    ufs: usize,
    qd: &mut FdkQmfDomain,
    in0: &[i32],
    in1: &[i32],
    time_data: &mut [i32],
    offset0: usize,
    offset1: usize,
    stride_out: i32,
    hdr: &mut SbrHeaderData,
    ps_d: Option<&mut PsDec>,
    ps_scratch: &mut PsDecCoefficients,
    settings: &TransposerSettings,
    flags: u32,
    codec_frame_size: i32,
    headroom: i32,
) -> bool {
    if flags & (SBRDEC_SKIP_QMF_ANA | SBRDEC_SKIP_QMF_SYN | SBRDEC_PS_DECODED) != 0 {
        return false;
    }
    let seed = par.poison.wrapping_add(PAR_ELEMENTS.load(std::sync::atomic::Ordering::Relaxed));
    if par.poison != 0 {
        // (also before the elements that are not split: no call reads what an earlier call
        // left in the work buffer)
        poison_buffers(qd.mem.work_buffer.iter_mut().filter_map(|b| b.as_deref_mut()), seed);
    }
    let (Some(il), Some(ol), Some(ir), Some(or)) = (
        l.sbr_dec.qmf_domain_in_ch,
        l.sbr_dec.qmf_domain_out_ch,
        r.sbr_dec.qmf_domain_in_ch,
        r.sbr_dec.qmf_domain_out_ch,
    ) else {
        return false;
    };
    let m = QMF_DOMAIN_MAX_CHANNELS;
    if il >= m || ol >= m || ir >= m || or >= m || il == ir || ol == or {
        return false;
    }
    let stride = stride_out.max(0) as usize;
    if stride < 2 || offset0 % stride == offset1 % stride || offset1 >= time_data.len() {
        return false;
    }
    // The writes of the header (see above).
    let active = hdr.sync_state == SBR_ACTIVE;
    for c in [&*l, &*r] {
        if active
            && c.frame_data[ufs].sbr_patching_mode as i32
                != c.sbr_dec.sbr_calculate_envelope.sbr_patching_mode
        {
            return false;
        }
    }
    if qd.qmf_domain_out[ol].fb.usb as u8 != hdr.freq_band_data.ov_high_subband {
        return false;
    }

    let s = scratch.get_or_insert_with(Default::default);
    let SbrParScratch { qd: aux, out } = &mut **s;
    aux.global_conf = qd.global_conf;
    for (a, b) in aux.mem.work_buffer.iter_mut().zip(qd.mem.work_buffer.iter()) {
        match b.as_deref() {
            Some(b) => {
                if a.as_deref().is_none_or(|a| a.len() != b.len()) {
                    *a = Some(vec![0; b.len()].into_boxed_slice());
                }
            }
            None => *a = None,
        }
    }
    core::mem::swap(&mut aux.qmf_domain_in[ir], &mut qd.qmf_domain_in[ir]);
    core::mem::swap(&mut aux.qmf_domain_out[or], &mut qd.qmf_domain_out[or]);
    core::mem::swap(&mut aux.mem.overlap_buffer[ir], &mut qd.mem.overlap_buffer[ir]);
    if par.poison != 0 {
        poison_buffers(aux.mem.work_buffer.iter_mut().filter_map(|b| b.as_deref_mut()), !seed);
    }
    if par.poison_overlap {
        poison_buffers(aux.mem.overlap_buffer[ir].as_deref_mut().into_iter(), 0x5eed);
    }
    // The part of the output the right channel's synthesis writes (no_col slots of no_channels
    // samples from offset1 on, with the stride).
    let fb_r = &aux.qmf_domain_out[or].fb;
    let n_r = fb_r.no_col.max(0) as usize * fb_r.no_channels.max(0) as usize;
    let end = if n_r == 0 { offset1 } else { offset1 + (n_r - 1) * stride + 1 };
    let end = end.min(time_data.len());
    out.clear();
    out.extend_from_slice(&time_data[..end]);
    let mut hdr_r = *hdr;
    let mut ps_r = PsDecCoefficients::ZERO;

    let mut tasks = [
        StereoTask {
            c: l,
            qd: &mut *qd,
            time_in: in0,
            out: &mut *time_data,
            out_off: offset0,
            hdr: &mut *hdr,
            ps_d,
            ps: ps_scratch,
        },
        StereoTask {
            c: r,
            qd: &mut *aux,
            time_in: in1,
            out: &mut out[..],
            out_off: offset1,
            hdr: &mut hdr_r,
            ps_d: None,
            ps: &mut ps_r,
        },
    ];
    par.pool.for_each_mut(&mut tasks, |_, t| {
        let SbrChannel { frame_data, prev_frame_data, sbr_dec: sd } = &mut *t.c;
        crate::sbr_dec::sbr_dec(
            sd,
            t.qd,
            t.time_in,
            t.out,
            t.out_off,
            None,
            0,
            stride_out,
            t.hdr,
            &frame_data[ufs],
            prev_frame_data,
            active as i32,
            t.ps_d.as_deref_mut(),
            t.ps,
            settings,
            flags,
            codec_frame_size,
            headroom,
        );
    });

    core::mem::swap(&mut aux.qmf_domain_in[ir], &mut qd.qmf_domain_in[ir]);
    core::mem::swap(&mut aux.qmf_domain_out[or], &mut qd.qmf_domain_out[or]);
    core::mem::swap(&mut aux.mem.overlap_buffer[ir], &mut qd.mem.overlap_buffer[ir]);
    debug_assert!(aux.global_conf == qd.global_conf);
    // The right channel's write of ovHighSubband is the last one.
    hdr.freq_band_data.ov_high_subband = hdr_r.freq_band_data.ov_high_subband;
    debug_assert!({
        let mut h = hdr_r;
        h.freq_band_data.ov_high_subband = hdr.freq_band_data.ov_high_subband;
        h == *hdr
    });
    for p in (offset1..end).step_by(stride) {
        time_data[p] = out[p];
    }
    PAR_ELEMENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    true
}

/// `sbrDecoder_Apply()`: applies SBR to the core output of one frame (see the module
/// documentation for the buffers). `map_descr` is `None` for `NULL`.
#[allow(clippy::too_many_arguments)]
pub fn sbr_decoder_apply(
    self_: &mut SbrDecoderInstance,
    qd: &mut FdkQmfDomain,
    input: &[i32],
    time_data: &mut [i32],
    time_data_size: i32,
    num_channels: &mut i32,
    sample_rate: &mut i32,
    map_descr: Option<&FdkChannelMapDescr>,
    map_idx: i32,
    core_decoded_ok: i32,
    ps_decoded: &mut u8,
    in_data_headroom: i32,
    out_data_headroom: &mut i32,
) -> SbrError {
    sbr_decoder_apply_pool(
        self_,
        qd,
        input,
        time_data,
        time_data_size,
        num_channels,
        sample_rate,
        map_descr,
        map_idx,
        core_decoded_ok,
        ps_decoded,
        in_data_headroom,
        out_data_headroom,
        None,
    )
}

/// Not in C: [`sbr_decoder_apply`] with per channel parallelism (`par`; `None`: the straight
/// function). The two channels of a stereo element are decoded on two threads when they are
/// independent (see `sbr_dec_stereo_pool()`); everything else runs in the calling thread in
/// the order of the straight function. The result is that of [`sbr_decoder_apply`].
#[allow(clippy::too_many_arguments)]
pub fn sbr_decoder_apply_pool(
    self_: &mut SbrDecoderInstance,
    qd: &mut FdkQmfDomain,
    input: &[i32],
    time_data: &mut [i32],
    time_data_size: i32,
    num_channels: &mut i32,
    sample_rate: &mut i32,
    map_descr: Option<&FdkChannelMapDescr>,
    map_idx: i32,
    core_decoded_ok: i32,
    ps_decoded: &mut u8,
    in_data_headroom: i32,
    out_data_headroom: &mut i32,
    par: Option<SbrPar<'_>>,
) -> SbrError {
    let mut error_status = SBRDEC_OK;

    let mut num_sbr_channels = 0;

    let Some(map_descr) = map_descr else {
        return SBRDEC_INVALID_ARGUMENT;
    };
    if fdk_ch_map_descr_is_valid(Some(map_descr)) == 0 {
        return SBRDEC_INVALID_ARGUMENT;
    }

    let mut ps_possible = *ps_decoded as i32;
    let num_core_channels = *num_channels;
    if num_core_channels <= 0 {
        return SBRDEC_INVALID_ARGUMENT;
    }

    if self_.num_sbr_elements < 1 {
        // exit immediately to avoid access violations
        return SBRDEC_NOT_INITIALIZED;
    }

    // Sanity check of allocated SBR elements.
    for sbr_element_num in 0..self_.num_sbr_elements as usize {
        if self_.p_sbr_element.get(sbr_element_num).is_none_or(|e| e.is_none()) {
            return SBRDEC_NOT_INITIALIZED;
        }
    }

    if self_.num_sbr_elements != 1
        || self_.p_sbr_element[0].as_ref().is_none_or(|e| e.element_id != ID_SCE)
    {
        ps_possible = 0;
    }

    self_.sbr_in_data_headroom = in_data_headroom;
    *out_data_headroom = 8;

    // Make sure that even if no SBR data was found/parsed *psDecoded is returned 1 if
    // psPossible was 0.
    if ps_possible == 0 {
        self_.flags &= !SBRDEC_PS_DECODED;
    }

    // replaces channel based reset inside sbr_dec()
    if (self_.flags & SBRDEC_LOW_POWER != 0) != (qd.global_conf.flags & QMF_FLAG_LP != 0) {
        if self_.flags & SBRDEC_LOW_POWER != 0 {
            qd.global_conf.flags |= QMF_FLAG_LP;
            qd.global_conf.flags_requested |= QMF_FLAG_LP;
        } else {
            qd.global_conf.flags &= !QMF_FLAG_LP;
            qd.global_conf.flags_requested &= !QMF_FLAG_LP;
        }
        if fdk_qmf_domain_init_filter_bank(qd, QMF_FLAG_KEEP_STATES) != 0 {
            return SBRDEC_UNSUPPORTED_CONFIG;
        }
    }
    if self_.num_sbr_channels > qd.global_conf.n_input_channels as i32 {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    if self_.flags & SBRDEC_FLUSH != 0 {
        // flushing is signalized, hence increment the flush frame counter
        self_.num_flushed_frames = self_.num_flushed_frames.wrapping_add(1);
    } else {
        // no flushing is signalized, hence reset the flush frame counter
        self_.num_flushed_frames = 0;
    }

    'bail: {
        // Loop over SBR elements
        for sbr_element_num in 0..self_.num_sbr_elements.min(8) as usize {
            if ps_possible != 0
                && self_.p_sbr_element[sbr_element_num]
                    .as_ref()
                    .is_some_and(|e| e.p_sbr_channel[1].is_none())
            {
                // Disable PS and try decoding SBR mono.
                ps_possible = 0;
            }

            let mut num_element_chan = if self_.p_sbr_element[sbr_element_num]
                .as_ref()
                .is_some_and(|e| e.element_id == ID_CPE)
            {
                2
            } else {
                1
            };

            // If core signal is bad then force upsampling
            if core_decoded_ok == 0 {
                set_frame_error_flag(
                    self_.p_sbr_element[sbr_element_num].as_deref_mut(),
                    FRAME_ERROR_ALLSLOTS,
                );
            }

            error_status = sbr_decoder_decode_element(
                self_,
                qd,
                input,
                time_data,
                time_data_size,
                map_descr,
                map_idx,
                num_sbr_channels,
                sbr_element_num,
                num_core_channels, // is correct even for USC SCI==2 case
                &mut num_element_chan,
                ps_possible,
                par,
            );

            if error_status != SBRDEC_OK {
                break 'bail;
            }

            num_sbr_channels += num_element_chan;

            if num_sbr_channels >= num_core_channels {
                break;
            }
        }

        // Update numChannels and samplerate
        // Do not mess with output channels in case of USAC. numSbrChannels != numChannels for
        // stereoConfigIndex == 2
        if self_.flags & SBRDEC_SYNTAX_USAC == 0 {
            *num_channels = num_sbr_channels;
        }
        *sample_rate = self_.sample_rate_out;
        *ps_decoded = if self_.flags & SBRDEC_PS_DECODED != 0 { 1 } else { 0 };

        // Clear reset and flush flag because everything seems to be done successfully.
        self_.flags &= !SBRDEC_FORCE_RESET;
        self_.flags &= !SBRDEC_FLUSH;
    }

    error_status
}

/// `sbrDecoder_Close()`: destroys the SBR decoder instance.
pub fn sbr_decoder_close(p_self: &mut Option<Box<SbrDecoderInstance>>) -> SbrError {
    if let Some(s) = p_self.as_deref_mut() {
        if s.h_parametric_stereo_dec.is_some() {
            delete_ps_dec(&mut s.h_parametric_stereo_dec);
        }

        for i in 0..SBRDEC_MAX_ELEMENTS {
            sbr_decoder_destroy_element(s, i);
        }

        *p_self = None;
    }
    SBRDEC_OK
}

/// `sbrDecoder_GetLibInfo()`: enters the library information into the next free entry of
/// `info` (`None` for `NULL`).
pub fn sbr_decoder_get_lib_info(info: Option<&mut [LibInfo]>) -> i32 {
    let Some(info) = info else {
        return -1;
    };

    // search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        match info.get(i) {
            Some(e) if e.module_id == FDK_NONE => break,
            Some(_) => {}
            None => return -1, // (guarded at the end of the caller's array)
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST as usize {
        return -1;
    }
    let info = &mut info[i];

    info.module_id = FDK_SBRDEC;
    info.version = lib_version(SBRDECODER_LIB_VL0, SBRDECODER_LIB_VL1, SBRDECODER_LIB_VL2);
    lib_version_string(info);
    info.build_date = SBRDECODER_LIB_BUILD_DATE;
    info.build_time = SBRDECODER_LIB_BUILD_TIME;
    info.title = SBRDECODER_LIB_TITLE;

    // Set flags
    info.flags = CAPF_SBR_HQ
        | CAPF_SBR_LP
        | CAPF_SBR_PS_MPEG
        | CAPF_SBR_DRM_BS
        | CAPF_SBR_CONCEALMENT
        | CAPF_SBR_DRC
        | CAPF_SBR_ELD_DOWNSCALE
        | CAPF_SBR_HBEHQ;
    // End of flags

    0
}

/// `sbrDecoder_GetDelay()`: the delay of the SBR decoder in output samples (`None` for
/// `NULL`).
pub fn sbr_decoder_get_delay(self_: Option<&SbrDecoderInstance>) -> u32 {
    let mut output_delay: u32 = 0;

    if let Some(s) = self_ {
        let flags = s.flags;

        // See chapter 1.6.7.2 of ISO/IEC 14496-3 for the GA-SBR figures below.

        // Are we initialized?
        if s.num_sbr_channels > 0 && s.num_sbr_elements > 0 {
            // Add QMF synthesis delay
            if flags & SBRDEC_ELD_GRID != 0 && is_lowdelay(s.core_codec) {
                // Low delay SBR:
                if flags & SBRDEC_SKIP_QMF_SYN == 0 {
                    output_delay += if flags & SBRDEC_DOWNSAMPLE != 0 { 32 } else { 64 }; // QMF synthesis
                    if flags & SBRDEC_LD_MPS_QMF != 0 {
                        output_delay += 32;
                    }
                }
            } else if !is_usac(s.core_codec) {
                // By the method of elimination this is the GA (AAC-LC, HE-AAC, ...) branch:
                output_delay += if flags & SBRDEC_DOWNSAMPLE != 0 { 481 } else { 962 };
                if flags & SBRDEC_SKIP_QMF_SYN != 0 {
                    output_delay = output_delay.wrapping_sub(257); // QMF synthesis
                }
            }
        }
    }

    output_delay
}
