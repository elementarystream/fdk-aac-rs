// Differential tests of fdk_aac_rs_aacenc::aacenc_pns (and the pns_func.h functions) against the
// C++ reference.
//
// The PNS configurations come from the real init chain (FDKaacEnc_InitPsyConfiguration, then
// FDKaacEnc_InitPnsConfiguration). FDKaacEnc_PnsDetect and the channel pair functions run on
// MDCT spectra of real PCM with band energies computed from them, and on random data; the
// complete PNS_DATA (copied from C into the port's #[repr(C)] struct) and all outputs are
// compared. One read of PnsDetect is reproduced from the C layout of PNS_DATA (the last word of
// noiseEnergyCorrelation); the tests fill that word with zero and non-zero values.

use crate::aacenc_tns_spectra::spectra;
use crate::noisedet::sfb_max_scale_spec;
use crate::psy_configuration::{block_types, init_psy_checked, GRANULES, RATES};
use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc_pns::*;
use fdk_aac_rs_aacenc::psy_configuration::PsyConfiguration;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_cref::aacenc_aacenc_pns as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{MAXVAL_DBL, MINVAL_DBL};

fn vp<T>(t: &mut T) -> *mut c_void {
    t as *mut T as *mut c_void
}

fn zeroed<T>() -> T {
    // SAFETY: only used for the integer-only C-layout structs.
    unsafe { core::mem::zeroed() }
}

fn init_pns_checked(
    p: &PsyConfiguration,
    a: (i32, i32, i32, i32, i32),
    stale: &PnsConfig,
) -> (i32, PnsConfig) {
    let (br, sr, use_pns, ch, lc) = a;
    let (mut cc, mut cr) = (*stale, *stale);
    let ec = unsafe {
        c::cref_aacenc_pns_InitPnsConfiguration(vp(&mut cc), br, sr, use_pns, p.sfb_cnt, p.sfb_offset.as_ptr(), ch, lc)
    };
    let er = fdk_aac_enc_init_pns_configuration(&mut cr, br, sr, use_pns, p.sfb_cnt, &p.sfb_offset, ch, lc);
    assert_eq!(er, ec, "{a:?}: return value");
    assert_eq!(cr, cc, "{a:?}: PNS_CONFIG");
    (er, cr)
}

#[test]
fn init_pns_configuration() {
    let mut r = Rng::new(0x7a53_0001);
    for &sr in &RATES {
        for &(gl, fb) in &GRANULES {
            for &bt in block_types(gl) {
                let (_, p) = init_psy_checked(48000, sr, sr / 2, bt, gl, 0, 0, fb);
                for _ in 0..40 {
                    let br = r.range(0, 999_999);
                    let mut stale: PnsConfig = zeroed();
                    if r.range(0, 1) == 0 {
                        stale.np.min_sfb_width = r.i32();
                        stale.use_pns = r.i32();
                        for x in stale.np.pow_dist_psd_curve.iter_mut() {
                            *x = r.i16();
                        }
                    }
                    let a = (br, sr, r.range(-1, 2), r.range(1, 2), (fb == FB_LC) as i32);
                    init_pns_checked(&p, a, &stale);
                }
            }
        }
    }
}

/// `log2(x) / 64` of a band energy as FIXP_DBL (the LD data of the encoder).
fn ld64(e: f64) -> i32 {
    if e <= 0.0 {
        i32::MIN
    } else {
        ((e.log2() / 64.0) * 2147483648.0).clamp(-2147483648.0, 2147483647.0) as i32
    }
}

/// Band energies (FIXP_DBL, scaled like the encoder's) and their LD data.
fn band_energies(spec: &[i32], off: &[i32], n: usize) -> (Vec<i32>, Vec<i32>) {
    let mut e = vec![0i32; MAX_GROUPED_SFB];
    let mut l = vec![i32::MIN; MAX_GROUPED_SFB];
    for b in 0..n {
        let mut s = 0.0f64;
        for j in off[b]..off[b + 1] {
            let x = spec[j as usize] as f64 / 2147483648.0;
            s += x * x;
        }
        e[b] = (s * 2147483648.0).min(2147483647.0) as i32;
        l[b] = ld64(s);
    }
    (e, l)
}

