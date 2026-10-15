// Differential tests of fdk_aac_rs_sacenc::sacenc_dmx_tdom_enh against the C++ reference.
//
// The initialization for every frame length of the encoder and random gains; the static
// helpers (fixpAdd, fixpAddNorm, the exponent versions of invSqrtNorm2/sqrtFixp, the ratio and
// downmix gain calculation) on random input; and the downmix of realistic stereo PCM (the
// corpus in the main checkout's work/corpus, s16le, if present, and synthetic signals: noise,
// silence, full scale, antiphase, one silent channel) over many frames with every input
// delay of the encoder, the output and the whole state compared after every frame.

use std::io::{Read, Seek, SeekFrom};

use fdk_aac_rs_cref::sacenc_sacenc_dmx_tdom_enh as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacenc::sacenc_dmx_tdom_enh as r;
use fdk_aac_rs_sacenc::sacenc_dmx_tdom_enh::EnhancedTimeDomainDmx;

use super::sacenc_filter::pcm;

/// The stereo s16le files of the corpus (`work/corpus/*.raw` of the main checkout).
pub fn corpus_files() -> Vec<std::path::PathBuf> {
    let lib = std::path::PathBuf::from(env!("FDK_AAC_RS_CREF_LIB"));
    let Some(work) = lib.ancestors().find(|p| p.file_name().is_some_and(|n| n == "build-cref")).and_then(|p| p.parent())
    else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(work.join("corpus")) else { return Vec::new() };
    let mut v: Vec<_> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            n.ends_with(".raw") && !n.contains("mono") && !n.starts_with("c51") && !n.contains("96k")
        })
        .collect();
    v.sort();
    v
}

