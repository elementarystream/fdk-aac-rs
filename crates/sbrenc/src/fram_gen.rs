// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/fram_gen.h, libSBRenc/src/fram_gen.cpp.

//! Port of `fram_gen.h` and `fram_gen.cpp`: the frame generator, which designs the
//! time/frequency grid of an SBR frame from the transient information.
//!
//! The vectors of borders and frequency resolutions (`v_bord`, `v_freq`, ...) have the sizes of
//! the C structure; the algorithm stays within them for transient positions inside the frame.

use fdk_aac_rs_fdk::fdk_assert;

use crate::sbr_def::{MAX_ENVELOPES, MAX_NOISE_ENVELOPES};
use crate::sbr_encoder::{FreqRes, FREQ_RES_LOW};
use crate::sbr_misc::{fdk_sbr_enc_add_left, fdk_sbr_enc_add_right, fdk_sbr_enc_add_vec_left};

mod tables;
pub use tables::*;

/// `MAX_ENVELOPES_VARVAR`: worst case number of envelopes in a VARVAR frame
pub const MAX_ENVELOPES_VARVAR: usize = MAX_ENVELOPES;
/// `MAX_ENVELOPES_FIXVAR_VARFIX`: worst case number of envelopes in VARFIX and FIXVAR frames
pub const MAX_ENVELOPES_FIXVAR_VARFIX: usize = 4;
/// `MAX_NUM_REL`: maximum number of relative borders in any VAR frame
pub const MAX_NUM_REL: usize = 3;

/// `FRAME_CLASS`
pub type FrameClass = i32;
/// `FIXFIX`: leading and trailing frame borders are fixed
pub const FIXFIX: FrameClass = 0;
/// `FIXVAR`: leading frame border is fixed, trailing frame border is variable
pub const FIXVAR: FrameClass = 1;
/// `VARFIX`: leading frame border is variable, trailing frame border is fixed
pub const VARFIX: FrameClass = 2;
/// `VARVAR`: leading and trailing frame borders are variable
pub const VARVAR: FrameClass = 3;
/// `FIXFIXonly`: leading border fixed (0), trailing border fixed (nrTimeSlots) and encased
/// borders are dynamically derived from the tranPos
pub const FIXFIX_ONLY: FrameClass = 4;

/// `DC`: helper constant: don't care
pub const DC: i32 = 4711;
/// `EMPTY`: helper constant: empty
pub const EMPTY: i32 = -99;

/// `FRAME_MIDDLE_SLOT_1920`
pub const FRAME_MIDDLE_SLOT_1920: i32 = 4;
/// `NUMBER_TIME_SLOTS_1920`
pub const NUMBER_TIME_SLOTS_1920: i32 = 15;

/// `LD_PRETRAN_OFF`
pub const LD_PRETRAN_OFF: i32 = 3;
/// `FRAME_MIDDLE_SLOT_512LD`
pub const FRAME_MIDDLE_SLOT_512LD: i32 = 4;
/// `NUMBER_TIME_SLOTS_512LD`
pub const NUMBER_TIME_SLOTS_512LD: i32 = 8;
/// `TRANSIENT_OFFSET_LD`
pub const TRANSIENT_OFFSET_LD: i32 = 0;

/// `FRAME_MIDDLE_SLOT_2048`
pub const FRAME_MIDDLE_SLOT_2048: i32 = 4;
/// `NUMBER_TIME_SLOTS_2048`
pub const NUMBER_TIME_SLOTS_2048: i32 = 16;

/// `FRAME_MIDDLE_SLOT_1152`
pub const FRAME_MIDDLE_SLOT_1152: i32 = 4;
/// `NUMBER_TIME_SLOTS_1152`
pub const NUMBER_TIME_SLOTS_1152: i32 = 9;

/// `FRAME_MIDDLE_SLOT_2304`
pub const FRAME_MIDDLE_SLOT_2304: i32 = 8;
/// `NUMBER_TIME_SLOTS_2304`
pub const NUMBER_TIME_SLOTS_2304: i32 = 18;

/// `SBR_GRID`: sbr_grid() signals to be converted to bitstream elements ("clear text").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrGrid {
    // system constants
    /// `bufferFrameStart`: frame generator vs analysis buffer time alignment
    pub buffer_frame_start: i32,
    /// `numberTimeSlots`: number of SBR timeslots per frame
    pub number_time_slots: i32,

    // will be adjusted for every frame
    /// `frameClass`: SBR frame class
    pub frame_class: FrameClass,
    /// `bs_num_env`: number of envelopes for FIXFIX
    pub bs_num_env: i32,
    /// `bs_abs_bord`: absolute border for VARFIX and FIXVAR
    pub bs_abs_bord: i32,
    /// `n`: number of relative borders for VARFIX and FIXVAR
    pub n: i32,
    /// `p`: pointer-to-transient-border
    pub p: i32,
    /// `bs_rel_bord`: relative borders for all VAR
    pub bs_rel_bord: [i32; MAX_NUM_REL],
    /// `v_f`: envelope frequency resolutions for FIXVAR and VARFIX
    pub v_f: [i32; MAX_ENVELOPES_FIXVAR_VARFIX],

    /// `bs_abs_bord_0`: leading absolute border for VARVAR
    pub bs_abs_bord_0: i32,
    /// `bs_abs_bord_1`: trailing absolute border for VARVAR
    pub bs_abs_bord_1: i32,
    /// `bs_num_rel_0`: number of relative borders associated with leading absolute border for
    /// VARVAR
    pub bs_num_rel_0: i32,
    /// `bs_num_rel_1`: number of relative borders associated with trailing absolute border for
    /// VARVAR
    pub bs_num_rel_1: i32,
    /// `bs_rel_bord_0`: relative borders associated with leading absolute border for VARVAR
    pub bs_rel_bord_0: [i32; MAX_NUM_REL],
    /// `bs_rel_bord_1`: relative borders associated with trailing absolute border for VARVAR
    pub bs_rel_bord_1: [i32; MAX_NUM_REL],
    /// `v_fLR`: envelope frequency resolutions for VARVAR
    pub v_f_lr: [i32; MAX_ENVELOPES_VARVAR],
}

/// `SBR_FRAME_INFO`: time/frequency grid description for one frame
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrFrameInfo {
    /// `nEnvelopes`: number of envelopes
    pub n_envelopes: i32,
    /// `borders`: envelope borders in SBR timeslots
    pub borders: [i32; MAX_ENVELOPES + 1],
    /// `freqRes`: frequency resolution of each envelope
    pub freq_res: [FreqRes; MAX_ENVELOPES],
    /// `shortEnv`: number of an envelope to be shortened (starting at 1) or 0 for no shortened
    /// envelope
    pub short_env: i32,
    /// `nNoiseEnvelopes`: number of noise floors
    pub n_noise_envelopes: i32,
    /// `bordersNoise`: noise floor borders in SBR timeslots
    pub borders_noise: [i32; MAX_NOISE_ENVELOPES + 1],
}

