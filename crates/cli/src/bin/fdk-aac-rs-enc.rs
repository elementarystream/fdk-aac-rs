// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! Encoder command line: the interface and the JSON Lines log of the reference harness
//! `fdkenc-ref` (`scripts/ref-harness/fdkenc-ref.c`, specified in PORTING.md, "Reference harness"),
//! on top of the straight-port API `fdk_aac_rs_aacenc::aacenc_lib`. For the same command line
//! it makes the same API calls in the same order and writes a byte-identical log and output.

use core::ffi::c_void;
use std::fs::File;
use std::io::{self, Write};
use std::process::exit;

use fdk_aac_rs_aacenc::aacenc_lib::*;
use fdk_aac_rs_fdk::audio::*;

const DEFAULT_OUT_SIZE: i32 = 20480;

/// `PARAMS`: the parameter names of `aacenc_lib.h` the harness knows, in header order.
const PARAMS: &[(&str, AacencParam)] = &[
    ("AACENC_AOT", AACENC_AOT),
    ("AACENC_BITRATE", AACENC_BITRATE),
    ("AACENC_BITRATEMODE", AACENC_BITRATEMODE),
    ("AACENC_SAMPLERATE", AACENC_SAMPLERATE),
    ("AACENC_SBR_MODE", AACENC_SBR_MODE),
    ("AACENC_GRANULE_LENGTH", AACENC_GRANULE_LENGTH),
    ("AACENC_CHANNELMODE", AACENC_CHANNELMODE),
    ("AACENC_CHANNELORDER", AACENC_CHANNELORDER),
    ("AACENC_SBR_RATIO", AACENC_SBR_RATIO),
    ("AACENC_AFTERBURNER", AACENC_AFTERBURNER),
    ("AACENC_BANDWIDTH", AACENC_BANDWIDTH),
    ("AACENC_PEAK_BITRATE", AACENC_PEAK_BITRATE),
    ("AACENC_TRANSMUX", AACENC_TRANSMUX),
    ("AACENC_HEADER_PERIOD", AACENC_HEADER_PERIOD),
    ("AACENC_SIGNALING_MODE", AACENC_SIGNALING_MODE),
    ("AACENC_TPSUBFRAMES", AACENC_TPSUBFRAMES),
    ("AACENC_AUDIOMUXVER", AACENC_AUDIOMUXVER),
    ("AACENC_PROTECTION", AACENC_PROTECTION),
    ("AACENC_ANCILLARY_BITRATE", AACENC_ANCILLARY_BITRATE),
    ("AACENC_METADATA_MODE", AACENC_METADATA_MODE),
    ("AACENC_CONTROL_STATE", AACENC_CONTROL_STATE),
];

/// `META`: the fields of `AACENC_MetaData` settable with `--meta KEY=VALUE`.
const META: &[&str] = &[
    "drc_profile",
    "comp_profile",
    "drc_TargetRefLevel",
    "comp_TargetRefLevel",
    "prog_ref_level_present",
    "prog_ref_level",
    "PCE_mixdown_idx_present",
    "ETSI_DmxLvl_present",
    "centerMixLevel",
    "surroundMixLevel",
    "dolbySurroundMode",
    "drcPresentationMode",
    "extAncDataEnable",
    "extDownmixLevelEnable",
    "extDownmixLevel_A",
    "extDownmixLevel_B",
    "dmxGainEnable",
    "dmxGain5",
    "dmxGain2",
    "lfeDmxEnable",
    "lfeDmxLevel",
];

/// `apply_meta()`: stores `value` in field `id` of `m` with the C conversion of its type
/// (`INT`, `UCHAR` or `SCHAR`).
fn apply_meta(m: &mut AacencMetaData, id: usize, value: i64) {
    let i = value as i32;
    let u = value as u8;
    let s = value as i8;
    match id {
        0 => m.drc_profile = i,
        1 => m.comp_profile = i,
        2 => m.drc_target_ref_level = i,
        3 => m.comp_target_ref_level = i,
        4 => m.prog_ref_level_present = i,
        5 => m.prog_ref_level = i,
        6 => m.pce_mixdown_idx_present = u,
        7 => m.etsi_dmx_lvl_present = u,
        8 => m.center_mix_level = s,
        9 => m.surround_mix_level = s,
        10 => m.dolby_surround_mode = u,
        11 => m.drc_presentation_mode = u,
        12 => m.ext_meta_data.ext_anc_data_enable = u,
        13 => m.ext_meta_data.ext_downmix_level_enable = u,
        14 => m.ext_meta_data.ext_downmix_level_a = u,
        15 => m.ext_meta_data.ext_downmix_level_b = u,
        16 => m.ext_meta_data.dmx_gain_enable = u,
        17 => m.ext_meta_data.dmx_gain5 = i,
        18 => m.ext_meta_data.dmx_gain2 = i,
        19 => m.ext_meta_data.lfe_dmx_enable = u,
        _ => m.ext_meta_data.lfe_dmx_level = u,
    }
}

