// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/block.h, libAACdec/src/block.cpp, libAACdec/src/arm/block_arm.cpp.

//! Port of `block.h`: long/short block decoding (scale factors, section and spectral data,
//! inverse quantization, noise filling, the tool chain of `ApplyTools()` and the frequency to
//! time transforms).
//!
//! The PNS and TNS functions declared in `block.h` are defined in `aacdec_pns.cpp` and
//! `aacdec_tns.cpp` and ported in [`crate::aacdec_pns`] and [`crate::aacdec_tns`].
//!
//! # Memory
//!
//! The functions follow the rules of [`crate::channelinfo`]: the spectrum
//! (`pSpectralCoefficient`) and the other memory that C reaches through the pointers of
//! `CAacDecoderChannelInfo` are appended to the C parameters. Where a C parameter always points
//! to a field of a structure that the function receives anyway (the library has exactly one
//! caller), the parameter is dropped and the field is used; this is noted at the function.
//!
//! # Architecture variants
//!
//! `block.cpp` includes `arm/block_arm.cpp` on ARM, which replaces the scaling loop of
//! `CBlock_ScaleSpectralData()` by [`cblock_scale_spectral_data_func1`] (a `do … while ((runs
//! -= 4) != 0)` loop over each band). The `.ii` files of the `aarch64` and `x86` builds differ
//! only in this function, in the expansion of `FDK_ASSERT` and in the shared header primitives.
//! Both versions give the same result for every band width that is a positive multiple of 4;
//! every scale factor band table of `aac_rom` has only such widths within its number of bands
//! (PORTING.md, "Function-level variants", and [`cblock_scale_spectral_data_func1`]).

