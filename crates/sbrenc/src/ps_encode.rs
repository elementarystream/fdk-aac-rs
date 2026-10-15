// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/ps_encode.h, libSBRenc/src/ps_encode.cpp.

//! Port of `ps_encode.h` and `ps_encode.cpp`: the parametric stereo parameter extraction,
//! quantization and coding decisions.
//!
//! The hybrid data (`FIXP_DBL *hybridData[HYBRID_FRAMESIZE][MAX_PS_CHANNELS][2]` in C, an
//! array of pointers into a static buffer and the dynamic RAM of the SBR encoder) is passed as
//! a [`HybridView`].

use fdk_aac_rs_fdk::common_fix::*;

use crate::ps_bitenc::*;
use crate::ps_const::*;

mod tables;
pub use tables::*;

/// `fixMax(a, b)` where the reference builds take `a` and `b` as not negative because they are
/// `fixp_abs()` results (or maxima of them): `fAbs()` negates with signed overflow, so clang
/// compiles the maximum as an unsigned one (arm64 `csel hi`/`umax`, x86 `cmova`/`ja`). It
/// differs from the signed maximum only when a value is `MINVAL_DBL` (`fAbs(MINVAL_DBL)` is
/// `MINVAL_DBL`), which it then returns.
#[inline(always)]
pub(crate) fn umax_abs(a: FixpDbl, b: FixpDbl) -> FixpDbl {
    (a as u32).max(b as u32) as FixpDbl
}

/// `CountLeadingBits(x)` (`fixnorm_D`) of an `x` the reference builds take as not negative (a
/// `fixp_abs()` result): they drop the sign fold `x ^ (x >> 31)` and compute `clz(x) - 1` (0 for
/// 0), which is -1 for `MINVAL_DBL` instead of 0.
#[inline(always)]
pub(crate) fn clb_abs(x: FixpDbl) -> i32 {
    if x == 0 {
        0
    } else {
        (x as u32).leading_zeros() as i32 - 1
    }
}

/// `IID_SCALE_FT`: maxVal in Quant tab is +/- 50
pub const IID_SCALE_FT: f32 = 64.0;
/// `IID_SCALE`: maxVal in Quant tab is +/- 50
pub const IID_SCALE: i32 = 6;
/// `IID_MAXVAL`
pub const IID_MAXVAL: i32 = 1 << IID_SCALE;

/// `PS_QUANT_SCALE_FT`: error smaller (64-25)/64 * 20 bands * 4 env -> QuantScale 64
pub const PS_QUANT_SCALE_FT: f32 = 64.0;
/// `PS_QUANT_SCALE`: error smaller (64-25)/64 * 20 bands * 4 env -> QuantScale 6 bit
pub const PS_QUANT_SCALE: i32 = 6;

/// `QMF_GROUPS_LO_RES`
pub const QMF_GROUPS_LO_RES: usize = 12;
/// `SUBQMF_GROUPS_LO_RES`
pub const SUBQMF_GROUPS_LO_RES: usize = 10;
/// `QMF_GROUPS_HI_RES`
pub const QMF_GROUPS_HI_RES: usize = 18;
/// `SUBQMF_GROUPS_HI_RES`
pub const SUBQMF_GROUPS_HI_RES: usize = 30;

/// `LOG10_2_10`: 10.0f*log10(2.f)
const LOG10_2_10: f32 = 3.01029995664;

/// `THRESH_SCALE`
const THRESH_SCALE: i32 = 7;

/// `PS_DATA` (`HANDLE_PS_DATA` is `&mut PsData`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsData {
    /// `iidEnable`
    pub iid_enable: i32,
    /// `iidEnableLast`
    pub iid_enable_last: i32,
    /// `iidQuantMode`
    pub iid_quant_mode: i32,
    /// `iidQuantModeLast`
    pub iid_quant_mode_last: i32,
    /// `iidDiffMode`
    pub iid_diff_mode: [i32; PS_MAX_ENVELOPES],
    /// `iidIdx`
    pub iid_idx: [[i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    /// `iidIdxLast`
    pub iid_idx_last: [i32; PS_MAX_BANDS],

    /// `iccEnable`
    pub icc_enable: i32,
    /// `iccEnableLast`
    pub icc_enable_last: i32,
    /// `iccQuantMode`
    pub icc_quant_mode: i32,
    /// `iccQuantModeLast`
    pub icc_quant_mode_last: i32,
    /// `iccDiffMode`
    pub icc_diff_mode: [i32; PS_MAX_ENVELOPES],
    /// `iccIdx`
    pub icc_idx: [[i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    /// `iccIdxLast`
    pub icc_idx_last: [i32; PS_MAX_BANDS],

    /// `nEnvelopesLast`
    pub n_envelopes_last: i32,

    /// `headerCnt`
    pub header_cnt: i32,
    /// `iidTimeCnt`
    pub iid_time_cnt: i32,
    /// `iccTimeCnt`
    pub icc_time_cnt: i32,
    /// `noEnvCnt`
    pub no_env_cnt: i32,
}

/// `PS_ENCODE` (`HANDLE_PS_ENCODE` is `&mut PsEncode`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsEncode {
    /// `psData`
    pub ps_data: PsData,

    /// `psEncMode`
    pub ps_enc_mode: PsBands,
    /// `nQmfIidGroups`
    pub n_qmf_iid_groups: i32,
    /// `nSubQmfIidGroups`
    pub n_sub_qmf_iid_groups: i32,
    /// `iidGroupBorders`
    pub iid_group_borders: [i32; QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES + 1],
    /// `subband2parameterIndex`
    pub subband2parameter_index: [i32; QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES],
    /// `iidGroupWidthLd`
    pub iid_group_width_ld: [u8; QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES],
    /// `iidQuantErrorThreshold`
    pub iid_quant_error_threshold: FixpDbl,

    /// `psBandNrgScale`
    pub ps_band_nrg_scale: [u8; PS_MAX_BANDS],
}

impl Default for PsEncode {
    fn default() -> Self {
        Self {
            ps_data: PsData::default(),
            ps_enc_mode: 0,
            n_qmf_iid_groups: 0,
            n_sub_qmf_iid_groups: 0,
            iid_group_borders: [0; QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES + 1],
            subband2parameter_index: [0; QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES],
            iid_group_width_ld: [0; QMF_GROUPS_HI_RES + SUBQMF_GROUPS_HI_RES],
            iid_quant_error_threshold: 0,
            ps_band_nrg_scale: [0; PS_MAX_BANDS],
        }
    }
}

/// A view of the hybrid data rows of the PS encoder: `hybridData[col][ch][re/im]` of C, a
/// pointer to the hybrid band values of time slot `col` (relative to `base`) of channel `ch`.
/// Rows `0..HYBRID_READ_OFFSET` are the static buffer of the PS encoder, the following rows
/// are in the dynamic RAM of the SBR encoder (see [`crate::ps_main`]).
#[derive(Clone, Copy)]
pub struct HybridView<'a> {
    /// `__staticHybridData`
    pub stat: &'a [[[[FixpDbl; MAX_HYBRID_BANDS]; 2]; MAX_PS_CHANNELS]; HYBRID_READ_OFFSET],
    /// The dynamic RAM of the SBR encoder.
    pub dyn_ram: &'a [FixpDbl],
    /// The row of `pHybridData` that is `hybridData[0]`.
    pub base: usize,
}

impl HybridView<'_> {
    /// `hybridData[col][ch][ri]`
    #[inline]
    pub fn row(&self, col: usize, ch: usize, ri: usize) -> &[FixpDbl] {
        let r = self.base + col;
        if r < HYBRID_READ_OFFSET {
            &self.stat[r][ch][ri]
        } else {
            let off = crate::ps_main::hybrid_dyn_offset(r - HYBRID_READ_OFFSET, ch, ri);
            &self.dyn_ram[off..off + MAX_HYBRID_BANDS]
        }
    }
}

