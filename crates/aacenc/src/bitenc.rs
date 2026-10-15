// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/bitenc.h, libAACenc/src/bitenc.cpp.

//! Port of `bitenc.h`: the bitstream writer of the AAC elements.
//!
//! Every writer works in two modes, as in C: with a transport encoder handle it writes into the
//! transport encoder's bitstream, without one (`hTpEnc == NULL`, `None`) it only counts bits.
//!
//! # Memory model
//!
//! `QC_OUT_ELEMENT::qcOutChannel[]`, `PSY_OUT_ELEMENT::psyOutChannel[]` point to the channels of
//! an element; the port passes the element's channels as slices (see
//! [`qc_data`](crate::qc_data) and [`interface`](crate::interface)). The extension payloads keep
//! the raw pointers of C (`QC_OUT_EXTENSION::pPayload`); they are only dereferenced when a
//! bitstream is written, which makes the writing functions `unsafe`.

use crate::aacenc::*;
use crate::aacenc_tns::TnsInfo;
use crate::bit_cnt::{
    fdk_aac_enc_code_scalefactor_delta, fdk_aac_enc_code_values, CODE_BOOK_IS_IN_PHASE_NO,
    CODE_BOOK_IS_OUT_OF_PHASE_NO, CODE_BOOK_PNS_NO, CODE_BOOK_ZERO_NO,
};
use crate::dyn_bits::{
    SectionData, PNS_PCM_BITS, SECT_BITS_LONG, SECT_BITS_SHORT, SECT_ESC_VAL_LONG,
    SECT_ESC_VAL_SHORT,
};
use crate::interface::*;
use crate::psy_const::*;
use crate::qc_data::*;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_byte_align, fdk_get_valid_bits, fdk_write_bits, FdkBitStream};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::tools_rom::{get_bitstream_element_list, RbdIdT};
use fdk_aac_rs_tpenc::tpenc_lib::{
    transport_enc_crc_end_reg, transport_enc_crc_start_reg, transport_enc_end_access_unit,
    TransportEnc,
};

/// `_MAX_CONST`
pub const MAX_ENCODER_CHANNELS: i32 = 9;
/// `MAX_BLOCK_TYPES`
pub const MAX_BLOCK_TYPES: i32 = 4;
/// `MAX_AAC_LAYERS`
pub const MAX_AAC_LAYERS: i32 = 9;
/// `MAX_LAYERS`: only one core layer if present
pub const MAX_LAYERS: i32 = MAX_AAC_LAYERS;
/// `FIRST_LAY`: default layer number for AAC nonscalable
pub const FIRST_LAY: i32 = 1;

/// `BUFFER_MX_HUFFCB_SIZE`: our FDK_bitbuffer needs size of power 2
pub const BUFFER_MX_HUFFCB_SIZE: i32 = 32 * 4;

/// `EL_ID_BITS`
pub const EL_ID_BITS: i32 = 3;

/// `globalGainOffset`
const GLOBAL_GAIN_OFFSET: i32 = 100;
/// `icsReservedBit`
const ICS_RESERVED_BIT: i32 = 0;
/// `noiseOffset`
const NOISE_OFFSET: i32 = 90;

/// The transport encoder's bitstream, or `None` (`NULL`) in counting mode.
#[inline]
fn bs<'a>(h: &'a mut Option<&mut TransportEnc>) -> Option<&'a mut FdkBitStream> {
    h.as_deref_mut().map(|t| &mut t.bit_stream)
}

/// `FDKwriteBits` on an optional bitstream that is known to be present.
#[inline]
fn wr(h: &mut Option<&mut FdkBitStream>, value: i32, n: i32) {
    if let Some(b) = h.as_deref_mut() {
        fdk_write_bits(b, value as u32, n as u32);
    }
}

/// `FDKaacEnc_encodeSpectralData(sfbOffset, sectionData, quantSpectrum, hBitStream)`: encode
/// spectral data; returns the number of written bits.
pub fn fdk_aac_enc_encode_spectral_data(
    sfb_offset: &[i32],
    section_data: &SectionData,
    quant_spectrum: &[i16],
    h_bit_stream: &mut FdkBitStream,
) -> i32 {
    let dbg_val = fdk_get_valid_bits(h_bit_stream) as i32;

    for i in 0..section_data.no_of_sections.max(0) as usize {
        let hs = &section_data.huffsection[i];
        if hs.code_book != CODE_BOOK_PNS_NO {
            // huffencode spectral data for this huffsection
            let tmp = hs.sfb_start + hs.sfb_cnt;
            for sfb in hs.sfb_start..tmp {
                let s = sfb as usize;
                fdk_aac_enc_code_values(
                    &quant_spectrum[sfb_offset[s] as usize..],
                    sfb_offset[s + 1] - sfb_offset[s],
                    hs.code_book,
                    h_bit_stream,
                );
            }
        }
    }
    (fdk_get_valid_bits(h_bit_stream) as i32).wrapping_sub(dbg_val)
}

/// `FDKaacEnc_encodeGlobalGain(globalGain, scalefac, hBitStream, mdctScale)`: encodes the
/// global gain (common scale factor); returns the number of static bits.
pub fn fdk_aac_enc_encode_global_gain(
    global_gain: i32,
    scalefac: i32,
    mut h_bit_stream: Option<&mut FdkBitStream>,
    mdct_scale: i32,
) -> i32 {
    wr(
        &mut h_bit_stream,
        global_gain
            .wrapping_sub(scalefac)
            .wrapping_add(GLOBAL_GAIN_OFFSET)
            .wrapping_sub(4 * (LOG_NORM_PCM - mdct_scale)),
        8,
    );
    8
}

/// `FDKaacEnc_encodeIcsInfo(blockType, windowShape, groupingMask, maxSfbPerGroup, hBitStream,
/// syntaxFlags)`: encodes the ics_info; returns the number of static bits.
pub fn fdk_aac_enc_encode_ics_info(
    block_type: i32,
    window_shape: i32,
    grouping_mask: i32,
    max_sfb_per_group: i32,
    mut h_bit_stream: Option<&mut FdkBitStream>,
    syntax_flags: u32,
) -> i32 {
    let stat_bits = if block_type == SHORT_WINDOW {
        8 + TRANS_FAC - 1
    } else if syntax_flags & AC_ELD != 0 {
        6
    } else if syntax_flags & AC_SCALABLE == 0 {
        11
    } else {
        10
    };

    if h_bit_stream.is_some() {
        if syntax_flags & AC_ELD == 0 {
            wr(&mut h_bit_stream, ICS_RESERVED_BIT, 1);
            wr(&mut h_bit_stream, block_type, 2);
            wr(
                &mut h_bit_stream,
                if window_shape == LOL_WINDOW { KBD_WINDOW } else { window_shape },
                1,
            );
        }

        match block_type {
            LONG_WINDOW | START_WINDOW | STOP_WINDOW => {
                wr(&mut h_bit_stream, max_sfb_per_group, 6);

                if syntax_flags & (AC_SCALABLE | AC_ELD) == 0 {
                    // If not scalable syntax then ... No predictor data present
                    wr(&mut h_bit_stream, 0, 1);
                }
            }
            SHORT_WINDOW => {
                wr(&mut h_bit_stream, max_sfb_per_group, 4);
                // Write grouping bits
                wr(&mut h_bit_stream, grouping_mask, TRANS_FAC - 1);
            }
            _ => {}
        }
    }

    stat_bits
}

