// Differential tests of fdk_aac_rs_aacenc::aacenc_lib against the C++ reference.
//
// API level: the same PCM and the same call sequence go through the reference API
// (cref_aacEnc*, shim aacenc_aacenc_lib.cpp) and the port's aacenc_lib. Every return value is
// compared, and after every call the complete state that the API exposes: the output buffer
// (all bytes, including the part the encoder clears), the access unit sizes, AACENC_OutArgs,
// aacEncInfo (AACENC_InfoStruct with the configuration buffer) and aacEncoder_GetParam of every
// parameter. The sequences cover every AOT (2, 5, 29, 23, 39 with and without SBR, ELDv2 212,
// 129, 132), CBR and VBR, the afterburner, the frame lengths, all transports with and without
// CRC, signaling modes, metadata modes 0-3 with metadata changes, ancillary data, channel
// orders, 1 to 8 channels, reconfiguration mid-stream and the flush at the end of the input,
// plus random parameter sequences.

use crate::adj_thr_enc::{pcm_frames, Signal};
use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc_lib::*;
use fdk_aac_rs_cref::aacenc_aacenc_lib as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;

/// The parameters of `aacenc_lib.h` (without `AACENC_NONE`), in header order.
pub const ALL_PARAMS: [AacencParam; 21] = [
    AACENC_AOT,
    AACENC_BITRATE,
    AACENC_BITRATEMODE,
    AACENC_SAMPLERATE,
    AACENC_SBR_MODE,
    AACENC_GRANULE_LENGTH,
    AACENC_CHANNELMODE,
    AACENC_CHANNELORDER,
    AACENC_SBR_RATIO,
    AACENC_AFTERBURNER,
    AACENC_BANDWIDTH,
    AACENC_PEAK_BITRATE,
    AACENC_TRANSMUX,
    AACENC_HEADER_PERIOD,
    AACENC_SIGNALING_MODE,
    AACENC_TPSUBFRAMES,
    AACENC_AUDIOMUXVER,
    AACENC_PROTECTION,
    AACENC_ANCILLARY_BITRATE,
    AACENC_METADATA_MODE,
    AACENC_CONTROL_STATE,
];

/// One `aacEncEncode()` call.
#[derive(Clone, Debug)]
pub struct EncCall {
    /// interleaved PCM (16 bit)
    pub pcm: Vec<i16>,
    /// `inargs.numInSamples`
    pub num_in_samples: i32,
    /// `bufElSizes` of the audio buffer: 2 (INT_PCM), 4 (LONG) or 1
    pub el_size: i32,
    /// ancillary data bytes (the buffer is passed if `Some`)
    pub anc: Option<Vec<u8>>,
    /// `inargs.numAncBytes`
    pub num_anc_bytes: i32,
    /// metadata setup (passed if `Some`)
    pub meta: Option<AacencMetaData>,
    /// size of the bitstream buffer
    pub out_size: i32,
    /// pass an OUT_AU_SIZES buffer
    pub au_sizes: bool,
    /// 0: normal; 1: no audio buffer descriptor (NULL inBufDesc); 2: NULL outargs;
    /// 3: output descriptor with a NULL buffer
    pub mode: i32,
}

impl EncCall {
    pub fn pcm(pcm: Vec<i16>, out_size: i32) -> Self {
        let n = pcm.len() as i32;
        EncCall {
            pcm,
            num_in_samples: if n > 0 { n } else { -1 },
            el_size: 2,
            anc: None,
            num_anc_bytes: 0,
            meta: None,
            out_size,
            au_sizes: false,
            mode: 0,
        }
    }
}

/// `AACENC_TRACE=1` prints every call.
fn trace() -> bool {
    std::env::var_os("AACENC_TRACE").is_some()
}

/// The reference encoder and the port's encoder, driven with the same calls.
pub struct Pair {
    pub c: *mut c_void,
    pub r: Option<Box<AacEncoder>>,
    pub what: String,
    pub calls: usize,
    /// all output bytes of the encode calls (reference)
    pub stream: Vec<u8>,
    /// a channel mode was set successfully (the tests call aacEncInfo and the initialization
    /// only then)
    pub ch_set: bool,
    /// encode calls with output bytes that are not compared (see `encode`)
    pub overlong: usize,
    /// save the given number of guard bytes of the reference before every reference encode
    /// call and restore them after it (aacenc_lib_reconfig.rs)
    pub guard: usize,
    /// reference encode calls in which these bytes changed (with `guard`)
    pub guard_hits: usize,
    /// run the port first in encode and init calls and stop before a reference call for which
    /// a counter of `deviation_events` moved (aacenc_lib_reconfig.rs)
    pub stop_undefined: bool,
    /// an encode or init call stopped for `stop_undefined`: the reference was not called, the
    /// encoders differ
    pub stopped_undefined: bool,
    /// with `stop_undefined`: called with `Some(n)` before a reference encode call in which the
    /// port counted `n` reads in mdct_block, and with `None` after that call returned
    /// (aacenc_lib_reconfig.rs)
    pub ref_reads_hook: Option<fn(Option<u64>)>,
}

/// The port's counters of the encoder sites that the reference does not define (see each
/// counter's documentation): `mdct_block_dropped_accesses` (without the reads),
/// `psy_main_negative_lowpass_clears`, `metadata_input_overruns`, `psy_main_stale_short_blocks`,
/// `psy_main_stale_element_mappings`, `psy_main_mixed_block_pairs`,
/// `psy_main_undefined_spectrum_reads`, `psy_main_lfe_flag_pairs` and
/// `adj_thr_stale_ah_flag_reads`.
pub fn deviation_events() -> (u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) {
    let (w, _, n) = fdk_aac_rs_fdk::mdct::mdct_block_dropped_accesses();
    (
        w,
        n,
        fdk_aac_rs_aacenc::psy_main::psy_main_negative_lowpass_clears(),
        fdk_aac_rs_aacenc::metadata_main::metadata_input_overruns(),
        fdk_aac_rs_aacenc::psy_main::psy_main_stale_short_blocks(),
        fdk_aac_rs_aacenc::psy_main::psy_main_stale_element_mappings(),
        fdk_aac_rs_aacenc::psy_main::psy_main_mixed_block_pairs(),
        fdk_aac_rs_aacenc::psy_main::psy_main_undefined_spectrum_reads(),
        fdk_aac_rs_aacenc::psy_main::psy_main_lfe_flag_pairs(),
        fdk_aac_rs_aacenc::adj_thr::adj_thr_stale_ah_flag_reads(),
    )
}

