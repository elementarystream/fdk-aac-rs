// Encoder harness for realistic, stateful differential tests of the encoder stages (reusable by
// the tests of qc_main, psy_main and the other encoder modules).
//
// The C side is shim/aacenc_adj_thr_enc.cpp (see its header for the details): a reference
// encoder opened through the library API with exactly the configuration of aacenc_lib.cpp, and a
// frame stepper that replicates FDKaacEnc_EncodeFrame and FDKaacEnc_QCMain stage by stage.
//
// API (module `crate::adj_thr_enc`)
// ---------------------------------
// * [`EncConfig`]: AOT, sample rate, channel mode, bit rate, bit rate mode (0 CBR, 1..5 VBR),
//   afterburner, frame length (0: default; ELD 256/240/128/120 select the downscaled modes),
//   transport. [`configs`] lists the test configurations (LC 1024, LD 512/480, ELD 512..120,
//   8..96 kHz, mono .. 7.1, CBR at low and high bit rates, VBR 1..5, afterburner on/off);
//   [`config_subset`] picks a deterministic random subset of the configurations that pass a
//   filter (`ADJ_THR_ENC_CONFIGS=all` selects all, `=N` N configurations).
// * [`CEnc::open`] opens a reference encoder (`Err(AACENC_ERROR)` if the library rejects the
//   configuration). [`CEnc::n_channels`], [`CEnc::n_elements`], [`CEnc::frame_length`] (input
//   samples per channel and frame), [`CEnc::channel_mapping`], [`CEnc::config`].
// * Frame stepping: [`CEnc::begin_frame`] (planar PCM, `pcm[ch * frame_length + i]`),
//   [`CEnc::next`] (the next [`Stage`] and its element), [`CEnc::run`] (runs it; an error ends
//   the frame), [`CEnc::run_to`] (runs until the next stage is the given one),
//   [`CEnc::finish`] (runs the rest of the frame and returns the access unit). [`CEnc::frame`]
//   encodes a whole frame through the stages, [`CEnc::lib_frame`] through the library's
//   aacEncEncode (interleaved PCM; use a second instance: both must give the same bytes).
//   [`CEnc::qc_vars`]/[`CEnc::set_qc_vars`]: the locals avgTotalBits, totalAvailableBits,
//   avgTotalDynBits, isCBRAdjustment of the replicated FDKaacEnc_QCMain.
// * Snapshots of the C state as the port's types (between stages): [`CEnc::qc_out`],
//   [`CEnc::psy_out`], [`CEnc::adj_thr`], [`CEnc::qc_state`] (with element bits, bit counter
//   buffers and the ADJ_THR_STATE), [`CEnc::psy_internal`] (PSY_CONFIGURATION[2], PSY_ELEMENTs,
//   PSY_STATICs with input buffer, overlap buffer, mdct state, block switching and pre-echo
//   state, PSY_DYNAMIC), [`CEnc::aac_enc`] (AAC_ENC with all of the above). Overwriting:
//   [`CEnc::set_qc_out`], [`CEnc::set_psy_out`], [`CEnc::set_adj_thr`], [`CEnc::set_qc_state`],
//   [`CEnc::set_psy_internal`]. The shared dynamic RAM: [`CEnc::p_buf_1`] / [`CEnc::set_p_buf_1`]
//   (all of P_BUF_1) and [`CEnc::adj_thr_dyn_mem`] / [`CEnc::set_adj_thr_dyn_mem`] (its
//   adjust-thresholds part as the port's AdjThrDynMem).
// * PCM: [`Signal`] and [`pcm_frames`]: corpus music (`work/corpus`), the generated corpus
//   (`work/corpus-gen`, when it has the rate and channel count), synthetic signals (sweep,
//   noise, clicks, silence, full-scale square, multi-tone); never committed, the corpus is
//   optional (synthetic fallback).
//
// Not exchanged: the pointers of QC_OUT_EXTENSION (no payloads are used), mdct_t's overlap,
// pFacZir and pAsymOvlp (the encoder's mdct_t has no overlap memory; flags are available
// through INTS_MDCT).

#![allow(dead_code)]

use fdk_aac_rs_aacenc::aac_enc_ram::AacEnc;
use fdk_aac_rs_aacenc::aacenc::AacencConfig;
use fdk_aac_rs_aacenc::aacenc_pns::PnsData;
use fdk_aac_rs_aacenc::aacenc_tns::TnsData;
use fdk_aac_rs_aacenc::adj_thr::AdjThrDynMem;
use fdk_aac_rs_aacenc::adj_thr_data::{AdjThrState, AtsElement, BresParam};
use fdk_aac_rs_aacenc::block_switch::BlockSwitchingControl;
use fdk_aac_rs_aacenc::dyn_bits::BitcntrState;
use fdk_aac_rs_aacenc::interface::{PsyOut, PsyOutChannel, PsyOutElement, ToolsInfo};
use fdk_aac_rs_aacenc::line_pe::PeData;
use fdk_aac_rs_aacenc::psy_configuration::PsyConfiguration;
use fdk_aac_rs_aacenc::psy_const::{MAX_INPUT_BUFFER_SIZE, MAX_SFB, MAX_SFB_LONG};
use fdk_aac_rs_aacenc::psy_data::{PsyData, PsyStatic, OVERLAP_ADD_BUFFER_SIZE};
use fdk_aac_rs_aacenc::psy_main::{PsyDynamic, PsyElement, PsyInternal};
use fdk_aac_rs_aacenc::qc_data::*;
use fdk_aac_rs_cref::aacenc_adj_thr_enc as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::FixpDbl;
use fdk_aac_rs_fdk::mdct::MdctT;
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;
use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// The stages of a frame (shim/aacenc_adj_thr_enc.cpp).
pub type Stage = i32;
pub const STAGE_BEGIN: Stage = 0;
pub const STAGE_PSY: Stage = 1;
pub const STAGE_FORM_FACTOR: Stage = 2;
pub const STAGE_PE: Stage = 3;
pub const STAGE_EL_BITS: Stage = 4;
pub const STAGE_GLOBAL: Stage = 5;
pub const STAGE_BITRES_REDIST: Stage = 6;
pub const STAGE_PREPARE_BITS: Stage = 7;
pub const STAGE_DISTRIBUTE: Stage = 8;
pub const STAGE_PREPARE_END: Stage = 9;
pub const STAGE_ADJUST_THR: Stage = 10;
pub const STAGE_QUANT: Stage = 11;
pub const STAGE_FINALIZE: Stage = 12;
pub const STAGE_DONE: Stage = 13;

