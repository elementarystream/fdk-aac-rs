//! Decoder targets: one call program, run on the reference and on the port.
//!
//! # Input format
//!
//! ```text
//! byte 0    flags: bit 0 packet mode (else stream mode), bit 1 AncDataInit, bit 2 decode once
//!           per step (else decode until AAC_DEC_NOT_ENOUGH_BITS), bit 3 drain with
//!           AACDEC_FLUSH at the end, bits 4-6 initial output buffer size (HDR_OUT_SIZES),
//!           bit 7 decode_raw: LATM MCP0 instead of raw
//! byte 1    number of op bytes L
//! byte 2    bits 0-3 ancillary buffer size (ANC_SIZES), bits 4-7 initial fill size
//!           (FILL_SIZES[16 + n])
//! L bytes   ops (see `Op` below), executed in order; then default steps until the data is used
//! decode_raw, decode_drm only: 1 byte length C, C bytes of configuration for ConfigRaw
//!           (AudioSpecificConfig, StreamMuxConfig or DRM SDC audio entity; C = 0: none)
//! rest      the bitstream: stream mode: bytes, fed in fill-size pieces; packet mode: access
//!           units, each a 16-bit little-endian length and the bytes
//! ```
//!
//! # Ops
//!
//! `op >> 5` selects the kind, `op & 31` is its argument:
//!
//! | kind | effect |
//! |---|---|
//! | 0 | `1 + (arg & 7)` steps; `arg >> 3`: 0 the current decode mode, 1 one decode, 2 decode loop, 3 fill only |
//! | 1 | fill size = `FILL_SIZES[arg]` |
//! | 2 | flags `arg & 15` (CONCEAL 1, FLUSH 2, INTR 4, CLRHIST 8) for the next decode call; with `arg & 16` for all following calls |
//! | 3 | SetParam(`DEC_PARAMS[arg]`, `param_value(next op byte)`) |
//! | 4 | DecodeFrame without fill, flags `arg & 15`; with `arg & 16` a decode loop |
//! | 5 | output buffer size = `OUT_SIZES[arg]` |
//! | 6 | ConfigRaw: `arg == 0`: the configuration of the input (if any), else the next `arg` op bytes |
//! | 7 | `arg` 0-7: AncDataInit(`ANC_SIZES[next op byte & 15]`); 8: GetFreeBytes; 9: Fill with 0 bytes; 10: drop the rest of the current access unit (packet mode) or `next op byte` bytes (stream mode); 11: toggle decode once/loop; otherwise one step |
//!
//! A step: GetFreeBytes, Fill with the next piece (stream mode: up to the fill size; packet mode:
//! the rest of the current access unit), then the decode calls of the decode mode.
//!
//! After every DecodeFrame call the trace holds the return value, the whole output buffer (also
//! the part beyond the valid samples), the complete CStreamInfo and, with an ancillary buffer,
//! every AncDataGet result and the whole ancillary buffer (PORTING.md, "Definition of
//! bit-exact").

use crate::guard::{Abort, Session};
use crate::{Blob, Outcome, Reader, Trace};
use fdk_aac_rs_aacdec::aacdecoder::AacDecoderInstance;
use fdk_aac_rs_aacdec::aacdecoder_lib::*;
use fdk_aac_rs_cref::aacdec_aacdecoder_lib as c;
use fdk_aac_rs_fdk::audio::*;

/// The decoder targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecTarget {
    Adts,
    Adif,
    Latm,
    Loas,
    Raw,
    Drm,
}

impl DecTarget {
    pub fn name(self) -> &'static str {
        match self {
            DecTarget::Adts => "decode_adts",
            DecTarget::Adif => "decode_adif",
            DecTarget::Latm => "decode_latm",
            DecTarget::Loas => "decode_loas",
            DecTarget::Raw => "decode_raw",
            DecTarget::Drm => "decode_drm",
        }
    }

    pub fn from_name(s: &str) -> Option<DecTarget> {
        [
            DecTarget::Adts,
            DecTarget::Adif,
            DecTarget::Latm,
            DecTarget::Loas,
            DecTarget::Raw,
            DecTarget::Drm,
        ]
        .into_iter()
        .find(|t| t.name() == s)
    }

    /// Whether the input carries a configuration for ConfigRaw.
    pub fn has_config(self) -> bool {
        matches!(self, DecTarget::Raw | DecTarget::Drm)
    }
}