/// `FDKaacEnc_encodeSectionData(sectionData, hBitStream, useVCB11)`: encode section data
/// (common Huffman codebooks for adjacent SFB's).
pub fn fdk_aac_enc_encode_section_data(
    section_data: Option<&SectionData>,
    h_bit_stream: Option<&mut FdkBitStream>,
    _use_vcb11: u32,
) -> i32 {
    if let Some(h) = h_bit_stream {
        // C dereferences sectionData unchecked here; the writer always passes it.
        let Some(section_data) = section_data else { return 0 };
        let mut sect_escape_val = 0;
        let mut sect_len_bits = 0;
        let dbg_val = fdk_get_valid_bits(h) as i32;
        let sect_cb_bits = 4;

        match section_data.block_type {
            LONG_WINDOW | START_WINDOW | STOP_WINDOW => {
                sect_escape_val = SECT_ESC_VAL_LONG;
                sect_len_bits = SECT_BITS_LONG;
            }
            SHORT_WINDOW => {
                sect_escape_val = SECT_ESC_VAL_SHORT;
                sect_len_bits = SECT_BITS_SHORT;
            }
            _ => {}
        }

        for i in 0..section_data.no_of_sections.max(0) as usize {
            let code_book = section_data.huffsection[i].code_book;

            fdk_write_bits(h, code_book as u32, sect_cb_bits);

            let mut sect_len = section_data.huffsection[i].sfb_cnt;

            // (an escape value of 0, an unknown block type, is not defined by the reference)
            while sect_escape_val > 0 && sect_len >= sect_escape_val {
                fdk_write_bits(h, sect_escape_val as u32, sect_len_bits as u32);
                sect_len -= sect_escape_val;
            }
            fdk_write_bits(h, sect_len as u32, sect_len_bits as u32);
        }
        return (fdk_get_valid_bits(h) as i32).wrapping_sub(dbg_val);
    }
    0
}

/// `FDKaacEnc_encodeScaleFactorData(maxValueInSfb, sectionData, scalefac, hBitStream, noiseNrg,
/// isScale, globalGain)`: encode DPCM coded scale factors.
pub fn fdk_aac_enc_encode_scale_factor_data(
    max_value_in_sfb: Option<&[u32]>,
    section_data: Option<&SectionData>,
    scalefac: Option<&[i32]>,
    h_bit_stream: Option<&mut FdkBitStream>,
    noise_nrg: &[i32],
    is_scale: &[i32],
    global_gain: i32,
) -> i32 {
    if let Some(h) = h_bit_stream {
        // C dereferences these unchecked here; the writer always passes them.
        let (Some(max_value_in_sfb), Some(section_data), Some(scalefac)) =
            (max_value_in_sfb, section_data, scalefac)
        else {
            return 0;
        };
        let mut noise_pcm_flag = TRUE;

        let dbg_val = fdk_get_valid_bits(h) as i32;

        let first = scalefac.get(section_data.first_scf as usize).copied().unwrap_or(0);
        let mut last_val_scf = first;
        let mut last_val_pns = global_gain
            .wrapping_sub(first)
            .wrapping_add(GLOBAL_GAIN_OFFSET)
            .wrapping_sub(4 * LOG_NORM_PCM)
            .wrapping_sub(NOISE_OFFSET);
        let mut last_val_is = 0;

        for i in 0..section_data.no_of_sections.max(0) as usize {
            let hs = &section_data.huffsection[i];
            if hs.code_book != CODE_BOOK_ZERO_NO {
                if (hs.code_book == CODE_BOOK_IS_OUT_OF_PHASE_NO)
                    || (hs.code_book == CODE_BOOK_IS_IN_PHASE_NO)
                {
                    let sfb_start = hs.sfb_start;
                    let tmp = sfb_start + hs.sfb_cnt;
                    for j in sfb_start..tmp {
                        let delta_is = is_scale[j as usize].wrapping_sub(last_val_is);
                        last_val_is = is_scale[j as usize];
                        if fdk_aac_enc_code_scalefactor_delta(delta_is, h) != 0 {
                            return 1;
                        }
                    } // sfb
                } else if hs.code_book == CODE_BOOK_PNS_NO {
                    let sfb_start = hs.sfb_start;
                    let tmp = sfb_start + hs.sfb_cnt;
                    for j in sfb_start..tmp {
                        let delta_pns = noise_nrg[j as usize].wrapping_sub(last_val_pns);
                        last_val_pns = noise_nrg[j as usize];

                        if noise_pcm_flag != 0 {
                            fdk_write_bits(
                                h,
                                delta_pns.wrapping_add(1 << (PNS_PCM_BITS - 1)) as u32,
                                PNS_PCM_BITS as u32,
                            );
                            noise_pcm_flag = FALSE;
                        } else if fdk_aac_enc_code_scalefactor_delta(delta_pns, h) != 0 {
                            return 1;
                        }
                    } // sfb
                } else {
                    let tmp = hs.sfb_start + hs.sfb_cnt;
                    for j in hs.sfb_start..tmp {
                        // check if we can repeat the last value to save bits
                        let delta_scf = if max_value_in_sfb[j as usize] == 0 {
                            0
                        } else {
                            let d = scalefac[j as usize].wrapping_sub(last_val_scf).wrapping_neg();
                            last_val_scf = scalefac[j as usize];
                            d
                        };
                        if fdk_aac_enc_code_scalefactor_delta(delta_scf, h) != 0 {
                            return 1;
                        }
                    } // sfb
                } // code scalefactor
            } // sectionData->huffsection[i].codeBook != CODE_BOOK_ZERO_NO
        } // section loop

        return (fdk_get_valid_bits(h) as i32).wrapping_sub(dbg_val);
    } // if (hBitStream != NULL)

    0
}

