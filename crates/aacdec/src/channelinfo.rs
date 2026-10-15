// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/channelinfo.h, libAACdec/src/channelinfo.cpp.

//! Port of `channelinfo.h`: individual channel stream info, the per channel data structures of
//! the decoder core, and the `ics_info()` parser of `channelinfo.cpp`.
//!
//! # How to use these types
//!
//! The C structures share memory through pointers. The Rust structures store **no references
//! to memory they do not own**; the memory is owned once (by the decoder instance or by the
//! structure itself) and passed to the functions explicitly. This reproduces exactly which
//! memory the C code reads and writes, including stale contents of reused buffers, without
//! raw-pointer aliasing.
//!
//! ## Who owns what
//!
//! | C | Owner in the port | Notes |
//! |---|---|---|
//! | `pAacDecoderChannelInfo[ch]` | the decoder: `Box<`[`CAacDecoderChannelInfo`]`>` per channel ([`crate::aac_ram::get_aac_decoder_channel_info`]) | |
//! | `pAacDecoderStaticChannelInfo[ch]` | the decoder: `Box<`[`CAacDecoderStaticChannelInfo`]`>` per channel ([`crate::aac_ram::get_aac_decoder_static_channel_info`]) | |
//! | `pAacDecoderChannelInfo[ch]->pSpectralCoefficient` | the decoder's `workBufferCore2` ([`crate::aac_ram::get_work_buffer_core2`], `8 * 1024` values; channel `ch` at `ch * 1024`) | **not stored**: see rule 1 |
//! | `pAacDecoderChannelInfo[ch]->pDynData` | **owned by the channel**: [`CAacDecoderChannelInfo::p_dyn_data`] | in C it is `&pComData->pAacDecoderDynamicData[i]` for channel `i` of the element; nothing else accesses `pAacDecoderDynamicData`, so [`CAacDecoderCommonData`] does not contain it |
//! | `pAacDecoderChannelInfo[ch]->pComData` | the decoder, one [`CAacDecoderCommonData`] per channel element (both channels of a CPE share it; for the first element it is `workBufferCore6`) | **not stored**: rule 2 |
//! | `pAacDecoderChannelInfo[ch]->pComStaticData` | the decoder, one [`CAacDecoderCommonStaticData`] per channel element | **not stored**: rule 3 |
//! | `pComStaticData->pWorkBufferCore1` | the decoder, **one** [`CWorkBufferCore1`] shared by all elements (a second, separate instance is the decoder's own `workBufferCore1`) | **not stored**: rule 3 |
//! | `pAacDecoderStaticChannelInfo[ch]->pOverlapBuffer` | `IMdct` ([`fdk_aac_rs_fdk::mdct::MdctT::overlap`]) | code that uses `pOverlapBuffer` directly uses `i_mdct.overlap` |
//! | `pAacDecoderStaticChannelInfo[ch]->hArCo` | the static channel info: [`CAacDecoderStaticChannelInfo::h_ar_co`] (`None` for `NULL`) | |
//! | `pAacDecoderStaticChannelInfo[ch]->pCpeStaticData` | the decoder's `cpeStaticData[el]` (`Option<Box<`[`CpePersistentData`]`>>`) | stored as the element index [`CAacDecoderStaticChannelInfo::p_cpe_static_data`] (`None` for `NULL`); rule 4 |
//! | `concealmentInfo.pConcealParams` | the decoder's `concealCommonData` | not stored; the concealment functions take `&CConcealParams` |
//! | `CJointStereoPersistentData::{spectralCoeffs, specScale, scratchBuffer}` | see [`stereo_types`] | not stored |
//! | `CPnsData::{pPnsInterChannelData, currentSeed, randomSeed}` | see [`aacdec_pns_types`] | not stored |
//! | `data.usac.fac_data[k]` | `NULL`, `data.usac.fac_data0`, a part of the spectrum, or a function-local buffer | [`FacDataPtr`] |
//! | the HCR and RVLC pointers | see [`crate::aacdec_hcr_types`] and [`crate::rvlc_info`] | not stored, or a selector |
//!
//! ## Function signatures
//!
//! Functions keep the C parameters in the C order, with `CAacDecoderChannelInfo *p` as
//! `p: &mut CAacDecoderChannelInfo` and `CAacDecoderChannelInfo *p[]` (the channels of an
//! element) as `p: &mut [&mut CAacDecoderChannelInfo]` (the same for
//! `CAacDecoderStaticChannelInfo`). A function whose C code (including the functions it calls)
//! reaches memory through one of the unstored pointers gets that memory as additional
//! parameters, **appended after the C parameters in this order**:
//!
//! 1. `spec: &mut [FixpDbl]` for `pSpectralCoefficient`. For a single channel it starts at the
//!    channel's `pSpectralCoefficient` (at least 1024 values); for the channels of an element
//!    (`p[]`) it starts at the first channel's `pSpectralCoefficient` and channel `i` is at
//!    `spec[i * 1024..]` (the channels of an element are adjacent in `workBufferCore2`). The
//!    macros of `overlapadd.h` give the offsets ([`crate::overlapadd`]).
//! 2. `com: &mut CAacDecoderCommonData` for `pComData` (the element's).
//! 3. `com_static: &mut CAacDecoderCommonStaticData` for `pComStaticData->cplxPredictionData`,
//!    then `work_buffer_core1: &mut CWorkBufferCore1` for `pComStaticData->pWorkBufferCore1`.
//! 4. `cpe: Option<&mut CpePersistentData>` for `pCpeStaticData` (resolved by the caller from
//!    [`CAacDecoderStaticChannelInfo::p_cpe_static_data`]).
//! 5. Further memory in the order the C code first uses it (e.g. the joint stereo scratch
//!    buffer, `&CConcealParams`, the PNS seeds), each documented at the function.
//!
//! A C parameter that is itself a pointer into memory passed by these rules (for example
//! `H_HCR_INFO hHcr = &pComData->overlay.aac.erHcrInfo` next to `pAacDecoderChannelInfo`) stays a
//! separate parameter; the caller passes the field (`&mut com.overlay.aac.er_hcr_info`), and the
//! function does not take `com` unless it uses other parts of it.
//!
//! ## Allocation and initial contents
//!
//! [`crate::aac_ram`] has one function per `C_ALLOC_MEM` of `aac_ram.cpp`. The C allocations
//! are zero-filled (`FDKcalloc`, `FDKaalloc`), and so are the `Default` values of every type here.
//! When the decoder re-allocates `pComData` (`CAacDecoder_Init()` after `CAacDecoder_DeInit()`),
//! the channels' [`CAacDecoderChannelInfo::p_dyn_data`] must be reset to zero as well, because
//! they live in that allocation in C.
//!
//! # Unions
//!
//! `shouldBeUnion` is a `union` in the reference build. Decisions (PORTING.md, "Memory"):
//!
//! - [`CAacDecoderChannelInfo::data`] (`usac` / `aac`): **separate storage**. The USAC members
//!   are read only with USAC flags or the `AACDEC_RENDER_LPD` render mode, `data.aac.PnsData`
//!   only without USAC flags. The one read of a member that the current syntax does not write,
//!   `CLpc_Conceal(data.usac.lsp_coeff, ...)` in `CConcealment_Apply()` for every
//!   `AACDEC_RENDER_IMDCT` channel (also AAC), reads memory behind `fac_data0`, `fac_data_e`,
//!   `fac_data[4]` and the mode fields (more than 552 bytes from the start), which lies beyond
//!   `PnsData` (about 160 bytes) and is therefore never written by the AAC code in C either.
//! - [`CAacDecoderDynamicData::specific_to`] (`aac` / `usac`): **separate storage**. Every read
//!   of a member is guarded by the syntax (USAC flags or not) that writes it in the same or an
//!   earlier frame of the same configuration.
//! - [`CAacDecoderCommonData::overlay`] has a single member (`aac`): a plain structure.
//! - [`CWorkBufferCore1`] (`DrmBsBuffer`, `mdctOutTemp`, `synth_buf`, `workBuffer`): **one
//!   buffer** with views, because the members are used by different modules within one frame
//!   (the DRM SBR payload buffer, the IMDCT output, the LPD synthesis), so a read of stale
//!   contents written through another member is reproduced by construction.
//!
//! # Types of other headers
//!
//! `channelinfo.h` contains structures of `stereo.h`, `pulsedata.h`, `aacdec_tns.h` and
//! `aacdec_pns.h` by value, and its functions return `AAC_DECODER_ERROR` of
//! `aacdecoder_lib.h`. These types and constants are defined in the child modules
//! [`stereo_types`], [`pulsedata_types`], [`aacdec_tns_types`], [`aacdec_pns_types`] and
//! [`aacdecoder_lib_types`], and re-exported here; the modules of those headers re-export
//! them.

