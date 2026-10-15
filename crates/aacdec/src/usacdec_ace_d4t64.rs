// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/usacdec_ace_d4t64.h, libAACdec/src/usacdec_ace_d4t64.cpp.

//! Port of `usacdec_ace_d4t64.h`: the algebraic (fixed) codebook of the ACELP decoder.
//!
//! The preprocessed `usacdec_ace_d4t64.cpp` of the `aarch64` and `x86` builds are identical
//! apart from `FDK_ASSERT`; no primitive with a variant is used.
//!
//! `SHORT` arithmetic is computed in `int` and truncated when stored, as in C (`wrapping_*` and
//! `as i16`). The pulse positions are always inside the codevector for the indices that the
//! bitstream reader produces; the port drops writes for a track or position outside of it.

use fdk_aac_rs_fdk::common_fix::{FixpDbl, FixpSgl, FRACT_BITS};
use fdk_aac_rs_fdk::fdk_assert;

/// `FIXP_COD`: data type of the fixed codebook vector
pub type FixpCod = FixpSgl;

/// `COD_BITS`
pub const COD_BITS: i32 = FRACT_BITS;

/// `FX_COD2FX_DBL(x)`: `FX_SGL2FX_DBL(x)`
#[inline(always)]
pub const fn fx_cod2fx_dbl(x: FixpCod) -> FixpDbl {
    (x as i32) << 16
}

/// `FX_DBL2FX_COD(x)`: `FX_DBL2FX_SGL((x) + (FIXP_DBL)0x8000)` (the addition wraps)
#[inline(always)]
pub const fn fx_dbl2fx_cod(x: FixpDbl) -> FixpCod {
    (x.wrapping_add(0x8000) >> 16) as i16
}

/// `FX_SGL2FX_COD(x)`
#[inline(always)]
pub const fn fx_sgl2fx_cod(x: FixpSgl) -> FixpCod {
    x
}

/// `L_SUBFR` of `usacdec_ace_d4t64.cpp`: subframe size
const L_SUBFR: usize = 64;

/// `D_ACELP_add_pulse(pos, nb_pulse, track, code)`: add pulses to the fixed codebook.
fn d_acelp_add_pulse(pos: &[i16], nb_pulse: i16, track: i16, code: &mut [FixpCod]) {
    for k in 0..nb_pulse.max(0) as usize {
        // i = ((pos[k] & (16-1))*NB_TRACK) + track;
        let i = (((pos[k] as i32 & (16 - 1)) << 2) + track as i32) as i16;
        // A position outside of code[] (an invalid track) is dropped.
        if let Some(c) = code.get_mut(i as usize).filter(|_| i >= 0) {
            if (pos[k] & 16) == 0 {
                *c = (*c as i32 + (512 << (COD_BITS - FRACT_BITS))) as i16;
            } else {
                *c = (*c as i32 - (512 << (COD_BITS - FRACT_BITS))) as i16;
            }
        }
    }
}

/// `D_ACELP_decode_1p_N1(index, N, offset, pos)`: decode 1 pulse with N+1 bits.
fn d_acelp_decode_1p_n1(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let mask = (1i32 << n) - 1;
    // Decode 1 pulse with N+1 bits
    let mut pos1 = ((index & mask) + offset as i32) as i16;
    let i = (index >> n) & 1;
    if i == 1 {
        pos1 = (pos1 as i32 + 16) as i16;
    }
    pos[0] = pos1;
}

/// `D_ACELP_decode_2p_2N1(index, N, offset, pos)`: decode 2 pulses with 2*N+1 bits.
fn d_acelp_decode_2p_2n1(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let mask = (1i32 << n) - 1;
    // Decode 2 pulses with 2*N+1 bits
    let mut pos1 = (((index >> n) & mask) + offset as i32) as i16;
    let i = (index >> (2 * n)) & 1;
    let mut pos2 = ((index & mask) + offset as i32) as i16;
    if (pos2 as i32 - pos1 as i32) < 0 {
        if i == 1 {
            pos1 = (pos1 as i32 + 16) as i16;
        } else {
            pos2 = (pos2 as i32 + 16) as i16;
        }
    } else {
        if i == 1 {
            pos1 = (pos1 as i32 + 16) as i16;
            pos2 = (pos2 as i32 + 16) as i16;
        }
    }
    pos[0] = pos1;
    pos[1] = pos2;
}

/// `D_ACELP_decode_3p_3N1(index, N, offset, pos)`: decode 3 pulses with 3*N+1 bits.
fn d_acelp_decode_3p_3n1(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    // Decode 3 pulses with 3*N+1 bits
    let mut mask = (1i32 << ((2 * n) - 1)) - 1;
    let mut idx = index & mask;
    let mut j = offset;
    if ((index >> ((2 * n) - 1)) & 1) == 1 {
        j = (j as i32 + (1i32 << (n - 1))) as i16;
    }
    d_acelp_decode_2p_2n1(idx, n - 1, j, pos);
    mask = (1i32 << (n + 1)) - 1;
    idx = (index >> (2 * n)) & mask;
    d_acelp_decode_1p_n1(idx, n, offset, &mut pos[2..]);
}

/// `D_ACELP_decode_4p_4N1(index, N, offset, pos)`: decode 4 pulses with 4*N+1 bits.
fn d_acelp_decode_4p_4n1(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    // Decode 4 pulses with 4*N+1 bits
    let mut mask = (1i32 << ((2 * n) - 1)) - 1;
    let mut idx = index & mask;
    let mut j = offset;
    if ((index >> ((2 * n) - 1)) & 1) == 1 {
        j = (j as i32 + (1i32 << (n - 1))) as i16;
    }
    d_acelp_decode_2p_2n1(idx, n - 1, j, pos);
    mask = (1i32 << ((2 * n) + 1)) - 1;
    idx = (index >> (2 * n)) & mask;
    d_acelp_decode_2p_2n1(idx, n, offset, &mut pos[2..]);
}

