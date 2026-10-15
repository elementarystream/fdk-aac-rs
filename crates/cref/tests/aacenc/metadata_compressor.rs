// Differential tests of fdk_aac_rs_aacenc::metadata_compressor against the C++ reference.
//
// The compressor is stateful: every test runs sequences of calls on a C instance and a port
// instance and compares the return values, dynrng/compr and the complete DRC_COMP state (C
// layout) after every call. The input is real PCM from the corpus in the main checkout's work/
// (multichannel input built from several stereo files and the 5.1 file; skipped if absent) and
// synthetic signals, with level envelopes that drive the compressor through boost, null band,
// cut, attack, release, hold and limiter.

use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc::{CH_ORDER_MPEG, CH_ORDER_WAV, CH_ORDER_WG4};
use fdk_aac_rs_aacenc::metadata_compressor::*;
use fdk_aac_rs_cref::aacenc_metadata_compressor as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL, MINVAL_DBL};
use std::io::{Read, Seek, SeekFrom};
use std::mem::{offset_of, size_of};

const IS_ARM: bool = matches!(fdk_aac_rs_fdk::ARCH, fdk_aac_rs_fdk::Arch::Aarch64);

/// `dmxTable` of metadata_main.cpp (the values the encoder passes as clev, slev, ext_leva,
/// ext_levb; dumped with scripts/dump_table.py libAACenc/src/metadata_main.cpp dmxTable LONG).
const DMX_TABLE: [FixpDbl; 8] =
    [2147483647, 1806033792, 1518270976, 1279900288, 1073741824, 906238080, 762356672, 0];
/// `dmxLfeTable` of metadata_main.cpp (lfe_lev; dumped likewise), and 0 (LFE downmix off).
const DMX_LFE_TABLE: [FixpDbl; 17] = [
    1697585792, 1073741824, 901406272, 758598592, 638339520, 536870912, 451508448, 379567744,
    319975072, 268435456, 169651216, 95563024, 53687092, 17179870, 5368709, 0, 0,
];

const MODES: [ChannelMode; 12] = [
    MODE_1,
    MODE_2,
    MODE_1_2,
    MODE_1_2_1,
    MODE_1_2_2,
    MODE_1_2_2_1,
    MODE_1_2_2_2_1,
    MODE_6_1,
    MODE_7_1_BACK,
    MODE_7_1_TOP_FRONT,
    MODE_7_1_REAR_SURROUND,
    MODE_7_1_FRONT_CENTER,
];
const ORDERS: [i32; 3] = [CH_ORDER_MPEG, CH_ORDER_WAV, CH_ORDER_WG4];
const SAMPLE_RATES: [u32; 12] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];
const BLOCK_LENGTHS: [i32; 9] = [120, 128, 240, 256, 480, 512, 768, 960, 1024];

/// The valid profiles (DRC_DELAY_TEST only on arm64: no defined x86 reference result, see
/// `tc2_x86_undefined`).
fn valid_profiles() -> Vec<DrcProfile> {
    let mut v = vec![
        DRC_NONE,
        DRC_FILMSTANDARD,
        DRC_FILMLIGHT,
        DRC_MUSICSTANDARD,
        DRC_MUSICLIGHT,
        DRC_SPEECH,
        DRC_NOT_PRESENT,
    ];
    if IS_ARM {
        v.push(DRC_DELAY_TEST);
    }
    v
}

fn any_profile(rng: &mut Rng) -> DrcProfile {
    if rng.next_u32() % 16 == 0 {
        [-1, 7, -3, 100][(rng.next_u32() % 4) as usize]
    } else {
        let v = valid_profiles();
        v[(rng.next_u32() as usize) % v.len()]
    }
}

/// The C instance and the port's instance.
struct Pair {
    c: *mut c_void,
    r: Option<Box<DrcComp>>,
}

impl Drop for Pair {
    fn drop(&mut self) {
        unsafe { c::cref_metadata_compressor_Close(self.c) };
    }
}

impl Pair {
    fn new() -> Self {
        let c = unsafe { c::cref_metadata_compressor_Open() };
        assert!(!c.is_null());
        let mut r = None;
        assert_eq!(fdk_drc_generator_open(&mut r), 0);
        let p = Pair { c, r };
        p.check("open");
        p
    }

    fn rs(&mut self) -> &mut DrcComp {
        self.r.as_mut().unwrap()
    }

    fn c_state(&self) -> DrcComp {
        let mut s = DrcComp::default();
        let n = unsafe { c::cref_metadata_compressor_state(self.c, &mut s as *mut _ as *mut _) };
        assert_eq!(n as usize, size_of::<DrcComp>());
        s
    }

    fn set_state(&mut self, s: &DrcComp) {
        unsafe { c::cref_metadata_compressor_set_state(self.c, s as *const _ as *const _) };
        **self.r.as_mut().unwrap() = *s;
    }

    fn check(&self, what: &str) {
        assert_eq!(**self.r.as_ref().unwrap(), self.c_state(), "{what}: DRC_COMP");
    }

    #[allow(clippy::too_many_arguments)]
    fn initialize(&mut self, pl: i32, prf: i32, bl: i32, sr: u32, mode: i32, order: i32, w: u8, what: &str) -> i32 {
        let want = unsafe { c::cref_metadata_compressor_Initialize(self.c, pl, prf, bl, sr, mode, order, w) };
        let got = fdk_drc_generator_initialize(self.rs(), pl, prf, bl, sr, mode, order, w);
        assert_eq!(got, want, "{what}: Initialize return");
        self.check(&format!("{what}: Initialize"));
        got
    }

