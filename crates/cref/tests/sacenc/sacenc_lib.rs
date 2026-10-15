// Differential tests of fdk_aac_rs_sacenc::sacenc_lib against the C++ reference.
//
// The encoder of the library and the port are driven through their APIs with the same calls
// and the same PCM, as the AAC encoder (mps_main.cpp) drives them: every configuration it can
// request (ELDv2 at 16, 22.05, 24, 32, 44.1 and 48 kHz with the MPS frame lengths of core frame
// lengths 512/480 with and without (downsampled or dual rate) SBR, the core coder delays with
// the SBR input delays, every downmix gain), and further parameter sets (band configurations,
// coarse and EBQ quantization, low delay modes, independency factors, time alignments,
// interleaved input, re-initialization of a handle with another configuration). After every
// call the return code, the output arguments, the downmix, the spatial payload, the
// SpatialSpecificConfig (getInfo) and the complete encoder state are compared. Realistic stereo
// PCM comes from the corpus of the main checkout (work/corpus, s16le) when present; synthetic
// signals otherwise and in addition.
//
// Calls without a defined result in the reference are made on the port only, which must not
// panic.

use core::ffi::c_void;

use fdk_aac_rs_cref::sacenc_sacenc_lib as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_sacenc::sacenc_lib as r;
use fdk_aac_rs_sacenc::sacenc_lib::*;

use super::sacenc_bitstream::{frame_bytes, prev_bytes, ssc_ints};
use super::sacenc_delay::delay_ints;
use super::sacenc_dmx_tdom_enh::{corpus_files, corpus_stereo, dmx_ints, synth_stereo};
use super::sacenc_framewindowing::{fw_ints, list_ints};
use super::sacenc_staticgain::gain_ints;
use super::sacenc_tree::tree_ints;