pub mod aacdec_pns_types;
pub mod aacdec_tns_types;
pub mod aacdecoder_lib_types;
pub mod pulsedata_types;
pub mod stereo_types;

pub use aacdec_pns_types::*;
pub use aacdec_tns_types::*;
pub use aacdecoder_lib_types::*;
pub use pulsedata_types::*;
pub use stereo_types::*;

use fdk_aac_rs_arith::ac_arith_coder::CArcoData;
use fdk_aac_rs_fdk::audio::{AC_BSAC, AC_ELD, AC_LD, AC_RSV603DA, AC_RSVD50, AC_SCALABLE, AC_USAC};
use fdk_aac_rs_fdk::bitstream::{fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::{FixpDbl, FixpSgl};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::lpc::FixpLpc;
use fdk_aac_rs_fdk::mdct::{BlockType, MdctT, BLOCK_LONG, BLOCK_SHORT};

use crate::aac_rom::{SFB_OFFSET_TABLES, TNS_MAX_BANDS_TBL};
use crate::aacdec_drc_types::CDrcChannelData;
use crate::aacdec_hcr_types::{CErHcrInfo, MAX_SFB_HCR};
use crate::conceal_types::CConcealmentInfo;
use crate::rvlc_info::{CErRvlcInfo, RVLC_MAX_SFB};
use crate::usacdec_acelp::{CAcelpChannelData, CAcelpStaticMem};
use crate::usacdec_const::{
    LFAC, L_FILT, L_FRAME_PLUS, L_SUBFR, M_LP_FILTER_ORDER, PIT_MAX_MAX, SYN_DELAY, SYN_SFD,
};

/// `WB_SECTION_SIZE`
pub const WB_SECTION_SIZE: i32 = 1024 * 2;

/// `DRM_BS_BUFFER_SIZE`: size of the dynamic buffer which is used to reverse the bits of the
/// DRM SBR payload
pub const DRM_BS_BUFFER_SIZE: i32 = 512;

/// `AACDEC_RENDER_MODE`: output rendering mode
pub type AacdecRenderMode = i32;
/// `AACDEC_RENDER_INVALID`
pub const AACDEC_RENDER_INVALID: AacdecRenderMode = 0;
/// `AACDEC_RENDER_IMDCT`
pub const AACDEC_RENDER_IMDCT: AacdecRenderMode = 1;
/// `AACDEC_RENDER_ELDFB`
pub const AACDEC_RENDER_ELDFB: AacdecRenderMode = 2;
/// `AACDEC_RENDER_LPD`
pub const AACDEC_RENDER_LPD: AacdecRenderMode = 3;
/// `AACDEC_RENDER_INTIMDCT`
pub const AACDEC_RENDER_INTIMDCT: AacdecRenderMode = 4;

/// `MAX_QUANTIZED_VALUE`
pub const MAX_QUANTIZED_VALUE: i32 = 8191;

/// `USAC_COREMODE`
pub type UsacCoremode = i32;
/// `FD_LONG`
pub const FD_LONG: UsacCoremode = 0;
/// `FD_SHORT`
pub const FD_SHORT: UsacCoremode = 1;
/// `LPD`
pub const LPD: UsacCoremode = 2;

/// `SamplingRateInfo`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SamplingRateInfo {
    /// `ScaleFactorBands_Long` (a table of `aac_rom`; empty for `NULL`)
    pub scale_factor_bands_long: &'static [i16],
    /// `ScaleFactorBands_Short` (a table of `aac_rom`; empty for `NULL`)
    pub scale_factor_bands_short: &'static [i16],
    /// `NumberOfScaleFactorBands_Long`
    pub number_of_scale_factor_bands_long: u8,
    /// `NumberOfScaleFactorBands_Short`
    pub number_of_scale_factor_bands_short: u8,
    /// `samplingRateIndex`
    pub sampling_rate_index: u32,
    /// `samplingRate`
    pub sampling_rate: u32,
}

