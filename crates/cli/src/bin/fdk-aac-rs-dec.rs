// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from scripts/ref-harness/fdkdec-ref.c (the reference harness of this project, not part of
// fdk-aac).

//! `fdk-aac-rs-dec`: the decoder command-line tool. It implements the interface of the reference
//! harness `fdkdec-ref` (PORTING.md, "Reference harness") on the straight-port API
//! (`fdk_aac_rs_aacdec::aacdecoder_lib`): the same options, the same call sequence, and the same
//! PCM output and JSON Lines log, byte for byte, so that `scripts/e2e.py` can compare the two.

#[path = "../harness/common.rs"]
mod common;
#[path = "../harness/dec_chunked.rs"]
mod dec_chunked;
#[path = "../harness/mp4demux.rs"]
mod mp4demux;

use std::io::Write;

use common::{
    die, open_out, parse_hex, parse_int, read_file, sha256_hex, split_at, strtok, truncate_c, Die,
    Json,
};
use fdk_aac_rs_aacdec::aacdecoder::AacDecoderInstance;
use fdk_aac_rs_aacdec::aacdecoder_lib::*;
use fdk_aac_rs_fdk::audio::{
    fdk_init_lib_info, LibInfo, FDK_MODULE_LAST, FDK_NONE, TT_DRM, TT_MP4_ADIF, TT_MP4_ADTS,
    TT_MP4_LATM_MCP0, TT_MP4_LATM_MCP1, TT_MP4_LOAS, TT_MP4_RAW,
};
use fdk_aac_rs_fdk::machine_type::IntPcm;

const DEFAULT_OUT_SIZE: i32 = 65536;
const DEFAULT_FILL_SIZE: u32 = 4096;
const DEFAULT_MAX_ERRORS: i64 = 1000;

const PARAMS: &[(&str, i32)] = &[
    ("AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE", AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE),
    ("AAC_PCM_OUTPUT_CHANNEL_MAPPING", AAC_PCM_OUTPUT_CHANNEL_MAPPING),
    ("AAC_PCM_LIMITER_ENABLE", AAC_PCM_LIMITER_ENABLE),
    ("AAC_PCM_LIMITER_ATTACK_TIME", AAC_PCM_LIMITER_ATTACK_TIME),
    ("AAC_PCM_LIMITER_RELEAS_TIME", AAC_PCM_LIMITER_RELEAS_TIME),
    ("AAC_PCM_MIN_OUTPUT_CHANNELS", AAC_PCM_MIN_OUTPUT_CHANNELS),
    ("AAC_PCM_MAX_OUTPUT_CHANNELS", AAC_PCM_MAX_OUTPUT_CHANNELS),
    ("AAC_METADATA_PROFILE", AAC_METADATA_PROFILE),
    ("AAC_METADATA_EXPIRY_TIME", AAC_METADATA_EXPIRY_TIME),
    ("AAC_CONCEAL_METHOD", AAC_CONCEAL_METHOD),
    ("AAC_DRC_BOOST_FACTOR", AAC_DRC_BOOST_FACTOR),
    ("AAC_DRC_ATTENUATION_FACTOR", AAC_DRC_ATTENUATION_FACTOR),
    ("AAC_DRC_REFERENCE_LEVEL", AAC_DRC_REFERENCE_LEVEL),
    ("AAC_DRC_HEAVY_COMPRESSION", AAC_DRC_HEAVY_COMPRESSION),
    ("AAC_DRC_DEFAULT_PRESENTATION_MODE", AAC_DRC_DEFAULT_PRESENTATION_MODE),
    ("AAC_DRC_ENC_TARGET_LEVEL", AAC_DRC_ENC_TARGET_LEVEL),
    ("AAC_UNIDRC_SET_EFFECT", AAC_UNIDRC_SET_EFFECT),
    ("AAC_UNIDRC_ALBUM_MODE", AAC_UNIDRC_ALBUM_MODE),
    ("AAC_QMF_LOWPOWER", AAC_QMF_LOWPOWER),
    ("AAC_TPDEC_CLEAR_BUFFER", AAC_TPDEC_CLEAR_BUFFER),
];

const TRANSPORTS: &[(&str, i32)] = &[
    ("raw", TT_MP4_RAW),
    ("adif", TT_MP4_ADIF),
    ("adts", TT_MP4_ADTS),
    ("latm-mcp1", TT_MP4_LATM_MCP1),
    ("latm-mcp0", TT_MP4_LATM_MCP0),
    ("loas", TT_MP4_LOAS),
    ("drm", TT_DRM),
    ("mp4", TT_MP4_RAW),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum EvKind {
    Param,
    Flags,
}

/// `Event`: an event applied at a call index (param-at, flags-at, ...).
#[derive(Clone)]
struct Event {
    at: i64,
    kind: EvKind,
    id: i64,
    value: i64,
    name: String,
}

/// The options and the state of a run (`St`).
struct St {
    /* options */
    input: Option<String>,
    pcm_path: Option<String>,
    log_path: Option<String>,
    au_sizes_path: Option<String>,
    transport: i32,
    mp4: bool,
    layers: i32,
    track: u32,
    asc: Vec<Vec<u8>>,
    fill_sizes: Vec<u32>,
    out_size: i32,
    poison: bool,
    anc_size: i32,
    decode_once: bool,
    decode_loop_opt: bool,
    free_bytes: bool,
    drain: i32,
    max_calls: i64,
    max_errors: i64,
    conceal_every: i64,
    init_params: Vec<Event>,
    events: Vec<Event>,
    /* state */
    h: Option<Box<AacDecoderInstance>>,
    j: Json,
    pcm: Option<Box<dyn Write>>,
    out: Vec<IntPcm>,
    /// the little-endian bytes of the output of a frame (reused)
    le: Vec<u8>,
    anc: bool,
    n_dec: i64,
    n_fill: i64,
    errors_in_row: i64,
    samples_total: u64,
    stop: Option<&'static str>,
    /* frame pipelining (-j N > 1) */
    threads: u32,
    /// the stream information the next decode record logs instead of aacDecoder_GetStreamInfo()
    si_override: Option<CStreamInfo>,
    /// the logical calls (calls of the sequential decoder) whose frame is pending: (call index,
    /// flags, fill calls before it)
    fifo: std::collections::VecDeque<(i64, u32, i64)>,
    /// logical calls started or answered (the call index of the sequential decoder)
    n_started: i64,
    /// the log lines written while a frame is pending (they follow its record)
    hold: Option<std::rc::Rc<std::cell::RefCell<Hold>>>,
    /* chunked parallel decoding */
    chunk: dec_chunked::ChunkArgs,
    /// the zero fills in a row of the next feed() call (after the chunked decoding)
    zero_fills: u32,
}

fn param_id(name: &str) -> Option<i64> {
    PARAMS.iter().find(|(n, _)| *n == name).map(|&(_, id)| id as i64)
}

fn param_name(id: i64) -> Option<&'static str> {
    PARAMS.iter().find(|&&(_, i)| i as i64 == id).map(|&(n, _)| n)
}