pub const HDR_OUT_SIZES: [i32; 8] = [65536, 65536, 32768, 8192, 4096, 2048, 1024, 128];
pub const OUT_SIZES: [i32; 32] = [
    65536, 65536, 49152, 32768, 24576, 18432, 16384, 12288, 9216, 8192, 6144, 4800, 4096, 3072,
    2048, 1920, 1536, 1024, 960, 768, 512, 480, 256, 240, 128, 120, 64, 16, 3, 2, 1, 0,
];
pub const FILL_SIZES: [u32; 32] = [
    1, 2, 3, 4, 5, 7, 8, 13, 16, 31, 32, 63, 64, 100, 127, 128, //
    4096, 256, 300, 384, 500, 512, 768, 1000, 1024, 1500, 2048, 3000, 6144, 8192, 16384, 65536,
];
pub const ANC_SIZES: [i32; 16] =
    [0, 1, 2, 3, 4, 7, 8, 16, 31, 64, 100, 128, 256, 1024, 4096, 65536];
/// `AACDEC_PARAM` values, and unknown ones.
pub const DEC_PARAMS: [i32; 32] = [
    0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0011, 0x0012, 0x0020, 0x0021, 0x0100, 0x0200, 0x0201,
    0x0202, 0x0203, 0x0204, 0x0205, 0x0206, 0x0207, 0x0300, 0x0603, // the API
    0x0004, 0x0012, 0x0200, 0x0202, 0x0206, 0x0100, // again (more weight)
    0x0000, 0x0001, 0x0013, 0x0101, 0x0301, 0x0602,
];

/// A SetParam value from one byte: small values around 0, then edge values.
pub fn param_value(b: u8) -> i32 {
    const EDGE: [i32; 32] = [
        127,
        128,
        255,
        256,
        511,
        512,
        1000,
        1023,
        1024,
        2000,
        4096,
        8191,
        32767,
        32768,
        65535,
        65536,
        0x7FFF_FFFF,
        i32::MIN,
        -1,
        -2,
        -100,
        -128,
        -129,
        -256,
        -32768,
        0x4000_0000,
        1 << 20,
        96,
        100,
        120,
        124,
        200,
    ];
    match b {
        0..=0x5F => b as i32 - 16,
        0x60..=0x7F => EDGE[(b - 0x60) as usize],
        _ => (b as i32 - 0x80) * 4,
    }
}

const MAX_DECODES: usize = 128;
const LOOP_MAX: usize = 16;
const MAX_STEPS: usize = 4096;

/// The program of one input.
#[derive(Clone, Debug)]
pub struct DecProgram {
    pub transport: i32,
    pub packet: bool,
    pub anc_size: Option<i32>,
    pub decode_once: bool,
    pub drain: bool,
    pub out_size: i32,
    pub fill_size: u32,
    pub ops: Vec<u8>,
    pub config: Option<Vec<u8>>,
    /// Stream mode: one entry; packet mode: the access units.
    pub chunks: Vec<Vec<u8>>,
}

impl DecProgram {
    pub fn parse(t: DecTarget, data: &[u8]) -> DecProgram {
        let mut r = Reader::new(data);
        let flags = r.u8();
        let nops = r.u8() as usize;
        let aux = r.u8();
        let ops = r.bytes(nops).to_vec();
        let transport = match t {
            DecTarget::Adts => TT_MP4_ADTS,
            DecTarget::Adif => TT_MP4_ADIF,
            DecTarget::Latm => TT_MP4_LATM_MCP1,
            DecTarget::Loas => TT_MP4_LOAS,
            DecTarget::Raw if flags & 0x80 != 0 => TT_MP4_LATM_MCP0,
            DecTarget::Raw => TT_MP4_RAW,
            DecTarget::Drm => TT_DRM,
        };
        let config = if t.has_config() {
            let n = r.u8() as usize;
            Some(r.bytes(n).to_vec())
        } else {
            None
        };
        let packet = flags & 1 != 0;
        let rest = r.rest();
        let chunks = if packet {
            let mut r = Reader::new(rest);
            let mut v = Vec::new();
            while r.pos < rest.len() {
                let n = r.u16() as usize;
                v.push(r.bytes(n).to_vec());
            }
            v
        } else {
            vec![rest.to_vec()]
        };
        DecProgram {
            transport,
            packet,
            anc_size: (flags & 2 != 0).then(|| ANC_SIZES[(aux & 15) as usize]),
            decode_once: flags & 4 != 0,
            drain: flags & 8 != 0,
            out_size: HDR_OUT_SIZES[((flags >> 4) & 7) as usize],
            fill_size: FILL_SIZES[16 + (aux >> 4) as usize],
            ops,
            config,
            chunks,
        }
    }

