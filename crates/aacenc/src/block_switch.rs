// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/block_switch.h, libAACenc/src/block_switch.cpp.

//! Port of `block_switch.h`: block switching (attack detection).
//!
//! The `aarch64` and `x86` translation units of `block_switch.cpp` are identical; the variants
//! differ only in the primitives (`fMult`, `fMultDiv2`, ...).
//!
//! # Tables and out-of-range indexes
//!
//! The file-local tables are kept flat, in the memory order of C, so that an index past the end
//! of a row reads the next row (reproduced from the C layout). The state machine only produces
//! in-range indexes
//! (`lastWindowSequence` in `LONG_WINDOW..=LOWOV_WINDOW`, `attackIndex` and `lastAttackIndex`
//! below `nBlockSwitchWindows`, which is 4 or 8). In the port, indexes outside a whole table
//! give `WRONG_WINDOW` (window sequences and shapes) or leave the grouping cleared.

use crate::psy_const::{
    FALSE, LOL_WINDOW, LONG_WINDOW, MAX_NO_OF_GROUPS, SHORT_WINDOW, SINE_WINDOW, STOP_WINDOW, TRUE,
    WRONG_WINDOW,
};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::machine_type::{IntPcm, SAMPLE_BITS};

/// `BLOCK_SWITCH_WINDOWS`: number of windows for energy calculation
pub const BLOCK_SWITCH_WINDOWS: usize = 8;
/// `BLOCK_SWITCHING_IIR_LEN`: length of HighPass-IIR-Filter for Attack-Detection
pub const BLOCK_SWITCHING_IIR_LEN: usize = 2;
/// `BLOCK_SWITCH_ENERGY_SHIFT`: should be logDualis(BLOCK_SWITCH_WINDOW_LEN) to avoid overflow
/// in windowNrgs.
pub const BLOCK_SWITCH_ENERGY_SHIFT: i32 = 7;

/// `LAST_WINDOW`
pub const LAST_WINDOW: usize = 0;
/// `THIS_WINDOW`
pub const THIS_WINDOW: usize = 1;

/// `BLOCK_SWITCHING_CONTROL` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockSwitchingControl {
    pub last_window_sequence: i32,
    pub window_shape: i32,
    pub last_window_shape: i32,
    /// number of windows for energy calculation
    pub n_block_switch_windows: u32,
    pub attack: i32,
    pub lastattack: i32,
    pub attack_index: i32,
    pub last_attack_index: i32,
    /// for Low Delay, don't allow short frames
    pub allow_short_frames: i32,
    /// for Low Delay, don't do look-ahead
    pub allow_look_ahead: i32,
    pub no_of_groups: i32,
    pub group_len: [i32; MAX_NO_OF_GROUPS],
    /// max energy in subwindows
    pub max_window_nrg: FixpDbl,
    /// time signal energy in Subwindows (last and current)
    pub window_nrg: [[FixpDbl; BLOCK_SWITCH_WINDOWS]; 2],
    /// filtered time signal energy in segments (last and current)
    pub window_nrg_f: [[FixpDbl; BLOCK_SWITCH_WINDOWS]; 2],
    /// recursively accumulated windowNrgF
    pub acc_window_nrg: FixpDbl,
    /// filter delay-line
    pub iir_states: [FixpDbl; BLOCK_SWITCHING_IIR_LEN],
}

// Tables, dumped with scripts/dump_table.py from libAACenc/src/block_switch.cpp (flat, in the
// memory order of the C tables).

/// `blockType2windowShape[2][5]` (`[allowShortFrames][blockType]`; LONG, START, SHORT, STOP,
/// LOWOV; row 0: LD, row 1: LC).
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp blockType2windowShape INT`
pub static BLOCK_TYPE2_WINDOW_SHAPE: [i32; 10] = [0, 1, 5, 0, 1, 1, 0, 0, 1, 5];

/// `hiPassCoeff[BLOCK_SWITCHING_IIR_LEN]`: IIR high pass coeffs.
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp hiPassCoeff SHORT`
pub static HI_PASS_COEFF: [FixpSgl; BLOCK_SWITCHING_IIR_LEN] = [-16695, 24733];