/// PnsDetect of C and of the port on copies of the inputs; compares PNS_DATA and noiseNrg.
#[allow(clippy::too_many_arguments)]
fn pns_detect_checked(
    conf: &PnsConfig,
    data: &mut (PnsData, PnsData),
    lws: i32,
    active: i32,
    max_sfb: i32,
    thr: &[i32],
    off: &[i32],
    spec: &[i32],
    ssc: &[i32],
    ton: &[i16],
    tns: (i32, i32, i32),
    nrg: &[i32],
    what: &str,
) -> Vec<i32> {
    let mut conf_c = *conf;
    let (mut thr_c, mut spec_c, mut ssc_c, mut ton_c, mut nrg_c) =
        (thr.to_vec(), spec.to_vec(), ssc.to_vec(), ton.to_vec(), nrg.to_vec());
    let fill = 0x77;
    let (mut nn_c, mut nn_r) = (vec![fill; MAX_GROUPED_SFB], vec![fill; MAX_GROUPED_SFB]);
    unsafe {
        c::cref_aacenc_pns_PnsDetect(
            vp(&mut conf_c),
            vp(&mut data.0),
            lws,
            active,
            max_sfb,
            thr_c.as_mut_ptr(),
            off.as_ptr(),
            spec_c.as_mut_ptr(),
            ssc_c.as_mut_ptr(),
            ton_c.as_mut_ptr(),
            tns.0,
            tns.1,
            tns.2,
            nrg_c.as_mut_ptr(),
            nn_c.as_mut_ptr(),
        )
    };
    fdk_aac_enc_pns_detect(
        conf, &mut data.1, lws, active, max_sfb, thr, off, spec, ssc, ton, tns.0, tns.1, tns.2, nrg,
        &mut nn_r,
    );
    assert_eq!(data.1, data.0, "{what}: PNS_DATA");
    assert_eq!(nn_r, nn_c, "{what}: noiseNrg");
    nn_r
}

fn random_pns_data(r: &mut Rng) -> PnsData {
    let mut d = PnsData::default();
    for x in d.noise_fuzzy_measure.iter_mut() {
        *x = r.i16();
    }
    for x in d.noise_energy_correlation.iter_mut() {
        *x = [0, 1, r.i32()][r.range(0, 2) as usize];
    }
    for x in d.pns_flag.iter_mut() {
        *x = r.range(-1, 1);
    }
    d
}

#[test]
fn pns_detect_realistic() {
    let mut r = Rng::new(0x7a53_0002);
    let mut stats = [0usize; 3];
    for (k, &sr) in RATES.iter().enumerate() {
        for &(gl, fb) in &[(1024, FB_LC), (512, FB_LD), (480, FB_LD), (512, FB_ELD)] {
            for &br in &[16000, 24000, 32000, 40000, 48000] {
                let (_, p) = init_psy_checked(br, sr, sr / 2, LONG_WINDOW, gl, 0, 0, fb);
                let (_, conf) = init_pns_checked(&p, (br, sr, 1, 1, (fb == FB_LC) as i32), &zeroed());
                if conf.use_pns == 0 {
                    continue;
                }
                let frames = spectra(&mut r, k + br as usize, gl, gl <= 512, 8);
                let mut data = (random_pns_data(&mut r), PnsData::default());
                data.1 = data.0;
                for (i, f) in frames.iter().enumerate() {
                    let active = p.sfb_active;
                    let spec: Vec<i32> = f.spectrum.iter().map(|&x| x >> 2).collect();
                    let ssc = sfb_max_scale_spec(&spec, &p.sfb_offset, active as usize);
                    let (_, eld) = band_energies(&spec, &p.sfb_offset, active as usize);
                    // threshold: energy minus a few dB, sometimes above
                    let thr: Vec<i32> = eld
                        .iter()
                        .map(|&e| e.saturating_sub(r.range(-0x0100_0000, 0x0400_0000)))
                        .collect();
                    let ton: Vec<i16> = (0..MAX_GROUPED_SFB).map(|_| r.range(0, 0x7fff) as i16).collect();
                    let max_sfb = match i % 4 {
                        0 => active,
                        1 => 1,
                        2 => r.range(0, 2),
                        _ => r.range(0, active),
                    };
                    let tns = (r.range(0, 12), r.range(1000, 2000), r.range(0, 1));
                    let lws = if i % 3 == 0 { f.block_type } else { LONG_WINDOW };
                    let nrg = pns_detect_checked(
                        &conf, &mut data, lws, active, max_sfb, &thr, &p.sfb_offset, &spec, &ssc, &ton,
                        tns, &eld, &format!("sr {sr} gl {gl} br {br} frame {i}"),
                    );
                    stats[0] += active as usize;
                    stats[1] += data.1.pns_flag.iter().filter(|&&f| f != 0).count();
                    stats[2] += nrg.iter().filter(|&&n| n != NO_NOISE_PNS).count();
                }
            }
        }
    }
    eprintln!("PnsDetect (bands, PNS flags, noise energies): {stats:?}");
    assert!(stats[1] > 0 && stats[2] > 0, "{stats:?}");
}