/// The state image of `cref_sacenc_lib_state()`.
pub fn state_ints(e: &Mp4SpaceEncoder) -> Vec<i32> {
    let mut v: Vec<i32> = Vec::with_capacity(40000);
    let u = &e.user;
    v.extend([
        u.enc_mode,
        u.n_param_bands,
        u.quant_mode,
        u.b_use_coarse_quant as i32,
        u.b_ld_mode as i32,
        u.b_time_domain_dmx as i32,
        u.sample_rate as i32,
        u.frame_time_slots as i32,
        u.independency_factor as i32,
        u.time_alignment,
    ]);
    let s = &e.setup;
    v.extend([
        s.b_enc_mode_212 as i32,
        s.max_hybrid_in_static_slots as i32,
        s.max_samplingrate,
        s.max_analysis_length_time_slots,
        s.max_hybrid_bands,
        s.max_qmf_bands,
        s.max_ch_in,
        s.max_frame_time_slots,
        s.max_frame_length,
        s.max_ch_out,
        s.max_ch_tot_out,
    ]);
    v.extend(fw_ints(&e.h_frame_window));
    v.extend([
        e.n_samples_valid,
        e.n_param_bands,
        e.use_time_dom_downmix as i32,
        e.enc_mode,
        e.b_enc_mode_212_only as i32,
        e.use_frame_keep as i32,
        e.independency_factor as i32,
        e.n_sample_rate as i32,
        e.n_input_channels as i32,
        e.n_output_channels as i32,
        e.n_frame_time_slots as i32,
        e.n_qmf_bands as i32,
        e.n_hybrid_bands as i32,
        e.n_frame_length as i32,
        e.n_samples_next,
        e.n_analysis_length_time_slots,
        e.n_analysis_lookahead_time_slots,
        e.n_update_hybrid_position_time_slots,
        e.pn_output_bits[0],
        e.n_input_delay,
        e.n_output_buffer_delay,
        e.n_surround_analysis_buffer_delay,
        e.n_bitstream_delay_buffer,
        e.n_bitstream_buffer_read,
        e.n_bitstream_buffer_write,
        e.n_discard_out_frames,
        e.avoid_keep,
        e.use_coarse_quant_cld as i32,
        e.use_coarse_quant_icc as i32,
        e.use_coarse_quant_cpc as i32,
        e.use_coarse_quant_arb_dmx as i32,
        e.quant_mode,
        e.core_coder_delay,
        e.time_alignment,
        e.independency_count,
        e.independency_flag,
        e.pp_tr_curr_pos[0][0],
        e.pp_tr_curr_pos[1][0],
        e.tr_prev_pos[0],
        e.tr_prev_pos[1],
    ]);
    v.extend(list_ints(&e.frame_win_list));
    v.extend(frame_bytes(&e.save_frame).iter().map(|&b| b as i32));
    let t = &e.space_tree_setup;
    v.extend([
        t.n_param_bands as i32,
        t.b_use_coarse_quant_tto_cld as i32,
        t.b_use_coarse_quant_tto_icc as i32,
        t.quant_mode,
        t.mode,
        t.n_channels_in_max as i32,
        t.n_hybrid_bands_max as i32,
    ]);
    v.push(e.ssc_buf.n_ssc_size_bits);
    v.extend(e.ssc_buf.p_ssc.iter().map(|&b| b as i32));
    for w in &e.p_frame_window_ana_fdk {
        v.extend_from_slice(w);
    }
    for ch in 0..2 {
        let q = &e.ph_qmf_filt_in_fdk[ch];
        v.extend([
            q.filter_size,
            q.filter_scale,
            q.no_channels,
            q.no_col,
            q.lsb,
            q.usb,
            q.syn_scalefactor,
            q.out_scalefactor,
            q.out_gain_m,
            q.out_gain_e,
            q.flags as i32,
            q.p_stride as i32,
        ]);
        for i in 0..4 {
            v.push(q.p_filter.get(i).map_or(-1, |&x| x as i32));
        }
        v.extend([!q.t_cos.is_empty() as i32, !q.t_sin.is_empty() as i32, q.filter_states as i32]);
        v.extend(e.qmf_filter_states[ch].iter().map(|&x| x as i32));
    }
    for ch in 0..2 {
        v.extend([e.ph_dc_filter_sig_in[ch].c_fdk, e.ph_dc_filter_sig_in[ch].state_fdk]);
    }
    for ch in 0..2 {
        let o = &e.ph_onset[ch];
        v.extend([
            o.max_time_slots,
            o.min_transient_distance,
            o.avg_energy_distance,
            o.lower_bound_onset_detection,
            o.upper_bound_onset_detection,
            o.avg_energy_distance_scale as i32,
        ]);
        v.extend_from_slice(&o.p_energy_hist_fdk[..32]);
        v.extend(o.p_energy_hist_scale[..32].iter().map(|&x| x as i32));
    }
    v.extend(tree_ints(&e.h_space_tree));
    let b = &e.h_bitstream_formatter;
    v.extend(ssc_ints(&b.spatial_specific_config));
    v.extend(frame_bytes(&b.frame).iter().map(|&x| x as i32));
    v.extend(prev_bytes(&b.prev_frame_data.prev_ott_data).iter().map(|&x| x as i32));
    let g = &e.h_static_gain_config;
    v.extend([g.enc_mode, g.fixed_gain_dmx, g.pre_gain_factor_db]);
    v.extend(gain_ints(&e.h_static_gain));
    v.extend(delay_ints(&e.h_delay));
    v.extend(dmx_ints(&e.h_enhanced_time_dmx));
    v.extend_from_slice(&e.h_enhanced_time_dmx.sinus_window_m);
    for ch in 0..2 {
        v.extend(e.pp_time_sig_in_fdk[ch].iter().map(|&x| x as i32));
    }
    for ch in 0..2 {
        v.extend(e.pp_time_sig_delay_in_fdk[ch].iter().map(|&x| x as i32));
    }
    for buf in [&e.ppp_hybrid_in_fdk, &e.ppp_hybrid_in_static_fdk] {
        for ch in buf.iter() {
            for slot in ch {
                for x in slot {
                    v.push(x.re);
                    v.push(x.im);
                }
            }
        }
    }
    v.extend(e.p_parameter_band2_hybrid_band_offset.iter().map(|&x| x as i32));
    v.push(e.static_gain_scale);
    v.extend_from_slice(&e.p_encoder_input_ch_scale);
    v.extend_from_slice(&e.static_time_domain_dmx_in_scale);
    v.extend([1, 1, 1, 1]);
    v
}

/// A C and a Rust encoder driven in lockstep.
pub struct Pair {
    c: *mut c_void,
    r: HandleMp4SpaceEncoder,
    /// The reference must not be called any more (no defined reference result).
    c_dead: bool,
}

impl Pair {
    pub fn open() -> Self {
        let cp = unsafe { c::cref_sacenc_lib_open() };
        assert!(!cp.is_null());
        let mut rp = None;
        assert_eq!(r::fdk_sacenc_open(Some(&mut rp)), SACENC_OK);
        let p = Pair { c: cp, r: rp, c_dead: false };
        p.check("open");
        p
    }