/// `parse_param(s, at)`: NAME=VALUE, NAME being a header name or a numeric parameter ID.
fn parse_param(s: &str, at: i64) -> Result<Event, Die> {
    let eq = match s.find('=') {
        Some(eq) if eq < 64 => eq,
        _ => return die(format!("expected NAME=VALUE: '{s}'")),
    };
    let mut name = s[..eq].to_string();
    let id = match param_id(&name) {
        Some(id) => id,
        None => {
            let id = parse_int(&name, "parameter (unknown name)")?;
            if let Some(known) = param_name(id) {
                name = known.to_string();
            }
            id
        }
    };
    let value = parse_int(&s[eq + 1..], "parameter value")?;
    Ok(Event { at, kind: EvKind::Param, id, value, name })
}

/// `parse_flags(s)`
fn parse_flags(s: &str) -> Result<u32, Die> {
    let mut f: u32 = 0;
    let buf = truncate_c(s, 128);
    for tok in strtok(buf, &['|', ',', '+']) {
        match tok {
            "CONCEAL" => f |= AACDEC_CONCEAL,
            "FLUSH" => f |= AACDEC_FLUSH,
            "INTR" => f |= AACDEC_INTR,
            "CLRHIST" => f |= AACDEC_CLRHIST,
            _ => f |= parse_int(tok, "flags")? as u32,
        }
    }
    Ok(f)
}

const USAGE: &str = "usage: fdkdec-ref -t TRANSPORT [options] INPUT
  -t, --transport T      raw|adif|adts|latm-mcp1|latm-mcp0|loas|drm|mp4 (or a number)
  --asc HEX              configuration for aacDecoder_ConfigRaw (repeat per layer)
  --asc-file FILE        same, from a binary file
  --layers N             nrOfLayers of aacDecoder_Open (default 1)
  --track ID             mp4: track ID (default: first sound track)
  --au-sizes FILE        packet input: one access unit size per line
  --fill-size N[,N...]   bytes per aacDecoder_Fill, cycled (default 4096)
  --param NAME=VALUE     aacDecoder_SetParam before decoding (repeatable)
  --param-at N:NAME=VALUE  aacDecoder_SetParam before DecodeFrame call N
  --flags-at N:FLAGS     OR FLAGS (CONCEAL|FLUSH|INTR|CLRHIST or a number) into call N
  --conceal-at N, --flush-at N, --intr-at N, --clrhist-at N
  --conceal-every N      AACDEC_CONCEAL on calls N-1, 2N-1, ...
  --drain N              N DecodeFrame calls with AACDEC_FLUSH after the input
  --decode-once          one DecodeFrame call per Fill call (then a loop at the end);
                         the default for packet input (mp4, --au-sizes), without the loop
  --decode-loop          packet input: decode until NOT_ENOUGH_BITS after each Fill
  --max-calls N          stop after N DecodeFrame calls
  --max-errors N         stop after N error returns in a row (default 1000, 0: off)
  --out-size N           timeDataSize in samples (default 65536)
  --poison-output        log writes into the output buffer beyond the returned samples
  --anc SIZE             ancillary data buffer of SIZE bytes
  --no-free-bytes        do not call aacDecoder_GetFreeBytes before each Fill
  -o, --pcm FILE         PCM output (s16le, interleaved)
  -l, --log FILE         JSON Lines log ('-': stdout)
  -j, --threads N        decoder threads (default 1; not a library call of C); with N > 1
                         the decoder returns each frame one call later, and the tool logs and
                         writes the calls in the order of the sequential decoder; with N >= 4
                         and a long input, chunked parallel decoding (the same output)
  --chunks N             chunked decoding: 0 off, N chunks (default: automatic)
  --chunk-warmup N       warm-up of a chunk in steps: fills of a stream, access units of packet
                         input (default: 48 KiB of a stream; 32 access units and 48 KiB)
  --chunk-verify A,B,..  verification points: steps (fills, access units) after a chunk start
  --chunk-mem MIB        memory limit of the decoded chunks waiting for their turn (256)
";

/// The usage error (exit code 2).
fn usage<T>() -> Result<T, Die> {
    die(USAGE.trim_end().to_string())
}

