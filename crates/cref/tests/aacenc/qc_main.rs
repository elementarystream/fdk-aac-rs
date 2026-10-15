// Differential tests of fdk_aac_rs_aacenc::qc_main against the C++ reference.
//
// * FDKaacEnc_QCMain is tested on real encoder states: the reference encoder (harness
//   adj_thr_enc.rs, a stage-by-stage replica of FDKaacEnc_EncodeFrame/FDKaacEnc_QCMain) encodes
//   consecutive frames of real and synthetic PCM in many configurations (LC, LD, ELD with all
//   granule lengths, mono to 7.1, CBR and VBR, afterburner on and off). At the start of
//   FDKaacEnc_QCMain the C state is copied into the port's types, the port runs
//   fdk_aac_enc_qc_main on the copy, and the result is compared with the C state after the last
//   stage of FDKaacEnc_QCMain: QC_OUT (all channels and elements), PSY_OUT and QC_STATE with the
//   element bits and the ADJ_THR_STATE. Frames that reach FDKaacEnc_ChannelElementWrite of
//   bitenc (through the crash recovery or FDKaacEnc_getMinimalStaticBitdemand) are counted.
// * FDKaacEnc_AdjustBitrate, FDKaacEnc_updateFillBits, FDKaacEnc_updateBitres,
//   FDKaacEnc_QCNew + FDKaacEnc_QCInit, FDKaacEnc_QCOutNew + FDKaacEnc_QCOutInit are tested on
//   their own (shim aacenc_qc_main.cpp).
// * The shared dynamic RAM (P_BUF_1): poisoning it before every stage of the reference encoder
//   must not change the access units.

use crate::adj_thr_enc::*;
use fdk_aac_rs_aacenc::aacenc::*;
use fdk_aac_rs_aacenc::adj_thr_data::{AtsElement, BresParam};
use fdk_aac_rs_aacenc::channel_map::fdk_aac_enc_init_channel_mapping;
use fdk_aac_rs_aacenc::qc_data::*;
use fdk_aac_rs_aacenc::qc_main::*;
use fdk_aac_rs_cref::aacenc_qc_main as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use std::ffi::c_void;

/// Runs `frames` frames of `sig` and checks FDKaacEnc_QCMain on every frame. Returns (checked
/// frames, checked frames that reached FDKaacEnc_getMinimalStaticBitdemand or
/// FDKaacEnc_crashRecovery).
fn run_config(cfg: &EncConfig, sig: Signal, frames: usize) -> Option<(usize, usize)> {
    let Ok(mut e) = CEnc::open(cfg) else { return None };
    let cm = e.channel_mapping();
    let pcm = pcm_frames(sig, cfg.sample_rate, e.n_channels(), e.frame_length(), frames);
    let (mut checked, mut recovered) = (0, 0);
    for (f, frame) in pcm.iter().enumerate() {
        let what = format!("{cfg:?} {sig:?} frame {f}");
        e.begin_frame(frame);
        if e.run_to(STAGE_BITRES_REDIST, 0) != 0 {
            return Some((checked, recovered));
        }
        if e.next().0 != STAGE_BITRES_REDIST {
            e.finish().ok();
            continue;
        }
        let mut qs = e.qc_state();
        let mut q = e.qc_out();
        let mut p = e.psy_out();
        let avg_total_bits = e.qc_vars()[0];
        let calls = channel_element_write_calls();
        let err_r = fdk_aac_enc_qc_main(
            &mut qs,
            &mut p,
            &mut q,
            avg_total_bits,
            &cm,
            e.aot(),
            e.syntax_flags(),
            e.ep_config(),
        );
        let err_c = e.run_to(STAGE_FINALIZE, 0);
        if channel_element_write_calls() != calls {
            // FDKaacEnc_getMinimalStaticBitdemand or FDKaacEnc_crashRecovery was reached
            recovered += 1;
        }
        assert_eq!(err_r, err_c, "{what}: error");
        if err_c != 0 {
            return Some((checked, recovered));
        }
        let qc = e.qc_out();
        for (ch, (a, b)) in q.p_qc_out_channels.iter().zip(qc.p_qc_out_channels.iter()).enumerate()
        {
            assert!(a.section_data == b.section_data, "{what}: ch {ch} section data");
            assert!(a.quant_spec == b.quant_spec, "{what}: ch {ch} quantSpec");
            assert!(a.scf == b.scf, "{what}: ch {ch} scf");
            assert!(a == b, "{what}: ch {ch} QC_OUT_CHANNEL");
        }
        assert_eq!(q, qc, "{what}: QC_OUT");
        assert_eq!(p, e.psy_out(), "{what}: PSY_OUT");
        let mut sc = e.qc_state();
        // The bit counter buffers share the dynamic RAM with the threshold adjustment buffers
        // and PSY_DYNAMIC in C; only the entries written by FDKaacEnc_dynBitCount are read (see
        // the dyn_bits tests), the others hold other data in C.
        sc.h_bit_counter = qs.h_bit_counter.clone();
        assert_eq!(qs, sc, "{what}: QC_STATE");
        checked += 1;
        if e.finish().is_err() {
            return Some((checked, recovered));
        }
    }
    Some((checked, recovered))
}

