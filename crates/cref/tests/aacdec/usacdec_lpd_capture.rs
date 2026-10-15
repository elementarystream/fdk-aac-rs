// Differential tests of the USAC LPD decoder on real data: the conformance streams with
// LPD coding (ISO/IEC 23003-7, `Lpd_*` and `Cct_*`) are decoded by the capture decoder of the
// shim aacdec_usacdec_lpd.cpp (a copy of the reference decoder whose calls of the LPD functions
// are recorded), and every recorded call is replayed with the Rust functions on the recorded
// inputs; the outputs and the channel state must be identical. Each stream is decoded once as
// is and once with corrupted and lost access units.
//
// The streams are test material and are not part of the repository; the test is skipped when
// `work/conformance/23003-7/compressedMp4` is missing (see PORTING.md, "Conformance bitstreams").

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use crate::usacdec_lpd_state::*;
use crate::usacdec_lpd_util::assert_vec_eq;
use fdk_aac_rs_aacdec::channelinfo::{CWorkBufferCore1, SamplingRateInfo};
use fdk_aac_rs_aacdec::usacdec_fac::{clpd_fac_acelp2mdct, clpd_fac_read};
use fdk_aac_rs_aacdec::usacdec_lpd::{
    bass_pf_1sf_delay, clpd_channel_stream_decode, clpd_channel_stream_read, clpd_render_time_signal,
};
use fdk_aac_rs_cref::aacdec_usacdec_lpd as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, FdkBitStream, BS_READER};
use std::path::{Path, PathBuf};

const CAP_READ: i32 = 1;
const CAP_DECODE: i32 = 2;
const CAP_RENDER: i32 = 3;
const CAP_ACELP2MDCT: i32 = 4;
const CAP_FAC_READ: i32 = 5;
const CAP_BASS_PF: i32 = 6;
const AACDEC_CONCEAL: i32 = 1;