/// `accWindowNrgFac`: factor for accumulating filtered window energies (`FL2FXCONST_DBL(0.3f)`).
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp '&accWindowNrgFac' LONG --count 1 --hex`
pub const ACC_WINDOW_NRG_FAC: FixpDbl = 0x26666680;
/// `oneMinusAccWindowNrgFac` (`FL2FXCONST_SGL(0.7f)`).
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp '&oneMinusAccWindowNrgFac' SHORT --count 1`
pub const ONE_MINUS_ACC_WINDOW_NRG_FAC: FixpSgl = 22938;
/// `invAttackRatio`: inverted lower ratio limit for attacks (`FL2FXCONST_SGL(0.1f)`).
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp '&invAttackRatio' SHORT --count 1`
pub const INV_ATTACK_RATIO: FixpSgl = 3277;
/// `minAttackNrg`: minimum energy for attacks
/// (`FL2FXCONST_DBL(1e+6f * NORM_PCM_ENERGY) >> BLOCK_SWITCH_ENERGY_SHIFT`).
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp '&minAttackNrg' LONG --count 1`
pub const MIN_ATTACK_NRG: FixpDbl = 15625;

/// `suggestedGroupingTable[TRANS_FAC][MAX_NO_OF_GROUPS]` (row: window of the attack).
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp suggestedGroupingTable INT`
pub static SUGGESTED_GROUPING_TABLE: [i32; 32] = [
    1, 3, 3, 1, 1, 1, 3, 3, 2, 1, 3, 2, 3, 1, 3, 1, 3, 1, 1, 3, 3, 2, 1, 2, 3, 3, 1, 1, 3, 3, 1, 1,
];

/// `chgWndSq[2][N_BLOCKTYPES]` (`[attack][blockType]`): change block type depending on current
/// blocktype and whether there's an attack; assume no look-ahead.
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp chgWndSq INT`
pub static CHG_WND_SQ: [i32; 12] = [0, 3, 5, 0, 3, 5, 1, 4, 5, 1, 4, 5];

