// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/pulsedata.h, libAACdec/src/pulsedata.cpp.

//! Port of `pulsedata.h`: the pulse data tool.
//!
//! The types and constants are defined in [`crate::channelinfo::pulsedata_types`] and
//! re-exported here.
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `pulsedata.cpp` apart from
//! the expansion of `FDK_ASSERT` and the shared header primitives.

pub use crate::channelinfo::pulsedata_types::*;

use fdk_aac_rs_fdk::bitstream::{fdk_read_bit, fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::FixpDbl;

use crate::channelinfo::{
    get_scale_factor_bands_transmitted, is_long_block, CIcsInfo, AAC_DEC_DECODE_FRAME_ERROR,
};

/// `CPulseData_Read(bs, PulseData, sfb_startlines, pIcsInfo, frame_length)`: read pulse data
/// from the bitstream. Returns 0 on success, `AAC_DEC_DECODE_FRAME_ERROR` on error.
///
/// `sfb_startlines` is the long block scale factor band table (`ScaleFactorBands_Long`). The
/// library reads the pulse start band only below `MaxSfBands`, which `IcsReadMaxSfb()` limited
/// to the number of bands of the table; an index outside the table (inconsistent `CIcsInfo`)
/// reads 0.
pub fn cpulse_data_read(
    bs: &mut FdkBitStream,
    pulse_data: &mut CPulseData,
    sfb_startlines: &[i16],
    p_ics_info: &CIcsInfo,
    frame_length: i16,
) -> i32 {
    let max_sf_bands: u32 = get_scale_factor_bands_transmitted(p_ics_info) as u32;

    /* reset pulse data flag */
    pulse_data.pulse_data_present = 0;

    pulse_data.pulse_data_present = fdk_read_bit(bs) as u8;
    if pulse_data.pulse_data_present != 0 {
        if is_long_block(p_ics_info) == 0 {
            return AAC_DEC_DECODE_FRAME_ERROR;
        }

        pulse_data.number_pulse = fdk_read_bits(bs, 2) as u8;
        pulse_data.pulse_start_band = fdk_read_bits(bs, 6) as u8;

        if pulse_data.pulse_start_band as u32 >= max_sf_bands {
            return AAC_DEC_DECODE_FRAME_ERROR;
        }

        let mut k: i32 =
            sfb_startlines.get(pulse_data.pulse_start_band as usize).copied().unwrap_or(0) as i32;

        for i in 0..=pulse_data.number_pulse as usize {
            pulse_data.pulse_offset[i] = fdk_read_bits(bs, 5) as u8;
            pulse_data.pulse_amp[i] = fdk_read_bits(bs, 4) as u8;
            k += pulse_data.pulse_offset[i] as i32;
        }

        if k >= frame_length as i32 {
            return AAC_DEC_DECODE_FRAME_ERROR;
        }
    }

    0
}

/// `CPulseData_Apply(PulseData, pScaleFactorBandOffsets, coef)`: apply the pulse data to the
/// spectral lines (`coef`: the long block spectrum).
///
/// The line index is below the frame length, which `CPulseData_Read()` checked. With pulse data
/// that was not checked against the frame length, lines outside `coef` and band offsets outside
/// the table are skipped.
pub fn cpulse_data_apply(
    pulse_data: &CPulseData,
    p_scale_factor_band_offsets: &[i16],
    coef: &mut [FixpDbl],
) {
    if pulse_data.pulse_data_present != 0 {
        let mut k: i32 = p_scale_factor_band_offsets
            .get(pulse_data.pulse_start_band as usize)
            .copied()
            .unwrap_or(0) as i32;

        for i in 0..=pulse_data.number_pulse as usize {
            // NumberPulse is read with 2 bits; a larger value (inconsistent state) stops at the
            // end of the arrays.
            let Some(&offset) = pulse_data.pulse_offset.get(i) else { break };
            k += offset as i32;
            let amp = pulse_data.pulse_amp[i] as i32;
            if let Some(c) = coef.get_mut(k as usize) {
                // The coefficients are quantized values here (at most 8192 in magnitude), so the
                // addition cannot overflow in the library.
                if *c > 0 {
                    *c = c.wrapping_add(amp);
                } else {
                    *c = c.wrapping_sub(amp);
                }
            }
        }
    }
}