// ---- SHA-256 (FIPS 180-4) ------------------------------------------------------------------

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_block(h: &mut [u32; 8], p: &[u8]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([p[4 * i], p[4 * i + 1], p[4 * i + 2], p[4 * i + 3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for i in 0..64 {
        let t1 = hh
            .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
            .wrapping_add((e & f) ^ (!e & g))
            .wrapping_add(K256[i])
            .wrapping_add(w[i]);
        let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
            .wrapping_add((a & b) ^ (a & c) ^ (b & c));
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
        *x = x.wrapping_add(y);
    }
}

/// Lowercase hex SHA-256 of `data`.
fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut chunks = data.chunks_exact(64);
    for c in &mut chunks {
        sha256_block(&mut h, c);
    }
    let rest = chunks.remainder();
    let mut tail = rest.to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for c in tail.chunks_exact(64) {
        sha256_block(&mut h, c);
    }
    let mut s = String::with_capacity(64);
    for x in h {
        s.push_str(&format!("{x:08x}"));
    }
    s
}

// ---- JSON Lines writer ---------------------------------------------------------------------

/// The JSON Lines writer of `common.c`: one record per line, keys in the order written, no
/// whitespace. Without a log the writer writes nothing.
struct Json {
    out: Option<Box<dyn Write>>,
    need_comma: bool,
    line: String,
    /// While set, records are kept here instead of written (the output delay of the
    /// multi-threading: the records after a call wait for its result).
    capture: Option<Vec<String>>,
}

impl Json {
    /// Whether a log is written.
    fn enabled(&self) -> bool {
        self.out.is_some()
    }

    fn key(&mut self, key: Option<&str>) {
        if self.need_comma {
            self.line.push(',');
        }
        if let Some(k) = key {
            self.line.push('"');
            self.line.push_str(k);
            self.line.push_str("\":");
        }
        self.need_comma = true;
    }

    fn begin(&mut self, call: &str) {
        if self.out.is_none() {
            return;
        }
        self.line.clear();
        self.line.push_str("{\"call\":\"");
        self.line.push_str(call);
        self.line.push('"');
        self.need_comma = true;
    }

    fn end(&mut self) {
        let Some(o) = self.out.as_mut() else { return };
        self.line.push_str("}\n");
        if let Some(c) = self.capture.as_mut() {
            c.push(self.line.clone());
        } else if o.write_all(self.line.as_bytes()).is_err() {
            die("cannot write the log");
        }
        self.need_comma = false;
    }

    /// Writes records kept by `capture`.
    fn write_lines(&mut self, lines: Vec<String>) {
        let Some(o) = self.out.as_mut() else { return };
        for l in lines {
            if o.write_all(l.as_bytes()).is_err() {
                die("cannot write the log");
            }
        }
    }

    fn int(&mut self, key: &str, v: i64) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.line.push_str(&v.to_string());
    }

    fn uint(&mut self, key: &str, v: u64) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.line.push_str(&v.to_string());
    }

    fn str(&mut self, key: &str, v: &str) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.line.push('"');
        self.line.push_str(v);
        self.line.push('"');
    }

    fn hex(&mut self, key: &str, data: &[u8]) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.line.push('"');
        for b in data {
            self.line.push_str(&format!("{b:02x}"));
        }
        self.line.push('"');
    }

    fn obj_begin(&mut self, key: Option<&str>) {
        if self.out.is_none() {
            return;
        }
        self.key(key);
        self.line.push('{');
        self.need_comma = false;
    }

    fn obj_end(&mut self) {
        if self.out.is_none() {
            return;
        }
        self.line.push('}');
        self.need_comma = true;
    }

    fn arr_begin(&mut self, key: &str) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.line.push('[');
        self.need_comma = false;
    }

    fn arr_end(&mut self) {
        if self.out.is_none() {
            return;
        }
        self.line.push(']');
        self.need_comma = true;
    }

    fn flush(&mut self) {
        if let Some(o) = self.out.as_mut() {
            let _ = o.flush();
        }
    }
}

// ---- Utilities -----------------------------------------------------------------------------

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    exit(2);
}

