// End-to-end differential tests of fdk_aac_rs_sacdec::sac_dec_lib: the reference AAC decoder
// (shim/sacdec_sac_dec_lib_e2e.cpp) decodes real MPEG Surround streams while every call of
// its MPEG Surround decoder is recorded with its inputs, outputs and the complete states
// (shim/sacdec_sac_dec_lib.cpp); the port replays each call and is compared after it (see
// sac_dec_lib.rs). The QMF domain of the AAC decoder (written by the SBR decoder between the
// calls) is loaded into the port's domain before the calls that use it.
//
// Streams: the USAC MPEG Surround 2-1-2 conformance streams of ISO/IEC 23003-7
// (compressedMp4/Mps_*), the LD MPEG Surround conformance streams of ISO/IEC 14496-26
// (compressedMp4/er_eld_ldmps_*, er_ad_ldmps_*), all in the work directory (not part of the
// repository; the tests skip what is missing), and ELDv2 streams of the reference encoder
// (AOT_ER_AAC_ELD with MODE_212, with and without SBR) encoded here. Each stream is decoded
// as it is and with corrupted and lost access units (error and concealment paths).

use crate::sac_dec_lib::*;
use fdk_aac_rs_cref::sacdec_sac_dec_lib as clib;
use fdk_aac_rs_cref::sacdec_sac_dec_lib_e2e as c;
use fdk_aac_rs_cref::testutil::Rng;
use std::path::PathBuf;

// ---------------------------------------------------------------------------------------------
// MP4 demuxing (moov/trak/mdia/minf/stbl; non-fragmented files)
// ---------------------------------------------------------------------------------------------

fn be32(d: &[u8], p: usize) -> u32 {
    u32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}
fn be64(d: &[u8], p: usize) -> u64 {
    ((be32(d, p) as u64) << 32) | be32(d, p + 4) as u64
}

/// The boxes in `d[start..end]`: (type, payload start, end).
fn boxes(d: &[u8], start: usize, end: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut v = Vec::new();
    let mut p = start;
    while p + 8 <= end {
        let size = be32(d, p) as u64;
        let t = [d[p + 4], d[p + 5], d[p + 6], d[p + 7]];
        let (hdr, size) = match size {
            0 => (8, (end - p) as u64),
            1 if p + 16 <= end => (16, be64(d, p + 8)),
            _ => (8, size),
        };
        if size < hdr as u64 || p as u64 + size > end as u64 {
            break;
        }
        v.push((t, p + hdr, p + size as usize));
        p += size as usize;
    }
    v
}
fn child(d: &[u8], start: usize, end: usize, t: &[u8; 4]) -> Option<(usize, usize)> {
    boxes(d, start, end).into_iter().find(|b| &b.0 == t).map(|b| (b.1, b.2))
}

/// The length of an MPEG-4 descriptor (at `p`): (length, bytes used).
fn desc_len(d: &[u8], mut p: usize) -> (usize, usize) {
    let mut len = 0usize;
    let start = p;
    for _ in 0..4 {
        let b = d[p];
        p += 1;
        len = (len << 7) | (b & 0x7f) as usize;
        if b & 0x80 == 0 {
            break;
        }
    }
    (len, p - start)
}

/// The AudioSpecificConfig of an esds box.
fn esds_asc(d: &[u8], start: usize, end: usize) -> Option<Vec<u8>> {
    let mut p = start + 4; // version, flags
    // ES_Descriptor
    if d[p] != 3 {
        return None;
    }
    let (_, n) = desc_len(d, p + 1);
    p += 1 + n;
    let flags = d[p + 2];
    p += 3;
    if flags & 0x80 != 0 {
        p += 2;
    }
    if flags & 0x40 != 0 {
        p += 1 + d[p] as usize;
    }
    if flags & 0x20 != 0 {
        p += 2;
    }
    // DecoderConfigDescriptor
    if p >= end || d[p] != 4 {
        return None;
    }
    let (_, n) = desc_len(d, p + 1);
    p += 1 + n + 13;
    // DecoderSpecificInfo
    if p >= end || d[p] != 5 {
        return None;
    }
    let (len, n) = desc_len(d, p + 1);
    p += 1 + n;
    Some(d[p..(p + len).min(end)].to_vec())
}