/// `n` stereo samples (interleaved L, R) from `offset` samples into a corpus file.
pub fn corpus_stereo(path: &std::path::Path, offset: u64, n: usize) -> Option<Vec<i16>> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let need = (n * 4) as u64;
    if len < need {
        return None;
    }
    let pos = (offset * 4) % (len - need + 1);
    f.seek(SeekFrom::Start(pos & !3)).ok()?;
    let mut b = vec![0u8; n * 4];
    f.read_exact(&mut b).ok()?;
    Some(b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

/// A synthetic stereo signal (interleaved).
pub fn synth_stereo(rng: &mut Rng, n: usize) -> Vec<i16> {
    let kind = rng.range(0, 5);
    let l = pcm(rng, n);
    let rr = pcm(rng, n);
    let mut v = Vec::with_capacity(2 * n);
    for i in 0..n {
        let (a, b) = match kind {
            0 => (l[i], rr[i]),
            1 => (l[i], l[i]),
            2 => (l[i], l[i].wrapping_neg()),
            3 => (l[i], 0),
            4 => (0, 0),
            _ => (l[i], (l[i] as i32 / 3) as i16),
        };
        v.push(a);
        v.push(b);
    }
    v
}

pub fn dmx_ints(d: &EnhancedTimeDomainDmx) -> [i32; 20] {
    [
        d.max_framelength,
        d.framelength,
        d.prev_gain_m[0],
        d.prev_gain_m[1],
        d.prev_gain_e,
        d.prev_h1_m[0],
        d.prev_h1_m[1],
        d.prev_h1_e,
        d.sinus_window_e as i32,
        d.prev_left_m,
        d.prev_left_e,
        d.prev_right_m,
        d.prev_right_e,
        d.prev_xnrg_m,
        d.prev_xnrg_e,
        d.lin_bb_cld_weight_m,
        d.lin_bb_cld_weight_e,
        d.gain_weight_m[0],
        d.gain_weight_m[1],
        d.gain_weight_e,
    ]
}

fn check(h: *mut core::ffi::c_void, d: &EnhancedTimeDomainDmx, what: &str) {
    let mut v = [0i32; 20];
    let mut w = vec![0i32; 1 + d.max_framelength as usize];
    unsafe { c::cref_sacenc_dmx_get(h, v.as_mut_ptr(), w.as_mut_ptr()) };
    assert_eq!(v, dmx_ints(d), "{what}: state");
    assert_eq!(w, d.sinus_window_m, "{what}: window");
}

const GAINS: [i32; 8] = [0x7FFF_FFFF, 0x6BA27F00, 0x5A827C00, 0x4C1BFA80, 0x40000000, 0x35D13F80, 0x2D413D00, 0x20000000];

#[test]
fn init() {
    let mut rng = Rng::new(1);
    for it in 0..2000 {
        let max = 1024;
        let h = unsafe { c::cref_sacenc_dmx_open(max) };
        let mut d = None;
        r::fdk_sacenc_open_enhanced_time_domain_dmx(Some(&mut d), max);
        let mut d = d.unwrap();
        check(h, &d, "open");
        let fl = match rng.range(0, 2) {
            0 => 64 * rng.range(1, 16),
            1 => 32 * rng.range(1, 16),
            _ => rng.range(1, 1100),
        };
        let ing = if rng.range(0, 2) == 0 {
            [0x7FFF_FFFF, 0x7FFF_FFFF]
        } else {
            [rng.range(1, i32::MAX), rng.range(1, i32::MAX)]
        };
        let ine = rng.range(-3, 4);
        let outg = if rng.range(0, 1) == 0 { GAINS[rng.range(0, 7) as usize] } else { rng.range(1, i32::MAX) };
        let oute = rng.range(-3, 4);
        let ec = unsafe { c::cref_sacenc_dmx_init(h, ing.as_ptr(), ine, outg, oute, fl) };
        let er = r::fdk_sacenc_init_enhanced_time_domain_dmx(&mut d, &ing, ine, outg, oute, fl);
        assert_eq!(ec, er, "it {it}");
        check(h, &d, &format!("init it {it} fl {fl}"));
        assert_eq!(unsafe { c::cref_sacenc_dmx_close(h) }, 0);
    }
    assert_eq!(unsafe { c::cref_sacenc_dmx_open_null() }, r::fdk_sacenc_open_enhanced_time_domain_dmx(None, 64));
    assert_eq!(unsafe { c::cref_sacenc_dmx_close_null() }, r::fdk_sacenc_close_enhanced_time_domain_dmx(None));
}

fn mant(rng: &mut Rng) -> i32 {
    match rng.range(0, 3) {
        0 => rng.i32(),
        1 => rng.range(1, i32::MAX) >> rng.range(0, 31),
        2 => 0,
        _ => rng.range(0x4000_0000, i32::MAX),
    }
}

#[test]
fn helpers() {
    let mut rng = Rng::new(2);
    for it in 0..50000 {
        let (m1, e1, m2, e2) = (mant(&mut rng), rng.range(-40, 40), mant(&mut rng), rng.range(-40, 40));
        let (mut dc, mut dr) = (0, 0);
        let rc = unsafe { c::cref_sacenc_dmx_fixpAdd(m1, e1, m2, e2, &mut dc) };
        let rr = r::fixp_add(m1, e1, m2, e2, &mut dr);
        assert_eq!((rc, dc), (rr, dr), "fixpAdd it {it}");

        // fixpAddNorm with energies (value1 >= 0) and cross energies.
        let v1 = if rng.range(0, 1) == 0 { m1.wrapping_abs() >> rng.range(1, 31) } else { m1 >> 1 };
        let (mut v2c, mut q2c) = (m2 >> rng.range(0, 3), e2);
        let (mut v2r, mut q2r) = (v2c, q2c);
        unsafe { c::cref_sacenc_dmx_fixpAddNorm(v1, e1, &mut v2c, &mut q2c) };
        r::fixp_add_norm(v1, e1, &mut v2r, &mut q2r);
        assert_eq!((v2c, q2c), (v2r, q2r), "fixpAddNorm it {it} v1 {v1} q1 {e1}");

        let pm = rng.range(1, i32::MAX) >> rng.range(0, 30);
        let (mut ec, mut er) = (0, 0);
        let rc = unsafe { c::cref_sacenc_dmx_invSqrtNorm2(pm, e1, &mut ec) };
        let rr = r::inv_sqrt_norm2_e(pm, e1, &mut er);
        assert_eq!((rc, ec), (rr, er), "invSqrtNorm2 it {it}");
        let rc = unsafe { c::cref_sacenc_dmx_sqrtFixp(pm, e1, &mut ec) };
        let rr = r::sqrt_fixp_e(pm, e1, &mut er);
        assert_eq!((rc, ec), (rr, er), "sqrtFixp it {it}");

        // calculateRatio / calculateDmxGains with values as the downmix computes them.
        let lcm = rng.range(0x2000_0000, i32::MAX);
        let lce = rng.range(-12, 12);
        let mut se = 0;
        let sm = r::sqrt_fixp_e(lcm, lce, &mut se);
        let icm = if rng.range(0, 3) == 0 { rng.range(0, i32::MAX) } else { rng.range(i32::MIN, 0) };
        let ice = rng.range(-2, 2);
        let mut gc = [0i32; 2];
        let mut gr = [0i32; 2];
        let (mut gec, mut ger) = (0, 0);
        unsafe { c::cref_sacenc_dmx_calculateRatio(sm, se, lcm, lce, icm, ice, gc.as_mut_ptr(), &mut gec) };
        r::calculate_ratio(sm, se, lcm, lce, icm, ice, &mut gr, &mut ger);
        assert_eq!((gc, gec), (gr, ger), "calculateRatio it {it}");

        let l2m = rng.range(0x2000_0000, i32::MAX);
        let l2e = rng.range(-24, 24);
        let mut hc = [0i32; 2];
        let mut hr = [0i32; 2];
        let (mut hec, mut her) = (0, 0);
        unsafe { c::cref_sacenc_dmx_calculateDmxGains(lcm, lce, l2m, l2e, icm, ice, gr.as_ptr(), ger, hc.as_mut_ptr(), &mut hec) };
        r::calculate_dmx_gains(lcm, lce, l2m, l2e, icm, ice, &gr, ger, &mut hr, &mut her);
        assert_eq!((hc, hec), (hr, her), "calculateDmxGains it {it}");
    }
}

#[test]
fn apply() {
    let mut rng = Rng::new(3);
    let files = corpus_files();
    for seq in 0..120 {
        let max = 1024;
        let h = unsafe { c::cref_sacenc_dmx_open(max) };
        let mut d = None;
        r::fdk_sacenc_open_enhanced_time_domain_dmx(Some(&mut d), max);
        let mut d = d.unwrap();
        let fl = [512, 480, 1024, 960, 256, 240, 64 * rng.range(1, 16), 32 * rng.range(1, 16)][rng.range(0, 7) as usize];
        let ing = [0x7FFF_FFFF, 0x7FFF_FFFF];
        let outg = GAINS[rng.range(0, 7) as usize];
        let e = if rng.range(0, 3) == 0 { rng.range(-2, 2) } else { 0 };
        unsafe { c::cref_sacenc_dmx_init(h, ing.as_ptr(), e, outg, e, fl) };
        r::fdk_sacenc_init_enhanced_time_domain_dmx(&mut d, &ing, e, outg, e, fl);
        let delay = rng.range(0, 5);
        let n = (fl + delay) as usize;
        let file = if !files.is_empty() && seq % 3 != 0 { Some(&files[seq % files.len()]) } else { None };
        let mut pos = rng.next_u32() as u64;
        for fr in 0..30 {
            let st = match file.and_then(|f| corpus_stereo(f, pos, n)) {
                Some(v) => v,
                None => synth_stereo(&mut rng, n),
            };
            pos += fl as u64;
            let (left, right): (Vec<i16>, Vec<i16>) = (st.iter().step_by(2).copied().collect(), st.iter().skip(1).step_by(2).copied().collect());
            let mut oc = vec![0x5555i16; fl as usize];
            let mut or = oc.clone();
            let ec = unsafe { c::cref_sacenc_dmx_apply(h, left.as_ptr(), right.as_ptr(), oc.as_mut_ptr(), delay) };
            let er = r::fdk_sacenc_apply_enhanced_time_domain_dmx(&mut d, &[&left[..], &right[..]], &mut or, delay);
            assert_eq!(ec, er);
            assert_eq!(oc, or, "seq {seq} frame {fr} fl {fl}: output");
            check(h, &d, &format!("seq {seq} frame {fr}"));
        }
        unsafe { c::cref_sacenc_dmx_close(h) };
    }
}
