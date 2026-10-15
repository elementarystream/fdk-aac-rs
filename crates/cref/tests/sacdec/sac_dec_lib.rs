// Differential tests of fdk_aac_rs_sacdec::sac_dec_lib (the mpegSurroundDecoder_* API) against
// the C++ reference. The calls of the reference go through the capture functions of
// shim/sacdec_sac_dec_lib.cpp, which record their inputs and outputs (with the complete decoder
// state and the QMF domain); `replay()` makes the same call on the port and compares
// everything. The tests here call the API with random configurations, payloads and signals;
// sac_dec_lib_e2e.rs records the calls of the reference AAC decoder on real streams.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use crate::sac_dec_state::*;
use crate::sac_dec_util::*;
use fdk_aac_rs_cref::sacdec_sac_dec_lib as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::fdk_init_bit_buffer;
use fdk_aac_rs_fdk::bitstream::FdkBitStream;
use fdk_aac_rs_fdk::qmf_domain::FdkQmfDomain;
use fdk_aac_rs_fdk::syslib_channel_map_descr::{fdk_ch_map_descr_init, FdkChannelMapDescr};
use fdk_aac_rs_sacdec::sac_dec_lib::*;
use fdk_aac_rs_sacdec::sac_dec_ssc_struct::SPATIAL_SPECIFIC_CONFIG_SIZE;

pub const K_ISFULL: i32 = 1;
pub const K_OPEN: i32 = 2;
pub const K_CONFIG: i32 = 3;
pub const K_CONFQMF: i32 = 4;
pub const K_PARSE: i32 = 5;
pub const K_PARSENH: i32 = 6;
pub const K_APPLY: i32 = 7;
pub const K_SETPARAM: i32 = 8;
pub const K_GETDELAY: i32 = 9;
pub const K_FREEMEM: i32 = 10;
pub const K_CLOSE: i32 = 11;
pub const K_PSEUDOLR: i32 = 12;
pub const K_INIT: i32 = 13;
const KIND_NAMES: [&str; 14] = [
    "", "IsFull", "Open", "Config", "ConfigureQmfDomain", "Parse", "ParseNoHeader", "Apply", "SetParam",
    "GetDelay", "FreeMem", "Close", "IsPseudoLR", "Init",
];

/// The offset of the padding byte of SPATIAL_SPECIFIC_CONFIG (not compared).
const SSC_PAD_OFFSET: usize = 171;

/// A recorded call: its kind and arrays (see the shim).
pub struct Rec {
    pub kind: i32,
    pub a: Vec<Vec<i32>>,
}

/// Takes the records of the calling thread's log.
pub fn take_log() -> Vec<Rec> {
    let n = unsafe { c::cref_saclib_log_len() } as usize;
    let mut v = vec![0i32; n];
    unsafe {
        c::cref_saclib_log_read(v.as_mut_ptr());
        c::cref_saclib_log_clear();
    }
    let mut recs = Vec::new();
    let mut p = 0;
    while p < n {
        assert_eq!(v[p], 0x5ACD, "log record header");
        let kind = v[p + 1];
        let narr = v[p + 2] as usize;
        p += 3;
        let mut a = Vec::with_capacity(narr);
        for _ in 0..narr {
            let len = v[p] as usize;
            a.push(v[p + 1..p + 1 + len].to_vec());
            p += 1 + len;
        }
        recs.push(Rec { kind, a });
    }
    recs
}

fn put_ssc(o: &mut Vec<i32>, ssc: &fdk_aac_rs_sacdec::sac_dec_ssc_struct::SpatialSpecificConfig) {
    let mut b: [u8; SPATIAL_SPECIFIC_CONFIG_SIZE] = *ssc.as_bytes();
    b[SSC_PAD_OFFSET] = 0;
    o.extend(b.iter().map(|&x| x as i32));
}

