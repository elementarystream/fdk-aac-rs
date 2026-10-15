// End-to-end differential test of fdk_aac_rs_sbrdec against the C++ reference.
//
// A namespaced copy of the reference AAC decoder (shim/sbrdec_capdec_*.cpp) decodes real
// streams. Every call it makes into the SBR decoder API (and every QMF domain configuration
// call) is captured by shim/sbrdec_e2e.cpp and replayed on a second reference SBR decoder and
// on the port (this file, in the callback). After every call the results, the output time
// signal and the complete state (SBR instance, elements, channels, PS decoder, HBE transposer
// and QMF domain) of the port are compared with the replay instance; the replay instance is
// compared with the original instance of the decoder (a check of the capture).
//
// Streams: ISO/IEC 14496-26 and 23003-7 conformance streams found in the work directory
// (HE-AAC, HE-AAC v2 with PS, downsampled SBR, ELD with SBR, USAC with eSBR: HBE, PVC, inter-TES,
// 8:3 and 4:1), each also decoded with randomly mutated access units. Missing streams are
// skipped (reported). MPEG Surround streams are not used (the MPS decoder accesses the QMF
// domain directly).

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use core::ffi::c_void;
use fdk_aac_rs_cref::sbrdec_e2e::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::qmf::QmfFilterBank;
use fdk_aac_rs_fdk::qmf_domain::*;
use fdk_aac_rs_fdk::syslib_channel_map_descr::*;
use fdk_aac_rs_sbrdec::env_extr::SbrHeaderData;
use fdk_aac_rs_sbrdec::hbe::HbeTransposer;
use fdk_aac_rs_sbrdec::psdec::PsDec;
use fdk_aac_rs_sbrdec::sbr_ram::pod_bytes;
use fdk_aac_rs_sbrdec::env_extr::{
    SBRDEC_DOWNSAMPLE, SBRDEC_ELD_GRID, SBRDEC_LOW_POWER, SBRDEC_PS_DECODED, SBRDEC_QUAD_RATE,
    SBRDEC_USAC_HARMONICSBR, SBRDEC_USAC_ITES,
};
use fdk_aac_rs_sbrdec::sbrdecoder::*;
use std::path::PathBuf;

pub(crate) const EV_OPEN: i32 = 1;
pub(crate) const EV_INIT_ELEMENT: i32 = 2;
pub(crate) const EV_HEADER: i32 = 3;
pub(crate) const EV_SET_PARAM: i32 = 4;
pub(crate) const EV_PARSE: i32 = 5;
pub(crate) const EV_APPLY: i32 = 6;
pub(crate) const EV_DRC_FEED: i32 = 7;
pub(crate) const EV_DRC_DISABLE: i32 = 8;
pub(crate) const EV_FREE_MEM: i32 = 9;
pub(crate) const EV_CLOSE: i32 = 10;
pub(crate) const EV_GET_DELAY: i32 = 11;
pub(crate) const EV_QMF_CLEAR_REQUESTED: i32 = 20;
pub(crate) const EV_QMF_CONFIGURE: i32 = 21;
pub(crate) const EV_QMF_FREE_MEM: i32 = 22;
pub(crate) const EV_QMF_CLEAR_PERSISTENT: i32 = 23;
pub(crate) const EV_QMF_CLOSE: i32 = 24;

const A_BS: i32 = 0;
const A_DRM_IN: i32 = 1;
const A_DRM_OUT: i32 = 2;
const A_INPUT: i32 = 3;
const A_TIME_IN: i32 = 4;
const A_TIME_OUT: i32 = 5;
const A_DRC_MAG: i32 = 6;
const A_DRC_TOP: i32 = 7;
const A_ACEL: i32 = 8;

/// Serializes the tests that use global state of the sbrdec shims (the transposer of
/// shim/sbrdec_hbe.cpp). The harness of shim/sbrdec_e2e.cpp keeps its state per thread
/// (`thread_local`), so that `e2e_streams` runs several streams at once without this lock.
pub(crate) fn harness_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn ev(k: i32) -> i32 {
    unsafe { cref_sbrdec_e2e_ev_int(k) }
}

fn ev_array<T: Copy + Default>(k: i32) -> Vec<T> {
    let n = unsafe { cref_sbrdec_e2e_ev_len(k) } as usize;
    let mut v = vec![T::default(); n];
    if n > 0 {
        unsafe { cref_sbrdec_e2e_ev_copy(k, v.as_mut_ptr() as *mut c_void) };
    }
    v
}

/// The bitstream of an event (state `ev(base..base + 8)` on a copy of the buffer).
fn bitstream(buf: &mut [u8], base: i32) -> FdkBitStream {
    let mut bs = FdkBitStream::default();
    fdk_init_bit_stream(&mut bs, &BitMem::from_slice(buf), ev(base + 7) as u32, 0, BS_READER);
    bs.cache_word = ev(base) as u32;
    bs.bits_in_cache = ev(base + 1) as u32;
    bs.config_cache = ev(base + 2) as u32;
    bs.h_bit_buf.valid_bits = ev(base + 3) as u32;
    bs.h_bit_buf.read_offset = ev(base + 4) as u32;
    bs.h_bit_buf.write_offset = ev(base + 5) as u32;
    bs.h_bit_buf.bit_ndx = ev(base + 6) as u32;
    bs
}

fn bs_state(bs: &FdkBitStream) -> [i32; 7] {
    [
        bs.cache_word as i32,
        bs.bits_in_cache as i32,
        bs.config_cache as i32,
        bs.h_bit_buf.valid_bits as i32,
        bs.h_bit_buf.read_offset as i32,
        bs.h_bit_buf.write_offset as i32,
        bs.h_bit_buf.bit_ndx as i32,
    ]
}

// ------------------------------------------------------------------------------------------
// State dump (mirrors dump_sbr() of shim/sbrdec_e2e.cpp).

pub(crate) struct Dump(pub(crate) Vec<i32>);

