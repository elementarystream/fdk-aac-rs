// Differential tests of fdk_aac_rs_sacdec::sac_process, sac_smoothing, sac_reshape_bb_env and
// sac_stp against the C++ reference: each function of the slot processing is called on a pair
// of initialized spatial decoders (random USAC and LD configurations, with the temporal shaping
// tools) after the signal buffers, matrices, parameters and states were set to the same random
// values on both sides; the complete states (decoder, frame, QMF domain) and the outputs are
// compared after every call.

use crate::sac_calc_m1_and_m2::usac_config;
use crate::sac_dec::*;
use crate::sac_dec_state::*;
use crate::sac_dec_util::*;
use fdk_aac_rs_cref::sacdec_sac_dec as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacdec::sac_dec::*;
use fdk_aac_rs_sacdec::sac_bitdec::TREE_212;
use fdk_aac_rs_sacdec::sac_dec_interface::*;
use fdk_aac_rs_sacdec::sac_process::*;
use fdk_aac_rs_sacdec::sac_reshape_bb_env::*;
use fdk_aac_rs_sacdec::sac_smoothing::*;
use fdk_aac_rs_sacdec::sac_stp::*;

fn random_signals(s: &mut SpatialDec, rng: &mut Rng) {
    let big = rng.next_u32() % 4 == 0;
    let v = |rng: &mut Rng| if big { rng.i32() } else { sig(rng) };
    for r in s.qmf_input_real__fdk.iter_mut().chain(s.qmf_input_imag__fdk.iter_mut()) {
        for x in r.iter_mut() {
            *x = v(rng);
        }
    }
    for r in s
        .hyb_input_real__fdk
        .iter_mut()
        .chain(s.hyb_input_imag__fdk.iter_mut())
        .chain(s.hyb_residual_real__fdk.iter_mut())
        .chain(s.hyb_residual_imag__fdk.iter_mut())
        .chain(s.w_real__fdk.iter_mut())
        .chain(s.w_imag__fdk.iter_mut())
        .chain(s.hyb_output_real_dry__fdk.iter_mut())
        .chain(s.hyb_output_imag_dry__fdk.iter_mut())
        .chain(s.hyb_output_real_wet__fdk.iter_mut())
        .chain(s.hyb_output_imag_wet__fdk.iter_mut())
    {
        for x in r.iter_mut() {
            *x = v(rng);
        }
    }
    for r in s.qmf_residual_real__fdk[0].iter_mut().chain(s.qmf_residual_imag__fdk[0].iter_mut()) {
        for x in r.iter_mut() {
            *x = v(rng);
        }
    }
}

fn random_matrices(s: &mut SpatialDec, rng: &mut Rng) {
    for m in [&mut s.m2_real__fdk, &mut s.m2_imag__fdk, &mut s.m2_real_prev__fdk, &mut s.m2_imag_prev__fdk] {
        for x in m.iter_mut().flatten().flatten() {
            *x = match rng.next_u32() % 8 {
                0 => rng.i32(),
                1 => 0,
                _ => (rng.next_u32() as i32) >> rng.range(1, 6),
            };
        }
    }
    for x in s
        .phase_left__fdk
        .iter_mut()
        .chain(s.phase_right__fdk.iter_mut())
        .chain(s.phase_prev_left__fdk.iter_mut())
        .chain(s.phase_prev_right__fdk.iter_mut())
    {
        // wrapPhase() and the OPD smoothing keep the phases in [0, 2*pi] (PI__IPD = 210828714);
        // the wrapping loops of the smoothing would overflow for values far outside.
        *x = if rng.next_u32() % 8 == 0 { rng.range(-421657428, 843314856) } else { rng.range(0, 421657428) };
    }
    for x in s.smooth_state.opd_left_state__fdk.iter_mut().chain(s.smooth_state.opd_right_state__fdk.iter_mut()) {
        *x = rng.range(0, 421657428);
    }
}

fn random_frame_tools(f: &mut SpatialBsFrame, rng: &mut Rng, n_ts: i32) {
    f.tsd_data[0].bs_tsd_enable = rng.range(0, 1) as u8;
    for v in f.tsd_data[0].bs_tsd_tr_phase_data.iter_mut() {
        *v = rng.range(-1, 7) as i8;
    }
    for v in f.temp_shape_enable_channel_ges.iter_mut().chain(f.temp_shape_enable_channel_stp.iter_mut()) {
        *v = rng.range(0, 1) as u8;
    }
    for v in f.bs_env_shape_data.iter_mut().flatten() {
        *v = rng.range(0, 4) as u8;
    }
    for (i, v) in f.param_slot.iter_mut().enumerate() {
        *v = ((i as i32 + 1) * n_ts / 9 - 1).max(0) + rng.range(0, 1);
    }
    f.opd_smoothing_mode = rng.range(0, 1);
}