/// `FDKaacEnc_encodeMSInfo(sfbCnt, grpSfb, maxSfb, msDigest, jsFlags, hBitStream)`: encodes
/// MS-Stereo Info; returns the number of static bits.
pub fn fdk_aac_enc_encode_ms_info(
    sfb_cnt: i32,
    grp_sfb: i32,
    max_sfb: i32,
    ms_digest: i32,
    js_flags: &[i32],
    h_bit_stream: Option<&mut FdkBitStream>,
) -> i32 {
    let mut ms_bits = 0;

    if let Some(h) = h_bit_stream {
        match ms_digest {
            MS_NONE => {
                fdk_write_bits(h, SI_MS_MASK_NONE as u32, 2);
                ms_bits += 2;
            }
            MS_ALL => {
                fdk_write_bits(h, SI_MS_MASK_ALL as u32, 2);
                ms_bits += 2;
            }
            MS_SOME => {
                fdk_write_bits(h, SI_MS_MASK_SOME as u32, 2);
                ms_bits += 2;
                let mut sfb_off = 0;
                while sfb_off < sfb_cnt {
                    for sfb in 0..max_sfb {
                        if js_flags[(sfb_off + sfb) as usize] & MS_ON != 0 {
                            fdk_write_bits(h, 1, 1);
                        } else {
                            fdk_write_bits(h, 0, 1);
                        }
                        ms_bits += 1;
                    }
                    if grp_sfb <= 0 {
                        // (not defined by the reference)
                        break;
                    }
                    sfb_off += grp_sfb;
                }
            }
            _ => {}
        }
    } else {
        ms_bits += 2;
        if ms_digest == MS_SOME {
            let mut sfb_off = 0;
            while sfb_off < sfb_cnt {
                ms_bits += max_sfb.max(0);
                if grp_sfb <= 0 {
                    // (not defined by the reference)
                    break;
                }
                sfb_off += grp_sfb;
            }
        }
    }
    ms_bits
}

/// `FDKaacEnc_encodeTnsDataPresent(tnsInfo, blockType, hBitStream)`: returns the number of
/// static bits.
pub fn fdk_aac_enc_encode_tns_data_present(
    tns_info: Option<&TnsInfo>,
    block_type: i32,
    h_bit_stream: Option<&mut FdkBitStream>,
) -> i32 {
    if let (Some(h), Some(tns_info)) = (h_bit_stream, tns_info) {
        let mut tns_present = 0;
        let num_of_windows = if block_type == SHORT_WINDOW { TRANS_FAC } else { 1 };

        for i in 0..num_of_windows as usize {
            if tns_info.num_of_filters[i] != 0 {
                tns_present = 1;
                break;
            }
        }

        if tns_present == 0 {
            fdk_write_bits(h, 0, 1);
        } else {
            fdk_write_bits(h, 1, 1);
        }
    }
    1
}

/// The number of bits of the TNS coefficients of filter `j` of window `i`.
fn tns_coef_bits(tns_info: &TnsInfo, i: usize, j: usize) -> i32 {
    let order = tns_info.order[i][j];
    if tns_info.coef_res[i] == 4 {
        let mut coef_bits = 3;
        for k in 0..order.max(0) as usize {
            if tns_info.coef[i][j][k] > 3 || tns_info.coef[i][j][k] < -4 {
                coef_bits = 4;
                break;
            }
        }
        coef_bits
    } else {
        let mut coef_bits = 2;
        for k in 0..order.max(0) as usize {
            if tns_info.coef[i][j][k] > 1 || tns_info.coef[i][j][k] < -2 {
                coef_bits = 3;
                break;
            }
        }
        coef_bits
    }
}

/// `FDKaacEnc_encodeTnsData(tnsInfo, blockType, hBitStream)`: encode TNS data (filter order,
/// coeffs, ..); returns the number of static bits.
pub fn fdk_aac_enc_encode_tns_data(
    tns_info: Option<&TnsInfo>,
    block_type: i32,
    h_bit_stream: Option<&mut FdkBitStream>,
) -> i32 {
    let mut tns_bits = 0;

    let Some(tns_info) = tns_info else { return 0 };
    let mut tns_present = 0;
    let num_of_windows = if block_type == SHORT_WINDOW { TRANS_FAC } else { 1 } as usize;
    let short = block_type == SHORT_WINDOW;

    for i in 0..num_of_windows {
        if tns_info.num_of_filters[i] != 0 {
            tns_present = 1;
        }
    }

    // C indexes the filter arrays without bounds; the TNS encoder writes at most 1 (short) and
    // 2 (long) filters of order <= 12 (FDK_ASSERT).
    if let Some(h) = h_bit_stream {
        if tns_present == 1 {
            // there is data to be written
            for i in 0..num_of_windows {
                let nf = tns_info.num_of_filters[i];
                fdk_write_bits(h, nf as u32, if short { 1 } else { 2 });
                tns_bits += if short { 1 } else { 2 };
                if nf != 0 {
                    fdk_write_bits(h, (tns_info.coef_res[i] == 4) as u32, 1);
                    tns_bits += 1;
                }
                for j in 0..nf.max(0) as usize {
                    fdk_write_bits(h, tns_info.length[i][j] as u32, if short { 4 } else { 6 });
                    tns_bits += if short { 4 } else { 6 };
                    fdk_assert!(tns_info.order[i][j] <= 12);
                    fdk_write_bits(h, tns_info.order[i][j] as u32, if short { 3 } else { 5 });
                    tns_bits += if short { 3 } else { 5 };
                    if tns_info.order[i][j] != 0 {
                        fdk_write_bits(h, tns_info.direction[i][j] as u32, 1);
                        tns_bits += 1; // direction
                        let coef_bits = tns_coef_bits(tns_info, i, j);
                        // coef_compres
                        fdk_write_bits(
                            h,
                            (coef_bits - tns_info.coef_res[i]).wrapping_neg() as u32,
                            1,
                        );
                        tns_bits += 1; // coef_compression
                        const RMASK: [i32; 5] = [0, 1, 3, 7, 15];
                        for k in 0..tns_info.order[i][j].max(0) as usize {
                            fdk_write_bits(
                                h,
                                (tns_info.coef[i][j][k] & RMASK[coef_bits as usize]) as u32,
                                coef_bits as u32,
                            );
                            tns_bits += coef_bits;
                        }
                    }
                }
            }
        }
    } else if tns_present != 0 {
        for i in 0..num_of_windows {
            tns_bits += if short { 1 } else { 2 };
            if tns_info.num_of_filters[i] != 0 {
                tns_bits += 1;
                for j in 0..tns_info.num_of_filters[i].max(0) as usize {
                    tns_bits += if short { 4 } else { 6 };
                    tns_bits += if short { 3 } else { 5 };
                    if tns_info.order[i][j] != 0 {
                        tns_bits += 1; // direction
                        tns_bits += 1; // coef_compression
                        let coef_bits = tns_coef_bits(tns_info, i, j);
                        tns_bits += coef_bits * tns_info.order[i][j].max(0);
                    }
                }
            }
        }
    }

    tns_bits
}

/// `FDKaacEnc_encodeGainControlData(hBitStream)`: unsupported
pub fn fdk_aac_enc_encode_gain_control_data(mut h_bit_stream: Option<&mut FdkBitStream>) -> i32 {
    wr(&mut h_bit_stream, 0, 1);
    1
}

/// `FDKaacEnc_encodePulseData(hBitStream)`: not supported yet (dummy)
pub fn fdk_aac_enc_encode_pulse_data(mut h_bit_stream: Option<&mut FdkBitStream>) -> i32 {
    wr(&mut h_bit_stream, 0, 1);
    1
}