    fn set_profile(&mut self, pl: i32, prf: i32, what: &str) {
        let want = unsafe { c::cref_metadata_compressor_setDrcProfile(self.c, pl, prf) };
        let got = fdk_drc_generator_set_drc_profile(self.rs(), pl, prf);
        assert_eq!(got, want, "{what}: setDrcProfile return");
        assert_eq!(
            fdk_drc_generator_get_drc_profile(self.r.as_ref().unwrap()),
            unsafe { c::cref_metadata_compressor_getDrcProfile(self.c) }
        );
        assert_eq!(
            fdk_drc_generator_get_comp_profile(self.r.as_ref().unwrap()),
            unsafe { c::cref_metadata_compressor_getCompProfile(self.c) }
        );
        self.check(&format!("{what}: setDrcProfile"));
    }

    /// One Calc() call on `buf` (with `PAD` guard samples of 0 before and after the slice the
    /// port gets).
    fn calc(&mut self, buf: &[i16], buf_size: u32, p: &CalcParams, what: &str) -> (i32, i32) {
        let mut padded = vec![0i16; buf.len() + 2 * PAD];
        padded[PAD..PAD + buf.len()].copy_from_slice(buf);
        let (mut cd, mut cc) = (0x1234_5678, 0x0765_4321);
        let want = unsafe {
            c::cref_metadata_compressor_Calc(
                self.c,
                padded.as_ptr().add(PAD),
                buf_size,
                p.dialnorm,
                p.drc_target,
                p.comp_target,
                p.clev,
                p.slev,
                p.leva,
                p.levb,
                p.lfe,
                p.gain5,
                p.gain2,
                &mut cd,
                &mut cc,
            )
        };
        let pre = **self.r.as_ref().unwrap();
        let (mut rd, mut rc) = (0x1234_5678, 0x0765_4321);
        let got = fdk_drc_generator_calc(
            self.rs(),
            buf,
            buf_size,
            p.dialnorm,
            p.drc_target,
            p.comp_target,
            p.clev,
            p.slev,
            p.leva,
            p.levb,
            p.lfe,
            p.gain5,
            p.gain2,
            &mut rd,
            &mut rc,
        );
        if (got, rd, rc) != (want, cd, cc) || **self.r.as_ref().unwrap() != self.c_state() {
            let h = unsafe { c::cref_metadata_compressor_Open() };
            unsafe { c::cref_metadata_compressor_set_state(h, &pre as *const _ as *const _) };
            let (mut xd, mut xc) = (0, 0);
            unsafe {
                c::cref_metadata_compressor_CalcCopy(h, padded.as_ptr().add(PAD), buf_size, p.dialnorm, p.drc_target, p.comp_target, p.clev, p.slev, p.leva, p.levb, p.lfe, p.gain5, p.gain2, &mut xd, &mut xc);
            }
            let mut xs = DrcComp::default();
            unsafe { c::cref_metadata_compressor_state(h, &mut xs as *mut _ as *mut _) };
            eprintln!("copy: {xd} {xc} smooth_level {:?} smooth_gain {:?}; lib {cd} {cc}; port {rd} {rc}", xs.smooth_level, xs.smooth_gain);
            eprintln!("pre {pre:?}");
            eprintln!("buf {:?}", &buf[..buf.len().min(64)]);
        }
        assert_eq!((got, rd, rc), (want, cd, cc), "{what}: Calc return, dynrng, compr ({p:?})");
        self.check(&format!("{what}: Calc"));
        (rd, rc)
    }
}

/// Guard samples of 0 around the input buffer of the C call (the port reads 0 outside the
/// frame).
const PAD: usize = 64;

#[derive(Clone, Copy, Debug)]
struct CalcParams {
    dialnorm: i32,
    drc_target: i32,
    comp_target: i32,
    clev: FixpDbl,
    slev: FixpDbl,
    leva: FixpDbl,
    levb: FixpDbl,
    lfe: FixpDbl,
    gain5: i32,
    gain2: i32,
}

impl CalcParams {
    /// Parameters as metadata_main.cpp passes them.
    fn realistic(rng: &mut Rng) -> Self {
        let pick = |rng: &mut Rng, t: &[FixpDbl]| t[(rng.next_u32() as usize) % t.len()];
        let dynrng = |rng: &mut Rng| {
            // decodeDynrng(dyn_rng_ctl, dyn_rng_sgn)
            let t = ((rng.next_u32() % 256) as i32) << (16 - 2);
            if rng.next_u32() % 2 == 0 { -t } else { t }
        };
        CalcParams {
            // progreflvl2dialnorm(prog_ref_level)
            dialnorm: -(((rng.next_u32() % 128) as i32) << (16 - 2)),
            drc_target: -(rng.range(0, 31 << 16)),
            comp_target: -(rng.range(0, 31 << 16)),
            clev: pick(rng, &DMX_TABLE),
            slev: pick(rng, &DMX_TABLE),
            leva: pick(rng, &DMX_TABLE),
            levb: pick(rng, &DMX_TABLE),
            lfe: pick(rng, &DMX_LFE_TABLE),
            gain5: if rng.next_u32() % 3 == 0 { 0 } else { dynrng(rng) },
            gain2: if rng.next_u32() % 3 == 0 { 0 } else { dynrng(rng) },
        }
    }