fn configs(rng: &mut Rng) -> Vec<(SpatialSpecificConfig, i32, i32)> {
    let mut v = Vec::new();
    for tsc in 0..4u32 {
        for (phase, sci, nres) in [(0u32, 1, 0u32), (1, 1, 0), (0, 2, 5), (1, 2, 4)] {
            let fr_idx = rng.range(1, 7) as u32;
            let fr = [0u32, 28, 20, 14, 10, 7, 5, 4][fr_idx as usize];
            v.push((
                usac_config(fr_idx, tsc, rng.range(0, 2) as u32, 1, phase, None, Some((nres.min(fr), 0)), sci, 3),
                64,
                2048,
            ));
        }
    }
    let mut n = 0;
    while n < 12 {
        if let Some(c) = random_config(rng) {
            if c.0.stereo_config_index != 3 {
                v.push(c);
                n += 1;
            }
        }
    }
    v
}

#[test]
fn sac_process_slot_functions() {
    let mut rng = Rng::new(0x5c1);
    let mut calls = 0;
    for (ci, (ssc, qmf, frame)) in configs(&mut rng).into_iter().enumerate() {
        let (mut p, r) = init_pair(&ssc, qmf, frame, INIT_ALL);
        if r != MPS_OK {
            continue;
        }
        let n_ts = p.r.sd.time_slots;
        for k in 0..60 {
            p.set_state(|s| {
                random_signals(s, &mut rng);
                random_matrices(s, &mut rng);
                s.tsd_ts = rng.range(0, 63);
                s.sac_in_data_headroom = rng.range(1, 8);
            });
            p.set_frame(|f, _| random_frame_tools(f, &mut rng, n_ts));
            let alpha = if rng.next_u32() % 4 == 0 { rng.i16() } else { (rng.next_u32() >> 17) as i16 };
            let ps = rng.range(0, 8);
            let op = rng.next_u32() % 7;
            let what = format!("config {ci} ({k}, operation {op})");
            match op {
                0 => {
                    let r = spatial_dec_apply_m1_create_w_mode212(&mut p.r.sd, &p.r.frame);
                    let cr = unsafe { c::cref_sacinst_apply_m1_create_w(p.c.0) };
                    assert_eq!(r, cr, "{what}: SpatialDecApplyM1_CreateW_Mode212");
                }
                1 => {
                    let which = rng.range(0, 2);
                    // _ResidualsPlusPhaseCoding is used for USAC only
                    let which = if which == 2 && p.r.ssc.syntax_flags & SACDEC_SYNTAX_LD != 0 { 1 } else { which };
                    let r = match which {
                        0 => spatial_dec_apply_m2(&mut p.r.sd, ps, alpha),
                        1 => spatial_dec_apply_m2_mode212(&mut p.r.sd, ps, alpha),
                        _ => spatial_dec_apply_m2_mode212_residuals_plus_phase_coding(&mut p.r.sd, &p.r.ssc, ps, alpha),
                    };
                    let cr = unsafe { c::cref_sacinst_apply_m2(p.c.0, which, ps, alpha) };
                    assert_eq!(r, cr, "{what}: SpatialDecApplyM2 ({which})");
                }
                2 => {
                    spatial_dec_buffer_matrices(&mut p.r.sd);
                    unsafe { c::cref_sacinst_buffer_matrices(p.c.0) };
                }
                3 => {
                    let last = rng.range(0, 1);
                    spatial_dec_apply_phase(&mut p.r.sd, alpha, last);
                    unsafe { c::cref_sacinst_apply_phase(p.c.0, alpha, last) };
                }
                4 => {
                    let n = if rng.next_u32() % 8 == 0 { 0 } else { 1 };
                    let r = spatial_dec_hybrid_analysis(&mut p.r.sd, &p.r.ssc, 0, n);
                    let cr = unsafe { c::cref_sacinst_hybrid_analysis(p.c.0, 0, n) };
                    assert_eq!(r, cr, "{what}: SpatialDecHybridAnalysis");
                }
                5 => {
                    // smoothing: random smoothing data, times and slots
                    p.set_state(|s| {
                        for v in s.smg_data.iter_mut().flatten() {
                            *v = rng.range(0, 1) as u8;
                        }
                        for v in s.smg_time.iter_mut() {
                            *v = [64, 128, 256, 512][rng.next_u32() as usize % 4];
                        }
                        s.smooth_state.prev_param_slot = rng.range(0, n_ts - 1);
                        s.residual_coding = rng.range(0, 1);
                        s.residual_bands[0] = rng.range(0, 28) as u8;
                    });
                    spatial_dec_smooth_m1_and_m2(&mut p.r.sd, &p.r.frame, ps);
                    unsafe { c::cref_sacinst_smooth_m1m2(p.c.0, ps) };
                    if rng.next_u32() % 2 == 0 {
                        spatial_dec_smooth_opd(&mut p.r.sd, &p.r.frame, ps);
                        unsafe { c::cref_sacinst_smooth_opd(p.c.0, ps) };
                    }
                    if rng.next_u32() % 4 == 0 {
                        init_parameter_smoothing(&mut p.r.sd);
                        unsafe { c::cref_sacinst_init_smoothing(p.c.0) };
                    }
                }
                _ => {
                    // temporal shaping: GES and STP (with random states)
                    p.set_state(|s| {
                        let r = &mut s.reshape_bb_env_state;
                        for v in r.part_nrg_prev__fdk.iter_mut().flatten() {
                            *v = (rng.next_u32() >> rng.range(1, 30)) as i32;
                        }
                        for v in r.norm_nrg_prev__fdk.iter_mut().chain(r.frame_nrg_prev__fdk.iter_mut()) {
                            *v = (rng.next_u32() >> rng.range(1, 30)) as i32;
                        }
                        for v in r
                            .part_nrg_prev_sf
                            .iter_mut()
                            .chain(r.part_nrg_prev2_sf.iter_mut())
                            .chain(r.norm_nrg_prev_sf.iter_mut())
                            .chain(r.frame_nrg_prev_sf.iter_mut())
                        {
                            *v = rng.range(-10, 40);
                        }
                        let h = &mut s.h_stp_dec;
                        for v in h.run_dry_ener.iter_mut().chain(h.run_wet_ener.iter_mut()) {
                            *v = (rng.next_u32() >> rng.range(1, 31)) as i32;
                        }
                        for v in h.old_dry_ener_ld64.iter_mut().chain(h.old_wet_ener_ld64.iter_mut()) {
                            *v = -((rng.next_u32() >> rng.range(2, 31)) as i32);
                        }
                        for v in h.prev_tp_scale.iter_mut() {
                            *v = (rng.next_u32() >> rng.range(2, 8)) as i32;
                        }
                        h.update_old_ener = [0, 31, 32, rng.range(0, 40)][rng.next_u32() as usize % 4];
                        s.env_quant_mode = rng.range(0, 1);
                        s.static_dec_scale = rng.range(20, 24);
                    });
                    let ts = rng.range(0, n_ts - 1);
                    if rng.next_u32() % 2 == 0 {
                        spatial_dec_reshape_bb_env(&mut p.r.sd, &p.r.frame, ts);
                        unsafe { c::cref_sacinst_reshape_bbenv(p.c.0, ts) };
                    } else {
                        let r = subband_tp_apply(&mut p.r.sd, &p.r.frame);
                        let cr = unsafe { c::cref_sacinst_stp_apply(p.c.0) };
                        assert_eq!(r, cr, "{what}: subbandTPApply");
                    }
                    if rng.next_u32() % 8 == 0 {
                        let f = rng.range(0, 1);
                        init_bb_env(&mut p.r.sd, f);
                        unsafe { c::cref_sacinst_init_bbenv(p.c.0, f) };
                        let r = subband_tp_init(&mut p.r.sd.h_stp_dec);
                        let cr = unsafe { c::cref_sacinst_stp_init(p.c.0) };
                        assert_eq!(r, cr);
                    }
                }
            }
            p.check(&what);
            calls += 1;
        }
    }
    assert!(calls > 1000, "{calls}");
}

