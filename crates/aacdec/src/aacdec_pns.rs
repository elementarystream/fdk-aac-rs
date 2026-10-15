// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_pns.h, libAACdec/src/aacdec_pns.cpp.

//! Port of `aacdec_pns.h`: the perceptual noise substitution tool.
//!
//! The types and constants are defined in [`crate::channelinfo::aacdec_pns_types`] and
//! re-exported here. `CPns_Read()` and `CPns_Apply()` are declared in `block.h`,
//! `CPns_IsPnsUsed()` and `CPns_SetCorrelation()` in `channelinfo.h`; all of them are defined in
//! `aacdec_pns.cpp` and ported here.
//!
//! The pointers of `CPnsData` are not dereferenced here (see
//! [`crate::channelinfo::aacdec_pns_types`]): the functions that use them take the memory they
//! point to:
//!
//! - `pPnsInterChannelData`: `&mut CPnsInterChannelData` (the channel's
//!   `pComData->pnsInterChannelData`);
//! - `currentSeed`: `&mut i32` (`pnsCurrentSeed` of the static channel info that
//!   `currentSeed` of the first channel of the element points to, [`CPnsData::current_seed`]);
//! - `randomSeed`: `&mut [i32]` (the channel's `pComData->pnsRandomSeed`, `8 * 16` values).
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `aacdec_pns.cpp` apart
//! from the expansion of `FDK_ASSERT` and the shared header primitives (`invSqrtNorm2()` is a
//! variant primitive of `fdk-aac-rs-fdk`).

pub use crate::channelinfo::aacdec_pns_types::*;