/// `SBR_ENVELOPE_FRAME`: frame generator main struct (tuning parameters, time/frequency grid
/// description, sbr_grid() bitstream elements and generator internal signals).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrEnvelopeFrame {
    // system constants
    /// `frameMiddleSlot`: transient detector offset in SBR timeslots
    pub frame_middle_slot: i32,

    // basic tuning parameters
    /// `staticFraming`: 1: run static framing in time, i.e. exclusive use of bs_frame_class =
    /// FIXFIX
    pub static_framing: i32,
    /// `numEnvStatic`: number of envelopes per frame for static framing
    pub num_env_static: i32,
    /// `freq_res_fixfix`: envelope frequency resolution to use for bs_frame_class = FIXFIX;
    /// single env and split
    pub freq_res_fixfix: [FreqRes; 2],
    /// `fResTransIsLow`: frequency resolution for transient frames - always low (0) or
    /// according to table (1)
    pub f_res_trans_is_low: u8,

    // expert tuning parameters
    /// `v_tuningSegm`: segment lengths to use around transient (`None` for `NULL`)
    pub v_tuning_segm: Option<&'static [i32]>,
    /// `v_tuningFreq`: frequency resolutions to use around transient (never set: `NULL`)
    pub v_tuning_freq: Option<&'static [i32]>,
    /// `dmin`: minimum length of dependent segments
    pub dmin: i32,
    /// `dmax`: maximum length of dependent segments
    pub dmax: i32,
    /// `allowSpread`: 1: allow isolated transient to influence grid of 3 consecutive frames
    pub allow_spread: i32,

    // internally used signals
    /// `frameClassOld`: frame class used for previous frame
    pub frame_class_old: FrameClass,
    /// `spreadFlag`: 1: use VARVAR instead of VARFIX to follow up old transient
    pub spread_flag: i32,

    /// `v_bord`: borders for current frame and preliminary borders for next frame (fixed
    /// borders excluded)
    pub v_bord: [i32; 2 * MAX_ENVELOPES_VARVAR + 1],
    /// `length_v_bord`: helper variable: length of v_bord
    pub length_v_bord: i32,
    /// `v_freq`: frequency resolutions for current frame and preliminary resolutions for next
    /// frame
    pub v_freq: [i32; 2 * MAX_ENVELOPES_VARVAR + 1],
    /// `length_v_freq`: helper variable: length of v_freq
    pub length_v_freq: i32,

    /// `v_bordFollow`: preliminary borders for current frame (calculated during previous frame)
    pub v_bord_follow: [i32; MAX_ENVELOPES_VARVAR],
    /// `length_v_bordFollow`: helper variable: length of v_bordFollow
    pub length_v_bord_follow: i32,
    /// `i_tranFollow`: points to transient border in v_bordFollow (may be negative, see
    /// keepForFollowUp())
    pub i_tran_follow: i32,
    /// `i_fillFollow`: points to first fill border in v_bordFollow
    pub i_fill_follow: i32,
    /// `v_freqFollow`: preliminary frequency resolutions for current frame (calculated during
    /// previous frame)
    pub v_freq_follow: [i32; MAX_ENVELOPES_VARVAR],
    /// `length_v_freqFollow`: helper variable: length of v_freqFollow
    pub length_v_freq_follow: i32,

    // externally needed signals
    /// `SbrGrid`: sbr_grid() signals to be converted to bitstream elements
    pub sbr_grid: SbrGrid,
    /// `SbrFrameInfo`: time/frequency grid description for one frame
    pub sbr_frame_info: SbrFrameInfo,
}