const PTR_AAC_ENC: i32 = 0;
const PTR_CONFIG: i32 = 1;
const PTR_CHANNEL_MAPPING: i32 = 2;
const PTR_QC_STATE: i32 = 3;
const PTR_ADJ_THR: i32 = 4;
const PTR_ATS_ELEMENT: i32 = 5;
const PTR_ELEMENT_BITS: i32 = 6;
const PTR_QC_OUT: i32 = 7;
const PTR_QC_ELEMENT: i32 = 8;
const PTR_QC_CHANNEL: i32 = 9;
const PTR_PSY_OUT: i32 = 10;
const PTR_PSY_ELEMENT_OUT: i32 = 11;
const PTR_PSY_CHANNEL: i32 = 12;
const PTR_PSY_INTERNAL: i32 = 13;
const PTR_PSY_ELEMENT: i32 = 14;
const PTR_PSY_STATIC: i32 = 15;
const PTR_PSY_DYNAMIC: i32 = 16;
const PTR_BIT_COUNTER: i32 = 17;
const PTR_TP_ENC: i32 = 18;
const PTR_INPUT: i32 = 19;
const PTR_DYN_RAM: i32 = 20;
const PTR_PSY_CONF: i32 = 21;

const INTS_QC_OUT: i32 = 0;
const INTS_QC_ELEMENT: i32 = 1;
const INTS_QC_STATE: i32 = 2;
const INTS_ADJ_THR: i32 = 3;
const INTS_PSY_OUT_ELEMENT: i32 = 4;
const INTS_PSY_ELEMENT: i32 = 5;
const INTS_AAC_ENC: i32 = 6;
const INTS_MDCT: i32 = 7;

/// An encoder configuration (parameters of aacEncoder_SetParam).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EncConfig {
    pub aot: AudioObjectType,
    pub sample_rate: i32,
    pub channel_mode: ChannelMode,
    /// total bit rate (CBR only)
    pub bitrate: i32,
    /// 0 CBR, 1..5 VBR
    pub bitrate_mode: i32,
    pub afterburner: i32,
    /// AACENC_GRANULE_LENGTH, 0 for the default
    pub frame_length: i32,
    pub transport: TransportType,
}

/// Number of channels of a channel mode.
pub fn mode_channels(mode: ChannelMode) -> i32 {
    match mode {
        MODE_1 => 1,
        MODE_2 => 2,
        MODE_1_2 => 3,
        MODE_1_2_1 => 4,
        MODE_1_2_2 => 5,
        MODE_1_2_2_1 => 6,
        MODE_6_1 => 7,
        _ => 8,
    }
}

/// The test configurations.
pub fn configs() -> Vec<EncConfig> {
    let mut v = Vec::new();
    let all_modes = [
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
    // (aot, frame length, transport)
    let codecs = [
        (AOT_AAC_LC, 0, TT_MP4_ADTS),
        (AOT_AAC_LC, 0, TT_MP4_RAW),
        (AOT_ER_AAC_LD, 512, TT_MP4_LOAS),
        (AOT_ER_AAC_LD, 480, TT_MP4_RAW),
        (AOT_ER_AAC_ELD, 512, TT_MP4_LOAS),
        (AOT_ER_AAC_ELD, 480, TT_MP4_RAW),
        (AOT_ER_AAC_ELD, 256, TT_MP4_LOAS),
        (AOT_ER_AAC_ELD, 240, TT_MP4_RAW),
        (AOT_ER_AAC_ELD, 128, TT_MP4_LOAS),
        (AOT_ER_AAC_ELD, 120, TT_MP4_RAW),
    ];
    for (k, &(aot, fl, tt)) in codecs.iter().enumerate() {
        for (r, &sr) in rates.iter().enumerate() {
            for (m, &mode) in all_modes.iter().enumerate() {
                let nch = mode_channels(mode);
                let lfe = matches!(mode, MODE_1_2_2_1 | MODE_6_1 | MODE_1_2_2_2_1 | 12 | 14 | 33 | 34);
                let neff = nch - lfe as i32;
                // low and high CBR bit rates per channel (scaled with the sample rate)
                let low = 8000.max(sr / 4) * neff;
                let high = (sr * 3).min(160_000) * neff;
                for (b, &br) in [low, high].iter().enumerate() {
                    v.push(EncConfig {
                        aot,
                        sample_rate: sr,
                        channel_mode: mode,
                        bitrate: br,
                        bitrate_mode: 0,
                        afterburner: ((k + r + m + b) % 2) as i32,
                        frame_length: fl,
                        transport: tt,
                    });
                }
                for vbr in 1..=5 {
                    v.push(EncConfig {
                        aot,
                        sample_rate: sr,
                        channel_mode: mode,
                        bitrate: 0,
                        bitrate_mode: vbr,
                        afterburner: ((k + r + m + vbr as usize) % 2) as i32,
                        frame_length: fl,
                        transport: tt,
                    });
                }
            }
        }
    }
    v
}

/// A deterministic random subset of the configurations of [`configs`] that pass `filter`
/// (`ADJ_THR_ENC_CONFIGS=all` selects all of them, `=N` N of them; default `n`).
pub fn config_subset(seed: u64, n: usize, filter: impl Fn(&EncConfig) -> bool) -> Vec<EncConfig> {
    let all: Vec<EncConfig> = configs().into_iter().filter(|c| filter(c)).collect();
    let n = match std::env::var("ADJ_THR_ENC_CONFIGS").as_deref() {
        Ok("all") => return all,
        Ok(s) => s.parse().unwrap_or(n),
        Err(_) => n,
    };
    let mut rng = fdk_aac_rs_cref::testutil::Rng::new(seed);
    let mut v: Vec<EncConfig> = Vec::new();
    while v.len() < n.min(all.len()) {
        let c = all[(rng.next_u64() % all.len() as u64) as usize];
        if !v.contains(&c) {
            v.push(c);
        }
    }
    v
}

fn ptr_bytes<T: Copy>(p: *const c_void) -> T {
    assert!(!p.is_null());
    // SAFETY: p points to a C structure with the layout of T (checked by the layout tests).
    unsafe { std::ptr::read_unaligned(p as *const T) }
}

fn write_bytes<T: Copy>(p: *mut c_void, v: &T) {
    assert!(!p.is_null());
    // SAFETY: as in ptr_bytes.
    unsafe { std::ptr::write_unaligned(p as *mut T, *v) }
}

fn offsets() -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for i in 0.. {
        let mut v = 0;
        let p = unsafe { c::cref_adj_thr_enc_offset(i, &mut v) };
        if p.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
        m.insert(name, v as usize);
    }
    m
}