fn parse_args(args: &[String]) -> Result<St, Die> {
    let mut st = St {
        input: None,
        pcm_path: None,
        log_path: None,
        au_sizes_path: None,
        transport: -100,
        mp4: false,
        layers: 1,
        track: 0,
        asc: Vec::new(),
        fill_sizes: Vec::new(),
        out_size: DEFAULT_OUT_SIZE,
        poison: false,
        anc_size: 0,
        decode_once: false,
        decode_loop_opt: false,
        free_bytes: true,
        drain: 0,
        max_calls: 0,
        max_errors: DEFAULT_MAX_ERRORS,
        conceal_every: 0,
        init_params: Vec::new(),
        events: Vec::new(),
        h: None,
        j: Json::new(None),
        pcm: None,
        out: Vec::new(),
        le: Vec::new(),
        anc: false,
        n_dec: 0,
        n_fill: 0,
        errors_in_row: 0,
        samples_total: 0,
        stop: None,
        threads: 1,
        si_override: None,
        fifo: std::collections::VecDeque::new(),
        n_started: 0,
        hold: None,
        chunk: dec_chunked::ChunkArgs::default(),
        zero_fills: 0,
    };

    let mut i = 1;
    while i < args.len() {
        let mut a: &str = &args[i];
        let mut v: Option<&str> = None;
        if !a.starts_with('-') || a == "-" {
            if st.input.is_some() {
                return die("more than one input");
            }
            st.input = Some(a.to_string());
            i += 1;
            continue;
        }
        /* --name=value */
        let full: &str = &args[i];
        if full.as_bytes().get(1) == Some(&b'-') {
            if let Some(eq) = full.find('=') {
                if eq < 64 {
                    a = &full[..eq];
                    v = Some(&full[eq + 1..]);
                }
            }
        }
        let val = |i: &mut usize| -> Result<String, Die> {
            if let Some(v) = v {
                return Ok(v.to_string());
            }
            if *i + 1 < args.len() {
                *i += 1;
                return Ok(args[*i].clone());
            }
            die(format!("missing value for {a}"))
        };
        match a {
            "-t" | "--transport" => {
                let t = val(&mut i)?;
                for &(name, tt) in TRANSPORTS {
                    if name == t {
                        st.transport = tt;
                    }
                }
                if t == "mp4" {
                    st.mp4 = true;
                }
                if st.transport == -100 {
                    st.transport = parse_int(&t, "transport")? as i32;
                }
            }
            "--asc" | "--asc-file" => {
                let file = a == "--asc-file";
                let s = val(&mut i)?;
                if st.asc.len() == 8 {
                    return die("too many --asc");
                }
                if file {
                    st.asc.push(read_file(&s)?);
                } else {
                    st.asc.push(parse_hex(&s, "--asc")?);
                }
            }
            "--layers" => st.layers = parse_int(&val(&mut i)?, a)? as i32,
            "--track" => st.track = parse_int(&val(&mut i)?, a)? as u32,
            "--au-sizes" => st.au_sizes_path = Some(val(&mut i)?),
            "--fill-size" => {
                let s = val(&mut i)?;
                let buf = truncate_c(&s, 1024);
                st.fill_sizes.clear();
                for tok in strtok(buf, &[',']) {
                    let fs = parse_int(tok, "--fill-size")? as u32;
                    if fs == 0 {
                        return die("--fill-size must be positive");
                    }
                    st.fill_sizes.push(fs);
                }
            }
            "--param" => {
                let s = val(&mut i)?;
                st.init_params.push(parse_param(&s, -1)?);
            }
            "--param-at" => {
                let s = val(&mut i)?;
                let (at, rest) = split_at(&s, a)?;
                st.events.push(parse_param(rest, at)?);
            }
            "--flags-at" | "--conceal-at" | "--flush-at" | "--intr-at" | "--clrhist-at" => {
                let mut e =
                    Event { at: 0, kind: EvKind::Flags, id: 0, value: 0, name: String::new() };
                let s = val(&mut i)?;
                if a == "--flags-at" {
                    let (at, rest) = split_at(&s, a)?;
                    e.at = at;
                    e.value = parse_flags(rest)? as i64;
                } else {
                    e.at = parse_int(&s, a)?;
                    e.value = match a {
                        "--conceal-at" => AACDEC_CONCEAL,
                        "--flush-at" => AACDEC_FLUSH,
                        "--intr-at" => AACDEC_INTR,
                        _ => AACDEC_CLRHIST,
                    } as i64;
                }
                st.events.push(e);
            }
            "--conceal-every" => st.conceal_every = parse_int(&val(&mut i)?, a)?,
            "--drain" => st.drain = parse_int(&val(&mut i)?, a)? as i32,
            "--decode-once" => st.decode_once = true,
            "--decode-loop" => st.decode_loop_opt = true,
            "--max-calls" => st.max_calls = parse_int(&val(&mut i)?, a)?,
            "--max-errors" => st.max_errors = parse_int(&val(&mut i)?, a)?,
            "--out-size" => {
                st.out_size = parse_int(&val(&mut i)?, a)? as i32;
                if st.out_size < 0 {
                    return die("--out-size must not be negative");
                }
            }
            "--poison-output" => st.poison = true,
            "--anc" => st.anc_size = parse_int(&val(&mut i)?, a)? as i32,
            "--no-free-bytes" => st.free_bytes = false,
            "-j" | "--threads" => {
                let n = parse_int(&val(&mut i)?, a)?;
                if !(1..=1024).contains(&n) {
                    return die("-j must be 1..1024");
                }
                st.threads = n as u32;
            }
            "--chunks" => {
                let n = parse_int(&val(&mut i)?, a)?;
                if !(0..=65536).contains(&n) {
                    return die("--chunks must be 0..65536");
                }
                st.chunk.chunks = Some(n as usize);
            }
            "--chunk-warmup" => {
                let n = parse_int(&val(&mut i)?, a)?;
                if n < 0 {
                    return die("--chunk-warmup must not be negative");
                }
                st.chunk.warmup = Some(n as usize);
            }
            "--chunk-verify" => {
                let s = val(&mut i)?;
                let mut v = Vec::new();
                for tok in strtok(&s, &[',']) {
                    let n = parse_int(tok, a)?;
                    if n < 0 {
                        return die("--chunk-verify must not be negative");
                    }
                    v.push(n as usize);
                }
                st.chunk.verify = Some(v);
            }
            "--chunk-mem" => {
                let n = parse_int(&val(&mut i)?, a)?;
                if !(0..=1 << 20).contains(&n) {
                    return die("--chunk-mem must be 0..1048576");
                }
                st.chunk.mem_mib = Some(n as usize);
            }
            "-o" | "--pcm" => st.pcm_path = Some(val(&mut i)?),
            "-l" | "--log" => st.log_path = Some(val(&mut i)?),
            "-h" | "--help" => return usage(),
            _ => return die(format!("unknown option {a}")),
        }
        i += 1;
    }
    if st.input.is_none() || st.transport == -100 {
        return usage();
    }
    if st.fill_sizes.is_empty() {
        st.fill_sizes.push(DEFAULT_FILL_SIZE);
    }
    Ok(st)
}

/* ---- API calls with logging ---------------------------------------------------------------- */

fn log_lib_info(st: &mut St) {
    /* Zeroed first: drcDec_GetLibInfo does not write the flags of its entry, so they would be
     * whatever the caller's array held. */
    let mut info = [LibInfo::default(); FDK_MODULE_LAST as usize];
    fdk_init_lib_info(&mut info);
    let ret = aac_decoder_get_lib_info(Some(&mut info));
    let j = &mut st.j;
    j.begin("get_lib_info");
    j.int("ret", ret as i64);
    j.arr_begin("modules");
    for e in info.iter().take_while(|e| e.module_id != FDK_NONE) {
        j.obj_begin(None);
        j.int("module_id", e.module_id as i64);
        j.str("title", e.title);
        j.int("version", e.version as i64);
        let n = e.version_str.iter().position(|&c| c == 0).unwrap_or(e.version_str.len());
        j.str_bytes("versionStr", &e.version_str[..n]);
        j.uint("flags", e.flags as u64);
        j.obj_end();
    }
    j.arr_end();
    j.end();
}