/// `D_ACELP_decode_4p_4N(index, N, offset, pos)`: decode 4 pulses with 4*N bits.
fn d_acelp_decode_4p_4n(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    // Decode 4 pulses with 4*N bits
    let n_1 = n - 1;
    let j = (offset as i32 + (1i32 << n_1)) as i16;
    match (index >> ((4 * n) - 2)) & 3 {
        0 => {
            if ((index >> ((4 * n_1) + 1)) & 1) == 0 {
                d_acelp_decode_4p_4n1(index, n_1, offset, pos);
            } else {
                d_acelp_decode_4p_4n1(index, n_1, j, pos);
            }
        }
        1 => {
            d_acelp_decode_1p_n1(index >> ((3 * n_1) + 1), n_1, offset, pos);
            d_acelp_decode_3p_3n1(index, n_1, j, &mut pos[1..]);
        }
        2 => {
            d_acelp_decode_2p_2n1(index >> ((2 * n_1) + 1), n_1, offset, pos);
            d_acelp_decode_2p_2n1(index, n_1, j, &mut pos[2..]);
        }
        _ => {
            d_acelp_decode_3p_3n1(index >> (n_1 + 1), n_1, offset, pos);
            d_acelp_decode_1p_n1(index, n_1, j, &mut pos[3..]);
        }
    }
}

/// `D_ACELP_decode_4t64(index, nbits, code)`: 12, 16, 20, 28, 36, 44, 52 and 64 bits algebraic
/// codebook; 4 tracks x 16 positions per track = 64 samples (`code` must hold 64 values).
///
/// All pulses can have two (2) possible amplitudes: +1 or -1. Each pulse can have sixteen (16)
/// possible positions. `index` must hold the 8 indices of the subframe (`icb_index[sfr]`).
pub fn d_acelp_decode_4t64(index: &[i16], nbits: i32, code: &mut [FixpCod]) {
    let mut pos = [0i16; 6];

    code[..L_SUBFR].fill(0);

    // decode the positions and signs of pulses and build the codeword
    match nbits {
        12 => {
            let mut k: i16 = 0;
            while k < 4 {
                let l_index = index[(2 * (k / 2) + 1) as usize] as i32;
                d_acelp_decode_1p_n1(l_index, 4, 0, &mut pos);
                let track = (2 * (index[(2 * (k / 2)) as usize] as i32) + (k / 2) as i32) as i16;
                d_acelp_add_pulse(&pos, 1, track, code);
                k += 2;
            }
        }
        16 => {
            let mut i = 0usize;
            let mut offset = index[i] as i32;
            i += 1;
            offset = if offset == 0 { 1 } else { 3 };
            for k in 0..4i16 {
                if k as i32 != offset {
                    let l_index = index[i] as i32;
                    i += 1;
                    d_acelp_decode_1p_n1(l_index, 4, 0, &mut pos);
                    d_acelp_add_pulse(&pos, 1, k, code);
                }
            }
        }
        20 => {
            for k in 0..4i16 {
                let l_index = index[k as usize] as i32;
                d_acelp_decode_1p_n1(l_index, 4, 0, &mut pos);
                d_acelp_add_pulse(&pos, 1, k, code);
            }
        }
        28 => {
            for k in 0..(4 - 2) as i16 {
                let l_index = index[k as usize] as i32;
                d_acelp_decode_2p_2n1(l_index, 4, 0, &mut pos);
                d_acelp_add_pulse(&pos, 2, k, code);
            }
            for k in 2..4i16 {
                let l_index = index[k as usize] as i32;
                d_acelp_decode_1p_n1(l_index, 4, 0, &mut pos);
                d_acelp_add_pulse(&pos, 1, k, code);
            }
        }
        36 => {
            for k in 0..4i16 {
                let l_index = index[k as usize] as i32;
                d_acelp_decode_2p_2n1(l_index, 4, 0, &mut pos);
                d_acelp_add_pulse(&pos, 2, k, code);
            }
        }
        44 => {
            for k in 0..(4 - 2) as i16 {
                let l_index = index[k as usize] as i32;
                d_acelp_decode_3p_3n1(l_index, 4, 0, &mut pos);
                d_acelp_add_pulse(&pos, 3, k, code);
            }
            for k in 2..4i16 {
                let l_index = index[k as usize] as i32;
                d_acelp_decode_2p_2n1(l_index, 4, 0, &mut pos);
                d_acelp_add_pulse(&pos, 2, k, code);
            }
        }
        52 => {
            for k in 0..4i16 {
                let l_index = index[k as usize] as i32;
                d_acelp_decode_3p_3n1(l_index, 4, 0, &mut pos);
                d_acelp_add_pulse(&pos, 3, k, code);
            }
        }
        64 => {
            for k in 0..4i16 {
                // (LONG)index[k] << 14: a negative index (not produced by the reader) shifts a
                // negative value, which wraps like the shift instruction.
                let l_index = (index[k as usize] as i32)
                    .wrapping_shl(14)
                    .wrapping_add(index[k as usize + 4] as i32);
                d_acelp_decode_4p_4n(l_index, 4, 0, &mut pos);
                d_acelp_add_pulse(&pos, 4, k, code);
            }
        }
        _ => {
            fdk_assert!(false);
        }
    }
}