use fdk_aac_rs_arith::ac_arith_coder::{
    carco_decode_arith_data, ARITH_CODER_ERROR, ARITH_CODER_OK,
};
use fdk_aac_rs_fdk::audio::{
    AC_ELD, AC_EL_ENHANCED_NOISE, AC_EL_FULLBANDLPD, AC_EL_LPDSTEREOIDX, AC_ER_HCR, AC_ER_VCB11,
    AC_INDEP, AC_MPEGD_RES, AC_RSV603DA, AC_RSVD50, AC_SCALABLE, AC_USAC,
};
use fdk_aac_rs_fdk::bitbuffer::fdk_get32;
use fdk_aac_rs_fdk::bitstream::{
    fdk_peek_bits, fdk_push_back_cache, fdk_push_for, fdk_read2_bits, fdk_read_bit, fdk_read_bits,
    fdk_skip_cached_bits, FdkBitStream, CACHE_BITS,
};
use fdk_aac_rs_fdk::common_fix::{
    cnt_leading_zeros, f_div_norm_signed, f_mult, f_mult_div2, f_normz, f_pow2_div2, fixp_abs,
    fl2fxconst_sgl, fx_dbl2fx_sgl, get_scalefactor, scale_value, scale_value_in_place,
    scale_values_saturate_to, FixpDbl, FixpSgl, DFRACT_BITS,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::lpc::FixpLpc;
use fdk_aac_rs_fdk::mdct::{
    imlt_block, BLOCK_LONG, BLOCK_SHORT, BLOCK_START, BLOCK_STOP, MDCT_OUT_HEADROOM,
    MLT_FLAG_CURR_ALIAS_SYMMETRY,
};
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;
use fdk_aac_rs_fdk::trig_fcts::fixp_cos;
use fdk_aac_rs_fdk::ARCH;

use crate::aac_rom::{
    CodeBookDescription, HuffmanBits, HuffmanEntries, PcmDec, AAC_CODE_BOOK_DESCRIPTION_TABLE,
    EXPONENT_TABLE, INVERSE_QUANT_TABLE, MANTISSA_TABLE,
};
use crate::aacdec_pns::{cpns_apply, cpns_read};
use crate::aacdec_tns::ctns_apply;
use crate::channelinfo::*;
use crate::ldfiltbank::inv_mdct_transform_low_delay_fdk;
use crate::overlapadd::spec as spec_off;
use crate::pulsedata::cpulse_data_apply;
use crate::usacdec_const::{
    BPF_SFD, LFAC, L_SUBFR, M_LP_FILTER_ORDER, NB_SUBFR_SUPERFR, PIT_MAX_MAX, SYN_SFD,
};
use crate::usacdec_rom::LSF_SCALE;

use crate::aacdec_hcr::{hcr_decoder, hcr_init, hcr_mute_erroneous_lines};
use crate::usacdec_fac::clpd_fac_acelp2mdct;
use crate::usacdec_lpc::{e_lpc_f_lsp_a_conversion, LSPARG_SCALE};
use crate::usacdec_lpd::bass_pf_1sf_delay;

/// `GetWindowGroupLength(pIcsInfo, group)` that reads 0 for a group index outside the array.
#[inline]
fn group_length(ics: &CIcsInfo, group: i32) -> i32 {
    ics.window_group_length.get(group as usize).copied().unwrap_or(0) as i32
}

/// `BandOffsets[i]`, 0 for an index outside the table.
#[inline]
fn band_offset(band_offsets: &[i16], i: i32) -> i32 {
    band_offsets.get(i as usize).copied().unwrap_or(0) as i32
}

/// `MantissaTable[lsb][exponent]`, flattened like the C array (`exponent >= 14` reads the next
/// row, as in C); 0 outside the whole table.
#[inline]
fn mantissa_table(lsb: u32, exponent: u32) -> FixpDbl {
    MANTISSA_TABLE
        .as_flattened()
        .get((lsb * 14).wrapping_add(exponent) as usize)
        .copied()
        .unwrap_or(0)
}

/// `ExponentTable[lsb][exponent]`, flattened like the C array; 0 outside the whole table.
#[inline]
fn exponent_table(lsb: u32, exponent: u32) -> i32 {
    EXPONENT_TABLE
        .as_flattened()
        .get((lsb * 14).wrapping_add(exponent) as usize)
        .copied()
        .unwrap_or(0) as i32
}

// ---------------------------------------------------------------------------------------------
// block.h (inline functions)
// ---------------------------------------------------------------------------------------------

/// `EvaluatePower43(pValue, lsb)`: calculate `2^(lsb/4) * value^(4/3)`.
///
/// `p_value` is the quantized value; the inverse quantized result is stored back. `lsb` are the
/// 2 LSBs of the scale factor (`scaleFactor % 4`). Returns the exponent of the result (mantissa)
/// stored into `p_value`.
///
/// The value must be positive and below 8192 (`FDK_ASSERT(exponent < 14)`); the callers check
/// it against `MAX_QUANTIZED_VALUE` first. For other values the table reads are taken from the
/// flattened tables as in C, and read 0 outside them (no defined reference result).
#[inline]
pub fn evaluate_power43(p_value: &mut FixpDbl, lsb: u32) -> i32 {
    let value = *p_value;
    let free_bits = f_normz(value) as u32;
    let exponent: u32 = (DFRACT_BITS as u32).wrapping_sub(free_bits);
    fdk_assert!(exponent < 14);

    // (((int)value << freeBits) >> 19): freeBits is 32 for value == 0 (never passed; the shift
    // amount is taken modulo 32 like lsl/shl).
    let mut x: u32 = (value.wrapping_shl(free_bits) >> 19) as u32;
    let table_index = ((x & 0x0FFF) >> 4) as usize;

    x &= 0x0F;

    let r0: u32 = INVERSE_QUANT_TABLE[table_index] as u32;
    let r1: u32 = INVERSE_QUANT_TABLE[table_index + 1] as u32;
    let nx: u16 = (16 - x) as u16;
    let temp: u32 = r0.wrapping_mul(nx as u32).wrapping_add(r1.wrapping_mul(x));
    let inv_q_val: FixpDbl = temp as FixpDbl;

    fdk_assert!(lsb < 4);
    *p_value = f_mult_div2(inv_q_val, mantissa_table(lsb, exponent));

    /* + 1 compensates fMultDiv2(). */
    exponent_table(lsb, exponent) + 1
}

/// `GetScaleFromValue(value, lsb)`: the required shift scale for the given quantized value and
/// scale (factor % 4) value.
#[inline]
pub fn get_scale_from_value(mut value: FixpDbl, lsb: u32) -> i32 {
    if value != 0 {
        let scale = evaluate_power43(&mut value, lsb);
        cnt_leading_zeros(value) - scale - 2
    } else {
        0 /* Return zero, because its useless to scale a zero value, saves workload and avoids
          scaling overshifts. */
    }
}

/// `CBlock_DecodeHuffmanWord(bs, hcb)`: read a huffman codeword from the bitstream and return
/// the index value.
#[inline]
pub fn cblock_decode_huffman_word(bs: &mut FdkBitStream, hcb: &CodeBookDescription) -> i32 {
    let code_book = hcb.code_book;
    let mut index: u32 = 0;
    let mut val: u32;

    loop {
        // An index outside the table (impossible with the tables of the library) ends the word.
        let Some(row) = code_book.get(index as usize) else { return 0 };
        val = row[fdk_read_bits(bs, HuffmanBits as u32) as usize] as u32; /* Expensive memory access */
        if (val & 1) == 0 {
            index = val >> 2;
            continue;
        } else {
            if val & 2 != 0 {
                fdk_push_back_cache(bs, 1);
            }
            val >>= 2;
            break;
        }
    }

    val as i32
}

/// `CBlock_DecodeHuffmanWordCB(bs, CodeBook)`
#[inline]
pub fn cblock_decode_huffman_word_cb(
    bs: &mut FdkBitStream,
    code_book: &[[u16; HuffmanEntries as usize]],
) -> i32 {
    let mut index: u32 = 0;

    loop {
        // An index outside the table (impossible with the tables of the library) ends the word.
        let Some(row) = code_book.get(index as usize) else { return 0 };
        index = row[fdk_read2_bits(bs) as usize] as u32; /* Expensive memory access */
        if index & 1 != 0 {
            break;
        }
        index >>= 2;
    }
    if index & 2 != 0 {
        fdk_push_back_cache(bs, 1);
    }
    (index >> 2) as i32
}

// ---------------------------------------------------------------------------------------------
// block.cpp
// ---------------------------------------------------------------------------------------------

/// `CBlock_GetEscape(bs, q)`: read the escape sequence of a codeword if the absolute value of
/// the quantized coefficient is 16.
///
/// A limitation is implemented to maximal 21 bits according to ISO/IEC 14496-3:2009(E)
/// 4.6.3.3. This limits the escape prefix to a maximum of eight 1's. If more than eight 1's are
/// read, `MAX_QUANTIZED_VALUE + 1` is returned, independent of the sign of `q`.
pub fn cblock_get_escape(bs: &mut FdkBitStream, q: i32) -> i32 {
    if fixp_abs(q) != 16 {
        return q;
    }

    let mut i: i32 = 4;
    while i < 13 {
        if fdk_read_bit(bs) == 0 {
            break;
        }
        i += 1;
    }

    if i == 13 {
        return MAX_QUANTIZED_VALUE + 1;
    }

    let off = fdk_read_bits(bs, i as u32) as i32;
    i = off + (1 << i);

    if q < 0 {
        i = -i;
    }

    i
}

/// [`cblock_get_escape`] with the escape prefix counted on peeked bits: the prefix of `k` ones
/// (at most 8) and its terminating zero and the following `4 + k` bits of the escape word are
/// read at once, which leaves the bitstream in the same state as the straight reads (see
/// `apply_sign_bits`); nine ones are skipped and give `MAX_QUANTIZED_VALUE + 1` as there.
/// Requires a cache of at most 32 bits.
#[inline(always)]
pub fn cblock_get_escape_fast(bs: &mut FdkBitStream, q: i32) -> i32 {
    if fixp_abs(q) != 16 {
        return q;
    }
    let prefix = fdk_peek_bits(bs, 9);
    let ones = (!(prefix << 23)).leading_zeros(); /* leading ones, at most 9 */
    if ones >= 9 {
        fdk_skip_cached_bits(bs, 9);
        return MAX_QUANTIZED_VALUE + 1;
    }
    let i = 4 + ones;
    let off = (fdk_read_bits(bs, ones + 1 + i) & ((1u32 << i) - 1)) as i32;
    let r = off + (1 << i);
    if q < 0 {
        -r
    } else {
        r
    }
}

/// `CBlock_ReadScaleFactorData(pAacDecoderChannelInfo, bs, flags)`: read the scale factor data
/// (ISO/IEC 14496-3, 4.6.2.3.2). The SF_OFFSET = 100 is already subtracted from the scale
/// factor values. Also reads the PNS data.
pub fn cblock_read_scale_factor_data(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    bs: &mut FdkBitStream,
    flags: u32,
) -> AacDecoderError {
    read_scale_factor_data::<false>(p_aac_decoder_channel_info, bs, flags)
}

/// [`cblock_read_scale_factor_data`] with the Huffman table decoding
/// ([`cblock_decode_huffman_word_lut`]) of the scale factors and intensity positions while the
/// cache holds at most 32 bits (which the library never exceeds).
pub fn cblock_read_scale_factor_data_fast(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    bs: &mut FdkBitStream,
    flags: u32,
) -> AacDecoderError {
    read_scale_factor_data::<true>(p_aac_decoder_channel_info, bs, flags)
}

#[inline(always)]
fn read_scale_factor_data<const LUT: bool>(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    bs: &mut FdkBitStream,
    flags: u32,
) -> AacDecoderError {
    let lut = &HUFF_LUT[BOOKSCL as usize];
    let decode = |bs: &mut FdkBitStream, code_book| {
        if LUT && bs.bits_in_cache <= CACHE_BITS {
            cblock_decode_huffman_word_lut(bs, code_book, lut)
        } else {
            cblock_decode_huffman_word_cb(bs, code_book)
        }
    };
    let p = p_aac_decoder_channel_info;
    let mut position: i32 = 0; /* accu for intensity delta coding */
    let mut factor: i32 = p.p_dyn_data.raw_data_info.global_gain as i32; /* accu for scale factor delta coding */
    let hcb = &AAC_CODE_BOOK_DESCRIPTION_TABLE[BOOKSCL as usize];
    let code_book = hcb.code_book;

    let scale_factor_bands_transmitted = get_scale_factor_bands_transmitted(&p.ics_info) as i32;
    for group in 0..get_window_groups(&p.ics_info) as i32 {
        for band in 0..scale_factor_bands_transmitted {
            let idx = (group * 16 + band) as usize;
            // Indexes outside the arrays (inconsistent CIcsInfo) are skipped.
            let Some(&cb) = p.p_dyn_data.a_code_book.get(idx) else { continue };
            match cb {
                ZERO_HCB => {
                    /* zero book */
                    p.p_dyn_data.a_scale_factor[idx] = 0;
                }
                INTENSITY_HCB | INTENSITY_HCB2 => {
                    /* intensity steering */
                    let temp = decode(bs, code_book);
                    position += temp - 60;
                    p.p_dyn_data.a_scale_factor[idx] = (position - 100) as i16;
                }
                NOISE_HCB => {
                    /* PNS */
                    if flags & (AC_MPEGD_RES | AC_USAC | AC_RSVD50 | AC_RSV603DA) != 0 {
                        return AAC_DEC_PARSE_ERROR;
                    }
                    let global_gain = p.p_dyn_data.raw_data_info.global_gain;
                    cpns_read(
                        &mut p.data.aac.pns_data,
                        bs,
                        hcb,
                        &mut p.p_dyn_data.a_scale_factor,
                        global_gain,
                        band,
                        group,
                    );
                }
                _ => {
                    /* decode scale factor */
                    if !((flags & (AC_USAC | AC_RSVD50 | AC_RSV603DA)) != 0
                        && band == 0
                        && group == 0)
                    {
                        let temp = decode(bs, code_book);
                        factor += temp - 60; /* MIDFAC 1.5 dB */
                    }
                    p.p_dyn_data.a_scale_factor[idx] = (factor - 100) as i16;
                }
            }
        }
    }

    AAC_DEC_OK
}

/// `CBlock_ScaleSpectralData_func1(pSpectrum, maxSfbs, BandOffsets, SpecScale_window,
/// pSfbScale, window)` of `arm/block_arm.cpp` (the `aarch64` variant): the scaling loop of
/// [`cblock_scale_spectral_data`], unrolled by 4 in a `do … while ((runs -= 4) != 0)` loop.
///
/// `p_spectrum` is the spectrum of the window; unlike the generic loop, the ARM loop starts at
/// line 0 and walks through the bands without reading `BandOffsets[0]` (which is 0 in every
/// table).
///
/// # Band widths (PORTING.md, "Function-level variants")
///
/// The C loop runs `runs / 4` times for a band width `runs` that is a positive multiple of 4
/// (`FDK_ASSERT((runs & 3) == 0)`), as every scale factor band table has within its number of
/// bands. For other widths the reference has no defined result; the port scales `max(runs, 0)`
/// lines rounded up to a multiple of 4 (at least 4, as the first iteration of the `do` loop
/// always runs), stopping at the end of `p_spectrum` (PORTING.md, "Deviations from the
/// reference").
pub fn cblock_scale_spectral_data_func1(
    p_spectrum: &mut [FixpDbl],
    max_sfbs: i32,
    band_offsets: &[i16],
    spec_scale_window: i32,
    p_sfb_scale: &[i16],
    window: i32,
) {
    let mut pos: usize = 0;
    let mut band_offset_v: i32 = 0;
    for band in 0..max_sfbs {
        let mut runs = band_offset_v;
        band_offset_v = band_offset(band_offsets, band + 1);
        runs = band_offset_v - runs; /* is always a multiple of 4 */
        fdk_assert!((runs & 3) == 0);
        let sfb_scale = p_sfb_scale.get((window * 16 + band) as usize).copied().unwrap_or(0) as i32;
        let scale = (DFRACT_BITS - 1).min(spec_scale_window - sfb_scale);

        if scale != 0 {
            // do { 4 lines } while ((runs = runs - 4) != 0);
            let blocks = if runs > 0 { (runs as usize).div_ceil(4) } else { 1 };
            for _ in 0..blocks {
                for _ in 0..4 {
                    if let Some(v) = p_spectrum.get_mut(pos) {
                        // tmp >>= scale; scale is at most 31 and not negative for every state
                        // of the library (asr with a register amount).
                        *v = v.wrapping_shr(scale as u32);
                    }
                    pos += 1;
                }
            }
        } else {
            pos = (pos as isize + runs as isize).max(0) as usize;
        }
    }
}

/// [`cblock_scale_spectral_data_func1`] with the lines of a band scaled on a slice (vectorized)
/// where all of them lie inside `p_spectrum`; elsewhere the straight per-line code runs.
pub fn cblock_scale_spectral_data_func1_fast(
    p_spectrum: &mut [FixpDbl],
    max_sfbs: i32,
    band_offsets: &[i16],
    spec_scale_window: i32,
    p_sfb_scale: &[i16],
    window: i32,
) {
    let mut pos: usize = 0;
    let mut band_offset_v: i32 = 0;
    for band in 0..max_sfbs {
        let mut runs = band_offset_v;
        band_offset_v = band_offset(band_offsets, band + 1);
        runs = band_offset_v - runs; /* is always a multiple of 4 */
        fdk_assert!((runs & 3) == 0);
        let sfb_scale = p_sfb_scale.get((window * 16 + band) as usize).copied().unwrap_or(0) as i32;
        let scale = (DFRACT_BITS - 1).min(spec_scale_window - sfb_scale);

        if scale != 0 {
            let blocks = if runs > 0 { (runs as usize).div_ceil(4) } else { 1 };
            let n = blocks * 4;
            if pos + n <= p_spectrum.len() {
                for v in p_spectrum[pos..pos + n].iter_mut() {
                    *v = v.wrapping_shr(scale as u32);
                }
            } else {
                for i in pos..pos + n {
                    if let Some(v) = p_spectrum.get_mut(i) {
                        *v = v.wrapping_shr(scale as u32);
                    }
                }
            }
            pos += n;
        } else {
            pos = (pos as isize + runs as isize).max(0) as usize;
        }
    }
}

/// `CBlock_ScaleSpectralData(pAacDecoderChannelInfo, maxSfbs, pSamplingRateInfo)`: find a
/// common exponent (shift factor) for all sfb in each spectral window, and store them into
/// `CAacDecoderChannelInfo::specScale`.
///
/// `spec` is the spectrum of the channel (`pSpectralCoefficient`). Lines outside `spec` and
/// band offsets outside the table (inconsistent `CIcsInfo`) are skipped.
pub fn cblock_scale_spectral_data(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    max_sfbs: u8,
    p_sampling_rate_info: &SamplingRateInfo,
    spec: &mut [FixpDbl],
) {
    scale_spectral_data::<false>(p_aac_decoder_channel_info, max_sfbs, p_sampling_rate_info, spec)
}

/// [`cblock_scale_spectral_data`] with the scaling of each band on a slice (vectorized) where
/// the band lies inside the spectrum ([`cblock_scale_spectral_data_func1_fast`] and the generic
/// loop); elsewhere the straight per-line code runs.
pub fn cblock_scale_spectral_data_fast(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    max_sfbs: u8,
    p_sampling_rate_info: &SamplingRateInfo,
    spec: &mut [FixpDbl],
) {
    scale_spectral_data::<true>(p_aac_decoder_channel_info, max_sfbs, p_sampling_rate_info, spec)
}

#[inline(always)]
fn scale_spectral_data<const FAST: bool>(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    max_sfbs: u8,
    p_sampling_rate_info: &SamplingRateInfo,
    spec: &mut [FixpDbl],
) {
    let p = p_aac_decoder_channel_info;
    let band_offsets = get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info);
    let max_sfbs = max_sfbs as i32;

    p.spec_scale = [0; 8];

    let mut window: i32 = 0;
    for group in 0..get_window_groups(&p.ics_info) as i32 {
        for _groupwin in 0..group_length(&p.ics_info, group) {
            if window >= 8 {
                // An inconsistent CIcsInfo with more than 8 windows.
                return;
            }
            let p_sfb_scale = &p.p_dyn_data.a_sfb_scale;
            let sfb_scale =
                |band: i32| p_sfb_scale.get((window * 16 + band) as usize).copied().unwrap_or(0);
            let mut spec_scale_window = p.spec_scale[window as usize] as i32;
            let spectrum_off = spec_off(window, p.granule_length);
            let spec_len = spec.len();

            /* find scaling for current window */
            for band in 0..max_sfbs {
                spec_scale_window = spec_scale_window.max(sfb_scale(band) as i32);
            }

            let tns = &p.p_dyn_data.tns_data;
            if tns.active != 0 && tns.number_of_filters[window as usize] > 0 {
                /* Find max scale of TNS bands */
                let mut spec_scale_window_tns: i32 = 0;
                let mut tns_start: i32 = get_maximum_tns_bands(
                    &p.ics_info,
                    p_sampling_rate_info.sampling_rate_index as i32,
                ) as i32;
                let mut tns_stop: i32 = 0;
                for filter_index in 0..tns.number_of_filters[window as usize] as usize {
                    let Some(filter) = tns.filter[window as usize].get(filter_index) else {
                        break;
                    };
                    for band in filter.start_band as i32..filter.stop_band as i32 {
                        spec_scale_window_tns = spec_scale_window_tns.max(sfb_scale(band) as i32);
                    }
                    /* Find TNS line boundaries for all TNS filters */
                    tns_start = tns_start.min(filter.start_band as i32);
                    tns_stop = tns_stop.max(filter.stop_band as i32);
                }
                spec_scale_window_tns += tns.gain_ld as i32;
                fdk_assert!(tns_stop >= tns_start);
                /* Consider existing headroom of all MDCT lines inside the TNS bands. */
                let start = band_offset(band_offsets, tns_start);
                let len = band_offset(band_offsets, tns_stop) - start;
                let begin = spectrum_off + start.max(0) as usize;
                // getScalefactor() loops `len` times (`for (i = len; i != 0; i--)`); len is
                // never negative in the library (tns_stop >= tns_start).
                let lines = if len > 0 && begin + len as usize <= spec.len() {
                    &spec[begin..begin + len as usize]
                } else {
                    &[][..]
                };
                spec_scale_window_tns -= get_scalefactor(lines);
                if spec_scale_window <= 17 {
                    spec_scale_window_tns += 1;
                }
                /* Add enough mantissa head room such that the spectrum is still representable
                after applying TNS. */
                spec_scale_window = spec_scale_window.max(spec_scale_window_tns);
            }

            /* store scaling of current window */
            p.spec_scale[window as usize] = spec_scale_window as i16;

            let p_spectrum = &mut spec[spectrum_off.min(spec_len)..];
            if ARCH == fdk_aac_rs_fdk::Arch::Aarch64 {
                /* FUNCTION_CBlock_ScaleSpectralData_func1 (arm/block_arm.cpp) */
                if FAST {
                    cblock_scale_spectral_data_func1_fast(
                        p_spectrum,
                        max_sfbs,
                        band_offsets,
                        spec_scale_window,
                        &p.p_dyn_data.a_sfb_scale,
                        window,
                    );
                } else {
                    cblock_scale_spectral_data_func1(
                        p_spectrum,
                        max_sfbs,
                        band_offsets,
                        spec_scale_window,
                        &p.p_dyn_data.a_sfb_scale,
                        window,
                    );
                }
            } else {
                for band in 0..max_sfbs {
                    let scale = (DFRACT_BITS - 1).min(spec_scale_window - sfb_scale(band) as i32);
                    if scale != 0 {
                        fdk_assert!(scale > 0);

                        /* following relation can be used for optimizations:
                         * (BandOffsets[i]%4) == 0 for all i */
                        let max_index = band_offset(band_offsets, band + 1);
                        let min_index = band_offset(band_offsets, band);
                        if FAST && 0 <= min_index && max_index as usize <= p_spectrum.len() {
                            // (an empty or reversed range scales nothing in either version)
                            for v in p_spectrum
                                [min_index as usize..max_index.max(min_index) as usize]
                                .iter_mut()
                            {
                                *v = v.wrapping_shr(scale as u32);
                            }
                        } else {
                            for index in min_index..max_index {
                                if let Some(v) = p_spectrum.get_mut(index as usize) {
                                    *v = v.wrapping_shr(scale as u32);
                                }
                            }
                        }
                    }
                }
            }
            window += 1;
        }
    }
}

/// `CBlock_ReadSectionData(bs, pAacDecoderChannelInfo, pSamplingRateInfo, flags)`
///
/// # Iteration bound (PORTING.md, "Deviations from the reference")
///
/// The section length escape and the band loop stop after more than `33 * bufBits` consecutive
/// escapes or zero-length sections; the port then returns `AAC_DEC_PARSE_ERROR`. Every other
/// input gives the same result as in C.
pub fn cblock_read_section_data(
    bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
) -> AacDecoderError {
    read_section_data::<false>(bs, p_aac_decoder_channel_info, p_sampling_rate_info, flags)
}

