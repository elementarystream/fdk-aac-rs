// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/sbrdec_drc.h, libSBRdec/src/sbrdec_drc.cpp.

//! Port of `sbrdec_drc.h` and `sbrdec_drc.cpp`: the dynamic range control (DRC) tool of the SBR
//! decoder, which applies the DRC gains of the AAC core in the QMF domain.

pub mod tables;
use tables::WIN_BORDER_TO_COL_MAPPING_TAB;

use crate::sbr_ram::pod_struct;
use fdk_aac_rs_fdk::common_fix::*;

/// `SBRDEC_MAX_DRC_CHANNELS`
pub const SBRDEC_MAX_DRC_CHANNELS: usize = 8;
/// `SBRDEC_MAX_DRC_BANDS`
pub const SBRDEC_MAX_DRC_BANDS: usize = 16;

pod_struct! {
    /// `SBRDEC_DRC_CHANNEL`
    pub struct SbrdecDrcChannel {
        /// `prevFact_mag`
        pub prev_fact_mag: [FixpDbl; 64],
        /// `prevFact_exp`
        pub prev_fact_exp: i32,
        /// `currFact_mag`
        pub curr_fact_mag: [FixpDbl; SBRDEC_MAX_DRC_BANDS],
        /// `nextFact_mag`
        pub next_fact_mag: [FixpDbl; SBRDEC_MAX_DRC_BANDS],
        /// `currFact_exp`
        pub curr_fact_exp: i32,
        /// `nextFact_exp`
        pub next_fact_exp: i32,
        /// `numBandsCurr`
        pub num_bands_curr: u32,
        /// `numBandsNext`
        pub num_bands_next: u32,
        /// `bandTopCurr`
        pub band_top_curr: [u16; SBRDEC_MAX_DRC_BANDS],
        /// `bandTopNext`
        pub band_top_next: [u16; SBRDEC_MAX_DRC_BANDS],
        /// `drcInterpolationSchemeCurr`
        pub drc_interpolation_scheme_curr: i16,
        /// `drcInterpolationSchemeNext`
        pub drc_interpolation_scheme_next: i16,
        /// `enable`
        pub enable: i16,
        /// `winSequenceCurr`
        pub win_sequence_curr: u8,
        /// `winSequenceNext`
        pub win_sequence_next: u8,
    }
}

/// `sbrDecoder_drcInitChannel(hDrcData)`: initializes the DRC QMF factors.
pub fn sbr_decoder_drc_init_channel(h_drc_data: Option<&mut SbrdecDrcChannel>) {
    let Some(h) = h_drc_data else {
        return;
    };

    h.prev_fact_mag.fill(fl2fxconst_dbl(0.5f32 as f64));

    h.curr_fact_mag.fill(fl2fxconst_dbl(0.5f32 as f64));
    h.next_fact_mag.fill(fl2fxconst_dbl(0.5f32 as f64));

    h.prev_fact_exp = 1;
    h.curr_fact_exp = 1;
    h.next_fact_exp = 1;

    h.num_bands_curr = 1;
    h.num_bands_next = 1;

    h.win_sequence_curr = 0;
    h.win_sequence_next = 0;

    h.drc_interpolation_scheme_curr = 0;
    h.drc_interpolation_scheme_next = 0;

    h.enable = 0;
}

/// `sbrDecoder_drcUpdateChannel(hDrcData)`: swaps the DRC QMF scaling factors after they have
/// been applied.
pub fn sbr_decoder_drc_update_channel(h_drc_data: Option<&mut SbrdecDrcChannel>) {
    let Some(h) = h_drc_data else {
        return;
    };
    if h.enable != 1 {
        return;
    }

    // swap previous data
    h.curr_fact_mag = h.next_fact_mag;

    h.curr_fact_exp = h.next_fact_exp;

    h.num_bands_curr = h.num_bands_next;

    h.band_top_curr = h.band_top_next;

    h.drc_interpolation_scheme_curr = h.drc_interpolation_scheme_next;

    h.win_sequence_curr = h.win_sequence_next;
}

/// Which factor set of the channel a slot uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fact {
    Curr,
    Next,
}

/// `sbrDecoder_drcApplySlot()`: applies the DRC factors to one QMF slot. `qmf_imag_slot` is
/// `None` (`NULL`) in low power mode.
///
/// Band and window indices outside the tables or slots, for which the reference has no defined
/// result, are skipped by the port.
pub fn sbr_decoder_drc_apply_slot(
    h_drc_data: Option<&mut SbrdecDrcChannel>,
    qmf_real_slot: &mut [FixpDbl],
    qmf_imag_slot: Option<&mut [FixpDbl]>,
    col: i32,
    num_qmf_sub_samples: i32,
    max_shift: i32,
) {
    let mut slot = SplitSlot { re: qmf_real_slot, im: qmf_imag_slot };
    apply_slot_core(h_drc_data, &mut slot, col, num_qmf_sub_samples, max_shift);
}