    /// Serializes a program (seed generation). `ops` and the configuration are limited to 255
    /// bytes, access units to 65535 bytes.
    pub fn to_bytes(&self, t: DecTarget, flags: u8, aux: u8) -> Vec<u8> {
        let mut v = vec![flags, self.ops.len().min(255) as u8, aux];
        v.extend_from_slice(&self.ops[..self.ops.len().min(255)]);
        if t.has_config() {
            let c = self.config.as_deref().unwrap_or(&[]);
            let n = c.len().min(255);
            v.push(n as u8);
            v.extend_from_slice(&c[..n]);
        }
        if flags & 1 != 0 {
            for au in &self.chunks {
                let n = au.len().min(65535);
                v.extend_from_slice(&(n as u16).to_le_bytes());
                v.extend_from_slice(&au[..n]);
            }
        } else {
            for ch in &self.chunks {
                v.extend_from_slice(ch);
            }
        }
        v
    }
}

/// One call and what it returned.
#[derive(Clone, Debug, PartialEq)]
pub enum DecEv {
    Open { transport: i32, ok: bool },
    ConfigRaw { len: usize, ret: i32, info: Option<Vec<i32>> },
    SetParam { param: i32, value: i32, ret: i32 },
    GetFreeBytes { ret: i32, free: u32 },
    Fill { size: usize, ret: i32, bytes_valid: u32 },
    AncDataInit { size: i32, ret: i32 },
    DecodeFrame { n: usize, flags: u32, size: i32, ret: i32, out: Blob, info: Option<Vec<i32>> },
    AncDataGet { index: i32, ret: i32, offset: i32, size: i32, data: Blob },
    AncBuffer { data: Blob },
}

/// The decoder API as both sides offer it.
trait DecApi {
    fn open(&mut self, tt: i32) -> Result<bool, Abort>;
    fn config_raw(&mut self, conf: &[u8]) -> Result<i32, Abort>;
    fn set_param(&mut self, param: i32, value: i32) -> Result<i32, Abort>;
    fn free_bytes(&mut self) -> Result<(i32, u32), Abort>;
    fn fill(&mut self, data: &[u8]) -> Result<(i32, u32), Abort>;
    fn anc_init(&mut self, size: i32) -> Result<i32, Abort>;
    fn decode(&mut self, out: &mut [i16], flags: u32) -> Result<i32, Abort>;
    fn stream_info(&mut self) -> Result<Option<Vec<i32>>, Abort>;
    /// (ret, offset in the ancillary buffer or -1, size)
    fn anc_get(&mut self, index: i32) -> Result<(i32, i32, i32), Abort>;
    fn anc_buf(&self) -> &[u8];
    fn close(&mut self);
}

/// The reference, through the shims of `crates/cref/shim/aacdec_aacdecoder_lib.cpp`.
struct RefDec {
    s: Session,
    h: *mut core::ffi::c_void,
    anc: Vec<u8>,
    /// Earlier ancillary buffers (kept alive while the handle exists).
    old_anc: Vec<Vec<u8>>,
}

impl DecApi for RefDec {
    fn open(&mut self, tt: i32) -> Result<bool, Abort> {
        let mut h = core::ptr::null_mut();
        // SAFETY: shim call.
        self.s.call(|| h = unsafe { c::cref_aacdecoder_lib_open(tt, 1) })?;
        self.h = h;
        Ok(!h.is_null())
    }

    fn config_raw(&mut self, conf: &[u8]) -> Result<i32, Abort> {
        let mut buf = conf.to_vec();
        let (h, p, n) = (self.h, buf.as_mut_ptr(), buf.len() as u32);
        let mut r = 0;
        // SAFETY: valid handle, buffer of n bytes.
        self.s.call(|| r = unsafe { c::cref_aacdecoder_lib_config_raw(h, p, n) })?;
        Ok(r)
    }

