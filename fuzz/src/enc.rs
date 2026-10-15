//! Encoder target: random parameter sequences and random PCM, run on the reference and on the
//! port.
//!
//! # Input format
//!
//! ```text
//! byte 0    encModules of aacEncOpen (MODULES[b & 15])
//! byte 1    maxChannels of aacEncOpen (MAX_CHANNELS[b & 15])
//! byte 2    number of op bytes L
//! L bytes   ops, executed in order; then frame-sized encode calls up to DEFAULT_ENCODES and
//!           flush calls (numInSamples = -1) until AACENC_ENCODE_EOF
//! rest      the PCM tape (16-bit little endian), read cyclically by PCM style 0
//! ```
//!
//! # Ops
//!
//! `op >> 5` selects the kind, `op & 31` is its argument:
//!
//! | kind | effect |
//! |---|---|
//! | 0 | SetParam(`ENC_PARAMS[arg]`, a value of that parameter from the next op byte, see `param_value`) |
//! | 1 | aacEncEncode with `CHUNKS[arg]` samples per channel; the next op byte: bits 0-1 element size (2, 2, 4, 1), bit 2 ancillary data, bit 3 metadata, bit 4 a small output buffer (next op byte), bit 5 an OUT_AU_SIZES buffer, bits 6-7 == 3: a NULL input descriptor / NULL outArgs / NULL output buffer |
//! | 2 | aacEncEncode(h, NULL, NULL, NULL, NULL) (initialization) |
//! | 3 | PCM style `arg & 7` (tape, tone, noise, silence, square, impulses, DC, scaled tape); `arg & 8` rewinds the tape |
//! | 4 | SetParam(AACENC_CONTROL_STATE, `CONTROL[arg % 7]`) |
//! | 5 | `1 + (arg & 15)` frame-sized encode calls |
//! | 6 | metadata from the next 8 op bytes (fields in their valid ranges); `arg & 1` passes it with the following encode calls |
//! | 7 | ancillary data of `next op byte` bytes with the following encode calls (`arg & 1`: off) |
//!
//! After every call the trace holds the return values, and after every call aacEncInfo (once a
//! channel mode is set) and aacEncoder_GetParam of every parameter; after aacEncEncode the
//! whole output buffer, AACENC_OutArgs and the OUT_AU_SIZES buffer (PORTING.md, "Definition of
//! bit-exact").
//!
//! # Constraints
//!
//! API sequences without a defined reference result are not generated, and output bytes without
//! a defined reference result are not compared (PORTING.md, "Deviations from the reference").
//! The port counts the remaining calls without a defined reference result
//! (`deviation_events`). The port runs first (`deviation_stop`); both runs stop before the first
//! initialization or encode call in which it counted one (`Abort::Deviation`, reported like a
//! stopped reference call).

use crate::guard::{Abort, Session};
use crate::{Blob, Outcome, Reader, Trace};
use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc_lib::*;
use fdk_aac_rs_aacenc::adj_thr::adj_thr_stale_ah_flag_reads;
use fdk_aac_rs_aacenc::metadata_main::metadata_input_overruns;
use fdk_aac_rs_aacenc::psy_main::{
    psy_main_lfe_flag_pairs, psy_main_mixed_block_pairs, psy_main_negative_lowpass_clears,
    psy_main_stale_element_mappings, psy_main_stale_short_blocks,
    psy_main_undefined_spectrum_reads,
};
use fdk_aac_rs_cref::aacenc_aacenc_lib as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::mdct::mdct_block_dropped_accesses;

pub const MODULES: [u32; 16] =
    [0, 0, 0, 0, 0, 0, 1, 3, 7, 0x0f, 0x11, 0x13, 0x17, 0x19, 0x1f, 0x10];
pub const MAX_CHANNELS: [u32; 16] =
    [0, 1, 2, 2, 2, 3, 4, 5, 6, 7, 8, 0x0102, 0x0201, 0x0802, 9, 0xff];
/// Parameters (all of aacenc_lib.h; some twice for weight; unknown ones).
pub const ENC_PARAMS: [i32; 32] = [
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
    AACENC_AOT,
    AACENC_BITRATE,
    AACENC_BITRATEMODE,
    AACENC_TRANSMUX,
    AACENC_SBR_MODE,
    AACENC_CHANNELMODE,
    AACENC_GRANULE_LENGTH,
    AACENC_AFTERBURNER,
    AACENC_NONE,
    0x1234,
    0x0201,
];
/// The parameters read back after every call (`ALL_PARAMS` of the cref test, plus unknown ones).
pub const GET_PARAMS: [i32; 23] = [
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
    AACENC_NONE,
    0x1234,
];
pub const RATES: [u32; 12] =
    [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];
