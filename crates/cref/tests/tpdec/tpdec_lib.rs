// Differential tests of fdk_aac_rs_tpdec::tpdec_lib against the C++ reference.
//
// The transport decoder runs as a whole on both sides: the same bytes are fed with the same
// fill pattern into transportDec_FillData(), and the same sequence of API calls follows
// (ReadAccessUnit, reading the access unit like a decoder, CRC regions, EndAccessUnit, the
// getters, SetParam, out-of-band and in-band configurations). After every call the return
// values, the complete internal state (flags, positions, counters, the parser of the transport
// format, the bitstream), both ASCs and the callback log are compared, and the bit buffer
// memory from time to time.
//
// Streams: the reference encoder (in this binary) encodes a synthetic signal in every transport
// and object type it supports (no test material); the streams are also mutated (bit flips,
// truncation, insertions, splices). LATM/LOAS with USAC and DRM are generated (configurations
// from the generators of tpdec_asc.rs, random payloads). Real-world ADTS files of
// work/rw/*.aac are used if present.

use core::ffi::c_void;
use std::sync::OnceLock;

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpdec_tpdec_lib as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_push_bi_directional, fdk_read_bits, FdkBitStream,
};
use fdk_aac_rs_tpdec::tp_data::*;
use fdk_aac_rs_tpdec::tpdec_lib::*;

use super::tp_data::{asc_dump, assert_dump_eq, flip_bits, hash, BitWriter, Rec, Recs, DUMP_MAX};
use super::tpdec_asc::{gen_asc, put_aot, put_sf};

// ---------------------------------------------------------------------------------------------
// Test streams from the reference encoder
// ---------------------------------------------------------------------------------------------

/// An encoded stream: transport type, bytes, sizes of the access units (encoder outputs) and
/// the configuration of aacEncInfo().
#[derive(Clone)]
pub struct Enc {
    pub name: String,
    pub tt: i32,
    pub data: Vec<u8>,
    pub au: Vec<usize>,
    pub conf: Vec<u8>,
}

const AACENC_AOT: i32 = 0x0100;
const AACENC_BITRATE: i32 = 0x0101;
const AACENC_SBR_MODE: i32 = 0x0104;
const AACENC_GRANULE_LENGTH: i32 = 0x0105;
const AACENC_CHANNELMODE: i32 = 0x0106;
const AACENC_TRANSMUX: i32 = 0x0300;
const AACENC_HEADER_PERIOD: i32 = 0x0301;
const AACENC_SIGNALING_MODE: i32 = 0x0302;
const AACENC_TPSUBFRAMES: i32 = 0x0303;
const AACENC_AUDIOMUXVER: i32 = 0x0304;
const AACENC_PROTECTION: i32 = 0x0306;

pub fn encode(name: &str, params: &[(i32, i32)], channels: i32, sr: i32, frames: i32, seed: i32) -> Option<Enc> {
    let tt = params.iter().find(|p| p.0 == AACENC_TRANSMUX).map(|p| p.1).unwrap_or(0);
    let flat: Vec<i32> = params.iter().flat_map(|p| [p.0, p.1]).collect();
    let mut out = vec![0u8; 1 << 21];
    let mut au = vec![0i32; 4096];
    let mut conf = vec![0u8; 64];
    let (mut out_len, mut n_au, mut conf_len) = (0, 0, 0);
    let r = unsafe {
        c::cref_tplib_encode(
            flat.as_ptr(),
            params.len() as i32,
            channels,
            sr,
            frames,
            seed,
            out.as_mut_ptr(),
            out.len() as i32,
            &mut out_len,
            au.as_mut_ptr(),
            au.len() as i32,
            &mut n_au,
            conf.as_mut_ptr(),
            &mut conf_len,
        )
    };
    if r != 0 || out_len == 0 {
        return None;
    }
    out.truncate(out_len as usize);
    conf.truncate(conf_len as usize);
    Some(Enc {
        name: name.to_string(),
        tt,
        data: out,
        au: au[..n_au as usize].iter().map(|&x| x as usize).collect(),
        conf,
    })
}

/// The encoder configurations: (name, aot, sample rate, channels, channel mode or 0, bit rate,
/// extra parameters).
fn codecs() -> Vec<(&'static str, i32, i32, i32, i32, i32, Vec<(i32, i32)>)> {
    vec![
        ("lc-st", 2, 44100, 2, 0, 128000, vec![]),
        ("lc-mono", 2, 48000, 1, 0, 64000, vec![]),
        ("lc-51", 2, 32000, 6, 0, 256000, vec![]),
        ("he", 5, 44100, 2, 0, 64000, vec![]),
        ("he-expl", 5, 44100, 2, 0, 64000, vec![(AACENC_SIGNALING_MODE, 1)]),
        ("he-hier", 5, 48000, 1, 0, 48000, vec![(AACENC_SIGNALING_MODE, 2)]),
        ("hev2", 29, 44100, 2, 0, 32000, vec![]),
        ("hev2-expl", 29, 32000, 2, 0, 32000, vec![(AACENC_SIGNALING_MODE, 1)]),
        ("ld", 23, 48000, 2, 0, 128000, vec![]),
        ("ld480", 23, 48000, 1, 0, 64000, vec![(AACENC_GRANULE_LENGTH, 480)]),
        ("eld", 39, 48000, 2, 0, 96000, vec![]),
        ("eld-sbr", 39, 48000, 2, 0, 64000, vec![(AACENC_SBR_MODE, 1)]),
        ("eld-sbr-mono", 39, 32000, 1, 0, 32000, vec![(AACENC_SBR_MODE, 1)]),
        ("eldv2", 39, 48000, 2, 128, 64000, vec![(AACENC_SBR_MODE, 1)]),
    ]
}

/// The transports: (name, TRANSPORT_TYPE, extra parameters).
fn transports() -> Vec<(&'static str, i32, Vec<(i32, i32)>)> {
    vec![
        ("raw", 0, vec![]),
        ("adif", 1, vec![]),
        ("adts", 2, vec![]),
        ("adts-crc", 2, vec![(AACENC_PROTECTION, 1)]),
        ("latm1", 6, vec![]),
        ("latm1-v1-sub2", 6, vec![(AACENC_AUDIOMUXVER, 1), (AACENC_TPSUBFRAMES, 2)]),
        ("latm0", 7, vec![]),
        ("loas", 10, vec![]),
        ("loas-v1", 10, vec![(AACENC_AUDIOMUXVER, 1), (AACENC_HEADER_PERIOD, 3)]),
        ("loas-v2-sub2", 10, vec![(AACENC_AUDIOMUXVER, 2), (AACENC_TPSUBFRAMES, 2)]),
    ]
}

pub fn streams() -> &'static Vec<Enc> {
    static S: OnceLock<Vec<Enc>> = OnceLock::new();
    S.get_or_init(|| {
        let mut v = Vec::new();
        let mut seed = 1;
        for (cn, aot, sr, ch, mode, br, extra) in codecs() {
            for (tn, tt, textra) in transports() {
                let mut p = vec![(AACENC_AOT, aot), (AACENC_BITRATE, br), (AACENC_TRANSMUX, tt)];
                if mode != 0 {
                    p.push((AACENC_CHANNELMODE, mode));
                }
                p.extend(extra.iter().copied());
                p.extend(textra.iter().copied());
                seed += 1;
                if let Some(e) = encode(&format!("{cn}-{tn}"), &p, ch, sr, 24, seed) {
                    v.push(e);
                }
            }
        }
        assert!(v.len() > 60, "only {} encoder streams", v.len());
        v
    })
}

/// The out-of-band configurations of the encoder: ASCs (raw transport) and StreamMuxConfigs
/// (LATM MCP0), with their transport type.
pub fn encoder_configs() -> Vec<(Vec<u8>, i32)> {
    streams().iter().filter(|e| (e.tt == 0 || e.tt == 7) && !e.conf.is_empty()).map(|e| (e.conf.clone(), e.tt)).collect()
}