/// A reference encoder with the frame stepper.
pub struct CEnc {
    h: *mut c_void,
    pub cfg: EncConfig,
    info: [i32; 8],
    off: HashMap<String, usize>,
}

impl Drop for CEnc {
    fn drop(&mut self) {
        unsafe { c::cref_adj_thr_enc_close(self.h) };
    }
}

impl CEnc {
    /// Opens a reference encoder; `Err(AACENC_ERROR)` if the configuration is rejected.
    pub fn open(cfg: &EncConfig) -> Result<CEnc, i32> {
        let mut err = 0;
        let h = unsafe {
            c::cref_adj_thr_enc_open(
                cfg.aot,
                cfg.sample_rate,
                cfg.channel_mode,
                cfg.bitrate,
                cfg.bitrate_mode,
                cfg.afterburner,
                cfg.frame_length,
                cfg.transport,
                &mut err,
            )
        };
        if h.is_null() {
            return Err(err);
        }
        let mut info = [0i32; 8];
        unsafe { c::cref_adj_thr_enc_info(h, info.as_mut_ptr()) };
        Ok(CEnc { h, cfg: *cfg, info, off: offsets() })
    }

    pub fn n_channels(&self) -> usize {
        self.info[0] as usize
    }
    pub fn n_elements(&self) -> usize {
        self.info[1] as usize
    }
    /// Input samples per channel and frame.
    pub fn frame_length(&self) -> usize {
        self.info[2] as usize
    }
    pub fn aot(&self) -> AudioObjectType {
        self.info[4]
    }
    pub fn syntax_flags(&self) -> u32 {
        self.info[5] as u32
    }
    pub fn ep_config(&self) -> i8 {
        self.info[6] as i8
    }

    fn ptr(&self, what: i32, idx: usize) -> *mut c_void {
        unsafe { c::cref_adj_thr_enc_ptr(self.h, what, idx as i32) }
    }
    fn off(&self, name: &str) -> usize {
        self.off[name]
    }
    fn ints(&self, what: i32, idx: usize) -> Vec<i32> {
        let mut v = [0i32; 64];
        let n = unsafe { c::cref_adj_thr_enc_get_ints(self.h, what, idx as i32, v.as_mut_ptr()) };
        v[..n as usize].to_vec()
    }
    fn set_ints(&self, what: i32, idx: usize, v: &[i32]) {
        unsafe { c::cref_adj_thr_enc_set_ints(self.h, what, idx as i32, v.as_ptr()) };
    }

    /// Starts a frame with planar PCM (`pcm[ch * frame_length + i]`).
    pub fn begin_frame(&mut self, pcm: &[i16]) {
        assert_eq!(pcm.len(), self.n_channels() * self.frame_length());
        unsafe { c::cref_adj_thr_enc_begin_frame(self.h, pcm.as_ptr()) };
    }
    /// The next stage and its element.
    pub fn next(&self) -> (Stage, usize) {
        let mut el = 0;
        let s = unsafe { c::cref_adj_thr_enc_next(self.h, &mut el) };
        (s, el as usize)
    }
    /// Runs the next stage; returns its AAC_ENCODER_ERROR (an error ends the frame).
    pub fn run(&mut self) -> i32 {
        unsafe { c::cref_adj_thr_enc_run(self.h) }
    }
    /// Runs stages until the next one is `(stage, el)` (or the frame is done).
    pub fn run_to(&mut self, stage: Stage, el: usize) -> i32 {
        loop {
            let n = self.next();
            if n.0 >= STAGE_DONE || n == (stage, el) {
                return 0;
            }
            let e = self.run();
            if e != 0 {
                return e;
            }
        }
    }
    /// Runs the rest of the frame; returns the access unit.
    pub fn finish(&mut self) -> Result<Vec<u8>, i32> {
        while self.next().0 < STAGE_DONE {
            let e = self.run();
            if e != 0 {
                return Err(e);
            }
        }
        let mut out = vec![0u8; 8 * 6144];
        let n = unsafe { c::cref_adj_thr_enc_output(self.h, out.as_mut_ptr(), out.len() as i32) };
        out.truncate(n as usize);
        Ok(out)
    }
    /// A whole frame through the stages.
    pub fn frame(&mut self, pcm: &[i16]) -> Result<Vec<u8>, i32> {
        self.begin_frame(pcm);
        self.finish()
    }
    /// A whole frame through the library's aacEncEncode (planar PCM, interleaved here).
    pub fn lib_frame(&mut self, pcm: &[i16]) -> Result<Vec<u8>, i32> {
        let (nch, len) = (self.n_channels(), self.frame_length());
        let mut il = vec![0i16; nch * len];
        for ch in 0..nch {
            for i in 0..len {
                il[i * nch + ch] = pcm[ch * len + i];
            }
        }
        let mut out = vec![0u8; 8 * 6144];
        let mut n = 0;
        let e = unsafe {
            c::cref_adj_thr_enc_lib_frame(self.h, il.as_ptr(), out.as_mut_ptr(), out.len() as i32, &mut n)
        };
        if e != 0 {
            return Err(e);
        }
        out.truncate(n as usize);
        Ok(out)
    }
    /// Samples per channel of the encoder's input buffer (`inputBufferSizePerChannel`).
    pub fn input_buffer_size_per_channel(&self) -> usize {
        self.info[3] as usize
    }
    /// The encoder's planar input buffer (the input of FDKaacEnc_psyMain):
    /// `n_channels * input_buffer_size_per_channel` samples.
    pub fn input_buffer(&self) -> Vec<i16> {
        let n = self.n_channels() * self.input_buffer_size_per_channel();
        let p = self.ptr(PTR_INPUT, 0) as *const i16;
        assert!(!p.is_null());
        // SAFETY: the input buffer of the reference encoder holds n samples.
        unsafe { std::slice::from_raw_parts(p, n) }.to_vec()
    }
    /// avgTotalBits, totalAvailableBits, avgTotalDynBits, isCBRAdjustment.
    pub fn qc_vars(&self) -> [i32; 4] {
        let mut v = [0; 4];
        unsafe { c::cref_adj_thr_enc_qc_vars(self.h, v.as_mut_ptr()) };
        v
    }
    /// Fills 64 KiB of stack with the byte `v` before every stage (`None`: off).
    pub fn set_stack_poison(&mut self, v: Option<u8>) {
        unsafe { c::cref_adj_thr_enc_set_stack_poison(self.h, v.map_or(-1, |x| x as i32)) };
    }
    pub fn set_qc_vars(&mut self, v: [i32; 4]) {
        unsafe { c::cref_adj_thr_enc_set_qc_vars(self.h, v.as_ptr()) };
    }