/// `FDKsbrEnc_frameInfoGenerator(hSbrEnvFrame, v_transient_info, rightBorderFIX,
/// v_transient_info_pre, ldGrid, v_tuning)`: produces the frame info of the current frame in
/// `h_sbr_env_frame.sbr_frame_info` (the C function returns a pointer to it).
pub fn fdk_sbr_enc_frame_info_generator(
    h_sbr_env_frame: &mut SbrEnvelopeFrame,
    v_transient_info: &[u8],
    right_border_fix: i32,
    v_transient_info_pre: &[u8],
    ld_grid: i32,
    v_tuning: &'static [i32],
) {
    let num_env;
    let mut tran_pos_internal = 0;
    let mut bmin = 0;
    let mut bmax = 0;
    let mut parts = 0;
    let mut d = 0;
    let mut i_cmon = 0;
    let mut i_tran = 0;
    let mut n_l = 0;
    let mut fmax = 0;

    let h = h_sbr_env_frame;
    let mut frame_class = FIXFIX;

    let allow_spread = h.allow_spread;
    let num_env_static = h.num_env_static;
    let static_framing = h.static_framing;
    let dmin = h.dmin;
    let dmax = h.dmax;

    let buffer_frame_start = h.sbr_grid.buffer_frame_start;
    let number_time_slots = h.sbr_grid.number_time_slots;
    let frame_middle_slot = h.frame_middle_slot;

    let mut tran_pos = v_transient_info[0] as i32;
    let mut tran_flag = v_transient_info[1] as i32;

    let v_tuning_segm = v_tuning;
    let v_tuning_freq = &v_tuning[3..];

    h.v_tuning_segm = Some(v_tuning_segm);

    if ld_grid != 0 {
        // in case there was a transient at the very end of the previous frame, start with a
        // transient envelope
        if tran_flag == 0
            && v_transient_info_pre[1] != 0
            && (number_time_slots - v_transient_info_pre[0] as i32) < MIN_FRAME_TRAN_DISTANCE
        {
            tran_flag = 1;
            tran_pos = 0;
        }
    }

    // Synopsis: see fram_gen.cpp. The frame generator creates the time-/frequency-grid for one
    // SBR frame from the transient detector and frame splitter signals. When no transients are
    // present, FIXFIX class frames are used; sparse transients are handled by [FIXVAR, VARFIX]
    // pairs or [FIXVAR, VARVAR, VARFIX] triples, tight transients by VARVAR sequences.

    if static_framing != 0 {
        // Ignore transient detector
        frame_class = FIXFIX;
        num_env = num_env_static; // {1,2,4,8}
        h.frame_class_old = FIXFIX; // for change to dyn
        h.sbr_grid.bs_num_env = num_env;
        h.sbr_grid.frame_class = frame_class;
    } else {
        // Calculate frame class to use
        if right_border_fix != 0 {
            tran_flag = 0;
            h.spread_flag = 0;
        }
        calc_frame_class(&mut frame_class, &mut h.frame_class_old, tran_flag, &mut h.spread_flag);

        // patch for new frame class FIXFIXonly for AAC LD
        if tran_flag != 0 && ld_grid != 0 {
            frame_class = FIXFIX_ONLY;
            h.frame_class_old = FIXFIX;
        }

        // every transient is processed below by inserting one border at the onset of the
        // transient, one or more "decay borders" and optionally one "attack border" (the
        // "mandatory borders", defined by the segment lengths of the tuning vector)

        // Design frame (or follow-up old design)
        if tran_flag != 0 {
            // Always for FixVar, often but not always for VarVar

            // Design part of T/F-grid around the new transient
            tran_pos_internal = frame_middle_slot + tran_pos + buffer_frame_start; // FH 00-06-26

            // add mandatory borders around transient
            fill_frame_tran(
                v_tuning_segm,
                v_tuning_freq,
                tran_pos_internal,
                &mut h.v_bord,
                &mut h.length_v_bord,
                &mut h.v_freq,
                &mut h.length_v_freq,
                &mut bmin,
                &mut bmax,
            );

            // make sure we stay within the maximum SBR frame overlap
            fmax = calc_fill_length_max(tran_pos, number_time_slots);
        }

        match frame_class {
            FIXFIX_ONLY => {
                fdk_assert!(ld_grid != 0);
                tran_pos_internal = tran_pos;
                generate_fix_fix_only(
                    &mut h.sbr_frame_info,
                    &mut h.sbr_grid,
                    tran_pos_internal,
                    number_time_slots,
                    h.f_res_trans_is_low,
                );

                return;
            }
            FIXVAR => {
                // Design remaining parts of T/F-grid (assuming next frame is VarFix)

                // Fill region before new transient:
                fill_frame_pre(
                    dmax,
                    &mut h.v_bord,
                    &mut h.length_v_bord,
                    &mut h.v_freq,
                    &mut h.length_v_freq,
                    bmin,
                    bmin - buffer_frame_start,
                ); // FH 00-06-26

                // Fill region after new transient:
                fill_frame_post(
                    &mut parts,
                    &mut d,
                    dmax,
                    &mut h.v_bord,
                    &mut h.length_v_bord,
                    &mut h.v_freq,
                    &mut h.length_v_freq,
                    bmax,
                    buffer_frame_start,
                    number_time_slots,
                    fmax,
                );

                // Take care of special case:
                if parts == 1 && d < dmin {
                    // no fill, short last envelope
                    special_case(
                        &mut h.spread_flag,
                        allow_spread,
                        &mut h.v_bord,
                        &mut h.length_v_bord,
                        &mut h.v_freq,
                        &mut h.length_v_freq,
                        &mut parts,
                        d,
                    );
                }

                // Calculate common border (split-point)
                calc_cmon_border(
                    &mut i_cmon,
                    &mut i_tran,
                    &h.v_bord,
                    h.length_v_bord,
                    tran_pos_internal,
                    buffer_frame_start,
                    number_time_slots,
                ); // FH 00-06-26

                // Extract data for proper follow-up in next frame
                keep_for_follow_up(
                    &mut h.v_bord_follow,
                    &mut h.length_v_bord_follow,
                    &mut h.v_freq_follow,
                    &mut h.length_v_freq_follow,
                    &mut h.i_tran_follow,
                    &mut h.i_fill_follow,
                    &h.v_bord,
                    h.length_v_bord,
                    &h.v_freq,
                    i_cmon,
                    i_tran,
                    parts,
                    number_time_slots,
                ); // FH 00-06-26

                // Calculate control signal
                calc_ctrl_signal(
                    &mut h.sbr_grid,
                    frame_class,
                    &h.v_bord,
                    h.length_v_bord,
                    &h.v_freq,
                    h.length_v_freq,
                    i_cmon,
                    i_tran,
                    h.spread_flag,
                    DC,
                );
            }
            VARFIX => {
                // Follow-up old transient - calculate control signal
                calc_ctrl_signal(
                    &mut h.sbr_grid,
                    frame_class,
                    &h.v_bord_follow,
                    h.length_v_bord_follow,
                    &h.v_freq_follow,
                    h.length_v_freq_follow,
                    DC,
                    h.i_tran_follow,
                    h.spread_flag,
                    DC,
                );
            }
            VARVAR => {
                if h.spread_flag != 0 {
                    // spread across three frames
                    // Follow-up old transient - calculate control signal
                    calc_ctrl_signal(
                        &mut h.sbr_grid,
                        frame_class,
                        &h.v_bord_follow,
                        h.length_v_bord_follow,
                        &h.v_freq_follow,
                        h.length_v_freq_follow,
                        DC,
                        h.i_tran_follow,
                        h.spread_flag,
                        DC,
                    );

                    h.spread_flag = 0;

                    // Extract data for proper follow-up in next frame
                    h.v_bord_follow[0] = h.sbr_grid.bs_abs_bord_1 - number_time_slots; // FH 00-06-26
                    h.v_freq_follow[0] = 1;
                    h.length_v_bord_follow = 1;
                    h.length_v_freq_follow = 1;

                    h.i_tran_follow = -DC;
                    h.i_fill_follow = -DC;
                } else {
                    // Design remaining parts of T/F-grid (assuming next frame is VarFix)
                    // adapt or fill region before new transient:
                    fill_frame_inter(
                        &mut n_l,
                        v_tuning_segm,
                        &mut h.v_bord,
                        &mut h.length_v_bord,
                        bmin,
                        &mut h.v_freq,
                        &mut h.length_v_freq,
                        &mut h.v_bord_follow,
                        &mut h.length_v_bord_follow,
                        &mut h.v_freq_follow,
                        &mut h.length_v_freq_follow,
                        h.i_fill_follow,
                        dmin,
                        dmax,
                        number_time_slots,
                    );

                    // Fill after transient:
                    fill_frame_post(
                        &mut parts,
                        &mut d,
                        dmax,
                        &mut h.v_bord,
                        &mut h.length_v_bord,
                        &mut h.v_freq,
                        &mut h.length_v_freq,
                        bmax,
                        buffer_frame_start,
                        number_time_slots,
                        fmax,
                    );

                    // Take care of special case:
                    if parts == 1 && d < dmin {
                        // no fill, short last envelope
                        special_case(
                            &mut h.spread_flag,
                            allow_spread,
                            &mut h.v_bord,
                            &mut h.length_v_bord,
                            &mut h.v_freq,
                            &mut h.length_v_freq,
                            &mut parts,
                            d,
                        );
                    }

                    // Calculate common border (split-point)
                    calc_cmon_border(
                        &mut i_cmon,
                        &mut i_tran,
                        &h.v_bord,
                        h.length_v_bord,
                        tran_pos_internal,
                        buffer_frame_start,
                        number_time_slots,
                    );

                    // Extract data for proper follow-up in next frame
                    keep_for_follow_up(
                        &mut h.v_bord_follow,
                        &mut h.length_v_bord_follow,
                        &mut h.v_freq_follow,
                        &mut h.length_v_freq_follow,
                        &mut h.i_tran_follow,
                        &mut h.i_fill_follow,
                        &h.v_bord,
                        h.length_v_bord,
                        &h.v_freq,
                        i_cmon,
                        i_tran,
                        parts,
                        number_time_slots,
                    );

                    // Calculate control signal
                    calc_ctrl_signal(
                        &mut h.sbr_grid,
                        frame_class,
                        &h.v_bord,
                        h.length_v_bord,
                        &h.v_freq,
                        h.length_v_freq,
                        i_cmon,
                        i_tran,
                        0,
                        n_l,
                    );
                }
            }
            FIXFIX => {
                if tran_pos == 0 {
                    num_env = 1;
                } else {
                    num_env = 2;
                }

                h.sbr_grid.bs_num_env = num_env;
                h.sbr_grid.frame_class = frame_class;
            }
            _ => {
                fdk_assert!(false);
            }
        }
    }

    // Convert control signal to frame info struct
    ctrl_signal2_frame_info(&mut h.sbr_grid, &mut h.sbr_frame_info, &h.freq_res_fixfix);
}

/// `generateFixFixOnly(hSbrFrameInfo, hSbrGrid, tranPosInternal, numberTimeSlots,
/// fResTransIsLow)`: the frame info of a FIXFIXonly frame (low delay).
///
/// Deviation: for a number of time slots other than 8, 15 or 16, or a transient position outside
/// of the table, the reference has no defined result; the port leaves the frame info unchanged.
fn generate_fix_fix_only(
    h_sbr_frame_info: &mut SbrFrameInfo,
    h_sbr_grid: &mut SbrGrid,
    tran_pos_internal: i32,
    number_time_slots: i32,
    f_res_trans_is_low: u8,
) {
    let t = tran_pos_internal as usize;
    let (p_table, freq_res_table): (&[i32], &[FreqRes]) = match number_time_slots {
        8 => match ENVELOPE_TABLE_8.get(t) {
            Some(r) => (r, &FREQ_RES_TABLE_8),
            None => return,
        },
        15 => match ENVELOPE_TABLE_15.get(t) {
            Some(r) => (r, &FREQ_RES_TABLE_16),
            None => return,
        },
        16 => match ENVELOPE_TABLE_16.get(t) {
            Some(r) => (r, &FREQ_RES_TABLE_16),
            None => return,
        },
        _ => return,
    };

    // look number of envolpes in table
    let n_env = p_table[0];
    // look up envolpe distribution in table
    h_sbr_frame_info.borders[1..(n_env as usize)]
        .copy_from_slice(&p_table[(1 + 2)..(n_env as usize + 2)]);
    // open and close frame border
    h_sbr_frame_info.borders[0] = 0;
    h_sbr_frame_info.borders[n_env as usize] = number_time_slots;

    // adjust segment-frequency-resolution according to the segment-length
    for i in 0..n_env as usize {
        let k = (h_sbr_frame_info.borders[i + 1] - h_sbr_frame_info.borders[i]) as usize;
        if f_res_trans_is_low == 0 {
            h_sbr_frame_info.freq_res[i] = freq_res_table[k];
        } else {
            h_sbr_frame_info.freq_res[i] = FREQ_RES_LOW;
        }
        h_sbr_grid.v_f[i] = h_sbr_frame_info.freq_res[i];
    }

    h_sbr_frame_info.n_envelopes = n_env;
    h_sbr_frame_info.short_env = p_table[2];
    // transient idx
    let tran_idx = p_table[1];

    // add noise floors
    h_sbr_frame_info.borders_noise[0] = 0;
    h_sbr_frame_info.borders_noise[1] =
        h_sbr_frame_info.borders[if tran_idx != 0 { tran_idx as usize } else { 1 }];
    h_sbr_frame_info.borders_noise[2] = number_time_slots;
    h_sbr_frame_info.n_noise_envelopes = 2;

    h_sbr_grid.frame_class = FIXFIX_ONLY;
    h_sbr_grid.bs_abs_bord = tran_pos_internal;
    h_sbr_grid.bs_num_env = n_env;
}