/// Real-world ADTS files in the main checkout's work/rw (not part of the repository).
fn real_world() -> Vec<(String, Vec<u8>)> {
    let mut v = Vec::new();
    let out = std::process::Command::new("git").args(["rev-parse", "--git-common-dir"]).output();
    let Ok(out) = out else { return v };
    let common = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let dir = common.join("..").join("work").join("rw");
    let Ok(rd) = std::fs::read_dir(&dir) else { return v };
    let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "aac")).collect();
    files.sort();
    for f in files {
        if let Ok(d) = std::fs::read(&f) {
            v.push((f.display().to_string(), d[..d.len().min(200_000)].to_vec()));
        }
    }
    v
}

// ---------------------------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------------------------

/// The internal state as `cref_tplib_state()` dumps it.
fn rs_state(t: &TransportDec) -> Vec<i32> {
    let mut o = vec![
        t.transport_fmt,
        t.flags as i32,
        t.global_frame_pos as i32,
        t.access_unit_anchor[0] as i32,
        t.au_length[0],
        t.number_of_raw_data_blocks,
        t.avg_bit_rate as i32,
        t.last_valid_buffer_fullness as i32,
        t.remainder,
        t.missing_access_units,
        t.burst_period as i32,
        t.hold_off_frames as i32,
        t.target_layout,
    ];
    let c = &t.ctrl_cfg_change[0];
    o.extend([
        c.flush_cnt as i32,
        c.flush_status as i32,
        c.build_up_cnt as i32,
        c.build_up_status as i32,
        c.cfg_changed as i32,
        c.content_changed as i32,
        c.force_cfg_change as i32,
    ]);
    let bs = &t.bit_stream[0];
    o.extend([
        bs.cache_word as i32,
        bs.bits_in_cache as i32,
        bs.config_cache as i32,
        bs.h_bit_buf.valid_bits as i32,
        bs.h_bit_buf.read_offset as i32,
        bs.h_bit_buf.write_offset as i32,
        bs.h_bit_buf.bit_ndx as i32,
        bs.h_bit_buf.buf_size() as i32,
    ]);
    let crc = |o: &mut Vec<i32>, c: &fdk_aac_rs_fdk::crc::FdkCrcInfo| {
        for r in c.crc_reg_data.iter() {
            o.extend([r.is_active as i32, r.max_bits, r.bit_buf_cnt_bits, r.valid_bits]);
        }
        o.extend([
            c.reg_start as i32,
            c.reg_stop as i32,
            c.crc_value as i32,
            c.crc_poly as i32,
            c.crc_mask as i32,
            c.start_value as i32,
            c.crc_len as i32,
        ]);
    };
    match t.transport_fmt {
        TT_MP4_ADTS => {
            let a = &t.parser.adts;
            let b = &a.bs;
            o.extend([
                b.mpeg_id as i32,
                b.layer as i32,
                b.protection_absent as i32,
                b.profile as i32,
                b.sample_freq_index as i32,
                b.private_bit as i32,
                b.channel_config as i32,
                b.original as i32,
                b.home as i32,
                b.copyright_id as i32,
                b.copyright_start as i32,
                b.frame_length as i32,
                b.adts_fullness as i32,
                b.num_raw_blocks as i32,
                b.num_pce_bits as i32,
                a.decoder_can_do_mpeg4 as i32,
                a.buffer_fullnes_start_flag as i32,
                a.crc_read_value as i32,
            ]);
            o.extend(a.raw_data_block_dist.iter().map(|&x| x as i32));
            crc(&mut o, &a.crc_info);
        }
        TT_MP4_ADIF => {
            let a = &t.parser.adif;
            o.extend([
                a.num_program_config_elements,
                a.bit_rate as i32,
                a.copyright_id_present as i32,
                a.original_copy as i32,
                a.home as i32,
                a.bitstream_type as i32,
            ]);
        }
        TT_MP4_LATM_MCP0 | TT_MP4_LATM_MCP1 | TT_MP4_LOAS => {
            o.extend(super::tpdec_latm::latm_state(&t.parser.latm));
        }
        TT_DRM => {
            crc(&mut o, &t.parser.drm.crc_info);
            o.push(t.parser.drm.crc_read_value as i32);
        }
        _ => {}
    }
    o
}

fn buf_hash(t: &TransportDec) -> u32 {
    let b = t.bs_buffer().to_vec();
    let words: Vec<i32> = b.chunks(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    hash(&words)
}

// Forwarding functions for the registered (function pointer) callbacks.
fn rec_of(d: *mut c_void) -> &'static mut Rec {
    // SAFETY: the user data is the `Rec` of a live `Sess` (boxed, not moved while registered).
    unsafe { &mut *(d as *mut Rec) }
}
fn f_update(d: *mut c_void, a: &CSAudioSpecificConfig, m: u8, cc: &mut u8) -> i32 {
    rec_of(d).update_config(a, m, cc)
}
fn f_free(d: *mut c_void, a: &CSAudioSpecificConfig) -> i32 {
    rec_of(d).free_mem(a)
}
fn f_ctrl(d: *mut c_void, x: &CCtrlCFGChange) -> i32 {
    rec_of(d).ctrl_cfg_change(x)
}
#[allow(clippy::too_many_arguments)]
fn f_ssc(
    d: *mut c_void,
    b: &mut FdkBitStream,
    a: AudioObjectType,
    sr: i32,
    fs: i32,
    nc: i32,
    sci: i32,
    cs: i32,
    cb: i32,
    m: u8,
    cc: &mut u8,
) -> i32 {
    rec_of(d).ssc(b, a, sr, fs, nc, sci, cs, cb, m, cc)
}
#[allow(clippy::too_many_arguments)]
fn f_sbr(
    d: *mut c_void,
    b: &mut FdkBitStream,
    a: i32,
    bb: i32,
    s: i32,
    cc0: AudioObjectType,
    e: Mp4ElementId,
    ei: i32,
    h: u8,
    sci: u8,
    m: u8,
    cc: &mut u8,
    ds: i32,
) -> i32 {
    rec_of(d).sbr(b, a, bb, s, cc0, e, ei, h, sci, m, cc, ds)
}
fn f_usac(d: *mut c_void, b: &mut FdkBitStream) -> i32 {
    rec_of(d).usac(b)
}
fn f_uni_drc(d: *mut c_void, b: Option<&mut FdkBitStream>, l: i32, t: i32, s: i32, p: i32, a: AudioObjectType) -> i32 {
    rec_of(d).uni_drc(b, l, t, s, p, a)
}

/// A transport decoder on both sides.
pub struct Sess {
    pub rs: Box<TransportDec>,
    pub c: *mut c_void,
    pub recs: Box<Recs>,
    /// Use the registered function pointer callbacks (otherwise the `_cb` functions).
    pub registered: bool,
    pub calls: usize,
    /// Number of successful ReadAccessUnit calls.
    pub ok_aus: usize,
}

impl Sess {
    pub fn open(fmt: i32, flags: u32, layers: u32, recs: Recs, registered: bool) -> Option<Sess> {
        let has = (0..4).fold(0, |m, i| {
            m | ((match i {
                0 => recs.rs_has_ssc(),
                1 => recs.rs_has_sbr(),
                2 => recs.rs_has_usac(),
                _ => recs.rs_has_uni_drc(),
            }) as i32)
                << i
        });
        let mut recs = Box::new(recs);
        let rs = transport_dec_open(fmt, flags, layers);
        let c = unsafe { c::cref_tplib_open(fmt, flags, layers, recs.c, has) };
        assert_eq!(rs.is_some(), !c.is_null(), "transportDec_Open({fmt}, {flags}, {layers})");
        let mut rs = rs?;
        let d = &mut recs.rs as *mut Rec as *mut c_void;
        transport_dec_register_asc_callback(&mut rs, Some(f_update), d);
        transport_dec_register_free_mem_callback(&mut rs, Some(f_free), d);
        transport_dec_register_ctrl_cfg_change_callback(&mut rs, Some(f_ctrl), d);
        if has & 1 != 0 {
            transport_dec_register_ssc_callback(&mut rs, Some(f_ssc), d);
        }
        if has & 2 != 0 {
            transport_dec_register_sbr_callback(&mut rs, Some(f_sbr), d);
        }
        if has & 4 != 0 {
            transport_dec_register_usac_callback(&mut rs, Some(f_usac), d);
        }
        if has & 8 != 0 {
            transport_dec_register_uni_drc_config_callback(&mut rs, Some(f_uni_drc), d, core::ptr::null_mut());
        }
        Some(Sess { rs, c, recs, registered, calls: 0, ok_aus: 0 })
    }

