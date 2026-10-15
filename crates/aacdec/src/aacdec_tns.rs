// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_tns.h, libAACdec/src/aacdec_tns.cpp.

//! Port of `aacdec_tns.h`: the temporal noise shaping tool.
//!
//! The types and constants are defined in [`crate::channelinfo::aacdec_tns_types`] and
//! re-exported here. `CTns_ReadDataPresentFlag()`, `CTns_ReadDataPresentUsac()`, `CTns_Read()`
//! and `CTns_Apply()` are declared in `block.h` but defined in `aacdec_tns.cpp`, so they are
//! ported here.
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `aacdec_tns.cpp` apart
//! from the expansion of `FDK_ASSERT` and the shared header primitives.

pub use crate::channelinfo::aacdec_tns_types::*;

use fdk_aac_rs_fdk::audio::{AC_RSV603DA, AC_RSVD50, AC_USAC};
use fdk_aac_rs_fdk::bitstream::{fdk_read_bit, fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::{fix_min, FixpDbl};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::lpc::clpc_synthesis_lattice;

use crate::aac_rom::{
    FixpTcc, FDK_AAC_DEC_TNS_COEFF3, FDK_AAC_DEC_TNS_COEFF4, TNS_MAX_BANDS_TBL_480,
    TNS_MAX_BANDS_TBL_512,
};
use crate::channelinfo::{
    get_maximum_tns_bands, get_scale_factor_band_offsets, get_scale_factor_bands_total,
    get_windows_per_frame, is_long_block, AacDecoderError, CIcsInfo, SamplingRateInfo, AAC_DEC_OK,
    AAC_DEC_TNS_READ_ERROR,
};
use crate::overlapadd::spec;

/// `TNS_MAXIMUM_ORDER_AAC`
pub const TNS_MAXIMUM_ORDER_AAC: i32 = 12;

/// `CTns_Reset(pTnsData)`: reset the TNS data.
pub fn ctns_reset(p_tns_data: &mut CTnsData) {
    /* Note: the following FDKmemclear should not be required. */
    p_tns_data.filter = Default::default();
    p_tns_data.number_of_filters = [0; TNS_MAX_WINDOWS as usize];
    p_tns_data.data_present = 0;
    p_tns_data.active = 0;
}

/// `CTns_ReadDataPresentFlag(bs, pTnsData)`: read the TNS data-present flag from the bitstream.
pub fn ctns_read_data_present_flag(bs: &mut FdkBitStream, p_tns_data: &mut CTnsData) {
    p_tns_data.data_present = fdk_read_bits(bs, 1) as u8;
}

/// `CTns_Read(bs, pTnsData, pIcsInfo, flags)`: read the TNS data from the bitstream.
pub fn ctns_read(
    bs: &mut FdkBitStream,
    p_tns_data: &mut CTnsData,
    p_ics_info: &CIcsInfo,
    flags: u32,
) -> AacDecoderError {
    let mut error_status = AAC_DEC_OK;

    if p_tns_data.data_present == 0 {
        return error_status;
    }

    let start_window: u8 = 0;
    let wins_per_frame: u8 = get_windows_per_frame(p_ics_info) as u8;
    let is_long_flag: u8 = is_long_block(p_ics_info);

    p_tns_data.gain_ld = 0;

    for window in start_window..wins_per_frame {
        let window = window as usize;
        let n_filt = fdk_read_bits(bs, if is_long_flag != 0 { 2 } else { 1 }) as u8;
        p_tns_data.number_of_filters[window] = n_filt;

        if n_filt != 0 {
            let coef_res = fdk_read_bits(bs, 1) as u8;

            let mut nextstopband: u8 = get_scale_factor_bands_total(p_ics_info);

            for index in 0..n_filt as usize {
                let filter = &mut p_tns_data.filter[window][index];

                let mut length = fdk_read_bits(bs, if is_long_flag != 0 { 6 } else { 4 }) as u8;

                if length > nextstopband {
                    length = nextstopband;
                }

                filter.start_band = nextstopband - length;
                filter.stop_band = nextstopband;
                nextstopband = filter.start_band;

                let order: u8;
                if flags & (AC_USAC | AC_RSVD50 | AC_RSV603DA) != 0 {
                    /* max(Order) = 15 (long), 7 (short) */
                    order = fdk_read_bits(bs, if is_long_flag != 0 { 4 } else { 3 }) as u8;
                    filter.order = order;
                } else {
                    order = fdk_read_bits(bs, if is_long_flag != 0 { 5 } else { 3 }) as u8;
                    filter.order = order;

                    if filter.order as i32 > TNS_MAXIMUM_ORDER {
                        error_status = AAC_DEC_TNS_READ_ERROR;
                        return error_status;
                    }
                }

                fdk_assert!(order as i32 <= TNS_MAXIMUM_ORDER); /* avoid illegal memory access */
                if order != 0 {
                    const SGN_MASK: [u8; 3] = [0x2, 0x4, 0x8];
                    const NEG_MASK: [i8; 3] = [!0x3, !0x7, !0xF];

                    filter.direction = if fdk_read_bits(bs, 1) != 0 { -1 } else { 1 };

                    let coef_compress = fdk_read_bits(bs, 1) as u8;

                    filter.resolution = (coef_res + 3) as i8;

                    let s_mask = SGN_MASK[(coef_res + 1 - coef_compress) as usize];
                    let n_mask = NEG_MASK[(coef_res + 1 - coef_compress) as usize];

                    for i in 0..order as usize {
                        let coef = fdk_read_bits(
                            bs,
                            (filter.resolution as i32 - coef_compress as i32) as u32,
                        ) as u8;
                        // (coef | n_mask) is computed in int and truncated to SCHAR.
                        filter.coeff[i] = if coef & s_mask != 0 {
                            ((coef as i32) | (n_mask as i32)) as i8
                        } else {
                            coef as i8
                        };
                    }
                    p_tns_data.gain_ld = 4;
                }
            }
        }
    }

    p_tns_data.active = 1;

    error_status
}

/// `CTns_ReadDataPresentUsac(hBs, pTnsData0, pTnsData1, ptns_on_lr, pIcsInfo, flags, elFlags,
/// fCommonWindow)`
pub fn ctns_read_data_present_usac(
    h_bs: &mut FdkBitStream,
    p_tns_data0: &mut CTnsData,
    p_tns_data1: &mut CTnsData,
    ptns_on_lr: &mut u8,
    p_ics_info: &CIcsInfo,
    flags: u32,
    _el_flags: u32,
    f_common_window: i32,
) {
    let mut common_tns = 0;

    if f_common_window != 0 {
        common_tns = fdk_read_bit(h_bs);
    }
    *ptns_on_lr = fdk_read_bit(h_bs) as u8;
    if common_tns != 0 {
        p_tns_data0.data_present = 1;
        ctns_read(h_bs, p_tns_data0, p_ics_info, flags);

        p_tns_data0.data_present = 0;
        p_tns_data0.active = 1;
        *p_tns_data1 = *p_tns_data0;
    } else {
        let tns_present_both = fdk_read_bit(h_bs);
        if tns_present_both != 0 {
            p_tns_data0.data_present = 1;
            p_tns_data1.data_present = 1;
        } else {
            p_tns_data1.data_present = fdk_read_bit(h_bs) as u8;
            p_tns_data0.data_present = (p_tns_data1.data_present == 0) as u8;
        }
    }
}

/// `CTns_Apply(pTnsData, pIcsInfo, pSpectralCoefficient, pSamplingRateInfo, granuleLength,
/// nbands, igf_active, flags)`: apply TNS to the spectral lines.
///
/// `p_spectral_coefficient` is the spectrum of the channel (`SPEC(.., window, granuleLength)`
/// gives the windows).
///
/// Indexes outside the tables have no defined reference result and read 0. A filter whose band
/// range lies outside the spectrum (inconsistent `CIcsInfo`) is not applied.
pub fn ctns_apply(
    p_tns_data: &CTnsData,
    p_ics_info: &CIcsInfo,
    p_spectral_coefficient: &mut [FixpDbl],
    p_sampling_rate_info: &SamplingRateInfo,
    granule_length: i32,
    nbands: u8,
    igf_active: u8,
    flags: u32,
) {
    if p_tns_data.active != 0 {
        let mut coeff: [FixpTcc; TNS_MAXIMUM_ORDER as usize] = [0; TNS_MAXIMUM_ORDER as usize];

        let start_window = 0;
        let wins_per_frame = get_windows_per_frame(p_ics_info);

        let band_offsets = get_scale_factor_band_offsets(p_ics_info, p_sampling_rate_info);
        let sri = p_sampling_rate_info.sampling_rate_index;

        for window in start_window..wins_per_frame {
            let p_spectrum_off = spec(window, granule_length);

            for index in 0..p_tns_data.number_of_filters[window as usize] as usize {
                // NumberOfFilters is at most 3 (2 bits); a larger value (inconsistent state)
                // stops at the end of the filter array.
                let Some(filter) = p_tns_data.filter[window as usize].get(index) else { break };

                if filter.order > 0 {
                    let order = (filter.order as usize).min(TNS_MAXIMUM_ORDER as usize);
                    if filter.resolution == 3 {
                        for i in 0..order {
                            coeff[i] = FDK_AAC_DEC_TNS_COEFF3
                                .get((filter.coeff[i] as i32 + 4) as usize)
                                .copied()
                                .unwrap_or(0);
                        }
                    } else {
                        for i in 0..order {
                            coeff[i] = FDK_AAC_DEC_TNS_COEFF4
                                .get((filter.coeff[i] as i32 + 8) as usize)
                                .copied()
                                .unwrap_or(0);
                        }
                    }

                    let tns_max_bands: u8 = match granule_length {
                        480 => TNS_MAX_BANDS_TBL_480.get(sri as usize).copied().unwrap_or(0),
                        512 => TNS_MAX_BANDS_TBL_512.get(sri as usize).copied().unwrap_or(0),
                        _ => {
                            let mut t = get_maximum_tns_bands(p_ics_info, sri as i32);
                            /* See redefinition of TNS_MAX_BANDS table */
                            if (flags & (AC_USAC | AC_RSVD50 | AC_RSV603DA)) != 0 && sri > 5 {
                                t = t.wrapping_add(1);
                            }
                            t
                        }
                    };
                    let start_band =
                        fix_min(fix_min(filter.start_band, tns_max_bands), nbands) as usize;
                    let start = band_offsets.get(start_band).copied().unwrap_or(0) as i32;

                    let stop_band = if igf_active != 0 {
                        fix_min(filter.stop_band, nbands)
                    } else {
                        fix_min(fix_min(filter.stop_band, tns_max_bands), nbands)
                    } as usize;
                    let stop = band_offsets.get(stop_band).copied().unwrap_or(0) as i32;

                    let size = stop - start;

                    if size != 0 {
                        let mut state: [FixpDbl; TNS_MAXIMUM_ORDER as usize] =
                            [0; TNS_MAXIMUM_ORDER as usize];

                        let begin = p_spectrum_off + start as usize;
                        if size > 0 && begin + size as usize <= p_spectral_coefficient.len() {
                            clpc_synthesis_lattice(
                                &mut p_spectral_coefficient[begin..begin + size as usize],
                                size,
                                0,
                                0,
                                filter.direction as i32,
                                &coeff,
                                order as i32,
                                &mut state,
                            );
                        }
                    }
                }
            }
        }
    }
}