/// `strtoll(s, &end, 0)` as `parse_int()` uses it: optional leading whitespace and sign, then
/// a `0x` hexadecimal, `0` octal or decimal number; the whole string must be used.
fn strtoll0(s: &str) -> Option<i64> {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let (radix, digits) = if (t.starts_with("0x") || t.starts_with("0X"))
        && t.len() > 2
        && t.as_bytes()[2].is_ascii_hexdigit()
    {
        (16, &t[2..])
    } else if t.starts_with('0') && t.len() > 1 {
        (8, &t[1..])
    } else {
        (10, t)
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    // ERANGE: strtoll fails for values outside the long long range.
    let v = i128::from_str_radix(digits, radix).ok()?;
    let v = if neg { -v } else { v };
    if v < i64::MIN as i128 || v > i64::MAX as i128 {
        return None;
    }
    Some(v as i64)
}

fn parse_int(s: &str, what: &str) -> i64 {
    match strtoll0(s) {
        Some(v) => v,
        None => die(&format!("invalid integer for {what}: '{s}'")),
    }
}

fn parse_hex(s: &str, what: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut hi: Option<u8> = None;
    for c in s.bytes() {
        if c.is_ascii_whitespace() || c == 0x0b {
            continue;
        }
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => die(&format!("invalid hex digit in {what}")),
        };
        match hi {
            None => hi = Some(v),
            Some(h) => {
                out.push(h << 4 | v);
                hi = None;
            }
        }
    }
    if hi.is_some() {
        die(&format!("odd number of hex digits in {what}"));
    }
    out
}

fn read_file(path: &str) -> Vec<u8> {
    match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => die(&format!("cannot open {path}")),
    }
}

fn open_out(path: Option<&str>) -> Option<Box<dyn Write>> {
    let path = path?;
    if path == "-" {
        return Some(Box::new(io::BufWriter::new(io::stdout())));
    }
    match File::create(path) {
        Ok(f) => Some(Box::new(io::BufWriter::new(f))),
        Err(_) => die(&format!("cannot create {path}")),
    }
}

/// `split_at()`: "N:REST".
fn split_at<'a>(s: &'a str, what: &str) -> (i64, &'a str) {
    match s.find(':') {
        Some(c) if c < 64 => (parse_int(&s[..c], what), &s[c + 1..]),
        _ => die(&format!("expected N:... for {what}: '{s}'")),
    }
}

// ---- Options -------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Param,
    Meta,
}

#[derive(Clone)]
struct Event {
    at: i64,
    kind: EventKind,
    id: i64,
    value: i64,
    name: String,
}

#[derive(Default)]
struct Opts {
    input: Option<String>,
    out_path: Option<String>,
    log_path: Option<String>,
    au_sizes_path: Option<String>,
    anc_path: Option<String>,
    raw: bool,
    rate: i32,
    channels: i32,
    modules: i32,
    max_channels: i32,
    threads: u32,
    no_init: bool,
    use_meta: bool,
    frame_input: i64,
    out_size: i32,
    params: Vec<Event>,
    events: Vec<Event>,
    meta: AacencMetaData,
}

fn param_index(name: &str) -> Option<usize> {
    PARAMS.iter().position(|(n, _)| *n == name)
}

fn parse_param(s: &str, at: i64) -> Event {
    let eq = match s.find('=') {
        Some(e) if e < 64 => e,
        _ => die(&format!("expected NAME=VALUE: '{s}'")),
    };
    let mut name = s[..eq].to_string();
    let id;
    if let Some(k) = param_index(&name) {
        id = PARAMS[k].1 as i64;
    } else {
        id = parse_int(&name, "parameter (unknown name)");
        for (n, pid) in PARAMS {
            if *pid as i64 == id {
                name = n.to_string();
            }
        }
    }
    let value = parse_int(&s[eq + 1..], "parameter value");
    Event { at, kind: EventKind::Param, id, value, name }
}

fn parse_meta(s: &str, at: i64) -> Event {
    let eq = match s.find('=') {
        Some(e) if e < 64 => e,
        _ => die(&format!("expected KEY=VALUE: '{s}'")),
    };
    let name = s[..eq].to_string();
    let mut id: i64 = -1;
    for (i, m) in META.iter().enumerate() {
        if *m == name {
            id = i as i64;
        }
    }
    if id < 0 {
        die(&format!("unknown metadata field '{name}'"));
    }
    let value = parse_int(&s[eq + 1..], "metadata value");
    Event { at, kind: EventKind::Meta, id, value, name }
}