    fn set_param(&mut self, param: i32, value: i32) -> Result<i32, Abort> {
        let h = self.h;
        let mut r = 0;
        // SAFETY: valid handle.
        self.s.call(|| r = unsafe { c::cref_aacdecoder_lib_set_param(h, param, value) })?;
        Ok(r)
    }

    fn free_bytes(&mut self) -> Result<(i32, u32), Abort> {
        let h = self.h;
        let (mut r, mut f) = (0, 0u32);
        let fp = &mut f as *mut u32;
        // SAFETY: valid handle and pointer.
        self.s.call(|| r = unsafe { c::cref_aacdecoder_lib_free_bytes(h, fp) })?;
        Ok((r, f))
    }

    fn fill(&mut self, data: &[u8]) -> Result<(i32, u32), Abort> {
        let mut buf = data.to_vec();
        let (h, p, n) = (self.h, buf.as_mut_ptr(), buf.len() as u32);
        let mut valid = n;
        let vp = &mut valid as *mut u32;
        let mut r = 0;
        // SAFETY: valid handle, buffer of n bytes.
        self.s.call(|| r = unsafe { c::cref_aacdecoder_lib_fill(h, p, n, vp) })?;
        Ok((r, valid))
    }

    fn anc_init(&mut self, size: i32) -> Result<i32, Abort> {
        let old = std::mem::replace(&mut self.anc, vec![0u8; size.max(0) as usize]);
        self.old_anc.push(old);
        let (h, p) = (self.h, self.anc.as_mut_ptr());
        let mut r = 0;
        // SAFETY: valid handle, buffer of `size` bytes, alive as long as the handle.
        self.s.call(|| r = unsafe { c::cref_aacdecoder_lib_anc_init(h, p, size) })?;
        Ok(r)
    }

    fn decode(&mut self, out: &mut [i16], flags: u32) -> Result<i32, Abort> {
        let (h, p, n) = (self.h, out.as_mut_ptr(), out.len() as i32);
        let mut r = 0;
        // SAFETY: valid handle, buffer of n samples.
        self.s.call(|| r = unsafe { c::cref_aacdecoder_lib_decode(h, p, n, flags) })?;
        Ok(r)
    }

    fn stream_info(&mut self) -> Result<Option<Vec<i32>>, Abort> {
        let mut o = [0i32; 64];
        let (h, p) = (self.h, o.as_mut_ptr());
        let mut n = 0;
        // SAFETY: valid handle, 64 ints.
        self.s.call(|| n = unsafe { c::cref_aacdecoder_lib_stream_info(h, p) })?;
        Ok((n > 0).then(|| o[..n as usize].to_vec()))
    }

    fn anc_get(&mut self, index: i32) -> Result<(i32, i32, i32), Abort> {
        let (h, base) = (self.h, self.anc.as_mut_ptr());
        let (mut r, mut off, mut size) = (0, 0, 0);
        let (op, sp) = (&mut off as *mut i32, &mut size as *mut i32);
        // SAFETY: valid handle and pointers.
        self.s.call(|| r = unsafe { c::cref_aacdecoder_lib_anc_get(h, index, base, op, sp) })?;
        Ok((r, off, size))
    }

    fn anc_buf(&self) -> &[u8] {
        &self.anc
    }

    fn close(&mut self) {
        if !self.h.is_null() {
            let h = self.h;
            // SAFETY: valid handle, closed once.
            let _ = self.s.call_unbudgeted(|| unsafe { c::cref_aacdecoder_lib_close(h) });
            self.h = core::ptr::null_mut();
        }
    }
}

/// The port (`fdk_aac_rs_aacdec::aacdecoder_lib`).
struct PortDec {
    h: Option<Box<AacDecoderInstance>>,
}

impl PortDec {
    fn h(&mut self) -> &mut AacDecoderInstance {
        self.h.as_deref_mut().expect("port handle")
    }
}

