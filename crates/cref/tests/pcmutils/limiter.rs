// Differential tests of fdk_aac_rs_pcmutils::limiter against the C++ reference.
//
// Random limiters run through long random sequences of calls (pcmLimiter_Apply with random,
// full-scale, clipping, quiet and silent signals, with and without per-sample gains, and random
// output scalings; parameter changes in between). After every call the return values, the output
// samples and the complete limiter state (including maxBuf and delayBuf) are compared.

use fdk_aac_rs_cref::pcmutils_limiter as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::{LibInfo, FDK_MODULE_LAST, FDK_TDLIMIT, FDK_TOOLS};
use fdk_aac_rs_pcmutils::limiter::*;
use std::ffi::c_void;

const SAMPLE_RATES: [u32; 13] =
    [7350, 8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000];

struct Lim {
    rs: Option<TDLimiterPtr>,
    c: *mut c_void,
}

impl Lim {
    fn create(max_attack_ms: u32, release_ms: u32, threshold: i32, max_channels: u32, sr: u32) -> Lim {
        let rs = pcm_limiter_create(max_attack_ms, release_ms, threshold, max_channels, sr);
        let c = unsafe { c::cref_limiter_create(max_attack_ms, release_ms, threshold, max_channels, sr) };
        assert_eq!(rs.is_some(), !c.is_null(), "pcmLimiter_Create result");
        Lim { rs, c }
    }

    fn rs(&mut self) -> &mut TDLimiter {
        self.rs.as_mut().unwrap()
    }

    fn check(&mut self, what: &str) {
        let l = self.rs.as_ref().unwrap();
        let mut s = [0i32; 18];
        let mut mb = vec![0i32; l.max_buf.len()];
        let mut db = vec![0i32; l.delay_buf.len()];
        unsafe {
            c::cref_limiter_state(
                self.c,
                s.as_mut_ptr(),
                mb.as_mut_ptr(),
                mb.len() as u32,
                db.as_mut_ptr(),
                db.len() as u32,
            )
        };
        let r = [
            l.attack as i32,
            l.attack_const,
            l.release_const,
            l.attack_ms as i32,
            l.release_ms as i32,
            l.max_attack_ms as i32,
            l.threshold,
            l.channels as i32,
            l.max_channels as i32,
            l.sample_rate as i32,
            l.max_sample_rate as i32,
            l.cor,
            l.max,
            l.max_buf_idx as i32,
            l.delay_buf_idx as i32,
            l.smooth_state0,
            l.min_gain,
            l.scaling,
        ];
        assert_eq!(r, s, "limiter state after {what}");
        assert_eq!(l.max_buf, mb, "maxBuf after {what}");
        assert_eq!(l.delay_buf, db, "delayBuf after {what}");
    }
}

impl Drop for Lim {
    fn drop(&mut self) {
        let e = pcm_limiter_destroy(self.rs.take());
        assert_eq!(e, unsafe { c::cref_limiter_destroy(self.c) }, "pcmLimiter_Destroy");
    }
}

/// A random signal of `n` interleaved samples of `ch` channels.
pub fn signal(rng: &mut Rng, n: usize, ch: usize) -> Vec<i32> {
    let len = n * ch;
    match rng.next_u32() % 8 {
        // silence
        0 => vec![0; len],
        // full scale random
        1 => (0..len).map(|_| rng.next_u32() as i32).collect(),
        // clipping: square waves at full scale, MINVAL_DBL included
        2 => {
            let period = 1 + rng.next_u32() as usize % 64;
            (0..len)
                .map(|i| if (i / ch / period) % 2 == 0 { i32::MAX } else { i32::MIN })
                .collect()
        }
        // quiet signals
        3 => {
            let sh = 8 + rng.next_u32() % 23;
            (0..len).map(|_| (rng.next_u32() as i32) >> sh).collect()
        }
        // a sine-like ramp with a random amplitude, occasionally exceeding the threshold
        4 => {
            let amp = (rng.next_u32() >> 1) as i64;
            let per = 8 + (rng.next_u32() % 400) as i64;
            (0..len)
                .map(|i| {
                    let t = ((i / ch) as i64 % per) * 4 - 2 * per;
                    let tri = if t < 0 { -t - per } else { per - t };
                    (amp * tri / per) as i32
                })
                .collect()
        }
        // bursts: silence with loud segments
        5 => {
            let mut v = vec![0; len];
            let start = rng.next_u32() as usize % (n.max(1));
            let l = rng.next_u32() as usize % (n.max(1));
            for s in start..(start + l).min(n) {
                for c in 0..ch {
                    v[s * ch + c] = rng.i32();
                }
            }
            v
        }
        // edge values
        _ => (0..len).map(|_| rng.i32()).collect(),
    }
}

