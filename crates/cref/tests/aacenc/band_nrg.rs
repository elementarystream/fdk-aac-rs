// Differential tests of fdk_aac_rs_aacenc::band_nrg against the C++ reference.
//
// Every function is compared on random spectra and band layouts (including MINVAL_DBL,
// MAXVAL_DBL, zero bands and saturating energies) and on realistic data: MDCT spectra of real and
// synthetic PCM from the ported filter banks (transform.rs), with the scalefactor bands of real
// configurations (FDKaacEnc_InitPsyConfiguration() of the reference) and the normalization of
// FDKaacEnc_psyMain().
//
// This file also provides the configurations and spectra for the other test modules of this
// group.

use crate::block_switch::{corpus_signals, synthetic_signals};
use crate::transform::Analysis;
use fdk_aac_rs_aacenc::band_nrg::*;
use fdk_aac_rs_aacenc::psy_configuration::PsyConfiguration;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_cref::aacenc_band_nrg as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::*;

// ---------------------------------------------------------------------------------------------
// Configurations and spectra (shared with the other test modules of this group)
// ---------------------------------------------------------------------------------------------

/// A psychoacoustic configuration of the reference (long blocks and, for granules above 512,
/// short blocks).
pub struct Conf {
    pub name: String,
    pub filterbank: i32,
    pub granule: usize,
    pub long: PsyConfiguration,
    pub short: Option<PsyConfiguration>,
}

/// `FDKaacEnc_InitPsyConfiguration()` of the reference.
pub fn psy_conf(
    bitrate: i32,
    samplerate: i32,
    bandwidth: i32,
    blocktype: i32,
    granule: i32,
    filterbank: i32,
) -> Option<PsyConfiguration> {
    assert_eq!(unsafe { c::cref_band_nrg_sizeof_psy_conf() } as usize, size_of::<PsyConfiguration>());
    let mut conf = PsyConfiguration::default();
    let err = unsafe {
        c::cref_band_nrg_psy_conf(
            bitrate,
            samplerate,
            bandwidth,
            blocktype,
            granule,
            0,
            1,
            filterbank,
            &mut conf as *mut _ as *mut std::ffi::c_void,
        )
    };
    (err == 0).then_some(conf)
}

/// Configurations of AAC-LC (1024), AAC-LD and AAC-ELD (also downscaled) at several
/// sampling rates, bit rates and bandwidths.
pub fn configs() -> Vec<Conf> {
    let mut v = Vec::new();
    for (fb, granule, sr, br, bw) in [
        (FB_LC, 1024, 48000, 64000, 16000),
        (FB_LC, 1024, 44100, 96000, 20000),
        (FB_LC, 1024, 32000, 48000, 13000),
        (FB_LC, 1024, 22050, 32000, 8000),
        (FB_LC, 1024, 16000, 24000, 6000),
        (FB_LC, 1024, 8000, 12000, 3500),
        (FB_LC, 1024, 96000, 128000, 22000),
        // AAC-LC with 960 lines is left out: no defined x86 reference result.
        (FB_LD, 512, 48000, 64000, 16000),
        (FB_LD, 480, 44100, 64000, 18000),
        (FB_LD, 512, 24000, 32000, 9000),
        (FB_ELD, 512, 48000, 64000, 20000),
        (FB_ELD, 480, 32000, 48000, 14000),
        (FB_ELD, 256, 24000, 48000, 11000),
        (FB_ELD, 240, 24000, 32000, 10000),
        (FB_ELD, 128, 12000, 24000, 5500),
        (FB_ELD, 120, 12000, 24000, 5000),
    ] {
        let Some(long) = psy_conf(br, sr, bw, LONG_WINDOW, granule, fb) else {
            panic!("no configuration for {fb} {granule} {sr}");
        };
        let short = if granule > 512 {
            Some(psy_conf(br, sr, bw, SHORT_WINDOW, granule, fb).expect("short configuration"))
        } else {
            None
        };
        v.push(Conf {
            name: format!("fb {fb} granule {granule} {sr} Hz {br} bit/s bandwidth {bw}"),
            filterbank: fb,
            granule: granule as usize,
            long,
            short,
        });
    }
    v
}

