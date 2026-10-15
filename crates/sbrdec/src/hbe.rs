// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/hbe.h, libSBRdec/src/hbe.cpp.

//! Port of `hbe.h` and `hbe.cpp`: the QMF based harmonic transposer (HBE) of USAC eSBR.
//!
//! # Memory
//!
//! The transposer is allocated with `FDKcalloc` (all zero), its buffers with `FDKcalloc` and
//! `FDKaalloc` (zeroed as well). The port owns them: the 13 rows of 64 values of
//! `qmfInBufReal_F`/`qmfInBufImag_F` and the 11 rows of `qmfHBEBufReal_F`/`qmfHBEBufImag_F` are
//! flat arrays.
//!
//! The analysis and synthesis filter banks use the `LONG` instantiation of `qmf_pcm.h` (time
//! signal `inBuf_F`), with the states `anaQmfStates`/`synQmfStates` of the transposer. The
//! analysis post twiddles are replaced by `post_twiddle_cos_*`/`post_twiddle_sin_*`
//! (`QmfFilterBank::t_cos`/`t_sin` are static slices).
//!
//! # Slots
//!
//! `QmfTransposerApply()` reads the codec QMF data (`qmfBufferCodecReal`) and writes the output
//! (`ppQmfBufferOutReal_F`) through pointer arrays that the callers build from the same array
//! (`pReal = pLowBandReal + ov_len + noCols`, ...). The port takes that one slot array
//! (`&mut [&mut [FixpDbl]]`, see [`crate::sbr_dec`]) and the index of `qmfBufferCodecReal[0]` in it.
//!
//! # Variants and undefined behavior
//!
//! The `.ii` files of the two builds differ only in `FDK_FORCEINLINE` of the root functions,
//! which changes no value (PORTING.md, "Function-level variants"). The shifts by exponent differences
//! that are not limited (`>> g_e`, `>> (scale_factor_hbe - gammaOut_e[k])`, `<<= reserve`) are
//! register shifts in both builds (amount modulo 32): `wrapping_shr`/`wrapping_shl`. Additions
//! that can overflow (signed overflow) wrap.

pub mod tables;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::tools_rom::FixpQtw;
use tables::*;

use crate::lpp_tran::{LPC_ORDER, MAX_NUM_PATCHES};
use crate::sbrdecoder::{SbrError, SBRDEC_OK, SBRDEC_UNSUPPORTED_CONFIG};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::qmf::*;

/// `QMF_SYNTH_CHANNELS`
pub const QMF_SYNTH_CHANNELS: i32 = 64;
/// `HBE_QMF_FILTER_STATE_ANA_SIZE`
pub const HBE_QMF_FILTER_STATE_ANA_SIZE: usize = 400;
/// `HBE_QMF_FILTER_STATE_SYN_SIZE`
pub const HBE_QMF_FILTER_STATE_SYN_SIZE: usize = 200;
/// `MAX_NUM_PATCHES_HBE`
pub const MAX_NUM_PATCHES_HBE: usize = 6;
/// `MAX_STRETCH_HBE`
pub const MAX_STRETCH_HBE: usize = 4;

/// `HBE_MAX_QMF_BANDS`
const HBE_MAX_QMF_BANDS: usize = 40;
/// `HBE_MAX_OUT_SLOTS`
const HBE_MAX_OUT_SLOTS: usize = 11;
/// `QMF_WIN_LEN`: 6 subband slots extra delay to align with HQ - 4 slots to compensate for
/// critical sampling delay - 1 slot to align critical sampling exactly (w additional time domain
/// delay)
const QMF_WIN_LEN: usize = 12 + 6 - 4 - 1;
/// `pmin`
const PMIN: i32 = 12;
/// Row length of the transposer's QMF buffers.
const ROW: usize = QMF_SYNTH_CHANNELS as usize;

/// `KEEP_STATES_SYNCED_MODE`
pub type KeepStatesSyncedMode = i32;
/// `KEEP_STATES_SYNCED_OFF`: normal QMF transposer behaviour
pub const KEEP_STATES_SYNCED_OFF: KeepStatesSyncedMode = 0;
/// `KEEP_STATES_SYNCED_NORMAL`: QMF transposer called for syncing of states; the last 8/14
/// slots are calculated in case next frame is HBE
pub const KEEP_STATES_SYNCED_NORMAL: KeepStatesSyncedMode = 1;
/// `KEEP_STATES_SYNCED_OUTDIFF`: as in the normal case, but the calculated slots are directly
/// written to the overlap area of the buffer; only used in resetSbrDec function
pub const KEEP_STATES_SYNCED_OUTDIFF: KeepStatesSyncedMode = 2;
/// `KEEP_STATES_SYNCED_NOOUT`: QMF transposer is called for syncing of states only, no output
/// is generated at all; only used in resetSbrDec function
pub const KEEP_STATES_SYNCED_NOOUT: KeepStatesSyncedMode = 3;

/// `struct hbeTransposer` (`HANDLE_HBE_TRANSPOSER` is `&mut HbeTransposer`).
#[derive(Clone, Debug)]
pub struct HbeTransposer {
    /// `anaQmfStates`
    pub ana_qmf_states: [FixpDbl; HBE_QMF_FILTER_STATE_ANA_SIZE],
    /// `synQmfStates`
    pub syn_qmf_states: [FixpQss; HBE_QMF_FILTER_STATE_SYN_SIZE],
    /// `xOverQmf`
    pub x_over_qmf: [i32; MAX_NUM_PATCHES_HBE],
    /// `maxStretch`
    pub max_stretch: i32,
    /// `timeDomainWinLen`
    pub time_domain_win_len: i32,
    /// `qmfInBufSize`
    pub qmf_in_buf_size: i32,
    /// `qmfOutBufSize`
    pub qmf_out_buf_size: i32,
    /// `noCols`
    pub no_cols: i32,
    /// `noChannels`
    pub no_channels: i32,
    /// `startBand`
    pub start_band: i32,
    /// `stopBand`
    pub stop_band: i32,
    /// `bSbr41`
    pub b_sbr41: i32,
    /// `inBuf_F`: `QMF_SYNTH_CHANNELS + 20 + 1` values
    pub in_buf_f: Vec<i32>,
    /// `qmfInBufReal_F`: `qmfInBufSize` rows of `QMF_SYNTH_CHANNELS` values
    pub qmf_in_buf_real_f: Vec<FixpDbl>,
    /// `qmfInBufImag_F`
    pub qmf_in_buf_imag_f: Vec<FixpDbl>,
    /// `qmfBufferCodecTempSlot_F`: `QMF_SYNTH_CHANNELS / 2` values
    pub qmf_buffer_codec_temp_slot_f: [FixpDbl; ROW / 2],
    /// `HBEAnalysiscQMF`
    pub hbe_analysisc_qmf: QmfFilterBank,
    /// `HBESynthesisQMF`
    pub hbe_synthesis_qmf: QmfFilterBank,
    /// `synthesisQmfPreModCos_F` (`&preModCos[kstart]`): the offset `kstart`
    pub synthesis_qmf_pre_mod_f: usize,
    /// `qmfHBEBufReal_F`: `HBE_MAX_OUT_SLOTS` rows of `QMF_SYNTH_CHANNELS` values
    pub qmf_hbe_buf_real_f: [FixpDbl; HBE_MAX_OUT_SLOTS * ROW],
    /// `qmfHBEBufImag_F`
    pub qmf_hbe_buf_imag_f: [FixpDbl; HBE_MAX_OUT_SLOTS * ROW],
    /// `bXProducts`
    pub b_x_products: [i32; MAX_STRETCH_HBE],
    /// `kstart`
    pub kstart: i32,
    /// `synthSize`
    pub synth_size: i32,
    /// `highband_exp`
    pub highband_exp: [i32; 2],
    /// `target_exp`
    pub target_exp: [i32; 2],
}