    fn enc(&mut self) -> &mut Mp4SpaceEncoder {
        self.r.as_deref_mut().unwrap()
    }

    pub fn check(&self, what: &str) {
        if self.c_dead {
            return;
        }
        let mut v = vec![0i32; 40000];
        let n = unsafe { c::cref_sacenc_lib_state(self.c, v.as_mut_ptr()) } as usize;
        let rv = state_ints(self.r.as_deref().unwrap());
        if v[..n] != rv[..] {
            let i = (0..n.min(rv.len())).find(|&i| v[i] != rv[i]).unwrap_or(n.min(rv.len()));
            panic!(
                "{what}: state differs at {i} (C {} values, Rust {}): C {:?}, Rust {:?}",
                n,
                rv.len(),
                &v[i.saturating_sub(2)..(i + 6).min(n)],
                &rv[i.saturating_sub(2)..(i + 6).min(rv.len())]
            );
        }
    }

    pub fn set_param(&mut self, param: i32, value: u32) -> i32 {
        let er = r::fdk_sacenc_set_param(self.r.as_deref_mut(), param, value);
        if !self.c_dead {
            let ec = unsafe { c::cref_sacenc_lib_setParam(self.c, param, value) };
            assert_eq!(ec, er, "setParam {param} {value}");
        }
        er
    }

    pub fn init(&mut self, delay: i32, what: &str) -> i32 {
        let er = r::fdk_sacenc_init(self.r.as_deref_mut(), delay);
        if !self.c_dead {
            let ec = unsafe { c::cref_sacenc_lib_init(self.c, delay) };
            assert_eq!(ec, er, "{what}: init");
            self.check(&format!("{what}: init"));
            self.info(what);
            if er != SACENC_OK {
                // No defined reference result for the following calls.
                self.c_dead = true;
            }
        }
        er
    }

    pub fn info(&self, what: &str) {
        let mut v = [0i32; 73];
        let ec = unsafe { c::cref_sacenc_lib_getInfo(self.c, v.as_mut_ptr()) };
        let mut info = Mp4SpaceEncInfo::default();
        let er = r::fdk_sacenc_get_info(self.r.as_deref(), Some(&mut info));
        assert_eq!(ec, er);
        let ssc = info.p_ssc_buf.unwrap();
        let mut rv = vec![
            info.n_sample_rate,
            info.n_samples_frame,
            info.n_total_input_channels,
            info.n_dmx_delay,
            info.n_codec_delay,
            info.n_decoder_delay,
            info.n_payload_delay,
            info.n_discard_out_frames,
            ssc.n_ssc_size_bits,
        ];
        rv.extend(ssc.p_ssc.iter().map(|&b| b as i32));
        assert_eq!(&v[..], &rv[..], "{what}: getInfo");
        assert_eq!(unsafe { c::cref_sacenc_lib_getInfo_null(self.c) }, {
            let a = r::fdk_sacenc_get_info(None, Some(&mut Mp4SpaceEncInfo::default()));
            let b = r::fdk_sacenc_get_info(self.r.as_deref(), None);
            a * 4 + b
        });
    }