/// Random per-sample gains: around 1.0 in various formats, small, or any value.
pub fn gains(rng: &mut Rng, n: usize) -> Vec<i32> {
    match rng.next_u32() % 4 {
        0 => vec![0x7FFF_FFFF; n],
        1 => {
            let g = rng.next_u32() as i32 & 0x7FFF_FFFF;
            vec![g; n]
        }
        2 => {
            let mut g = (rng.next_u32() >> 1) as i32;
            (0..n)
                .map(|_| {
                    g = g.wrapping_add((rng.next_u32() as i32) >> 20) & 0x7FFF_FFFF;
                    g
                })
                .collect()
        }
        _ => (0..n).map(|_| rng.i32()).collect(),
    }
}

pub fn random_scaling(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 8 {
        0 => rng.range(-40, 40),
        1 => 0,
        _ => rng.range(0, 12),
    }
}

fn apply(l: &mut Lim, rng: &mut Rng, scaling: i32, what: &str) {
    let ch = l.rs().channels as usize;
    let n = match rng.next_u32() % 6 {
        0 => rng.next_u32() as usize % 4,
        1 => 2048,
        2 => [120, 128, 240, 256, 480, 512, 960, 1024][(rng.next_u32() % 8) as usize],
        _ => 1 + rng.next_u32() as usize % 700,
    };
    let input = signal(rng, n, ch);
    let g = if rng.next_u32() % 3 == 0 { Some(gains(rng, n)) } else { None };
    let mut out_r = vec![0x5A5Ai16; n * ch + 4];
    let mut out_c = out_r.clone();
    let mut in_c = input.clone();
    let mut g_c = g.clone();
    let er = pcm_limiter_apply(l.rs.as_deref_mut(), &input, &mut out_r, g.as_deref(), scaling, n as u32);
    let ec = unsafe {
        c::cref_limiter_apply(
            l.c,
            in_c.as_mut_ptr(),
            out_c.as_mut_ptr(),
            g_c.as_mut().map_or(core::ptr::null_mut(), |g| g.as_mut_ptr()),
            scaling,
            n as u32,
        )
    };
    assert_eq!(er, ec, "pcmLimiter_Apply result ({what})");
    assert_eq!(out_r, out_c, "pcmLimiter_Apply output ({what}, n {n}, ch {ch}, scaling {scaling})");
    assert_eq!(in_c, input, "pcmLimiter_Apply modified its input");
    l.check(&format!("pcmLimiter_Apply ({what}, n {n}, ch {ch}, scaling {scaling})"));
}

/// A random threshold. Negative thresholds are excluded: they violate the preconditions of
/// fDivNorm() (`num >= 0`) and can reach a division without a defined result on x86.
pub fn random_threshold(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 6 {
        0 => 0x7FFF_FFFF,
        1 => 0x7214_8173, // FL2FXCONST_DBL(0.89125094f)
        2 => rng.i32() & 0x7FFF_FFFF,
        _ => (rng.next_u32() >> (1 + rng.next_u32() % 8)) as i32,
    }
}