impl Default for HbeTransposer {
    /// The zeroed structure of `FDKcalloc`, without buffers.
    fn default() -> Self {
        HbeTransposer {
            ana_qmf_states: [0; HBE_QMF_FILTER_STATE_ANA_SIZE],
            syn_qmf_states: [0; HBE_QMF_FILTER_STATE_SYN_SIZE],
            x_over_qmf: [0; MAX_NUM_PATCHES_HBE],
            max_stretch: 0,
            time_domain_win_len: 0,
            qmf_in_buf_size: 0,
            qmf_out_buf_size: 0,
            no_cols: 0,
            no_channels: 0,
            start_band: 0,
            stop_band: 0,
            b_sbr41: 0,
            in_buf_f: Vec::new(),
            qmf_in_buf_real_f: Vec::new(),
            qmf_in_buf_imag_f: Vec::new(),
            qmf_buffer_codec_temp_slot_f: [0; ROW / 2],
            hbe_analysisc_qmf: QmfFilterBank::default(),
            hbe_synthesis_qmf: QmfFilterBank::default(),
            synthesis_qmf_pre_mod_f: 0,
            qmf_hbe_buf_real_f: [0; HBE_MAX_OUT_SLOTS * ROW],
            qmf_hbe_buf_imag_f: [0; HBE_MAX_OUT_SLOTS * ROW],
            b_x_products: [0; MAX_STRETCH_HBE],
            kstart: 0,
            synth_size: 0,
            highband_exp: [0; 2],
            target_exp: [0; 2],
        }
    }
}

/// `CUBE_ROOT_BITS`
const CUBE_ROOT_BITS: i32 = 7;
/// `CUBE_ROOT_BITS_MASK`
const CUBE_ROOT_BITS_MASK: i32 = 0x7f;
/// `CUBE_ROOT_FRACT_BITS_MASK`
const CUBE_ROOT_FRACT_BITS_MASK: i32 = 0x007FFFFF;
/// `FOURTHROOT_BITS`
const FOURTHROOT_BITS: i32 = 7;
/// `FOURTHROOT_BITS_MASK`
const FOURTHROOT_BITS_MASK: i32 = 0x7f;
/// `FOURTHROOT_FRACT_BITS_MASK`
const FOURTHROOT_FRACT_BITS_MASK: i32 = 0x007FFFFF;
/// `THREEIGTHROOT_BITS`
const THREEIGTHROOT_BITS: i32 = 7;
/// `THREEIGTHROOT_BITS_MASK`
const THREEIGTHROOT_BITS_MASK: i32 = 0x7f;
/// `THREEIGTHROOT_FRACT_BITS_MASK`
const THREEIGTHROOT_FRACT_BITS_MASK: i32 = 0x007FFFFF;

/// The normalization of the root functions: `exponent = fNormz(op_m) - 1; op_m <<= exponent`,
/// the table index (7 bits) and the fraction.
#[inline(always)]
fn root_normalize(
    op_m: FixpDbl,
    bits: i32,
    bits_mask: i32,
    fract_mask: i32,
) -> (FixpDbl, i32, usize, FixpDbl) {
    let exponent = f_normz(op_m) - 1;
    // op_m > 0 (FDK_ASSERT); for op_m <= 0 the amount is -1 (register shift modulo 32).
    let op_m = op_m.wrapping_shl(exponent as u32);
    let index = ((op_m >> (DFRACT_BITS - 1 - (bits + 1))) & bits_mask) as usize;
    let fract = (op_m & fract_mask) << (bits + 1);
    (op_m, exponent, index, fract)
}

/// `invCubeRootNorm2(op_m, op_e)`: `1/cuberoot(op)` in Q1.31 and the modified exponent.
pub fn inv_cube_root_norm2(op_m: FixpDbl, op_e: &mut i32) -> FixpDbl {
    fdk_assert!(op_m > 0);

    // normalize input, calculate shift value
    let (_, mut exponent, index, fract) =
        root_normalize(op_m, CUBE_ROOT_BITS, CUBE_ROOT_BITS_MASK, CUBE_ROOT_FRACT_BITS_MASK);
    let diff = INV_CUBE_ROOT_TAB[index + 1].wrapping_sub(INV_CUBE_ROOT_TAB[index]);
    let mut op_m = f_mult_add_div2(INV_CUBE_ROOT_TAB[index], diff << 1, fract);

    // calculate the output exponent = input * exp/3 = cubicroot(m)*2^(exp/3) where 2^(exp/3) =
    // 2^k'*2 or 2^k'*2^(1/3) or 2^k'*2^(2/3)
    exponent = exponent.wrapping_sub(*op_e).wrapping_add(3);
    // (INT)fMultDiv2((FIXP_SGL)fAbs(exponent), (FIXP_SGL)0x5556) >> 16
    let mut shift_tmp = (fixmuldiv2_ss(fixabs_i(exponent) as i16, 0x5556)) >> 16;
    if exponent < 0 {
        shift_tmp = -shift_tmp;
    }
    let mut rem = exponent.wrapping_sub(3 * shift_tmp);
    if rem < 0 {
        rem += 3;
        shift_tmp -= 1;
    }

    *op_e = shift_tmp;
    // rem is 0..=2 for the exponents of the transposer; others read 0 in the port.
    let corr = INV_CUBE_ROOT_CORRECTION.get(rem as usize).copied().unwrap_or(0);
    op_m = f_mult_div2(op_m, corr) << 2;

    op_m
}

/// `invFourthRootNorm2(op_m, op_e)`: `1/fourthroot(op)` in Q1.31 and the modified exponent.
pub fn inv_fourth_root_norm2(op_m: FixpDbl, op_e: &mut i32) -> FixpDbl {
    fdk_assert!(op_m > 0);

    // normalize input, calculate shift value
    let (_, mut exponent, index, fract) =
        root_normalize(op_m, FOURTHROOT_BITS, FOURTHROOT_BITS_MASK, FOURTHROOT_FRACT_BITS_MASK);
    let diff = INV_FOURTH_ROOT_TAB[index + 1].wrapping_sub(INV_FOURTH_ROOT_TAB[index]);
    let mut op_m = INV_FOURTH_ROOT_TAB[index].wrapping_add(f_mult_div2(diff, fract) << 1);

    exponent = exponent.wrapping_sub(*op_e).wrapping_add(4);
    let rem = exponent & 0x00000003;
    let shift_tmp = exponent >> 2;

    *op_e = shift_tmp;
    op_m = f_mult_div2(op_m, INV_FOURTH_ROOT_CORRECTION[rem as usize]) << 2;

    op_m
}

/// `inv3EigthRootNorm2(op_m, op_e)`: `1/(op)^(3/8)` and the modified exponent.
pub fn inv3_eigth_root_norm2(op_m: FixpDbl, op_e: &mut i32) -> FixpDbl {
    fdk_assert!(op_m > 0);

    // normalize input, calculate shift op_mue
    let (_, mut exponent, index, fract) = root_normalize(
        op_m,
        THREEIGTHROOT_BITS,
        THREEIGTHROOT_BITS_MASK,
        THREEIGTHROOT_FRACT_BITS_MASK,
    );
    let diff = INV3_EIGTH_ROOT_TAB[index + 1].wrapping_sub(INV3_EIGTH_ROOT_TAB[index]);
    let mut op_m = INV3_EIGTH_ROOT_TAB[index].wrapping_add(f_mult_div2(diff, fract) << 1);

    exponent = exponent.wrapping_sub(*op_e).wrapping_add(8);
    let rem = exponent & 0x00000007;
    let shift_tmp = exponent >> 3;

    *op_e = shift_tmp.wrapping_mul(3);
    op_m = f_mult_div2(op_m, INV3_EIGTH_ROOT_CORRECTION[rem as usize]) << 2;

    f_mult(op_m, f_mult(op_m, op_m))
}

/// `QmfTransposerCreate(hQmfTransposer, frameSize, bDisableCrossProducts, bSbr41)`: allocates
/// a transposer (zeroed) with its buffers.
pub fn qmf_transposer_create(
    h_qmf_transposer: &mut Option<Box<HbeTransposer>>,
    frame_size: i32,
    b_disable_cross_products: i32,
    b_sbr41: i32,
) -> SbrError {
    let mut h = Box::<HbeTransposer>::default();

    for i in 0..MAX_STRETCH_HBE - 1 {
        h.b_x_products[i] = if b_disable_cross_products != 0 { 0 } else { X_PRODUCTS[i] };
    }

    h.time_domain_win_len = frame_size;
    if frame_size == 768 {
        h.no_cols = (8 * frame_size / 3) / QMF_SYNTH_CHANNELS; // 32 for 24:64
    } else {
        // 32 for 32:64 and 64 for 16:64 -> identical to sbrdec->no_cols
        h.no_cols = (b_sbr41 + 1) * 2 * frame_size / QMF_SYNTH_CHANNELS;
    }

    // Guarded: frame sizes of 0..=31 give noCols 0 and 0 channels.
    h.no_channels = if h.no_cols != 0 { frame_size / h.no_cols } else { 0 };

    h.qmf_in_buf_size = QMF_WIN_LEN as i32;
    h.qmf_out_buf_size = 2 * (h.no_cols / 2 + QMF_WIN_LEN as i32 - 1);

    // buffered time signal needs to be delayed by synthesis_size; max synthesis_size = 20;
    h.in_buf_f = vec![0; QMF_SYNTH_CHANNELS as usize + 20 + 1];

    h.qmf_in_buf_real_f = vec![0; QMF_WIN_LEN * ROW];
    h.qmf_in_buf_imag_f = vec![0; QMF_WIN_LEN * ROW];

    h.b_sbr41 = b_sbr41;

    h.highband_exp = [0; 2];
    h.target_exp = [0; 2];

    *h_qmf_transposer = Some(h);

    SBRDEC_OK
}