/// `EXT_TYPE_BITS`
const EXT_TYPE_BITS: i32 = 4;
/// `DATA_EL_VERSION_BITS`
const DATA_EL_VERSION_BITS: i32 = 4;
/// `FILL_NIBBLE_BITS`
const FILL_NIBBLE_BITS: i32 = 4;

/// Reads byte `i` of an extension payload.
///
/// # Safety
/// `p` must be valid for reads of `i + 1` bytes.
#[inline]
unsafe fn payload_byte(p: *const u8, i: usize) -> u8 {
    if p.is_null() {
        // Guarded: reads 0 (the encoder checks the pointers of payloads with data).
        return 0;
    }
    // SAFETY: the caller's guarantee.
    unsafe { *p.add(i) }
}

/// `FDKaacEnc_writeExtensionPayload(hBitStream, extPayloadType, extPayloadData,
/// extPayloadBits)`: write extension payload to bitstream; returns the number of written bits.
///
/// # Safety
/// With a bitstream, `ext_payload_data` must be valid for reads of the payload bytes (see
/// [`fdk_aac_enc_write_extension_data`]).
pub unsafe fn fdk_aac_enc_write_extension_payload(
    mut h_bit_stream: Option<&mut FdkBitStream>,
    ext_payload_type: ExtPayloadType,
    ext_payload_data: *const u8,
    ext_payload_bits: i32,
) -> i32 {
    let mut ext_bits_used = 0;
    let mut data = 0usize; // extPayloadData++ offset

    if ext_payload_bits >= EXT_TYPE_BITS {
        let mut fill_byte: u8 = 0x00; // for EXT_FIL and EXT_FILL_DATA

        wr(&mut h_bit_stream, ext_payload_type, EXT_TYPE_BITS);
        ext_bits_used += EXT_TYPE_BITS;

        let mut sbr_like = false;
        match ext_payload_type {
            // case EXT_SAC_DATA:
            EXT_LDSAC_DATA => {
                if let Some(h) = h_bit_stream.as_deref_mut() {
                    // SAFETY: the caller's guarantee.
                    let b = unsafe { payload_byte(ext_payload_data, data) };
                    data += 1;
                    fdk_write_bits(h, b as u32, 4); // nibble
                }
                ext_bits_used += 4;
                sbr_like = true; // FDK_FALLTHROUGH
            }
            EXT_DYNAMIC_RANGE | EXT_SBR_DATA | EXT_SBR_DATA_CRC => {
                sbr_like = true;
            }
            EXT_DATA_ELEMENT => {
                let mut data_element_length = ext_payload_bits.wrapping_add(7) >> 3;
                let cnt = data_element_length;
                let mut loop_counter = 1;

                while data_element_length >= 255 {
                    loop_counter += 1;
                    data_element_length -= 255;
                }

                if let Some(h) = h_bit_stream.as_deref_mut() {
                    // data_element_version = ANC_DATA
                    fdk_write_bits(h, 0x00, DATA_EL_VERSION_BITS as u32);

                    for _ in 1..loop_counter {
                        fdk_write_bits(h, 255, 8);
                    }
                    fdk_write_bits(h, data_element_length as u32, 8);

                    for i in 0..cnt.max(0) as usize {
                        // SAFETY: the caller's guarantee.
                        let b = unsafe { payload_byte(ext_payload_data, i) };
                        fdk_write_bits(h, b as u32, 8);
                    }
                }
                ext_bits_used += DATA_EL_VERSION_BITS + (loop_counter * 8) + (cnt * 8);
            }
            _ => {
                // EXT_FILL_DATA (fillByte 0xA5, FDK_FALLTHROUGH), EXT_FIL, default
                if ext_payload_type == EXT_FILL_DATA {
                    fill_byte = 0xA5;
                }
                if let Some(h) = h_bit_stream.as_deref_mut() {
                    let mut write_bits = ext_payload_bits;
                    fdk_write_bits(h, 0x00, FILL_NIBBLE_BITS as u32);
                    write_bits -= 8; // acount for the extension type and the fill nibble
                    while write_bits >= 8 {
                        fdk_write_bits(h, fill_byte as u32, 8);
                        write_bits -= 8;
                    }
                }
                ext_bits_used += FILL_NIBBLE_BITS + (ext_payload_bits & !0x7) - 8;
            }
        }

        if sbr_like {
            if let Some(h) = h_bit_stream {
                let mut write_bits = ext_payload_bits;
                while write_bits >= 8 {
                    // SAFETY: the caller's guarantee.
                    let b = unsafe { payload_byte(ext_payload_data, data) };
                    data += 1;
                    fdk_write_bits(h, b as u32, 8);
                    write_bits -= 8;
                }
                if write_bits > 0 {
                    // SAFETY: the caller's guarantee.
                    let b = unsafe { payload_byte(ext_payload_data, data) };
                    fdk_write_bits(h, (b >> (8 - write_bits)) as u32, write_bits as u32);
                }
            }
            ext_bits_used += ext_payload_bits;
        }
    }

    ext_bits_used
}

/// `DATA_BYTE_ALIGN_FLAG`
const DATA_BYTE_ALIGN_FLAG: i32 = 0;
/// `EL_INSTANCE_TAG_BITS`
const EL_INSTANCE_TAG_BITS: i32 = 4;
/// `DATA_BYTE_ALIGN_FLAG_BITS`
const DATA_BYTE_ALIGN_FLAG_BITS: i32 = 1;
/// `DATA_LEN_COUNT_BITS`
const DATA_LEN_COUNT_BITS: i32 = 8;
/// `DATA_LEN_ESC_COUNT_BITS`
const DATA_LEN_ESC_COUNT_BITS: i32 = 8;
/// `MAX_DATA_ALIGN_BITS`
const MAX_DATA_ALIGN_BITS: i32 = 7;
/// `MAX_DSE_DATA_BYTES`
const MAX_DSE_DATA_BYTES: i32 = 510;