/// The port's CStreamInfo in the order of `cref_aacdecoder_lib_stream_info`.
pub fn si_port(si: &CStreamInfo) -> Vec<i32> {
    let mut o = vec![si.sample_rate, si.frame_size, si.num_channels];
    o.extend(si.p_channel_type.iter().copied());
    o.extend(si.p_channel_indices.iter().map(|&x| x as i32));
    o.extend([
        si.aac_sample_rate,
        si.profile,
        si.aot,
        si.channel_config,
        si.bit_rate,
        si.aac_samples_per_frame,
        si.aac_num_channels,
        si.ext_aot,
        si.ext_sampling_rate,
        si.output_delay as i32,
        si.flags as i32,
        si.ep_config as i32,
        si.num_lost_access_units,
    ]);
    for v in
        [si.num_total_bytes, si.num_bad_bytes, si.num_total_access_units, si.num_bad_access_units]
    {
        o.push(v as i32);
        o.push((v >> 32) as i32);
    }
    o.extend([si.drc_prog_ref_lev as i32, si.drc_pres_mode as i32, si.output_loudness]);
    o
}

impl DecApi for PortDec {
    fn open(&mut self, tt: i32) -> Result<bool, Abort> {
        self.h = aac_decoder_open(tt, 1);
        Ok(self.h.is_some())
    }

    fn config_raw(&mut self, conf: &[u8]) -> Result<i32, Abort> {
        Ok(aac_decoder_config_raw(self.h(), &[conf], &[conf.len() as u32]))
    }

    fn set_param(&mut self, param: i32, value: i32) -> Result<i32, Abort> {
        Ok(aac_decoder_set_param(self.h.as_deref_mut(), param, value))
    }

    fn free_bytes(&mut self) -> Result<(i32, u32), Abort> {
        let mut f = 0;
        let r = aac_decoder_get_free_bytes(self.h.as_deref_mut(), &mut f);
        Ok((r, f))
    }

    fn fill(&mut self, data: &[u8]) -> Result<(i32, u32), Abort> {
        let mut valid = [data.len() as u32];
        let r = aac_decoder_fill(self.h(), &[data], &[data.len() as u32], &mut valid);
        Ok((r, valid[0]))
    }

    fn anc_init(&mut self, size: i32) -> Result<i32, Abort> {
        let buf = vec![0u8; size.max(0) as usize].into_boxed_slice();
        Ok(aac_decoder_anc_data_init(self.h(), Some(buf), size))
    }

    fn decode(&mut self, out: &mut [i16], flags: u32) -> Result<i32, Abort> {
        let n = out.len() as i32;
        Ok(aac_decoder_decode_frame(self.h.as_deref_mut(), out, n, flags))
    }

    fn stream_info(&mut self) -> Result<Option<Vec<i32>>, Abort> {
        Ok(aac_decoder_get_stream_info(self.h.as_deref()).map(si_port))
    }

    fn anc_get(&mut self, index: i32) -> Result<(i32, i32, i32), Abort> {
        let (mut off, mut size) = (None, 0);
        let r = aac_decoder_anc_data_get(self.h.as_deref().unwrap(), index, &mut off, &mut size);
        Ok((r, off.map_or(-1, |o| o as i32), size))
    }

    fn anc_buf(&self) -> &[u8] {
        self.h.as_deref().and_then(aac_decoder_anc_data_buffer).unwrap_or(&[])
    }

    fn close(&mut self) {
        aac_decoder_close(self.h.take());
    }
}

/// The output buffer pattern before each DecodeFrame call (as `fdkdec-ref --poison-output`),
/// for the largest buffer.
fn out_pattern() -> &'static [i16] {
    static P: std::sync::OnceLock<Vec<i16>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        (0..OUT_SIZES[0] as u32)
            .map(|i| (i.wrapping_mul(40503).wrapping_add(0x6A5D) | 1) as i16)
            .collect()
    })
}

struct Driver<'a, A: DecApi> {
    api: &'a mut A,
    p: &'a DecProgram,
    verbose: bool,
    trace: Trace<DecEv>,
    out: Vec<i16>,
    out_size: i32,
    fill_size: u32,
    decode_once: bool,
    next_flags: u32,
    sticky_flags: u32,
    anc: bool,
    decodes: usize,
    /// current chunk and offset in it
    ci: usize,
    co: usize,
    stalls: usize,
}