impl Dump {
    fn push(&mut self, x: i32) {
        self.0.push(x);
    }
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend(b.iter().map(|&x| x as i32));
    }
    fn ints(&mut self, v: &[i32]) {
        self.0.extend_from_slice(v);
    }
    /// `n` values of `v` (missing values as 0).
    fn ints_n(&mut self, v: &[i32], n: usize) {
        for k in 0..n {
            self.push(v.get(k).copied().unwrap_or(0));
        }
    }
    fn mark(&mut self, id: i32) {
        self.push(0x5EC00000 + id);
    }

    fn fb(&mut self, fb: &QmfFilterBank) {
        self.push(fb.filter_size);
        self.push(fb.filter_scale);
        self.push(fb.no_channels);
        self.push(fb.no_col);
        self.push(fb.lsb);
        self.push(fb.usb);
        self.push(fb.syn_scalefactor);
        self.push(fb.out_scalefactor);
        self.push(fb.out_gain_m);
        self.push(fb.out_gain_e);
        self.push(fb.flags as i32);
        self.push(fb.p_stride as i32);
        self.push(fb.filter_states as i32);
    }

    fn qd(&mut self, qd: &FdkQmfDomain) {
        let gc = &qd.global_conf;
        self.mark(10);
        for v in [
            gc.qmf_domain_explicit_config as i32,
            gc.n_input_channels as i32,
            gc.n_input_channels_requested as i32,
            gc.n_output_channels as i32,
            gc.n_output_channels_requested as i32,
            gc.park_channel as i32,
            gc.park_channel_requested as i32,
            gc.flags as i32,
            gc.flags_requested as i32,
            gc.n_bands_analysis as i32,
            gc.n_bands_analysis_requested as i32,
            gc.n_bands_synthesis as i32,
            gc.n_bands_synthesis_requested as i32,
            gc.n_qmf_time_slots as i32,
            gc.n_qmf_time_slots_requested as i32,
            gc.n_qmf_ov_time_slots as i32,
            gc.n_qmf_ov_time_slots_requested as i32,
            gc.n_qmf_proc_bands as i32,
            gc.n_qmf_proc_bands_requested as i32,
            gc.n_qmf_proc_channels as i32,
            gc.n_qmf_proc_channels_requested as i32,
        ] {
            self.push(v);
        }
        for b in qd.mem.work_buffer.iter() {
            self.push(b.is_some() as i32);
        }
        for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
            let inp = &qd.qmf_domain_in[ch];
            self.mark(11);
            self.push(inp.p_global_conf as i32);
            self.fb(&inp.fb);
            self.push(inp.scaling.lb_scale);
            self.push(inp.scaling.ov_lb_scale);
            self.push(inp.scaling.hb_scale);
            self.push(inp.scaling.ov_hb_scale);
            self.push(inp.work_buf_n_time_slots as i32);
            self.push(inp.work_buf_n_bands as i32);
            self.push(inp.work_buffer_offset as i32);
            self.push(inp.work_buffer_sect_size as i32);
            self.push(inp.p_ana_qmf_states.is_some() as i32);
            if let Some(s) = inp.p_ana_qmf_states.as_deref() {
                self.ints_n(s, 10 * inp.fb.no_channels.max(0) as usize);
            }
            self.push(qd.mem.overlap_buffer[ch].is_some() as i32);
            self.push(inp.p_work_buffer as i32);
            self.push(inp.h_qmf_slots_real.is_some() as i32);
            self.push(inp.h_qmf_slots_imag.is_some() as i32);
            let n_slots = gc.n_qmf_time_slots as usize + gc.n_qmf_ov_time_slots as usize;
            let n_bands = gc.n_qmf_proc_bands as usize;
            for slots in [&inp.h_qmf_slots_real, &inp.h_qmf_slots_imag] {
                if let (Some(p), true) = (slots.as_deref(), inp.p_global_conf) {
                    for s in 0..n_slots {
                        let ptr = p.get(s).copied().unwrap_or_default();
                        self.push((ptr != QmfSlotPtr::Null) as i32);
                        if ptr != QmfSlotPtr::Null {
                            self.ints_n(qd.mem.slot(ptr), n_bands);
                        }
                    }
                }
            }
        }
        for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
            let out = &qd.qmf_domain_out[ch];
            self.mark(12);
            self.fb(&out.fb);
            self.push(out.p_syn_qmf_states.is_some() as i32);
            if let Some(s) = out.p_syn_qmf_states.as_deref() {
                self.ints_n(s, 9 * out.fb.no_channels.max(0) as usize);
            }
        }
    }

    pub(crate) fn ps(&mut self, ps: Option<&PsDec>) {
        self.mark(20);
        self.push(ps.is_some() as i32);
        let Some(ps) = ps else {
            return;
        };
        self.push(ps.no_sub_samples as i32);
        self.push(ps.no_channels as i32);
        self.push(ps.proc_frame_based as i32);
        self.push(ps.b_ps_data_avail[0]);
        self.push(ps.b_ps_data_avail[1]);
        self.push(ps.ps_decoded_prv as i32);
        self.push(ps.bs_last_slot as i32);
        self.push(ps.bs_read_slot as i32);
        self.push(ps.process_slot as i32);
        self.bytes(pod_bytes(&ps.bs_data[0]));
        self.bytes(pod_bytes(&ps.bs_data[1]));
        self.mark(21);
        for &v in ps.a_iid_prev_frame_index.iter() {
            self.push(v as u8 as i32);
        }
        for &v in ps.a_icc_prev_frame_index.iter() {
            self.push(v as u8 as i32);
        }
        self.push(ps.b_prev_frame_fine_iid_q as i32);
        self.push(ps.prev_freq_res_iid as i32);
        self.push(ps.prev_freq_res_icc as i32);
        self.push(ps.last_usb as i32);
        let a = &ps.hybrid_analysis;
        self.ints_n(&a.p_lf_memory, 2 * 13 * 3);
        self.mark(22);
        for k in 0..3 {
            self.push(a.buffer_lf_real[k] as i32);
        }
        for k in 0..3 {
            self.push(a.buffer_lf_imag[k] as i32);
        }
        self.push(a.buffer_lf_pos);
        self.push(a.buffer_hf_pos);
        self.push(a.nr_bands);
        self.push(a.cplx_bands);
        self.push(a.hf_mode as i32);
        self.push(a.lf_memory_size as i32);
        self.push(a.hf_memory_size as i32);
        for k in 0..2 {
            self.push(ps.hybrid_synthesis[k].nr_bands);
            self.push(ps.hybrid_synthesis[k].cplx_bands);
        }
        let dc = &ps.ap_decor;
        self.mark(23);
        let off = |o: Option<usize>| -> i32 {
            match (o, dc.state_buffer_cplx.is_some()) {
                (None, _) => -1,
                (Some(_), false) => -2,
                (Some(o), true) => o as i32,
            }
        };
        self.push(dc.l_state_buffer_cplx);
        self.push(dc.l_delay_buffer_cplx);
        self.push(off(dc.delay_buffer_cplx));
        self.ints(&dc.reverb_band_delay_buffer_index);
        self.bytes(&dc.state_buffer_offset);
        for f in dc.filter.iter() {
            self.push(off(f.state_cplx));
            self.push(off(f.delay_buffer_cplx));
        }
        self.push(dc.ducker.hybrid_bands);
        self.push(dc.ducker.parameter_bands);
        self.push(dc.ducker.partially_complex);
        self.push(dc.ducker.ducker_type);
        self.ints(&dc.ducker.smooth_dir_rev_nrg);
        self.ints(&dc.ducker.peak_decay);
        self.ints(&dc.ducker.peak_diff);
        self.push(dc.ducker.max_val_direct_data);
        self.push(dc.ducker.max_val_reverb_data);
        self.push(dc.ducker.scale_direct_nrg as i32);
        self.push(dc.ducker.scale_reverb_nrg as i32);
        self.push(dc.ducker.scale_smooth_dir_rev_nrg as i32);
        self.push(dc.ducker.headroom_smooth_dir_rev_nrg as i32);
        self.push(dc.numbins);
        self.push(dc.partially_complex);
        self.push(dc.state_buffer_cplx.is_some() as i32);
        self.ints_n(dc.state_buffer_cplx.as_deref().unwrap_or(&[]), 2 * (825 + 373));
        self.mark(24);
        self.ints(&ps.h11r_prev);
        self.ints(&ps.h12r_prev);
        self.ints(&ps.h21r_prev);
        self.ints(&ps.h22r_prev);
    }

    pub(crate) fn hbe(&mut self, h: Option<&HbeTransposer>) {
        self.mark(40);
        self.push(h.is_some() as i32);
        let Some(h) = h else {
            return;
        };
        self.ints(&h.ana_qmf_states);
        self.ints(&h.syn_qmf_states);
        self.ints(&h.x_over_qmf);
        for v in [
            h.max_stretch,
            h.time_domain_win_len,
            h.qmf_in_buf_size,
            h.qmf_out_buf_size,
            h.no_cols,
            h.no_channels,
            h.start_band,
            h.stop_band,
            h.b_sbr41,
        ] {
            self.push(v);
        }
        self.mark(41);
        self.ints_n(&h.in_buf_f, 64 + 20 + 1);
        let n = h.qmf_in_buf_size.max(0) as usize * 64;
        self.ints_n(&h.qmf_in_buf_real_f, n);
        self.ints_n(&h.qmf_in_buf_imag_f, n);
        self.ints(&h.qmf_buffer_codec_temp_slot_f);
        self.fb(&h.hbe_analysisc_qmf);
        self.fb(&h.hbe_synthesis_qmf);
        self.mark(42);
        self.ints(&h.qmf_hbe_buf_real_f);
        self.ints(&h.qmf_hbe_buf_imag_f);
        self.ints(&h.b_x_products);
        self.push(h.kstart);
        self.push(h.synth_size);
        self.ints(&h.highband_exp);
        self.ints(&h.target_exp);
    }

    fn matrix(&mut self, m: &[i32], rows: usize, nch: usize) {
        self.push(!m.is_empty() as i32);
        if m.is_empty() {
            return;
        }
        let nch = if rows > 1 { nch } else { 0 };
        self.push((rows * nch) as i32);
        self.ints_n(m, rows * nch);
    }

    fn sbr(&mut self, h: Option<&SbrDecoderInstance>, qd: &FdkQmfDomain) {
        self.mark(1);
        self.push(h.is_some() as i32);
        let Some(h) = h else {
            self.qd(qd);
            return;
        };
        for v in [
            h.core_codec,
            h.num_sbr_elements,
            h.num_sbr_channels,
            h.sample_rate_in,
            h.sample_rate_out,
            h.codec_frame_size as i32,
            h.syn_downsample_fac as i32,
            h.downscale_factor,
            h.num_delay_frames as i32,
            h.harmonic_sbr as i32,
            h.num_flushed_frames as i32,
            h.flags as i32,
            h.sbr_in_data_headroom,
        ] {
            self.push(v);
        }
        self.mark(2);
        for el in h.sbr_header.iter() {
            for s in el.iter() {
                self.bytes(pod_bytes::<SbrHeaderData>(s));
            }
        }
        self.ps(h.h_parametric_stereo_dec.as_deref());
        for (el_idx, e) in h.p_sbr_element.iter().enumerate() {
            self.mark(3);
            self.push(e.is_some() as i32);
            let Some(e) = e.as_deref() else {
                continue;
            };
            self.bytes(pod_bytes(&e.transposer_settings));
            self.push(e.element_id);
            self.push(e.n_channels);
            self.bytes(&e.frame_error_flag);
            self.push(e.use_frame_slot as i32);
            self.bytes(&e.use_header_slot);
            for c in e.p_sbr_channel.iter() {
                self.mark(4);
                self.push(c.is_some() as i32);
                let Some(c) = c.as_deref() else {
                    continue;
                };
                self.bytes(pod_bytes(&c.frame_data[0]));
                self.bytes(pod_bytes(&c.frame_data[1]));
                self.bytes(pod_bytes(&c.prev_frame_data));
                let sd = &c.sbr_dec;
                self.mark(5);
                self.bytes(pod_bytes(&sd.sbr_calculate_envelope));
                self.mark(6);
                self.bytes(pod_bytes(&sd.lpp_trans));
                self.mark(7);
                self.bytes(pod_bytes(&sd.pvc_static_data));
                self.push(sd.scale_ov as i32);
                self.push(sd.scale_lb as i32);
                self.push(sd.scale_hbe as i32);
                self.push(sd.prev_frame_l_sbr as i32);
                self.push(sd.prev_frame_hbe_sbr as i32);
                self.push(sd.codec_frame_size);
                self.push(sd.qmf_domain_in_ch.map_or(-1, |c| c as i32));
                self.push(sd.qmf_domain_out_ch.map_or(-1, |c| c as i32));
                self.bytes(pod_bytes(&sd.sbr_drc_channel));
                self.push(sd.saved_states as i32);
                self.push(sd.apply_sbr_proc_old);
                let hd = &h.sbr_header[el_idx][0];
                let rows = hd.number_time_slots as usize * hd.time_step as usize;
                self.mark(8);
                self.matrix(&sd.hbe_slots_real, rows, sd.hbe_no_channels);
                self.matrix(&sd.hbe_slots_imag, rows, sd.hbe_no_channels);
                self.matrix(&sd.codec_qmf_buffer_real, rows, sd.hbe_no_channels);
                self.matrix(&sd.codec_qmf_buffer_imag, rows, sd.hbe_no_channels);
                self.hbe(sd.h_hbe.as_deref());
            }
        }
        self.qd(qd);
    }
}

