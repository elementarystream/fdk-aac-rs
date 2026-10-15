// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/code_env.h, libSBRenc/src/code_env.cpp.

//! Port of `code_env.h` and `code_env.cpp`: the DPCM coding (in time or frequency direction)
//! of the envelope and noise floor data.

use fdk_aac_rs_fdk::common_fix::*;

use crate::bit_sbr::SbrEnvData;
use crate::sbr_def::*;
use crate::sbr_encoder::{FreqRes, FREQ_RES_HIGH, FREQ_RES_LOW};
use crate::sbrenc_rom::*;

/// `SBR_CODE_ENVELOPE` (`HANDLE_SBR_CODE_ENVELOPE` is `&mut SbrCodeEnvelope`). The Huffman
/// length tables are references to the tables of [`crate::sbrenc_rom`] (`None` for `NULL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbrCodeEnvelope {
    /// `offset`
    pub offset: i32,
    /// `upDate`
    pub up_date: i32,
    /// `nSfb`
    pub n_sfb: [i32; 2],
    /// `sfb_nrg_prev`
    pub sfb_nrg_prev: [i8; MAX_FREQ_COEFFS],
    /// `deltaTAcrossFrames`
    pub delta_t_across_frames: i32,
    /// `dF_edge_1stEnv`
    pub d_f_edge_1st_env: FixpDbl,
    /// `dF_edge_incr`
    pub d_f_edge_incr: FixpDbl,
    /// `dF_edge_incr_fac`
    pub d_f_edge_incr_fac: i32,

    /// `codeBookScfLavTime`
    pub code_book_scf_lav_time: i32,
    /// `codeBookScfLavFreq`
    pub code_book_scf_lav_freq: i32,

    /// `codeBookScfLavLevelTime`
    pub code_book_scf_lav_level_time: i32,
    /// `codeBookScfLavLevelFreq`
    pub code_book_scf_lav_level_freq: i32,
    /// `codeBookScfLavBalanceTime`
    pub code_book_scf_lav_balance_time: i32,
    /// `codeBookScfLavBalanceFreq`
    pub code_book_scf_lav_balance_freq: i32,

    /// `start_bits`
    pub start_bits: i32,
    /// `start_bits_balance`
    pub start_bits_balance: i32,

    /// `hufftableTimeL`
    pub hufftable_time_l: Option<&'static [u8]>,
    /// `hufftableFreqL`
    pub hufftable_freq_l: Option<&'static [u8]>,

    /// `hufftableLevelTimeL`
    pub hufftable_level_time_l: Option<&'static [u8]>,
    /// `hufftableBalanceTimeL`
    pub hufftable_balance_time_l: Option<&'static [u8]>,
    /// `hufftableLevelFreqL`
    pub hufftable_level_freq_l: Option<&'static [u8]>,
    /// `hufftableBalanceFreqL`
    pub hufftable_balance_freq_l: Option<&'static [u8]>,
}

impl Default for SbrCodeEnvelope {
    fn default() -> Self {
        Self {
            offset: 0,
            up_date: 0,
            n_sfb: [0; 2],
            sfb_nrg_prev: [0; MAX_FREQ_COEFFS],
            delta_t_across_frames: 0,
            d_f_edge_1st_env: 0,
            d_f_edge_incr: 0,
            d_f_edge_incr_fac: 0,
            code_book_scf_lav_time: 0,
            code_book_scf_lav_freq: 0,
            code_book_scf_lav_level_time: 0,
            code_book_scf_lav_level_freq: 0,
            code_book_scf_lav_balance_time: 0,
            code_book_scf_lav_balance_freq: 0,
            start_bits: 0,
            start_bits_balance: 0,
            hufftable_time_l: None,
            hufftable_freq_l: None,
            hufftable_level_time_l: None,
            hufftable_balance_time_l: None,
            hufftable_level_freq_l: None,
            hufftable_balance_freq_l: None,
        }
    }
}