/// `sbrDecoder_drcApplySlot()` for the right channel of parametric stereo in `sbr_dec()`,
/// whose real and imaginary slots are `pWorkBuffer` and `pWorkBuffer + no_channels`: with
/// fewer than 64 synthesis channels, the real values above `no_channels` are the imaginary
/// values, which the reference then scales twice (in the order of the bins).
pub fn sbr_decoder_drc_apply_slot_shared(
    h_drc_data: Option<&mut SbrdecDrcChannel>,
    work_buffer: &mut [FixpDbl],
    imag_offset: usize,
    col: i32,
    num_qmf_sub_samples: i32,
    max_shift: i32,
) {
    let mut slot = SharedSlot { buf: work_buffer, im_off: imag_offset };
    apply_slot_core(h_drc_data, &mut slot, col, num_qmf_sub_samples, max_shift);
}

/// Access to the bins of one QMF slot (real part, then imaginary part).
trait DrcSlot {
    /// `qmfRealSlot[bin] = fMult(qmfRealSlot[bin], f)`, then the same for the imaginary part
    /// (if any); positions outside the slot are skipped.
    fn scale_bin(&mut self, bin: usize, f: FixpDbl);
}

/// Separate real and imaginary slots.
struct SplitSlot<'a> {
    re: &'a mut [FixpDbl],
    im: Option<&'a mut [FixpDbl]>,
}

impl DrcSlot for SplitSlot<'_> {
    #[inline]
    fn scale_bin(&mut self, bin: usize, f: FixpDbl) {
        if let Some(v) = self.re.get_mut(bin) {
            *v = f_mult(*v, f);
        }
        if let Some(im) = self.im.as_deref_mut() {
            if let Some(v) = im.get_mut(bin) {
                *v = f_mult(*v, f);
            }
        }
    }
}

/// Real slot at `buf[0]` and imaginary slot at `buf[im_off]` of one buffer.
struct SharedSlot<'a> {
    buf: &'a mut [FixpDbl],
    im_off: usize,
}

impl DrcSlot for SharedSlot<'_> {
    #[inline]
    fn scale_bin(&mut self, bin: usize, f: FixpDbl) {
        if let Some(v) = self.buf.get_mut(bin) {
            *v = f_mult(*v, f);
        }
        if let Some(v) = self.buf.get_mut(self.im_off + bin) {
            *v = f_mult(*v, f);
        }
    }
}