fn info_bytes(i: &AacencInfoStruct) -> Vec<u8> {
    let mut v = Vec::new();
    for x in [
        i.max_out_buf_bytes,
        i.max_anc_bytes,
        i.in_buf_fill_level,
        i.input_channels,
        i.frame_length,
        i.n_delay,
        i.n_delay_core,
    ] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&i.conf_buf);
    v.extend_from_slice(&i.conf_size.to_le_bytes());
    v
}

impl Pair {
    /// `aacEncOpen` on both; `None` (after comparing the errors) if it fails.
    pub fn open(modules: u32, max_channels: u32, what: &str) -> Option<Pair> {
        let mut h: *mut c_void = core::ptr::null_mut();
        // SAFETY: shim call with a valid out pointer.
        let ec = unsafe { c::cref_aacenc_lib_open(&mut h, modules, max_channels) };
        let mut r: Option<Box<AacEncoder>> = None;
        let er = aac_enc_open(Some(&mut r), modules, max_channels);
        assert_eq!(er, ec, "{what}: aacEncOpen({modules:#x}, {max_channels:#x})");
        assert_eq!(h.is_null(), r.is_none(), "{what}: aacEncOpen handle");
        if ec != AACENC_OK {
            return None;
        }
        let mut p =
            Pair {
            c: h,
            r,
            what: what.to_string(),
            calls: 0,
            stream: Vec::new(),
            ch_set: false,
            overlong: 0,
            guard: 0,
            guard_hits: 0,
            stop_undefined: false,
            stopped_undefined: false,
            ref_reads_hook: None,
        };
        p.check_params("after open");
        Some(p)
    }

    pub fn set(&mut self, param: AacencParam, value: u32) -> i32 {
        if trace() {
            eprintln!("{}: SetParam({param:#x}, {value:#x})", self.what);
        }
        // SAFETY: valid handle.
        let ec = unsafe { c::cref_aacenc_lib_set_param(self.c, param, value) };
        let er = aac_encoder_set_param(self.r.as_deref_mut(), param, value);
        assert_eq!(er, ec, "{}: SetParam({param:#x}, {value:#x})", self.what);
        if param == AACENC_CHANNELMODE && ec == AACENC_OK {
            self.ch_set = true;
        }
        self.check_params(&format!("after SetParam({param:#x}, {value:#x})"));
        ec
    }

    pub fn check_params(&mut self, when: &str) {
        for p in ALL_PARAMS.iter().copied().chain([AACENC_NONE, 0x1234]) {
            // SAFETY: valid handle.
            let vc = unsafe { c::cref_aacenc_lib_get_param(self.c, p) };
            let vr = aac_encoder_get_param(self.r.as_deref(), p);
            assert_eq!(vr, vc, "{}: GetParam({p:#x}) {when}", self.what);
        }
    }

    pub fn info(&mut self, when: &str) -> AacencInfoStruct {
        let mut ic = AacencInfoStruct::default();
        let mut ir = AacencInfoStruct::default();
        // poison both, aacEncInfo clears the structure first
        ic.conf_buf = [0x5a; 64];
        ir.conf_buf = [0x5a; 64];
        // SAFETY: valid handle, AacencInfoStruct has the C layout (layout test).
        let ec = unsafe {
            c::cref_aacenc_lib_info(self.c, &mut ic as *mut AacencInfoStruct as *mut c_void)
        };
        let er = aac_enc_info(self.r.as_deref_mut(), Some(&mut ir));
        assert_eq!(er, ec, "{}: aacEncInfo {when}", self.what);
        assert_eq!(info_bytes(&ir), info_bytes(&ic), "{}: AACENC_InfoStruct {when}", self.what);
        ic
    }

    pub fn check_state(&mut self, when: &str) {
        if self.ch_set {
            self.info(when);
        }
        self.check_params(when);
    }

    /// `aacEncEncode(h, NULL, NULL, NULL, NULL)`
    pub fn init(&mut self) -> i32 {
        assert!(self.ch_set, "{}: init without a channel mode", self.what);
        // With `stop_undefined`, the port runs first, as in `encode`.
        let events = deviation_events();
        let er = if self.stop_undefined {
            // SAFETY: no buffers.
            let er = unsafe { aac_enc_encode(self.r.as_deref_mut(), None, None, None, None) };
            if deviation_events() != events {
                self.stopped_undefined = true;
                return er;
            }
            Some(er)
        } else {
            None
        };
        // SAFETY: no buffers.
        let ec = unsafe {
            c::cref_aacenc_lib_encode(
                self.c,
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null_mut(),
            )
        };
        let er = er.unwrap_or_else(|| {
            // SAFETY: no buffers.
            unsafe { aac_enc_encode(self.r.as_deref_mut(), None, None, None, None) }
        });
        assert_eq!(er, ec, "{}: init", self.what);
        self.check_state("after init");
        ec
    }