/// [`cblock_read_section_data`] with the codebook of a section stored with one slice fill (the
/// indexes of a section are consecutive; those outside `aCodeBook`, which only an inconsistent
/// `CIcsInfo` gives, are skipped in both versions).
pub fn cblock_read_section_data_fast(
    bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
) -> AacDecoderError {
    read_section_data::<true>(bs, p_aac_decoder_channel_info, p_sampling_rate_info, flags)
}

#[inline(always)]
fn read_section_data<const FAST: bool>(
    bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
) -> AacDecoderError {
    let p = p_aac_decoder_channel_info;
    /* HCR input (long) */
    let mut num_lines_in_sec_idx: i32 = 0;
    let band_offsets = get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info);
    p.p_dyn_data.specific_to.aac.number_section = 0;
    let error_status = AAC_DEC_OK;

    p.p_dyn_data.a_code_book = [0; 8 * 16];

    let nbits: u32 = if is_long_block(&p.ics_info) == 1 { 5 } else { 3 };

    let sect_esc_val: i32 = (1 << nbits) - 1;

    // The iteration bound (see above).
    let hang_bound: u64 = 33 * bs.h_bit_buf.buf_bits().max(1) as u64 + 1;
    let mut no_progress: u64 = 0;

    let scale_factor_bands_transmitted: u8 = get_scale_factor_bands_transmitted(&p.ics_info);
    for group in 0..get_window_groups(&p.ics_info) as i32 {
        let mut band: i32 = 0;
        while band < scale_factor_bands_transmitted as i32 {
            let mut sect_len: i32 = 0;
            let sect_cb: u8 = if flags & AC_ER_VCB11 != 0 {
                fdk_read_bits(bs, 5) as u8
            } else {
                fdk_read_bits(bs, 4) as u8
            };

            let mut sect_len_incr: i32;
            if (flags & AC_ER_VCB11) == 0 || (sect_cb < 11) || ((sect_cb > 11) && (sect_cb < 16)) {
                sect_len_incr = fdk_read_bits(bs, nbits) as i32;
                let mut escapes: u64 = 0;
                while sect_len_incr == sect_esc_val {
                    sect_len = sect_len.wrapping_add(sect_esc_val);
                    sect_len_incr = fdk_read_bits(bs, nbits) as i32;
                    escapes += 1;
                    if escapes > hang_bound {
                        // The iteration bound (see above).
                        return AAC_DEC_PARSE_ERROR;
                    }
                }
            } else {
                sect_len_incr = 1;
            }

            sect_len = sect_len.wrapping_add(sect_len_incr);

            let top: i32 = band.wrapping_add(sect_len);

            if flags & AC_ER_HCR != 0 {
                /* HCR input (long) -- collecting sideinfo (for HCR-_long_ only) */
                if num_lines_in_sec_idx >= crate::aacdec_hcr_types::MAX_SFB_HCR {
                    return AAC_DEC_PARSE_ERROR;
                }
                if top > get_number_of_scale_factor_bands(&p.ics_info, p_sampling_rate_info) as i32
                {
                    return AAC_DEC_PARSE_ERROR;
                }
                p.p_dyn_data.specific_to.aac.a_num_line_in_sec4_hcr
                    [num_lines_in_sec_idx as usize] =
                    (band_offset(band_offsets, top) - band_offset(band_offsets, band)) as i16;
                if sect_cb == BOOKSCL {
                    return AAC_DEC_INVALID_CODE_BOOK;
                } else {
                    p.p_dyn_data.specific_to.aac.a_code_books4_hcr[num_lines_in_sec_idx as usize] =
                        sect_cb;
                }
                num_lines_in_sec_idx += 1;
                p.p_dyn_data.specific_to.aac.number_section =
                    p.p_dyn_data.specific_to.aac.number_section.wrapping_add(1);
            }

            /* Check spectral line limits */
            if is_long_block(&p.ics_info) != 0 {
                if top > 64 {
                    return AAC_DEC_DECODE_FRAME_ERROR;
                }
            } else {
                /* short block */
                if top + group * 16 > (8 * 16) {
                    return AAC_DEC_DECODE_FRAME_ERROR;
                }
            }

            /* Check if decoded codebook index is feasible */
            if (sect_cb == BOOKSCL)
                || ((sect_cb == INTENSITY_HCB || sect_cb == INTENSITY_HCB2)
                    && p.p_dyn_data.raw_data_info.common_window == 0)
            {
                return AAC_DEC_INVALID_CODE_BOOK;
            }

            if top <= band {
                no_progress += 1;
                if no_progress > hang_bound {
                    // The iteration bound (see above).
                    return AAC_DEC_PARSE_ERROR;
                }
            } else {
                no_progress = 0;
            }

            /* Store codebook index */
            if FAST {
                if band < top {
                    let cbs = &mut p.p_dyn_data.a_code_book;
                    let start = ((group * 16 + band) as usize).min(cbs.len());
                    let end = ((group * 16 + top) as usize).min(cbs.len());
                    cbs[start..end].fill(sect_cb);
                    band = top;
                }
            } else {
                while band < top {
                    if let Some(c) = p.p_dyn_data.a_code_book.get_mut((group * 16 + band) as usize)
                    {
                        *c = sect_cb;
                    }
                    band += 1;
                }
            }
        }
    }

    error_status
}

/// `InverseQuantizeBand(spectrum, InverseQuantTabler, MantissaTabler, ExponentTabler, noLines,
/// scale)` (static in C): inverse quantize one sfb. Each value of the sfb is processed according
/// to the formula: `spectrum[i] = Sign(spectrum[i]) * Matissa(spectrum[i])^(4/3) * 2^(lsb/4)`.
///
/// `spectrum` holds the `no_lines` lines of the band. The values must be below 8192 in
/// magnitude (the caller checks the maximum against `MAX_QUANTIZED_VALUE`); for other values
/// the table reads use the flattened tables and read 0 outside them.
pub fn inverse_quantize_band(spectrum: &mut [FixpDbl], lsb: u32, no_lines: i32, mut scale: i32) {
    scale += 1; /* +1 to compensate fMultDiv2 shift-right in loop */

    for v in spectrum.iter_mut().take(no_lines.max(0) as usize) {
        let signed_value = *v;
        if signed_value != 0 {
            let mut value = fixp_abs(signed_value);
            let free_bits = cnt_leading_zeros(value) as u32;
            let exponent: u32 = 32u32.wrapping_sub(free_bits);

            let mut x: u32 = (value as u32).wrapping_shl(free_bits);
            x = x.wrapping_shl(1); /* shift out sign bit to avoid masking later on */
            let table_index = (x >> 24) as usize;
            x = (x >> 20) & 0x0F;

            let r0: u32 = INVERSE_QUANT_TABLE[table_index] as u32;
            let r1: u32 = INVERSE_QUANT_TABLE[table_index + 1] as u32;
            let temp: u32 = (r1.wrapping_sub(r0)).wrapping_mul(x).wrapping_add(r0 << 4);

            value = f_mult_div2(temp as FixpDbl, mantissa_table(lsb, exponent));

            /* + 1 compensates fMultDiv2() */
            // scaleValueInPlace: lsl/asr with a register amount (modulo 32) in both builds.
            scale_value_in_place(&mut value, scale.wrapping_add(exponent_table(lsb, exponent)));

            *v = if signed_value < 0 { value.wrapping_neg() } else { value };
        }
    }
}

/// Quantized magnitudes below this bound are inverse quantized with `IQ_TABLE` by
/// [`inverse_quantize_band_fast`].
pub const IQ_TABLE_SIZE: usize = 1024;

/// One entry of [`IQ_TABLE`]: the result of the `fMultDiv2()` of `InverseQuantizeBand()` for a
/// magnitude and `ExponentTabler[exponent]` of that magnitude.
#[derive(Clone, Copy)]
#[repr(C)]
struct IqEntry {
    value: FixpDbl,
    exp: i32,
}

/// The per-value computation of `InverseQuantizeBand()` up to the final scaling, for a magnitude
/// `0 < a < 2^13` (0 gives a zero entry, which every scaling keeps at 0).
const fn iq_entry(lsb: usize, a: u32) -> IqEntry {
    if a == 0 {
        return IqEntry { value: 0, exp: 0 };
    }
    let free_bits = a.leading_zeros();
    let exponent = (32 - free_bits) as usize;
    let mut x: u32 = (a << free_bits) << 1;
    let table_index = (x >> 24) as usize;
    x = (x >> 20) & 0x0F;
    let r0 = INVERSE_QUANT_TABLE[table_index] as u32;
    let r1 = INVERSE_QUANT_TABLE[table_index + 1] as u32;
    let temp: u32 = (r1.wrapping_sub(r0)).wrapping_mul(x).wrapping_add(r0 << 4);
    IqEntry {
        value: fdk_aac_rs_fdk::fixmul::fixmuldiv2_dd(
            temp as FixpDbl,
            MANTISSA_TABLE[lsb][exponent],
        ),
        exp: EXPONENT_TABLE[lsb][exponent] as i32,
    }
}

const fn iq_table() -> [[IqEntry; IQ_TABLE_SIZE]; 4] {
    let mut t = [[IqEntry { value: 0, exp: 0 }; IQ_TABLE_SIZE]; 4];
    let mut lsb = 0;
    while lsb < 4 {
        let mut a = 0;
        while a < IQ_TABLE_SIZE {
            t[lsb][a] = iq_entry(lsb, a as u32);
            a += 1;
        }
        lsb += 1;
    }
    t
}

/// `InverseQuantizeBand()` before the final scaling for every `lsb` and every magnitude below
/// [`IQ_TABLE_SIZE`] (32 KiB).
static IQ_TABLE: [[IqEntry; IQ_TABLE_SIZE]; 4] = iq_table();

/// [`inverse_quantize_band`] with a table lookup for the magnitudes below [`IQ_TABLE_SIZE`] (the
/// entry holds exactly the value and the exponent table entry that the straight code computes,
/// and zero lines get a zero entry, which every scaling keeps at 0) and without a branch on the
/// sign. Larger magnitudes (and `MINVAL_DBL`, whose magnitude is `2^31`) take the straight
/// per-value code, so the result equals [`inverse_quantize_band`] for every input.
///
/// The SIMD kernels (NEON on aarch64, AVX2 on x86 and x86-64 selected at run time) compute the
/// same per value: the table entries are loaded per lane, and the shift of
/// `scaleValueInPlace()` (left by `sf` or right by `-sf`, the amount modulo 32 as compiled) is a
/// per-lane shift by `sf` (`vshlq_s32`: left for positive, arithmetic right for negative
/// amounts; `vpsllvd`/`vpsravd` selected by the sign of `sf`), which equals it for
/// `-32 < sf < 32`. They run only when every nonzero entry's `sf = scale + exp` lies in that
/// range for this band's `scale` (`IQ_EXP_RANGE`); zero lines give 0 for every shift. A group of
/// lanes with a magnitude outside the table (or `MINVAL_DBL`) takes the scalar code.
pub fn inverse_quantize_band_fast(spectrum: &mut [FixpDbl], lsb: u32, no_lines: i32, scale: i32) {
    if lsb >= 4 {
        return inverse_quantize_band(spectrum, lsb, no_lines, scale);
    }
    let scale = scale.wrapping_add(1); /* +1 to compensate fMultDiv2 shift-right in loop */
    let tab = &IQ_TABLE[lsb as usize];
    let n = (no_lines.max(0) as usize).min(spectrum.len());
    let spectrum = &mut spectrum[..n];

    #[allow(unused_mut)]
    let mut done = 0;
    let (emin, emax) = IQ_EXP_RANGE[lsb as usize];
    if (scale as i64 + emin as i64) > -32 && (scale as i64 + emax as i64) < 32 {
        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: NEON is part of the aarch64 baseline.
            done = unsafe { iq_band_neon(spectrum, tab, lsb, scale) };
        }
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if fdk_aac_rs_fdk::simd::avx2() {
            // SAFETY: the CPU supports AVX2.
            done = unsafe { iq_band_avx2(spectrum, tab, lsb, scale) };
        }
    }

    for v in &mut spectrum[done..] {
        *v = iq_value_fast(*v, tab, lsb, scale);
    }
}

/// One value of [`inverse_quantize_band_fast`] (`scale` includes the `+ 1`).
#[inline(always)]
fn iq_value_fast(
    signed_value: FixpDbl,
    tab: &[IqEntry; IQ_TABLE_SIZE],
    lsb: u32,
    scale: i32,
) -> FixpDbl {
    // Branch-free: the signs are random, so branches on them are mispredicted.
    let sign = signed_value >> 31; /* -1 for a negative value, else 0 */
    let a = (signed_value ^ sign).wrapping_sub(sign) as u32; /* 2^31 for MINVAL_DBL */
    let value = if (a as usize) < IQ_TABLE_SIZE {
        let e = tab[a as usize];
        let sf = scale.wrapping_add(e.exp);
        // scaleValueInPlace(): shift left by sf (sf >= 0) or right by -sf (both modulo 32).
        let neg = sf >> 31;
        let left = e.value.wrapping_shl(sf as u32);
        let right = e.value.wrapping_shr(sf.wrapping_neg() as u32);
        (left & !neg) | (right & neg)
    } else {
        inverse_quantize_value_straight(signed_value, lsb, scale)
    };
    /* value or -value (wrapping) */
    (value ^ sign).wrapping_sub(sign)
}