    /// One encode call with the buffers of the AAC encoder: `pcm` (two channels of `stride`
    /// samples, not interleaved unless `interleaved`), the downmix written over its start, the
    /// payload into a buffer of `bs_size` declared bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        pcm: &[i16],
        n_input_samples: i32,
        stride: u32,
        interleaved: u32,
        out_pcm_bytes: u32,
        bs_size: u32,
        bs_init: &[u8],
        bs_type: u32,
        num_out: i32,
        out_null: bool,
        what: &str,
    ) -> i32 {
        // Rust
        let mut pr = pcm.to_vec();
        let mut bsr = vec![0u8; 16384];
        bsr[..bs_init.len()].copy_from_slice(bs_init);
        let mut in_base = [pr.as_mut_ptr() as *mut c_void];
        let mut in_size = [0u32];
        let mut in_ele = [2u32];
        let mut in_type = [FDK_BUF_TYPE_INPUT | FDK_BUF_TYPE_PCM_DATA];
        let mut out_base = [
            if out_null { core::ptr::null_mut() } else { pr.as_mut_ptr() as *mut c_void },
            bsr.as_mut_ptr() as *mut c_void,
        ];
        let mut out_size = [out_pcm_bytes, bs_size];
        let mut out_ele = [2u32, 1];
        let mut out_type = [FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_PCM_DATA, bs_type];
        let in_desc = FdkBufDescr {
            pp_base: in_base.as_mut_ptr(),
            p_buf_size: in_size.as_mut_ptr(),
            p_ele_size: in_ele.as_mut_ptr(),
            p_buf_type: in_type.as_mut_ptr(),
            num_bufs: 1,
        };
        let out_desc = FdkBufDescr {
            pp_base: out_base.as_mut_ptr(),
            p_buf_size: out_size.as_mut_ptr(),
            p_ele_size: out_ele.as_mut_ptr(),
            p_buf_type: out_type.as_mut_ptr(),
            num_bufs: num_out as u32,
        };
        let inargs = SacencInArgs {
            n_input_samples,
            input_buffer_size_per_channel: stride,
            is_input_interleaved: interleaved,
        };
        let mut oar = SacencOutArgs { n_output_bits: -5, n_output_samples: -6, n_samples_consumed: 7 };
        let er = unsafe { r::fdk_sacenc_encode(self.r.as_deref_mut(), &in_desc, &out_desc, &inargs, &mut oar) };
        if self.c_dead {
            return er;
        }

        // C
        let mut pc = pcm.to_vec();
        let mut bsc = vec![0u8; 16384];
        bsc[..bs_init.len()].copy_from_slice(bs_init);
        let mut oac = [-5i32, -6, 7];
        let ec = unsafe {
            c::cref_sacenc_lib_encode(
                self.c,
                pc.as_mut_ptr(),
                n_input_samples,
                stride,
                interleaved,
                out_pcm_bytes,
                bsc.as_mut_ptr(),
                bs_size,
                bs_type,
                num_out,
                out_null as i32,
                oac.as_mut_ptr(),
            )
        };
        assert_eq!(ec, er, "{what}: encode");
        assert_eq!(oac, [oar.n_output_bits, oar.n_output_samples, oar.n_samples_consumed as i32], "{what}: outargs");
        assert!(pc == pr, "{what}: downmix / PCM buffer");
        let n = (bs_size as usize).min(16384);
        assert!(bsc[..n] == bsr[..n], "{what}: payload");
        self.check(what);
        er
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        unsafe { c::cref_sacenc_lib_close(self.c) };
        assert_eq!(r::fdk_sacenc_close(Some(&mut self.r)), SACENC_OK);
        assert!(self.r.is_none());
    }
}

/// The configuration of `FDK_MpegsEnc_Init()`.
pub fn glue_params(p: &mut Pair, fs: u32, slots: u32) -> bool {
    let mut ok = true;
    for (param, value) in [
        (SACENC_LOWDELAY, 2),
        (SACENC_ENC_MODE, SACENC_212 as u32),
        (SACENC_SAMPLERATE, fs),
        (SACENC_FRAME_TIME_SLOTS, slots),
        (SACENC_PARAM_BANDS, SACENC_BANDS_15 as u32),
        (SACENC_TIME_DOM_DMX, 2),
        (SACENC_COARSE_QUANT, 0),
        (SACENC_QUANT_MODE, SACENC_QUANTMODE_FINE as u32),
        (SACENC_TIME_ALIGNMENT, 0),
        (SACENC_INDEPENDENCY_FACTOR, 20),
    ] {
        ok &= p.set_param(param, value) == SACENC_OK;
    }
    ok
}

/// Stereo PCM of a frame: from the corpus when present, synthetic otherwise.
pub struct Source {
    files: Vec<std::path::PathBuf>,
    file: usize,
    pos: u64,
    synth: bool,
}

impl Source {
    pub fn new(rng: &mut Rng, synth: bool) -> Self {
        let files = corpus_files();
        let file = if files.is_empty() { 0 } else { rng.range(0, files.len() as i32 - 1) as usize };
        Source { files, file, pos: rng.next_u32() as u64, synth }
    }

    /// `n` samples per channel, interleaved.
    pub fn next(&mut self, rng: &mut Rng, n: usize) -> Vec<i16> {
        let v = if self.synth || self.files.is_empty() {
            None
        } else {
            corpus_stereo(&self.files[self.file], self.pos, n)
        };
        self.pos += n as u64;
        v.unwrap_or_else(|| synth_stereo(rng, n))
    }
}

