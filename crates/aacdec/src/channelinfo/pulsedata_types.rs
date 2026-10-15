// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/pulsedata.h (types and constants).

//! The types and constants of `pulsedata.h`, which `channelinfo.h` contains by value. They are
//! defined here; `crate::pulsedata` re-exports them
//! (`pub use crate::channelinfo::pulsedata_types::*;`) and adds the functions.

/// `N_MAX_LINES`
pub const N_MAX_LINES: i32 = 4;

/// `CPulseData`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CPulseData {
    /// `PulseDataPresent`
    pub pulse_data_present: u8,
    /// `NumberPulse`
    pub number_pulse: u8,
    /// `PulseStartBand`
    pub pulse_start_band: u8,
    /// `PulseOffset`
    pub pulse_offset: [u8; N_MAX_LINES as usize],
    /// `PulseAmp`
    pub pulse_amp: [u8; N_MAX_LINES as usize],
}