fn usage() -> ! {
    eprint!(
        "usage: fdk-aac-rs-enc [options] INPUT\n\
         \x20 -f, --format wav|s16le  input format (default wav)\n\
         \x20 --rate N, --channels N  sample rate and channels of s16le input\n\
         \x20 --param NAME=VALUE      aacEncoder_SetParam after aacEncOpen (repeatable)\n\
         \x20 --param-at N:NAME=VALUE aacEncoder_SetParam before aacEncEncode call N\n\
         \x20 --modules N             encModules of aacEncOpen (default 0)\n\
         \x20 --max-channels N        maxChannels of aacEncOpen (default: input channels)\n\
         \x20 --no-init               no aacEncEncode(h, NULL, NULL, NULL, NULL) call\n\
         \x20 --frame-input N         samples per channel per call (default: frameLength)\n\
         \x20 --anc FILE              ancillary data: line N holds the hex bytes of call N\n\
         \x20 --meta KEY=VALUE        AACENC_MetaData field, passed on every call\n\
         \x20 --meta-at N:KEY=VALUE   changes a metadata field from call N on\n\
         \x20 --out-size N            output buffer size in bytes (default 20480)\n\
         \x20 -o, --output FILE       bitstream output (all output bytes in call order)\n\
         \x20 --au-sizes FILE         size of each non-empty output, one per line\n\
         \x20 -l, --log FILE          JSON Lines log ('-': stdout)\n\
         \x20 -j, --threads N         encoder threads (default 1; not a library call of C)\n"
    );
    exit(2);
}

fn parse_args(args: &[String]) -> Opts {
    let mut o = Opts { out_size: DEFAULT_OUT_SIZE, ..Default::default() };
    let mut i = 1;
    while i < args.len() {
        let full = args[i].as_str();
        if !full.starts_with('-') || full == "-" {
            if o.input.is_some() {
                die("more than one input");
            }
            o.input = Some(full.to_string());
            i += 1;
            continue;
        }
        let mut a = full;
        let mut v: Option<String> = None;
        if full.as_bytes().get(1) == Some(&b'-') {
            if let Some(eq) = full.find('=') {
                if eq < 64 {
                    a = &full[..eq];
                    v = Some(full[eq + 1..].to_string());
                }
            }
        }
        let mut val = |i: &mut usize| -> String {
            if let Some(x) = v.take() {
                x
            } else if *i + 1 < args.len() {
                *i += 1;
                args[*i].clone()
            } else {
                die(&format!("missing value for {a}"))
            }
        };
        match a {
            "-f" | "--format" => {
                let f = val(&mut i);
                match f.as_str() {
                    "s16le" => o.raw = true,
                    "wav" => o.raw = false,
                    _ => die(&format!("unknown format {f}")),
                }
            }
            "--rate" => o.rate = parse_int(&val(&mut i), a) as i32,
            "--channels" => o.channels = parse_int(&val(&mut i), a) as i32,
            "--param" => {
                let s = val(&mut i);
                o.params.push(parse_param(&s, -1));
            }
            "--param-at" => {
                let s = val(&mut i);
                let (at, rest) = split_at(&s, a);
                o.events.push(parse_param(rest, at));
            }
            "--modules" => o.modules = parse_int(&val(&mut i), a) as i32,
            "--max-channels" => o.max_channels = parse_int(&val(&mut i), a) as i32,
            "--no-init" => o.no_init = true,
            "--frame-input" => {
                o.frame_input = parse_int(&val(&mut i), a);
                if o.frame_input <= 0 {
                    die("--frame-input must be positive");
                }
            }
            "--anc" => o.anc_path = Some(val(&mut i)),
            "--meta" => {
                let s = val(&mut i);
                let e = parse_meta(&s, -1);
                apply_meta(&mut o.meta, e.id as usize, e.value);
                o.use_meta = true;
            }
            "--meta-at" => {
                let s = val(&mut i);
                let (at, rest) = split_at(&s, a);
                o.events.push(parse_meta(rest, at));
                o.use_meta = true;
            }
            "--out-size" => {
                o.out_size = parse_int(&val(&mut i), a) as i32;
                if o.out_size < 0 {
                    die("--out-size must not be negative");
                }
            }
            "-o" | "--output" => o.out_path = Some(val(&mut i)),
            "--au-sizes" => o.au_sizes_path = Some(val(&mut i)),
            "-l" | "--log" => o.log_path = Some(val(&mut i)),
            "-j" | "--threads" => {
                let n = parse_int(&val(&mut i), a);
                if !(1..=1024).contains(&n) {
                    die("--threads must be 1 to 1024");
                }
                o.threads = n as u32;
            }
            "-h" | "--help" => usage(),
            _ => die(&format!("unknown option {a}")),
        }
        i += 1;
    }
    if o.input.is_none() {
        usage();
    }
    if o.no_init && o.frame_input == 0 {
        die("--no-init requires --frame-input");
    }
    o
}

fn le32(p: &[u8]) -> u32 {
    u32::from_le_bytes([p[0], p[1], p[2], p[3]])
}