/// The smallest and the largest `exp` of the entries of [`IQ_TABLE`] for magnitudes `1..`
/// [`IQ_TABLE_SIZE`], per `lsb`.
const fn iq_exp_range() -> [(i32, i32); 4] {
    let t = iq_table();
    let mut r = [(0, 0); 4];
    let mut lsb = 0;
    while lsb < 4 {
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        let mut a = 1;
        while a < IQ_TABLE_SIZE {
            let e = t[lsb][a].exp;
            if e < lo {
                lo = e;
            }
            if e > hi {
                hi = e;
            }
            a += 1;
        }
        r[lsb] = (lo, hi);
        lsb += 1;
    }
    r
}

const IQ_EXP_RANGE: [(i32, i32); 4] = iq_exp_range();

/// The NEON kernel of [`inverse_quantize_band_fast`] for the groups of 4 lines of `s`; returns the
/// number of lines processed. Requires `-32 < scale + exp < 32` for every entry (see there).
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn iq_band_neon(
    s: &mut [FixpDbl],
    tab: &[IqEntry; IQ_TABLE_SIZE],
    lsb: u32,
    scale: i32,
) -> usize {
    use core::arch::aarch64::*;
    let n4 = s.len() & !3;
    let vscale = vdupq_n_s32(scale);
    let limit = vdupq_n_u32(IQ_TABLE_SIZE as u32);
    let tp = tab.as_ptr() as *const i32; /* IqEntry is (value, exp) */
    let mut i = 0;
    while i < n4 {
        // SAFETY: i + 4 <= s.len(); the table indexes are below IQ_TABLE_SIZE (checked).
        unsafe {
            let p = s.as_mut_ptr().add(i);
            let x = vld1q_s32(p);
            let sign = vshrq_n_s32::<31>(x);
            let a = vreinterpretq_u32_s32(vabsq_s32(x)); /* 2^31 for MINVAL_DBL */
            if vmaxvq_u32(vcgeq_u32(a, limit)) != 0 {
                for k in 0..4 {
                    *p.add(k) = iq_value_fast(*p.add(k), tab, lsb, scale);
                }
            } else {
                let e0 = vld1_s32(tp.add(2 * vgetq_lane_u32::<0>(a) as usize));
                let e1 = vld1_s32(tp.add(2 * vgetq_lane_u32::<1>(a) as usize));
                let e2 = vld1_s32(tp.add(2 * vgetq_lane_u32::<2>(a) as usize));
                let e3 = vld1_s32(tp.add(2 * vgetq_lane_u32::<3>(a) as usize));
                let e01 = vcombine_s32(e0, e1);
                let e23 = vcombine_s32(e2, e3);
                let value = vuzp1q_s32(e01, e23);
                let exp = vuzp2q_s32(e01, e23);
                let r = vshlq_s32(value, vaddq_s32(exp, vscale));
                vst1q_s32(p, vsubq_s32(veorq_s32(r, sign), sign));
            }
        }
        i += 4;
    }
    n4
}

/// The AVX2 kernel of [`inverse_quantize_band_fast`] for the groups of 8 lines of `s`; returns the
/// number of lines processed. Requires `-32 < scale + exp < 32` for every entry (see there).
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn iq_band_avx2(
    s: &mut [FixpDbl],
    tab: &[IqEntry; IQ_TABLE_SIZE],
    lsb: u32,
    scale: i32,
) -> usize {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;
    let n8 = s.len() & !7;
    let vscale = _mm256_set1_epi32(scale);
    let max_index = _mm256_set1_epi32(IQ_TABLE_SIZE as i32 - 1);
    let zero = _mm256_setzero_si256();
    let tp = tab.as_ptr() as *const i32; /* IqEntry is (value, exp) */
    let mut i = 0;
    while i < n8 {
        // SAFETY: i + 8 <= s.len(); the table indexes are below IQ_TABLE_SIZE (checked).
        unsafe {
            let p = s.as_mut_ptr().add(i);
            let x = _mm256_loadu_si256(p as *const __m256i);
            let sign = _mm256_srai_epi32::<31>(x);
            let a = _mm256_abs_epi32(x); /* MINVAL_DBL stays negative */
            let big =
                _mm256_or_si256(_mm256_cmpgt_epi32(a, max_index), _mm256_cmpgt_epi32(zero, a));
            if _mm256_movemask_epi8(big) != 0 {
                for k in 0..8 {
                    *p.add(k) = iq_value_fast(*p.add(k), tab, lsb, scale);
                }
            } else {
                let value = _mm256_i32gather_epi32::<8>(tp, a);
                let exp = _mm256_i32gather_epi32::<8>(tp.add(1), a);
                let sf = _mm256_add_epi32(exp, vscale);
                // left for sf >= 0, arithmetic right by -sf for sf < 0 (the other shift of each
                // lane, by an amount of 32 or more, is discarded)
                let left = _mm256_sllv_epi32(value, sf);
                let right = _mm256_srav_epi32(value, _mm256_sub_epi32(zero, sf));
                let r = _mm256_blendv_epi8(left, right, _mm256_srai_epi32::<31>(sf));
                let out = _mm256_sub_epi32(_mm256_xor_si256(r, sign), sign);
                _mm256_storeu_si256(p as *mut __m256i, out);
            }
        }
        i += 8;
    }
    n8
}

/// The magnitude part of one value of [`inverse_quantize_band`] (`scale` includes the `+ 1`).
#[cold]
#[inline(never)]
fn inverse_quantize_value_straight(signed_value: FixpDbl, lsb: u32, scale: i32) -> FixpDbl {
    let mut value = fixp_abs(signed_value);
    let free_bits = cnt_leading_zeros(value) as u32;
    let exponent: u32 = 32u32.wrapping_sub(free_bits);

    let mut x: u32 = (value as u32).wrapping_shl(free_bits);
    x = x.wrapping_shl(1);
    let table_index = (x >> 24) as usize;
    x = (x >> 20) & 0x0F;

    let r0: u32 = INVERSE_QUANT_TABLE[table_index] as u32;
    let r1: u32 = INVERSE_QUANT_TABLE[table_index + 1] as u32;
    let temp: u32 = (r1.wrapping_sub(r0)).wrapping_mul(x).wrapping_add(r0 << 4);

    value = f_mult_div2(temp as FixpDbl, mantissa_table(lsb, exponent));
    scale_value_in_place(&mut value, scale.wrapping_add(exponent_table(lsb, exponent)));
    value
}

/// [`maxabs_d`] over all lines of `band` (a fold that the compiler vectorizes; the maximum does
/// not depend on the order, and `fAbs(MINVAL_DBL)`, which stays negative, never wins in either
/// version).
#[inline]
pub fn maxabs_d_fast(band: &[FixpDbl]) -> FixpDbl {
    band.iter().fold(0, |m, &v| fixp_abs(v).max(m))
}

/// `maxabs_D(pSpectralCoefficient, noLines)` (static in C): the maximum absolute value of the
/// lines of the band.
pub fn maxabs_d(p_spectral_coefficient: &[FixpDbl], no_lines: i32) -> FixpDbl {
    /* Find max spectral line value of the current sfb */
    let mut loc_max: FixpDbl = 0;

    for i in (0..no_lines.max(0) as usize).rev() {
        /* Expensive memory access */
        let v = p_spectral_coefficient.get(i).copied().unwrap_or(0);
        loc_max = fixp_abs(v).max(loc_max);
    }

    loc_max
}

/// `CBlock_InverseQuantizeSpectralData(pAacDecoderChannelInfo, pSamplingRateInfo,
/// band_is_noise, active_band_search)`
///
/// `band_is_noise` is always `pAacDecoderChannelInfo->pDynData->band_is_noise` in the library
/// (`CChannelElement_Read()` is the only caller), so the port uses that field. `spec` is the
/// spectrum of the channel. Bands outside `spec` or the tables (inconsistent `CIcsInfo`) are
/// skipped.
pub fn cblock_inverse_quantize_spectral_data(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    active_band_search: u8,
    spec: &mut [FixpDbl],
) -> AacDecoderError {
    inverse_quantize_spectral_data::<false>(
        p_aac_decoder_channel_info,
        p_sampling_rate_info,
        active_band_search,
        spec,
    )
}

/// [`cblock_inverse_quantize_spectral_data`] with [`maxabs_d_fast`],
/// [`inverse_quantize_band_fast`] and the `EvaluatePower43()` of a band maximum below
/// [`IQ_TABLE_SIZE`] taken from `IQ_TABLE` (it computes the same table index, interpolation,
/// `fMultDiv2()` and exponent as `InverseQuantizeBand()`; `+ 1` as there).
pub fn cblock_inverse_quantize_spectral_data_fast(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    active_band_search: u8,
    spec: &mut [FixpDbl],
) -> AacDecoderError {
    inverse_quantize_spectral_data::<true>(
        p_aac_decoder_channel_info,
        p_sampling_rate_info,
        active_band_search,
        spec,
    )
}

#[inline(always)]
fn inverse_quantize_spectral_data<const FAST: bool>(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    active_band_search: u8,
    spec: &mut [FixpDbl],
) -> AacDecoderError {
    let p = p_aac_decoder_channel_info;
    let scale_factor_bands_transmitted = get_scale_factor_bands_transmitted(&p.ics_info) as i32;
    let band_offsets = get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info);
    let total_bands: i16 = get_scale_factor_bands_total(&p.ics_info) as i16;
    let d = &mut *p.p_dyn_data;

    d.a_sfb_scale = [0; 8 * 16];

    let mut window: i32 = 0;
    for group in 0..get_window_groups(&p.ics_info) as i32 {
        for _groupwin in 0..group_length(&p.ics_info, group) {
            let win_off = spec_off(window, p.granule_length);
            /* inverse quantization */
            for band in 0..scale_factor_bands_transmitted {
                let off0 = band_offset(band_offsets, band);
                let no_lines = band_offset(band_offsets, band + 1) - off0;
                let bnds = (group * 16 + band) as usize;
                let sfb = (window * 16 + band) as usize;
                if bnds >= 128 || sfb >= 128 {
                    continue;
                }
                let begin = win_off + off0.max(0) as usize;
                let band_spec: &mut [FixpDbl] = if begin <= spec.len() {
                    let end = (begin + no_lines.max(0) as usize).min(spec.len());
                    &mut spec[begin..end]
                } else {
                    &mut []
                };

                let cb = d.a_code_book[bnds];
                if (cb == ZERO_HCB) || (cb == INTENSITY_HCB) || (cb == INTENSITY_HCB2) {
                    continue;
                }

                if cb == NOISE_HCB {
                    /* Leave headroom for PNS values. + 1 because ceil(log2(2^(0.25*3))) = 1,
                    worst case of additional headroom required because of the scalefactor. */
                    d.a_sfb_scale[sfb] = ((d.a_scale_factor[bnds] >> 2) + 1) as i16;
                    continue;
                }

                // band_spec holds exactly the lines that maxabs_D() reads within the spectrum (the
                // others read 0, which does not change the maximum).
                let mut loc_max =
                    if FAST { maxabs_d_fast(band_spec) } else { maxabs_d(band_spec, no_lines) };

                if active_band_search != 0 {
                    if loc_max != 0 {
                        d.band_is_noise[(group * 16 + band) as usize] = 0;
                    }
                }

                /* Cheap robustness improvement - Do not remove!!! */
                if fixp_abs(loc_max) > MAX_QUANTIZED_VALUE {
                    return AAC_DEC_PARSE_ERROR;
                }

                /* The inverse quantization operation is given by (ISO/IEC 14496-3:2009(E))
                x_invquant=Sign(x_quant). abs(x_quant)^(4/3), with the gain 2^(0.25*ScaleFactor)
                split into msb=(ScaleFactor>>2) and lsb=(ScaleFactor%4) (see the C++ source). */

                let msb: i32 = (d.a_scale_factor[bnds] >> 2) as i32;

                /* Inverse quantize band only if it is not empty */
                if loc_max != 0 {
                    let lsb: i32 = (d.a_scale_factor[bnds] & 0x03) as i32;

                    let mut scale = if FAST && (loc_max as u32 as usize) < IQ_TABLE_SIZE {
                        // loc_max > 0 here; lsb is 0..3 (scale factor & 3)
                        let e = IQ_TABLE[lsb as usize][loc_max as usize];
                        loc_max = e.value;
                        e.exp + 1
                    } else {
                        evaluate_power43(&mut loc_max, lsb as u32)
                    };

                    scale = cnt_leading_zeros(loc_max) - scale - 2;

                    d.a_sfb_scale[sfb] = (msb - scale) as i16;
                    if FAST {
                        inverse_quantize_band_fast(band_spec, lsb as u32, no_lines, scale);
                    } else {
                        inverse_quantize_band(band_spec, lsb as u32, no_lines, scale);
                    }
                } else {
                    d.a_sfb_scale[sfb] = msb as i16;
                }
            } /* for (band=0; band < ScaleFactorBandsTransmitted; band++) */

            /* Make sure the array is cleared to the end */
            let start_clear: i16 = band_offset(band_offsets, scale_factor_bands_transmitted) as i16;
            let end_clear: i16 = band_offset(band_offsets, total_bands as i32) as i16;
            let diff_clear: i32 = end_clear as i32 - start_clear as i32;
            // A negative length (MaxSfBands > TotalSfBands) clears nothing.
            let begin = win_off + (start_clear as i32).max(0) as usize;
            if diff_clear > 0 && begin + diff_clear as usize <= spec.len() {
                spec[begin..begin + diff_clear as usize].fill(0);
            }

            window += 1;
        }
    }

    AAC_DEC_OK
}