/// The work directory of the main checkout (`$FDK_AAC_RS_WORK_DIR`, or `work/` next to the git
/// common directory).
fn work_dir() -> Option<PathBuf> {
    if let Some(w) = std::env::var_os("FDK_AAC_RS_WORK_DIR") {
        return Some(PathBuf::from(w));
    }
    let manifest = env!("CARGO_MANIFEST_DIR");
    let o = std::process::Command::new("git")
        .args(["-C", manifest, "rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    let common = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
    let w = common.parent()?.join("work");
    w.is_dir().then_some(w)
}

/// A minimal MP4 reader: the AudioSpecificConfig and the samples of the first audio track.
fn read_mp4(path: &Path) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let d = std::fs::read(path).ok()?;
    let be32 = |o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
    let mut asc = None;
    let mut sizes = Vec::new();
    let mut chunk_offsets = Vec::new();
    let mut stsc = Vec::new();

    fn walk(
        d: &[u8],
        mut o: usize,
        end: usize,
        be32: &dyn Fn(usize) -> usize,
        asc: &mut Option<Vec<u8>>,
        sizes: &mut Vec<usize>,
        chunks: &mut Vec<usize>,
        stsc: &mut Vec<(usize, usize)>,
    ) {
        while o + 8 <= end {
            let mut sz = be32(o);
            let typ = &d[o + 4..o + 8];
            let mut hdr = 8;
            if sz == 1 {
                sz = u64::from_be_bytes(d[o + 8..o + 16].try_into().unwrap()) as usize;
                hdr = 16;
            } else if sz == 0 {
                sz = end - o;
            }
            if sz < hdr || o + sz > end {
                return;
            }
            let body = o + hdr;
            match typ {
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => {
                    if !(typ == b"trak" && !sizes.is_empty()) {
                        walk(d, body, o + sz, be32, asc, sizes, chunks, stsc);
                    }
                }
                b"stsd" => walk(d, body + 8, o + sz, be32, asc, sizes, chunks, stsc),
                b"mp4a" => walk(d, body + 28, o + sz, be32, asc, sizes, chunks, stsc),
                b"esds" => {
                    // ES_Descriptor (3) -> DecoderConfigDescriptor (4) -> DecSpecificInfo (5)
                    let e = &d[body + 4..o + sz];
                    let mut i = 0;
                    let len = |i: &mut usize| {
                        let mut l = 0usize;
                        for _ in 0..4 {
                            let b = e[*i];
                            *i += 1;
                            l = (l << 7) | (b & 0x7f) as usize;
                            if b & 0x80 == 0 {
                                break;
                            }
                        }
                        l
                    };
                    while i < e.len() {
                        let tag = e[i];
                        i += 1;
                        let l = len(&mut i);
                        match tag {
                            3 => {
                                let flags = e[i + 2];
                                i += 3;
                                if flags & 0x80 != 0 {
                                    i += 2;
                                }
                                if flags & 0x40 != 0 {
                                    i += 1 + e[i] as usize;
                                }
                                if flags & 0x20 != 0 {
                                    i += 2;
                                }
                            }
                            4 => i += 13,
                            5 => {
                                *asc = Some(e[i..i + l].to_vec());
                                break;
                            }
                            _ => i += l,
                        }
                    }
                }
                b"stsz" => {
                    let fixed = be32(body + 4);
                    let n = be32(body + 8);
                    for k in 0..n {
                        sizes.push(if fixed != 0 { fixed } else { be32(body + 12 + 4 * k) });
                    }
                }
                b"stco" => {
                    let n = be32(body + 4);
                    for k in 0..n {
                        chunks.push(be32(body + 8 + 4 * k));
                    }
                }
                b"co64" => {
                    let n = be32(body + 4);
                    for k in 0..n {
                        let p = body + 8 + 8 * k;
                        chunks.push(u64::from_be_bytes(d[p..p + 8].try_into().unwrap()) as usize);
                    }
                }
                b"stsc" => {
                    let n = be32(body + 4);
                    for k in 0..n {
                        stsc.push((be32(body + 8 + 12 * k), be32(body + 12 + 12 * k)));
                    }
                }
                _ => {}
            }
            o += sz;
        }
    }
    walk(&d, 0, d.len(), &be32, &mut asc, &mut sizes, &mut chunk_offsets, &mut stsc);

    // sample offsets from the chunks
    let mut aus = Vec::new();
    let mut s = 0usize;
    for (ci, &off) in chunk_offsets.iter().enumerate() {
        let chunk = ci + 1;
        let per = stsc.iter().rev().find(|&&(first, _)| first <= chunk).map_or(1, |&(_, n)| n);
        let mut o = off;
        for _ in 0..per {
            if s >= sizes.len() {
                break;
            }
            aus.push(d.get(o..o + sizes[s])?.to_vec());
            o += sizes[s];
            s += 1;
        }
    }
    Some((asc?, aus))
}

/// A reader over a record.
struct Rec<'a> {
    v: &'a [i32],
    n: usize,
}

impl<'a> Rec<'a> {
    fn next(&mut self) -> i32 {
        let x = self.v[self.n];
        self.n += 1;
        x
    }
    fn take(&mut self, k: usize) -> &'a [i32] {
        let s = &self.v[self.n..self.n + k];
        self.n += k;
        s
    }
}

/// A Rust bitstream with the recorded state and memory.
struct RecBs {
    bs: FdkBitStream,
    _mem: Vec<u8>,
}

