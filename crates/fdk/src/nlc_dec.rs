// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/nlc_dec.h, libFDK/src/nlc_dec.cpp.

//! Port of `nlc_dec.h`: SAC/SAOC decoder noiseless coding (Huffman, PCM and differential
//! decoding of the parameter data).
//!
//! Both variants compile the same code. The decoding reads any bits without failing: the node
//! tables are trees whose entries stay within their table (see [`crate::huff_nodes`]), and all
//! array indices are bounded by `dataBands <= 28` (`MAX_PARAMETER_BANDS`). Values are stored to
//! `SCHAR` with truncation (`as i8`), like the C++ code.
//!
//! Deviation: `EcDataPairDec()` with `dataBands` outside of `0..=28` has no defined result in
//! C++; the port returns `HUFFDEC_NOTOK` without reading.

use crate::bitstream::{fdk_get_valid_bits, fdk_read_bits, FdkBitStream};
use crate::huff_nodes::*;

/// `DECODER_TYPE`
pub type DecoderType = i32;
/// `SAC_DECODER`
pub const SAC_DECODER: DecoderType = 0;
/// `SAOC_DECODER`
pub const SAOC_DECODER: DecoderType = 1;
/// `USAC_DECODER`
pub const USAC_DECODER: DecoderType = 2;

/// `DATA_TYPE`
pub type DataType = i32;
/// `t_CLD`
pub const t_CLD: DataType = 0;
/// `t_ICC`
pub const t_ICC: DataType = 1;
/// `t_IPD`
pub const t_IPD: DataType = 2;
/// `t_OLD`
pub const t_OLD: DataType = 3;
/// `t_IOC`
pub const t_IOC: DataType = 4;
/// `t_NRG`
pub const t_NRG: DataType = 5;
/// `t_DCLD`
pub const t_DCLD: DataType = 6;
/// `t_DMG`
pub const t_DMG: DataType = 7;
/// `t_PDG`
pub const t_PDG: DataType = 8;

/// `DIRECTION`
pub type Direction = i32;
/// `BACKWARDS`
pub const BACKWARDS: Direction = 0x0;
/// `FORWARDS`
pub const FORWARDS: Direction = 0x1;

/// `DIFF_TYPE`
pub type DiffType = i32;
/// `DIFF_FREQ`
pub const DIFF_FREQ: DiffType = 0x0;
/// `DIFF_TIME`
pub const DIFF_TIME: DiffType = 0x1;

/// `CODING_SCHEME`
pub type CodingScheme = i32;
/// `HUFF_1D`
pub const HUFF_1D: CodingScheme = 0x0;
/// `HUFF_2D`
pub const HUFF_2D: CodingScheme = 0x1;

/// `PAIRING`
pub type Pairing = i32;
/// `FREQ_PAIR`
pub const FREQ_PAIR: Pairing = 0x0;
/// `TIME_PAIR`
pub const TIME_PAIR: Pairing = 0x1;

/// `MAX_ENTRIES`
pub const MAX_ENTRIES: usize = 168;

/// `HUFFDEC_OK`
pub const HUFFDEC_OK: ErrorT = 0;
/// `HUFFDEC_NOTOK`
pub const HUFFDEC_NOTOK: ErrorT = -1;

/// `ERROR_t`
pub type ErrorT = i32;

/// `MAX_PARAMETER_BANDS` (the length of the local arrays of `nlc_dec.cpp`).
const MAX_PARAMETER_BANDS: usize = 28;

/// `HANDLE_HUFF_NODE`: a node table.
type HuffNode = [[i16; 2]];