/// Control states: all with AACENC_INIT_CONFIG (see the constraints).
pub const CONTROL: [u32; 7] = [1, 3, 0x1001, 0x2001, 0xFFFF, 0x1003, 0x3003];
/// Samples per channel of an encode op (`FRAME`: the frame length of aacEncInfo; `FLUSH`:
/// numInSamples = -1).
const FRAME: i32 = -2;
const FLUSH: i32 = -1;
pub const CHUNKS: [i32; 32] = [
    FRAME, FRAME, FRAME, FRAME, FRAME, FRAME, FRAME, FRAME, FLUSH, FLUSH, 0, 1, 2, 7, 64, 120, 128,
    240, 256, 480, 512, 960, 1000, 1023, 1024, 1025, 1500, 2047, 2048, 3000, 4096, 6000,
];
const OUT_SMALL: [i32; 8] = [0, 1, 7, 64, 300, 768, 1536, 6144];
const DEFAULT_OUT: i32 = 20480;
const DEFAULT_ENCODES: usize = 6;
const MAX_ENCODES: usize = 48;
const MAX_PER_CHANNEL: usize = 6000;

/// A value of `param` from one byte (and the following op bytes for raw values).
pub fn param_value(param: i32, b: u8, rate: u32, more: &mut impl FnMut() -> u8) -> u32 {
    let pick = |t: &[u32]| t[b as usize % t.len()];
    if b >= 0xF8 {
        // edge values for every parameter
        return [0u32, 1, 0xFFFF_FFFF, 0x7FFF_FFFF, 0x8000_0000, 0x10000, 255]
            [(b - 0xF8) as usize % 7];
    }
    if b == 0xF7 {
        return u32::from_le_bytes([more(), more(), more(), more()]);
    }
    match param {
        AACENC_AOT => {
            pick(&[2, 5, 29, 23, 39, 129, 132, 2, 5, 29, 39, 1, 0, 42, 17, 20, 22, 256, 3, 4])
        }
        AACENC_BITRATE | AACENC_PEAK_BITRATE => pick(&[
            8000, 12000, 16000, 24000, 32000, 40000, 48000, 56000, 64000, 80000, 96000, 112000,
            128000, 160000, 192000, 256000, 320000, 384000, 448000, 512000, 640000, 800000, 1000,
            6000, 0, 100, 9000, 1_000_000, 2_000_000, 64001,
        ]),
        AACENC_BITRATEMODE => b as u32 % 10,
        AACENC_SAMPLERATE => pick(&[
            8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000, 0,
            7350, 44101, 192000,
        ]),
        AACENC_SBR_MODE => pick(&[0, 1, 0xFFFF_FFFF, 2]),
        AACENC_GRANULE_LENGTH => pick(&[1024, 512, 480, 256, 240, 128, 120, 960, 0, 2048, 1023]),
        AACENC_CHANNELMODE => pick(&[
            1, 2, 3, 4, 5, 6, 7, 11, 12, 13, 14, 33, 34, 128, 1, 2, 2, 0, 8, 9, 10, 15, 16, 0xFFFF,
        ]),
        AACENC_CHANNELORDER
        | AACENC_SBR_RATIO
        | AACENC_AFTERBURNER
        | AACENC_SIGNALING_MODE
        | AACENC_AUDIOMUXVER
        | AACENC_PROTECTION => b as u32 % 4,
        AACENC_BANDWIDTH => {
            // at most fs/2 (see the constraints)
            let half = rate / 2;
            if b < 0x40 {
                b as u32 * half / 0x3F
            } else {
                (b as u32 - 0x40) * 997 % (half + 1)
            }
        }
        AACENC_TRANSMUX => pick(&[0, 1, 2, 6, 7, 10, 2, 6, 10, 12, 3, 0xFFFF_FFFF, 11]),
        AACENC_HEADER_PERIOD => pick(&[0, 1, 2, 3, 5, 9, 10, 16, 255, 256, 0xFF]),
        AACENC_TPSUBFRAMES => b as u32 % 6,
        AACENC_ANCILLARY_BITRATE => pick(&[0, 2000, 4000, 8000, 0xFFFF_FFFF, 64000, 1]),
        AACENC_METADATA_MODE => b as u32 % 5,
        _ => b as u32,
    }
}