/// `chgWndSqLkAhd[2][2][N_BLOCKTYPES]` (`[lastattack][attack][blockType]`): change block type
/// depending on current blocktype and whether there's an attack; assume look-ahead.
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp chgWndSqLkAhd INT`
pub static CHG_WND_SQ_LK_AHD: [i32; 24] =
    [0, 2, 3, 0, 5, 5, 1, 2, 2, 1, 5, 5, 0, 2, 2, 0, 5, 5, 1, 2, 2, 1, 5, 5];

/// `synchronizedBlockTypeTable[5][5]` (`[patchType][blockType]`).
/// `scripts/dump_table.py libAACenc/src/block_switch.cpp synchronizedBlockTypeTable UCHAR`
pub static SYNCHRONIZED_BLOCK_TYPE_TABLE: [u8; 25] =
    [0, 1, 2, 3, 4, 1, 1, 2, 2, 4, 2, 2, 2, 2, 5, 3, 2, 2, 3, 4, 4, 4, 5, 4, 4];

/// `blockType2windowShape[allowShortFrames][blockType]` (see the
/// [module documentation](self) for indexes outside the table).
fn block_type2_window_shape(allow_short_frames: i32, block_type: i32) -> i32 {
    let idx = (allow_short_frames as i64) * 5 + block_type as i64;
    usize::try_from(idx)
        .ok()
        .and_then(|i| BLOCK_TYPE2_WINDOW_SHAPE.get(i))
        .copied()
        .unwrap_or(WRONG_WINDOW)
}

/// `FDKaacEnc_InitBlockSwitching()`
pub fn fdk_aac_enc_init_block_switching(
    block_switching_control: &mut BlockSwitchingControl,
    is_low_delay: i32,
) {
    *block_switching_control = BlockSwitchingControl::default();

    if is_low_delay != 0 {
        block_switching_control.n_block_switch_windows = 4;
        block_switching_control.allow_short_frames = 0;
        block_switching_control.allow_look_ahead = 0;
    } else {
        block_switching_control.n_block_switch_windows = 8;
        block_switching_control.allow_short_frames = 1;
        block_switching_control.allow_look_ahead = 1;
    }

    block_switching_control.no_of_groups = MAX_NO_OF_GROUPS as i32;

    // Initialize startvalue for blocktype
    block_switching_control.last_window_sequence = LONG_WINDOW;
    block_switching_control.window_shape = block_type2_window_shape(
        block_switching_control.allow_short_frames,
        block_switching_control.last_window_sequence,
    );
}

/// `FDKaacEnc_BlockSwitching()`: attack detection on `granule_length` samples of
/// `p_time_signal` and the next window sequence (`lastWindowSequence`) and shape.
///
/// `p_time_signal` must hold `granule_length` samples (`nBlockSwitchWindows` windows of
/// `granuleLength >> (nBlockSwitchWindows == 4 ? 2 : 3)` samples are read).
pub fn fdk_aac_enc_block_switching(
    block_switching_control: &mut BlockSwitchingControl,
    granule_length: i32,
    is_lfe: i32,
    p_time_signal: &[IntPcm],
) -> i32 {
    block_switching(block_switching_control, granule_length, is_lfe, p_time_signal, None)
}

/// [`fdk_aac_enc_block_switching`] of the two channels of a channel pair, with the window
/// energies of both channels computed in one interleaved loop ([`window_energies_pair`]; the
/// high-pass IIR of one channel is a serial recurrence, two of them run in parallel). Same
/// results as two calls of [`fdk_aac_enc_block_switching`], in the order of the arguments.
pub fn fdk_aac_enc_block_switching_pair(
    bsc: [&mut BlockSwitchingControl; 2],
    granule_length: i32,
    is_lfe: [i32; 2],
    p_time_signal: [&[IntPcm]; 2],
) -> [i32; 2] {
    let [b0, b1] = bsc;
    // the energies depend only on the IIR states, the window count and the signal, which the
    // part of FDKaacEnc_BlockSwitching() before FDKaacEnc_CalcWindowEnergy() does not change;
    // LFE channels return before
    if is_lfe[0] != 0 || is_lfe[1] != 0 || b0.n_block_switch_windows != b1.n_block_switch_windows {
        return [
            block_switching(b0, granule_length, is_lfe[0], p_time_signal[0], None),
            block_switching(b1, granule_length, is_lfe[1], p_time_signal[1], None),
        ];
    }
    let window_len = granule_length >> (if b0.n_block_switch_windows == 4 { 2 } else { 3 });
    let (e0, e1) = window_energies_pair([b0, b1], window_len, p_time_signal);
    [
        block_switching(b0, granule_length, is_lfe[0], p_time_signal[0], Some(&e0)),
        block_switching(b1, granule_length, is_lfe[1], p_time_signal[1], Some(&e1)),
    ]
}

/// `FDKaacEnc_BlockSwitching()`, with the result of `FDKaacEnc_CalcWindowEnergy()` computed in
/// advance (`energies`) or not (`None`).
fn block_switching(
    block_switching_control: &mut BlockSwitchingControl,
    granule_length: i32,
    is_lfe: i32,
    p_time_signal: &[IntPcm],
    energies: Option<&WindowEnergies>,
) -> i32 {
    let bsc = block_switching_control;
    let n_block_switch_windows: u32 = bsc.n_block_switch_windows;

    // for LFE : only LONG window allowed
    if is_lfe != 0 {
        // case LFE:
        // only long blocks, always use sine windows (MPEG2 AAC, MPEG4 AAC)
        bsc.last_window_sequence = LONG_WINDOW;
        bsc.window_shape = SINE_WINDOW;
        bsc.no_of_groups = 1;
        bsc.group_len[0] = 1;

        return 0;
    }

    // nBlockSwitchWindows is 4 or 8 (set by FDKaacEnc_InitBlockSwitching()); the windows below
    // are indexed with it.
    fdk_assert!((1..=BLOCK_SWITCH_WINDOWS as u32).contains(&n_block_switch_windows));
    let n = (n_block_switch_windows as usize).clamp(1, BLOCK_SWITCH_WINDOWS);

    // Save current attack index as last attack index
    bsc.lastattack = bsc.attack;
    bsc.last_attack_index = bsc.attack_index;

    // Save current window energy as last window energy
    bsc.window_nrg[0] = bsc.window_nrg[1];
    bsc.window_nrg_f[0] = bsc.window_nrg_f[1];

    if bsc.allow_short_frames != 0 {
        // Calculate suggested grouping info for the last frame

        // Reset grouping info
        bsc.group_len = [0; MAX_NO_OF_GROUPS];

        // Set grouping info
        bsc.no_of_groups = MAX_NO_OF_GROUPS as i32;

        // lastAttackIndex is below nBlockSwitchWindows (see the module documentation).
        let row = usize::try_from(bsc.last_attack_index).ok().and_then(|i| {
            SUGGESTED_GROUPING_TABLE.get(i * MAX_NO_OF_GROUPS..(i + 1) * MAX_NO_OF_GROUPS)
        });
        if let Some(row) = row {
            bsc.group_len.copy_from_slice(row);
        }

        if bsc.attack == TRUE {
            // C passes windowNrg[0]; an index past it reads windowNrg[1] (the flattened array).
            bsc.max_window_nrg =
                fdk_aac_enc_get_window_energy(bsc.window_nrg.as_flattened(), bsc.last_attack_index);
        } else {
            bsc.max_window_nrg = 0;
        }
    }

    // Calculate unfiltered and filtered energies in subwindows and combine to segments
    match energies {
        Some(e) => e.store(bsc),
        None => fdk_aac_enc_calc_window_energy(
            bsc,
            granule_length >> (if n_block_switch_windows == 4 { 2 } else { 3 }),
            p_time_signal,
        ),
    }

    // now calculate if there is an attack

    // reset attack
    bsc.attack = FALSE;

    // look for attack
    let mut en_max: FixpDbl = 0;
    let mut en_m1: FixpDbl = bsc.window_nrg_f[0][n - 1];

    for i in 0..n {
        let tmp: FixpDbl = f_mult_div2(ONE_MINUS_ACC_WINDOW_NRG_FAC, bsc.acc_window_nrg);
        bsc.acc_window_nrg = f_mult_add(tmp, ACC_WINDOW_NRG_FAC, en_m1);

        if f_mult(bsc.window_nrg_f[1][i], INV_ATTACK_RATIO) > bsc.acc_window_nrg {
            bsc.attack = TRUE;
            bsc.attack_index = i as i32;
        }
        en_m1 = bsc.window_nrg_f[1][i];
        en_max = fix_max(en_max, en_m1);
    }

    if en_max < MIN_ATTACK_NRG {
        bsc.attack = FALSE;
    }

    // Check if attack spreads over frame border
    if (bsc.attack == FALSE) && (bsc.lastattack == TRUE) {
        // if attack is in last window repeat SHORT_WINDOW
        if ((bsc.window_nrg_f[0][n - 1] >> 4)
            > f_mult((10 << (DFRACT_BITS - 1 - 4)) as FixpDbl, bsc.window_nrg_f[1][1]))
            && (bsc.last_attack_index == n_block_switch_windows as i32 - 1)
        {
            bsc.attack = TRUE;
            bsc.attack_index = 0;
        }
    }

    if bsc.allow_look_ahead != 0 {
        // chgWndSqLkAhd[lastattack][attack][lastWindowSequence]
        let idx = (bsc.lastattack as i64) * 12
            + (bsc.attack as i64) * 6
            + bsc.last_window_sequence as i64;
        bsc.last_window_sequence = usize::try_from(idx)
            .ok()
            .and_then(|i| CHG_WND_SQ_LK_AHD.get(i))
            .copied()
            .unwrap_or(WRONG_WINDOW);
    } else {
        // Low Delay: chgWndSq[attack][lastWindowSequence]
        let idx = (bsc.attack as i64) * 6 + bsc.last_window_sequence as i64;
        bsc.last_window_sequence = usize::try_from(idx)
            .ok()
            .and_then(|i| CHG_WND_SQ.get(i))
            .copied()
            .unwrap_or(WRONG_WINDOW);
    }

    // update window shape
    bsc.window_shape = block_type2_window_shape(bsc.allow_short_frames, bsc.last_window_sequence);

    0
}

/// `FDKaacEnc_GetWindowEnergy()` (static in C)
///
/// For coherency, change FDKaacEnc_GetWindowEnergy() to calcluate the energy for a block
/// switching analysis windows, not for a short block. The same is done
/// FDKaacEnc_CalcWindowEnergy(). The result of FDKaacEnc_GetWindowEnergy() is used for a
/// comparision of the max energy of left/right channel.
///
/// `in_` is `windowNrg[0]` (the caller passes both rows flattened); the index is below
/// `nBlockSwitchWindows` (see the [module documentation](self)); outside the slice the port
/// returns 0.
pub fn fdk_aac_enc_get_window_energy(in_: &[FixpDbl], bl_sw_wnd_idx: i32) -> FixpDbl {
    usize::try_from(bl_sw_wnd_idx).ok().and_then(|i| in_.get(i)).copied().unwrap_or(0)
}

/// `FDKaacEnc_CalcWindowEnergy()` (static in C): unfiltered and high-pass filtered energies
/// of the `nBlockSwitchWindows` windows of `window_len` samples into `windowNrg[1]` and
/// `windowNrgF[1]`.
pub fn fdk_aac_enc_calc_window_energy(
    block_switching_control: &mut BlockSwitchingControl,
    window_len: i32,
    p_time_signal: &[IntPcm],
) {
    let bsc = block_switching_control;
    let hi_pass_coeff0: FixpSgl = HI_PASS_COEFF[0];
    let hi_pass_coeff1: FixpSgl = HI_PASS_COEFF[1];

    let mut temp_iir_state0: FixpDbl = bsc.iir_states[0];
    let mut temp_iir_state1: FixpDbl = bsc.iir_states[1];

    let mut t = 0usize;
    let n = (bsc.n_block_switch_windows as usize).min(BLOCK_SWITCH_WINDOWS);

    // sum up scalarproduct of timesignal as windowed Energies
    for w in 0..n {
        // ULONG (= UINT) accumulators: the sums wrap modulo 2^32 as in C.
        let mut temp_window_nrg: u32 = 0x0;
        let mut temp_window_nrg_f: u32 = 0x0;

        // windowNrg = sum(timesample^2)
        for _ in 0..window_len.max(0) {
            // tempUnfiltered is scaled with 1 to prevent overflows during calculation of
            // tempFiltred
            let temp_unfiltered: FixpDbl =
                (p_time_signal[t] as FixpDbl) << (DFRACT_BITS - SAMPLE_BITS - 1);
            t += 1;
            let t1: FixpDbl =
                f_mult_div2(hi_pass_coeff1, temp_unfiltered.wrapping_sub(temp_iir_state0));
            let t2: FixpDbl = f_mult_div2(hi_pass_coeff0, temp_iir_state1);
            temp_iir_state0 = temp_unfiltered;
            temp_iir_state1 = t1.wrapping_sub(t2) << 1;

            temp_window_nrg = temp_window_nrg.wrapping_add(
                (f_pow2_div2(temp_iir_state0) >> (BLOCK_SWITCH_ENERGY_SHIFT - 1 - 2)) as u32,
            );
            temp_window_nrg_f = temp_window_nrg_f.wrapping_add(
                (f_pow2_div2(temp_iir_state1) >> (BLOCK_SWITCH_ENERGY_SHIFT - 1 - 2)) as u32,
            );
        }
        bsc.window_nrg[1][w] = fixmin_ui(temp_window_nrg, MAXVAL_DBL as u32) as FixpDbl;
        bsc.window_nrg_f[1][w] = fixmin_ui(temp_window_nrg_f, MAXVAL_DBL as u32) as FixpDbl;
    }
    bsc.iir_states[0] = temp_iir_state0;
    bsc.iir_states[1] = temp_iir_state1;
}

/// The results of `FDKaacEnc_CalcWindowEnergy()`: `windowNrg[1]`, `windowNrgF[1]` and the
/// IIR states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowEnergies {
    pub window_nrg: [FixpDbl; BLOCK_SWITCH_WINDOWS],
    pub window_nrg_f: [FixpDbl; BLOCK_SWITCH_WINDOWS],
    pub iir_states: [FixpDbl; BLOCK_SWITCHING_IIR_LEN],
}

impl WindowEnergies {
    /// Stores the results as `FDKaacEnc_CalcWindowEnergy()` does (the windows past
    /// `nBlockSwitchWindows` keep their values).
    fn store(&self, bsc: &mut BlockSwitchingControl) {
        let n = (bsc.n_block_switch_windows as usize).min(BLOCK_SWITCH_WINDOWS);
        bsc.window_nrg[1][..n].copy_from_slice(&self.window_nrg[..n]);
        bsc.window_nrg_f[1][..n].copy_from_slice(&self.window_nrg_f[..n]);
        bsc.iir_states = self.iir_states;
    }
}

/// [`fdk_aac_enc_calc_window_energy`] of two channels with the same `nBlockSwitchWindows`, in
/// one loop over the samples (the two high-pass recurrences are independent). Returns the
/// results instead of storing them.
pub fn window_energies_pair(
    bsc: [&BlockSwitchingControl; 2],
    window_len: i32,
    p_time_signal: [&[IntPcm]; 2],
) -> (WindowEnergies, WindowEnergies) {
    debug_assert_eq!(bsc[0].n_block_switch_windows, bsc[1].n_block_switch_windows);
    let hi_pass_coeff0: FixpSgl = HI_PASS_COEFF[0];
    let hi_pass_coeff1: FixpSgl = HI_PASS_COEFF[1];
    let mut e = [WindowEnergies {
        window_nrg: [0; BLOCK_SWITCH_WINDOWS],
        window_nrg_f: [0; BLOCK_SWITCH_WINDOWS],
        iir_states: [0; BLOCK_SWITCHING_IIR_LEN],
    }; 2];
    let mut s0 = [bsc[0].iir_states[0], bsc[1].iir_states[0]];
    let mut s1 = [bsc[0].iir_states[1], bsc[1].iir_states[1]];
    let n = (bsc[0].n_block_switch_windows as usize).min(BLOCK_SWITCH_WINDOWS);
    let len = window_len.max(0) as usize;
    let (sig0, sig1) = (&p_time_signal[0][..n * len], &p_time_signal[1][..n * len]);
    for w in 0..n {
        let mut nrg = [0u32; 2];
        let mut nrg_f = [0u32; 2];
        let (w0, w1) = (&sig0[w * len..(w + 1) * len], &sig1[w * len..(w + 1) * len]);
        for (&x0, &x1) in w0.iter().zip(w1) {
            for (c, x) in [x0, x1].into_iter().enumerate() {
                // as in fdk_aac_enc_calc_window_energy
                let temp_unfiltered: FixpDbl = (x as FixpDbl) << (DFRACT_BITS - SAMPLE_BITS - 1);
                let t1: FixpDbl = f_mult_div2(hi_pass_coeff1, temp_unfiltered.wrapping_sub(s0[c]));
                let t2: FixpDbl = f_mult_div2(hi_pass_coeff0, s1[c]);
                s0[c] = temp_unfiltered;
                s1[c] = t1.wrapping_sub(t2) << 1;
                nrg[c] = nrg[c].wrapping_add(
                    (f_pow2_div2(s0[c]) >> (BLOCK_SWITCH_ENERGY_SHIFT - 1 - 2)) as u32,
                );
                nrg_f[c] = nrg_f[c].wrapping_add(
                    (f_pow2_div2(s1[c]) >> (BLOCK_SWITCH_ENERGY_SHIFT - 1 - 2)) as u32,
                );
            }
        }
        for c in 0..2 {
            e[c].window_nrg[w] = fixmin_ui(nrg[c], MAXVAL_DBL as u32) as FixpDbl;
            e[c].window_nrg_f[w] = fixmin_ui(nrg_f[c], MAXVAL_DBL as u32) as FixpDbl;
        }
    }
    for c in 0..2 {
        e[c].iir_states = [s0[c], s1[c]];
    }
    (e[0], e[1])
}

/// `synchronizedBlockTypeTable[patchType][blockType]` (see the [module documentation](self)).
fn synchronized_block_type(patch_type: u8, block_type: i32) -> u8 {
    let idx = (patch_type as i64) * 5 + block_type as i64;
    usize::try_from(idx)
        .ok()
        .and_then(|i| SYNCHRONIZED_BLOCK_TYPE_TABLE.get(i))
        .copied()
        .unwrap_or(WRONG_WINDOW as u8)
}

/// `FDKaacEnc_SyncBlockSwitching()`: synchronizes the window sequences and groupings of the two
/// channels of a channel pair, and translates `LOWOV_WINDOW` (AAC-LD/ELD) to a long window with
/// the low-overlap shape.
///
/// `block_switching_control_right` is `NULL` in C for one channel (it is only accessed for
/// `n_channels == 2`). With `n_channels == 2` and `None` the port works on a scratch default
/// structure.
pub fn fdk_aac_enc_sync_block_switching(
    block_switching_control_left: &mut BlockSwitchingControl,
    block_switching_control_right: Option<&mut BlockSwitchingControl>,
    n_channels: i32,
    common_window: i32,
) -> i32 {
    let left = block_switching_control_left;
    let mut scratch = BlockSwitchingControl::default();
    let right = match block_switching_control_right {
        Some(r) => r,
        None => {
            fdk_assert!(n_channels != 2);
            &mut scratch
        }
    };

    let mut patch_type: u8 = LONG_WINDOW as u8;

    if n_channels == 2 && common_window == TRUE {
        // could be better with a channel loop (need a handle to psy_data)
        // get suggested Block Types and synchronize
        patch_type = synchronized_block_type(patch_type, left.last_window_sequence);
        patch_type = synchronized_block_type(patch_type, right.last_window_sequence);

        // sanity check (no change from low overlap window to short winow and vice versa)
        if patch_type as i32 == WRONG_WINDOW {
            return -1; // mixed up AAC-LC and AAC-LD
        }

        // Set synchronized Blocktype
        left.last_window_sequence = patch_type as i32;
        right.last_window_sequence = patch_type as i32;

        // update window shape
        left.window_shape =
            block_type2_window_shape(left.allow_short_frames, left.last_window_sequence);
        right.window_shape =
            block_type2_window_shape(left.allow_short_frames, right.last_window_sequence);
    }

    if left.allow_short_frames != 0 {
        if n_channels == 2 {
            if common_window == TRUE {
                // Synchronize grouping info
                let window_sequence_left_old = left.last_window_sequence;
                let window_sequence_right_old = right.last_window_sequence;

                // Long Blocks
                if patch_type as i32 != SHORT_WINDOW {
                    // Set grouping info
                    left.no_of_groups = 1;
                    right.no_of_groups = 1;
                    left.group_len[0] = 1;
                    right.group_len[0] = 1;

                    for i in 1..MAX_NO_OF_GROUPS {
                        left.group_len[i] = 0;
                        right.group_len[i] = 0;
                    }
                }
                // Short Blocks
                else {
                    // in case all two channels were detected as short-blocks before syncing,
                    // use the grouping of channel with higher maxWindowNrg
                    if (window_sequence_left_old == SHORT_WINDOW)
                        && (window_sequence_right_old == SHORT_WINDOW)
                    {
                        if left.max_window_nrg > right.max_window_nrg {
                            // Left Channel wins
                            right.no_of_groups = left.no_of_groups;
                            right.group_len = left.group_len;
                        } else {
                            // Right Channel wins
                            left.no_of_groups = right.no_of_groups;
                            left.group_len = right.group_len;
                        }
                    } else if (window_sequence_left_old == SHORT_WINDOW)
                        && (window_sequence_right_old != SHORT_WINDOW)
                    {
                        // else use grouping of short-block channel
                        right.no_of_groups = left.no_of_groups;
                        right.group_len = left.group_len;
                    } else if (window_sequence_right_old == SHORT_WINDOW)
                        && (window_sequence_left_old != SHORT_WINDOW)
                    {
                        left.no_of_groups = right.no_of_groups;
                        left.group_len = right.group_len;
                    } else {
                        // syncing a start and stop window ...
                        right.no_of_groups = 2;
                        left.no_of_groups = 2;
                        right.group_len[0] = 4;
                        left.group_len[0] = 4;
                        right.group_len[1] = 4;
                        left.group_len[1] = 4;
                    }
                } // Short Blocks
            } else {
                // stereo, no common window
                if left.last_window_sequence != SHORT_WINDOW {
                    left.no_of_groups = 1;
                    left.group_len[0] = 1;
                    for i in 1..MAX_NO_OF_GROUPS {
                        left.group_len[i] = 0;
                    }
                }
                if right.last_window_sequence != SHORT_WINDOW {
                    right.no_of_groups = 1;
                    right.group_len[0] = 1;
                    for i in 1..MAX_NO_OF_GROUPS {
                        right.group_len[i] = 0;
                    }
                }
            } // common window
        } else {
            // Mono
            if left.last_window_sequence != SHORT_WINDOW {
                left.no_of_groups = 1;
                left.group_len[0] = 1;

                for i in 1..MAX_NO_OF_GROUPS {
                    left.group_len[i] = 0;
                }
            }
        }
    } // allowShortFrames

    // Translate LOWOV_WINDOW block type to a meaningful window shape.
    if left.allow_short_frames == 0
        && left.last_window_sequence != LONG_WINDOW
        && left.last_window_sequence != STOP_WINDOW
    {
        left.last_window_sequence = LONG_WINDOW;
        left.window_shape = LOL_WINDOW;
    }
    if n_channels == 2
        && right.allow_short_frames == 0
        && right.last_window_sequence != LONG_WINDOW
        && right.last_window_sequence != STOP_WINDOW
    {
        right.last_window_sequence = LONG_WINDOW;
        right.window_shape = LOL_WINDOW;
    }

    0
}