fn rec_bitstream(r: &mut Rec) -> RecBs {
    let st: Vec<u32> = r.take(9).iter().map(|&x| x as u32).collect();
    let size = r.next() as usize;
    let words = r.take(size.div_ceil(4));
    let mut mem = vec![0u8; size];
    for (i, b) in mem.iter_mut().enumerate() {
        *b = (words[i / 4] as u32 >> (8 * (i % 4))) as u8;
    }
    let mut bs = FdkBitStream::default();
    // SAFETY: mem outlives bs (both are owned by the returned RecBs; the heap memory does not
    // move).
    fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&mem), size as u32, 0, BS_READER);
    bs.cache_word = st[0];
    bs.bits_in_cache = st[1];
    bs.config_cache = st[2];
    bs.h_bit_buf.valid_bits = st[3];
    bs.h_bit_buf.read_offset = st[4];
    bs.h_bit_buf.write_offset = st[5];
    bs.h_bit_buf.bit_ndx = st[6];
    assert_eq!((bs.h_bit_buf.buf_size(), bs.h_bit_buf.buf_bits()), (st[7], st[8]));
    RecBs { bs, _mem: mem }
}

fn check_bs(bs: &FdkBitStream, post: &[i32], what: &str) {
    let s = crate::usacdec_lpd_util::bs_state(bs);
    let s: Vec<i32> = s.iter().map(|&x| x as i32).collect();
    assert_vec_eq(&s, post, what);
}

fn wb1_from(v: &[i32]) -> Box<CWorkBufferCore1> {
    let mut w = Box::new(CWorkBufferCore1::default());
    w.buf.copy_from_slice(v);
    w
}

/// Counts of the replayed calls by kind.
#[derive(Default, Debug)]
struct Counts {
    n: [usize; 7],
    read_errors: usize,
    lost_renders: usize,
    /// renders that start with a pFacZir left by the previous frame
    fac_zir_pending: usize,
    /// calls that start with a pFacZir set by the previous call of the channel, compared
    fac_zir_checked: usize,
}