/// `CBlock_ReadSpectralData(bs, pAacDecoderChannelInfo, pSamplingRateInfo, flags)`: read the
/// Huffman encoded spectral data.
///
/// `spec` is the spectrum of the channel (at least 1024 values), `com` the channel's
/// `pComData` (HCR uses `com.overlay.aac.er_hcr_info`).
///
/// A codebook number outside `AACcodeBookDescriptionTable` (inconsistent section data) skips
/// the band; lines outside `spec` are not written.
pub fn cblock_read_spectral_data(
    bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
    spec: &mut [FixpDbl],
    com: &mut CAacDecoderCommonData,
) -> AacDecoderError {
    let p = p_aac_decoder_channel_info;
    let band_offsets = get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info);

    fdk_assert!(!band_offsets.is_empty());

    let n_clear = spec.len().min(1024);
    spec[..n_clear].fill(0);

    if (flags & AC_ER_HCR) == 0 {
        let scale_factor_bands_transmitted = get_scale_factor_bands_transmitted(&p.ics_info) as i32;
        let granule_length = p.granule_length;

        let mut groupoffset: i32 = 0;

        /* plain huffman decoder  short */
        let max_group = get_window_groups(&p.ics_info) as i32;

        for group in 0..max_group {
            let max_groupwin = group_length(&p.ics_info, group);

            let mut bnds = (group * 16) as usize;

            let mut band_offset1 = band_offset(band_offsets, 0);
            for band in 0..scale_factor_bands_transmitted {
                let Some(&cb) = p.p_dyn_data.a_code_book.get(bnds) else { break };
                let mut current_cb = cb;
                let band_offset0 = band_offset1;
                band_offset1 = band_offset(band_offsets, band + 1);

                /* patch to run plain-huffman-decoder with vcb11 input codebooks (LAV-checking
                 * might be possible below using the virtual cb and a LAV-table) */
                if (16..=31).contains(&current_cb) {
                    current_cb = 11;
                    p.p_dyn_data.a_code_book[bnds] = current_cb;
                }
                if (current_cb != ZERO_HCB)
                    && (current_cb != NOISE_HCB)
                    && (current_cb != INTENSITY_HCB)
                    && (current_cb != INTENSITY_HCB2)
                {
                    let Some(hcb) = AAC_CODE_BOOK_DESCRIPTION_TABLE.get(current_cb as usize) else {
                        bnds += 1;
                        continue;
                    };
                    let step = hcb.dimension as i32;
                    let offset = hcb.offset as i32;
                    let bits = hcb.num_bits as i32;
                    let mask = (1 << bits) - 1;
                    let code_book = hcb.code_book;

                    let mut md = (groupoffset * granule_length) as isize;

                    let put = |spec: &mut [FixpDbl], md: isize, index: i32, v: FixpDbl| {
                        if let Some(s) = spec.get_mut((md + index as isize) as usize) {
                            *s = v;
                        }
                    };
                    let get = |spec: &[FixpDbl], md: isize, index: i32| -> FixpDbl {
                        spec.get((md + index as isize) as usize).copied().unwrap_or(0)
                    };

                    for _groupwin in 0..max_groupwin {
                        let mut index = band_offset0;
                        while index < band_offset1 {
                            let mut idx = cblock_decode_huffman_word_cb(bs, code_book);
                            if offset == 0 {
                                for i in 0..step {
                                    let mut tmp: FixpDbl = (idx & mask) - offset;
                                    if tmp != 0 {
                                        tmp = if fdk_read_bit(bs) != 0 { -tmp } else { tmp };
                                    }
                                    put(spec, md, index + i, tmp);
                                    idx >>= bits;
                                }
                            } else {
                                for i in 0..step {
                                    put(spec, md, index + i, (idx & mask) - offset);
                                    idx >>= bits;
                                }
                            }

                            if current_cb == ESCBOOK {
                                for j in 0..2 {
                                    let q = get(spec, md, index + j);
                                    put(spec, md, index + j, cblock_get_escape(bs, q));
                                }
                            }
                            index += step.max(1);
                        }
                        md += granule_length as isize;
                    }
                }
                bnds += 1;
            }
            groupoffset += max_groupwin;
        }
        /* plain huffman decoding (short) finished */
    } else {
        /* HCR - Huffman Codeword Reordering  short */
        let h_hcr = &mut com.overlay.aac.er_hcr_info;

        /* advanced Huffman decoding starts here (HCR decoding :) */
        if p.p_dyn_data.specific_to.aac.len_of_reordered_spectral_data != 0 {
            /* HCR initialization short */
            let mut hcr_status = hcr_init(h_hcr, p, p_sampling_rate_info, bs);

            if hcr_status != 0 {
                return AAC_DEC_DECODE_FRAME_ERROR;
            }

            /* HCR decoding short */
            hcr_status = hcr_decoder(h_hcr, p, p_sampling_rate_info, bs, spec);

            if hcr_status != 0 {
                /* HCR_ERROR_CONCEALMENT is 1 */
                hcr_mute_erroneous_lines(h_hcr, spec);
            }

            // Read after HcrDecoder(): HcrInit() can write the byte after aCodeBooks4Hcr, the
            // low byte of lenOfReorderedSpectralData (reproduced from the C layout), which
            // changes this length as in C.
            fdk_push_for(bs, p.p_dyn_data.specific_to.aac.len_of_reordered_spectral_data as u32);
        }
    }
    /* HCR - Huffman Codeword Reordering short finished */

    if is_long_block(&p.ics_info) != 0 && (flags & (AC_ELD | AC_SCALABLE)) == 0 {
        /* apply pulse data */
        cpulse_data_apply(
            &p.p_dyn_data.specific_to.aac.pulse_data,
            get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info),
            spec,
        );
    }

    AAC_DEC_OK
}

/// Reads the sign bits of the nonzero values of one codeword of an unsigned codebook and
/// applies them, without branches on the values or the bits (they are unpredictable).
///
/// The straight code reads one bit per nonzero value with `FDKreadBit()`, in the order of the
/// values. This reads them with one `FDKreadBits(n)`: `FDKreadBit()` equals `FDKreadBits(1)`,
/// and a sequence of reads of `n1, n2, ...` bits (each at most 32) leaves the bitstream in the
/// same state as one read of their sum, as long as the cache holds at most 32 bits (a read
/// refills exactly when it needs more bits than the cache holds, and every refill adds 32 bits).
/// `FDKreadBits(0)` changes nothing. The first value's bit is the most significant one.
#[inline(always)]
fn apply_sign_bits<const DIM: usize>(bs: &mut FdkBitStream, v: &mut [FixpDbl; DIM]) {
    let mut n: u32 = 0;
    for x in v.iter() {
        n += (*x != 0) as u32;
    }
    let bits = fdk_read_bits(bs, n);
    let mut shift = n;
    for x in v.iter_mut() {
        let nz = (*x != 0) as u32;
        shift -= nz;
        let s = ((bits >> shift) & nz) as FixpDbl; /* 1: negative */
        *x = (*x ^ s.wrapping_neg()).wrapping_add(s);
    }
}

/// Number of bits that [`HUFF_LUT`] looks ahead (an even number: the decode trees consume two
/// bits per step).
pub const HUFF_LUT_BITS: u32 = 8;

const HUFF_LUT_RESOLVED: u32 = 1 << 31;
const HUFF_LUT_PUSH_BACK: u32 = 1 << 30;

/// The [`HUFF_LUT`] entry of the codeword that starts with the `HUFF_LUT_BITS` bits `prefix`:
/// the walk of [`cblock_decode_huffman_word_cb`] over these bits. Either it ends within them
/// (`HUFF_LUT_RESOLVED`, the bits it read in bits 16..21, `HUFF_LUT_PUSH_BACK` if it pushed one
/// bit back, the index in bits 0..16), or the entry holds the node it continues from after
/// reading all of them.
const fn huff_lut_entry(code_book: &[[u16; HuffmanEntries as usize]], prefix: u32) -> u32 {
    let mut index: u32 = 0;
    let mut step = 0;
    while step < HUFF_LUT_BITS / 2 {
        let two = (prefix >> (HUFF_LUT_BITS - 2 - 2 * step)) & 3;
        index = code_book[index as usize][two as usize] as u32;
        if index & 1 != 0 {
            let push_back = if index & 2 != 0 { HUFF_LUT_PUSH_BACK } else { 0 };
            return HUFF_LUT_RESOLVED | push_back | ((2 * (step + 1)) << 16) | (index >> 2);
        }
        index >>= 2;
        step += 1;
    }
    index
}

const fn huff_lut() -> [[u32; 1 << HUFF_LUT_BITS]; 13] {
    let mut t = [[0u32; 1 << HUFF_LUT_BITS]; 13];
    let mut cb = 1;
    while cb < 13 {
        let code_book = AAC_CODE_BOOK_DESCRIPTION_TABLE[cb].code_book;
        let mut prefix = 0;
        while prefix < 1 << HUFF_LUT_BITS {
            t[cb][prefix] = huff_lut_entry(code_book, prefix as u32);
            prefix += 1;
        }
        cb += 1;
    }
    t
}

/// The first `HUFF_LUT_BITS` bits of the decoding of every spectral codebook (1..=11) and of the
/// scale factor codebook (`BOOKSCL`, 12), built at compile time from the decode trees of
/// `AACcodeBookDescriptionTable` (13 KiB).
pub static HUFF_LUT: [[u32; 1 << HUFF_LUT_BITS]; 13] = huff_lut();

/// [`cblock_decode_huffman_word_cb`] with a table lookup for the first `HUFF_LUT_BITS` bits.
///
/// The straight walk reads two bits per step with `FDKread2Bits()` and pushes one bit back at
/// the end of an odd-length codeword. This peeks the bits instead, takes the result of the walk
/// over them from `lut` and then skips the bits the walk read (and pushes one back), or skips
/// all of them and continues the walk. Reads of 2, 2, ... bits leave the bitstream in the same
/// state as one read of their sum (see `apply_sign_bits`), so the state equals the straight
/// one, including the cache refill of a walk that pushes back the last bit of a refilled
/// cache. Requires a cache of at most 32 bits.
#[inline(always)]
pub fn cblock_decode_huffman_word_lut(
    bs: &mut FdkBitStream,
    code_book: &[[u16; HuffmanEntries as usize]],
    lut: &[u32; 1 << HUFF_LUT_BITS],
) -> i32 {
    let e = lut[fdk_peek_bits(bs, HUFF_LUT_BITS) as usize];
    if e & HUFF_LUT_RESOLVED != 0 {
        fdk_skip_cached_bits(bs, (e >> 16) & 0x1F);
        if e & HUFF_LUT_PUSH_BACK != 0 {
            fdk_push_back_cache(bs, 1);
        }
        (e & 0xFFFF) as i32
    } else {
        fdk_skip_cached_bits(bs, HUFF_LUT_BITS);
        decode_huffman_word_cb_from(bs, code_book, e)
    }
}

/// The walk of [`cblock_decode_huffman_word_cb`] from the node `index`.
#[inline(never)]
fn decode_huffman_word_cb_from(
    bs: &mut FdkBitStream,
    code_book: &[[u16; HuffmanEntries as usize]],
    mut index: u32,
) -> i32 {
    loop {
        let Some(row) = code_book.get(index as usize) else { return 0 };
        index = row[fdk_read2_bits(bs) as usize] as u32;
        if index & 1 != 0 {
            break;
        }
        index >>= 2;
    }
    if index & 2 != 0 {
        fdk_push_back_cache(bs, 1);
    }
    (index >> 2) as i32
}

/// Number of bits that [`HUFF_SIGN_LUT`] looks ahead (at most 16). 10 bits (4 KiB per codebook)
/// measured fastest on arm64: 9 and 11 bits were slower, 12 and 14 bits slower than the
/// decoding without this table (the larger tables miss the cache).
pub const HUFF_SIGN_LUT_BITS: u32 = 10;

const HSL_FULL: u32 = 1 << 31;
const HSL_SIGN: u32 = 1 << 30;
const HSL_PUSH_BACK: u32 = 1 << 26;

/// The [`HUFF_SIGN_LUT`] entry of a spectral codebook (`dim` values of `bits` bits, `offset`)
/// for the `HUFF_SIGN_LUT_BITS` bits `prefix`, from the walk of
/// [`cblock_decode_huffman_word_cb`] over these bits and, for an unsigned codebook, the sign
/// bits of the nonzero values that follow the codeword (read as in [`apply_sign_bits`]):
///
/// - `HSL_FULL`: codeword and sign bits lie within the prefix. Bits 0..16 hold the values with
///   their signs applied (`dim` fields of `16 / dim` bits, two's complement, the first value in
///   the lowest field), bits 16..21 the number `t` of bits that codeword and signs occupy,
///   bits 21..26 the refill threshold `r`: `t + 1` if the walk pushed one bit back and no sign
///   bits follow, otherwise `t` (see [`decode_huffman_word_signs`]).
/// - `HSL_SIGN`: the codeword lies within the prefix, its sign bits do not. The values without
///   signs as above, bits 16..21 the bits the walk read, `HSL_PUSH_BACK` if it pushed one back.
/// - neither: the walk continues from the node in bits 0..16, which it reaches after the first
///   `HUFF_SIGN_LUT_BITS & !1` bits of the prefix (the walk reads two bits per step).
const fn huff_sign_lut_entry(
    code_book: &[[u16; HuffmanEntries as usize]],
    dim: u32,
    bits: u32,
    offset: i32,
    prefix: u32,
) -> u32 {
    const K: u32 = HUFF_SIGN_LUT_BITS;
    let mut index: u32 = 0;
    let mut step = 0;
    while step < K / 2 {
        let two = (prefix >> (K - 2 - 2 * step)) & 3;
        index = code_book[index as usize][two as usize] as u32;
        if index & 1 != 0 {
            let walk = 2 * (step + 1);
            let push_back = (index >> 1) & 1;
            let len = walk - push_back;
            let mut idx = index >> 2;
            let mask = (1u32 << bits) - 1;
            let w = 16 / dim;
            let field = (1u32 << w) - 1;
            // values and the number of sign bits
            let mut v = [0i32; 4];
            let mut n = 0;
            let mut i = 0;
            while i < dim as usize {
                v[i] = (idx & mask) as i32 - offset;
                idx >>= bits;
                if offset == 0 && v[i] != 0 {
                    n += 1;
                }
                i += 1;
            }
            if len + n <= K {
                // apply the signs: the first value's bit is the most significant one
                let mut pos = len;
                let mut packed = 0u32;
                let mut i = 0;
                while i < dim as usize {
                    if offset == 0 && v[i] != 0 {
                        if (prefix >> (K - 1 - pos)) & 1 != 0 {
                            v[i] = -v[i];
                        }
                        pos += 1;
                    }
                    packed |= ((v[i] as u32) & field) << (w * i as u32);
                    i += 1;
                }
                let t = len + n;
                let r = if push_back != 0 && n == 0 { t + 1 } else { t };
                return HSL_FULL | (r << 21) | (t << 16) | packed;
            }
            let mut packed = 0u32;
            let mut i = 0;
            while i < dim as usize {
                packed |= ((v[i] as u32) & field) << (w * i as u32);
                i += 1;
            }
            let pb = if push_back != 0 { HSL_PUSH_BACK } else { 0 };
            return HSL_SIGN | pb | (walk << 16) | packed;
        }
        index >>= 2;
        step += 1;
    }
    index
}