#[test]
fn sac_process_qmf_and_synthesis() {
    let mut rng = Rng::new(0x5c2);
    for (ci, (ssc, qmf, frame)) in configs(&mut rng).into_iter().enumerate() {
        let (mut p, r) = init_pair(&ssc, qmf, frame, INIT_ALL);
        if r != MPS_OK {
            continue;
        }
        // The QMF interface with bShareDelayWithSBR reads HYBRID_FILTER_DELAY (6) slots ahead:
        // the SBR decoder configures that overlap of the QMF domain (the slot pointer arrays of
        // the domain hold 16 + 3, 32 + 6 or 64 + 12 slots).
        let mut gc = time_interface_gc(&ssc, qmf, frame);
        let ov = match gc[14] {
            16 => 3,
            32 => 6,
            _ => 12,
        };
        gc[16] = ov;
        p.configure_qd(&gc);
        p.check("QMF domain with overlap");
        let n_ts = p.r.sd.time_slots;
        for k in 0..30 {
            p.set_state(|s| {
                random_signals(s, &mut rng);
                s.sac_in_data_headroom = rng.range(1, 8);
                s.b_share_delay_with_sbr = if ov >= 6 { rng.range(0, 1) } else { 0 };
            });
            let what = format!("config {ci} ({k})");
            let ts = rng.range(0, n_ts - 1);
            let bypass = rng.range(0, 1);
            match rng.next_u32() % 3 {
                0 => {
                    let input = sigs(&mut rng, (frame * 2) as usize);
                    let r = spatial_dec_qmf_analysis(&mut p.r.sd, &mut p.r.qd, PcmMpsInput::Buffer(&input), &[], ts, bypass, 1);
                    let cr = unsafe { c::cref_sacinst_qmf_analysis(p.c.0, input.as_ptr(), ts, bypass, 1) };
                    assert_eq!(r, cr, "{what}: SpatialDecQMFAnalysis");
                }
                1 => {
                    // random QMF data and scalings in the domain
                    for b in p.r.qd.mem.work_buffer.iter_mut().flatten() {
                        for x in b.iter_mut() {
                            *x = sig(&mut rng);
                        }
                    }
                    for d in p.r.qd.qmf_domain_in.iter_mut() {
                        d.scaling.lb_scale = rng.range(-4, 10);
                        d.scaling.ov_lb_scale = rng.range(-4, 10);
                        d.scaling.hb_scale = rng.range(-4, 10);
                    }
                    let v = dump_qd(&p.r.qd);
                    unsafe { c::cref_sacinst_load_qd_data(p.c.0, v.as_ptr()) };
                    p.check(&format!("{what}: loaded QMF data"));
                    let r = spatial_dec_feed_qmf(&mut p.r.sd, &mut p.r.qd, ts, bypass, 1);
                    let cr = unsafe { c::cref_sacinst_feed_qmf(p.c.0, ts, bypass, 1) };
                    assert_eq!(r, cr, "{what}: SpatialDecFeedQMF");
                }
                _ => {
                    let pass = rng.range(0, 1);
                    let n = (2 * frame) as usize + 64;
                    let mut out = vec![0x777i32; n];
                    let mut cout = out.clone();
                    let md = map_descr(pass as u32);
                    let r = spatial_dec_synthesis(&mut p.r.sd, &mut p.r.qd, &p.r.ssc.clone(), ts, &mut out, 1, &md);
                    let cr = unsafe { c::cref_sacinst_synthesis(p.c.0, ts, cout.as_mut_ptr(), 1, pass) };
                    assert_eq!(r, cr, "{what}: SpatialDecSynthesis");
                    assert_eq!(out, cout, "{what}: output");
                }
            }
            p.check(&what);
        }
    }
}