/// The state dump of the C++ PS decoder at `ps` (mirrors [`Dump::ps`]).
pub(crate) fn dump_ps_reference(ps: *mut c_void) -> Vec<i32> {
    let n = unsafe { cref_sbrdec_e2e_dump_ps(ps) } as usize;
    let mut v = vec![0i32; n];
    unsafe { cref_sbrdec_e2e_dump_copy(v.as_mut_ptr()) };
    v
}

/// The state dump of the C++ HBE transposer at `h` (mirrors [`Dump::hbe`]).
pub(crate) fn dump_hbe_reference(h: *mut c_void) -> Vec<i32> {
    let n = unsafe { cref_sbrdec_e2e_dump_hbe(h) } as usize;
    let mut v = vec![0i32; n];
    unsafe { cref_sbrdec_e2e_dump_copy(v.as_mut_ptr()) };
    v
}

fn dump_replay() -> Vec<i32> {
    let n = unsafe { cref_sbrdec_e2e_dump() } as usize;
    let mut v = vec![0i32; n];
    unsafe { cref_sbrdec_e2e_dump_copy(v.as_mut_ptr()) };
    v
}

/// The first difference of two dumps, described with the section marker before it. With the
/// environment variable `SBRDEC_E2E_DIFFS=N`, up to N differences are listed.
pub(crate) fn first_diff(a: &[i32], b: &[i32]) -> Option<String> {
    let max: usize = std::env::var("SBRDEC_E2E_DIFFS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let n = a.len().min(b.len());
    let mut out = Vec::new();
    for k in 0..n {
        if a[k] != b[k] {
            let mut sec = (0, 0);
            for j in (0..=k).rev() {
                if (a[j] as u32 & 0xFFF0_0000) == 0x5EC0_0000 && a[j] == b[j] {
                    sec = (a[j] - 0x5EC00000, j);
                    break;
                }
            }
            out.push(format!(
                "value {k} (section {} + {}): port {} ({:#x}), reference {} ({:#x})",
                sec.0,
                k - sec.1,
                a[k],
                a[k],
                b[k],
                b[k]
            ));
            if out.len() >= max {
                break;
            }
        }
    }
    if out.is_empty() && a.len() != b.len() {
        out.push(format!("dump length: port {}, reference {}", a.len(), b.len()));
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("\n  "))
    }
}

// ------------------------------------------------------------------------------------------
// The port side of the replay.