/// The complete state of a decoder instance in the layout of dump_lib() of the shim.
pub fn dump_lib(d: Option<&CMpegSurroundDecoder>) -> Vec<i32> {
    let Some(d) = d else { return vec![0] };
    let mut o = vec![1];
    o.extend(d.mps_data.iter().map(|&b| b as i32));
    o.push(d.mps_data_bits);
    put_ssc(&mut o, &d.spatial_specific_config[0]);
    put_ssc(&mut o, &d.spatial_specific_config_backup);
    o.push(d.mpeg_surround_decoder_level as i32);
    o.push(d.mpeg_surround_ssc_is_global_cfg as i32);
    o.push(d.mpeg_surround_use_time_interface as i32);
    o.extend(dump_frame(&d.bs_frames[0], &d.ll_state));
    o.push(d.bs_frame_parse as i32);
    o.push(d.bs_frame_decode as i32);
    o.push(d.bs_frame_delay as i32);
    let p = &d.mpeg_surround_user_params;
    o.extend([
        p.output_mode as i32,
        p.blind_enable as i32,
        p.bypass_mode as i32,
        p.conceal_method as i32,
        p.conceal_num_keep_frames as i32,
        p.conceal_fade_out_slope_length as i32,
        p.conceal_fade_in_slope_length as i32,
        p.conceal_num_release_frames as i32,
    ]);
    o.push(d.upmix_type);
    o.push(d.init_flags[0]);
    o.push(d.anc_start_stop_prev);
    o.push(d.f_on_sync[0]);
    let c = &d.dec_config;
    o.extend([c.decoder_level, c.decoder_mode, c.max_num_output_channels, c.binaural_mode]);
    o.push(d.p_spatial_dec.is_some() as i32);
    if let Some(sd) = d.p_spatial_dec.as_deref() {
        // pConfigCurrent is &spatialSpecificConfig[bsFrameDecode] once set
        o.push(sd.p_config_current as i32);
        o.extend(dump_self(sd));
    }
    o
}

/// Names the position `i` of a dump of dump_lib().
fn lib_member_at(i: usize, frame_len: usize) -> String {
    let parts: [(&str, usize); 16] = [
        ("present", 1),
        ("mpsData", MPS_DATA_BUFFER_SIZE),
        ("mpsDataBits", 1),
        ("spatialSpecificConfig[0]", SPATIAL_SPECIFIC_CONFIG_SIZE),
        ("spatialSpecificConfigBackup", SPATIAL_SPECIFIC_CONFIG_SIZE),
        ("level, sscIsGlobalCfg, useTimeInterface", 3),
        ("bsFrames[0] and llState", frame_len),
        ("bsFrameParse, bsFrameDecode, bsFrameDelay", 3),
        ("mpegSurroundUserParams", 8),
        ("upmixType", 1),
        ("initFlags", 1),
        ("ancStartStopPrev", 1),
        ("fOnSync", 1),
        ("decConfig", 4),
        ("pSpatialDec != NULL", 1),
        ("pConfigCurrent", 1),
    ];
    let mut off = 0;
    for (name, len) in parts {
        if i < off + len {
            return format!("{name}[{}]", i - off);
        }
        off += len;
    }
    format!("pSpatialDec: {}", member_at(i - off))
}

fn cmp(what: &str, name: &str, port: &[i32], c: &[i32]) {
    if port != c {
        let i = port.iter().zip(c).position(|(a, b)| a != b).unwrap_or(port.len().min(c.len()));
        let lo = i.saturating_sub(2);
        panic!(
            "{what}: {name} differs at {i}: port {:?} C {:?} (lengths {} {})",
            &port[lo..(i + 6).min(port.len())],
            &c[lo..(i + 6).min(c.len())],
            port.len(),
            c.len()
        );
    }
}

fn cmp_lib(what: &str, d: Option<&CMpegSurroundDecoder>, c: &[i32]) {
    let r = dump_lib(d);
    if r != c {
        let i = r.iter().zip(c).position(|(a, b)| a != b).unwrap_or(r.len().min(c.len()));
        let frame_len = dump_frame(&Default::default(), &Default::default()).len();
        let lo = i.saturating_sub(2);
        panic!(
            "{what}: decoder state differs at {i} ({}): port {:?} C {:?} (lengths {} {})",
            lib_member_at(i, frame_len),
            &r[lo..(i + 6).min(r.len())],
            &c[lo..(i + 6).min(c.len())],
            r.len(),
            c.len()
        );
    }
}

/// A bitstream in the recorded state (`s`: the shim's 9 values) over a copy of the recorded
/// buffer.
struct RecBs {
    _buf: Box<[u8]>,
    bs: FdkBitStream,
}
fn rec_bs(s: &[i32], bytes: &[i32]) -> RecBs {
    let buf: Box<[u8]> = bytes.iter().map(|&b| b as u8).collect();
    let mut bs = FdkBitStream::default();
    fdk_init_bit_buffer(&mut bs.h_bit_buf, &BitMem::from_slice(&buf), s[6] as u32, 0);
    bs.cache_word = s[0] as u32;
    bs.bits_in_cache = s[1] as u32;
    bs.h_bit_buf.valid_bits = s[2] as u32;
    bs.h_bit_buf.read_offset = s[3] as u32;
    bs.h_bit_buf.write_offset = s[4] as u32;
    bs.h_bit_buf.bit_ndx = s[5] as u32;
    bs.config_cache = s[8] as u32;
    RecBs { _buf: buf, bs }
}
fn bs_state(bs: &FdkBitStream) -> Vec<i32> {
    vec![
        bs.cache_word as i32,
        bs.bits_in_cache as i32,
        bs.h_bit_buf.valid_bits as i32,
        bs.h_bit_buf.read_offset as i32,
        bs.h_bit_buf.write_offset as i32,
        bs.h_bit_buf.bit_ndx as i32,
        bs.config_cache as i32,
    ]
}
fn c_bs_state(s: &[i32]) -> Vec<i32> {
    vec![s[0], s[1], s[2], s[3], s[4], s[5], s[8]]
}