    /// Compares the complete state after a call.
    pub fn check(&mut self, what: &str) {
        self.calls += 1;
        let mut cs = vec![0i32; 256];
        let n = unsafe { c::cref_tplib_state(self.c, cs.as_mut_ptr()) } as usize;
        let rs = rs_state(&self.rs);
        assert_eq!(rs, cs[..n], "{what}: state");
        for idx in 0..2 {
            let mut cd = vec![0i32; DUMP_MAX];
            let n = unsafe { c::cref_tplib_asc(self.c, idx, cd.as_mut_ptr()) } as usize;
            assert_dump_eq(&asc_dump(&self.rs.asc[idx as usize]), &cd[..n], &format!("{what}: asc[{idx}]"));
        }
        self.recs.check(what);
        if self.calls % 16 == 0 {
            assert_eq!(buf_hash(&self.rs), unsafe { c::cref_tplib_buf_hash(self.c) }, "{what}: buffer");
        }
    }

    pub fn fill(&mut self, buf: &[u8], size: u32, bytes_valid: &mut u32, layer: i32) -> i32 {
        let mut cv = *bytes_valid;
        let r = transport_dec_fill_data(&mut self.rs, buf, size, bytes_valid, layer);
        let e = unsafe { c::cref_tplib_fill(self.c, buf.as_ptr(), size, &mut cv, layer) };
        assert_eq!(r, e, "FillData");
        assert_eq!(*bytes_valid, cv, "FillData bytesValid");
        self.check("FillData");
        r
    }

    pub fn read_au(&mut self) -> i32 {
        let r = if self.registered {
            transport_dec_read_access_unit(&mut self.rs, 0)
        } else {
            transport_dec_read_access_unit_cb(&mut self.rs, 0, &mut self.recs.rs)
        };
        let e = unsafe { c::cref_tplib_read_au(self.c, 0) };
        assert_eq!(r, e, "ReadAccessUnit");
        if r == TRANSPORTDEC_OK {
            self.ok_aus += 1;
        }
        self.check("ReadAccessUnit");
        r
    }

    pub fn end_au(&mut self) -> i32 {
        let r = transport_dec_end_access_unit(&mut self.rs);
        let e = unsafe { c::cref_tplib_end_au(self.c) };
        assert_eq!(r, e, "EndAccessUnit");
        self.check("EndAccessUnit");
        r
    }

    pub fn au_bits_remaining(&mut self) -> i32 {
        let r = transport_dec_get_au_bits_remaining(&mut self.rs, 0);
        let e = unsafe { c::cref_tplib_au_bits_remaining(self.c, 0) };
        assert_eq!(r, e, "GetAuBitsRemaining");
        self.check("GetAuBitsRemaining");
        r
    }

    pub fn getters(&mut self) {
        assert_eq!(transport_dec_get_au_bits_total(&self.rs, 0), unsafe { c::cref_tplib_au_bits_total(self.c, 0) }, "GetAuBitsTotal");
        assert_eq!(transport_dec_get_buffer_fullness(&self.rs), unsafe { c::cref_tplib_buffer_fullness(self.c) }, "GetBufferFullness");
        let (mut a, mut b) = (0, 0);
        assert_eq!(transport_dec_get_missing_access_unit_count(&mut a, &self.rs), unsafe { c::cref_tplib_missing_au(self.c, &mut b) });
        assert_eq!(a, b, "GetMissingAccessUnitCount");
        assert_eq!(transport_dec_get_nr_of_sub_frames(&self.rs), unsafe { c::cref_tplib_nr_sub_frames(self.c) }, "GetNrOfSubFrames");
        assert_eq!(transport_dec_get_format(&self.rs), unsafe { c::cref_tplib_get_format(self.c) }, "GetFormat");
        for layer in 0..2 {
            let mut asc = CSAudioSpecificConfig::zeroed();
            let r = transport_dec_get_asc(&self.rs, layer, &mut asc);
            let mut cd = vec![0i32; DUMP_MAX];
            let mut n = 0;
            let e = unsafe { c::cref_tplib_get_asc(self.c, layer, cd.as_mut_ptr(), &mut n) };
            assert_eq!(r, e, "GetAsc");
            assert_dump_eq(&asc_dump(&asc), &cd[..n as usize], "GetAsc");
        }
        self.check("getters");
    }

    pub fn set_param(&mut self, p: i32, v: i32) {
        let r = transport_dec_set_param(&mut self.rs, p, v);
        let e = unsafe { c::cref_tplib_set_param(self.c, p, v) };
        assert_eq!(r, e, "SetParam({p}, {v})");
        self.check(&format!("SetParam({p}, {v})"));
    }

    /// Reads `n` bits of the bitstream on both sides.
    pub fn consume(&mut self, mut n: i32) {
        while n > 0 {
            let k = n.min(32) as u32;
            let a = fdk_read_bits(&mut self.rs.bit_stream[0], k);
            let b = unsafe { c::cref_tplib_bs_read(self.c, k) };
            assert_eq!(a, b, "bits read");
            n -= k as i32;
        }
        self.check("consume");
    }

    pub fn push(&mut self, n: i32) {
        fdk_push_bi_directional(&mut self.rs.bit_stream[0], n);
        unsafe { c::cref_tplib_bs_push(self.c, n) };
        self.check("push");
    }

    pub fn valid_bits(&mut self) -> i32 {
        let a = fdk_get_valid_bits(&mut self.rs.bit_stream[0]) as i32;
        let b = unsafe { c::cref_tplib_bs_valid_bits(self.c) };
        assert_eq!(a, b, "valid bits");
        a
    }

    pub fn crc_start(&mut self, m_bits: i32) -> i32 {
        let r = transport_dec_crc_start_reg(&mut self.rs, m_bits);
        let e = unsafe { c::cref_tplib_crc_start_reg(self.c, m_bits) };
        assert_eq!(r, e, "CrcStartReg");
        self.check("CrcStartReg");
        r
    }

    pub fn crc_end(&mut self, reg: i32) {
        transport_dec_crc_end_reg(&mut self.rs, reg);
        unsafe { c::cref_tplib_crc_end_reg(self.c, reg) };
        self.check("CrcEndReg");
    }

    pub fn crc_check(&mut self) -> i32 {
        let r = transport_dec_crc_check(&mut self.rs);
        let e = unsafe { c::cref_tplib_crc_check(self.c) };
        assert_eq!(r, e, "CrcCheck");
        self.check("CrcCheck");
        r
    }

    pub fn oob_config(&mut self, conf: &[u8], length: u32, layer: u32) -> i32 {
        let r = if self.registered {
            transport_dec_out_of_band_config(&mut self.rs, conf, length, layer)
        } else {
            transport_dec_out_of_band_config_cb(&mut self.rs, conf, length, layer, &mut self.recs.rs)
        };
        let mut padded = conf.to_vec();
        padded.resize(conf.len().max(length as usize).max(1), 0);
        let e = unsafe { c::cref_tplib_out_of_band_config(self.c, padded.as_ptr(), length, layer) };
        assert_eq!(r, e, "OutOfBandConfig");
        self.check("OutOfBandConfig");
        r
    }