/// The PCM buffer of the AAC encoder: two channels of `stride` samples.
fn deinterleave(st: &[i16], n: usize, stride: usize) -> Vec<i16> {
    let mut b = vec![0x3333i16; 2 * stride];
    for i in 0..n {
        b[i] = st[2 * i];
        b[stride + i] = st[2 * i + 1];
    }
    b
}

/// Runs `frames` frames of the glue's encode calls.
fn run_frames(p: &mut Pair, rng: &mut Rng, src: &mut Source, fl: usize, frames: usize, what: &str) {
    for fr in 0..frames {
        let st = src.next(rng, fl);
        let stride = fl + rng.range(0, 40) as usize;
        let pcm = deinterleave(&st, fl, stride);
        let init: Vec<u8> = (0..1023).map(|_| rng.next_u32() as u8).collect();
        let e = p.encode(
            &pcm,
            2 * fl as i32,
            stride as u32,
            0,
            (2 * fl) as u32,
            1023,
            &init,
            FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_BS_DATA,
            2,
            false,
            &format!("{what} frame {fr}"),
        );
        assert_eq!(e, SACENC_OK, "{what} frame {fr}");
    }
}

/// The MPS frame lengths that the AAC encoder can configure at a sampling rate (core frame
/// lengths 512/480, also 256/240, with dual rate SBR doubled at 64 bands).
fn frame_lengths(fs: u32) -> Vec<usize> {
    if fs < 27713 {
        vec![512, 480, 256, 240]
    } else {
        vec![512, 1024, 960, 480, 256]
    }
}

#[test]
fn glue_configurations() {
    let mut rng = Rng::new(1);
    for fs in [16000u32, 22050, 24000, 32000, 44100, 48000] {
        let bands = if fs < 27713 { 32 } else { 64 };
        for fl in frame_lengths(fs) {
            for d in [0, 1, 3, 5, 6, -1] {
                let what = format!("fs {fs} fl {fl} delay +{d}");
                let mut p = Pair::open();
                let slots = (fl / bands) as u32;
                // FDK_MpegsEnc_Init refuses frame lengths that are no multiple of the bands.
                if slots as usize * bands != fl {
                    continue;
                }
                assert!(glue_params(&mut p, fs, slots));
                let dmx = rng.range(-1, 7);
                if dmx >= 0 {
                    assert_eq!(p.set_param(SACENC_DMX_GAIN, dmx as u32), SACENC_OK);
                }
                let e = p.init(fl as i32 / 2 + d, &what);
                if (0..=5).contains(&d) {
                    assert_eq!(e, SACENC_OK, "{what}");
                    let synth = rng.range(0, 4) == 0;
                    let mut src = Source::new(&mut rng, synth);
                    run_frames(&mut p, &mut rng, &mut src, fl, 60, &format!("{what} dmx {dmx}"));
                } else {
                    assert_ne!(e, SACENC_OK, "{what}");
                }
            }
        }
    }
}

#[test]
fn dmx_gains_and_parameters() {
    let mut rng = Rng::new(2);
    for it in 0..120 {
        let fs = [16000u32, 22050, 24000, 32000, 44100, 48000, rng.range(8000, 48000) as u32][rng.range(0, 6) as usize];
        let bands = if fs < 27713 { 32 } else { 64 };
        let slots = rng.range(1, 16) as u32;
        let fl = bands * slots as usize;
        let mut p = Pair::open();
        assert!(glue_params(&mut p, fs, slots));
        p.set_param(SACENC_DMX_GAIN, (it % 8) as u32);
        // Further parameters (some invalid).
        for _ in 0..rng.range(0, 4) {
            let (param, value) = match rng.range(0, 8) {
                0 => (SACENC_LOWDELAY, rng.range(0, 3) as u32),
                1 => (SACENC_PARAM_BANDS, [4, 5, 7, 9, 12, 15, 23, 6][rng.range(0, 7) as usize]),
                2 => (SACENC_COARSE_QUANT, rng.range(0, 2) as u32),
                3 => (SACENC_QUANT_MODE, rng.range(0, 3) as u32),
                4 => (SACENC_INDEPENDENCY_FACTOR, rng.range(0, 30) as u32),
                5 => (SACENC_INDEPENDENCY_COUNT, rng.range(0, 30) as u32),
                6 => (SACENC_DMX_GAIN, rng.range(0, 8) as u32),
                _ => (SACENC_TIME_ALIGNMENT, if rng.range(0, 1) == 0 { 0 } else { rng.range(-3, 3) as u32 }),
            };
            p.set_param(param, value);
        }
        let delay = fl as i32 / 2 + rng.range(0, 5);
        let what = format!("it {it} fs {fs} slots {slots}");
        if p.init(delay, &what) != SACENC_OK {
            continue;
        }
        let interleaved = rng.range(0, 3) == 0;
        let mut src = Source::new(&mut rng, false);
        for fr in 0..20 {
            let st = src.next(&mut rng, fl);
            let what = format!("{what} frame {fr}");
            let e = if interleaved {
                p.encode(&st, 2 * fl as i32, 0, 1, (2 * fl) as u32, 1023, &[], FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_BS_DATA, 2, false, &what)
            } else {
                let stride = fl + 3;
                let pcm = deinterleave(&st, fl, stride);
                p.encode(&pcm, 2 * fl as i32, stride as u32, 0, (2 * fl) as u32, 1023, &[], FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_BS_DATA, 2, false, &what)
            };
            assert_eq!(e, SACENC_OK, "{what}");
        }
    }
}