/// The port's side of a replay: the decoder handle and the QMF domain.
#[derive(Default)]
pub struct Port {
    pub h: Option<Box<CMpegSurroundDecoder>>,
    pub qd: FdkQmfDomain,
    /// Calls per kind.
    pub calls: [usize; 14],
    /// Apply calls with output (result 0).
    pub applied: usize,
    /// Nonzero output samples of these calls.
    pub nonzero: usize,
    /// Parse calls with result 0.
    pub parsed: usize,
}

fn load_qd(qd: &mut FdkQmfDomain, v: &[i32]) {
    if !v.is_empty() {
        load_qd_full(qd, v);
    }
}
fn cmp_qd(what: &str, qd: &FdkQmfDomain, v: &[i32]) {
    if !v.is_empty() {
        cmp(what, "QMF domain", &dump_qd(qd), v);
    }
}

/// Makes the recorded call on the port and compares the results and states.
pub fn replay(p: &mut Port, r: &Rec, what: &str) {
    let a = &r.a;
    let what = format!("{what}: {}", KIND_NAMES.get(r.kind as usize).unwrap_or(&"?"));
    let what = what.as_str();
    p.calls[r.kind as usize] += 1;
    match r.kind {
        K_ISFULL => {
            let d = p.h.as_deref().expect("IsFull without a handle");
            assert_eq!(mpeg_surround_decoder_is_full_mpeg_surround_decoder_instance_available(d), a[0][0], "{what}");
        }
        K_OPEN => {
            assert_eq!(p.h.is_some() as i32, a[0][0], "{what}: handle");
            let ret = mpeg_surround_decoder_open(&mut p.h, a[0][1], None);
            assert_eq!(ret, a[1][0], "{what}: result");
            cmp_lib(what, p.h.as_deref(), &a[2]);
        }
        K_INIT => {
            load_qd(&mut p.qd, &a[0]);
            let d = p.h.as_deref_mut().expect("Init without a handle");
            let ret = mpeg_surround_decoder_init(d, &mut p.qd);
            assert_eq!(ret, a[1][0], "{what}: result");
            cmp_lib(what, p.h.as_deref(), &a[2]);
            cmp_qd(what, &p.qd, &a[3]);
        }
        K_CONFIG => {
            let i = &a[0];
            let mut bs = rec_bs(&a[1], &a[2]);
            load_qd(&mut p.qd, &a[3]);
            let d = p.h.as_deref_mut().expect("Config without a handle");
            let mut cc = i[8] as u8;
            let ret = mpeg_surround_decoder_config(d, &mut bs.bs, i[0], i[1], i[2], i[3], i[4], i[5], i[6], i[7] as u8, &mut cc);
            assert_eq!([ret, cc as i32], [a[4][0], a[4][1]], "{what}: result, configChanged");
            cmp(what, "bitstream", &bs_state(&bs.bs), &c_bs_state(&a[5]));
            cmp_lib(what, p.h.as_deref(), &a[6]);
            cmp_qd(what, &p.qd, &a[7]);
        }
        K_CONFQMF => {
            let i = &a[0];
            load_qd(&mut p.qd, &a[1]);
            let ret = mpeg_surround_decoder_configure_qmf_domain(p.h.as_deref_mut(), &mut p.qd, i[0], i[1] as u32, i[2]);
            assert_eq!(ret, a[2][0], "{what}: result");
            cmp_lib(what, p.h.as_deref(), &a[3]);
            cmp_qd(what, &p.qd, &a[4]);
        }
        K_PARSE | K_PARSENH => {
            let i = &a[0];
            let mut bs = rec_bs(&a[1], &a[2]);
            let mut bits = i[0];
            let ret = if r.kind == K_PARSE {
                mpeg_surround_decoder_parse(p.h.as_deref_mut(), &mut bs.bs, &mut bits, i[1], i[2], i[3], i[4])
            } else {
                mpeg_surround_decoder_parse_no_header(p.h.as_deref_mut(), Some(&mut bs.bs), &mut bits, i[1])
            };
            assert_eq!([ret, bits], [a[3][0], a[3][1]], "{what}: result, mpsDataBits");
            p.parsed += (ret == 0) as usize;
            cmp(what, "bitstream", &bs_state(&bs.bs), &c_bs_state(&a[4]));
            cmp_lib(what, p.h.as_deref(), &a[5]);
        }
        K_APPLY => {
            let i = &a[0];
            let mut input = a[1].clone();
            let mut time = a[2].clone();
            let mut ct = a[3].clone();
            let mut ci: Vec<u8> = a[4].iter().map(|&x| x as u8).collect();
            load_qd(&mut p.qd, &a[5]);
            let (mut nch, mut fs) = (i[2], i[3]);
            let mut headroom = 0;
            let mut md = FdkChannelMapDescr::default();
            if i[7] >= 0 {
                fdk_ch_map_descr_init(&mut md, None, 0, i[7] as u32);
                assert_eq!(md.map_info_tab_len as i32, i[8], "{what}: mapInfoTabLen");
            }
            let ret = mpeg_surround_decoder_apply(
                p.h.as_deref_mut(),
                &mut p.qd,
                &mut input,
                &mut time,
                i[0],
                i[1],
                &mut nch,
                &mut fs,
                i[4],
                i[5],
                if i[9] != 0 { None } else { Some(&mut ct[..]) },
                if i[10] != 0 { None } else { Some(&mut ci[..]) },
                if i[7] >= 0 { Some(&md) } else { None },
                i[6],
                &mut headroom,
            );
            let what = format!("{what} (results port {:?} C {:?}) {:?}", [ret, nch, fs, headroom], a[6], a[0]);
            let what = what.as_str();
            // The decoder passes its WorkBufferCore5 (65536 values, FDKaalloc) with the size
            // GetRequiredMemWorkBufferCore5() / 4 = 65540, which counts the alignment overhead:
            // only the first 65536 + sizeof(void *) / 4 values have defined contents and are
            // compared.
            let n = if i[0] == 65540 { 65536 + core::mem::size_of::<usize>() / 4 } else { time.len() };
            cmp(what, "time data", &time[..n.min(time.len())], &a[8][..n.min(a[8].len())]);
            cmp_lib(what, p.h.as_deref(), &a[11]);
            cmp_qd(what, &p.qd, &a[12]);
            cmp(what, "input", &input, &a[7]);
            cmp(what, "channelType", &ct, &a[9]);
            cmp(what, "channelIndices", &ci.iter().map(|&x| x as i32).collect::<Vec<_>>(), &a[10]);
            assert_eq!([ret, nch, fs, headroom], [a[6][0], a[6][1], a[6][2], a[6][3]], "{what}: result, nChannels, frameSize, outDataHeadroom");
            if ret == 0 {
                p.applied += 1;
                p.nonzero += time[..(nch.max(0) * fs.max(0)) as usize].iter().filter(|&&v| v != 0).count();
            }
        }
        K_SETPARAM => {
            let i = &a[0];
            let h = if i[2] != 0 { p.h.as_deref_mut() } else { None };
            let ret = mpeg_surround_decoder_set_param(h, i[0], i[1]);
            assert_eq!(ret, a[1][0], "{what} ({:#x}, {}): result", i[0], i[1]);
            cmp_lib(what, if i[2] != 0 { p.h.as_deref() } else { None }, &a[2]);
        }
        K_GETDELAY => {
            let h = if a[0][1] != 0 { p.h.as_deref() } else { None };
            assert_eq!(mpeg_surround_decoder_get_delay(h) as i32, a[0][0], "{what}");
        }
        K_FREEMEM => {
            let ret = mpeg_surround_decoder_free_mem(p.h.as_deref_mut());
            assert_eq!(ret, a[0][0], "{what}: result");
            cmp_lib(what, p.h.as_deref(), &a[1]);
        }
        K_CLOSE => {
            mpeg_surround_decoder_close(p.h.take());
        }
        K_PSEUDOLR => {
            let h = if a[0][2] != 0 { p.h.as_deref() } else { None };
            let mut v = 0;
            let ret = mpeg_surround_decoder_is_pseudo_lr(h, &mut v);
            assert_eq!(ret, a[0][0], "{what}: result");
            if ret == 0 {
                assert_eq!(v, a[0][1], "{what}: bsPseudoLr");
            }
        }
        k => panic!("{what}: unknown record kind {k}"),
    }
}