/// One frame of a channel pair after the filter bank (as in `FDKaacEnc_psyMain()`, with the
/// spectrum above the low-pass line cleared).
pub struct Frame {
    pub spectrum: [Vec<i32>; 2],
    pub mdct_scale: [i32; 2],
    pub short: bool,
    /// `noOfGroups` and `groupLen` of the (synchronized) block switching
    pub groups: (i32, [i32; MAX_NO_OF_GROUPS]),
}

impl Frame {
    /// The configuration of the frame's block type.
    pub fn conf<'a>(&self, conf: &'a Conf) -> &'a PsyConfiguration {
        if self.short {
            conf.short.as_ref().unwrap()
        } else {
            &conf.long
        }
    }

    /// (number of windows, window length)
    pub fn windows(&self, conf: &Conf) -> (usize, usize) {
        if self.short {
            (8, conf.granule / 8)
        } else {
            (1, conf.granule)
        }
    }
}

/// Up to `per_signal` frames of each of a few real and synthetic stereo signals through the
/// ported filter banks of `conf`.
pub fn frames(conf: &Conf, per_signal: usize) -> Vec<Frame> {
    let n = conf.granule;
    let len = n * per_signal;
    let mut signals = corpus_signals(len);
    signals.truncate(3);
    signals.extend(
        synthetic_signals(len, 21)
            .into_iter()
            .filter(|s| ["bursts", "clicks", "noise_quiet", "steps"].contains(&s.0.as_str())),
    );
    let mut out = Vec::new();
    for (name, s) in &signals {
        let mut a = Analysis::new(conf.filterbank, n, 2, 0);
        for f in 0..per_signal {
            let pcm = [&s[0][f * n..(f + 1) * n], &s[1][f * n..(f + 1) * n]];
            let res = a.frame(&pcm, &format!("{} {name} frame {f}", conf.name));
            let short = a.bsc[0].last_window_sequence == SHORT_WINDOW;
            let mut frame = Frame {
                spectrum: [res[0].0.clone(), res[1].0.clone()],
                mdct_scale: [res[0].1, res[1].1],
                short,
                groups: (a.bsc[0].no_of_groups, a.bsc[0].group_len),
            };
            let pc = frame.conf(conf);
            let (nw, wl) = frame.windows(conf);
            for ch in 0..2 {
                for w in 0..nw {
                    for line in pc.lowpass_line as usize..wl {
                        frame.spectrum[ch][w * wl + line] = 0;
                    }
                }
            }
            out.push(frame);
        }
    }
    out
}