fn set_param(st: &mut St, e: &Event) {
    let ret = aac_decoder_set_param(st.h.as_deref_mut(), e.id as i32, e.value as i32);
    let j = &mut st.j;
    j.begin("set_param");
    j.str("param", &e.name);
    j.int("id", e.id);
    j.int("value", e.value as i32 as i64);
    j.int("ret", ret as i64);
    j.end();
}

fn log_stream_info(j: &mut Json, si: &CStreamInfo) {
    let n = si.num_channels.clamp(0, 8) as usize;
    j.obj_begin(Some("info"));
    j.int("sampleRate", si.sample_rate as i64);
    j.int("frameSize", si.frame_size as i64);
    j.int("numChannels", si.num_channels as i64);
    j.arr_begin("pChannelType");
    for c in 0..n {
        j.arr_int(si.p_channel_type[c] as i64);
    }
    j.arr_end();
    j.arr_begin("pChannelIndices");
    for c in 0..n {
        j.arr_int(si.p_channel_indices[c] as i64);
    }
    j.arr_end();
    j.int("aacSampleRate", si.aac_sample_rate as i64);
    j.int("profile", si.profile as i64);
    j.int("aot", si.aot as i64);
    j.int("channelConfig", si.channel_config as i64);
    j.int("bitRate", si.bit_rate as i64);
    j.int("aacSamplesPerFrame", si.aac_samples_per_frame as i64);
    j.int("aacNumChannels", si.aac_num_channels as i64);
    j.int("extAot", si.ext_aot as i64);
    j.int("extSamplingRate", si.ext_sampling_rate as i64);
    j.uint("outputDelay", si.output_delay as u64);
    j.uint("flags", si.flags as u64);
    j.int("epConfig", si.ep_config as i64);
    j.int("numLostAccessUnits", si.num_lost_access_units as i64);
    j.int("numTotalBytes", si.num_total_bytes);
    j.int("numBadBytes", si.num_bad_bytes);
    j.int("numTotalAccessUnits", si.num_total_access_units);
    j.int("numBadAccessUnits", si.num_bad_access_units);
    j.int("drcProgRefLev", si.drc_prog_ref_lev as i64);
    j.int("drcPresMode", si.drc_pres_mode as i64);
    j.int("outputLoudness", si.output_loudness as i64);
    j.obj_end();
}

/// Poison pattern: odd, so never 0 (the decoder clears buffers with zeros).
fn poison_value(i: i32) -> IntPcm {
    ((i as u32).wrapping_mul(40503).wrapping_add(0x6A5D) | 1) as IntPcm
}

fn decode(st: &mut St, flags: u32) -> Result<AacDecoderError, Die> {
    if st.threads > 1 {
        return decode_pipelined(st, flags);
    }
    let flags = call_flags(st, flags, st.n_dec);
    poison_out(st);
    let out_size = st.out_size;
    let ret = aac_decoder_decode_frame(st.h.as_deref_mut(), &mut st.out[..], out_size, flags);
    log_decode(st, st.n_dec, flags, ret)?;
    account(st, ret);
    Ok(ret)
}

/// The flags of decode call `n` (the call index of the sequential decoder), after its
/// `--param-at` events.
fn call_flags(st: &mut St, mut flags: u32, n: i64) -> u32 {
    let events: Vec<Event> = st.events.iter().filter(|e| e.at == n).cloned().collect();
    for e in &events {
        if e.kind == EvKind::Param {
            set_param(st, e);
        } else {
            flags |= e.value as u32;
        }
    }
    if st.conceal_every > 0 && n % st.conceal_every == st.conceal_every - 1 {
        flags |= AACDEC_CONCEAL;
    }
    flags
}

/// Fills the output buffer with the poison pattern (`--poison-output`).
fn poison_out(st: &mut St) {
    if st.poison {
        for i in 0..st.out_size {
            st.out[i as usize] = poison_value(i);
        }
    }
}

/// The number of samples of a decode call's output: `frameSize * numChannels` of the stream
/// information after the call for valid output, limited to `out_size`.
fn output_samples(ret: AacDecoderError, si: Option<&CStreamInfo>, out_size: i32) -> i32 {
    let mut n: i32 = 0;
    if is_output_valid(ret) != 0 {
        if let Some(si) = si {
            let m = si.frame_size as i64 * si.num_channels as i64;
            n = if m < 0 {
                0
            } else if m > out_size as i64 {
                out_size
            } else {
                m as i32
            };
        }
    }
    n
}

/// The `poison` object of a decode record (`--poison-output`): the output buffer after the call
/// compared with the pattern written before it.
fn poison_info(out: &[IntPcm], out_size: i32, n: i32) -> [i64; 4] {
    let (mut modified, mut beyond, mut beyond_zero, mut max) = (0i64, 0i64, 0i64, -1i64);
    for i in 0..out_size {
        let x = out[i as usize];
        if x != poison_value(i) {
            modified += 1;
            if i >= n {
                beyond += 1;
                if x == 0 {
                    beyond_zero += 1;
                }
            }
            max = i as i64;
        }
    }
    [modified, beyond, beyond_zero, max]
}

/// Writes the record of a decode call.
#[allow(clippy::too_many_arguments)]
fn write_decode(
    j: &mut Json,
    call: i64,
    flags: u32,
    ret: AacDecoderError,
    n: i32,
    hash: Option<&str>,
    poison: Option<&[i64; 4]>,
    si: Option<&CStreamInfo>,
) {
    j.begin("decode_frame");
    j.int("n", call);
    j.uint("flags", flags as u64);
    j.int("ret", ret as i64);
    j.int("samples", n as i64);
    if let Some(h) = hash {
        j.str("pcm", h);
    }
    if let Some(&[modified, beyond, beyond_zero, max]) = poison {
        j.obj_begin(Some("poison"));
        j.int("modified", modified);
        j.int("beyond", beyond);
        j.int("beyond_zero", beyond_zero);
        j.int("max", max);
        j.obj_end();
    }
    match si {
        Some(si) => log_stream_info(j, si),
        None => j.null("info"),
    }
    j.end();
}

/// Writes the record of an `aacDecoder_AncDataGet()` call (`data`: the offset and the bytes of a
/// non-empty element).
fn write_anc(j: &mut Json, idx: i32, r: AacDecoderError, size: i32, data: Option<(usize, &[u8])>) {
    j.begin("anc_data_get");
    j.int("index", idx as i64);
    j.int("ret", r as i64);
    j.int("size", size as i64);
    if let Some((off, data)) = data {
        j.int("offset", off as i64);
        j.hex("data", data);
    }
    j.end();
}