/// Metadata from 8 bytes, every field in the range of its table (as `random_meta` of the cref
/// tests).
fn meta_from(b: &[u8; 8]) -> AacencMetaData {
    let mut m = AacencMetaData::default();
    let profiles = [0, 1, 2, 3, 4, 5, 256];
    m.drc_profile = profiles[(b[0] & 7) as usize % 7];
    m.comp_profile = profiles[(b[0] >> 4) as usize % 7];
    m.drc_target_ref_level = -(((b[1] & 31) as i32) << 16);
    m.comp_target_ref_level = -(((b[1] >> 3) as i32 % 32) << 16);
    m.prog_ref_level_present = (b[2] & 1) as i32;
    m.prog_ref_level = -(((b[2] >> 1) as i32 % 125) << 14);
    m.pce_mixdown_idx_present = b[3] & 1;
    m.etsi_dmx_lvl_present = (b[3] >> 1) & 1;
    m.center_mix_level = ((b[3] >> 2) & 7) as i8;
    m.surround_mix_level = ((b[3] >> 5) & 7) as i8;
    m.dolby_surround_mode = b[4] % 3;
    m.drc_presentation_mode = (b[4] >> 2) % 3;
    m.ext_meta_data.ext_anc_data_enable = (b[4] >> 4) & 1;
    m.ext_meta_data.ext_downmix_level_enable = (b[4] >> 5) & 1;
    m.ext_meta_data.ext_downmix_level_a = b[5] & 7;
    m.ext_meta_data.ext_downmix_level_b = (b[5] >> 3) & 7;
    m.ext_meta_data.dmx_gain_enable = (b[5] >> 6) & 1;
    m.ext_meta_data.dmx_gain5 = ((b[6] & 127) as i32 - 63) << 14;
    m.ext_meta_data.dmx_gain2 = ((b[7] & 127) as i32 - 63) << 14;
    m.ext_meta_data.lfe_dmx_enable = (b[6] >> 7) & 1;
    m.ext_meta_data.lfe_dmx_level = (b[7] >> 4) & 15;
    m
}

/// The program of one input.
#[derive(Clone, Debug)]
pub struct EncProgram {
    pub modules: u32,
    pub max_channels: u32,
    pub ops: Vec<u8>,
    pub tape: Vec<u8>,
}

impl EncProgram {
    pub fn parse(data: &[u8]) -> EncProgram {
        let mut r = Reader::new(data);
        let modules = MODULES[(r.u8() & 15) as usize];
        let max_channels = MAX_CHANNELS[(r.u8() & 15) as usize];
        let n = r.u8() as usize;
        let ops = r.bytes(n).to_vec();
        EncProgram { modules, max_channels, ops, tape: r.rest().to_vec() }
    }
}

/// One `aacEncEncode()` call.
#[derive(Clone, Debug, Default)]
struct EncCall {
    pcm: Vec<i16>,
    num_in_samples: i32,
    el_size: i32,
    anc: Option<Vec<u8>>,
    meta: Option<AacencMetaData>,
    out_size: i32,
    au_sizes: bool,
    mode: i32,
}

/// One call and what it returned.
#[derive(Clone, Debug, PartialEq)]
pub enum EncEv {
    Open {
        modules: u32,
        max_channels: u32,
        ret: i32,
        null: bool,
    },
    SetParam {
        param: i32,
        value: u32,
        ret: i32,
    },
    GetParams {
        values: Vec<u32>,
    },
    Info {
        ret: i32,
        info: Vec<u8>,
    },
    Init {
        ret: i32,
    },
    Encode {
        n: usize,
        num_in_samples: i32,
        el_size: i32,
        anc: i32,
        meta: bool,
        out_size: i32,
        mode: i32,
        ret: i32,
        out_args: [i32; 4],
        /// The output buffer, with the bytes beyond the internal output buffer cleared.
        out: Blob,
        au_sizes: [i32; 4],
    },
    Close {
        ret: i32,
    },
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

/// The encoder API as both sides offer it. `encode` gets the C-layout descriptors.
trait EncApi {
    fn open(&mut self, modules: u32, max_channels: u32) -> Result<(i32, bool), Abort>;
    fn set_param(&mut self, param: i32, value: u32) -> Result<i32, Abort>;
    fn get_param(&mut self, param: i32) -> Result<u32, Abort>;
    fn info(&mut self, info: &mut AacencInfoStruct) -> Result<i32, Abort>;
    /// # Safety
    /// The descriptors must be valid as aacEncEncode requires.
    unsafe fn encode(
        &mut self,
        inb: Option<&AacencBufDesc>,
        outb: Option<&AacencBufDesc>,
        ina: Option<&AacencInArgs>,
        outa: Option<&mut AacencOutArgs>,
    ) -> Result<i32, Abort>;
    fn close(&mut self) -> Result<i32, Abort>;
}

struct RefEnc {
    s: Session,
    h: *mut c_void,
}

impl EncApi for RefEnc {
    fn open(&mut self, modules: u32, max_channels: u32) -> Result<(i32, bool), Abort> {
        let mut h: *mut c_void = core::ptr::null_mut();
        let hp = &mut h as *mut *mut c_void;
        let mut r = 0;
        // SAFETY: shim call with a valid out pointer.
        self.s.call(|| r = unsafe { c::cref_aacenc_lib_open(hp, modules, max_channels) })?;
        self.h = h;
        Ok((r, h.is_null()))
    }