/// `parse_wav()`: a 16-bit PCM WAV file (RIFF chunks, WAVE_FORMAT_PCM or
/// WAVE_FORMAT_EXTENSIBLE). Returns (rate, channels, PCM byte range).
fn parse_wav(d: &[u8]) -> (i32, i32, usize, usize) {
    let n = d.len();
    let mut pos = 12usize;
    let mut have_fmt = false;
    let (mut rate, mut channels) = (0i32, 0i32);
    if n < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        die("not a RIFF/WAVE file");
    }
    while pos + 8 <= n {
        let len = le32(&d[pos + 4..]);
        let c = pos + 8;
        let avail = n - pos - 8;
        if &d[pos..pos + 4] == b"fmt " {
            if len < 16 || avail < 16 {
                die("WAV: short 'fmt ' chunk");
            }
            let mut fmt = d[c] as u32 | (d[c + 1] as u32) << 8;
            channels = (d[c + 2] as u32 | (d[c + 3] as u32) << 8) as i32;
            rate = le32(&d[c + 4..]) as i32;
            let bits = d[c + 14] as u32 | (d[c + 15] as u32) << 8;
            if fmt == 0xfffe {
                if len < 28 || avail < 28 {
                    die("WAV: short WAVE_FORMAT_EXTENSIBLE");
                }
                fmt = d[c + 24] as u32 | (d[c + 25] as u32) << 8;
            }
            if fmt != 1 {
                die(&format!("WAV: unsupported format {fmt}"));
            }
            if bits != 16 {
                die(&format!("WAV: unsupported sample size {bits}"));
            }
            have_fmt = true;
        } else if &d[pos..pos + 4] == b"data" {
            if !have_fmt {
                die("WAV: 'data' before 'fmt '");
            }
            let l = len as usize;
            let bytes = if len == 0 || len >= 0x7fff0000 || l > avail { avail } else { l };
            return (rate, channels, c, bytes);
        }
        if len as usize > avail {
            break;
        }
        pos += 8 + len as usize + (len & 1) as usize;
    }
    die("WAV: no 'data' chunk");
}

/// An `aacEncEncode()` call of the loop: its number, input arguments, the numInSamples it
/// returned and the number of input samples it was given.
#[derive(Clone, Copy)]
struct Call {
    n: i64,
    in_args: AacencInArgs,
    num_in_samples: i32,
    chunk: i64,
}

/// The result of a call (with the output delay: returned by the next call).
struct CallOut {
    ret: AacencError,
    num_out_bytes: i32,
    num_anc_bytes: i32,
    bit_res_state: i32,
}

/// Logs the `encode` record of call `c` with result `r` (the access unit in `outbuf`) and writes
/// the access unit and its size.
fn log_encode(
    j: &mut Json,
    c: &Call,
    r: &CallOut,
    outbuf: &[u8],
    out_size: i32,
    out: &mut Option<Box<dyn Write>>,
    au: &mut Option<Box<dyn Write>>,
) {
    j.begin("encode");
    j.int("n", c.n);
    j.obj_begin(Some("inArgs"));
    j.int("numInSamples", c.in_args.num_in_samples as i64);
    j.int("numAncBytes", c.in_args.num_anc_bytes as i64);
    j.obj_end();
    j.int("ret", r.ret as i64);
    j.obj_begin(Some("outArgs"));
    j.int("numOutBytes", r.num_out_bytes as i64);
    j.int("numInSamples", c.num_in_samples as i64);
    j.int("numAncBytes", r.num_anc_bytes as i64);
    j.int("bitResState", r.bit_res_state as i64);
    j.obj_end();
    if r.num_out_bytes > 0 && r.num_out_bytes <= out_size {
        let bytes = &outbuf[..r.num_out_bytes as usize];
        if j.enabled() {
            // the hash is only logged (as fdkenc-ref)
            j.str("out", &sha256_hex(bytes));
        }
        if let Some(f) = out.as_mut() {
            if f.write_all(bytes).is_err() {
                die("cannot write output");
            }
        }
        if let Some(f) = au.as_mut() {
            if writeln!(f, "{}", r.num_out_bytes).is_err() {
                die("cannot write output");
            }
        }
    }
    j.end();
}

fn log_lib_info(j: &mut Json) {
    // Zeroed first: the flags of entries a module does not write stay 0.
    let mut info = [LibInfo::default(); FDK_MODULE_LAST as usize];
    fdk_init_lib_info(&mut info);
    let ret = aac_enc_get_lib_info(Some(&mut info));
    j.begin("get_lib_info");
    j.int("ret", ret as i64);
    j.arr_begin("modules");
    for e in info.iter() {
        if e.module_id == FDK_NONE {
            break;
        }
        j.obj_begin(None);
        j.int("module_id", e.module_id as i64);
        j.str("title", e.title);
        j.int("version", e.version as i64);
        let n = e.version_str.iter().position(|&b| b == 0).unwrap_or(e.version_str.len());
        j.str("versionStr", std::str::from_utf8(&e.version_str[..n]).unwrap_or(""));
        j.uint("flags", e.flags as u64);
        j.obj_end();
    }
    j.arr_end();
    j.end();
}

