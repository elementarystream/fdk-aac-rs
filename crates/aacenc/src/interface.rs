// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/interface.h.

//! Port of `interface.h`: the interface between the psychoacoustic model and the quantizer.
//!
//! # Memory model
//!
//! In C, `PSY_OUT_CHANNEL` holds six pointers (`mdctSpectrum`, `sfbEnergy`, `sfbSpreadEnergy`,
//! `sfbThresholdLdData`, `sfbMinSnrLdData`, `sfbEnergyLdData`) that `FDKaacEnc_EncodeFrame()`
//! points to the arrays of the same name in the `QC_OUT_CHANNEL` of the same element and channel.
//! The port keeps these arrays only in [`QcOutChannel`](crate::qc_data::QcOutChannel); functions
//! that read them through a `PSY_OUT_CHANNEL` take the matching `QcOutChannel` as well.
//!
//! The pointers can name another element's `QC_OUT_CHANNEL`: `PSY_OUT_CHANNEL`s can be shared
//! between elements, and the port follows the same array pointers as C. [`PsyOut::p_array_qc`]
//! records the pointers; the readers get the other channel's arrays as a
//! [`PsyArrayViews`](crate::adj_thr::PsyArrayViews).
//!
//! The `PSY_OUT_ELEMENT::psyOutChannel[ch]` pointers point into `PSY_OUT::pPsyOutChannels[]`;
//! the port stores the indexes. `FDKaacEnc_psyInit()` assigns the channels of an element
//! consecutively, so the channels of an element are a contiguous slice
//! `p_psy_out_channels[psy_out_channel[0]..psy_out_channel[0] + nChannelsInEl]`.

use crate::aacenc_tns::TnsInfo;
use crate::psy_const::{MAX_GROUPED_SFB, MAX_NO_OF_GROUPS};
use fdk_aac_rs_fdk::audio::{AudioObjectType, AOT_ER_AAC_ELD, AOT_ER_AAC_LD};

/// `MS_NONE`
pub const MS_NONE: i32 = 0;
/// `MS_SOME`
pub const MS_SOME: i32 = 1;
/// `MS_ALL`
pub const MS_ALL: i32 = 2;
/// `MS_ON`
pub const MS_ON: i32 = 1;

/// `struct TOOLSINFO` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolsInfo {
    /// 0 = no MS; 1 = some MS, 2 = all MS
    pub ms_digest: i32,
    pub ms_mask: [i32; MAX_GROUPED_SFB],
}

impl Default for ToolsInfo {
    fn default() -> Self {
        ToolsInfo { ms_digest: 0, ms_mask: [0; MAX_GROUPED_SFB] }
    }
}

/// `PSY_OUT_CHANNEL` without the six pointers into `QC_OUT_CHANNEL` (see the
/// [module documentation](self)). The members have the layout of the C struct up to
/// `isScale` (the pointers follow it in C).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsyOutChannel {
    pub sfb_cnt: i32,
    pub sfb_per_group: i32,
    pub max_sfb_per_group: i32,
    pub last_window_sequence: i32,
    pub window_shape: i32,
    pub grouping_mask: i32,
    pub sfb_offsets: [i32; MAX_GROUPED_SFB + 1],
    /// number of transform shifts
    pub mdct_scale: i32,
    pub group_len: [i32; MAX_NO_OF_GROUPS],
    pub tns_info: TnsInfo,
    pub noise_nrg: [i32; MAX_GROUPED_SFB],
    pub is_book: [i32; MAX_GROUPED_SFB],
    pub is_scale: [i32; MAX_GROUPED_SFB],
}

impl Default for PsyOutChannel {
    fn default() -> Self {
        PsyOutChannel {
            sfb_cnt: 0,
            sfb_per_group: 0,
            max_sfb_per_group: 0,
            last_window_sequence: 0,
            window_shape: 0,
            grouping_mask: 0,
            sfb_offsets: [0; MAX_GROUPED_SFB + 1],
            mdct_scale: 0,
            group_len: [0; MAX_NO_OF_GROUPS],
            tns_info: TnsInfo::default(),
            noise_nrg: [0; MAX_GROUPED_SFB],
            is_book: [0; MAX_GROUPED_SFB],
            is_scale: [0; MAX_GROUPED_SFB],
        }
    }
}

/// `PSY_OUT_ELEMENT`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsyOutElement {
    /// `psyOutChannel[ch]`: indexes into [`PsyOut::p_psy_out_channels`] (information specific to
    /// each channel)
    pub psy_out_channel: [usize; 2],
    /// information shared by both channels
    pub common_window: i32,
    pub tools_info: ToolsInfo,
}

/// `PSY_OUT`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PsyOut {
    /// `psyOutElement[]` (`nElements` entries)
    pub psy_out_element: Vec<PsyOutElement>,
    /// `pPsyOutChannels[]` (`nChannels` entries)
    pub p_psy_out_channels: Vec<PsyOutChannel>,
    /// The six array pointers of `pPsyOutChannels[i]` (see the [module documentation](self)):
    /// the index of the `QC_OUT_CHANNEL` in `QcOut::p_qc_out_channels` they point to, as
    /// `FDKaacEnc_EncodeFrame()` last set them. Missing entries and `usize::MAX`: not set (the
    /// readers then use the element's own `QC_OUT_CHANNEL`).
    pub p_array_qc: Vec<usize>,
}

/// `isLowDelay(aot)`
#[inline]
pub fn is_low_delay(aot: AudioObjectType) -> i32 {
    (aot == AOT_ER_AAC_LD || aot == AOT_ER_AAC_ELD) as i32
}