/// Runs a random sequence; returns the number of pcmLimiter_Apply calls that limited (minimum gain
/// below 1.0) and that did not.
fn run(seed: u64, steps: usize) -> (u32, u32) {
    let mut limited = (0, 0);
    let mut rng = Rng::new(seed);
    let max_attack_ms = match rng.next_u32() % 3 {
        0 => TDL_ATTACK_DEFAULT_MS,
        _ => 1 + rng.next_u32() % 20,
    };
    let release_ms = match rng.next_u32() % 3 {
        0 => TDL_RELEASE_DEFAULT_MS,
        1 => rng.next_u32() % 5000,
        _ => 1 + rng.next_u32() % 300,
    };
    let max_channels = 1 + rng.next_u32() % 8;
    let sr = SAMPLE_RATES[(rng.next_u32() % 13) as usize];
    let threshold = random_threshold(&mut rng);
    let mut l = Lim::create(max_attack_ms, release_ms, threshold, max_channels, sr);
    l.check("pcmLimiter_Create");
    let mut scaling = random_scaling(&mut rng);
    for step in 0..steps {
        let what = format!("seed {seed} step {step}");
        match rng.next_u32() % 24 {
            0 => {
                let t = random_threshold(&mut rng);
                let e = pcm_limiter_set_threshold(l.rs.as_deref_mut(), t);
                assert_eq!(e, unsafe { c::cref_limiter_set_threshold(l.c, t) }, "SetThreshold");
                l.check(&format!("pcmLimiter_SetThreshold ({what})"));
            }
            1 => {
                let ms = rng.next_u32() % (max_attack_ms + 3);
                let e = pcm_limiter_set_attack(l.rs.as_deref_mut(), ms);
                assert_eq!(e, unsafe { c::cref_limiter_set_attack(l.c, ms) }, "SetAttack");
                l.check(&format!("pcmLimiter_SetAttack({ms}) ({what})"));
            }
            2 => {
                let ms = match rng.next_u32() % 3 {
                    0 => rng.next_u32() % 100_000,
                    _ => rng.next_u32() % 1000,
                };
                let e = pcm_limiter_set_release(l.rs.as_deref_mut(), ms);
                assert_eq!(e, unsafe { c::cref_limiter_set_release(l.c, ms) }, "SetRelease");
                l.check(&format!("pcmLimiter_SetRelease({ms}) ({what})"));
            }
            3 => {
                let s = match rng.next_u32() % 4 {
                    0 => rng.next_u32() % 100_000,
                    _ => SAMPLE_RATES[(rng.next_u32() % 13) as usize],
                };
                let e = pcm_limiter_set_sample_rate(l.rs.as_deref_mut(), s);
                assert_eq!(e, unsafe { c::cref_limiter_set_sample_rate(l.c, s) }, "SetSampleRate");
                l.check(&format!("pcmLimiter_SetSampleRate({s}) ({what})"));
            }
            4 => {
                let n = rng.next_u32() % (max_channels + 2);
                let e = pcm_limiter_set_n_channels(l.rs.as_deref_mut(), n);
                assert_eq!(e, unsafe { c::cref_limiter_set_n_channels(l.c, n) }, "SetNChannels");
                l.check(&format!("pcmLimiter_SetNChannels({n}) ({what})"));
            }
            5 => {
                let e = pcm_limiter_reset(l.rs.as_deref_mut());
                assert_eq!(e, unsafe { c::cref_limiter_reset(l.c) }, "Reset");
                l.check(&format!("pcmLimiter_Reset ({what})"));
            }
            6 => {
                scaling = random_scaling(&mut rng);
            }
            _ => {
                apply(&mut l, &mut rng, scaling, &what);
                if l.rs().min_gain < 0x4000_0000 {
                    limited.0 += 1;
                } else {
                    limited.1 += 1;
                }
            }
        }
        let d = pcm_limiter_get_delay(l.rs());
        assert_eq!(d, unsafe { c::cref_limiter_get_delay(l.c) }, "pcmLimiter_GetDelay ({what})");
        let g = pcm_limiter_get_max_gain_reduction(l.rs());
        assert_eq!(
            g,
            unsafe { c::cref_limiter_get_max_gain_reduction(l.c) },
            "pcmLimiter_GetMaxGainReduction ({what})"
        );
    }
    limited
}

#[test]
fn pcm_limiter_random_sequences() {
    let mut limited = (0, 0);
    for seed in 0..400 {
        let (a, b) = run(seed, 60);
        limited = (limited.0 + a, limited.1 + b);
    }
    // Both the limiting and the transparent path are exercised.
    assert!(limited.0 > 1000 && limited.1 > 1000, "{limited:?}");
}

#[test]
fn pcm_limiter_long_sequences() {
    let mut limited = (0, 0);
    for seed in 1000..1012 {
        let (a, b) = run(seed, 1500);
        limited = (limited.0 + a, limited.1 + b);
    }
    assert!(limited.0 > 1000 && limited.1 > 1000, "{limited:?}");
}

/// Every sample rate with the decoder's configuration (pcmLimiter_Create(15, 50, MAXVAL_DBL, 8,
/// 96000), attack 5 ms, threshold 0.89125094) and loud signals.
#[test]
fn pcm_limiter_decoder_configuration() {
    let mut rng = Rng::new(77);
    for &sr in SAMPLE_RATES.iter() {
        for ch in 1..=8u32 {
            let mut l = Lim::create(TDL_ATTACK_DEFAULT_MS, TDL_RELEASE_DEFAULT_MS, 0x7FFF_FFFF, 8, 96000);
            let e = pcm_limiter_set_attack(l.rs.as_deref_mut(), 5);
            assert_eq!(e, unsafe { c::cref_limiter_set_attack(l.c, 5) });
            let e = pcm_limiter_set_threshold(l.rs.as_deref_mut(), 0x7214_8173);
            assert_eq!(e, unsafe { c::cref_limiter_set_threshold(l.c, 0x7214_8173) });
            let e = pcm_limiter_set_n_channels(l.rs.as_deref_mut(), ch);
            assert_eq!(e, unsafe { c::cref_limiter_set_n_channels(l.c, ch) });
            let e = pcm_limiter_set_sample_rate(l.rs.as_deref_mut(), sr);
            assert_eq!(e, unsafe { c::cref_limiter_set_sample_rate(l.c, sr) });
            l.check("setup");
            let scaling = rng.range(8, 11);
            for f in 0..12 {
                apply(&mut l, &mut rng, scaling, &format!("sr {sr} ch {ch} frame {f}"));
            }
        }
    }
}

