// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_reshapeBBEnv.h, libSACdec/src/sac_reshapeBBEnv.cpp.

//! Port of `sac_reshapeBBEnv.h` and `sac_reshapeBBEnv.cpp`: guided envelope shaping (GES).
//!
//! The `aarch64` build declares `shapeBBEnv()` `__attribute__((noinline))`; this changes no
//! value (PORTING.md, "Function-level variants").
//!
//! # Undefined behavior
//!
//! The energy sums and the shifts of the mantissas (`<< maxValSF`, `<< scaleCur`, `<< 1`, ...)
//! can overflow for extreme signals; both builds compile them as 32 bit wrapping operations
//! (the port uses `wrapping_*`). All run-time shift amounts are limited to `0..=31` by the
//! code itself, except where noted.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::scale::saturate_left_shift;

use crate::sac_dec::*;
use crate::sac_dec_interface::*;
use crate::sac_rom::*;

/// `BB_ENV_SIZE`: `END_BB_ENV - START_BB_ENV`
pub const BB_ENV_SIZE: usize = 9;

/// `INP_DRY_WET`
const INP_DRY_WET: i32 = 0;
/// `INP_DMX`
const INP_DMX: i32 = 1;

/// `SF_SHAPE`
const SF_SHAPE: i32 = 1;
/// `SF_DIV32`
const SF_DIV32: i32 = 6;
/// `SF_FACTOR_SLOT`
const SF_FACTOR_SLOT: i32 = 5;

/// `START_BB_ENV`
const START_BB_ENV: usize = 0;
/// `END_BB_ENV`
const END_BB_ENV: usize = 9;

/// `SF_ALPHA1`
const SF_ALPHA1: i32 = 8;
/// `SF_BETA1`
const SF_BETA1: i32 = 4;

/// `initBBEnv(self, initStatesFlag)`
pub fn init_bb_env(self_: &mut SpatialDec, init_states_flag: i32) {
    for ch in 0..(self_.num_output_channels.max(0) as usize).min(MAX_OUTPUT_CHANNELS) {
        let k = row2channel_ges(self_.tree_config, ch);
        self_.row2channel_dmx_ges[ch] = k;
        if k == -1 {
            continue;
        }

        if self_.tree_config == crate::sac_bitdec::TREE_212 {
            self_.row2channel_dmx_ges[ch] = 0;
        }
    }

    if init_states_flag != 0 {
        let st = &mut self_.reshape_bb_env_state;
        for k in 0..BB_ENV_CHANNELS {
            st.norm_nrg_prev__fdk[k] = fl2fxconst_dbl(0.5); // 32768.f*32768.f
            st.norm_nrg_prev_sf[k] = DFRACT_BITS - 1;
            st.part_nrg_prev_sf[k] = 0;
            st.part_nrg_prev2_sf[k] = 0;
            st.frame_nrg_prev_sf[k] = 0;
        }
    }

    self_.reshape_bb_env_state.alpha__fdk = fl2fxconst_dbl(0.99637845575f32 as f64); // FDKexp(-64 / (0.4f  * 44100))
    self_.reshape_bb_env_state.beta__fdk = fl2fxconst_dbl(0.96436909488f32 as f64);
    // FDKexp(-64 / (0.04f * 44100))
}

/// `row2channelGES[treeConfig][ch]` (deviation: entries outside of the table give -1).
fn row2channel_ges(tree_config: i32, ch: usize) -> i8 {
    ROW2CHANNEL_GES.get(tree_config as usize).and_then(|r| r.get(ch)).copied().unwrap_or(-1)
}

/// One term `fPow2Div2(x << maxValSF) >> (SF_FACTOR_SLOT - 1)` of `getSlotNrgHQ()`.
#[inline(always)]
fn nrg_term(x: FixpDbl, max_val_sf: i32) -> FixpDbl {
    f_pow2_div2(x.wrapping_shl(max_val_sf as u32)) >> (SF_FACTOR_SLOT - 1)
}