const fn huff_sign_lut() -> [[u32; 1 << HUFF_SIGN_LUT_BITS]; 12] {
    let mut t = [[0u32; 1 << HUFF_SIGN_LUT_BITS]; 12];
    let mut cb = 1;
    while cb < 12 {
        let d = &AAC_CODE_BOOK_DESCRIPTION_TABLE[cb];
        let mut prefix = 0;
        while prefix < 1 << HUFF_SIGN_LUT_BITS {
            t[cb][prefix] = huff_sign_lut_entry(
                d.code_book,
                d.dimension as u32,
                d.num_bits as u32,
                d.offset as i32,
                prefix as u32,
            );
            prefix += 1;
        }
        cb += 1;
    }
    t
}

/// The codeword and sign decoding of every spectral codebook (1..=11) for the first
/// `HUFF_SIGN_LUT_BITS` bits, built at compile time from the decode trees of
/// `AACcodeBookDescriptionTable` and the parameters of the codebooks (see
/// `huff_sign_lut_entry`).
pub static HUFF_SIGN_LUT: [[u32; 1 << HUFF_SIGN_LUT_BITS]; 12] = huff_sign_lut();

/// The `DIM` values of a [`HUFF_SIGN_LUT`] entry (fields of `16 / DIM` bits).
#[inline(always)]
fn huff_sign_lut_values<const DIM: usize>(e: u32) -> [FixpDbl; DIM] {
    let w = 16 / DIM as u32;
    let mut v = [0 as FixpDbl; DIM];
    for (i, x) in v.iter_mut().enumerate() {
        *x = ((e << (32 - w * (i as u32 + 1))) as i32) >> (32 - w);
    }
    v
}

/// One codeword of a spectral codebook with the sign bits of its values (unsigned codebooks):
/// the values of [`cblock_decode_huffman_word_cb`] unpacked as in [`cblock_read_spectral_data`]
/// (`DIM` values of `BITS` bits, minus `OFFSET`), with the signs of `apply_sign_bits` applied
/// when `OFFSET` is 0. `lut` is the codebook's [`HUFF_SIGN_LUT`] row, `code_book` its tree.
///
/// The straight code reads the codeword with `FDKread2Bits()` (`w` bits), pushes back `p`
/// (0 or 1) bits and then reads the `n` sign bits. Where the table decides all of it, this skips
/// `t = w - p + n` bits at once. A read refills the cache (the next `FDK_get32()` word, 32 more
/// bits) exactly when it needs more bits than the cache holds; with `c` bits in the cache:
///
/// - `w <= c`: the straight reads refill (in the sign read) iff `n > c - w + p`, i.e. `t > c`,
///   as the single skip does, and leave `c - t` (plus 32 after a refill) bits.
/// - `w > c`: the codeword read refills; the sign read cannot refill again (`c + 32 - w + p >=
///   32 - HUFF_SIGN_LUT_BITS > n`). The single skip refills too iff `t > c`, which fails only
///   for `t = c = w - 1`, i.e. `p = 1, n = 0`: there the straight code refills and pushes the
///   last bit back into the full cache (32 bits). The entry's refill threshold `r` is `t + 1`
///   exactly in this case, so "refill iff `r > c`, then `c + 32 - t` bits" covers all cases.
///
/// Entries that do not decide the signs take the table walk: the walk read as in
/// [`cblock_decode_huffman_word_lut`] (or continued from the entry's node), then
/// `apply_sign_bits`. A branch-free form (peeking from the cache and the next word computed on
/// a copy of the bit buffer, the refill as selects) measured slower.
///
/// Requires a cache of at most 32 bits.
#[inline(always)]
pub fn decode_huffman_word_signs<const DIM: usize, const BITS: u32, const OFFSET: i32>(
    bs: &mut FdkBitStream,
    code_book: &[[u16; HuffmanEntries as usize]],
    lut: &[u32; 1 << HUFF_SIGN_LUT_BITS],
) -> [FixpDbl; DIM] {
    let e = lut[fdk_peek_bits(bs, HUFF_SIGN_LUT_BITS) as usize];
    if e & HSL_FULL != 0 {
        let t = (e >> 16) & 0x1F;
        let r = (e >> 21) & 0x1F;
        if r > bs.bits_in_cache {
            bs.cache_word = fdk_get32(&mut bs.h_bit_buf) as u32;
            bs.bits_in_cache += CACHE_BITS;
        }
        bs.bits_in_cache -= t;
        huff_sign_lut_values::<DIM>(e)
    } else {
        decode_huffman_word_signs_slow::<DIM, BITS, OFFSET>(bs, code_book, e)
    }
}

/// The [`decode_huffman_word_signs`] of entries that do not decide the signs: the walk (from
/// the table, or continued from its node), then [`apply_sign_bits`].
#[inline(always)]
fn decode_huffman_word_signs_slow<const DIM: usize, const BITS: u32, const OFFSET: i32>(
    bs: &mut FdkBitStream,
    code_book: &[[u16; HuffmanEntries as usize]],
    e: u32,
) -> [FixpDbl; DIM] {
    let mut v;
    if e & HSL_SIGN != 0 {
        fdk_skip_cached_bits(bs, (e >> 16) & 0x1F);
        if e & HSL_PUSH_BACK != 0 {
            fdk_push_back_cache(bs, 1);
        }
        v = huff_sign_lut_values::<DIM>(e);
    } else {
        fdk_skip_cached_bits(bs, HUFF_SIGN_LUT_BITS & !1);
        let mut idx = decode_huffman_word_cb_from(bs, code_book, e);
        let mask: i32 = (1 << BITS) - 1;
        v = [0 as FixpDbl; DIM];
        for x in v.iter_mut() {
            *x = (idx & mask) - OFFSET;
            idx >>= BITS;
        }
    }
    if OFFSET == 0 {
        apply_sign_bits(bs, &mut v);
    }
    v
}

/// The plain Huffman decoding of one band of [`cblock_read_spectral_data`] (all windows of the
/// group), specialized for the codebook parameters. Equals the loop of the straight function:
/// the values of a codeword are unpacked and signed ([`decode_huffman_word_signs`]) and escaped
/// in the same order, and written to the spectrum at once where all of them are inside it (the escape
/// sequence then uses the values themselves instead of reading them back); elsewhere the
/// straight per-value code runs, which drops writes and reads 0 outside the spectrum.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn read_band_fast<const DIM: usize, const BITS: u32, const OFFSET: i32, const ESC: bool>(
    bs: &mut FdkBitStream,
    code_book: &[[u16; HuffmanEntries as usize]],
    lut: &[u32; 1 << HUFF_SIGN_LUT_BITS],
    spec: &mut [FixpDbl],
    mut md: isize,
    granule_length: i32,
    max_groupwin: i32,
    band_offset0: i32,
    band_offset1: i32,
) {
    for _groupwin in 0..max_groupwin {
        let mut index = band_offset0;
        while index < band_offset1 {
            let mut v = decode_huffman_word_signs::<DIM, BITS, OFFSET>(bs, code_book, lut);
            let base = md + index as isize;
            if base >= 0 && (base as usize) + DIM <= spec.len() {
                if ESC {
                    v[0] = cblock_get_escape_fast(bs, v[0]);
                    v[1] = cblock_get_escape_fast(bs, v[1]);
                }
                spec[base as usize..base as usize + DIM].copy_from_slice(&v);
            } else {
                for (i, &x) in v.iter().enumerate() {
                    if let Some(s) = spec.get_mut((base + i as isize) as usize) {
                        *s = x;
                    }
                }
                if ESC {
                    for j in 0..2 {
                        let pos = (base + j) as usize;
                        let q = spec.get(pos).copied().unwrap_or(0);
                        let e = cblock_get_escape(bs, q);
                        if let Some(s) = spec.get_mut(pos) {
                            *s = e;
                        }
                    }
                }
            }
            index += DIM as i32;
        }
        md += granule_length as isize;
    }
}

/// [`cblock_read_spectral_data`] with the plain Huffman decoding specialized per codebook
/// (`read_band_fast`). HCR, and a cache that holds more than 32 bits (which the library never
/// leaves), take the straight function.
pub fn cblock_read_spectral_data_fast(
    bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
    spec: &mut [FixpDbl],
    com: &mut CAacDecoderCommonData,
) -> AacDecoderError {
    if (flags & AC_ER_HCR) != 0 || bs.bits_in_cache > CACHE_BITS {
        return cblock_read_spectral_data(
            bs,
            p_aac_decoder_channel_info,
            p_sampling_rate_info,
            flags,
            spec,
            com,
        );
    }
    let p = p_aac_decoder_channel_info;
    let band_offsets = get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info);

    fdk_assert!(!band_offsets.is_empty());

    let n_clear = spec.len().min(1024);
    spec[..n_clear].fill(0);

    let scale_factor_bands_transmitted = get_scale_factor_bands_transmitted(&p.ics_info) as i32;
    let granule_length = p.granule_length;

    let mut groupoffset: i32 = 0;

    let max_group = get_window_groups(&p.ics_info) as i32;

    for group in 0..max_group {
        let max_groupwin = group_length(&p.ics_info, group);

        let mut bnds = (group * 16) as usize;

        let mut band_offset1 = band_offset(band_offsets, 0);
        for band in 0..scale_factor_bands_transmitted {
            let Some(&cb) = p.p_dyn_data.a_code_book.get(bnds) else { break };
            let mut current_cb = cb;
            let band_offset0 = band_offset1;
            band_offset1 = band_offset(band_offsets, band + 1);

            if (16..=31).contains(&current_cb) {
                current_cb = 11;
                p.p_dyn_data.a_code_book[bnds] = current_cb;
            }
            if (current_cb != ZERO_HCB)
                && (current_cb != NOISE_HCB)
                && (current_cb != INTENSITY_HCB)
                && (current_cb != INTENSITY_HCB2)
            {
                let Some(hcb) = AAC_CODE_BOOK_DESCRIPTION_TABLE.get(current_cb as usize) else {
                    bnds += 1;
                    continue;
                };
                let code_book = hcb.code_book;
                let md = (groupoffset * granule_length) as isize;
                let esc = current_cb == ESCBOOK;
                macro_rules! band {
                    ($dim:literal, $bits:literal, $off:literal, $esc:literal) => {
                        read_band_fast::<$dim, $bits, $off, $esc>(
                            bs,
                            code_book,
                            &HUFF_SIGN_LUT[current_cb as usize],
                            spec,
                            md,
                            granule_length,
                            max_groupwin,
                            band_offset0,
                            band_offset1,
                        )
                    };
                }
                match (hcb.dimension, hcb.num_bits, hcb.offset, esc) {
                    (4, 2, 1, false) => band!(4, 2, 1, false),
                    (4, 2, 0, false) => band!(4, 2, 0, false),
                    (2, 4, 4, false) => band!(2, 4, 4, false),
                    (2, 4, 0, false) => band!(2, 4, 0, false),
                    (2, 5, 0, true) => band!(2, 5, 0, true),
                    // Other parameters (none in the tables of the library): the straight loop.
                    _ => read_band_straight(
                        bs,
                        hcb,
                        current_cb,
                        spec,
                        md,
                        granule_length,
                        max_groupwin,
                        band_offset0,
                        band_offset1,
                    ),
                }
            }
            bnds += 1;
        }
        groupoffset += max_groupwin;
    }

    if is_long_block(&p.ics_info) != 0 && (flags & (AC_ELD | AC_SCALABLE)) == 0 {
        /* apply pulse data */
        cpulse_data_apply(
            &p.p_dyn_data.specific_to.aac.pulse_data,
            get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info),
            spec,
        );
    }

    AAC_DEC_OK
}

