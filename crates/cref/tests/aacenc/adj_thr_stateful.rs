// Stateful differential tests of fdk_aac_rs_aacenc::adj_thr on real encoder states.
//
// The reference encoder (harness: adj_thr_enc.rs) encodes consecutive frames of real PCM in
// many configurations. Before each of the stages FDKaacEnc_peCalculation (per element),
// FDKaacEnc_DistributeBits (per element) and FDKaacEnc_AdjustThresholds, the C state is copied
// into the port's types, the port runs the same call on the copy, and the result is compared
// with the C state after the stage: the complete QC_OUT (all QC_OUT_CHANNEL bytes, the elements
// with their PE_DATA), PSY_OUT, ADJ_THR_STATE with all ATS_ELEMENTs, and the adjust-thresholds
// buffers of the shared dynamic RAM (AdjThrDynMem).

use crate::adj_thr_enc::*;
use fdk_aac_rs_aacenc::adj_thr::*;
use fdk_aac_rs_aacenc::adj_thr_data::AdjThrState;
use fdk_aac_rs_aacenc::interface::PsyOut;
use fdk_aac_rs_aacenc::qc_data::{QcOut, QcState};

fn check_qc(r: &QcOut, c: &QcOut, what: &str) {
    for (ch, (a, b)) in r.p_qc_out_channels.iter().zip(c.p_qc_out_channels.iter()).enumerate() {
        if a != b {
            for (name, u, v) in [
                ("threshold", &a.sfb_threshold_ld_data[..], &b.sfb_threshold_ld_data[..]),
                ("min_snr", &a.sfb_min_snr_ld_data[..], &b.sfb_min_snr_ld_data[..]),
                ("spread", &a.sfb_spread_energy[..], &b.sfb_spread_energy[..]),
                ("weighted", &a.sfb_weighted_energy_ld_data[..], &b.sfb_weighted_energy_ld_data[..]),
                ("en_fac", &a.sfb_en_fac_ld[..], &b.sfb_en_fac_ld[..]),
            ] {
                if let Some(i) = (0..u.len()).find(|&i| u[i] != v[i]) {
                    panic!("{what}: ch {ch} {name}[{i}]: rust {:#x} c {:#x}", u[i], v[i]);
                }
            }
            panic!("{what}: ch {ch} QC_OUT_CHANNEL differs");
        }
    }
    for (el, (a, b)) in r.qc_element.iter().zip(c.qc_element.iter()).enumerate() {
        assert_eq!(a.pe_data, b.pe_data, "{what}: element {el} PE_DATA");
        assert_eq!(a, b, "{what}: element {el}");
    }
    assert_eq!(r, c, "{what}: QC_OUT");
}