impl<A: DecApi> Driver<'_, A> {
    fn push(&mut self, e: DecEv) {
        crate::echo(self.trace.events.len(), &e);
        self.trace.events.push(e);
    }

    fn data_left(&self) -> bool {
        self.ci < self.p.chunks.len() && self.co < self.p.chunks[self.ci].len()
            || self.p.packet && self.ci < self.p.chunks.len()
    }

    fn config(&mut self, conf: &[u8]) -> Result<(), Abort> {
        let ret = self.api.config_raw(conf)?;
        let info = self.api.stream_info()?;
        self.push(DecEv::ConfigRaw { len: conf.len(), ret, info });
        Ok(())
    }

    fn anc_init(&mut self, size: i32) -> Result<(), Abort> {
        let ret = self.api.anc_init(size)?;
        self.anc = ret == 0;
        self.push(DecEv::AncDataInit { size, ret });
        Ok(())
    }

    fn fill(&mut self, data: &[u8]) -> Result<u32, Abort> {
        let (ret, free) = self.api.free_bytes()?;
        self.push(DecEv::GetFreeBytes { ret, free });
        let (ret, bytes_valid) = self.api.fill(data)?;
        self.push(DecEv::Fill { size: data.len(), ret, bytes_valid });
        Ok((data.len() as u32).saturating_sub(bytes_valid))
    }

    fn decode(&mut self, flags: u32) -> Result<i32, Abort> {
        let n = self.out_size.max(0) as usize;
        if self.out.len() < n {
            self.out.resize(n, 0);
        }
        self.out[..n].copy_from_slice(&out_pattern()[..n]);
        let ret = self.api.decode(&mut self.out[..n], flags)?;
        let info = self.api.stream_info()?;
        let out = Blob::from_i16(&self.out[..n], self.verbose);
        self.push(DecEv::DecodeFrame {
            n: self.decodes,
            flags,
            size: self.out_size,
            ret,
            out,
            info,
        });
        self.decodes += 1;
        if self.anc {
            for index in 0..18 {
                let (ret, offset, size) = self.api.anc_get(index)?;
                let buf = self.api.anc_buf();
                let data = if offset >= 0 && size > 0 {
                    let a = (offset as usize).min(buf.len());
                    let b = (offset as usize).saturating_add(size as usize).min(buf.len());
                    Blob::new(&buf[a..b], self.verbose)
                } else {
                    Blob::new(&[], false)
                };
                self.push(DecEv::AncDataGet { index, ret, offset, size, data });
                if ret != 0 || size <= 0 {
                    break;
                }
            }
            let data = Blob::new(self.api.anc_buf(), self.verbose);
            self.push(DecEv::AncBuffer { data });
        }
        Ok(ret)
    }

    fn take_flags(&mut self) -> u32 {
        let f = self.next_flags | self.sticky_flags;
        self.next_flags = 0;
        f
    }

    /// One decode (`lp == false`) or a decode loop.
    fn decode_mode(&mut self, lp: bool) -> Result<(), Abort> {
        let n = if lp { LOOP_MAX } else { 1 };
        for _ in 0..n {
            if self.decodes >= MAX_DECODES {
                break;
            }
            let f = self.take_flags();
            let ret = self.decode(f)?;
            // The loop ends at AAC_DEC_NOT_ENOUGH_BITS (as the harness's decode loop), and also
            // at errors that further calls without new input repeat (initialization, API and
            // ancillary errors): only output (decode errors included) and sync errors go on.
            if !(ret == AAC_DEC_OK
                || ret == AAC_DEC_TRANSPORT_SYNC_ERROR
                || (0x4000..=0x4FFF).contains(&ret))
            {
                break;
            }
        }
        Ok(())
    }

    /// Fill with the next piece of the input (if any).
    fn fill_next(&mut self) -> Result<(), Abort> {
        if !self.data_left() {
            return Ok(());
        }
        let chunk = &self.p.chunks[self.ci];
        let end = if self.p.packet {
            chunk.len()
        } else {
            (self.co + self.fill_size as usize).min(chunk.len())
        };
        let piece = chunk[self.co..end].to_vec();
        let used = self.fill(&piece)? as usize;
        self.co += used;
        if used == 0 && !piece.is_empty() {
            self.stalls += 1;
        } else {
            self.stalls = 0;
        }
        if self.co >= chunk.len() {
            self.ci += 1;
            self.co = 0;
        }
        Ok(())
    }

    /// style: 0 current mode, 1 once, 2 loop, 3 fill only
    fn step(&mut self, style: u8) -> Result<(), Abort> {
        self.fill_next()?;
        match style {
            1 => self.decode_mode(false),
            2 => self.decode_mode(true),
            3 => Ok(()),
            _ => self.decode_mode(!self.decode_once),
        }
    }

    fn run(&mut self) -> Result<(), Abort> {
        let ok = self.api.open(self.p.transport)?;
        self.push(DecEv::Open { transport: self.p.transport, ok });
        if !ok {
            return Ok(());
        }
        if let Some(c) = self.p.config.clone() {
            if !c.is_empty() {
                self.config(&c)?;
            }
        }
        if let Some(size) = self.p.anc_size {
            self.anc_init(size)?;
        }
        let ops = self.p.ops.clone();
        let mut k = 0usize;
        let mut steps = 0usize;
        while steps < MAX_STEPS && self.decodes < MAX_DECODES && self.stalls < 2 {
            steps += 1;
            let Some(&op) = ops.get(k) else {
                if !self.data_left() {
                    break;
                }
                self.step(0)?;
                continue;
            };
            k += 1;
            let mut next = || {
                let b = ops.get(k).copied().unwrap_or(0);
                k += 1;
                b
            };
            let arg = op & 31;
            match op >> 5 {
                0 => {
                    for _ in 0..=(arg & 7) {
                        self.step(arg >> 3)?;
                    }
                }
                1 => self.fill_size = FILL_SIZES[arg as usize],
                2 => {
                    if arg & 16 != 0 {
                        self.sticky_flags = (arg & 15) as u32;
                    } else {
                        self.next_flags = (arg & 15) as u32;
                    }
                }
                3 => {
                    let param = DEC_PARAMS[arg as usize];
                    let value = param_value(next());
                    let ret = self.api.set_param(param, value)?;
                    self.push(DecEv::SetParam { param, value, ret });
                }
                4 => {
                    self.next_flags |= (arg & 15) as u32;
                    self.decode_mode(arg & 16 != 0)?;
                }
                5 => self.out_size = OUT_SIZES[arg as usize],
                6 => {
                    let conf = if arg == 0 {
                        self.p.config.clone().unwrap_or_default()
                    } else {
                        let a = k.min(ops.len());
                        let b = (k + arg as usize).min(ops.len());
                        k += arg as usize;
                        ops[a..b].to_vec()
                    };
                    self.config(&conf)?;
                }
                _ => match arg {
                    0..=7 => {
                        let size = ANC_SIZES[(next() & 15) as usize];
                        self.anc_init(size)?;
                    }
                    8 => {
                        let (ret, free) = self.api.free_bytes()?;
                        self.push(DecEv::GetFreeBytes { ret, free });
                    }
                    9 => {
                        self.fill(&[])?;
                    }
                    10 => {
                        if self.p.packet {
                            if self.ci < self.p.chunks.len() {
                                self.ci += 1;
                                self.co = 0;
                            }
                        } else {
                            let n = next() as usize;
                            self.co += n;
                            if self.ci < self.p.chunks.len()
                                && self.co >= self.p.chunks[self.ci].len()
                            {
                                self.ci += 1;
                                self.co = 0;
                            }
                        }
                    }
                    11 => self.decode_once = !self.decode_once,
                    _ => self.step(0)?,
                },
            }
        }
        // The rest of the buffered data, then the drain.
        if self.stalls < 2 {
            self.decode_mode(true)?;
        }
        if self.p.drain {
            for _ in 0..2 {
                if self.decodes >= MAX_DECODES {
                    break;
                }
                self.decode(AACDEC_FLUSH | self.sticky_flags)?;
            }
        }
        Ok(())
    }
}