#[derive(Default)]
pub(crate) struct Port {
    sbr: Option<Box<SbrDecoderInstance>>,
    qd: FdkQmfDomain,
    pub(crate) events: usize,
    pub(crate) applies: usize,
    pub(crate) error: Option<String>,
    pub(crate) kinds: [usize; 32],
    /// Applies with: HBE, PVC, inter-TES, PS, ELD grid, 4:1, low power, downsampled SBR, DRC.
    pub(crate) cov: [usize; 10],
    /// Records only a hash of the reference results of every event (no port).
    pub(crate) ref_only: bool,
    pub(crate) ref_hashes: Vec<u64>,
}

impl Port {
    fn fail(&mut self, msg: String) {
        if self.error.is_none() {
            self.error = Some(format!("event {} (kind {}): {msg}", self.events, ev(0)));
        }
    }

    fn check<T: PartialEq + std::fmt::Debug>(&mut self, what: &str, port: T, reference: T) {
        if port != reference {
            self.fail(format!("{what}: port {port:?}, reference {reference:?}"));
        }
    }

    fn on_event(&mut self, kind: i32) {
        self.events += 1;
        if self.ref_only {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            let mut add = |v: i32| {
                h = (h ^ v as u32 as u64).wrapping_mul(0x0000_0100_0000_01b3);
            };
            for k in 0..64 {
                add(ev(k));
            }
            for v in ev_array::<i32>(A_TIME_OUT) {
                add(v);
            }
            for v in dump_replay() {
                add(v);
            }
            self.ref_hashes.push(h);
            return;
        }
        static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *TRACE.get_or_init(|| std::env::var_os("SBRDEC_E2E_TRACE").is_some()) {
            eprintln!("event {} kind {kind} ret {} p {:?}", self.events, ev(1), (16..30).map(ev).collect::<Vec<_>>());
        }
        if self.error.is_some() {
            return;
        }
        self.kinds[(kind as usize).min(31)] += 1;

        // QMF domain configuration of the decoder
        {
            let gc = &mut self.qd.global_conf;
            gc.qmf_domain_explicit_config = ev(2) as u8;
            gc.n_input_channels_requested = ev(3) as u8;
            gc.n_output_channels_requested = ev(4) as u8;
            gc.park_channel = ev(5) as u8;
            gc.park_channel_requested = ev(6) as u8;
            gc.flags_requested = ev(7) as u32;
            gc.n_bands_analysis_requested = ev(8) as u8;
            gc.n_bands_synthesis_requested = ev(9) as u16;
            gc.n_qmf_time_slots_requested = ev(10) as u8;
            gc.n_qmf_ov_time_slots_requested = ev(11) as u8;
            gc.n_qmf_proc_bands_requested = ev(12) as u8;
            gc.n_qmf_proc_channels_requested = ev(13) as u8;
        }

        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run(kind)));
        if let Err(e) = r {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            self.fail(format!("port panicked: {msg}"));
            return;
        }
        let failed = self.error.is_some();
        if kind == EV_APPLY && !failed {
            self.coverage();
        }

        let port = {
            let mut d = Dump(Vec::with_capacity(1 << 16));
            d.sbr(self.sbr.as_deref(), &self.qd);
            d.0
        };
        let reference = dump_replay();
        let diff = first_diff(&port, &reference);
        match (failed, diff) {
            (false, Some(diff)) => self.fail(format!("state: {diff}")),
            (true, diff) => {
                // the state after a failed call, for the diagnosis
                if let Some(e) = self.error.as_mut() {
                    e.push_str(&format!("; state: {}", diff.as_deref().unwrap_or("equal")));
                }
            }
            _ => {}
        }
    }

    /// Counts the tools active in the frame that was just applied.
    fn coverage(&mut self) {
        let Some(s) = self.sbr.as_deref() else {
            return;
        };
        let f = s.flags;
        let mut hbe = false;
        let mut pvc = false;
        let mut ites = false;
        let mut drc = false;
        for (ei, e) in s.p_sbr_element.iter().enumerate() {
            let Some(e) = e.as_deref() else {
                continue;
            };
            // the slot that was just applied: useFrameSlot (no delay frames in use) or the last
            let slot = e.use_frame_slot as usize % 2;
            let hdr = &s.sbr_header[ei][e.use_header_slot[slot] as usize % 2];
            pvc |= hdr.bs_info.pvc_mode > 0;
            for c in e.p_sbr_channel.iter().flatten() {
                drc |= c.sbr_dec.sbr_drc_channel.enable != 0;
                for fd in c.frame_data.iter() {
                    hbe |= f & SBRDEC_USAC_HARMONICSBR != 0 && fd.sbr_patching_mode == 0;
                    ites |= f & SBRDEC_USAC_ITES != 0 && fd.i_tes_active != 0;
                }
            }
        }
        let flags = [
            hbe,
            pvc,
            ites,
            f & SBRDEC_PS_DECODED != 0,
            f & SBRDEC_ELD_GRID != 0,
            f & SBRDEC_QUAD_RATE != 0,
            f & SBRDEC_LOW_POWER != 0,
            f & SBRDEC_DOWNSAMPLE != 0,
            drc,
            f & SBRDEC_PS_DECODED != 0 && f & SBRDEC_DOWNSAMPLE != 0,
        ];
        for (k, &on) in flags.iter().enumerate() {
            self.cov[k] += on as usize;
        }
    }

    fn run(&mut self, kind: i32) {
        let ret = ev(1);
        match kind {
            EV_OPEN => {
                if self.sbr.is_some() {
                    sbr_decoder_close(&mut self.sbr);
                }
                fdk_qmf_domain_close(&mut self.qd);
                self.qd = FdkQmfDomain::default();
                let r = sbr_decoder_open(&mut self.sbr);
                self.check("sbrDecoder_Open", r, ret);
            }
            EV_INIT_ELEMENT => {
                let mut cc = ev(25) as u8;
                let Some(s) = self.sbr.as_deref_mut() else {
                    return self.fail("no instance".into());
                };
                let r = sbr_decoder_init_element(
                    s,
                    &mut self.qd,
                    ev(16),
                    ev(17),
                    ev(18),
                    ev(19),
                    ev(20),
                    ev(21),
                    ev(22) as u8,
                    ev(23) as u8,
                    ev(24) as u8,
                    &mut cc,
                    ev(26),
                );
                self.check("sbrDecoder_InitElement", r, ret);
                self.check("configChanged", cc as i32, ev(27));
            }
            EV_HEADER => {
                let mut buf: Vec<u8> = ev_array(A_BS);
                let mut bs = bitstream(&mut buf, 40);
                let mut cc = ev(25) as u8;
                let Some(s) = self.sbr.as_deref_mut() else {
                    return self.fail("no instance".into());
                };
                let r = sbr_decoder_header(
                    s,
                    &mut self.qd,
                    &mut bs,
                    ev(16),
                    ev(17),
                    ev(18),
                    ev(19),
                    ev(20),
                    ev(21),
                    ev(22) as u8,
                    ev(23) as u8,
                    ev(24) as u8,
                    &mut cc,
                    ev(26),
                );
                self.check("sbrDecoder_Header", r, ret);
                self.check("configChanged", cc as i32, ev(27));
                let after: Vec<i32> = (48..55).map(ev).collect();
                self.check("bitstream", bs_state(&bs).to_vec(), after);
            }
            EV_SET_PARAM => {
                let s = if ev(18) != 0 { None } else { self.sbr.as_deref_mut() };
                let r = sbr_decoder_set_param(s, ev(16), ev(17));
                self.check("sbrDecoder_SetParam", r, ret);
            }
            EV_PARSE => {
                let mut buf: Vec<u8> = ev_array(A_BS);
                let mut bs = bitstream(&mut buf, 40);
                let mut drm: Vec<u8> = ev_array(A_DRM_IN);
                let drm_out: Vec<u8> = ev_array(A_DRM_OUT);
                let ac: Vec<u32> = ev_array(A_ACEL);
                let mut count = ev(17);
                let Some(s) = self.sbr.as_deref_mut() else {
                    return self.fail("no instance".into());
                };
                let r = sbr_decoder_parse(
                    s,
                    &mut bs,
                    &mut drm,
                    &mut count,
                    ev(18),
                    ev(19),
                    ev(20),
                    ev(21),
                    ev(22) as u32,
                    &ac,
                );
                self.check("sbrDecoder_Parse", r, ret);
                self.check("count", count, ev(23));
                let after: Vec<i32> = (48..55).map(ev).collect();
                self.check("bitstream", bs_state(&bs).to_vec(), after);
                self.check("pDrmBsBuffer", drm, drm_out);
            }
            EV_APPLY => {
                self.applies += 1;
                let input: Vec<i32> = ev_array(A_INPUT);
                let mut time: Vec<i32> = ev_array(A_TIME_IN);
                let time_ref: Vec<i32> = ev_array(A_TIME_OUT);
                let mut nch = ev(17);
                let mut sr = ev(18);
                let mut psd = ev(21) as u8;
                let mut odh = 0;
                let mut map = FdkChannelMapDescr::default();
                let map_ok = ev(27) >= 0;
                if map_ok {
                    if ev(29) != 1 {
                        return self.fail("channel map descriptor without the default table".into());
                    }
                    fdk_ch_map_descr_init(&mut map, None, 0, ev(27) as u32);
                    self.check("mapInfoTabLen", map.map_info_tab_len as i32, ev(28));
                }
                let Some(s) = self.sbr.as_deref_mut() else {
                    return self.fail("no instance".into());
                };
                let r = sbr_decoder_apply(
                    s,
                    &mut self.qd,
                    &input,
                    &mut time,
                    ev(16),
                    &mut nch,
                    &mut sr,
                    if map_ok { Some(&map) } else { None },
                    ev(19),
                    ev(20),
                    &mut psd,
                    ev(22),
                    &mut odh,
                );
                self.check("sbrDecoder_Apply", r, ret);
                self.check("numChannels", nch, ev(23));
                self.check("sampleRate", sr, ev(24));
                self.check("psDecoded", psd as i32, ev(25));
                self.check("outDataHeadroom", odh, ev(26));
                if time != time_ref {
                    let k = time.iter().zip(&time_ref).position(|(a, b)| a != b).unwrap_or(0);
                    self.fail(format!(
                        "timeData[{k}]: port {}, reference {}",
                        time.get(k).copied().unwrap_or(0),
                        time_ref.get(k).copied().unwrap_or(0)
                    ));
                }
            }
            EV_DRC_FEED => {
                let mag: Vec<i32> = ev_array(A_DRC_MAG);
                let top: Vec<u16> = ev_array(A_DRC_TOP);
                let Some(s) = self.sbr.as_deref_mut() else {
                    return self.fail("no instance".into());
                };
                let r = sbr_decoder_drc_feed_channel(
                    s,
                    ev(16),
                    ev(17) as u32,
                    if ev(21) != 0 { None } else { Some(&mag) },
                    ev(18),
                    ev(19) as i16,
                    ev(20) as u8,
                    &top,
                );
                self.check("sbrDecoder_drcFeedChannel", r, ret);
            }
            EV_DRC_DISABLE => {
                if let Some(s) = self.sbr.as_deref_mut() {
                    sbr_decoder_drc_disable(s, ev(16));
                }
            }
            EV_FREE_MEM => {
                let r = sbr_decoder_free_mem(self.sbr.as_deref_mut());
                self.check("sbrDecoder_FreeMem", r, ret);
            }
            EV_CLOSE => {
                let r = sbr_decoder_close(&mut self.sbr);
                self.check("sbrDecoder_Close", r, ret);
            }
            EV_GET_DELAY => {
                let r = sbr_decoder_get_delay(self.sbr.as_deref());
                self.check("sbrDecoder_GetDelay", r as i32, ret);
            }
            EV_QMF_CLEAR_REQUESTED => fdk_qmf_domain_clear_requested(&mut self.qd.global_conf),
            EV_QMF_CONFIGURE => {
                let r = fdk_qmf_domain_configure(&mut self.qd);
                self.check("FDK_QmfDomain_Configure", r, ret);
            }
            EV_QMF_FREE_MEM => fdk_qmf_domain_free_mem(&mut self.qd),
            EV_QMF_CLEAR_PERSISTENT => {
                let r = fdk_qmf_domain_clear_persistent_memory(Some(&mut self.qd));
                self.check("FDK_QmfDomain_ClearPersistentMemory", r, ret);
            }
            EV_QMF_CLOSE => fdk_qmf_domain_close(&mut self.qd),
            _ => self.fail(format!("unknown event {kind}")),
        }
    }
}