#[test]
fn reinitialization() {
    // A handle re-initialized with other configurations (stale buffers of the earlier ones).
    let mut rng = Rng::new(3);
    for it in 0..30 {
        let mut p = Pair::open();
        for k in 0..4 {
            let fs = [16000u32, 22050, 24000, 32000, 44100, 48000][rng.range(0, 5) as usize];
            let bands = if fs < 27713 { 32 } else { 64 };
            let slots = [8u32, 15, 16, rng.range(1, 16) as u32][rng.range(0, 3) as usize];
            let fl = bands * slots as usize;
            assert!(glue_params(&mut p, fs, slots));
            p.set_param(SACENC_PARAM_BANDS, [15, 23, 4, 9][rng.range(0, 3) as usize]);
            p.set_param(SACENC_LOWDELAY, rng.range(0, 2) as u32);
            let what = format!("it {it} k {k} fs {fs} slots {slots}");
            let e = p.init(fl as i32 / 2 + rng.range(0, 5), &what);
            if e != SACENC_OK {
                // The onset detection refuses more time slots than in its first
                // initialization (see sacenc_onsetdetect); the reference is not used further.
                assert!(k > 0, "{what}");
                break;
            }
            let mut src = Source::new(&mut rng, false);
            run_frames(&mut p, &mut rng, &mut src, fl, 12, &what);
        }
    }
}

#[test]
fn feeding_and_buffers() {
    // Partial and wrong input sizes, small output buffers, missing descriptor entries.
    let mut rng = Rng::new(4);
    for it in 0..60 {
        let fs = [24000u32, 48000][rng.range(0, 1) as usize];
        let bands = if fs < 27713 { 32 } else { 64 };
        let slots = 8u32 + rng.range(0, 1) as u32 * 8;
        let fl = bands * slots as usize;
        let mut p = Pair::open();
        assert!(glue_params(&mut p, fs, slots));
        assert_eq!(p.init(fl as i32 / 2, &format!("it {it}")), SACENC_OK);
        let mut src = Source::new(&mut rng, false);
        for fr in 0..30 {
            let st = src.next(&mut rng, fl);
            let stride = fl + 5;
            let pcm = deinterleave(&st, fl, stride);
            let what = format!("it {it} frame {fr}");
            let n_in = match rng.range(0, 6) {
                0 => rng.range(0, 2 * fl as i32 / 2) * 2,
                1 => rng.range(0, 2 * fl as i32),
                _ => 2 * fl as i32,
            };
            let out_bytes = if rng.range(0, 8) == 0 { rng.range(0, 2 * fl as i32) as u32 } else { (2 * fl) as u32 };
            let bs_size = if rng.range(0, 8) == 0 { rng.range(0, 40) as u32 } else { 1023 };
            let bs_type = if rng.range(0, 12) == 0 { FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_ANC_DATA } else { FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_BS_DATA };
            let out_null = rng.range(0, 20) == 0;
            p.encode(&pcm, n_in, stride as u32, 0, out_bytes, bs_size, &[], bs_type, 2, out_null, &what);
        }
    }
}