/// Replays the records of the calling thread's log.
pub fn replay_log(p: &mut Port, what: &str) -> usize {
    let recs = take_log();
    for (k, r) in recs.iter().enumerate() {
        replay(p, r, &format!("{what} #{k}"));
    }
    recs.len()
}

// ---------------------------------------------------------------------------------------------
// Random API sessions
// ---------------------------------------------------------------------------------------------

struct CLib(*mut core::ffi::c_void);
impl Drop for CLib {
    fn drop(&mut self) {
        unsafe { c::cref_saclib_free(self.0) };
    }
}

/// A random configuration as the AAC decoder passes it to mpegSurroundDecoder_Config(), with
/// the frame sizes of the decoder.
struct ConfigCall {
    data: Vec<u8>,
    aot: i32,
    /// samplingRate and frameSize of mpegSurroundDecoder_Config()
    fs: i32,
    cfg_frame: i32,
    sci: i32,
    csfli: i32,
    cfg_bytes: i32,
    /// aacSamplesPerFrame (the QMF time slots of the time interface, timeDataFrameSize)
    core_frame: i32,
    /// the frameSize of mpegSurroundDecoder_Apply()
    apply_frame: i32,
    /// a parameter was made inconsistent: no frames are applied (the reference has no defined
    /// result then)
    perturbed: bool,
}