/// `CRawDataInfo`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CRawDataInfo {
    /// `CommonWindow`
    pub common_window: u8,
    /// `GlobalGain`
    pub global_gain: u8,
}

/// `CIcsInfo`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CIcsInfo {
    /// `WindowGroupLength`
    pub window_group_length: [u8; 8],
    /// `WindowGroups`
    pub window_groups: u8,
    /// `Valid`
    pub valid: u8,

    /// `WindowShape`: 0: sine window, 1: KBD, 2: low overlap
    pub window_shape: u8,
    /// `WindowSequence` (`BLOCK_TYPE` of mdct.h): 0: long, 1: start, 2: short, 3: stop
    pub window_sequence: BlockType,
    /// `MaxSfBands`
    pub max_sf_bands: u8,
    /// `max_sfb_ste`
    pub max_sfb_ste: u8,
    /// `ScaleFactorGrouping`
    pub scale_factor_grouping: u8,

    /// `TotalSfBands`
    pub total_sf_bands: u8,
}

/* Huffman codebook numbers (an anonymous enum in C; `u8` like the `aCodeBook` entries) */
/// `ZERO_HCB`
pub const ZERO_HCB: u8 = 0;
/// `ESCBOOK`
pub const ESCBOOK: u8 = 11;
/// `NSPECBOOKS`
pub const NSPECBOOKS: u8 = ESCBOOK + 1;
/// `BOOKSCL`
pub const BOOKSCL: u8 = NSPECBOOKS;
/// `NOISE_HCB`
pub const NOISE_HCB: u8 = 13;
/// `INTENSITY_HCB2`
pub const INTENSITY_HCB2: u8 = 14;
/// `INTENSITY_HCB`
pub const INTENSITY_HCB: u8 = 15;
/// `LAST_HCB`
pub const LAST_HCB: u8 = 16;

/// `CpePersistentData`: the persistent data shared by both channels of a CPE. It needs to be
/// allocated for each CPE.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpePersistentData {
    /// `jointStereoPersistentData`
    pub joint_stereo_persistent_data: CJointStereoPersistentData,
}

/// Length of `CAacDecoderStaticChannelInfo::old_synth`: `PIT_MAX_MAX - L_SUBFR`.
pub const OLD_SYNTH_SIZE: usize = (PIT_MAX_MAX - L_SUBFR) as usize;
/// Length of `CAacDecoderStaticChannelInfo::mem_bpf`: `L_FILT + L_SUBFR`.
pub const MEM_BPF_SIZE: usize = (L_FILT + L_SUBFR) as usize;
const M_LP: usize = M_LP_FILTER_ORDER as usize;

/// `CAacDecoderStaticChannelInfo`: allocated once for every channel; persistent.
///
/// `pOverlapBuffer` is owned by `IMdct` ([`MdctT::overlap`]); `pCpeStaticData` is the index of
/// the element whose `CpePersistentData` it points to (see the
/// [module documentation](self)).
#[derive(Clone, Debug)]
pub struct CAacDecoderStaticChannelInfo {
    /// `IMdct` (owns `pOverlapBuffer`)
    pub i_mdct: MdctT,

    /// `hArCo` (`None` for `NULL`)
    pub h_ar_co: Option<Box<CArcoData>>,

    /// `pnsCurrentSeed`
    pub pns_current_seed: i32,

    /* LPD memory */
    /// `old_synth`
    pub old_synth: [FixpDbl; OLD_SYNTH_SIZE],
    /// `old_T_pf`
    pub old_t_pf: [i32; SYN_SFD as usize],
    /// `old_gain_pf`
    pub old_gain_pf: [FixpDbl; SYN_SFD as usize],
    /// `mem_bpf`
    pub mem_bpf: [FixpDbl; MEM_BPF_SIZE],
    /// `old_bpf_control_info`: (1: enable, 0: disable) bpf for past superframe
    pub old_bpf_control_info: u8,

    /// `last_core_mode`: core mode used by the decoder in previous frame. (not signalled by the
    /// bitstream, see `CAacDecoderChannelInfo::core_mode_last`!)
    pub last_core_mode: UsacCoremode,
    /// `last_lpd_mode`: LPD mode used by the decoder in last LPD subframe (not signalled by the
    /// bitstream, see `CAacDecoderChannelInfo::lpd_mode_last`!)
    pub last_lpd_mode: u8,
    /// `last_last_lpd_mode`: LPD mode used in second last LPD subframe (not signalled by the
    /// bitstream)
    pub last_last_lpd_mode: u8,
    /// `last_lpc_lost`: flag indicating that the previous LPC is lost
    pub last_lpc_lost: u8,

    /// `lpc4_lsf`: last LPC4 coefficients in LSF domain.
    pub lpc4_lsf: [FixpLpc; M_LP],
    /// `lsf_adaptive_mean`: adaptive mean of LPC coefficients in LSF domain for concealment.
    pub lsf_adaptive_mean: [FixpLpc; M_LP],
    /// `lp_coeff_old`: last LPC coefficients in LP domain. `lp_coeff_old[0]` is lpc4 (coeffs
    /// for right folding point of last tcx frame), `lp_coeff_old[1]` are coeffs for left
    /// folding point of last tcx frame
    pub lp_coeff_old: [[FixpLpc; M_LP]; 2],
    /// `lp_coeff_old_exp`
    pub lp_coeff_old_exp: [i32; 2],