pub(crate) extern "C" fn callback(ctx: *mut c_void, kind: i32) {
    let port = unsafe { &mut *(ctx as *mut Port) };
    port.on_event(kind);
}

// ------------------------------------------------------------------------------------------
// Streams.

fn work_dir() -> PathBuf {
    if let Some(w) = std::env::var_os("FDK_AAC_RS_WORK_DIR") {
        return PathBuf::from(w);
    }
    // <work>/build-cref/<variant>/<key>/libfdkaac_ref.a
    let lib = PathBuf::from(env!("FDK_AAC_RS_CREF_LIB"));
    lib.ancestors().nth(4).map(PathBuf::from).unwrap_or_default()
}

/// The AudioSpecificConfig and the samples (offset, size) of an MP4 track.
type Mp4Track = (Vec<u8>, Vec<(usize, usize)>);

/// A minimal MP4 demuxer: the AudioSpecificConfig and the samples (offset, size) of the first
/// audio track (no movie fragments).
fn mp4_demux(data: &[u8]) -> Option<Mp4Track> {
    fn boxes(d: &[u8]) -> Vec<([u8; 4], &[u8])> {
        let mut out = Vec::new();
        let mut p = 0usize;
        while p + 8 <= d.len() {
            let mut size = u32::from_be_bytes(d[p..p + 4].try_into().unwrap()) as usize;
            let typ: [u8; 4] = d[p + 4..p + 8].try_into().unwrap();
            let mut hdr = 8;
            if size == 1 {
                if p + 16 > d.len() {
                    break;
                }
                size = u64::from_be_bytes(d[p + 8..p + 16].try_into().unwrap()) as usize;
                hdr = 16;
            } else if size == 0 {
                size = d.len() - p;
            }
            if size < hdr || p + size > d.len() {
                break;
            }
            out.push((typ, &d[p + hdr..p + size]));
            p += size;
        }
        out
    }
    fn find<'a>(d: &'a [u8], t: &[u8; 4]) -> Option<&'a [u8]> {
        boxes(d).into_iter().find(|(ty, _)| ty == t).map(|(_, b)| b)
    }
    fn be32(d: &[u8], p: usize) -> Option<u32> {
        d.get(p..p + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    }
    fn desc_len(d: &[u8], p: &mut usize) -> Option<usize> {
        let mut len = 0usize;
        for _ in 0..4 {
            let b = *d.get(*p)?;
            *p += 1;
            len = (len << 7) | (b & 0x7f) as usize;
            if b & 0x80 == 0 {
                break;
            }
        }
        Some(len)
    }
    fn asc_from_esds(e: &[u8]) -> Option<Vec<u8>> {
        let mut p = 4; // version, flags
        if *e.get(p)? != 0x03 {
            return None;
        }
        p += 1;
        desc_len(e, &mut p)?;
        p += 2; // ES_ID
        let flags = *e.get(p)?;
        p += 1;
        if flags & 0x80 != 0 {
            p += 2;
        }
        if flags & 0x40 != 0 {
            p += 1 + *e.get(p)? as usize;
        }
        if flags & 0x20 != 0 {
            p += 2;
        }
        if *e.get(p)? != 0x04 {
            return None;
        }
        p += 1;
        desc_len(e, &mut p)?;
        p += 13;
        if *e.get(p)? != 0x05 {
            return None;
        }
        p += 1;
        let n = desc_len(e, &mut p)?;
        e.get(p..p + n).map(|s| s.to_vec())
    }

    let moov = find(data, b"moov")?;
    for (t, trak) in boxes(moov) {
        if &t != b"trak" {
            continue;
        }
        let mdia = find(trak, b"mdia")?;
        let hdlr = find(mdia, b"hdlr")?;
        if hdlr.get(8..12)? != b"soun" {
            continue;
        }
        let stbl = find(find(mdia, b"minf")?, b"stbl")?;
        let stsd = find(stbl, b"stsd")?;
        let entries = boxes(stsd.get(8..)?);
        let (ety, entry) = entries.first()?;
        if ety != b"mp4a" {
            return None;
        }
        let asc = asc_from_esds(find(entry.get(28..)?, b"esds")?)?;
        let stsz = find(stbl, b"stsz")?;
        let fixed = be32(stsz, 4)? as usize;
        let count = be32(stsz, 8)? as usize;
        let sizes: Vec<usize> = (0..count)
            .map(|i| if fixed != 0 { Some(fixed) } else { be32(stsz, 12 + 4 * i).map(|v| v as usize) })
            .collect::<Option<_>>()?;
        let stsc = find(stbl, b"stsc")?;
        let n_stsc = be32(stsc, 4)? as usize;
        let stsc_e: Vec<(u32, u32)> =
            (0..n_stsc).map(|i| Some((be32(stsc, 8 + 12 * i)?, be32(stsc, 12 + 12 * i)?))).collect::<Option<_>>()?;
        let offsets: Vec<u64> = if let Some(stco) = find(stbl, b"stco") {
            let n = be32(stco, 4)? as usize;
            (0..n).map(|i| be32(stco, 8 + 4 * i).map(|v| v as u64)).collect::<Option<_>>()?
        } else {
            let co64 = find(stbl, b"co64")?;
            let n = be32(co64, 4)? as usize;
            (0..n)
                .map(|i| co64.get(8 + 8 * i..16 + 8 * i).map(|b| u64::from_be_bytes(b.try_into().unwrap())))
                .collect::<Option<_>>()?
        };
        let mut samples = Vec::with_capacity(count);
        let mut s = 0usize;
        for (ci, &off) in offsets.iter().enumerate() {
            let chunk = ci as u32 + 1;
            let spc = stsc_e.iter().rev().find(|(first, _)| *first <= chunk).map(|e| e.1).unwrap_or(0);
            let mut o = off as usize;
            for _ in 0..spc {
                if s >= count {
                    break;
                }
                samples.push((o, sizes[s]));
                o += sizes[s];
                s += 1;
            }
        }
        return Some((asc, samples));
    }
    None
}