fn set_param(j: &mut Json, h: &mut Option<Box<AacEncoder>>, e: &Event) {
    let ret = aac_encoder_set_param(h.as_deref_mut(), e.id as AacencParam, e.value as u32);
    j.begin("set_param");
    j.str("param", &e.name);
    j.int("id", e.id);
    j.uint("value", e.value as u32 as u64);
    j.int("ret", ret as i64);
    j.end();
}

fn log_info(j: &mut Json, h: &mut Option<Box<AacEncoder>>) -> AacencInfoStruct {
    let mut info = AacencInfoStruct::default();
    let ret = aac_enc_info(h.as_deref_mut(), Some(&mut info));
    j.begin("info");
    j.int("ret", ret as i64);
    j.obj_begin(Some("info"));
    j.uint("maxOutBufBytes", info.max_out_buf_bytes as u64);
    j.uint("maxAncBytes", info.max_anc_bytes as u64);
    j.uint("inBufFillLevel", info.in_buf_fill_level as u64);
    j.uint("inputChannels", info.input_channels as u64);
    j.uint("frameLength", info.frame_length as u64);
    j.uint("nDelay", info.n_delay as u64);
    j.uint("nDelayCore", info.n_delay_core as u64);
    let n = if info.conf_size <= 64 { info.conf_size as usize } else { 64 };
    j.hex("confBuf", &info.conf_buf[..n]);
    j.uint("confSize", info.conf_size as u64);
    j.obj_end();
    j.end();
    info
}

fn log_get_params(j: &mut Json, h: &Option<Box<AacEncoder>>) {
    for (name, id) in PARAMS {
        let v = aac_encoder_get_param(h.as_deref(), *id);
        j.begin("get_param");
        j.str("param", name);
        j.int("id", *id as i64);
        j.uint("value", v as u64);
        j.end();
    }
}