    pub fn channel_mapping(&self) -> ChannelMapping {
        ptr_bytes(self.ptr(PTR_CHANNEL_MAPPING, 0))
    }
    pub fn config(&self) -> AacencConfig {
        ptr_bytes(self.ptr(PTR_CONFIG, 0))
    }

    /// QC_OUT (with its elements and channels).
    pub fn qc_out(&self) -> QcOut {
        let cm = self.channel_mapping();
        let v = self.ints(INTS_QC_OUT, 0);
        let mut q = QcOut::default();
        for ch in 0..self.n_channels() {
            q.p_qc_out_channels.push(ptr_bytes(self.ptr(PTR_QC_CHANNEL, ch)));
        }
        for el in 0..cm.n_elements as usize {
            let e = self.ints(INTS_QC_ELEMENT, el);
            let p = self.ptr(PTR_QC_ELEMENT, el);
            let pe: PeData = ptr_bytes(unsafe { (p as *const u8).add(self.off("QC_OUT_ELEMENT.peData")) } as *const c_void);
            let mut x = QcOutElement {
                static_bits_used: e[0],
                dyn_bits_used: e[1],
                ext_bits_used: e[2],
                n_extensions: e[3],
                granted_dyn_bits: e[6],
                granted_pe: e[7],
                granted_pe_corr: e[8],
                pe_data: pe,
                qc_out_channel: [e[9].max(0) as usize, e[10].max(0) as usize],
                ..Default::default()
            };
            if e[10] < 0 {
                x.qc_out_channel[1] = x.qc_out_channel[0] + 1;
            }
            x.extension[0].type_ = e[4];
            x.extension[0].n_payload_bits = e[5];
            q.qc_element.push(x);
        }
        q.n_extensions = v[0];
        q.max_dyn_bits = v[1];
        q.granted_dyn_bits = v[2];
        q.tot_fill_bits = v[3];
        q.element_ext_bits = v[4];
        q.global_ext_bits = v[5];
        q.static_bits = v[6];
        q.total_no_red_pe = v[7];
        q.total_granted_pe_corr = v[8];
        q.used_dyn_bits = v[9];
        q.align_bits = v[10];
        q.total_bits = v[11];
        for k in 0..4 {
            q.extension[k].type_ = v[12 + 2 * k];
            q.extension[k].n_payload_bits = v[13 + 2 * k];
        }
        q
    }
    /// Writes QC_OUT (channels, elements, scalars; the channel assignment is not changed).
    pub fn set_qc_out(&mut self, q: &QcOut) {
        for (ch, x) in q.p_qc_out_channels.iter().enumerate() {
            write_bytes(self.ptr(PTR_QC_CHANNEL, ch), x);
        }
        for (el, x) in q.qc_element.iter().enumerate() {
            let p = self.ptr(PTR_QC_ELEMENT, el);
            write_bytes(unsafe { (p as *mut u8).add(self.off("QC_OUT_ELEMENT.peData")) } as *mut c_void, &x.pe_data);
            let v = [
                x.static_bits_used,
                x.dyn_bits_used,
                x.ext_bits_used,
                x.n_extensions,
                x.extension[0].type_,
                x.extension[0].n_payload_bits,
                x.granted_dyn_bits,
                x.granted_pe,
                x.granted_pe_corr,
            ];
            self.set_ints(INTS_QC_ELEMENT, el, &v);
        }
        let mut v = vec![
            q.n_extensions,
            q.max_dyn_bits,
            q.granted_dyn_bits,
            q.tot_fill_bits,
            q.element_ext_bits,
            q.global_ext_bits,
            q.static_bits,
            q.total_no_red_pe,
            q.total_granted_pe_corr,
            q.used_dyn_bits,
            q.align_bits,
            q.total_bits,
        ];
        for k in 0..4 {
            v.push(q.extension[k].type_);
            v.push(q.extension[k].n_payload_bits);
        }
        self.set_ints(INTS_QC_OUT, 0, &v);
    }

    /// PSY_OUT (the PSY_OUT_CHANNELs without their pointers into QC_OUT_CHANNEL).
    pub fn psy_out(&self) -> PsyOut {
        let cm = self.channel_mapping();
        let mut p = PsyOut::default();
        for ch in 0..self.n_channels() {
            p.p_psy_out_channels.push(ptr_bytes::<PsyOutChannel>(self.ptr(PTR_PSY_CHANNEL, ch)));
        }
        for el in 0..cm.n_elements as usize {
            let e = self.ints(INTS_PSY_OUT_ELEMENT, el);
            let ptr = self.ptr(PTR_PSY_ELEMENT_OUT, el);
            let tools: ToolsInfo = ptr_bytes(unsafe { (ptr as *const u8).add(self.off("PSY_OUT_ELEMENT.toolsInfo")) } as *const c_void);
            let c0 = e[0].max(0) as usize;
            p.psy_out_element.push(PsyOutElement {
                psy_out_channel: [c0, if e[1] < 0 { c0 + 1 } else { e[1] as usize }],
                common_window: e[2],
                tools_info: tools,
            });
        }
        p
    }
    /// Writes PSY_OUT (channel prefixes, commonWindow, toolsInfo).
    pub fn set_psy_out(&mut self, p: &PsyOut) {
        for (ch, x) in p.p_psy_out_channels.iter().enumerate() {
            write_bytes(self.ptr(PTR_PSY_CHANNEL, ch), x);
        }
        for (el, x) in p.psy_out_element.iter().enumerate() {
            let ptr = self.ptr(PTR_PSY_ELEMENT_OUT, el);
            write_bytes(unsafe { (ptr as *mut u8).add(self.off("PSY_OUT_ELEMENT.toolsInfo")) } as *mut c_void, &x.tools_info);
            self.set_ints(INTS_PSY_OUT_ELEMENT, el, &[0, 0, x.common_window]);
        }
    }