#[test]
fn pns_detect_random() {
    let mut r = Rng::new(0x7a53_0003);
    for it in 0..20_000 {
        let sr = RATES[r.range(0, 11) as usize];
        let (gl, fb) = [(1024, FB_LC), (512, FB_LD), (480, FB_ELD)][r.range(0, 2) as usize];
        let (_, p) = init_psy_checked(32000, sr, sr / 2, LONG_WINDOW, gl, 0, 0, fb);
        let (_, mut conf) = init_pns_checked(&p, (32000, sr, 1, r.range(1, 2), (fb == FB_LC) as i32), &zeroed());
        if it % 2 == 0 {
            conf.use_pns = r.range(0, 1);
            conf.np.detection_algorithm_flags = r.range(0, 63) as u16;
            conf.np.gap_fill_thr = r.i16();
            conf.np.start_sfb = r.range(-2, 30) as i16;
            conf.np.tns_gain_threshold = r.range(900, 1500);
            conf.np.tns_pns_gain_threshold = r.range(900, 1500);
        }
        let active = if it % 7 == 0 { r.range(0, MAX_GROUPED_SFB as i32) } else { p.sfb_active };
        let max_sfb = [1, 0, 2, active, r.range(0, active)][r.range(0, 4) as usize];
        // band offsets for up to 60 bands
        let off: Vec<i32> = if active > p.sfb_cnt {
            (0..=MAX_GROUPED_SFB as i32).map(|i| i * 16).collect()
        } else {
            p.sfb_offset.to_vec()
        };
        let spec: Vec<i32> = (0..1024).map(|_| r.i32() >> r.range(0, 31)).collect();
        let ssc: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| r.range(0, 30)).collect();
        let thr: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| r.i32()).collect();
        let eld: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| r.i32()).collect();
        let ton: Vec<i16> = (0..MAX_GROUPED_SFB).map(|_| r.i16()).collect();
        let tns = (r.range(-1, 13), r.range(0, 3000), r.range(0, 1));
        let lws = r.range(0, 3);
        let mut data = (random_pns_data(&mut r), PnsData::default());
        data.1 = data.0;
        pns_detect_checked(
            &conf, &mut data, lws, active, max_sfb, &thr, &off, &spec, &ssc, &ton, tns, &eld,
            &format!("it {it}"),
        );
    }
}

#[test]
fn noise_detection_and_noise_nrgs() {
    let mut r = Rng::new(0x7a53_0004);
    for it in 0..5000 {
        let sr = RATES[r.range(0, 11) as usize];
        let (_, p) = init_psy_checked(32000, sr, sr / 2, LONG_WINDOW, 1024, 0, 0, FB_LC);
        let (_, mut conf) = init_pns_checked(&p, (32000, sr, 1, 1, 1), &zeroed());
        conf.np.detection_algorithm_flags = r.range(0, 63) as u16;
        let spec: Vec<i32> = (0..1024).map(|_| r.i32() >> r.range(0, 31)).collect();
        let ssc = sfb_max_scale_spec(&spec, &p.sfb_offset, p.sfb_active as usize);
        let ton: Vec<i16> = (0..MAX_GROUPED_SFB).map(|_| r.i16()).collect();
        let tns = (r.range(0, 12), r.range(900, 2000), r.range(0, 1));
        let mut dc = random_pns_data(&mut r);
        let mut dr = dc;
        let mut cc = conf;
        let (mut sc, mut sscc, mut tc) = (spec.clone(), ssc.clone(), ton.clone());
        unsafe {
            c::cref_aacenc_pns_noiseDetection(vp(&mut cc), vp(&mut dc), p.sfb_active, p.sfb_offset.as_ptr(), tns.0, tns.1, tns.2, sc.as_mut_ptr(), sscc.as_mut_ptr(), tc.as_mut_ptr())
        };
        fdk_aac_enc_fdk_aac_enc_noise_detection(&conf, &mut dr, p.sfb_active, &p.sfb_offset, tns.0, tns.1, tns.2, &spec, &ssc, &ton);
        assert_eq!(dr, dc, "it {it}: noiseDetection");

        let active = r.range(0, MAX_GROUPED_SFB as i32);
        let mut flags: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| r.range(0, 1)).collect();
        let mut eld: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| r.i32()).collect();
        let fill = r.i32();
        let (mut nc, mut nr) = (vec![fill; MAX_GROUPED_SFB], vec![fill; MAX_GROUPED_SFB]);
        unsafe { c::cref_aacenc_pns_CalcNoiseNrgs(active, flags.as_mut_ptr(), eld.as_mut_ptr(), nc.as_mut_ptr()) };
        fdk_aac_enc_calc_noise_nrgs(active, &flags, &eld, &mut nr);
        assert_eq!(nr, nc, "it {it}: CalcNoiseNrgs");
    }
}