/// An `aacDecoder_AncDataGet()` call: index, return code, size, offset and data of a non-empty
/// element.
type AncCall = (i32, AacDecoderError, i32, Option<(usize, Vec<u8>)>);

/// The `aacDecoder_AncDataGet()` calls after a frame with valid output, until an error, an empty
/// element or index 16.
fn anc_calls(h: Option<&AacDecoderInstance>) -> Vec<AncCall> {
    let mut v = Vec::new();
    let mut idx = 0;
    loop {
        let mut p: Option<usize> = None;
        let mut size: i32 = 0;
        let (r, buf) = match h {
            Some(h) => (
                aac_decoder_anc_data_get(h, idx, &mut p, &mut size),
                aac_decoder_anc_data_buffer(h),
            ),
            None => (AAC_DEC_INVALID_HANDLE, None),
        };
        let data = match (p, size > 0) {
            (Some(off), true) => {
                let d = buf
                    .and_then(|b| b.get(off..off.saturating_add(size as usize).min(b.len())))
                    .unwrap_or(&[]);
                Some((off, d.to_vec()))
            }
            _ => None,
        };
        v.push((idx, r, size, data));
        if r != AAC_DEC_OK || size <= 0 || idx >= 16 {
            break;
        }
        idx += 1;
    }
    v
}

/// Logs the record of decode call `st.n_dec` with `flags` and its result `ret` (the output
/// buffer, the stream information and the ancillary data are those of the result), writes its
/// PCM and updates the stop conditions.
fn log_decode(st: &mut St, call: i64, flags: u32, ret: AacDecoderError) -> Result<(), Die> {
    let out_size = st.out_size;
    let si = match st.si_override.take() {
        Some(si) => Some(si),
        None => aac_decoder_get_stream_info(st.h.as_deref()).copied(),
    };
    let valid = is_output_valid(ret) != 0;
    let n = output_samples(ret, si.as_ref(), out_size);

    let mut hash = None;
    if n > 0 {
        let le = &mut st.le;
        le.resize(n as usize * 2, 0);
        for (d, s) in le.chunks_exact_mut(2).zip(&st.out[..n as usize]) {
            d.copy_from_slice(&s.to_le_bytes());
        }
        if st.j.enabled() {
            // the hash is only logged; skipping it keeps benchmarks fair (as fdkdec-ref)
            hash = Some(sha256_hex(&st.le));
        }
    }
    let poison = st.poison.then(|| poison_info(&st.out, out_size, n));
    write_decode(&mut st.j, call, flags, ret, n, hash.as_deref(), poison.as_ref(), si.as_ref());
    if n > 0 {
        if let Some(p) = st.pcm.as_mut() {
            if p.write_all(&st.le).is_err() {
                return die("cannot write PCM");
            }
        }
        st.samples_total += n as u64;
    }
    st.n_dec += 1;

    if valid && st.anc {
        for (idx, r, size, data) in anc_calls(st.h.as_deref()) {
            write_anc(&mut st.j, idx, r, size, data.as_ref().map(|(o, d)| (*o, d.as_slice())));
        }
    }

    Ok(())
}

/// The stop condition of the error returns in a row, after the result `ret` of a decode call.
fn account(st: &mut St, ret: AacDecoderError) {
    /* Loop guard: some error states never consume input nor return AAC_DEC_NOT_ENOUGH_BITS (for
     * example an ADIF stream whose last bits are not a complete raw_data_block returns concealed
     * output with AAC_DEC_DECODE_FRAME_ERROR forever). */
    if ret == AAC_DEC_OK || ret == AAC_DEC_NOT_ENOUGH_BITS {
        st.errors_in_row = 0;
    } else {
        st.errors_in_row += 1;
        if st.max_errors > 0 && st.errors_in_row >= st.max_errors {
            st.stop = Some("errors");
        }
    }
}

/* ---- Frame pipelining (-j N > 1) ------------------------------------------------------------
 *
 * With more than one thread the decoder returns the frame of a DecodeFrame call one call later
 * (fdk_aac_rs_aacdec::pipeline::decode_frame), while AAC_DEC_NOT_ENOUGH_BITS comes at once. The
 * tool emulates the calls of the sequential decoder: decode() makes the call of the sequential
 * decoder (with its flags and events) and answers the questions the loops ask of its result;
 * the record of a call whose frame is pending is written when the frame arrives, and the log
 * lines written meanwhile (fills, a NOT_ENOUGH_BITS record, parameters) are held back and follow
 * it. A call that the sequential decoder does not make because of the error stop of the call
 * before it is made anyway (its frame was started before the stop was known); it and its log
 * lines are dropped. So the PCM and the log are those of the sequential decoder. */

/// The log lines held back while a frame is pending.
#[derive(Default)]
struct Hold {
    /// a frame is pending: lines go to `buf`
    holding: bool,
    /// the pending frame's record is being written: lines go through
    bypass: bool,
    buf: Vec<u8>,
    /// lines released after the pending frame's record (written before any later line)
    release: Vec<u8>,
    /// NOT_ENOUGH_BITS records held back: their stop accounting follows the pending frame's
    neb_accounts: u32,
}

/// The log output with [`Hold`].
struct HoldWriter {
    inner: Box<dyn Write>,
    hold: std::rc::Rc<std::cell::RefCell<Hold>>,
}

impl Write for HoldWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let mut h = self.hold.borrow_mut();
        if h.holding && !h.bypass {
            h.buf.extend_from_slice(b);
            return Ok(b.len());
        }
        if !h.release.is_empty() {
            let r = core::mem::take(&mut h.release);
            self.inner.write_all(&r)?;
        }
        self.inner.write(b)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut h = self.hold.borrow_mut();
        if !h.release.is_empty() {
            let r = core::mem::take(&mut h.release);
            self.inner.write_all(&r)?;
        }
        self.inner.flush()
    }
}

fn hold_set(st: &St, f: impl FnOnce(&mut Hold)) {
    if let Some(h) = st.hold.as_ref() {
        f(&mut h.borrow_mut());
    }
}

/// The number of DecodeFrame calls of the sequential decoder so far (for --max-calls).
fn calls(st: &St) -> i64 {
    if st.threads > 1 {
        st.n_started
    } else {
        st.n_dec
    }
}

fn pending(st: &St) -> u32 {
    st.h.as_deref().map_or(0, aac_decoder_pending_frames)
}

