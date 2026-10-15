// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_framewindowing.h, libSACenc/src/sacenc_framewindowing.cpp.

//! Port of `sacenc_framewindowing.h` and `sacenc_framewindowing.cpp`: the analysis windows
//! and the framing (parameter set positions) of a frame.
//!
//! # Variants
//!
//! The code is identical in the `aarch64` and `x86` builds; the results differ through `fMult`
//! and `fDivNormHighPrec`.
//!
//! # Memory
//!
//! The windows are written at positions that depend on the transient positions; with the
//! positions that the encoder passes (none: it always passes -1) and the transient positions
//! inside the frame they stay inside the windows (`3 * nTimeSlotsMax` values). The taper
//! windows (32 values) hold `nTimeSlotsMax / 2 + 1` values (the encoder uses at most 16 time
//! slots); writes outside them are dropped.

use fdk_aac_rs_fdk::common_fix::*;

use crate::sacenc_bitstream::{FramingInfo, MAX_NUM_PARAMS, MAX_TIME_SLOTS};
use crate::sacenc_lib::*;

/// `FIXP_WIN`
pub type FixpWin = FixpDbl;

/// `FW_SLOTTYPE`
pub type FwSlotType = i32;
pub const FW_INTP: FwSlotType = 0;
pub const FW_HOLD: FwSlotType = 1;

/// `FW_DIMENSION`
pub type FwDimension = i32;
pub const FW_LEAVE_DIM: FwDimension = 0;
pub const FW_CHANGE_DIM: FwDimension = 1;

/// `FRAMEWINDOW_CONFIG`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWindowConfig {
    pub n_time_slots_max: i32,
    pub b_frame_keep: i32,
}

/// `FRAMEWIN_DATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWinData {
    pub slot: i32,
    pub hold: FwSlotType,
}

/// `FRAMEWIN_LIST`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWinList {
    pub dat: [FrameWinData; MAX_NUM_PARAMS],
    pub n: i32,
}

/// `FRAMEWINDOW` (`struct T_FRAMEWINDOW`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWindow {
    pub n_time_slots_max: i32,
    pub b_frame_keep: i32,
    pub start_slope: i32,
    pub stop_slope: i32,
    pub start_rect: i32,
    pub stop_rect: i32,

    pub taper_ana_len: i32,
    pub taper_syn_len: i32,
    pub p_taper_ana_fdk: [FixpWin; MAX_TIME_SLOTS],
    pub p_taper_syn_fdk: [FixpWin; MAX_TIME_SLOTS],
}

/// `HANDLE_FRAMEWINDOW`
pub type HandleFrameWindow = Option<Box<FrameWindow>>;

/// `VAR_TYPE`
type VarType = i32;
const VAR_HOLD: VarType = 0;
const VAR_ISOLATE: VarType = 1;

/// `FDKmemset_flex(&w[off], val, n)`
#[inline]
fn set(w: &mut [FixpWin], off: i32, val: FixpWin, n: i32) {
    for i in 0..n.max(0) {
        w[(off + i) as usize] = val;
    }
}

/// `calcTaperWin(pTaperWin, timeSlots)` (static in C): `pTaperWin[i] = i / timeSlots`,
/// `pTaperWin[timeSlots] = 1`.
pub fn calc_taper_win(p_taper_win: &mut [FixpWin], time_slots: i32) {
    for i in 0..time_slots.max(0) {
        let mut scale = 0;
        let x = f_div_norm_high_prec(i, time_slots, &mut scale);

        let v = if scale < 0 { x >> (-scale) } else { x << scale };
        if let Some(t) = p_taper_win.get_mut(i as usize) {
            *t = v;
        }
    }
    if let Some(t) = p_taper_win.get_mut(time_slots.max(0) as usize) {
        *t = MAXVAL_DBL;
    }
}

