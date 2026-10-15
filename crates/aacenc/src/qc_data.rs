// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/qc_data.h.

//! Port of `qc_data.h`: data of the quantizing & coding stage.
//!
//! # Memory model
//!
//! `QC_OUT_ELEMENT::qcOutChannel[ch]` points into `QC_OUT::pQcOutChannels[]` in C; the port
//! stores the indexes. `FDKaacEnc_QCOutInit()` assigns the channels of an element
//! consecutively, so they form the contiguous slice
//! `p_qc_out_channels[qc_out_channel[0]..qc_out_channel[0] + nChannelsInEl]`. The same holds for
//! `PSY_OUT` (see [`interface`](crate::interface)), whose channel `i` aliases the
//! `QC_OUT_CHANNEL` arrays of channel `i` (`mdctSpectrum`, `sfbEnergy`, ...).
//!
//! The `QC_OUT_CHANNEL`s live in the shared dynamic RAM of the encoder in C (`P_BUF_0`, used for
//! nothing else, so they persist between frames); here they are owned by [`QcOut`]. The scratch
//! buffers `dynMem_Ah_Flag`, `dynMem_Thr_Exp` and `dynMem_SfbNActiveLinesLdData` of
//! `QC_OUT_ELEMENT` (all elements share the same memory in `P_BUF_1`) are not part of the port's
//! structure: the threshold adjustment uses its own buffers.

use crate::adj_thr_data::{AacencBitresMode, AdjThrState};
use crate::dyn_bits::{BitcntrState, SectionData};
use crate::line_pe::PeData;
use crate::psy_const::MAX_GROUPED_SFB;
use fdk_aac_rs_fdk::audio::{ChannelMode, ExtPayloadType, Mp4ElementId};
use fdk_aac_rs_fdk::common_fix::FixpDbl;

/// `QCDATA_BR_MODE`
pub type QcdataBrMode = i32;
/// `QCDATA_BR_MODE_INVALID`
pub const QCDATA_BR_MODE_INVALID: QcdataBrMode = -1;
/// Constant bit rate, given average bitrate
pub const QCDATA_BR_MODE_CBR: QcdataBrMode = 0;
/// Variable bit rate, very low
pub const QCDATA_BR_MODE_VBR_1: QcdataBrMode = 1;
/// Variable bit rate, low
pub const QCDATA_BR_MODE_VBR_2: QcdataBrMode = 2;
/// Variable bit rate, medium
pub const QCDATA_BR_MODE_VBR_3: QcdataBrMode = 3;
/// Variable bit rate, high
pub const QCDATA_BR_MODE_VBR_4: QcdataBrMode = 4;
/// Variable bit rate, very high
pub const QCDATA_BR_MODE_VBR_5: QcdataBrMode = 5;
/// Fixed frame mode.
pub const QCDATA_BR_MODE_FF: QcdataBrMode = 6;
/// Superframe mode.
pub const QCDATA_BR_MODE_SFR: QcdataBrMode = 7;

/// `ELEMENT_INFO` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ElementInfo {
    pub el_type: Mp4ElementId,
    pub instance_tag: i32,
    pub n_channels_in_el: i32,
    pub channel_index: [i32; 2],
    pub relative_bits: FixpDbl,
}

/// `CHANNEL_MAPPING` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelMapping {
    pub enc_mode: ChannelMode,
    pub n_channels: i32,
    pub n_channels_eff: i32,
    pub n_elements: i32,
    pub el_info: [ElementInfo; 8],
}

/// `PADDING`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Padding {
    pub padding_rest: i32,
}