/// A decode call of the sequential decoder on the pipelined decoder (see above). Returns
/// AAC_DEC_NOT_ENOUGH_BITS for a call that found not enough bits, AAC_DEC_OK otherwise (its
/// result follows with a later call).
fn decode_pipelined(st: &mut St, flags: u32) -> Result<AacDecoderError, Die> {
    let call = st.n_started;
    let flags = call_flags(st, flags, call);
    poison_out(st);
    let before = pending(st);
    let out_size = st.out_size;
    let ret = aac_decoder_decode_frame(st.h.as_deref_mut(), &mut st.out[..], out_size, flags);
    let after = pending(st);
    if ret == AAC_DEC_NOT_ENOUGH_BITS && after > before {
        // The call started a frame; no earlier frame to return.
        st.fifo.push_back((call, flags, st.n_fill));
        st.n_started += 1;
        hold_set(st, |h| h.holding = true);
        return Ok(AAC_DEC_OK);
    }
    if ret == AAC_DEC_NOT_ENOUGH_BITS {
        // The call found not enough bits (as the sequential decoder). Its stream
        // information is complete when the frame being decoded meanwhile is.
        st.n_started += 1;
        if st.j.enabled() {
            if let Some(h) = st.h.as_deref_mut() {
                aac_decoder_sync_stream_info(h);
            }
        }
        log_decode(st, call, flags, ret)?;
        if st.fifo.is_empty() {
            account(st, ret);
        } else {
            hold_set(st, |h| h.neb_accounts += 1);
        }
        return Ok(ret);
    }
    // The frame of an earlier call.
    deliver(st, ret)?;
    if st.stop.is_some() {
        // (an error stop: this call is not one of the sequential decoder)
        return Ok(ret);
    }
    if after == before {
        st.fifo.push_back((call, flags, st.n_fill));
        st.n_started += 1;
        hold_set(st, |h| h.holding = true);
        return Ok(AAC_DEC_OK);
    }
    // A drain (a call that found not enough bits directly after NOT_ENOUGH_BITS without a
    // Fill returns the pending frame): the call itself is one of the sequential decoder
    // that returned NOT_ENOUGH_BITS (its output buffer cleared, the stream information of
    // the decoder after it).
    st.n_started += 1;
    let n = (st.out_size.max(0) as usize).min(st.out.len());
    st.out[..n].fill(0);
    st.si_override = st.h.as_deref().map(|h| *fdk_aac_rs_aacdec::pipeline::decoder_stream_info(h));
    log_decode(st, call, flags, AAC_DEC_NOT_ENOUGH_BITS)?;
    account(st, AAC_DEC_NOT_ENOUGH_BITS);
    Ok(AAC_DEC_NOT_ENOUGH_BITS)
}

/// Logs the frame a call returned (output buffer, stream information and ancillary data are
/// those of the frame) as the record of the oldest pending call, then the lines held back.
fn deliver(st: &mut St, ret: AacDecoderError) -> Result<(), Die> {
    let Some((call, flags, fills)) = st.fifo.pop_front() else {
        return die("frame without a pending call");
    };
    hold_set(st, |h| h.bypass = true);
    log_decode(st, call, flags, ret)?;
    hold_set(st, |h| h.bypass = false);
    account(st, ret);
    let mut nebs = 0;
    hold_set(st, |h| {
        h.holding = false;
        let b = core::mem::take(&mut h.buf);
        h.release.extend_from_slice(&b);
        nebs = core::mem::take(&mut h.neb_accounts);
    });
    if st.stop.is_some() {
        // The sequential decoder stops after this call: the calls after it are dropped, and
        // the fills made after it.
        st.fifo.clear();
        st.n_fill = fills;
        hold_set(st, |h| h.release.clear());
        return Ok(());
    }
    for _ in 0..nebs {
        account(st, AAC_DEC_NOT_ENOUGH_BITS);
    }
    Ok(())
}

/// Returns the pending frame at the end (the call of the sequential decoder that decoded it
/// has been made).
fn finish_pipeline(st: &mut St) -> Result<(), Die> {
    while !st.fifo.is_empty() {
        poison_out(st);
        let out_size = st.out_size;
        let ret = aac_decoder_drain(st.h.as_deref_mut(), &mut st.out[..], out_size);
        if ret == AAC_DEC_NOT_ENOUGH_BITS && pending(st) == 0 {
            return die("pending call without a frame");
        }
        deliver(st, ret)?;
    }
    hold_set(st, |h| {
        h.holding = false;
        let b = core::mem::take(&mut h.buf);
        h.release.extend_from_slice(&b);
    });
    Ok(())
}

fn decode_loop(st: &mut St) -> Result<(), Die> {
    loop {
        if st.max_calls > 0 && calls(st) >= st.max_calls {
            st.stop = Some("max_calls");
            return Ok(());
        }
        let ret = decode(st, 0)?;
        if st.stop.is_some() || st.decode_once || ret == AAC_DEC_NOT_ENOUGH_BITS {
            return Ok(());
        }
    }
}

/// Writes the record of an `aacDecoder_GetFreeBytes()` call.
fn write_free_bytes(j: &mut Json, ret: AacDecoderError, free: u32) {
    j.begin("get_free_bytes");
    j.int("ret", ret as i64);
    j.uint("free", free as u64);
    j.end();
}

/// Writes the record of an `aacDecoder_Fill()` call.
fn write_fill(j: &mut Json, n: i64, size: u32, ret: AacDecoderError, valid: u32) {
    j.begin("fill");
    j.int("n", n);
    j.uint("size", size as u64);
    j.int("ret", ret as i64);
    j.uint("bytesValid", valid as u64);
    j.end();
}

/// Fills `data` and returns the number of bytes consumed.
fn fill(st: &mut St, data: &[u8]) -> u32 {
    let size = data.len() as u32;
    let mut valid = [size];
    if st.free_bytes {
        let mut free_bytes: u32 = 0;
        let ret = aac_decoder_get_free_bytes(st.h.as_deref_mut(), &mut free_bytes);
        write_free_bytes(&mut st.j, ret, free_bytes);
    }
    let ret = match st.h.as_deref_mut() {
        Some(h) => aac_decoder_fill(h, &[data], &[size], &mut valid),
        None => AAC_DEC_INVALID_HANDLE,
    };
    write_fill(&mut st.j, st.n_fill, size, ret, valid[0]);
    st.n_fill += 1;
    size.wrapping_sub(valid[0])
}