    fn set_param(&mut self, param: i32, value: u32) -> Result<i32, Abort> {
        let h = self.h;
        let mut r = 0;
        // SAFETY: valid handle.
        self.s.call(|| r = unsafe { c::cref_aacenc_lib_set_param(h, param, value) })?;
        Ok(r)
    }

    fn get_param(&mut self, param: i32) -> Result<u32, Abort> {
        let h = self.h;
        let mut r = 0;
        // SAFETY: valid handle.
        self.s.call(|| r = unsafe { c::cref_aacenc_lib_get_param(h, param) })?;
        Ok(r)
    }

    fn info(&mut self, info: &mut AacencInfoStruct) -> Result<i32, Abort> {
        let (h, p) = (self.h, info as *mut AacencInfoStruct as *mut c_void);
        let mut r = 0;
        // SAFETY: valid handle; AacencInfoStruct has the C layout (cref layout test).
        self.s.call(|| r = unsafe { c::cref_aacenc_lib_info(h, p) })?;
        Ok(r)
    }

    unsafe fn encode(
        &mut self,
        inb: Option<&AacencBufDesc>,
        outb: Option<&AacencBufDesc>,
        ina: Option<&AacencInArgs>,
        outa: Option<&mut AacencOutArgs>,
    ) -> Result<i32, Abort> {
        let h = self.h;
        let ip = inb.map_or(core::ptr::null(), |d| d as *const AacencBufDesc as *const c_void);
        let op = outb.map_or(core::ptr::null(), |d| d as *const AacencBufDesc as *const c_void);
        let iap = ina.map_or(core::ptr::null(), |a| a as *const AacencInArgs as *const c_void);
        let oap = outa.map_or(core::ptr::null_mut(), |a| a as *mut AacencOutArgs as *mut c_void);
        let mut r = 0;
        // SAFETY: the caller's guarantee; the structures have the C layout.
        self.s.call(|| r = unsafe { c::cref_aacenc_lib_encode(h, ip, op, iap, oap) })?;
        Ok(r)
    }

    fn close(&mut self) -> Result<i32, Abort> {
        let h = self.h;
        let mut r = 0;
        // SAFETY: valid handle, closed once.
        self.s.call_unbudgeted(|| r = unsafe { c::cref_aacenc_lib_close(h) })?;
        self.h = core::ptr::null_mut();
        Ok(r)
    }
}

struct PortEnc {
    h: Option<Box<AacEncoder>>,
}

impl EncApi for PortEnc {
    fn open(&mut self, modules: u32, max_channels: u32) -> Result<(i32, bool), Abort> {
        let r = aac_enc_open(Some(&mut self.h), modules, max_channels);
        Ok((r, self.h.is_none()))
    }

    fn set_param(&mut self, param: i32, value: u32) -> Result<i32, Abort> {
        Ok(aac_encoder_set_param(self.h.as_deref_mut(), param, value))
    }

    fn get_param(&mut self, param: i32) -> Result<u32, Abort> {
        Ok(aac_encoder_get_param(self.h.as_deref(), param))
    }

    fn info(&mut self, info: &mut AacencInfoStruct) -> Result<i32, Abort> {
        Ok(aac_enc_info(self.h.as_deref_mut(), Some(info)))
    }

    unsafe fn encode(
        &mut self,
        inb: Option<&AacencBufDesc>,
        outb: Option<&AacencBufDesc>,
        ina: Option<&AacencInArgs>,
        outa: Option<&mut AacencOutArgs>,
    ) -> Result<i32, Abort> {
        // SAFETY: the caller's guarantee.
        Ok(unsafe { aac_enc_encode(self.h.as_deref_mut(), inb, outb, ina, outa) })
    }