/// Runs `frames` frames of `sig` through the stepper of `cfg` and checks the adj_thr stages.
/// Returns the number of checked stage calls (0 if the configuration is rejected).
fn run_config(cfg: &EncConfig, sig: Signal, frames: usize) -> usize {
    let Ok(mut e) = CEnc::open(cfg) else { return 0 };
    let cm = e.channel_mapping();
    let pcm = pcm_frames(sig, cfg.sample_rate, e.n_channels(), e.frame_length(), frames);
    let mut checks = 0;
    for (f, frame) in pcm.iter().enumerate() {
        e.begin_frame(frame);
        loop {
            let (stage, el) = e.next();
            if stage >= STAGE_DONE {
                break;
            }
            let what = format!("{cfg:?} {sig:?} frame {f} stage {stage} el {el}");
            match stage {
                STAGE_PE => {
                    let mut q = e.qc_out();
                    let p = e.psy_out();
                    let mut a = e.adj_thr();
                    let n = cm.el_info[el].n_channels_in_el as usize;
                    let qs = q.qc_element[el].qc_out_channel[0];
                    let ps = p.psy_out_element[el].psy_out_channel[0];
                    let mut pe = q.qc_element[el].pe_data;
                    fdk_aac_enc_pe_calculation(
                        &mut pe,
                        &p.p_psy_out_channels[ps..ps + n],
                        &mut q.p_qc_out_channels[qs..qs + n],
                        &p.psy_out_element[el].tools_info,
                        &mut a.adj_thr_state_elem[el],
                        n as i32,
                    );
                    q.qc_element[el].pe_data = pe;
                    assert_eq!(e.run(), 0, "{what}");
                    check_qc(&q, &e.qc_out(), &what);
                    assert_eq!(a, e.adj_thr(), "{what}: ADJ_THR_STATE");
                    checks += 1;
                }
                STAGE_DISTRIBUTE => {
                    let mut q = e.qc_out();
                    let p = e.psy_out();
                    let s: QcState = e.qc_state();
                    let mut a: AdjThrState = s.h_adj_thr.clone();
                    let n = cm.el_info[el].n_channels_in_el as usize;
                    let ps = p.psy_out_element[el].psy_out_channel[0];
                    let x = &mut q.qc_element[el];
                    let (mut gp, mut gpc) = (x.granted_pe, x.granted_pe_corr);
                    fdk_aac_enc_distribute_bits(
                        &mut a,
                        el,
                        &p.p_psy_out_channels[ps..ps + n],
                        &mut x.pe_data,
                        &mut gp,
                        &mut gpc,
                        n as i32,
                        p.psy_out_element[el].common_window,
                        x.granted_dyn_bits,
                        s.element_bits[el].bit_res_level_el,
                        s.element_bits[el].max_bit_res_bits_el,
                        s.max_bit_fac,
                        s.bit_res_mode,
                    );
                    x.granted_pe = gp;
                    x.granted_pe_corr = gpc;
                    let mut vars = e.qc_vars();
                    assert_eq!(e.run(), 0, "{what}");
                    // the sum of the replicated prepareBitDistribution
                    q.total_granted_pe_corr = q.total_granted_pe_corr.wrapping_add(gpc);
                    vars[1] = vars[1].wrapping_add(s.element_bits[el].bit_res_level_el);
                    check_qc(&q, &e.qc_out(), &what);
                    assert_eq!(a, e.adj_thr(), "{what}: ADJ_THR_STATE");
                    assert_eq!(vars, e.qc_vars(), "{what}: totalAvailableBits");
                    checks += 1;
                }
                STAGE_ADJUST_THR => {
                    let mut q = e.qc_out();
                    let mut p: PsyOut = e.psy_out();
                    let mut a = e.adj_thr();
                    let cbr = e.qc_vars()[3];
                    let mut d = e.adj_thr_dyn_mem();
                    fdk_aac_enc_adjust_thresholds_dyn_mem(&mut a, &mut q, &mut p, cbr, &cm, &mut d);
                    assert_eq!(e.run(), 0, "{what}");
                    assert!(*d == *e.adj_thr_dyn_mem(), "{what}: dynMem");
                    check_qc(&q, &e.qc_out(), &what);
                    assert_eq!(p, e.psy_out(), "{what}: PSY_OUT");
                    assert_eq!(a, e.adj_thr(), "{what}: ADJ_THR_STATE");
                    checks += 1;
                }
                _ => {
                    let err = e.run();
                    if err != 0 {
                        // the reference rejects the frame (e.g. bit reservoir underflow); the
                        // states are not comparable any more
                        eprintln!("{what}: AAC_ENCODER_ERROR {err:#x}, configuration stopped");
                        return checks;
                    }
                }
            }
        }
    }
    checks
}

fn run(seed: u64, n_configs: usize, frames: usize, filter: impl Fn(&EncConfig) -> bool) {
    let mut checks = 0;
    let mut ran = 0;
    for (k, cfg) in config_subset(seed, n_configs, filter).iter().enumerate() {
        let sig = SIGNALS[(k + seed as usize) % SIGNALS.len()];
        let n = run_config(cfg, sig, frames);
        ran += (n > 0) as usize;
        checks += n;
    }
    eprintln!("{ran} configurations, {checks} checked calls");
    assert!(ran > 0);
}

#[test]
fn lc_cbr() {
    run(11, 40, 30, |c| c.aot == 2 && c.bitrate_mode == 0);
}

#[test]
fn lc_vbr() {
    run(12, 40, 30, |c| c.aot == 2 && c.bitrate_mode != 0);
}

#[test]
fn ld_eld() {
    run(13, 60, 40, |c| c.aot != 2);
}

#[test]
fn multichannel() {
    run(14, 30, 25, |c| c.channel_mode >= 3);
}