#[test]
fn code_pns_channel() {
    let mut r = Rng::new(0x7a53_0005);
    for it in 0..50_000 {
        let mut conf: PnsConfig = zeroed();
        conf.use_pns = [0, 1, 1, 2][r.range(0, 3) as usize];
        let active = r.range(0, MAX_GROUPED_SFB as i32);
        let flags: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| [0, 1, 1, -3][r.range(0, 3) as usize]).collect();
        let eld: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| if it % 2 == 0 { r.range(-0x4000_0000, 0x1000_0000) } else { r.i32() }).collect();
        let nrg: Vec<i32> = (0..MAX_GROUPED_SFB)
            .map(|_| match r.range(0, 3) {
                0 => NO_NOISE_PNS,
                1 => r.range(-200, 300),
                2 => r.range(-30, 90),
                _ => r.i32(),
            })
            .collect();
        let thr: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| r.i32()).collect();
        let (mut fc, mut ec, mut nc, mut tc) = (flags.clone(), eld.clone(), nrg.clone(), thr.clone());
        let (mut nr, mut tr) = (nrg.clone(), thr.clone());
        unsafe { c::cref_aacenc_pns_CodePnsChannel(active, vp(&mut conf), fc.as_mut_ptr(), ec.as_mut_ptr(), nc.as_mut_ptr(), tc.as_mut_ptr()) };
        fdk_aac_enc_code_pns_channel(active, &conf, &flags, &eld, &mut nr, &mut tr);
        assert_eq!((nr, tr), (nc, tc), "it {it}");
    }
}

/// Whether the C++ reference of this test binary was compiled by Apple clang for x86-64
/// (`--target x86_64-apple-darwin`, ref-x86_64-mac). For `accu == 0` and a positive LD energy, it
/// computes `ccf` without testing the sign of the overflowed sum, while the port's `x86` variant
/// follows ref-x86_64 (Debian clang 19).
const APPLE_CLANG_X86: bool = cfg!(all(target_arch = "x86_64", target_vendor = "apple"));