    fn close(&mut self) -> Result<i32, Abort> {
        Ok(aac_enc_close(Some(&mut self.h)))
    }
}

struct Driver<'a, A: EncApi> {
    api: &'a mut A,
    p: &'a EncProgram,
    verbose: bool,
    trace: Trace<EncEv>,
    /// a channel mode was set successfully
    ch_set: bool,
    /// an encode call succeeded
    initialized: bool,
    /// the last initialization failed in the transport encoder (see the constraints)
    tp_failed: bool,
    /// the last sample rate set successfully (0: none)
    rate: u32,
    /// the last bandwidth set successfully
    bandwidth: u32,
    /// from the last aacEncInfo
    channels: usize,
    frame_length: usize,
    out_buffer_in_bytes: usize,
    encodes: usize,
    style: u8,
    tape_pos: usize,
    t: u64,
    anc_len: Option<usize>,
    meta: AacencMetaData,
    meta_on: bool,
    /// initialization and encode calls made so far (the index of `stop_at`)
    calls: usize,
    /// the initialization or encode call (index) not to make (see `deviation_stop`)
    stop_at: Option<usize>,
    /// the first initialization or encode call in which the port counted a deviation (see the
    /// constraints)
    deviation_at: Option<usize>,
}

impl<A: EncApi> Driver<'_, A> {
    fn push(&mut self, e: EncEv) {
        crate::echo(self.trace.events.len(), &e);
        self.trace.events.push(e);
    }

    /// aacEncInfo (once a channel mode is set) and GetParam of every parameter.
    fn check_state(&mut self) -> Result<(), Abort> {
        if self.ch_set {
            let mut info = AacencInfoStruct { conf_buf: [0x5a; 64], ..Default::default() };
            let ret = self.api.info(&mut info)?;
            self.channels = info.input_channels as usize;
            self.frame_length = info.frame_length as usize;
            // as aacEncOpen: the next power of two above nMaxAacChannels * 6144 / 8
            let m = info.max_out_buf_bytes.max(1);
            self.out_buffer_in_bytes = 1usize << (32 - m.leading_zeros());
            self.push(EncEv::Info { ret, info: info_bytes(&info) });
        }
        let mut values = Vec::with_capacity(GET_PARAMS.len());
        for p in GET_PARAMS {
            values.push(self.api.get_param(p)?);
        }
        self.push(EncEv::GetParams { values });
        Ok(())
    }

    fn set(&mut self, param: i32, value: u32) -> Result<i32, Abort> {
        let ret = self.api.set_param(param, value)?;
        self.push(EncEv::SetParam { param, value, ret });
        if ret == AACENC_OK {
            match param {
                // MODE_UNKNOWN (0) "succeeds" while it is unchanged, but does not count as a
                // channel mode (see the constraints)
                AACENC_CHANNELMODE => self.ch_set |= value != 0,
                AACENC_SAMPLERATE => self.rate = value,
                AACENC_BANDWIDTH => self.bandwidth = value,
                _ => {}
            }
        }
        self.check_state()?;
        Ok(ret)
    }

    fn note_result(&mut self, ret: i32) {
        if ret == AACENC_OK {
            self.initialized = true;
            self.tp_failed = false;
        } else if ret == AACENC_INIT_TP_ERROR {
            self.tp_failed = true;
        }
    }

    /// AACENC_CONTROL_STATE: after a failed transport initialization always with
    /// AACENC_INIT_TRANSPORT (see the constraints).
    fn control_state(&mut self, value: u32) -> Result<(), Abort> {
        let value = if self.tp_failed { value | AACENC_INIT_TRANSPORT } else { value };
        self.set(AACENC_CONTROL_STATE, value).map(|_| ())
    }

    /// Whether the initialization and encode calls may run: a channel mode and a sample rate
    /// were set (see the constraints).
    fn ready(&self) -> bool {
        self.ch_set && self.rate != 0
    }

    /// Keeps the bandwidth within fs/2 before an initialization (see the constraints).
    fn fix_bandwidth(&mut self) -> Result<(), Abort> {
        if self.bandwidth > self.rate.max(8000) / 2 {
            self.set(AACENC_BANDWIDTH, 0)?;
        }
        Ok(())
    }

    fn init(&mut self) -> Result<(), Abort> {
        if !self.ready() {
            return Ok(());
        }
        self.fix_bandwidth()?;
        let k = self.calls;
        self.calls += 1;
        if self.stop_at == Some(k) {
            return Err(Abort::Deviation(DEVIATION_STOP));
        }
        let before = deviation_events();
        // SAFETY: no buffers.
        let ret = unsafe { self.api.encode(None, None, None, None)? };
        // Only the port changes these counters.
        if self.deviation_at.is_none() && deviation_events() != before {
            self.deviation_at = Some(k);
        }
        self.push(EncEv::Init { ret });
        self.note_result(ret);
        self.check_state()
    }

    fn pcm(&mut self, n: usize) -> Vec<i16> {
        let tape = &self.p.tape;
        let style =
            if (self.style == 0 || self.style == 7) && tape.len() < 2 { 1 } else { self.style };
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let t = self.t;
            self.t += 1;
            let s = match style {
                0 | 7 => {
                    let k = self.tape_pos % (tape.len() - 1);
                    self.tape_pos += 2;
                    let x = i16::from_le_bytes([tape[k], tape[k + 1]]);
                    if style == 7 {
                        x >> 3
                    } else {
                        x
                    }
                }
                1 => {
                    ((((t * 37) % 400) as i32 - 200) * 60 + ((t * 7919 % 1023) as i32 - 512)) as i16
                }
                2 => {
                    let x = t.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    (x >> 48) as i16
                }
                3 => 0,
                4 => {
                    if (t / 64) % 2 == 0 {
                        32767
                    } else {
                        -32768
                    }
                }
                5 => {
                    if t % 997 == 0 {
                        32767
                    } else {
                        0
                    }
                }
                _ => 16384,
            };
            v.push(s);
        }
        v
    }

    fn encode(&mut self, call: &EncCall) -> Result<i32, Abort> {
        if !self.ready() || self.encodes >= MAX_ENCODES {
            return Ok(AACENC_ENCODE_ERROR);
        }
        self.fix_bandwidth()?;
        let n = self.encodes;
        self.encodes += 1;
        let k = self.calls;
        self.calls += 1;
        if self.stop_at == Some(k) {
            return Err(Abort::Deviation(DEVIATION_STOP));
        }
        let mut pcm16 = call.pcm.clone();
        let mut pcm32: Vec<i32> = call
            .pcm
            .iter()
            .enumerate()
            .map(|(i, &s)| ((s as i32) << 16) | ((i as i32 * 7919) & 0xffff))
            .collect();
        let mut anc = call.anc.clone().unwrap_or_default();
        let mut meta = call.meta.unwrap_or_default();
        let mut dummy = [0u8; 16];
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
        let num_anc_bytes = call.anc.as_ref().map_or(0, |a| a.len() as i32);
        let in_args = AacencInArgs { num_in_samples: call.num_in_samples, num_anc_bytes };
        let mut out_args = AacencOutArgs {
            num_out_bytes: -7,
            num_in_samples: -7,
            num_anc_bytes: -7,
            bit_res_state: -7,
        };
        let in_desc_p = if call.mode == 1 { None } else { Some(&in_desc) };
        let out_args_p = if call.mode == 2 { None } else { Some(&mut out_args) };
        let before = deviation_events();
        // SAFETY: the descriptors point to live buffers of the announced sizes; the structures
        // have the C layout.
        let ret =
            unsafe { self.api.encode(in_desc_p, Some(&out_desc), Some(&in_args), out_args_p)? };
        // Only the port changes these counters.
        if self.deviation_at.is_none() && deviation_events() != before {
            self.deviation_at = Some(k);
        }
        // Bytes beyond the internal output buffer are not compared (see the constraints).
        let nob = out_args.num_out_bytes;
        if nob > 0 && nob as usize > self.out_buffer_in_bytes {
            let a = self.out_buffer_in_bytes.min(outbuf.len());
            let b = (nob as usize).min(outbuf.len());
            outbuf[a..b].fill(0);
        }
        let ev = EncEv::Encode {
            n,
            num_in_samples: call.num_in_samples,
            el_size: call.el_size,
            anc: call.anc.as_ref().map_or(-1, |a| a.len() as i32),
            meta: call.meta.is_some(),
            out_size: call.out_size,
            mode: call.mode,
            ret,
            out_args: [
                out_args.num_out_bytes,
                out_args.num_in_samples,
                out_args.num_anc_bytes,
                out_args.bit_res_state,
            ],
            out: Blob::new(&outbuf, self.verbose),
            au_sizes: au,
        };
        self.push(ev);
        self.note_result(ret);
        self.check_state()?;
        Ok(ret)
    }

    /// A standard call: `per_channel` samples per channel (FRAME, FLUSH or a count).
    fn call(&mut self, per_channel: i32, extras: u8, extra2: u8) -> Result<i32, Abort> {
        if !self.ready() {
            return Ok(AACENC_ENCODE_ERROR);
        }
        let per = match per_channel {
            FRAME => self.frame_length.max(1) as i32,
            x => x,
        };
        let (pcm, num_in_samples) = if per == FLUSH {
            (Vec::new(), -1)
        } else {
            let n = (per as usize).min(MAX_PER_CHANNEL) * self.channels.max(1);
            (self.pcm(n), n as i32)
        };
        let call = EncCall {
            pcm,
            num_in_samples,
            el_size: [2, 2, 4, 1][(extras & 3) as usize],
            anc: (extras & 4 != 0 || self.anc_len.is_some()).then(|| {
                let n = self.anc_len.unwrap_or(extra2 as usize);
                (0..n).map(|i| (i as u8).wrapping_mul(37) ^ extra2).collect()
            }),
            meta: (extras & 8 != 0 || self.meta_on).then_some(self.meta),
            out_size: if extras & 16 != 0 { OUT_SMALL[(extra2 & 7) as usize] } else { DEFAULT_OUT },
            au_sizes: extras & 32 != 0,
            mode: if extras >> 6 == 3 { 1 + (extra2 >> 3) as i32 % 3 } else { 0 },
        };
        self.encode(&call)
    }

    fn run(&mut self) -> Result<(), Abort> {
        let (ret, null) = self.api.open(self.p.modules, self.p.max_channels)?;
        self.push(EncEv::Open {
            modules: self.p.modules,
            max_channels: self.p.max_channels,
            ret,
            null,
        });
        if ret != AACENC_OK || null {
            return Ok(());
        }
        self.check_state()?;
        let ops = self.p.ops.clone();
        let mut k = 0usize;
        while k < ops.len() && self.encodes < MAX_ENCODES {
            let op = ops[k];
            k += 1;
            let mut next = || {
                let b = ops.get(k).copied().unwrap_or(0);
                k += 1;
                b
            };
            let arg = op & 31;
            match op >> 5 {
                0 => {
                    let param = ENC_PARAMS[arg as usize];
                    let b = next();
                    let value = param_value(param, b, self.rate.max(8000), &mut next);
                    if param == AACENC_CONTROL_STATE {
                        if self.initialized {
                            self.control_state(CONTROL[b as usize % CONTROL.len()])?;
                        }
                    } else if param == AACENC_CHANNELMODE
                        && value == MODE_212 as u32
                        && self.p.max_channels & 0xff == 1
                    {
                        // skipped (see the constraints)
                    } else {
                        self.set(param, value)?;
                    }
                }
                1 => {
                    let (e1, e2) = (next(), next());
                    self.call(CHUNKS[arg as usize], e1, e2)?;
                }
                2 => self.init()?,
                3 => {
                    self.style = arg & 7;
                    if arg & 8 != 0 {
                        self.tape_pos = 0;
                    }
                }
                4 => {
                    if self.initialized {
                        self.control_state(CONTROL[arg as usize % CONTROL.len()])?;
                    }
                }
                5 => {
                    for _ in 0..=(arg & 15) {
                        self.call(FRAME, 0, 0)?;
                    }
                }
                6 => {
                    let b = [next(), next(), next(), next(), next(), next(), next(), next()];
                    self.meta = meta_from(&b);
                    self.meta_on = arg & 1 != 0;
                }
                _ => {
                    let n = next() as usize;
                    self.anc_len = if arg & 1 != 0 { None } else { Some(n) };
                }
            }
        }
        // Frame-sized calls, then flush to the end.
        while self.ready() && self.encodes < DEFAULT_ENCODES {
            if self.call(FRAME, 0, 0)? != AACENC_OK {
                break;
            }
        }
        for _ in 0..8 {
            if !self.ready() || self.call(FLUSH, 0, 0)? != AACENC_OK {
                break;
            }
        }
        Ok(())
    }
}