/// `getSlotNrgHQ(pReal, pImag, slotNrg, maxValSF, hybBands)`: `p_real`/`p_imag` start at hybrid
/// band 12.
fn get_slot_nrg_hq(
    p_real: &[FixpDbl],
    p_imag: &[FixpDbl],
    slot_nrg: &mut [FixpDbl; BB_ENV_SIZE],
    max_val_sf: i32,
    hyb_bands: i32,
) {
    let mut q = 0usize;
    let mut term = || {
        let t = nrg_term(p_real[q], max_val_sf).wrapping_add(nrg_term(p_imag[q], max_val_sf));
        q += 1;
        t
    };

    // qs = 12, 13, 14
    slot_nrg[0] = term();
    slot_nrg[1] = term();
    slot_nrg[2] = term();
    // qs = 15
    slot_nrg[3] = term();
    // qs = 16, 17
    let nrg = term();
    slot_nrg[4] = nrg.wrapping_add(term());
    // qs = 18, 19, 20
    let mut nrg = term();
    nrg = nrg.wrapping_add(term());
    slot_nrg[5] = nrg.wrapping_add(term());
    // qs = 21, 22
    let nrg = term();
    slot_nrg[6] = nrg.wrapping_add(term());
    // qs = 23, 24
    if hyb_bands > 23 {
        slot_nrg[6] = slot_nrg[6].wrapping_add(term());
        slot_nrg[6] = slot_nrg[6].wrapping_add(term());
        // qs = 25, 26, 29, 28, 29
        let mut nrg = term();
        nrg = nrg.wrapping_add(term());
        nrg = nrg.wrapping_add(term());
        nrg = nrg.wrapping_add(term());
        slot_nrg[7] = nrg.wrapping_add(term());
        // qs = 30 ... min(41,hybBands-1)
        let mut nrg = term();
        for _qs in 31..hyb_bands {
            nrg = nrg.wrapping_add(term());
        }
        slot_nrg[8] = nrg;
    } else {
        slot_nrg[7] = 0;
        slot_nrg[8] = 0;
    }
}

/// `combineDryWet(pReal, pImag, pHybOutputRealDry, pHybOutputImagDry, pHybOutputRealWet,
/// pHybOutputImagWet, cplxBands, hybBands)`
fn combine_dry_wet(
    p_real: &mut [FixpDbl],
    p_imag: &mut [FixpDbl],
    p_hyb_output_real_dry: &[FixpDbl],
    p_hyb_output_imag_dry: &[FixpDbl],
    p_hyb_output_real_wet: &[FixpDbl],
    p_hyb_output_imag_wet: &[FixpDbl],
    cplx_bands: i32,
    hyb_bands: i32,
) {
    let mut qs = 12usize;
    while (qs as i32) < cplx_bands {
        p_real[qs] = (p_hyb_output_real_dry[qs] >> 1).wrapping_add(p_hyb_output_real_wet[qs] >> 1);
        p_imag[qs] = (p_hyb_output_imag_dry[qs] >> 1).wrapping_add(p_hyb_output_imag_wet[qs] >> 1);
        qs += 1;
    }
    while (qs as i32) < hyb_bands {
        p_real[qs] = (p_hyb_output_real_dry[qs] >> 1).wrapping_add(p_hyb_output_real_wet[qs] >> 1);
        qs += 1;
    }
}