    /// Arbitrary values (overflows wrap in both).
    fn random(rng: &mut Rng) -> Self {
        CalcParams {
            dialnorm: rng.i32(),
            drc_target: rng.i32(),
            comp_target: rng.i32(),
            clev: rng.i32(),
            slev: rng.i32(),
            leva: rng.i32(),
            levb: rng.i32(),
            lfe: rng.i32(),
            gain5: rng.i32(),
            gain2: rng.i32(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Input signals
// ---------------------------------------------------------------------------------------------

/// The main checkout's work/ directory.
fn work_dir() -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    let common = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let w = common.join("..").join("work");
    w.is_dir().then_some(w)
}

/// A corpus file with its number of interleaved channels.
struct Source {
    path: std::path::PathBuf,
    channels: usize,
}

fn corpus_sources() -> Vec<Source> {
    let Some(w) = work_dir() else { return Vec::new() };
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(w.join("corpus")) {
        let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            if !n.ends_with(".raw") {
                continue;
            }
            let channels = if n.contains("mono") {
                1
            } else if n.starts_with("c51") {
                6
            } else {
                2
            };
            v.push(Source { path: p, channels });
        }
    }
    // A few multichannel synthetic files of corpus-gen.
    if let Ok(rd) = std::fs::read_dir(w.join("corpus-gen")) {
        let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            if !n.ends_with(".s16le") || !n.contains("_48000_") {
                continue;
            }
            let channels = if n.ends_with("_7.1.s16le") || n.ends_with("_7.1back.s16le") || n.ends_with("_7.1top.s16le") {
                8
            } else if n.ends_with("_6.1.s16le") {
                7
            } else if n.ends_with("_5.1.s16le") {
                6
            } else {
                continue;
            };
            v.push(Source { path: p, channels });
        }
    }
    v
}

/// `frames` frames of channel `ch` of a source, starting at frame `pos` (modulo its length).
fn read_channel(s: &Source, ch: usize, pos: u64, frames: usize) -> Option<Vec<i16>> {
    let mut f = std::fs::File::open(&s.path).ok()?;
    let fb = 2 * s.channels as u64;
    let len = f.metadata().ok()?.len() / fb;
    if len < frames as u64 {
        return None;
    }
    let start = pos % (len - frames as u64 + 1);
    f.seek(SeekFrom::Start(start * fb)).ok()?;
    let mut buf = vec![0u8; frames * fb as usize];
    f.read_exact(&mut buf).ok()?;
    let ch = ch % s.channels;
    Some(buf.chunks(fb as usize).map(|c| i16::from_le_bytes([c[2 * ch], c[2 * ch + 1]])).collect())
}

/// `channels` channel signals of `frames` samples: real PCM where available, synthetic
/// otherwise; partly attenuated, partly silent.
fn signals(rng: &mut Rng, sources: &[Source], channels: usize, frames: usize) -> Vec<Vec<i16>> {
    let mut out = Vec::new();
    // Often take all channels of one multichannel source.
    let multi: Vec<&Source> = sources.iter().filter(|s| s.channels >= channels && s.channels > 2).collect();
    let common = if !multi.is_empty() && rng.next_u32() % 2 == 0 {
        Some((multi[(rng.next_u32() as usize) % multi.len()], rng.next_u64() % 100_000_000))
    } else {
        None
    };
    for ch in 0..channels {
        let mut sig = None;
        if let Some((s, pos)) = common {
            sig = read_channel(s, ch, pos, frames);
        } else if !sources.is_empty() && rng.next_u32() % 8 != 0 {
            let s = &sources[(rng.next_u32() as usize) % sources.len()];
            sig = read_channel(s, (rng.next_u32() % 8) as usize, rng.next_u64() % 100_000_000, frames);
        }
        let mut sig = sig.unwrap_or_else(|| synthetic(rng, frames));
        match rng.next_u32() % 8 {
            0 => sig.iter_mut().for_each(|x| *x = 0),
            1 | 2 => {
                let sh = rng.range(1, 15);
                sig.iter_mut().for_each(|x| *x >>= sh);
            }
            _ => {}
        }
        out.push(sig);
    }
    out
}

fn synthetic(rng: &mut Rng, frames: usize) -> Vec<i16> {
    match rng.next_u32() % 5 {
        0 => (0..frames).map(|_| rng.i16()).collect(),
        1 => (0..frames).map(|i| if (i / 37) % 2 == 0 { i16::MAX } else { i16::MIN }).collect(),
        2 => {
            let f = rng.range(1, 2000) as f64 / 48000.0;
            let a = rng.range(1, 32767) as f64;
            (0..frames).map(|i| (a * (2.0 * std::f64::consts::PI * f * i as f64).sin()) as i16).collect()
        }
        3 => vec![0; frames],
        _ => (0..frames).map(|i| if i % 1000 == 0 { i16::MAX } else { 0 }).collect(),
    }
}

/// A gain envelope per block (shift amounts; 16: silence) that changes every few blocks.
fn envelope(rng: &mut Rng, blocks: usize) -> Vec<i32> {
    let mut v = Vec::new();
    while v.len() < blocks {
        let sh = [0, 0, 1, 2, 4, 6, 8, 10, 16][(rng.next_u32() % 9) as usize];
        let n = rng.range(1, 12) as usize;
        v.extend(std::iter::repeat_n(sh, n));
    }
    v.truncate(blocks);
    v
}

/// The planar input buffer of one block: channel c at `c * buf_size`, `block_length` samples.
fn planar_block(sig: &[Vec<i16>], blk: usize, block_length: usize, buf_size: usize, sh: i32) -> Vec<i16> {
    let mut buf = vec![0i16; sig.len().max(1) * buf_size];
    for (c, s) in sig.iter().enumerate() {
        for i in 0..block_length {
            let x = s[blk * block_length + i];
            buf[c * buf_size + i] = if sh >= 16 { 0 } else { x >> sh };
        }
        // Junk after the block (inside the buffer).
        for i in block_length..buf_size {
            buf[c * buf_size + i] = (i as i16).wrapping_mul(7919);
        }
    }
    buf
}

/// Runs `blocks` Calc() calls on a pair initialized with the given configuration; with
/// `interleave_ops`, setDrcProfile() and re-initializations happen in between.
#[allow(clippy::too_many_arguments)]
fn run_sequence(
    rng: &mut Rng,
    sources: &[Source],
    (pl, prf, bl, sr, mode, order, w): (i32, i32, i32, u32, i32, i32, u8),
    blocks: usize,
    random_params: bool,
    interleave_ops: bool,
    what: &str,
) {
    let mut p = Pair::new();
    let init = p.initialize(pl, prf, bl, sr, mode, order, w, what);
    let channels = p.r.as_ref().unwrap().channels.max(1) as usize;
    let block_length = bl.max(0) as usize;
    let sig = signals(rng, sources, channels.max(8), blocks * block_length);
    let env = envelope(rng, blocks);
    let mut params = CalcParams::realistic(rng);
    for blk in 0..blocks {
        let what = format!("{what} block {blk} (init {init})");
        if rng.next_u32() % 8 == 0 {
            params = if random_params { CalcParams::random(rng) } else { CalcParams::realistic(rng) };
        }
        if interleave_ops {
            match rng.next_u32() % 32 {
                0 => {
                    let (a, b) = (any_profile(rng), any_profile(rng));
                    p.set_profile(a, b, &what);
                }
                1 => {
                    // A re-initialization (possibly failing) with another configuration of the same
                    // block length.
                    let m = if rng.next_u32() % 4 == 0 { [0, -1, 8, 15][(rng.next_u32() % 4) as usize] } else { MODES[(rng.next_u32() as usize) % MODES.len()] };
                    let (a, b) = (any_profile(rng), any_profile(rng));
                    let o = ORDERS[(rng.next_u32() as usize) % 3];
                    let w2 = (rng.next_u32() % 2) as u8;
                    p.initialize(a, b, bl, sr, m, o, w2, &what);
                }
                _ => {}
            }
        }
        let ch = p.r.as_ref().unwrap().channels.max(1) as usize;
        let buf_size = block_length + [0, 0, 1, 7, 64][(rng.next_u32() % 5) as usize];
        let buf = planar_block(&sig[..ch.min(sig.len())], blk, block_length, buf_size, env[blk]);
        p.calc(&buf, buf_size as u32, &params, &what);
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn layout() {
    let mut s = DrcComp::default();
    let n = {
        let h = unsafe { c::cref_metadata_compressor_Open() };
        let n = unsafe { c::cref_metadata_compressor_state(h, &mut s as *mut _ as *mut _) };
        unsafe { c::cref_metadata_compressor_Close(h) };
        n
    };
    assert_eq!(n as usize, size_of::<DrcComp>());
    assert_eq!(unsafe { c::cref_metadata_compressor_offset(0) } as usize, offset_of!(DrcComp, use_weighting));
    assert_eq!(unsafe { c::cref_metadata_compressor_offset(1) } as usize, offset_of!(DrcComp, channels));
    assert_eq!(unsafe { c::cref_metadata_compressor_offset(2) } as usize, offset_of!(DrcComp, filter));
    assert_eq!(unsafe { c::cref_metadata_compressor_offset(3) } as usize, offset_of!(DrcComp, lim_decay));
    // FDK_DRC_Generator_Open() clears the structure.
    assert_eq!(s, DrcComp::default());
}

#[test]
fn tables() {
    let rust: [&[i32]; 17] = [
        &TAB_MAX_BOOST_THR,
        &TAB_BOOST_THR,
        &TAB_EARLY_CUT_THR,
        &TAB_CUT_THR,
        &TAB_MAX_CUT_THR,
        &TAB_BOOST_RATIO,
        &TAB_EARLY_CUT_RATIO,
        &TAB_CUT_RATIO,
        &TAB_MAX_BOOST,
        &TAB_MAX_CUT,
        &TAB_FAST_ATTACK,
        &TAB_FAST_DECAY,
        &TAB_SLOW_ATTACK,
        &TAB_SLOW_DECAY,
        &TAB_HOLD_OFF,
        &TAB_ATTACK_THR,
        &TAB_DECAY_THR,
    ];
    for (i, t) in rust.iter().enumerate() {
        let mut out = [0i32; 6];
        let n = unsafe { c::cref_metadata_compressor_table(i as i32, out.as_mut_ptr()) };
        assert_eq!(&out[..n as usize], *t, "table {i}");
    }
    let mut out = [0i32; 6];
    let n = unsafe { c::cref_metadata_compressor_table(17, out.as_mut_ptr()) };
    assert_eq!(&out[..n as usize], &[B0, A1, A2]);
}

#[test]
fn get_shift_factor_matches() {
    let mut rng = Rng::new(0x6d63_7366);
    let mut lens: Vec<u32> = (0..=4100).collect();
    lens.extend([1 << 30, (1 << 30) + 1, 1 << 31]);
    for _ in 0..1000 {
        lens.push(rng.next_u32() >> 1);
    }
    for len in lens {
        // The reference is called with lengths up to 2^31 only (see the port's documentation).
        let want = unsafe { c::cref_metadata_compressor_getShiftFactor(len) };
        assert_eq!(get_shift_factor(len), want, "length {len}");
    }
    assert_eq!(get_shift_factor(u32::MAX), 32);
}

#[test]
fn fixp_add_matches() {
    let mut rng = Rng::new(0x6d63_6661);
    for iter in 0..200_000 {
        let v1 = rng.i32();
        let mut v2 = rng.i32();
        // Exponents that keep the shifts inside -31..31 (as in the compressor), and sometimes
        // beyond (undefined in C; measured as compiled into the shim).
        let wide = iter % 8 == 0;
        let (q1, mut q2) = if wide {
            (rng.range(-80, 80), rng.range(-80, 80))
        } else {
            let q = rng.range(-10, 40);
            (q, q + rng.range(-15, 15))
        };
        if rng.next_u32() % 8 == 0 {
            v2 = 0;
            q2 = 31;
        }
        let (mut cv, mut cq) = (v2, q2);
        unsafe { c::cref_metadata_compressor_fixpAdd(v1, q1, &mut cv, &mut cq) };
        let (mut rv, mut rq) = (v2, q2);
        fixp_add(v1, q1, &mut rv, &mut rq);
        assert_eq!((rv, rq), (cv, cq), "fixpAdd({v1}, {q1}, {v2}, {q2})");
    }
}

/// Whether tc2Coeff() has no defined x86 reference result (x86 division by zero; the port gives
/// the arm64 result).
fn tc2_x86_undefined(t: FixpDbl, sr: i32, bl: i32) -> bool {
    use fdk_aac_rs_fdk::fixpoint_math::{f_div_norm, f_mult_norm};
    use fdk_aac_rs_fdk::scale::scale_value;
    let srf = sr << 11;
    let blf = bl << 11;
    if blf == 0 && srf != 0 {
        return true;
    }
    let mut e = 0;
    let f = scale_value(f_div_norm(srf, blf, &mut e), e - 10);
    let product = f_mult_norm(t, f, &mut e);
    scale_value(product, e + 10) == 0
}

/// Whether setDrcProfile() has no defined x86 reference result (see `tc2_x86_undefined`).
fn init_x86_undefined(pl: DrcProfile, prf: DrcProfile, bl: i32, sr: u32) -> bool {
    [pl, prf].iter().any(|&prof| {
        let idx = match prof {
            DRC_NONE | DRC_NOT_PRESENT | DRC_FILMSTANDARD => 0,
            DRC_FILMLIGHT => 1,
            DRC_MUSICSTANDARD => 2,
            DRC_MUSICLIGHT => 3,
            DRC_SPEECH => 4,
            DRC_DELAY_TEST => 5,
            _ => return false,
        };
        [&TAB_FAST_ATTACK, &TAB_FAST_DECAY, &TAB_SLOW_ATTACK, &TAB_SLOW_DECAY]
            .iter()
            .any(|t| tc2_x86_undefined(t[idx], sr as i32, bl))
    })
}

#[test]
fn tc2_coeff_matches() {
    let mut rng = Rng::new(0x6d63_7463);
    let mut ts: Vec<FixpDbl> = Vec::new();
    for t in [&TAB_FAST_ATTACK, &TAB_FAST_DECAY, &TAB_SLOW_ATTACK, &TAB_SLOW_DECAY] {
        ts.extend(t.iter());
    }
    let mut cases = Vec::new();
    for &t in &ts {
        for &sr in &SAMPLE_RATES {
            for bl in [1, 2, 16, 64, 120, 128, 240, 256, 480, 512, 768, 960, 1024, 2048, 4096] {
                cases.push((t, sr as i32, bl));
            }
        }
    }
    for _ in 0..50_000 {
        let t = match rng.next_u32() % 3 {
            0 => rng.range(1, 1 << 24),
            1 => rng.i32() & 0x7fff_ffff,
            _ => rng.i32(),
        };
        let sr = if rng.next_u32() % 4 == 0 { rng.i32() } else { rng.range(1, 200_000) };
        let bl = if rng.next_u32() % 4 == 0 { rng.i32() } else { rng.range(0, 8192) };
        cases.push((t, sr, bl));
    }
    for (t, sr, bl) in cases {
        if !IS_ARM && tc2_x86_undefined(t, sr, bl) {
            continue;
        }
        let want = unsafe { c::cref_metadata_compressor_tc2Coeff(t, sr, bl) };
        assert_eq!(tc2_coeff(t, sr, bl), want, "tc2Coeff({t}, {sr}, {bl})");
    }
}

#[test]
fn initialize_and_set_profile() {
    let mut rng = Rng::new(0x6d63_696e);
    let mut modes: Vec<i32> = MODES.to_vec();
    modes.extend([MODE_INVALID, MODE_UNKNOWN, 8, 9, 10, 13, 15, MODE_212]);
    let mut profiles = valid_profiles();
    profiles.extend([-1, 7, -3]);
    let mut p = Pair::new();
    for &mode in &modes {
        for order in [CH_ORDER_MPEG, CH_ORDER_WAV, CH_ORDER_WG4, 3] {
            for &pl in &profiles {
                for &prf in &profiles {
                    let sr = SAMPLE_RATES[(rng.next_u32() as usize) % SAMPLE_RATES.len()];
                    let bl = BLOCK_LENGTHS[(rng.next_u32() as usize) % BLOCK_LENGTHS.len()];
                    let w = (rng.next_u32() % 2) as u8;
                    let what = format!("mode {mode} order {order} profiles {pl}/{prf} sr {sr} bl {bl}");
                    p.initialize(pl, prf, bl, sr, mode, order, w, &what);
                    p.set_profile(prf, pl, &what);
                }
            }
        }
    }
    // Every sample rate and block length (the time constants), including unusual ones.
    let mut bls: Vec<i32> = BLOCK_LENGTHS.to_vec();
    bls.extend([1, 2, 3, 15, 16, 17, 64, 2048, 4096, 65536]);
    if IS_ARM {
        bls.extend([0, -1, -1024]);
    }
    for &sr in SAMPLE_RATES.iter().chain([1u32, 7350, 192000, 1 << 20, 0xffff_ffff].iter()) {
        for &bl in &bls {
            for &pl in &profiles {
                let prf = profiles[(rng.next_u32() as usize) % profiles.len()];
                if !IS_ARM && init_x86_undefined(pl, prf, bl, sr) {
                    continue;
                }
                let what = format!("sr {sr} bl {bl} profiles {pl}/{prf}");
                p.initialize(pl, prf, bl, sr, MODE_2, CH_ORDER_MPEG, 1, &what);
            }
        }
    }
}

#[test]
fn calc_all_configurations() {
    let sources = corpus_sources();
    if sources.is_empty() {
        eprintln!("calc_all_configurations: work/corpus not available, synthetic input only");
    } else {
        eprintln!("calc_all_configurations: {} corpus files", sources.len());
    }
    let mut rng = Rng::new(0x6d63_6361);
    let profiles = valid_profiles();
    // Every channel mode and order, with weighting on and off.
    for &mode in &MODES {
        for &order in &ORDERS {
            for w in [0u8, 1] {
                let pl = profiles[(rng.next_u32() as usize) % profiles.len()];
                let prf = profiles[(rng.next_u32() as usize) % profiles.len()];
                let sr = SAMPLE_RATES[(rng.next_u32() as usize) % SAMPLE_RATES.len()];
                let bl = BLOCK_LENGTHS[(rng.next_u32() as usize) % BLOCK_LENGTHS.len()];
                let what = format!("mode {mode} order {order} w {w} profiles {pl}/{prf} sr {sr} bl {bl}");
                run_sequence(&mut rng, &sources, (pl, prf, bl, sr, mode, order, w), 24, false, false, &what);
            }
        }
    }
    // Every pair of profiles.
    for &pl in &profiles {
        for &prf in &profiles {
            let mode = MODES[(rng.next_u32() as usize) % MODES.len()];
            let w = (rng.next_u32() % 2) as u8;
            let sr = SAMPLE_RATES[(rng.next_u32() as usize) % SAMPLE_RATES.len()];
            let bl = BLOCK_LENGTHS[(rng.next_u32() as usize) % BLOCK_LENGTHS.len()];
            let what = format!("mode {mode} w {w} profiles {pl}/{prf} sr {sr} bl {bl}");
            run_sequence(&mut rng, &sources, (pl, prf, bl, sr, mode, CH_ORDER_MPEG, w), 40, false, false, &what);
        }
    }
    // Every sample rate and block length.
    for &sr in &SAMPLE_RATES {
        for &bl in &BLOCK_LENGTHS {
            let mode = MODES[(rng.next_u32() as usize) % MODES.len()];
            let w = (rng.next_u32() % 2) as u8;
            let pl = profiles[(rng.next_u32() as usize) % profiles.len()];
            let prf = profiles[(rng.next_u32() as usize) % profiles.len()];
            let what = format!("mode {mode} w {w} profiles {pl}/{prf} sr {sr} bl {bl}");
            run_sequence(&mut rng, &sources, (pl, prf, bl, sr, mode, CH_ORDER_WAV, w), 16, false, false, &what);
        }
    }
}

#[test]
fn calc_random_sequences() {
    let sources = corpus_sources();
    let mut rng = Rng::new(0x6d63_7273);
    for iter in 0..300 {
        let mode = if rng.next_u32() % 16 == 0 { [0, -1, 8][(rng.next_u32() % 3) as usize] } else { MODES[(rng.next_u32() as usize) % MODES.len()] };
        let order = ORDERS[(rng.next_u32() as usize) % 3];
        let w = (rng.next_u32() % 2) as u8;
        let (pl, prf) = (any_profile(&mut rng), any_profile(&mut rng));
        let sr = SAMPLE_RATES[(rng.next_u32() as usize) % SAMPLE_RATES.len()];
        let bl = if rng.next_u32() % 8 == 0 {
            [1, 2, 3, 16, 17, 100, 2048][(rng.next_u32() % 7) as usize]
        } else {
            BLOCK_LENGTHS[(rng.next_u32() as usize) % BLOCK_LENGTHS.len()]
        };
        let random_params = rng.next_u32() % 3 == 0;
        let what = format!("iter {iter} mode {mode} order {order} w {w} profiles {pl}/{prf} sr {sr} bl {bl} random {random_params}");
        run_sequence(&mut rng, &sources, (pl, prf, bl, sr, mode, order, w), 30, random_params, true, &what);
    }
}

/// Calc() without Initialize() (a cleared instance) and on random states with a consistent
/// channel configuration.
#[test]
fn calc_from_cleared_and_random_states() {
    let mut rng = Rng::new(0x6d63_7374);
    let buf: Vec<i16> = (0..8 * 1100).map(|_| rng.i16()).collect();
    {
        let mut p = Pair::new();
        for _ in 0..4 {
            p.calc(&buf, 1024, &CalcParams::realistic(&mut rng), "cleared");
        }
    }
    for iter in 0..3000 {
        let mut p = Pair::new();
        let mode = MODES[(rng.next_u32() as usize) % MODES.len()];
        let pl = valid_profiles()[(rng.next_u32() as usize) % valid_profiles().len()];
        let bl = [16, 64, 120, 128, 256, 512, 1024][(rng.next_u32() % 7) as usize];
        p.initialize(pl, pl, bl, 48000, mode, ORDERS[(rng.next_u32() % 3) as usize], (iter % 2) as u8, "random state");
        let mut s = p.c_state();
        // Random smoothing, hold and limiter states and filter memories.
        for i in 0..2 {
            s.smooth_level[i] = rng.i32() >> rng.range(0, 8);
            s.smooth_gain[i] = rng.i32() >> rng.range(0, 8);
            s.hold_cnt[i] = rng.next_u32() % 20;
            s.lim_gain[i] = rng.i32() >> rng.range(0, 8);
            s.prev_peak[i] = rng.i32() & 0x7fff_ffff;
        }
        for f in s.filter.iter_mut() {
            *f = WeightingStates { x1: rng.i32() >> 2, x2: rng.i32() >> 2, y1: rng.i32() >> 1, y2: rng.i32() >> 1 };
        }
        if rng.next_u32() % 4 == 0 {
            s.lim_decay = rng.i32();
        }
        p.set_state(&s);
        let params = if rng.next_u32() % 2 == 0 { CalcParams::random(&mut rng) } else { CalcParams::realistic(&mut rng) };
        let sh = rng.range(0, 15);
        let b: Vec<i16> = buf.iter().map(|x| x >> sh).collect();
        p.calc(&b, 1100, &params, &format!("random state {iter}"));
    }
}

/// findPeakLevels() directly (from the private copy of the shim) on initialized states.
#[test]
fn find_peak_levels_matches() {
    let sources = corpus_sources();
    let mut rng = Rng::new(0x6d63_706b);
    for iter in 0..2000 {
        let mut p = Pair::new();
        let mode = MODES[(rng.next_u32() as usize) % MODES.len()];
        let bl = [1, 16, 120, 256, 1024][(rng.next_u32() % 5) as usize];
        p.initialize(DRC_NONE, DRC_NONE, bl, 48000, mode, ORDERS[(rng.next_u32() % 3) as usize], 0, "peak");
        let ch = p.r.as_ref().unwrap().channels as usize;
        let sig = signals(&mut rng, &sources, 8, bl as usize);
        let buf = planar_block(&sig[..ch], 0, bl as usize, bl as usize, 0);
        let prm = if rng.next_u32() % 2 == 0 { CalcParams::random(&mut rng) } else { CalcParams::realistic(&mut rng) };
        let (g5, g2) = (rng.i32() >> rng.range(0, 31), rng.i32() >> rng.range(0, 31));
        let mut cpk = [0x11, 0x22];
        unsafe {
            c::cref_metadata_compressor_findPeakLevels(
                p.c,
                buf.as_ptr(),
                prm.clev,
                prm.slev,
                prm.leva,
                prm.levb,
                prm.lfe,
                g5,
                g2,
                cpk.as_mut_ptr(),
            )
        };
        let mut rpk = [0x11, 0x22];
        find_peak_levels(p.r.as_ref().unwrap(), &buf, prm.clev, prm.slev, prm.leva, prm.levb, prm.lfe, g5, g2, &mut rpk);
        assert_eq!(rpk, cpk, "iter {iter} mode {mode} bl {bl}");
    }
    // Edge values.
    let mut p = Pair::new();
    p.initialize(DRC_NONE, DRC_NONE, 64, 48000, MODE_1_2_2_2_1, CH_ORDER_MPEG, 0, "peak edge");
    for v in [i16::MIN, i16::MAX, 0, -1] {
        let buf = vec![v; 8 * 64];
        for g in [MINVAL_DBL, MAXVAL_DBL, 0, -1] {
            let mut cpk = [0, 0];
            unsafe { c::cref_metadata_compressor_findPeakLevels(p.c, buf.as_ptr(), g, g, g, g, g, g, g, cpk.as_mut_ptr()) };
            let mut rpk = [0, 0];
            find_peak_levels(p.r.as_ref().unwrap(), &buf, g, g, g, g, g, g, g, &mut rpk);
            assert_eq!(rpk, cpk, "edge {v} {g}");
        }
    }
}

/// findPeakLevels() with downmix gains whose wrapping `<< 10` makes a downmix exactly
/// `MINVAL_DBL` (`fixp_abs()` returns it unchanged) or makes `peak[0]` negative: the reference
/// builds compile some of its `fixMax()` calls as unsigned comparisons (`fix_max_after_abs`).
/// Found by the encoder fuzz target (7.1 with ETSI downmix gains of 16 dB: one DRC byte of the
/// access unit differed). Samples are drawn from values of 1024 modulo 2048 (which a gain
/// product of 256 turns into `MINVAL_DBL`) and others; the gains are the dB values Calc() passes
/// (`dmxGain << 5`, so 16 dB is 16.0 in Q21) and random ones; then the whole Calc() as well.
#[test]
fn find_peak_levels_unsigned_maxima() {
    const VALUES: [i16; 14] =
        [-29696, 1024, -1024, 3072, -3072, 30720, 30047, -30047, 0, -1, 9728, i16::MIN, i16::MAX, 5120];
    let mut rng = Rng::new(0x6d63_7566);
    let gain = |rng: &mut Rng| -> FixpDbl {
        match rng.next_u32() % 4 {
            // decodeDynrng(): -127..127 quarter dB (as Q16), << 5 (Q21)
            0 | 1 => (rng.range(-127, 127) << 14) << 5,
            2 => [16 << 21, -(16 << 21), 1 << 21, 63 << 19, 0][(rng.next_u32() % 5) as usize],
            _ => rng.i32(),
        }
    };
    for iter in 0..3000 {
        let mut p = Pair::new();
        let mode = MODES[(rng.next_u32() as usize) % MODES.len()];
        let bl = [16, 120, 1024][(rng.next_u32() % 3) as usize];
        p.initialize(DRC_NONE, DRC_NONE, bl, 48000, mode, ORDERS[(rng.next_u32() % 3) as usize], 0, "peak u");
        let ch = p.r.as_ref().unwrap().channels as usize;
        let n_values = rng.range(1, VALUES.len() as i32) as usize;
        let buf: Vec<i16> = (0..ch * bl as usize)
            .map(|_| match rng.next_u32() % 8 {
                0 => rng.i16(),
                _ => VALUES[(rng.next_u32() as usize) % n_values],
            })
            .collect();
        let prm = match rng.next_u32() % 3 {
            0 => CalcParams::random(&mut rng),
            1 => CalcParams::realistic(&mut rng),
            _ => CalcParams { clev: 0, slev: 0, leva: 0, levb: 0, lfe: 0, ..CalcParams::realistic(&mut rng) },
        };
        let (g5, g2) = (gain(&mut rng), gain(&mut rng));
        let mut cpk = [0x11, 0x22];
        unsafe {
            c::cref_metadata_compressor_findPeakLevels(
                p.c,
                buf.as_ptr(),
                prm.clev,
                prm.slev,
                prm.leva,
                prm.levb,
                prm.lfe,
                g5,
                g2,
                cpk.as_mut_ptr(),
            )
        };
        let mut rpk = [0x11, 0x22];
        find_peak_levels(p.r.as_ref().unwrap(), &buf, prm.clev, prm.slev, prm.leva, prm.levb, prm.lfe, g5, g2, &mut rpk);
        assert_eq!(rpk, cpk, "iter {iter} mode {mode} bl {bl} g5 {g5} g2 {g2}");
    }
    // The whole Calc() on 7.1 with 16 dB gains (the fuzz finding's configuration).
    for &mode in &[MODE_7_1_BACK, MODE_7_1_REAR_SURROUND, MODE_1_2_2_1, MODE_6_1] {
        let mut p = Pair::new();
        p.initialize(DRC_MUSICSTANDARD, DRC_FILMSTANDARD, 1024, 48000, mode, CH_ORDER_MPEG, 1, "calc u");
        let ch = p.r.as_ref().unwrap().channels as usize;
        for blk in 0..16 {
            let buf: Vec<i16> = (0..ch * 1024).map(|_| VALUES[(rng.next_u32() % 9) as usize]).collect();
            let prm = CalcParams {
                clev: 0,
                slev: 0,
                leva: 0,
                levb: 0,
                lfe: 0,
                gain5: 64 << 14,
                gain2: 64 << 14,
                ..CalcParams::realistic(&mut rng)
            };
            p.calc(&buf, 1024, &prm, &format!("calc u {mode} {blk}"));
        }
    }
}

/// Full-scale negative input: the unweighted energy sum of a granule overflows to `INT_MIN`
/// (see `fixp_add_unweighted`), for every profile, with and without weighting.
#[test]
fn calc_full_scale() {
    let mut rng = Rng::new(0x6d63_6673);
    for &pl in &valid_profiles() {
        for w in [0u8, 1] {
            for &mode in &[MODE_1, MODE_2, MODE_1_2_2_1, MODE_7_1_BACK] {
                let mut p = Pair::new();
                p.initialize(pl, pl, 1024, 48000, mode, CH_ORDER_MPEG, w, "full scale");
                let ch = p.r.as_ref().unwrap().channels as usize;
                for blk in 0..12 {
                    let v = [i16::MIN, i16::MIN, i16::MAX, 0][blk % 4];
                    let buf = vec![v; ch * 1024];
                    p.calc(&buf, 1024, &CalcParams::realistic(&mut rng), &format!("full scale {pl} {w} {mode} {blk}"));
                }
            }
        }
    }
}