    /// `oldStability`: LPC coeff stability value from last frame (required for TCX
    /// concealment).
    pub old_stability: FixpSgl,
    /// `numLostLpdFrames`: number of consecutive lost subframes.
    pub num_lost_lpd_frames: u32,

    /* TCX memory */
    /// `last_tcx_gain`
    pub last_tcx_gain: FixpDbl,
    /// `last_tcx_gain_e`
    pub last_tcx_gain_e: i32,
    /// `last_alfd_gains`: scaled by one bit.
    pub last_alfd_gains: [FixpDbl; 32],
    /// `last_tcx_pitch`
    pub last_tcx_pitch: i16,
    /// `last_tcx_noise_factor`
    pub last_tcx_noise_factor: u8,

    /* ACELP memory */
    /// `acelp`
    pub acelp: CAcelpStaticMem,

    /// `nfRandomSeed`: seed value for USAC noise filling random generator
    pub nf_random_seed: u32,

    /// `drcData`
    pub drc_data: CDrcChannelData,
    /// `concealmentInfo`
    pub concealment_info: CConcealmentInfo,

    /// `pCpeStaticData`: the index `el` of the decoder's `cpeStaticData[el]` it points to
    /// (`None` for `NULL`).
    pub p_cpe_static_data: Option<usize>,
}

impl Default for CAacDecoderStaticChannelInfo {
    /// Zero-initialized like `GetAacDecoderStaticChannelInfo()` (`FDKcalloc`). The overlap
    /// buffer of `i_mdct` is empty until the decoder assigns it (`GetOverlapBuffer()`).
    fn default() -> Self {
        CAacDecoderStaticChannelInfo {
            i_mdct: MdctT::default(),
            h_ar_co: None,
            pns_current_seed: 0,
            old_synth: [0; OLD_SYNTH_SIZE],
            old_t_pf: [0; SYN_SFD as usize],
            old_gain_pf: [0; SYN_SFD as usize],
            mem_bpf: [0; MEM_BPF_SIZE],
            old_bpf_control_info: 0,
            last_core_mode: 0,
            last_lpd_mode: 0,
            last_last_lpd_mode: 0,
            last_lpc_lost: 0,
            lpc4_lsf: [0; M_LP],
            lsf_adaptive_mean: [0; M_LP],
            lp_coeff_old: [[0; M_LP]; 2],
            lp_coeff_old_exp: [0; 2],
            old_stability: 0,
            num_lost_lpd_frames: 0,
            last_tcx_gain: 0,
            last_tcx_gain_e: 0,
            last_alfd_gains: [0; 32],
            last_tcx_pitch: 0,
            last_tcx_noise_factor: 0,
            acelp: CAcelpStaticMem::default(),
            nf_random_seed: 0,
            drc_data: CDrcChannelData::default(),
            concealment_info: CConcealmentInfo::default(),
            p_cpe_static_data: None,
        }
    }
}

/// `CAacDecoderDynamicData::specificTo.aac`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CAacDecoderDynamicDataAac {
    /// `PulseData`
    pub pulse_data: CPulseData,
    /// `aNumLineInSec4Hcr`: needed once for all channels except for Drm syntax
    pub a_num_line_in_sec4_hcr: [i16; MAX_SFB_HCR as usize],
    /// `aCodeBooks4Hcr`: needed once for all channels except for Drm syntax. Same as
    /// "aCodeBook" ?
    pub a_code_books4_hcr: [u8; MAX_SFB_HCR as usize],
    /// `lenOfReorderedSpectralData`
    pub len_of_reordered_spectral_data: i16,
    /// `lenOfLongestCodeword`
    pub len_of_longest_codeword: i8,
    /// `numberSection`
    pub number_section: i8,
    /// `rvlcCurrentScaleFactorOK`
    pub rvlc_current_scale_factor_ok: i8,
    /// `rvlcIntensityUsed`
    pub rvlc_intensity_used: i8,
}

impl Default for CAacDecoderDynamicDataAac {
    fn default() -> Self {
        CAacDecoderDynamicDataAac {
            pulse_data: CPulseData::default(),
            a_num_line_in_sec4_hcr: [0; MAX_SFB_HCR as usize],
            a_code_books4_hcr: [0; MAX_SFB_HCR as usize],
            len_of_reordered_spectral_data: 0,
            len_of_longest_codeword: 0,
            number_section: 0,
            rvlc_current_scale_factor_ok: 0,
            rvlc_intensity_used: 0,
        }
    }
}

/// `CAacDecoderDynamicData::specificTo.usac`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CAacDecoderDynamicDataUsac {
    /// `fd_noise_level_and_offset`
    pub fd_noise_level_and_offset: u8,
    /// `tns_active`
    pub tns_active: u8,
    /// `tns_on_lr`
    pub tns_on_lr: u8,
    /// `tcx_noise_factor`
    pub tcx_noise_factor: [u8; 4],
    /// `tcx_global_gain`
    pub tcx_global_gain: [u8; 4],
}

/// `CAacDecoderDynamicData::specificTo` (a union in C, separate storage here; see the
/// [module documentation](self#unions)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CAacDecoderDynamicDataSpecificTo {
    /// `aac`
    pub aac: CAacDecoderDynamicDataAac,
    /// `usac`
    pub usac: CAacDecoderDynamicDataUsac,
}