/// `FDKsbrEnc_InitSbrHuffmanTables(sbrEnvData, henv, hnoise, amp_res)`: initializes the
/// Huffman tables for the amplitude resolution. Returns 0 if OK.
pub fn fdk_sbr_enc_init_sbr_huffman_tables(
    sbr_env_data: &mut SbrEnvData,
    henv: &mut SbrCodeEnvelope,
    hnoise: &mut SbrCodeEnvelope,
    amp_res: AmpRes,
) -> i32 {
    sbr_env_data.init_sbr_amp_res = amp_res;

    match amp_res {
        SBR_AMP_RES_3_0 => {
            // envelope data
            // Level/Pan - coding
            sbr_env_data.hufftable_level_time_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_11T);
            sbr_env_data.hufftable_level_time_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_11T);
            sbr_env_data.hufftable_balance_time_c = Some(&BOOK_SBR_ENV_BALANCE_C_11T);
            sbr_env_data.hufftable_balance_time_l = Some(&BOOK_SBR_ENV_BALANCE_L_11T);

            sbr_env_data.hufftable_level_freq_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_11F);
            sbr_env_data.hufftable_level_freq_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_11F);
            sbr_env_data.hufftable_balance_freq_c = Some(&BOOK_SBR_ENV_BALANCE_C_11F);
            sbr_env_data.hufftable_balance_freq_l = Some(&BOOK_SBR_ENV_BALANCE_L_11F);

            // Right/Left - coding
            sbr_env_data.hufftable_time_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_11T);
            sbr_env_data.hufftable_time_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_11T);
            sbr_env_data.hufftable_freq_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_11F);
            sbr_env_data.hufftable_freq_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_11F);

            sbr_env_data.code_book_scf_lav_balance = CODE_BOOK_SCF_LAV_BALANCE11;
            sbr_env_data.code_book_scf_lav = CODE_BOOK_SCF_LAV11;

            sbr_env_data.si_sbr_start_env_bits = SI_SBR_START_ENV_BITS_AMP_RES_3_0;
            sbr_env_data.si_sbr_start_env_bits_balance = SI_SBR_START_ENV_BITS_BALANCE_AMP_RES_3_0;
        }
        SBR_AMP_RES_1_5 => {
            // envelope data
            // Level/Pan - coding
            sbr_env_data.hufftable_level_time_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_10T);
            sbr_env_data.hufftable_level_time_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_10T);
            sbr_env_data.hufftable_balance_time_c = Some(&BOOK_SBR_ENV_BALANCE_C_10T);
            sbr_env_data.hufftable_balance_time_l = Some(&BOOK_SBR_ENV_BALANCE_L_10T);

            sbr_env_data.hufftable_level_freq_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_10F);
            sbr_env_data.hufftable_level_freq_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_10F);
            sbr_env_data.hufftable_balance_freq_c = Some(&BOOK_SBR_ENV_BALANCE_C_10F);
            sbr_env_data.hufftable_balance_freq_l = Some(&BOOK_SBR_ENV_BALANCE_L_10F);

            // Right/Left - coding
            sbr_env_data.hufftable_time_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_10T);
            sbr_env_data.hufftable_time_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_10T);
            sbr_env_data.hufftable_freq_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_10F);
            sbr_env_data.hufftable_freq_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_10F);

            sbr_env_data.code_book_scf_lav_balance = CODE_BOOK_SCF_LAV_BALANCE10;
            sbr_env_data.code_book_scf_lav = CODE_BOOK_SCF_LAV10;

            sbr_env_data.si_sbr_start_env_bits = SI_SBR_START_ENV_BITS_AMP_RES_1_5;
            sbr_env_data.si_sbr_start_env_bits_balance = SI_SBR_START_ENV_BITS_BALANCE_AMP_RES_1_5;
        }
        _ => return 1, // undefined amp_res mode
    }

    // these are common to both amp_res values
    // Noise data
    // Level/Pan - coding
    sbr_env_data.hufftable_noise_level_time_c = Some(&V_HUFF_NOISE_LEVEL_C_11T);
    sbr_env_data.hufftable_noise_level_time_l = Some(&V_HUFF_NOISE_LEVEL_L_11T);
    sbr_env_data.hufftable_noise_balance_time_c = Some(&BOOK_SBR_NOISE_BALANCE_C_11T);
    sbr_env_data.hufftable_noise_balance_time_l = Some(&BOOK_SBR_NOISE_BALANCE_L_11T);

    sbr_env_data.hufftable_noise_level_freq_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_11F);
    sbr_env_data.hufftable_noise_level_freq_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_11F);
    sbr_env_data.hufftable_noise_balance_freq_c = Some(&BOOK_SBR_ENV_BALANCE_C_11F);
    sbr_env_data.hufftable_noise_balance_freq_l = Some(&BOOK_SBR_ENV_BALANCE_L_11F);

    // Right/Left - coding
    sbr_env_data.hufftable_noise_time_c = Some(&V_HUFF_NOISE_LEVEL_C_11T);
    sbr_env_data.hufftable_noise_time_l = Some(&V_HUFF_NOISE_LEVEL_L_11T);
    sbr_env_data.hufftable_noise_freq_c = Some(&V_HUFF_ENVELOPE_LEVEL_C_11F);
    sbr_env_data.hufftable_noise_freq_l = Some(&V_HUFF_ENVELOPE_LEVEL_L_11F);

    sbr_env_data.si_sbr_start_noise_bits = SI_SBR_START_NOISE_BITS_AMP_RES_3_0;
    sbr_env_data.si_sbr_start_noise_bits_balance = SI_SBR_START_NOISE_BITS_BALANCE_AMP_RES_3_0;

    // init envelope tables and codebooks
    henv.code_book_scf_lav_balance_time = sbr_env_data.code_book_scf_lav_balance;
    henv.code_book_scf_lav_balance_freq = sbr_env_data.code_book_scf_lav_balance;
    henv.code_book_scf_lav_level_time = sbr_env_data.code_book_scf_lav;
    henv.code_book_scf_lav_level_freq = sbr_env_data.code_book_scf_lav;
    henv.code_book_scf_lav_time = sbr_env_data.code_book_scf_lav;
    henv.code_book_scf_lav_freq = sbr_env_data.code_book_scf_lav;

    henv.hufftable_level_time_l = sbr_env_data.hufftable_level_time_l;
    henv.hufftable_balance_time_l = sbr_env_data.hufftable_balance_time_l;
    henv.hufftable_time_l = sbr_env_data.hufftable_time_l;
    henv.hufftable_level_freq_l = sbr_env_data.hufftable_level_freq_l;
    henv.hufftable_balance_freq_l = sbr_env_data.hufftable_balance_freq_l;
    henv.hufftable_freq_l = sbr_env_data.hufftable_freq_l;

    henv.code_book_scf_lav_freq = sbr_env_data.code_book_scf_lav;
    henv.code_book_scf_lav_time = sbr_env_data.code_book_scf_lav;

    henv.start_bits = sbr_env_data.si_sbr_start_env_bits;
    henv.start_bits_balance = sbr_env_data.si_sbr_start_env_bits_balance;

    // init noise tables and codebooks

    hnoise.code_book_scf_lav_balance_time = CODE_BOOK_SCF_LAV_BALANCE11;
    hnoise.code_book_scf_lav_balance_freq = CODE_BOOK_SCF_LAV_BALANCE11;
    hnoise.code_book_scf_lav_level_time = CODE_BOOK_SCF_LAV11;
    hnoise.code_book_scf_lav_level_freq = CODE_BOOK_SCF_LAV11;
    hnoise.code_book_scf_lav_time = CODE_BOOK_SCF_LAV11;
    hnoise.code_book_scf_lav_freq = CODE_BOOK_SCF_LAV11;

    hnoise.hufftable_level_time_l = sbr_env_data.hufftable_noise_level_time_l;
    hnoise.hufftable_balance_time_l = sbr_env_data.hufftable_noise_balance_time_l;
    hnoise.hufftable_time_l = sbr_env_data.hufftable_noise_time_l;
    hnoise.hufftable_level_freq_l = sbr_env_data.hufftable_noise_level_freq_l;
    hnoise.hufftable_balance_freq_l = sbr_env_data.hufftable_noise_balance_freq_l;
    hnoise.hufftable_freq_l = sbr_env_data.hufftable_noise_freq_l;

    hnoise.start_bits = sbr_env_data.si_sbr_start_noise_bits;
    hnoise.start_bits_balance = sbr_env_data.si_sbr_start_noise_bits_balance;

    // No delta coding in time from the previous frame due to 1.5dB FIx-FIX rule
    henv.up_date = 0;
    hnoise.up_date = 0;
    0
}