fn random_config_call(rng: &mut Rng) -> ConfigCall {
    let mut data = random_bytes(rng, 32);
    let mut c = if rng.next_u32() % 2 == 0 {
        let fsi: u32 = [3, 4, 5, 6, 7, 8, 11][rng.next_u32() as usize % 7];
        let fs = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000][fsi as usize];
        let frame = [512, 480][rng.next_u32() as usize % 2];
        let qmf = if fs < 27713 { 32 } else { 64 };
        // bsFrameLength: mostly the slots of the frame
        let ts = if rng.next_u32() % 4 == 0 { rng.range(0, 31) as u32 } else { (frame / qmf - 1) as u32 };
        let mut bits: u64 = 0;
        let mut n = 0;
        let mut put = |v: u32, w: u32| {
            bits = (bits << w) | (v as u64 & ((1u64 << w) - 1));
            n += w;
        };
        put(fsi, 4);
        put(ts, 5);
        put(rng.range(1, 7) as u32, 3);
        put(if rng.next_u32() % 16 == 0 { rng.range(0, 15) as u32 } else { 7 }, 4);
        put(if rng.next_u32() % 4 == 0 { rng.range(0, 3) as u32 } else { 0 }, 2);
        put((rng.next_u32() % 6 == 0) as u32, 1);
        put(rng.range(0, 7) as u32, 3);
        put(rng.range(0, 3) as u32, 2);
        put(rng.range(0, 3) as u32, 2);
        put(rng.next_u32() & 1, 1);
        let bits = bits << (64 - n);
        data[..8].copy_from_slice(&bits.to_be_bytes());
        if rng.next_u32() % 4 != 0 {
            for b in data[4..].iter_mut() {
                *b = 0;
            }
        }
        let cfg_bytes = [4, 5, 32, rng.range(0, 40)][rng.next_u32() as usize % 4];
        ConfigCall {
            data: Vec::new(),
            aot: AOT_ER_AAC_ELD,
            fs,
            cfg_frame: frame,
            sci: 0,
            csfli: 0,
            cfg_bytes,
            core_frame: frame,
            apply_frame: frame,
            perturbed: false,
        }
    } else {
        let sci = rng.range(1, 3);
        // stereoConfigIndex 3 comes with SBR (coreSbrFrameLengthIndex 2..4, the indices with a
        // defined reference result in mpegSurroundDecoder_GetNrOfQmfBands()).
        let csfli = rng.range(2, 4);
        data[0] = (data[0] & 0x1f) | ((rng.range(1, 7) as u8) << 5);
        if (data[0] >> 1) & 3 == 3 && rng.next_u32() % 4 != 0 {
            data[0] &= !2;
        }
        let core = if csfli == 2 { 768 } else { 1024 };
        let out = if csfli == 4 { 4096 } else { 2048 };
        let fs = [44100, 48000, 32000, 24000, 16000][rng.next_u32() as usize % 5];
        // stereoConfigIndex 3: the MPEG Surround decoder works at the core rate (the time
        // interface with 24, 32 or 16 bands); else at the output rate with 64 bands.
        let (core_frame, apply_frame) = if sci == 3 { (core, core) } else { (out, out) };
        ConfigCall {
            data: Vec::new(),
            aot: AOT_USAC,
            fs,
            cfg_frame: out,
            sci,
            csfli,
            cfg_bytes: 0,
            core_frame,
            apply_frame,
            perturbed: false,
        }
    };
    c.data = data;
    match rng.next_u32() % 32 {
        0 => c.cfg_frame = rng.range(-1, 4096),
        1 => c.fs = rng.range(-1, 96000),
        2 => c.sci = rng.range(-1, 4),
        3 => c.csfli = rng.range(-1, 5),
        _ => return c,
    }
    c.perturbed = true;
    c
}