/// `FDKsbrEnc_initFrameInfoGenerator(hSbrEnvFrame, allowSpread, numEnvStatic, staticFraming,
/// timeSlots, freq_res_fixfix, fResTransIsLow, ldGrid)`
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_init_frame_info_generator(
    h_sbr_env_frame: &mut SbrEnvelopeFrame,
    allow_spread: i32,
    num_env_static: i32,
    static_framing: i32,
    time_slots: i32,
    freq_res_fixfix: &[FreqRes; 2],
    f_res_trans_is_low: u8,
    ld_grid: i32,
) {
    // FH 00-06-26
    *h_sbr_env_frame = SbrEnvelopeFrame::default();
    let h = h_sbr_env_frame;

    // Initialisation
    h.frame_class_old = FIXFIX;
    h.spread_flag = 0;

    h.allow_spread = allow_spread;
    h.num_env_static = num_env_static;
    h.static_framing = static_framing;
    h.freq_res_fixfix[0] = freq_res_fixfix[0];
    h.freq_res_fixfix[1] = freq_res_fixfix[1];
    h.f_res_trans_is_low = f_res_trans_is_low;

    h.length_v_bord = 0;
    h.length_v_bord_follow = 0;

    h.length_v_freq = 0;
    h.length_v_freq_follow = 0;

    h.i_tran_follow = 0;
    h.i_fill_follow = 0;

    h.sbr_grid.number_time_slots = time_slots;

    if ld_grid != 0 {
        // case CODEC_AACLD:
        h.dmin = 2;
        h.dmax = 16;
        h.frame_middle_slot = FRAME_MIDDLE_SLOT_512LD;
        h.sbr_grid.buffer_frame_start = 0;
    } else {
        match time_slots {
            NUMBER_TIME_SLOTS_1920 => {
                h.dmin = 4;
                h.dmax = 12;
                h.sbr_grid.buffer_frame_start = 0;
                h.frame_middle_slot = FRAME_MIDDLE_SLOT_1920;
            }
            NUMBER_TIME_SLOTS_2048 => {
                h.dmin = 4;
                h.dmax = 12;
                h.sbr_grid.buffer_frame_start = 0;
                h.frame_middle_slot = FRAME_MIDDLE_SLOT_2048;
            }
            NUMBER_TIME_SLOTS_1152 => {
                h.dmin = 2;
                h.dmax = 8;
                h.sbr_grid.buffer_frame_start = 0;
                h.frame_middle_slot = FRAME_MIDDLE_SLOT_1152;
            }
            NUMBER_TIME_SLOTS_2304 => {
                h.dmin = 4;
                h.dmax = 15;
                h.sbr_grid.buffer_frame_start = 0;
                h.frame_middle_slot = FRAME_MIDDLE_SLOT_2304;
            }
            _ => {
                fdk_assert!(false);
            }
        }
    }
}

/// `fillFrameTran(v_tuningSegm, v_tuningFreq, tran, v_bord, length_v_bord, v_freq,
/// length_v_freq, bmin, bmax)`: adds the mandatory borders, as described by the tuning vector
/// and the current transient position.
#[allow(clippy::too_many_arguments)]
fn fill_frame_tran(
    v_tuning_segm: &[i32],
    v_tuning_freq: &[i32],
    tran: i32,
    v_bord: &mut [i32],
    length_v_bord: &mut i32,
    v_freq: &mut [i32],
    length_v_freq: &mut i32,
    bmin: &mut i32,
    bmax: &mut i32,
) {
    *length_v_bord = 0;
    *length_v_freq = 0;

    // add attack env leading border (optional)
    if v_tuning_segm[0] != 0 {
        // v_bord = [(Ba)] start of attack env
        fdk_sbr_enc_add_right(v_bord, length_v_bord, tran - v_tuning_segm[0]);

        // v_freq = [(Fa)] res of attack env
        fdk_sbr_enc_add_right(v_freq, length_v_freq, v_tuning_freq[0]);
    }

    // add attack env trailing border/first decay env leading border
    let mut bord = tran;
    fdk_sbr_enc_add_right(v_bord, length_v_bord, tran); // v_bord = [(Ba),Bd1]

    // add first decay env trailing border/2:nd decay env leading border
    if v_tuning_segm[1] != 0 {
        bord += v_tuning_segm[1];

        // v_bord = [(Ba),Bd1,Bd2]
        fdk_sbr_enc_add_right(v_bord, length_v_bord, bord);

        // v_freq = [(Fa),Fd1]
        fdk_sbr_enc_add_right(v_freq, length_v_freq, v_tuning_freq[1]);
    }

    // add 2:nd decay env trailing border (optional)
    if v_tuning_segm[2] != 0 {
        bord += v_tuning_segm[2];

        // v_bord = [(Ba),Bd1, Bd2,(Bd3)]
        fdk_sbr_enc_add_right(v_bord, length_v_bord, bord);

        // v_freq = [(Fa),Fd1,(Fd2)]
        fdk_sbr_enc_add_right(v_freq, length_v_freq, v_tuning_freq[2]);
    }

    //  v_freq = [(Fa),Fd1,(Fd2),1]
    fdk_sbr_enc_add_right(v_freq, length_v_freq, 1);

    //  calc min and max values of mandatory borders
    *bmin = v_bord[0];
    for &b in v_bord.iter().take(*length_v_bord as usize) {
        if b < *bmin {
            *bmin = b;
        }
    }

    *bmax = v_bord[0];
    for &b in v_bord.iter().take(*length_v_bord as usize) {
        if b > *bmax {
            *bmax = b;
        }
    }
}

/// `fillFramePre(dmax, v_bord, length_v_bord, v_freq, length_v_freq, bmin, rest)`: adds borders
/// before the mandatory borders, if needed.
fn fill_frame_pre(
    dmax: i32,
    v_bord: &mut [i32],
    length_v_bord: &mut i32,
    v_freq: &mut [i32],
    length_v_freq: &mut i32,
    bmin: i32,
    rest: i32,
) {
    // input state:
    // v_bord = [(Ba),Bd1, Bd2 ,(Bd3)]
    // v_freq = [(Fa),Fd1,(Fd2),1 ]
    let mut s = 0;

    // start with one envelope
    let mut parts = 1;
    let mut d = rest;

    // calc # of additional envelopes and corresponding lengths
    while d > dmax {
        parts += 1;

        let segm = rest / parts;
        let big_s = (segm - 2) >> 1;
        s = (2 * big_s + 2).min(8);
        d = rest - (parts - 1) * s;
    }

    // add borders before mandatory borders
    let mut bord = bmin;

    for _ in 0..=(parts - 2) {
        bord -= s;

        // v_bord = [...,(Bf),(Ba),Bd1, Bd2 ,(Bd3)]
        fdk_sbr_enc_add_left(v_bord, length_v_bord, bord);

        // v_freq = [...,(1 ),(Fa),Fd1,(Fd2), 1   ]
        fdk_sbr_enc_add_left(v_freq, length_v_freq, 1);
    }
}

