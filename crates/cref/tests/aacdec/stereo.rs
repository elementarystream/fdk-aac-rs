// Differential tests of fdk_aac_rs_aacdec::stereo against the C++ reference.
//
// CJointStereo_filterAndAdd() and CJointStereo_GenerateMSOutput() on random inputs;
// CJointStereo_Read on random states and bitstreams (M/S masks and complex prediction data);
// CJointStereo_ApplyMS (M/S and complex stereo prediction, long and short blocks, with and without
// the previous frame) and CJointStereo_ApplyIS on random states of the block test context
// (block_ctx.rs).

use crate::block::random_codebooks;
use crate::block_ctx::*;
use fdk_aac_rs_aacdec::aac_rom::{MDST_FILT_COEF_CURR, MDST_FILT_COEF_PREV};
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_aacdec::stereo::*;
use fdk_aac_rs_cref::aacdec_stereo as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;

/// Also compares the function-local tables INDICES_1, INDICES_2, INDICES_3, SUBTR_1, SUBTR_2
/// and SUBTR_3 (`indices_1[]` ... `subtr_3[]` of CJointStereo_filterAndAdd()) through the
/// function: each element selects the input sample of one product of the first three or the
/// last three outputs (`len` >= 7), and the inputs are random.
#[test]
fn filter_and_add() {
    let mut rng = Rng::new(0x57e0_0001);
    for _ in 0..20000 {
        let window_len = [128usize, 1024, 96, 768][(rng.next_u32() % 4) as usize];
        let len = rng.range(7, window_len as i32);
        let curr = (rng.next_u32() % 2) as i32;
        let row = if curr != 0 { rng.range(0, 19) } else { rng.range(0, 5) };
        let sh = rng.next_u32() % 32;
        let input: Vec<i32> = (0..window_len).map(|_| rng.i32() >> sh).collect();
        let out: Vec<i32> = (0..window_len).map(|_| rng.i32() >> sh).collect();
        // the library filters with the coefficients of the current frame (3 per row) for
        // isCurrent == 1 and with those of the previous frame (4 per row) for isCurrent == 0
        let is_current = curr;
        let mut ci = input.clone();
        let mut co = out.clone();
        unsafe { c::cref_stereo_filter_and_add(ci.as_mut_ptr(), len, window_len as i32, curr, row, co.as_mut_ptr(), is_current) };
        let coeff: &[i32] = if curr != 0 { &MDST_FILT_COEF_CURR[row as usize] } else { &MDST_FILT_COEF_PREV[row as usize] };
        let mut ro = out.clone();
        cjoint_stereo_filter_and_add(&input, len, window_len as i32, coeff, &mut ro, is_current as u8);
        assert_eq!(ro, co, "filterAndAdd(len {len}, curr {curr}, row {row}, isCurrent {is_current})");
    }
}

#[test]
fn generate_ms_output() {
    let mut rng = Rng::new(0x57e0_0002);
    for _ in 0..20000 {
        let n = 4 * rng.range(0, 24) as usize;
        let l: Vec<i32> = (0..n).map(|_| rng.i32()).collect();
        let r: Vec<i32> = (0..n).map(|_| rng.i32()).collect();
        let (ls, rs) = (rng.range(0, 31) as u32, rng.range(0, 31) as u32);
        let (mut cl, mut cr) = (l.clone(), r.clone());
        unsafe { c::cref_stereo_generate_ms_output(cl.as_mut_ptr(), cr.as_mut_ptr(), ls, rs, n as u32) };
        let (mut rl, mut rr) = (l.clone(), r.clone());
        cjoint_stereo_generate_ms_output(&mut rl, &mut rr, ls, rs, n as u32);
        assert_eq!((rl, rr), (cl, cr), "GenerateMSOutput({n}, {ls}, {rs})");
    }
}