#[test]
fn sac_process_ch_gain_and_channel_properties() {
    let mut rng = Rng::new(0x5c3);
    let ssc = usac_config(1, 0, 0, 1, 0, None, None, 1, 3);
    let (mut p, r) = init_pair(&ssc, 64, 2048, INIT_ALL);
    assert_eq!(r, MPS_OK);
    for k in 0..2000 {
        p.set_state(|s| {
            s.upmix_type = rng.range(-1, 4);
            s.num_output_channels_at = rng.range(-1, 9);
            // TREE_212 or one of the 8 entries of treePropertyTable
            s.tree_config = [TREE_212, rng.range(0, 7)][rng.next_u32() as usize % 2];
        });
        let ch = [0, 1, 2, 3, rng.next_u32()][rng.next_u32() as usize % 5];
        let (g, sc) = get_ch_gain(&p.r.sd, ch);
        let mut csc = 0;
        let cg = unsafe { c::cref_sacinst_get_ch_gain(p.c.0, ch, &mut csc) };
        assert_eq!((g, sc), (cg, csc), "getChGain ({k})");

        // The arrays of the AAC decoder hold 8 channels.
        if p.r.sd.num_output_channels_at > 8 {
            continue;
        }
        let nulls = if rng.next_u32() % 4 == 0 { rng.range(0, 15) } else { 0 };
        let mut t: [i32; 8] = core::array::from_fn(|_| rng.range(0, 5));
        let mut i: [u8; 8] = core::array::from_fn(|_| rng.range(0, 255) as u8);
        let (mut ct, mut cidx) = (t, i);
        let md = map_descr(0);
        spatial_dec_channel_properties(
            if nulls & 1 != 0 { None } else { Some(&p.r.sd) },
            if nulls & 2 != 0 { None } else { Some(&mut t[..]) },
            if nulls & 4 != 0 { None } else { Some(&mut i[..]) },
            if nulls & 8 != 0 { None } else { Some(&md) },
        );
        unsafe { c::cref_sacinst_channel_properties(p.c.0, ct.as_mut_ptr(), cidx.as_mut_ptr(), nulls) };
        assert_eq!((t, i), (ct, cidx), "SpatialDecChannelProperties ({k}, nulls {nulls})");
    }
}