    /// ADJ_THR_STATE (with the ATS_ELEMENTs of the channel mapping's elements).
    pub fn adj_thr(&self) -> AdjThrState {
        let v = self.ints(INTS_ADJ_THR, 0);
        let bres = |o: usize| -> BresParam {
            let w: [i32; 8] = v[o..o + 8].try_into().unwrap();
            // SAFETY: BresParam is repr(C) with 8 FIXP_DBL.
            unsafe { std::mem::transmute(w) }
        };
        let n = self.channel_mapping().n_elements as usize;
        AdjThrState {
            bres_param_long: bres(0),
            bres_param_short: bres(8),
            adj_thr_state_elem: (0..n).map(|i| ptr_bytes::<AtsElement>(self.ptr(PTR_ATS_ELEMENT, i))).collect(),
            bit_distribution_mode: v[16],
            max_iter_2nd_guess: v[17],
        }
    }
    pub fn set_adj_thr(&mut self, a: &AdjThrState) {
        let mut v = Vec::new();
        for b in [a.bres_param_long, a.bres_param_short] {
            // SAFETY: BresParam is repr(C) with 8 FIXP_DBL.
            v.extend_from_slice(&unsafe { std::mem::transmute::<BresParam, [i32; 8]>(b) });
        }
        v.push(a.bit_distribution_mode);
        v.push(a.max_iter_2nd_guess);
        self.set_ints(INTS_ADJ_THR, 0, &v);
        for (i, e) in a.adj_thr_state_elem.iter().enumerate() {
            write_bytes(self.ptr(PTR_ATS_ELEMENT, i), e);
        }
    }