#[test]
fn read() {
    let mut rng = Rng::new(0x57e0_0003);
    let mut p = Pair::new();
    for it in 0..6000 {
        let usac = rng.next_u32() % 2 == 0;
        random_state(&mut p, &mut rng, usac, true, true);
        let ics = p.r.ci[0].ics_info;
        let window_groups = ics.window_groups as i32;
        let sfbt = ics.max_sf_bands.max(p.r.ci[1].ics_info.max_sf_bands) as i32;
        let cplx_activ = (usac && rng.next_u32() % 4 != 0) as i32;
        let use_persist = cplx_activ != 0;
        let use_cplx = cplx_activ != 0;
        p.r.sci[0].p_cpe_static_data = if use_persist { Some(0) } else { None };
        p.r.cplx_present = use_cplx || rng.next_u32() % 2 == 0;
        {
            let j = &mut p.r.cpe.joint_stereo_persistent_data;
            j.win_seq_prev = (rng.next_u32() % 4) as u8;
            j.win_groups_prev = (rng.next_u32() % 9) as u8;
        }
        let flags = if usac { AC_USAC | if rng.next_u32() % 2 == 0 { AC_INDEP } else { 0 } } else { 0 };
        let data = random_bytes(&mut rng, 256);
        p.set_bs(&data, 256, 2048);
        p.sync();
        let total = ics.total_sf_bands as i32;
        let ws = ics.window_sequence;
        let ce = unsafe {
            c::cref_stereo_read(p.c.0, window_groups, sfbt, 64, use_persist as i32, use_cplx as i32, cplx_activ, total, ws, flags)
        };
        let mut com_static = p.r.take_com_static();
        let re = {
            let RCtx { com, cpe, bs, .. } = &mut p.r;
            cjoint_stereo_read(
                bs,
                &mut com.joint_stereo_data,
                window_groups,
                sfbt,
                64,
                if use_persist { Some(&mut cpe.joint_stereo_persistent_data) } else { None },
                if use_cplx { com_static.cplx_prediction_data.as_deref_mut() } else { None },
                cplx_activ,
                total,
                ws,
                flags,
            )
        };
        p.r.put_com_static(com_static);
        assert_eq!(re, ce, "CJointStereo_Read {it}");
        p.check(&format!("CJointStereo_Read {it}"));
        p.check_bs(&format!("CJointStereo_Read {it}"));
    }
}

/// A random state of a CPE with a common window for the apply functions; returns
/// (max_sfb_ste_outside, sfbtL, sfbtR).
fn cpe_state(p: &mut Pair, rng: &mut Rng, usac: bool) -> (i32, i32, i32) {
    random_state(p, rng, usac, !usac, true);
    p.r.ci[1].ics_info = p.r.ci[0].ics_info;
    if usac && rng.next_u32() % 3 == 0 {
        // common_max_sfb == 0: an own max_sfb for the second channel
        p.r.ci[1].ics_info.max_sf_bands = rng.range(0, p.r.ci[1].ics_info.total_sf_bands as i32) as u8;
    }
    let (l, r) = (p.r.ci[0].ics_info.max_sf_bands as i32, p.r.ci[1].ics_info.max_sf_bands as i32);
    let max = l.max(r);
    for ch in 0..2 {
        p.r.ci[ch].ics_info.max_sfb_ste = max as u8;
        let nb = p.r.ci[ch].ics_info.max_sf_bands as usize;
        for i in 0..128 {
            // CBlock_InverseQuantizeSpectralData() leaves the bands that were not transmitted at
            // 0; other values can give negative shift amounts in the complex prediction, which
            // the library never computes
            let long = p.r.ci[ch].ics_info.window_sequence != 2;
            let transmitted = if long { i < nb } else { i % 16 < nb };
            p.r.ci[ch].p_dyn_data.a_sfb_scale[i] = if transmitted {
                rng.range(-10, 35) as i16
            } else {
                0
            };
        }
        let sh = 2 + rng.next_u32() % 12;
        for i in 0..1024 {
            p.r.spec[ch * 1024 + i] = random_spec(rng) >> sh;
        }
    }
    let j = &mut p.r.com.joint_stereo_data;
    j.ms_mask_present = (rng.next_u32() % 4) as u8;
    for x in j.ms_used.iter_mut() {
        *x = if rng.next_u32() % 3 == 0 { 0 } else { rng.next_u32() as u8 };
    }
    (max, l, r)
}

#[test]
fn apply_ms() {
    let mut rng = Rng::new(0x57e0_0004);
    let mut p = Pair::new();
    for it in 0..4000 {
        let (max, l, r) = cpe_state(&mut p, &mut rng, false);
        p.r.com.joint_stereo_data.cplx_pred_flag = 0;
        let groups = p.r.ci[0].ics_info.window_groups as i32;
        p.sync();
        unsafe { c::cref_stereo_apply_ms(p.c.0, groups, max, l, r, 1) };
        apply_ms_rust(&mut p, groups, max, l, r, 1);
        p.check(&format!("CJointStereo_ApplyMS (M/S) {it}"));
    }
}