struct Stream {
    path: &'static str,
    /// transport type: 0 raw (MP4), 2 ADTS, 10 LOAS
    tt: i32,
}

const STREAMS: &[Stream] = &[
    // HE-AAC (dual rate), mono, stereo, multichannel
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_cm_48_1_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_cm_48_2_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_cm_48_5.1_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_sr_16_2_fsaac08_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_sr_44_2_fsaac22_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_sr_96_2_fsaac48_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_twi_48_1_fsaac24_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_sig_48_2_sig1_amv1.ass", tt: 10 },
    // downsampled SBR (single rate)
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_sr_24_2_fsaac24_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_sr_48_2_fsaac48_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_twi_22_1_fsaac22_amv0.ass", tt: 10 },
    // HE-AAC v2 (PS)
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_ps_00_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_ps_03_sig1_amv1.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedAss/add-opt/al_sbr_ps_06_new_amv0.ass", tt: 10 },
    Stream { path: "conformance/14496-26/compressedMp4/al_sbr_ps_04_new.mp4", tt: 0 },
    // explicit/backward compatible signalling in MP4
    Stream { path: "conformance/14496-26/compressedMp4/al_sbr_cm_48_2.mp4", tt: 0 },
    Stream { path: "conformance/14496-26/compressedMp4/al_sbr_esbr_01_48_2.mp4", tt: 0 },
    Stream { path: "conformance/14496-26/compressedMp4/al_sbr_gen_44_7.1Top.mp4", tt: 0 },
    // USAC eSBR: HBE, PVC, inter-TES, 8:3, 2:1, 4:1
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c3_Htq_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c2_Htq_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c4_Htq_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c3_Pvc_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c4_Pvc_0x12.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c3_Tes_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c3_Tes_0x15.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c3_Eaa_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c3_Hgt_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c3_Qma_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c3_Xo_0x05.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_1_c4_TttXo_0x03.mp4", tt: 0 },
    Stream { path: "conformance/23003-7/compressedMp4/eSbr_2_c3_CmMps0_0x03.mp4", tt: 0 },
    // SBR with MPEG-4 DRC (dynamic_range_info): no conformance stream combines them. Encoded by
    // the reference encoder (work/build-ref/ref-arm64/harness/fdkenc-ref) from the first 20 s
    // of corpus files with
    //   -f s16le --param AACENC_METADATA_MODE=1 --meta drc_profile=3 --meta comp_profile=4
    //   --meta drc_TargetRefLevel=-80 --meta comp_TargetRefLevel=-80
    //   --meta prog_ref_level_present=1 --meta prog_ref_level=-6400
    // and (SHA-256 of the output):
    //   he_drc_44_1.adts: music_b_mono_44k, --rate 44100 --channels 1 AACENC_AOT=5
    //     AACENC_BITRATE=32000 AACENC_TRANSMUX=2
    //   ps_drc_44.adts: music_d_44k, --rate 44100 --channels 2 AACENC_AOT=29
    //     AACENC_BITRATE=32000 AACENC_TRANSMUX=2
    //   he_drc_48_2.adts: broadcast_a_48k, --rate 48000 --channels 2 AACENC_AOT=5
    //     AACENC_BITRATE=64000 AACENC_TRANSMUX=2
    //   eld_drc_44_2.loas: music_d_44k, --rate 44100 --channels 2 AACENC_AOT=39
    //     AACENC_SBR_MODE=1 AACENC_BITRATE=64000 AACENC_TRANSMUX=10
    //   ps_ds_drc_44.loas: HE-AAC v2 with downsampled SBR (the PS path with 32 synthesis
    //     channels; the encoder only writes dual-rate PS): music_d_44k, --rate 44100
    //     --channels 2 AACENC_AOT=29 AACENC_BITRATE=32000 AACENC_TRANSMUX=10
    //     AACENC_SIGNALING_MODE=2 (SHA-256 of the encoder output
    //     fd9cf4a1f2188277f5b0d932865dc6708b776a4d707c33c8c8730358aae0af24), then in every
    //     StreamMuxConfig (useSameStreamMux == 0) the extensionSamplingFrequencyIndex of the
    //     AudioSpecificConfig (4 bits from bit 53 of the AudioSyncStream frame: 11 sync, 13
    //     length, 1 useSameStreamMux, 15 StreamMuxConfig fields, 5 AOT, 4 sampling frequency
    //     index, 4 channel configuration) set to its samplingFrequencyIndex (4 -> 7)
    // SHA-256:
    //   he_drc_44_1.adts   cb120d1c616ec9984befc13cd05cb570bd2e904f04dbccf5b813159fab1a49c7
    //   ps_drc_44.adts     b52bd7a191f6d577aac0e92ee170a4f1fbc4c0edbcd748cede093b46e76375e4
    //   he_drc_48_2.adts   8181c8328692a45566b005fe0aa1dd1c072f4d771b0b1fc739143bdc6b844abd
    //   eld_drc_44_2.loas  3e507551b0454f0c4e18d6e341479f64a290e0b82c374837fc7115ef3de8aae0
    //   ps_ds_drc_44.loas  b46afc93929c7c752183166fae0a6f2b8a46b4114d108a18ad6eb9c4dd69c7f9
    Stream { path: "sbrdec-e2e/he_drc_44_1.adts", tt: 2 },
    Stream { path: "sbrdec-e2e/ps_drc_44.adts", tt: 2 },
    Stream { path: "sbrdec-e2e/he_drc_48_2.adts", tt: 2 },
    Stream { path: "sbrdec-e2e/eld_drc_44_2.loas", tt: 10 },
    Stream { path: "sbrdec-e2e/ps_ds_drc_44.loas", tt: 10 },
];

