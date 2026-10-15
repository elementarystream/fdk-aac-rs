// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_pns.h (types and constants).

//! The types and constants of `aacdec_pns.h`, which `channelinfo.h` contains by value. They are
//! defined here; `crate::aacdec_pns` re-exports them
//! (`pub use crate::channelinfo::aacdec_pns_types::*;`) and adds the functions.
//!
//! # Pointers of [`CPnsData`]
//!
//! The port stores only `currentSeed`, as the channel whose seed it points to. The other two
//! always point to the same memory when they are dereferenced:
//!
//! | C | Set by | Points to when used | Port |
//! |---|---|---|---|
//! | `pPnsInterChannelData` | `CPns_ResetData(&PnsData, &pComData->pnsInterChannelData)` before every element is read | `pnsInterChannelData` of the channel's `CAacDecoderCommonData` (both channels of a CPE of the configuration share it) | the PNS functions take `&mut CPnsInterChannelData` |
//! | `currentSeed` | `CPns_UpdateNoiseState()` in `CAacDecoder_Init()` (`&pAacDecoderStaticChannelInfo[ch]->pnsCurrentSeed`) and in `CChannelElement_Decode()` right before `ApplyTools()` (channel 0: its own pointer, channel 1: the pointer of channel 0) | `*PnsData.currentSeed` of the first channel of the element, for both channels of a CPE. That is not always the first channel's own seed: a channel keeps the pointer it was given as the second channel of a CPE when a later element starts with it (an SCE, or a CPE that starts one channel later) | [`CPnsData::current_seed`]; `CPns_Apply()` takes `&mut i32` |
//! | `randomSeed` | `CPns_UpdateNoiseState()` (`pComData->pnsRandomSeed`), in `CChannelElement_Decode()` right before it is used | `pnsRandomSeed` of the channel's `CAacDecoderCommonData` | `CPns_Apply()` takes `&mut [i32]` |
//!
//! The dummy channel of `CAacDecoder_DecodeFrame()` for CCE parsing never calls
//! `CPns_UpdateNoiseState()` (its seed pointers stay uninitialized in C); it only parses, so the
//! seeds are not used there.

/// `NO_OFBANDS`
pub const NO_OFBANDS: i32 = 8 * 16;

/// `CPnsInterChannelData`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CPnsInterChannelData {
    /// `correlated`
    pub correlated: [u8; NO_OFBANDS as usize],
}

impl Default for CPnsInterChannelData {
    fn default() -> Self {
        CPnsInterChannelData { correlated: [0; NO_OFBANDS as usize] }
    }
}

/// `CPnsData`. The pointers `pPnsInterChannelData` and `randomSeed` are not stored, see the
/// [module documentation](self).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CPnsData {
    /// `pnsUsed`
    pub pns_used: [u8; NO_OFBANDS as usize],
    /// `CurrentEnergy`
    pub current_energy: i32,
    /// `PnsActive`
    pub pns_active: u8,
    /// `currentSeed`: the channel whose `pnsCurrentSeed` (in its
    /// `CAacDecoderStaticChannelInfo`) it points to; `None` for `NULL`.
    pub current_seed: Option<usize>,
}

impl Default for CPnsData {
    fn default() -> Self {
        CPnsData {
            pns_used: [0; NO_OFBANDS as usize],
            current_energy: 0,
            pns_active: 0,
            current_seed: None,
        }
    }
}