fn run(seed: u64, n_configs: usize, frames: usize, filter: impl Fn(&EncConfig) -> bool) {
    let (mut checked, mut recovered, mut ran) = (0, 0, 0);
    for (k, cfg) in config_subset(seed, n_configs, filter).iter().enumerate() {
        let sig = SIGNALS[(k + seed as usize) % SIGNALS.len()];
        if let Some((c, s)) = run_config(cfg, sig, frames) {
            ran += 1;
            checked += c;
            recovered += s;
        }
    }
    eprintln!(
        "qc_main: {ran} configurations, {checked} frames checked, {recovered} of them through \
         the minimal static bit demand or the crash recovery"
    );
    assert!(ran > 0 && checked > 0);
}

#[test]
fn qc_main_lc_cbr() {
    run(21, 40, 30, |c| c.aot == AOT_AAC_LC && c.bitrate_mode == 0);
}

#[test]
fn qc_main_lc_vbr() {
    run(22, 40, 30, |c| c.aot == AOT_AAC_LC && c.bitrate_mode != 0);
}

#[test]
fn qc_main_ld_eld() {
    run(23, 60, 40, |c| c.aot != AOT_AAC_LC);
}

#[test]
fn qc_main_multichannel() {
    run(24, 30, 25, |c| c.channel_mode >= 3);
}

/// Poisoning the shared dynamic RAM (P_BUF_1: PSY_DYNAMIC, the threshold adjustment buffers and
/// the bit counter tables) before every stage of the reference encoder does not change the
/// access units: no stage uses values another stage (or an earlier frame) left there.
#[test]
fn p_buf_1_poisoning_keeps_output() {
    let mut r = Rng::new(0x5042_5546);
    let mut ran = 0;
    for (k, cfg) in config_subset(31, 40, |_| true).iter().enumerate() {
        let (Ok(mut a), Ok(mut b)) = (CEnc::open(cfg), CEnc::open(cfg)) else { continue };
        let sig = SIGNALS[k % SIGNALS.len()];
        let pcm = pcm_frames(sig, cfg.sample_rate, a.n_channels(), a.frame_length(), 12);
        let len = a.p_buf_1().len();
        for (f, frame) in pcm.iter().enumerate() {
            let out_a = a.frame(frame);
            b.begin_frame(frame);
            let out_b = loop {
                if b.next().0 >= STAGE_DONE {
                    break b.finish();
                }
                let poison: Vec<u8> = (0..len).map(|_| r.next_u32() as u8).collect();
                b.set_p_buf_1(&poison);
                if b.run() != 0 {
                    break Err(-1);
                }
            };
            assert_eq!(out_a.is_ok(), out_b.is_ok(), "{cfg:?} frame {f}");
            if let (Ok(x), Ok(y)) = (&out_a, &out_b) {
                assert_eq!(x, y, "{cfg:?} {sig:?} frame {f}: access unit");
            }
        }
        ran += 1;
    }
    assert!(ran > 10);
}

#[test]
fn adjust_bitrate() {
    let mut r = Rng::new(0x4144_4a42);
    let rates = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];
    for i in 0..20000 {
        let (bit_rate, sample_rate, granule) = if i % 2 == 0 {
            (
                r.range(8000, 512000),
                rates[(r.next_u32() % 12) as usize],
                [1024, 512, 480, 256, 240, 128, 120][(r.next_u32() % 7) as usize],
            )
        } else {
            (r.i32(), r.i32(), r.i32())
        };
        let mut pr_c = if i % 3 == 0 { r.i32() } else { sample_rate };
        let mut pr_r = pr_c;
        let (mut avg_c, mut avg_r) = (0, 0);
        if sample_rate == 0 || sample_rate == -1 {
            // x86 division by zero: no defined reference result; skipped.
            continue;
        }
        let e_c = unsafe {
            c::cref_qc_main_adjust_bitrate(&mut pr_c, bit_rate, sample_rate, granule, &mut avg_c)
        };
        let mut qs = QcState { padding: Padding { padding_rest: pr_r }, ..Default::default() };
        let e_r = fdk_aac_enc_adjust_bitrate(
            &mut qs,
            &ChannelMapping::default(),
            &mut avg_r,
            bit_rate,
            sample_rate,
            granule,
        );
        pr_r = qs.padding.padding_rest;
        assert_eq!((e_r, pr_r, avg_r), (e_c, pr_c, avg_c), "{bit_rate} {sample_rate} {granule}");
    }
}