/// Create with every sample rate and a range of attack and release times (the constants of
/// pcmLimiter_Create).
#[test]
fn pcm_limiter_create_constants() {
    let mut rng = Rng::new(5);
    for &sr in SAMPLE_RATES.iter() {
        for attack in 1..=30u32 {
            let release = rng.next_u32() % 2000;
            let l = Lim::create(attack, release, random_threshold(&mut rng), 1 + rng.next_u32() % 8, sr);
            let mut l = l;
            l.check(&format!("pcmLimiter_Create({attack}, {release}, sr {sr})"));
        }
    }
}

#[test]
fn pcm_limiter_null_handle() {
    let n = core::ptr::null_mut();
    unsafe {
        assert_eq!(pcm_limiter_reset(None), c::cref_limiter_reset(n));
        assert_eq!(pcm_limiter_destroy(None), c::cref_limiter_destroy(n));
        assert_eq!(pcm_limiter_set_threshold(None, 1), c::cref_limiter_set_threshold(n, 1));
        assert_eq!(pcm_limiter_set_n_channels(None, 1), c::cref_limiter_set_n_channels(n, 1));
        assert_eq!(pcm_limiter_set_sample_rate(None, 1), c::cref_limiter_set_sample_rate(n, 1));
        assert_eq!(pcm_limiter_set_attack(None, 1), c::cref_limiter_set_attack(n, 1));
        assert_eq!(pcm_limiter_set_release(None, 1), c::cref_limiter_set_release(n, 1));
        let mut i = [0i32; 4];
        let mut o = [0i16; 4];
        assert_eq!(
            pcm_limiter_apply(None, &i, &mut o, None, 0, 1),
            c::cref_limiter_apply(n, i.as_mut_ptr(), o.as_mut_ptr(), core::ptr::null_mut(), 0, 1)
        );
    }
}

const N: usize = FDK_MODULE_LAST as usize;

fn check_lib_info(ids_in: &[i32; N], null_info: bool) {
    let mut ids = [0i32; N];
    let mut version = [0i32; N];
    let mut flags = [0u32; N];
    let mut vstr = [0i8; 32 * N];
    let mut title = [0i8; 32 * N];
    let e = unsafe {
        c::cref_limiter_get_lib_info(
            null_info as i32,
            ids_in.as_ptr(),
            ids.as_mut_ptr(),
            version.as_mut_ptr(),
            flags.as_mut_ptr(),
            vstr.as_mut_ptr(),
            title.as_mut_ptr(),
        )
    };
    let mut info = [LibInfo::default(); N];
    for (i, e) in info.iter_mut().enumerate() {
        e.module_id = ids_in[i];
        e.version_str = [b'x'; 32];
    }
    let r = pcm_limiter_get_lib_info(if null_info { None } else { Some(&mut info) });
    assert_eq!(r, e, "pcmLimiter_GetLibInfo result for {ids_in:?}");
    for i in 0..N {
        let what = format!("entry {i} for {ids_in:?}");
        assert_eq!(info[i].module_id, ids[i], "{what}: module_id");
        assert_eq!(info[i].version, version[i], "{what}: version");
        assert_eq!(info[i].flags, flags[i], "{what}: flags");
        let cv: Vec<u8> = vstr[32 * i..32 * i + 32].iter().map(|&b| b as u8).collect();
        assert_eq!(info[i].version_str.as_slice(), cv.as_slice(), "{what}: versionStr");
        let ct: Vec<u8> =
            title[32 * i..32 * i + 32].iter().map(|&b| b as u8).take_while(|&b| b != 0).collect();
        assert_eq!(info[i].title.as_bytes(), ct.as_slice(), "{what}: title");
    }
}

#[test]
fn pcm_limiter_get_lib_info_() {
    assert_eq!(FDK_MODULE_LAST, unsafe { c::cref_limiter_module_last() });
    check_lib_info(&[0; N], true);
    check_lib_info(&[0; N], false);
    check_lib_info(&[3; N], false);
    let mut ids = [0; N];
    ids[0] = FDK_TDLIMIT;
    ids[1] = FDK_TOOLS;
    check_lib_info(&ids, false);
    let mut rng = Rng::new(402);
    for _ in 0..1000 {
        let mut ids = [0i32; N];
        let n = (rng.next_u32() as usize) % (N + 1);
        for id in ids.iter_mut().take(n) {
            *id = 1 + (rng.next_u32() % 40) as i32;
        }
        if rng.next_u32() % 2 == 0 {
            for id in ids.iter_mut().skip(n + 1) {
                *id = (rng.next_u32() % 40) as i32;
            }
        }
        check_lib_info(&ids, false);
    }
}