/// `slotAmp(slotAmp_dry, slotAmp_dry_e, slotAmp_wet, slotAmp_wet_e, pHybOutputRealDry,
/// pHybOutputImagDry, pHybOutputRealWet, pHybOutputImagWet, cplxBands, hybBands)`. Returns
/// `(slotAmp_dry, slotAmp_dry_e, slotAmp_wet, slotAmp_wet_e)`.
fn slot_amp(
    p_hyb_output_real_dry: &[FixpDbl],
    p_hyb_output_imag_dry: &[FixpDbl],
    p_hyb_output_real_wet: &[FixpDbl],
    p_hyb_output_imag_wet: &[FixpDbl],
    cplx_bands: i32,
    hyb_bands: i32,
) -> (FixpDbl, i32, FixpDbl, i32) {
    let hb = hyb_bands.max(0) as usize;
    let cb = cplx_bands.max(0) as usize;

    // headroom can be reduced by 1 bit due to use of fPow2Div2
    let mut s1 = DFRACT_BITS - 1 - cnt_leading_zeros(hyb_bands + cplx_bands);
    let headroom_dry = f_min(
        get_scalefactor(&p_hyb_output_real_dry[..hb]),
        get_scalefactor(&p_hyb_output_imag_dry[..cb]),
    );
    let headroom_wet = f_min(
        get_scalefactor(&p_hyb_output_real_wet[..hb]),
        get_scalefactor(&p_hyb_output_imag_wet[..cb]),
    );

    let mut dry: FixpDbl = 0;
    let mut wet: FixpDbl = 0;
    let t = |x: FixpDbl, h: i32, s1: i32| f_pow2_div2(x.wrapping_shl(h as u32)) >> s1;
    let mut qs = 0usize;
    while qs < cb {
        // sum up dry part
        dry = dry.wrapping_add(t(p_hyb_output_real_dry[qs], headroom_dry, s1));
        dry = dry.wrapping_add(t(p_hyb_output_imag_dry[qs], headroom_dry, s1));
        // sum up wet part
        wet = wet.wrapping_add(t(p_hyb_output_real_wet[qs], headroom_wet, s1));
        wet = wet.wrapping_add(t(p_hyb_output_imag_wet[qs], headroom_wet, s1));
        qs += 1;
    }
    while qs < hb {
        dry = dry.wrapping_add(t(p_hyb_output_real_dry[qs], headroom_dry, s1));
        wet = wet.wrapping_add(t(p_hyb_output_real_wet[qs], headroom_wet, s1));
        qs += 1;
    }

    // consider fPow2Div2()
    s1 += 1;

    // normalize dry part, ensure that exponent is even
    let s2 = fix_max(0, cnt_leading_zeros(dry) - 1);
    let mut slot_amp_dry = dry.wrapping_shl(s2 as u32);
    let mut slot_amp_dry_e = s1 - s2 - 2 * headroom_dry;
    if slot_amp_dry_e & 1 != 0 {
        slot_amp_dry >>= 1;
        slot_amp_dry_e += 1;
    }

    // normalize wet part, ensure that exponent is even
    let s2 = fix_max(0, cnt_leading_zeros(wet) - 1);
    let mut slot_amp_wet = wet.wrapping_shl(s2 as u32);
    let mut slot_amp_wet_e = s1 - s2 - 2 * headroom_wet;
    if slot_amp_wet_e & 1 != 0 {
        slot_amp_wet >>= 1;
        slot_amp_wet_e += 1;
    }

    (slot_amp_dry, slot_amp_dry_e, slot_amp_wet, slot_amp_wet_e)
}

/// `shapeBBEnv(pHybOutputRealDry, pHybOutputImagDry, dryFac, scale, cplxBands, hybBands)`
fn shape_bb_env(
    p_hyb_output_real_dry: &mut [FixpDbl],
    p_hyb_output_imag_dry: &mut [FixpDbl],
    dry_fac: FixpDbl,
    scale: i32,
    cplx_bands: i32,
    hyb_bands: i32,
) {
    let cb = cplx_bands.max(0) as usize;
    let hb = hyb_bands.max(0) as usize;

    if scale == 0 {
        let mut qs = 0;
        while qs < cb {
            p_hyb_output_real_dry[qs] = f_mult_div2(p_hyb_output_real_dry[qs], dry_fac);
            p_hyb_output_imag_dry[qs] = f_mult_div2(p_hyb_output_imag_dry[qs], dry_fac);
            qs += 1;
        }
        while qs < hb {
            p_hyb_output_real_dry[qs] = f_mult_div2(p_hyb_output_real_dry[qs], dry_fac);
            qs += 1;
        }
    } else {
        // scale is in 1..=31 (the caller limits it).
        let mut qs = 0;
        while qs < cb {
            p_hyb_output_real_dry[qs] = saturate_left_shift(
                f_mult_div2(p_hyb_output_real_dry[qs], dry_fac),
                scale,
                DFRACT_BITS,
            );
            p_hyb_output_imag_dry[qs] = saturate_left_shift(
                f_mult_div2(p_hyb_output_imag_dry[qs], dry_fac),
                scale,
                DFRACT_BITS,
            );
            qs += 1;
        }
        while qs < hb {
            p_hyb_output_real_dry[qs] = saturate_left_shift(
                f_mult_div2(p_hyb_output_real_dry[qs], dry_fac),
                scale,
                DFRACT_BITS,
            );
            qs += 1;
        }
    }
}