fn run_api<A: EncApi>(
    api: &mut A,
    p: &EncProgram,
    verbose: bool,
    stop_at: Option<usize>,
) -> (Trace<EncEv>, Option<usize>) {
    let mut d = Driver {
        api,
        p,
        verbose,
        trace: Trace::default(),
        ch_set: false,
        initialized: false,
        tp_failed: false,
        rate: 0,
        bandwidth: 0,
        channels: 0,
        frame_length: 0,
        out_buffer_in_bytes: usize::MAX,
        encodes: 0,
        calls: 0,
        style: 0,
        tape_pos: 0,
        t: 0,
        anc_len: None,
        meta: AacencMetaData::default(),
        meta_on: false,
        stop_at,
        deviation_at: None,
    };
    let r = d.run();
    let mut trace = std::mem::take(&mut d.trace);
    match r {
        Ok(()) => match d.api.close() {
            Ok(ret) => trace.events.push(EncEv::Close { ret }),
            // an aborted Close (the handle leaks)
            Err(a) => trace.abort = Some(a),
        },
        Err(a) => {
            trace.abort = Some(a);
            // Close frees the memory (an aborted Close is caught as well; the handle then leaks).
            let _ = d.api.close();
        }
    }
    (trace, d.deviation_at)
}

/// Why an initialization or encode call is not made (see the constraints).
const DEVIATION_STOP: &str = "the next call has no defined reference result";