#[test]
fn update_fill_bits_and_bitres() {
    let mut r = Rng::new(0x5550_4442);
    for i in 0..40000 {
        let small = |r: &mut Rng| r.range(-20000, 50000);
        let mut qs = [
            r.range(-2, 8),
            small(&mut r),
            small(&mut r),
            small(&mut r),
            small(&mut r),
        ];
        let mut qo = [0i32; 8];
        for v in qo.iter_mut() {
            *v = small(&mut r);
        }
        if i % 10 == 0 {
            for v in qs[1..].iter_mut().chain(qo.iter_mut()) {
                *v = r.i32() >> 2;
            }
        }
        for which in 0..2 {
            let (mut qs_c, mut qo_c) = (qs, qo);
            let e_c = unsafe { c::cref_qc_main_update(which, qs_c.as_mut_ptr(), qo_c.as_mut_ptr()) };
            let mut s = QcState {
                bitrate_mode: qs[0],
                bit_res_tot_max: qs[1],
                bit_res_tot: qs[2],
                min_bits_per_frame: qs[3],
                max_bits_per_frame: qs[4],
                ..Default::default()
            };
            let mut o = QcOut {
                granted_dyn_bits: qo[0],
                used_dyn_bits: qo[1],
                static_bits: qo[2],
                element_ext_bits: qo[3],
                global_ext_bits: qo[4],
                tot_fill_bits: qo[5],
                total_bits: qo[6],
                align_bits: qo[7],
                ..Default::default()
            };
            let cm = ChannelMapping::default();
            let e_r = if which == 0 {
                fdk_aac_enc_update_fill_bits(&cm, &s, &mut o)
            } else {
                fdk_aac_enc_update_bitres(&cm, &mut s, &o);
                0
            };
            let qs_r =
                [s.bitrate_mode, s.bit_res_tot_max, s.bit_res_tot, s.min_bits_per_frame, s.max_bits_per_frame];
            let qo_r = [
                o.granted_dyn_bits,
                o.used_dyn_bits,
                o.static_bits,
                o.element_ext_bits,
                o.global_ext_bits,
                o.tot_fill_bits,
                o.total_bits,
                o.align_bits,
            ];
            assert_eq!((e_r, qs_r, qo_r), (e_c, qs_c, qo_c), "which {which}: {qs:?} {qo:?}");
        }
    }
}