/// `indexLow2High(offset, index, res)`: patch function to cope with non-factor-2 ratios
/// between high-res and low-res.
fn index_low2_high(mut offset: i32, index: i32, res: FreqRes) -> i32 {
    if res == FREQ_RES_LOW {
        if offset >= 0 {
            if index < offset {
                index
            } else {
                2 * index - offset
            }
        } else {
            offset = -offset;
            if index < offset {
                2 * index + index
            } else {
                2 * index + offset
            }
        }
    } else {
        index
    }
}

/// `mapLowResEnergyVal(currVal, prevData, offset, index, res)`
fn map_low_res_energy_val(
    curr_val: i8,
    prev_data: &mut [i8],
    mut offset: i32,
    index: i32,
    res: FreqRes,
) {
    let set = |p: &mut [i8], i: i32, v: i8| {
        // Bounded: writes outside of sfb_nrg_prev are dropped.
        if let Some(x) = usize::try_from(i).ok().and_then(|i| p.get_mut(i)) {
            *x = v;
        }
    };
    if res == FREQ_RES_LOW {
        if offset >= 0 {
            if index < offset {
                set(prev_data, index, curr_val);
            } else {
                set(prev_data, 2 * index - offset, curr_val);
                set(prev_data, 2 * index + 1 - offset, curr_val);
            }
        } else {
            offset = -offset;
            if index < offset {
                set(prev_data, 3 * index, curr_val);
                set(prev_data, 3 * index + 1, curr_val);
                set(prev_data, 3 * index + 2, curr_val);
            } else {
                set(prev_data, 2 * index + offset, curr_val);
                set(prev_data, 2 * index + 1 + offset, curr_val);
            }
        }
    } else {
        set(prev_data, index, curr_val);
    }
}