/// `FDKaacEnc_writeDataStreamElement(hTpEnc, elementInstanceTag, dataPayloadBytes, dataBuffer,
/// alignAnchor)`: write data stream elements like ancillary data ...; returns the amount of
/// used bits.
///
/// Every DSE of a payload longer than 510 bytes repeats the bytes from the start of
/// `data_buffer` (C does not advance it), as in C.
///
/// # Safety
/// With a transport encoder, `data_buffer` must be valid for reads of
/// `min(data_payload_bytes, 510)` bytes.
pub unsafe fn fdk_aac_enc_write_data_stream_element(
    mut h_tp_enc: Option<&mut TransportEnc>,
    element_instance_tag: i32,
    mut data_payload_bytes: i32,
    data_buffer: *const u8,
    align_anchor: u32,
) -> i32 {
    let mut dse_bits_used = 0;

    while data_payload_bytes > 0 {
        let mut esc_count = -1;

        dse_bits_used +=
            EL_ID_BITS + EL_INSTANCE_TAG_BITS + DATA_BYTE_ALIGN_FLAG_BITS + DATA_LEN_COUNT_BITS;

        if DATA_BYTE_ALIGN_FLAG != 0 {
            dse_bits_used += MAX_DATA_ALIGN_BITS;
        }

        let cnt = MAX_DSE_DATA_BYTES.min(data_payload_bytes);
        if cnt >= 255 {
            esc_count = cnt - 255;
            dse_bits_used += DATA_LEN_ESC_COUNT_BITS;
        }

        data_payload_bytes -= cnt;
        dse_bits_used += cnt * 8;

        if let Some(tp) = h_tp_enc.as_deref_mut() {
            fdk_write_bits(&mut tp.bit_stream, ID_DSE as u32, EL_ID_BITS as u32);

            let crc_reg = transport_enc_crc_start_reg(tp, 0);

            let h = &mut tp.bit_stream;
            fdk_write_bits(h, element_instance_tag as u32, EL_INSTANCE_TAG_BITS as u32);
            fdk_write_bits(h, DATA_BYTE_ALIGN_FLAG as u32, DATA_BYTE_ALIGN_FLAG_BITS as u32);

            // write length field(s)
            if esc_count >= 0 {
                fdk_write_bits(h, 255, DATA_LEN_COUNT_BITS as u32);
                fdk_write_bits(h, esc_count as u32, DATA_LEN_ESC_COUNT_BITS as u32);
            } else {
                fdk_write_bits(h, cnt as u32, DATA_LEN_COUNT_BITS as u32);
            }

            if DATA_BYTE_ALIGN_FLAG != 0 {
                let tmp = fdk_get_valid_bits(h) as i32;
                fdk_byte_align(h, align_anchor);
                // count actual bits
                dse_bits_used += fdk_get_valid_bits(h) as i32 - tmp - MAX_DATA_ALIGN_BITS;
            }

            // write payload
            for i in 0..cnt as usize {
                // SAFETY: the caller's guarantee.
                let b = unsafe { payload_byte(data_buffer, i) };
                fdk_write_bits(h, b as u32, 8);
            }
            transport_enc_crc_end_reg(tp, crc_reg);
        }
    }

    dse_bits_used
}

/// `FILL_EL_COUNT_BITS`
const FILL_EL_COUNT_BITS: i32 = 4;
/// `FILL_EL_ESC_COUNT_BITS`
const FILL_EL_ESC_COUNT_BITS: i32 = 8;
/// `MAX_FILL_DATA_BYTES`
const MAX_FILL_DATA_BYTES: i32 = 269;

/// `FDKaacEnc_writeExtensionData(hTpEnc, pExtension, elInstanceTag, alignAnchor, syntaxFlags,
/// aot, epConfig)`: write extension payload to bitstream (or only count its bits without a
/// transport encoder); returns the number of written bits.
///
/// # Safety
/// With a transport encoder, `p_extension.p_payload` must be valid for reads of the payload
/// bytes: `(n_payload_bits + 7) / 8` bytes and the further bytes the writer reads (one byte for
/// `EXT_LDSAC_DATA`, and at most 510 bytes of a DSE). Without one, nothing is dereferenced.
pub unsafe fn fdk_aac_enc_write_extension_data(
    mut h_tp_enc: Option<&mut TransportEnc>,
    p_extension: &QcOutExtension,
    el_instance_tag: i32, // for DSE only
    align_anchor: u32,    // for DSE only
    syntax_flags: u32,
    _aot: AudioObjectType,
    _ep_config: i8,
) -> i32 {
    let mut payload_bits = p_extension.n_payload_bits;
    let mut ext_bits_used = 0;

    if syntax_flags & (AC_SCALABLE | AC_ER) != 0 {
        if (syntax_flags & AC_ELD != 0)
            && ((p_extension.type_ == EXT_SBR_DATA) || (p_extension.type_ == EXT_SBR_DATA_CRC))
        {
            if let Some(h) = bs(&mut h_tp_enc) {
                let mut write_bits = payload_bits;
                let ext_payload_data = p_extension.p_payload;
                let mut i = 0usize;
                while write_bits >= 8 {
                    // SAFETY: the caller's guarantee.
                    let b = unsafe { payload_byte(ext_payload_data, i) };
                    fdk_write_bits(h, b as u32, 8);
                    write_bits -= 8;
                    i += 1;
                }
                if write_bits > 0 {
                    // SAFETY: the caller's guarantee.
                    let b = unsafe { payload_byte(ext_payload_data, i) };
                    fdk_write_bits(h, (b >> (8 - write_bits)) as u32, write_bits as u32);
                }
            }
            ext_bits_used += payload_bits;
        } else {
            // ER or scalable syntax -> write extension en bloc
            // SAFETY: the caller's guarantee.
            ext_bits_used += unsafe {
                fdk_aac_enc_write_extension_payload(
                    bs(&mut h_tp_enc),
                    p_extension.type_,
                    p_extension.p_payload,
                    payload_bits,
                )
            };
        }
    } else {
        // We have normal GA bitstream payload (AOT 2,5,29) so pack the data into a fill
        // elements or DSEs

        if p_extension.type_ == EXT_DATA_ELEMENT {
            // SAFETY: the caller's guarantee.
            ext_bits_used += unsafe {
                fdk_aac_enc_write_data_stream_element(
                    h_tp_enc,
                    el_instance_tag,
                    p_extension.n_payload_bits >> 3,
                    p_extension.p_payload,
                    align_anchor,
                )
            };
        } else {
            while payload_bits >= (EL_ID_BITS + FILL_EL_COUNT_BITS) {
                let mut esc_count = -1;
                let mut align_bits = 7;

                if (p_extension.type_ == EXT_FILL_DATA) || (p_extension.type_ == EXT_FIL) {
                    payload_bits -= EL_ID_BITS + FILL_EL_COUNT_BITS;
                    if payload_bits >= 15 * 8 {
                        payload_bits -= FILL_EL_ESC_COUNT_BITS;
                        esc_count = 0; // write esc_count even if cnt becomes smaller 15
                    }
                    align_bits = 0;
                }

                let mut cnt = MAX_FILL_DATA_BYTES.min(payload_bits.wrapping_add(align_bits) >> 3);

                if cnt >= 15 {
                    esc_count = cnt - 15 + 1;
                }

                if let Some(h) = bs(&mut h_tp_enc) {
                    // write bitstream
                    fdk_write_bits(h, ID_FIL as u32, EL_ID_BITS as u32);
                    if esc_count >= 0 {
                        fdk_write_bits(h, 15, FILL_EL_COUNT_BITS as u32);
                        fdk_write_bits(h, esc_count as u32, FILL_EL_ESC_COUNT_BITS as u32);
                    } else {
                        fdk_write_bits(h, cnt as u32, FILL_EL_COUNT_BITS as u32);
                    }
                }

                ext_bits_used += EL_ID_BITS
                    + FILL_EL_COUNT_BITS
                    + if esc_count >= 0 { FILL_EL_ESC_COUNT_BITS } else { 0 };

                cnt = (cnt * 8).min(payload_bits); // convert back to bits
                                                   // SAFETY: the caller's guarantee.
                ext_bits_used += unsafe {
                    fdk_aac_enc_write_extension_payload(
                        bs(&mut h_tp_enc),
                        p_extension.type_,
                        p_extension.p_payload,
                        cnt,
                    )
                };
                payload_bits -= cnt;
            }
        }
    }

    ext_bits_used
}