/// FDKaacEnc_PreProcessPnsChannelPair at its undefined operations: `accu == 0` with LD
/// energies whose sum `quot` is around and above 1/64 (`CalcLdData(0) + 1/64 - quot` overflows
/// for `quot > 1/64`), and `accu == MINVAL_DBL` (`fixp_abs` overflows) with every sign of `quot`.
#[test]
fn pns_channel_pair_overflow() {
    if APPLE_CLANG_X86 {
        eprintln!("skipped: the reference is ref-x86_64-mac (Apple clang)");
        return;
    }
    let mut r = Rng::new(0x7a53_0007);
    let quots = [
        -0x4000_0000,
        -1,
        0,
        1,
        0x01ff_ffff,
        0x0200_0000,
        0x0200_0001,
        0x0400_0000,
        0x1000_0000,
        0x3fff_ffff,
        0x7fff_fffe,
    ];
    let mut cases = 0;
    for it in 0..4000 {
        let mut el = vec![0i32; MAX_GROUPED_SFB];
        let mut er = vec![0i32; MAX_GROUPED_SFB];
        let mut ell = vec![0i32; MAX_GROUPED_SFB];
        let mut erl = vec![0i32; MAX_GROUPED_SFB];
        let mut em = vec![0i32; MAX_GROUPED_SFB];
        for b in 0..MAX_GROUPED_SFB {
            // quot = (ell >> 1) + (erl >> 1): q up to the low bit
            let q = if r.range(0, 3) == 0 {
                r.i32()
            } else {
                quots[r.range(0, quots.len() as i32 - 1) as usize]
            };
            let d = r.range(-0x100, 0x100);
            ell[b] = q.saturating_add(d);
            erl[b] = q.saturating_sub(d);
            el[b] = r.i32();
            er[b] = r.i32();
            let mid = ((el[b] >> 1) + (er[b] >> 1)) >> 1;
            em[b] = match r.range(0, 3) {
                0 | 1 => mid,
                2 => mid.wrapping_add(i32::MIN),
                _ => r.i32(),
            };
        }
        let mut conf: PnsConfig = zeroed();
        conf.use_pns = 1;
        let mut dl = random_pns_data(&mut r);
        let mut dr_ = random_pns_data(&mut r);
        let (mut cl, mut cr) = (dl, dr_);
        let active = r.range(1, MAX_GROUPED_SFB as i32);
        let (mut a1, mut a2, mut a3, mut a4, mut a5) = (el.clone(), er.clone(), ell.clone(), erl.clone(), em.clone());
        unsafe {
            c::cref_aacenc_pns_PreProcessPnsChannelPair(active, a1.as_mut_ptr(), a2.as_mut_ptr(), a3.as_mut_ptr(), a4.as_mut_ptr(), a5.as_mut_ptr(), vp(&mut conf), vp(&mut cl), vp(&mut cr))
        };
        fdk_aac_enc_pre_process_pns_channel_pair(active, &el, &er, &ell, &erl, &em, &conf, &mut dl, &mut dr_);
        assert_eq!((dl, dr_), (cl, cr), "it {it}: PreProcessPnsChannelPair");
        cases += active;
    }
    assert!(cases > 10000, "{cases}");
}