/// `calcFillLengthMax(tranPos, numberTimeSlots)`: overlap control; the maximum length of
/// trailing fill segments, such that there is always a border within the frame overlap region.
fn calc_fill_length_max(tran_pos: i32, number_time_slots: i32) -> i32 {
    // calculate transient position within envelope buffer
    match number_time_slots {
        NUMBER_TIME_SLOTS_2048 => {
            if tran_pos < 4 {
                6
            } else if tran_pos == 4 || tran_pos == 5 {
                4
            } else {
                8
            }
        }
        NUMBER_TIME_SLOTS_1920 => {
            if tran_pos < 4 {
                5
            } else if tran_pos == 4 || tran_pos == 5 {
                3
            } else {
                7
            }
        }
        _ => 8,
    }
}

/// `fillFramePost(parts, d, dmax, v_bord, length_v_bord, v_freq, length_v_freq, bmax,
/// bufferFrameStart, numberTimeSlots, fmax)`: adds borders after the mandatory borders, if
/// needed (a preliminary design of the next frame, assuming no transient is present there).
#[allow(clippy::too_many_arguments)]
fn fill_frame_post(
    parts: &mut i32,
    d: &mut i32,
    dmax: i32,
    v_bord: &mut [i32],
    length_v_bord: &mut i32,
    v_freq: &mut [i32],
    length_v_freq: &mut i32,
    bmax: i32,
    buffer_frame_start: i32,
    number_time_slots: i32,
    fmax: i32,
) {
    let mut s = 0;

    // input state:
    // v_bord = [...,(Bf),(Ba),Bd1, Bd2 ,(Bd3)]
    // v_freq = [...,(1 ),(Fa),Fd1,(Fd2),1    ]

    let rest = buffer_frame_start + 2 * number_time_slots - bmax;
    *d = rest;

    if *d > 0 {
        *parts = 1; // start with one envelope

        // calc # of additional envelopes and corresponding lengths
        while *d > dmax {
            *parts += 1;

            let segm = rest / *parts;
            let big_s = (segm - 2) >> 1;
            s = fmax.min(2 * big_s + 2);
            *d = rest - (*parts - 1) * s;
        }

        // add borders after mandatory borders
        let mut bord = bmax;
        for _ in 0..=(*parts - 2) {
            bord += s;

            // v_bord =  [...,(Bf),(Ba),Bd1, Bd2 ,(Bd3),(Bf)]
            fdk_sbr_enc_add_right(v_bord, length_v_bord, bord);

            // v_freq =  [...,(1 ),(Fa),Fd1,(Fd2), 1   , 1! ,1]
            fdk_sbr_enc_add_right(v_freq, length_v_freq, 1);
        }
    } else {
        *parts = 1;

        // remove last element from v_bord and v_freq
        *length_v_bord -= 1;
        *length_v_freq -= 1;
    }
}

/// `fillFrameInter(nL, v_tuningSegm, v_bord, length_v_bord, bmin, v_freq, length_v_freq,
/// v_bordFollow, length_v_bordFollow, v_freqFollow, length_v_freqFollow, i_fillFollow, dmin,
/// dmax, numberTimeSlots)`
#[allow(clippy::too_many_arguments)]
fn fill_frame_inter(
    n_l: &mut i32,
    v_tuning_segm: &[i32],
    v_bord: &mut [i32],
    length_v_bord: &mut i32,
    mut bmin: i32,
    v_freq: &mut [i32],
    length_v_freq: &mut i32,
    v_bord_follow: &mut [i32],
    length_v_bord_follow: &mut i32,
    v_freq_follow: &mut [i32],
    length_v_freq_follow: &mut i32,
    i_fill_follow: i32,
    dmin: i32,
    dmax: i32,
    number_time_slots: i32,
) {
    let mut middle;
    let b_new;
    let mut num_bord_follow;
    let mut bord_max_follow;

    if number_time_slots != NUMBER_TIME_SLOTS_1152 {
        // remove fill borders:
        if i_fill_follow >= 1 {
            *length_v_bord_follow = i_fill_follow;
            *length_v_freq_follow = i_fill_follow;
        }

        num_bord_follow = *length_v_bord_follow;
        bord_max_follow = v_bord_follow[(num_bord_follow - 1) as usize];

        // remove even more borders if needed
        middle = bmin - bord_max_follow;
        while middle < 0 {
            num_bord_follow -= 1;
            bord_max_follow = v_bord_follow[(num_bord_follow - 1) as usize];
            middle = bmin - bord_max_follow;
        }

        *length_v_bord_follow = num_bord_follow;
        *length_v_freq_follow = num_bord_follow;

        *n_l = num_bord_follow - 1;

        b_new = *length_v_bord;

        if middle <= dmax {
            if middle >= dmin {
                // concatenate
                fdk_sbr_enc_add_vec_left(
                    v_bord,
                    length_v_bord,
                    v_bord_follow,
                    *length_v_bord_follow,
                );
                fdk_sbr_enc_add_vec_left(
                    v_freq,
                    length_v_freq,
                    v_freq_follow,
                    *length_v_freq_follow,
                );
            } else if v_tuning_segm[0] != 0 {
                // remove one new border and concatenate
                *length_v_bord = b_new - 1;
                fdk_sbr_enc_add_vec_left(
                    v_bord,
                    length_v_bord,
                    v_bord_follow,
                    *length_v_bord_follow,
                );

                *length_v_freq = b_new - 1;
                fdk_sbr_enc_add_vec_left(
                    &mut v_freq[1..],
                    length_v_freq,
                    v_freq_follow,
                    *length_v_freq_follow,
                );
            } else if *length_v_bord_follow > 1 {
                // remove one old border and concatenate
                fdk_sbr_enc_add_vec_left(
                    v_bord,
                    length_v_bord,
                    v_bord_follow,
                    *length_v_bord_follow - 1,
                );
                fdk_sbr_enc_add_vec_left(
                    v_freq,
                    length_v_freq,
                    v_freq_follow,
                    *length_v_bord_follow - 1,
                );

                *n_l -= 1;
            } else {
                // remove new "transient" border and concatenate
                for i in 0..(*length_v_bord - 1).max(0) as usize {
                    v_bord[i] = v_bord[i + 1];
                }

                for i in 0..(*length_v_freq - 1).max(0) as usize {
                    v_freq[i] = v_freq[i + 1];
                }

                *length_v_bord = b_new - 1;
                *length_v_freq = b_new - 1;

                fdk_sbr_enc_add_vec_left(
                    v_bord,
                    length_v_bord,
                    v_bord_follow,
                    *length_v_bord_follow,
                );
                fdk_sbr_enc_add_vec_left(
                    v_freq,
                    length_v_freq,
                    v_freq_follow,
                    *length_v_freq_follow,
                );
            }
        } else {
            // middle > dmax
            fill_frame_pre(dmax, v_bord, length_v_bord, v_freq, length_v_freq, bmin, middle);
            fdk_sbr_enc_add_vec_left(v_bord, length_v_bord, v_bord_follow, *length_v_bord_follow);
            fdk_sbr_enc_add_vec_left(v_freq, length_v_freq, v_freq_follow, *length_v_freq_follow);
        }
    } else {
        // numberTimeSlots==NUMBER_TIME_SLOTS_1152

        // remove fill borders
        if i_fill_follow >= 1 {
            *length_v_bord_follow = i_fill_follow;
            *length_v_freq_follow = i_fill_follow;
        }

        num_bord_follow = *length_v_bord_follow;
        bord_max_follow = v_bord_follow[(num_bord_follow - 1) as usize];

        // remove more borders if necessary to eliminate overlap

        // check for overlap
        middle = bmin - bord_max_follow;

        // intervals:
        // i)             middle <  0     : overlap, must remove borders
        // ii)       0 <= middle <  dmin  : no overlap but too tight, must remove borders
        // iii)   dmin <= middle <= dmax  : ok, just concatenate
        // iv)    dmax <= middle          : too wide, must add borders

        // first remove old non-fill-borders...
        while middle < 0 {
            // ...but don't remove all of them
            if num_bord_follow == 1 {
                break;
            }

            num_bord_follow -= 1;
            bord_max_follow = v_bord_follow[(num_bord_follow - 1) as usize];
            middle = bmin - bord_max_follow;
        }

        // if this isn't enough, remove new non-fill borders
        if middle < 0 {
            let mut m = 0usize;
            let mut l = 0;
            while l < *length_v_bord {
                if v_bord[l as usize] > bord_max_follow {
                    v_bord[m] = v_bord[l as usize];
                    v_freq[m] = v_freq[l as usize];
                    m += 1;
                }
                l += 1;
            }

            *length_v_bord = l;
            *length_v_freq = l;

            bmin = v_bord[0];
        }

        // update modified follow-up data
        *length_v_bord_follow = num_bord_follow;
        *length_v_freq_follow = num_bord_follow;

        // left relative borders correspond to follow-up
        *n_l = num_bord_follow - 1;

        // take care of intervals ii through iv

        // now middle should be >= 0
        middle = bmin - bord_max_follow;

        if middle <= dmin {
            // (ii)
            let b_new = *length_v_bord;

            if v_tuning_segm[0] != 0 {
                // remove new "luxury" border and concatenate
                *length_v_bord = b_new - 1;
                fdk_sbr_enc_add_vec_left(
                    v_bord,
                    length_v_bord,
                    v_bord_follow,
                    *length_v_bord_follow,
                );

                *length_v_freq = b_new - 1;
                fdk_sbr_enc_add_vec_left(
                    &mut v_freq[1..],
                    length_v_freq,
                    v_freq_follow,
                    *length_v_freq_follow,
                );
            } else if *length_v_bord_follow > 1 {
                // remove old border and concatenate
                fdk_sbr_enc_add_vec_left(
                    v_bord,
                    length_v_bord,
                    v_bord_follow,
                    *length_v_bord_follow - 1,
                );
                fdk_sbr_enc_add_vec_left(
                    v_freq,
                    length_v_freq,
                    v_freq_follow,
                    *length_v_bord_follow - 1,
                );

                *n_l -= 1;
            } else {
                // remove new border and concatenate
                for i in 0..(*length_v_bord - 1).max(0) as usize {
                    v_bord[i] = v_bord[i + 1];
                }

                for i in 0..(*length_v_freq - 1).max(0) as usize {
                    v_freq[i] = v_freq[i + 1];
                }

                *length_v_bord = b_new - 1;
                *length_v_freq = b_new - 1;

                fdk_sbr_enc_add_vec_left(
                    v_bord,
                    length_v_bord,
                    v_bord_follow,
                    *length_v_bord_follow,
                );
                fdk_sbr_enc_add_vec_left(
                    v_freq,
                    length_v_freq,
                    v_freq_follow,
                    *length_v_freq_follow,
                );
            }
        } else if middle >= dmin && middle <= dmax {
            // (iii)
            // concatenate
            fdk_sbr_enc_add_vec_left(v_bord, length_v_bord, v_bord_follow, *length_v_bord_follow);
            fdk_sbr_enc_add_vec_left(v_freq, length_v_freq, v_freq_follow, *length_v_freq_follow);
        } else {
            // (iv)
            fill_frame_pre(dmax, v_bord, length_v_bord, v_freq, length_v_freq, bmin, middle);
            fdk_sbr_enc_add_vec_left(v_bord, length_v_bord, v_bord_follow, *length_v_bord_follow);
            fdk_sbr_enc_add_vec_left(v_freq, length_v_freq, v_freq_follow, *length_v_freq_follow);
        }
    }
}