/// `CAacDecoderDynamicData`: data of one channel required for one element only
/// (`CAacDecoderChannelInfo::pDynData`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CAacDecoderDynamicData {
    /* Common bit stream data */
    /// `aScaleFactor`: spectral scale factors for each sfb in each window.
    pub a_scale_factor: [i16; 8 * 16],
    /// `aSfbScale`: could be free after ApplyTools()
    pub a_sfb_scale: [i16; 8 * 16],
    /// `aCodeBook`: section data: codebook for each window and sfb.
    pub a_code_book: [u8; 8 * 16],
    /// `band_is_noise`
    pub band_is_noise: [u8; 8 * 16],
    /// `TnsData`
    pub tns_data: CTnsData,
    /// `RawDataInfo`
    pub raw_data_info: CRawDataInfo,

    /// `specificTo`
    pub specific_to: CAacDecoderDynamicDataSpecificTo,
}

impl Default for CAacDecoderDynamicData {
    fn default() -> Self {
        CAacDecoderDynamicData {
            a_scale_factor: [0; 8 * 16],
            a_sfb_scale: [0; 8 * 16],
            a_code_book: [0; 8 * 16],
            band_is_noise: [0; 8 * 16],
            tns_data: CTnsData::default(),
            raw_data_info: CRawDataInfo::default(),
            specific_to: CAacDecoderDynamicDataSpecificTo::default(),
        }
    }
}

/// Length of `CWorkBufferCore1::synth_buf`: `PIT_MAX_MAX + SYN_DELAY + L_FRAME_PLUS`.
pub const SYNTH_BUF_SIZE: usize = (PIT_MAX_MAX + SYN_DELAY + L_FRAME_PLUS) as usize;

/// `CWorkBufferCore1`: `union { UCHAR DrmBsBuffer[512]; FIXP_DBL mdctOutTemp[1024]; FIXP_DBL
/// synth_buf[PIT_MAX_MAX + SYN_DELAY + L_FRAME_PLUS]; FIXP_DBL workBuffer[WB_SECTION_SIZE]; }`.
///
/// Kept as **one buffer** of `WB_SECTION_SIZE` values (8192 bytes, the size of the C union),
/// with a view per member: all members start at the same address, as in C (see the
/// [module documentation](self#unions)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CWorkBufferCore1 {
    /// The memory of the union (`workBuffer`).
    pub buf: [FixpDbl; WB_SECTION_SIZE as usize],
}

impl Default for CWorkBufferCore1 {
    /// Zero-initialized like `GetWorkBufferCore1()` (`FDKaalloc_L`).
    fn default() -> Self {
        CWorkBufferCore1 { buf: [0; WB_SECTION_SIZE as usize] }
    }
}

impl CWorkBufferCore1 {
    /// `mdctOutTemp` (1024 values)
    #[inline]
    pub fn mdct_out_temp(&mut self) -> &mut [FixpDbl] {
        &mut self.buf[..1024]
    }

    /// `synth_buf` (`PIT_MAX_MAX + SYN_DELAY + L_FRAME_PLUS` values)
    #[inline]
    pub fn synth_buf(&mut self) -> &mut [FixpDbl] {
        &mut self.buf[..SYNTH_BUF_SIZE]
    }

    /// `workBuffer` (`WB_SECTION_SIZE` values)
    #[inline]
    pub fn work_buffer(&mut self) -> &mut [FixpDbl] {
        &mut self.buf[..]
    }

    /// `DrmBsBuffer` (`DRM_BS_BUFFER_SIZE` bytes): the first bytes of the union, in the memory
    /// order of the (little-endian) reference.
    #[inline]
    pub fn drm_bs_buffer(&mut self) -> &mut [u8] {
        // SAFETY: `buf` is an array of `i32` of 8192 bytes; every byte pattern is a valid `u8`,
        // `u8` has alignment 1, and the returned slice (512 bytes) borrows `self` mutably, so
        // no other access to `buf` can overlap it.
        unsafe {
            core::slice::from_raw_parts_mut(
                self.buf.as_mut_ptr() as *mut u8,
                DRM_BS_BUFFER_SIZE as usize,
            )
        }
    }
}

/// `CAacDecoderCommonData::overlay.aac`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CAacDecoderCommonDataOverlayAac {
    /// `erHcrInfo`
    pub er_hcr_info: CErHcrInfo,
    /// `erRvlcInfo`
    pub er_rvlc_info: CErRvlcInfo,
    /// `aRvlcScfEsc`: needed once for all channels
    pub a_rvlc_scf_esc: [i16; RVLC_MAX_SFB as usize],
    /// `aRvlcScfFwd`: needed once for all channels
    pub a_rvlc_scf_fwd: [i16; RVLC_MAX_SFB as usize],
    /// `aRvlcScfBwd`: needed once for all channels
    pub a_rvlc_scf_bwd: [i16; RVLC_MAX_SFB as usize],
}

impl Default for CAacDecoderCommonDataOverlayAac {
    fn default() -> Self {
        CAacDecoderCommonDataOverlayAac {
            er_hcr_info: CErHcrInfo::default(),
            er_rvlc_info: CErRvlcInfo::default(),
            a_rvlc_scf_esc: [0; RVLC_MAX_SFB as usize],
            a_rvlc_scf_fwd: [0; RVLC_MAX_SFB as usize],
            a_rvlc_scf_bwd: [0; RVLC_MAX_SFB as usize],
        }
    }
}

/// `CAacDecoderCommonData::overlay` (a union with one member in C)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CAacDecoderCommonDataOverlay {
    /// `aac`
    pub aac: CAacDecoderCommonDataOverlayAac,
}

/// `CAacDecoderCommonData`: common data referenced by all channels of an element
/// (`pComData`). `pAacDecoderDynamicData[2]` is owned by the channels
/// ([`CAacDecoderChannelInfo::p_dyn_data`]), see the [module documentation](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CAacDecoderCommonData {
    /// `pnsInterChannelData`
    pub pns_inter_channel_data: CPnsInterChannelData,
    /// `pnsRandomSeed`
    pub pns_random_seed: [i32; 8 * 16],

    /// `jointStereoData`: one for one element
    pub joint_stereo_data: CJointStereoData,

    /// `overlay`
    pub overlay: CAacDecoderCommonDataOverlay,
}