fn run_api<A: DecApi>(api: &mut A, p: &DecProgram, verbose: bool) -> Trace<DecEv> {
    let mut d = Driver {
        api,
        p,
        verbose,
        trace: Trace::default(),
        out: Vec::new(),
        out_size: p.out_size,
        fill_size: p.fill_size,
        decode_once: p.decode_once,
        next_flags: 0,
        sticky_flags: 0,
        anc: false,
        decodes: 0,
        ci: 0,
        co: 0,
        stalls: 0,
    };
    let r = d.run();
    let mut trace = std::mem::take(&mut d.trace);
    if let Err(a) = r {
        trace.abort = Some(a);
    }
    // Also after an aborted reference call: Close frees the memory (an aborted Close is caught
    // as well, and the handle is then leaked).
    d.api.close();
    trace
}

/// Runs the program on the reference with the given stack pattern.
pub fn run_ref(p: &DecProgram, pattern: i32, verbose: bool) -> Trace<DecEv> {
    let mut api = RefDec {
        s: Session::begin(pattern),
        h: core::ptr::null_mut(),
        anc: Vec::new(),
        old_anc: Vec::new(),
    };
    run_api(&mut api, p, verbose)
}

/// Runs the program on the port.
pub fn run_port(p: &DecProgram, verbose: bool) -> Trace<DecEv> {
    run_api(&mut PortDec { h: None }, p, verbose)
}