    /// One `aacEncEncode()` call on both encoders; returns (error, outargs).
    pub fn encode(&mut self, call: &EncCall) -> (i32, AacencOutArgs) {
        assert!(self.ch_set, "{}: encode without a channel mode", self.what);
        let n = self.calls;
        if trace() {
            eprintln!(
                "{}: encode {n}: {} samples (el {}), anc {:?} ({}), meta {}, out {}, au {}, mode {}",
                self.what,
                call.num_in_samples,
                call.el_size,
                call.anc.as_ref().map(|a| a.len()),
                call.num_anc_bytes,
                call.meta.is_some(),
                call.out_size,
                call.au_sizes,
                call.mode
            );
        }
        self.calls += 1;
        let pcm16 = call.pcm.clone();
        // LONG input: the sample in the high half, noise in the low half (dropped by >> 16)
        let pcm32: Vec<i32> = call
            .pcm
            .iter()
            .enumerate()
            .map(|(i, &s)| ((s as i32) << 16) | ((i as i32 * 7919) & 0xffff))
            .collect();
        let mut anc = call.anc.clone().unwrap_or_default();
        let mut meta = call.meta.unwrap_or_default();
        let mut dummy = [0u8; 16];

        let mut results = [None, None];
        // With `stop_undefined`, the port runs first; if a counter of `deviation_events` moved, the
        // reference, which has no defined result there, is not called and the call is not
        // compared.
        let order = if self.stop_undefined { [1, 0] } else { [0, 1] };
        let events = deviation_events();
        let reads_before = fdk_aac_rs_fdk::mdct::mdct_block_dropped_accesses().1;
        let mut reads_hook = None;
        for side in order {
            if side == 0 && self.stop_undefined && deviation_events() != events {
                self.stopped_undefined = true;
                let (e, o, _, _) = results[1].take().unwrap();
                return (e, o);
            }
            if side == 0 && self.stop_undefined {
                let reads = fdk_aac_rs_fdk::mdct::mdct_block_dropped_accesses().1 - reads_before;
                if let (Some(h), true) = (self.ref_reads_hook, reads > 0) {
                    h(Some(reads));
                    reads_hook = Some(h);
                }
            }
            let mut pcm16 = pcm16.clone();
            let mut pcm32 = pcm32.clone();
            let mut in_ptr: [*mut c_void; 3] = [core::ptr::null_mut(); 3];
            let mut in_id = [0i32; 3];
            let mut in_size = [0i32; 3];
            let mut in_el = [0i32; 3];
            let mut nb = 0;
            in_ptr[nb] = if call.el_size == 4 {
                if pcm32.is_empty() {
                    dummy.as_mut_ptr() as *mut c_void
                } else {
                    pcm32.as_mut_ptr() as *mut c_void
                }
            } else if pcm16.is_empty() {
                dummy.as_mut_ptr() as *mut c_void
            } else {
                pcm16.as_mut_ptr() as *mut c_void
            };
            in_id[nb] = IN_AUDIO_DATA;
            in_size[nb] = call.pcm.len() as i32 * call.el_size;
            in_el[nb] = call.el_size;
            nb += 1;
            if call.anc.is_some() {
                in_ptr[nb] = if anc.is_empty() {
                    dummy.as_mut_ptr() as *mut c_void
                } else {
                    anc.as_mut_ptr() as *mut c_void
                };
                in_id[nb] = IN_ANCILLRY_DATA;
                in_size[nb] = anc.len() as i32;
                in_el[nb] = 1;
                nb += 1;
            }
            if call.meta.is_some() {
                in_ptr[nb] = &mut meta as *mut AacencMetaData as *mut c_void;
                in_id[nb] = IN_METADATA_SETUP;
                in_size[nb] = core::mem::size_of::<AacencMetaData>() as i32;
                in_el[nb] = 1;
                nb += 1;
            }
            let in_desc = AacencBufDesc {
                num_bufs: nb as i32,
                bufs: in_ptr.as_mut_ptr(),
                buffer_identifiers: in_id.as_mut_ptr(),
                buf_sizes: in_size.as_mut_ptr(),
                buf_el_sizes: in_el.as_mut_ptr(),
            };
            let mut outbuf = vec![0xA5u8; call.out_size.max(1) as usize];
            let mut au = [0x5a5a5a5ai32; 4];
            let mut out_ptr: [*mut c_void; 2] =
                [outbuf.as_mut_ptr() as *mut c_void, au.as_mut_ptr() as *mut c_void];
            if call.mode == 3 {
                out_ptr[0] = core::ptr::null_mut();
            }
            let mut out_id = [OUT_BITSTREAM_DATA, OUT_AU_SIZES];
            let mut out_size = [call.out_size, 16];
            let mut out_el = [1i32, 4];
            let out_desc = AacencBufDesc {
                num_bufs: if call.au_sizes { 2 } else { 1 },
                bufs: out_ptr.as_mut_ptr(),
                buffer_identifiers: out_id.as_mut_ptr(),
                buf_sizes: out_size.as_mut_ptr(),
                buf_el_sizes: out_el.as_mut_ptr(),
            };
            let in_args =
                AacencInArgs { num_in_samples: call.num_in_samples, num_anc_bytes: call.num_anc_bytes };
            let mut out_args = AacencOutArgs { num_out_bytes: -7, num_in_samples: -7, num_anc_bytes: -7, bit_res_state: -7 };
            let in_desc_p = if call.mode == 1 { None } else { Some(&in_desc) };
            let out_args_p = if call.mode == 2 { None } else { Some(&mut out_args) };
            let mut saved = vec![0u8; self.guard];
            if side == 0 && self.guard > 0 {
                // SAFETY: valid handle; the shim copies `guard` bytes of the reference's state.
                unsafe {
                    fdk_aac_rs_cref::aacenc_aacenc_lib_reconfig::cref_aacenc_lib_reconfig_save(
                        self.c,
                        saved.as_mut_ptr(),
                        self.guard as i32,
                    )
                };
            }
            let e = if side == 0 {
                // SAFETY: the descriptors point to live buffers of the announced sizes; the
                // structures have the C layout (layout test).
                unsafe {
                    c::cref_aacenc_lib_encode(
                        self.c,
                        in_desc_p.map_or(core::ptr::null(), |d| d as *const AacencBufDesc as *const c_void),
                        &out_desc as *const AacencBufDesc as *const c_void,
                        &in_args as *const AacencInArgs as *const c_void,
                        out_args_p.map_or(core::ptr::null_mut(), |o| o as *mut AacencOutArgs as *mut c_void),
                    )
                }
            } else {
                // SAFETY: as above.
                unsafe {
                    aac_enc_encode(
                        self.r.as_deref_mut(),
                        in_desc_p,
                        Some(&out_desc),
                        Some(&in_args),
                        out_args_p,
                    )
                }
            };
            if let (0, Some(h)) = (side, reads_hook) {
                h(None);
            }
            if side == 0 && self.guard > 0 {
                // SAFETY: as for the save above.
                let changed = unsafe {
                    fdk_aac_rs_cref::aacenc_aacenc_lib_reconfig::cref_aacenc_lib_reconfig_restore(
                        self.c,
                        saved.as_ptr(),
                        self.guard as i32,
                    )
                };
                if changed > 0 {
                    self.guard_hits += 1;
                }
            }
            results[side] = Some((e, out_args, outbuf, au));
        }
        let (ec, oc, bc, ac) = results[0].as_ref().unwrap();
        let (er, or, br, ar) = results[1].as_ref().unwrap();
        let what = format!("{}: encode call {n} ({} samples)", self.what, call.num_in_samples);
        assert_eq!(er, ec, "{what}: error");
        assert_eq!(or, oc, "{what}: outArgs");
        // Output bytes without a defined value in the reference are not compared; the port
        // leaves them cleared (PORTING.md, "Deviations from the reference").
        let obib = self.r.as_ref().map_or(0, |h| h.out_buffer_in_bytes) as usize;
        let skip = if oc.num_out_bytes as usize > obib {
            self.overlong += 1;
            obib..(oc.num_out_bytes as usize).min(bc.len())
        } else {
            0..0
        };
        if let Some(k) = (0..bc.len()).find(|&k| br[k] != bc[k] && !skip.contains(&k)) {
            panic!("{what}: output bytes differ at {k} of {} (numOutBytes {})", bc.len(), oc.num_out_bytes);
        }
        assert_eq!(ar, ac, "{what}: AU sizes");
        if oc.num_out_bytes > 0 && (oc.num_out_bytes as usize) <= bc.len() {
            self.stream.extend_from_slice(&bc[..oc.num_out_bytes as usize]);
        }
        self.check_state(&format!("after encode call {n}"));
        (*ec, *oc)
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        // SAFETY: valid handle, closed once.
        let ec = unsafe { c::cref_aacenc_lib_close(self.c) };
        let er = aac_enc_close(Some(&mut self.r));
        if !std::thread::panicking() {
            assert_eq!(er, ec, "{}: aacEncClose", self.what);
        }
    }
}