/// ELD with SBR streams: found in the conformance index (group `ER AAC ELD` with SBR).
fn eld_sbr_streams(work: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(work.join("conformance/index.json")) else {
        return Vec::new();
    };
    // A minimal scan of the index: entries with "aot": 39 and "sbr": 1, without LD MPEG
    // Surround (not supported by the capture).
    let mut out = Vec::new();
    for entry in text.split("\n  {").skip(1) {
        if entry.contains("\"aot\": 39,") && entry.contains("\"sbr\": 1,") && !entry.contains("ldmps") {
            if let Some(p) = entry.find("\"file\": \"") {
                let rest = &entry[p + 9..];
                if let Some(e) = rest.find('"') {
                    out.push(format!("conformance/{}", &rest[..e]));
                }
            }
        }
    }
    out.sort();
    out
}

/// Decodes one stream through the capture harness; returns (frames, events, applies) or an
/// error. `mutate` flips random bits of the access units (of the stream for LOAS).
/// The configuration of one decoding run.
#[derive(Clone, Copy, Debug)]
struct Variant {
    /// Seed of the random bit flips in the access units (of the stream for LOAS).
    mutate: Option<u64>,
    /// `AAC_QMF_LOWPOWER` (-1: decoder default, 0: HQ, 1: LP).
    qmf_mode: i32,
    /// Enables the MPEG-4 DRC of the decoder (`AAC_DRC_REFERENCE_LEVEL`,
    /// `AAC_DRC_ATTENUATION_FACTOR`, `AAC_DRC_BOOST_FACTOR`, `AAC_DRC_HEAVY_COMPRESSION`).
    drc: bool,
}

/// The result of a run: frames, events, applies, coverage, events per kind.
type RunResult = (usize, usize, usize, [usize; 10], [usize; 32]);

fn run_stream(data: &[u8], tt: i32, variant: Variant) -> Result<RunResult, String> {
    let mutate = variant.mutate;
    // The port is mutated by the callback while the C++ code runs: it is only accessed through
    // the raw pointer, and only between calls into the harness.
    let port: *mut Port = Box::into_raw(Box::<Port>::default());
    unsafe { cref_sbrdec_e2e_set_callback(callback as *mut c_void, port as *mut c_void, 1) };

    let mut rng = mutate.map(Rng::new);
    let mut mutated = |d: &[u8]| -> Vec<u8> {
        let mut v = d.to_vec();
        if let Some(r) = rng.as_mut() {
            for b in v.iter_mut() {
                if r.next_u32() % 200 == 0 {
                    *b ^= 1 << (r.next_u32() % 8);
                }
            }
        }
        v
    };

    let mut out = vec![0i16; 8 * 4096 * 2];
    let mut info = [0i32; 4];
    let mut frames = 0usize;
    // Decodes one frame: Ok(false) if more data is needed.
    let mut decode = || -> Result<bool, String> {
        let e = unsafe {
            cref_sbrdec_e2e_decode(out.as_mut_ptr(), out.len() as i32, 0, info.as_mut_ptr())
        };
        if let Some(err) = unsafe { (*port).error.clone() } {
            return Err(err);
        }
        let mut at = 0;
        let m = unsafe { cref_sbrdec_e2e_orig_mismatch(&mut at) };
        if m >= 0 {
            return Err(format!(
                "capture check: original and replay instance differ at event {m} (value {at})"
            ));
        }
        Ok(e != 0x1002) // AAC_DEC_NOT_ENOUGH_BITS
    };

    let result = (|| -> Result<(), String> {
        if unsafe { cref_sbrdec_e2e_open(tt) } != 0 {
            return Err("aacDecoder_Open failed".into());
        }
        let mut params = vec![(0x0300, variant.qmf_mode)]; // AAC_QMF_LOWPOWER
        if variant.drc {
            // AAC_DRC_REFERENCE_LEVEL, AAC_DRC_ATTENUATION_FACTOR, AAC_DRC_BOOST_FACTOR,
            // AAC_DRC_HEAVY_COMPRESSION
            params.extend([(0x0202, 64), (0x0201, 127), (0x0200, 96), (0x0203, 1)]);
        }
        for (param, value) in params {
            let r = unsafe { cref_sbrdec_e2e_set_param(param, value) };
            if r != 0 {
                return Err(format!("aacDecoder_SetParam({param:#x}, {value}): {r:#x}"));
            }
        }
        if tt == 0 {
            let (asc, samples) = mp4_demux(data).ok_or("cannot demux")?;
            let r = unsafe { cref_sbrdec_e2e_config_raw(asc.as_ptr(), asc.len() as i32) };
            if let Some(err) = unsafe { (*port).error.clone() } {
                return Err(err);
            }
            if r != 0 {
                return Err(format!("aacDecoder_ConfigRaw: {r:#x}"));
            }
            for &(o, n) in samples.iter() {
                let Some(au) = data.get(o..o + n) else {
                    break;
                };
                let au = mutated(au);
                unsafe { cref_sbrdec_e2e_fill(au.as_ptr(), au.len() as i32) };
                if decode()? {
                    frames += 1;
                }
            }
        } else {
            let data = mutated(data);
            let mut pos = 0usize;
            while pos < data.len() {
                let chunk = (data.len() - pos).min(4096);
                let used =
                    unsafe { cref_sbrdec_e2e_fill(data[pos..].as_ptr(), chunk as i32) } as usize;
                pos += used;
                let mut n = 0;
                while decode()? {
                    frames += 1;
                    n += 1;
                }
                if used == 0 && n == 0 {
                    break;
                }
            }
        }
        Ok(())
    })();
    unsafe { cref_sbrdec_e2e_close() };
    unsafe { cref_sbrdec_e2e_set_callback(core::ptr::null_mut(), core::ptr::null_mut(), 0) };
    let port = unsafe { Box::from_raw(port) };
    if let Some(err) = &port.error {
        return Err(err.clone());
    }
    result?;
    Ok((frames, port.events, port.applies, port.cov, port.kinds))
}