fn detail(a: &DecEv, b: &DecEv) -> String {
    match (a, b) {
        (
            DecEv::DecodeFrame { out: x, info: ix, .. },
            DecEv::DecodeFrame { out: y, info: iy, .. },
        ) => {
            let mut s = crate::diff_bytes("output samples", x, y, 2);
            if ix != iy {
                if let (Some(ix), Some(iy)) = (ix, iy) {
                    let k: Vec<usize> =
                        (0..ix.len().max(iy.len())).filter(|&k| ix.get(k) != iy.get(k)).collect();
                    s +=
                        &format!("; CStreamInfo differs at value indices {k:?} (order of si_port)");
                }
            }
            s
        }
        (DecEv::AncDataGet { data: x, .. }, DecEv::AncDataGet { data: y, .. })
        | (DecEv::AncBuffer { data: x }, DecEv::AncBuffer { data: y }) => {
            crate::diff_bytes("ancillary bytes", x, y, 1)
        }
        _ => String::new(),
    }
}

/// `aacDecoder_GetLibInfo` of both, with two fill patterns of the array (the DRC entry's flags
/// are left as the caller's array held them).
pub fn check_lib_info() -> Result<(), String> {
    for fill in [0u8, 0xAB] {
        let mut ints = [0i32; 3 * 64];
        let mut strs = [0u8; 64 * 64];
        let mut n = 0;
        // SAFETY: arrays of FDK_MODULE_LAST entries.
        let r = unsafe {
            c::cref_aacdecoder_lib_lib_info(fill, ints.as_mut_ptr(), strs.as_mut_ptr(), &mut n)
        };
        let mut info = [LibInfo { flags: u32::from_ne_bytes([fill; 4]), ..LibInfo::default() };
            FDK_MODULE_LAST as usize];
        fdk_init_lib_info(&mut info);
        let rp = aac_decoder_get_lib_info(Some(&mut info));
        let np = info.iter().take_while(|e| e.module_id != FDK_NONE).count();
        if (r, n as usize) != (rp, np) {
            return Err(format!("decoder: ret/count {r}/{n} vs {rp}/{np}"));
        }
        for (i, e) in info.iter().take(np).enumerate() {
            let t = &strs[64 * i..64 * i + 32];
            let tl = t.iter().position(|&b| b == 0).unwrap_or(32);
            let vs = &strs[64 * i + 32..64 * i + 64];
            let vl = vs.iter().position(|&b| b == 0).unwrap_or(32);
            let pl = e.version_str.iter().position(|&b| b == 0).unwrap_or(32);
            if ints[3 * i] != e.module_id
                || ints[3 * i + 1] != e.version
                || ints[3 * i + 2] as u32 != e.flags
                || &t[..tl] != e.title.as_bytes()
                || vs[..vl] != e.version_str[..pl]
            {
                return Err(format!("decoder entry {i} (fill {fill:#x}): {e:?}"));
            }
        }
    }
    Ok(())
}

/// Compares the reference and the port on one input.
pub fn check(t: DecTarget, data: &[u8]) -> Outcome {
    crate::lib_info_once();
    let p = DecProgram::parse(t, data);
    let describe = format!(
        "transport {}, {} mode, {} access unit(s) / {} bytes, {} op bytes, config {:?}",
        p.transport,
        if p.packet { "packet" } else { "stream" },
        p.chunks.len(),
        p.chunks.iter().map(|c| c.len()).sum::<usize>(),
        p.ops.len(),
        p.config.as_ref().map(|c| c.iter().map(|b| format!("{b:02x}")).collect::<String>())
    );
    crate::compare(t.name(), &describe, |pat, v| run_ref(&p, pat, v), |v| run_port(&p, v), detail)
}

/// The fuzz target body.
pub fn fuzz(t: DecTarget, data: &[u8]) {
    crate::handle(t.name(), check(t, data));
}