    /// The shared dynamic RAM `P_BUF_1` (BUF_SIZE_1 bytes: PSY_DYNAMIC, the bit counter
    /// look-up tables and the adjust-thresholds buffers overlap there).
    pub fn p_buf_1(&self) -> Vec<u8> {
        let p = self.ptr(PTR_DYN_RAM, 0) as *const u8;
        // SAFETY: the dynamic RAM has P_BUF_1 + BUF_SIZE_1 bytes.
        unsafe { std::slice::from_raw_parts(p.add(self.off("P_BUF_1")), self.off("BUF_SIZE_1")) }
            .to_vec()
    }
    pub fn set_p_buf_1(&mut self, b: &[u8]) {
        assert_eq!(b.len(), self.off("BUF_SIZE_1"));
        let p = self.ptr(PTR_DYN_RAM, 0) as *mut u8;
        // SAFETY: as in p_buf_1.
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p.add(self.off("P_BUF_1")), b.len()) };
    }
    /// The adjust-thresholds buffers at the start of `P_BUF_1` (dynMem_Ah_Flag, dynMem_Thr_Exp,
    /// dynMem_SfbNActiveLinesLdData).
    pub fn adj_thr_dyn_mem(&self) -> Box<AdjThrDynMem> {
        assert_eq!(self.off("ADJ_THR_SIZE"), size_of::<AdjThrDynMem>());
        let p = self.ptr(PTR_DYN_RAM, 0) as *const u8;
        Box::new(ptr_bytes(unsafe { p.add(self.off("P_BUF_1")) } as *const c_void))
    }
    pub fn set_adj_thr_dyn_mem(&mut self, d: &AdjThrDynMem) {
        let p = self.ptr(PTR_DYN_RAM, 0) as *mut u8;
        write_bytes(unsafe { p.add(self.off("P_BUF_1")) } as *mut c_void, d);
    }

    /// QC_STATE (element bits, bit counter buffers, ADJ_THR_STATE).
    pub fn qc_state(&self) -> QcState {
        let v = self.ints(INTS_QC_STATE, 0);
        let n = self.channel_mapping().n_elements as usize;
        let bc = self.ptr(PTR_BIT_COUNTER, 0) as *const *const i32;
        // SAFETY: BITCNTR_STATE = { INT *bitLookUp; INT *mergeGainLookUp; } with the sizes of
        // the port's buffers (dynamic RAM).
        let h_bit_counter = unsafe {
            let a = *bc;
            let b = *bc.add(1);
            BitcntrState {
                bit_look_up: std::slice::from_raw_parts(a, MAX_SFB_LONG * 12).to_vec(),
                merge_gain_look_up: std::slice::from_raw_parts(b, MAX_SFB_LONG).to_vec(),
            }
        };
        QcState {
            glob_hdr_bits: v[0],
            max_bits_per_frame: v[1],
            min_bits_per_frame: v[2],
            n_elements: v[3],
            bitrate_mode: v[4],
            bit_res_mode: v[5],
            bit_res_tot: v[6],
            bit_res_tot_max: v[7],
            max_iterations: v[8],
            inv_quant: v[9],
            vbr_qual_factor: v[10],
            max_bit_fac: v[11],
            padding: Padding { padding_rest: v[12] },
            element_bits: (0..n).map(|i| ptr_bytes::<ElementBits>(self.ptr(PTR_ELEMENT_BITS, i))).collect(),
            h_bit_counter,
            h_adj_thr: self.adj_thr(),
            d_zone_quant_enable: v[13],
        }
    }
    /// Writes QC_STATE (scalars, element bits, ADJ_THR_STATE; not the bit counter buffers).
    pub fn set_qc_state(&mut self, q: &QcState) {
        let v = [
            q.glob_hdr_bits,
            q.max_bits_per_frame,
            q.min_bits_per_frame,
            q.n_elements,
            q.bitrate_mode,
            q.bit_res_mode,
            q.bit_res_tot,
            q.bit_res_tot_max,
            q.max_iterations,
            q.inv_quant,
            q.vbr_qual_factor,
            q.max_bit_fac,
            q.padding.padding_rest,
            q.d_zone_quant_enable,
        ];
        self.set_ints(INTS_QC_STATE, 0, &v);
        for (i, e) in q.element_bits.iter().enumerate() {
            write_bytes(self.ptr(PTR_ELEMENT_BITS, i), e);
        }
        self.set_adj_thr(&q.h_adj_thr);
    }

    fn psy_static(&self, ch: usize) -> PsyStatic {
        let p = self.ptr(PTR_PSY_STATIC, ch) as *const u8;
        let at = |name: &str| unsafe { p.add(self.off(name)) } as *const c_void;
        // SAFETY: psyInputBuffer is the first member (a pointer to MAX_INPUT_BUFFER_SIZE INT_PCM).
        let input = unsafe {
            let ib = *(p as *const *const i16);
            std::slice::from_raw_parts(ib, MAX_INPUT_BUFFER_SIZE).to_vec()
        };
        let m = self.ints(INTS_MDCT, ch);
        let mdct = MdctT {
            overlap: Vec::new(),
            prev_wrs: if m[7] > 0 { fdk_get_window_slope(m[7], m[8]) } else { None },
            prev_tl: m[0],
            prev_nr: m[1],
            prev_fr: m[2],
            ov_offset: m[3],
            ov_size: m[4],
            prev_alias_symmetry: m[5],
            prev_prev_alias_symmetry: m[6],
            p_fac_zir: None,
            p_asym_ovlp: None,
        };
        PsyStatic {
            psy_input_buffer: input,
            overlap_add_buffer: ptr_bytes::<[FixpDbl; OVERLAP_ADD_BUFFER_SIZE]>(at("PSY_STATIC.overlapAddBuffer")),
            mdct_pers: mdct,
            block_switching_control: ptr_bytes::<BlockSwitchingControl>(at("PSY_STATIC.blockSwitchingControl")),
            sfb_thresholdnm1: ptr_bytes::<[FixpDbl; MAX_SFB]>(at("PSY_STATIC.sfbThresholdnm1")),
            mdct_scalenm1: ptr_bytes::<i32>(at("PSY_STATIC.mdctScalenm1")),
            calc_pre_echo: ptr_bytes::<i32>(at("PSY_STATIC.calcPreEcho")),
            is_lfe: ptr_bytes::<i32>(at("PSY_STATIC.isLFE")),
        }
    }

    fn set_psy_static(&mut self, ch: usize, s: &PsyStatic) {
        let p = self.ptr(PTR_PSY_STATIC, ch) as *mut u8;
        let at = |name: &str| unsafe { p.add(self.off(name)) } as *mut c_void;
        // SAFETY: as in psy_static.
        unsafe {
            let ib = *(p as *const *mut i16);
            std::ptr::copy_nonoverlapping(s.psy_input_buffer.as_ptr(), ib, MAX_INPUT_BUFFER_SIZE);
        }
        write_bytes(at("PSY_STATIC.overlapAddBuffer"), &s.overlap_add_buffer);
        write_bytes(at("PSY_STATIC.blockSwitchingControl"), &s.block_switching_control);
        write_bytes(at("PSY_STATIC.sfbThresholdnm1"), &s.sfb_thresholdnm1);
        write_bytes(at("PSY_STATIC.mdctScalenm1"), &s.mdct_scalenm1);
        write_bytes(at("PSY_STATIC.calcPreEcho"), &s.calc_pre_echo);
        write_bytes(at("PSY_STATIC.isLFE"), &s.is_lfe);
        let m = &s.mdct_pers;
        let v = [
            m.prev_tl,
            m.prev_nr,
            m.prev_fr,
            m.ov_offset,
            m.ov_size,
            m.prev_alias_symmetry,
            m.prev_prev_alias_symmetry,
        ];
        self.set_ints(INTS_MDCT, ch, &v);
    }

    fn psy_dynamic(&self) -> PsyDynamic {
        let p = self.ptr(PTR_PSY_DYNAMIC, 0) as *const u8;
        let sd = self.off("sizeof PSY_DATA");
        let st = self.off("sizeof TNS_DATA");
        let sp = self.off("sizeof PNS_DATA");
        let skip = self.off("PSY_DATA.sfbThreshold");
        let at = |o: usize| unsafe { p.add(o) } as *const c_void;
        let (od, ot, op) = (self.off("PSY_DYNAMIC.psyData"), self.off("PSY_DYNAMIC.tnsData"), self.off("PSY_DYNAMIC.pnsData"));
        PsyDynamic {
            psy_data: std::array::from_fn(|k| ptr_bytes::<PsyData>(at(od + k * sd + skip))),
            tns_data: std::array::from_fn(|k| ptr_bytes::<TnsData>(at(ot + k * st))),
            pns_data: std::array::from_fn(|k| ptr_bytes::<PnsData>(at(op + k * sp))),
        }
    }

    fn set_psy_dynamic(&mut self, d: &PsyDynamic) {
        let p = self.ptr(PTR_PSY_DYNAMIC, 0) as *mut u8;
        let sd = self.off("sizeof PSY_DATA");
        let st = self.off("sizeof TNS_DATA");
        let sp = self.off("sizeof PNS_DATA");
        let skip = self.off("PSY_DATA.sfbThreshold");
        let at = |o: usize| unsafe { p.add(o) } as *mut c_void;
        let (od, ot, op) = (self.off("PSY_DYNAMIC.psyData"), self.off("PSY_DYNAMIC.tnsData"), self.off("PSY_DYNAMIC.pnsData"));
        for k in 0..2 {
            write_bytes(at(od + k * sd + skip), &d.psy_data[k]);
            write_bytes(at(ot + k * st), &d.tns_data[k]);
            write_bytes(at(op + k * sp), &d.pns_data[k]);
        }
    }

    /// PSY_INTERNAL (PSY_DYNAMIC as it is in the shared dynamic RAM at this point).
    pub fn psy_internal(&self) -> PsyInternal {
        let cm = self.channel_mapping();
        let pi = self.ptr(PTR_PSY_INTERNAL, 0) as *const u8;
        PsyInternal {
            psy_conf: [
                ptr_bytes::<PsyConfiguration>(self.ptr(PTR_PSY_CONF, 0)),
                ptr_bytes::<PsyConfiguration>(self.ptr(PTR_PSY_CONF, 1)),
            ],
            psy_element: (0..cm.n_elements as usize)
                .map(|el| {
                    let e = self.ints(INTS_PSY_ELEMENT, el);
                    let s0 = e[0].max(0) as usize;
                    PsyElement { psy_static: [s0, if e[1] < 0 { s0 + 1 } else { e[1] as usize }] }
                })
                .collect(),
            // all allocated statics (FDKaacEnc_psyInit may assign them from index 1 on)
            p_static_channels: (0..self.ints(INTS_AAC_ENC, 0)[5].max(self.n_channels() as i32)
                as usize)
                .map(|ch| self.psy_static(ch))
                .collect(),
            psy_dynamic: Box::new(self.psy_dynamic()),
            granule_length: ptr_bytes::<i32>(unsafe { pi.add(self.off("PSY_INTERNAL.granuleLength")) } as *const c_void),
        }
    }
    /// Writes the PSY_STATICs and PSY_DYNAMIC (not the configuration).
    pub fn set_psy_internal(&mut self, p: &PsyInternal) {
        for (ch, s) in p.p_static_channels.iter().enumerate() {
            self.set_psy_static(ch, s);
        }
        self.set_psy_dynamic(&p.psy_dynamic);
    }

    /// AAC_ENC with all of its state.
    pub fn aac_enc(&self) -> AacEnc {
        let v = self.ints(INTS_AAC_ENC, 0);
        AacEnc {
            config: self.config(),
            ancillary_bits_per_frame: v[0],
            channel_mapping: self.channel_mapping(),
            qc_kernel: Some(Box::new(self.qc_state())),
            qc_out: [Some(Box::new(self.qc_out()))],
            psy_out: [Some(Box::new(self.psy_out()))],
            psy_kernel: Some(Box::new(self.psy_internal())),
            encoder_mode: v[1],
            bandwidth90d_b: v[2],
            bitrate_mode: v[3],
            dont_write_adif: v[4],
            max_channels: v[5],
            max_elements: v[6],
            max_frames: v[7],
            aot: v[8],
        }
    }
}