/// The band loop of [`cblock_read_spectral_data`] for any codebook parameters (the fallback of
/// [`cblock_read_spectral_data_fast`]).
#[allow(clippy::too_many_arguments)]
fn read_band_straight(
    bs: &mut FdkBitStream,
    hcb: &CodeBookDescription,
    current_cb: u8,
    spec: &mut [FixpDbl],
    mut md: isize,
    granule_length: i32,
    max_groupwin: i32,
    band_offset0: i32,
    band_offset1: i32,
) {
    let step = hcb.dimension as i32;
    let offset = hcb.offset as i32;
    let bits = hcb.num_bits as i32;
    let mask = (1 << bits) - 1;
    let code_book = hcb.code_book;
    let put = |spec: &mut [FixpDbl], md: isize, index: i32, v: FixpDbl| {
        if let Some(s) = spec.get_mut((md + index as isize) as usize) {
            *s = v;
        }
    };
    let get = |spec: &[FixpDbl], md: isize, index: i32| -> FixpDbl {
        spec.get((md + index as isize) as usize).copied().unwrap_or(0)
    };
    for _groupwin in 0..max_groupwin {
        let mut index = band_offset0;
        while index < band_offset1 {
            let mut idx = cblock_decode_huffman_word_cb(bs, code_book);
            if offset == 0 {
                for i in 0..step {
                    let mut tmp: FixpDbl = (idx & mask) - offset;
                    if tmp != 0 {
                        tmp = if fdk_read_bit(bs) != 0 { -tmp } else { tmp };
                    }
                    put(spec, md, index + i, tmp);
                    idx >>= bits;
                }
            } else {
                for i in 0..step {
                    put(spec, md, index + i, (idx & mask) - offset);
                    idx >>= bits;
                }
            }
            if current_cb == ESCBOOK {
                for j in 0..2 {
                    let q = get(spec, md, index + j);
                    put(spec, md, index + j, cblock_get_escape(bs, q));
                }
            }
            index += step.max(1);
        }
        md += granule_length as isize;
    }
}

/// `noise_level_tab[8]` (static in C): `FDKpow(2, (float)(noise_level-14)/3.0f) * 2` (*2 to
/// compensate for fMultDiv2); `noise_level_tab(noise_level==0) == 0` by definition.
///
/// `scripts/dump_table.py libAACdec/src/block.cpp noise_level_tab SHORT --hex`
pub static NOISE_LEVEL_TAB: [FixpSgl; 8] =
    [0x0000, 0x0CB3, 0x1000, 0x1429, 0x1966, 0x2000, 0x2851, 0x32CC];

/// `CBlock_ApplyNoise(pAacDecoderChannelInfo, pSamplingRateInfo, nfRandomSeed, band_is_noise)`:
/// USAC noise filling.
///
/// `band_is_noise` is always `pAacDecoderChannelInfo->pDynData->band_is_noise` in the library
/// (`CChannelElement_Read()` is the only caller), so the port uses that field. `spec` is the
/// spectrum of the channel.
///
/// The search for the first band of the noise filling ends at the end of the band offset table.
/// Lines outside `spec` and array indexes outside the arrays (inconsistent `CIcsInfo`) are
/// skipped.
pub fn cblock_apply_noise(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    nf_random_seed: &mut u32,
    spec: &mut [FixpDbl],
) {
    let p = p_aac_decoder_channel_info;
    let swb_offset = get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info);

    /* Obtain noise level and scale factor offset. */
    let noise_level = (p.p_dyn_data.specific_to.usac.fd_noise_level_and_offset >> 5) as usize;
    let noise_val_pos: FixpSgl = NOISE_LEVEL_TAB[noise_level];

    /* noise_offset can change even when noise_level=0. Neccesary for IGF stereo filling */
    let noise_offset: i32 =
        (p.p_dyn_data.specific_to.usac.fd_noise_level_and_offset & 0x1f) as i32 - 16;

    let max_sfb = get_scale_factor_bands_transmitted(&p.ics_info) as i32;

    let mut noise_filling_start_offset: i32 =
        if get_window_sequence(&p.ics_info) == BLOCK_SHORT { 20 } else { 160 };
    if p.granule_length == 96 {
        noise_filling_start_offset = (3 * noise_filling_start_offset) / 4; /* scale offset with 3/4 for coreCoderFrameLength == 768 */
    }

    /* determine sfb from where on noise filling is applied */
    let mut sfb: i32 = 0;
    while (sfb as usize) < swb_offset.len()
        && (swb_offset[sfb as usize] as i32) < noise_filling_start_offset
    {
        sfb += 1;
    }
    let nf_start_offset_sfb = sfb;

    /* if (noise_level!=0) */
    {
        let mut win: i32 = 0;
        for g in 0..get_window_groups(&p.ics_info) as i32 {
            let window_group_length = group_length(&p.ics_info, g);
            for sfb in nf_start_offset_sfb..max_sfb {
                let bin_start = band_offset(swb_offset, sfb);
                let bin_stop = band_offset(swb_offset, sfb + 1);

                let gsfb = (g * 16 + sfb) as usize;
                if gsfb >= 128 {
                    continue;
                }
                let flag_n = p.p_dyn_data.band_is_noise[gsfb] as i32;

                /* if all bins of one sfb in one window group are zero modify the scale factor
                 * by noise_offset */
                if flag_n != 0 {
                    /* Change scaling factors for empty signal bands */
                    p.p_dyn_data.a_scale_factor[gsfb] =
                        (p.p_dyn_data.a_scale_factor[gsfb] as i32 + noise_offset) as i16;
                    /* scale factor "sf" implied gain "g" is g = 2^(sf/4) */
                    for gwin in 0..window_group_length {
                        if let Some(s) =
                            p.p_dyn_data.a_sfb_scale.get_mut(((win + gwin) * 16 + sfb) as usize)
                        {
                            *s = (*s as i32 + (noise_offset >> 2)) as i16;
                        }
                    }
                }

                let mut seed: u32 = *nf_random_seed;
                /* + 1 because exponent of MantissaTable[lsb][0] is always 1. */
                let scale: i32 = (p.p_dyn_data.a_scale_factor[gsfb] as i32 >> 2) + 1;
                let lsb = (p.p_dyn_data.a_scale_factor[gsfb] & 3) as usize;
                let mantissa: FixpDbl = MANTISSA_TABLE[lsb][0];

                for gwin in 0..window_group_length {
                    let p_spec_off = spec_off(win + gwin, p.granule_length);

                    let sfb_scale = p
                        .p_dyn_data
                        .a_sfb_scale
                        .get(((win + gwin) * 16 + sfb) as usize)
                        .copied()
                        .unwrap_or(0) as i32;
                    let scale1 = scale - sfb_scale;
                    // scaleValue(): lsl/asr with a register amount (modulo 32) in both builds.
                    let scaled_noise_val_pos =
                        scale_value(f_mult_div2(noise_val_pos, mantissa), scale1);
                    let scaled_noise_val_neg = scaled_noise_val_pos.wrapping_neg();

                    /* If the whole band is zero, just fill without checking */
                    if flag_n != 0 {
                        for bin in bin_start..bin_stop {
                            seed = seed.wrapping_mul(69069).wrapping_add(5); /* Inlined: UsacRandomSign - origin in usacdec_lpd.h */
                            if let Some(s) = spec.get_mut(p_spec_off + bin as usize) {
                                *s = if seed & 0x10000 != 0 {
                                    scaled_noise_val_neg
                                } else {
                                    scaled_noise_val_pos
                                };
                            }
                        } /* for (bin...) */
                    }
                    /*If band is sparsely filled, check for 0 and fill */
                    else {
                        for bin in bin_start..bin_stop {
                            if let Some(s) = spec.get_mut(p_spec_off + bin as usize) {
                                if *s == 0 {
                                    seed = seed.wrapping_mul(69069).wrapping_add(5); /* Inlined: UsacRandomSign - origin in usacdec_lpd.h */
                                    *s = if seed & 0x10000 != 0 {
                                        scaled_noise_val_neg
                                    } else {
                                        scaled_noise_val_pos
                                    };
                                }
                            }
                        } /* for (bin...) */
                    }
                } /* for (gwin...) */
                *nf_random_seed = seed;
            } /* for (sfb...) */
            win += window_group_length;
        } /* for (g...) */
    } /* ... */
}

/// `CBlock_ReadAcSpectralData(hBs, pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo,
/// pSamplingRateInfo, frame_length, flags)`: read the arithmetic coded spectral data.
///
/// `spec` is the spectrum of the channel. Without an arithmetic decoder state (`hArCo == NULL`,
/// where the reference has no defined result) the port returns `AAC_DEC_PARSE_ERROR`.
pub fn cblock_read_ac_spectral_data(
    h_bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    frame_length: u32,
    flags: u32,
    spec: &mut [FixpDbl],
) -> AacDecoderError {
    let p = p_aac_decoder_channel_info;
    let mut error_aac = AAC_DEC_OK;
    let mut error = ARITH_CODER_OK;

    /* number of transmitted spectral coefficients */
    let band_offsets = get_scale_factor_band_offsets(&p.ics_info, p_sampling_rate_info);
    let lg = band_offset(band_offsets, get_scale_factor_bands_transmitted(&p.ics_info) as i32);

    let num_win = get_windows_per_frame(&p.ics_info);
    let win_len: i32 = if is_long_block(&p.ics_info) != 0 {
        frame_length as i32
    } else {
        frame_length as i32 / num_win
    };

    let arith_reset_flag: i32 =
        if flags & AC_INDEP != 0 { 1 } else { fdk_read_bits(h_bs, 1) as u16 as i32 };

    let Some(h_ar_co) = p_aac_decoder_static_channel_info.h_ar_co.as_deref_mut() else {
        return AAC_DEC_PARSE_ERROR;
    };

    for win in 0..num_win {
        let off = spec_off(win, p.granule_length).min(spec.len());
        error = carco_decode_arith_data(
            h_ar_co,
            h_bs,
            &mut spec[off..],
            lg,
            win_len,
            (arith_reset_flag != 0 && win == 0) as i32,
        );
        if error != ARITH_CODER_OK {
            break;
        }
    }

    /* bail: */
    if error == ARITH_CODER_ERROR {
        error_aac = AAC_DEC_PARSE_ERROR;
    }

    error_aac
}

/// `ApplyTools(pAacDecoderChannelInfo[], pSamplingRateInfo, flags, elFlags, channel,
/// common_window)`: apply the PNS and TNS tools to channel `channel` of the element.
///
/// `spec` is the spectrum of the element (channel `i` at `spec[i * 1024..]`), `com` the
/// `pComData` of channel `channel` (PNS inter channel data and random seeds; see
/// [`crate::channel`] for elements whose channels have different `pComData`), `current_seed` the PNS seed
/// of the element (`pnsCurrentSeed` of the static channel info of its first channel, see
/// [`crate::aacdec_pns`]).
pub fn apply_tools(
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
    el_flags: u32,
    channel: i32,
    _common_window: i32,
    spec: &mut [FixpDbl],
    com: &mut CAacDecoderCommonData,
    current_seed: &mut i32,
) {
    let off = (channel as usize * 1024).min(spec.len());
    let ch_spec = &mut spec[off..];
    let p = &mut *p_aac_decoder_channel_info[channel as usize];
    if (flags & (AC_USAC | AC_RSVD50 | AC_MPEGD_RES | AC_RSV603DA)) == 0 {
        cpns_apply(
            &p.data.aac.pns_data,
            &p.ics_info,
            ch_spec,
            &p.spec_scale,
            &p.p_dyn_data.a_scale_factor,
            p_sampling_rate_info,
            p.granule_length,
            channel,
            &com.pns_inter_channel_data,
            current_seed,
            &mut com.pns_random_seed,
        );
    }

    let nbands: u8 = get_scale_factor_bands_transmitted(&p.ics_info);

    ctns_apply(
        &p.p_dyn_data.tns_data,
        &p.ics_info,
        ch_spec,
        p_sampling_rate_info,
        p.granule_length,
        nbands,
        if el_flags & AC_EL_ENHANCED_NOISE != 0 { 1 } else { 0 },
        flags,
    );
}

/// `getWindow2Nr(length, shape)` (static in C)
pub fn get_window2_nr(length: i32, shape: i32) -> i32 {
    let mut nr = 0;

    if shape == 2 {
        /* Low Overlap, 3/4 zeroed */
        nr = (length * 3) >> 2;
    }

    nr
}

/// `get_gain(x, y, n)`: recalculate the gain (`<x, y> / <y, y>`).
pub fn get_gain(x: &[FixpDbl], y: &[FixpDbl], n: i32) -> FixpDbl {
    let mut corr: FixpDbl = 0;
    let mut ener: FixpDbl = 1;
    let n_u = n.max(0) as usize;

    let headroom_x = get_scalefactor(&x[..n_u.min(x.len())]);
    let headroom_y = get_scalefactor(&y[..n_u.min(y.len())]);

    /*Calculate the normalization necessary due to addition*/
    /* Check for power of two /special case */
    let mut width_shift: i32 = f_normz(n);
    /* Get the number of bits necessary minus one, because we need one sign bit only */
    width_shift = 31 - width_shift;

    for i in 0..n_u.min(x.len()).min(y.len()) {
        corr = corr.wrapping_add(
            f_mult_div2(x[i].wrapping_shl(headroom_x as u32), y[i].wrapping_shl(headroom_y as u32))
                >> width_shift,
        );
        ener = ener.wrapping_add(f_pow2_div2(y[i].wrapping_shl(headroom_y as u32)) >> width_shift);
    }

    let exp_corr = (17 - headroom_x) + (17 - headroom_y) + width_shift + 1;
    let exp_ener = ((17 - headroom_y) << 1) + width_shift + 1;

    let mut temp_exp = 0;
    let mut output = f_div_norm_signed(corr, ener, &mut temp_exp);

    let output_exp = (exp_corr - exp_ener) + temp_exp;

    let mut output_shift: i32 = 17 - output_exp;
    output_shift = output_shift.min(31);

    output = scale_value(output, -output_shift);

    output
}