/// `computeBits(delta, codeBookScfLavLevel, codeBookScfLavBalance, hufftableLevel,
/// hufftableBalance, coupling, channel)`: the bits of a delta value (10000 if it exceeds the
/// codebook, which then clips it).
fn compute_bits(
    delta: &mut i8,
    code_book_scf_lav_level: i32,
    code_book_scf_lav_balance: i32,
    hufftable_level: &[u8],
    hufftable_balance: &[u8],
    coupling: i32,
    channel: i32,
) -> i32 {
    let d = *delta as i32;
    let (lav, table) = if coupling != 0 && channel == 1 {
        (code_book_scf_lav_balance, hufftable_balance)
    } else {
        (code_book_scf_lav_level, hufftable_level)
    };
    let index = if d < 0 { fix_max(d, -lav) } else { fix_min(d, lav) };
    if index != d {
        *delta = index as i8;
        return 10000;
    }
    table.get((index + lav) as usize).copied().unwrap_or(0) as i32
}

/// `FDKsbrEnc_codeEnvelope(sfb_nrg, freq_res, h_sbrCodeEnvelope, directionVec, coupling,
/// nEnvelopes, channel, headerActive)`: codes the envelopes (or noise floors) of a frame in
/// time or frequency direction, whichever needs fewer bits. `sfb_nrg` is replaced by the
/// deltas; `h_sbrCodeEnvelope.sfb_nrg_prev`, `.upDate` and `directionVec` are updated.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_code_envelope(
    sfb_nrg: &mut [i8],
    freq_res: &[FreqRes],
    h: &mut SbrCodeEnvelope,
    direction_vec: &mut [i32],
    coupling: i32,
    n_envelopes: i32,
    channel: i32,
    header_active: i32,
) {
    let offset = h.offset;
    let mut delta_t_bits = 0;
    let mut delta_f = [0i8; MAX_FREQ_COEFFS];
    let mut delta_t = [0i8; MAX_FREQ_COEFFS];

    let tmp1: FixpDbl = fl2fxconst_dbl(0.5f32 as f64) >> (DFRACT_BITS - 16 - 1);
    let tmp2: FixpDbl = h.d_f_edge_1st_env >> (DFRACT_BITS - 16);
    let tmp3: FixpDbl = f_mult(h.d_f_edge_incr, h.d_f_edge_incr_fac.wrapping_shl(15));

    let d_f_edge_1st_env = tmp1.wrapping_add(tmp2).wrapping_add(tmp3);

    let empty: &'static [u8] = &[];
    let (
        code_book_scf_lav_level_time,
        code_book_scf_lav_level_freq,
        code_book_scf_lav_balance_time,
        code_book_scf_lav_balance_freq,
        hufftable_level_time_l,
        hufftable_balance_time_l,
        hufftable_level_freq_l,
        hufftable_balance_freq_l,
    ) = if coupling != 0 {
        (
            h.code_book_scf_lav_level_time,
            h.code_book_scf_lav_level_freq,
            h.code_book_scf_lav_balance_time,
            h.code_book_scf_lav_balance_freq,
            h.hufftable_level_time_l.unwrap_or(empty),
            h.hufftable_balance_time_l.unwrap_or(empty),
            h.hufftable_level_freq_l.unwrap_or(empty),
            h.hufftable_balance_freq_l.unwrap_or(empty),
        )
    } else {
        (
            h.code_book_scf_lav_time,
            h.code_book_scf_lav_freq,
            h.code_book_scf_lav_time,
            h.code_book_scf_lav_freq,
            h.hufftable_time_l.unwrap_or(empty),
            h.hufftable_time_l.unwrap_or(empty),
            h.hufftable_freq_l.unwrap_or(empty),
            h.hufftable_freq_l.unwrap_or(empty),
        )
    };

    let env_data_table_comp_factor = if coupling == 1 && channel == 1 {
        1 // should be one when the new huffman-tables are ready
    } else {
        0
    };

    if h.delta_t_across_frames == 0 {
        h.up_date = 0;
    }

    // no delta coding in time in case of a header
    if header_active != 0 {
        h.up_date = 0;
    }

    let mut base = 0usize;
    for i in 0..n_envelopes.max(0) as usize {
        let no_of_bands = if freq_res[i] == FREQ_RES_HIGH {
            h.n_sfb[FREQ_RES_HIGH as usize]
        } else {
            h.n_sfb[FREQ_RES_LOW as usize]
        };
        let nb = no_of_bands.max(0) as usize;

        let ptr_nrg = &mut sfb_nrg[base..];
        let mut curr_nrg = ptr_nrg[0];

        delta_f[0] = (curr_nrg as i32 >> env_data_table_comp_factor) as i8;

        let mut delta_f_bits =
            if coupling != 0 && channel == 1 { h.start_bits_balance } else { h.start_bits };

        if h.up_date != 0 {
            delta_t[0] =
                ((curr_nrg as i32 - h.sfb_nrg_prev[0] as i32) >> env_data_table_comp_factor) as i8;

            delta_t_bits = compute_bits(
                &mut delta_t[0],
                code_book_scf_lav_level_time,
                code_book_scf_lav_balance_time,
                hufftable_level_time_l,
                hufftable_balance_time_l,
                coupling,
                channel,
            );
        }

        map_low_res_energy_val(curr_nrg, &mut h.sfb_nrg_prev, offset, 0, freq_res[i]);

        // ensure that nrg difference is not higher than codeBookScfLavXXXFreq
        let lav = if coupling != 0 && channel == 1 {
            code_book_scf_lav_balance_freq
        } else {
            code_book_scf_lav_level_freq
        };
        let mut band = nb as i32 - 1;
        while band > 0 {
            let b = band as usize;
            if ptr_nrg[b] as i32 - ptr_nrg[b - 1] as i32 > lav {
                ptr_nrg[b - 1] = (ptr_nrg[b] as i32 - lav) as i8;
            }
            band -= 1;
        }
        for b in 1..nb {
            if ptr_nrg[b - 1] as i32 - ptr_nrg[b] as i32 > lav {
                ptr_nrg[b] = (ptr_nrg[b - 1] as i32 - lav) as i8;
            }
        }

        // Coding loop
        for b in 1..nb {
            let last_nrg = ptr_nrg[b - 1];
            curr_nrg = ptr_nrg[b];

            delta_f[b] = ((curr_nrg as i32 - last_nrg as i32) >> env_data_table_comp_factor) as i8;

            delta_f_bits += compute_bits(
                &mut delta_f[b],
                code_book_scf_lav_level_freq,
                code_book_scf_lav_balance_freq,
                hufftable_level_freq_l,
                hufftable_balance_freq_l,
                coupling,
                channel,
            );

            if h.up_date != 0 {
                let prev = h.sfb_nrg_prev[index_low2_high(offset, b as i32, freq_res[i])
                    .clamp(0, MAX_FREQ_COEFFS as i32 - 1)
                    as usize];
                delta_t[b] = (curr_nrg as i32 - prev as i32) as i8;
                delta_t[b] = (delta_t[b] as i32 >> env_data_table_comp_factor) as i8;
            }

            map_low_res_energy_val(curr_nrg, &mut h.sfb_nrg_prev, offset, b as i32, freq_res[i]);

            if h.up_date != 0 {
                delta_t_bits += compute_bits(
                    &mut delta_t[b],
                    code_book_scf_lav_level_time,
                    code_book_scf_lav_balance_time,
                    hufftable_level_time_l,
                    hufftable_balance_time_l,
                    coupling,
                    channel,
                );
            }
        }

        // Replace sfb_nrg with deltacoded samples and set flag
        let use_dt = if i == 0 {
            // C: (((delta_T_bits * dF_edge_1stEnv) >> (DFRACT_BITS - 18)) + 1) >> 1; the
            // product is computed in 32 bits and can wrap.
            let tmp_bits = ((delta_t_bits.wrapping_mul(d_f_edge_1st_env) >> (DFRACT_BITS - 18))
                .wrapping_add(1))
                >> 1;
            h.up_date != 0 && delta_f_bits > tmp_bits
        } else {
            delta_t_bits < delta_f_bits && h.up_date != 0
        };

        if use_dt {
            direction_vec[i] = TIME;
            sfb_nrg[base..base + nb].copy_from_slice(&delta_t[..nb]);
        } else {
            h.up_date = 0;
            direction_vec[i] = FREQ;
            sfb_nrg[base..base + nb].copy_from_slice(&delta_f[..nb]);
        }
        base += nb;
        h.up_date = 1;
    }
}

/// `FDKsbrEnc_InitSbrCodeEnvelope(h_sbrCodeEnvelope, nSfb, deltaTAcrossFrames,
/// dF_edge_1stEnv, dF_edge_incr)`: returns 0.
pub fn fdk_sbr_enc_init_sbr_code_envelope(
    h: &mut SbrCodeEnvelope,
    n_sfb: &[i32; 2],
    delta_t_across_frames: i32,
    d_f_edge_1st_env: FixpDbl,
    d_f_edge_incr: FixpDbl,
) -> i32 {
    *h = SbrCodeEnvelope::default();
    h.delta_t_across_frames = delta_t_across_frames;
    h.d_f_edge_1st_env = d_f_edge_1st_env;
    h.d_f_edge_incr = d_f_edge_incr;
    h.d_f_edge_incr_fac = 0;
    h.up_date = 0;
    h.n_sfb[FREQ_RES_LOW as usize] = n_sfb[FREQ_RES_LOW as usize];
    h.n_sfb[FREQ_RES_HIGH as usize] = n_sfb[FREQ_RES_HIGH as usize];
    h.offset = 2 * h.n_sfb[FREQ_RES_LOW as usize] - h.n_sfb[FREQ_RES_HIGH as usize];

    0
}
