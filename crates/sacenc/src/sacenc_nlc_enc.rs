// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_nlc_enc.h, libSACenc/src/sacenc_nlc_enc.cpp.

//! Port of `sacenc_nlc_enc.h` and `sacenc_nlc_enc.cpp`: the noiseless coding (grouped PCM and
//! Huffman coding of frequency and time differences) of CLD and ICC data.
//!
//! The functions that count bits instead of writing them take `None` for the `NULL`
//! bitstream (`FDKwriteBits(NULL, ...)` returns the number of bits).
//!
//! # Variants
//!
//! The code is identical in the `aarch64` and `x86` builds; it uses no arithmetic primitive
//! whose result differs (`fAbs(FIXP_SGL)` truncates both variants of `fixabs_S` to the same
//! value).
//!
//! # Preconditions
//!
//! The data must lie in the ranges of the quantizers (CLD indices -15..=15, ICC indices 0..=7,
//! as the parameter extraction produces them); the C code indexes its Huffman tables with
//! these values. The `SHORT` arithmetic is reproduced with 16 bit truncation.

use fdk_aac_rs_fdk::bitstream::{fdk_write_bits, fdk_write_bits_opt, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::f_abs;

use crate::sacenc_bitstream::MAX_NUM_BINS;
use crate::sacenc_huff_tab::*;

mod tables;
pub use tables::*;

/// `MAXBANDS`: maximum number of frequency bands
pub const MAXBANDS: usize = MAX_NUM_BINS;

/// `DATA_TYPE`
pub type DataType = i32;
/// `t_CLD`
pub const t_CLD: DataType = 0;
/// `t_ICC`
pub const t_ICC: DataType = 1;

/// `PAIR_SHIFT`
const PAIR_SHIFT: i32 = 4;
/// `PAIR_MASK`
const PAIR_MASK: i32 = 0xf;

/// `DIFF_TYPE`
pub type DiffType = i32;
/// `DIFF_FREQ`
pub const DIFF_FREQ: DiffType = 0x0;
/// `DIFF_TIME`
pub const DIFF_TIME: DiffType = 0x1;

/// `CODING_SCHEME`: `HUFF_1D`
pub const HUFF_1D: i32 = 0x0;
/// `CODING_SCHEME`: `HUFF_2D`
pub const HUFF_2D: i32 = 0x1;

/// `PAIRING`: `FREQ_PAIR`
pub const FREQ_PAIR: i32 = 0x0;
/// `PAIRING`: `TIME_PAIR`
pub const TIME_PAIR: i32 = 0x1;

/// `split_lsb(in_data, offset, num_val, out_data_lsb, out_data_msb)` (static in C)
pub fn split_lsb(
    in_data: &[i16],
    offset: i16,
    num_val: i32,
    mut out_data_lsb: Option<&mut [i16]>,
    mut out_data_msb: Option<&mut [i16]>,
) {
    for i in 0..num_val.max(0) as usize {
        let val = (in_data[i] as i32 + offset as i32) as i16;
        if let Some(o) = out_data_lsb.as_deref_mut() {
            o[i] = val & 0x0001;
        }
        if let Some(o) = out_data_msb.as_deref_mut() {
            o[i] = val >> 1;
        }
    }
}

/// `apply_lsb_coding(strm, in_data_lsb, num_lsb, num_val)` (static in C)
pub fn apply_lsb_coding(strm: &mut FdkBitStream, in_data_lsb: &[i16], num_lsb: u32, num_val: i32) {
    for i in 0..num_val.max(0) as usize {
        fdk_write_bits(strm, in_data_lsb[i] as i32 as u32, num_lsb);
    }
}

/// `calc_diff_freq(in_data, out_data, num_val)` (static in C)
pub fn calc_diff_freq(in_data: &[i16], out_data: &mut [i16], num_val: i32) {
    out_data[0] = in_data[0];

    for i in 1..num_val.max(0) as usize {
        out_data[i] = (in_data[i] as i32 - in_data[i - 1] as i32) as i16;
    }
}

/// `calc_diff_time(in_data, prev_data, out_data, num_val)` (static in C)
pub fn calc_diff_time(in_data: &[i16], prev_data: &[i16], out_data: &mut [i16], num_val: i32) {
    out_data[0] = in_data[0];
    out_data[1] = prev_data[0];

    for i in 0..num_val.max(0) as usize {
        out_data[i + 2] = (in_data[i] as i32 - prev_data[i] as i32) as i16;
    }
}

/// `sym_check(data, lav, pSym_bits)` (static in C): maps a pair to its symmetric
/// representative and returns the number of sign bits.
pub fn sym_check(data: &mut [i16; 2], lav: i32, p_sym_bits: Option<&mut i16>) -> i32 {
    let mut sym_bits: u8 = 0;
    let mut sum_val = data[0] as i32 + data[1] as i32;
    let mut diff_val = data[0] as i32 - data[1] as i32;
    let mut num_sbits = 0;

    if sum_val != 0 {
        let sum_neg = (sum_val < 0) as u8;
        if sum_neg != 0 {
            sum_val = -sum_val;
            diff_val = -diff_val;
        }
        sym_bits = (sym_bits << 1) | sum_neg;
        num_sbits += 1;
    }

    if diff_val != 0 {
        let diff_neg = (diff_val < 0) as u8;
        if diff_neg != 0 {
            diff_val = -diff_val;
        }
        sym_bits = (sym_bits << 1) | diff_neg;
        num_sbits += 1;
    }

    if let Some(p) = p_sym_bits {
        *p = sym_bits as i16;
    }

    if sum_val % 2 != 0 {
        data[0] = (lav - sum_val / 2) as i16;
        data[1] = (lav - diff_val / 2) as i16;
    } else {
        data[0] = (sum_val / 2) as i16;
        data[1] = (diff_val / 2) as i16;
    }

    num_sbits
}

/// `ilog2(i)` (static in C)
pub fn ilog2(mut i: u32) -> i32 {
    let mut l = 0;

    i = i.saturating_sub(1);
    while i > 0 {
        i >>= 1;
        l += 1;
    }

    l
}

/// `calc_pcm_bits(num_val, num_levels)` (static in C)
pub fn calc_pcm_bits(num_val: i16, num_levels: i16) -> i16 {
    let max_grp_len: i16 = match num_levels {
        3 => 5,
        6 => 5,
        7 => 6,
        11 => 2,
        13 => 4,
        19 => 4,
        25 => 3,
        51 => 4,
        _ => 1,
    };

    let num_complete_chunks = (num_val as i32 / max_grp_len as i32) as i16;
    let rest_chunk_size = (num_val as i32 % max_grp_len as i32) as i16;

    let mut chunk_levels: i32 = 1;
    for _ in 1..=max_grp_len {
        chunk_levels = chunk_levels.wrapping_mul(num_levels as i32);
    }

    let mut bits_pcm = ilog2(chunk_levels as u32).wrapping_mul(num_complete_chunks as i32) as i16;
    bits_pcm = (bits_pcm as i32
        + ilog2(num_levels as i32 as u32).wrapping_mul(rest_chunk_size as i32) as i16 as i32)
        as i16;

    bits_pcm
}

/// `apply_pcm_coding(strm, in_data_1, in_data_2, offset, num_val, num_levels)` (static in C).
/// With both inputs, the values are taken alternately from `in_data_1` and `in_data_2`.
pub fn apply_pcm_coding(
    strm: &mut FdkBitStream,
    in_data_1: Option<&[i16]>,
    in_data_2: Option<&[i16]>,
    offset: i16,
    num_val: i16,
    num_levels: i16,
) {
    let mut pcm_chunk_size = [0i16; 7];

    let max_grp_len: i16 = match num_levels {
        3 => 5,
        5 => 3,
        6 => 5,
        7 => 6,
        9 => 5,
        11 => 2,
        13 => 4,
        19 => 4,
        25 => 3,
        51 => 4,
        _ => 1,
    };

    let mut chunk_levels: i32 = 1;
    for i in 1..=max_grp_len as usize {
        chunk_levels = chunk_levels.wrapping_mul(num_levels as i32);
        pcm_chunk_size[i] = ilog2(chunk_levels as u32) as i16;
    }

    let mut i: i16 = 0;
    while i < num_val {
        let grp_len = max_grp_len.min((num_val as i32 - i as i32) as i16);
        let mut grp_val: i32 = 0;
        for j in 0..grp_len {
            let idx = (i as i32 + j as i32) as i16;
            let mut next_val: i16 = match (in_data_1, in_data_2) {
                (Some(d1), None) => d1[idx as usize],
                (None, Some(d2)) => d2[idx as usize],
                (Some(d1), Some(d2)) => {
                    if idx % 2 != 0 {
                        d2[(idx / 2) as usize]
                    } else {
                        d1[(idx / 2) as usize]
                    }
                }
                // (not defined by the reference)
                (None, None) => 0,
            };
            next_val = (next_val as i32 + offset as i32) as i16;
            grp_val = grp_val.wrapping_mul(num_levels as i32).wrapping_add(next_val as i32);
        }

        fdk_write_bits(strm, grp_val as u32, pcm_chunk_size[grp_len as usize] as i32 as u32);
        i = (i as i32 + max_grp_len as i32) as i16;
    }
}

/// `huff_enc_1D(strm, data_type, dim1, in_data, num_val, p0_flag)` (static in C)
pub fn huff_enc_1d(
    mut strm: Option<&mut FdkBitStream>,
    data_type: DataType,
    dim1: i32,
    in_data: &[i16],
    num_val: i16,
    p0_flag: i16,
) -> u32 {
    let mut offset = 0usize;
    let mut huff_bits: u32 = 0;

    let p_huff_tab: &[HuffEntry] = match data_type {
        t_CLD => &FDK_SACENC_HUFF_CLD_TAB.h1d[dim1 as usize],
        _ => &FDK_SACENC_HUFF_ICC_TAB.h1d[dim1 as usize],
    };

    if p0_flag != 0 {
        let part0 = match data_type {
            t_CLD => FDK_SACENC_HUFF_PART0_TAB.cld[in_data[0] as usize],
            _ => FDK_SACENC_HUFF_PART0_TAB.icc[in_data[0] as usize],
        };
        huff_bits +=
            fdk_write_bits_opt(strm.as_deref_mut(), huff_value(part0), huff_length(part0)) as u32;
        offset = 1;
    }

    for i in offset..num_val.max(0) as usize {
        let mut id_sign = 0u32;
        let mut id = in_data[i] as i32;

        if id != 0 {
            id_sign = 0;
            if id < 0 {
                id = -id;
                id_sign = 1;
            }
        }

        let e = p_huff_tab[id as usize];
        huff_bits += fdk_write_bits_opt(strm.as_deref_mut(), huff_value(e), huff_length(e)) as u32;

        if id != 0 {
            huff_bits += fdk_write_bits_opt(strm.as_deref_mut(), id_sign, 1) as u32;
        }
    }

    huff_bits
}

/// `getHuffEntry(lav, data_type, i, tab_idx_2D, in_data, pEntry, pEscape)` (static in C)
pub fn get_huff_entry(
    lav: i32,
    data_type: DataType,
    i: usize,
    tab_idx_2d: [i16; 2],
    in_data: &[[i16; 2]],
    p_entry: &mut HuffEntry,
    p_escape: &mut HuffEntry,
) {
    let cld = &FDK_SACENC_HUFF_CLD_TAB.h2d[tab_idx_2d[0] as usize][tab_idx_2d[1] as usize];
    let icc = &FDK_SACENC_HUFF_ICC_TAB.h2d[tab_idx_2d[0] as usize][tab_idx_2d[1] as usize];
    let (a, b) = (in_data[i][0] as usize, in_data[i][1] as usize);

    fn pick<const N: usize>(
        t: &Lav2d<N>,
        a: usize,
        b: usize,
        e: &mut HuffEntry,
        x: &mut HuffEntry,
    ) {
        *e = t.entry[a][b];
        *x = t.escape;
    }

    match lav {
        1 => {
            // No CLD table for LAV 1.
            if data_type != t_CLD {
                pick(&icc.lav1, a, b, p_entry, p_escape);
            }
        }
        3 => {
            if data_type == t_CLD {
                pick(&cld.lav3, a, b, p_entry, p_escape);
            } else {
                pick(&icc.lav3, a, b, p_entry, p_escape);
            }
        }
        5 => {
            if data_type == t_CLD {
                pick(&cld.lav5, a, b, p_entry, p_escape);
            } else {
                pick(&icc.lav5, a, b, p_entry, p_escape);
            }
        }
        7 => {
            if data_type == t_CLD {
                pick(&cld.lav7, a, b, p_entry, p_escape);
            } else {
                pick(&icc.lav7, a, b, p_entry, p_escape);
            }
        }
        9 => {
            // No ICC table for LAV 9.
            if data_type == t_CLD {
                pick(&cld.lav9, a, b, p_entry, p_escape);
            }
        }
        _ => {}
    }
}

/// `huff_enc_2D(strm, data_type, tab_idx_2D, lav_idx, in_data, num_val, stride, p0_data)`
/// (static in C). `p0_data` are the values that the C pointers `p0_data[]` point to.
/// `in_data` is changed (`sym_check()`).
pub fn huff_enc_2d(
    mut strm: Option<&mut FdkBitStream>,
    data_type: DataType,
    tab_idx_2d: [i16; 2],
    lav_idx: i16,
    in_data: &mut [[i16; 2]],
    num_val: i16,
    stride: i16,
    p0_data: [Option<i16>; 2],
) -> u32 {
    let mut esc_idx: i16 = 0;
    let mut esc_data = [[0i16; MAXBANDS]; 2];

    let mut huff_bits: u32 = 0;

    let (lav, p_huff_entry): (i16, &[HuffEntry]) = match data_type {
        t_CLD => ((2 * lav_idx as i32 + 3) as i16, &FDK_SACENC_HUFF_PART0_TAB.cld[..]),
        _ => ((2 * lav_idx as i32 + 1) as i16, &FDK_SACENC_HUFF_PART0_TAB.icc[..]),
    };

    // Partition 0
    for p in p0_data.into_iter().flatten() {
        let entry = p_huff_entry[p as usize];
        huff_bits +=
            fdk_write_bits_opt(strm.as_deref_mut(), huff_value(entry), huff_length(entry)) as u32;
    }

    let mut i: i16 = 0;
    while i < num_val {
        let mut entry = HuffEntry::default();
        let mut escape = HuffEntry::default();
        let mut sym_bits: i16 = 0;

        esc_data[0][esc_idx as usize] = (in_data[i as usize][0] as i32 + lav as i32) as i16;
        esc_data[1][esc_idx as usize] = (in_data[i as usize][1] as i32 + lav as i32) as i16;

        let num_sbits = sym_check(&mut in_data[i as usize], lav as i32, Some(&mut sym_bits)) as i16;

        get_huff_entry(
            lav as i32,
            data_type,
            i as usize,
            tab_idx_2d,
            in_data,
            &mut entry,
            &mut escape,
        );

        huff_bits +=
            fdk_write_bits_opt(strm.as_deref_mut(), huff_value(entry), huff_length(entry)) as u32;

        if (huff_value(entry) == huff_value(escape)) && (huff_length(entry) == huff_length(escape))
        {
            esc_idx += 1;
        } else {
            huff_bits += fdk_write_bits_opt(
                strm.as_deref_mut(),
                sym_bits as i32 as u32,
                num_sbits as i32 as u32,
            ) as u32;
        }
        i = (i as i32 + stride as i32) as i16;
    }

    if esc_idx > 0 {
        huff_bits = huff_bits
            .wrapping_add(calc_pcm_bits((2 * esc_idx as i32) as i16, (2 * lav as i32 + 1) as i16)
                as i32 as u32);
        if let Some(s) = strm {
            apply_pcm_coding(
                s,
                Some(&esc_data[0]),
                Some(&esc_data[1]),
                0, // offset
                (2 * esc_idx as i32) as i16,
                (2 * lav as i32 + 1) as i16,
            );
        }
    }

    huff_bits
}

/// `get_next_lav_step(lav, data_type)` (static in C)
pub fn get_next_lav_step(lav: i32, data_type: DataType) -> i8 {
    match data_type {
        t_CLD => {
            if lav > 9 {
                -1
            } else {
                LAV_STEP_CLD[lav as usize] as i8
            }
        }
        _ => {
            if lav > 7 {
                -1
            } else {
                LAV_STEP_ICC[lav as usize] as i8
            }
        }
    }
}

/// `diff_type_offset(diff_type)` (static in C)
pub fn diff_type_offset(diff_type: DiffType) -> usize {
    match diff_type {
        DIFF_TIME => 2,
        _ => 0,
    }
}

/// `in_data_short + num_val_short`: the value after the pairs (`df_rest_flag`). Bounded:
/// indices outside the array read 0.
fn rest_value(d: &[i16], off: usize, num_val_short: i16) -> [i16; 1] {
    let idx = off as isize + num_val_short as isize;
    [usize::try_from(idx).ok().and_then(|i| d.get(i)).copied().unwrap_or(0)]
}

/// `SHORT += UINT`: the sum converted to `SHORT`.
#[inline(always)]
fn add_s(a: i16, b: u32) -> i16 {
    (a as i32 as u32).wrapping_add(b) as i16
}

/// `calc_huff_bits(in_data_1, in_data_2, data_type, diff_type_1, diff_type_2, num_val,
/// lav_idx, cdg_scheme)` (static in C): the bits of the cheapest Huffman coding scheme (1D or
/// 2D frequency pairs), which is returned in `lav_idx` and `cdg_scheme`.
pub fn calc_huff_bits(
    in_data_1: Option<&[i16]>,
    in_data_2: Option<&[i16]>,
    data_type: DataType,
    diff_type_1: DiffType,
    diff_type_2: DiffType,
    num_val: i16,
    lav_idx: &mut [i16; 2],
    cdg_scheme: &mut i16,
) -> i16 {
    let mut tab_idx_2d = [[0i16; 2]; 2];
    let mut tab_idx_1d = [0i16; 2];
    let mut df_rest_flag = [0i16; 2];
    let mut p0_flag = [0i16; 2];

    let mut pair_vec = [[0i16; 2]; MAXBANDS];

    let mut p0_data_1: [Option<i16>; 2] = [None; 2];
    let mut p0_data_2: [Option<i16>; 2] = [None; 2];

    let mut lav_fp = [0i16; 2];

    // 1D Huffman coding
    let mut bit_count_1d: i16 = 1; // HUFF_1D

    let mut num_val_1_short = num_val;
    let mut num_val_2_short = num_val;

    // in_data_1_short, in_data_2_short: offsets into in_data_1, in_data_2
    let mut in_1_off = diff_type_offset(diff_type_1);
    let mut in_2_off = diff_type_offset(diff_type_2);

    p0_flag[0] = (diff_type_1 == DIFF_FREQ) as i16;
    p0_flag[1] = (diff_type_2 == DIFF_FREQ) as i16;

    tab_idx_1d[0] = if diff_type_1 == DIFF_FREQ { 0 } else { 1 };
    tab_idx_1d[1] = if diff_type_2 == DIFF_FREQ { 0 } else { 1 };

    if let Some(d1) = in_data_1 {
        bit_count_1d = add_s(
            bit_count_1d,
            huff_enc_1d(
                None,
                data_type,
                tab_idx_1d[0] as i32,
                &d1[in_1_off..],
                num_val_1_short,
                p0_flag[0],
            ),
        );
    }
    if let Some(d2) = in_data_2 {
        bit_count_1d = add_s(
            bit_count_1d,
            huff_enc_1d(
                None,
                data_type,
                tab_idx_1d[1] as i32,
                &d2[in_2_off..],
                num_val_2_short,
                p0_flag[1],
            ),
        );
    }

    let mut bit_count_min = bit_count_1d;
    *cdg_scheme = (HUFF_1D << PAIR_SHIFT) as i16;
    lav_idx[0] = -1;
    lav_idx[1] = -1;

    // Huffman 2D frequency pairs
    let mut bit_count_2d_freq: i16 = 1; // HUFF_2D

    num_val_1_short = num_val;
    num_val_2_short = num_val;

    in_1_off = diff_type_offset(diff_type_1);
    in_2_off = diff_type_offset(diff_type_2);

    if let Some(d1) = in_data_1 {
        if diff_type_1 == DIFF_FREQ {
            p0_data_1[0] = Some(d1[0]);
            p0_data_1[1] = None;

            num_val_1_short -= 1;
            in_1_off += 1;
        }

        df_rest_flag[0] = num_val_1_short % 2;

        if df_rest_flag[0] != 0 {
            num_val_1_short -= 1;
        }

        let mut i = 0i32;
        while i < num_val_1_short as i32 - 1 {
            let k = i as usize;
            pair_vec[k][0] = d1[in_1_off + k];
            pair_vec[k][1] = d1[in_1_off + k + 1];

            lav_fp[0] = lav_fp[0].max(f_abs(pair_vec[k][0]));
            lav_fp[0] = lav_fp[0].max(f_abs(pair_vec[k][1]));
            i += 2;
        }

        tab_idx_2d[0][0] = if diff_type_1 == DIFF_TIME { 1 } else { 0 };
        tab_idx_2d[0][1] = 0;

        tab_idx_1d[0] = if diff_type_1 == DIFF_FREQ { 0 } else { 1 };

        lav_fp[0] = get_next_lav_step(lav_fp[0] as i32, data_type) as i16;

        if lav_fp[0] != -1 {
            bit_count_2d_freq += LAV_HUFF_LEN[lav_fp[0] as usize] as i16;
        }
    }

    if let Some(d2) = in_data_2 {
        if diff_type_2 == DIFF_FREQ {
            p0_data_2[0] = None;
            p0_data_2[1] = Some(d2[0]);

            num_val_2_short -= 1;
            in_2_off += 1;
        }

        df_rest_flag[1] = num_val_2_short % 2;

        if df_rest_flag[1] != 0 {
            num_val_2_short -= 1;
        }

        let mut i = 0i32;
        while i < num_val_2_short as i32 - 1 {
            let k = i as usize;
            pair_vec[k + 1][0] = d2[in_2_off + k];
            pair_vec[k + 1][1] = d2[in_2_off + k + 1];

            lav_fp[1] = lav_fp[1].max(f_abs(pair_vec[k + 1][0]));
            lav_fp[1] = lav_fp[1].max(f_abs(pair_vec[k + 1][1]));
            i += 2;
        }

        tab_idx_2d[1][0] = if diff_type_2 == DIFF_TIME { 1 } else { 0 };
        tab_idx_2d[1][1] = 0;

        tab_idx_1d[1] = if diff_type_2 == DIFF_FREQ { 0 } else { 1 };

        lav_fp[1] = get_next_lav_step(lav_fp[1] as i32, data_type) as i16;

        if lav_fp[1] != -1 {
            bit_count_2d_freq += LAV_HUFF_LEN[lav_fp[1] as usize] as i16;
        }
    }

    if (lav_fp[0] != -1) && (lav_fp[1] != -1) {
        if in_data_1.is_some() {
            bit_count_2d_freq = add_s(
                bit_count_2d_freq,
                huff_enc_2d(
                    None,
                    data_type,
                    tab_idx_2d[0],
                    lav_fp[0],
                    &mut pair_vec,
                    num_val_1_short,
                    2,
                    p0_data_1,
                ),
            );
        }
        if in_data_2.is_some() {
            bit_count_2d_freq = add_s(
                bit_count_2d_freq,
                huff_enc_2d(
                    None,
                    data_type,
                    tab_idx_2d[1],
                    lav_fp[1],
                    &mut pair_vec[1..],
                    num_val_2_short,
                    2,
                    p0_data_2,
                ),
            );
        }
        if let Some(d1) = in_data_1 {
            if df_rest_flag[0] != 0 {
                bit_count_2d_freq = add_s(
                    bit_count_2d_freq,
                    huff_enc_1d(
                        None,
                        data_type,
                        tab_idx_1d[0] as i32,
                        &rest_value(d1, in_1_off, num_val_1_short),
                        1,
                        0,
                    ),
                );
            }
        }
        if let Some(d2) = in_data_2 {
            if df_rest_flag[1] != 0 {
                bit_count_2d_freq = add_s(
                    bit_count_2d_freq,
                    huff_enc_1d(
                        None,
                        data_type,
                        tab_idx_1d[1] as i32,
                        &rest_value(d2, in_2_off, num_val_2_short),
                        1,
                        0,
                    ),
                );
            }
        }

        if bit_count_2d_freq < bit_count_min {
            bit_count_min = bit_count_2d_freq;
            *cdg_scheme = ((HUFF_2D << PAIR_SHIFT) | FREQ_PAIR) as i16;
            lav_idx[0] = lav_fp[0];
            lav_idx[1] = lav_fp[1];
        }
    }

    bit_count_min
}

/// `apply_huff_coding(strm, in_data_1, in_data_2, data_type, diff_type_1, diff_type_2,
/// num_val, lav_idx, cdg_scheme)` (static in C)
pub fn apply_huff_coding(
    strm: &mut FdkBitStream,
    in_data_1: Option<&[i16]>,
    in_data_2: Option<&[i16]>,
    data_type: DataType,
    diff_type_1: DiffType,
    diff_type_2: DiffType,
    num_val: i16,
    lav_idx: &[i16; 2],
    cdg_scheme: i16,
) {
    let mut tab_idx_2d = [[0i16; 2]; 2];
    let mut tab_idx_1d = [0i16; 2];
    let mut df_rest_flag = [0i16; 2];
    let mut p0_flag = [0i16; 2];

    let mut pair_vec = [[0i16; 2]; MAXBANDS];

    let mut p0_data_1: [Option<i16>; 2] = [None; 2];
    let mut p0_data_2: [Option<i16>; 2] = [None; 2];

    let mut num_val_1_short = num_val;
    let mut num_val_2_short = num_val;

    // Offset
    let mut in_1_off = diff_type_offset(diff_type_1);
    let mut in_2_off = diff_type_offset(diff_type_2);

    // Signalize coding scheme
    fdk_write_bits(strm, (cdg_scheme as i32 >> PAIR_SHIFT) as u32, 1);

    match cdg_scheme as i32 >> PAIR_SHIFT {
        HUFF_1D => {
            p0_flag[0] = (diff_type_1 == DIFF_FREQ) as i16;
            p0_flag[1] = (diff_type_2 == DIFF_FREQ) as i16;

            tab_idx_1d[0] = if diff_type_1 == DIFF_FREQ { 0 } else { 1 };
            tab_idx_1d[1] = if diff_type_2 == DIFF_FREQ { 0 } else { 1 };

            if let Some(d1) = in_data_1 {
                huff_enc_1d(
                    Some(strm),
                    data_type,
                    tab_idx_1d[0] as i32,
                    &d1[in_1_off..],
                    num_val_1_short,
                    p0_flag[0],
                );
            }
            if let Some(d2) = in_data_2 {
                huff_enc_1d(
                    Some(strm),
                    data_type,
                    tab_idx_1d[1] as i32,
                    &d2[in_2_off..],
                    num_val_2_short,
                    p0_flag[1],
                );
            }
        }

        HUFF_2D => match cdg_scheme as i32 & PAIR_MASK {
            FREQ_PAIR => {
                if let Some(d1) = in_data_1 {
                    if diff_type_1 == DIFF_FREQ {
                        p0_data_1[0] = Some(d1[0]);
                        p0_data_1[1] = None;

                        num_val_1_short -= 1;
                        in_1_off += 1;
                    }

                    df_rest_flag[0] = num_val_1_short % 2;

                    if df_rest_flag[0] != 0 {
                        num_val_1_short -= 1;
                    }

                    let mut i = 0i32;
                    while i < num_val_1_short as i32 - 1 {
                        let k = i as usize;
                        pair_vec[k][0] = d1[in_1_off + k];
                        pair_vec[k][1] = d1[in_1_off + k + 1];
                        i += 2;
                    }

                    tab_idx_2d[0][0] = if diff_type_1 == DIFF_TIME { 1 } else { 0 };
                    tab_idx_2d[0][1] = 0;

                    tab_idx_1d[0] = if diff_type_1 == DIFF_FREQ { 0 } else { 1 };
                }

                if let Some(d2) = in_data_2 {
                    if diff_type_2 == DIFF_FREQ {
                        p0_data_2[0] = None;
                        p0_data_2[1] = Some(d2[0]);

                        num_val_2_short -= 1;
                        in_2_off += 1;
                    }

                    df_rest_flag[1] = num_val_2_short % 2;

                    if df_rest_flag[1] != 0 {
                        num_val_2_short -= 1;
                    }

                    let mut i = 0i32;
                    while i < num_val_2_short as i32 - 1 {
                        let k = i as usize;
                        pair_vec[k + 1][0] = d2[in_2_off + k];
                        pair_vec[k + 1][1] = d2[in_2_off + k + 1];
                        i += 2;
                    }

                    tab_idx_2d[1][0] = if diff_type_2 == DIFF_TIME { 1 } else { 0 };
                    tab_idx_2d[1][1] = 0;

                    tab_idx_1d[1] = if diff_type_2 == DIFF_FREQ { 0 } else { 1 };
                }

                if let Some(d1) = in_data_1 {
                    let l = lav_idx[0] as usize;
                    fdk_write_bits(strm, LAV_HUFF_VAL[l] as u32, LAV_HUFF_LEN[l] as u32);
                    huff_enc_2d(
                        Some(strm),
                        data_type,
                        tab_idx_2d[0],
                        lav_idx[0],
                        &mut pair_vec,
                        num_val_1_short,
                        2,
                        p0_data_1,
                    );
                    if df_rest_flag[0] != 0 {
                        huff_enc_1d(
                            Some(strm),
                            data_type,
                            tab_idx_1d[0] as i32,
                            &rest_value(d1, in_1_off, num_val_1_short),
                            1,
                            0,
                        );
                    }
                }
                if let Some(d2) = in_data_2 {
                    let l = lav_idx[1] as usize;
                    fdk_write_bits(strm, LAV_HUFF_VAL[l] as u32, LAV_HUFF_LEN[l] as u32);
                    huff_enc_2d(
                        Some(strm),
                        data_type,
                        tab_idx_2d[1],
                        lav_idx[1],
                        &mut pair_vec[1..],
                        num_val_2_short,
                        2,
                        p0_data_2,
                    );
                    if df_rest_flag[1] != 0 {
                        huff_enc_1d(
                            Some(strm),
                            data_type,
                            tab_idx_1d[1] as i32,
                            &rest_value(d2, in_2_off, num_val_2_short),
                            1,
                            0,
                        );
                    }
                }
            }

            TIME_PAIR => {
                // Not produced by calc_huff_bits(). Where the reference has no defined result, a
                // missing input reads as 0.
                let d1 = in_data_1.unwrap_or(&[0; MAXBANDS + 2]);
                let d2 = in_data_2.unwrap_or(&[0; MAXBANDS + 2]);
                if (diff_type_1 == DIFF_FREQ) || (diff_type_2 == DIFF_FREQ) {
                    p0_data_1[0] = Some(d1[0]);
                    p0_data_1[1] = Some(d2[0]);

                    in_1_off += 1;
                    in_2_off += 1;

                    num_val_1_short -= 1;
                }

                for i in 0..num_val_1_short.max(0) as usize {
                    pair_vec[i][0] = d1[in_1_off + i];
                    pair_vec[i][1] = d2[in_2_off + i];
                }

                tab_idx_2d[0][0] =
                    if (diff_type_1 == DIFF_TIME) || (diff_type_2 == DIFF_TIME) { 1 } else { 0 };
                tab_idx_2d[0][1] = 1;

                let l = lav_idx[0] as usize;
                fdk_write_bits(strm, LAV_HUFF_VAL[l] as u32, LAV_HUFF_LEN[l] as u32);

                huff_enc_2d(
                    Some(strm),
                    data_type,
                    tab_idx_2d[0],
                    lav_idx[0],
                    &mut pair_vec,
                    num_val_1_short,
                    1,
                    p0_data_1,
                );
            }
            _ => {}
        },

        _ => {}
    }
}

/// The quantizer of a data type: (`quant_levels`, `quant_offset`).
fn quantizer(data_type: DataType, coarse_flag: i32) -> (i16, i16) {
    match data_type {
        t_CLD => {
            if coarse_flag != 0 {
                (15, 7)
            } else {
                (31, 15)
            }
        }
        _ => {
            if coarse_flag != 0 {
                (4, 0)
            } else {
                (8, 0)
            }
        }
    }
}

/// `fdk_sacenc_ecDataPairEnc(strm, aaInData, aHistory, data_type, setIdx, startBand,
/// dataBands, coarse_flag, independency_flag)`: encodes the data pair
/// `aaInData[setIdx..=setIdx + 1][startBand..startBand + dataBands]`.
pub fn fdk_sacenc_ec_data_pair_enc(
    strm: &mut FdkBitStream,
    aa_in_data: &[[i16; MAXBANDS]],
    a_history: &[i16; MAXBANDS],
    data_type: DataType,
    set_idx: i32,
    start_band: i32,
    data_bands: i32,
    coarse_flag: i32,
    independency_flag: i32,
) -> i32 {
    let reset: i16 = 0;
    let set_idx_u = set_idx as usize;
    let sb = start_band as usize;
    let nb = data_bands.max(0) as usize;

    // splitLsb_flag is 0 for every data type.
    let split_lsb_flag: i16 = 0;

    let allow_diff_time_back_flag = (independency_flag == 0) || (set_idx > 0);

    let (quant_levels, quant_offset) = quantizer(data_type, coarse_flag);

    let mut quant_data_lsb = [[0i16; MAXBANDS]; 2];
    let mut quant_data_msb = [[0i16; MAXBANDS]; 2];
    let mut quant_data_hist_lsb = [0i16; MAXBANDS];
    let mut quant_data_hist_msb = [0i16; MAXBANDS];

    let mut data_diff_freq = [[0i16; MAXBANDS]; 2];
    let mut data_diff_time = [[0i16; MAXBANDS + 2]; 2];

    let num_lsb_bits: i16;

    // Split off LSB
    if split_lsb_flag != 0 {
        let (l0, l1) = quant_data_lsb.split_at_mut(1);
        let (m0, m1) = quant_data_msb.split_at_mut(1);
        split_lsb(
            &aa_in_data[set_idx_u][sb..],
            quant_offset,
            data_bands,
            Some(&mut l0[0]),
            Some(&mut m0[0]),
        );
        split_lsb(
            &aa_in_data[set_idx_u + 1][sb..],
            quant_offset,
            data_bands,
            Some(&mut l1[0]),
            Some(&mut m1[0]),
        );
        num_lsb_bits = (2 * data_bands) as i16;
    } else {
        // With quant_offset == 0, p_quant_data_msb points into aaInData, which is the same as
        // a copy of the values.
        for pb in 0..nb {
            quant_data_msb[0][pb] =
                (aa_in_data[set_idx_u][sb + pb] as i32 + quant_offset as i32) as i16;
            quant_data_msb[1][pb] =
                (aa_in_data[set_idx_u + 1][sb + pb] as i32 + quant_offset as i32) as i16;
        }
        num_lsb_bits = 0;
    }

    if allow_diff_time_back_flag {
        if split_lsb_flag != 0 {
            split_lsb(
                &a_history[sb..],
                quant_offset,
                data_bands,
                Some(&mut quant_data_hist_lsb),
                Some(&mut quant_data_hist_msb),
            );
        } else {
            for pb in 0..nb {
                quant_data_hist_msb[pb] = (a_history[sb + pb] as i32 + quant_offset as i32) as i16;
            }
        }
    }

    // Calculate frequency differences
    calc_diff_freq(&quant_data_msb[0], &mut data_diff_freq[0], data_bands);
    calc_diff_freq(&quant_data_msb[1], &mut data_diff_freq[1], data_bands);

    // Calculate time differences
    if allow_diff_time_back_flag {
        calc_diff_time(
            &quant_data_msb[0],
            &quant_data_hist_msb,
            &mut data_diff_time[0],
            data_bands,
        );
    }

    calc_diff_time(&quant_data_msb[1], &quant_data_msb[0], &mut data_diff_time[1], data_bands);

    // Calculate coding scheme with minumum bit consumption
    let num_pcm_bits = calc_pcm_bits((2 * data_bands) as i16, quant_levels);
    let num_pcm_val = (2 * data_bands) as i16;

    let mut min_bits_all = num_pcm_bits;

    let mut lav_df_df = [-1i16; 2];
    let mut lav_df_dt = [-1i16; 2];
    let mut lav_dtbw_df = [-1i16; 2];
    let mut lav_dt_dt = [-1i16; 2];

    let mut coding_scheme_df_df: i16 = 0;
    let mut coding_scheme_df_dt: i16 = 0;
    let mut coding_scheme_dtbw_df: i16 = 0;
    let mut coding_scheme_dt_dt: i16 = 0;

    let mut min_bits_dtbw_df: i16 = -1;
    let mut min_bits_dt_dt: i16 = -1;

    let nv = data_bands as i16;

    let mut min_bits_df_df = calc_huff_bits(
        Some(&data_diff_freq[0]),
        Some(&data_diff_freq[1]),
        data_type,
        DIFF_FREQ,
        DIFF_FREQ,
        nv,
        &mut lav_df_df,
        &mut coding_scheme_df_df,
    );
    min_bits_df_df = (min_bits_df_df as i32 + 2 + num_lsb_bits as i32) as i16;
    if min_bits_df_df < min_bits_all {
        min_bits_all = min_bits_df_df;
    }

    let mut min_bits_df_dt = calc_huff_bits(
        Some(&data_diff_freq[0]),
        Some(&data_diff_time[1]),
        data_type,
        DIFF_FREQ,
        DIFF_TIME,
        nv,
        &mut lav_df_dt,
        &mut coding_scheme_df_dt,
    );
    min_bits_df_dt = (min_bits_df_dt as i32 + 2 + num_lsb_bits as i32) as i16;
    if min_bits_df_dt < min_bits_all {
        min_bits_all = min_bits_df_dt;
    }

    if allow_diff_time_back_flag {
        min_bits_dtbw_df = calc_huff_bits(
            Some(&data_diff_time[0]),
            Some(&data_diff_freq[1]),
            data_type,
            DIFF_TIME,
            DIFF_FREQ,
            nv,
            &mut lav_dtbw_df,
            &mut coding_scheme_dtbw_df,
        );
        min_bits_dtbw_df = (min_bits_dtbw_df as i32 + 2 + num_lsb_bits as i32) as i16;
        if min_bits_dtbw_df < min_bits_all {
            min_bits_all = min_bits_dtbw_df;
        }

        min_bits_dt_dt = calc_huff_bits(
            Some(&data_diff_time[0]),
            Some(&data_diff_time[1]),
            data_type,
            DIFF_TIME,
            DIFF_TIME,
            nv,
            &mut lav_dt_dt,
            &mut coding_scheme_dt_dt,
        );
        min_bits_dt_dt = (min_bits_dt_dt as i32 + 2 + num_lsb_bits as i32) as i16;
        if min_bits_dt_dt < min_bits_all {
            min_bits_all = min_bits_dt_dt;
        }
    }

    // Start actual coding now

    // PCM or Diff/Huff Coding?
    let pcm_coding_flag = min_bits_all == num_pcm_bits;

    fdk_write_bits(strm, pcm_coding_flag as u32, 1);

    if pcm_coding_flag {
        // Grouped PCM Coding
        apply_pcm_coding(
            strm,
            Some(&aa_in_data[set_idx_u][sb..]),
            Some(&aa_in_data[set_idx_u + 1][sb..]),
            quant_offset,
            num_pcm_val,
            quant_levels,
        );
    } else {
        // Diff/Huff Coding
        let mut min_found = false;

        if min_bits_all == min_bits_df_df {
            fdk_write_bits(strm, DIFF_FREQ as u32, 1);
            fdk_write_bits(strm, DIFF_FREQ as u32, 1);

            apply_huff_coding(
                strm,
                Some(&data_diff_freq[0]),
                Some(&data_diff_freq[1]),
                data_type,
                DIFF_FREQ,
                DIFF_FREQ,
                nv,
                &lav_df_df,
                coding_scheme_df_df,
            );

            min_found = true;
        }

        if !min_found && (min_bits_all == min_bits_df_dt) {
            fdk_write_bits(strm, DIFF_FREQ as u32, 1);
            fdk_write_bits(strm, DIFF_TIME as u32, 1);

            apply_huff_coding(
                strm,
                Some(&data_diff_freq[0]),
                Some(&data_diff_time[1]),
                data_type,
                DIFF_FREQ,
                DIFF_TIME,
                nv,
                &lav_df_dt,
                coding_scheme_df_dt,
            );

            min_found = true;
        }

        if allow_diff_time_back_flag {
            if !min_found && (min_bits_all == min_bits_dtbw_df) {
                fdk_write_bits(strm, DIFF_TIME as u32, 1);
                fdk_write_bits(strm, DIFF_FREQ as u32, 1);

                apply_huff_coding(
                    strm,
                    Some(&data_diff_time[0]),
                    Some(&data_diff_freq[1]),
                    data_type,
                    DIFF_TIME,
                    DIFF_FREQ,
                    nv,
                    &lav_dtbw_df,
                    coding_scheme_dtbw_df,
                );

                min_found = true;
            }

            if !min_found && (min_bits_all == min_bits_dt_dt) {
                fdk_write_bits(strm, DIFF_TIME as u32, 1);
                fdk_write_bits(strm, DIFF_TIME as u32, 1);

                apply_huff_coding(
                    strm,
                    Some(&data_diff_time[0]),
                    Some(&data_diff_time[1]),
                    data_type,
                    DIFF_TIME,
                    DIFF_TIME,
                    nv,
                    &lav_dt_dt,
                    coding_scheme_dt_dt,
                );
            }
        }

        // LSB coding
        if split_lsb_flag != 0 {
            apply_lsb_coding(strm, &quant_data_lsb[0], 1, data_bands);
            apply_lsb_coding(strm, &quant_data_lsb[1], 1, data_bands);
        }
    }

    reset as i32
}

/// `fdk_sacenc_ecDataSingleEnc(strm, aaInData, aHistory, data_type, setIdx, startBand,
/// dataBands, coarse_flag, independency_flag)`: encodes
/// `aaInData[setIdx][startBand..startBand + dataBands]`.
pub fn fdk_sacenc_ec_data_single_enc(
    strm: &mut FdkBitStream,
    aa_in_data: &[[i16; MAXBANDS]],
    a_history: &[i16; MAXBANDS],
    data_type: DataType,
    set_idx: i32,
    start_band: i32,
    data_bands: i32,
    coarse_flag: i32,
    independency_flag: i32,
) -> i32 {
    let reset: i16 = 0;
    let set_idx_u = set_idx as usize;
    let sb = start_band as usize;
    let nb = data_bands.max(0) as usize;

    // splitLsb_flag is 0 for every data type.
    let split_lsb_flag: i16 = 0;

    let allow_diff_time_back_flag = (independency_flag == 0) || (set_idx > 0);

    let (quant_levels, quant_offset) = quantizer(data_type, coarse_flag);

    let mut quant_data_lsb = [0i16; MAXBANDS];
    let mut quant_data_msb = [0i16; MAXBANDS];
    let mut quant_data_hist_lsb = [0i16; MAXBANDS];
    let mut quant_data_hist_msb = [0i16; MAXBANDS];

    let mut data_diff_freq = [0i16; MAXBANDS];
    let mut data_diff_time = [0i16; MAXBANDS + 2];

    let num_lsb_bits: i16;

    // Split off LSB
    if split_lsb_flag != 0 {
        split_lsb(
            &aa_in_data[set_idx_u][sb..],
            quant_offset,
            data_bands,
            Some(&mut quant_data_lsb),
            Some(&mut quant_data_msb),
        );
        num_lsb_bits = data_bands as i16;
    } else {
        // With quant_offset == 0, p_quant_data_msb points into aaInData, which is the same as
        // a copy of the values.
        for pb in 0..nb {
            quant_data_msb[pb] =
                (aa_in_data[set_idx_u][sb + pb] as i32 + quant_offset as i32) as i16;
        }
        num_lsb_bits = 0;
    }

    if allow_diff_time_back_flag {
        if split_lsb_flag != 0 {
            split_lsb(
                &a_history[sb..],
                quant_offset,
                data_bands,
                Some(&mut quant_data_hist_lsb),
                Some(&mut quant_data_hist_msb),
            );
        } else {
            for pb in 0..nb {
                quant_data_hist_msb[pb] = (a_history[sb + pb] as i32 + quant_offset as i32) as i16;
            }
        }
    }

    // Calculate frequency differences
    calc_diff_freq(&quant_data_msb, &mut data_diff_freq, data_bands);

    // Calculate time differences
    if allow_diff_time_back_flag {
        calc_diff_time(&quant_data_msb, &quant_data_hist_msb, &mut data_diff_time, data_bands);
    }

    // Calculate coding scheme with minumum bit consumption
    let num_pcm_bits = calc_pcm_bits(data_bands as i16, quant_levels);
    let num_pcm_val = data_bands as i16;

    let mut min_bits_all = num_pcm_bits;

    let mut lav_df = [-1i16; 2];
    let mut lav_dt = [-1i16; 2];

    let mut coding_scheme_df: i16 = 0;
    let mut coding_scheme_dt: i16 = 0;

    let mut min_bits_dt: i16 = -1;

    let nv = data_bands as i16;

    let mut min_bits_df = calc_huff_bits(
        Some(&data_diff_freq),
        None,
        data_type,
        DIFF_FREQ,
        DIFF_FREQ,
        nv,
        &mut lav_df,
        &mut coding_scheme_df,
    );

    if allow_diff_time_back_flag {
        min_bits_df = (min_bits_df as i32 + 1) as i16;
    }

    min_bits_df = (min_bits_df as i32 + num_lsb_bits as i32) as i16;

    if min_bits_df < min_bits_all {
        min_bits_all = min_bits_df;
    }

    if allow_diff_time_back_flag {
        min_bits_dt = calc_huff_bits(
            Some(&data_diff_time),
            None,
            data_type,
            DIFF_TIME,
            DIFF_TIME,
            nv,
            &mut lav_dt,
            &mut coding_scheme_dt,
        );

        min_bits_dt = (min_bits_dt as i32 + 1 + num_lsb_bits as i32) as i16;

        if min_bits_dt < min_bits_all {
            min_bits_all = min_bits_dt;
        }
    }

    // Start actual coding now

    // PCM or Diff/Huff Coding?
    let pcm_coding_flag = min_bits_all == num_pcm_bits;

    fdk_write_bits(strm, pcm_coding_flag as u32, 1);

    if pcm_coding_flag {
        // Grouped PCM Coding
        apply_pcm_coding(
            strm,
            Some(&aa_in_data[set_idx_u][sb..]),
            None,
            quant_offset,
            num_pcm_val,
            quant_levels,
        );
    } else {
        // Diff/Huff Coding
        let mut min_found = false;

        if min_bits_all == min_bits_df {
            if allow_diff_time_back_flag {
                fdk_write_bits(strm, DIFF_FREQ as u32, 1);
            }

            apply_huff_coding(
                strm,
                Some(&data_diff_freq),
                None,
                data_type,
                DIFF_FREQ,
                DIFF_FREQ,
                nv,
                &lav_df,
                coding_scheme_df,
            );

            min_found = true;
        }

        if allow_diff_time_back_flag && !min_found && (min_bits_all == min_bits_dt) {
            fdk_write_bits(strm, DIFF_TIME as u32, 1);

            apply_huff_coding(
                strm,
                Some(&data_diff_time),
                None,
                data_type,
                DIFF_TIME,
                DIFF_TIME,
                nv,
                &lav_dt,
                coding_scheme_dt,
            );
        }

        // LSB coding
        if split_lsb_flag != 0 {
            apply_lsb_coding(strm, &quant_data_lsb, 1, data_bands);
        }
    }

    reset as i32
}