/// A random MPEG Surround payload: LD ancillary data (with the 4 header bits: ancType,
/// ancStart, ancStop) or a USAC SpatialFrame.
fn random_payload(rng: &mut Rng, ld: bool) -> (Vec<u8>, i32) {
    let n = rng.range(1, 60) as usize;
    let mut v = random_bytes(rng, n);
    if ld {
        // ancType: mostly 0 (frame) or 1 (header and frame); start/stop mostly set
        let t = [0u8, 1, 0, 1, rng.range(0, 3) as u8][rng.next_u32() as usize % 5];
        let ss = if rng.next_u32() % 4 == 0 { rng.range(0, 3) as u8 } else { 3 };
        v[0] = (t << 6) | (ss << 4) | (v[0] & 0xf);
        let bits = [n as i32 * 8 - 4, n as i32 * 8, rng.range(-8, 8 * n as i32)][rng.next_u32() as usize % 3];
        (v, bits)
    } else {
        let bits = [n as i32 * 8, rng.range(0, 8 * n as i32), rng.range(-8, 8 * n as i32 + 8)][rng.next_u32() as usize % 3];
        (v, bits)
    }
}

#[test]
fn sac_dec_lib_api_random() {
    let mut rng = Rng::new(0x11b);
    let mut stats = [0usize; 3];
    let mut applied = 0;
    for iter in 0..500 {
        let ci = CLib(unsafe { c::cref_saclib_new() });
        let mut p = Port::default();
        unsafe { c::cref_saclib_capture(1) };
        let what = format!("session {iter}");
        let h = ci.0;
        // open (the first call allocates the structure, the second the spatial decoder)
        for _ in 0..2 {
            let sci = [-1, 0, 1, 2, 3][rng.next_u32() as usize % 5];
            unsafe { c::cref_saclib_open(h, sci) };
            replay_log(&mut p, &format!("{what} open"));
        }
        unsafe { c::cref_saclib_is_full(h) };
        replay_log(&mut p, &what);
        let cc = random_config_call(&mut rng);
        let (aot, fs, sci) = (cc.aot, cc.fs, cc.sci);
        let ld = aot == AOT_ER_AAC_ELD;
        for _ in 0..rng.range(0, 3) {
            let param = [
                SACDEC_OUTPUT_MODE,
                SACDEC_BLIND_ENABLE,
                SACDEC_PARTIALLY_COMPLEX,
                SACDEC_INTERFACE,
                SACDEC_BS_DELAY,
                SACDEC_BINAURAL_QUALITY,
                SACDEC_BS_INTERRUPTION,
                SACDEC_CLEAR_HISTORY,
                SACDEC_CONCEAL_NUM_KEEP_FRAMES,
                SACDEC_CONCEAL_FADE_OUT_SLOPE_LENGTH,
                SACDEC_CONCEAL_FADE_IN_SLOPE_LENGTH,
                SACDEC_CONCEAL_NUM_RELEASE_FRAMES,
                0x0303 + 0x10,
                rng.range(0, 0x400),
            ][rng.next_u32() as usize % 14];
            let mut value = [0, 1, 2, rng.range(-3, 300), rng.i32()][rng.next_u32() as usize % 5];
            if param == SACDEC_INTERFACE && value == SAC_INTERFACE_QMF {
                // the QMF interface needs the SBR decoder (the end-to-end tests)
                value = SAC_INTERFACE_TIME;
            }
            unsafe { c::cref_saclib_set_param(h, param, value, (rng.next_u32() % 16 == 0) as i32) };
            replay_log(&mut p, &format!("{what} setParam"));
        }
        // configuration: detection mode first (as the AAC decoder), then the real one
        let mut cfg_ok = false;
        // the number of core channels (the downmix, and the residual of stereoConfigIndex > 1)
        let n_ch = if rng.next_u32() % 16 == 0 { rng.range(0, 3) } else if ld { [1, 0][rng.next_u32() as usize % 2] } else { 1 + (sci > 1) as i32 };
        for mode in [1i32, [0, 2][rng.next_u32() as usize % 2]] {
            let mut buf = cc.data.clone();
            let valid = if rng.next_u32() % 8 == 0 { rng.range(0, 256) } else { 256 } as u32;
            let r = unsafe {
                c::cref_saclib_config(h, buf.as_mut_ptr(), 32, valid, aot, fs, cc.cfg_frame, n_ch, sci, cc.csfli, cc.cfg_bytes, mode, 0)
            };
            replay_log(&mut p, &format!("{what} config mode {mode}"));
            stats[0] += (r == 0) as usize;
            cfg_ok = r == 0;
        }
        // The QMF domain: LD uses the time interface (mpegSurroundDecoder_ConfigureQmfDomain()
        // and the slots of the core frame); USAC with stereoConfigIndex 3 the domain that the
        // AAC decoder configures explicitly; USAC with stereoConfigIndex 1 or 2 the QMF
        // interface, which needs the SBR decoder (the end-to-end tests): no frames are applied.
        let r = if ld {
            let r = unsafe { c::cref_saclib_configure_qmf_domain(h, SAC_INTERFACE_TIME, fs as u32, aot, cc.core_frame) };
            replay_log(&mut p, &format!("{what} configureQmfDomain"));
            r
        } else if sci == 3 {
            let sbr_ratio_index = [0, 0, 2, 3, 1][cc.csfli as usize];
            let mut gc = [0i32; 21];
            gc[0] = 1; // qmfDomainExplicitConfig
            gc[2] = 2; // nInputChannels_requested (ascChannels)
            gc[4] = 2; // nOutputChannels_requested
            gc[5] = 1; // parkChannel
            gc[6] = 1; // parkChannel_requested
            gc[10] = [16, 24, 32][sbr_ratio_index - 1]; // nBandsAnalysis_requested
            gc[12] = 64; // nBandsSynthesis_requested
            gc[14] = if sbr_ratio_index == 1 { 64 } else { 32 }; // nQmfTimeSlots_requested
            gc[16] = if sbr_ratio_index == 1 { 12 } else { 6 }; // nQmfOvTimeSlots_requested
            gc[18] = 64; // nQmfProcBands_requested
            gc[20] = 1; // nQmfProcChannels_requested
            unsafe { c::cref_saclib_configure_qd(h, gc.as_ptr()) }
        } else {
            -1
        };
        // The AAC decoder applies only with a configuration and a configured QMF domain.
        let n_frames = if cfg_ok && !cc.perturbed { rng.range(1, 12) } else { 0 };
        let can_apply = r == 0;
        let n_in = if ld || sci < 2 { 1 } else { 2 };
        for f in 0..n_frames {
            let wf = format!("{what} frame {f}");
            match rng.next_u32() % 8 {
                0 => {
                    let param = [SACDEC_BS_INTERRUPTION, SACDEC_CLEAR_HISTORY, SACDEC_OUTPUT_MODE][rng.next_u32() as usize % 3];
                    unsafe { c::cref_saclib_set_param(h, param, rng.range(0, 2), 0) };
                }
                1 => unsafe {
                    c::cref_saclib_get_delay(h);
                    let mut v = 0;
                    c::cref_saclib_is_pseudo_lr(h, &mut v);
                },
                _ => {}
            }
            replay_log(&mut p, &wf);
            if rng.next_u32() % 8 != 0 {
                let (mut payload, mut bits) = random_payload(&mut rng, ld);
                let len = payload.len();
                payload.resize(128, 0);
                let valid = if bits > 0 { (bits as u32).min(128 * 8) } else { 0 };
                let r = unsafe {
                    if ld {
                        c::cref_saclib_parse(h, payload.as_mut_ptr(), 128, valid, &mut bits, aot, fs, cc.core_frame, rng.range(0, 1))
                    } else {
                        c::cref_saclib_parse_no_header(h, payload.as_mut_ptr(), 128, valid.min(8 * len as u32), &mut bits, rng.range(0, 1))
                    }
                };
                replay_log(&mut p, &format!("{wf} parse"));
                stats[1] += (r == 0) as usize;
            }
            if !can_apply {
                continue;
            }
            // apply: time interface input (stereoConfigIndex > 1: the input is in the output buffer)
            let n_out = 2 * cc.core_frame.max(cc.apply_frame) as usize;
            let mut input: Vec<i32> = (0..8192).map(|_| rng.next_u32() as i32 >> 8).collect();
            let tsize = if rng.next_u32() % 16 == 0 { rng.range(0, n_out as i32) } else { (n_out + 64) as i32 };
            let mut time = vec![0x5a5a; tsize.max(0) as usize];
            for (k, v) in time.iter_mut().enumerate().take(n_in * cc.core_frame as usize) {
                *v = if k % 3 == 0 { sig(&mut rng) } else { (rng.next_u32() as i32) >> 12 };
            }
            let nch = if rng.next_u32() % 8 == 0 { rng.range(0, 3) } else { n_in as i32 };
            let mut io = [nch, cc.apply_frame, 0];
            let mut ct = [0i32; 8];
            let mut chi = [0u8; 8];
            let r = unsafe {
                c::cref_saclib_apply(
                    h,
                    input.as_mut_ptr(),
                    time.as_mut_ptr(),
                    tsize,
                    cc.core_frame,
                    io.as_mut_ptr(),
                    fs,
                    aot,
                    ct.as_mut_ptr(),
                    chi.as_mut_ptr(),
                    (rng.next_u32() % 8 == 0) as i32,
                    [0, 1, 3, 8][rng.next_u32() as usize % 4],
                )
            };
            replay_log(&mut p, &format!("{wf} apply"));
            stats[2] += (r == 0) as usize;
        }
        if rng.next_u32() % 4 == 0 {
            unsafe { c::cref_saclib_free_mem(h) };
            replay_log(&mut p, &format!("{what} freeMem"));
        }
        unsafe { c::cref_saclib_close(h) };
        replay_log(&mut p, &format!("{what} close"));
        unsafe { c::cref_saclib_capture(0) };
        applied += p.applied;
    }
    // configurations, parses and applies that succeeded
    assert!(stats[0] > 150 && stats[1] > 80 && stats[2] > 200, "{stats:?}");
    assert!(applied > 200, "{applied}");
}