/// `struct QC_INIT`
///
/// `channelMapping` is a pointer in C, which `FDKaacEnc_QCInit()` only reads; here it is a
/// copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QcInit {
    pub channel_mapping: ChannelMapping,
    /// not used yet
    pub sce_cpe: i32,
    /// maximum number of bits in reservoir
    pub max_bits: i32,
    /// average number of bits we should use
    pub average_bits: i32,
    pub bit_res: i32,
    /// output sample rate
    pub sample_rate: i32,
    /// if set, calc bits2PE factor depending on samplerate
    pub is_low_delay: i32,
    /// Bits per frame consumed by transport layers.
    pub static_bits: i32,
    pub bitrate_mode: QcdataBrMode,
    pub mean_pe: i32,
    /// Bitrate/channel
    pub ch_bitrate: i32,
    pub inv_quant: i32,
    /// Maximum number of allowed iterations before FDKaacEnc_crashRecovery() is applied.
    pub max_iterations: i32,
    pub max_bit_fac: FixpDbl,
    pub bitrate: i32,
    /// helper variable
    pub n_sub_frames: i32,
    /// minimal number of bits in one frame
    pub min_bits: i32,
    /// 0: full bitreservoir, 1: reduced bitreservoir, 2: disabled bitreservoir
    pub bit_res_mode: AacencBitresMode,
    /// Configure element-wise execution or execution over all elements for the pe-dependent
    /// threshold-adaption
    pub bit_distribution_mode: i32,
    pub padding: Padding,
}

/// `QC_OUT_CHANNEL` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QcOutChannel {
    pub mdct_spectrum: [FixpDbl; 1024],
    pub quant_spec: [i16; 1024],
    pub max_value_in_sfb: [u32; MAX_GROUPED_SFB],
    pub scf: [i32; MAX_GROUPED_SFB],
    pub global_gain: i32,
    pub section_data: SectionData,
    pub sfb_form_factor_ld_data: [FixpDbl; MAX_GROUPED_SFB],
    pub sfb_threshold_ld_data: [FixpDbl; MAX_GROUPED_SFB],
    pub sfb_min_snr_ld_data: [FixpDbl; MAX_GROUPED_SFB],
    pub sfb_energy_ld_data: [FixpDbl; MAX_GROUPED_SFB],
    pub sfb_energy: [FixpDbl; MAX_GROUPED_SFB],
    pub sfb_weighted_energy_ld_data: [FixpDbl; MAX_GROUPED_SFB],
    pub sfb_en_fac_ld: [FixpDbl; MAX_GROUPED_SFB],
    pub sfb_spread_energy: [FixpDbl; MAX_GROUPED_SFB],
}

impl Default for QcOutChannel {
    fn default() -> Self {
        QcOutChannel {
            mdct_spectrum: [0; 1024],
            quant_spec: [0; 1024],
            max_value_in_sfb: [0; MAX_GROUPED_SFB],
            scf: [0; MAX_GROUPED_SFB],
            global_gain: 0,
            section_data: SectionData::default(),
            sfb_form_factor_ld_data: [0; MAX_GROUPED_SFB],
            sfb_threshold_ld_data: [0; MAX_GROUPED_SFB],
            sfb_min_snr_ld_data: [0; MAX_GROUPED_SFB],
            sfb_energy_ld_data: [0; MAX_GROUPED_SFB],
            sfb_energy: [0; MAX_GROUPED_SFB],
            sfb_weighted_energy_ld_data: [0; MAX_GROUPED_SFB],
            sfb_en_fac_ld: [0; MAX_GROUPED_SFB],
            sfb_spread_energy: [0; MAX_GROUPED_SFB],
        }
    }
}

/// `QC_OUT_EXTENSION`
///
/// `pPayload` points to memory outside the encoder core (caller or SBR/MPS payload buffers) and
/// stays a raw pointer (null for `NULL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QcOutExtension {
    /// type of the extension payload
    pub type_: ExtPayloadType,
    /// size of the payload
    pub n_payload_bits: i32,
    /// pointer to payload
    pub p_payload: *const u8,
}

impl Default for QcOutExtension {
    fn default() -> Self {
        QcOutExtension { type_: 0, n_payload_bits: 0, p_payload: core::ptr::null() }
    }
}