    /// `cfg` holds TP_USAC_MAX_CONFIG_LEN bytes.
    pub fn in_band_config(&mut self, cfg: &[u8; TP_USAC_MAX_CONFIG_LEN], length: u32, build_up: u8) -> (i32, u8, u8) {
        let (mut ch, mut diff) = (0x55u8, 0x55u8);
        let r = if self.registered {
            transport_dec_in_band_config(&mut self.rs, cfg, length, build_up, &mut ch, 0, &mut diff)
        } else {
            transport_dec_in_band_config_cb(&mut self.rs, cfg, length, build_up, &mut ch, 0, &mut diff, &mut self.recs.rs)
        };
        let (mut cch, mut cdiff) = (0x55u8, 0x55u8);
        let mut cc = *cfg;
        let e = unsafe { c::cref_tplib_in_band_config(self.c, cc.as_mut_ptr(), length, build_up, &mut cch, 0, &mut cdiff) };
        assert_eq!(r, e, "InBandConfig");
        assert_eq!((ch, diff), (cch, cdiff), "InBandConfig outputs");
        self.check("InBandConfig");
        (r, ch, diff)
    }
}

impl Recs {
    fn rs_has_ssc(&self) -> bool {
        self.rs.has_ssc()
    }
    fn rs_has_sbr(&self) -> bool {
        self.rs.has_sbr()
    }
    fn rs_has_usac(&self) -> bool {
        self.rs.has_usac()
    }
    fn rs_has_uni_drc(&self) -> bool {
        self.rs.has_uni_drc()
    }
}

impl Drop for Sess {
    fn drop(&mut self) {
        unsafe { c::cref_tplib_close(self.c) };
    }
}

// ---------------------------------------------------------------------------------------------
// Driving a stream
// ---------------------------------------------------------------------------------------------

/// Random API calls between the access units.
fn random_extras(rng: &mut Rng, s: &mut Sess, params: bool) {
    if rng.next_u32() % 4 == 0 {
        s.getters();
    }
    if rng.next_u32() % 32 == 0 {
        // CRC functions for any format (between access units, as no decoder would)
        let reg = s.crc_start(0);
        s.crc_end(if reg >= 0 { reg } else { 0 });
        s.crc_check();
    }
    if params && rng.next_u32() % 40 == 0 {
        let (p, v) = match rng.next_u32() % 9 {
            0 => (TPDEC_PARAM_RESET, 1),
            1 => (TPDEC_PARAM_MINIMIZE_DELAY, (rng.next_u32() & 1) as i32),
            2 => (TPDEC_PARAM_EARLY_CONFIG, (rng.next_u32() & 1) as i32),
            3 => (TPDEC_PARAM_IGNORE_BUFFERFULLNESS, (rng.next_u32() & 1) as i32),
            4 => (TPDEC_PARAM_SET_BITRATE, [0, 64000, 128000, 1_000_000][(rng.next_u32() % 4) as usize]),
            5 => (TPDEC_PARAM_BURST_PERIOD, [0, 10, 100, 1000][(rng.next_u32() % 4) as usize]),
            6 => (TPDEC_PARAM_TARGETLAYOUT, rng.next_u32() as i32 % 20),
            7 => (TPDEC_PARAM_FORCE_CONFIG_CHANGE, 0),
            _ => (if rng.next_u32() % 2 == 0 { TPDEC_PARAM_USE_ELEM_SKIPPING } else { 99 }, (rng.next_u32() & 1) as i32),
        };
        s.set_param(p, v);
    }
}

/// Reads the access unit like a decoder: the remaining bits of the access unit (byte-wise
/// changes sometimes), with CRC regions for ADTS and DRM.
fn decode_au(rng: &mut Rng, s: &mut Sess, fmt: i32, adif_end: Option<i32>) {
    let mut n = s.au_bits_remaining();
    if let Some(target) = adif_end {
        n = target;
    }
    let crc = (fmt == TT_MP4_ADTS || fmt == TT_DRM) && rng.next_u32() % 2 == 0;
    let mut reg = -1;
    if crc {
        let m_bits = [0, 0, 192, -8, 16, 1000][(rng.next_u32() % 6) as usize];
        reg = s.crc_start(m_bits);
        if fmt == TT_DRM {
            n -= 8;
        }
    }
    match rng.next_u32() % 10 {
        0 => n += 8 * (rng.next_u32() % 5) as i32,
        1 => n -= 8 * (rng.next_u32() % 5) as i32,
        2 if fmt == TT_MP4_RAW || fmt == TT_DRM || fmt == TT_MP4_LATM_MCP1 || fmt == TT_MP4_LATM_MCP0 => {
            n += rng.next_u32() as i32 % 13 - 6
        }
        _ => {}
    }
    let avail = s.valid_bits();
    n = n.min(avail.max(0) + if rng.next_u32() % 8 == 0 { 8 } else { 0 });
    if n > 0 {
        if crc && rng.next_u32() % 2 == 0 {
            let half = (n / 16) * 8;
            s.consume(half);
            s.crc_end(reg);
            let reg2 = s.crc_start(0);
            s.consume(n - half);
            s.crc_end(reg2);
        } else {
            s.consume(n);
            if crc {
                s.crc_end(reg);
            }
        }
    } else if crc {
        s.crc_end(reg);
    }
    if crc {
        s.crc_check();
    }
}

/// Feeds `data` (access units `au` for packet formats, random chunks otherwise) and decodes it.
fn run_stream(rng: &mut Rng, s: &mut Sess, fmt: i32, data: &[u8], au: &[usize], params: bool) {
    let packet = tt_is_packet(fmt);
    let mut errors = 0;
    if packet {
        let mut pos = 0;
        let mut k = 0;
        let mut pending: Option<(usize, usize)> = None;
        loop {
            let (start, len) = match pending.take() {
                Some(p) => p,
                None => {
                    if k >= au.len() {
                        break;
                    }
                    let l = au[k];
                    k += 1;
                    let p = (pos, l);
                    pos += l;
                    p
                }
            };
            let chunk = &data[start.min(data.len())..(start + len).min(data.len())];
            let mut bv = chunk.len() as u32;
            let r = s.fill(chunk, chunk.len() as u32, &mut bv, 0);
            if r == TRANSPORTDEC_OK && bv != 0 {
                // not taken (raw data blocks remaining)
                pending = Some((start + chunk.len() - bv as usize, bv as usize));
            }
            for _ in 0..8 {
                let r = s.read_au();
                if r == TRANSPORTDEC_OK {
                    decode_au(rng, s, fmt, None);
                    s.end_au();
                } else if r == TRANSPORTDEC_NOT_ENOUGH_BITS {
                    break;
                } else {
                    errors += 1;
                    if rng.next_u32() % 2 == 0 {
                        s.end_au();
                    }
                }
                random_extras(rng, s, params);
                if s.rs.number_of_raw_data_blocks <= 0 || rng.next_u32() % 4 == 0 {
                    break;
                }
            }
            if errors > 400 {
                break;
            }
        }
    } else {
        // AU end positions (in bits of the stream) for ADIF
        let mut ends = Vec::new();
        let mut acc = 0;
        for &l in au {
            acc += l;
            ends.push(acc as i64 * 8);
        }
        let mut pos = 0usize;
        let mut fed = 0usize;
        let mut stall = 0;
        let fill_sizes = [4096usize, 1, 7, 100, 333, 1500, 2048, 8192, 40000];
        let fill_size = fill_sizes[(rng.next_u32() as usize) % fill_sizes.len()];
        let mut ai = 0;
        while pos < data.len() || stall < 3 {
            if pos < data.len() {
                let sz = if rng.next_u32() % 4 == 0 { 1 + (rng.next_u32() as usize) % 3000 } else { fill_size };
                let chunk = &data[pos..(pos + sz).min(data.len())];
                let mut bv = chunk.len() as u32;
                s.fill(chunk, chunk.len() as u32, &mut bv, 0);
                let taken = chunk.len() - bv as usize;
                pos += taken;
                fed += taken;
            } else {
                stall += 1;
            }
            for _ in 0..200 {
                let r = s.read_au();
                if r == TRANSPORTDEC_OK {
                    let adif_end = if fmt == TT_MP4_ADIF {
                        let consumed = fed as i64 * 8 - s.valid_bits() as i64;
                        while ai < ends.len() && ends[ai] <= consumed {
                            ai += 1;
                        }
                        Some(ends.get(ai).map(|&e| (e - consumed) as i32).unwrap_or(0))
                    } else {
                        None
                    };
                    decode_au(rng, s, fmt, adif_end);
                    s.end_au();
                } else if r == TRANSPORTDEC_NOT_ENOUGH_BITS {
                    break;
                } else {
                    errors += 1;
                    if rng.next_u32() % 2 == 0 {
                        s.end_au();
                    }
                    if errors > 400 {
                        break;
                    }
                }
                random_extras(rng, s, params);
            }
            if errors > 400 {
                break;
            }
        }
    }
}