/// Replays one recorded call with the Rust functions and compares the results.
/// Replays one recorded call; returns the channel (the address of its static channel info)
/// and the static channel info before and after the call, for the calls with one.
fn replay<'a>(v: &'a [i32], sci_len: usize, ci_len: usize, counts: &mut Counts, what: &str) -> Option<(u64, &'a [i32], &'a [i32])> {
    let mut r = Rec { v, n: 0 };
    let kind = r.next();
    counts.n[kind as usize] += 1;
    let mut states: Option<(&[i32], &[i32])> = None;
    match kind {
        CAP_READ => {
            let sr = r.next() as u32;
            let flags = r.next() as u32;
            let mut b = rec_bitstream(&mut r);
            let pre_s = r.take(sci_len);
            let mut sci = sci_from_vec(pre_s);
            let mut ci = ci_from_vec(r.take(ci_len));
            let mut spec = r.take(1024).to_vec();
            let err = r.next();
            let st_post = r.take(9);
            let sri = SamplingRateInfo { sampling_rate: sr, ..Default::default() };
            let e = clpd_channel_stream_read(&mut b.bs, &mut ci, &mut sci, &sri, flags, &mut spec);
            assert_eq!(e, err, "{what}: CLpdChannelStream_Read error");
            if e != 0 {
                counts.read_errors += 1;
            }
            check_bs(&b.bs, st_post, &format!("{what}: CLpdChannelStream_Read bitstream"));
            let post_s = r.take(sci_len);
            assert_vec_eq(&sci_to_vec(&sci), post_s, &format!("{what}: CLpdChannelStream_Read static channel info"));
            states = Some((pre_s, post_s));
            assert_vec_eq(&ci_to_vec(&ci), r.take(ci_len), &format!("{what}: CLpdChannelStream_Read channel info"));
            assert_vec_eq(&spec, r.take(1024), &format!("{what}: CLpdChannelStream_Read spectrum"));
        }
        CAP_DECODE => {
            let flags = r.next() as u32;
            let pre_s = r.take(sci_len);
            let mut sci = sci_from_vec(pre_s);
            let mut ci = ci_from_vec(r.take(ci_len));
            let mut spec = r.take(1024).to_vec();
            clpd_channel_stream_decode(&mut ci, &mut sci, flags, &mut spec);
            let post_s = r.take(sci_len);
            assert_vec_eq(&sci_to_vec(&sci), post_s, &format!("{what}: CLpdChannelStream_Decode static channel info"));
            states = Some((pre_s, post_s));
            assert_vec_eq(&ci_to_vec(&ci), r.take(ci_len), &format!("{what}: CLpdChannelStream_Decode channel info"));
            assert_vec_eq(&spec, r.take(1024), &format!("{what}: CLpdChannelStream_Decode spectrum"));
        }
        CAP_RENDER => {
            let l_frame = r.next();
            let sr = r.next() as u32;
            let frame_ok = r.next() as u32;
            let headroom = r.next();
            let flags = r.next() as u32;
            let strm_flags = r.next() as u32;
            if frame_ok == 0 {
                counts.lost_renders += 1;
            }
            let sci_pre = r.take(sci_len);
            // pFacZir flag (after the 8 IMdct values): set by an ACELP -> TCX20 transition at the
            // end of the previous frame
            if sci_pre[8] != 0 {
                counts.fac_zir_pending += 1;
            }
            let pre_s = sci_pre;
            let mut sci = sci_from_vec(sci_pre);
            let mut ci = ci_from_vec(r.take(ci_len));
            let mut spec = r.take(1024).to_vec();
            let mut wb1 = wb1_from(r.take(2048));
            let err = r.next();
            let sri = SamplingRateInfo { sampling_rate: sr, ..Default::default() };
            let mut time = vec![0i32; l_frame.max(0) as usize];
            let e = clpd_render_time_signal(
                &mut sci, &mut ci, &mut time, l_frame, &sri, frame_ok, headroom, flags, strm_flags, &mut spec, &mut wb1,
            );
            assert_eq!(e, err, "{what}: CLpd_RenderTimeSignal error");
            let post_s = r.take(sci_len);
            assert_vec_eq(&sci_to_vec(&sci), post_s, &format!("{what}: CLpd_RenderTimeSignal static channel info"));
            states = Some((pre_s, post_s));
            assert_vec_eq(&ci_to_vec(&ci), r.take(ci_len), &format!("{what}: CLpd_RenderTimeSignal channel info"));
            assert_vec_eq(&spec, r.take(1024), &format!("{what}: CLpd_RenderTimeSignal spectrum"));
            assert_vec_eq(&wb1.buf, r.take(2048), &format!("{what}: CLpd_RenderTimeSignal work buffer"));
            assert_vec_eq(&time, r.take(l_frame as usize), &format!("{what}: CLpd_RenderTimeSignal output"));
        }
        CAP_ACELP2MDCT => {
            let out_off = r.next();
            let n_spec = r.next();
            let fac_e = r.next();
            let fac_length = r.next();
            let nr = r.next();
            let tl = r.next();
            let wrs_id = r.next();
            let fr = r.next();
            let a_exp = r.next();
            let gain = r.next();
            let lost = r.next();
            let is_fd = r.next();
            let last_lpd = r.next() as u8;
            let k = r.next();
            let cas = r.next();
            let acelp_is_sci = r.next();
            assert!(out_off >= 0 && acelp_is_sci == 1, "{what}: unexpected pointers");
            let a: Vec<i16> = r.take(16).iter().map(|&x| x as i16).collect();
            let spec_scale: Vec<i16> = r.take(8).iter().map(|&x| x as i16).collect();
            let pre_s = r.take(sci_len);
            let mut sci = sci_from_vec(pre_s);
            let mut spec = r.take(1024).to_vec();
            let mut fac = r.take(128).to_vec();
            let mut wb1 = r.take(2048).to_vec();
            let ret = r.next();
            let fl = fac_length.clamp(0, 128) as usize;
            let rr = clpd_fac_acelp2mdct(
                &mut sci.i_mdct,
                &mut wb1,
                out_off as usize,
                &mut spec,
                &spec_scale,
                n_spec,
                &mut fac[..fl],
                fac_e,
                fac_length,
                nr,
                tl,
                win_ptr(wrs_id),
                fr,
                &a,
                a_exp,
                &mut sci.acelp,
                gain,
                lost,
                is_fd,
                last_lpd,
                k,
                cas,
            );
            assert_eq!(rr, ret, "{what}: CLpd_FAC_Acelp2Mdct samples");
            let post_s = r.take(sci_len);
            assert_vec_eq(&sci_to_vec(&sci), post_s, &format!("{what}: CLpd_FAC_Acelp2Mdct static channel info"));
            states = Some((pre_s, post_s));
            assert_vec_eq(&spec, r.take(1024), &format!("{what}: CLpd_FAC_Acelp2Mdct spectrum"));
            assert_vec_eq(&fac, r.take(128), &format!("{what}: CLpd_FAC_Acelp2Mdct FAC data"));
            assert_vec_eq(&wb1, r.take(2048), &format!("{what}: CLpd_FAC_Acelp2Mdct output"));
        }
        CAP_FAC_READ => {
            let length = r.next();
            let use_gain = r.next();
            let frame = r.next();
            let mut b = rec_bitstream(&mut r);
            let mut fac = r.take(128).to_vec();
            let mut scale: Vec<i8> = r.take(4).iter().map(|&x| x as i8).collect();
            let err = r.next();
            let st_post = r.take(9);
            let e = clpd_fac_read(&mut b.bs, &mut fac[..length.clamp(0, 128) as usize], &mut scale, length, use_gain, frame);
            assert_eq!(e, err, "{what}: CLpd_FAC_Read error");
            check_bs(&b.bs, st_post, &format!("{what}: CLpd_FAC_Read bitstream"));
            assert_vec_eq(&fac, r.take(128), &format!("{what}: CLpd_FAC_Read data"));
            let sc: Vec<i32> = scale.iter().map(|&x| x as i32).collect();
            assert_vec_eq(&sc, r.take(4), &format!("{what}: CLpd_FAC_Read scale"));
        }
        CAP_BASS_PF => {
            let syn_off = r.next();
            let frame_length = r.next();
            let l_frame = r.next();
            let l_next = r.next();
            let headroom = r.next();
            assert!(syn_off >= 0, "{what}: unexpected pointers");
            let t_sf = r.take(23).to_vec();
            let pit_gain = r.take(23).to_vec();
            let mut mem = r.take(76).to_vec();
            let wb1 = r.take(2048);
            let mut out = vec![0i32; frame_length as usize];
            bass_pf_1sf_delay(wb1, syn_off as usize, &t_sf, &pit_gain, frame_length, l_frame, l_next, &mut out, headroom, &mut mem);
            assert_vec_eq(&mem, r.take(76), &format!("{what}: bass_pf_1sf_delay memory"));
            assert_vec_eq(&out, r.take(frame_length as usize), &format!("{what}: bass_pf_1sf_delay output"));
        }
        _ => panic!("{what}: unknown record {kind}"),
    }
    let chan = states.map(|(pre, post)| {
        let lo = r.next() as u32 as u64;
        let hi = r.next() as u32 as u64;
        (lo | (hi << 32), pre, post)
    });
    assert_eq!(r.n, v.len(), "{what}: record length");
    chan
}