/// `fdk_sacenc_frameWindow_Create(phFrameWindow)`
pub fn fdk_sacenc_frame_window_create(
    ph_frame_window: Option<&mut HandleFrameWindow>,
) -> FdkSacencError {
    match ph_frame_window {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            *h = Some(Box::default());
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_frameWindow_Init(hFrameWindow, pFrameWindowConfig)`
pub fn fdk_sacenc_frame_window_init(
    h_frame_window: &mut FrameWindow,
    p_frame_window_config: &FrameWindowConfig,
) -> FdkSacencError {
    if p_frame_window_config.n_time_slots_max < 0 {
        return SACENC_INIT_ERROR;
    }
    h_frame_window.b_frame_keep = p_frame_window_config.b_frame_keep;
    h_frame_window.n_time_slots_max = p_frame_window_config.n_time_slots_max;

    let win_max_val: FixpWin = MAXVAL_DBL;
    let time_slots = p_frame_window_config.n_time_slots_max;
    h_frame_window.start_slope = 0;
    h_frame_window.stop_slope = ((3 * time_slots) >> 1) - 1;
    h_frame_window.start_rect = time_slots >> 1;
    h_frame_window.stop_rect = time_slots;
    calc_taper_win(&mut h_frame_window.p_taper_syn_fdk, time_slots >> 1);
    h_frame_window.taper_syn_len = time_slots >> 1;

    // Calculate Taper for non-rect. ana. windows
    h_frame_window.taper_ana_len = h_frame_window.start_rect - h_frame_window.start_slope;
    for ts in 0..h_frame_window.taper_ana_len.max(0) as usize {
        if let Some(t) = h_frame_window.p_taper_ana_fdk.get_mut(ts) {
            *t = win_max_val;
        }
    }

    SACENC_OK
}

/// `fdk_sacenc_frameWindow_Destroy(phFrameWindow)`
pub fn fdk_sacenc_frame_window_destroy(
    ph_frame_window: Option<&mut HandleFrameWindow>,
) -> FdkSacencError {
    if let Some(h) = ph_frame_window {
        *h = None;
    }
    SACENC_OK
}

/// `FrameWinList_Reset(pFrameWinList)` (static in C)
pub fn frame_win_list_reset(p_frame_win_list: &mut FrameWinList) -> FdkSacencError {
    for d in p_frame_win_list.dat.iter_mut() {
        d.slot = -1;
        d.hold = FW_INTP;
    }
    p_frame_win_list.n = 0;
    SACENC_OK
}

/// `FrameWindowList_Add(pFrameWinList, slot, hold)` (static in C)
pub fn frame_window_list_add(
    p_frame_win_list: &mut FrameWinList,
    slot: i32,
    hold: FwSlotType,
) -> FdkSacencError {
    let n = p_frame_win_list.n;
    if n >= MAX_NUM_PARAMS as i32 {
        // Place left in List ?
        SACENC_PARAM_ERROR
    } else if n > 0 && p_frame_win_list.dat[(n - 1) as usize].slot.wrapping_sub(slot) > 0 {
        SACENC_PARAM_ERROR
    } else {
        p_frame_win_list.dat[n as usize].slot = slot;
        p_frame_win_list.dat[n as usize].hold = hold;
        p_frame_win_list.n += 1;
        SACENC_OK
    }
}

/// `FrameWindowList_Remove(pFrameWinList, idx)` (static in C)
pub fn frame_window_list_remove(p_frame_win_list: &mut FrameWinList, idx: i32) -> FdkSacencError {
    if idx < 0 || idx >= MAX_NUM_PARAMS as i32 {
        SACENC_PARAM_ERROR
    } else {
        if p_frame_win_list.n > 0 {
            let idx = idx as usize;
            if idx == MAX_NUM_PARAMS - 1 {
                p_frame_win_list.dat[idx].slot = -1;
                p_frame_win_list.dat[idx].hold = FW_INTP;
            } else {
                for k in idx..MAX_NUM_PARAMS - 1 {
                    p_frame_win_list.dat[k] = p_frame_win_list.dat[k + 1];
                }
            }
            p_frame_win_list.n -= 1;
        }
        SACENC_OK
    }
}

/// `FrameWindowList_Limit(pFrameWinList, ll, ul)` (static in C): removes the slots outside of
/// `ll..=ul`.
pub fn frame_window_list_limit(
    p_frame_win_list: &mut FrameWinList,
    ll: i32,
    ul: i32,
) -> FdkSacencError {
    let mut k = 0i32;
    while k < p_frame_win_list.n {
        // `n <= MAX_NUM_PARAMS` (the list only grows through FrameWindowList_Add).
        let slot = p_frame_win_list.dat.get(k as usize).map_or(-1, |d| d.slot);
        if slot < ll || slot > ul {
            frame_window_list_remove(p_frame_win_list, k);
            k -= 1;
        }
        k += 1;
    }
    SACENC_OK
}

/// `fdk_sacenc_frameWindow_GetWindow(hFrameWindow, tr_pos, timeSlots, pFramingInfo,
/// pWindowAna__FDK, pFrameWinList, avoid_keep)`: the analysis windows and the framing of a
/// frame with the transient positions `tr_pos`.
pub fn fdk_sacenc_frame_window_get_window<W: AsMut<[FixpWin]>>(
    h_frame_window: &FrameWindow,
    tr_pos: &[i32; MAX_NUM_PARAMS],
    time_slots: i32,
    p_framing_info: &mut FramingInfo,
    p_window_ana_fdk: &mut [W; MAX_NUM_PARAMS],
    p_frame_win_list: &mut FrameWinList,
    avoid_keep: i32,
) -> FdkSacencError {
    let var_type: VarType = VAR_HOLD;
    let tran_l = 4;
    let mut win_cnt: usize;

    let start_slope = h_frame_window.start_slope;
    let stop_slope = h_frame_window.stop_slope;
    let start_rect = h_frame_window.start_rect;
    let stop_rect = h_frame_window.stop_rect;
    let taper_ana_len = h_frame_window.taper_ana_len;

    let win_max_val: FixpWin = MAXVAL_DBL;
    let mut apply_right_window_gain_fdk = [0 as FixpWin; MAX_NUM_PARAMS];
    let p_taper_ana_fdk = &h_frame_window.p_taper_ana_fdk;

    if (time_slots > h_frame_window.n_time_slots_max) || (time_slots < 0) {
        return SACENC_INVALID_CONFIG;
    }

    // Reset
    let error = frame_win_list_reset(p_frame_win_list);
    if error != SACENC_OK {
        return error;
    }

    let mut error;
    if tr_pos[0] > -1 {
        // Transients in first (left) half?
        let p_l = tr_pos[0];
        win_cnt = 0;

        // Create Parameter Positions
        match var_type {
            VAR_HOLD => {
                error = frame_window_list_add(p_frame_win_list, p_l - 1, FW_HOLD);
                if error != SACENC_OK {
                    return error;
                }
                error = frame_window_list_add(p_frame_win_list, p_l, FW_INTP);
                if error != SACENC_OK {
                    return error;
                }
            }
            VAR_ISOLATE => {
                for (slot, hold) in [
                    (p_l - 1, FW_HOLD),
                    (p_l, FW_INTP),
                    (p_l + tran_l, FW_HOLD),
                    (p_l + tran_l + 1, FW_INTP),
                ] {
                    error = frame_window_list_add(p_frame_win_list, slot, hold);
                    if error != SACENC_OK {
                        return error;
                    }
                }
            }
            _ => {}
        }

        // Outside of frame? => Kick Out
        error = frame_window_list_limit(p_frame_win_list, 0, time_slots - 1);
        if error != SACENC_OK {
            return error;
        }

        // Add timeSlots as temporary border for window creation
        error = frame_window_list_add(p_frame_win_list, time_slots - 1, FW_HOLD);
        if error != SACENC_OK {
            return error;
        }

        // Create Windows
        let mut ps = 0usize;
        while (ps as i32) < p_frame_win_list.n - 1 {
            if FW_HOLD != p_frame_win_list.dat[ps].hold {
                let start = p_frame_win_list.dat[ps].slot;
                let stop = p_frame_win_list.dat[ps + 1].slot;

                // Analysis Window
                let w = p_window_ana_fdk[win_cnt].as_mut();
                set(w, 0, 0, start);
                set(w, start, win_max_val, stop - start + 1);
                set(w, stop + 1, 0, time_slots - stop - 1);

                // `pWindowAna__FDK[winCnt][timeSlots - 1]`; where the reference has no defined
                // result, the port reads 0.
                apply_right_window_gain_fdk[win_cnt] =
                    if time_slots >= 1 { w[(time_slots - 1) as usize] } else { 0 };
                win_cnt += 1;
            }
            ps += 1;
        }

        // Pop temporary frame border
        error = frame_window_list_remove(p_frame_win_list, p_frame_win_list.n - 1);
        if error != SACENC_OK {
            return error;
        }
    } else {
        // No transient in left half of ana. window
        win_cnt = 0;

        // Add paramter set at end of frame
        error = frame_window_list_add(p_frame_win_list, time_slots - 1, FW_INTP);
        if error != SACENC_OK {
            return error;
        }
        // Analysis Window
        let w = p_window_ana_fdk[win_cnt].as_mut();
        set(w, 0, 0, start_slope);
        for i in 0..taper_ana_len.max(0) {
            w[(start_slope + i) as usize] = p_taper_ana_fdk[i as usize];
        }
        set(w, start_rect, win_max_val, time_slots - start_rect);

        apply_right_window_gain_fdk[win_cnt] = win_max_val;
        win_cnt += 1;
    }

    for wi in 0..win_cnt {
        let w = p_window_ana_fdk[wi].as_mut();
        if apply_right_window_gain_fdk[wi] > 0 {
            if tr_pos[1] > -1 {
                // Transients in second (right) half?
                let p_r = tr_pos[1];

                // Analysis Window
                set(w, time_slots, win_max_val, p_r - time_slots);
                set(w, p_r, 0, 2 * time_slots - p_r);
            } else {
                // No transient in right half of ana. window
                // Analysis Window
                set(w, time_slots, win_max_val, stop_rect - time_slots + 1);
                for i in 0..taper_ana_len.max(0) {
                    w[(stop_rect + i) as usize] = p_taper_ana_fdk[(taper_ana_len - 1 - i) as usize];
                }
                set(w, stop_slope + 1, 0, 2 * time_slots - stop_slope - 1);
            }

            // Weight
            if apply_right_window_gain_fdk[wi] < win_max_val {
                for ts in 0..time_slots.max(0) as usize {
                    w[time_slots as usize + ts] =
                        f_mult(w[time_slots as usize + ts], apply_right_window_gain_fdk[wi]);
                }
            }
        } else {
            // All Zero
            set(w, time_slots, 0, time_slots);
        }
    }

    if h_frame_window.b_frame_keep == 1 {
        let w = p_window_ana_fdk[0].as_mut();
        let t = time_slots.max(0) as usize;
        for i in 0..t {
            w[2 * t + i] = w[t + i];
        }
        for i in 0..t {
            w[t + i] = w[i];
        }

        if avoid_keep != 0 {
            set(w, 0, 0, time_slots);
        } else {
            set(w, 0, win_max_val, time_slots);
        }
    }

    // Feed Info to Bitstream Formatter
    p_framing_info.num_param_sets = p_frame_win_list.n as u8;
    p_framing_info.bs_framing_type = 1; // variable framing
    for ps in 0..p_framing_info.num_param_sets as usize {
        p_framing_info.bs_param_slots[ps] = p_frame_win_list.dat[ps].slot as u8;
    }

    // if there is just one param set at last slot, use fixed framing to save some bits
    if (p_framing_info.num_param_sets == 1)
        && (p_framing_info.bs_param_slots[0] as i32 == time_slots - 1)
    {
        p_framing_info.bs_framing_type = 0;
    }

    SACENC_OK
}

/// `fdk_sacenc_analysisWindowing(nTimeSlots, startTimeSlot, pFrameWindowAna__FDK,
/// ppDataIn__FDK, ppDataOut__FDK, nHybridBands, dim)` with separate input and output.
pub fn fdk_sacenc_analysis_windowing<R: AsRef<[FixpDpk]>, W: AsMut<[FixpDpk]>>(
    n_time_slots: i32,
    start_time_slot: i32,
    p_frame_window_ana_fdk: &[FixpWin],
    pp_data_in_fdk: &[R],
    pp_data_out_fdk: &mut [W],
    n_hybrid_bands: i32,
    dim: i32,
) -> FdkSacencError {
    let max_val: FixpWin = MAXVAL_DBL;
    let nb = n_hybrid_bands.max(0) as usize;

    for ts in start_time_slot.max(0) as usize..n_time_slots.max(0) as usize {
        let win = p_frame_window_ana_fdk[ts];
        let input = pp_data_in_fdk[ts].as_ref();
        for i in 0..nb {
            let v = if win == max_val {
                input[i]
            } else {
                FixpDpk::new(f_mult(win, input[i].re), f_mult(win, input[i].im))
            };
            if dim == FW_CHANGE_DIM {
                pp_data_out_fdk[i].as_mut()[ts] = v;
            } else {
                pp_data_out_fdk[ts].as_mut()[i] = v;
            }
        }
    }

    SACENC_OK
}

/// `fdk_sacenc_analysisWindowing()` with `ppDataOut__FDK == ppDataIn__FDK` and
/// `FW_LEAVE_DIM`, as the space tree calls it (in place: every value is read and then written
/// at the same position).
pub fn fdk_sacenc_analysis_windowing_in_place<W: AsMut<[FixpDpk]>>(
    n_time_slots: i32,
    start_time_slot: i32,
    p_frame_window_ana_fdk: &[FixpWin],
    pp_data: &mut [W],
    n_hybrid_bands: i32,
) -> FdkSacencError {
    let max_val: FixpWin = MAXVAL_DBL;
    let nb = n_hybrid_bands.max(0) as usize;

    for ts in start_time_slot.max(0) as usize..n_time_slots.max(0) as usize {
        let win = p_frame_window_ana_fdk[ts];
        if win != max_val {
            for v in pp_data[ts].as_mut()[..nb].iter_mut() {
                v.re = f_mult(win, v.re);
                v.im = f_mult(win, v.im);
            }
        }
    }

    SACENC_OK
}