/// `calcFrameClass(frameClass, frameClassOld, tranFlag, spreadFlag)`
fn calc_frame_class(
    frame_class: &mut FrameClass,
    frame_class_old: &mut FrameClass,
    tran_flag: i32,
    spread_flag: &mut i32,
) {
    match *frame_class_old {
        FIXFIX_ONLY | FIXFIX => {
            if tran_flag != 0 {
                *frame_class = FIXVAR;
            } else {
                *frame_class = FIXFIX;
            }
        }
        FIXVAR => {
            if tran_flag != 0 {
                *frame_class = VARVAR;
                *spread_flag = 0;
            } else if *spread_flag != 0 {
                *frame_class = VARVAR;
            } else {
                *frame_class = VARFIX;
            }
        }
        VARFIX => {
            if tran_flag != 0 {
                *frame_class = FIXVAR;
            } else {
                *frame_class = FIXFIX;
            }
        }
        VARVAR => {
            if tran_flag != 0 {
                *frame_class = VARVAR;
                *spread_flag = 0;
            } else if *spread_flag != 0 {
                *frame_class = VARVAR;
            } else {
                *frame_class = VARFIX;
            }
        }
        _ => {}
    }

    *frame_class_old = *frame_class;
}

/// `specialCase(spreadFlag, allowSpread, v_bord, length_v_bord, v_freq, length_v_freq, parts,
/// d)`
#[allow(clippy::too_many_arguments)]
fn special_case(
    spread_flag: &mut i32,
    allow_spread: i32,
    v_bord: &mut [i32],
    length_v_bord: &mut i32,
    v_freq: &mut [i32],
    length_v_freq: &mut i32,
    parts: &mut i32,
    d: i32,
) {
    let l = *length_v_bord;

    if allow_spread != 0 {
        // add one "step 8"
        *spread_flag = 1;
        let b = v_bord[(l - 1) as usize] + 8;
        fdk_sbr_enc_add_right(v_bord, length_v_bord, b);
        fdk_sbr_enc_add_right(v_freq, length_v_freq, 1);
        *parts += 1;
    } else if d == 1 {
        // stretch one slot
        *length_v_bord = l - 1;
        *length_v_freq = l - 1;
    } else if (v_bord[(l - 1) as usize] - v_bord[(l - 2) as usize]) > 2 {
        // compress one quant step
        v_bord[(l - 1) as usize] -= 2;
        v_freq[(*length_v_freq - 1) as usize] = 0; // use low res for short segment
    }
}

/// `calcCmonBorder(i_cmon, i_tran, v_bord, length_v_bord, tran, bufferFrameStart,
/// numberTimeSlots)`
fn calc_cmon_border(
    i_cmon: &mut i32,
    i_tran: &mut i32,
    v_bord: &[i32],
    length_v_bord: i32,
    tran: i32,
    buffer_frame_start: i32,
    number_time_slots: i32,
) {
    // FH 00-06-26
    for i in 0..length_v_bord.max(0) as usize {
        if v_bord[i] >= buffer_frame_start + number_time_slots {
            // FH 00-06-26
            *i_cmon = i as i32;
            break;
        }
    }

    // keep track of transient:
    for i in 0..length_v_bord.max(0) as usize {
        if v_bord[i] >= tran {
            *i_tran = i as i32;
            break;
        } else {
            *i_tran = EMPTY;
        }
    }
}

/// `keepForFollowUp(v_bordFollow, length_v_bordFollow, v_freqFollow, length_v_freqFollow,
/// i_tranFollow, i_fillFollow, v_bord, length_v_bord, v_freq, i_cmon, i_tran, parts,
/// numberTimeSlots)`
#[allow(clippy::too_many_arguments)]
fn keep_for_follow_up(
    v_bord_follow: &mut [i32],
    length_v_bord_follow: &mut i32,
    v_freq_follow: &mut [i32],
    length_v_freq_follow: &mut i32,
    i_tran_follow: &mut i32,
    i_fill_follow: &mut i32,
    v_bord: &[i32],
    length_v_bord: i32,
    v_freq: &[i32],
    i_cmon: i32,
    i_tran: i32,
    parts: i32,
    number_time_slots: i32,
) {
    // FH 00-06-26
    let l = length_v_bord;

    *length_v_bord_follow = 0;
    *length_v_freq_follow = 0;

    let mut j = 0usize;
    let mut i = i_cmon;
    while i < l {
        v_bord_follow[j] = v_bord[i as usize] - number_time_slots; // FH 00-06-26
        v_freq_follow[j] = v_freq[i as usize];

        *length_v_bord_follow += 1;
        *length_v_freq_follow += 1;
        i += 1;
        j += 1;
    }
    if i_tran != EMPTY {
        *i_tran_follow = i_tran - i_cmon;
    } else {
        *i_tran_follow = EMPTY;
    }
    *i_fill_follow = l - (parts - 1) - i_cmon;
}

