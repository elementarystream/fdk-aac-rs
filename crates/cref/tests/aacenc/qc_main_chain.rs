// The encoder core of the port (psychoacoustic model, FDKaacEnc_QCMainPrepare's form factor and
// PE stages, FDKaacEnc_QCMain) run as a chain over many frames with its own persistent state,
// next to the reference encoder (harness adj_thr_enc.rs), in many configurations.
//
// The port's state is taken from the reference once, after the encoder is opened; from then on
// every stage of the port works on the port's own state, and the complete state is compared
// with the reference after every stage. FDKaacEnc_QCMainPrepare (form factor, PE and the static
// bits of FDKaacEnc_ChannelElementWrite) runs as a whole and is compared after the three
// reference stages it covers. The stages that need the transport encoder's state are taken
// from the reference: the transport header bits of the global stage and
// FDKaacEnc_FinalizeBitConsumption with the bitstream writing (tested on their own and through
// the API, see bitenc.rs and aacenc_lib.rs); their results are copied into the port's state.
//
// Two parts of the reference's state are not modeled by the port and are copied as well:
// * PSY_DYNAMIC lives in the shared dynamic RAM, which the quantizer overwrites; the port's
//   PSY_DYNAMIC is set to the reference's dynamic RAM contents before the first element of each
//   frame.
// * The spectral lines of grouped short blocks without a defined value in the reference
//   (FDKaacEnc_groupShortData) are copied into the port's spectrum (the port writes 0; the lines
//   are not used).

use crate::adj_thr_enc::*;
use fdk_aac_rs_aacenc::interface::*;
use fdk_aac_rs_aacenc::psy_const::SHORT_WINDOW;
use fdk_aac_rs_aacenc::psy_main::*;
use fdk_aac_rs_aacenc::qc_data::*;
use fdk_aac_rs_aacenc::qc_main::*;
use fdk_aac_rs_fdk::audio::*;

/// The port's encoder core state.
struct Core {
    psy: PsyInternal,
    psy_out: PsyOut,
    qc_out: QcOut,
    qc: QcState,
}

fn check_psy(r: &PsyInternal, c: &PsyInternal, what: &str) {
    for (i, (a, b)) in r.p_static_channels.iter().zip(c.p_static_channels.iter()).enumerate() {
        assert!(a.psy_input_buffer == b.psy_input_buffer, "{what}: static {i} input buffer");
        assert!(a.overlap_add_buffer == b.overlap_add_buffer, "{what}: static {i} overlap");
        assert_eq!(a.block_switching_control, b.block_switching_control, "{what}: static {i}");
        assert_eq!(a.sfb_thresholdnm1, b.sfb_thresholdnm1, "{what}: static {i}");
        assert_eq!(
            (a.mdct_scalenm1, a.calc_pre_echo, a.is_lfe),
            (b.mdct_scalenm1, b.calc_pre_echo, b.is_lfe),
            "{what}: static {i}"
        );
        let (m, n) = (&a.mdct_pers, &b.mdct_pers);
        assert_eq!(
            (m.prev_tl, m.prev_nr, m.prev_fr, m.ov_offset, m.ov_size),
            (n.prev_tl, n.prev_nr, n.prev_fr, n.ov_offset, n.ov_size),
            "{what}: static {i} mdct"
        );
        assert_eq!(m.prev_wrs.map(|w| w.len()), n.prev_wrs.map(|w| w.len()), "{what}: static {i}");
    }
    assert!(*r.psy_dynamic == *c.psy_dynamic, "{what}: PSY_DYNAMIC");
}

fn check_qc_out(r: &QcOut, c: &QcOut, what: &str) {
    for (ch, (a, b)) in r.p_qc_out_channels.iter().zip(c.p_qc_out_channels.iter()).enumerate() {
        if let Some(k) = (0..1024).find(|&k| a.mdct_spectrum[k] != b.mdct_spectrum[k]) {
            panic!("{what}: ch {ch} spectrum line {k}");
        }
        assert!(a == b, "{what}: ch {ch} QC_OUT_CHANNEL");
    }
    assert_eq!(r, c, "{what}: QC_OUT");
}

/// Copies the reference's values of the spectral lines without a defined value (see the file
/// header) into the port's spectra of element `el`.
fn copy_undefined_lines(core: &mut Core, c: &QcOut, cm: &ChannelMapping, el: usize) {
    let n = cm.el_info[el].n_channels_in_el as usize;
    for ch in 0..n {
        let po = &core.psy_out.p_psy_out_channels[core.psy_out.psy_out_element[el].psy_out_channel[ch]];
        if po.last_window_sequence != SHORT_WINDOW {
            continue;
        }
        let q = core.qc_out.qc_element[el].qc_out_channel[ch];
        let cnt = po.sfb_per_group as usize;
        let active = core.psy.psy_conf[1].sfb_active as usize;
        for g in 0..po.sfb_cnt as usize / cnt.max(1) {
            let a = po.sfb_offsets[g * cnt + active] as usize;
            let b = (po.sfb_offsets[(g + 1) * cnt] as usize).min(1024);
            for k in a..b {
                assert_eq!(core.qc_out.p_qc_out_channels[q].mdct_spectrum[k], 0);
                core.qc_out.p_qc_out_channels[q].mdct_spectrum[k] =
                    c.p_qc_out_channels[q].mdct_spectrum[k];
            }
        }
    }
}