/// `n` samples per channel of a signal, interleaved.
pub fn signal(sig: Signal, rate: i32, chans: usize, n: usize) -> Vec<i16> {
    let fl = 256usize;
    let frames = n.div_ceil(fl).max(1);
    let planar = pcm_frames(sig, rate, chans, fl, frames);
    let mut out = Vec::with_capacity(n * chans);
    'f: for f in planar.iter() {
        for i in 0..fl {
            if out.len() >= n * chans {
                break 'f;
            }
            for ch in 0..chans {
                out.push(f[ch * fl + i]);
            }
        }
    }
    out
}

/// A configuration of the scenario tests: parameters set after `aacEncOpen`.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub modules: u32,
    pub max_channels: u32,
    pub params: Vec<(AacencParam, u32)>,
    pub channels: usize,
    pub rate: i32,
    pub sig: Signal,
    /// samples per channel of the input
    pub samples: usize,
    /// samples per channel offered per call (0: the frame length)
    pub chunk: usize,
    /// call the initialization `aacEncEncode(h, NULL, NULL, NULL, NULL)` first
    pub init: bool,
    /// (call index, parameter, value) changes
    pub changes: Vec<(usize, AacencParam, u32)>,
    pub anc: bool,
    pub meta: Option<AacencMetaData>,
    /// (call index, metadata) changes
    pub meta_changes: Vec<(usize, AacencMetaData)>,
    pub el_size: i32,
    pub out_size: i32,
    pub au_sizes: bool,
}

/// Runs a scenario through both encoders. Returns the number of encode calls and the output.
pub fn run_scenario(s: &Scenario, what: &str) -> (usize, Vec<u8>) {
    let Some(mut p) = Pair::open(s.modules, s.max_channels, what) else {
        return (0, Vec::new());
    };
    for &(param, value) in &s.params {
        p.set(param, value);
    }
    if !p.ch_set {
        return (0, Vec::new());
    }
    let mut frame_length = 0usize;
    if s.init {
        p.init();
        frame_length = p.info("frame length").frame_length as usize;
    }
    let chunk = if s.chunk != 0 { s.chunk } else if frame_length != 0 { frame_length } else { 1024 };
    let pcm = signal(s.sig, s.rate, s.channels, s.samples);
    let mut rng = Rng::new(s.samples as u64 * 131 + s.channels as u64);
    let mut pos = 0usize;
    let mut meta = s.meta;
    let mut calls = 0usize;
    let mut zero = 0;
    loop {
        for &(at, param, value) in &s.changes {
            if at == calls {
                p.set(param, value);
            }
        }
        for (at, m) in &s.meta_changes {
            if *at == calls {
                meta = Some(*m);
            }
        }
        let n = (chunk * s.channels).min(pcm.len() - pos);
        let mut call = EncCall::pcm(pcm[pos..pos + n].to_vec(), s.out_size);
        call.el_size = s.el_size;
        call.au_sizes = s.au_sizes;
        call.meta = meta;
        if s.anc {
            let k = rng.range(0, 40) as usize;
            let bytes: Vec<u8> = (0..k).map(|_| rng.next_u32() as u8).collect();
            call.num_anc_bytes = bytes.len() as i32;
            call.anc = Some(bytes);
        }
        let (e, o) = p.encode(&call);
        calls += 1;
        if e == AACENC_ENCODE_EOF || calls > 400 {
            break;
        }
        if e != AACENC_OK {
            // the reference continues after errors (and reinitializes); stop after a few
            if calls > 3 && rng.range(0, 3) == 0 {
                break;
            }
        }
        pos += o.num_in_samples.max(0) as usize;
        pos = pos.min(pcm.len());
        if n > 0 && o.num_in_samples == 0 && o.num_out_bytes == 0 {
            zero += 1;
            if zero >= 2 {
                break;
            }
        } else {
            zero = 0;
        }
    }
    (calls, p.stream.clone())
}