#[test]
fn sac_dec_lib_check_time_slots_and_lib_info() {
    let mut rng = Rng::new(0x11c);
    for _ in 0..20000 {
        let fl = [512, 480, 1024, 2048, 960, 4096, rng.range(-10, 5000)][rng.next_u32() as usize % 7];
        let qb = [32, 64, 16, 24, rng.range(-1, 128)][rng.next_u32() as usize % 5];
        let ts = [8, 15, 16, 32, 7, 64, rng.range(-1, 70)][rng.next_u32() as usize % 7];
        if qb == 0 {
            continue; // no defined reference result
        }
        assert_eq!(check_time_slots(fl, qb, ts), unsafe { c::cref_saclib_check_time_slots(fl, qb, ts) }, "checkTimeSlots({fl}, {qb}, {ts})");
    }
    for used in 0..(FDK_MODULE_LAST as i32 + 1) {
        for null in [0, 1] {
            let mut info = vec![LibInfo::default(); FDK_MODULE_LAST as usize];
            fdk_init_lib_info(&mut info);
            for (i, e) in info.iter_mut().enumerate().take(used as usize) {
                e.module_id = 100 + i as i32;
            }
            let r = mpeg_surround_decoder_get_lib_info(if null != 0 { None } else { Some(&mut info) });
            let mut out = vec![0i32; 3 * FDK_MODULE_LAST as usize];
            let mut strs = vec![0u8; 3 * 32 * FDK_MODULE_LAST as usize];
            let cr = unsafe { c::cref_saclib_get_lib_info(used, null, out.as_mut_ptr(), strs.as_mut_ptr()) };
            assert_eq!(r, cr, "GetLibInfo ({used}, {null})");
            for (i, e) in info.iter().enumerate() {
                assert_eq!([e.module_id, e.version, e.flags as i32], [out[3 * i], out[3 * i + 1], out[3 * i + 2]], "entry {i}");
                let s = |k: usize| {
                    let b = &strs[(3 * i + k) * 32..(3 * i + k + 1) * 32];
                    String::from_utf8_lossy(&b[..b.iter().position(|&x| x == 0).unwrap_or(32)]).to_string()
                };
                assert_eq!(e.title, s(0), "title of entry {i}");
                // The build date and time are __DATE__ and __TIME__ in C (the port's are empty).
                if e.module_id != FDK_MPSDEC {
                    assert_eq!((e.build_date, e.build_time), (s(1).as_str(), s(2).as_str()), "entry {i}");
                }
            }
        }
    }
}

/// The function-local tables of sac_dec_lib.cpp, element by element: `mapIdx2QmfBands` of
/// `mpegSurroundDecoder_GetNrOfQmfBands()` and `decConfig` of `mpegSurroundDecoder_Create()`.
#[test]
fn sac_dec_lib_function_local_tables() {
    for (i, &v) in MAP_IDX2_QMF_BANDS.iter().enumerate() {
        let csfli = i as i32 + 2;
        assert_eq!(v as i32, unsafe { c::cref_saclib_nr_of_qmf_bands_sci3(csfli) }, "mapIdx2QmfBands[{i}]");
    }
    let mut cfg = [0i32; 4];
    unsafe { c::cref_saclib_dec_config(cfg.as_mut_ptr()) };
    let d = &DEC_CONFIG;
    assert_eq!([d.decoder_level, d.decoder_mode, d.max_num_output_channels, d.binaural_mode], cfg, "decConfig");
}