/// Runs the chain over `frames` frames. Returns (frames compared through FDKaacEnc_QCMain,
/// frames among them that reached FDKaacEnc_getMinimalStaticBitdemand or
/// FDKaacEnc_crashRecovery).
fn run_config(cfg: &EncConfig, sig: Signal, frames: usize) -> Option<(usize, usize)> {
    let Ok(mut e) = CEnc::open(cfg) else { return None };
    let cm = e.channel_mapping();
    let pcm = pcm_frames(sig, cfg.sample_rate, e.n_channels(), e.frame_length(), frames);
    let ae = e.aac_enc();
    let mut core = Core {
        psy: *ae.psy_kernel.unwrap(),
        psy_out: *ae.psy_out[0].clone().unwrap(),
        qc_out: *ae.qc_out[0].clone().unwrap(),
        qc: *ae.qc_kernel.unwrap(),
    };
    let size = e.input_buffer_size_per_channel() as u32;
    let mut checked = 0;
    let mut recovered = 0;
    for (f, frame) in pcm.iter().enumerate() {
        e.begin_frame(frame);
        let input = e.input_buffer();
        // BEGIN
        assert_eq!(e.run(), 0);
        core.qc_out.element_ext_bits = 0;
        core.qc_out.static_bits = 0;
        core.qc_out.total_no_red_pe = 0;
        let mut first = true;
        loop {
            let (stage, el) = e.next();
            let what = format!("{cfg:?} {sig:?} frame {f} stage {stage} el {el}");
            match stage {
                STAGE_PSY => {
                    if first {
                        // PSY_DYNAMIC: the reference's shared dynamic RAM contents
                        core.psy.psy_dynamic = e.psy_internal().psy_dynamic;
                        first = false;
                    }
                    let n = cm.el_info[el].n_channels_in_el as usize;
                    let s0 = core.psy.psy_element[el].psy_static[0];
                    let p0 = core.psy_out.psy_out_element[el].psy_out_channel[0];
                    let q0 = core.qc_out.qc_element[el].qc_out_channel[0];
                    let err_r = fdk_aac_enc_psy_main(
                        n as i32,
                        &mut core.psy.p_static_channels[s0..s0 + n],
                        &mut core.psy.psy_dynamic,
                        &mut core.psy.psy_conf,
                        &mut core.psy_out.psy_out_element[el],
                        &mut core.psy_out.p_psy_out_channels[p0..p0 + n],
                        &mut core.qc_out.p_qc_out_channels[q0..q0 + n],
                        &input,
                        size,
                        &cm.el_info[el].channel_index,
                        cm.n_channels,
                    );
                    let err_c = e.run();
                    assert_eq!(err_r, err_c, "{what}: psyMain error");
                    if err_c != 0 {
                        return Some((checked, recovered));
                    }
                    let qc = e.qc_out();
                    copy_undefined_lines(&mut core, &qc, &cm, el);
                    check_psy(&core.psy, &e.psy_internal(), &what);
                    assert_eq!(core.psy_out, e.psy_out(), "{what}: PSY_OUT");
                    check_qc_out(&core.qc_out, &qc, &what);
                }
                STAGE_FORM_FACTOR => {
                    // FDKaacEnc_QCMainPrepare: form factor, PE, static bits
                    let n = cm.el_info[el].n_channels_in_el as usize;
                    let p0 = core.psy_out.psy_out_element[el].psy_out_channel[0];
                    let q0 = core.qc_out.qc_element[el].qc_out_channel[0];
                    let el_info = cm.el_info[el];
                    let err_r = fdk_aac_enc_qc_main_prepare(
                        &el_info,
                        &mut core.qc.h_adj_thr.adj_thr_state_elem[el],
                        &mut core.psy_out.psy_out_element[el],
                        &mut core.psy_out.p_psy_out_channels[p0..p0 + n],
                        &mut core.qc_out.qc_element[el],
                        &mut core.qc_out.p_qc_out_channels[q0..q0 + n],
                        e.aot(),
                        e.syntax_flags(),
                        e.ep_config(),
                    );
                    assert_eq!(e.run(), 0, "{what}: form factor");
                    assert_eq!(e.next(), (STAGE_PE, el), "{what}");
                    assert_eq!(e.run(), 0, "{what}: PE");
                    assert_eq!(e.next(), (STAGE_EL_BITS, el), "{what}");
                    let err_c = e.run();
                    assert_eq!(err_r, err_c, "{what}: QCMainPrepare error");
                    if err_c != 0 {
                        return Some((checked, recovered));
                    }
                    // the extension payloads of the element (none in the harness) and the sums
                    let r = &mut core.qc_out.qc_element[el];
                    r.ext_bits_used = 0;
                    r.n_extensions = 0;
                    r.extension = Default::default();
                    core.qc_out.element_ext_bits += r.ext_bits_used;
                    core.qc_out.static_bits += r.static_bits_used;
                    core.qc_out.total_no_red_pe += r.pe_data.pe;
                    check_qc_out(&core.qc_out, &e.qc_out(), &what);
                    assert_eq!(core.qc.h_adj_thr, e.adj_thr(), "{what}: ADJ_THR_STATE");
                }
                STAGE_GLOBAL => {
                    // extension payloads, FDKaacEnc_AdjustBitrate,
                    // transportEnc_GetStaticBits
                    let mut avg = 0;
                    fdk_aac_enc_adjust_bitrate(
                        &mut core.qc,
                        &cm,
                        &mut avg,
                        e.config().bit_rate,
                        e.config().sample_rate,
                        e.config().framelength,
                    );
                    avg *= e.config().n_sub_frames;
                    assert_eq!(e.run(), 0);
                    assert_eq!(avg, e.qc_vars()[0], "{what}: avgTotalBits");
                    let c = e.qc_out();
                    core.qc_out.n_extensions = c.n_extensions;
                    core.qc_out.global_ext_bits = c.global_ext_bits;
                    core.qc_out.extension = c.extension;
                    core.qc.glob_hdr_bits = e.qc_state().glob_hdr_bits;
                    check_qc_out(&core.qc_out, &c, &what);
                }
                STAGE_BITRES_REDIST => {
                    let calls = channel_element_write_calls();
                    let err_r = fdk_aac_enc_qc_main(
                        &mut core.qc,
                        &mut core.psy_out,
                        &mut core.qc_out,
                        e.qc_vars()[0],
                        &cm,
                        e.aot(),
                        e.syntax_flags(),
                        e.ep_config(),
                    );
                    let err_c = e.run_to(STAGE_FINALIZE, 0);
                    if channel_element_write_calls() != calls {
                        // FDKaacEnc_getMinimalStaticBitdemand or FDKaacEnc_crashRecovery
                        recovered += 1;
                    }
                    assert_eq!(err_r, err_c, "{what}: QCMain error");
                    if err_c != 0 {
                        return Some((checked, recovered));
                    }
                    let mut sc = e.qc_state();
                    sc.h_bit_counter = core.qc.h_bit_counter.clone();
                    check_qc_out(&core.qc_out, &e.qc_out(), &what);
                    assert_eq!(core.psy_out, e.psy_out(), "{what}: PSY_OUT");
                    assert_eq!(core.qc, sc, "{what}: QC_STATE");
                    checked += 1;
                }
                STAGE_FINALIZE => {
                    // FDKaacEnc_updateFillBits (port), FDKaacEnc_FinalizeBitConsumption,
                    // FDKaacEnc_updateBitres, bitstream writing: the bit counts are
                    // taken from the reference.
                    let err_c = e.run();
                    if err_c != 0 {
                        return Some((checked, recovered));
                    }
                    let c = e.qc_out();
                    core.qc_out.tot_fill_bits = c.tot_fill_bits;
                    core.qc_out.total_bits = c.total_bits;
                    core.qc_out.align_bits = c.align_bits;
                    core.qc_out.granted_dyn_bits = c.granted_dyn_bits;
                    // FDKaacEnc_WriteBitstream registers the fill payload in the global
                    // extension list.
                    core.qc_out.extension = c.extension;
                    core.qc_out.n_extensions = c.n_extensions;
                    let s = e.qc_state();
                    core.qc.bit_res_tot = s.bit_res_tot;
                    core.qc.glob_hdr_bits = s.glob_hdr_bits;
                    check_qc_out(&core.qc_out, &c, &what);
                }
                s if s >= STAGE_DONE => break,
                _ => panic!("{what}: unexpected stage"),
            }
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
        "core chain: {ran} configurations, {checked} frames, {recovered} of them through the \
         minimal static bit demand or the crash recovery"
    );
    assert!(ran > 0 && checked > 0);
}

#[test]
fn chain_lc() {
    run(51, 40, 30, |c| c.aot == AOT_AAC_LC);
}

#[test]
fn chain_ld_eld() {
    run(52, 50, 40, |c| c.aot != AOT_AAC_LC);
}

#[test]
fn chain_multichannel() {
    run(53, 25, 25, |c| c.channel_mode >= 3);
}