/// `calcCtrlSignal(hSbrGrid, frameClass, v_bord, length_v_bord, v_freq, length_v_freq, i_cmon,
/// i_tran, spreadFlag, nL)`
#[allow(clippy::too_many_arguments)]
fn calc_ctrl_signal(
    h_sbr_grid: &mut SbrGrid,
    frame_class: FrameClass,
    v_bord: &[i32],
    mut length_v_bord: i32,
    v_freq: &[i32],
    length_v_freq: i32,
    i_cmon: i32,
    i_tran: i32,
    spread_flag: i32,
    mut n_l: i32,
) {
    let mut length_v_r = 0;
    let mut length_v_r_r = 0;
    let mut length_v_r_l = 0;

    match frame_class {
        FIXVAR => {
            // absolute border:
            let a = v_bord[i_cmon as usize];

            // relative borders:
            length_v_r = 0;
            let mut i = i_cmon;

            while i >= 1 {
                let r = v_bord[i as usize] - v_bord[(i - 1) as usize];
                fdk_sbr_enc_add_right(&mut h_sbr_grid.bs_rel_bord, &mut length_v_r, r);
                i -= 1;
            }

            //  number of relative borders:
            let n = length_v_r;

            // freq res:
            for i in 0..i_cmon as usize {
                h_sbr_grid.v_f[i] = v_freq[(i_cmon - 1) as usize - i];
            }
            h_sbr_grid.v_f[i_cmon as usize] = 1;

            // pointer:
            let p = if i_cmon >= i_tran && i_tran != EMPTY { i_cmon - i_tran + 1 } else { 0 };

            h_sbr_grid.frame_class = frame_class;
            h_sbr_grid.bs_abs_bord = a;
            h_sbr_grid.n = n;
            h_sbr_grid.p = p;
        }
        VARFIX => {
            // absolute border:
            let a = v_bord[0];

            // relative borders:
            length_v_r = 0;

            for i in 1..length_v_bord.max(0) as usize {
                let r = v_bord[i] - v_bord[i - 1];
                fdk_sbr_enc_add_right(&mut h_sbr_grid.bs_rel_bord, &mut length_v_r, r);
            }

            // number of relative borders:
            let n = length_v_r;

            // freq res: (a VARFIX frame has at most 4 envelopes, the size of v_f)
            let lf = (length_v_freq.max(0) as usize).min(MAX_ENVELOPES_FIXVAR_VARFIX);
            h_sbr_grid.v_f[..lf].copy_from_slice(&v_freq[..lf]);

            // pointer:
            let p = if i_tran >= 0 && i_tran != EMPTY { i_tran + 1 } else { 0 };

            h_sbr_grid.frame_class = frame_class;
            h_sbr_grid.bs_abs_bord = a;
            h_sbr_grid.n = n;
            h_sbr_grid.p = p;
        }
        VARVAR => {
            let b;
            let a_l;
            let a_r;
            let n_r;
            let p;
            if spread_flag != 0 {
                // absolute borders:
                b = length_v_bord;

                a_l = v_bord[0];
                a_r = v_bord[(b - 1) as usize];

                // number of relative borders:
                let ntot = b - 2;

                let nmax = 2; // n: {0,1,2}
                if ntot > nmax {
                    n_l = nmax;
                    n_r = ntot - nmax;
                } else {
                    n_l = ntot;
                    n_r = 0;
                }

                // relative borders:
                length_v_r_l = 0;
                for i in 1..=n_l.max(0) as usize {
                    let r = v_bord[i] - v_bord[i - 1];
                    fdk_sbr_enc_add_right(&mut h_sbr_grid.bs_rel_bord_0, &mut length_v_r_l, r);
                }

                length_v_r_r = 0;
                let mut i = b - 1;
                while i >= b - n_r {
                    let r = v_bord[i as usize] - v_bord[(i - 1) as usize];
                    fdk_sbr_enc_add_right(&mut h_sbr_grid.bs_rel_bord_1, &mut length_v_r_r, r);
                    i -= 1;
                }

                // pointer (only one due to constraint in frame info):
                p = if i_tran > 0 && i_tran != EMPTY { b - i_tran } else { 0 };

                // freq res:
                h_sbr_grid.v_f_lr[..((b - 1).max(0) as usize)]
                    .copy_from_slice(&v_freq[..((b - 1).max(0) as usize)]);
            } else {
                length_v_bord = i_cmon + 1;

                // absolute borders:
                b = length_v_bord;

                a_l = v_bord[0];
                a_r = v_bord[(b - 1) as usize];

                // number of relative borders:
                let ntot = b - 2;
                n_r = ntot - n_l;

                // relative borders:
                length_v_r_l = 0;
                for i in 1..=n_l.max(0) as usize {
                    let r = v_bord[i] - v_bord[i - 1];
                    fdk_sbr_enc_add_right(&mut h_sbr_grid.bs_rel_bord_0, &mut length_v_r_l, r);
                }

                length_v_r_r = 0;
                let mut i = b - 1;
                while i >= b - n_r {
                    let r = v_bord[i as usize] - v_bord[(i - 1) as usize];
                    fdk_sbr_enc_add_right(&mut h_sbr_grid.bs_rel_bord_1, &mut length_v_r_r, r);
                    i -= 1;
                }

                // pointer (only one due to constraint in frame info):
                p = if i_cmon >= i_tran && i_tran != EMPTY { i_cmon - i_tran + 1 } else { 0 };

                // freq res:
                h_sbr_grid.v_f_lr[..((b - 1).max(0) as usize)]
                    .copy_from_slice(&v_freq[..((b - 1).max(0) as usize)]);
            }

            h_sbr_grid.frame_class = frame_class;
            h_sbr_grid.bs_abs_bord_0 = a_l;
            h_sbr_grid.bs_abs_bord_1 = a_r;
            h_sbr_grid.bs_num_rel_0 = n_l;
            h_sbr_grid.bs_num_rel_1 = n_r;
            h_sbr_grid.p = p;
        }
        _ => {
            // do nothing
        }
    }
    let _ = (length_v_r, length_v_r_l, length_v_r_r);
}

/// `createDefFrameInfo(hSbrFrameInfo, nEnv, nTimeSlots)`: copies the default (static) frame
/// info for FIXFIX frames.
///
/// Deviation: an unsupported combination leaves the frame info unchanged in both (C asserts).
fn create_def_frame_info(h_sbr_frame_info: &mut SbrFrameInfo, n_env: i32, n_time_slots: i32) {
    let src = match (n_env, n_time_slots) {
        (1, NUMBER_TIME_SLOTS_1920) => &FRAME_INFO_1_1920,
        (1, NUMBER_TIME_SLOTS_2048) => &FRAME_INFO_1_2048,
        (1, NUMBER_TIME_SLOTS_1152) => &FRAME_INFO_1_1152,
        (1, NUMBER_TIME_SLOTS_2304) => &FRAME_INFO_1_2304,
        (1, NUMBER_TIME_SLOTS_512LD) => &FRAME_INFO_1_512LD,
        (2, NUMBER_TIME_SLOTS_1920) => &FRAME_INFO_2_1920,
        (2, NUMBER_TIME_SLOTS_2048) => &FRAME_INFO_2_2048,
        (2, NUMBER_TIME_SLOTS_1152) => &FRAME_INFO_2_1152,
        (2, NUMBER_TIME_SLOTS_2304) => &FRAME_INFO_2_2304,
        (2, NUMBER_TIME_SLOTS_512LD) => &FRAME_INFO_2_512LD,
        (4, NUMBER_TIME_SLOTS_1920) => &FRAME_INFO_4_1920,
        (4, NUMBER_TIME_SLOTS_2048) => &FRAME_INFO_4_2048,
        (4, NUMBER_TIME_SLOTS_1152) => &FRAME_INFO_4_1152,
        (4, NUMBER_TIME_SLOTS_2304) => &FRAME_INFO_4_2304,
        (4, NUMBER_TIME_SLOTS_512LD) => &FRAME_INFO_4_512LD,
        _ => {
            fdk_assert!(false);
            return;
        }
    };
    *h_sbr_frame_info = *src;
}