/// The AudioSpecificConfig and the access units (offset, size) of an MP4 file.
type Track = (Vec<u8>, Vec<(usize, usize)>);

/// The AudioSpecificConfig and the access units of the first audio track.
fn mp4_demux(d: &[u8]) -> Option<Track> {
    let (ms, me) = child(d, 0, d.len(), b"moov")?;
    for (t, ts, te) in boxes(d, ms, me) {
        if &t != b"trak" {
            continue;
        }
        let Some((as_, ae)) = child(d, ts, te, b"mdia") else { continue };
        let Some((hs, _)) = child(d, as_, ae, b"hdlr") else { continue };
        if &d[hs + 8..hs + 12] != b"soun" {
            continue;
        }
        let (ns, ne) = child(d, as_, ae, b"minf")?;
        let (ss, se) = child(d, ns, ne, b"stbl")?;
        // stsd: the first sample entry ('mp4a')
        let (ds, de) = child(d, ss, se, b"stsd")?;
        let entries = boxes(d, ds + 8, de);
        let (et, es, ee) = *entries.first()?;
        if &et != b"mp4a" {
            return None;
        }
        let version = u16::from_be_bytes([d[es + 8], d[es + 9]]);
        let skip = 28 + match version {
            1 => 16,
            2 => 36,
            _ => 0,
        };
        let (zs, ze) = child(d, es + skip, ee, b"esds")?;
        let asc = esds_asc(d, zs, ze)?;
        // sample sizes
        let (zs, _) = child(d, ss, se, b"stsz")?;
        let fixed = be32(d, zs + 4) as usize;
        let count = be32(d, zs + 8) as usize;
        let sizes: Vec<usize> = (0..count).map(|i| if fixed != 0 { fixed } else { be32(d, zs + 12 + 4 * i) as usize }).collect();
        // chunks
        let offsets: Vec<usize> = if let Some((cs, _)) = child(d, ss, se, b"stco") {
            (0..be32(d, cs + 4) as usize).map(|i| be32(d, cs + 8 + 4 * i) as usize).collect()
        } else {
            let (cs, _) = child(d, ss, se, b"co64")?;
            (0..be32(d, cs + 4) as usize).map(|i| be64(d, cs + 8 + 8 * i) as usize).collect()
        };
        let (cs, _) = child(d, ss, se, b"stsc")?;
        let stsc: Vec<(usize, usize)> =
            (0..be32(d, cs + 4) as usize).map(|i| (be32(d, cs + 8 + 12 * i) as usize, be32(d, cs + 12 + 12 * i) as usize)).collect();
        let mut aus = Vec::with_capacity(count);
        let mut s = 0;
        for (ci, &off) in offsets.iter().enumerate() {
            let chunk = ci + 1;
            let per = stsc.iter().rev().find(|e| e.0 <= chunk).map_or(0, |e| e.1);
            let mut o = off;
            for _ in 0..per {
                if s >= count {
                    break;
                }
                aus.push((o, sizes[s]));
                o += sizes[s];
                s += 1;
            }
        }
        if aus.iter().any(|&(o, n)| o + n > d.len()) {
            return None;
        }
        return Some((asc, aus));
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Decoding with replay
// ---------------------------------------------------------------------------------------------

/// The work directory of the main checkout (`$FDK_AAC_RS_WORK_DIR` or `work/` next to the
/// common git directory).
fn work_dir() -> Option<PathBuf> {
    if let Some(w) = std::env::var_os("FDK_AAC_RS_WORK_DIR") {
        return Some(PathBuf::from(w));
    }
    let out = std::process::Command::new("git").args(["rev-parse", "--git-common-dir"]).output().ok()?;
    let common = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let common = if common.is_absolute() {
        common
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(common)
    };
    let w = common.join("..").join("work");
    w.is_dir().then_some(w)
}

/// The files of `dir` whose names start with one of `prefixes`, sorted.
fn files(dir: &str, prefixes: &[&str]) -> Vec<PathBuf> {
    let Some(w) = work_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(w.join(dir)) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| prefixes.iter().any(|x| n.starts_with(x))))
        .collect();
    v.sort();
    v
}