fn apply_slot_core<S: DrcSlot>(
    h_drc_data: Option<&mut SbrdecDrcChannel>,
    slot: &mut S,
    col: i32,
    num_qmf_sub_samples: i32,
    max_shift: i32,
) {
    let Some(h) = h_drc_data else {
        return;
    };
    if h.enable != 1 {
        return;
    }

    let indx = num_qmf_sub_samples - (num_qmf_sub_samples >> 1) - 10; // l_border
    let frame_len_flag = if num_qmf_sub_samples == 30 { 1 } else { 0 };
    let frame_size = if frame_len_flag == 1 { 960 } else { 1024 };

    let mut alpha_value: FixpDbl = 0;
    let mut short_drc = false;

    let win_border_to_col_map = &WIN_BORDER_TO_COL_MAPPING_TAB[frame_len_flag];
    let map =
        |i: i32| -> i32 { win_border_to_col_map.get(i as usize).copied().unwrap_or(0) as i32 };

    let mut col = col + indx;
    let mut bottom_mdct = 0i32;

    // interpolation factor of a long window: j * (1/frame of 32 or 30 slots), or a step at the
    // border of the interpolation scheme
    let long_alpha = |j: i32, scheme: i16| -> FixpDbl {
        if j < map(15) {
            if scheme == 0 {
                let k: i32 = if frame_len_flag != 0 { 0x4444445 } else { 0x4000000 };
                j.wrapping_mul(k)
            } else if j >= map(scheme as i32) {
                MAXVAL_DBL
            } else {
                0
            }
        } else {
            MAXVAL_DBL
        }
    };

    // get respective data and calc interpolation factor
    let fact = if col < (num_qmf_sub_samples >> 1) {
        // first half of current frame
        if h.win_sequence_curr != 2 {
            // long window
            let j = col + (num_qmf_sub_samples >> 1);
            alpha_value = long_alpha(j, h.drc_interpolation_scheme_curr);
        } else {
            // short windows
            short_drc = true;
        }
        Fact::Curr
    } else if col < num_qmf_sub_samples {
        // second half of current frame
        if h.win_sequence_next != 2 {
            // next: long window
            let j = col - (num_qmf_sub_samples >> 1);
            alpha_value = long_alpha(j, h.drc_interpolation_scheme_next);
            Fact::Next
        } else {
            // next: short windows
            if h.win_sequence_curr != 2 {
                // current: long window
                alpha_value = 0;
                Fact::Next
            } else {
                // current: short windows
                short_drc = true;
                Fact::Curr
            }
        }
    } else {
        // first half of next frame
        if h.win_sequence_next != 2 {
            // long window
            let j = col - (num_qmf_sub_samples >> 1);
            alpha_value = long_alpha(j, h.drc_interpolation_scheme_next);
        } else {
            // short windows
            short_drc = true;
        }

        col -= num_qmf_sub_samples;
        Fact::Next
    };

    let (fact_mag, fact_exp, num_bands, band_top) = match fact {
        Fact::Curr => (h.curr_fact_mag, h.curr_fact_exp, h.num_bands_curr, h.band_top_curr),
        Fact::Next => (h.next_fact_mag, h.next_fact_exp, h.num_bands_next, h.band_top_next),
    };

    // numBands <= SBRDEC_MAX_DRC_BANDS (sbrDecoder_drcFeedChannel())
    let num_bands = (num_bands as usize).min(SBRDEC_MAX_DRC_BANDS) as i32;

    // process bands
    for band in 0..num_bands {
        let bu = band as usize;
        let mut top_mdct = (band_top[bu] as i32 + 1) << 2;

        if !short_drc {
            // long window
            let bottom_qmf;
            let mut top_qmf;
            if frame_len_flag != 0 {
                // 960 framing
                bottom_qmf = f_mult_i_floor(0x4444445, bottom_mdct);
                top_qmf = f_mult_i_floor(0x4444445, top_mdct);

                top_mdct = 30 * top_qmf;
            } else {
                // 1024 framing
                top_mdct &= !0x1f;

                bottom_qmf = bottom_mdct >> 5;
                top_qmf = top_mdct >> 5;
            }

            if band == num_bands - 1 {
                top_qmf = 64;
            }

            // DEVIATION: the bins are limited to the 64 entries of prevFact_mag[] and of the QMF
            // slots; beyond them the reference has no defined result.
            for bin in bottom_qmf.max(0)..top_qmf.min(64) {
                let bin = bin as usize;
                let mut drc_fact1_mag = h.prev_fact_mag[bin];
                let mut drc_fact2_mag = fact_mag[bu];

                // normalize scale factors
                if h.prev_fact_exp < max_shift {
                    drc_fact1_mag =
                        drc_fact1_mag.wrapping_shr((max_shift - h.prev_fact_exp) as u32);
                }
                if fact_exp < max_shift {
                    drc_fact2_mag = drc_fact2_mag.wrapping_shr((max_shift - fact_exp) as u32);
                }

                // interpolate
                let drc_fact_mag = if alpha_value == 0 {
                    drc_fact1_mag
                } else if alpha_value == MAXVAL_DBL {
                    drc_fact2_mag
                } else {
                    f_mult(alpha_value, drc_fact2_mag)
                        .wrapping_add(f_mult(MAXVAL_DBL.wrapping_sub(alpha_value), drc_fact1_mag))
                };

                // apply scaling
                slot.scale_bin(bin, drc_fact_mag);

                // save previous factors
                if col == (num_qmf_sub_samples >> 1) - 1 {
                    h.prev_fact_mag[bin] = fact_mag[bu];
                }
            }
        } else {
            // short windows
            let inv_frame_size_div8: FixpDbl =
                if frame_len_flag != 0 { 0x1111112 } else { 0x1000000 };

            // limit top at the frame borders
            if top_mdct < 0 {
                top_mdct = 0;
            }
            if top_mdct >= frame_size {
                top_mdct = frame_size - 1;
            }

            let start_win_idx: u32;
            let mut stop_win_idx: u32;
            if frame_len_flag != 0 {
                // 960 framing
                top_mdct = f_mult_i_floor(0x78000000, f_mult_i_floor(0x22222223, top_mdct) << 2);

                // winBorderToColMap table has offset of 1
                start_win_idx = (f_mult_i_floor(inv_frame_size_div8, bottom_mdct) + 1) as u32;
                stop_win_idx = (f_mult_i_ceil(inv_frame_size_div8 - 1, top_mdct) + 1) as u32;
            } else {
                // 1024 framing
                top_mdct &= !0x03;

                start_win_idx = (f_mult_i_floor(inv_frame_size_div8, bottom_mdct) + 1) as u32;
                stop_win_idx = (f_mult_i_ceil(inv_frame_size_div8, top_mdct) + 1) as u32;
            }

            // startCol is truncated to the nearest corresponding start subsample in the QMF of
            // the short window bottom is present in:
            let start_col = map(start_win_idx as i32);

            // stopCol is rounded upwards to the nearest corresponding stop subsample in the QMF
            // of the short window top is present in.
            let mut stop_col = map(stop_win_idx as i32);

            let mut bottom_qmf = f_mult_i_floor(
                inv_frame_size_div8,
                (bottom_mdct % (num_qmf_sub_samples << 2)) << 5,
            );
            let mut top_qmf =
                f_mult_i_floor(inv_frame_size_div8, (top_mdct % (num_qmf_sub_samples << 2)) << 5);

            // extend last band
            if band == num_bands - 1 {
                top_qmf = 64;
                stop_col = num_qmf_sub_samples;
                stop_win_idx = 10;
            }

            if top_qmf == 0 {
                if frame_len_flag != 0 {
                    let rem = f_mult(inv_frame_size_div8, top_mdct << (DFRACT_BITS - 12));
                    if rem & 0x1F != 0 {
                        stop_win_idx = stop_win_idx.wrapping_sub(1);
                        stop_col = map(stop_win_idx as i32);
                    }
                }
                top_qmf = 64;
            }

            // save previous factors
            if stop_col == num_qmf_sub_samples {
                let mut tmp_bottom = bottom_qmf;

                if map(8) > start_col {
                    tmp_bottom = 0; // band starts in previous short window
                }

                for bin in tmp_bottom.max(0)..top_qmf.min(64) {
                    h.prev_fact_mag[bin as usize] = fact_mag[bu];
                }
            }

            // apply
            if col >= start_col && col < stop_col {
                if col >= map(start_win_idx as i32 + 1) {
                    bottom_qmf = 0; // band starts in previous short window
                }
                if col < map(stop_win_idx as i32 - 1) {
                    top_qmf = 64; // band ends in next short window
                }

                let mut drc_fact_mag = fact_mag[bu];

                // normalize scale factor
                if fact_exp < max_shift {
                    drc_fact_mag = drc_fact_mag.wrapping_shr((max_shift - fact_exp) as u32);
                }

                // apply scaling
                for bin in bottom_qmf.max(0)..top_qmf.min(64) {
                    let bin = bin as usize;
                    slot.scale_bin(bin, drc_fact_mag);
                }
            }
        }

        bottom_mdct = top_mdct;
    } // end of bands loop

    if col == (num_qmf_sub_samples >> 1) - 1 {
        h.prev_fact_exp = fact_exp;
    }
}