/// `ctrlSignal2FrameInfo(hSbrGrid, hSbrFrameInfo, freq_res_fixfix)`: converts the "clear-text"
/// sbr_grid() to the frame info used by the envelope and noise floor estimators (basically the
/// bitstream decoder of ISO/IEC 14496-3, 4.6.18.3.3).
fn ctrl_signal2_frame_info(
    h_sbr_grid: &mut SbrGrid,
    h_sbr_frame_info: &mut SbrFrameInfo,
    freq_res_fixfix: &[FreqRes; 2],
) {
    let mut n_env = 0;
    let mut border = 0;
    let frame_class = h_sbr_grid.frame_class;
    let buffer_frame_start = h_sbr_grid.buffer_frame_start;
    let number_time_slots = h_sbr_grid.number_time_slots;

    match frame_class {
        FIXFIX => {
            create_def_frame_info(h_sbr_frame_info, h_sbr_grid.bs_num_env, number_time_slots);

            let frame_split = (h_sbr_frame_info.n_envelopes > 1) as usize;
            for i in 0..h_sbr_frame_info.n_envelopes.max(0) as usize {
                h_sbr_frame_info.freq_res[i] = freq_res_fixfix[frame_split];
                h_sbr_grid.v_f[i] = h_sbr_frame_info.freq_res[i];
            }
        }
        FIXVAR | VARFIX => {
            n_env = h_sbr_grid.n + 1; // read n [SBR_NUM_BITS bits]
            fdk_assert!(n_env as usize <= MAX_ENVELOPES_FIXVAR_VARFIX);

            h_sbr_frame_info.n_envelopes = n_env;

            border = h_sbr_grid.bs_abs_bord; // read the absolute border

            if n_env == 1 {
                h_sbr_frame_info.n_noise_envelopes = 1;
            } else {
                h_sbr_frame_info.n_noise_envelopes = 2;
            }
        }
        _ => {
            // do nothing
        }
    }

    match frame_class {
        FIXVAR => {
            h_sbr_frame_info.borders[0] = buffer_frame_start; // start-position of 1st envelope

            h_sbr_frame_info.borders[n_env as usize] = border;

            let mut i = n_env - 1;
            for k in 0..(n_env - 1).max(0) as usize {
                border -= h_sbr_grid.bs_rel_bord[k];
                h_sbr_frame_info.borders[i as usize] = border;
                i -= 1;
            }

            // make either envelope nr. nEnv + 1 - p short; or don't shorten if p == 0
            let p = h_sbr_grid.p;
            if p == 0 {
                h_sbr_frame_info.short_env = 0;
            } else {
                h_sbr_frame_info.short_env = n_env + 1 - p;
            }

            let mut i = n_env - 1;
            for k in 0..n_env.max(0) as usize {
                h_sbr_frame_info.freq_res[i as usize] = h_sbr_grid.v_f[k];
                i -= 1;
            }

            // if either there is no short envelope or the last envelope is short...
            if p == 0 || p == 1 {
                h_sbr_frame_info.borders_noise[1] = h_sbr_frame_info.borders[(n_env - 1) as usize];
            } else {
                h_sbr_frame_info.borders_noise[1] =
                    h_sbr_frame_info.borders[h_sbr_frame_info.short_env as usize];
            }
        }
        VARFIX => {
            // in this case 'border' indicates the start of the 1st envelope
            h_sbr_frame_info.borders[0] = border;

            for k in 0..(n_env - 1).max(0) as usize {
                border += h_sbr_grid.bs_rel_bord[k];
                h_sbr_frame_info.borders[k + 1] = border;
            }

            h_sbr_frame_info.borders[n_env as usize] = buffer_frame_start + number_time_slots;

            let p = h_sbr_grid.p;
            if p == 0 || p == 1 {
                h_sbr_frame_info.short_env = 0;
            } else {
                h_sbr_frame_info.short_env = p - 1;
            }

            for k in 0..n_env.max(0) as usize {
                h_sbr_frame_info.freq_res[k] = h_sbr_grid.v_f[k];
            }

            match p {
                0 => {
                    h_sbr_frame_info.borders_noise[1] = h_sbr_frame_info.borders[1];
                }
                1 => {
                    h_sbr_frame_info.borders_noise[1] =
                        h_sbr_frame_info.borders[(n_env - 1) as usize];
                }
                _ => {
                    h_sbr_frame_info.borders_noise[1] =
                        h_sbr_frame_info.borders[h_sbr_frame_info.short_env as usize];
                }
            }
        }
        VARVAR => {
            n_env = h_sbr_grid.bs_num_rel_0 + h_sbr_grid.bs_num_rel_1 + 1;
            fdk_assert!(n_env as usize <= MAX_ENVELOPES_VARVAR); // just to be sure
            h_sbr_frame_info.n_envelopes = n_env;

            border = h_sbr_grid.bs_abs_bord_0;
            h_sbr_frame_info.borders[0] = border;

            let mut i = 1usize;
            for k in 0..h_sbr_grid.bs_num_rel_0.max(0) as usize {
                border += h_sbr_grid.bs_rel_bord_0[k];
                h_sbr_frame_info.borders[i] = border;
                i += 1;
            }

            border = h_sbr_grid.bs_abs_bord_1;
            h_sbr_frame_info.borders[n_env as usize] = border;

            let mut i = n_env - 1;
            for k in 0..h_sbr_grid.bs_num_rel_1.max(0) as usize {
                border -= h_sbr_grid.bs_rel_bord_1[k];
                h_sbr_frame_info.borders[i as usize] = border;
                i -= 1;
            }

            let p = h_sbr_grid.p;
            if p == 0 {
                h_sbr_frame_info.short_env = 0;
            } else {
                h_sbr_frame_info.short_env = n_env + 1 - p;
            }

            for k in 0..n_env.max(0) as usize {
                h_sbr_frame_info.freq_res[k] = h_sbr_grid.v_f_lr[k];
            }

            if n_env == 1 {
                h_sbr_frame_info.n_noise_envelopes = 1;
                h_sbr_frame_info.borders_noise[0] = h_sbr_grid.bs_abs_bord_0;
                h_sbr_frame_info.borders_noise[1] = h_sbr_grid.bs_abs_bord_1;
            } else {
                h_sbr_frame_info.n_noise_envelopes = 2;
                h_sbr_frame_info.borders_noise[0] = h_sbr_grid.bs_abs_bord_0;

                if p == 0 || p == 1 {
                    h_sbr_frame_info.borders_noise[1] =
                        h_sbr_frame_info.borders[(n_env - 1) as usize];
                } else {
                    h_sbr_frame_info.borders_noise[1] =
                        h_sbr_frame_info.borders[h_sbr_frame_info.short_env as usize];
                }
                h_sbr_frame_info.borders_noise[2] = h_sbr_grid.bs_abs_bord_1;
            }
        }
        _ => {
            // do nothing
        }
    }

    if frame_class == VARFIX || frame_class == FIXVAR {
        h_sbr_frame_info.borders_noise[0] = h_sbr_frame_info.borders[0];
        if n_env == 1 {
            h_sbr_frame_info.borders_noise[1] = h_sbr_frame_info.borders[n_env as usize];
        } else {
            h_sbr_frame_info.borders_noise[2] = h_sbr_frame_info.borders[n_env as usize];
        }
    }
}