use fdk_aac_rs_fdk::bitstream::{fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::{
    f_mult, f_mult_div2, f_pow2_add_div2, fix_min, inv_sqrt_norm2, FixpDbl, DFRACT_BITS,
};
use fdk_aac_rs_fdk::fdk_assert;

use crate::aac_rom::{CodeBookDescription, MANTISSA_TABLE};
use crate::block::cblock_decode_huffman_word;
use crate::channelinfo::{
    get_scale_factor_band_offsets, get_scale_factor_bands_transmitted, get_window_group_length,
    get_window_groups, CIcsInfo, SamplingRateInfo,
};
use crate::overlapadd::spec;

/// `NOISE_OFFSET`: cf. ISO 14496-3 p. 175
pub const NOISE_OFFSET: i32 = 90;

/// `GEN_NOISE_NRG_SCALE`
const GEN_NOISE_NRG_SCALE: i32 = 7;

/// `CPns_ResetData(pPnsData, pPnsInterChannelData)`: reset the inter-channel and the PNS data.
/// `pPnsData->pPnsInterChannelData = pPnsInterChannelData` is not stored (see the
/// [module documentation](self)).
pub fn cpns_reset_data(
    p_pns_data: &mut CPnsData,
    p_pns_inter_channel_data: &mut CPnsInterChannelData,
) {
    /* Assign pointer always, since pPnsData is not persistent data */
    p_pns_data.pns_active = 0;
    p_pns_data.current_energy = 0;

    p_pns_data.pns_used = [0; NO_OFBANDS as usize];
    p_pns_inter_channel_data.correlated = [0; NO_OFBANDS as usize];
}

/// `CPns_UpdateNoiseState(pPnsData, currentSeed, randomSeed)`: sets the seed pointers for PNS
/// noise generation. `current_seed` is the channel whose `pnsCurrentSeed` `currentSeed` points to
/// ([`CPnsData::current_seed`]); `randomSeed` is not stored (see the
/// [module documentation](self)). The seeds are passed to [`cpns_apply`].
#[inline]
pub fn cpns_update_noise_state(p_pns_data: &mut CPnsData, current_seed: Option<usize>) {
    p_pns_data.current_seed = current_seed;
}

/// `CPns_IsPnsUsed(pPnsData, group, band)`: whether PNS is used in the band.
pub fn cpns_is_pns_used(p_pns_data: &CPnsData, group: i32, band: i32) -> i32 {
    let pns_band = (group * 16 + band) as u32;

    (p_pns_data.pns_used.get(pns_band as usize).copied().unwrap_or(0) & 1) as i32
}

/// `CPns_SetCorrelation(pPnsData, group, band, outofphase)`: activates the noise correlation
/// between the channel pair. Writes `pPnsData->pPnsInterChannelData`.
pub fn cpns_set_correlation(
    p_inter_channel_data: &mut CPnsInterChannelData,
    group: i32,
    band: i32,
    outofphase: i32,
) {
    let pns_band = (group * 16 + band) as u32;

    if let Some(c) = p_inter_channel_data.correlated.get_mut(pns_band as usize) {
        *c = if outofphase != 0 { 3 } else { 1 };
    }
}

/// `CPns_IsCorrelated(pPnsData, group, band)` (static in C): reads
/// `pPnsData->pPnsInterChannelData`.
pub fn cpns_is_correlated(
    p_inter_channel_data: &CPnsInterChannelData,
    group: i32,
    band: i32,
) -> i32 {
    let pns_band = (group * 16 + band) as u32;

    if p_inter_channel_data.correlated.get(pns_band as usize).copied().unwrap_or(0) & 0x01 != 0 {
        1
    } else {
        0
    }
}

/// `CPns_IsOutOfPhase(pPnsData, group, band)` (static in C): reads
/// `pPnsData->pPnsInterChannelData`.
pub fn cpns_is_out_of_phase(
    p_inter_channel_data: &CPnsInterChannelData,
    group: i32,
    band: i32,
) -> i32 {
    let pns_band = (group * 16 + band) as u32;

    if p_inter_channel_data.correlated.get(pns_band as usize).copied().unwrap_or(0) & 0x02 != 0 {
        1
    } else {
        0
    }
}

/// `CPns_Read(pPnsData, bs, hcb, pScaleFactor, global_gain, band, group)`: read the PNS
/// information from the bitstream.
pub fn cpns_read(
    p_pns_data: &mut CPnsData,
    bs: &mut FdkBitStream,
    hcb: &CodeBookDescription,
    p_scale_factor: &mut [i16],
    global_gain: u8,
    band: i32,
    group: i32,
) {
    let delta: i32;
    let pns_band = (group * 16 + band) as u32;

    if p_pns_data.pns_active != 0 {
        /* Next PNS band case */
        delta = cblock_decode_huffman_word(bs, hcb) - 60;
    } else {
        /* First PNS band case */
        let noise_start_value = fdk_read_bits(bs, 9) as i32;

        delta = noise_start_value - 256;
        p_pns_data.pns_active = 1;
        p_pns_data.current_energy = global_gain as i32 - NOISE_OFFSET;
    }

    p_pns_data.current_energy = p_pns_data.current_energy.wrapping_add(delta);
    if let Some(s) = p_scale_factor.get_mut(pns_band as usize) {
        *s = p_pns_data.current_energy as i16;
    }

    if let Some(u) = p_pns_data.pns_used.get_mut(pns_band as usize) {
        *u = 1;
    }
}

/// `GenerateRandomVector(spec, size, pRandomState)` (static in C): generate a vector of noise of
/// the given length. The noise values are scaled in order to yield a noise energy of 1.0.
/// Returns the exponent of the generated noise vector.
pub fn generate_random_vector(spec: &mut [FixpDbl], size: i32, p_random_state: &mut i32) -> i32 {
    let mut inv_nrg_e: i32 = 0;
    let mut nrg_m: FixpDbl = 0;
    let mut random_state = *p_random_state;

    /* Generate noise and calculate energy. */
    for i in 0..size.max(0) as usize {
        // (((INT64)1664525 * randomState) + (INT64)1013904223) & 0xFFFFFFFF, assigned to int.
        random_state = (random_state as u32).wrapping_mul(1664525).wrapping_add(1013904223) as i32;
        nrg_m = f_pow2_add_div2(nrg_m, random_state >> GEN_NOISE_NRG_SCALE);
        spec[i] = random_state;
    }
    let nrg_e = GEN_NOISE_NRG_SCALE * 2 + 1;

    /* weight noise with = 1 / sqrt_nrg; */
    let inv_nrg_m = inv_sqrt_norm2(nrg_m.wrapping_shl(1), &mut inv_nrg_e);
    inv_nrg_e += -((nrg_e - 1) >> 1);

    for i in (0..size.max(0) as usize).rev() {
        spec[i] = f_mult(spec[i], inv_nrg_m);
    }

    /* Store random state */
    *p_random_state = random_state;

    inv_nrg_e
}

/// `ScaleBand(spec, size, scaleFactor, specScale, noise_e, out_of_phase)` (static in C)
pub fn scale_band(
    spec: &mut [FixpDbl],
    size: i32,
    scale_factor: i32,
    spec_scale: i32,
    noise_e: i32,
    out_of_phase: i32,
) {
    /* Get gain from scale factor value = 2^(scaleFactor * 0.25) */
    let mut sf_matissa = MANTISSA_TABLE[(scale_factor & 0x03) as usize][0];
    /* sfExponent = (scaleFactor >> 2) + ExponentTable[scaleFactor & 0x03][0]; */
    /* Note:  ExponentTable[scaleFactor & 0x03][0] is always 1. */
    let sf_exponent = (scale_factor >> 2) + 1;

    if out_of_phase != 0 {
        sf_matissa = -sf_matissa;
    }

    /* +1 because of fMultDiv2 below. */
    let mut shift = sf_exponent - spec_scale + 1 + noise_e;

    /* Apply gain to noise values */
    if shift >= 0 {
        shift = fix_min(shift, DFRACT_BITS - 1);
        for i in (0..size.max(0) as usize).rev() {
            // fMultDiv2(spec[i], sfMatissa) << shift: a left shift that may overflow (the
            // reference builds use lsl/shl).
            spec[i] = f_mult_div2(spec[i], sf_matissa).wrapping_shl(shift as u32);
        }
    } else {
        shift = fix_min(-shift, DFRACT_BITS - 1);
        for i in (0..size.max(0) as usize).rev() {
            spec[i] = f_mult_div2(spec[i], sf_matissa) >> shift;
        }
    }
}

/// `CPns_Apply(pPnsData, pIcsInfo, pSpectrum, pSpecScale, pScaleFactor, pSamplingRateInfo,
/// granuleLength, channel)`: apply PNS (generate noise) on the bands flagged as noisy.
///
/// The memory reached through the pointers of `CPnsData` is passed explicitly (see the
/// [module documentation](self)): `p_inter_channel_data` (`pPnsInterChannelData`),
/// `current_seed` (`*currentSeed`) and `random_seed` (`randomSeed`, `8 * 16` values).
///
/// Bands whose lines lie outside `p_spectrum` (inconsistent `CIcsInfo`) are skipped.
pub fn cpns_apply(
    p_pns_data: &CPnsData,
    p_ics_info: &CIcsInfo,
    p_spectrum: &mut [FixpDbl],
    p_spec_scale: &[i16],
    p_scale_factor: &[i16],
    p_sampling_rate_info: &SamplingRateInfo,
    granule_length: i32,
    channel: i32,
    p_inter_channel_data: &CPnsInterChannelData,
    current_seed: &mut i32,
    random_seed: &mut [i32],
) {
    if p_pns_data.pns_active != 0 {
        let band_offsets = get_scale_factor_band_offsets(p_ics_info, p_sampling_rate_info);

        let scale_factor_bands_transmitted = get_scale_factor_bands_transmitted(p_ics_info) as i32;

        let mut window: i32 = 0;
        for group in 0..get_window_groups(p_ics_info) as i32 {
            for _groupwin in 0..get_window_group_length(p_ics_info, group) {
                let spectrum_off = spec(window, granule_length);

                for band in 0..scale_factor_bands_transmitted {
                    if cpns_is_pns_used(p_pns_data, group, band) != 0 {
                        let pns_band = (window * 16 + band) as u32 as usize;

                        let off0 = band_offsets.get(band as usize).copied().unwrap_or(0) as i32;
                        let off1 = band_offsets.get(band as usize + 1).copied().unwrap_or(0) as i32;
                        let band_width = off1 - off0;

                        fdk_assert!(band_width >= 0);

                        let begin = spectrum_off + off0.max(0) as usize;
                        let width = band_width.max(0) as usize;
                        if begin + width > p_spectrum.len() || pns_band >= random_seed.len() {
                            continue;
                        }
                        let s = &mut p_spectrum[begin..begin + width];

                        let noise_e = if channel > 0
                            && cpns_is_correlated(p_inter_channel_data, group, band) != 0
                        {
                            generate_random_vector(s, band_width, &mut random_seed[pns_band])
                        } else {
                            random_seed[pns_band] = *current_seed;

                            generate_random_vector(s, band_width, current_seed)
                        };

                        let out_of_phase = cpns_is_out_of_phase(p_inter_channel_data, group, band);

                        scale_band(
                            s,
                            band_width,
                            p_scale_factor.get((group * 16 + band) as usize).copied().unwrap_or(0)
                                as i32,
                            p_spec_scale.get(window as usize).copied().unwrap_or(0) as i32,
                            noise_e,
                            out_of_phase,
                        );
                    }
                }
                window += 1;
            }
        }
    }
}
