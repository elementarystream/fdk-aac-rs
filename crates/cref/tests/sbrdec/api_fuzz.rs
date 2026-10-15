// Randomized differential test of the API of fdk_aac_rs_sbrdec against the C++ reference.
//
// Random call sequences (configurations of HE-AAC, HE-AAC v2, downsampled SBR, ELD, USAC with
// HBE, PVC and inter-TES, DRM; random and partly structured SBR payloads, random core signals,
// DRC gains, parameter changes, flushes, resets and invalid arguments) are injected into the
// replay instance of shim/sbrdec_e2e.cpp and into the port; after every call the results, the
// output and the complete state are compared (the comparison of tests/sbrdec/e2e.rs).

use super::e2e::*;
use core::ffi::c_void;
use fdk_aac_rs_cref::sbrdec_e2e::*;
use fdk_aac_rs_cref::testutil::Rng;

/// A bit writer (MSB first).
struct Bits {
    bytes: Vec<u8>,
    n: usize,
}

impl Bits {
    fn new() -> Self {
        Bits { bytes: Vec::new(), n: 0 }
    }
    fn put(&mut self, v: u32, bits: u32) {
        for k in (0..bits).rev() {
            if self.n % 8 == 0 {
                self.bytes.push(0);
            }
            if (v >> k) & 1 != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
    }
    fn random(&mut self, rng: &mut Rng, bits: usize) {
        for _ in 0..bits {
            self.put(rng.next_u32() & 1, 1);
        }
    }
}

/// A configuration of the decoder that drives the SBR decoder.
#[derive(Clone, Copy, Debug)]
struct Config {
    aot: i32,
    sr_in: i32,
    sr_out: i32,
    frame: i32,
    element: i32,
    element_index: i32,
    harmonic: u8,
    downscale: i32,
    /// USAC: bs_pvc / bs_inter_tes of the element
    pvc: bool,
    ites: bool,
}

const AOT_AAC_LC: i32 = 2;
const AOT_SBR: i32 = 5;
const AOT_PS: i32 = 29;
const AOT_ER_AAC_ELD: i32 = 39;
const AOT_USAC: i32 = 42;
const AOT_DRM_AAC: i32 = 143;

fn random_config(rng: &mut Rng) -> Config {
    let pick = |rng: &mut Rng, v: &[i32]| v[(rng.next_u32() as usize) % v.len()];
    let kind = rng.next_u32() % 6;
    let mut c = Config {
        aot: AOT_SBR,
        sr_in: 24000,
        sr_out: 48000,
        frame: 1024,
        element: 0,
        element_index: 0,
        harmonic: 0,
        downscale: 1,
        pvc: false,
        ites: false,
    };
    match kind {
        0 | 1 => {
            // HE-AAC, HE-AAC v2 (dual rate or downsampled)
            c.aot = pick(rng, &[AOT_AAC_LC, AOT_SBR, AOT_PS]);
            c.sr_in = pick(rng, &[8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000]);
            c.sr_out = match rng.next_u32() % 4 {
                0 => c.sr_in,
                1 => 0,
                _ => 2 * c.sr_in,
            };
            c.frame = pick(rng, &[1024, 960]);
            c.element = pick(rng, &[0, 0, 1, 1, 3]);
        }
        2 => {
            // ELD
            c.aot = AOT_ER_AAC_ELD;
            c.sr_in = pick(rng, &[16000, 22050, 24000, 32000, 44100, 48000]);
            c.sr_out = if rng.next_u32() % 3 == 0 { c.sr_in } else { 2 * c.sr_in };
            c.frame = pick(rng, &[512, 480]);
            c.element = pick(rng, &[0, 1]);
            c.downscale = pick(rng, &[1, 1, 1, 2, 4]);
        }
        3 | 4 => {
            // USAC: 2:1, 8:3, 4:1
            c.aot = AOT_USAC;
            let r = rng.next_u32() % 3;
            c.sr_out = pick(rng, &[32000, 44100, 48000, 38400, 25600]);
            let (num, den, frame) = match r {
                0 => (1, 2, 1024),
                1 => (3, 8, 768),
                _ => (1, 4, 1024),
            };
            c.sr_in = c.sr_out * num / den;
            c.frame = frame;
            c.element = pick(rng, &[0, 1]);
            c.harmonic = (rng.next_u32() % 2) as u8;
            c.pvc = rng.next_u32() % 2 == 0;
            c.ites = rng.next_u32() % 2 == 0;
        }
        _ => {
            // DRM (reverse SBR syntax, CRC)
            c.aot = AOT_DRM_AAC;
            c.sr_in = pick(rng, &[12000, 16000, 24000]);
            c.sr_out = 2 * c.sr_in;
            c.frame = 960;
            c.element = pick(rng, &[0, 1]);
        }
    }
    // Element indices 0 to 7 (InitElement also gets larger ones).
    if rng.next_u32() % 16 == 0 {
        c.element_index = rng.range(0, 7);
    }
    if rng.next_u32() % 32 == 0 {
        c.element = rng.range(-1, 7);
    }
    c
}

/// An SBR header (bs_amp_res .. bs_smoothing_mode) with random values.
fn put_header(b: &mut Bits, rng: &mut Rng, usac: bool) {
    if !usac {
        b.put(rng.next_u32() & 1, 1); // bs_amp_res
    }
    b.put(rng.next_u32() & 15, 4); // bs_start_freq
    b.put(rng.next_u32() & 15, 4); // bs_stop_freq
    if !usac {
        b.put(rng.next_u32() & 7, 3); // bs_xover_band
        b.put(0, 2); // bs_reserved
    }
    let extra1 = rng.next_u32() & 1;
    let extra2 = rng.next_u32() & 1;
    b.put(extra1, 1);
    b.put(extra2, 1);
    if extra1 != 0 {
        b.put(rng.next_u32() & 3, 2); // bs_freq_scale
        b.put(rng.next_u32() & 1, 1); // bs_alter_scale
        b.put(rng.next_u32() & 3, 2); // bs_noise_bands
    }
    if extra2 != 0 {
        b.put(rng.next_u32() & 3, 2); // bs_limiter_bands
        b.put(rng.next_u32() & 3, 2); // bs_limiter_gains
        b.put(rng.next_u32() & 1, 1); // bs_interpol_freq
        b.put(rng.next_u32() & 1, 1); // bs_smoothing_mode
    }
}

/// A payload: mostly a header flag with a random header, then random frame bits.
fn payload(rng: &mut Rng, c: &Config, crc: bool) -> Bits {
    let mut b = Bits::new();
    if crc {
        b.random(rng, if c.aot == AOT_DRM_AAC { 8 } else { 10 });
    }
    match rng.next_u32() % 8 {
        0 => {
            let n = rng.range(0, 64) as usize;
            b.random(rng, n)
        }
        1 => {
            let n = rng.range(64, 2000) as usize;
            b.random(rng, n)
        }
        _ => {
            if c.aot == AOT_USAC {
                // sbrInfoPresent, sbrHeaderPresent (without independency)
                b.put(rng.next_u32() & 1, 1);
                b.put(rng.next_u32() & 1, 1);
                b.put(rng.next_u32() & 1, 1); // ampResolution
                b.put(rng.next_u32() & 15, 4); // xover_band
                b.put(rng.next_u32() & 1, 1); // sbr_preprocessing
                if c.pvc {
                    b.put(rng.next_u32() % 3, 2);
                }
                b.put(rng.next_u32() & 1, 1); // useDfltHeader
                put_header(&mut b, rng, true);
            } else {
                b.put(1, 1); // bs_header_flag
                put_header(&mut b, rng, false);
            }
            let n = rng.range(0, 400) as usize;
            b.random(rng, n);
        }
    }
    b
}

/// A buffer for a bitstream: a power of two of at least 512 bytes.
fn bs_buffer(b: &Bits) -> Vec<u8> {
    let mut size = 512;
    while size < b.bytes.len() {
        size *= 2;
    }
    let mut v = b.bytes.clone();
    v.resize(size, 0);
    v
}

fn signal(rng: &mut Rng, n: usize) -> Vec<i32> {
    let kind = rng.next_u32() % 6;
    let amp_shift = rng.range(0, 31) as u32;
    let mut v = vec![0i32; n];
    let mut phase = 0u32;
    let step = rng.next_u32() >> 6;
    for (k, x) in v.iter_mut().enumerate() {
        *x = match kind {
            0 => 0,
            1 => rng.i32(),
            2 => (rng.next_u32() as i32) >> amp_shift,
            3 => {
                phase = phase.wrapping_add(step);
                (((phase >> 16) as i16 as i32) << 16) >> amp_shift
            }
            4 => {
                if k % 97 == 0 {
                    i32::MAX >> amp_shift
                } else {
                    0
                }
            }
            _ => {
                if rng.next_u32() % 2 == 0 {
                    i32::MAX
                } else {
                    i32::MIN
                }
            }
        };
    }
    // the core signal of the AAC decoder has 3 bits of headroom (aacOutDataHeadroom)
    for x in v.iter_mut() {
        *x >>= 3;
    }
    v
}

/// One random call sequence; returns (events, applies, coverage) or the first difference.
fn scenario(seed: u64, frames: usize) -> Result<(usize, usize, [usize; 10]), String> {
    scenario_mode(seed, frames, false).map(|(e, a, _, _, cov)| (e, a, cov))
}

/// Runs a scenario; with `ref_only` only the reference runs and a hash of its results per
/// event is returned.
#[allow(clippy::type_complexity)]
fn scenario_mode(
    seed: u64,
    frames: usize,
    ref_only: bool,
) -> Result<(usize, usize, Config, Vec<u64>, [usize; 10]), String> {
    let _lock = harness_lock();
    let mut rng = Rng::new(seed);
    let c = random_config(&mut rng);
    let mut p = Box::<Port>::default();
    p.ref_only = ref_only;
    let port: *mut Port = Box::into_raw(p);
    unsafe {
        cref_sbrdec_e2e_set_callback(callback as *mut c_void, port as *mut c_void, 0);
        cref_sbrdec_e2e_inject_reset();
        cref_sbrdec_e2e_inject_open();
    }
    let err = || unsafe { (*port).error.clone() };
    let elp = [
        c.sr_in,
        c.sr_out,
        c.frame,
        c.aot,
        c.element,
        c.element_index,
        c.harmonic as i32,
        0,
        0,
        0,
        c.downscale,
    ];
    let usac = c.aot == AOT_USAC;
    let n_ch = if c.element == 1 { 2 } else { 1 };
    let drm = c.aot == AOT_DRM_AAC;

    let result = (|| -> Result<(), String> {
        // decoder parameters before the configuration
        if rng.next_u32() % 3 == 0 {
            unsafe { cref_sbrdec_e2e_inject_set_param(0, 1, (rng.next_u32() % 2) as i32) }; // SBR_QMF_MODE
        }
        if rng.next_u32() % 4 == 0 {
            unsafe { cref_sbrdec_e2e_inject_set_param(0, 0, rng.range(-1, 2)) }; // bitstream delay
        }
        if c.aot == AOT_ER_AAC_ELD && rng.next_u32() % 2 == 0 {
            unsafe { cref_sbrdec_e2e_inject_set_param(0, 2, 1) }; // SBR_LD_QMF_TIME_ALIGN
        }
        // configuration (USAC and ELD carry the header in the configuration)
        if usac || c.aot == AOT_ER_AAC_ELD {
            let mut b = Bits::new();
            if usac {
                put_header(&mut b, &mut rng, true);
            } else {
                b.put(1, 1);
                put_header(&mut b, &mut rng, false);
            }
            let buf = bs_buffer(&b);
            let mut p = elp;
            p[8] = 2; // AC_CM_ALLOC_MEM
            unsafe {
                cref_sbrdec_e2e_inject_header(buf.as_ptr(), buf.len() as i32, b.n as i32, p.as_ptr())
            };
        } else {
            unsafe { cref_sbrdec_e2e_inject_init_element(elp.as_ptr()) };
        }
        if let Some(e) = err() {
            return Err(e);
        }
        if rng.next_u32() % 8 == 0 {
            // a second element or a reconfiguration
            let mut p = elp;
            p[5] = rng.range(0, 9);
            p[4] = rng.range(0, 3);
            unsafe { cref_sbrdec_e2e_inject_init_element(p.as_ptr()) };
        }
        unsafe { cref_sbrdec_e2e_inject_simple(21) }; // FDK_QmfDomain_Configure
        if let Some(e) = err() {
            return Err(e);
        }

        let time_size = 4 * 2048 * 2;
        for f in 0..frames {
            match rng.next_u32() % 24 {
                0 => unsafe { cref_sbrdec_e2e_inject_set_param(0, 3, 1) }, // SBR_FLUSH_DATA
                1 => unsafe { cref_sbrdec_e2e_inject_set_param(0, 4, 1) }, // SBR_CLEAR_HISTORY
                2 => unsafe { cref_sbrdec_e2e_inject_set_param(0, 5, 0) }, // SBR_BS_INTERRUPTION
                3 => {
                    // any parameter; SBR_SKIP_QMF only with 0 or 2 (skipping the analysis needs
                    // QMF data from an MPEG Surround decoder)
                    let param = rng.range(0, 8);
                    let value = if param == 6 { 2 * (rng.next_u32() % 2) as i32 } else { rng.range(-2, 3) };
                    unsafe { cref_sbrdec_e2e_inject_set_param(0, param, value) }
                }
                4 => unsafe { cref_sbrdec_e2e_inject_simple(11) }, // GetDelay
                5 => unsafe { cref_sbrdec_e2e_inject_simple(23) }, // ClearPersistentMemory
                6 => unsafe { cref_sbrdec_e2e_inject_set_param(0, 1, (rng.next_u32() % 2) as i32) },
                _ => {}
            }
            // DRC gains
            if rng.next_u32() % 3 == 0 {
                for ch in 0..n_ch.max(1) + 1 {
                    if rng.next_u32() % 5 == 0 {
                        unsafe { cref_sbrdec_e2e_inject_drc_disable(ch) };
                        continue;
                    }
                    let nb = rng.range(1, 16);
                    let mag: Vec<i32> = (0..nb).map(|_| (rng.next_u32() >> 1) as i32).collect();
                    // Band tops of at most 255 (drc_band_top is an 8-bit field); the port skips
                    // the bins beyond the 64 of a QMF slot.
                    let mut top: Vec<u16> = Vec::new();
                    let mut t = 0u16;
                    for _ in 0..nb {
                        t = (t + rng.range(0, 40) as u16).min(255);
                        top.push(t);
                    }
                    unsafe {
                        cref_sbrdec_e2e_inject_drc_feed(
                            ch,
                            nb,
                            mag.as_ptr(),
                            rng.range(0, 8),
                            rng.range(0, 7),
                            rng.range(0, 3),
                            top.as_ptr(),
                        )
                    };
                }
            }
            // payload
            if rng.next_u32() % 10 != 0 {
                let crc = drm || rng.next_u32() % 6 == 0;
                let b = payload(&mut rng, &c, crc);
                let buf = bs_buffer(&b);
                let indep = if usac && (f == 0 || rng.next_u32() % 4 == 0) { 0x0010_0000 } else { 0 };
                let mut el_flags = 0u32;
                if c.pvc {
                    el_flags |= 0x8;
                }
                if c.ites {
                    el_flags |= 0x4;
                }
                let ac: Vec<u32> = vec![el_flags; (c.element_index + 1).max(1) as usize];
                let count = if rng.next_u32() % 16 == 0 { rng.range(-8, 8) } else { b.n as i32 };
                let p = [
                    count,
                    if rng.next_u32() % 2 == 0 { b.n as i32 } else { 0 },
                    crc as i32,
                    if rng.next_u32() % 16 == 0 { rng.range(-1, 7) } else { c.element },
                    c.element_index,
                    indep,
                    if drm { 512 } else { 0 },
                ];
                unsafe {
                    cref_sbrdec_e2e_inject_parse(buf.as_ptr(), buf.len() as i32, b.n as i32, p.as_ptr(), ac.as_ptr())
                };
            }
            // apply
            let input = signal(&mut rng, 8 * 1024);
            let time = vec![0i32; time_size];
            let ps_possible = (c.aot != AOT_USAC && c.element == 0 && rng.next_u32() % 4 != 0) as i32;
            let p = [
                if rng.next_u32() % 32 == 0 { rng.range(0, time_size as i32) } else { time_size as i32 },
                if rng.next_u32() % 32 == 0 { rng.range(-1, 4) } else { n_ch },
                c.sr_in,
                0,
                (rng.next_u32() % 8 != 0) as i32,
                ps_possible,
                3, // inDataHeadroom: aacOutDataHeadroom of the AAC decoder (always 3)
                if rng.next_u32() % 16 == 0 { -1 } else { (rng.next_u32() % 2) as i32 },
            ];
            unsafe {
                cref_sbrdec_e2e_inject_apply(input.as_ptr(), input.len() as i32, time.as_ptr(), p.as_ptr())
            };
            if let Some(e) = err() {
                return Err(format!("frame {f}: {e}"));
            }
        }
        if rng.next_u32() % 2 == 0 {
            unsafe { cref_sbrdec_e2e_inject_simple(9) }; // sbrDecoder_FreeMem
        }
        unsafe { cref_sbrdec_e2e_inject_simple(10) }; // sbrDecoder_Close
        unsafe { cref_sbrdec_e2e_inject_simple(24) }; // FDK_QmfDomain_Close
        match err() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    })();
    unsafe { cref_sbrdec_e2e_set_callback(core::ptr::null_mut(), core::ptr::null_mut(), 0) };
    let port = unsafe { Box::from_raw(port) };
    result?;
    Ok((port.events, port.applies, c, port.ref_hashes, port.cov))
}

/// Seeds (beyond the default 400 scenarios) without a defined reference result that
/// `nondeterministic_event()` does not always detect.
const KNOWN_UNDEFINED: &[(u64, &str)] = &[
    (1492, "no defined reference result"),
    (1708, "no defined reference result"),
    (2128, "no defined reference result"),
];

/// The first event without a defined reference result: the scenario is run twice on the
/// reference only, with two different scribble patterns, and the results are compared.
fn nondeterministic_event(seed: u64, frames: usize) -> Option<usize> {
    let run = |pattern: i32| {
        unsafe { cref_sbrdec_e2e_set_scribble(pattern) };
        let r = scenario_mode(seed, frames, true).map(|r| r.3).unwrap_or_default();
        unsafe { cref_sbrdec_e2e_set_scribble(0) };
        r
    };
    let (a, b) = (run(1), run(2));
    (0..a.len().max(b.len())).find(|&k| a.get(k) != b.get(k)).map(|k| k + 1)
}

#[test]
fn api_fuzz() {
    let n: u64 = std::env::var("SBRDEC_FUZZ_SCENARIOS").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
    let mut failures = Vec::new();
    let mut nondeterministic = Vec::new();
    let mut events = 0;
    let mut applies = 0;
    let mut coverage = [0usize; 10];
    let first: u64 = std::env::var("SBRDEC_FUZZ_FIRST").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let trace = std::env::var("SBRDEC_FUZZ_TRACE").is_ok();
    for seed in first..first + n {
        if trace {
            eprintln!("seed {seed} {:?}", random_config(&mut Rng::new(seed)));
        }
        match scenario(seed, 24) {
            Ok((e, a, cov)) => {
                events += e;
                applies += a;
                for k in 0..cov.len() {
                    coverage[k] += cov[k];
                }
            }
            Err(e) => {
                let c = random_config(&mut Rng::new(seed));
                // A difference after the first event without a defined reference result is not
                // a port error.
                let event = e
                    .split("event ")
                    .nth(1)
                    .and_then(|r| r.split(' ').next())
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(usize::MAX);
                if let Some((_, why)) = KNOWN_UNDEFINED.iter().find(|(s, _)| *s == seed) {
                    eprintln!("seed {seed} {c:?}: known seed ({why}); port difference: {e}");
                    nondeterministic.push(seed);
                    continue;
                }
                match nondeterministic_event(seed, 24) {
                    Some(k) if k <= event => {
                        eprintln!("seed {seed} {c:?}: no defined reference result from event {k} on (port difference: {e})");
                        nondeterministic.push(seed);
                    }
                    _ => {
                        eprintln!("FAIL seed {seed} {c:?}: {e}");
                        failures.push(seed);
                    }
                }
            }
        }
    }
    eprintln!(
        "api fuzz: {n} scenarios, {events} events, {applies} applies, coverage (hbe pvc ites ps \
         eld 4:1 lp ds drc ps+ds) {coverage:?}; {} scenarios without a defined reference \
         result: {nondeterministic:?}",
        nondeterministic.len()
    );
    assert!(failures.is_empty(), "{} failing seeds: {failures:?}", failures.len());
}