/// Decodes the access units with the capture decoder and replays every recorded call.
/// `corrupt`: flip bits in and drop some access units.
fn run_stream(name: &str, asc: &[u8], aus: &[Vec<u8>], corrupt: bool, counts: &mut Counts) {
    let sci_len = unsafe { c::cref_lpd_sci_ints() } as usize;
    let ci_len = unsafe { c::cref_lpd_ci_ints() } as usize;
    let mut asc = asc.to_vec();
    let h = unsafe { c::cref_cap_open(asc.as_mut_ptr(), asc.len() as u32) };
    assert!(!h.is_null(), "{name}: cannot open the decoder");
    unsafe { c::cref_cap_clear() };
    let mut rng = Rng::new(name.bytes().fold(0x12CA_0001u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64)));
    let mut pcm = vec![0i32; 1 << 16];
    let mut rec = vec![0i32; 1 << 20];
    let mut last_post: std::collections::HashMap<u64, Vec<i32>> = std::collections::HashMap::new();
    for (f, au) in aus.iter().enumerate() {
        let mut au = au.clone();
        let mut flags = 0;
        if corrupt {
            match rng.next_u32() % 16 {
                0 | 1 => {
                    // flip a few bits
                    for _ in 0..1 + rng.next_u32() % 8 {
                        if !au.is_empty() {
                            let i = (rng.next_u32() as usize) % au.len();
                            au[i] ^= 1 << (rng.next_u32() % 8);
                        }
                    }
                }
                2 => {
                    // a lost access unit
                    au.clear();
                    flags = AACDEC_CONCEAL;
                }
                3 => {
                    // a truncated access unit
                    let n = (rng.next_u32() as usize) % (au.len() + 1);
                    au.truncate(n);
                }
                _ => {}
            }
        }
        let _err = unsafe { c::cref_cap_decode(h, au.as_mut_ptr(), au.len() as u32, flags, pcm.as_mut_ptr(), pcm.len() as i32) };
        let n = unsafe { c::cref_cap_count() };
        for i in 0..n {
            let len = unsafe { c::cref_cap_get(i, rec.as_mut_ptr(), rec.len() as i32) } as usize;
            assert!(len <= rec.len());
            let what = format!("{name} AU {f} call {i}");
            if let Some((addr, pre, post)) = replay(&rec[..len], sci_len, ci_len, counts, &what) {
                // The ACELP memory that pFacZir points to must not change
                // between two LPD calls of a channel (the port keeps a copy).
                if pre[8] != 0 {
                    if let Some(last) = last_post.get(&addr) {
                        if last[8] != 0 {
                            counts.fac_zir_checked += 1;
                            assert_vec_eq(&pre[8..438], &last[8..438], &format!("{what}: pFacZir memory changed"));
                        }
                    }
                }
                last_post.insert(addr, post.to_vec());
            }
        }
        unsafe { c::cref_cap_clear() };
    }
    unsafe { c::cref_cap_close(h) };
}