/// `FDKsbrEnc_addFIXP_DBL(X, Y, Z, n)`: `Z[i] = (X[i] >> 1) + (Y[i] >> 1)` (`Z` may be `X`).
fn add_fixp_dbl(
    rows: &mut [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    x: usize,
    y: usize,
    z: usize,
    n: i32,
) {
    for i in 0..n.max(0) as usize {
        rows[z][i] = (rows[x][i] >> 1).wrapping_add(rows[y][i] >> 1);
    }
}

/// `InitPSData(hPsData)`
fn init_ps_data(h_ps_data: &mut PsData) {
    *h_ps_data = PsData::default();

    for i in 0..PS_MAX_BANDS {
        h_ps_data.iid_idx_last[i] = 0;
        h_ps_data.icc_idx_last[i] = 0;
    }

    h_ps_data.iid_enable = 0;
    h_ps_data.iid_enable_last = 0;
    h_ps_data.icc_enable = 0;
    h_ps_data.icc_enable_last = 0;
    h_ps_data.iid_quant_mode = PS_IID_RES_COARSE;
    h_ps_data.iid_quant_mode_last = PS_IID_RES_COARSE;
    h_ps_data.icc_quant_mode = PS_ICC_ROT_A;
    h_ps_data.icc_quant_mode_last = PS_ICC_ROT_A;

    for env in 0..PS_MAX_ENVELOPES {
        h_ps_data.icc_diff_mode[env] = PS_DELTA_FREQ;
        h_ps_data.icc_diff_mode[env] = PS_DELTA_FREQ;
        for i in 0..PS_MAX_BANDS {
            h_ps_data.iid_idx[env][i] = 0;
            h_ps_data.icc_idx[env][i] = 0;
        }
    }

    h_ps_data.n_envelopes_last = 0;

    h_ps_data.header_cnt = MAX_PS_NOHEADER_CNT;
    h_ps_data.iid_time_cnt = MAX_TIME_DIFF_FRAMES;
    h_ps_data.icc_time_cnt = MAX_TIME_DIFF_FRAMES;
    h_ps_data.no_env_cnt = MAX_NOENV_CNT;
}

/// `quantizeCoef(input, nBands, quantTable, idxOffset, nQuantSteps, quantOut)`: returns the
/// quantization error.
fn quantize_coef(
    input: &[FixpDbl],
    n_bands: i32,
    quant_table: &[FixpDbl],
    idx_offset: i32,
    n_quant_steps: i32,
    quant_out: &mut [i32],
) -> FixpDbl {
    let mut quant_err: FixpDbl = 0;

    for band in 0..n_bands.max(0) as usize {
        let mut idx = 0usize;
        while (idx as i32) < n_quant_steps - 1 {
            if fixp_abs((input[band] >> 1).wrapping_sub(quant_table[idx + 1] >> 1))
                > fixp_abs((input[band] >> 1).wrapping_sub(quant_table[idx] >> 1))
            {
                break;
            }
            idx += 1;
        }
        // don't scale before subtraction; diff smaller (64-25)/64
        quant_err = quant_err
            .wrapping_add(fixp_abs(input[band].wrapping_sub(quant_table[idx])) >> PS_QUANT_SCALE);
        quant_out[band] = idx as i32 - idx_offset;
    }

    quant_err
}

/// `getICCMode(nBands, rotType)`
fn get_icc_mode(n_bands: i32, rot_type: i32) -> i32 {
    let mut mode = match n_bands {
        PS_BANDS_COARSE => PS_RES_COARSE,
        PS_BANDS_MID => PS_RES_MID,
        _ => 0,
    };
    if rot_type == PS_ICC_ROT_B {
        mode += 3;
    }
    mode
}

/// `getIIDMode(nBands, iidRes)`
fn get_iid_mode(n_bands: i32, iid_res: i32) -> i32 {
    let mut mode = match n_bands {
        PS_BANDS_COARSE => PS_RES_COARSE,
        PS_BANDS_MID => PS_RES_MID,
        _ => 0,
    };
    if iid_res == PS_IID_RES_FINE {
        mode += 3;
    }
    mode
}

/// `envelopeReducible(iid, icc, psBands, nEnvelopes)`
fn envelope_reducible(
    iid: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    icc: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    ps_bands: i32,
    n_envelopes: i32,
) -> i32 {
    let mut reducible = 1; // true

    // square values to prevent sqrt, multiply bands to prevent division; bands shifted
    // DFRACT_BITS instead (DFRACT_BITS-1) because fMultDiv2 used
    let iid_err_threshold = f_mult_div2(
        fl2fxconst_dbl((6.5f32 * 6.5f32 / (IID_SCALE_FT * IID_SCALE_FT)) as f64),
        ps_bands.wrapping_shl((DFRACT_BITS - THRESH_SCALE) as u32),
    );
    let icc_err_threshold = f_mult_div2(
        fl2fxconst_dbl((0.75f32 * 0.75f32) as f64),
        ps_bands.wrapping_shl((DFRACT_BITS - THRESH_SCALE) as u32),
    );

    if n_envelopes <= 1 {
        reducible = 0;
    } else {
        // mean error criterion
        let mut e = 0usize;
        while (e as i32) < n_envelopes / 2 && reducible != 0 {
            let mut iid_mean_error: FixpDbl = 0;
            let mut icc_mean_error: FixpDbl = 0;
            for b in 0..ps_bands.max(0) as usize {
                let d_iid = (iid[2 * e][b] >> 1).wrapping_sub(iid[2 * e + 1][b] >> 1); // scale 1 bit; squared -> 2 bit
                let d_icc = (icc[2 * e][b] >> 1).wrapping_sub(icc[2 * e + 1][b] >> 1);
                iid_mean_error = iid_mean_error.wrapping_add(f_pow2_div2(d_iid) >> (5 - 1)); // + (bands=20) scale = 5
                icc_mean_error = icc_mean_error.wrapping_add(f_pow2_div2(d_icc) >> (5 - 1));
            } // --> scaling = 7 bit = THRESH_SCALE !!

            // instead sqrt values are squared! instead of division, multiply threshold with
            // psBands scaling necessary!!

            // quit as soon as threshold is reached
            if iid_mean_error > iid_err_threshold || icc_mean_error > icc_err_threshold {
                reducible = 0;
            }
            e += 1;
        }
    }

    reducible
}

/// `processIidData(psData, iid, psBands, nEnvelopes, quantErrorThreshold)`
fn process_iid_data(
    ps_data: &mut PsData,
    iid: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    ps_bands: i32,
    n_envelopes: i32,
    quant_error_threshold: FixpDbl,
) {
    let mut iid_idx_fine = [[0i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES];
    let mut iid_idx_coarse = [[0i32; PS_MAX_BANDS]; PS_MAX_ENVELOPES];

    let mut err_iid: FixpDbl = 0;
    let mut err_iid_fine: FixpDbl = 0;
    let mut bits_iid_freq;
    let mut bits_iid_time;
    let mut bits_fine_tot;
    let mut bits_coarse_tot;
    let mut error = 0;
    let mut diff_mode = [0i32; PS_MAX_ENVELOPES];
    let mut diff_mode_fine = [0i32; PS_MAX_ENVELOPES];
    let mut loudn_diff = 0;
    let mut iid_transmit = 0;
    let ne = n_envelopes.max(0) as usize;
    let nb = ps_bands.max(0) as usize;

    // Quantize IID coefficients
    for env in 0..ne {
        err_iid = err_iid.wrapping_add(quantize_coef(
            &iid[env],
            ps_bands,
            &IID_QUANT_FX,
            7,
            15,
            &mut iid_idx_coarse[env],
        ));
        err_iid_fine = err_iid_fine.wrapping_add(quantize_coef(
            &iid[env],
            ps_bands,
            &IID_QUANT_FINE_FX,
            15,
            31,
            &mut iid_idx_fine[env],
        ));
    }

    // normalize error to number of envelopes, ps bands
    // errIID /= psBands*nEnvelopes;
    // errIIDFine /= psBands*nEnvelopes;

    // Check if IID coefficients should be used in this frame
    ps_data.iid_enable = 0;
    for env in 0..ne {
        for band in 0..nb {
            loudn_diff += iid_idx_coarse[env][band].abs();
            iid_transmit += 1;
        }
    }

    if loudn_diff > f_mult_i(fl2fxconst_dbl(0.7f32 as f64), iid_transmit) {
        // 0.7f empiric value
        ps_data.iid_enable = 1;
    }

    // if iid not active -> RESET data
    if ps_data.iid_enable == 0 {
        ps_data.iid_time_cnt = MAX_TIME_DIFF_FRAMES;
        for env in 0..ne {
            ps_data.iid_diff_mode[env] = PS_DELTA_FREQ;
            ps_data.iid_idx[env][..nb].fill(0);
        }
        return;
    }

    // count COARSE quantization bits for first envelope
    bits_iid_freq = fdk_sbr_enc_encode_iid(
        None,
        &iid_idx_coarse[0],
        &[],
        ps_bands,
        PS_IID_RES_COARSE,
        PS_DELTA_FREQ,
        &mut error,
    );
    if ps_data.iid_time_cnt >= MAX_TIME_DIFF_FRAMES
        || ps_data.iid_quant_mode_last == PS_IID_RES_FINE
    {
        bits_iid_time = DO_NOT_USE_THIS_MODE;
    } else {
        bits_iid_time = fdk_sbr_enc_encode_iid(
            None,
            &iid_idx_coarse[0],
            &ps_data.iid_idx_last,
            ps_bands,
            PS_IID_RES_COARSE,
            PS_DELTA_TIME,
            &mut error,
        );
    }

    // decision DELTA_FREQ vs DELTA_TIME
    if bits_iid_time > bits_iid_freq {
        diff_mode[0] = PS_DELTA_FREQ;
        bits_coarse_tot = bits_iid_freq;
    } else {
        diff_mode[0] = PS_DELTA_TIME;
        bits_coarse_tot = bits_iid_time;
    }

    // count COARSE quantization bits for following envelopes
    for env in 1..ne {
        bits_iid_freq = fdk_sbr_enc_encode_iid(
            None,
            &iid_idx_coarse[env],
            &[],
            ps_bands,
            PS_IID_RES_COARSE,
            PS_DELTA_FREQ,
            &mut error,
        );
        bits_iid_time = fdk_sbr_enc_encode_iid(
            None,
            &iid_idx_coarse[env],
            &iid_idx_coarse[env - 1],
            ps_bands,
            PS_IID_RES_COARSE,
            PS_DELTA_TIME,
            &mut error,
        );

        // decision DELTA_FREQ vs DELTA_TIME
        if bits_iid_time > bits_iid_freq {
            diff_mode[env] = PS_DELTA_FREQ;
            bits_coarse_tot += bits_iid_freq;
        } else {
            diff_mode[env] = PS_DELTA_TIME;
            bits_coarse_tot += bits_iid_time;
        }
    }

    // count FINE quantization bits for first envelope
    bits_iid_freq = fdk_sbr_enc_encode_iid(
        None,
        &iid_idx_fine[0],
        &[],
        ps_bands,
        PS_IID_RES_FINE,
        PS_DELTA_FREQ,
        &mut error,
    );
    if ps_data.iid_time_cnt >= MAX_TIME_DIFF_FRAMES
        || ps_data.iid_quant_mode_last == PS_IID_RES_COARSE
    {
        bits_iid_time = DO_NOT_USE_THIS_MODE;
    } else {
        bits_iid_time = fdk_sbr_enc_encode_iid(
            None,
            &iid_idx_fine[0],
            &ps_data.iid_idx_last,
            ps_bands,
            PS_IID_RES_FINE,
            PS_DELTA_TIME,
            &mut error,
        );
    }

    // decision DELTA_FREQ vs DELTA_TIME
    if bits_iid_time > bits_iid_freq {
        diff_mode_fine[0] = PS_DELTA_FREQ;
        bits_fine_tot = bits_iid_freq;
    } else {
        diff_mode_fine[0] = PS_DELTA_TIME;
        bits_fine_tot = bits_iid_time;
    }

    // count FINE quantization bits for following envelopes
    for env in 1..ne {
        bits_iid_freq = fdk_sbr_enc_encode_iid(
            None,
            &iid_idx_fine[env],
            &[],
            ps_bands,
            PS_IID_RES_FINE,
            PS_DELTA_FREQ,
            &mut error,
        );
        bits_iid_time = fdk_sbr_enc_encode_iid(
            None,
            &iid_idx_fine[env],
            &iid_idx_fine[env - 1],
            ps_bands,
            PS_IID_RES_FINE,
            PS_DELTA_TIME,
            &mut error,
        );

        // decision DELTA_FREQ vs DELTA_TIME
        if bits_iid_time > bits_iid_freq {
            diff_mode_fine[env] = PS_DELTA_FREQ;
            bits_fine_tot += bits_iid_freq;
        } else {
            diff_mode_fine[env] = PS_DELTA_TIME;
            bits_fine_tot += bits_iid_time;
        }
    }

    if bits_fine_tot == bits_coarse_tot {
        // if same number of bits is needed, use the quantization with lower error
        if err_iid_fine < err_iid {
            bits_coarse_tot = DO_NOT_USE_THIS_MODE;
        } else {
            bits_fine_tot = DO_NOT_USE_THIS_MODE;
        }
    } else {
        // const FIXP_DBL minThreshold =
        // FL2FXCONST_DBL(0.2f/(IID_SCALE_FT*PS_QUANT_SCALE_FT)*(psBands*nEnvelopes));
        let min_threshold: FixpDbl = 0x0001_9999i32.wrapping_mul(ps_bands * n_envelopes);

        // decision RES_FINE vs RES_COARSE
        // test if errIIDFine*quantErrorThreshold < errIID
        // shiftVal 2 comes from scaling of quantErrorThreshold
        if fix_max(
            ((err_iid_fine >> 1).wrapping_add(min_threshold >> 1)) >> 1,
            f_mult(quant_error_threshold, err_iid_fine),
        ) < (err_iid >> 2)
        {
            bits_coarse_tot = DO_NOT_USE_THIS_MODE;
        } else if fix_max(
            ((err_iid >> 1).wrapping_add(min_threshold >> 1)) >> 1,
            f_mult(quant_error_threshold, err_iid),
        ) < (err_iid_fine >> 2)
        {
            bits_fine_tot = DO_NOT_USE_THIS_MODE;
        }
    }

    // decision RES_FINE vs RES_COARSE
    if bits_fine_tot < bits_coarse_tot {
        ps_data.iid_quant_mode = PS_IID_RES_FINE;
        for env in 0..ne {
            ps_data.iid_diff_mode[env] = diff_mode_fine[env];
            ps_data.iid_idx[env][..nb].copy_from_slice(&iid_idx_fine[env][..nb]);
        }
    } else {
        ps_data.iid_quant_mode = PS_IID_RES_COARSE;
        for env in 0..ne {
            ps_data.iid_diff_mode[env] = diff_mode[env];
            ps_data.iid_idx[env][..nb].copy_from_slice(&iid_idx_coarse[env][..nb]);
        }
    }

    // Count DELTA_TIME encoding streaks
    for env in 0..ne {
        if ps_data.iid_diff_mode[env] == PS_DELTA_TIME {
            ps_data.iid_time_cnt += 1;
        } else {
            ps_data.iid_time_cnt = 0;
        }
    }
}

/// `similarIid(psData, psBands, nEnvelopes)`
fn similar_iid(ps_data: &PsData, ps_bands: i32, n_envelopes: i32) -> i32 {
    let diff_thr = if ps_data.iid_quant_mode == PS_IID_RES_COARSE { 2 } else { 3 };
    let sum_diff_thr = diff_thr * ps_bands / 4;
    let mut similar = 0;

    if n_envelopes == ps_data.n_envelopes_last && n_envelopes == 1 {
        similar = 1;
        for env in 0..n_envelopes as usize {
            let mut sum_diff = 0;
            let mut b = 0usize;
            loop {
                let diff = (ps_data.iid_idx[env][b] - ps_data.iid_idx_last[b]).abs();
                sum_diff += diff;
                if diff > diff_thr // more than x quantization steps in any band
                    || sum_diff > sum_diff_thr
                {
                    // more than x quantisations steps overall difference
                    similar = 0;
                }
                b += 1;
                if !((b as i32) < ps_bands && similar > 0) {
                    break;
                }
            }
        }
    } // nEnvelopes==1

    similar
}

/// `similarIcc(psData, psBands, nEnvelopes)`
fn similar_icc(ps_data: &PsData, ps_bands: i32, n_envelopes: i32) -> i32 {
    let diff_thr = 2;
    let sum_diff_thr = diff_thr * ps_bands / 4;
    let mut similar = 0;

    if n_envelopes == ps_data.n_envelopes_last && n_envelopes == 1 {
        similar = 1;
        for env in 0..n_envelopes as usize {
            let mut sum_diff = 0;
            let mut b = 0usize;
            loop {
                let diff = (ps_data.icc_idx[env][b] - ps_data.icc_idx_last[b]).abs();
                sum_diff += diff;
                if diff > diff_thr // more than x quantisation step in any band
                    || sum_diff > sum_diff_thr
                {
                    // more than x quantisations steps overall difference
                    similar = 0;
                }
                b += 1;
                if !((b as i32) < ps_bands && similar > 0) {
                    break;
                }
            }
        }
    } // nEnvelopes==1

    similar
}

/// `processIccData(psData, icc, psBands, nEnvelopes)`
fn process_icc_data(
    ps_data: &mut PsData,
    icc: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    ps_bands: i32,
    n_envelopes: i32,
) {
    let mut err_icc: FixpDbl = 0;
    let mut error = 0;
    let mut in_coherence = 0;
    let mut icc_transmit = 0;
    let ne = n_envelopes.max(0) as usize;
    let nb = ps_bands.max(0) as usize;

    // Quantize ICC coefficients
    for env in 0..ne {
        err_icc = err_icc.wrapping_add(quantize_coef(
            &icc[env],
            ps_bands,
            &ICC_QUANT,
            0,
            8,
            &mut ps_data.icc_idx[env],
        ));
    }
    let _ = err_icc;

    // Check if ICC coefficients should be used
    ps_data.icc_enable = 0;
    for env in 0..ne {
        for band in 0..nb {
            in_coherence += ps_data.icc_idx[env][band];
            icc_transmit += 1;
        }
    }
    if in_coherence > f_mult_i(fl2fxconst_dbl(0.5f32 as f64), icc_transmit) {
        // 0.5f empiric value
        ps_data.icc_enable = 1;
    }

    if ps_data.icc_enable == 0 {
        ps_data.icc_time_cnt = MAX_TIME_DIFF_FRAMES;
        for env in 0..ne {
            ps_data.icc_diff_mode[env] = PS_DELTA_FREQ;
            ps_data.icc_idx[env][..nb].fill(0);
        }
        return;
    }

    let mut icc_idx_last: [i32; PS_MAX_BANDS] = ps_data.icc_idx_last;
    for env in 0..ne {
        let bits_icc_freq = fdk_sbr_enc_encode_icc(
            None,
            &ps_data.icc_idx[env],
            &[],
            ps_bands,
            PS_DELTA_FREQ,
            &mut error,
        );

        let bits_icc_time = if ps_data.icc_time_cnt < MAX_TIME_DIFF_FRAMES {
            fdk_sbr_enc_encode_icc(
                None,
                &ps_data.icc_idx[env],
                &icc_idx_last,
                ps_bands,
                PS_DELTA_TIME,
                &mut error,
            )
        } else {
            DO_NOT_USE_THIS_MODE
        };

        if bits_icc_freq > bits_icc_time {
            ps_data.icc_diff_mode[env] = PS_DELTA_TIME;
            ps_data.icc_time_cnt += 1;
        } else {
            ps_data.icc_diff_mode[env] = PS_DELTA_FREQ;
            ps_data.icc_time_cnt = 0;
        }
        icc_idx_last = ps_data.icc_idx[env];
    }
}

/// `calculateIID(ldPwrL, ldPwrR, iid, nEnvelopes, psBands)`
fn calculate_iid(
    ld_pwr_l: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    ld_pwr_r: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    iid: &mut [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    n_envelopes: i32,
    ps_bands: i32,
) {
    for env in 0..n_envelopes.max(0) as usize {
        for i in 0..ps_bands.max(0) as usize {
            // iid[env][i] = 10.0f*(float)log10(pwrL[env][i]/pwrR[env][i]);
            let mut iid_v = f_mult_div2(
                fl2fxconst_dbl((LOG10_2_10 / IID_SCALE_FT) as f64),
                ld_pwr_l[env][i].wrapping_sub(ld_pwr_r[env][i]),
            );

            iid_v = fix_min(iid_v, MAXVAL_DBL >> (LD_DATA_SHIFT + 1));
            iid_v = fix_max(iid_v, MINVAL_DBL >> (LD_DATA_SHIFT + 1));
            iid[env][i] = iid_v << (LD_DATA_SHIFT + 1);
        }
    }
}

/// `calculateICC(pwrL, pwrR, pwrCr, pwrCi, icc, nEnvelopes, psBands)` (file-static).
pub fn calculate_icc(
    pwr_l: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    pwr_r: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    pwr_cr: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    pwr_ci: &[[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    icc: &mut [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    n_envelopes: i32,
    ps_bands: i32,
) {
    let border = match ps_bands {
        PS_BANDS_COARSE => 5,
        PS_BANDS_MID => 11,
        _ => ps_bands,
    };

    for env in 0..n_envelopes.max(0) as usize {
        let mut i = 0usize;
        while (i as i32) < border {
            // icc[env][i] = min( pwrCr[env][i] / (float) sqrt(pwrL[env][i] * pwrR[env][i]) ,
            // 1.f);
            let mut scale = 0;
            let inv_nrg =
                inv_sqrt_norm2(f_max(f_mult(pwr_l[env][i], pwr_r[env][i]), 1), &mut scale);
            icc[env][i] = saturate_left_shift(f_mult(pwr_cr[env][i], inv_nrg), scale, DFRACT_BITS);
            i += 1;
        }

        while (i as i32) < ps_bands {
            let mut denom_e = 0;
            let denom_m = f_mult_norm(pwr_l[env][i], pwr_r[env][i], &mut denom_e);

            if denom_m == 0 {
                icc[env][i] = MAXVAL_DBL;
            } else {
                let mut result_e = 0;
                // CountLeadingBits(fixMax(fixp_abs(...), fixp_abs(...))) as compiled: an
                // unsigned maximum without the sign fold; num_e is -1 when a value is
                // MINVAL_DBL, and the shifts below then take the amount modulo 32 (31), as the
                // register shifts of both builds do.
                let num_e = clb_abs(umax_abs(fixp_abs(pwr_cr[env][i]), fixp_abs(pwr_ci[env][i])));
                let num_m = f_pow2_div2(pwr_cr[env][i].wrapping_shl(num_e as u32))
                    .wrapping_add(f_pow2_div2(pwr_ci[env][i].wrapping_shl(num_e as u32)));

                let result_m = f_div_norm(num_m, denom_m, &mut result_e);
                result_e += (-2 * num_e + 1) - denom_e;
                icc[env][i] = scale_value_saturate(
                    sqrt_fixp(result_m >> (result_e & 1)),
                    (result_e + (result_e & 1)) >> 1,
                );
            }
            i += 1;
        }
    }
}

/// `FDKsbrEnc_initPsBandNrgScale(hPsEncode)`
pub fn fdk_sbr_enc_init_ps_band_nrg_scale(h_ps_encode: &mut PsEncode) {
    let n_iid_groups = h_ps_encode.n_qmf_iid_groups + h_ps_encode.n_sub_qmf_iid_groups;

    h_ps_encode.ps_band_nrg_scale = [0; PS_MAX_BANDS];

    for group in 0..n_iid_groups.max(0) as usize {
        // Translate group to bin
        let mut bin = h_ps_encode.subband2parameter_index[group] as usize;

        // Translate from 20 bins to 10 bins
        if h_ps_encode.ps_enc_mode == PS_BANDS_COARSE {
            bin >>= 1;
        }

        h_ps_encode.ps_band_nrg_scale[bin] = if h_ps_encode.ps_band_nrg_scale[bin] == 0 {
            h_ps_encode.iid_group_width_ld[group] + 5
        } else {
            h_ps_encode.iid_group_width_ld[group].max(h_ps_encode.ps_band_nrg_scale[bin]) + 1
        };
    }
}

/// `FDKsbrEnc_CreatePSEncode(phPsEncode)`: the PS encoder (cleared).
pub fn fdk_sbr_enc_create_ps_encode() -> Box<PsEncode> {
    Box::default()
}

/// `FDKsbrEnc_InitPSEncode(hPsEncode, psEncMode, iidQuantErrorThreshold)`
pub fn fdk_sbr_enc_init_ps_encode(
    h_ps_encode: Option<&mut PsEncode>,
    ps_enc_mode: PsBands,
    iid_quant_error_threshold: FixpDbl,
) -> FdkPsencError {
    let Some(h) = h_ps_encode else {
        return PSENC_INVALID_HANDLE;
    };

    init_ps_data(&mut h.ps_data);

    match ps_enc_mode {
        PS_BANDS_COARSE | PS_BANDS_MID => {
            h.n_qmf_iid_groups = QMF_GROUPS_LO_RES as i32;
            h.n_sub_qmf_iid_groups = SUBQMF_GROUPS_LO_RES as i32;
            let n = QMF_GROUPS_LO_RES + SUBQMF_GROUPS_LO_RES;
            h.iid_group_borders[..n + 1].copy_from_slice(&IID_GROUP_BORDERS_LO_RES[..n + 1]);
            h.subband2parameter_index[..n].copy_from_slice(&SUBBAND2PARAMETER20[..n]);
            h.iid_group_width_ld[..n].copy_from_slice(&IID_GROUP_WIDTH_LD_LO_RES[..n]);
        }
        _ => return PSENC_INIT_ERROR,
    }

    h.ps_enc_mode = ps_enc_mode;
    h.iid_quant_error_threshold = iid_quant_error_threshold;
    fdk_sbr_enc_init_ps_band_nrg_scale(h);

    PSENC_OK
}

/// `PS_PWR_DATA`: the scratch of `FDKsbrEnc_PSEncode()`.
#[derive(Default)]
struct PsPwrData {
    pwr_l: [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    pwr_r: [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    ld_pwr_l: [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    ld_pwr_r: [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    pwr_cr: [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
    pwr_ci: [[FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES],
}

/// `FDKsbrEnc_PSEncode(hPsEncode, hPsOut, dynBandScale, maxEnvelopes, hybridData, frameSize,
/// sendHeader)`: the PS parameters of one frame into `h_ps_out`.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_ps_encode(
    h_ps_encode: &mut PsEncode,
    h_ps_out: &mut PsOut,
    dyn_band_scale: &[u8],
    max_envelopes: u32,
    hybrid_data: &HybridView,
    frame_size: i32,
    send_header: i32,
) -> FdkPsencError {
    let error = PSENC_OK;
    let mut iid = [[0 as FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES];
    let mut icc = [[0 as FixpDbl; PS_MAX_BANDS]; PS_MAX_ENVELOPES];
    let mut env_border = [0i32; PS_MAX_ENVELOPES + 1];
    let ps_bands = h_ps_encode.ps_enc_mode;
    let n_iid_groups = h_ps_encode.n_qmf_iid_groups + h_ps_encode.n_sub_qmf_iid_groups;
    let mut n_envelopes = max_envelopes.min(PS_MAX_ENVELOPES as u32) as i32;
    let mut pwr_data = Box::<PsPwrData>::default();
    let nb = ps_bands.max(0) as usize;

    for env in 0..(n_envelopes + 1) as usize {
        env_border[env] = f_mult_i(get_inv_int(n_envelopes), frame_size * env as i32);
    }

    for env in 0..n_envelopes as usize {
        // clear energy array
        for band in 0..nb {
            pwr_data.pwr_l[env][band] = 1;
            pwr_data.pwr_r[env][band] = 1;
            pwr_data.pwr_cr[env][band] = 1;
            pwr_data.pwr_ci[env][band] = 1;
        }

        // calculate energies and correlation
        // start with hybrid data
        for group in 0..n_iid_groups.max(0) as usize {
            // Translate group to bin
            let mut bin = h_ps_encode.subband2parameter_index[group] as usize;

            // Translate from 20 bins to 10 bins
            if h_ps_encode.ps_enc_mode == PS_BANDS_COARSE {
                bin >>= 1;
            }

            // determine group border
            let b_scale = h_ps_encode.ps_band_nrg_scale[bin] as u32;

            let mut pwr_l_env_bin = pwr_data.pwr_l[env][bin];
            let mut pwr_r_env_bin = pwr_data.pwr_r[env][bin];
            let mut pwr_cr_env_bin = pwr_data.pwr_cr[env][bin];
            let mut pwr_ci_env_bin = pwr_data.pwr_ci[env][bin];

            let scale = dyn_band_scale[bin] as u32;
            for col in env_border[env]..env_border[env + 1] {
                let col = col as usize;
                let lr = hybrid_data.row(col, 0, 0);
                let li = hybrid_data.row(col, 0, 1);
                let rr = hybrid_data.row(col, 1, 0);
                let ri = hybrid_data.row(col, 1, 1);
                for subband in
                    h_ps_encode.iid_group_borders[group]..h_ps_encode.iid_group_borders[group + 1]
                {
                    let s = subband as usize;
                    let l_real = lr[s].wrapping_shl(scale);
                    let l_imag = li[s].wrapping_shl(scale);
                    let r_real = rr[s].wrapping_shl(scale);
                    let r_imag = ri[s].wrapping_shl(scale);

                    pwr_l_env_bin = pwr_l_env_bin.wrapping_add(
                        f_pow2_div2(l_real).wrapping_add(f_pow2_div2(l_imag)).wrapping_shr(b_scale),
                    );
                    pwr_r_env_bin = pwr_r_env_bin.wrapping_add(
                        f_pow2_div2(r_real).wrapping_add(f_pow2_div2(r_imag)).wrapping_shr(b_scale),
                    );
                    pwr_cr_env_bin = pwr_cr_env_bin.wrapping_add(
                        f_mult_div2(l_real, r_real)
                            .wrapping_add(f_mult_div2(l_imag, r_imag))
                            .wrapping_shr(b_scale),
                    );
                    pwr_ci_env_bin = pwr_ci_env_bin.wrapping_add(
                        f_mult_div2(r_real, l_imag)
                            .wrapping_sub(f_mult_div2(l_real, r_imag))
                            .wrapping_shr(b_scale),
                    );
                }
            }
            // assure, nrg's of left and right channel are not negative; necessary on 16 bit
            // multiply units
            pwr_data.pwr_l[env][bin] = fix_max(0, pwr_l_env_bin);
            pwr_data.pwr_r[env][bin] = fix_max(0, pwr_r_env_bin);

            pwr_data.pwr_cr[env][bin] = pwr_cr_env_bin;
            pwr_data.pwr_ci[env][bin] = pwr_ci_env_bin;
        } // nIidGroups

        // calc logarithmic energy
        let PsPwrData { pwr_l, pwr_r, ld_pwr_l, ld_pwr_r, .. } = &mut *pwr_data;
        ld_data_vector(&pwr_l[env][..nb], &mut ld_pwr_l[env]);
        ld_data_vector(&pwr_r[env][..nb], &mut ld_pwr_r[env]);
    } // nEnvelopes

    // calculate iid and icc
    calculate_iid(&pwr_data.ld_pwr_l, &pwr_data.ld_pwr_r, &mut iid, n_envelopes, ps_bands);
    calculate_icc(
        &pwr_data.pwr_l,
        &pwr_data.pwr_r,
        &pwr_data.pwr_cr,
        &pwr_data.pwr_ci,
        &mut icc,
        n_envelopes,
        ps_bands,
    );

    // Envelope Reduction
    while envelope_reducible(&iid, &icc, ps_bands, n_envelopes) != 0 {
        // sum energies of two neighboring envelopes
        n_envelopes >>= 1;
        for e in 0..n_envelopes as usize {
            add_fixp_dbl(&mut pwr_data.pwr_l, 2 * e, 2 * e + 1, e, ps_bands);
            add_fixp_dbl(&mut pwr_data.pwr_r, 2 * e, 2 * e + 1, e, ps_bands);
            add_fixp_dbl(&mut pwr_data.pwr_cr, 2 * e, 2 * e + 1, e, ps_bands);
            add_fixp_dbl(&mut pwr_data.pwr_ci, 2 * e, 2 * e + 1, e, ps_bands);

            // calc logarithmic energy
            let PsPwrData { pwr_l, pwr_r, ld_pwr_l, ld_pwr_r, .. } = &mut *pwr_data;
            ld_data_vector(&pwr_l[e][..nb], &mut ld_pwr_l[e]);
            ld_data_vector(&pwr_r[e][..nb], &mut ld_pwr_r[e]);

            // reduce number of envelopes and adjust borders
            env_border[e] = env_border[2 * e];
        }
        env_border[n_envelopes as usize] = env_border[2 * n_envelopes as usize];

        // re-calculate iid and icc
        calculate_iid(&pwr_data.ld_pwr_l, &pwr_data.ld_pwr_r, &mut iid, n_envelopes, ps_bands);
        calculate_icc(
            &pwr_data.pwr_l,
            &pwr_data.pwr_r,
            &pwr_data.pwr_cr,
            &pwr_data.pwr_ci,
            &mut icc,
            n_envelopes,
            ps_bands,
        );
    }

    let h_ps_data = &mut h_ps_encode.ps_data;

    if send_header != 0 {
        h_ps_data.header_cnt = MAX_PS_NOHEADER_CNT;
        h_ps_data.iid_time_cnt = MAX_TIME_DIFF_FRAMES;
        h_ps_data.icc_time_cnt = MAX_TIME_DIFF_FRAMES;
        h_ps_data.no_env_cnt = MAX_NOENV_CNT;
    }

    // Parameter processing, quantisation etc
    process_iid_data(h_ps_data, &iid, ps_bands, n_envelopes, h_ps_encode.iid_quant_error_threshold);
    process_icc_data(h_ps_data, &icc, ps_bands, n_envelopes);

    // Initialize output struct

    // PS Header on/off ?
    if h_ps_data.header_cnt < MAX_PS_NOHEADER_CNT
        && (h_ps_data.iid_quant_mode == h_ps_data.iid_quant_mode_last
            && h_ps_data.icc_quant_mode == h_ps_data.icc_quant_mode_last)
        && (h_ps_data.iid_enable == h_ps_data.iid_enable_last
            && h_ps_data.icc_enable == h_ps_data.icc_enable_last)
    {
        h_ps_out.enable_ps_header = 0;
    } else {
        h_ps_out.enable_ps_header = 1;
        h_ps_data.header_cnt = 0;
    }

    // nEnvelopes = 0 ?
    if h_ps_data.no_env_cnt < MAX_NOENV_CNT
        && similar_iid(h_ps_data, ps_bands, n_envelopes) != 0
        && similar_icc(h_ps_data, ps_bands, n_envelopes) != 0
    {
        n_envelopes = 0;
        h_ps_out.n_envelopes = 0;
        h_ps_data.no_env_cnt += 1;
    } else {
        h_ps_data.no_env_cnt = 0;
    }

    if n_envelopes > 0 {
        h_ps_out.enable_iid = h_ps_data.iid_enable;
        h_ps_out.iid_mode = get_iid_mode(ps_bands, h_ps_data.iid_quant_mode);

        h_ps_out.enable_icc = h_ps_data.icc_enable;
        h_ps_out.icc_mode = get_icc_mode(ps_bands, h_ps_data.icc_quant_mode);

        h_ps_out.enable_ipd_opd = 0;
        h_ps_out.frame_class = 0;
        h_ps_out.n_envelopes = n_envelopes;

        for env in 0..n_envelopes as usize {
            h_ps_out.frame_border[env] = env_border[env + 1];
            h_ps_out.delta_iid[env] = h_ps_data.iid_diff_mode[env];
            h_ps_out.delta_icc[env] = h_ps_data.icc_diff_mode[env];
            for band in 0..nb {
                h_ps_out.iid[env][band] = h_ps_data.iid_idx[env][band];
                h_ps_out.icc[env][band] = h_ps_data.icc_idx[env][band];
            }
        }

        // IPD OPD not supported right now
        h_ps_out.ipd = [[0; PS_MAX_BANDS]; PS_MAX_ENVELOPES];
        for env in 0..PS_MAX_ENVELOPES {
            h_ps_out.delta_ipd[env] = PS_DELTA_FREQ;
            h_ps_out.delta_opd[env] = PS_DELTA_FREQ;
        }

        h_ps_out.ipd_last = [0; PS_MAX_BANDS];
        h_ps_out.opd_last = [0; PS_MAX_BANDS];

        for band in 0..PS_MAX_BANDS {
            h_ps_out.iid_last[band] = h_ps_data.iid_idx_last[band];
            h_ps_out.icc_last[band] = h_ps_data.icc_idx_last[band];
        }

        // save iids and iccs for differential time coding in the next frame
        h_ps_data.n_envelopes_last = n_envelopes;
        h_ps_data.iid_enable_last = h_ps_data.iid_enable;
        h_ps_data.icc_enable_last = h_ps_data.icc_enable;
        h_ps_data.iid_quant_mode_last = h_ps_data.iid_quant_mode;
        h_ps_data.icc_quant_mode_last = h_ps_data.icc_quant_mode;
        for i in 0..nb {
            h_ps_data.iid_idx_last[i] = h_ps_data.iid_idx[n_envelopes as usize - 1][i];
            h_ps_data.icc_idx_last[i] = h_ps_data.icc_idx[n_envelopes as usize - 1][i];
        }
    } // Envelope > 0

    error
}