/// `QC_OUT_ELEMENT` (see the [module documentation](self) for `qcOutChannel` and the `dynMem_*`
/// buffers).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QcOutElement {
    /// for verification purposes
    pub static_bits_used: i32,
    /// for verification purposes
    pub dyn_bits_used: i32,
    /// bit consumption of extended fill elements
    pub ext_bits_used: i32,
    /// number of extension payloads for this element
    pub n_extensions: i32,
    /// reffering extension payload
    pub extension: [QcOutExtension; 1],
    pub granted_dyn_bits: i32,
    pub granted_pe: i32,
    pub granted_pe_corr: i32,
    pub pe_data: PeData,
    /// `qcOutChannel[ch]`: indexes into [`QcOut::p_qc_out_channels`]
    pub qc_out_channel: [usize; 2],
}

/// `QC_OUT`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QcOut {
    /// `qcElement[]` (`nElements` entries)
    pub qc_element: Vec<QcOutElement>,
    /// `pQcOutChannels[]` (`nChannels` entries)
    pub p_qc_out_channels: Vec<QcOutChannel>,
    /// global extension payload
    pub extension: [QcOutExtension; 2 + 2],
    /// number of extension payloads for this AU
    pub n_extensions: i32,
    /// maximal allowed dynamic bits in frame
    pub max_dyn_bits: i32,
    /// granted dynamic bits in frame
    pub granted_dyn_bits: i32,
    /// fill bits
    pub tot_fill_bits: i32,
    /// element associated extension payload bits, e.g. sbr, drc ...
    pub element_ext_bits: i32,
    /// frame/au associated extension payload bits (anc data ...)
    pub global_ext_bits: i32,
    /// aac side info bits
    pub static_bits: i32,
    pub total_no_red_pe: i32,
    pub total_granted_pe_corr: i32,
    /// number of dynamic bits in use
    pub used_dyn_bits: i32,
    /// AU alignment bits
    pub align_bits: i32,
    /// sum of static, dyn, sbr, fill, align and dse bits
    pub total_bits: i32,
    /// Not in C: the lines of `pQcOutChannels[ch]->mdctSpectrum` (bit `line` of entry `ch`) that
    /// the reference does not define (the lines of the bands above `sfbActive` after
    /// `FDKaacEnc_groupShortData()`; 0 in the port), until a transform overwrites them (see
    /// [`psy_main_undefined_spectrum_reads`](crate::psy_main::psy_main_undefined_spectrum_reads)).
    pub mdct_undefined: Vec<[u64; 16]>,
}

/// `ELEMENT_BITS` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ElementBits {
    /// channel bitrate in element (totalbitrate*el_relativeBits/el_channels)
    pub ch_bitrate_el: i32,
    /// used in crash recovery
    pub max_bits_el: i32,
    /// update bitreservoir level in each call of FDKaacEnc_QCMain
    pub bit_res_level_el: i32,
    /// nEffChannels*6144 - averageBitsInFrame
    pub max_bit_res_bits_el: i32,
    /// Bits relative to total Bits
    pub relative_bits_el: FixpDbl,
}

/// `QC_STATE`
///
/// `elementBits[]`, `hBitCounter` and `hAdjThr` point to separately allocated structures in C;
/// here they are owned (`elementBits` has `nElements` entries).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QcState {
    pub glob_hdr_bits: i32,
    /// maximal allowed bits per frame, 6144*nChannelsEff
    pub max_bits_per_frame: i32,
    /// minimal allowd bits per fram, superframing - DRM
    pub min_bits_per_frame: i32,
    pub n_elements: i32,
    pub bitrate_mode: QcdataBrMode,
    /// 0: full bitreservoir, 1: reduced bitreservoir, 2: disabled bitreservoir
    pub bit_res_mode: AacencBitresMode,
    pub bit_res_tot: i32,
    pub bit_res_tot_max: i32,
    /// Maximum number of allowed iterations before FDKaacEnc_crashRecovery() is applied.
    pub max_iterations: i32,
    pub inv_quant: i32,
    pub vbr_qual_factor: FixpDbl,
    pub max_bit_fac: FixpDbl,
    pub padding: Padding,
    pub element_bits: Vec<ElementBits>,
    pub h_bit_counter: BitcntrState,
    pub h_adj_thr: AdjThrState,
    /// enable dead zone quantizer
    pub d_zone_quant_enable: i32,
}
