// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_tns.h (types and constants).

//! The types and constants of `aacdec_tns.h`, which `channelinfo.h` contains by value. They are
//! defined here; `crate::aacdec_tns` re-exports them
//! (`pub use crate::channelinfo::aacdec_tns_types::*;`) and adds the functions.

/// `TNS_MAX_WINDOWS`
pub const TNS_MAX_WINDOWS: i32 = 8;
/// `TNS_MAXIMUM_FILTERS`
pub const TNS_MAXIMUM_FILTERS: i32 = 3;

/// `TNS_MAXIMUM_ORDER` (for memory allocation): 12 for AAC-LC and AAC-SSR. Set to 20 for
/// AAC-Main (AOT 1). Some broken encoders also do order 20 for AAC-LC :( 15 for USAC (AOT 42)
pub const TNS_MAXIMUM_ORDER: i32 = 20;

/// `CFilter`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CFilter {
    /// `Coeff`
    pub coeff: [i8; TNS_MAXIMUM_ORDER as usize],

    /// `StartBand`
    pub start_band: u8,
    /// `StopBand`
    pub stop_band: u8,

    /// `Direction`
    pub direction: i8,
    /// `Resolution`
    pub resolution: i8,

    /// `Order`
    pub order: u8,
}

/// `CTnsData`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CTnsData {
    /// `Filter`
    pub filter: [[CFilter; TNS_MAXIMUM_FILTERS as usize]; TNS_MAX_WINDOWS as usize],
    /// `NumberOfFilters`
    pub number_of_filters: [u8; TNS_MAX_WINDOWS as usize],
    /// `DataPresent`
    pub data_present: u8,
    /// `Active`
    pub active: u8,

    /// `GainLd`: log2 of the maximum total filter gains. The value is required to keep
    /// necessary mantissa headroom so that while applying the TNS predictor the mantissas do
    /// not overflow.
    pub gain_ld: u8,
}