const AACDEC_CONCEAL: u32 = 1;
const TT_MP4_RAW: i32 = 0;

/// What was decoded and replayed.
#[derive(Default, Debug)]
struct Stats {
    streams: usize,
    frames: usize,
    calls: [usize; 14],
    applied: usize,
    nonzero: usize,
    parsed: usize,
}

/// How the access units are damaged.
#[derive(Clone, Copy)]
enum Damage {
    None,
    /// Random bit errors and lost access units.
    Random(u64),
}

/// Decodes the access units with the reference decoder and replays its MPEG Surround calls on
/// the port.
fn run(name: &str, asc: &[u8], aus: &[Vec<u8>], damage: Damage, st: &mut Stats) {
    let mut port = Port::default();
    let mut rng = Rng::new(match damage {
        Damage::None => 1,
        Damage::Random(s) => s,
    });
    unsafe { clib::cref_saclib_capture(1) };
    let h = unsafe { c::cref_sacx_open(TT_MP4_RAW) };
    assert!(!h.is_null());
    replay_log(&mut port, &format!("{name}: open"));
    let mut conf = asc.to_vec();
    let r = unsafe { c::cref_sacx_config_raw(h, conf.as_mut_ptr(), conf.len() as u32) };
    replay_log(&mut port, &format!("{name}: configRaw ({r})"));
    let mut pcm = vec![0i16; 8 * 4096 * 2];
    for (k, au) in aus.iter().enumerate() {
        let mut au = au.clone();
        let mut lost = false;
        if let Damage::Random(_) = damage {
            match rng.next_u32() % 16 {
                0 => lost = true,
                1 | 2 => {
                    for _ in 0..rng.range(1, 4) {
                        if !au.is_empty() {
                            let i = rng.next_u32() as usize % au.len();
                            au[i] ^= 1 << rng.range(0, 7);
                        }
                    }
                }
                3 => {
                    let n = rng.next_u32() as usize % (au.len() + 1);
                    au.truncate(n);
                }
                _ => {}
            }
        }
        let mut info = [0i32; 5];
        unsafe {
            if lost {
                c::cref_sacx_decode(h, core::ptr::null_mut(), 0, pcm.as_mut_ptr(), pcm.len() as i32, AACDEC_CONCEAL, info.as_mut_ptr());
            } else {
                c::cref_sacx_decode(h, au.as_mut_ptr(), au.len() as u32, pcm.as_mut_ptr(), pcm.len() as i32, 0, info.as_mut_ptr());
            }
        }
        replay_log(&mut port, &format!("{name}: frame {k}"));
        st.frames += 1;
    }
    unsafe { c::cref_sacx_close(h) };
    replay_log(&mut port, &format!("{name}: close"));
    unsafe { clib::cref_saclib_capture(0) };
    assert!(port.h.is_none(), "{name}: the port's decoder is still open");
    for (a, b) in st.calls.iter_mut().zip(port.calls) {
        *a += b;
    }
    st.applied += port.applied;
    st.nonzero += port.nonzero;
    st.parsed += port.parsed;
    st.streams += 1;
}

/// Decodes the files (at most `max_frames` access units each), as they are and damaged.
fn run_files(paths: &[PathBuf], max_frames: usize, damaged_frames: usize, st: &mut Stats) {
    for (i, p) in paths.iter().enumerate() {
        let d = std::fs::read(p).unwrap();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let Some((asc, aus)) = mp4_demux(&d) else { panic!("{name}: not demuxed") };
        let aus: Vec<Vec<u8>> = aus.iter().map(|&(o, n)| d[o..o + n].to_vec()).collect();
        run(&name, &asc, &aus[..aus.len().min(max_frames)], Damage::None, st);
        if damaged_frames > 0 {
            run(&format!("{name} (damaged)"), &asc, &aus[..aus.len().min(damaged_frames)], Damage::Random(0xe2e0 + i as u64), st);
        }
    }
}