#[test]
fn pns_channel_pair() {
    let mut r = Rng::new(0x7a53_0006);
    // realistic energies from the spectra of two channels
    let mut realistic = Vec::new();
    for (k, &sr) in RATES.iter().enumerate() {
        let (_, p) = init_psy_checked(32000, sr, sr / 2, LONG_WINDOW, 1024, 0, 0, FB_LC);
        let a = spectra(&mut r, k, 1024, false, 4);
        let b = spectra(&mut r, k + 3, 1024, false, 4);
        for (fa, fb) in a.iter().zip(&b) {
            let n = p.sfb_active as usize;
            let l: Vec<i32> = fa.spectrum.iter().map(|&x| x >> 2).collect();
            let rr: Vec<i32> = fb.spectrum.iter().zip(&l).map(|(&x, &y)| (x >> 3) + (y >> 1)).collect();
            let m: Vec<i32> = l.iter().zip(&rr).map(|(&x, &y)| (x >> 1) + (y >> 1)).collect();
            let (el, ell) = band_energies(&l, &p.sfb_offset, n);
            let (er, erl) = band_energies(&rr, &p.sfb_offset, n);
            let (em, _) = band_energies(&m, &p.sfb_offset, n);
            realistic.push((p.sfb_active, el, er, ell, erl, em));
        }
    }
    for it in 0..20_000 {
        let (active, el, er, ell, erl, em) = if it % 4 == 0 {
            realistic[r.range(0, realistic.len() as i32 - 1) as usize].clone()
        } else {
            // Random energies, in the encoder's domain for it % 4 == 1 and 2: band energies are
            // not negative, LD energies (CalcLdData of an energy) not positive. Outside it
            // (it % 4 == 3, every value random), the C++ code overflows in `sfbEnergyMid -
            // (...)` (fixp_abs(MINVAL_DBL) for a difference of MINVAL_DBL) and in
            // `CalcLdData(accu) + 1/64 - quot` (quot > 1/64 with accu == 0), undefined behavior
            // that the reference builds compile differently (Apple clang 21 x86 gives 0 for
            // that ccf, Debian clang 19 x86 and arm64 MAXVAL_DBL; the port follows the latter,
            // so the flavour is restricted where the reference is Apple clang x86). An
            // UndefinedBehaviorSanitizer build of the reference (arm64) encoded all 785 encoder
            // entries of the e2e corpus without a report in aacenc_pns.cpp or abs.h.
            let edge = |x: i32, r: &mut Rng, e: [i32; 4]| {
                if r.range(0, 9) == 0 {
                    e[r.range(0, 3) as usize]
                } else {
                    x
                }
            };
            let pos = |r: &mut Rng| -> Vec<i32> {
                let e = [0, 1, MAXVAL_DBL, MAXVAL_DBL - 1];
                (0..MAX_GROUPED_SFB).map(|_| edge(r.i32() & 0x7fff_ffff, r, e)).collect()
            };
            let ld = |r: &mut Rng| -> Vec<i32> {
                let e = [0, -1, MINVAL_DBL, MINVAL_DBL + 1];
                (0..MAX_GROUPED_SFB).map(|_| edge(r.range(-0x7000_0000, 0), r, e)).collect()
            };
            let ld_any = |r: &mut Rng| -> Vec<i32> {
                let e = [0, -1, MINVAL_DBL, -0x2000_0000];
                (0..MAX_GROUPED_SFB).map(|_| edge(r.i32() | i32::MIN, r, e)).collect()
            };
            let v = |r: &mut Rng| -> Vec<i32> { (0..MAX_GROUPED_SFB).map(|_| r.i32()).collect() };
            if it % 4 == 2 {
                (r.range(0, 60), pos(&mut r), pos(&mut r), ld(&mut r), ld(&mut r), pos(&mut r))
            } else if it % 4 == 1 || APPLE_CLANG_X86 {
                (r.range(0, 60), pos(&mut r), pos(&mut r), ld_any(&mut r), ld_any(&mut r), pos(&mut r))
            } else {
                (r.range(0, 60), v(&mut r), v(&mut r), v(&mut r), v(&mut r), v(&mut r))
            }
        };
        let mut em = em;
        if it % 4 != 0 {
            // accu == 0 in some bands, and accu == MINVAL_DBL (fixp_abs overflows) outside the
            // encoder's domain
            let unrestricted = it % 4 == 3 && !APPLE_CLANG_X86;
            for b in 0..MAX_GROUPED_SFB {
                let mid = ((el[b] >> 1) + (er[b] >> 1)) >> 1;
                match r.range(0, 5) {
                    0 => em[b] = mid,
                    1 if unrestricted => em[b] = mid.wrapping_add(i32::MIN),
                    _ => {}
                }
            }
        }
        let mut conf: PnsConfig = zeroed();
        conf.use_pns = r.range(0, 2);
        conf.noise_correlation_thresh = [0x2e147ae1, r.i32()][r.range(0, 1) as usize];
        let mut dl = random_pns_data(&mut r);
        let mut dr_ = random_pns_data(&mut r);
        let (mut cl, mut cr) = (dl, dr_);
        let (mut a1, mut a2, mut a3, mut a4, mut a5) = (el.clone(), er.clone(), ell.clone(), erl.clone(), em.clone());
        unsafe {
            c::cref_aacenc_pns_PreProcessPnsChannelPair(active, a1.as_mut_ptr(), a2.as_mut_ptr(), a3.as_mut_ptr(), a4.as_mut_ptr(), a5.as_mut_ptr(), vp(&mut conf), vp(&mut cl), vp(&mut cr))
        };
        fdk_aac_enc_pre_process_pns_channel_pair(active, &el, &er, &ell, &erl, &em, &conf, &mut dl, &mut dr_);
        assert_eq!((dl, dr_), (cl, cr), "it {it}: PreProcessPnsChannelPair");

        // PostProcess on the result
        let mask: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| r.range(0, 1)).collect();
        let digest = r.range(0, 2);
        let (mut mc, mut mr) = (mask.clone(), mask);
        let (mut gc, mut gr) = (digest, digest);
        unsafe {
            c::cref_aacenc_pns_PostProcessPnsChannelPair(active, vp(&mut conf), vp(&mut cl), vp(&mut cr), mc.as_mut_ptr(), &mut gc)
        };
        fdk_aac_enc_post_process_pns_channel_pair(active, &conf, &mut dl, &mut dr_, &mut mr, &mut gr);
        assert_eq!((dl, dr_), (cl, cr), "it {it}: PostProcessPnsChannelPair PNS_DATA");
        assert_eq!((mr, gr), (mc, gc), "it {it}: PostProcessPnsChannelPair msMask, msDigest");
    }
}