// ---------------------------------------------------------------------------------------------
// PCM
// ---------------------------------------------------------------------------------------------

/// Test signals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Signal {
    /// corpus music (`work/corpus`; synthetic multi-tone if missing)
    Music(u32),
    /// a file of `work/corpus-gen` with the rate and channel count (else noise)
    Generated(u32),
    Sweep,
    Noise,
    Clicks,
    Silence,
    Square,
    MultiTone,
}

pub const SIGNALS: [Signal; 9] = [
    Signal::Music(0),
    Signal::Music(1),
    Signal::Generated(0),
    Signal::Sweep,
    Signal::Noise,
    Signal::Clicks,
    Signal::Silence,
    Signal::Square,
    Signal::MultiTone,
];

fn work_dir() -> PathBuf {
    // FDK_AAC_RS_CREF_LIB = <work>/build-cref/<variant>/<key>/libfdkaac_ref.a
    let lib = PathBuf::from(env!("FDK_AAC_RS_CREF_LIB"));
    lib.ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build-cref"))
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
}

const MUSIC: &[(&str, usize)] = &[
    ("music_c_48k.raw", 2),
    ("c51_48k.raw", 6),
    ("music_a_44k.raw", 2),
    ("music_b_mono_44k.raw", 1),
    ("broadcast_a_48k.raw", 2),
    ("music_g_48k.raw", 2),
    ("music_e_96k.raw", 2),
];