fn apply_ms_rust(p: &mut Pair, groups: i32, max: i32, l: i32, r: i32, mainband: i32) {
    let sri = p.r.sri;
    let offsets = get_scale_factor_band_offsets(&p.r.ci[0].ics_info, &sri);
    let wgl = p.r.ci[0].ics_info.window_group_length;
    let mut com_static = p.r.take_com_static();
    {
        let cpe_present = p.r.sci[0].p_cpe_static_data.is_some();
        let RCtx { ci, sci, com, cpe, spec, scratch, .. } = &mut p.r;
        let [c0, c1] = ci;
        let [s0, s1] = sci;
        let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
        let mut statics: [&mut CAacDecoderStaticChannelInfo; 2] = [&mut **s0, &mut **s1];
        cjoint_stereo_apply_ms(
            &mut chans,
            &mut statics,
            offsets,
            &wgl,
            groups,
            max,
            l,
            r,
            mainband,
            spec,
            com,
            &mut com_static,
            if cpe_present { Some(&mut **cpe) } else { None },
            scratch,
        );
    }
    p.r.put_com_static(com_static);
}

#[test]
fn apply_ms_complex_prediction() {
    let mut rng = Rng::new(0x57e0_0005);
    let mut p = Pair::new();
    for it in 0..4000 {
        let (max, l, r) = cpe_state(&mut p, &mut rng, true);
        p.r.com.joint_stereo_data.cplx_pred_flag = 1;
        p.r.com.joint_stereo_data.store_dmx_re_prev_e = rng.range(-5, 40) as i16;
        p.r.sci[0].p_cpe_static_data = Some(0);
        p.r.cplx_present = true;
        {
            let cp = &mut p.r.cplx;
            cp.pred_dir = (rng.next_u32() % 2) as u8;
            cp.complex_coef = (rng.next_u32() % 2) as u8;
            cp.use_prev_frame = if cp.complex_coef != 0 { (rng.next_u32() % 2) as u8 } else { 0 };
            for g in 0..8 {
                for b in 0..64 {
                    cp.alpha_q_re[g][b] = rng.range(-30, 30) as i16;
                    cp.alpha_q_im[g][b] = rng.range(-30, 30) as i16;
                }
            }
        }
        {
            let j = &mut p.r.cpe.joint_stereo_persistent_data;
            j.win_seq_prev = (rng.next_u32() % 4) as u8;
            j.win_shape_prev = (rng.next_u32() % 2) as u8;
            j.clear_spectral_coeffs = (rng.next_u32() % 4 == 0) as u8;
        }
        p.r.ci[0].ics_info.window_shape = (rng.next_u32() % 2) as u8;
        p.r.ci[1].ics_info.window_shape = p.r.ci[0].ics_info.window_shape;
        for ch in 0..2 {
            let sh = 2 + rng.next_u32() % 12;
            for x in p.r.sci[ch].concealment_info.spectral_coefficient.iter_mut() {
                *x = random_spec(&mut rng) >> sh;
            }
            for x in p.r.sci[ch].concealment_info.spec_scale.iter_mut() {
                *x = rng.range(-5, 35) as i16;
            }
        }
        let groups = p.r.ci[0].ics_info.window_groups as i32;
        p.sync();
        unsafe { c::cref_stereo_apply_ms(p.c.0, groups, max, l, r, 1) };
        apply_ms_rust(&mut p, groups, max, l, r, 1);
        p.check(&format!("CJointStereo_ApplyMS (complex prediction) {it}"));
    }
}

#[test]
fn apply_is() {
    let mut rng = Rng::new(0x57e0_0006);
    let mut p = Pair::new();
    for it in 0..4000 {
        let (_, l, _) = cpe_state(&mut p, &mut rng, false);
        random_codebooks(&mut rng, &mut p.r.ci[1], false);
        for i in 0..128 {
            p.r.ci[1].p_dyn_data.a_scale_factor[i] = rng.range(-300, 100) as i16;
        }
        let groups = p.r.ci[0].ics_info.window_groups as i32;
        p.sync();
        unsafe { c::cref_stereo_apply_is(p.c.0, groups, l) };
        let sri = p.r.sri;
        let offsets = get_scale_factor_band_offsets(&p.r.ci[0].ics_info, &sri);
        let wgl = p.r.ci[0].ics_info.window_group_length;
        {
            let RCtx { ci, com, spec, .. } = &mut p.r;
            let [c0, c1] = ci;
            let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
            cjoint_stereo_apply_is(&mut chans, offsets, &wgl, groups, l, spec, com);
        }
        p.check(&format!("CJointStereo_ApplyIS {it}"));
    }
}