/// `CBlock_FrequencyToTime(pAacDecoderStaticChannelInfo, pAacDecoderChannelInfo, outSamples,
/// frameLen, frameOk, pWorkBuffer1, aacOutDataHeadroom, elFlags, elCh)`: transform the MDCT
/// spectral data into the time domain.
///
/// `p_work_buffer1` is `pWorkBuffer1`, which the library always sets to
/// `pComStaticData->pWorkBufferCore1->mdctOutTemp` (the start of the `CWorkBufferCore1` union):
/// pass the whole union (`&mut work_buffer_core1.buf[..]`). The port also uses it for the
/// `mdctOutTemp` that the non-LPD path reaches through `pComStaticData` (the same memory). `spec`
/// is the spectrum of the channel.
pub fn cblock_frequency_to_time(
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    out_samples: &mut [PcmDec],
    frame_len: i16,
    frame_ok: i32,
    p_work_buffer1: &mut [FixpDbl],
    aac_out_data_headroom: i32,
    el_flags: u32,
    _el_ch: i32,
    spec: &mut [FixpDbl],
) {
    let ps = p_aac_decoder_static_channel_info;
    let p = p_aac_decoder_channel_info;
    let frame_len = frame_len as i32;
    let fl: i32;
    let mut fr: i32;
    let mut tl: i32;
    let mut n_spec: i32;

    /* Determine left slope length (fl), right slope length (fr) and transform length (tl).
    USAC: The slope length may mismatch with the previous frame in case of LPD / FD
    transitions. The adjustment is handled by the imdct implementation. */
    tl = frame_len;
    n_spec = 1;

    match p.ics_info.window_sequence {
        BLOCK_STOP => {
            fl = frame_len >> 3;
            fr = frame_len;
        }
        BLOCK_START => {
            /* or StopStartSequence */
            fl = frame_len;
            fr = frame_len >> 3;
        }
        BLOCK_SHORT => {
            fl = frame_len >> 3;
            fr = frame_len >> 3;
            tl >>= 3;
            n_spec = 8;
        }
        _ => {
            /* default: case BLOCK_LONG: */
            fr = frame_len - get_window2_nr(frame_len, get_window_shape(&p.ics_info) as i32);
            /* New startup needs differentiation between sine shape and low overlap shape. This
            is a special case for the LD-AAC transformation windows, because the slope length
            can be different while using the same window sequence. */
            fl = if ps.i_mdct.prev_tl == 0 { fr } else { frame_len };
        }
    }
    let shape = get_window_shape(&p.ics_info) as i32;
    let _ = &mut fr;
    let _ = &mut n_spec;

    {
        let last_frame_lost = ps.last_lpc_lost as i32;

        if ps.last_core_mode == LPD {
            let mut fac_fb: i32 = 1;
            if el_flags & AC_EL_FULLBANDLPD != 0 {
                fac_fb = 2;
            }

            /* Keep some free space at the beginning of the buffer. To be used for past data */
            let synth_off: usize = if el_flags & AC_EL_LPDSTEREOIDX == 0 {
                ((PIT_MAX_MAX - (1 * L_SUBFR)) * fac_fb) as usize
            } else {
                (PIT_MAX_MAX * fac_fb) as usize
            };

            let fac_length: i32 = if p.ics_info.window_sequence == BLOCK_SHORT {
                frame_len >> 4
            } else {
                frame_len >> 3
            };

            let mut pitch = [0i32; (NB_SUBFR_SUPERFR + SYN_SFD) as usize];
            let mut pit_gain: [FixpDbl; (NB_SUBFR_SUPERFR + SYN_SFD) as usize] =
                [0; (NB_SUBFR_SUPERFR + SYN_SFD) as usize];

            let nb_div: i32 = if el_flags & AC_EL_FULLBANDLPD != 0 { 2 } else { 4 };
            let l_frame: i32 =
                if el_flags & AC_EL_FULLBANDLPD != 0 { frame_len / 2 } else { frame_len };
            let nb_subfr: i32 = l_frame / (nb_div * L_SUBFR); /* number of subframes per division */
            let lpd_sfd: i32 = (nb_div * nb_subfr) >> 1;
            let syn_sfd: i32 = lpd_sfd - BPF_SFD;

            /* FAC case */
            if ps.last_lpd_mode == 0 || ps.last_lpd_mode == 4 {
                let mut fac_buf: [FixpDbl; LFAC as usize] = [0; LFAC as usize];

                if frame_ok == 0
                    || last_frame_lost != 0
                    || p.data.usac.fac_data[0] == FacDataPtr::Null
                {
                    let n = (p.granule_length.max(0) as usize).min(LFAC as usize);
                    fac_buf[..n].fill(0);
                    p.data.usac.fac_data[0] = FacDataPtr::Scratch;
                    p.data.usac.fac_data_e[0] = 0;
                }
                // Note: a FacDataPtr::Scratch left from an earlier call is read as the zeroed
                // fac_buf here (the reference has no defined result; PORTING.md, "Deviations from
                // the reference").

                let mut a_exp: i32 = 0; /* linear prediction coefficients exponent */
                {
                    let a = &mut p.data.usac.lp_coeff[0];
                    for i in 0..M_LP_FILTER_ORDER as usize {
                        a[i] = fx_dbl2fx_sgl(fixp_cos(
                            f_mult(
                                ps.lpc4_lsf[i],
                                fl2fxconst_sgl(
                                    (1 << LSPARG_SCALE) as f64 * std::f64::consts::PI / 6400.0,
                                ),
                            ),
                            LSF_SCALE - LSPARG_SCALE,
                        )) as FixpLpc;
                    }

                    // E_LPC_f_lsp_a_conversion(A, A, &A_exp): C converts in place; the function
                    // reads all of lsp before it writes a, so a copy of the input is equivalent.
                    let lsp = *a;
                    e_lpc_f_lsp_a_conversion(&lsp, a, &mut a_exp);
                }

                let wrs = fdk_get_window_slope(fr, shape);
                let fac_data_e = p.data.usac.fac_data_e[0] as i32;
                let fac_ptr = p.data.usac.fac_data[0];
                // FAC data in an unused part of the spectrum (not expected here: the fac_data
                // element of an FD channel sets fac_data[0] to fac_data0 in
                // CChannelElement_Read(), and a bad frame uses fac_buf): work on a copy and
                // write it back.
                let mut fac_spectral: Vec<FixpDbl> = Vec::new();
                let mut fac_range = 0..0;
                if let FacDataPtr::Spectral(off) = fac_ptr {
                    let off = off.min(spec.len());
                    let end = (off + LFAC as usize).min(spec.len());
                    fac_spectral = spec[off..end].to_vec();
                    fac_range = off..end;
                }
                let p_fac: &mut [FixpDbl] = match fac_ptr {
                    FacDataPtr::FacData0 => &mut p.data.usac.fac_data0,
                    FacDataPtr::Spectral(_) => &mut fac_spectral,
                    FacDataPtr::Null | FacDataPtr::Scratch => &mut fac_buf,
                };
                clpd_fac_acelp2mdct(
                    &mut ps.i_mdct,
                    p_work_buffer1,
                    synth_off,
                    spec,
                    &p.spec_scale,
                    n_spec,
                    p_fac,
                    fac_data_e,
                    fac_length,
                    frame_len,
                    tl,
                    wrs,
                    fr,
                    &p.data.usac.lp_coeff[0],
                    a_exp,
                    &mut ps.acelp,
                    0, /* FAC gain has already been applied. */
                    (last_frame_lost != 0 || frame_ok == 0) as i32,
                    1,
                    ps.last_lpd_mode,
                    0,
                    p.curr_aliasing_symmetry,
                );
                if !fac_range.is_empty() {
                    spec[fac_range].copy_from_slice(&fac_spectral);
                }
            } else {
                let synth_end = p_work_buffer1.len();
                imlt_block(
                    &mut ps.i_mdct,
                    &mut p_work_buffer1[synth_off.min(synth_end)..],
                    spec,
                    &p.spec_scale,
                    n_spec,
                    frame_len,
                    tl,
                    fdk_get_window_slope(fl, shape),
                    fl,
                    fdk_get_window_slope(fr, shape),
                    fr,
                    0,
                    if p.curr_aliasing_symmetry != 0 { MLT_FLAG_CURR_ALIAS_SYMMETRY } else { 0 },
                );
            }

            /* The "if" clause is entered both for fullbandLpd mono and non-fullbandLpd*. The
            "else"-> just for fullbandLpd stereo*/
            if el_flags & AC_EL_LPDSTEREOIDX == 0 {
                let ss = syn_sfd.clamp(0, SYN_SFD) as usize;
                pitch[..ss].copy_from_slice(&ps.old_t_pf[..ss]);
                pit_gain[..ss].copy_from_slice(&ps.old_gain_pf[..ss]);

                for i in syn_sfd..lpd_sfd + 3 {
                    if let (Some(pt), Some(pg)) =
                        (pitch.get_mut(i as usize), pit_gain.get_mut(i as usize))
                    {
                        *pt = L_SUBFR;
                        *pg = 0;
                    }
                }

                if ps.last_lpd_mode == 0 && syn_sfd >= 1 {
                    let s = syn_sfd as usize;
                    pitch[s] = pitch[s - 1];
                    pit_gain[s] = pit_gain[s - 1];
                    if is_long_block(&p.ics_info) != 0 {
                        pitch[s + 1] = pitch[s];
                        pit_gain[s + 1] = pit_gain[s];
                    }
                }

                /* Copy old data to the beginning of the buffer */
                {
                    // (PIT_MAX_MAX - L_SUBFR) * fac_FB values; the port copies at most
                    // old_synth (larger counts have no defined reference result).
                    let n = (((PIT_MAX_MAX - (1 * L_SUBFR)) * fac_fb) as usize)
                        .min(ps.old_synth.len())
                        .min(p_work_buffer1.len());
                    p_work_buffer1[..n].copy_from_slice(&ps.old_synth[..n]);
                }

                let p2_synth: usize = (PIT_MAX_MAX * fac_fb) as usize;

                /* recalculate pitch gain to allow postfilering on FAC area */
                for i in 0..(syn_sfd + 2).max(0) as usize {
                    if i >= pitch.len() {
                        break;
                    }
                    let t = pitch[i];
                    let mut gain = pit_gain[i];

                    if gain > 0 {
                        let xo = p2_synth + i * (L_SUBFR * fac_fb) as usize;
                        let yo = xo as isize - (fac_fb * t) as isize;
                        let n = (L_SUBFR * fac_fb) as usize;
                        if yo >= 0 && xo + n <= p_work_buffer1.len() {
                            let yo = yo as usize;
                            if yo + n <= p_work_buffer1.len() {
                                gain = get_gain(
                                    &p_work_buffer1[xo..xo + n],
                                    &p_work_buffer1[yo..yo + n],
                                    n as i32,
                                );
                            }
                        }
                        pit_gain[i] = gain;
                    }
                }

                bass_pf_1sf_delay(
                    p_work_buffer1,
                    p2_synth,
                    &pitch,
                    &pit_gain,
                    frame_len,
                    (lpd_sfd + 2) * L_SUBFR + BPF_SFD * L_SUBFR,
                    frame_len - (lpd_sfd + 4) * L_SUBFR,
                    out_samples,
                    aac_out_data_headroom,
                    &mut ps.mem_bpf,
                );
            }
        } else {
            /* last_core_mode was not LPD */
            let n = (frame_len.max(0) as usize).min(p_work_buffer1.len().min(1024));
            {
                let tmp_len = p_work_buffer1.len().min(1024);
                let tmp = &mut p_work_buffer1[..tmp_len];
                imlt_block(
                    &mut ps.i_mdct,
                    tmp,
                    spec,
                    &p.spec_scale,
                    n_spec,
                    frame_len,
                    tl,
                    fdk_get_window_slope(fl, shape),
                    fl,
                    fdk_get_window_slope(fr, shape),
                    fr,
                    0,
                    if p.curr_aliasing_symmetry != 0 { MLT_FLAG_CURR_ALIAS_SYMMETRY } else { 0 },
                );
            }

            let n = n.min(out_samples.len());
            scale_values_saturate_to(
                &mut out_samples[..n],
                &p_work_buffer1[..n],
                MDCT_OUT_HEADROOM - aac_out_data_headroom,
            );
        }
    }

    ps.last_core_mode = if p.ics_info.window_sequence == BLOCK_SHORT { FD_SHORT } else { FD_LONG };
    ps.last_lpd_mode = 255;
    let _ = BLOCK_LONG;
}

/// `CBlock_FrequencyToTimeLowDelay(pAacDecoderStaticChannelInfo, pAacDecoderChannelInfo,
/// outSamples, frameLen)`: transform the double lapped MDCT (AAC-ELD) spectral data into the
/// time domain. `spec` is the spectrum of the channel; `pOverlapBuffer` is the overlap buffer of
/// `IMdct` ([`fdk_aac_rs_fdk::mdct::MdctT::overlap`]).
pub fn cblock_frequency_to_time_low_delay(
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    out_samples: &mut [PcmDec],
    frame_len: i16,
    spec: &mut [FixpDbl],
) {
    inv_mdct_transform_low_delay_fdk(
        spec,
        p_aac_decoder_channel_info.spec_scale[0] as i32,
        out_samples,
        &mut p_aac_decoder_static_channel_info.i_mdct.overlap,
        frame_len as i32,
    );
}