#[test]
fn layout_matches() {
    let mut v = [0i32; 64];
    // SAFETY: shim writes 46 values.
    unsafe { c::cref_aacenc_lib_layout(v.as_mut_ptr()) };
    use core::mem::{offset_of, size_of};
    let r: [usize; 46] = [
        size_of::<AacencInfoStruct>(),
        offset_of!(AacencInfoStruct, max_out_buf_bytes),
        offset_of!(AacencInfoStruct, max_anc_bytes),
        offset_of!(AacencInfoStruct, in_buf_fill_level),
        offset_of!(AacencInfoStruct, input_channels),
        offset_of!(AacencInfoStruct, frame_length),
        offset_of!(AacencInfoStruct, n_delay),
        offset_of!(AacencInfoStruct, n_delay_core),
        offset_of!(AacencInfoStruct, conf_buf),
        offset_of!(AacencInfoStruct, conf_size),
        size_of::<AacencBufDesc>(),
        offset_of!(AacencBufDesc, num_bufs),
        offset_of!(AacencBufDesc, bufs),
        offset_of!(AacencBufDesc, buffer_identifiers),
        offset_of!(AacencBufDesc, buf_sizes),
        offset_of!(AacencBufDesc, buf_el_sizes),
        size_of::<AacencInArgs>(),
        offset_of!(AacencInArgs, num_in_samples),
        offset_of!(AacencInArgs, num_anc_bytes),
        size_of::<AacencOutArgs>(),
        offset_of!(AacencOutArgs, num_out_bytes),
        offset_of!(AacencOutArgs, num_in_samples),
        offset_of!(AacencOutArgs, num_anc_bytes),
        offset_of!(AacencOutArgs, bit_res_state),
        size_of::<AacencMetaData>(),
        offset_of!(AacencMetaData, drc_profile),
        offset_of!(AacencMetaData, comp_profile),
        offset_of!(AacencMetaData, drc_target_ref_level),
        offset_of!(AacencMetaData, comp_target_ref_level),
        offset_of!(AacencMetaData, prog_ref_level_present),
        offset_of!(AacencMetaData, prog_ref_level),
        offset_of!(AacencMetaData, pce_mixdown_idx_present),
        offset_of!(AacencMetaData, etsi_dmx_lvl_present),
        offset_of!(AacencMetaData, center_mix_level),
        offset_of!(AacencMetaData, surround_mix_level),
        offset_of!(AacencMetaData, dolby_surround_mode),
        offset_of!(AacencMetaData, drc_presentation_mode),
        offset_of!(AacencMetaData, ext_meta_data),
        offset_of!(AacencExtMetaData, ext_anc_data_enable),
        offset_of!(AacencExtMetaData, ext_downmix_level_enable),
        offset_of!(AacencExtMetaData, ext_downmix_level_a),
        offset_of!(AacencExtMetaData, ext_downmix_level_b),
        offset_of!(AacencExtMetaData, dmx_gain_enable),
        offset_of!(AacencExtMetaData, dmx_gain5),
        offset_of!(AacencExtMetaData, dmx_gain2),
        offset_of!(AacencExtMetaData, lfe_dmx_enable),
    ];
    for (k, &x) in r.iter().enumerate() {
        assert_eq!(x as i32, v[k], "layout entry {k}");
    }
    assert_eq!(offset_of!(AacencExtMetaData, lfe_dmx_level) as i32, v[46]);
}

#[test]
fn get_lib_info() {
    let mut n = 0i32;
    let mut ids = [0i32; 40];
    let mut ver = [0i32; 40];
    let mut flags = [0u32; 40];
    let mut titles = vec![0i8; 64 * 40];
    let mut vs = vec![0i8; 32 * 40];
    // SAFETY: arrays of FDK_MODULE_LAST entries.
    let ec = unsafe {
        c::cref_aacenc_lib_get_lib_info(
            &mut n,
            ids.as_mut_ptr(),
            ver.as_mut_ptr(),
            flags.as_mut_ptr(),
            titles.as_mut_ptr(),
            vs.as_mut_ptr(),
        )
    };
    let mut info = [LibInfo::default(); FDK_MODULE_LAST as usize];
    fdk_init_lib_info(&mut info);
    let er = aac_enc_get_lib_info(Some(&mut info));
    assert_eq!(er, ec);
    let m: Vec<&LibInfo> = info.iter().take_while(|e| e.module_id != FDK_NONE).collect();
    assert_eq!(m.len(), n as usize);
    for (i, e) in m.iter().enumerate() {
        assert_eq!(e.module_id, ids[i], "entry {i}");
        assert_eq!(e.version, ver[i], "entry {i}");
        assert_eq!(e.flags, flags[i], "entry {i}");
        let t: Vec<u8> = titles[64 * i..64 * i + 64].iter().map(|&b| b as u8).collect();
        let tl = t.iter().position(|&b| b == 0).unwrap();
        assert_eq!(e.title.as_bytes(), &t[..tl], "entry {i} title");
        let v: Vec<u8> = vs[32 * i..32 * i + 32].iter().map(|&b| b as u8).collect();
        assert_eq!(e.version_str.to_vec(), v, "entry {i} versionStr");
    }
    // SAFETY: shim call.
    assert_eq!(aac_enc_get_lib_info(None), unsafe { c::cref_aacenc_lib_get_lib_info_null() });
}

#[test]
fn open_variants() {
    let mut rng = Rng::new(17);
    let mut cases: Vec<(u32, u32)> = Vec::new();
    for m in [0u32, 1, 3, 7, 0x0f, 0x11, 0x1f, 0x13, 0x08, 0x02] {
        for ch in [0u32, 1, 2, 6, 8, 9, 0x0102, 0x0201, 0x0802, 0x0902, 0xff] {
            cases.push((m, ch));
        }
    }
    for _ in 0..40 {
        cases.push((rng.next_u32() & 0x1f, rng.next_u32() & 0x0f0f));
    }
    for (m, ch) in cases {
        let what = format!("open({m:#x}, {ch:#x})");
        let Some(mut p) = Pair::open(m, ch, &what) else { continue };
        // a short LC/HE/PS run on whatever was allocated
        p.set(AACENC_SAMPLERATE, 44100);
        p.set(AACENC_CHANNELMODE, 2);
        p.set(AACENC_AOT, [2u32, 5, 29, 23, 39][rng.range(0, 4) as usize]);
        if !p.ch_set {
            continue;
        }
        p.init();
        let pcm = signal(Signal::MultiTone, 44100, 2, 2048);
        p.encode(&EncCall::pcm(pcm, 8192));
    }
}

/// A random metadata setup with every field in the range of its table.
pub fn random_meta(rng: &mut Rng) -> AacencMetaData {
    let mut m = AacencMetaData::default();
    let profiles = [0, 1, 2, 3, 4, 5, 256];
    m.drc_profile = profiles[rng.range(0, 6) as usize];
    m.comp_profile = profiles[rng.range(0, 6) as usize];
    m.drc_target_ref_level = -(rng.range(0, 31) << 16);
    m.comp_target_ref_level = -(rng.range(0, 31) << 16);
    m.prog_ref_level_present = rng.range(0, 1);
    m.prog_ref_level = -(rng.range(0, 31 * 4) << 14);
    m.pce_mixdown_idx_present = rng.range(0, 1) as u8;
    m.etsi_dmx_lvl_present = rng.range(0, 1) as u8;
    m.center_mix_level = rng.range(0, 7) as i8;
    m.surround_mix_level = rng.range(0, 7) as i8;
    m.dolby_surround_mode = rng.range(0, 2) as u8;
    m.drc_presentation_mode = rng.range(0, 2) as u8;
    m.ext_meta_data.ext_anc_data_enable = rng.range(0, 1) as u8;
    m.ext_meta_data.ext_downmix_level_enable = rng.range(0, 1) as u8;
    m.ext_meta_data.ext_downmix_level_a = rng.range(0, 7) as u8;
    m.ext_meta_data.ext_downmix_level_b = rng.range(0, 7) as u8;
    m.ext_meta_data.dmx_gain_enable = rng.range(0, 1) as u8;
    m.ext_meta_data.dmx_gain5 = rng.range(-63, 63) << 14;
    m.ext_meta_data.dmx_gain2 = rng.range(-63, 63) << 14;
    m.ext_meta_data.lfe_dmx_enable = rng.range(0, 1) as u8;
    m.ext_meta_data.lfe_dmx_level = rng.range(0, 15) as u8;
    m
}