/// `FDKaacEnc_ByteAlignment(hBitStream, alignBits)`
pub fn fdk_aac_enc_byte_alignment(h_bit_stream: &mut FdkBitStream, align_bits: i32) {
    fdk_write_bits(h_bit_stream, 0, align_bits as u32);
}

/// `FDKaacEnc_ChannelElementWrite(hTpEnc, pElInfo, qcOutChannel, psyOutElement, psyOutChannel,
/// syntaxFlags, aot, epConfig, pBitDemand, minCnt)`: arbitrary order bitstream writer. With a
/// transport encoder it writes the element into its bitstream; without one it only counts the
/// static (signal independent) bits into `*p_bit_demand`, and with `min_cnt != 0` the minimum
/// number of static bits (all tools off, no scalefactor bands).
///
/// `qc_out_channel` and `psy_out_channel` are the channels of the element (`None`: C's `NULL`
/// for `qcOutChannel`).
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_channel_element_write(
    mut h_tp_enc: Option<&mut TransportEnc>,
    p_el_info: &ElementInfo,
    qc_out_channel: Option<&[QcOutChannel]>,
    psy_out_element: &PsyOutElement,
    psy_out_channel: &[PsyOutChannel],
    syntax_flags: u32,
    aot: AudioObjectType,
    ep_config: i8,
    p_bit_demand: Option<&mut i32>,
    min_cnt: u8,
) -> AacEncoderError {
    let mut error = AAC_ENC_OK;
    let mut bit_demand: i32 = 0;
    let mut crc_reg1: i32 = -1;
    let mut crc_reg2: i32 = -1;

    // C dereferences psyOutChannel[ch]/qcOutChannel[ch] for ch < numberOfChannels; the encoder
    // always passes as many channels as the element type has. Missing channels read as zero.
    // (created once, not per call: QC_OUT_CHANNEL is about 10 KiB)
    static DEFAULT_PSY: std::sync::LazyLock<PsyOutChannel> =
        std::sync::LazyLock::new(PsyOutChannel::default);
    static DEFAULT_QC: std::sync::LazyLock<QcOutChannel> =
        std::sync::LazyLock::new(QcOutChannel::default);
    let (default_psy, default_qc): (&PsyOutChannel, &QcOutChannel) = (&DEFAULT_PSY, &DEFAULT_QC);

    let number_of_channels: u8 =
        if (p_el_info.el_type == ID_SCE) || (p_el_info.el_type == ID_LFE) { 1 } else { 2 };

    // Get channel element sequence table
    let list = get_bitstream_element_list(aot, ep_config, number_of_channels, 0, 0);
    'bail: {
        let Some(mut list) = list else {
            error = AAC_ENC_UNSUPPORTED_AOT;
            break 'bail;
        };

        if syntax_flags & (AC_SCALABLE | AC_ER) == 0 {
            if let Some(h) = bs(&mut h_tp_enc) {
                fdk_write_bits(h, p_el_info.el_type as u32, EL_ID_BITS as u32);
            }
            bit_demand += EL_ID_BITS;
        }

        // Iterate through sequence table
        let mut i: i32 = 0;
        let mut ch: usize = 0;
        let mut decision_bit: usize = 0;
        loop {
            // some tmp values
            let mut p_ch_section_data: Option<&SectionData> = None;
            let mut p_ch_scf: Option<&[i32]> = None;
            let mut p_ch_max_value_in_sfb: Option<&[u32]> = None;
            let mut p_tns_info: Option<&TnsInfo> = None;
            let mut ch_global_gain = 0;
            let mut ch_block_type = 0;
            let mut ch_max_sfb_per_grp = 0;
            let mut ch_sfb_per_grp = 0;
            let mut ch_sfb_cnt = 0;
            let mut ch_first_scf = 0;

            let psy_ch = psy_out_channel.get(ch).unwrap_or(default_psy);
            let qc_ch = qc_out_channel.map(|qc| qc.get(ch).unwrap_or(default_qc));
            if min_cnt == 0 {
                if let Some(q) = qc_ch {
                    p_ch_section_data = Some(&q.section_data);
                    p_ch_scf = Some(&q.scf);
                    ch_global_gain = q.global_gain;
                    p_ch_max_value_in_sfb = Some(&q.max_value_in_sfb);
                    ch_block_type = q.section_data.block_type;
                    ch_max_sfb_per_grp = q.section_data.max_sfb_per_group;
                    ch_sfb_per_grp = q.section_data.sfb_per_group;
                    ch_sfb_cnt = q.section_data.sfb_cnt;
                    ch_first_scf =
                        q.scf.get(q.section_data.first_scf as usize).copied().unwrap_or(0);
                } else {
                    // get values from PSY
                    ch_sfb_cnt = psy_ch.sfb_cnt;
                    ch_sfb_per_grp = psy_ch.sfb_per_group;
                    ch_max_sfb_per_grp = psy_ch.max_sfb_per_group;
                }
                p_tns_info = Some(&psy_ch.tns_info);
            } // minCnt==0

            if qc_out_channel.is_none() {
                ch_block_type = psy_ch.last_window_sequence;
            }

            match list.id[i as usize] {
                RbdIdT::element_instance_tag => {
                    // Write element instance tag
                    if let Some(h) = bs(&mut h_tp_enc) {
                        fdk_write_bits(h, p_el_info.instance_tag as u32, 4);
                    }
                    bit_demand += 4;
                }
                RbdIdT::common_window => {
                    // Write common window flag
                    decision_bit = psy_out_element.common_window as usize;
                    if let Some(h) = bs(&mut h_tp_enc) {
                        fdk_write_bits(h, psy_out_element.common_window as u32, 1);
                    }
                    bit_demand += 1;
                }
                RbdIdT::ics_info => {
                    // Write individual channel info
                    bit_demand += fdk_aac_enc_encode_ics_info(
                        ch_block_type,
                        psy_ch.window_shape,
                        psy_ch.grouping_mask,
                        ch_max_sfb_per_grp,
                        bs(&mut h_tp_enc),
                        syntax_flags,
                    );
                }
                RbdIdT::ltp_data_present => {
                    // Write LTP data present flag
                    if let Some(h) = bs(&mut h_tp_enc) {
                        fdk_write_bits(h, 0, 1);
                    }
                    bit_demand += 1;
                }
                RbdIdT::ltp_data => {
                    // Predictor data not supported. Nothing to do here.
                }
                RbdIdT::ms => {
                    // Write MS info
                    bit_demand += fdk_aac_enc_encode_ms_info(
                        ch_sfb_cnt,
                        ch_sfb_per_grp,
                        ch_max_sfb_per_grp,
                        if min_cnt == 0 { psy_out_element.tools_info.ms_digest } else { MS_NONE },
                        &psy_out_element.tools_info.ms_mask,
                        bs(&mut h_tp_enc),
                    );
                }
                RbdIdT::global_gain => {
                    bit_demand += fdk_aac_enc_encode_global_gain(
                        ch_global_gain,
                        ch_first_scf,
                        bs(&mut h_tp_enc),
                        psy_ch.mdct_scale,
                    );
                }
                RbdIdT::section_data => {
                    let has_bs = h_tp_enc.is_some();
                    let si_bits = fdk_aac_enc_encode_section_data(
                        p_ch_section_data,
                        bs(&mut h_tp_enc),
                        (syntax_flags & AC_ER_VCB11 != 0) as u32,
                    );
                    if has_bs {
                        if let Some(q) = qc_ch {
                            if si_bits != q.section_data.side_info_bits {
                                error = AAC_ENC_WRITE_SEC_ERROR;
                            }
                        }
                    }
                    bit_demand += si_bits;
                }
                RbdIdT::scale_factor_data => {
                    let has_bs = h_tp_enc.is_some();
                    let sf_data_bits = fdk_aac_enc_encode_scale_factor_data(
                        p_ch_max_value_in_sfb,
                        p_ch_section_data,
                        p_ch_scf,
                        bs(&mut h_tp_enc),
                        &psy_ch.noise_nrg,
                        &psy_ch.is_scale,
                        ch_global_gain,
                    );
                    if has_bs {
                        if let Some(q) = qc_ch {
                            let sd = &q.section_data;
                            if sf_data_bits != sd.scalefac_bits.wrapping_add(sd.noise_nrg_bits) {
                                error = AAC_ENC_WRITE_SCAL_ERROR;
                            }
                        }
                    }
                    bit_demand += sf_data_bits;
                }
                RbdIdT::esc2_rvlc => {
                    if syntax_flags & AC_ER_RVLC != 0 {
                        // write RVLC data into bitstream (error sens. cat. 2)
                        error = AAC_ENC_UNSUPPORTED_AOT;
                    }
                }
                RbdIdT::pulse => {
                    // Write pulse data
                    bit_demand += fdk_aac_enc_encode_pulse_data(bs(&mut h_tp_enc));
                }
                RbdIdT::tns_data_present => {
                    // Write TNS data present flag
                    bit_demand += fdk_aac_enc_encode_tns_data_present(
                        p_tns_info,
                        ch_block_type,
                        bs(&mut h_tp_enc),
                    );
                }
                RbdIdT::tns_data => {
                    // Write TNS data
                    bit_demand +=
                        fdk_aac_enc_encode_tns_data(p_tns_info, ch_block_type, bs(&mut h_tp_enc));
                }
                RbdIdT::gain_control_data => {
                    // Nothing to do here
                }
                RbdIdT::gain_control_data_present => {
                    bit_demand += fdk_aac_enc_encode_gain_control_data(bs(&mut h_tp_enc));
                }
                RbdIdT::esc1_hcr => {
                    if syntax_flags & AC_ER_HCR != 0 {
                        error = AAC_ENC_UNKNOWN;
                    }
                }
                RbdIdT::spectral_data => {
                    if let Some(h) = bs(&mut h_tp_enc) {
                        // C dereferences qcOutChannel unchecked here; the bitstream writer
                        // always passes it.
                        if let (Some(q), Some(sd)) = (qc_ch, p_ch_section_data) {
                            let spectral_bits = fdk_aac_enc_encode_spectral_data(
                                &psy_ch.sfb_offsets,
                                sd,
                                &q.quant_spec,
                                h,
                            );

                            if spectral_bits != q.section_data.huffman_bits {
                                return AAC_ENC_WRITE_SPEC_ERROR;
                            }
                            bit_demand += spectral_bits;
                        }
                    }
                }
                // Non data cases
                RbdIdT::adtscrc_start_reg1 => {
                    if let Some(tp) = h_tp_enc.as_deref_mut() {
                        crc_reg1 = transport_enc_crc_start_reg(tp, 192);
                    }
                }
                RbdIdT::adtscrc_start_reg2 => {
                    if let Some(tp) = h_tp_enc.as_deref_mut() {
                        crc_reg2 = transport_enc_crc_start_reg(tp, 128);
                    }
                }
                RbdIdT::adtscrc_end_reg1 | RbdIdT::drmcrc_end_reg => {
                    if let Some(tp) = h_tp_enc.as_deref_mut() {
                        transport_enc_crc_end_reg(tp, crc_reg1);
                    }
                }
                RbdIdT::adtscrc_end_reg2 => {
                    if let Some(tp) = h_tp_enc.as_deref_mut() {
                        transport_enc_crc_end_reg(tp, crc_reg2);
                    }
                }
                RbdIdT::drmcrc_start_reg => {
                    if let Some(tp) = h_tp_enc.as_deref_mut() {
                        crc_reg1 = transport_enc_crc_start_reg(tp, 0);
                    }
                }
                RbdIdT::next_channel => {
                    ch = (ch + 1) % number_of_channels as usize;
                }
                RbdIdT::link_sequence => {
                    // Only linked lists contain link_sequence.
                    match list.next[decision_bit.min(1)] {
                        Some(n) => list = n,
                        None => return AAC_ENC_UNKNOWN,
                    }
                    i = -1;
                }
                _ => {
                    error = AAC_ENC_UNKNOWN;
                }
            }

            if error != AAC_ENC_OK {
                return error;
            }

            i += 1;

            if list.id[i as usize] == RbdIdT::end_of_sequence {
                break;
            }
        }
    }

    // bail:
    if let Some(p) = p_bit_demand {
        *p = bit_demand;
    }

    error
}