/// `min(a, b)`
#[inline(always)]
fn min(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `sym_restoreIPD(strm, lav, data)`
pub fn sym_restore_ipd(strm: &mut FdkBitStream, lav: i32, data: &mut [i8; 2]) -> ErrorT {
    let sum_val = data[0] as i32 + data[1] as i32;
    let diff_val = data[0] as i32 - data[1] as i32;

    if sum_val > lav {
        data[0] = (-sum_val + (2 * lav + 1)) as i8;
        data[1] = (-diff_val) as i8;
    } else {
        data[0] = sum_val as i8;
        data[1] = diff_val as i8;
    }

    if data[0] as i32 - data[1] as i32 != 0 {
        let sym_bit = fdk_read_bits(strm, 1);
        if sym_bit != 0 {
            data.swap(0, 1);
        }
    }

    HUFFDEC_OK
}

/// `ilog2(i)`: number of bits of `i - 1`.
fn ilog2(mut i: u32) -> i32 {
    let mut l = 0;

    i = i.saturating_sub(1);
    while i > 0 {
        i >>= 1;
        l += 1;
    }

    l
}

/// `pcm_decode(strm, out_data_1, out_data_2, offset, num_val, num_levels)`
///
/// With both outputs, the values are interleaved (even indices to `out_data_1`, odd ones to
/// `out_data_2`).
fn pcm_decode(
    strm: &mut FdkBitStream,
    mut out_data_1: Option<&mut [i8]>,
    mut out_data_2: Option<&mut [i8]>,
    offset: i32,
    num_val: i32,
    num_levels: i32,
) -> ErrorT {
    let mut pcm_chunk_size = [0i32; 7];

    let max_grp_len: i32 = match num_levels {
        3 => 5,
        7 => 6,
        11 => 2,
        13 => 4,
        19 => 4,
        25 => 3,
        51 => 4,
        4 | 8 | 15 | 16 | 26 | 31 => 1,
        _ => return HUFFDEC_NOTOK,
    };

    let mut tmp: u32 = 1;
    for i in 1..=max_grp_len as usize {
        tmp = tmp.wrapping_mul(num_levels as u32);
        pcm_chunk_size[i] = ilog2(tmp);
    }

    let mut i = 0;
    while i < num_val {
        let grp_len = min(max_grp_len, num_val - i);
        let data = fdk_read_bits(strm, pcm_chunk_size[grp_len as usize] as u32) as i32;

        let mut grp_val = data;

        for j in 0..grp_len {
            let idx = i + (grp_len - j - 1);
            let next_val = grp_val % num_levels;
            let v = (next_val - offset) as i8;

            match (&mut out_data_1, &mut out_data_2) {
                (Some(o1), None) => o1[idx as usize] = v,
                (None, Some(o2)) => o2[idx as usize] = v,
                (Some(o1), Some(o2)) => {
                    if idx % 2 != 0 {
                        o2[(idx / 2) as usize] = v;
                    } else {
                        o1[(idx / 2) as usize] = v;
                    }
                }
                // No defined result in C++; the port stores nothing.
                (None, None) => {}
            }

            grp_val = (grp_val - next_val) / num_levels;
        }
        i += max_grp_len;
    }

    HUFFDEC_OK
}

/// `huff_read(strm, nodeTab, out_data)`: walks the node table bit by bit until a leaf.
fn huff_read(strm: &mut FdkBitStream, node_tab: &HuffNode, out_data: &mut i32) -> ErrorT {
    let mut node: i32 = 0;

    loop {
        let next_bit = fdk_read_bits(strm, 1);
        node = node_tab[node as usize][next_bit as usize] as i32;
        if node <= 0 {
            break;
        }
    }

    *out_data = node;

    HUFFDEC_OK
}

/// `huff_read_2D(strm, nodeTab, out_data, escape)`
fn huff_read_2d(
    strm: &mut FdkBitStream,
    node_tab: &HuffNode,
    out_data: &mut [i8; 2],
    escape: &mut i32,
) -> ErrorT {
    let mut node = 0;

    let err = huff_read(strm, node_tab, &mut node);
    if err != HUFFDEC_OK {
        return err;
    }
    *escape = (node == 0) as i32;

    if *escape != 0 {
        out_data[0] = 0;
        out_data[1] = 1;
    } else {
        let huff_2d_8bit = -(node + 1);
        out_data[0] = (huff_2d_8bit >> 4) as i8;
        out_data[1] = (huff_2d_8bit & 0xf) as i8;
    }

    HUFFDEC_OK
}

/// `sym_restore(strm, lav, data)`
fn sym_restore(strm: &mut FdkBitStream, lav: i32, data: &mut [i8; 2]) -> ErrorT {
    let sum_val = data[0] as i32 + data[1] as i32;
    let diff_val = data[0] as i32 - data[1] as i32;

    if sum_val > lav {
        data[0] = (-sum_val + (2 * lav + 1)) as i8;
        data[1] = (-diff_val) as i8;
    } else {
        data[0] = sum_val as i8;
        data[1] = diff_val as i8;
    }

    if data[0] as i32 + data[1] as i32 != 0 {
        let sym_bit = fdk_read_bits(strm, 1);
        if sym_bit != 0 {
            data[0] = (-(data[0] as i32)) as i8;
            data[1] = (-(data[1] as i32)) as i8;
        }
    }

    if data[0] as i32 - data[1] as i32 != 0 {
        let sym_bit = fdk_read_bits(strm, 1);
        if sym_bit != 0 {
            data.swap(0, 1);
        }
    }

    HUFFDEC_OK
}

/// The 1D node table of `data_type` (`h1D[dim1]`), `None` for types without one.
fn node_tab_1d(data_type: DataType, dim1: i32) -> Option<(&'static HuffNode, &'static HuffNode)> {
    let d = dim1 as usize;
    Some(match data_type {
        t_CLD => (&FDK_HUFF_PART0_NODES.cld[..], &FDK_HUFF_CLD_NODES.h1d[d].node_tab[..]),
        t_ICC => (&FDK_HUFF_PART0_NODES.icc[..], &FDK_HUFF_ICC_NODES.h1d[d].node_tab[..]),
        t_OLD => (&FDK_HUFF_PART0_NODES.old[..], &HUFF_OLD_NODES.h1d[d].node_tab[..]),
        t_IPD => (&FDK_HUFF_PART0_NODES.ipd[..], &FDK_HUFF_IPD_NODES.h1d[d].node_tab[..]),
        _ => return None,
    })
}

/// `huff_dec_1D(strm, data_type, dim1, out_data, num_val, p0_flag)`
fn huff_dec_1d(
    strm: &mut FdkBitStream,
    data_type: DataType,
    dim1: i32,
    out_data: &mut [i8],
    num_val: i32,
    p0_flag: i32,
) -> ErrorT {
    let mut node = 0;
    let mut offset = 0;

    let Some((part_tab, node_tab)) = node_tab_1d(data_type, dim1) else {
        fdk_assert!(false, "FDK_ASSERT(0)");
        return HUFFDEC_NOTOK;
    };

    if p0_flag != 0 {
        let err = huff_read(strm, part_tab, &mut node);
        if err != HUFFDEC_OK {
            return err;
        }

        out_data[0] = (-(node + 1)) as i8;
        offset = 1;
    }

    for i in offset..num_val.max(offset) {
        let bits_avail = fdk_get_valid_bits(strm) as i32;
        if bits_avail < 1 {
            return HUFFDEC_NOTOK;
        }

        let err = huff_read(strm, node_tab, &mut node);
        if err != HUFFDEC_OK {
            return err;
        }
        let mut od = -(node + 1);

        if data_type != t_IPD && od != 0 {
            let bits_avail = fdk_get_valid_bits(strm) as i32;
            if bits_avail < 1 {
                return HUFFDEC_NOTOK;
            }

            let od_sign = fdk_read_bits(strm, 1);

            if od_sign != 0 {
                od = -od;
            }
        }

        out_data[i as usize] = od as i8;
    }

    HUFFDEC_OK
}

/// `huff_dec_2D(strm, data_type, dim1, dim2, out_data, num_val, stride, p0_data)`
fn huff_dec_2d(
    strm: &mut FdkBitStream,
    data_type: DataType,
    dim1: i32,
    dim2: i32,
    out_data: &mut [[i8; 2]],
    num_val: i32,
    stride: i32,
    p0_data: [Option<&mut i8>; 2],
) -> ErrorT {
    let mut escape = 0;
    let mut esc_cntr = 0usize;
    let mut node = 0;

    let mut esc_data = [[0i8; MAX_PARAMETER_BANDS]; 2];
    let mut esc_idx = [0i32; MAX_PARAMETER_BANDS];

    // LAV
    let err = huff_read(strm, &FDK_HUFF_LAV_IDX_NODES.node_tab[..], &mut node);
    if err != HUFFDEC_OK {
        return err;
    }
    // unsigned long data = -(node + 1): 0..3
    let mut data = (-(node + 1)) as u32;

    let lav: i32;
    let mut node_tab: &HuffNode = match data_type {
        t_CLD => {
            lav = (2 * data + 3) as i32; // 3, 5, 7, 9
            &FDK_HUFF_PART0_NODES.cld[..]
        }
        t_ICC => {
            lav = (2 * data + 1) as i32; // 1, 3, 5, 7
            &FDK_HUFF_PART0_NODES.icc[..]
        }
        t_OLD => {
            lav = (3 * data + 3) as i32;
            &FDK_HUFF_PART0_NODES.old[..]
        }
        t_IPD => {
            if data == 0 {
                data = 3;
            } else {
                data -= 1;
            }
            lav = (2 * data + 1) as i32; // 1, 3, 5, 7
            &FDK_HUFF_PART0_NODES.ipd[..]
        }
        _ => {
            fdk_assert!(false, "FDK_ASSERT(0)");
            return HUFFDEC_NOTOK;
        }
    };

    // Partition 0
    let [p0_0, p0_1] = p0_data;
    if let Some(p) = p0_0 {
        let err = huff_read(strm, node_tab, &mut node);
        if err != HUFFDEC_OK {
            return err;
        }
        *p = (-(node + 1)) as i8;
    }
    if let Some(p) = p0_1 {
        let err = huff_read(strm, node_tab, &mut node);
        if err != HUFFDEC_OK {
            return err;
        }
        *p = (-(node + 1)) as i8;
    }

    let (d1, d2) = (dim1 as usize, dim2 as usize);
    match data_type {
        t_CLD => {
            let t = FDK_HUFF_CLD_NODES.h2d[d1][d2];
            match lav {
                3 => node_tab = &t.lav3[..],
                5 => node_tab = &t.lav5[..],
                7 => node_tab = &t.lav7[..],
                9 => node_tab = &t.lav9[..],
                _ => {}
            }
        }
        t_ICC => {
            let t = FDK_HUFF_ICC_NODES.h2d[d1][d2];
            match lav {
                1 => node_tab = &t.lav1[..],
                3 => node_tab = &t.lav3[..],
                5 => node_tab = &t.lav5[..],
                7 => node_tab = &t.lav7[..],
                _ => {}
            }
        }
        t_OLD => {
            let t = HUFF_OLD_NODES.h2d[d1][d2];
            match lav {
                3 => node_tab = &t.lav3[..],
                6 => node_tab = &t.lav6[..],
                9 => node_tab = &t.lav9[..],
                12 => node_tab = &t.lav12[..],
                _ => {}
            }
        }
        t_IPD => {
            let t = &FDK_HUFF_IPD_NODES.h2d[d1][d2];
            match lav {
                1 => node_tab = &t.lav1[..],
                3 => node_tab = &t.lav3[..],
                5 => node_tab = &t.lav5[..],
                7 => node_tab = &t.lav7[..],
                _ => {}
            }
        }
        _ => {}
    }

    let mut i = 0;
    while i < num_val {
        let out = &mut out_data[i as usize];
        let err = huff_read_2d(strm, node_tab, out, &mut escape);
        if err != HUFFDEC_OK {
            return err;
        }

        if escape != 0 {
            esc_idx[esc_cntr] = i;
            esc_cntr += 1;
        } else {
            let err = if data_type == t_IPD {
                sym_restore_ipd(strm, lav, out)
            } else {
                sym_restore(strm, lav, out)
            };
            if err != HUFFDEC_OK {
                return err;
            }
        }
        i += stride;
    }

    if esc_cntr > 0 {
        let [e0, e1] = &mut esc_data;
        let err = pcm_decode(strm, Some(e0), Some(e1), 0, 2 * esc_cntr as i32, 2 * lav + 1);
        if err != HUFFDEC_OK {
            return err;
        }

        for i in 0..esc_cntr {
            out_data[esc_idx[i] as usize][0] = (esc_data[0][i] as i32 - lav) as i8;
            out_data[esc_idx[i] as usize][1] = (esc_data[1][i] as i32 - lav) as i8;
        }
    }

    HUFFDEC_OK
}

/// `huff_decode(strm, out_data_1, out_data_2, data_type, diff_type_1, diff_type_2, num_val,
/// pairing_scheme, ldMode)`
fn huff_decode(
    strm: &mut FdkBitStream,
    mut out_data_1: Option<&mut [i8]>,
    mut out_data_2: Option<&mut [i8]>,
    data_type: DataType,
    diff_type_1: DiffType,
    diff_type_2: DiffType,
    num_val: i32,
    pairing_scheme: &mut Pairing,
    ld_mode: i32,
) -> ErrorT {
    let mut pair_vec = [[0i8; 2]; MAX_PARAMETER_BANDS];

    let mut num_val_1_int = num_val;
    let mut num_val_2_int = num_val;

    // out_data_X_int = out_data_X + off_X
    let mut off_1 = 0usize;
    let mut off_2 = 0usize;

    let mut df_rest_flag_1 = 0;
    let mut df_rest_flag_2 = 0;

    // Coding scheme
    let coding_scheme = fdk_read_bits(strm, 1) as CodingScheme;

    if coding_scheme == HUFF_2D {
        if out_data_1.is_some() && out_data_2.is_some() && ld_mode == 0 {
            *pairing_scheme = fdk_read_bits(strm, 1) as Pairing;
        } else {
            *pairing_scheme = FREQ_PAIR;
        }
    }

    let huf_yy1 = diff_type_1;
    let huf_yy2 = diff_type_2;

    match coding_scheme {
        HUFF_1D => {
            let p0_flag = [(diff_type_1 == DIFF_FREQ) as i32, (diff_type_2 == DIFF_FREQ) as i32];
            if let Some(o1) = out_data_1.as_deref_mut() {
                let err = huff_dec_1d(strm, data_type, huf_yy1, o1, num_val_1_int, p0_flag[0]);
                if err != HUFFDEC_OK {
                    return err;
                }
            }
            if let Some(o2) = out_data_2.as_deref_mut() {
                let err = huff_dec_1d(strm, data_type, huf_yy2, o2, num_val_2_int, p0_flag[1]);
                if err != HUFFDEC_OK {
                    return err;
                }
            }
        }
        _ => {
            // HUFF_2D
            match *pairing_scheme {
                FREQ_PAIR => {
                    if out_data_1.is_some() {
                        if diff_type_1 == DIFF_FREQ {
                            // p0_data_1 = { &out_data_1[0], NULL }
                            num_val_1_int -= 1;
                            off_1 += 1;
                        }
                        df_rest_flag_1 = num_val_1_int % 2;
                        if df_rest_flag_1 != 0 {
                            num_val_1_int -= 1;
                        }
                        if num_val_1_int < 0 {
                            return HUFFDEC_NOTOK;
                        }
                    }
                    if out_data_2.is_some() {
                        if diff_type_2 == DIFF_FREQ {
                            // p0_data_2 = { NULL, &out_data_2[0] }
                            num_val_2_int -= 1;
                            off_2 += 1;
                        }
                        df_rest_flag_2 = num_val_2_int % 2;
                        if df_rest_flag_2 != 0 {
                            num_val_2_int -= 1;
                        }
                        if num_val_2_int < 0 {
                            return HUFFDEC_NOTOK;
                        }
                    }

                    if let Some(o1) = out_data_1.as_deref_mut() {
                        let p0 = if diff_type_1 == DIFF_FREQ { Some(&mut o1[0]) } else { None };
                        let err = huff_dec_2d(
                            strm,
                            data_type,
                            huf_yy1,
                            FREQ_PAIR,
                            &mut pair_vec[..],
                            num_val_1_int,
                            2,
                            [p0, None],
                        );
                        if err != HUFFDEC_OK {
                            return err;
                        }
                        if df_rest_flag_1 != 0 {
                            let start = off_1 + num_val_1_int as usize;
                            let err = huff_dec_1d(strm, data_type, huf_yy1, &mut o1[start..], 1, 0);
                            if err != HUFFDEC_OK {
                                return err;
                            }
                        }
                    }
                    if let Some(o2) = out_data_2.as_deref_mut() {
                        let p0 = if diff_type_2 == DIFF_FREQ { Some(&mut o2[0]) } else { None };
                        let err = huff_dec_2d(
                            strm,
                            data_type,
                            huf_yy2,
                            FREQ_PAIR,
                            &mut pair_vec[1..],
                            num_val_2_int,
                            2,
                            [None, p0],
                        );
                        if err != HUFFDEC_OK {
                            return err;
                        }
                        if df_rest_flag_2 != 0 {
                            let start = off_2 + num_val_2_int as usize;
                            let err = huff_dec_1d(strm, data_type, huf_yy2, &mut o2[start..], 1, 0);
                            if err != HUFFDEC_OK {
                                return err;
                            }
                        }
                    }

                    if let Some(o1) = out_data_1.as_deref_mut() {
                        let mut i = 0;
                        while i < num_val_1_int - 1 {
                            o1[off_1 + i as usize] = pair_vec[i as usize][0];
                            o1[off_1 + i as usize + 1] = pair_vec[i as usize][1];
                            i += 2;
                        }
                    }
                    if let Some(o2) = out_data_2.as_deref_mut() {
                        let mut i = 0;
                        while i < num_val_2_int - 1 {
                            o2[off_2 + i as usize] = pair_vec[i as usize + 1][0];
                            o2[off_2 + i as usize + 1] = pair_vec[i as usize + 1][1];
                            i += 2;
                        }
                    }
                }
                TIME_PAIR => {
                    // Only reached with both outputs (see the pairing scheme above).
                    let (Some(o1), Some(o2)) = (out_data_1, out_data_2) else {
                        return HUFFDEC_OK;
                    };
                    let has_p0 = (diff_type_1 == DIFF_FREQ) || (diff_type_2 == DIFF_FREQ);
                    if has_p0 {
                        off_1 += 1;
                        off_2 += 1;

                        num_val_1_int -= 1;
                    }

                    let diff_type = if (diff_type_1 == DIFF_TIME) || (diff_type_2 == DIFF_TIME) {
                        DIFF_TIME
                    } else {
                        DIFF_FREQ
                    };
                    let huf_yy = diff_type;

                    let (p0a, p0b) = if has_p0 {
                        let (a, b) = (&mut o1[0], &mut o2[0]);
                        (Some(a), Some(b))
                    } else {
                        (None, None)
                    };
                    let err = huff_dec_2d(
                        strm,
                        data_type,
                        huf_yy,
                        TIME_PAIR,
                        &mut pair_vec[..],
                        num_val_1_int,
                        1,
                        [p0a, p0b],
                    );
                    if err != HUFFDEC_OK {
                        return err;
                    }

                    for i in 0..num_val_1_int.max(0) as usize {
                        o1[off_1 + i] = pair_vec[i][0];
                        o2[off_2 + i] = pair_vec[i][1];
                    }
                }
                _ => {}
            }
        }
    }

    HUFFDEC_OK
}

/// `diff_freq_decode(diff_data, out_data, num_val)`
fn diff_freq_decode(diff_data: &[i8], out_data: &mut [i8], num_val: i32) {
    out_data[0] = diff_data[0];

    for i in 1..num_val.max(1) as usize {
        out_data[i] = out_data[i - 1].wrapping_add(diff_data[i]);
    }
}

/// `diff_time_decode_backwards(prev_data, diff_data, out_data, mixed_diff_type, num_val)`
fn diff_time_decode_backwards(
    prev_data: &[i8],
    diff_data: &[i8],
    out_data: &mut [i8],
    mixed_diff_type: i32,
    num_val: i32,
) {
    let mut i = 0; // default start value

    if mixed_diff_type != 0 {
        out_data[0] = diff_data[0];
        i = 1; // new start value
    }
    while i < num_val.max(0) as usize {
        out_data[i] = prev_data[i].wrapping_add(diff_data[i]);
        i += 1;
    }
}

/// `diff_time_decode_forwards(prev_data, diff_data, out_data, mixed_diff_type, num_val)`
fn diff_time_decode_forwards(
    prev_data: &[i8],
    diff_data: &[i8],
    out_data: &mut [i8],
    mixed_diff_type: i32,
    num_val: i32,
) {
    let mut i = 0; // default start value

    if mixed_diff_type != 0 {
        out_data[0] = diff_data[0];
        i = 1; // new start value
    }
    while i < num_val.max(0) as usize {
        out_data[i] = prev_data[i].wrapping_sub(diff_data[i]);
        i += 1;
    }
}

/// `attach_lsb(strm, in_data_msb, offset, num_lsb, num_val, out_data)` in place
/// (`in_data_msb == out_data` at both call sites).
fn attach_lsb(
    strm: &mut FdkBitStream,
    data: &mut [i8],
    offset: i32,
    num_lsb: i32,
    num_val: i32,
) -> ErrorT {
    for i in 0..num_val.max(0) as usize {
        let msb = data[i] as i32;

        if num_lsb > 0 {
            let lsb = fdk_read_bits(strm, num_lsb as u32) as i32;

            data[i] = ((msb << num_lsb) | lsb).wrapping_sub(offset) as i8;
        } else {
            data[i] = (msb - offset) as i8;
        }
    }

    HUFFDEC_OK // dummy
}

/// `EcDataPairDec(DECODER, strm, aaOutData1, aaOutData2, aHistory, data_type, startBand,
/// dataBands, pair_flag, coarse_flag, allowDiffTimeBack_flag)`: decodes one or two (pair) sets
/// of parameter data.
///
/// On success, `aa_out_data1[start_band..][..data_bands]` (and `aa_out_data2` for `pair_flag`)
/// receive the values; `a_history[start_band..][..data_bands]` holds the previous values (read
/// for time differential coding). On error nothing is written.
pub fn ec_data_pair_dec(
    decoder: DecoderType,
    strm: &mut FdkBitStream,
    aa_out_data1: &mut [i8],
    aa_out_data2: &mut [i8],
    a_history: &[i8],
    data_type: DataType,
    start_band: i32,
    data_bands: i32,
    pair_flag: i32,
    coarse_flag: i32,
    allow_diff_time_back_flag: i32,
) -> ErrorT {
    let attach_lsb_flag;
    let quant_levels;
    let quant_offset;

    let mut aa_data_pair = [[0i8; MAX_PARAMETER_BANDS]; 2];
    let mut aa_data_diff = [[0i8; MAX_PARAMETER_BANDS]; 2];

    let mut a_history_msb = [0i8; MAX_PARAMETER_BANDS];

    let mut diff_type: [DiffType; 2] = [DIFF_FREQ, DIFF_FREQ];
    let mut pairing: Pairing = FREQ_PAIR;

    match data_type {
        t_CLD => {
            if coarse_flag != 0 {
                attach_lsb_flag = 0;
                quant_levels = 15;
                quant_offset = 7;
            } else {
                attach_lsb_flag = 0;
                quant_levels = 31;
                quant_offset = 15;
            }
        }
        t_ICC => {
            if coarse_flag != 0 {
                attach_lsb_flag = 0;
                quant_levels = 4;
                quant_offset = 0;
            } else {
                attach_lsb_flag = 0;
                quant_levels = 8;
                quant_offset = 0;
            }
        }
        t_OLD => {
            if coarse_flag != 0 {
                attach_lsb_flag = 0;
                quant_levels = 8;
                quant_offset = 0;
            } else {
                attach_lsb_flag = 0;
                quant_levels = 16;
                quant_offset = 0;
            }
        }
        t_NRG => {
            if coarse_flag != 0 {
                attach_lsb_flag = 0;
                quant_levels = 32;
                quant_offset = 0;
            } else {
                attach_lsb_flag = 0;
                quant_levels = 64;
                quant_offset = 0;
            }
        }
        t_IPD => {
            if coarse_flag == 0 {
                attach_lsb_flag = 1;
                quant_levels = 16;
                quant_offset = 0;
            } else {
                attach_lsb_flag = 0;
                quant_levels = 8;
                quant_offset = 0;
            }
        }
        _ => return HUFFDEC_NOTOK,
    }

    // Deviation (see the module documentation): the local arrays hold MAX_PARAMETER_BANDS values.
    if !(0..=MAX_PARAMETER_BANDS as i32).contains(&data_bands) {
        return HUFFDEC_NOTOK;
    }

    let pcm_coding_flag = fdk_read_bits(strm, 1);

    if pcm_coding_flag != 0 {
        let num_val_pcm;
        let [p0, p1] = &mut aa_data_pair;
        let (v0, v1): (&mut [i8], Option<&mut [i8]>) = if pair_flag != 0 {
            num_val_pcm = 2 * data_bands;
            (p0, Some(p1))
        } else {
            num_val_pcm = data_bands;
            (p0, None)
        };

        let err = pcm_decode(strm, Some(v0), v1, quant_offset, num_val_pcm, quant_levels);
        if err != HUFFDEC_OK {
            return HUFFDEC_NOTOK;
        }
    } else {
        // Differential/Huffman/LSB Coding

        diff_type[0] = DIFF_FREQ;
        diff_type[1] = DIFF_FREQ;

        let mut direction: Direction = BACKWARDS;
        {
            if pair_flag != 0 || allow_diff_time_back_flag != 0 {
                diff_type[0] = fdk_read_bits(strm, 1) as DiffType;
            }

            if pair_flag != 0 && ((diff_type[0] == DIFF_FREQ) || allow_diff_time_back_flag != 0) {
                diff_type[1] = fdk_read_bits(strm, 1) as DiffType;
            }
        }
        // Huffman decoding
        let err = {
            let [d0, d1] = &mut aa_data_diff;
            let v1: Option<&mut [i8]> = if pair_flag != 0 { Some(d1) } else { None };
            huff_decode(
                strm,
                Some(d0),
                v1,
                data_type,
                diff_type[0],
                diff_type[1],
                data_bands,
                &mut pairing,
                (decoder == SAOC_DECODER) as i32,
            )
        };
        if err != HUFFDEC_OK {
            return HUFFDEC_NOTOK;
        }

        {
            // Differential decoding
            if (diff_type[0] == DIFF_TIME) || (diff_type[1] == DIFF_TIME) {
                if decoder == SAOC_DECODER {
                    direction = BACKWARDS;
                } else {
                    if pair_flag != 0 {
                        if (diff_type[0] == DIFF_TIME) && allow_diff_time_back_flag == 0 {
                            direction = FORWARDS;
                        } else if diff_type[1] == DIFF_TIME {
                            direction = BACKWARDS;
                        } else {
                            direction = fdk_read_bits(strm, 1) as Direction;
                        }
                    } else {
                        direction = BACKWARDS;
                    }
                }
            }

            let mixed_time_pair = ((diff_type[0] != diff_type[1]) && (pairing == TIME_PAIR)) as i32;

            let [pair0, pair1] = &mut aa_data_pair;
            if direction == BACKWARDS {
                if diff_type[0] == DIFF_FREQ {
                    diff_freq_decode(&aa_data_diff[0], pair0, data_bands);
                } else {
                    for i in 0..data_bands as usize {
                        a_history_msb[i] =
                            (a_history[i + start_band as usize] as i32 + quant_offset) as i8;
                        if attach_lsb_flag != 0 {
                            a_history_msb[i] >>= 1;
                        }
                    }
                    diff_time_decode_backwards(
                        &a_history_msb,
                        &aa_data_diff[0],
                        pair0,
                        mixed_time_pair,
                        data_bands,
                    );
                }
                if diff_type[1] == DIFF_FREQ {
                    diff_freq_decode(&aa_data_diff[1], pair1, data_bands);
                } else {
                    diff_time_decode_backwards(
                        pair0,
                        &aa_data_diff[1],
                        pair1,
                        mixed_time_pair,
                        data_bands,
                    );
                }
            } else {
                // diff_type[1] MUST BE DIFF_FREQ
                diff_freq_decode(&aa_data_diff[1], pair1, data_bands);

                if diff_type[0] == DIFF_FREQ {
                    diff_freq_decode(&aa_data_diff[0], pair0, data_bands);
                } else {
                    diff_time_decode_forwards(
                        pair1,
                        &aa_data_diff[0],
                        pair0,
                        mixed_time_pair,
                        data_bands,
                    );
                }
            }
        }

        // LSB decoding
        let err = attach_lsb(strm, &mut aa_data_pair[0], quant_offset, attach_lsb_flag, data_bands);
        if err != HUFFDEC_OK {
            return err;
        }

        if pair_flag != 0 {
            let err =
                attach_lsb(strm, &mut aa_data_pair[1], quant_offset, attach_lsb_flag, data_bands);
            if err != HUFFDEC_OK {
                return err;
            }
        }
    } // End: Differential/Huffman/LSB Coding

    // Copy data to output arrays
    let (s, n) = (start_band as usize, data_bands as usize);
    aa_out_data1[s..s + n].copy_from_slice(&aa_data_pair[0][..n]);
    if pair_flag != 0 {
        aa_out_data2[s..s + n].copy_from_slice(&aa_data_pair[1][..n]);
    }

    HUFFDEC_OK
}

/// `huff_dec_reshape(strm, out_data, num_val)`: run length decoding of `num_val` values (for
/// the GES and STP tools). `out_data` holds at least `num_val` values.
pub fn huff_dec_reshape(strm: &mut FdkBitStream, out_data: &mut [i32], num_val: i32) -> ErrorT {
    let mut val_rcvd = 0;
    let mut dummy = 0;
    let mut rl_data = [0i8; 2];

    while val_rcvd < num_val {
        let err =
            huff_read_2d(strm, &FDK_HUFF_RESHAPE_NODES.node_tab[..], &mut rl_data, &mut dummy);
        if err != HUFFDEC_OK {
            return err;
        }
        let val = rl_data[0] as i32;
        let len = rl_data[1] as i32 + 1;
        if val_rcvd + len > num_val {
            return HUFFDEC_NOTOK;
        }
        for i in val_rcvd..val_rcvd + len {
            out_data[i as usize] = val;
        }
        val_rcvd += len;
    }
    HUFFDEC_OK
}