#[test]
fn sac_dec_lib_e2e_usac_mps212() {
    let paths = files("conformance/23003-7/compressedMp4", &["Mps_"]);
    if paths.is_empty() {
        eprintln!("sac_dec_lib_e2e_usac_mps212: no streams in the work directory, skipped");
        return;
    }
    let mut st = Stats::default();
    run_files(&paths, 48, 24, &mut st);
    eprintln!("sac_dec_lib_e2e_usac_mps212: {st:?}");
    assert!(st.calls[K_APPLY as usize] > 0 && st.applied > 0, "{st:?}");
}

#[test]
fn sac_dec_lib_e2e_ld_mps() {
    let paths = files("conformance/14496-26/compressedMp4", &["er_eld_ldmps_", "er_ad_ldmps_"]);
    if paths.is_empty() {
        eprintln!("sac_dec_lib_e2e_ld_mps: no streams in the work directory, skipped");
        return;
    }
    let mut st = Stats::default();
    run_files(&paths, 96, 48, &mut st);
    eprintln!("sac_dec_lib_e2e_ld_mps: {st:?}");
    assert!(st.calls[K_PARSE as usize] > 0 && st.applied > 0, "{st:?}");
}

/// A stereo test signal: two tones and noise with different levels and phases per channel
/// (interleaved, `n` samples per channel).
fn test_signal(rng: &mut Rng, n: usize, fs: f64) -> Vec<i16> {
    let mut v = Vec::with_capacity(2 * n);
    for i in 0..n {
        let t = i as f64 / fs;
        let pan = 0.5 + 0.5 * (2.0 * std::f64::consts::PI * 0.7 * t).sin();
        let a = (2.0 * std::f64::consts::PI * 440.0 * t).sin() * 6000.0;
        let b = (2.0 * std::f64::consts::PI * 1870.0 * t + 1.0).sin() * 3000.0;
        let noise = (rng.next_u32() as i32 >> 20) as f64;
        let l = a * pan + b * 0.3 + noise;
        let r = a * (1.0 - pan) - b * 0.6 + noise * 0.5;
        v.push(l as i16);
        v.push(r as i16);
    }
    v
}

#[test]
fn sac_dec_lib_e2e_eldv2_encoder() {
    let mut st = Stats::default();
    let mut rng = Rng::new(0xe1d2);
    let mut encoded = 0;
    for &(fs, br, sbr) in &[(48000, 64000, 0), (44100, 48000, 0), (32000, 40000, 0), (48000, 48000, 1), (44100, 32000, 1), (24000, 32000, 0)] {
        let enc = unsafe { c::cref_sacx_enc_open(fs, br, sbr) };
        if enc.is_null() {
            eprintln!("ELDv2 encoder: {fs} Hz, {br} bit/s, SBR {sbr}: not supported");
            continue;
        }
        let mut asc = vec![0u8; 64];
        let mut frame_len = 0;
        let n = unsafe { c::cref_sacx_enc_info(enc, asc.as_mut_ptr(), &mut frame_len) };
        assert!(n > 0);
        asc.truncate(n as usize);
        let mut aus = Vec::new();
        let sig = test_signal(&mut rng, frame_len as usize * 120, fs as f64);
        let mut out = vec![0u8; 8192];
        for chunk in sig.chunks(2 * frame_len as usize) {
            let mut pcm = chunk.to_vec();
            let k = unsafe { c::cref_sacx_enc_encode(enc, pcm.as_mut_ptr(), (pcm.len() / 2) as i32, out.as_mut_ptr(), out.len() as i32) };
            assert!(k >= 0, "encoding failed");
            if k > 0 {
                aus.push(out[..k as usize].to_vec());
            }
        }
        unsafe { c::cref_sacx_enc_close(enc) };
        let name = format!("ELDv2 {fs} Hz {br} bit/s SBR {sbr}");
        run(&name, &asc, &aus, Damage::None, &mut st);
        run(&format!("{name} (damaged)"), &asc, &aus, Damage::Random(0xe1d3 + fs as u64), &mut st);
        encoded += 1;
    }
    eprintln!("sac_dec_lib_e2e_eldv2_encoder: {st:?}");
    assert!(encoded >= 4 && st.applied > 0, "{encoded} {st:?}");
}