/// The rescaling of `FDKaacEnc_psyMain()` ("Do some rescaling to get maximum possible accuracy
/// for energies") with the ported functions: returns `sfbMaxScaleSpec` per channel (windows at
/// `w * MAX_SFB_SHORT` for short blocks) and shifts the spectra and the exponents.
pub fn normalize(frame: &mut Frame, conf: &Conf) -> [Vec<i32>; 2] {
    let pc = *frame.conf(conf);
    let (nw, wl) = frame.windows(conf);
    let max_sfb = if frame.short { MAX_SFB_SHORT } else { MAX_GROUPED_SFB };
    let active = pc.sfb_active as usize;
    let mut scale = [vec![0i32; 8 * MAX_SFB_SHORT], vec![0i32; 8 * MAX_SFB_SHORT]];
    let mut min_spec_shift = 31;
    for ch in 0..2 {
        for w in 0..nw {
            fdk_aac_enc_calc_sfb_max_scale_spec(
                &frame.spectrum[ch][w * wl..],
                &pc.sfb_offset,
                &mut scale[ch][w * max_sfb..],
                pc.sfb_active,
            );
            for sfb in 0..active {
                min_spec_shift = min_spec_shift.min(scale[ch][w * max_sfb + sfb]);
            }
        }
    }
    let mut max_nrg = 0;
    let mut nrg = [0i32; MAX_GROUPED_SFB];
    let mut nrg_ld = [0i32; MAX_GROUPED_SFB];
    for ch in 0..2 {
        for w in 0..nw {
            let cur = fdk_aac_enc_check_band_energy_optim(
                &frame.spectrum[ch][w * wl..],
                &scale[ch][w * max_sfb..],
                &pc.sfb_offset,
                pc.sfb_active,
                &mut nrg,
                &mut nrg_ld,
                min_spec_shift - 4,
            );
            max_nrg = max_nrg.max(cur);
        }
    }
    let mut nrg_shift = 31;
    if max_nrg != 0 {
        nrg_shift = (count_leading_bits(max_nrg) >> 1) + (min_spec_shift - 4);
    }
    if frame.short {
        nrg_shift -= 1;
    }
    let mut final_shift = min_spec_shift.min(nrg_shift);
    if final_shift > frame.mdct_scale[0] + 3 {
        final_shift = frame.mdct_scale[0] + 3;
    }
    if max_nrg != 0 && final_shift > 0 {
        for ch in 0..2 {
            for w in 0..nw {
                for line in 0..pc.lowpass_line as usize {
                    frame.spectrum[ch][w * wl + line] <<= final_shift;
                }
                for sfb in 0..active {
                    scale[ch][w * max_sfb + sfb] -= final_shift;
                }
            }
            frame.mdct_scale[ch] -= final_shift;
        }
    }
    scale
}

// ---------------------------------------------------------------------------------------------
// Random data
// ---------------------------------------------------------------------------------------------

/// A random band layout: `n` bands of 1..=max_width lines (offsets from 0).
pub fn random_offsets(r: &mut Rng, n: usize, max_width: i32) -> Vec<i32> {
    let mut o = vec![0i32];
    for _ in 0..n {
        let w = if r.next_u32() % 8 == 0 { 0 } else { r.range(1, max_width) };
        o.push(o.last().unwrap() + w);
    }
    o
}

/// A random spectrum with a random headroom per band.
pub fn random_spectrum(r: &mut Rng, offsets: &[i32]) -> Vec<i32> {
    let mut s = vec![0i32; *offsets.last().unwrap() as usize + 8];
    for b in 0..offsets.len() - 1 {
        let kind = r.next_u32() % 8;
        let shift = r.next_u32() % 32;
        for x in &mut s[offsets[b] as usize..offsets[b + 1] as usize] {
            *x = match kind {
                0 => 0,
                1 => [MINVAL_DBL, MAXVAL_DBL, 0, 1, -1][(r.next_u32() % 5) as usize],
                2 => r.i32(),
                _ => (r.next_u32() as i32) >> shift,
            };
        }
    }
    s
}