/// `sbrDecoder_drcApply(hDrcData, QmfBufferReal, QmfBufferImag, numQmfSubSamples,
/// scaleFactor)`: applies the DRC factors to a whole frame. `qmf_buffer_imag` is empty in low
/// power mode (`NULL`).
pub fn sbr_decoder_drc_apply(
    h_drc_data: Option<&mut SbrdecDrcChannel>,
    qmf_buffer_real: &mut [&mut [FixpDbl]],
    qmf_buffer_imag: &mut [&mut [FixpDbl]],
    num_qmf_sub_samples: i32,
    scale_factor: &mut i32,
) {
    let Some(h) = h_drc_data else {
        return;
    };
    if h.enable == 0 {
        return; // Avoid changing the scaleFactor even though the processing is disabled.
    }

    // get max scale factor
    let mut max_shift = 0;
    if h.prev_fact_exp > max_shift {
        max_shift = h.prev_fact_exp;
    }
    if h.curr_fact_exp > max_shift {
        max_shift = h.curr_fact_exp;
    }
    if h.next_fact_exp > max_shift {
        max_shift = h.next_fact_exp;
    }

    let use_imag = !qmf_buffer_imag.is_empty();
    for col in 0..num_qmf_sub_samples.max(0) as usize {
        let qmf_slot_imag =
            if use_imag { qmf_buffer_imag.get_mut(col).map(|s| &mut **s) } else { None };
        let Some(qmf_slot_real) = qmf_buffer_real.get_mut(col) else {
            break;
        };

        sbr_decoder_drc_apply_slot(
            Some(h),
            qmf_slot_real,
            qmf_slot_imag,
            col as i32,
            num_qmf_sub_samples,
            max_shift,
        );
    }

    *scale_factor += max_shift;
}