const RATES: [i32; 12] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];

fn mode_for(channels: usize) -> u32 {
    [0u32, 1, 2, 3, 4, 5, 6, 11, 12][channels]
}

/// A random scenario of the configuration space (pairwise-random choice of every parameter).
pub fn random_scenario(rng: &mut Rng, k: usize) -> Scenario {
    // AOT variants: 2, 5, 29, 23, 39, 39+SBR, ELDv2 (212), ELDv2+SBR, 129, 132
    let variant = rng.range(0, 9);
    let aot = [2u32, 5, 29, 23, 39, 39, 39, 39, 129, 132][variant as usize];
    let mut channels = match variant {
        2 | 6 | 7 => 2,
        3..=5 => [1usize, 2, 1, 2, 3, 6][rng.range(0, 5) as usize],
        _ => [1usize, 2, 2, 1, 2, 3, 4, 5, 6, 7, 8][rng.range(0, 10) as usize],
    };
    if rng.range(0, 15) == 0 {
        channels = rng.range(1, 8) as usize;
    }
    let rate = match variant {
        1 | 2 | 9 => [16000, 22050, 24000, 32000, 44100, 48000][rng.range(0, 5) as usize],
        5 | 7 => [16000, 22050, 24000, 32000, 44100, 48000][rng.range(0, 5) as usize],
        6 => [16000, 22050, 24000, 32000, 44100, 48000][rng.range(0, 5) as usize],
        _ => RATES[rng.range(0, 11) as usize],
    };
    let mut params: Vec<(AacencParam, u32)> = vec![(AACENC_AOT, aot)];
    match variant {
        4 => params.push((AACENC_SBR_MODE, rng.range(0, 1) as u32)),
        5 => params.push((AACENC_SBR_MODE, 1)),
        7 => params.push((AACENC_SBR_MODE, 1)),
        _ => {}
    }
    params.push((AACENC_SAMPLERATE, rate as u32));
    let mode = if variant == 6 || variant == 7 { 128 } else { mode_for(channels) };
    params.push((AACENC_CHANNELMODE, mode));
    let vbr = rng.range(0, 3) == 0;
    if vbr {
        params.push((AACENC_BITRATEMODE, rng.range(1, 5) as u32));
        if rng.range(0, 2) == 0 {
            params.push((AACENC_PEAK_BITRATE, rng.range(8000, 400000) as u32));
        }
    } else {
        let per = [8000, 16000, 24000, 32000, 48000, 64000, 96000, 128000, 160000][rng.range(0, 8) as usize];
        let ce = channels.min(7) as i32;
        params.push((AACENC_BITRATE, (per * ce) as u32));
        if rng.range(0, 4) == 0 {
            params.push((AACENC_PEAK_BITRATE, (per * ce * 3 / 2) as u32));
        }
    }
    if rng.range(0, 1) == 1 {
        params.push((AACENC_AFTERBURNER, 1));
    }
    if matches!(variant, 3..=7) {
        let fl = [512u32, 480, 512, 480, 256, 240, 128, 120][rng.range(0, if variant == 4 { 7 } else { 3 }) as usize];
        params.push((AACENC_GRANULE_LENGTH, fl));
    }
    let tt = [0u32, 1, 2, 6, 7, 10][rng.range(0, 5) as usize];
    params.push((AACENC_TRANSMUX, tt));
    if rng.range(0, 2) == 0 {
        params.push((AACENC_PROTECTION, 1));
    }
    if rng.range(0, 2) == 0 {
        params.push((AACENC_SIGNALING_MODE, rng.range(0, 2) as u32));
    }
    if rng.range(0, 2) == 0 {
        params.push((AACENC_AUDIOMUXVER, rng.range(0, 2) as u32));
    }
    if rng.range(0, 3) == 0 {
        params.push((AACENC_HEADER_PERIOD, rng.range(0, 12) as u32));
    }
    if rng.range(0, 4) == 0 {
        params.push((AACENC_TPSUBFRAMES, rng.range(1, 4) as u32));
    }
    if rng.range(0, 2) == 0 {
        params.push((AACENC_CHANNELORDER, rng.range(0, 2) as u32));
    }
    if rng.range(0, 5) == 0 {
        params.push((AACENC_BANDWIDTH, rng.range(0, rate / 2) as u32));
    }
    if matches!(variant, 1 | 2 | 4 | 5 | 9) && rng.range(0, 3) == 0 {
        params.push((AACENC_SBR_RATIO, rng.range(0, 2) as u32));
    }
    let meta_mode = rng.range(0, 3) as u32;
    if meta_mode != 0 || rng.range(0, 3) == 0 {
        params.push((AACENC_METADATA_MODE, meta_mode));
    }
    let anc = rng.range(0, 2) == 0;
    if anc && rng.range(0, 2) == 0 {
        params.push((AACENC_ANCILLARY_BITRATE, [0u32, 2000, 4000, 8000, 0xFFFF_FFFF][rng.range(0, 4) as usize]));
    }
    // shuffle a little (the order of SetParam matters for the validations)
    if rng.range(0, 3) == 0 {
        let n = params.len();
        let a = rng.range(0, n as i32 - 1) as usize;
        let b = rng.range(0, n as i32 - 1) as usize;
        params.swap(a, b);
    }
    let meta = if meta_mode != 0 || rng.range(0, 4) == 0 { Some(random_meta(rng)) } else { None };
    let mut meta_changes = Vec::new();
    if meta.is_some() {
        for _ in 0..rng.range(0, 3) {
            meta_changes.push((rng.range(0, 12) as usize, random_meta(rng)));
        }
    }
    let mut changes = Vec::new();
    for _ in 0..rng.range(0, 2) {
        let at = rng.range(1, 10) as usize;
        let (p, v) = match rng.range(0, 9) {
            0 => (AACENC_BITRATE, rng.range(16000, 256000) as u32),
            1 => (AACENC_TRANSMUX, [0u32, 1, 2, 6, 7, 10][rng.range(0, 5) as usize]),
            2 => (AACENC_METADATA_MODE, rng.range(0, 3) as u32),
            3 => (AACENC_AFTERBURNER, rng.range(0, 1) as u32),
            4 => (AACENC_BITRATEMODE, rng.range(0, 5) as u32),
            5 => (AACENC_PROTECTION, rng.range(0, 1) as u32),
            6 => (AACENC_CHANNELMODE, mode_for(rng.range(1, channels as i32) as usize)),
            7 => (AACENC_BANDWIDTH, rng.range(0, rate / 2) as u32),
            _ => (AACENC_HEADER_PERIOD, rng.range(0, 10) as u32),
        };
        changes.push((at, p, v));
    }
    let sigs = [
        Signal::Music(0),
        Signal::Music(1),
        Signal::Sweep,
        Signal::Noise,
        Signal::Clicks,
        Signal::Silence,
        Signal::Square,
        Signal::MultiTone,
    ];
    Scenario {
        modules: if rng.range(0, 5) == 0 { [1u32, 3, 7, 0x0f, 0x17, 0x19][rng.range(0, 5) as usize] } else { 0 },
        // (MODE_212 with one channel: the port rejects it)
        max_channels: match if rng.range(0, 6) == 0 { 0 } else { channels as u32 } {
            1 if mode == 128 => 2,
            m => m,
        },
        params,
        channels,
        rate,
        sig: sigs[(k + rng.range(0, 7) as usize) % sigs.len()],
        samples: rng.range(1000, 30000) as usize,
        chunk: if rng.range(0, 4) == 0 { rng.range(1, 3000) as usize } else { 0 },
        init: rng.range(0, 5) != 0,
        changes,
        anc,
        meta,
        meta_changes,
        el_size: [2, 2, 2, 2, 4, 1][rng.range(0, 5) as usize],
        out_size: if rng.range(0, 10) == 0 { rng.range(0, 3000) } else { 20480 },
        au_sizes: rng.range(0, 3) == 0,
    }
}