/// Interleaved s16le samples of a file: `frames` sample frames from sample frame `start`.
fn read_s16(path: &PathBuf, chans: usize, start: usize, frames: usize) -> Option<Vec<i16>> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len() as usize / (2 * chans);
    if len == 0 {
        return None;
    }
    let start = start % len;
    let n = frames.min(len - start);
    let mut buf = vec![0u8; n * 2 * chans];
    f.seek(SeekFrom::Start((start * 2 * chans) as u64)).ok()?;
    f.read_exact(&mut buf).ok()?;
    Some(buf.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

/// A file of `work/corpus-gen` with the given rate and channel count (k-th match).
fn generated_file(rate: i32, chans: usize, k: u32) -> Option<PathBuf> {
    let dir = work_dir().join("corpus-gen");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter(|p| {
            let t = std::fs::read_to_string(p).unwrap_or_default();
            let num = |key: &str| -> Option<i64> {
                let i = t.find(&format!("\"{key}\""))?;
                let r = &t[i + key.len() + 2..];
                let r = r.trim_start_matches([':', ' ']);
                r.split(|ch: char| !ch.is_ascii_digit()).next()?.parse().ok()
            };
            num("rate") == Some(rate as i64) && num("channels") == Some(chans as i64)
        })
        .collect();
    found.sort();
    if found.is_empty() {
        return None;
    }
    Some(found[k as usize % found.len()].with_extension("s16le"))
}

/// `n_frames` frames of planar PCM (`frame[ch * frame_length + i]`) of a signal.
pub fn pcm_frames(sig: Signal, rate: i32, chans: usize, frame_length: usize, n_frames: usize) -> Vec<Vec<i16>> {
    let total = frame_length * n_frames;
    let mut planar = vec![vec![0i16; total]; chans];
    let mut done = false;
    match sig {
        Signal::Music(k) => {
            let (name, fch) = MUSIC[(k as usize + chans) % MUSIC.len()];
            let path = work_dir().join("corpus").join(name);
            // per channel a different file channel and a time offset (decorrelated channels)
            if let Some(d) = read_s16(&path, fch, 48000 * (5 + k as usize), total + 7919 * chans) {
                let nf = d.len() / fch;
                for (ch, p) in planar.iter_mut().enumerate() {
                    for (i, x) in p.iter_mut().enumerate() {
                        let t = (i + 7919 * (ch / fch)) % nf;
                        *x = d[t * fch + ch % fch];
                    }
                }
                done = true;
            }
        }
        Signal::Generated(k) => {
            if let Some(path) = generated_file(rate, chans, k) {
                if let Some(d) = read_s16(&path, chans, 0, total) {
                    let nf = d.len() / chans;
                    for (ch, p) in planar.iter_mut().enumerate() {
                        for (i, x) in p.iter_mut().enumerate() {
                            *x = d[(i % nf) * chans + ch];
                        }
                    }
                    done = true;
                }
            }
        }
        _ => {}
    }
    if !done {
        let mut rng = fdk_aac_rs_cref::testutil::Rng::new(rate as u64 * 31 + chans as u64);
        let fs = rate as f64;
        for (ch, p) in planar.iter_mut().enumerate() {
            for (i, x) in p.iter_mut().enumerate() {
                let t = i as f64 / fs;
                let v: f64 = match sig {
                    Signal::Sweep => {
                        let f0 = 20.0;
                        let f1 = fs / 2.0;
                        let dur = total as f64 / fs;
                        let ph = std::f64::consts::TAU * (f0 * t + (f1 - f0) * t * t / (2.0 * dur));
                        20000.0 * (ph + ch as f64).sin()
                    }
                    Signal::Noise => (rng.next_u32() as i32 >> 16) as f64 * (0.3 + 0.1 * ch as f64),
                    Signal::Clicks => {
                        if (i + 97 * ch) % 1500 < 3 {
                            if i % 2 == 0 { 32767.0 } else { -32768.0 }
                        } else {
                            (rng.next_u32() as i32 >> 24) as f64
                        }
                    }
                    Signal::Silence => 0.0,
                    Signal::Square => {
                        if (i / (7 + ch)) % 2 == 0 { 32767.0 } else { -32768.0 }
                    }
                    _ => {
                        // multi-tone (also the fallback for missing files)
                        let mut s = 0.0;
                        for (k, f) in [440.0, 1234.5, 3000.0, 7777.0, 15000.0].iter().enumerate() {
                            if *f < fs / 2.0 {
                                s += 5000.0 / (k + 1) as f64
                                    * (std::f64::consts::TAU * f * (1.0 + 0.01 * ch as f64) * t).sin();
                            }
                        }
                        s + (rng.next_u32() as i32 >> 22) as f64
                    }
                };
                *x = v.clamp(-32768.0, 32767.0) as i16;
            }
        }
    }
    (0..n_frames)
        .map(|f| {
            let mut fr = Vec::with_capacity(chans * frame_length);
            for p in planar.iter() {
                fr.extend_from_slice(&p[f * frame_length..(f + 1) * frame_length]);
            }
            fr
        })
        .collect()
}

#[test]
fn harness_matches_library() {
    // The replicated stages give the same access units as the library's aacEncEncode, and the
    // snapshots read back what they wrote.
    let mut ran = 0;
    let mut bytes = 0;
    for (k, cfg) in config_subset(1, 60, |_| true).iter().enumerate() {
        let mut a = match CEnc::open(cfg) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("skip {cfg:?}: AACENC_ERROR {e:#x}");
                continue;
            }
        };
        let Ok(mut b) = CEnc::open(cfg) else { panic!("second instance of {cfg:?}") };
        let sig = SIGNALS[k % SIGNALS.len()];
        let frames = pcm_frames(sig, cfg.sample_rate, a.n_channels(), a.frame_length(), 12);
        for (f, pcm) in frames.iter().enumerate() {
            let x = a.frame(pcm);
            let y = b.lib_frame(pcm);
            assert_eq!(x, y, "{cfg:?} {sig:?} frame {f}");
            let x = x.unwrap_or_else(|e| panic!("{cfg:?} {sig:?} frame {f}: error {e:#x}"));
            assert!(!x.is_empty() || f == 0, "{cfg:?} frame {f}: empty access unit");
            bytes += x.len();
            // the complete states agree as well
            let (mut qa, mut qb) = (a.qc_out(), b.qc_out());
            // Some mdctSpectrum values of FDKaacEnc_psyMain have no defined value in LC
            // configurations: they differ between the two call paths without changing the access
            // unit. They are not compared.
            for q in qa.p_qc_out_channels.iter_mut().chain(qb.p_qc_out_channels.iter_mut()) {
                q.mdct_spectrum = [0; 1024];
            }
            assert_eq!(qa, qb, "{cfg:?} frame {f}: QC_OUT");
            assert_eq!(a.psy_out(), b.psy_out(), "{cfg:?} frame {f}: PSY_OUT");
            assert_eq!(a.qc_state(), b.qc_state(), "{cfg:?} frame {f}: QC_STATE");
            if f % 4 == 3 {
                let (pa, pb) = (a.psy_internal(), b.psy_internal());
                assert_eq!(pa.p_static_channels.len(), pb.p_static_channels.len());
                for (x, y) in pa.p_static_channels.iter().zip(pb.p_static_channels.iter()) {
                    assert_eq!(x.psy_input_buffer, y.psy_input_buffer, "{cfg:?} frame {f}");
                    assert_eq!(x.block_switching_control, y.block_switching_control, "{cfg:?} frame {f}");
                    assert_eq!(x.sfb_thresholdnm1, y.sfb_thresholdnm1, "{cfg:?} frame {f}");
                    assert_eq!(x.overlap_add_buffer, y.overlap_add_buffer, "{cfg:?} frame {f}");
                    assert_eq!(x.mdct_pers.prev_wrs.map(|w| w.as_ptr()), y.mdct_pers.prev_wrs.map(|w| w.as_ptr()));
                }
                assert_eq!(pa.psy_dynamic, pb.psy_dynamic, "{cfg:?} frame {f}: PSY_DYNAMIC");
            }
            if f == 5 {
                let s = a.aac_enc();
                let q = s.qc_out[0].as_ref().unwrap();
                a.set_qc_out(q);
                a.set_psy_out(s.psy_out[0].as_ref().unwrap());
                a.set_qc_state(s.qc_kernel.as_ref().unwrap());
                a.set_psy_internal(s.psy_kernel.as_ref().unwrap());
                let t = a.aac_enc();
                assert_eq!(t.qc_out[0], s.qc_out[0]);
                assert_eq!(t.psy_out[0], s.psy_out[0]);
                assert_eq!(t.qc_kernel, s.qc_kernel);
            }
        }
        ran += 1;
    }
    eprintln!("{ran} configurations, {bytes} bytes");
    assert!(ran >= 12, "only {ran} configurations could be opened");
}

