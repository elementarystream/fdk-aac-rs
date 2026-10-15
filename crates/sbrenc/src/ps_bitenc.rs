// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/ps_bitenc.h, libSBRenc/src/ps_bitenc.cpp.

//! Port of `ps_bitenc.h` and `ps_bitenc.cpp`: the parametric stereo bitstream writer.
//!
//! The functions take the bitstream as `Option<&mut FdkBitStream>`: C calls them with
//! `hBitBuf == NULL` to count the bits only.

use fdk_aac_rs_fdk::bitstream::{fdk_write_bits, FdkBitStream};

use crate::ps_const::*;

mod tables;
pub use tables::*;

/// `PS_EXT_ID_V0`
const PS_EXT_ID_V0: u32 = 0;

/// `PS_OUT` (`HANDLE_PS_OUT` is `&mut PsOut`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsOut {
    /// `enablePSHeader`
    pub enable_ps_header: i32,
    /// `enableIID`
    pub enable_iid: i32,
    /// `iidMode`
    pub iid_mode: i32,
    /// `enableICC`
    pub enable_icc: i32,
    /// `iccMode`
    pub icc_mode: i32,
    /// `enableIpdOpd`
    pub enable_ipd_opd: i32,

    /// `frameClass`
    pub frame_class: i32,
    /// `nEnvelopes`
    pub n_envelopes: i32,
    // ENV data
    /// `frameBorder`
    pub frame_border: [i32; PS_MAX_ENVELOPES],

    // iid data
    /// `deltaIID`
    pub delta_iid: [PsDelta; PS_MAX_ENVELOPES],
    /// `iid`
    pub iid: [[i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    /// `iidLast`
    pub iid_last: [i32; PS_MAX_BANDS],

    // icc data
    /// `deltaICC`
    pub delta_icc: [PsDelta; PS_MAX_ENVELOPES],
    /// `icc`
    pub icc: [[i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    /// `iccLast`
    pub icc_last: [i32; PS_MAX_BANDS],

    // ipd data
    /// `deltaIPD`
    pub delta_ipd: [PsDelta; PS_MAX_ENVELOPES],
    /// `ipd`
    pub ipd: [[i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    /// `ipdLast`
    pub ipd_last: [i32; PS_MAX_BANDS],

    // opd data
    /// `deltaOPD`
    pub delta_opd: [PsDelta; PS_MAX_ENVELOPES],
    /// `opd`
    pub opd: [[i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    /// `opdLast`
    pub opd_last: [i32; PS_MAX_BANDS],
}

/// `FDKsbrEnc_WriteBits_ps(hBitStream, value, numberOfBits)`: writes the bits if there is a
/// bitstream (`hBitStream == NULL` happens here intentionally) and returns their number.
#[inline]
fn write_bits_ps(
    h_bit_stream: &mut Option<&mut FdkBitStream>,
    value: u32,
    number_of_bits: u32,
) -> i32 {
    if let Some(bs) = h_bit_stream.as_deref_mut() {
        fdk_write_bits(bs, value, number_of_bits);
    }
    number_of_bits as i32
}

/// `getNoBands(mode)`
fn get_no_bands(mode: i32) -> i32 {
    match mode {
        0 | 3 => PS_BANDS_COARSE, // coarse
        1 | 4 => PS_BANDS_MID,    // mid
        _ => PS_BANDS_COARSE,     // fine not supported, coarse as default
    }
}

/// `getIIDRes(iidMode)`
fn get_iid_res(iid_mode: i32) -> i32 {
    if iid_mode < 3 {
        PS_IID_RES_COARSE
    } else {
        PS_IID_RES_FINE
    }
}

/// `encodeDeltaFreq(hBitBuf, val, nBands, codeTable, lengthTable, tableOffset, maxVal, error)`
#[allow(clippy::too_many_arguments)]
fn encode_delta_freq(
    h_bit_buf: &mut Option<&mut FdkBitStream>,
    val: &[i32],
    n_bands: i32,
    code_table: &[u32],
    length_table: &[u32],
    table_offset: i32,
    max_val: i32,
    error: &mut i32,
) -> i32 {
    let mut bit_cnt = 0;
    let mut last_val = 0;

    for band in 0..n_bands.max(0) as usize {
        let mut delta = val[band].wrapping_sub(last_val).wrapping_add(table_offset);
        last_val = val[band];
        if delta > max_val || delta < 0 {
            *error = 1;
            delta = if delta > 0 { max_val } else { 0 };
        }
        bit_cnt +=
            write_bits_ps(h_bit_buf, code_table[delta as usize], length_table[delta as usize]);
    }

    bit_cnt
}

/// `encodeDeltaTime(hBitBuf, val, valLast, nBands, codeTable, lengthTable, tableOffset,
/// maxVal, error)`
#[allow(clippy::too_many_arguments)]
fn encode_delta_time(
    h_bit_buf: &mut Option<&mut FdkBitStream>,
    val: &[i32],
    val_last: &[i32],
    n_bands: i32,
    code_table: &[u32],
    length_table: &[u32],
    table_offset: i32,
    max_val: i32,
    error: &mut i32,
) -> i32 {
    let mut bit_cnt = 0;

    for band in 0..n_bands.max(0) as usize {
        let mut delta = val[band].wrapping_sub(val_last[band]).wrapping_add(table_offset);
        if delta > max_val || delta < 0 {
            *error = 1;
            delta = if delta > 0 { max_val } else { 0 };
        }
        bit_cnt +=
            write_bits_ps(h_bit_buf, code_table[delta as usize], length_table[delta as usize]);
    }

    bit_cnt
}

/// `FDKsbrEnc_EncodeIid(hBitBuf, iidVal, iidValLast, nBands, res, mode, error)`: `iid_val_last`
/// may be empty (`NULL`) for delta frequency coding.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_encode_iid(
    mut h_bit_buf: Option<&mut FdkBitStream>,
    iid_val: &[i32],
    iid_val_last: &[i32],
    n_bands: i32,
    res: PsIidResolution,
    mode: PsDelta,
    error: &mut i32,
) -> i32 {
    let mut bit_cnt = 0;
    let h = &mut h_bit_buf;

    match mode {
        PS_DELTA_FREQ => match res {
            PS_IID_RES_COARSE => {
                bit_cnt += encode_delta_freq(
                    h,
                    iid_val,
                    n_bands,
                    &IID_DELTA_FREQ_COARSE_CODE,
                    &IID_DELTA_FREQ_COARSE_LENGTH,
                    IID_DELTA_COARSE_OFFSET,
                    IID_DELTA_COARSE_MAX_VAL,
                    error,
                );
            }
            PS_IID_RES_FINE => {
                bit_cnt += encode_delta_freq(
                    h,
                    iid_val,
                    n_bands,
                    &IID_DELTA_FREQ_FINE_CODE,
                    &IID_DELTA_FREQ_FINE_LENGTH,
                    IID_DELTA_FINE_OFFSET,
                    IID_DELTA_FINE_MAX_VAL,
                    error,
                );
            }
            _ => *error = 1,
        },
        PS_DELTA_TIME => match res {
            PS_IID_RES_COARSE => {
                bit_cnt += encode_delta_time(
                    h,
                    iid_val,
                    iid_val_last,
                    n_bands,
                    &IID_DELTA_TIME_COARSE_CODE,
                    &IID_DELTA_TIME_COARSE_LENGTH,
                    IID_DELTA_COARSE_OFFSET,
                    IID_DELTA_COARSE_MAX_VAL,
                    error,
                );
            }
            PS_IID_RES_FINE => {
                bit_cnt += encode_delta_time(
                    h,
                    iid_val,
                    iid_val_last,
                    n_bands,
                    &IID_DELTA_TIME_FINE_CODE,
                    &IID_DELTA_TIME_FINE_LENGTH,
                    IID_DELTA_FINE_OFFSET,
                    IID_DELTA_FINE_MAX_VAL,
                    error,
                );
            }
            _ => *error = 1,
        },
        _ => *error = 1,
    }

    bit_cnt
}

/// `FDKsbrEnc_EncodeIcc(hBitBuf, iccVal, iccValLast, nBands, mode, error)`
pub fn fdk_sbr_enc_encode_icc(
    mut h_bit_buf: Option<&mut FdkBitStream>,
    icc_val: &[i32],
    icc_val_last: &[i32],
    n_bands: i32,
    mode: PsDelta,
    error: &mut i32,
) -> i32 {
    let h = &mut h_bit_buf;
    match mode {
        PS_DELTA_FREQ => encode_delta_freq(
            h,
            icc_val,
            n_bands,
            &ICC_DELTA_FREQ_CODE,
            &ICC_DELTA_FREQ_LENGTH,
            ICC_DELTA_OFFSET,
            ICC_DELTA_MAX_VAL,
            error,
        ),
        PS_DELTA_TIME => encode_delta_time(
            h,
            icc_val,
            icc_val_last,
            n_bands,
            &ICC_DELTA_TIME_CODE,
            &ICC_DELTA_TIME_LENGTH,
            ICC_DELTA_OFFSET,
            ICC_DELTA_MAX_VAL,
            error,
        ),
        _ => {
            *error = 1;
            0
        }
    }
}

/// `FDKsbrEnc_EncodeIpd(hBitBuf, ipdVal, ipdValLast, nBands, mode, error)`
pub fn fdk_sbr_enc_encode_ipd(
    mut h_bit_buf: Option<&mut FdkBitStream>,
    ipd_val: &[i32],
    ipd_val_last: &[i32],
    n_bands: i32,
    mode: PsDelta,
    error: &mut i32,
) -> i32 {
    let h = &mut h_bit_buf;
    match mode {
        PS_DELTA_FREQ => encode_delta_freq(
            h,
            ipd_val,
            n_bands,
            &IPD_DELTA_FREQ_CODE,
            &IPD_DELTA_FREQ_LENGTH,
            IPD_DELTA_OFFSET,
            IPD_DELTA_MAX_VAL,
            error,
        ),
        PS_DELTA_TIME => encode_delta_time(
            h,
            ipd_val,
            ipd_val_last,
            n_bands,
            &IPD_DELTA_TIME_CODE,
            &IPD_DELTA_TIME_LENGTH,
            IPD_DELTA_OFFSET,
            IPD_DELTA_MAX_VAL,
            error,
        ),
        _ => {
            *error = 1;
            0
        }
    }
}

/// `FDKsbrEnc_EncodeOpd(hBitBuf, opdVal, opdValLast, nBands, mode, error)`
pub fn fdk_sbr_enc_encode_opd(
    mut h_bit_buf: Option<&mut FdkBitStream>,
    opd_val: &[i32],
    opd_val_last: &[i32],
    n_bands: i32,
    mode: PsDelta,
    error: &mut i32,
) -> i32 {
    let h = &mut h_bit_buf;
    match mode {
        PS_DELTA_FREQ => encode_delta_freq(
            h,
            opd_val,
            n_bands,
            &OPD_DELTA_FREQ_CODE,
            &OPD_DELTA_FREQ_LENGTH,
            OPD_DELTA_OFFSET,
            OPD_DELTA_MAX_VAL,
            error,
        ),
        PS_DELTA_TIME => encode_delta_time(
            h,
            opd_val,
            opd_val_last,
            n_bands,
            &OPD_DELTA_TIME_CODE,
            &OPD_DELTA_TIME_LENGTH,
            OPD_DELTA_OFFSET,
            OPD_DELTA_MAX_VAL,
            error,
        ),
        _ => {
            *error = 1;
            0
        }
    }
}

/// `encodeIpdOpd(psOut, hBitBuf)`. Quirk: the first bit (`enableIpdOpd`) is written but not
/// counted.
fn encode_ipd_opd(ps_out: &PsOut, mut h_bit_buf: Option<&mut FdkBitStream>) -> i32 {
    let mut bit_cnt = 0;
    let mut error = 0;

    write_bits_ps(&mut h_bit_buf, ps_out.enable_ipd_opd as u32, 1);

    if ps_out.enable_ipd_opd == 1 {
        let ipd_last = &ps_out.ipd_last;
        let opd_last = &ps_out.opd_last;

        for env in 0..ps_out.n_envelopes.max(0) as usize {
            bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.delta_ipd[env] as u32, 1);
            bit_cnt += fdk_sbr_enc_encode_ipd(
                h_bit_buf.as_deref_mut(),
                &ps_out.ipd[env],
                ipd_last,
                get_no_bands(ps_out.iid_mode),
                ps_out.delta_ipd[env],
                &mut error,
            );

            bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.delta_opd[env] as u32, 1);
            bit_cnt += fdk_sbr_enc_encode_opd(
                h_bit_buf.as_deref_mut(),
                &ps_out.opd[env],
                opd_last,
                get_no_bands(ps_out.iid_mode),
                ps_out.delta_opd[env],
                &mut error,
            );
        }
        // reserved bit
        bit_cnt += write_bits_ps(&mut h_bit_buf, 0, 1);
    }

    bit_cnt
}

/// `getEnvIdx(nEnvelopes, frameClass)`
fn get_env_idx(n_envelopes: i32, frame_class: i32) -> i32 {
    match n_envelopes {
        0 => 0,
        1 => {
            if frame_class == 0 {
                1
            } else {
                0
            }
        }
        2 => {
            if frame_class == 0 {
                2
            } else {
                1
            }
        }
        3 => 2,
        4 => 3,
        _ => 0, // unsupported number of envelopes
    }
}

/// `encodePSExtension(psOut, hBitBuf)`
fn encode_ps_extension(ps_out: &PsOut, mut h_bit_buf: Option<&mut FdkBitStream>) -> i32 {
    let mut bit_cnt = 0;

    if ps_out.enable_ipd_opd == 1 {
        let mut ipd_opd_bits = 0;
        let ext_size = (2 + encode_ipd_opd(ps_out, None) + 7) >> 3;

        if ext_size < 15 {
            bit_cnt += write_bits_ps(&mut h_bit_buf, ext_size as u32, 4);
        } else {
            bit_cnt += write_bits_ps(&mut h_bit_buf, 15, 4);
            bit_cnt += write_bits_ps(&mut h_bit_buf, (ext_size - 15) as u32, 8);
        }

        // write ipd opd data
        ipd_opd_bits += write_bits_ps(&mut h_bit_buf, PS_EXT_ID_V0, 2);
        ipd_opd_bits += encode_ipd_opd(ps_out, h_bit_buf.as_deref_mut());

        // byte align the ipd opd data
        if ipd_opd_bits % 8 != 0 {
            ipd_opd_bits += write_bits_ps(&mut h_bit_buf, 0, (8 - (ipd_opd_bits % 8)) as u32);
        }

        bit_cnt += ipd_opd_bits;
    }

    bit_cnt
}

/// `FDKsbrEnc_WritePSBitstream(psOut, hBitBuf)`: writes ps_data() (or counts its bits only, for
/// `None`) and returns the number of bits.
pub fn fdk_sbr_enc_write_ps_bitstream(
    ps_out: Option<&PsOut>,
    mut h_bit_buf: Option<&mut FdkBitStream>,
) -> i32 {
    let mut ps_ext_enable = 0;
    let mut bit_cnt = 0;
    let mut error = 0;

    if let Some(ps_out) = ps_out {
        // PS HEADER
        bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.enable_ps_header as u32, 1);

        if ps_out.enable_ps_header != 0 {
            bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.enable_iid as u32, 1);
            if ps_out.enable_iid != 0 {
                bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.iid_mode as u32, 3);
            }
            bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.enable_icc as u32, 1);
            if ps_out.enable_icc != 0 {
                bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.icc_mode as u32, 3);
            }
            if ps_out.enable_ipd_opd != 0 {
                ps_ext_enable = 1;
            }
            bit_cnt += write_bits_ps(&mut h_bit_buf, ps_ext_enable as u32, 1);
        }

        // Frame class, number of envelopes
        bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.frame_class as u32, 1);
        bit_cnt += write_bits_ps(
            &mut h_bit_buf,
            get_env_idx(ps_out.n_envelopes, ps_out.frame_class) as u32,
            2,
        );

        if ps_out.frame_class == 1 {
            for env in 0..ps_out.n_envelopes.max(0) as usize {
                bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.frame_border[env] as u32, 5);
            }
        }

        if ps_out.enable_iid == 1 {
            let mut iid_last: &[i32] = &ps_out.iid_last;
            for env in 0..ps_out.n_envelopes.max(0) as usize {
                bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.delta_iid[env] as u32, 1);
                bit_cnt += fdk_sbr_enc_encode_iid(
                    h_bit_buf.as_deref_mut(),
                    &ps_out.iid[env],
                    iid_last,
                    get_no_bands(ps_out.iid_mode),
                    get_iid_res(ps_out.iid_mode),
                    ps_out.delta_iid[env],
                    &mut error,
                );

                iid_last = &ps_out.iid[env];
            }
        }

        if ps_out.enable_icc == 1 {
            let mut icc_last: &[i32] = &ps_out.icc_last;
            for env in 0..ps_out.n_envelopes.max(0) as usize {
                bit_cnt += write_bits_ps(&mut h_bit_buf, ps_out.delta_icc[env] as u32, 1);
                bit_cnt += fdk_sbr_enc_encode_icc(
                    h_bit_buf.as_deref_mut(),
                    &ps_out.icc[env],
                    icc_last,
                    get_no_bands(ps_out.icc_mode),
                    ps_out.delta_icc[env],
                    &mut error,
                );

                icc_last = &ps_out.icc[env];
            }
        }

        if ps_ext_enable != 0 {
            bit_cnt += encode_ps_extension(ps_out, h_bit_buf);
        }
    }

    bit_cnt
}