/// Feeds `data` completely (or until a stop condition), decoding after each Fill.
fn feed(st: &mut St, data: &[u8], packet: bool, fill_idx: &mut u32) -> Result<(), Die> {
    let size = data.len() as u32;
    let mut pos: u32 = 0;
    let mut zero_fills = core::mem::take(&mut st.zero_fills);
    loop {
        let mut chunk = size - pos;
        if !packet {
            let fs = st.fill_sizes[(*fill_idx % st.fill_sizes.len() as u32) as usize];
            if chunk > fs {
                chunk = fs;
            }
            *fill_idx = fill_idx.wrapping_add(1);
        }
        let consumed = fill(st, &data[pos as usize..(pos + chunk) as usize]);
        pos = pos.wrapping_add(consumed);
        if chunk > 0 && consumed == 0 {
            zero_fills += 1;
            if zero_fills >= 2 {
                st.stop = Some("stall");
                return Ok(());
            }
        } else {
            zero_fills = 0;
        }
        decode_loop(st)?;
        if st.stop.is_some() || pos >= size {
            return Ok(());
        }
    }
}

/* ---- Output writer thread (-j N > 1) ------------------------------------------------------- */

enum WriterMsg {
    Data(Vec<u8>),
    Flush(std::sync::mpsc::SyncSender<std::io::Result<()>>),
}

/// An output whose bytes are written by a thread of its own (in blocks of 1 MiB, in order), so
/// that the file writes run beside the decoding. `flush()` waits until everything written so far
/// is in the file.
struct ThreadedWriter {
    buf: Vec<u8>,
    tx: std::sync::mpsc::SyncSender<WriterMsg>,
    thread: Option<std::thread::JoinHandle<()>>,
}

const WRITER_BLOCK: usize = 1 << 20;

impl ThreadedWriter {
    fn new(mut out: Box<dyn Write + Send>) -> ThreadedWriter {
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriterMsg>(4);
        let thread = std::thread::spawn(move || {
            let mut err: Option<std::io::Error> = None;
            for m in rx {
                match m {
                    WriterMsg::Data(b) => {
                        if err.is_none() {
                            if let Err(e) = out.write_all(&b) {
                                err = Some(e);
                            }
                        }
                    }
                    WriterMsg::Flush(ack) => {
                        let r = match err.take() {
                            Some(e) => Err(e),
                            None => out.flush(),
                        };
                        let _ = ack.send(r);
                    }
                }
            }
        });
        ThreadedWriter { buf: Vec::with_capacity(WRITER_BLOCK), tx, thread: Some(thread) }
    }

    fn send(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let b = core::mem::replace(&mut self.buf, Vec::with_capacity(WRITER_BLOCK));
        self.tx.send(WriterMsg::Data(b)).map_err(|_| std::io::Error::other("writer thread ended"))
    }
}

impl Write for ThreadedWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        if self.buf.len() >= WRITER_BLOCK {
            self.send()?;
        }
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send()?;
        let (ack, done) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(WriterMsg::Flush(ack))
            .map_err(|_| std::io::Error::other("writer thread ended"))?;
        done.recv().map_err(|_| std::io::Error::other("writer thread ended"))?
    }
}