/// `value << s` for `s < 0` / `value >> s` otherwise with `-31 <= s <= 31` (the amounts of
/// `extractBBEnv()`).
#[inline(always)]
fn shift_signed(value: FixpDbl, s: i32) -> FixpDbl {
    if s < 0 {
        value.wrapping_shl((-s) as u32)
    } else {
        value >> s
    }
}

/// `extractBBEnv(self, inp, start, channels, pEnv, frame)`
fn extract_bb_env(
    self_: &mut SpatialDec,
    inp: i32,
    start: i32,
    channels: i32,
    p_env: &mut [FixpDbl],
    frame: &SpatialBsFrame,
) {
    // C_ALLOC_SCRATCH_START(pScratchBuffer, FIXP_DBL, (2 * 42 + MAX_PARAMETER_BANDS)): only
    // the parts written by combineDryWet() (bands 12..hybBands) are read.
    let mut p_scratch_real = [0 as FixpDbl; 42];
    let mut p_scratch_imag = [0 as FixpDbl; 42];
    let mut slot_nrg = [0 as FixpDbl; BB_ENV_SIZE];
    let mut res_pb = [0 as FixpDbl; END_BB_ENV - START_BB_ENV];
    let mut res_pb_sf = [0i32; END_BB_ENV - START_BB_ENV];

    let alpha = self_.reshape_bb_env_state.alpha__fdk;
    // FIXP_DBL alpha1 = (FL2FXCONST_DBL(1.0f) - alpha) << SF_ALPHA1;
    let alpha1 = MAXVAL_DBL.wrapping_sub(alpha).wrapping_shl(SF_ALPHA1 as u32);
    let beta = self_.reshape_bb_env_state.beta__fdk;
    // FIXP_DBL beta1 = (FL2FXCONST_DBL(1.0f) - beta) << SF_BETA1;
    let beta1 = MAXVAL_DBL.wrapping_sub(beta).wrapping_shl(SF_BETA1 as u32);
    let mut shape_activ = 1;
    let hyb_bands = fix_min(42, self_.hybrid_bands);
    let static_scale = self_.static_dec_scale + 1;
    let cplx_bands = fix_min(42, self_.hybrid_bands);

    for ch in start..channels {
        let prev_ch_offs: usize;
        let p_real: &[FixpDbl];
        let p_imag: &[FixpDbl];
        let cu = ch.max(0) as usize;

        if inp == INP_DRY_WET {
            let ch2 = row2channel_ges(self_.tree_config, cu);
            if ch2 == -1 {
                continue;
            } else if frame.temp_shape_enable_channel_ges
                [(ch2 as usize).min(MAX_OUTPUT_CHANNELS - 1)]
                != 0
            {
                shape_activ = 1;
            } else {
                shape_activ = 0;
            }
            prev_ch_offs = cu;
            let c = cu.min(NUM_OUT_CH - 1);
            combine_dry_wet(
                &mut p_scratch_real,
                &mut p_scratch_imag,
                &self_.hyb_output_real_dry__fdk[c],
                &self_.hyb_output_imag_dry__fdk[c],
                &self_.hyb_output_real_wet__fdk[c],
                &self_.hyb_output_imag_wet__fdk[c],
                cplx_bands,
                hyb_bands,
            );
            p_real = &p_scratch_real;
            p_imag = &p_scratch_imag;
        } else {
            prev_ch_offs = cu + self_.num_output_channels.max(0) as usize;
            let c = cu.min(NUM_IN_CH - 1);
            p_real = &self_.hyb_input_real__fdk[c];
            p_imag = &self_.hyb_input_imag__fdk[c];
        }
        // hybBands >= 23 for every configuration of the library (hybridBands >= 23).
        let r12 = &p_real[12..];
        let i12 = &p_imag[12..];
        let clz = f_min(
            get_scalefactor(&r12[..f_max(0, hyb_bands - 12) as usize]),
            get_scalefactor(&i12[..f_max(0, cplx_bands - 12) as usize]),
        );

        // calculate slot energy
        {
            // scale slotNrg: 2*(staticScale-clz) + SF_FACTOR_SLOT
            get_slot_nrg_hq(r12, i12, &mut slot_nrg, clz, fix_min(42, self_.hybrid_bands));
        }

        let st = &mut self_.reshape_bb_env_state;
        // prevChOffs < 2 * MAX_OUTPUT_CHANNELS + MAX_INPUT_CHANNELS for the 212 tree.
        let po = prev_ch_offs.min(BB_ENV_CHANNELS - 1);

        let slot_nrg_sf =
            2 * (static_scale - clz + if inp == INP_DRY_WET { 1 } else { 0 }) + SF_FACTOR_SLOT;
        let mut frame_nrg_sf =
            2 * (static_scale - clz + if inp == INP_DRY_WET { 1 } else { 0 }) + SF_FACTOR_SLOT;

        let part_nrg_sf = fix_max(
            slot_nrg_sf - SF_ALPHA1 + 1,
            st.part_nrg_prev_sf[po] - st.part_nrg_prev2_sf[po] + 1,
        );
        let mut scale_prev = fix_max(
            fix_min(part_nrg_sf - st.part_nrg_prev_sf[po], DFRACT_BITS - 1),
            -(DFRACT_BITS - 1),
        );
        let mut scale_cur = fix_max(
            fix_min(part_nrg_sf - slot_nrg_sf + SF_ALPHA1, DFRACT_BITS - 1),
            -(DFRACT_BITS - 1),
        );

        let mut max_val: FixpDbl = 0;
        let mut frame_nrg: FixpDbl = 0;
        let part_nrg = &mut st.part_nrg_prev__fdk[po];
        // partNrg and partNrgPrev are the same array: each band is read before it is written.
        if scale_cur < 0 && scale_prev < 0 {
            scale_cur = -scale_cur;
            scale_prev = -scale_prev;
            for pb in START_BB_ENV..END_BB_ENV {
                part_nrg[pb] = (f_mult_div2(alpha1, slot_nrg[pb])
                    .wrapping_shl(scale_cur as u32)
                    .wrapping_add(
                        f_mult_div2(alpha, part_nrg[pb]).wrapping_shl(scale_prev as u32),
                    ))
                .wrapping_shl(1);
                max_val |= part_nrg[pb];
                frame_nrg = frame_nrg.wrapping_add(slot_nrg[pb] >> 3);
            }
        } else if scale_cur >= 0 && scale_prev >= 0 {
            for pb in START_BB_ENV..END_BB_ENV {
                part_nrg[pb] = ((f_mult_div2(alpha1, slot_nrg[pb]) >> scale_cur)
                    .wrapping_add(f_mult_div2(alpha, part_nrg[pb]) >> scale_prev))
                .wrapping_shl(1);
                max_val |= part_nrg[pb];
                frame_nrg = frame_nrg.wrapping_add(slot_nrg[pb] >> 3);
            }
        } else if scale_cur < 0 && scale_prev >= 0 {
            scale_cur = -scale_cur;
            for pb in START_BB_ENV..END_BB_ENV {
                part_nrg[pb] = (f_mult_div2(alpha1, slot_nrg[pb])
                    .wrapping_shl(scale_cur as u32)
                    .wrapping_add(f_mult_div2(alpha, part_nrg[pb]) >> scale_prev))
                .wrapping_shl(1);
                max_val |= part_nrg[pb];
                frame_nrg = frame_nrg.wrapping_add(slot_nrg[pb] >> 3);
            }
        } else {
            // if ( (scaleCur >= 0) && (scalePrev < 0) )
            scale_prev = -scale_prev;
            for pb in START_BB_ENV..END_BB_ENV {
                part_nrg[pb] = ((f_mult_div2(alpha1, slot_nrg[pb]) >> scale_cur).wrapping_add(
                    f_mult_div2(alpha, part_nrg[pb]).wrapping_shl(scale_prev as u32),
                ))
                .wrapping_shl(1);
                max_val |= part_nrg[pb];
                frame_nrg = frame_nrg.wrapping_add(slot_nrg[pb] >> 3);
            }
        }
        let part_nrg = *part_nrg;

        // frameNrg /= (END_BB_ENV - START_BB_ENV); 0.88888888888f =
        // (1/(END_BB_ENV-START_BB_ENV)<<3; shift with 3 is compensated in loop above
        frame_nrg = f_mult(frame_nrg, fl2fxconst_dbl(0.88888888888f32 as f64));

        // store scalefactor and headroom for part nrg prev
        st.part_nrg_prev_sf[po] = part_nrg_sf;
        st.part_nrg_prev2_sf[po] = fix_max(0, cnt_leading_zeros(max_val) - 1);

        let common_scale = fix_max(frame_nrg_sf - SF_ALPHA1 + 1, st.frame_nrg_prev_sf[po] + 1);
        let scale_prev = fix_min(common_scale - st.frame_nrg_prev_sf[po], DFRACT_BITS - 1);
        let scale_cur = fix_min(common_scale - frame_nrg_sf + SF_ALPHA1, DFRACT_BITS - 1);
        frame_nrg_sf = common_scale;

        // scaleCur, scalePrev >= 1 here (commonScale is the maximum of both terms + 1).
        frame_nrg = ((f_mult_div2(alpha1, frame_nrg) >> scale_cur)
            .wrapping_add(f_mult_div2(alpha, st.frame_nrg_prev__fdk[po]) >> scale_prev))
        .wrapping_shl(1);

        let clz2 = fix_max(0, cnt_leading_zeros(frame_nrg) - 1);
        st.frame_nrg_prev__fdk[po] = frame_nrg.wrapping_shl(clz2 as u32);
        st.frame_nrg_prev_sf[po] = frame_nrg_sf - clz2;

        let mut env: FixpDbl = 0;

        let scale = clz2 + part_nrg_sf - frame_nrg_sf;
        let mut scale_min = DFRACT_BITS - 1;
        for pb in START_BB_ENV..END_BB_ENV {
            if (part_nrg[pb] | slot_nrg[pb]) != 0 {
                let mut sc = 0;
                let sn = fix_max(0, cnt_leading_zeros(slot_nrg[pb]) - 1);
                let inv_sqrt = inv_sqrt_norm2(part_nrg[pb], &mut sc);
                let mut res = f_mult(slot_nrg[pb].wrapping_shl(sn as u32), f_pow2(inv_sqrt));
                let s = fix_max(0, cnt_leading_zeros(res) - 1);
                res = res.wrapping_shl(s as u32);
                let sc = scale - (2 * sc - sn - s);
                scale_min = fix_min(scale_min, sc);
                res_pb[pb] = res;
                res_pb_sf[pb] = sc;
            } else {
                res_pb[pb] = 0;
                res_pb_sf[pb] = 0;
            }
        }

        scale_min = 4 - scale_min;

        for pb in START_BB_ENV..END_BB_ENV {
            let sc =
                fix_max(fix_min(res_pb_sf[pb] + scale_min, DFRACT_BITS - 1), -(DFRACT_BITS - 1));
            env = env.wrapping_add(shift_signed(res_pb[pb], sc));
        }

        env = f_mult_div2(env, st.frame_nrg_prev__fdk[po]);
        let env_sf = slot_nrg_sf + scale_min + 1;

        let common_scale = fix_max(env_sf - SF_BETA1 + 1, st.norm_nrg_prev_sf[po] + 1);
        let scale_prev = fix_min(common_scale - st.norm_nrg_prev_sf[po], DFRACT_BITS - 1);
        let scale_cur = fix_min(common_scale - env_sf + SF_BETA1, DFRACT_BITS - 1);

        let norm_nrg = ((f_mult_div2(beta1, env) >> scale_cur)
            .wrapping_add(f_mult_div2(beta, st.norm_nrg_prev__fdk[po]) >> scale_prev))
        .wrapping_shl(1);

        let clz3 = fix_max(0, cnt_leading_zeros(norm_nrg) - 1);
        st.norm_nrg_prev__fdk[po] = norm_nrg.wrapping_shl(clz3 as u32);
        st.norm_nrg_prev_sf[po] = common_scale - clz3;

        if shape_activ != 0 {
            if (env | norm_nrg) != 0 {
                let mut sn = 0;
                let se = fix_max(0, cnt_leading_zeros(env) - 1);
                let sc = common_scale + SF_DIV32 - env_sf + se;
                env = f_mult(
                    sqrt_fixp(env.wrapping_shl(se as u32) >> (sc & 0x1)),
                    inv_sqrt_norm2(norm_nrg, &mut sn),
                );

                let sc = fix_min((sc >> 1) - sn, DFRACT_BITS - 1);
                // sc >= -31 for the energies of the library; a larger left shift would be
                // undefined in C (the port takes the amount modulo 32).
                if sc < 0 {
                    env = env.wrapping_shl((-sc) as u32);
                } else {
                    env >>= sc;
                }
            }
            // env is scaled by SF_DIV32/2 bits
        }
        if let Some(e) = p_env.get_mut(cu) {
            *e = env;
        }
    }
}