impl Default for CAacDecoderCommonData {
    fn default() -> Self {
        CAacDecoderCommonData {
            pns_inter_channel_data: CPnsInterChannelData::default(),
            pns_random_seed: [0; 8 * 16],
            joint_stereo_data: CJointStereoData::default(),
            overlay: CAacDecoderCommonDataOverlay::default(),
        }
    }
}

/// `CAacDecoderCommonStaticData` (`pComStaticData`) without `pWorkBufferCore1`, which is one
/// decoder-wide [`CWorkBufferCore1`] passed separately (see the [module documentation](self)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CAacDecoderCommonStaticData {
    /// `cplxPredictionData` (`None` for `NULL`)
    pub cplx_prediction_data: Option<Box<CCplxPredictionData>>,
}

/// `data.usac.fac_data[k]`: where the FAC data pointer points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FacDataPtr {
    /// `NULL`
    #[default]
    Null,
    /// `data.usac.fac_data0` of the same channel
    FacData0,
    /// `pSpectralCoefficient + offset` of the same channel (`CLpd_FAC_GetMemory()`)
    Spectral(usize),
    /// A buffer local to the function that set it (`fac_buf` in `CBlock_FrequencyToTime()` and
    /// `CLpd_RenderTimeSignal()`). After that function returns, C keeps a dangling pointer,
    /// which is only compared with `NULL` before it is set again.
    Scratch,
}

/// `CAacDecoderChannelInfo::data.usac`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CAacDecoderChannelInfoUsac {
    /// `fac_data0`
    pub fac_data0: [FixpDbl; LFAC as usize],
    /// `fac_data_e`
    pub fac_data_e: [i8; 4],
    /// `fac_data`: pointers to unused parts of `pSpectralCoefficient` ([`FacDataPtr`])
    pub fac_data: [FacDataPtr; 4],

    /// `core_mode`: current core mode
    pub core_mode: u8,
    /// `core_mode_last`: previous core mode, signalled in the bitstream (not done by the
    /// decoder, see `CAacDecoderStaticChannelInfo::last_core_mode`!)
    pub core_mode_last: UsacCoremode,
    /// `lpd_mode_last`: previous LPD mode, signalled in the bitstream (not done by the decoder,
    /// see `CAacDecoderStaticChannelInfo::last_core_mode`!)
    pub lpd_mode_last: u8,
    /// `mod`
    pub r#mod: [u8; 4],
    /// `bpf_control_info`: (1: enable, 0: disable) bpf for current superframe
    pub bpf_control_info: u8,

    /// `lsp_coeff`: linear prediction coefficients in LSP domain
    pub lsp_coeff: [[FixpLpc; M_LP]; 5],
    /// `lp_coeff`: linear prediction coefficients in LP domain
    pub lp_coeff: [[FixpLpc; M_LP]; 5],
    /// `lp_coeff_exp`
    pub lp_coeff_exp: [i32; 5],
    /// `lsf_adaptive_mean_cand`: concealment: is copied to
    /// `CAacDecoderStaticChannelInfo->lsf_adaptive_mean` once frame is assumed to be correct
    pub lsf_adaptive_mean_cand: [FixpLpc; M_LP],
    /// `aStability`: LPC coeff stability values required for ACELP and TCX (concealment)
    pub a_stability: [FixpSgl; 4],

    /// `acelp`
    pub acelp: [CAcelpChannelData; 4],

    /// `tcx_gain`
    pub tcx_gain: [FixpDbl; 4],
    /// `tcx_gain_e`
    pub tcx_gain_e: [i8; 4],
}

impl Default for CAacDecoderChannelInfoUsac {
    fn default() -> Self {
        CAacDecoderChannelInfoUsac {
            fac_data0: [0; LFAC as usize],
            fac_data_e: [0; 4],
            fac_data: [FacDataPtr::Null; 4],
            core_mode: 0,
            core_mode_last: 0,
            lpd_mode_last: 0,
            r#mod: [0; 4],
            bpf_control_info: 0,
            lsp_coeff: [[0; M_LP]; 5],
            lp_coeff: [[0; M_LP]; 5],
            lp_coeff_exp: [0; 5],
            lsf_adaptive_mean_cand: [0; M_LP],
            a_stability: [0; 4],
            acelp: [CAcelpChannelData::default(); 4],
            tcx_gain: [0; 4],
            tcx_gain_e: [0; 4],
        }
    }
}

/// `CAacDecoderChannelInfo::data.aac`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CAacDecoderChannelInfoAac {
    /// `PnsData`: not required for USAC
    pub pns_data: CPnsData,
}

/// `CAacDecoderChannelInfo::data` (a union in C, separate storage here; see the
/// [module documentation](self#unions)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CAacDecoderChannelInfoData {
    /// `usac`
    pub usac: CAacDecoderChannelInfoUsac,
    /// `aac`
    pub aac: CAacDecoderChannelInfoAac,
}

/// `CAacDecoderChannelInfo`: allocated once for every channel of every element; persistent.
///
/// `pSpectralCoefficient`, `pComData` and `pComStaticData` are not stored; `pDynData` is
/// owned. See the [module documentation](self) for the rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CAacDecoderChannelInfo {
    /// `data`
    pub data: CAacDecoderChannelInfoData,

    /// `specScale`: scale shift values of each spectrum window
    pub spec_scale: [i16; 8],
    /// `icsInfo`
    pub ics_info: CIcsInfo,
    /// `granuleLength`: size of smallest spectrum piece
    pub granule_length: i32,
    /// `ElementInstanceTag`
    pub element_instance_tag: u8,

    /// `renderMode`: output signal rendering mode
    pub render_mode: AacdecRenderMode,

    /// `pDynData`: data required for one element and discarded after decoding (owned, see the
    /// [module documentation](self))
    pub p_dyn_data: Box<CAacDecoderDynamicData>,

    /// `currAliasingSymmetry`: required for RSVD60 MCT
    pub curr_aliasing_symmetry: i32,
}