/// Opens a session for a stream and configures it out of band where needed.
fn open_for(fmt: i32, conf: &[u8], error_rate: u32, rng: &mut Rng) -> Option<Sess> {
    let recs = Recs::random(error_rate, rng);
    let flags = if rng.next_u32() % 8 == 0 { 0 } else { TP_FLAG_MPEG4 };
    let registered = rng.next_u32() % 2 == 0;
    let mut s = Sess::open(fmt, flags, 1, recs, registered)?;
    s.check("open");
    // The AAC decoder always ignores the buffer fullness (aacDecoder_Open()).
    if rng.next_u32() % 8 != 0 {
        s.set_param(TPDEC_PARAM_IGNORE_BUFFERFULLNESS, 1);
    }
    if (fmt == TT_MP4_RAW || fmt == TT_MP4_LATM_MCP0 || fmt == TT_DRM) && !conf.is_empty() {
        s.oob_config(conf, conf.len() as u32, 0);
    }
    Some(s)
}

fn mutate(rng: &mut Rng, e: &Enc) -> (Vec<u8>, Vec<usize>) {
    let mut data = e.data.clone();
    let mut au = e.au.clone();
    match rng.next_u32() % 5 {
        0 => {
            let n = 1 + rng.next_u32() % 20;
            flip_bits(&mut data, n, rng);
        }
        1 => {
            // remove a random range
            if data.len() > 10 {
                let a = (rng.next_u32() as usize) % data.len();
                let l = 1 + (rng.next_u32() as usize) % 300;
                let b = (a + l).min(data.len());
                data.drain(a..b);
                // fix the AU sizes: shorten the AUs that overlap
                let mut cut = b - a;
                let mut acc = 0;
                for x in au.iter_mut() {
                    if acc + *x > a && cut > 0 {
                        let r = (*x).min(cut);
                        *x -= r;
                        cut -= r;
                    }
                    acc += *x;
                }
            }
        }
        2 => {
            // insert garbage
            let a = (rng.next_u32() as usize) % (data.len() + 1);
            let l = 1 + (rng.next_u32() as usize) % 200;
            let g: Vec<u8> = (0..l).map(|_| rng.next_u32() as u8).collect();
            data.splice(a..a, g);
            if let Some(x) = au.first_mut() {
                *x += l;
            }
        }
        3 => {
            // splice with another stream of the same transport
            let others: Vec<&Enc> = streams().iter().filter(|o| o.tt == e.tt).collect();
            let o = others[(rng.next_u32() as usize) % others.len()];
            let cut = au.len() / 2;
            let a: usize = au[..cut].iter().sum();
            let ocut = o.au.len() / 3;
            let b: usize = o.au[..ocut].iter().sum();
            data.truncate(a);
            data.extend_from_slice(&o.data[b..]);
            au.truncate(cut);
            au.extend_from_slice(&o.au[ocut..]);
        }
        _ => {
            // truncate the end
            let l = (rng.next_u32() as usize) % (data.len() + 1);
            data.truncate(l);
        }
    }
    (data, au)
}

#[test]
fn encoder_streams() {
    let mut rng = Rng::new(0x51);
    let (mut ok, mut total) = (0, 0);
    for (i, e) in streams().iter().enumerate() {
        for k in 0..2 {
            let Some(mut s) = open_for(e.tt, &e.conf, if k == 0 { 0 } else { 6 }, &mut rng) else {
                panic!("open {}", e.name)
            };
            run_stream(&mut rng, &mut s, e.tt, &e.data, &e.au, k == 1);
            let _ = i;
            if k == 0 && s.rs.flags & TPDEC_IGNORE_BUFFERFULLNESS != 0 {
                ok += s.ok_aus;
                total += e.au.len();
                if s.ok_aus * 4 < e.au.len() * 3 {
                    eprintln!("{} ({i}): {} of {} access units", e.name, s.ok_aus, e.au.len());
                }
            }
        }
    }
    // The unmodified streams decode: the test runs the real parsing paths.
    eprintln!("encoder_streams: {ok} of {total} access units read");
    assert!(ok * 10 >= total * 9, "only {ok} of {total} access units read");
}

#[test]
fn mutated_streams() {
    let mut rng = Rng::new(0x52);
    let n = streams().len();
    for it in 0..(6 * n) {
        let e = &streams()[it % n];
        let (data, au) = mutate(&mut rng, e);
        let mut conf = e.conf.clone();
        if rng.next_u32() % 6 == 0 {
            flip_bits(&mut conf, 1, &mut rng);
        }
        let Some(mut s) = open_for(e.tt, &conf, if rng.next_u32() % 3 == 0 { 5 } else { 0 }, &mut rng) else {
            continue;
        };
        run_stream(&mut rng, &mut s, e.tt, &data, &au, true);
    }
}

/// Streams fed with a transport type other than the one they were encoded for.
#[test]
fn wrong_transport() {
    let mut rng = Rng::new(0x53);
    let fmts = [TT_MP4_RAW, TT_MP4_ADIF, TT_MP4_ADTS, TT_MP4_LATM_MCP1, TT_MP4_LATM_MCP0, TT_MP4_LOAS, TT_DRM];
    for (i, e) in streams().iter().enumerate().filter(|(i, _)| i % 3 == 0) {
        let fmt = fmts[(rng.next_u32() as usize) % fmts.len()];
        let Some(mut s) = open_for(fmt, &e.conf, 0, &mut rng) else { continue };
        run_stream(&mut rng, &mut s, fmt, &e.data, &e.au, true);
        let _ = i;
    }
}

#[test]
fn real_world_adts() {
    let mut rng = Rng::new(0x54);
    let files = real_world();
    if files.is_empty() {
        eprintln!("real_world_adts: work/rw not available, skipped");
    }
    for (_name, data) in files {
        for k in 0..2 {
            let Some(mut s) = open_for(TT_MP4_ADTS, &[], 0, &mut rng) else { continue };
            let mut d = data.clone();
            if k == 1 {
                flip_bits(&mut d, 30, &mut rng);
            }
            run_stream(&mut rng, &mut s, TT_MP4_ADTS, &d, &[], k == 1);
        }
    }
}

#[test]
fn open_close_formats() {
    for fmt in -2..=14 {
        for layers in 0..3u32 {
            let recs = Recs::new(&[0; 64], &[0; 64], 15);
            if let Some(mut s) = Sess::open(fmt, TP_FLAG_MPEG4, layers, recs, true) {
                s.check("open");
                if layers == 1 {
                    // a little data
                    let d = [0xffu8, 0xf1, 0x50, 0x80, 0x01, 0x7f, 0xfc, 0, 0, 0, 0];
                    let mut bv = d.len() as u32;
                    s.fill(&d, d.len() as u32, &mut bv, 0);
                    s.read_au();
                    s.getters();
                }
                // an invalid layer for FillData
                let mut bv = 1;
                s.fill(&[0], 1, &mut bv, 1);
            }
        }
    }
    let mut ph = transport_dec_open(TT_MP4_ADTS, 0, 1);
    transport_dec_close(&mut ph);
    assert!(ph.is_none());
}