#[test]
fn api_misc() {
    let mut rng = Rng::new(5);
    // NULL handles
    assert_eq!(unsafe { c::cref_sacenc_lib_open_null() }, r::fdk_sacenc_open(None));
    assert_eq!(unsafe { c::cref_sacenc_lib_close_null() }, r::fdk_sacenc_close(None) * 2 + r::fdk_sacenc_close(Some(&mut None)));
    assert_eq!(r::fdk_sacenc_init(None, 0), SACENC_INVALID_HANDLE);
    assert_eq!(r::fdk_sacenc_set_param(None, 0, 0), SACENC_INVALID_HANDLE);

    // setParam on every parameter with edge values
    let mut p = Pair::open();
    for param in -1..14 {
        for value in [0u32, 1, 2, 3, 4, 5, 7, 8, 9, 12, 15, 16, 17, 23, 24, 40, 47999, 48000, 48001, 32767, 32768, 0xFFFF_8000, 0xFFFF_7FFF, u32::MAX] {
            p.set_param(param, value);
        }
    }
    p.check("setParam");
    drop(p);

    // getLibInfo
    for used in [0, 1, 5, FDK_MODULE_LAST - 1, FDK_MODULE_LAST] {
        let mut v = vec![0i32; FDK_MODULE_LAST as usize * 99];
        let ec = unsafe { c::cref_sacenc_lib_getLibInfo(used, v.as_mut_ptr()) };
        let mut info = vec![LibInfo::default(); FDK_MODULE_LAST as usize];
        for (i, e) in info.iter_mut().enumerate() {
            e.module_id = if (i as i32) < used { FDK_AACDEC } else { FDK_NONE };
        }
        let er = r::fdk_sacenc_get_lib_info(Some(&mut info));
        assert_eq!(ec, er, "getLibInfo {used}");
        let mut rv = Vec::new();
        for e in &info {
            rv.extend([e.module_id, e.version, e.flags as i32]);
            rv.extend(e.version_str.iter().map(|&b| b as i32));
            let t = e.title.as_bytes();
            rv.extend((0..64).map(|j| t.get(j).copied().unwrap_or(0) as i32));
        }
        assert_eq!(v, rv, "getLibInfo {used}");
    }
    assert_eq!(unsafe { c::cref_sacenc_lib_getLibInfo_null() }, r::fdk_sacenc_get_lib_info(None));

    // Static helpers
    for fs in [0u32, 1, 16000, 27712, 27713, 48000, 55425, 55426, 96000] {
        assert_eq!(unsafe { c::cref_sacenc_lib_GetNumQmfBands(fs) }, r::mp4_space_enc_get_num_qmf_bands(fs));
    }
    for _ in 0..2000 {
        let flags = rng.next_u32();
        let keep = rng.range(0, 1);
        assert_eq!(unsafe { c::cref_sacenc_lib_updateQmfFlags(flags, keep) }, r::update_qmf_flags(flags, keep));
        let f = rng.range(0, 30000) as u32;
        let fs = rng.range(1, 48000) as u32;
        let b = [32u32, 64][rng.range(0, 1) as usize];
        assert_eq!(unsafe { c::cref_sacenc_lib_freq2HybridBand(f, fs, b) }, r::freq2_hybrid_band(f, fs, b), "freq2HybridBand {f} {fs} {b}");
        let mut w: Vec<i32> = (0..48).map(|_| if rng.range(0, 2) == 0 { 0 } else { rng.i32() }).collect();
        let n = rng.range(0, 48);
        assert_eq!(unsafe { c::cref_sacenc_lib_getAnalysisLengthTimeSlots(w.as_mut_ptr(), n) }, r::get_analysis_length_time_slots(&w, n));
        assert_eq!(unsafe { c::cref_sacenc_lib_getAnalysisStartTimeSlot(w.as_mut_ptr(), n) }, r::get_analysis_start_time_slot(&w, n));
        let m = rng.range(-3, 5);
        assert_eq!(unsafe { c::cref_sacenc_lib_mapQuantMode(m) }, r::map_quant_mode(m));
        let mut types: Vec<u32> = (0..4).map(|_| [FDK_BUF_TYPE_INPUT | FDK_BUF_TYPE_PCM_DATA, FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_BS_DATA, 0][rng.range(0, 2) as usize]).collect();
        let nb = rng.range(0, 4) as u32;
        let t = types[rng.range(0, 3) as usize];
        let desc = FdkBufDescr { p_buf_type: types.as_mut_ptr(), num_bufs: nb, ..Default::default() };
        assert_eq!(unsafe { c::cref_sacenc_lib_getBufDescIdx(types.as_mut_ptr(), nb, t) }, unsafe { r::get_buf_desc_idx(&desc, t) });
    }
}

