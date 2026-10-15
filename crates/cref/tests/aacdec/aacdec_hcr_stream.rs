// Differential tests of fdk_aac_rs_aacdec::{aacdec_hcr, aacdec_hcrs, rvlc, rvlcconceal} on real
// ER streams.
//
// The ER AAC LC and ER AAC LD conformance streams in LOAS format (ISO/IEC 14496-26,
// `work/conformance/`; fetched by scripts/fetch_conformance.py, never committed) cover every
// combination of VCB11, RVLC and HCR, long and short blocks, mono and stereo. Each selected
// stream is decoded by the reference decoder of shim/aacdec_aacdec_hcr_stream.cpp, which records
// every call of HcrInit(), HcrDecoder(), HcrMuteErroneousLines(), CHcr_Read(), CRvlc_Read(),
// CRvlc_Decode() and CRvlc_ElementCheck() with the state before and after it. Every recorded call
// is replayed on the C++ function and on the port (from the recorded state, compared as in
// aacdec_hcr.rs); the C++ replay must give the recorded state (a check of the harness), and the
// steps of HcrDecoder() and CRvlc_Decode() are compared one by one. The streams are decoded clean
// and with corruptions (bit flips, bursts, cut and repeated data), which drive the error and
// concealment paths of HCR and RVLC.
//
// Without the conformance streams the test prints a note and passes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::aacdec_hcr::*;
use super::aacdec_hcrs::call_decode_non_pcws;
use super::rvlc::{call_crvlc_decode, call_crvlc_element_check, call_crvlc_read, crvlc_decode_steps};
use fdk_aac_rs_cref::aacdec_aacdec_hcr_stream as h;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::TT_MP4_LOAS;

const KIND_CHCR_READ: i32 = 1;
const KIND_HCR_INIT: i32 = 2;
const KIND_HCR_DECODER: i32 = 3;
const KIND_HCR_MUTE: i32 = 4;
const KIND_RVLC_READ: i32 = 5;
const KIND_RVLC_DECODE: i32 = 6;
const KIND_RVLC_ELEMENT_CHECK: i32 = 7;
const KIND_NAMES: [&str; 8] = [
    "",
    "CHcr_Read",
    "HcrInit",
    "HcrDecoder",
    "HcrMuteErroneousLines",
    "CRvlc_Read",
    "CRvlc_Decode",
    "CRvlc_ElementCheck",
];

const REC_ARGS: i32 = 1;
const REC_BS: i32 = 2;
const REC_SNAP: i32 = 3;
const REC_CH0: i32 = 4;
const REC_CH1: i32 = 5;
const REC_HCR: i32 = 6;
const REC_RV: i32 = 7;
const REC_ST0: i32 = 8;
const REC_ST1: i32 = 9;
const REC_SPEC: i32 = 10;
const REC_RET: i32 = 11;
const REC_OUT: i32 = 100;

/// `work/` of the main checkout (`FDK_AAC_RS_CREF_LIB` is
/// `work/build-cref/<variant>/<key>/libfdkaac_ref.a`).
fn work_dir() -> PathBuf {
    Path::new(env!("FDK_AAC_RS_CREF_LIB")).ancestors().nth(4).unwrap().to_path_buf()
}

/// An entry of `work/conformance/index.json`.
#[derive(Clone, Debug)]
struct Stream {
    file: String,
    group: String,
    flags: (i32, i32, i32),
    channels: i32,
    frame_length: i32,
}

/// The value of `"key": value` in a flat JSON object.
fn json_value<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let k = format!("\"{key}\":");
    let i = obj.find(&k)? + k.len();
    let rest = obj[i..].trim_start();
    let end = if let Some(r) = rest.strip_prefix('"') {
        return r.split('"').next();
    } else {
        rest.find([',', '}', '\n']).unwrap_or(rest.len())
    };
    Some(rest[..end].trim())
}