fn run_random(seed: u64, n: usize) {
    let mut rng = Rng::new(seed);
    let mut calls = 0;
    let mut bytes = 0;
    for k in 0..n {
        let s = random_scenario(&mut rng, k);
        let (c, out) = run_scenario(&s, &format!("seed {seed} scenario {k} {s:?}"));
        calls += c;
        bytes += out.len();
    }
    eprintln!("aacenc_lib: {n} scenarios, {calls} encode calls, {bytes} output bytes");
}

#[test]
fn api_random_scenarios_1() {
    run_random(1, 200);
}

#[test]
fn api_random_scenarios_2() {
    run_random(2, 200);
}

#[test]
fn api_random_scenarios_3() {
    run_random(3, 200);
}

/// Every AOT variant with each transport, CBR and VBR, a few frames and the flush.
#[test]
fn api_aot_transport_matrix() {
    let mut rng = Rng::new(99);
    let variants: [(u32, Option<u32>, u32, usize, i32); 10] = [
        (2, None, 2, 2, 44100),
        (5, None, 2, 2, 44100),
        (29, None, 2, 2, 48000),
        (23, None, 2, 2, 48000),
        (39, Some(0), 2, 2, 48000),
        (39, Some(1), 2, 2, 48000),
        (39, None, 128, 2, 48000),
        (39, Some(1), 128, 2, 32000),
        (129, None, 1, 1, 32000),
        (132, None, 2, 2, 44100),
    ];
    for (aot, sbr, mode, ch, rate) in variants {
        for tt in [0u32, 1, 2, 6, 7, 10] {
            for prot in [0u32, 1] {
                for brm in [0u32, 3] {
                    let mut params = vec![(AACENC_AOT, aot)];
                    if let Some(s) = sbr {
                        params.push((AACENC_SBR_MODE, s));
                    }
                    params.extend([
                        (AACENC_SAMPLERATE, rate as u32),
                        (AACENC_CHANNELMODE, mode),
                        (AACENC_TRANSMUX, tt),
                        (AACENC_PROTECTION, prot),
                        (AACENC_BITRATEMODE, brm),
                    ]);
                    if brm == 0 {
                        params.push((AACENC_BITRATE, 64000));
                    }
                    let s = Scenario {
                        modules: 0,
                        max_channels: ch as u32,
                        params,
                        channels: ch,
                        rate,
                        sig: Signal::Music(rng.range(0, 1) as u32),
                        samples: 6000,
                        chunk: 0,
                        init: true,
                        changes: vec![],
                        anc: false,
                        meta: None,
                        meta_changes: vec![],
                        el_size: 2,
                        out_size: 20480,
                        au_sizes: true,
                    };
                    run_scenario(&s, &format!("matrix aot {aot} sbr {sbr:?} mode {mode} tt {tt} crc {prot} brm {brm}"));
                }
            }
        }
    }
}

/// A switch from ADTS to LATM MCP0, directly and with an MCP1 initialization in between. The
/// port reproduces the transport writer union from the C layout; the test compares the port
/// with the reference, and reports whether the two sequences give different reference output.
#[test]
fn transmux_reinit() {
    let mut differing = 0;
    let mut cases = 0;
    for (aot, rate, ch, br) in [(2u32, 48000, 2usize, 128000u32), (2, 44100, 1, 64000), (5, 48000, 2, 64000), (23, 48000, 2, 96000), (39, 48000, 1, 64000)] {
        let mut outs = Vec::new();
        for via_mcp1 in [false, true] {
            let what = format!("transmux aot {aot} via MCP1 {via_mcp1}");
            let Some(mut p) = Pair::open(0, ch as u32, &what) else { continue };
            for (a, v) in [(AACENC_AOT, aot), (AACENC_SAMPLERATE, rate as u32), (AACENC_CHANNELMODE, ch as u32), (AACENC_BITRATE, br), (AACENC_TRANSMUX, 2)] {
                p.set(a, v);
            }
            p.init();
            let fl = p.info("fl").frame_length as usize;
            let pcm = signal(Signal::Music(0), rate, ch, fl * 12);
            for f in 0..4 {
                p.encode(&EncCall::pcm(pcm[f * fl * ch..(f + 1) * fl * ch].to_vec(), 20480));
            }
            if via_mcp1 {
                p.set(AACENC_TRANSMUX, 6);
                p.init();
            }
            p.set(AACENC_TRANSMUX, 7);
            let start = p.stream.len();
            for f in 4..12 {
                p.encode(&EncCall::pcm(pcm[f * fl * ch..(f + 1) * fl * ch].to_vec(), 20480));
            }
            outs.push(p.stream[start..].to_vec());
        }
        if outs.len() == 2 {
            cases += 1;
            if outs[0] != outs[1] {
                differing += 1;
                eprintln!("transmux: aot {aot}, {rate} Hz, {ch} ch, {br} bit/s: MCP0 output differs");
            }
        }
    }
    eprintln!("transmux: {differing} of {cases} configurations give other MCP0 output after ADTS than after an MCP1 initialization");
}