fn channel_mode(channels: i32) -> ChannelMode {
    const MODES: [ChannelMode; 9] = [
        MODE_INVALID,
        MODE_1,
        MODE_2,
        MODE_1_2,
        MODE_1_2_1,
        MODE_1_2_2,
        MODE_1_2_2_1,
        MODE_6_1,
        MODE_7_1_BACK,
    ];
    if (1..=8).contains(&channels) {
        MODES[channels as usize]
    } else {
        MODE_INVALID
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut o = parse_args(&args);

    let input = o.input.clone().unwrap_or_default();
    let mut file = read_file(&input);
    let (rate, channels, pcm_start, pcm_bytes) = if o.raw {
        if o.rate <= 0 || o.channels <= 0 {
            die("s16le input needs --rate and --channels");
        }
        (o.rate, o.channels, 0, file.len())
    } else {
        parse_wav(&file)
    };
    if channels <= 0 {
        die("invalid channel count");
    }
    let n_samples = (pcm_bytes / 2) as i64;
    // The samples (little endian) where they are in the file, if they are aligned on this
    // (little-endian) host, else a converted copy; one zero sample for an empty input.
    let mut pcm_copy: Vec<i16>;
    let pcm_bytes_slice = &mut file[pcm_start..pcm_start + 2 * n_samples as usize];
    // SAFETY: every bit pattern is a valid i16.
    let (head, aligned, _) = unsafe { pcm_bytes_slice.align_to_mut::<i16>() };
    let pcm: &mut [i16] = if cfg!(target_endian = "little")
        && head.is_empty()
        && aligned.len() == n_samples as usize
        && n_samples > 0
    {
        aligned
    } else {
        pcm_copy = file[pcm_start..pcm_start + 2 * n_samples as usize]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        if pcm_copy.is_empty() {
            pcm_copy.push(0);
        }
        &mut pcm_copy
    };

    let mut anc_lines: Vec<String> = Vec::new();
    if let Some(p) = o.anc_path.as_deref() {
        let txt = read_file(p);
        // The C harness works on a NUL-terminated copy: a NUL byte ends the text.
        let end = txt.iter().position(|&b| b == 0).unwrap_or(txt.len());
        let txt = String::from_utf8_lossy(&txt[..end]).into_owned();
        // Keep empty lines: line N belongs to call N.
        let mut rest: &str = &txt;
        loop {
            match rest.find('\n') {
                Some(k) => {
                    anc_lines.push(rest[..k].to_string());
                    rest = &rest[k + 1..];
                }
                None => {
                    if !rest.is_empty() {
                        anc_lines.push(rest.to_string());
                    }
                    break;
                }
            }
        }
    }

    let mut j = Json {
        out: open_out(o.log_path.as_deref()),
        need_comma: false,
        line: String::new(),
        capture: None,
    };
    let mut out = open_out(o.out_path.as_deref());
    let mut au = open_out(o.au_sizes_path.as_deref());
    let mut outbuf = vec![0u8; if o.out_size > 0 { o.out_size as usize } else { 1 }];
    let mut dummy = [0i16; 8];

    log_lib_info(&mut j);

    let mut h: Option<Box<AacEncoder>> = None;
    {
        let max_ch = if o.max_channels != 0 { o.max_channels } else { channels };
        let ret = aac_enc_open(Some(&mut h), o.modules as u32, max_ch as u32);
        j.begin("open");
        j.int("encModules", o.modules as i64);
        j.int("maxChannels", max_ch as i64);
        j.int("ret", ret as i64);
        j.end();
        if ret != AACENC_OK {
            j.flush();
            eprintln!("fdk-aac-rs-enc: aacEncOpen failed");
            exit(1);
        }
        // the opt-in multi-threading of the port (not logged: the log is that of the C calls)
        if let Some(h) = h.as_deref_mut() {
            aac_enc_set_threads(h, o.threads.max(1));
        }
    }

    {
        let mut have_rate = false;
        let mut have_mode = false;
        let params = o.params.clone();
        for e in &params {
            set_param(&mut j, &mut h, e);
            if e.id == AACENC_SAMPLERATE as i64 {
                have_rate = true;
            }
            if e.id == AACENC_CHANNELMODE as i64 {
                have_mode = true;
            }
        }
        if !have_rate {
            let e = Event {
                at: -1,
                kind: EventKind::Param,
                id: AACENC_SAMPLERATE as i64,
                value: rate as i64,
                name: "AACENC_SAMPLERATE".into(),
            };
            set_param(&mut j, &mut h, &e);
        }
        if !have_mode {
            let e = Event {
                at: -1,
                kind: EventKind::Param,
                id: AACENC_CHANNELMODE as i64,
                value: channel_mode(channels) as i64,
                name: "AACENC_CHANNELMODE".into(),
            };
            set_param(&mut j, &mut h, &e);
        }
    }

    let mut info = AacencInfoStruct::default();
    let mut stop = "eof";
    let mut n_enc: i64 = 0;
    'run: {
        if !o.no_init {
            // SAFETY: no buffers are passed.
            let ret = unsafe { aac_enc_encode(h.as_deref_mut(), None, None, None, None) };
            j.begin("init");
            j.int("ret", ret as i64);
            j.end();
            info = log_info(&mut j, &mut h);
            log_get_params(&mut j, &h);
        }
        if o.frame_input == 0 {
            if info.frame_length == 0 {
                stop = "no_frame_length";
                break 'run;
            }
            o.frame_input = info.frame_length as i64;
        }

        let mut pos: i64 = 0;
        let mut call: i64 = 0;
        let mut reinit = false;
        let mut zero_calls = 0;
        // the call whose result the next call returns (output delay)
        let mut pending: Option<Call> = None;
        loop {
            // the kept records of the pending call (those of this call follow)
            let tail_split = j.capture.as_ref().map_or(0, |c| c.len());
            let events: Vec<Event> = o.events.iter().filter(|e| e.at == call).cloned().collect();
            for e in &events {
                if e.kind == EventKind::Param {
                    set_param(&mut j, &mut h, e);
                    reinit = true;
                } else {
                    apply_meta(&mut o.meta, e.id as usize, e.value);
                }
            }

            let mut chunk = o.frame_input.wrapping_mul(channels as i64);
            if chunk > n_samples - pos {
                chunk = n_samples - pos;
            }

            let mut anc: Option<Vec<u8>> = None;
            if o.anc_path.is_some() && (call as usize) < anc_lines.len() && call >= 0 {
                anc = Some(parse_hex(&anc_lines[call as usize], "--anc line"));
            }
            let anc_len = anc.as_ref().map_or(0, |a| a.len());

            let mut in_ptr: [*mut c_void; 3] = [core::ptr::null_mut(); 3];
            let mut in_id = [0i32; 3];
            let mut in_size = [0i32; 3];
            let mut in_el = [0i32; 3];
            let mut nb = 0usize;
            let mut in_args = AacencInArgs::default();
            let mut out_args = AacencOutArgs::default();

            in_ptr[nb] = if chunk > 0 {
                pcm[pos as usize..].as_mut_ptr() as *mut c_void
            } else {
                dummy.as_mut_ptr() as *mut c_void
            };
            in_id[nb] = IN_AUDIO_DATA;
            in_size[nb] = (chunk * 2) as i32;
            in_el[nb] = 2;
            nb += 1;
            in_args.num_in_samples = if chunk > 0 { chunk as i32 } else { -1 };
            let mut anc_buf = anc.unwrap_or_default();
            if o.anc_path.is_some() {
                in_ptr[nb] = if (call as usize) < anc_lines.len() {
                    anc_buf.as_mut_ptr() as *mut c_void
                } else {
                    dummy.as_mut_ptr() as *mut c_void
                };
                // An empty Vec has a dangling (non-null) pointer, like malloc() of C for an
                // empty line; it is not read for 0 bytes.
                in_id[nb] = IN_ANCILLRY_DATA;
                in_size[nb] = anc_len as i32;
                in_el[nb] = 1;
                nb += 1;
                in_args.num_anc_bytes = anc_len as i32;
            }
            let mut meta = o.meta;
            if o.use_meta {
                in_ptr[nb] = &mut meta as *mut AacencMetaData as *mut c_void;
                in_id[nb] = IN_METADATA_SETUP;
                in_size[nb] = core::mem::size_of::<AacencMetaData>() as i32;
                in_el[nb] = 1;
                nb += 1;
            }
            let in_buf = AacencBufDesc {
                num_bufs: nb as i32,
                bufs: in_ptr.as_mut_ptr(),
                buffer_identifiers: in_id.as_mut_ptr(),
                buf_sizes: in_size.as_mut_ptr(),
                buf_el_sizes: in_el.as_mut_ptr(),
            };
            let mut out_ptr: *mut c_void = outbuf.as_mut_ptr() as *mut c_void;
            let mut out_id = OUT_BITSTREAM_DATA;
            let mut out_size = o.out_size;
            let mut out_el = 1i32;
            let out_buf = AacencBufDesc {
                num_bufs: 1,
                bufs: &mut out_ptr,
                buffer_identifiers: &mut out_id,
                buf_sizes: &mut out_size,
                buf_el_sizes: &mut out_el,
            };

            // SAFETY: the descriptors point to live buffers of the announced sizes (the PCM
            // slice holds `chunk` samples, the output buffer `out_size` bytes).
            let delayed_before = h.as_deref().is_some_and(|h| aac_enc_get_output_delay(h) != 0);
            let ret = unsafe {
                aac_enc_encode(
                    h.as_deref_mut(),
                    Some(&in_buf),
                    Some(&out_buf),
                    Some(&in_args),
                    Some(&mut out_args),
                )
            };
            let this = Call { n: call, in_args, num_in_samples: out_args.num_in_samples, chunk };
            let result = CallOut {
                ret,
                num_out_bytes: out_args.num_out_bytes,
                num_anc_bytes: out_args.num_anc_bytes,
                bit_res_state: out_args.bit_res_state,
            };

            // With the output delay of the multi-threading, this call returned the result of
            // the previous one (the first delayed call: nothing). The records of a call and
            // what follows it are kept until its result arrives; then the loop decides as
            // without the delay (the records of calls it would not have made are dropped).
            let delayed = h.as_deref().is_some_and(|h| aac_enc_get_output_delay(h) != 0);
            let (resolved, post, pre) = if delayed {
                let mut post = j.capture.take().unwrap_or_default();
                let pre = post.split_off(tail_split.min(post.len()));
                let prev = if delayed_before { pending.take() } else { None };
                pending = Some(this);
                (prev, post, pre)
            } else {
                (Some(this), Vec::new(), Vec::new())
            };
            if let Some(c) = resolved {
                log_encode(&mut j, &c, &result, &outbuf, o.out_size, &mut out, &mut au);
                if result.ret == AACENC_ENCODE_EOF {
                    n_enc = c.n + 1;
                    break;
                }
                if result.ret != AACENC_OK {
                    n_enc = c.n + 1;
                    stop = "error";
                    break;
                }
                if delayed {
                    j.write_lines(post);
                } else if reinit || (o.no_init && call == 0) {
                    log_info(&mut j, &mut h);
                    log_get_params(&mut j, &h);
                    reinit = false;
                }
                if c.chunk > 0 && c.num_in_samples == 0 && result.num_out_bytes == 0 {
                    zero_calls += 1;
                    if zero_calls >= 2 {
                        n_enc = c.n + 1;
                        stop = "stall";
                        break;
                    }
                } else {
                    zero_calls = 0;
                }
                j.write_lines(pre);
            }
            if delayed {
                j.capture = Some(Vec::new());
                if reinit || (o.no_init && call == 0) {
                    log_info(&mut j, &mut h);
                    log_get_params(&mut j, &h);
                    reinit = false;
                }
            }
            pos += this.num_in_samples as i64;
            call += 1;
        }
    }

    // (lines of calls after the result that ended the loop: not made without the output delay)
    j.capture = None;
    j.begin("end");
    j.str("reason", stop);
    j.int("encodes", n_enc);
    j.end();
    let ret = aac_enc_close(Some(&mut h));
    j.begin("close");
    j.int("ret", ret as i64);
    j.end();

    if let Some(f) = out.as_mut() {
        if f.flush().is_err() {
            die("cannot write output");
        }
    }
    if let Some(f) = au.as_mut() {
        if f.flush().is_err() {
            die("cannot write output");
        }
    }
    j.flush();
}