/// `QmfTransposerReInit(hQmfTransposer, FreqBandTable, NSfb)`: configures the transposer for the
/// frequency band tables (low and high resolution) of the header.
pub fn qmf_transposer_re_init(
    h_qmf_transposer: Option<&mut HbeTransposer>,
    freq_band_table: [&[u8]; 2],
    n_sfb: [u8; 2],
) -> SbrError {
    let Some(h) = h_qmf_transposer else {
        return SBRDEC_OK;
    };
    let fbt = |t: usize, i: usize| freq_band_table[t].get(i).copied().unwrap_or(0) as i32;

    h.start_band = fbt(0, 0);
    // is checked by resetFreqBandTables()
    fdk_assert!((h.b_sbr41 == 0 && h.start_band <= 32) || (h.b_sbr41 != 0 && h.start_band <= 16));
    h.stop_band = fbt(0, n_sfb[0] as usize);

    h.synth_size = 4 * ((h.start_band + 4) / 8 + 1); // 8, 12, 16, 20
                                                     // startBand <= 32 (resetFreqBandTables())
    h.kstart = START_SUBBAND2K_L[(h.start_band.max(0) as usize).min(32)];

    if h.b_sbr41 != 0 {
        if h.kstart + h.synth_size > 16 {
            h.kstart = 16 - h.synth_size;
        }
    } else if h.time_domain_win_len == 768 && h.kstart + h.synth_size > 24 {
        h.kstart = 24 - h.synth_size;
    }

    h.synthesis_qmf_pre_mod_f = h.kstart.max(0) as usize;

    let l = 2 * h.synth_size; // 8, 16, 24, 32, 40
                              // Change analysis post twiddles
    let (tmp_t_cos, tmp_t_sin): (&'static [FixpQtw], &'static [FixpQtw]) = match l {
        8 => (&POST_TWIDDLE_COS_8, &POST_TWIDDLE_SIN_8),
        16 => (&POST_TWIDDLE_COS_16, &POST_TWIDDLE_SIN_16),
        24 => (&POST_TWIDDLE_COS_24, &POST_TWIDDLE_SIN_24),
        32 => (&POST_TWIDDLE_COS_32, &POST_TWIDDLE_SIN_32),
        40 => (&POST_TWIDDLE_COS_40, &POST_TWIDDLE_SIN_40),
        _ => return SBRDEC_UNSUPPORTED_CONFIG,
    };

    let qmf_err = qmf_init_synthesis_filter_bank(
        &mut h.hbe_synthesis_qmf,
        Some(&mut h.syn_qmf_states[..]),
        h.no_cols,
        0,
        h.synth_size,
        h.synth_size,
        1,
    );
    if qmf_err != 0 {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    let qmf_err = qmf_init_analysis_filter_bank::<FixpDbl>(
        &mut h.hbe_analysisc_qmf,
        Some(&mut h.ana_qmf_states[..]),
        h.no_cols / 2,
        0,
        2 * h.synth_size,
        2 * h.synth_size,
        0,
    );

    if qmf_err != 0 {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    h.hbe_analysisc_qmf.t_cos = tmp_t_cos;
    h.hbe_analysisc_qmf.t_sin = tmp_t_sin;

    h.x_over_qmf[..MAX_NUM_PATCHES].fill(0); // global
    let mut sfb = 0usize;
    let stop_patch;
    if h.b_sbr41 != 0 {
        stop_patch = MAX_NUM_PATCHES as i32;
        h.max_stretch = MAX_STRETCH_HBE as i32;
    } else {
        stop_patch = MAX_STRETCH_HBE as i32;
    }

    for patch in 1..=stop_patch {
        while sfb <= n_sfb[0] as usize && fbt(0, sfb) <= patch * h.start_band {
            sfb += 1;
        }
        if sfb <= n_sfb[0] as usize {
            // If the distance is larger than three QMF bands - try aligning to high resolution
            // frequency bands instead.
            if (patch * h.start_band - fbt(0, sfb.wrapping_sub(1))) <= 3 {
                h.x_over_qmf[patch as usize - 1] = fbt(0, sfb.wrapping_sub(1));
            } else {
                let mut sfb_tmp = 0usize;
                while sfb_tmp <= n_sfb[1] as usize && fbt(1, sfb_tmp) <= patch * h.start_band {
                    sfb_tmp += 1;
                }
                h.x_over_qmf[patch as usize - 1] = fbt(1, sfb_tmp.wrapping_sub(1));
            }
        } else {
            h.x_over_qmf[patch as usize - 1] = h.stop_band;
            h.max_stretch = f_min(patch, MAX_STRETCH_HBE as i32);
            break;
        }
    }

    h.highband_exp = [0; 2];
    h.target_exp = [0; 2];

    SBRDEC_OK
}

/// `QmfTransposerClose(hQmfTransposer)`: frees the transposer.
pub fn qmf_transposer_close(h_qmf_transposer: &mut Option<Box<HbeTransposer>>) {
    *h_qmf_transposer = None;
}

/// `scaleUp(real_m, imag_m, _e)`: shifts the mantissas up as far as possible (leaving one bit
/// headroom), limited by the exponent.
#[inline]
pub fn scale_up(real_m: &mut FixpDbl, imag_m: &mut FixpDbl, e: &mut i32) {
    // shift gc_r and gc_i up if possible
    let mut reserve =
        cnt_leading_zeros((*real_m ^ (*real_m >> 31)) | (*imag_m ^ (*imag_m >> 31))) - 1;
    // Leave one bit headroom such that (real_m^2 + imag_m^2) does not overflow later if both
    // are 0x80000000.
    reserve = f_max(reserve - 1, 0);
    reserve = f_min(reserve, *e);
    fdk_assert!(reserve >= 0);
    // reserve is negative for a negative exponent: register shift (amount modulo 32)
    *real_m = real_m.wrapping_shl(reserve as u32);
    *imag_m = imag_m.wrapping_shl(reserve as u32);
    *e -= reserve;
}

/// `calculateCenterFIXP()`: the normalized phase vector of `gammaVec` raised to the power
/// `mult + 1` (`mult` 0..=2), and its exponent.
#[allow(clippy::too_many_arguments)]
pub fn calculate_center_fixp(
    mut gamma_vec_real: FixpDbl,
    mut gamma_vec_imag: FixpDbl,
    center_real: &mut FixpDbl,
    center_imag: &mut FixpDbl,
    exponent: &mut i32,
    stretch: i32,
    mult: i32,
) {
    scale_up(&mut gamma_vec_real, &mut gamma_vec_imag, exponent);
    let energy = f_pow2_div2(gamma_vec_real).wrapping_add(f_pow2_div2(gamma_vec_imag));

    if energy != 0 {
        let mut factor_m: FixpDbl = 0;
        let mut factor_e = 2 * *exponent + 1;

        match stretch {
            2 => factor_m = inv_fourth_root_norm2(energy, &mut factor_e),
            3 => factor_m = inv_cube_root_norm2(energy, &mut factor_e),
            4 => factor_m = inv3_eigth_root_norm2(energy, &mut factor_e),
            _ => {}
        }

        let mut gc_r_m = f_mult_div2(gamma_vec_real, factor_m); // exponent = HBE_SCALE + factor_e + 1
        let mut gc_i_m = f_mult_div2(gamma_vec_imag, factor_m); // exponent = HBE_SCALE + factor_e + 1
        let mut gc_e = *exponent + factor_e + 1;

        scale_up(&mut gc_r_m, &mut gc_i_m, &mut gc_e);

        match mult {
            0 => {
                *center_real = gc_r_m;
                *center_imag = gc_i_m;
            }
            1 => {
                *center_real = f_pow2_div2(gc_r_m).wrapping_sub(f_pow2_div2(gc_i_m));
                *center_imag = f_mult(gc_r_m, gc_i_m);
                gc_e = 2 * gc_e + 1;
            }
            2 => {
                let tmp_r = gc_r_m;
                let tmp_i = gc_i_m;
                gc_r_m = f_pow2_div2(gc_r_m).wrapping_sub(f_pow2_div2(gc_i_m));
                gc_i_m = f_mult(tmp_r, gc_i_m);
                gc_e = 3 * gc_e + 1 + 1;
                let (re, im) = cplx_mult_div2(gc_r_m, gc_i_m, tmp_r, tmp_i);
                *center_real = re;
                *center_imag = im;
            }
            _ => {}
        }

        scale_up(center_real, center_imag, &mut gc_e);

        fdk_assert!(gc_e >= 0);
        *exponent = gc_e;
    } else {
        *center_real = energy; // energy = 0
        *center_imag = energy; // energy = 0
        *exponent = energy;
    }
}

/// `getHBEScaleFactorFrame(bSbr41, maxStretch, pitchInBins)`
pub fn get_hbe_scale_factor_frame(b_sbr41: i32, max_stretch: i32, pitch_in_bins: i32) -> i32 {
    if pitch_in_bins >= PMIN * (1 + b_sbr41) {
        // crossproducts enabled
        26
    } else if max_stretch == 2 {
        24
    } else {
        25
    }
}

/// `addHighBandPart()`: adds the contribution of one phase vector to the HBE output buffers.
#[allow(clippy::too_many_arguments)]
pub fn add_high_band_part(
    mut g_r_m: FixpDbl,
    mut g_i_m: FixpDbl,
    mut g_e: i32,
    mult: FixpDbl,
    gamma_center_real_m: FixpDbl,
    gamma_center_imag_m: FixpDbl,
    gamma_center_e: i32,
    stretch: i32,
    scale_factor_hbe: i32,
    qmf_hbe_buf_real_f: &mut FixpDbl,
    qmf_hbe_buf_imag_f: &mut FixpDbl,
) {
    if (g_r_m | g_i_m) != 0 {
        let mut factor_m: FixpDbl = 0;
        let add = if stretch == 4 { 1 } else { 0 };
        let shift = if stretch == 4 { 1 } else { 2 };

        scale_up(&mut g_r_m, &mut g_i_m, &mut g_e);
        let energy = f_pow2_add_div2(f_pow2_div2(g_r_m), g_i_m);
        let mut factor_e = 2 * g_e + 1;

        match stretch {
            2 => factor_m = inv_fourth_root_norm2(energy, &mut factor_e),
            3 => factor_m = inv_cube_root_norm2(energy, &mut factor_e),
            4 => factor_m = inv3_eigth_root_norm2(energy, &mut factor_e),
            _ => {}
        }

        factor_m = f_mult(factor_m, mult);

        let (tmp_r, tmp_i) = cplx_mult_div2(g_r_m, g_i_m, gamma_center_real_m, gamma_center_imag_m);

        g_r_m = f_mult_div2(tmp_r, factor_m) << shift;
        g_i_m = f_mult_div2(tmp_i, factor_m) << shift;
        g_e = scale_factor_hbe - (g_e + factor_e + gamma_center_e + add);
        g_e = f_max(0, g_e);
        // g_e can be 32 or more: register shift (amount modulo 32)
        *qmf_hbe_buf_real_f = qmf_hbe_buf_real_f.wrapping_add(g_r_m.wrapping_shr(g_e as u32));
        *qmf_hbe_buf_imag_f = qmf_hbe_buf_imag_f.wrapping_add(g_i_m.wrapping_shr(g_e as u32));
    }
}

/// `hintReal_F[i % 4][c]`: for a negative source band (FDK_ASSERT(sourceband >= 0)) the
/// reference has no defined result; the port reads 0 there.
#[inline(always)]
fn hint(i: i32, c: usize) -> FixpDbl {
    if (0..4).contains(&i) {
        HINT_REAL_F[i as usize][c]
    } else {
        0
    }
}

/// `factors[i % 4]` (see [`hint`])
#[inline(always)]
fn factor(i: i32) -> FixpDbl {
    if (0..4).contains(&i) {
        FACTORS[i as usize]
    } else {
        0
    }
}

/// `qmfInBufReal_F[slot][band]` / `qmfInBufImag_F[slot][band]`: the rows are separate
/// allocations in C; an index outside a row reads 0.
#[inline(always)]
fn in_buf(buf: &[FixpDbl], slot: i32, band: i32) -> FixpDbl {
    if (0..ROW as i32).contains(&band) && slot >= 0 {
        buf.get(slot as usize * ROW + band as usize).copied().unwrap_or(0)
    } else {
        0
    }
}

/// `QmfTransposerApply()`: the harmonic transposition of one frame.
///
/// `slots_re`/`slots_im` are the QMF slots of the caller; `qmfBufferCodecReal[i]` is
/// `slots_re[codec_offset + i]` and `ppQmfBufferOutReal_F[i]` is `slots_re[i]`. `lpc_states` are
/// `lpcFilterStatesReal`/`lpcFilterStatesImag` (14 rows of 64 values each), which are only used
/// with `KEEP_STATES_SYNCED_OFF` (`None` otherwise).
#[allow(clippy::too_many_arguments)]
pub fn qmf_transposer_apply(
    h: &mut HbeTransposer,
    slots_re: &mut [&mut [FixpDbl]],
    slots_im: &mut [&mut [FixpDbl]],
    codec_offset: usize,
    n_cols_in: i32,
    lpc_states: Option<(&mut [FixpDbl], &mut [FixpDbl])>,
    pitch_in_bins: i32,
    scale_lb: i32,
    scale_hbe: i32,
    scale_hb: &mut i32,
    time_step: i32,
    first_slot_offsset: i32,
    ov_len: i32,
    keep_states_synced_mode: KeepStatesSyncedMode,
) {
    let qmf_vocoder_cols_in = h.no_cols / 2;
    let b_sbr41 = h.b_sbr41;

    const SLOT_OFFSET: i32 = 6; // hQmfTransposer->winLen-6;

    let qmf_offset = 2 * h.kstart;
    let scale_border = if n_cols_in == 64 { 32 } else { n_cols_in };

    let mut offset = 0; // where to take QmfTransposer data

    let pre_mod = h.synthesis_qmf_pre_mod_f.min(PRE_MOD_COS.len());
    let sign_pre_mod: i32 =
        if PRE_MOD_COS[pre_mod.min(PRE_MOD_COS.len() - 1)] < 0 { 1 } else { -1 };

    let mut scale_factor_hbe = get_hbe_scale_factor_frame(b_sbr41, h.max_stretch, pitch_in_bins);

    if keep_states_synced_mode != KEEP_STATES_SYNCED_OFF {
        offset = h.no_cols - ov_len - LPC_ORDER as i32;
    }

    h.highband_exp[0] = h.highband_exp[1];
    h.target_exp[0] = h.target_exp[1];

    h.highband_exp[1] = scale_factor_hbe;
    h.target_exp[1] = fix_max(h.highband_exp[1], h.highband_exp[0]);

    scale_factor_hbe = h.target_exp[1];

    let shift_ov = h.target_exp[0] - h.target_exp[1];

    if shift_ov != 0 {
        scale_values_saturate(&mut h.qmf_hbe_buf_real_f, shift_ov);
        scale_values_saturate(&mut h.qmf_hbe_buf_imag_f, shift_ov);

        if keep_states_synced_mode == KEEP_STATES_SYNCED_OFF {
            let n_bands = f_max(0, h.stop_band - h.start_band) as usize;
            let sb = h.start_band.max(0) as usize;

            for i in (time_step * first_slot_offsset).max(0)..ov_len {
                let i = i as usize;
                if let Some(s) = slots_re.get_mut(i).and_then(|s| s.get_mut(sb..sb + n_bands)) {
                    scale_values_saturate(s, shift_ov);
                }
                if let Some(s) = slots_im.get_mut(i).and_then(|s| s.get_mut(sb..sb + n_bands)) {
                    scale_values_saturate(s, shift_ov);
                }
            }

            // shift lpc filterstates
            if let Some((lpc_re, lpc_im)) = lpc_states {
                for i in 0..(time_step * first_slot_offsset + LPC_ORDER as i32).max(0) as usize {
                    if let Some(s) = lpc_re.get_mut(i * 64..i * 64 + 64) {
                        scale_values_saturate(s, shift_ov);
                    }
                    if let Some(s) = lpc_im.get_mut(i * 64..i * 64 + 64) {
                        scale_values_saturate(s, shift_ov);
                    }
                }
            }
        }
    }

    let mut twid_m_new = [[0 as FixpDbl; 2]; 3]; // [stretch][cos/sin]
    let stepsize = 1 + (b_sbr41 == 0) as i32;
    let sine_offset = 24;
    let modulo = 96;

    for s in 0..=MAX_STRETCH_HBE - 2 {
        let a = MULT[s].wrapping_mul(stepsize.wrapping_mul(pitch_in_bins));
        twid_m_new[s][0] = TWIDDLE[a.rem_euclid(modulo) as usize];
        twid_m_new[s][1] = TWIDDLE[(a + sine_offset).rem_euclid(modulo) as usize];
    }

    let synth_size = h.synth_size.max(0) as usize;
    let kstart = h.kstart;
    let codec_at = |slots: &[&mut [FixpDbl]], s: i32, k: i32| -> FixpDbl {
        if s < 0 || k < 0 {
            return 0;
        }
        slots.get(codec_offset + s as usize).and_then(|sl| sl.get(k as usize)).copied().unwrap_or(0)
    };

    // Time-stretch
    for j in 0..qmf_vocoder_cols_in {
        // update inbuf
        for i in 0..synth_size {
            h.in_buf_f[i] = h.in_buf_f[i + 2 * synth_size];
        }

        // run synthesis for two sbr slots as transposer uses half slots double bands
        // representation
        for z in 0..2 {
            let scale_factor =
                if n_cols_in == 64 && (2 * j + z) < scale_border { scale_lb } else { scale_hbe };
            let codec_temp_e = scale_factor - 1; // -2 for Div2 and cos/sin scale of 1

            for k in 0..synth_size {
                let ki = kstart + k as i32;
                let pm = pre_mod + k;
                let cos = PRE_MOD_COS.get(pm).copied().unwrap_or(0);
                let sin = PRE_MOD_SIN.get(pm).copied().unwrap_or(0);
                let mut v =
                    f_mult_div2(sign_pre_mod.wrapping_mul(cos), codec_at(slots_re, 2 * j + z, ki));
                v = v.wrapping_add(f_mult_div2(
                    sign_pre_mod.wrapping_mul(sin),
                    codec_at(slots_im, 2 * j + z, ki),
                ));
                h.qmf_buffer_codec_temp_slot_f[k] = v;
            }

            let mut p_work_buffer = [0 as FixpDbl; HBE_MAX_QMF_BANDS << 1];

            let off = synth_size * (z as usize + 1);
            qmf_synthesis_filtering_slot::<i32>(
                &h.hbe_synthesis_qmf,
                &mut h.syn_qmf_states,
                &h.qmf_buffer_codec_temp_slot_f,
                &[],
                0,
                -7 - h.hbe_synthesis_qmf.filter_scale - codec_temp_e + 1,
                &mut h.in_buf_f[off..],
                1,
                &mut p_work_buffer,
            );
        }

        {
            let mut p_work_buffer = [0 as FixpDbl; HBE_MAX_QMF_BANDS << 1];
            let last = (QMF_WIN_LEN - 1) * ROW;
            qmf_analysis_filtering_slot::<i32>(
                &h.hbe_analysisc_qmf,
                &mut h.ana_qmf_states,
                &mut h.qmf_in_buf_real_f[last..last + ROW],
                &mut h.qmf_in_buf_imag_f[last..last + ROW],
                &h.in_buf_f[1..],
                1,
                &mut p_work_buffer,
            );
        }

        let ana_no_channels = (h.hbe_analysisc_qmf.no_channels.max(0) as usize).min(ROW);

        if keep_states_synced_mode == KEEP_STATES_SYNCED_NORMAL
            && j <= qmf_vocoder_cols_in
                - ((LPC_ORDER as i32 + ov_len + QMF_WIN_LEN as i32 - 1) >> 1)
        {
            // update in buffer
            for i in 0..QMF_WIN_LEN - 1 {
                h.qmf_in_buf_real_f
                    .copy_within((i + 1) * ROW..(i + 1) * ROW + ana_no_channels, i * ROW);
                h.qmf_in_buf_imag_f
                    .copy_within((i + 1) * ROW..(i + 1) * ROW + ana_no_channels, i * ROW);
            }
            continue;
        }

        for stretch in 2..=h.max_stretch.min(MAX_STRETCH_HBE as i32) {
            let wl = WIN_LENGTH[(stretch - 2) as usize];
            let start = SLOT_OFFSET - wl / 2;
            let stop = SLOT_OFFSET + wl / 2;

            let mut factor_v: FixpDbl;

            let band_lo = h.x_over_qmf[(stretch - 2) as usize];
            let band_hi = h.x_over_qmf[(stretch - 1) as usize];
            for band in band_lo..band_hi {
                let mut gamma_center_real_m = [0 as FixpDbl; 2];
                let mut gamma_center_imag_m = [0 as FixpDbl; 2];
                let mut gamma_center_e = [0i32; 2];

                let mut gamma_vec_real_m = [0 as FixpDbl; 2];
                let mut gamma_vec_imag_m = [0 as FixpDbl; 2];
                let mut gamma_vec_e = [0i32; 2];

                let mut wingain: FixpDbl = 0;

                let sc_e = 15i32.wrapping_add(h.hbe_analysisc_qmf.out_scalefactor); // SCALE2EXP(-outScalefactor)
                gamma_center_e[0] = sc_e;
                gamma_center_e[1] = sc_e;

                // interpolation filters for 3rd order
                let mut sourceband = 2 * band / stretch - qmf_offset;
                fdk_assert!(sourceband >= 0);

                // maximum gammaCenter_e == 20
                let (re, im) = (
                    in_buf(&h.qmf_in_buf_real_f, SLOT_OFFSET, sourceband),
                    in_buf(&h.qmf_in_buf_imag_f, SLOT_OFFSET, sourceband),
                );
                calculate_center_fixp(
                    re,
                    im,
                    &mut gamma_center_real_m[0],
                    &mut gamma_center_imag_m[0],
                    &mut gamma_center_e[0],
                    stretch,
                    stretch - 2,
                );

                let r: i32;
                let p_slot_stretch: &[i32];
                let p_filt: &[i32];
                if stretch == 4 {
                    r = band - 2 * (band / 2);
                    sourceband += if r == 0 { -1 } else { 1 };
                    p_slot_stretch = &SLOT_STRETCH4;
                    factor_v = fl2fxconst_dbl((2.0f32 / 3.0f32) as f64);
                    p_filt = &FILT_DUMMY;
                } else if stretch == 2 {
                    r = 0;
                    sourceband = 2 * band / stretch - qmf_offset;
                    p_slot_stretch = &SLOT_STRETCH2;
                    factor_v = fl2fxconst_dbl((1.0f32 / 3.0f32) as f64);
                    p_filt = &FILT_DUMMY;
                } else {
                    r = 2 * band - 3 * (2 * band / 3);
                    sourceband = 2 * band / stretch - qmf_offset;
                    p_slot_stretch = &SLOT_STRETCH3;
                    factor_v = fl2fxconst_dbl((1.4142f32 / 3.0f32) as f64);
                    p_filt = &FILT_STRETCH3;
                }

                if r == 2 {
                    let re = in_buf(&h.qmf_in_buf_real_f, SLOT_OFFSET, sourceband + 1);
                    let im = in_buf(&h.qmf_in_buf_imag_f, SLOT_OFFSET, sourceband + 1);
                    let (mut cr, mut ci, mut ce) =
                        (gamma_center_real_m[1], gamma_center_imag_m[1], gamma_center_e[1]);
                    calculate_center_fixp(re, im, &mut cr, &mut ci, &mut ce, stretch, stretch - 2);
                    gamma_center_real_m[1] = cr;
                    gamma_center_imag_m[1] = ci;
                    gamma_center_e[1] = ce;

                    factor_v = fl2fxconst_dbl((1.4142f32 / 6.0f32) as f64);
                }

                let sb = sourceband;
                let hr = &h.qmf_in_buf_real_f;
                let hi = &h.qmf_in_buf_imag_f;
                if r == 2 {
                    for k in start..stop {
                        let ku = k as usize;
                        let ps = p_slot_stretch[ku];
                        gamma_vec_real_m[0] = in_buf(hr, ps, sb);
                        gamma_vec_real_m[1] = in_buf(hr, ps, sb + 1);
                        gamma_vec_imag_m[0] = in_buf(hi, ps, sb);
                        gamma_vec_imag_m[1] = in_buf(hi, ps, sb + 1);
                        gamma_vec_e[0] = sc_e;
                        gamma_vec_e[1] = sc_e;

                        if p_filt[ku] == 1 {
                            let mut tmp_real_f = gamma_vec_real_m[0];
                            // sum should be <= 1 because of sin/cos multiplication
                            gamma_vec_real_m[0] = f_mult(gamma_vec_real_m[0], hint(sb % 4, 1))
                                .wrapping_sub(f_mult(gamma_vec_imag_m[0], hint((sb + 3) % 4, 1)))
                                >> 1;
                            gamma_vec_imag_m[0] = f_mult(tmp_real_f, hint((sb + 3) % 4, 1))
                                .wrapping_add(f_mult(gamma_vec_imag_m[0], hint(sb % 4, 1)))
                                >> 1;

                            tmp_real_f = in_buf(hr, ps + 1, sb);
                            let tmp_imag_f = in_buf(hi, ps + 1, sb);

                            gamma_vec_real_m[0] = gamma_vec_real_m[0].wrapping_add(
                                f_mult(tmp_real_f, hint(sb % 4, 1))
                                    .wrapping_sub(f_mult(tmp_imag_f, hint((sb + 1) % 4, 1)))
                                    >> 1,
                            );
                            gamma_vec_imag_m[0] = gamma_vec_imag_m[0].wrapping_add(
                                f_mult(tmp_real_f, hint((sb + 1) % 4, 1))
                                    .wrapping_add(f_mult(tmp_imag_f, hint(sb % 4, 1)))
                                    >> 1,
                            );
                            gamma_vec_e[0] += 1;

                            tmp_real_f = gamma_vec_real_m[1];

                            gamma_vec_real_m[1] = f_mult(gamma_vec_real_m[1], hint(sb % 4, 2))
                                .wrapping_sub(f_mult(gamma_vec_imag_m[1], hint((sb + 3) % 4, 2)))
                                >> 1;
                            gamma_vec_imag_m[1] = f_mult(tmp_real_f, hint((sb + 3) % 4, 2))
                                .wrapping_add(f_mult(gamma_vec_imag_m[1], hint(sb % 4, 2)))
                                >> 1;

                            tmp_real_f = in_buf(hr, ps + 1, sb + 1);
                            let tmp_imag_f = in_buf(hi, ps + 1, sb + 1);

                            gamma_vec_real_m[1] = gamma_vec_real_m[1].wrapping_add(
                                f_mult(tmp_real_f, hint(sb % 4, 2))
                                    .wrapping_sub(f_mult(tmp_imag_f, hint((sb + 1) % 4, 2)))
                                    >> 1,
                            );
                            gamma_vec_imag_m[1] = gamma_vec_imag_m[1].wrapping_add(
                                f_mult(tmp_real_f, hint((sb + 1) % 4, 2))
                                    .wrapping_add(f_mult(tmp_imag_f, hint(sb % 4, 2)))
                                    >> 1,
                            );
                            gamma_vec_e[1] += 1;
                        }

                        let idx = ku * ROW + band as usize;
                        let (bre, bim) = (&mut h.qmf_hbe_buf_real_f, &mut h.qmf_hbe_buf_imag_f);
                        if idx < bre.len() {
                            add_high_band_part(
                                gamma_vec_real_m[1],
                                gamma_vec_imag_m[1],
                                gamma_vec_e[1],
                                factor_v,
                                gamma_center_real_m[0],
                                gamma_center_imag_m[0],
                                gamma_center_e[0],
                                stretch,
                                scale_factor_hbe,
                                &mut bre[idx],
                                &mut bim[idx],
                            );

                            add_high_band_part(
                                gamma_vec_real_m[0],
                                gamma_vec_imag_m[0],
                                gamma_vec_e[0],
                                factor_v,
                                gamma_center_real_m[1],
                                gamma_center_imag_m[1],
                                gamma_center_e[1],
                                stretch,
                                scale_factor_hbe,
                                &mut bre[idx],
                                &mut bim[idx],
                            );
                        }
                    }
                } else {
                    for k in start..stop {
                        let ku = k as usize;
                        let ps = p_slot_stretch[ku];
                        gamma_vec_real_m[0] = in_buf(hr, ps, sb);
                        gamma_vec_imag_m[0] = in_buf(hi, ps, sb);
                        gamma_vec_e[0] = sc_e;

                        if p_filt[ku] == 1 {
                            let mut tmp_real_f = gamma_vec_real_m[0];
                            // sum should be <= 1 because of sin/cos multiplication
                            gamma_vec_real_m[0] = f_mult(gamma_vec_real_m[0], hint(sb % 4, 1))
                                .wrapping_sub(f_mult(gamma_vec_imag_m[0], hint((sb + 3) % 4, 1)))
                                >> 1;
                            gamma_vec_imag_m[0] = f_mult(tmp_real_f, hint((sb + 3) % 4, 1))
                                .wrapping_add(f_mult(gamma_vec_imag_m[0], hint(sb % 4, 1)))
                                >> 1;

                            tmp_real_f = in_buf(hr, ps + 1, sb);
                            let tmp_imag_f = in_buf(hi, ps + 1, sb);

                            gamma_vec_real_m[0] = gamma_vec_real_m[0].wrapping_add(
                                f_mult(tmp_real_f, hint(sb % 4, 1))
                                    .wrapping_sub(f_mult(tmp_imag_f, hint((sb + 1) % 4, 1)))
                                    >> 1,
                            );
                            gamma_vec_imag_m[0] = gamma_vec_imag_m[0].wrapping_add(
                                f_mult(tmp_real_f, hint((sb + 1) % 4, 1))
                                    .wrapping_add(f_mult(tmp_imag_f, hint(sb % 4, 1)))
                                    >> 1,
                            );
                            gamma_vec_e[0] += 1;
                        }

                        let idx = ku * ROW + band as usize;
                        if idx < h.qmf_hbe_buf_real_f.len() {
                            let (bre, bim) = (&mut h.qmf_hbe_buf_real_f, &mut h.qmf_hbe_buf_imag_f);
                            add_high_band_part(
                                gamma_vec_real_m[0],
                                gamma_vec_imag_m[0],
                                gamma_vec_e[0],
                                factor_v,
                                gamma_center_real_m[0],
                                gamma_center_imag_m[0],
                                gamma_center_e[0],
                                stretch,
                                scale_factor_hbe,
                                &mut bre[idx],
                                &mut bim[idx],
                            );
                        }
                    }
                }

                // pitchInBins is given with the resolution of a 768 bins FFT and we need 64 QMF
                // units so factor 768/64 = 12
                if pitch_in_bins >= PMIN * (1 + b_sbr41) {
                    let mut m_tr = 0;
                    let mut ts1 = 0;
                    let mut ts2 = 0;
                    let mut m_val_e = 0;
                    let mut temp_e;
                    let mut sqmag0_e = sc_e;

                    let mut m_val_f: FixpDbl = 0;
                    let mut sign: i32 = -1;

                    // consistent with the already computed for stretch = 3,4.
                    let sourceband = 2 * band / stretch - qmf_offset;
                    fdk_assert!(sourceband >= 0);

                    let mut sqmag0r_f = in_buf(hr, SLOT_OFFSET, sourceband);
                    let mut sqmag0i_f = in_buf(hi, SLOT_OFFSET, sourceband);
                    scale_up(&mut sqmag0r_f, &mut sqmag0i_f, &mut sqmag0_e);

                    let mut sqmag0_f = f_pow2_div2(sqmag0r_f);
                    sqmag0_f = sqmag0_f.wrapping_add(f_pow2_div2(sqmag0i_f));
                    sqmag0_e = 2 * sqmag0_e + 1;

                    for tr in 1..stretch {
                        let mut sqmag1_e = sc_e;
                        let mut sqmag2_e = sc_e;

                        let tmp_band = BAND_F[(band.max(0) as usize).min(63)];
                        let p = P_F[(pitch_in_bins.max(0) as usize).min(127)] >> b_sbr41;
                        let tr_p = f_mult(p, TR_STR[(tr - 1) as usize]); // scale 7
                        let f1_f =
                            f_mult(tmp_band.wrapping_sub(tr_p), STRETCHFAC[(stretch - 2) as usize]); // scale 7
                        let ti1 = (f1_f >> (DFRACT_BITS - 1 - 7)) - qmf_offset;
                        let ti2 = (f1_f.wrapping_add(p >> 2) >> (DFRACT_BITS - 1 - 7)) - qmf_offset;

                        if ti1 >= 0 && ti2 < 2 * h.synth_size {
                            let mut sqmag1r_f = in_buf(hr, SLOT_OFFSET, ti1);
                            let mut sqmag1i_f = in_buf(hi, SLOT_OFFSET, ti1);
                            scale_up(&mut sqmag1r_f, &mut sqmag1i_f, &mut sqmag1_e);
                            let mut sqmag1_f = f_pow2_div2(sqmag1r_f);
                            sqmag1_f = sqmag1_f.wrapping_add(f_pow2_div2(sqmag1i_f));
                            sqmag1_e = 2 * sqmag1_e + 1;

                            let mut sqmag2r_f = in_buf(hr, SLOT_OFFSET, ti2);
                            let mut sqmag2i_f = in_buf(hi, SLOT_OFFSET, ti2);
                            scale_up(&mut sqmag2r_f, &mut sqmag2i_f, &mut sqmag2_e);
                            let mut sqmag2_f = f_pow2_div2(sqmag2r_f);
                            sqmag2_f = sqmag2_f.wrapping_add(f_pow2_div2(sqmag2i_f));
                            sqmag2_e = 2 * sqmag2_e + 1;

                            let shift1 = f_min(f_max(sqmag1_e, sqmag2_e) - sqmag1_e, 31);
                            let shift2 = f_min(f_max(sqmag1_e, sqmag2_e) - sqmag2_e, 31);

                            let temp_f = f_min(sqmag1_f >> shift1, sqmag2_f >> shift2);
                            temp_e = f_max(sqmag1_e, sqmag2_e);

                            let shift3 = f_min(f_max(temp_e, m_val_e) - temp_e, 31);
                            let shift4 = f_min(f_max(temp_e, m_val_e) - m_val_e, 31);

                            if (temp_f >> shift3) > (m_val_f >> shift4) {
                                m_val_f = temp_f;
                                m_val_e = temp_e; // equals sqmag2_e + shift2
                                m_tr = tr;
                                ts1 = ti1;
                                ts2 = ti2;
                            }
                        }
                    }

                    let shift1 = f_min(f_max(sqmag0_e, m_val_e) - sqmag0_e, 31);
                    let shift2 = f_min(f_max(sqmag0_e, m_val_e) - m_val_e, 31);

                    if (m_val_f >> shift2) > (sqmag0_f >> shift1)
                        && ts1 >= 0
                        && ts2 < 2 * h.synth_size
                    {
                        let mut gamma_out_e = [0i32; 2];
                        let mut gamma_out_real_m = [0 as FixpDbl; 2];
                        let mut gamma_out_imag_m = [0 as FixpDbl; 2];

                        // default phase power parameters
                        let mut t_center = stretch - m_tr;
                        let mut t_vec = m_tr;
                        // 2 tap block creation design depends on stretch order
                        match stretch {
                            2 => {
                                wingain = fl2fxconst_dbl((5.0f32 / 12.0f32) as f64); // sum of taps divided by two

                                if h.b_x_products[0] != 0 {
                                    gamma_center_real_m[0] = in_buf(hr, SLOT_OFFSET, ts1);
                                    gamma_center_imag_m[0] = in_buf(hi, SLOT_OFFSET, ts1);

                                    for k in 0..2 {
                                        gamma_vec_real_m[k] =
                                            in_buf(hr, SLOT_OFFSET - 1 + k as i32, ts2);
                                        gamma_vec_imag_m[k] =
                                            in_buf(hi, SLOT_OFFSET - 1 + k as i32, ts2);
                                    }

                                    gamma_center_e[0] = sc_e;
                                    gamma_vec_e[0] = sc_e;
                                    gamma_vec_e[1] = sc_e;
                                }
                            }
                            4 => {
                                wingain = fl2fxconst_dbl((6.0f32 / 12.0f32) as f64); // sum of taps divided by two
                                if h.b_x_products[2] != 0 {
                                    if m_tr == 1 {
                                        gamma_center_real_m[0] = in_buf(hr, SLOT_OFFSET, ts1);
                                        gamma_center_imag_m[0] = in_buf(hi, SLOT_OFFSET, ts1);

                                        for k in 0..2i32 {
                                            gamma_vec_real_m[k as usize] =
                                                in_buf(hr, SLOT_OFFSET + 2 * (k - 1), ts2);
                                            gamma_vec_imag_m[k as usize] =
                                                in_buf(hi, SLOT_OFFSET + 2 * (k - 1), ts2);
                                        }
                                    } else if m_tr == 2 {
                                        gamma_center_real_m[0] = in_buf(hr, SLOT_OFFSET, ts1);
                                        gamma_center_imag_m[0] = in_buf(hi, SLOT_OFFSET, ts1);

                                        for k in 0..2i32 {
                                            gamma_vec_real_m[k as usize] =
                                                in_buf(hr, SLOT_OFFSET + (k - 1), ts2);
                                            gamma_vec_imag_m[k as usize] =
                                                in_buf(hi, SLOT_OFFSET + (k - 1), ts2);
                                        }
                                    } else {
                                        // (mTr == 3)
                                        sign = 1;
                                        // opposite phase power parameters as ts2 is center
                                        t_center = m_tr;
                                        t_vec = stretch - m_tr;

                                        gamma_center_real_m[0] = in_buf(hr, SLOT_OFFSET, ts2);
                                        gamma_center_imag_m[0] = in_buf(hi, SLOT_OFFSET, ts2);

                                        for k in 0..2i32 {
                                            gamma_vec_real_m[k as usize] =
                                                in_buf(hr, SLOT_OFFSET + 2 * (k - 1), ts1);
                                            gamma_vec_imag_m[k as usize] =
                                                in_buf(hi, SLOT_OFFSET + 2 * (k - 1), ts1);
                                        }
                                    }

                                    gamma_center_e[0] = sc_e;
                                    gamma_vec_e[0] = sc_e;
                                    gamma_vec_e[1] = sc_e;
                                }
                            }
                            3 => {
                                wingain = fl2fxconst_dbl((5.6568f32 / 12.0f32) as f64); // sum of taps divided by two

                                if h.b_x_products[1] != 0 {
                                    let addrshift = -2;
                                    let (tc, tv) = if m_tr == 1 { (ts1, ts2) } else { (ts2, ts1) };
                                    if m_tr != 1 {
                                        // (mTr == 2)
                                        sign = 1;
                                        // opposite phase power parameters as ts2 is center
                                        t_center = m_tr;
                                        t_vec = stretch - m_tr;
                                    }
                                    gamma_center_real_m[0] = in_buf(hr, SLOT_OFFSET, tc);
                                    gamma_center_imag_m[0] = in_buf(hi, SLOT_OFFSET, tc);
                                    gamma_vec_real_m[1] = in_buf(hr, SLOT_OFFSET, tv);
                                    gamma_vec_imag_m[1] = in_buf(hi, SLOT_OFFSET, tv);

                                    let tmp_real_f = in_buf(hr, addrshift + SLOT_OFFSET, tv);
                                    let tmp_imag_f = in_buf(hi, addrshift + SLOT_OFFSET, tv);

                                    gamma_vec_real_m[0] = f_mult(factor(tv % 4), tmp_real_f)
                                        .wrapping_sub(f_mult(factor((tv + 3) % 4), tmp_imag_f))
                                        >> 1;
                                    gamma_vec_imag_m[0] = f_mult(factor((tv + 3) % 4), tmp_real_f)
                                        .wrapping_add(f_mult(factor(tv % 4), tmp_imag_f))
                                        >> 1;

                                    let tmp_real_f = in_buf(hr, addrshift + 1 + SLOT_OFFSET, tv);
                                    let tmp_imag_f = in_buf(hi, addrshift + 1 + SLOT_OFFSET, tv);

                                    gamma_vec_real_m[0] = gamma_vec_real_m[0].wrapping_add(
                                        f_mult(factor(tv % 4), tmp_real_f)
                                            .wrapping_sub(f_mult(factor((tv + 1) % 4), tmp_imag_f))
                                            >> 1,
                                    );
                                    gamma_vec_imag_m[0] = gamma_vec_imag_m[0].wrapping_add(
                                        f_mult(factor((tv + 1) % 4), tmp_real_f)
                                            .wrapping_add(f_mult(factor(tv % 4), tmp_imag_f))
                                            >> 1,
                                    );

                                    gamma_center_e[0] = sc_e;
                                    gamma_vec_e[1] = sc_e;
                                    gamma_vec_e[0] = sc_e + 1;
                                }
                            }
                            _ => {
                                fdk_assert!(false);
                            }
                        } // stretch cases

                        // parameter controlled phase modification parts
                        // maximum *_e == 20
                        {
                            let (r0, i0) = (gamma_center_real_m[0], gamma_center_imag_m[0]);
                            calculate_center_fixp(
                                r0,
                                i0,
                                &mut gamma_center_real_m[0],
                                &mut gamma_center_imag_m[0],
                                &mut gamma_center_e[0],
                                stretch,
                                t_center - 1,
                            );
                            let (r0, i0) = (gamma_vec_real_m[0], gamma_vec_imag_m[0]);
                            calculate_center_fixp(
                                r0,
                                i0,
                                &mut gamma_vec_real_m[0],
                                &mut gamma_vec_imag_m[0],
                                &mut gamma_vec_e[0],
                                stretch,
                                t_vec - 1,
                            );
                            let (r1, i1) = (gamma_vec_real_m[1], gamma_vec_imag_m[1]);
                            calculate_center_fixp(
                                r1,
                                i1,
                                &mut gamma_vec_real_m[1],
                                &mut gamma_vec_imag_m[1],
                                &mut gamma_vec_e[1],
                                stretch,
                                t_vec - 1,
                            );
                        }

                        // Final multiplication of prepared parts
                        for k in 0..2 {
                            gamma_out_real_m[k] =
                                f_mult_div2(gamma_vec_real_m[k], gamma_center_real_m[0])
                                    .wrapping_sub(f_mult_div2(
                                        gamma_vec_imag_m[k],
                                        gamma_center_imag_m[0],
                                    ));
                            gamma_out_imag_m[k] =
                                f_mult_div2(gamma_vec_real_m[k], gamma_center_imag_m[0])
                                    .wrapping_add(f_mult_div2(
                                        gamma_vec_imag_m[k],
                                        gamma_center_real_m[0],
                                    ));
                            gamma_out_e[k] = gamma_center_e[0] + gamma_vec_e[k] + 1;
                        }

                        {
                            let (a, b) = gamma_out_real_m.split_at_mut(1);
                            let (c, d) = gamma_out_imag_m.split_at_mut(1);
                            let (e0, e1) = gamma_out_e.split_at_mut(1);
                            scale_up(&mut a[0], &mut c[0], &mut e0[0]);
                            scale_up(&mut b[0], &mut d[0], &mut e1[0]);
                        }
                        fdk_assert!(gamma_out_e[0] >= 0);
                        fdk_assert!(gamma_out_e[0] < 32);

                        let tmp_real_m = gamma_out_real_m[0];
                        let tmp_imag_m = gamma_out_imag_m[0];

                        let modstretch4 = (stretch == 4 && m_tr == 2) as i32;

                        let tw = twid_m_new[(stretch - 2 - modstretch4) as usize];
                        let cos_twid = tw[0];
                        let sin_twid = sign.wrapping_mul(tw[1]);

                        // sum should be <= 1 because of sin/cos multiplication
                        gamma_out_real_m[0] =
                            f_mult(tmp_real_m, cos_twid).wrapping_sub(f_mult(tmp_imag_m, sin_twid));
                        // sum should be <= 1 because of sin/cos multiplication
                        gamma_out_imag_m[0] =
                            f_mult(tmp_imag_m, cos_twid).wrapping_add(f_mult(tmp_real_m, sin_twid));

                        // wingain
                        for k in 0..2 {
                            gamma_out_real_m[k] = f_mult(gamma_out_real_m[k], wingain) << 1;
                            gamma_out_imag_m[k] = f_mult(gamma_out_imag_m[k], wingain) << 1;
                        }

                        gamma_out_real_m[1] >>= 1;
                        gamma_out_imag_m[1] >>= 1;
                        gamma_out_e[0] += 2;
                        gamma_out_e[1] += 2;

                        // OLA including window scaling by wingain/3
                        for k in 0..2 {
                            // need k=1 to correspond to grainModImag[slotOffset] -> out to
                            // j*2+(slotOffset-offset)
                            let idx = (k + SLOT_OFFSET as usize - 1) * ROW + band as usize;
                            // scale_factor_hbe - gammaOut_e[k] can be negative or 32 or more:
                            // register shift (amount modulo 32)
                            let s = (scale_factor_hbe - gamma_out_e[k]) as u32;
                            if let Some(v) = h.qmf_hbe_buf_real_f.get_mut(idx) {
                                *v = v.wrapping_add(gamma_out_real_m[k].wrapping_shr(s));
                            }
                            if let Some(v) = h.qmf_hbe_buf_imag_f.get_mut(idx) {
                                *v = v.wrapping_add(gamma_out_imag_m[k].wrapping_shr(s));
                            }
                        }
                    } // mVal > qThrQMF * qThrQMF * sqmag0 && ts1 > 0 && ts2 < 64
                } // p >= pmin
            } // for band
        } // for stretch

        for i in 0..QMF_WIN_LEN - 1 {
            h.qmf_in_buf_real_f
                .copy_within((i + 1) * ROW..(i + 1) * ROW + ana_no_channels, i * ROW);
            h.qmf_in_buf_imag_f
                .copy_within((i + 1) * ROW..(i + 1) * ROW + ana_no_channels, i * ROW);
        }

        let xo = (h.x_over_qmf[0].clamp(0, ROW as i32)) as usize;
        if keep_states_synced_mode != KEEP_STATES_SYNCED_NOOUT && 2 * j >= offset {
            // copy first two slots of internal buffer to output
            for i in 0..2usize {
                let dst = if keep_states_synced_mode == KEEP_STATES_SYNCED_OUTDIFF {
                    2 * j - offset + i as i32
                } else {
                    2 * j + i as i32 + ov_len
                };
                let src = i * ROW;
                if let Some(d) = slots_re.get_mut(dst as usize).and_then(|s| s.get_mut(xo..ROW)) {
                    d.copy_from_slice(&h.qmf_hbe_buf_real_f[src + xo..src + ROW]);
                }
                if let Some(d) = slots_im.get_mut(dst as usize).and_then(|s| s.get_mut(xo..ROW)) {
                    d.copy_from_slice(&h.qmf_hbe_buf_imag_f[src + xo..src + ROW]);
                }
            }
        }

        // move slots up
        for i in 0..HBE_MAX_OUT_SLOTS - 2 {
            h.qmf_hbe_buf_real_f.copy_within((i + 2) * ROW + xo..(i + 3) * ROW, i * ROW + xo);
            h.qmf_hbe_buf_imag_f.copy_within((i + 2) * ROW + xo..(i + 3) * ROW, i * ROW + xo);
        }

        // finally set last two slot to zero
        for i in 0..2 {
            let r = HBE_MAX_OUT_SLOTS - 1 - i;
            h.qmf_hbe_buf_real_f[r * ROW + xo..(r + 1) * ROW].fill(0);
            h.qmf_hbe_buf_imag_f[r * ROW + xo..(r + 1) * ROW].fill(0);
        }
    } // qmfVocoderColsIn

    if keep_states_synced_mode != KEEP_STATES_SYNCED_NOOUT {
        let (first, last, shift) = if keep_states_synced_mode == KEEP_STATES_SYNCED_OUTDIFF {
            (0, ov_len + LPC_ORDER as i32, 0)
        } else {
            (offset, h.no_cols, ov_len)
        };
        for i in first.max(0)..last {
            let s = (i + shift) as usize;
            for band in h.start_band.max(0)..h.stop_band.min(64) {
                let b = band as usize;
                let (Some(tmp_r), Some(tmp_i)) = (
                    slots_re.get(s).and_then(|sl| sl.get(b)).copied(),
                    slots_im.get(s).and_then(|sl| sl.get(b)).copied(),
                ) else {
                    continue;
                };

                // sum should be <= 1 because of sin/cos multiplication
                let re = f_mult(tmp_r, COS_F[b])
                    .wrapping_sub(f_mult(tmp_i, COS_F[64 - b - 1].wrapping_neg()));
                // sum should by <= 1 because of sin/cos multiplication
                let im = f_mult(tmp_r, COS_F[64 - b - 1].wrapping_neg())
                    .wrapping_add(f_mult(tmp_i, COS_F[b]));
                slots_re[s][b] = re;
                slots_im[s][b] = im;
            }
        }
    }

    *scale_hb = 15 - scale_factor_hbe; // EXP2SCALE(scale_factor_hbe)
}

/// `GetxOverBandQmfTransposer(hQmfTransposer)`: the crossover bands of the transposer (`None`
/// for `NULL`).
pub fn getx_over_band_qmf_transposer(
    h_qmf_transposer: Option<&HbeTransposer>,
) -> Option<&[i32; MAX_NUM_PATCHES_HBE]> {
    h_qmf_transposer.map(|h| &h.x_over_qmf)
}

/// `Get41SbrQmfTransposer(hQmfTransposer)`: whether the transposer works with 4:1 SBR.
pub fn get41_sbr_qmf_transposer(h_qmf_transposer: Option<&HbeTransposer>) -> i32 {
    match h_qmf_transposer {
        Some(h) => h.b_sbr41,
        None => 0,
    }
}