/// `SpatialDecReshapeBBEnv(self, frame, ts)`
pub fn spatial_dec_reshape_bb_env(self_: &mut SpatialDec, frame: &SpatialBsFrame, ts: i32) {
    let mut env_dry = [0 as FixpDbl; MAX_OUTPUT_CHANNELS];
    let mut env_dmx = [0 as FixpDbl; 2];

    let hyb_bands = self_.hybrid_bands - 6;
    let cplx_bands = self_.hybrid_bands - 6;

    // extract downmix envelope(s)
    {
        let n = f_min(self_.num_input_channels, 2);
        extract_bb_env(self_, INP_DMX, 0, n, &mut env_dmx, frame);
    }

    // extract dry and wet envelopes
    let n_out = self_.num_output_channels;
    extract_bb_env(self_, INP_DRY_WET, 0, n_out, &mut env_dry, frame);

    for ch in 0..(n_out.max(0) as usize).min(MAX_OUTPUT_CHANNELS) {
        let ch2 = row2channel_ges(self_.tree_config, ch);
        if ch2 == -1 {
            continue;
        }

        let ch2 = (ch2 as usize).min(MAX_OUTPUT_CHANNELS - 1);
        if frame.temp_shape_enable_channel_ges[ch2] != 0 {
            // reshape dry and wet signals according to transmitted envelope

            // De-quantize GES data
            let ts_u = (ts.max(0) as usize).min(MAX_TIME_SLOTS as usize - 1);
            let shape_data = frame.bs_env_shape_data[ch2][ts_u];
            fdk_assert!(shape_data <= 4);
            fdk_assert!(self_.env_quant_mode == 0 || self_.env_quant_mode == 1);
            // Deviation: values outside of the table read 0.
            let env_shape = ENV_SHAPE_DATA_TABLE__FDK
                .get(shape_data as usize)
                .and_then(|r| r.get(self_.env_quant_mode as usize))
                .copied()
                .unwrap_or(0);
            let env_shape = fx_cfg2fx_dbl(env_shape);

            // get downmix channel
            let ch2d = self_.row2channel_dmx_ges[ch];

            // multiply ratio with dmx envelope; tmp is scaled by SF_DIV32/2+SF_SHAPE bits
            let tmp = if ch2d == 2 {
                f_mult_div2(env_shape, env_dmx[0]).wrapping_add(f_mult_div2(env_shape, env_dmx[1]))
            } else {
                f_mult(env_shape, env_dmx[(ch2d.max(0) as usize).min(1)])
            };

            // weighting factors
            let mut dry_fac_sf = 0;
            let mut slot_amp_sf = 0;
            let mut dry_fac: FixpDbl = 0;
            let mut slot_amp_ratio: FixpDbl = 0;

            // dryFac will be scaled by dryFacSF bits
            if env_dry[ch] != 0 {
                env_dry[ch] = inv_sqrt_norm2(env_dry[ch], &mut dry_fac_sf);
                dry_fac = f_mult_div2(tmp, f_pow2_div2(env_dry[ch])).wrapping_shl(2);
                dry_fac_sf = SF_SHAPE + 2 * dry_fac_sf;
            }

            // calculate slotAmp_dry and slotAmp_wet
            let (mut slot_amp_dry, slot_amp_dry_e, mut slot_amp_wet, slot_amp_wet_e) = slot_amp(
                &self_.hyb_output_real_dry__fdk[ch][6..],
                &self_.hyb_output_imag_dry__fdk[ch][6..],
                &self_.hyb_output_real_wet__fdk[ch][6..],
                &self_.hyb_output_imag_wet__fdk[ch][6..],
                cplx_bands,
                hyb_bands,
            );

            // exponents must be even due to subsequent square root calculation
            fdk_assert!((slot_amp_dry_e & 1) == 0 && (slot_amp_wet_e & 1) == 0);

            // slotAmp_ratio will be scaled by slotAmpSF bits
            if slot_amp_dry != 0 {
                slot_amp_wet = sqrt_fixp(slot_amp_wet);
                slot_amp_dry = inv_sqrt_norm2(slot_amp_dry, &mut slot_amp_sf);
                slot_amp_ratio = f_mult(slot_amp_wet, slot_amp_dry);
                slot_amp_sf = slot_amp_sf + (slot_amp_wet_e >> 1) - (slot_amp_dry_e >> 1);
            }

            // calculate common scale factor
            // scale is at least with 3 bits to avoid overflows when calculating dryFac
            let mut scale = fix_max(3, fix_max(dry_fac_sf, slot_amp_sf));
            dry_fac >>= fix_min(scale - dry_fac_sf, DFRACT_BITS - 1);
            slot_amp_ratio >>= fix_min(scale - slot_amp_sf, DFRACT_BITS - 1);

            // limit dryFac
            dry_fac = fix_max(
                fl2fxconst_dbl(0.25) >> fix_min(2 * scale, DFRACT_BITS - 1),
                f_mult(dry_fac, slot_amp_ratio)
                    .wrapping_sub(slot_amp_ratio >> fix_min(scale, DFRACT_BITS - 1))
                    .wrapping_add(dry_fac >> fix_min(scale, DFRACT_BITS - 1)),
            );
            // reduce shift bits by 3, because upper limit 4.0 is scaled with 3 bits
            dry_fac =
                fix_min(fl2fxconst_dbl(0.50) >> fix_min(2 * scale - 3, DFRACT_BITS - 1), dry_fac);
            scale = 2 * scale + 1;

            // improve precision for dryFac
            let sc = fix_max(0, cnt_leading_zeros(dry_fac) - 1);
            dry_fac = dry_fac.wrapping_shl(fix_min(scale, sc) as u32);
            scale -= fix_min(scale, sc);

            // shaping
            let (real, imag) =
                (&mut self_.hyb_output_real_dry__fdk[ch], &mut self_.hyb_output_imag_dry__fdk[ch]);
            shape_bb_env(
                &mut real[6..],
                &mut imag[6..],
                dry_fac,
                fix_min(scale, DFRACT_BITS - 1),
                cplx_bands,
                hyb_bands,
            );
        }
    }
}