impl Default for CAacDecoderChannelInfo {
    /// Zero-initialized like `GetAacDecoderChannelInfo()` (`FDKaalloc`) and the `pComData`
    /// allocation that holds `pDynData`.
    fn default() -> Self {
        CAacDecoderChannelInfo {
            data: CAacDecoderChannelInfoData::default(),
            spec_scale: [0; 8],
            ics_info: CIcsInfo::default(),
            granule_length: 0,
            element_instance_tag: 0,
            render_mode: AACDEC_RENDER_INVALID,
            p_dyn_data: Box::default(),
            curr_aliasing_symmetry: 0,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// channelinfo.cpp
// ---------------------------------------------------------------------------------------------

/// `IcsReadMaxSfb(hBs, pIcsInfo, pSamplingRateInfo)`: read max SFB from bit stream and assign
/// `TotalSfBands` according to the window sequence and sample rate.
pub fn ics_read_max_sfb(
    bs: &mut FdkBitStream,
    p_ics_info: &mut CIcsInfo,
    p_sampling_rate_info: &SamplingRateInfo,
) -> AacDecoderError {
    let mut error_status = AAC_DEC_OK;
    let nbits;

    if is_long_block(p_ics_info) != 0 {
        nbits = 6;
        p_ics_info.total_sf_bands = p_sampling_rate_info.number_of_scale_factor_bands_long;
    } else {
        nbits = 4;
        p_ics_info.total_sf_bands = p_sampling_rate_info.number_of_scale_factor_bands_short;
    }
    p_ics_info.max_sf_bands = fdk_read_bits(bs, nbits) as u8;

    if p_ics_info.max_sf_bands > p_ics_info.total_sf_bands {
        error_status = AAC_DEC_PARSE_ERROR;
    }

    error_status
}

/// `IcsRead(bs, pIcsInfo, pSamplingRateInfo, flags)`
pub fn ics_read(
    bs: &mut FdkBitStream,
    p_ics_info: &mut CIcsInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
) -> AacDecoderError {
    let mut error_status;

    p_ics_info.valid = 0;

    'bail: {
        if flags & AC_ELD != 0 {
            p_ics_info.window_sequence = BLOCK_LONG;
            p_ics_info.window_shape = 0;
        } else {
            if flags & (AC_USAC | AC_RSVD50 | AC_RSV603DA) == 0 {
                fdk_read_bits(bs, 1);
            }
            p_ics_info.window_sequence = fdk_read_bits(bs, 2) as BlockType;
            p_ics_info.window_shape = fdk_read_bits(bs, 1) as u8;
            if flags & AC_LD != 0 {
                if p_ics_info.window_shape != 0 {
                    p_ics_info.window_shape = 2; /* select low overlap instead of KBD */
                }
            }
        }

        /* Sanity check */
        if (flags & (AC_ELD | AC_LD)) != 0 && p_ics_info.window_sequence != BLOCK_LONG {
            p_ics_info.window_sequence = BLOCK_LONG;
            error_status = AAC_DEC_PARSE_ERROR;
            break 'bail;
        }

        error_status = ics_read_max_sfb(bs, p_ics_info, p_sampling_rate_info);
        if error_status != AAC_DEC_OK {
            break 'bail;
        }

        if is_long_block(p_ics_info) != 0 {
            /* If not ELD nor Scalable nor BSAC nor USAC syntax then ... */
            if flags & (AC_ELD | AC_SCALABLE | AC_BSAC | AC_USAC | AC_RSVD50 | AC_RSV603DA) == 0 {
                if fdk_read_bits(bs, 1) as u8 != 0
                /* UCHAR PredictorDataPresent */
                {
                    error_status = AAC_DEC_UNSUPPORTED_PREDICTION;
                    break 'bail;
                }
            }

            p_ics_info.window_groups = 1;
            p_ics_info.window_group_length[0] = 1;
        } else {
            p_ics_info.scale_factor_grouping = fdk_read_bits(bs, 7) as u8;

            p_ics_info.window_groups = 0;

            for i in 0..(8 - 1) {
                let mask: u32 = 1 << (6 - i);
                p_ics_info.window_group_length[i] = 1;

                if p_ics_info.scale_factor_grouping as u32 & mask != 0 {
                    p_ics_info.window_group_length[p_ics_info.window_groups as usize] += 1;
                } else {
                    p_ics_info.window_groups += 1;
                }
            }

            /* loop runs to i < 7 only */
            p_ics_info.window_group_length[8 - 1] = 1;
            p_ics_info.window_groups += 1;
        }
    }

    /* bail: */
    if error_status == AAC_DEC_OK {
        p_ics_info.valid = 1;
    }

    error_status
}

/// `getSamplingRateInfo(t, samplesPerFrame, samplingRateIndex, samplingRate)`
///
/// Table entries are sorted as following: `| num_swb_long_window | sfbands_long |
/// num_swb_short_window | sfbands_short |`
pub fn get_sampling_rate_info(
    t: &mut SamplingRateInfo,
    samples_per_frame: u32,
    mut sampling_rate_index: u32,
    sampling_rate: u32,
) -> AacDecoderError {
    /* Search closest samplerate according to ISO/IEC 13818-7:2005(E) 8.2.4 (Table 38): */
    if (sampling_rate_index >= 15) || (samples_per_frame == 768) {
        const BORDERS: [u32; 12] =
            [u32::MAX, 92017, 75132, 55426, 46009, 37566, 27713, 23004, 18783, 13856, 11502, 9391];
        let mut sampling_rate_search = sampling_rate;

        if samples_per_frame == 768 {
            // (samplingRate * 4) / 3 in UINT arithmetic (wraps for rates above 2^30).
            sampling_rate_search = sampling_rate.wrapping_mul(4) / 3;
        }

        let mut i: u32 = 0;
        while i < 11 {
            if BORDERS[i as usize] > sampling_rate_search
                && sampling_rate_search >= BORDERS[i as usize + 1]
            {
                break;
            }
            i += 1;
        }
        sampling_rate_index = i;
    }

    t.sampling_rate_index = sampling_rate_index;
    t.sampling_rate = sampling_rate;

    let index: usize = match samples_per_frame {
        1024 => 0,
        960 => 1,
        768 => 2,
        512 => 3,
        480 => 4,
        _ => return AAC_DEC_UNSUPPORTED_FORMAT,
    };

    // samplingRateIndex is below 15 here (or 11 at most after the search).
    let info = &SFB_OFFSET_TABLES[index][sampling_rate_index as usize];
    t.scale_factor_bands_long = info.sfb_offset_long;
    t.scale_factor_bands_short = info.sfb_offset_short;
    t.number_of_scale_factor_bands_long = info.number_of_sfb_long;
    t.number_of_scale_factor_bands_short = info.number_of_sfb_short;

    if t.scale_factor_bands_long.is_empty() || t.number_of_scale_factor_bands_long == 0 {
        t.sampling_rate = 0;
        return AAC_DEC_UNSUPPORTED_FORMAT;
    }

    fdk_assert!(t
        .scale_factor_bands_long
        .get(t.number_of_scale_factor_bands_long as usize)
        .is_some_and(|&v| v as u32 == samples_per_frame));
    fdk_assert!(
        t.scale_factor_bands_short.is_empty()
            || t.scale_factor_bands_short
                .get(t.number_of_scale_factor_bands_short as usize)
                .is_some_and(|&v| (v as u32).wrapping_mul(8) == samples_per_frame)
    );

    AAC_DEC_OK
}

/****************** inline functions ******************/

/// `IsValid(pIcsInfo)`
#[inline]
pub fn is_valid(p_ics_info: &CIcsInfo) -> u8 {
    p_ics_info.valid
}

/// `IsLongBlock(pIcsInfo)`
#[inline]
pub fn is_long_block(p_ics_info: &CIcsInfo) -> u8 {
    (p_ics_info.window_sequence != BLOCK_SHORT) as u8
}

/// `GetWindowShape(pIcsInfo)`
#[inline]
pub fn get_window_shape(p_ics_info: &CIcsInfo) -> u8 {
    p_ics_info.window_shape
}

/// `GetWindowSequence(pIcsInfo)`
#[inline]
pub fn get_window_sequence(p_ics_info: &CIcsInfo) -> BlockType {
    p_ics_info.window_sequence
}

/// `GetScaleFactorBandOffsets(pIcsInfo, samplingRateInfo)` (empty for `NULL`)
#[inline]
pub fn get_scale_factor_band_offsets(
    p_ics_info: &CIcsInfo,
    sampling_rate_info: &SamplingRateInfo,
) -> &'static [i16] {
    if is_long_block(p_ics_info) != 0 {
        sampling_rate_info.scale_factor_bands_long
    } else {
        sampling_rate_info.scale_factor_bands_short
    }
}

/// `GetNumberOfScaleFactorBands(pIcsInfo, samplingRateInfo)`
#[inline]
pub fn get_number_of_scale_factor_bands(
    p_ics_info: &CIcsInfo,
    sampling_rate_info: &SamplingRateInfo,
) -> u8 {
    if is_long_block(p_ics_info) != 0 {
        sampling_rate_info.number_of_scale_factor_bands_long
    } else {
        sampling_rate_info.number_of_scale_factor_bands_short
    }
}

/// `GetWindowsPerFrame(pIcsInfo)`
#[inline]
pub fn get_windows_per_frame(p_ics_info: &CIcsInfo) -> i32 {
    if p_ics_info.window_sequence == BLOCK_SHORT {
        8
    } else {
        1
    }
}

/// `GetWindowGroups(pIcsInfo)`
#[inline]
pub fn get_window_groups(p_ics_info: &CIcsInfo) -> u8 {
    p_ics_info.window_groups
}

/// `GetWindowGroupLength(pIcsInfo, index)`
#[inline]
pub fn get_window_group_length(p_ics_info: &CIcsInfo, index: i32) -> u8 {
    p_ics_info.window_group_length[index as usize]
}

/// `GetWindowGroupLengthTable(pIcsInfo)`
#[inline]
pub fn get_window_group_length_table(p_ics_info: &CIcsInfo) -> &[u8; 8] {
    &p_ics_info.window_group_length
}

/// `GetScaleFactorBandsTransmitted(pIcsInfo)`
#[inline]
pub fn get_scale_factor_bands_transmitted(p_ics_info: &CIcsInfo) -> u8 {
    p_ics_info.max_sf_bands
}

/// `GetScaleMaxFactorBandsTransmitted(pIcsInfo0, pIcsInfo1)`
#[inline]
pub fn get_scale_max_factor_bands_transmitted(
    p_ics_info0: &CIcsInfo,
    p_ics_info1: &CIcsInfo,
) -> u8 {
    p_ics_info0.max_sf_bands.max(p_ics_info1.max_sf_bands)
}

/// `GetScaleFactorBandsTotal(pIcsInfo)`
#[inline]
pub fn get_scale_factor_bands_total(p_ics_info: &CIcsInfo) -> u8 {
    p_ics_info.total_sf_bands
}

/// `GetMaximumTnsBands(pIcsInfo, samplingRateIndex)`. Note: This function applies to AAC-LC
/// only!
///
/// `samplingRateIndex` must be below 13 (`tns_max_bands_tbl[13][2]`); otherwise the port
/// returns 0.
#[inline]
pub fn get_maximum_tns_bands(p_ics_info: &CIcsInfo, sampling_rate_index: i32) -> u8 {
    TNS_MAX_BANDS_TBL
        .get(sampling_rate_index as usize)
        .map_or(0, |e| e[(is_long_block(p_ics_info) == 0) as usize])
}