fn conformance_streams(prefixes: &[&str]) -> Vec<PathBuf> {
    let Some(work) = work_dir() else { return Vec::new() };
    let dir = work.join("conformance/23003-7/compressedMp4");
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            n.ends_with(".mp4") && prefixes.iter().any(|pre| n.starts_with(pre))
        })
        .collect();
    v.sort();
    v
}

/// The capture decoder records into global memory of the shim: one stream at a time.
static CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run_streams(prefixes: &[&str]) {
    let _guard = CAPTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let streams = conformance_streams(prefixes);
    if streams.is_empty() {
        eprintln!("conformance streams not found; skipped");
        return;
    }
    let mut counts = Counts::default();
    for p in &streams {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let (asc, aus) = read_mp4(p).unwrap_or_else(|| panic!("{name}: cannot read"));
        run_stream(&name, &asc, &aus, false, &mut counts);
        run_stream(&format!("{name} (corrupted)"), &asc, &aus, true, &mut counts);
    }
    eprintln!(
        "{} streams: {} reads ({} errors), {} decodes, {} renders ({} lost, {} with a pending pFacZir, {} pFacZir checks), {} ACELP->FD FACs, {} FD FAC reads, {} FD bass post-filters",
        streams.len(),
        counts.n[1],
        counts.read_errors,
        counts.n[2],
        counts.n[3],
        counts.lost_renders,
        counts.fac_zir_pending,
        counts.fac_zir_checked,
        counts.n[4],
        counts.n[5],
        counts.n[6]
    );
    assert!(counts.n[1] > 0 && counts.n[3] > 0);
}

#[test]
fn conformance_lpd_streams() {
    run_streams(&["Lpd_"]);
}

#[test]
fn conformance_core_transition_streams() {
    run_streams(&["Cct_"]);
}