#[test]
fn qc_new_and_init() {
    let mut r = Rng::new(0x5143_494e);
    let modes = [
        MODE_1,
        MODE_2,
        MODE_1_2,
        MODE_1_2_1,
        MODE_1_2_2,
        MODE_1_2_2_1,
        MODE_6_1,
        MODE_1_2_2_2_1,
        MODE_7_1_BACK,
        MODE_7_1_TOP_FRONT,
        MODE_7_1_REAR_SURROUND,
        MODE_7_1_FRONT_CENTER,
    ];
    let rates = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];
    for i in 0..6000 {
        let mode = modes[(r.next_u32() % modes.len() as u32) as usize];
        let mut cm = ChannelMapping::default();
        assert_eq!(fdk_aac_enc_init_channel_mapping(mode, (i % 3) as i32, &mut cm), AAC_ENC_OK);
        let n_el = cm.n_elements;
        let n_eff = cm.n_channels_eff;
        let sample_rate = rates[(r.next_u32() % 12) as usize];
        let frame_length = [1024, 512, 480, 256, 120][(r.next_u32() % 5) as usize];
        let bitrate = r.range(8000, 100000) * n_eff;
        let avg = ((bitrate as i64 * frame_length as i64 / sample_rate as i64) as i32 + 7) & !7;
        let bitrate_mode = r.range(-1, 7);
        let max_bits = 6144 * n_eff;
        let bit_res = if r.next_u32() % 4 == 0 { r.range(0, 2000) } else { max_bits - avg };
        let mut iv = [
            0,
            max_bits,
            avg,
            bit_res,
            sample_rate,
            (frame_length < 1024) as i32,
            r.range(0, 120),
            bitrate_mode,
            r.range(1, 20000),
            bitrate / cm.n_channels,
            [0, 2][(r.next_u32() % 2) as usize],
            [2, 5][(r.next_u32() % 2) as usize],
            r.range(0, 0x7fff_ffff),
            bitrate,
            1,
            r.range(0, avg.max(1)),
            r.range(0, 2),
            r.range(0, 1),
            sample_rate,
        ];
        if i % 10 == 9 {
            // extreme values (only where the x86 reference result is defined)
            for k in [1, 2, 3, 6, 8, 9, 12, 13, 15] {
                iv[k] = r.i32() >> (r.next_u32() % 8);
            }
        }
        let prev = [r.range(-1000, 30000), if r.next_u32() % 2 == 0 { iv[3] } else { r.range(0, 30000) }];
        let init_flags = r.next_u32() % 2;

        let n = n_el as usize;
        let mut sc = [0i32; 14];
        let mut eb_c = vec![ElementBits::default(); n];
        let mut ats_c = vec![AtsElement::default(); n];
        let (mut bl_c, mut bs_c) = (BresParam::default(), BresParam::default());
        let mut misc = [0i32; 2];
        let e_c = unsafe {
            c::cref_qc_main_qc_init(
                n_el,
                iv.as_ptr(),
                &cm as *const ChannelMapping as *const c_void,
                init_flags,
                prev.as_ptr(),
                sc.as_mut_ptr(),
                eb_c.as_mut_ptr() as *mut c_void,
                &mut bl_c as *mut BresParam as *mut c_void,
                &mut bs_c as *mut BresParam as *mut c_void,
                ats_c.as_mut_ptr() as *mut c_void,
                misc.as_mut_ptr(),
            )
        };
        assert!(e_c >= 0);

        let mut h = None;
        assert_eq!(fdk_aac_enc_qc_new(&mut h, n_el), AAC_ENC_OK);
        let mut h = h.unwrap();
        h.bit_res_tot = prev[0];
        h.bit_res_tot_max = prev[1];
        let init = QcInit {
            channel_mapping: cm,
            sce_cpe: iv[0],
            max_bits: iv[1],
            average_bits: iv[2],
            bit_res: iv[3],
            sample_rate: iv[4],
            is_low_delay: iv[5],
            static_bits: iv[6],
            bitrate_mode: iv[7],
            mean_pe: iv[8],
            ch_bitrate: iv[9],
            inv_quant: iv[10],
            max_iterations: iv[11],
            max_bit_fac: iv[12],
            bitrate: iv[13],
            n_sub_frames: iv[14],
            min_bits: iv[15],
            bit_res_mode: iv[16],
            bit_distribution_mode: iv[17],
            padding: Padding { padding_rest: iv[18] },
        };
        let e_r = fdk_aac_enc_qc_init(&mut h, &init, init_flags);
        let sr = [
            h.glob_hdr_bits,
            h.max_bits_per_frame,
            h.min_bits_per_frame,
            h.n_elements,
            h.bitrate_mode,
            h.bit_res_mode,
            h.bit_res_tot,
            h.bit_res_tot_max,
            h.max_iterations,
            h.inv_quant,
            h.vbr_qual_factor,
            h.max_bit_fac,
            h.padding.padding_rest,
            h.d_zone_quant_enable,
        ];
        let what = format!("{iv:?} prev {prev:?} flags {init_flags} cm {cm:?}");
        assert_eq!((e_r, sr), (e_c, sc), "{what}");
        assert_eq!(h.element_bits, eb_c, "{what}: element bits");
        let a = &h.h_adj_thr;
        assert_eq!((a.bres_param_long, a.bres_param_short), (bl_c, bs_c), "{what}");
        assert_eq!([a.bit_distribution_mode, a.max_iter_2nd_guess], misc, "{what}");
        assert_eq!(a.adj_thr_state_elem, ats_c, "{what}: ATS_ELEMENT");
    }
}

/// FDKaacEnc_QCOutNew + FDKaacEnc_QCOutInit give the channel assignment of the reference
/// encoder.
#[test]
fn qc_out_new_and_init() {
    for cfg in config_subset(41, 60, |_| true) {
        let Ok(e) = CEnc::open(&cfg) else { continue };
        let cm = e.channel_mapping();
        let mut q = [None];
        assert_eq!(
            fdk_aac_enc_qc_out_new(&mut q, cm.n_elements, cm.n_channels, 1),
            AAC_ENC_OK
        );
        assert_eq!(fdk_aac_enc_qc_out_init(&mut q, 1, &cm), AAC_ENC_OK);
        let q = q[0].take().unwrap();
        let qc = e.qc_out();
        for el in 0..cm.n_elements as usize {
            for ch in 0..cm.el_info[el].n_channels_in_el as usize {
                assert_eq!(
                    q.qc_element[el].qc_out_channel[ch],
                    qc.qc_element[el].qc_out_channel[ch],
                    "{cfg:?}"
                );
            }
        }
        assert_eq!(q.p_qc_out_channels.len(), cm.n_channels as usize);
    }
}