impl Drop for ThreadedWriter {
    fn drop(&mut self) {
        let _ = self.flush();
        // (closing the channel ends the thread)
        let (tx, _) = std::sync::mpsc::sync_channel(1);
        drop(core::mem::replace(&mut self.tx, tx));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// `open_out()` with a writer thread.
fn open_out_threaded(path: Option<&str>) -> Result<Option<Box<dyn Write>>, Die> {
    let out: Box<dyn Write + Send> = match path {
        None => return Ok(None),
        Some("-") => Box::new(std::io::stdout()),
        Some(p) => match std::fs::File::create(p) {
            Ok(f) => Box::new(f),
            Err(_) => return die(format!("cannot create {p}")),
        },
    };
    Ok(Some(Box::new(ThreadedWriter::new(out))))
}

fn run(st: &mut St) -> Result<i32, Die> {
    let input = st.input.clone().unwrap_or_default();
    let data = read_file(&input)?;
    let log = open_out(st.log_path.as_deref())?;
    let log: Option<Box<dyn Write>> = match log {
        Some(w) if st.threads > 1 => {
            let hold = std::rc::Rc::new(std::cell::RefCell::new(Hold::default()));
            st.hold = Some(hold.clone());
            Some(Box::new(HoldWriter { inner: w, hold }))
        }
        w => w,
    };
    st.j = Json::new(log);
    st.pcm = if st.threads > 1 {
        open_out_threaded(st.pcm_path.as_deref())?
    } else {
        open_out(st.pcm_path.as_deref())?
    };
    st.out = vec![0; st.out_size as usize + 1];

    let mut au_sizes: Option<Vec<u32>> = None;
    if let Some(path) = st.au_sizes_path.clone() {
        let txt = read_file(&path)?;
        let txt = String::from_utf8_lossy(&txt).into_owned();
        // strtok_r stops at a NUL byte, as the C string ends there.
        let txt = txt.split('\0').next().unwrap_or("");
        let mut v = Vec::new();
        for line in strtok(txt, &['\r', '\n']) {
            v.push(parse_int(line, "--au-sizes entry")? as u32);
        }
        au_sizes = Some(v);
    }

    log_lib_info(st);

    let mut trk = None;
    if st.mp4 {
        match mp4demux::mp4_demux(&data, st.track) {
            Err(err) => {
                st.j.begin("demux");
                st.j.str("error", "unsupported");
                st.j.end();
                eprintln!("fdkdec-ref: {input}: {err}");
                return Ok(3);
            }
            Ok(t) => {
                st.j.begin("demux");
                st.j.uint("track", t.track_id as u64);
                st.j.str_bytes("codec", &t.codec);
                st.j.int("oti", t.oti as i64);
                st.j.hex("asc", t.asc(&data));
                st.j.uint("samples", t.samples.len() as u64);
                st.j.boolean("fragmented", t.fragmented);
                st.j.end();
                if st.asc.is_empty() && !t.asc(&data).is_empty() {
                    st.asc.push(t.asc(&data).to_vec());
                }
                trk = Some(t);
            }
        }
    }

    st.h = aac_decoder_open(st.transport, st.layers as u32);
    st.j.begin("open");
    st.j.int("transport", st.transport as i64);
    st.j.int("layers", st.layers as i64);
    st.j.boolean("ok", st.h.is_some());
    st.j.end();
    if st.h.is_none() {
        eprintln!("fdkdec-ref: aacDecoder_Open failed");
        return Ok(1);
    }
    // (the chunked decoding sets the threads after its part: its decoders are single-threaded)
    let chunk_args = st.chunk.clone();
    let chunked = dec_chunked::eligible(st, &chunk_args);
    if st.threads > 1 && !chunked {
        if let Some(h) = st.h.as_deref_mut() {
            aac_decoder_set_threads(h, st.threads);
        }
    }

    if !st.asc.is_empty() {
        let conf: Vec<&[u8]> = st.asc.iter().map(|a| a.as_slice()).collect();
        let len: Vec<u32> = st.asc.iter().map(|a| a.len() as u32).collect();
        let ret = match st.h.as_deref_mut() {
            Some(h) => aac_decoder_config_raw(h, &conf, &len),
            None => AAC_DEC_INVALID_HANDLE,
        };
        st.j.begin("config_raw");
        st.j.arr_begin("length");
        for &l in &len {
            st.j.arr_int(l as i64);
        }
        st.j.arr_end();
        st.j.int("ret", ret as i64);
        st.j.end();
    }

    let init_params = st.init_params.clone();
    for e in &init_params {
        set_param(st, e);
    }

    if st.anc_size > 0 {
        let size = st.anc_size;
        let ret = match st.h.as_deref_mut() {
            Some(h) => aac_decoder_anc_data_init(
                h,
                Some(vec![0u8; size as usize].into_boxed_slice()),
                size,
            ),
            None => AAC_DEC_INVALID_HANDLE,
        };
        st.anc = true;
        st.j.begin("anc_data_init");
        st.j.int("size", size as i64);
        st.j.int("ret", ret as i64);
        st.j.end();
    }

    /* Packet input: one DecodeFrame call per access unit, as applications such as FFmpeg do. A
     * decode loop would call DecodeFrame on an empty buffer, which some configurations (USAC)
     * answer with AAC_DEC_DECODE_FRAME_ERROR instead of AAC_DEC_NOT_ENOUGH_BITS. */
    let packet_once = (st.mp4 || au_sizes.is_some()) && !st.decode_loop_opt && !st.decode_once;
    if packet_once {
        st.decode_once = true;
    }

    /* The chunked parallel decoding of a first part of the feeding (all of it unless it is given
     * up); the sequential loops below continue where it ended. */
    let (mut first_step, mut first_pos) = (0usize, 0usize);
    if chunked && st.h.is_some() {
        let input = if let Some(t) = trk.as_ref() {
            let aus = t.samples.iter().map(|s| (s.offset as usize, s.size as usize)).collect();
            Some(dec_chunked::ChunkInput::Packets(&data, aus))
        } else if let Some(sizes) = au_sizes.as_ref() {
            let total: u64 = sizes.iter().map(|&s| s as u64).sum();
            (total <= data.len() as u64).then(|| {
                let mut off = 0usize;
                let aus = sizes
                    .iter()
                    .map(|&s| {
                        off += s as usize;
                        (off - s as usize, s as usize)
                    })
                    .collect();
                dec_chunked::ChunkInput::Packets(&data, aus)
            })
        } else {
            (data.len() <= 0x4000_0000).then_some(dec_chunked::ChunkInput::Stream(&data))
        };
        if let Some(input) = input {
            let r = dec_chunked::decode(st, &chunk_args, &input)?;
            (first_step, first_pos) = (r.step, r.pos);
        }
        if st.threads > 1 {
            if let Some(h) = st.h.as_deref_mut() {
                aac_decoder_set_threads(h, st.threads);
            }
        }
    }

    let mut fill_idx: u32 = first_step as u32;
    if let Some(t) = trk.as_ref() {
        for s in &t.samples[first_step.min(t.samples.len())..] {
            if st.stop.is_some() {
                break;
            }
            let o = s.offset as usize;
            feed(st, &data[o..o + s.size as usize], true, &mut fill_idx)?;
        }
    } else if let Some(sizes) = au_sizes.as_ref() {
        let mut off: usize = sizes[..first_step.min(sizes.len())].iter().map(|&s| s as usize).sum();
        for &s in &sizes[first_step.min(sizes.len())..] {
            if st.stop.is_some() {
                break;
            }
            if s as usize > data.len() - off {
                return die("--au-sizes exceed the input size");
            }
            feed(st, &data[off..off + s as usize], true, &mut fill_idx)?;
            off += s as usize;
        }
    } else {
        let mut off: usize = first_pos;
        while off < data.len() && st.stop.is_none() {
            let part = (data.len() - off).min(0x4000_0000);
            feed(st, &data[off..off + part], false, &mut fill_idx)?;
            off += part;
        }
    }

    /* --decode-once: the access units still buffered after the last Fill call are decoded by a
     * normal decode loop. */
    if st.stop.is_none() && st.decode_once && !packet_once {
        st.decode_once = false;
        decode_loop(st)?;
    }

    if st.stop.is_none() {
        let mut d = 0;
        while d < st.drain && st.stop.is_none() {
            if st.max_calls > 0 && calls(st) >= st.max_calls {
                st.stop = Some("max_calls");
                break;
            }
            decode(st, AACDEC_FLUSH)?;
            d += 1;
        }
    }

    if st.threads > 1 {
        finish_pipeline(st)?;
        if std::env::var_os("FDK_AAC_RS_PIPELINE_STATS").is_some() {
            if let Some(h) = st.h.as_deref() {
                let (deferred, inline, restarts) = fdk_aac_rs_aacdec::pipeline::stats(h);
                eprintln!(
                    "pipeline: {deferred} frames deferred, {inline} in the calling thread \
                     ({restarts} decoded again), {} mismatches",
                    fdk_aac_rs_aacdec::pipeline::mismatches()
                );
            }
        }
    }

    st.j.begin("end");
    st.j.str("reason", st.stop.unwrap_or("eof"));
    st.j.int("fills", st.n_fill);
    st.j.int("decodes", st.n_dec);
    st.j.uint("samples", st.samples_total);
    st.j.end();

    aac_decoder_close(st.h.take());
    st.j.begin("close");
    st.j.end();

    Ok(0)
}

fn flush(st: &mut St) {
    st.j.flush();
    if let Some(p) = st.pcm.as_mut() {
        let _ = p.flush();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut st = match parse_args(&args) {
        Ok(st) => st,
        Err(Die(msg)) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    let code = match run(&mut st) {
        Ok(code) => code,
        Err(Die(msg)) => {
            flush(&mut st);
            eprintln!("{msg}");
            2
        }
    };
    flush(&mut st);
    std::process::exit(code);
}