#[test]
fn layouts() {
    use fdk_aac_rs_sbrdec::env_extr::*;
    use fdk_aac_rs_sbrdec::sbr_dec::*;
    let rust = [
        core::mem::size_of::<SbrHeaderData>(),
        core::mem::size_of::<SbrFrameData>(),
        core::mem::size_of::<SbrPrevFrameData>(),
        core::mem::size_of::<fdk_aac_rs_sbrdec::env_calc::SbrCalculateEnvelope>(),
        core::mem::size_of::<fdk_aac_rs_sbrdec::lpp_tran::SbrLppTrans>(),
        core::mem::size_of::<fdk_aac_rs_sbrdec::pvc_dec::PvcStaticData>(),
        core::mem::size_of::<fdk_aac_rs_sbrdec::sbrdec_drc::SbrdecDrcChannel>(),
        core::mem::size_of::<fdk_aac_rs_sbrdec::lpp_tran::TransposerSettings>(),
        core::mem::size_of::<fdk_aac_rs_sbrdec::psdec::MpegPsBsData>(),
    ];
    let _ = core::mem::size_of::<SbrChannel>();
    for (k, &r) in rust.iter().enumerate() {
        assert_eq!(r as i32, unsafe { cref_sbrdec_e2e_sizeof(k as i32) }, "structure {k}");
    }
}

/// The number of threads of `e2e_streams`: RUST_TEST_THREADS (libtest's own setting) if set,
/// otherwise the available parallelism.
fn worker_threads() -> usize {
    std::env::var("RUST_TEST_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// The result of a variant of a stream.
type Outcome = (Variant, Result<RunResult, String>);

/// The variant of a job of `e2e_streams` (3 and 4: the fourth variant with the QMF mode 0 or 1).
fn job_variant(job: usize, data: &[u8]) -> Variant {
    match job {
        0 => Variant { mutate: None, qmf_mode: -1, drc: false },
        1 => Variant { mutate: Some(0x5b5 ^ data.len() as u64), qmf_mode: -1, drc: false },
        2 => Variant { mutate: None, qmf_mode: -1, drc: true },
        _ => Variant { mutate: None, qmf_mode: job as i32 - 3, drc: true },
    }
}

/// Every stream in four variants: as is, with randomly mutated access units, with DRC, and with
/// DRC in the other QMF mode (the one the first variant did not use by default). The runs are
/// independent (the harness state is thread-local) and are spread over `worker_threads` threads,
/// the largest streams first. The fourth variant depends on the result of the first: it runs
/// after it on the same thread, except for streams whose two runs would take longer than a
/// thread's share of all runs (by the stream size); for those, both QMF modes run at once with
/// the first variant, and the result of the mode the first variant did not use is kept (the
/// other run is not part of the test). The results are reported in the order of the streams and
/// variants.
#[test]
fn e2e_streams() {
    let work = work_dir();
    let mut list: Vec<(String, i32)> = STREAMS.iter().map(|s| (s.path.to_string(), s.tt)).collect();
    for p in eld_sbr_streams(&work) {
        list.push((p, 0));
    }
    let only = std::env::var("SBRDEC_E2E_ONLY").ok();
    // (path, transport type, data or None if missing)
    let mut streams: Vec<(String, i32, Option<Vec<u8>>)> = Vec::new();
    for (path, tt) in list {
        if let Some(o) = &only {
            if !path.contains(o.as_str()) {
                continue;
            }
        }
        let data = std::fs::read(work.join(&path)).ok();
        streams.push((path, tt, data));
    }
    let size = |s: usize| streams[s].2.as_ref().map_or(0, |d| d.len());
    let threads = worker_threads();
    let share = (0..streams.len()).map(|s| 4 * size(s)).sum::<usize>() / threads;
    // The jobs (stream, job, cost): 0 (followed by the fourth variant unless split), 1, 2, and
    // for split streams 3 and 4.
    let mut jobs: Vec<(usize, usize, usize)> = Vec::new();
    let mut split = vec![false; streams.len()];
    for s in 0..streams.len() {
        if streams[s].2.is_none() {
            continue;
        }
        split[s] = 2 * size(s) > share;
        jobs.extend([(s, 0, if split[s] { 1 } else { 2 } * size(s)), (s, 1, size(s)), (s, 2, size(s))]);
        if split[s] {
            jobs.extend([(s, 3, size(s)), (s, 4, size(s))]);
        }
    }
    jobs.sort_by_key(|j| std::cmp::Reverse(j.2));
    // per stream: the results of the jobs 0 to 4
    let results: std::sync::Mutex<Vec<[Option<Outcome>; 5]>> =
        std::sync::Mutex::new(streams.iter().map(|_| [None, None, None, None, None]).collect());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads.min(jobs.len()) {
            scope.spawn(|| {
                while let Some(&(s, job, _)) = jobs.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)) {
                    let (_, tt, data) = &streams[s];
                    let data = data.as_deref().unwrap();
                    let v = job_variant(job, data);
                    let r = run_stream(data, *tt, v);
                    let default_lp = matches!(&r, Ok(r) if r.3[6] > 0);
                    results.lock().unwrap()[s][job] = Some((v, r));
                    if job == 0 && !split[s] {
                        // the other QMF mode, with DRC
                        let job = if default_lp { 3 } else { 4 };
                        let v = job_variant(job, data);
                        let r = run_stream(data, *tt, v);
                        results.lock().unwrap()[s][job] = Some((v, r));
                    }
                }
            });
        }
    });
    let mut failures = Vec::new();
    let mut ran = 0;
    for ((path, _, data), outcomes) in streams.iter().zip(results.into_inner().unwrap()) {
        if data.is_none() {
            eprintln!("skipped (missing): {path}");
            continue;
        }
        let [o0, o1, o2, o3, o4] = outcomes;
        let default_lp = matches!(&o0, Some((_, Ok(r))) if r.3[6] > 0);
        for (v, r) in [o0, o1, o2, if default_lp { o3 } else { o4 }].into_iter().map(Option::unwrap) {
            ran += 1;
            match r {
                Ok((frames, events, applies, cov, kinds)) => {
                    eprintln!(
                        "ok: {path} {v:?}: {frames} frames, {events} events, {applies} applies, \
                         coverage (hbe pvc ites ps eld 4:1 lp ds drc ps+ds) {cov:?}, drc feed {} disable {}",
                        kinds[EV_DRC_FEED as usize], kinds[EV_DRC_DISABLE as usize]
                    );
                }
                Err(e) => {
                    eprintln!("FAIL: {path} {v:?}: {e}");
                    failures.push(format!("{path} {v:?}: {e}"));
                }
            }
        }
    }
    assert!(ran > 0, "no streams found under {}", work.display());
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