/// The ER AAC LC and LD streams in LOAS format with epConfig 0 that use VCB11, RVLC or HCR.
fn er_streams(work: &Path) -> Vec<Stream> {
    let Ok(text) = std::fs::read_to_string(work.join("conformance/index.json")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for obj in text.split('{').skip(1) {
        let obj = obj.split('}').next().unwrap_or("");
        let get = |k: &str| json_value(obj, k).unwrap_or("");
        let num = |k: &str| get(k).parse::<i32>().unwrap_or(-1);
        let (Some(file), group) = (json_value(obj, "file"), get("group")) else { continue };
        if get("container") != "loas" || !(group == "er_al" || group == "er_ad") || num("ep_config") != 0 {
            continue;
        }
        let flags = (num("vcb11"), num("rvlc"), num("hcr"));
        if flags == (0, 0, 0) {
            continue;
        }
        out.push(Stream {
            file: file.to_string(),
            group: group.to_string(),
            flags,
            channels: num("channels"),
            frame_length: num("frame_length"),
        });
    }
    out
}

/// One recorded call: its kind and its blocks (tag -> values).
struct Record {
    kind: i32,
    blocks: BTreeMap<i32, Vec<i32>>,
}

/// The recording of one decode.
struct Recording {
    records: Vec<Record>,
    snaps: Vec<Vec<u8>>,
    info: [i32; 3],
}

fn decode(data: &[u8], frames: i32) -> Recording {
    let mut info = [0i32; 3];
    let rec = unsafe {
        h::cref_aacdec_hcr_stream_decode(data.as_ptr(), data.len() as i32, TT_MP4_LOAS as i32, frames, info.as_mut_ptr())
    };
    let n = unsafe { h::cref_aacdec_hcr_stream_len(rec) } as usize;
    let mut d = vec![0i32; n];
    unsafe { h::cref_aacdec_hcr_stream_copy(rec, d.as_mut_ptr()) };
    let ns = unsafe { h::cref_aacdec_hcr_stream_snap_count(rec) };
    let snaps = (0..ns)
        .map(|i| {
            let l = unsafe { h::cref_aacdec_hcr_stream_snap_len(rec, i) } as usize;
            let mut b = vec![0u8; l];
            unsafe { h::cref_aacdec_hcr_stream_snap_copy(rec, i, b.as_mut_ptr()) };
            b
        })
        .collect();
    unsafe { h::cref_aacdec_hcr_stream_free(rec) };
    let mut records = Vec::new();
    let mut p = 0;
    while p < n {
        let (kind, nb) = (d[p], d[p + 1]);
        p += 2;
        let mut blocks = BTreeMap::new();
        for _ in 0..nb {
            let (tag, len) = (d[p], d[p + 1] as usize);
            blocks.insert(tag, d[p + 2..p + 2 + len].to_vec());
            p += 2 + len;
        }
        records.push(Record { kind, blocks });
    }
    Recording { records, snaps, info }
}

/// Counts of replayed calls per kind (and of HCR / RVLC errors).
#[derive(Default, Debug)]
struct Counts {
    calls: [usize; 8],
    hcr_errors: usize,
    hcr_short: usize,
    hcr_cpe: usize,
    rvlc_not_ok: usize,
}

fn bs_of(st: &[i32], buf: &[u8]) -> Bs {
    let mut s = [0u32; 9];
    for (a, b) in s.iter_mut().zip(st) {
        *a = *b as u32;
    }
    Bs { st: s, buf: buf.to_vec() }
}

/// Replays every recorded call on both sides.
fn replay(name: &str, r: &Recording, counts: &mut Counts, steps: bool) {
    for (i, rec) in r.records.iter().enumerate() {
        let what = format!("{name}: call {i} ({})", KIND_NAMES[rec.kind as usize]);
        let b = |t: i32| rec.blocks.get(&t).map(|v| v.as_slice());
        // the state before the call
        let mut f = Flat::zero();
        let input = |f: &mut Flat, out: i32| {
            if let Some(v) = b(REC_CH0 + out) {
                f.ch[0] = v.to_vec();
            }
            if let Some(v) = b(REC_CH1 + out) {
                f.ch[1] = v.to_vec();
            }
            if let Some(v) = b(REC_HCR + out) {
                f.hcr = v.to_vec();
            }
            if let Some(v) = b(REC_RV + out) {
                f.rv = v.to_vec();
            }
            if let Some(v) = b(REC_ST0 + out) {
                f.st[0] = v.to_vec();
            }
            if let Some(v) = b(REC_ST1 + out) {
                f.st[1] = v.to_vec();
            }
            if let Some(v) = b(REC_SPEC + out) {
                f.spec[..1024].copy_from_slice(v);
            }
        };
        input(&mut f, 0);
        let bs = b(REC_BS).map(|st| bs_of(st, &r.snaps[b(REC_SNAP).unwrap()[0] as usize]));
        let args = b(REC_ARGS).unwrap_or(&[]);
        let sri = || Sri::new(args[0] as u32, args[1] as u32, args[2] as u32).unwrap();
        counts.calls[rec.kind as usize] += 1;
        let (cf, cbs, ret): (Flat, Option<Bs>, Option<i32>) = match rec.kind {
            KIND_CHCR_READ => {
                if args[0] == fdk_aac_rs_fdk::audio::ID_CPE {
                    counts.hcr_cpe += 1;
                }
                let (f, bs) = call_chcr_read(&what, &f, bs.as_ref().unwrap(), 0, args[0]);
                (f, Some(bs), None)
            }
            KIND_HCR_INIT => {
                let (f, bs, r) = call_hcr_init(&what, &f, bs.as_ref().unwrap(), 0, &sri());
                (f, Some(bs), Some(r as i32))
            }
            KIND_HCR_DECODER => {
                if steps {
                    hcr_decoder_steps(&what, &f, bs.as_ref().unwrap(), 0, &sri());
                }
                let (f, bs, r) = call_hcr_decoder(&what, &f, bs.as_ref().unwrap(), 0, &sri());
                if r != 0 {
                    counts.hcr_errors += 1;
                }
                if f.to_rs().ch[0].ics_info.window_sequence == fdk_aac_rs_fdk::mdct::BLOCK_SHORT {
                    counts.hcr_short += 1;
                }
                (f, Some(bs), Some(r as i32))
            }
            KIND_HCR_MUTE => (call_hcr_mute(&what, &f, 0), None, None),
            KIND_RVLC_READ => {
                let (f, bs) = call_crvlc_read(&what, &f, bs.as_ref().unwrap(), 0);
                (f, Some(bs), None)
            }
            KIND_RVLC_DECODE => {
                if steps {
                    crvlc_decode_steps(&what, &f, bs.as_ref().unwrap(), 0);
                }
                let (f, bs) = call_crvlc_decode(&what, &f, bs.as_ref().unwrap(), 0);
                if f.to_rs().ch[0].p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok == 0 {
                    counts.rvlc_not_ok += 1;
                }
                (f, Some(bs), None)
            }
            KIND_RVLC_ELEMENT_CHECK => {
                (call_crvlc_element_check(&what, &f, args[0] != 0, args[1] as u32, args[2]), None, None)
            }
            k => panic!("{what}: unknown record kind {k}"),
        };
        // The C++ replay gives the state the decoder recorded (the replay is complete).
        let mut expected = f.clone();
        input(&mut expected, REC_OUT);
        cmp_flat(&format!("{what}: C++ replay vs. recording"), &cf, &expected);
        if let (Some(c), Some(st)) = (&cbs, b(REC_BS + REC_OUT)) {
            assert_eq!(c.st, bs_of(st, &[]).st, "{what}: C++ replay vs. recording: bitstream");
        }
        if let (Some(r), Some(v)) = (ret, b(REC_RET)) {
            assert_eq!(r, v[0], "{what}: C++ replay vs. recording: return value");
        }
    }
}

/// A corrupted copy of `data` (the first 64 bytes, with the first configuration, are kept).
fn corrupt(rng: &mut Rng, data: &[u8], mode: u32) -> Vec<u8> {
    let mut d = data.to_vec();
    let n = d.len();
    if n < 256 {
        return d;
    }
    match mode {
        0 => {
            // single bit flips, about one per 4000 bits
            for _ in 0..(n / 500) {
                let i = 64 + (rng.next_u32() as usize) % (n - 64);
                d[i] ^= 1 << (rng.next_u32() % 8);
            }
        }
        1 => {
            // bursts of random bytes
            for _ in 0..(n / 2000).max(1) {
                let i = 64 + (rng.next_u32() as usize) % (n - 96);
                for x in &mut d[i..i + 1 + (rng.next_u32() % 24) as usize] {
                    *x = rng.next_u32() as u8;
                }
            }
        }
        _ => {
            // cut and repeated pieces (truncated and spliced access units)
            let mut out = d[..64].to_vec();
            let mut p = 64;
            while p < n {
                let len = 1 + (rng.next_u32() as usize) % 700;
                let end = (p + len).min(n);
                out.extend_from_slice(&d[p..end]);
                p = match rng.next_u32() % 4 {
                    0 => end + (rng.next_u32() as usize) % 300, /* skip */
                    1 => p.saturating_sub((rng.next_u32() as usize) % 300).max(64), /* repeat */
                    _ => end,
                };
            }
            d = out;
        }
    }
    d
}

#[test]
fn er_streams_replay() {
    let work = work_dir();
    let streams = er_streams(&work);
    if streams.is_empty() {
        eprintln!("aacdec_hcr_stream: no ER conformance streams in {} (scripts/fetch_conformance.py); skipped", work.display());
        return;
    }
    // Two streams of every combination of tools, channels and frame length.
    let mut per: BTreeMap<(String, (i32, i32, i32), i32, i32), Vec<Stream>> = BTreeMap::new();
    for s in streams {
        per.entry((s.group.clone(), s.flags, s.channels, s.frame_length)).or_default().push(s);
    }
    let mut rng = Rng::new(0x5e7);
    let mut counts = Counts::default();
    let mut corrupt_counts = Counts::default();
    for (key, list) in &per {
        for (k, s) in list.iter().step_by((list.len() / 2).max(1)).take(2).enumerate() {
            let path = work.join("conformance").join(&s.file);
            let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let name = s.file.rsplit('/').next().unwrap().to_string();
            let r = decode(&data, 96);
            assert!(r.info[0] > 10, "{name}: only {} frames decoded", r.info[0]);
            replay(&name, &r, &mut counts, k == 0);
            if k == 0 {
                for mode in 0..3 {
                    let d = corrupt(&mut rng, &data, mode);
                    let r = decode(&d, 48);
                    replay(&format!("{name} (corruption {mode})"), &r, &mut corrupt_counts, true);
                }
            }
            eprintln!("{key:?} {name}: {} records", r.records.len());
        }
    }
    eprintln!("clean: {counts:?}");
    eprintln!("corrupt: {corrupt_counts:?}");
    for k in [KIND_CHCR_READ, KIND_HCR_INIT, KIND_HCR_DECODER, KIND_RVLC_READ, KIND_RVLC_DECODE, KIND_RVLC_ELEMENT_CHECK] {
        assert!(counts.calls[k as usize] > 0, "no {} calls", KIND_NAMES[k as usize]);
    }
    assert!(corrupt_counts.calls[KIND_HCR_MUTE as usize] > 0, "no HcrMuteErroneousLines calls");
    assert!(counts.hcr_short > 0 && counts.hcr_cpe > 0, "no short blocks or CPEs with HCR");
    assert!(corrupt_counts.rvlc_not_ok > 0, "no RVLC frame concealment");
    // the steps of DecodeNonPCWs are replayed within hcr_decoder_steps()
    let _ = call_decode_non_pcws;
}