/// Random `sfbMaxScaleSpec` values: mostly the true headroom, else values in 0..=30.
fn random_scale(r: &mut Rng, spec: &[i32], offsets: &[i32]) -> Vec<i32> {
    let n = offsets.len() - 1;
    let mut s = vec![0i32; n];
    fdk_aac_enc_calc_sfb_max_scale_spec(spec, offsets, &mut s, n as i32);
    for x in s.iter_mut() {
        if r.next_u32() % 4 == 0 {
            *x = r.range(0, 30);
        }
    }
    s
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

fn check_max_scale_spec(spec: &[i32], offsets: &[i32], n: i32, what: &str) {
    let mut a = vec![-7i32; 64];
    let mut b = a.clone();
    fdk_aac_enc_calc_sfb_max_scale_spec(spec, offsets, &mut a, n);
    unsafe { c::cref_band_nrg_CalcSfbMaxScaleSpec(spec.as_ptr(), offsets.as_ptr(), b.as_mut_ptr(), n) };
    assert_eq!(a, b, "{what}: CalcSfbMaxScaleSpec");
}

fn check_check_band_energy(spec: &[i32], scale: &[i32], offsets: &[i32], n: i32, min_spec_shift: i32, what: &str) {
    let (mut e_a, mut l_a) = (vec![-7i32; 64], vec![-9i32; 64]);
    let (mut e_b, mut l_b) = (e_a.clone(), l_a.clone());
    let ra = fdk_aac_enc_check_band_energy_optim(spec, scale, offsets, n, &mut e_a, &mut l_a, min_spec_shift);
    let rb = unsafe {
        c::cref_band_nrg_CheckBandEnergyOptim(
            spec.as_ptr(),
            scale.as_ptr(),
            offsets.as_ptr(),
            n,
            e_b.as_mut_ptr(),
            l_b.as_mut_ptr(),
            min_spec_shift,
        )
    };
    assert_eq!((ra, &e_a, &l_a), (rb, &e_b, &l_b), "{what}: CheckBandEnergyOptim");
}

fn check_long(spec: &[i32], scale: &[i32], offsets: &[i32], n: i32, what: &str) {
    let (mut e_a, mut l_a) = (vec![-7i32; 64], vec![-9i32; 64]);
    let (mut e_b, mut l_b) = (e_a.clone(), l_a.clone());
    let mut s_b = scale.to_vec();
    let ra = fdk_aac_enc_calc_band_energy_optim_long(spec, scale, offsets, n, &mut e_a, &mut l_a);
    let rb = unsafe {
        c::cref_band_nrg_CalcBandEnergyOptimLong(
            spec.as_ptr(),
            s_b.as_mut_ptr(),
            offsets.as_ptr(),
            n,
            e_b.as_mut_ptr(),
            l_b.as_mut_ptr(),
        )
    };
    assert_eq!(s_b, scale, "{what}: sfbMaxScaleSpec changed");
    assert_eq!((ra, &e_a, &l_a), (rb, &e_b, &l_b), "{what}: CalcBandEnergyOptimLong");
}

fn check_short(spec: &[i32], scale: &[i32], offsets: &[i32], n: i32, what: &str) {
    let mut e_a = vec![-7i32; 64];
    let mut e_b = e_a.clone();
    let mut s_b = scale.to_vec();
    fdk_aac_enc_calc_band_energy_optim_short(spec, scale, offsets, n, &mut e_a);
    unsafe {
        c::cref_band_nrg_CalcBandEnergyOptimShort(spec.as_ptr(), s_b.as_mut_ptr(), offsets.as_ptr(), n, e_b.as_mut_ptr())
    };
    assert_eq!(e_a, e_b, "{what}: CalcBandEnergyOptimShort");
}

#[allow(clippy::too_many_arguments)]
fn check_ms(l: &[i32], r: &[i32], sl: &[i32], sr: &[i32], offsets: &[i32], n: i32, calc_ld: i32, what: &str) {
    let (mut m_a, mut s_a, mut ml_a, mut sl_a) = (vec![-7i32; 64], vec![-8i32; 64], vec![-9i32; 64], vec![-10i32; 64]);
    let (mut m_b, mut s_b, mut ml_b, mut sl_b) = (m_a.clone(), s_a.clone(), ml_a.clone(), sl_a.clone());
    let (mut sl_c, mut sr_c) = (sl.to_vec(), sr.to_vec());
    fdk_aac_enc_calc_band_nrg_ms_opt(l, r, sl, sr, offsets, n, &mut m_a, &mut s_a, calc_ld, &mut ml_a, &mut sl_a);
    unsafe {
        c::cref_band_nrg_CalcBandNrgMSOpt(
            l.as_ptr(),
            r.as_ptr(),
            sl_c.as_mut_ptr(),
            sr_c.as_mut_ptr(),
            offsets.as_ptr(),
            n,
            m_b.as_mut_ptr(),
            s_b.as_mut_ptr(),
            calc_ld,
            ml_b.as_mut_ptr(),
            sl_b.as_mut_ptr(),
        )
    };
    assert_eq!((&m_a, &s_a, &ml_a, &sl_a), (&m_b, &s_b, &ml_b, &sl_b), "{what}: CalcBandNrgMSOpt");
}

#[test]
fn band_nrg_random() {
    let mut r = Rng::new(31);
    for it in 0..3000 {
        let nb = r.range(0, 51) as usize;
        let max_width = if r.next_u32() % 2 == 0 { 32 } else { 96 };
        let offsets = random_offsets(&mut r, nb, max_width);
        let spec = random_spectrum(&mut r, &offsets);
        let spec2 = random_spectrum(&mut r, &offsets);
        let n = nb as i32;
        let what = format!("iteration {it}");
        check_max_scale_spec(&spec, &offsets, n, &what);
        let scale = random_scale(&mut r, &spec, &offsets);
        let scale2 = random_scale(&mut r, &spec2, &offsets);
        if nb > 0 {
            let min_spec_shift = scale.iter().copied().min().unwrap_or(30) - 4 + r.range(-2, 0);
            check_check_band_energy(&spec, &scale, &offsets, n, min_spec_shift, &what);
        }
        check_long(&spec, &scale, &offsets, n, &what);
        check_short(&spec, &scale, &offsets, n, &what);
        check_ms(&spec, &spec2, &scale, &scale2, &offsets, n, r.range(0, 1), &what);
    }
}

#[test]
fn band_nrg_realistic() {
    let mut count = 0;
    for conf in configs() {
        for mut frame in frames(&conf, 12) {
            let what = format!("{} short {}", conf.name, frame.short);
            let pc = *frame.conf(&conf);
            let (nw, wl) = frame.windows(&conf);
            let max_sfb = if frame.short { MAX_SFB_SHORT } else { MAX_GROUPED_SFB };
            let n = pc.sfb_active;
            // before the rescaling of psyMain
            for ch in 0..2 {
                for w in 0..nw {
                    check_max_scale_spec(&frame.spectrum[ch][w * wl..], &pc.sfb_offset, n, &what);
                }
            }
            let mut probe = Frame {
                spectrum: frame.spectrum.clone(),
                mdct_scale: frame.mdct_scale,
                short: frame.short,
                groups: frame.groups,
            };
            let s0 = normalize(&mut probe, &conf);
            for ch in 0..2 {
                for w in 0..nw {
                    let min = s0[ch][w * max_sfb..w * max_sfb + n as usize].iter().copied().min().unwrap_or(30);
                    check_check_band_energy(&frame.spectrum[ch][w * wl..], &s0[ch][w * max_sfb..], &pc.sfb_offset, n, min - 4, &what);
                }
            }
            // after it
            let scale = normalize(&mut frame, &conf);
            for w in 0..nw {
                let (l, r) = (&frame.spectrum[0][w * wl..], &frame.spectrum[1][w * wl..]);
                let (sl, sr) = (&scale[0][w * max_sfb..], &scale[1][w * max_sfb..]);
                for ch in 0..2 {
                    let s = &frame.spectrum[ch][w * wl..];
                    let sc = &scale[ch][w * max_sfb..];
                    check_max_scale_spec(s, &pc.sfb_offset, n, &what);
                    if frame.short {
                        check_short(s, sc, &pc.sfb_offset, n, &what);
                    } else {
                        check_long(s, sc, &pc.sfb_offset, n, &what);
                    }
                }
                check_ms(l, r, sl, sr, &pc.sfb_offset, n, (!frame.short) as i32, &what);
                count += 1;
            }
        }
    }
    assert!(count > 1000);
}