/// `FDKaacEnc_WriteBitstream(hTpEnc, channelMapping, qcOut, psyOut, qcKernel, aot, syntaxFlags,
/// epConfig)`: writes the raw data block of the access unit into the transport encoder's
/// bitstream and ends the access unit (`transportEnc_EndAccessUnit()`).
///
/// # Safety
/// The extension payloads of `qc_out` (`p_payload`) must be valid for reads of their payload
/// bytes (see [`fdk_aac_enc_write_extension_data`]).
#[allow(clippy::too_many_arguments)]
pub unsafe fn fdk_aac_enc_write_bitstream(
    h_tp_enc: &mut TransportEnc,
    channel_mapping: &ChannelMapping,
    qc_out: &mut QcOut,
    psy_out: &PsyOut,
    qc_kernel: &QcState,
    aot: AudioObjectType,
    syntax_flags: u32,
    ep_config: i8,
) -> AacEncoderError {
    let mut error_status = AAC_ENC_OK;
    let do_byte_align = 1;
    // Get first bit of raw data block. In case of ADTS+PCE, AU would start at PCE. This is okay
    // because PCE assures alignment.
    let align_anchor: u32 = fdk_get_valid_bits(&mut h_tp_enc.bit_stream);

    let mut bit_mark_up: i32 = align_anchor as i32;
    let mut frame_bits: i32 = bit_mark_up;

    let n_elements = channel_mapping.n_elements.clamp(0, 8) as usize;

    // Channel element loop
    for i in 0..n_elements {
        let el_info = channel_mapping.el_info[i];
        let mut element_used_bits: i32 = 0;

        match el_info.el_type {
            ID_SCE | ID_CPE | ID_LFE => {
                let n = el_info.n_channels_in_el.clamp(0, 2) as usize;
                let qe = &qc_out.qc_element[i];
                let q0 = qe.qc_out_channel[0];
                let pe = &psy_out.psy_out_element[i];
                let p0 = pe.psy_out_channel[0];
                // An element with fewer channels than its type implies is never set up.
                let nq = n.max(1);
                error_status = fdk_aac_enc_channel_element_write(
                    Some(h_tp_enc),
                    &el_info,
                    Some(
                        &qc_out.p_qc_out_channels
                            [q0..(q0 + nq).min(qc_out.p_qc_out_channels.len())],
                    ),
                    pe,
                    &psy_out.p_psy_out_channels
                        [p0..(p0 + nq).min(psy_out.p_psy_out_channels.len())],
                    syntax_flags, // syntaxFlags (ER tools ...)
                    aot,          // aot: AOT_AAC_LC, AOT_SBR, AOT_PS
                    ep_config,    // epConfig -1, 0, 1
                    None,
                    0,
                );
                if error_status != AAC_ENC_OK {
                    return error_status;
                }

                if syntax_flags & AC_ER == 0 {
                    // Write associated extension payload
                    for n in 0..qe.n_extensions.max(0) as usize {
                        let ext = element_extension(qe, n);
                        // SAFETY: the caller's guarantee.
                        unsafe {
                            fdk_aac_enc_write_extension_data(
                                Some(h_tp_enc),
                                &ext,
                                0,
                                align_anchor,
                                syntax_flags,
                                aot,
                                ep_config,
                            );
                        }
                    }
                }
            }

            // In FDK, DSE signalling explicit done in elDSE. See channel_map.cpp
            _ => return AAC_ENC_INVALID_ELEMENTINFO_TYPE,
        }

        if el_info.el_type != ID_DSE {
            element_used_bits -= bit_mark_up;
            bit_mark_up = fdk_get_valid_bits(&mut h_tp_enc.bit_stream) as i32;
            element_used_bits += bit_mark_up;
            frame_bits += element_used_bits;
        }
    } // for (i=0; i<channelMapping.nElements; i++)

    if (syntax_flags & AC_ER != 0) && (syntax_flags & AC_DRM == 0) {
        // 0: extension not touched, 1: extension already written
        let mut channel_element_extension_written = [[0u8; 1]; 8];

        if syntax_flags & AC_ELD != 0 {
            for i in 0..n_elements {
                let qe = &qc_out.qc_element[i];
                for n in 0..qe.n_extensions.max(0) as usize {
                    let ext = element_extension(qe, n);
                    if (ext.type_ == EXT_SBR_DATA) || (ext.type_ == EXT_SBR_DATA_CRC) {
                        // Write sbr extension payload
                        // SAFETY: the caller's guarantee.
                        unsafe {
                            fdk_aac_enc_write_extension_data(
                                Some(h_tp_enc),
                                &ext,
                                0,
                                align_anchor,
                                syntax_flags,
                                aot,
                                ep_config,
                            );
                        }

                        if n < 1 {
                            channel_element_extension_written[i][n] = 1;
                        }
                    } // SBR
                } // n
            } // i
        } // AC_ELD

        for i in 0..n_elements {
            let qe = &qc_out.qc_element[i];
            for n in 0..qe.n_extensions.max(0) as usize {
                // Bounded to the 1-entry array (an element gets at most one extension payload).
                let written = if n < 1 { channel_element_extension_written[i][n] } else { 0 };
                if written == 0 {
                    // Write all ramaining extension payloads in element
                    let ext = element_extension(qe, n);
                    // SAFETY: the caller's guarantee.
                    unsafe {
                        fdk_aac_enc_write_extension_data(
                            Some(h_tp_enc),
                            &ext,
                            0,
                            align_anchor,
                            syntax_flags,
                            aot,
                            ep_config,
                        );
                    }
                }
            } // n
        } // i
    } // if AC_ER

    // Extend global extension payload table with fill bits
    let n = qc_out.n_extensions;

    // Add fill data / stuffing bits
    if (0..4).contains(&n) {
        qc_out.extension[n as usize].type_ = EXT_FILL_DATA;
        qc_out.extension[n as usize].n_payload_bits = qc_out.tot_fill_bits;
    }
    // Bounded: writes outside the array are dropped.
    qc_out.n_extensions += 1;

    // Write global extension payload and fill data
    let mut n = 0;
    while (n < qc_out.n_extensions) && (n < (2 + 2)) {
        let ext = qc_out.extension[n as usize];
        // SAFETY: the caller's guarantee.
        unsafe {
            fdk_aac_enc_write_extension_data(
                Some(h_tp_enc),
                &ext,
                0,
                align_anchor,
                syntax_flags,
                aot,
                ep_config,
            );
        }
        // For EXT_FIL or EXT_FILL_DATA we could do an additional sanity check here
        n += 1;
    }

    if syntax_flags & (AC_SCALABLE | AC_ER) == 0 {
        fdk_write_bits(&mut h_tp_enc.bit_stream, ID_END as u32, EL_ID_BITS as u32);
    }

    if do_byte_align != 0 {
        // Assure byte alignment
        if ((fdk_get_valid_bits(&mut h_tp_enc.bit_stream)
            .wrapping_sub(align_anchor)
            .wrapping_add(qc_out.align_bits as u32))
            & 0x7)
            != 0
        {
            return AAC_ENC_WRITTEN_BITS_ERROR;
        }

        fdk_aac_enc_byte_alignment(&mut h_tp_enc.bit_stream, qc_out.align_bits);
    }

    frame_bits -= bit_mark_up;
    frame_bits = frame_bits.wrapping_add(fdk_get_valid_bits(&mut h_tp_enc.bit_stream) as i32);

    transport_enc_end_access_unit(h_tp_enc, &mut frame_bits);

    if frame_bits != qc_out.total_bits.wrapping_add(qc_kernel.glob_hdr_bits) {
        return AAC_ENC_WRITTEN_BITS_ERROR;
    }

    error_status
}

/// `qcElement->extension[n]`: the one extension slot of the element (an element gets at most
/// one payload, see `FDKaacEnc_EncodeFrame()`); the port gives an empty payload for other `n`.
fn element_extension(qe: &QcOutElement, n: usize) -> QcOutExtension {
    qe.extension.get(n).copied().unwrap_or_default()
}