#[test]
fn get_lib_info() {
    const N: usize = FDK_MODULE_LAST as usize;
    let mut rng = Rng::new(0x55);
    for it in 0..200 {
        let null_info = it == 0;
        let mut ids_in = [0i32; N];
        for (i, x) in ids_in.iter_mut().enumerate() {
            *x = match rng.next_u32() % 4 {
                0 => FDK_NONE,
                _ => 1 + (i as i32 + rng.next_u32() as i32 % 3).rem_euclid(40),
            };
        }
        if it % 5 == 0 {
            ids_in = [7; N];
        }
        let (mut ids, mut version, mut flags) = ([0i32; N], [0i32; N], [0u32; N]);
        let (mut title, mut vstr) = ([0i8; 32 * N], [0i8; 32 * N]);
        let e = unsafe {
            c::cref_tplib_get_lib_info(
                null_info as i32,
                ids_in.as_ptr(),
                ids.as_mut_ptr(),
                version.as_mut_ptr(),
                flags.as_mut_ptr(),
                title.as_mut_ptr(),
                vstr.as_mut_ptr(),
            )
        };
        let mut info = [LibInfo::default(); N];
        for (i, e) in info.iter_mut().enumerate() {
            e.module_id = ids_in[i];
            e.version_str = [b'x'; 32];
        }
        let r = transport_dec_get_lib_info(if null_info { None } else { Some(&mut info) });
        assert_eq!(r, e, "transportDec_GetLibInfo {it}");
        for i in 0..N {
            assert_eq!(info[i].module_id, ids[i], "{it}/{i} module_id");
            assert_eq!(info[i].version, version[i], "{it}/{i} version");
            assert_eq!(info[i].flags, flags[i], "{it}/{i} flags");
            let cv: Vec<u8> = vstr[32 * i..32 * i + 32].iter().map(|&b| b as u8).collect();
            assert_eq!(info[i].version_str.as_slice(), cv.as_slice(), "{it}/{i} versionStr");
            let ct: Vec<u8> = title[32 * i..32 * i + 32].iter().map(|&b| b as u8).take_while(|&b| b != 0).collect();
            assert_eq!(info[i].title.as_bytes(), ct.as_slice(), "{it}/{i} title");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Generated LATM/LOAS streams (USAC, random payloads) and DRM
// ---------------------------------------------------------------------------------------------

fn latm_value(w: &mut BitWriter, v: u32) {
    let n = if v < 0x100 { 0 } else if v < 0x10000 { 1 } else if v < 0x1000000 { 2 } else { 3 };
    w.put(n, 2);
    for i in (0..=n).rev() {
        w.put((v >> (8 * i)) & 0xff, 8);
    }
}

/// A StreamMuxConfig with the given ASC (bits of `asc`).
fn gen_smc(rng: &mut Rng, w: &mut BitWriter, asc: &BitWriter, sub_frames: u32) {
    let ver = (rng.next_u32() % 3 == 0) as u32;
    w.put(ver, 1);
    if ver == 1 {
        w.put((rng.next_u32() % 16 == 0) as u32, 1); // audioMuxVersionA
        latm_value(w, rng.next_u32() & 0xff);
    }
    w.put(if rng.next_u32() % 16 == 0 { 0 } else { 1 }, 1); // allStreamsSameTimeFraming
    w.put(sub_frames - 1, 6);
    w.put(if rng.next_u32() % 32 == 0 { 1 } else { 0 }, 4); // numProgram - 1
    w.put(if rng.next_u32() % 32 == 0 { 1 } else { 0 }, 3); // numLayer - 1
    if ver == 1 {
        let len = if rng.next_u32() % 16 == 0 { rng.next_u32() % 400 } else { asc.nbits as u32 };
        latm_value(w, len);
    }
    for i in 0..asc.nbits {
        w.put(((asc.bytes[i / 8] >> (7 - i % 8)) & 1) as u32, 1);
    }
    let flt = if rng.next_u32() % 16 == 0 { rng.next_u32() & 7 } else { 0 };
    w.put(flt, 3);
    if flt == 0 {
        w.put(if rng.next_u32() % 2 == 0 { 0xff } else { rng.next_u32() & 0xff }, 8);
    } else if flt == 1 {
        w.put(rng.next_u32() & 0x1ff, 9);
    }
    let other = rng.next_u32() % 8 == 0;
    w.put(other as u32, 1);
    if other {
        if ver == 1 {
            latm_value(w, rng.next_u32() % 64);
        } else {
            w.put(0, 1);
            w.put(rng.next_u32() % 64, 8);
        }
    }
    let crc = rng.next_u32() % 4 == 0;
    w.put(crc as u32, 1);
    if crc {
        w.put(rng.next_u32() & 0xff, 8);
    }
}

/// A LATM (MCP1) / LOAS stream with the configuration `asc` and random payloads. Returns the
/// bytes and the AudioMuxElement sizes.
fn gen_latm(rng: &mut Rng, asc: &BitWriter, alt: Option<&BitWriter>, loas: bool, frames: usize) -> (Vec<u8>, Vec<usize>, Vec<u8>) {
    let mut out = Vec::new();
    let mut sizes = Vec::new();
    let sub = if rng.next_u32() % 4 == 0 { 2 } else { 1 };
    let mut smc_w = BitWriter::new();
    gen_smc(rng, &mut smc_w, asc, sub);
    for f in 0..frames {
        let mut w = BitWriter::new();
        let send_smc = f == 0 || rng.next_u32() % 5 == 0;
        w.put(!send_smc as u32, 1); // useSameStreamMux
        if send_smc {
            let mut s = BitWriter::new();
            if let (Some(alt), true) = (alt, rng.next_u32() % 3 == 0) {
                gen_smc(rng, &mut s, alt, sub);
            } else if rng.next_u32() % 6 == 0 {
                gen_smc(rng, &mut s, asc, sub);
            } else {
                s = smc_w.clone();
            }
            for i in 0..s.nbits {
                w.put(((s.bytes[i / 8] >> (7 - i % 8)) & 1) as u32, 1);
            }
        }
        for _ in 0..sub {
            let len = (rng.next_u32() % 600) as usize;
            let mut l = len;
            while l >= 255 {
                w.put(255, 8);
                l -= 255;
            }
            w.put(l as u32, 8);
            w.random_bytes(len, rng);
        }
        w.align();
        if loas {
            let mut h = BitWriter::new();
            h.put(0x2b7, 11);
            h.put(w.bytes.len() as u32, 13);
            out.extend_from_slice(&h.bytes);
            sizes.push(w.bytes.len() + 3);
        } else {
            sizes.push(w.bytes.len());
        }
        out.extend_from_slice(&w.bytes);
    }
    (out, sizes, smc_w.bytes)
}

fn gen_usac_asc(rng: &mut Rng) -> BitWriter {
    loop {
        let mut w = BitWriter::new();
        gen_asc(rng, &mut w);
        // keep the USAC ones (AOT 42 is escaped: 11111 001010)
        if w.nbits >= 11 && (w.bytes[0] >> 3) == 31 && (((w.bytes[0] & 7) << 3) | (w.bytes[1] >> 5)) == 10 {
            return w;
        }
    }
}

#[test]
fn generated_latm_loas() {
    let mut rng = Rng::new(0x56);
    for it in 0..1200 {
        let asc = if it % 2 == 0 {
            gen_usac_asc(&mut rng)
        } else {
            let mut w = BitWriter::new();
            gen_asc(&mut rng, &mut w);
            w
        };
        let fmt = [TT_MP4_LOAS, TT_MP4_LATM_MCP1, TT_MP4_LATM_MCP0][(rng.next_u32() % 3) as usize];
        let alt = if it % 2 == 0 && rng.next_u32() % 2 == 0 { Some(gen_usac_asc(&mut rng)) } else { None };
        let (mut data, au, smc) = gen_latm(&mut rng, &asc, alt.as_ref(), fmt == TT_MP4_LOAS, 12);
        if rng.next_u32() % 3 == 0 {
            flip_bits(&mut data, 1 + rng.next_u32() % 4, &mut rng);
        }
        let conf = if fmt == TT_MP4_LATM_MCP0 { smc } else { Vec::new() };
        let Some(mut s) = open_for(fmt, &conf, if rng.next_u32() % 3 == 0 { 5 } else { 0 }, &mut rng) else {
            continue;
        };
        run_stream_with_in_band(&mut rng, &mut s, fmt, &data, &au, &asc);
    }
}

/// `run_stream()` with in-band USAC configurations (AudioPreRoll) between the access units.
fn run_stream_with_in_band(rng: &mut Rng, s: &mut Sess, fmt: i32, data: &[u8], au: &[usize], asc: &BitWriter) {
    // Split the stream into a few parts, with in-band configurations in between.
    let parts = 1 + rng.next_u32() as usize % 3;
    let per = au.len().div_ceil(parts);
    let mut off = 0;
    for p in 0..parts {
        let a = &au[(p * per).min(au.len())..((p + 1) * per).min(au.len())];
        let len: usize = a.iter().sum();
        run_stream(rng, s, fmt, &data[off.min(data.len())..(off + len).min(data.len())], a, true);
        off += len;
        in_band(rng, s, asc);
    }
}

/// transportDec_InBandConfig() with the current UsacConfig() or another one, if the decoder has a
/// USAC configuration (the AAC decoder calls it for AudioPreRoll() of USAC only).
fn in_band(rng: &mut Rng, s: &mut Sess, _asc: &BitWriter) {
    for _ in 0..1 + rng.next_u32() % 3 {
        if s.rs.asc[0].m_aot != AOT_USAC {
            return;
        }
        let mut cfg = [0u8; TP_USAC_MAX_CONFIG_LEN];
        let u = &s.rs.asc[0].m_sc.m_usac_config;
        let mut len = (u.usac_config_bits as u32).div_ceil(8);
        match rng.next_u32() % 4 {
            0 | 1 => cfg = u.usac_config,
            2 => {
                // another UsacConfig(): from a generated ASC (skip the 11-bit AOT, sf index and
                // channel configuration is not possible bit-exactly; use the raw bytes after the
                // first byte as an approximation)
                let w = gen_usac_asc(rng);
                let n = w.bytes.len().min(TP_USAC_MAX_CONFIG_LEN - 1);
                cfg[..n - 1].copy_from_slice(&w.bytes[1..n]);
                len = (n - 1) as u32;
            }
            _ => {
                cfg = u.usac_config;
                flip_bits(&mut cfg[..len.max(1) as usize], 1, rng);
            }
        }
        if rng.next_u32() % 8 == 0 {
            len = rng.next_u32() % 300;
        }
        let build_up = (rng.next_u32() % 6) as u8;
        s.in_band_config(&cfg, len, build_up);
        if rng.next_u32() % 2 == 0 {
            s.getters();
        }
    }
}

/// Raw packets with USAC configurations, out of band and in band.
#[test]
fn raw_usac_in_band() {
    let mut rng = Rng::new(0x57);
    for _ in 0..800 {
        let asc = gen_usac_asc(&mut rng);
        let Some(mut s) = open_for(TT_MP4_RAW, &asc.bytes, if rng.next_u32() % 3 == 0 { 5 } else { 0 }, &mut rng) else {
            continue;
        };
        let mut data = Vec::new();
        let mut au = Vec::new();
        for _ in 0..10 {
            let l = (rng.next_u32() % 300) as usize;
            for _ in 0..l {
                data.push(rng.next_u32() as u8);
            }
            au.push(l);
        }
        run_stream_with_in_band(&mut rng, &mut s, TT_MP4_RAW, &data, &au, &asc);
        // A second out-of-band configuration (config change).
        let asc2 = if rng.next_u32() % 2 == 0 { gen_usac_asc(&mut rng) } else { asc.clone() };
        s.oob_config(&asc2.bytes, asc2.bytes.len() as u32, 0);
        in_band(&mut rng, &mut s, &asc2);
    }
}

fn gen_drm_conf(rng: &mut Rng) -> Vec<u8> {
    let mut w = BitWriter::new();
    let coding = [0, 0, 3, 3, 1, 2][(rng.next_u32() % 6) as usize];
    w.put(coding, 2);
    w.put(rng.next_u32() & 1, 1);
    w.put([0, 1, 2, 3][(rng.next_u32() % 4) as usize], 2);
    w.put(rng.next_u32() & 7, 3);
    w.put(rng.next_u32() & 3, 2);
    w.put(rng.next_u32() & 31, 5);
    w.put(rng.next_u32() & 1, 1);
    w.random_bytes(8, rng);
    w.bytes
}

#[test]
fn drm() {
    let mut rng = Rng::new(0x58);
    for _ in 0..600 {
        let conf = gen_drm_conf(&mut rng);
        let Some(mut s) = open_for(TT_DRM, &conf, if rng.next_u32() % 3 == 0 { 5 } else { 0 }, &mut rng) else {
            continue;
        };
        let mut data = Vec::new();
        let mut au = Vec::new();
        for _ in 0..12 {
            let l = (rng.next_u32() % 200) as usize;
            for _ in 0..l {
                data.push(rng.next_u32() as u8);
            }
            au.push(l);
        }
        if s.rs.asc[0].m_aot == AOT_USAC {
            run_stream_with_in_band(&mut rng, &mut s, TT_DRM, &data, &au, &BitWriter::new());
        } else {
            run_stream(&mut rng, &mut s, TT_DRM, &data, &au, true);
        }
    }
}

#[test]
fn drm_raw_sdc_audio_config_check() {
    let mut rng = Rng::new(0x59);
    for it in 0..5000 {
        let mut conf = gen_drm_conf(&mut rng);
        if rng.next_u32() % 3 == 0 {
            flip_bits(&mut conf, 2, &mut rng);
        }
        // the slice covers every byte the parser can read
        let length = if rng.next_u32() % 4 == 0 { rng.next_u32() % 4 } else { conf.len() as u32 };
        let r = transport_dec_drm_raw_sdc_audio_config_check(&conf, length);
        let e = unsafe { c::cref_tplib_drm_check(conf.as_ptr(), length) };
        assert_eq!(r, e, "DrmRawSdcAudioConfig_Check {it}");
    }
}

/// Out-of-band configurations of every kind for every transport, with random bytes and
/// generated configurations; layers 0 and 1; lengths beyond the configuration.
#[test]
fn out_of_band_configs() {
    let mut rng = Rng::new(0x5a);
    let fmts = [TT_MP4_RAW, TT_MP4_ADIF, TT_MP4_ADTS, TT_MP4_LATM_MCP1, TT_MP4_LATM_MCP0, TT_MP4_LOAS, TT_DRM];
    let configs = encoder_configs();
    for it in 0..6000 {
        let fmt = fmts[it % fmts.len()];
        let Some(mut s) = open_for(fmt, &[], if rng.next_u32() % 3 == 0 { 4 } else { 0 }, &mut rng) else {
            continue;
        };
        for _ in 0..1 + rng.next_u32() % 3 {
            let mut conf = match rng.next_u32() % 5 {
                0 => configs[(rng.next_u32() as usize) % configs.len()].0.clone(),
                1 => gen_drm_conf(&mut rng),
                2 => {
                    let asc = if rng.next_u32() % 3 == 0 { gen_usac_asc(&mut rng) } else {
                        let mut w = BitWriter::new();
                        gen_asc(&mut rng, &mut w);
                        w
                    };
                    let mut w = BitWriter::new();
                    gen_smc(&mut rng, &mut w, &asc, 1);
                    w.bytes
                }
                3 => {
                    let mut w = BitWriter::new();
                    gen_asc(&mut rng, &mut w);
                    w.bytes
                }
                _ => (0..rng.next_u32() % 40).map(|_| rng.next_u32() as u8).collect(),
            };
            if rng.next_u32() % 4 == 0 {
                flip_bits(&mut conf, 1 + rng.next_u32() % 3, &mut rng);
            }
            let length = match rng.next_u32() % 8 {
                0 => rng.next_u32() % 1100,
                1 => conf.len() as u32 / 2,
                _ => conf.len() as u32,
            };
            let layer = if rng.next_u32() % 8 == 0 { 1 } else { 0 };
            let mut padded = conf.clone();
            padded.resize(conf.len().max(length as usize), 0);
            s.oob_config(&padded, length, layer);
            s.getters();
        }
        // and a few access units of random data
        let mut data = Vec::new();
        let mut au = Vec::new();
        for _ in 0..4 {
            let l = (rng.next_u32() % 100) as usize;
            for _ in 0..l {
                data.push(rng.next_u32() as u8);
            }
            au.push(l);
        }
        run_stream(&mut rng, &mut s, fmt, &data, &au, true);
        let _ = put_aot;
        let _ = put_sf;
    }
}

/// ADTS and LOAS with the buffer fullness evaluated, average bit rate and burst period set
/// (additionalHoldOffNeeded(), the estimation of lost access units), and early config
/// discovery / delay minimization (frame traversal), on unmodified and mutated streams.
#[test]
fn stream_params() {
    let mut rng = Rng::new(0x5b);
    let unaligned_before = fdk_aac_rs_tpdec::tpdec_lib::synchronization_unaligned_searches();
    let cand: Vec<&Enc> = streams().iter().filter(|e| e.tt == TT_MP4_ADTS || e.tt == TT_MP4_LOAS).collect();
    for it in 0..(8 * cand.len()) {
        let e = cand[it % cand.len()];
        let (data, au) = if rng.next_u32() % 2 == 0 { (e.data.clone(), e.au.clone()) } else { mutate(&mut rng, e) };
        let recs = Recs::random(if rng.next_u32() % 4 == 0 { 6 } else { 0 }, &mut rng);
        let Some(mut s) = Sess::open(e.tt, TP_FLAG_MPEG4, 1, recs, rng.next_u32() % 2 == 0) else { continue };
        s.check("open");
        let ignore = (rng.next_u32() % 3 == 0) as i32;
        s.set_param(TPDEC_PARAM_IGNORE_BUFFERFULLNESS, ignore);
        // avgBitRate * samplesPerFrame stays below 2^31 (bit rates with a defined reference
        // result)
        let br = [0, 32000, 64000, 128000, 200000, 1_000_000][(rng.next_u32() % 6) as usize];
        s.set_param(TPDEC_PARAM_SET_BITRATE, br);
        let bp = [0, 10, 100, 500, 1000][(rng.next_u32() % 5) as usize];
        s.set_param(TPDEC_PARAM_BURST_PERIOD, bp);
        match rng.next_u32() % 4 {
            0 => s.set_param(TPDEC_PARAM_EARLY_CONFIG, 1),
            1 => s.set_param(TPDEC_PARAM_MINIMIZE_DELAY, 1),
            2 => {
                s.set_param(TPDEC_PARAM_EARLY_CONFIG, 1);
                s.set_param(TPDEC_PARAM_MINIMIZE_DELAY, 1);
            }
            _ => {}
        }
        let params = rng.next_u32() % 2 == 0;
        run_stream(&mut rng, &mut s, e.tt, &data, &au, params);
        if rng.next_u32() % 2 == 0 {
            // a reset in the middle, then the stream again
            s.set_param(TPDEC_PARAM_RESET, 1);
            s.set_param(TPDEC_PARAM_SET_BITRATE, br);
            run_stream(&mut rng, &mut s, e.tt, &data, &au, false);
        }
    }
    // A fixed sweep over the conditions of unaligned sync searches (LOAS, buffer fullness
    // evaluated, average bit rate and burst period set): whether the random combinations above
    // reach them depends on the encoded streams, which differ between the variants. On aarch64 a
    // high bit rate with a long burst period and the mid-stream parameter calls reach them
    // (eld-sbr-mono-loas).
    for e in cand.iter().filter(|e| e.tt == TT_MP4_LOAS) {
        for br in [32000, 128000, 1_000_000] {
            for bp in [100, 1000] {
                for params in [false, true] {
                    let Some(mut s) = Sess::open(e.tt, TP_FLAG_MPEG4, 1, Recs::random(0, &mut rng), false) else { continue };
                    s.check("open");
                    s.set_param(TPDEC_PARAM_IGNORE_BUFFERFULLNESS, 0);
                    s.set_param(TPDEC_PARAM_SET_BITRATE, br);
                    s.set_param(TPDEC_PARAM_BURST_PERIOD, bp);
                    run_stream(&mut rng, &mut s, e.tt, &e.data, &e.au, params);
                }
            }
        }
    }
    // Sync word searches at bit positions that are not byte aligned (the burst
    // buffer fullness check returns TRANSPORTDEC_SYNC_ERROR after a LOAS header without
    // rewinding it); the reference and the port continue in the same way.
    let unaligned =fdk_aac_rs_tpdec::tpdec_lib::synchronization_unaligned_searches() - unaligned_before;
    eprintln!("stream_params: {unaligned} unaligned sync searches");
    assert!(unaligned > 0);
}

/// ADTS frames with several raw data blocks and CRC protection (header CRC and raw data block
/// positions), generated (the encoder writes one raw data block per frame).
#[test]
fn adts_multiple_raw_data_blocks() {
    let mut rng = Rng::new(0x5c);
    for _ in 0..800 {
        let mut w = BitWriter::new();
        for _ in 0..10 {
            let prot = rng.next_u32() % 4 != 0;
            let nrb = if rng.next_u32() % 4 == 0 { 0 } else { 1 + rng.next_u32() % 3 };
            let sizes: Vec<u32> = (0..=nrb).map(|_| 4 + rng.next_u32() % 150).collect();
            let total: u32 = sizes.iter().sum();
            let frame_length = 7 + if prot { 2 * nrb + 2 } else { 0 } + total;
            let start = w.nbits;
            w.put(0xfff, 12);
            w.put(rng.next_u32() & 1, 1);
            w.put(0, 2);
            w.put(!prot as u32, 1);
            w.put(1, 2);
            w.put(3 + rng.next_u32() % 5, 4);
            w.put(0, 1);
            w.put(1 + rng.next_u32() % 2, 3);
            w.put(0, 4);
            w.put(frame_length, 13);
            w.put(0x7ff, 11);
            w.put(nrb, 2);
            if prot {
                let mut pos = 0;
                for i in 0..nrb as usize {
                    pos += sizes[i];
                    w.put(if rng.next_u32() % 10 == 0 { rng.next_u32() & 0xffff } else { pos }, 16);
                }
                let crc = super::tpdec_asc::crc_bits(&w, start, w.nbits, 0x8005, 0xFFFF, 16);
                w.put(if rng.next_u32() % 10 == 0 { crc as u32 ^ 1 } else { crc as u32 }, 16);
            }
            for &b in &sizes {
                w.random_bytes(b as usize, &mut rng);
            }
        }
        let mut data = w.bytes.clone();
        if rng.next_u32() % 4 == 0 {
            flip_bits(&mut data, 1 + rng.next_u32() % 3, &mut rng);
        }
        let Some(mut s) = open_for(TT_MP4_ADTS, &[], if rng.next_u32() % 4 == 0 { 6 } else { 0 }, &mut rng) else {
            continue;
        };
        run_stream(&mut rng, &mut s, TT_MP4_ADTS, &data, &[], true);
    }
}

/// Raw packets larger than the internal buffer (TRANSPORTDEC_TOO_MANY_BITS) and empty packets.
#[test]
fn raw_packet_sizes() {
    let mut rng = Rng::new(0x5d);
    for _ in 0..40 {
        let fmt = [TT_MP4_RAW, TT_DRM, TT_MP4_LATM_MCP1][(rng.next_u32() % 3) as usize];
        let conf = streams().iter().find(|e| e.tt == TT_MP4_RAW).unwrap().conf.clone();
        let Some(mut s) = open_for(fmt, &conf, 0, &mut rng) else { continue };
        for _ in 0..4 {
            let len = [0usize, 1, 32767, 32768, 32769, 40000][(rng.next_u32() % 6) as usize];
            let data: Vec<u8> = (0..len).map(|_| rng.next_u32() as u8).collect();
            run_stream(&mut rng, &mut s, fmt, &data, &[len], true);
        }
    }
}