/// The port's counters of calls without a defined reference result (see the constraints).
fn deviation_events() -> (u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) {
    let (w, _, n) = mdct_block_dropped_accesses();
    (
        w,
        n,
        psy_main_negative_lowpass_clears(),
        metadata_input_overruns(),
        psy_main_stale_short_blocks(),
        psy_main_stale_element_mappings(),
        psy_main_mixed_block_pairs(),
        psy_main_undefined_spectrum_reads(),
        psy_main_lfe_flag_pairs(),
        adj_thr_stale_ah_flag_reads(),
    )
}

/// The first initialization or encode call (index) in which the port counts a deviation
/// (`deviation_events`). Both runs stop before that call (see the constraints).
pub fn deviation_stop(p: &EncProgram) -> Option<usize> {
    run_api(&mut PortEnc { h: None }, p, false, None).1
}

pub fn run_ref(p: &EncProgram, pattern: i32, verbose: bool, stop: Option<usize>) -> Trace<EncEv> {
    let mut api = RefEnc { s: Session::begin(pattern), h: core::ptr::null_mut() };
    run_api(&mut api, p, verbose, stop).0
}

pub fn run_port(p: &EncProgram, verbose: bool, stop: Option<usize>) -> Trace<EncEv> {
    run_api(&mut PortEnc { h: None }, p, verbose, stop).0
}

