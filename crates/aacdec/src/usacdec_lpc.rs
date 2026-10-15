// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/usacdec_lpc.h, libAACdec/src/usacdec_lpc.cpp.

//! Port of `usacdec_lpc.h`: USAC LPC/AVQ decoding (LSF decoding with the RE8 lattice vector
//! quantizer, LSF/LSP/LP conversion, LPC concealment, TCX/FAC gain decoding).
//!
//! The preprocessed `usacdec_lpc.cpp` of the `aarch64` and `x86` builds differ only in the
//! spelling of `M_PI` (both literals give the same `double`) and in the form of the
//! `SATURATE_LEFT_SHIFT` macro (the same values); the variants differ in the primitives
//! (`fMult`, `sqrtFixp`).
//!
//! `FIXP_LPC` is `FIXP_SGL`: `FIXP_LPC` arithmetic is computed in `int` and truncated when
//! stored, as in C. `FIXP_DBL` sums use `wrapping_*` where C can overflow.

use fdk_aac_rs_fdk::bitstream::{fdk_read_bit, fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::{
    f_mult, f_mult_add, f_mult_div2, f_pow2_div2, fl2fxconst_dbl, fl2fxconst_sgl, fx_dbl2fx_sgl,
    FixpDbl, FixpSgl, DFRACT_BITS,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixpoint_math::{f_add_saturate_sgl, f_ld_pow, sqrt_fixp};
use fdk_aac_rs_fdk::lpc::{fx_dbl2fx_lpc, fx_lpc2fx_dbl, FixpLpc};
use fdk_aac_rs_fdk::scale::{get_scalefactor, saturate_left_shift, scale_values};
use fdk_aac_rs_fdk::trig_fcts::fixp_cos;

use crate::usacdec_const::{M_LP_FILTER_ORDER, NB_DIV};
use crate::usacdec_rom::{
    FDK_DEC_A3, FDK_DEC_A4, FDK_DEC_DA, FDK_DEC_DICO_LSF_ABS_8B, FDK_DEC_DS, FDK_DEC_I3,
    FDK_DEC_I4, FDK_DEC_IA, FDK_DEC_IS, FDK_DEC_LSF_INIT, FDK_DEC_NS, FDK_DEC_TAB_FACTORIAL,
    LSF_SCALE, NB_LDQ3, NB_LDQ4,
};

/// `LSPARG_SCALE`
pub const LSPARG_SCALE: i32 = 10;

const M: usize = M_LP_FILTER_ORDER as usize;

/// `NQ_MAX`
const NQ_MAX: i32 = 36;

/*
 * Helper functions.
 */

/// `get_vlclbf(hBs)`: read unary code.
fn get_vlclbf(h_bs: &mut FdkBitStream) -> i32 {
    let mut result = 0;

    while fdk_read_bits(h_bs, 1) != 0 && result <= NQ_MAX {
        result += 1;
    }
    result
}

/// `get_vlclbf_n(hBs, n)`: read bit count limited unary code.
fn get_vlclbf_n(h_bs: &mut FdkBitStream, mut n: i32) -> i32 {
    let mut result = 0;

    while fdk_read_bits(h_bs, 1) != 0 {
        result += 1;
        n -= 1;
        if n <= 0 {
            break;
        }
    }

    result
}

/*
 * Algebraic Vector Quantizer
 */

/// `ZF_SCALE`: must be greater than (number of `FIXP_ZF`)/2 because of the loss of precision
/// caused by `fPow2Div2` in `RE8_PPV()`.
const ZF_SCALE: i32 = DFRACT_BITS / 2;

/// `FIXP_ZF`
type FixpZf = FixpDbl;

/// `INT2ZF(x, s)`: `(FIXP_ZF)((x) << (ZF_SCALE - (s)))`
#[inline(always)]
fn int2zf(x: i32, s: i32) -> FixpZf {
    x.wrapping_shl((ZF_SCALE - s) as u32)
}

/// `ZF2INT(x)`: `(INT)((x) >> ZF_SCALE)`
#[inline(always)]
fn zf2int(x: FixpZf) -> i32 {
    x >> ZF_SCALE
}

/// `ONEZF`: 1.0 in ZF format
const ONEZF: FixpZf = 1 << ZF_SCALE;

/// `nearest_neighbor_2D8(x, y)`: round `x` into 2Z^8 (with the Wagner rule).
pub fn nearest_neighbor_2d8(x: &[FixpZf; 8], y: &mut [i32; 8]) {
    let mut e = [0 as FixpZf; 8];

    // round x into 2Z^8 i.e. compute y=(y1,...,y8) such that yi = 2[xi/2] where [.] is the
    // nearest integer operator in the mean time, compute sum = y1+...+y8
    //
    // `ONEZF - x[i]` (x < 0) and `ONEZF + x[i]` (x >= 0) are positive unless they overflow
    // (undefined); both reference builds assume they do not: they shift right logically
    // (`lsr`/`shrl`), and for x >= 0 they compute `2 * (tmp >> 17)` as `(tmp >> 16) & 0x7ffe`
    // (the 15 bits a positive `int` can have). The port computes the same for every input (the
    // overflow is not reachable from the decoder, whose lattice points stay below 2^30).
    let mut sum: i32 = 0;
    for i in 0..8 {
        // round to ..., -2, 0, 2, ... ([-1..1[ --> 0)
        if x[i] < 0 {
            let tmp = (ONEZF as u32).wrapping_sub(x[i] as u32);
            y[i] = -2 * ((tmp >> ZF_SCALE) >> 1) as i32;
        } else {
            let tmp = (ONEZF as u32).wrapping_add(x[i] as u32);
            y[i] = ((tmp >> ZF_SCALE) & 0x7ffe) as i32;
        }
        sum = sum.wrapping_add(y[i]);
    }
    // check if y1+...+y8 is a multiple of 4 if not, y is not round xj in the wrong way where j
    // is defined by j = arg max_i | xi -yi| (this is called the Wagner rule)
    if sum % 4 != 0 {
        // find j = arg max_i | xi -yi|
        // |ei| is compared unsigned by both reference builds (clang assumes that the negation
        // does not overflow): |MINVAL| is the largest value.
        let mut em: u32 = 0;
        let mut j = 0usize;
        for i in 0..8 {
            // compute ei = xi-yi
            e[i] = x[i].wrapping_sub(int2zf(y[i], 0));
        }
        for i in 0..8 {
            // compute |ei| = | xi-yi |
            let s = e[i].unsigned_abs();
            // check if |ei| is maximal, if so, set j=i
            if em < s {
                em = s;
                j = i;
            }
        }
        // round xj in the "wrong way"
        if e[j] < 0 {
            y[j] -= 2;
        } else {
            y[j] += 2;
        }
    }
}

/// `RE8_PPV(x, y, r)`: nearest neighbor search in the infinite lattice RE8, based on the
/// definition RE8 = (2D8) U (2D8+`1,1,1,1,1,1,1,1`) (coset decoding of Sloane and Conway).
/// `x`: point in R^8 in 32-ZF_SCALE.ZF_SCALE format; `y`: point in RE8.
pub fn re8_ppv(x: &[FixpZf; 8], y: &mut [i16; 8], r: i32) {
    let mut y0 = [0i32; 8];
    let mut y1 = [0i32; 8];
    let mut x1 = [0 as FixpZf; 8];

    // find the nearest neighbor y0 of x in 2D8
    nearest_neighbor_2d8(x, &mut y0);
    // find the nearest neighbor y1 of x in 2D8+(1,...,1) (by coset decoding)
    for i in 0..8 {
        x1[i] = x[i].wrapping_sub(ONEZF);
    }
    nearest_neighbor_2d8(&x1, &mut y1);
    for i in 0..8 {
        y1[i] += 1;
    }

    // compute e0=||x-y0||^2 and e1=||x-y1||^2
    let mut e: i64 = 0;
    for i in 0..8 {
        let mut tmp = x[i].wrapping_sub(int2zf(y0[i], 0));
        // shift left to ensure that no fract part bits get lost.
        e = e.wrapping_add(f_pow2_div2(tmp.wrapping_shl(r as u32)) as i64);
        tmp = x[i].wrapping_sub(int2zf(y1[i], 0));
        e = e.wrapping_sub(f_pow2_div2(tmp.wrapping_shl(r as u32)) as i64);
    }
    // select best candidate y0 or y1 to minimize distortion
    if e < 0 {
        for i in 0..8 {
            y[i] = y0[i] as i16;
        }
    } else {
        for i in 0..8 {
            y[i] = y1[i] as i16;
        }
    }
}

/// `table_lookup(table, index, range)`: table look-up of unsigned value: find i where
/// `index >= table[i]`. Note: range must be >= 2, index must be >= `table[0]`.
fn table_lookup(table: &[u16], index: u32, range: i32) -> i32 {
    let mut i = 4;
    while i < range {
        if index < table[i as usize] as u32 {
            break;
        }
        i += 4;
    }
    if i > range {
        i = range;
    }

    if index < table[(i - 2) as usize] as u32 {
        i -= 2;
    }
    if index < table[(i - 1) as usize] as u32 {
        i -= 1;
    }
    i -= 1;

    i // index >= table[i]
}

/// `re8_decode_rank_of_permutation(rank, xs, x)`: decoding of the rank of the permutation of
/// the signed leader `xs` into the point `x` in RE8 (Schalkwijk's formula).
pub fn re8_decode_rank_of_permutation(rank: i32, xs: &[i32; 8], x: &mut [i16; 8]) {
    let mut a = [0i32; 8];
    let mut w = [0i32; 8];

    // pre-processing based on the signed leader xs: compute the alphabet a[0..q-1] of x, the
    // number of occurences w[0..q-1] of each symbol and B = prod_j (w[j]!)
    let mut j = 0usize;
    w[j] = 1;
    a[j] = xs[0];
    let mut b: i32 = 1;
    for i in 1..8 {
        if xs[i] != xs[i - 1] {
            j += 1;
            w[j] = 1;
            a[j] = xs[i];
        } else {
            w[j] += 1;
            b *= w[j];
        }
    }

    // actual rank decoding: the rank of x (where x is a permutation of xs) is based on
    // Schalkwijk's formula rank=sum_{k=0..7} (A_k * fac_k/B_k); the decoding of this rank is
    // sequential and reconstructs x[0..7] element by element from x[0] to x[7]
    if w[0] == 8 {
        for i in 0..8 {
            x[i] = a[0] as i16; // avoid fac of 40320
        }
    } else {
        let mut target = rank.wrapping_mul(b);
        let mut fac_b: i32 = 1;
        // decode x element by element
        for i in 0..8 {
            let fac = fac_b.wrapping_mul(FDK_DEC_TAB_FACTORIAL[i] as i32); // fac = 1..5040
            let mut jj: usize = 0;
            let mut first = true;
            loop {
                if first {
                    first = false;
                } else {
                    jj += 1;
                }
                // w[++j]: the alphabet has at most 8 symbols; positions beyond w[] have no
                // defined reference result. The port stops at the last entry.
                if jj >= 8 {
                    jj = 7;
                    break;
                }
                target = target.wrapping_sub(w[jj].wrapping_mul(fac));
                if target < 0 {
                    break;
                }
            }
            x[i] = a[jj] as i16;
            // update rank, denominator B (B_k) and counter w[j]
            target = target.wrapping_add(w[jj].wrapping_mul(fac)); // target = fac_B*B*rank
            fac_b = fac_b.wrapping_mul(w[jj]);
            w[jj] -= 1;
        }
    }
}

/// `re8_decode_base_index(n, I, y)`: decoding of an index in Qn (n=0,2,3 or 4) into the point
/// `y` in RE8.
pub fn re8_decode_base_index(n: &mut i32, index: u32, y: &mut [i16; 8]) {
    if *n < 2 {
        for i in 0..8 {
            y[i] = 0;
        }
    } else {
        // search for the identifier ka of the absolute leader (table-lookup); Q2 is a subset of
        // Q3 - the two cases are considered in the same branch
        let ka: usize = match *n {
            2 | 3 => {
                let i = table_lookup(&FDK_DEC_I3, index, NB_LDQ3);
                FDK_DEC_A3[i as usize] as usize
            }
            4 => {
                let i = table_lookup(&FDK_DEC_I4, index, NB_LDQ4);
                FDK_DEC_A4[i as usize] as usize
            }
            _ => {
                fdk_assert!(false);
                return;
            }
        };
        // reconstruct the absolute leader
        let mut leader = [0i32; 8];
        for i in 0..8 {
            leader[i] = FDK_DEC_DA[ka][i] as i32;
        }
        // search for the identifier ks of the signed leader (table look-up) (this search is
        // focused based on the identifier ka of the absolute leader)
        let t = FDK_DEC_IA[ka] as usize;
        let im = FDK_DEC_NS[ka] as i32;
        let ks = table_lookup(&FDK_DEC_IS[t..], index, im) as usize;

        // reconstruct the signed leader from its sign code
        let mut sign_code = 2 * FDK_DEC_DS[t + ks] as i32;
        for i in (0..8).rev() {
            leader[i] *= 1 - (sign_code & 2);
            sign_code >>= 1;
        }

        // compute and decode the rank of the permutation
        let rank = index.wrapping_sub(FDK_DEC_IS[t + ks] as u32) as i32; // rank = index - cardinality offset

        re8_decode_rank_of_permutation(rank, &leader, y);
    }
}

/// `re8_k2y(k, r, y)`: Voronoi indexing (index decoding) `k` -> `y`. `k`: Voronoi index
/// k[0..7]; `r`: Voronoi order (m = 2^r); `y`: 8-dimensional point in RE8.
pub fn re8_k2y(k: &[i32; 8], r: i32, y: &mut [i16; 8]) {
    let mut v = [0i16; 8];
    let mut zf = [0 as FixpZf; 8];

    fdk_assert!(r <= ZF_SCALE);

    // compute y = k M and z=(y-a)/m, where
    //   M = [4        ]
    //       [2 2      ]
    //       [|   \    ]
    //       [2     2  ]
    //       [1 1 _ 1 1]
    //   a=(2,0,...,0)
    //   m = 1<<r
    for i in 0..8 {
        y[i] = k[7] as i16;
    }
    zf[7] = int2zf(y[7] as i32, r);
    let mut sum: i32 = 0;
    for i in (1..=6).rev() {
        let tmp = 2i32.wrapping_mul(k[i]);
        sum = sum.wrapping_add(tmp);
        y[i] = (y[i] as i32).wrapping_add(tmp) as i16;
        zf[i] = int2zf(y[i] as i32, r);
    }
    y[0] = (y[0] as i32).wrapping_add(4i32.wrapping_mul(k[0]).wrapping_add(sum)) as i16;
    zf[0] = int2zf(y[0] as i32 - 2, r);
    // find nearest neighbor v of z in infinite RE8
    re8_ppv(&zf, &mut v, r);
    // compute y -= m v
    for i in 0..8 {
        y[i] = (y[i] as i32 - ((v[i] as i32).wrapping_shl(r as u32) as i16) as i32) as i16;
    }
}

/// `RE8_dec(n, I, k, y)`: multi-rate indexing of a point y in the lattice RE8 (index
/// decoding). `n`: codebook number (0,2,3,4,..,n_max = 36); `I`: base codebook index; `k`:
/// Voronoi index; `y`: point in RE8. Returns 0 on success, -1 on error.
pub fn re8_dec(mut n: i32, i_idx: i32, k: &[i32; 8], y: &mut [FixpDbl]) -> i32 {
    let mut v = [0i16; 8];
    let mut y_ = [0i16; 8];

    // Check bound of codebook qn
    if n > NQ_MAX {
        return -1;
    }

    // decode the sub-indices I and kv[] according to the codebook number n: if n=0,2,3,4,
    // decode I (no Voronoi extension); if n>4, Voronoi extension is used, decode I and kv[]
    if n <= 4 {
        re8_decode_base_index(&mut n, i_idx as u32, &mut y_);
        for i in 0..8 {
            y[i] = y_[i] as i32;
        }
    } else {
        // compute the Voronoi modulo m = 2^r where r is extension order
        let r = ((n - 3) >> 1) as u32;

        while n > 4 {
            n -= 2;
        }
        // decode base codebook index I into c (c is an element of Q3 or Q4) [here c is stored
        // in y to save memory]
        re8_decode_base_index(&mut n, i_idx as u32, &mut y_);
        // decode Voronoi index k[] into v
        re8_k2y(k, r as i32, &mut v);
        // reconstruct y as y = m c + v (with m=2^r, r integer >=1)
        for i in 0..8 {
            y[i] = (y_[i] as i32).wrapping_shl(r).wrapping_add(v[i] as i32);
        }
    }
    0
}

/**************************/
/* start LPC decode stuff */
/**************************/

/// `FREQ_MAX`
const FREQ_MAX: f32 = 6400.0;
/// `FREQ_DIV`
const FREQ_DIV: f32 = 400.0;
/// `LSF_GAP`
const LSF_GAP: f32 = 50.0;

/// `FL2FXCONST_LPC(FREQ_MAX / (1 << LSF_SCALE))`
#[inline(always)]
fn freq_max_lpc() -> FixpLpc {
    fl2fxconst_sgl((FREQ_MAX / (1 << LSF_SCALE) as f32) as f64)
}

/// `lsf_weight_2st(lsfq, xq, nk_mode)`: calculate the inverse weighting factor and add the
/// non-weighted residual LSF vector `xq` to the first stage LSF approximation `lsfq`.
pub fn lsf_weight_2st(lsfq: &mut [FixpLpc], xq: &[FixpDbl], nk_mode: i32) {
    let mut d = [0 as FixpLpc; M + 1];

    // compute lsf distance
    d[0] = lsfq[0];
    d[M] = (freq_max_lpc() as i32 - lsfq[M - 1] as i32) as FixpLpc;
    for i in 1..M {
        d[i] = (lsfq[i] as i32 - lsfq[i - 1] as i32) as FixpLpc;
    }

    let factor: FixpSgl = match nk_mode {
        0 => fl2fxconst_sgl((2.0f32 * 60.0f32 / FREQ_DIV) as f64), // abs
        1 => fl2fxconst_sgl((2.0f32 * 65.0f32 / FREQ_DIV) as f64), // mid
        2 => fl2fxconst_sgl((2.0f32 * 64.0f32 / FREQ_DIV) as f64), // rel1
        _ => fl2fxconst_sgl((2.0f32 * 63.0f32 / FREQ_DIV) as f64), // rel2
    };
    // add non-weighted residual LSF vector to LSF1st
    for i in 0..M {
        let w: i32 = f_mult_div2(factor, sqrt_fixp(f_mult(d[i], d[i + 1])));
        lsfq[i] = f_add_saturate_sgl(
            lsfq[i],
            fx_dbl2fx_lpc(((w as i64).wrapping_mul(xq[i] as i64)) as FixpDbl),
        );
    }
}

/// `decode_qn(hBs, nk_mode, nqn, qn)`: decode `nqn` code book numbers, which determine the
/// amount of following bits for `nqn` AVQ RE8 vectors.
fn decode_qn(h_bs: &mut FdkBitStream, nk_mode: i32, nqn: i32, qn: &mut [i32; 2]) {
    let nqn = nqn as usize;
    if nk_mode == 1 {
        // nk mode 1: Unary code for mid LPC1/LPC3: Q0=0, Q2=10, Q3=110, ...
        for n in 0..nqn {
            qn[n] = get_vlclbf(h_bs);
            if qn[n] > 0 {
                qn[n] += 1;
            }
        }
    } else {
        // nk_mode 0, 3 and 2: 2 bits to specify Q2,Q3,Q4,ext
        for n in 0..nqn {
            qn[n] = 2 + fdk_read_bits(h_bs, 2) as i32;
        }
        if nk_mode == 2 {
            // Unary code for rel LPC1/LPC3: Q0 = 0, Q5=10, Q6=110, ...
            for n in 0..nqn {
                if qn[n] > 4 {
                    qn[n] = get_vlclbf(h_bs);
                    if qn[n] > 0 {
                        qn[n] += 4;
                    }
                }
            }
        } else {
            // nk_mode == (0 and 3): Unary code for abs and rel LPC0/LPC2:
            // Q5 = 0, Q6=10, Q0=110, Q7=1110, ...
            for n in 0..nqn {
                if qn[n] > 4 {
                    qn[n] = get_vlclbf(h_bs);
                    match qn[n] {
                        0 => qn[n] = 5,
                        1 => qn[n] = 6,
                        2 => qn[n] = 0,
                        _ => qn[n] += 4,
                    }
                }
            }
        }
    }
}

/// `reorder_lsf(lsf, min_dist, n)`: reorder LSF coefficients to minimum distance.
pub fn reorder_lsf(lsf: &mut [FixpLpc], min_dist: FixpLpc, n: usize) {
    let mut lsf_min = min_dist;
    for i in 0..n {
        if lsf[i] < lsf_min {
            lsf[i] = lsf_min;
        }
        lsf_min = f_add_saturate_sgl(lsf[i], min_dist);
    }

    // reverse
    lsf_min = (freq_max_lpc() as i32 - min_dist as i32) as FixpLpc;
    for i in (0..n).rev() {
        if lsf[i] > lsf_min {
            lsf[i] = lsf_min;
        }

        lsf_min = (lsf[i] as i32 - min_dist as i32) as FixpLpc;
    }
}

/// `vlpc_1st_dec(hBs, lsfq)`: first stage approximation.
fn vlpc_1st_dec(h_bs: &mut FdkBitStream, lsfq: &mut [FixpLpc]) {
    let index = fdk_read_bits(h_bs, 8) as usize;
    let p_dico = &FDK_DEC_DICO_LSF_ABS_8B[index * M..index * M + M];
    lsfq[..M].copy_from_slice(p_dico);
}

/// `vlpc_2st_dec(hBs, lsfq, nk_mode)`: do first stage approximation weighting and multiply with
/// AVQ refinement. Returns 0 on success, -1 on error.
fn vlpc_2st_dec(h_bs: &mut FdkBitStream, lsfq: &mut [FixpLpc], nk_mode: i32) -> i32 {
    // weighted residual LSF vector
    let mut xq = [0 as FixpDbl; M];

    // Decode AVQ refinement
    let err = clpc_decode_avq(h_bs, &mut xq, nk_mode, 2, 8);
    if err != 0 {
        return -1;
    }

    // add non-weighted residual LSF vector to LSF1st
    lsf_weight_2st(lsfq, &xq, nk_mode);

    // reorder
    reorder_lsf(lsfq, fl2fxconst_sgl((LSF_GAP / (1 << LSF_SCALE) as f32) as f64), M);

    0
}

/*
 * Externally visible functions
 */

/// `CLpc_DecodeAVQ(hBs, pOutput, nk_mode, no_qn, length)`: AVQ (refinement) decode of `length`
/// values into `p_output` (`nqn` amount of split/interleaved RE8 vectors). Returns 0 on
/// success, -1 on error.
pub fn clpc_decode_avq(
    h_bs: &mut FdkBitStream,
    p_output: &mut [FixpDbl],
    nk_mode: i32,
    no_qn: i32,
    length: i32,
) -> i32 {
    let mut i = 0;
    while i < length {
        let mut qn = [0i32; 2];
        let mut kv = [0i32; 8];

        decode_qn(h_bs, nk_mode, no_qn, &mut qn);

        for l in 0..no_qn as usize {
            let o = i as usize + l * 8;
            // The output vector of this block; values past the end of p_output (a length that
            // is not a multiple of 8 or larger than the output) are dropped.
            let mut blk = [0 as FixpDbl; 8];
            for j in 0..8 {
                blk[j] = p_output.get(o + j).copied().unwrap_or(0);
            }
            if qn[l] == 0 {
                blk.fill(0);
            }

            // Voronoi extension order ( nk )
            let mut nk = 0;
            let mut n = qn[l];
            if qn[l] > 4 {
                nk = (qn[l] - 3) >> 1;
                n = qn[l] - nk * 2;
            }

            // Base codebook index, in reverse bit group order (!)
            let i_idx = fdk_read_bits(h_bs, (4 * n) as u32) as i32;

            if nk > 0 {
                for j in 0..8 {
                    kv[j] = fdk_read_bits(h_bs, nk as u32) as i32;
                }
            }

            if re8_dec(qn[l], i_idx, &kv, &mut blk) != 0 {
                return -1;
            }
            for j in 0..8 {
                if let Some(v) = p_output.get_mut(o + j) {
                    *v = blk[j];
                }
            }
        }
        i += 8 * no_qn;
    }
    0
}

/// `FX_DBL2FX_LPC(fixp_cos(fMult(lsf, FL2FXCONST_SGL((1 << LSPARG_SCALE) * M_PI / 6400.0)),
/// LSF_SCALE - LSPARG_SCALE))`: conversion of a LSF value into the LSP domain.
#[inline(always)]
fn lsf_to_lsp(lsf: FixpLpc) -> FixpLpc {
    fx_dbl2fx_lpc(fixp_cos(
        f_mult(lsf, fl2fxconst_sgl((1 << LSPARG_SCALE) as f64 * core::f64::consts::PI / 6400.0)),
        LSF_SCALE - LSPARG_SCALE,
    ))
}

/// `CLpc_Read(hBs, lsp, lpc4_lsf, lsf_adaptive_mean_cand, pStability, mod, first_lpd_flag,
/// last_lpc_lost, last_frame_ok)`: read and decode LPC coefficient sets (first stage
/// approximation + AVQ decode). Returns 0 on success, -1 on error.
///
/// `lsp`: the decoded LSP coefficients (`lsp_coeff[5]`); `lpc4_lsf`: the LPC4 LSF coefficients
/// (persistent); `lsf_adaptive_mean_cand`: lsf adaptive mean vector for concealment;
/// `p_stability`: stability values for the ACELP decoder (`aStability[4]`); `mod_`: the modes of
/// the current superframe.
pub fn clpc_read(
    h_bs: &mut FdkBitStream,
    lsp: &mut [[FixpLpc; M]; 5],
    lpc4_lsf: &mut [FixpLpc; M],
    lsf_adaptive_mean_cand: &mut [FixpLpc; M],
    p_stability: &mut [FixpSgl; 4],
    mod_: &[u8; 4],
    first_lpd_flag: i32,
    last_lpc_lost: i32,
    last_frame_ok: i32,
) -> i32 {
    let mut lpc_present = [0i32; 5];
    let mut lpc0_available = 1;
    let s = 0usize;
    let l = 3;
    let nb_div = NB_DIV as usize;

    lpc_present[4 >> s] = 1; // LPC4

    // Decode LPC filters in the following order: LPC 4,0,2,1,3

    // Decode LPC4
    vlpc_1st_dec(h_bs, &mut lsp[4 >> s]);
    let err = vlpc_2st_dec(h_bs, &mut lsp[4 >> s], 0); // nk_mode = 0
    if err != 0 {
        return err;
    }

    // Decode LPC0 and LPC2
    let mut k = 0usize;
    if first_lpd_flag == 0 {
        lpc_present[0] = 1;
        lpc0_available = (last_lpc_lost == 0) as i32;
        // old LPC4 is new LPC0
        lsp[0] = *lpc4_lsf;
        // skip LPC0 and continue with LPC2
        k = 2;
    }

    while k < l {
        let mut nk_mode = 0;

        if (k == 2) && (mod_[0] == 3) {
            break; // skip LPC2
        }

        lpc_present[k >> s] = 1;

        let mode_lpc_bin = fdk_read_bit(h_bs);

        if mode_lpc_bin == 0 {
            // LPC0/LPC2: Abs
            vlpc_1st_dec(h_bs, &mut lsp[k >> s]);
        } else {
            // LPC0/LPC2: RelR
            lsp[k >> s] = lsp[4 >> s];
            nk_mode = 3;
        }

        let err = vlpc_2st_dec(h_bs, &mut lsp[k >> s], nk_mode);
        if err != 0 {
            return err;
        }
        k += 2;
    }

    // Decode LPC1
    if mod_[0] < 2 {
        // else: skip LPC1
        lpc_present[1] = 1;
        let mode_lpc_bin = get_vlclbf_n(h_bs, 2);

        match mode_lpc_bin {
            1 => {
                // LPC1: abs
                vlpc_1st_dec(h_bs, &mut lsp[1]);
                let err = vlpc_2st_dec(h_bs, &mut lsp[1], 0);
                if err != 0 {
                    return err;
                }
            }
            2 => {
                // LPC1: mid0 (no second stage AVQ quantizer in this case)
                if lpc0_available != 0 {
                    // LPC0/lsf[0] might be zero some times
                    for i in 0..M {
                        lsp[1][i] = ((lsp[0][i] >> 1) as i32 + (lsp[2][i] >> 1) as i32) as FixpLpc;
                    }
                } else {
                    lsp[1] = lsp[2];
                }
            }
            0 => {
                // LPC1: RelR
                lsp[1] = lsp[2];
                let err = vlpc_2st_dec(h_bs, &mut lsp[1], 2 << s);
                if err != 0 {
                    return err;
                }
            }
            _ => {}
        }
    }

    // Decode LPC3
    if mod_[2] < 2 {
        // else: skip LPC3
        let mut nk_mode = 0;
        lpc_present[3] = 1;

        let mode_lpc_bin = get_vlclbf_n(h_bs, 3);

        match mode_lpc_bin {
            1 => {
                // LPC3: abs
                vlpc_1st_dec(h_bs, &mut lsp[3]);
            }
            0 => {
                // LPC3: mid
                for i in 0..M {
                    lsp[3][i] = ((lsp[2][i] >> 1) as i32 + (lsp[4][i] >> 1) as i32) as FixpLpc;
                }
                nk_mode = 1;
            }
            2 => {
                // LPC3: relL
                lsp[3] = lsp[2];
                nk_mode = 2;
            }
            3 => {
                // LPC3: relR
                lsp[3] = lsp[4];
                nk_mode = 2;
            }
            _ => {}
        }
        let err = vlpc_2st_dec(h_bs, &mut lsp[3], nk_mode);
        if err != 0 {
            return err;
        }
    }

    if lpc0_available == 0 && last_frame_ok == 0 {
        // LPC(0) was lost. Use next available LPC(k) instead
        for k in 1..(nb_div + 1) {
            if lpc_present[k] != 0 {
                for i in 0..M {
                    // LSF_INIT_TILT (0.25f)
                    if mod_[0] > 0 {
                        lsp[0][i] = fx_dbl2fx_lpc(
                            f_mult(lsp[k][i], fl2fxconst_sgl((1.0f32 - 0.25f32) as f64))
                                .wrapping_add(f_mult(
                                    FDK_DEC_LSF_INIT[i],
                                    fl2fxconst_sgl(0.25f32 as f64),
                                )),
                        );
                    } else {
                        lsp[0][i] = lsp[k][i];
                    }
                }
                break;
            }
        }
    }

    *lpc4_lsf = lsp[4 >> s];

    {
        let mut num_lpc = 0;

        let mut i: isize = nb_div as isize;
        loop {
            num_lpc += lpc_present[i as usize];
            i -= 1;
            if !(i >= 0 && num_lpc < 3) {
                break;
            }
        }

        let last = i;

        let div_fac: FixpDbl = match num_lpc {
            3 => fl2fxconst_dbl((1.0f32 / 3.0f32) as f64),
            2 => fl2fxconst_dbl((1.0f32 / 2.0f32) as f64),
            _ => fl2fxconst_dbl(1.0f32 as f64),
        };

        // get the adaptive mean for the next (bad) frame
        for k in 0..M {
            let mut tmp: FixpDbl = 0;
            let mut i = nb_div as isize;
            while i > last {
                if lpc_present[i as usize] != 0 {
                    tmp = f_mult_add(tmp >> 1, lsp[i as usize][k], div_fac);
                }
                i -= 1;
            }
            lsf_adaptive_mean_cand[k] = fx_dbl2fx_lpc(tmp);
        }
    }

    // calculate stability factor Theta. Needed for ACELP decoder and concealment
    {
        let mut k = 0usize;

        fdk_assert!(lpc_present[0] == 1 && lpc_present[4 >> s] == 1);
        let mut lsf_prev = 0usize;
        for i in 1..(nb_div + 1) {
            if lpc_present[i] != 0 {
                let mut tmp: FixpDbl = 0;
                let lsf_curr = i;

                // sum = tmp * 2^(LSF_SCALE*2 + 4)
                for j in 0..M {
                    tmp = tmp.wrapping_add(
                        f_pow2_div2((lsp[lsf_curr][j] as i32 - lsp[lsf_prev][j] as i32) as FixpSgl)
                            >> 3,
                    );
                }

                // tmp = (float)(FL2FXCONST_DBL(1.25f) - fMult(tmp,
                // FL2FXCONST_DBL(1/400000.0f)));
                tmp = fl2fxconst_dbl((1.25f32 / (1 << LSF_SCALE) as f32) as f64).wrapping_sub(
                    f_mult(
                        tmp,
                        fl2fxconst_dbl(((1 << (LSF_SCALE + 4)) as f32 / 400000.0f32) as f64),
                    ),
                );
                if tmp >= fl2fxconst_dbl((1.0f32 / (1 << LSF_SCALE) as f32) as f64) {
                    p_stability[k] = fl2fxconst_sgl((1.0f32 / 2.0f32) as f64);
                } else if tmp < fl2fxconst_dbl(0.0) {
                    p_stability[k] = fl2fxconst_sgl(0.0);
                } else {
                    p_stability[k] = fx_dbl2fx_sgl(tmp << (LSF_SCALE - 1));
                }

                lsf_prev = lsf_curr;
                k = i;
            } else {
                // Mark stability value as undefined.
                p_stability[i] = -1;
            }
        }
    }

    // convert into LSP domain
    for i in 0..(nb_div + 1) {
        if lpc_present[i] != 0 {
            for k in 0..M {
                lsp[i][k] = lsf_to_lsp(lsp[i][k]);
            }
        }
    }

    0
}

/// `CLpc_Conceal(lsp, lpc4_lsf, isf_adaptive_mean, first_lpd_flag)`: generate LPC coefficient
/// sets in case of frame loss.
pub fn clpc_conceal(
    lsp: &mut [[FixpLpc; M]; 5],
    lpc4_lsf: &mut [FixpLpc; M],
    lsf_adaptive_mean: &[FixpLpc; M],
    first_lpd_flag: i32,
) {
    let beta: FixpSgl = fl2fxconst_sgl(0.25f32 as f64);
    let one_beta: FixpSgl = fl2fxconst_sgl(0.75f32 as f64);
    let bfi_fac: FixpSgl = fl2fxconst_sgl(0.90f32 as f64);
    let one_bfi_fac: FixpSgl = fl2fxconst_sgl(0.10f32 as f64);

    // Frame loss concealment (could be improved)

    if first_lpd_flag != 0 {
        // Reset past LSF values
        lpc4_lsf[..M].copy_from_slice(&FDK_DEC_LSF_INIT[..M]);
        lsp[0][..M].copy_from_slice(&lpc4_lsf[..M]);
    } else {
        // old LPC4 is new LPC0
        lsp[0] = *lpc4_lsf;
    }

    // LPC1
    for i in 0..M {
        let lsf_mean: FixpLpc = fx_dbl2fx_lpc(
            f_mult(beta, FDK_DEC_LSF_INIT[i]).wrapping_add(f_mult(one_beta, lsf_adaptive_mean[i])),
        );

        lsp[1][i] =
            fx_dbl2fx_lpc(f_mult(bfi_fac, lpc4_lsf[i]).wrapping_add(f_mult(one_bfi_fac, lsf_mean)));
    }

    // LPC2 - LPC4
    let tenth = fl2fxconst_sgl(0.1f32 as f64) as i32;
    for j in 2..=4usize {
        for i in 0..M {
            // lsf_mean[i] =  FX_DBL2FX_LPC(fMult((FIXP_LPC)(BETA + j * FL2FXCONST_LPC(0.1f)),
            //   fdk_dec_lsf_init[i]) + fMult((FIXP_LPC)(ONE_BETA - j * FL2FXCONST_LPC(0.1f)),
            //   lsf_adaptive_mean[i]));
            let jt = (j as i32 * tenth) as FixpSgl as i32;
            let lsf_mean: FixpLpc = fx_dbl2fx_lpc(
                f_mult((beta as i32 + jt) as FixpSgl, FDK_DEC_LSF_INIT[i])
                    .wrapping_add(f_mult((one_beta as i32 - jt) as FixpSgl, lsf_adaptive_mean[i])),
            );

            lsp[j][i] = fx_dbl2fx_lpc(
                f_mult(bfi_fac, lsp[j - 1][i]).wrapping_add(f_mult(one_bfi_fac, lsf_mean)),
            );
        }
    }

    // Update past values for the future
    *lpc4_lsf = lsp[4];

    // convert into LSP domain
    for j in 0..5 {
        for i in 0..M {
            lsp[j][i] = lsf_to_lsp(lsp[j][i]);
        }
    }
}

/// `E_LPC_a_weight(wA, A, m)`: apply absolute weighting. `wA`: weighted LPC coefficient vector
/// output; `A`: LPC coefficient vector (the first coefficient is implicitly 1.0); `m`: length of
/// `A`. For `wA == A` (in-place weighting in C) use [`e_lpc_a_weight_in_place`].
pub fn e_lpc_a_weight(wa: &mut [FixpLpc], a: &[FixpLpc], m: i32) {
    let mut f: FixpDbl = fl2fxconst_dbl(0.92f32 as f64);
    for i in 0..m.max(0) as usize {
        wa[i] = fx_dbl2fx_lpc(f_mult(a[i], f));
        f = f_mult(f, fl2fxconst_dbl(0.92f32 as f64));
    }
}

/// `E_LPC_a_weight(A, A, m)`: in-place weighting (every element is read before it is written).
pub fn e_lpc_a_weight_in_place(a: &mut [FixpLpc], m: i32) {
    let mut f: FixpDbl = fl2fxconst_dbl(0.92f32 as f64);
    for i in 0..m.max(0) as usize {
        a[i] = fx_dbl2fx_lpc(f_mult(a[i], f));
        f = f_mult(f, fl2fxconst_dbl(0.92f32 as f64));
    }
}

/// `CLpd_DecodeGain(gain, gain_e, gain_code)`: decode the TCX/FAC gain
/// `gain * 2^gain_e = 10^(gain_code/28)`. In case of TCX the lg/sqrt(rms) part must still be
/// applied to obtain the gain value.
pub fn clpd_decode_gain(gain: &mut FixpDbl, gain_e: &mut i32, gain_code: i32) {
    // gain * 2^(gain_e) = 10^(gain_code/28)
    *gain = f_ld_pow(
        fl2fxconst_dbl(3.3219280948873623478703194294894 / 4.0), // log2(10)
        2,
        f_mult_div2(
            gain_code.wrapping_shl((DFRACT_BITS - 1 - 7) as u32),
            fl2fxconst_dbl((2.0f32 / 28.0f32) as f64),
        ),
        7,
        gain_e,
    );
}

/// `SF_F`
const SF_F: i32 = 8;

/// `get_lsppol(lsp, f, n, flag)`: find the polynomial F1(z) (`flag` 1) or F2(z) (`flag` 2) from
/// the LSPs by expanding the product polynomials
/// `F1(z) = product (1 - 2 LSP_i z^-1 + z^-2)` over the even (F2: odd) LSPs, where LSP_i are
/// the LSPs in the cosine domain. `f`: the coefficients of F1 or F2, scaled by 8 bits; `n`: no
/// of coefficients (m/2).
fn get_lsppol(lsp: &[FixpLpc], f: &mut [FixpDbl], n: usize, flag: usize) {
    let mut plsp = flag - 1;
    f[0] = fl2fxconst_dbl((1.0f32 / (1 << SF_F) as f32) as f64);
    let mut b: FixpDbl = fx_lpc2fx_dbl(lsp[plsp]).wrapping_neg();
    f[1] = b >> (SF_F - 1);
    for i in 2..=n {
        plsp += 2;
        b = fx_lpc2fx_dbl(lsp[plsp]).wrapping_neg();
        f[i] = saturate_left_shift(
            f_mult_div2(b, f[i - 1]).wrapping_add(f[i - 2] >> 1),
            2,
            DFRACT_BITS,
        );
        for j in (2..i).rev() {
            f[j] = saturate_left_shift(
                (f[j] >> 2).wrapping_add(f_mult_div2(b, f[j - 1])).wrapping_add(f[j - 2] >> 2),
                2,
                DFRACT_BITS,
            );
        }
        f[1] = f[1].wrapping_add(b >> (SF_F - 1));
    }
}

/// `NC`: `M_LP_FILTER_ORDER / 2`
const NC: usize = M / 2;

/// `E_LPC_f_lsp_a_conversion(lsp, a, a_exp)`: convert LSP coefficients into the LP domain.
/// `a`: output LP filter coefficient vector scaled by `SF_A_COEFFS`. The in-place use of C
/// (`lsp == a`) reads `lsp` completely before writing `a`; callers pass a copy.
pub fn e_lpc_f_lsp_a_conversion(lsp: &[FixpLpc], a: &mut [FixpLpc], a_exp: &mut i32) {
    let mut f1 = [0 as FixpDbl; NC + 1];
    let mut f2 = [0 as FixpDbl; NC + 1];

    // Find the polynomials F1(z) and F2(z)
    get_lsppol(lsp, &mut f1, NC, 1);
    get_lsppol(lsp, &mut f2, NC, 2);

    // Multiply F1(z) by (1+z^-1) and F2(z) by (1-z^-1)
    scale_values(&mut f1, -2);
    scale_values(&mut f2, -2);

    for i in (1..=NC).rev() {
        f1[i] = f1[i].wrapping_add(f1[i - 1]);
        f2[i] = f2[i].wrapping_sub(f2[i - 1]);
    }

    let mut a_dbl = [0 as FixpDbl; M];

    let mut k = M - 1;
    for i in 1..=NC {
        a_dbl[i - 1] = f1[i].wrapping_add(f2[i]);
        a_dbl[k] = f1[i].wrapping_sub(f2[i]);
        k -= 1;
    }

    let headroom_a = get_scalefactor(&a_dbl);

    for i in 0..M {
        a[i] = fx_dbl2fx_lpc(a_dbl[i] << headroom_a);
    }

    *a_exp = SF_F + (2 - 1) - headroom_a;
}