/// Random parameters (valid and invalid), then encode calls with random input sizes and
/// buffers (compared with the reference after a successful initialization).
fn random_session(rng: &mut Rng, p: &mut Pair, it: usize) {
    let fs = rng.range(0, 48000) as u32;
    let slots = rng.range(0, 16) as u32;
    if rng.range(0, 1) == 0 {
        glue_params(p, fs, slots);
    }
    for _ in 0..rng.range(0, 12) {
        let param = rng.range(-1, 12);
        let value = match rng.range(0, 4) {
            0 => rng.range(0, 3) as u32,
            1 => rng.range(0, 48000) as u32,
            2 => rng.next_u32(),
            _ => [SACENC_212 as u32, 15, 2, fs, slots][rng.range(0, 4) as usize],
        };
        p.set_param(param, value);
    }
    let fl = p.enc().user.frame_time_slots as i32 * if p.enc().user.sample_rate < 27713 { 32 } else { 64 };
    let delay = match rng.range(0, 3) {
        0 => rng.i32(),
        1 => rng.range(-2000, 2000),
        _ => fl / 2 + rng.range(-2, 7),
    };
    // Configurations without a defined reference result: the reference is not called.
    if p.enc().user.n_param_bands == 0 {
        p.c_dead = true;
    }
    // A sampling rate of 0: no defined x86 reference result (x86 division by zero).
    if cfg!(any(target_arch = "x86_64", target_arch = "x86")) && p.enc().user.sample_rate == 0 {
        p.c_dead = true;
    }
    if p.c_dead {
        r::fdk_sacenc_init(p.r.as_deref_mut(), delay);
    } else {
        p.init(delay, &format!("random it {it}"));
    }
    for fr in 0..8 {
        let n = rng.range(1, 1100) as usize;
        let st = synth_stereo(rng, n);
        let stride = 1100usize;
        let pcm = deinterleave(&st, n, stride);
        let n_in = match rng.range(0, 3) {
            0 => rng.i32() >> rng.range(0, 31),
            1 => rng.range(-2, 2200),
            _ => 2 * n as i32,
        };
        let _ = p.encode(
            &pcm,
            n_in.min(2 * stride as i32),
            stride as u32,
            rng.range(0, 1) as u32,
            rng.range(0, 4400) as u32,
            rng.range(0, 1023) as u32,
            &[],
            FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_BS_DATA,
            2,
            rng.range(0, 10) == 0,
            &format!("random it {it} frame {fr}"),
        );
    }
    let mut info = Mp4SpaceEncInfo::default();
    r::fdk_sacenc_get_info(p.r.as_deref(), Some(&mut info));
}

#[test]
fn random_parameters() {
    let mut rng = Rng::new(std::env::var("SACENC_SEED").map_or(6, |v| v.parse().unwrap()));
    for it in 0..std::env::var("SACENC_ITERS").map_or(400, |v| v.parse().unwrap()) {
        let mut p = Pair::open();
        random_session(&mut rng, &mut p, it);
        // Another session on the same handle.
        random_session(&mut rng, &mut p, it);
    }
}

#[test]
fn port_only_robustness() {
    // Encoding before the first initialization and after failed ones: the port only, which
    // must not panic.
    let mut rng = Rng::new(std::env::var("SACENC_SEED").map_or(7, |v| v.parse().unwrap()));
    for it in 0..std::env::var("SACENC_ITERS").map_or(300, |v| v.parse().unwrap()) {
        let mut p = Pair::open();
        p.c_dead = true;
        random_session(&mut rng, &mut p, it);
        random_session(&mut rng, &mut p, it);
    }
}

/// The table `id` of the reference as INTs.
fn c_table(f: unsafe extern "C" fn(i32, *mut i32) -> i32, id: i32) -> Vec<i32> {
    let mut v = vec![0i32; 1024];
    let n = unsafe { f(id, v.as_mut_ptr()) } as usize;
    v.truncate(n);
    v
}

#[test]
fn tables() {
    let f = c::cref_sacenc_lib_table;
    let i = |t: &[u8]| t.iter().map(|&x| x as i32).collect::<Vec<_>>();
    assert_eq!(c_table(f, 0), i(&P_VALID_BANDS_LD), "pValidBands_Ld");
    assert_eq!(c_table(f, 1), i(&QMF2QMF), "qmf2qmf");
}