fn detail(a: &EncEv, b: &EncEv) -> String {
    match (a, b) {
        (EncEv::Encode { out: x, .. }, EncEv::Encode { out: y, .. }) => {
            crate::diff_bytes("output bytes", x, y, 1)
        }
        (EncEv::GetParams { values: x }, EncEv::GetParams { values: y }) => {
            let d: Vec<String> = (0..x.len())
                .filter(|&k| x[k] != y[k])
                .map(|k| format!("param {:#x}: {:#x} vs {:#x}", GET_PARAMS[k], x[k], y[k]))
                .collect();
            d.join(", ")
        }
        _ => String::new(),
    }
}

/// `aacEncGetLibInfo` of both.
pub fn check_lib_info() -> Result<(), String> {
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
    let m: Vec<&LibInfo> = info.iter().take_while(|e| e.module_id != FDK_NONE).collect();
    if er != ec || m.len() != n as usize {
        return Err(format!("encoder: ret/count {ec}/{n} vs {er}/{}", m.len()));
    }
    for (i, e) in m.iter().enumerate() {
        let t: Vec<u8> = titles[64 * i..64 * i + 64].iter().map(|&b| b as u8).collect();
        let tl = t.iter().position(|&b| b == 0).unwrap_or(64);
        let v: Vec<u8> = vs[32 * i..32 * i + 32].iter().map(|&b| b as u8).collect();
        if e.module_id != ids[i]
            || e.version != ver[i]
            || e.flags != flags[i]
            || e.title.as_bytes() != &t[..tl]
            || e.version_str.to_vec() != v
        {
            return Err(format!("encoder entry {i}: {e:?}"));
        }
    }
    Ok(())
}

pub fn check(data: &[u8]) -> Outcome {
    crate::lib_info_once();
    let p = EncProgram::parse(data);
    let describe = format!(
        "aacEncOpen({:#x}, {:#x}), {} op bytes, {} PCM tape bytes",
        p.modules,
        p.max_channels,
        p.ops.len(),
        p.tape.len()
    );
    // The port runs first: it decides where the runs stop (see the constraints); without a
    // stop, its trace is the port's (non-verbose) trace of the comparison.
    let (probe, stop) = run_api(&mut PortEnc { h: None }, &p, false, None);
    let mut first = if stop.is_none() { Some(probe) } else { None };
    crate::compare(
        "encode",
        &describe,
        |pat, v| run_ref(&p, pat, v, stop),
        |v| match first.take() {
            Some(t) if !v => t,
            _ => run_port(&p, v, stop),
        },
        detail,
    )
}

pub fn fuzz(data: &[u8]) {
    crate::handle("encode", check(data));
}