/// Random parameter sequences: SetParam with random (also invalid) parameters and values,
/// interleaved with encode calls of random sizes, element sizes, ancillary data, metadata and
/// output buffer sizes. Inputs without a defined reference result are left out.
#[test]
fn api_random_sequences() {
    let mut rng = Rng::new(4242);
    for k in 0..300 {
        let ch = rng.range(1, 8) as usize;
        let what = format!("sequence {k}");
        let Some(mut p) = Pair::open(if rng.range(0, 4) == 0 { rng.next_u32() & 0x1f } else { 0 }, ch as u32, &what) else { continue };
        let rate = RATES[rng.range(0, 11) as usize];
        p.set(AACENC_SAMPLERATE, rate as u32);
        p.set(AACENC_CHANNELMODE, mode_for(ch));
        if !p.ch_set {
            continue;
        }
        let pcm = signal(Signal::Music(rng.range(0, 1) as u32), rate, ch, 20000);
        let mut pos = 0usize;
        let mut initialized = false;
        for _ in 0..40 {
            match rng.range(0, 9) {
                0..=2 => {
                    let param = ALL_PARAMS[rng.range(0, 19) as usize];
                    let value = match rng.range(0, 5) {
                        0 => rng.next_u32(),
                        1 => rng.range(0, 10) as u32,
                        2 => [2u32, 5, 29, 23, 39, 129, 132, 1, 128, 1024, 512, 480, 960, 64000, 0xFFFF_FFFF][rng.range(0, 14) as usize],
                        _ => rng.range(0, 300000) as u32,
                    };
                    // keep the channel count (the PCM layout) and a valid sample rate
                    if param == AACENC_CHANNELMODE || param == AACENC_SAMPLERATE {
                        continue;
                    }
                    // inputs without a defined reference result are left out (the port returns
                    // an error for them)
                    let value =
                        if param == AACENC_BANDWIDTH { value % (rate as u32 / 2 + 1) } else { value };
                    p.set(param, value);
                }
                3 if initialized => {
                    // only values with a defined reference result
                    let v = [1u32, 3, 0x1001, 0x2001, 0xFFFF, 0x1003, 0x3003][rng.range(0, 6) as usize];
                    p.set(AACENC_CONTROL_STATE, v);
                }
                4 => {
                    p.init();
                }
                _ => {
                    let n = (rng.range(0, 3000) as usize * ch).min(pcm.len() - pos);
                    let mut call = EncCall::pcm(pcm[pos..pos + n].to_vec(), if rng.range(0, 8) == 0 { rng.range(0, 2000) } else { 20480 });
                    call.el_size = [2, 2, 4, 1][rng.range(0, 3) as usize];
                    if rng.range(0, 3) == 0 {
                        let a: Vec<u8> = (0..rng.range(0, 300)).map(|_| rng.next_u32() as u8).collect();
                        call.num_anc_bytes = a.len() as i32;
                        call.anc = Some(a);
                    }
                    if rng.range(0, 3) == 0 {
                        call.meta = Some(random_meta(&mut rng));
                    }
                    if rng.range(0, 12) == 0 {
                        call.mode = rng.range(1, 3);
                    }
                    call.au_sizes = rng.range(0, 2) == 0;
                    let (e, o) = p.encode(&call);
                    if e == AACENC_OK {
                        initialized = true;
                    }
                    pos = (pos + o.num_in_samples.max(0) as usize).min(pcm.len());
                    if pos >= pcm.len() {
                        pos = 0;
                    }
                }
            }
        }
        // flush to the end
        for _ in 0..30 {
            let (e, _) = p.encode(&EncCall::pcm(Vec::new(), 20480));
            if e != AACENC_OK {
                break;
            }
        }
    }
}

/// MODE_212 after aacEncOpen with one channel: the reference accepts it, the port rejects it
/// (PORTING.md, "Deviations from the reference").
#[test]
fn mode_212_needs_two_channels() {
    for max_channels in [1u32, 2] {
        let mut r = None;
        assert_eq!(aac_enc_open(Some(&mut r), 0, max_channels), AACENC_OK);
        let mut c: *mut core::ffi::c_void = core::ptr::null_mut();
        // SAFETY: shim call; the handle is closed below.
        assert_eq!(unsafe { c::cref_aacenc_lib_open(&mut c, 0, max_channels) }, AACENC_OK);
        assert_eq!(aac_encoder_set_param(r.as_deref_mut(), AACENC_AOT, 39), AACENC_OK);
        // SAFETY: valid handle.
        assert_eq!(unsafe { c::cref_aacenc_lib_set_param(c, AACENC_AOT, 39) }, AACENC_OK);
        let er = aac_encoder_set_param(r.as_deref_mut(), AACENC_CHANNELMODE, 128);
        // SAFETY: valid handle.
        let ec = unsafe { c::cref_aacenc_lib_set_param(c, AACENC_CHANNELMODE, 128) };
        assert_eq!(ec, AACENC_OK, "reference, maxChannels {max_channels}");
        let want = if max_channels == 1 { AACENC_INVALID_CONFIG } else { AACENC_OK };
        assert_eq!(er, want, "port, maxChannels {max_channels}");
        // SAFETY: valid handle, not used afterwards.
        unsafe { c::cref_aacenc_lib_close(c) };
        assert_eq!(aac_enc_close(Some(&mut r)), AACENC_OK);
    }
}
