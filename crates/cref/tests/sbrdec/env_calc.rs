// Differential tests of fdk_aac_rs_sbrdec::env_calc against the C++ reference.
//
// Every function of env_calc.cpp (the static ones through a copy compiled in a namespace, see
// shim/sbrdec_env_calc.cpp) is run on random inputs: QMF slots of random magnitude (silence,
// full scale, edge values), random energies and exponents, random band tables and limiter
// configurations, and on random but consistent headers and frames for calculateSbrEnvelope
// (legacy SBR, low power and high quality, ELD grid, inter-TES, frame errors). Real data is
// covered by the end-to-end test (tests/sbrdec/e2e.rs).

use core::ffi::c_void;
use fdk_aac_rs_cref::sbrdec_env_calc::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::qmf::QmfScaleFactor;
use fdk_aac_rs_sbrdec::env_calc::*;
use fdk_aac_rs_sbrdec::env_extr::*;
use fdk_aac_rs_sbrdec::lpp_tran::*;
use fdk_aac_rs_sbrdec::pvc_dec::PvcDynamicData;
use fdk_aac_rs_sbrdec::sbr_ram::{pod_bytes, pod_bytes_mut, Pod};
use fdk_aac_rs_sbrdec::sbrdec_freq_sca::reset_freq_band_tables;

fn ptr<T: Pod>(v: &mut T) -> *mut c_void {
    pod_bytes_mut(v).as_mut_ptr() as *mut c_void
}

/// A random QMF value of a random magnitude class.
fn sample(rng: &mut Rng, class: u32) -> i32 {
    match class {
        0 => 0,
        1 => rng.i32(),
        2 => (rng.next_u32() as i32) >> rng.range(0, 31),
        3 => (rng.next_u32() as i32) >> 24,
        _ => (rng.next_u32() as i32) >> 4,
    }
}

/// `n` random slots of 64 values (flat).
fn random_slots(rng: &mut Rng, n: usize) -> Vec<i32> {
    let class = rng.next_u32() % 5;
    (0..n * 64).map(|_| sample(rng, class)).collect()
}

fn as_slots(flat: &mut [i32]) -> Vec<&mut [i32]> {
    flat.chunks_mut(64).collect()
}

fn random_nrgs(rng: &mut Rng) -> EnvCalcNrgs {
    let mut n = EnvCalcNrgs::ZERO;
    let mant = |rng: &mut Rng| -> i32 {
        match rng.next_u32() % 6 {
            0 => 0,
            1 => rng.i32(),
            _ => ((rng.next_u32() >> 1) | 0x4000_0000) as i32,
        }
    };
    let exp = |rng: &mut Rng| -> i8 {
        match rng.next_u32() % 4 {
            0 => rng.range(-128, 127) as i8,
            _ => rng.range(-40, 40) as i8,
        }
    };
    for k in 0..MAX_FREQ_COEFFS {
        n.nrg_ref[k] = mant(rng).max(0);
        n.nrg_est[k] = mant(rng).max(0);
        n.nrg_gain[k] = mant(rng).max(0);
        n.noise_level[k] = mant(rng).max(0);
        n.nrg_sine[k] = if rng.next_u32() % 3 == 0 { mant(rng).max(0) } else { 0 };
        n.nrg_ref_e[k] = exp(rng);
        n.nrg_est_e[k] = exp(rng);
        n.nrg_gain_e[k] = exp(rng);
        n.noise_level_e[k] = exp(rng);
        n.nrg_sine_e[k] = exp(rng);
    }
    n
}

#[test]
fn layout() {
    assert_eq!(unsafe { cref_sbrdec_env_calc_sizeof_nrgs() } as usize, core::mem::size_of::<EnvCalcNrgs>());
}

#[test]
fn nrg_per_subband_and_sfb() {
    let mut rng = Rng::new(1);
    for it in 0..3000 {
        let n_slots = rng.range(2, 44) as usize;
        let mut re = random_slots(&mut rng, n_slots);
        let mut im = random_slots(&mut rng, n_slots);
        let use_im = rng.next_u32() % 2 == 0;
        let lo = rng.range(0, 63);
        let hi = rng.range(lo, (lo + 56).min(64));
        let start = rng.range(0, n_slots as i32 - 1);
        let next = rng.range(start + 1, n_slots as i32);
        let e = rng.range(-128, 127);
        let mut c_n = [0i32; MAX_FREQ_COEFFS];
        let mut c_e = [0i8; MAX_FREQ_COEFFS];
        let mut p_n = [0i32; MAX_FREQ_COEFFS];
        let mut p_e = [0i8; MAX_FREQ_COEFFS];
        unsafe {
            cref_sbrdec_env_calc_nrg_per_subband(
                re.as_mut_ptr(),
                if use_im { im.as_mut_ptr() } else { core::ptr::null_mut() },
                n_slots as i32,
                lo,
                hi,
                start,
                next,
                e,
                c_n.as_mut_ptr(),
                c_e.as_mut_ptr(),
            )
        };
        {
            let r = as_slots(&mut re);
            let i = as_slots(&mut im);
            calc_nrg_per_subband(&r, if use_im { Some(&i) } else { None }, lo, hi, start, next, e as i8, &mut p_n, &mut p_e);
        }
        assert_eq!((p_n, p_e), (c_n, c_e), "calcNrgPerSubband {it}");

        // calcNrgPerSfb: a random increasing table within the slot width
        let n_sfb = rng.range(0, 20);
        let mut table = vec![0u8; 64];
        let mut b = rng.range(0, 40);
        for t in table.iter_mut().take(n_sfb as usize + 1) {
            *t = b as u8;
            b = (b + rng.range(1, 3)).min(63);
        }
        let mut c_n = [0i32; MAX_FREQ_COEFFS + 64];
        let mut c_e = [0i8; MAX_FREQ_COEFFS + 64];
        let mut p_n = [0i32; MAX_FREQ_COEFFS + 64];
        let mut p_e = [0i8; MAX_FREQ_COEFFS + 64];
        unsafe {
            cref_sbrdec_env_calc_nrg_per_sfb(
                re.as_mut_ptr(),
                if use_im { im.as_mut_ptr() } else { core::ptr::null_mut() },
                n_slots as i32,
                n_sfb,
                table.as_mut_ptr(),
                start,
                next,
                e,
                c_n.as_mut_ptr(),
                c_e.as_mut_ptr(),
            )
        };
        {
            let r = as_slots(&mut re);
            let i = as_slots(&mut im);
            calc_nrg_per_sfb(&r, if use_im { Some(&i) } else { None }, n_sfb, &table, start, next, e as i8, &mut p_n, &mut p_e);
        }
        assert_eq!((p_n.to_vec(), p_e.to_vec()), (c_n.to_vec(), c_e.to_vec()), "calcNrgPerSfb {it}");
    }
}

#[test]
fn subband_gain_avg_gain_aliasing() {
    let mut rng = Rng::new(2);
    let mut traps = 0;
    for it in 0..20000 {
        let mut c = random_nrgs(&mut rng);
        let mut p = c;
        let i = rng.range(0, 55);
        let nrg_ref = (rng.next_u32() >> rng.range(1, 31)) as i32;
        let nrg_ref_e = rng.range(-60, 60);
        let tmp_noise = (rng.next_u32() >> rng.range(1, 31)) as i32;
        let tmp_noise_e = rng.range(-60, 60);
        let (sp, sm, nn) = (rng.range(0, 1), rng.range(0, 1), rng.range(0, 1));
        let trapped = unsafe { cref_sbrdec_env_calc_subband_gain(nrg_ref, nrg_ref_e, ptr(&mut c), i, tmp_noise, tmp_noise_e, sp, sm, nn) };
        if trapped != 0 {
            // (no defined reference result: skipped; the number of these cases differs between
            // the variants: fMult() of x86 drops the lowest bit)
            traps += 1;
            continue;
        }
        calc_subband_gain(nrg_ref, nrg_ref_e as i8, &mut p, i as usize, tmp_noise, tmp_noise_e as i8, sp as u8, sm as u8, nn);
        assert_eq!(pod_bytes(&p), pod_bytes(&c), "calcSubbandGain {it}");

        let lo = rng.range(0, 55);
        let hi = rng.range(lo, 56);
        let mut out = [0i32; 4];
        unsafe { cref_sbrdec_env_calc_avg_gain(ptr(&mut c), lo, hi, out.as_mut_ptr()) };
        let (a, b, cc, d) = calc_avg_gain(&p, lo as usize, hi as usize);
        assert_eq!([a, b as i32, cc, d as i32], out, "calcAvgGain {it}");

        // aliasingReduction: degreeAlias starts at lowSubband (64 - lowSubband values)
        let low = rng.range(0, 40);
        let nsb = rng.range(0, (64 - low).min(56));
        let mut alias = [0i32; 64];
        for a in alias.iter_mut() {
            *a = if rng.next_u32() % 3 == 0 { 0 } else { (rng.next_u32() >> 1) as i32 };
        }
        let mut use_alias = [0u8; 64];
        for u in use_alias.iter_mut() {
            *u = (rng.next_u32() % 2) as u8;
        }
        let mut c2 = c;
        unsafe {
            cref_sbrdec_env_calc_aliasing_reduction(
                alias.as_mut_ptr().add(low as usize),
                ptr(&mut c2),
                use_alias.as_mut_ptr(),
                nsb,
            )
        };
        aliasing_reduction(&alias[low as usize..], &mut p, &use_alias, nsb);
        assert_eq!(pod_bytes(&p), pod_bytes(&c2), "aliasingReduction {it}");
    }
    eprintln!("calcSubbandGain: {traps} of 20000 cases without a defined reference result");
    assert!(traps < 2000, "{traps} cases without a defined reference result");
}

#[test]
fn equalize_filt_buffer() {
    let mut rng = Rng::new(3);
    for it in 0..20000 {
        let n = rng.range(0, 56) as usize;
        let mut fb: Vec<i32> = (0..56).map(|_| rng.i32()).collect();
        let mut fbe: Vec<i8> = (0..56).map(|_| rng.range(-60, 60) as i8).collect();
        let mut g: Vec<i32> = (0..56).map(|_| rng.i32()).collect();
        let mut ge: Vec<i8> = (0..56).map(|_| rng.range(-60, 60) as i8).collect();
        let (mut fb2, mut fbe2, mut g2, mut ge2) = (fb.clone(), fbe.clone(), g.clone(), ge.clone());
        unsafe { cref_sbrdec_env_calc_equalize(fb.as_mut_ptr(), fbe.as_mut_ptr(), g.as_mut_ptr(), ge.as_mut_ptr(), n as i32) };
        equalize_filt_buffer_exp(&mut fb2, &mut fbe2, &mut g2, &mut ge2, n as i32);
        assert_eq!((fb2, fbe2, g2, ge2), (fb, fbe, g, ge), "equalizeFiltBufferExp {it}");
    }
}

#[test]
fn adjust_time_slots() {
    let mut rng = Rng::new(4);
    for it in 0..20000 {
        let nrgs = random_nrgs(&mut rng);
        let low = rng.range(1, 62);
        let nsb = rng.range(1, (63 - low).min(56));
        let scale = rng.range(-31, 31);
        let nn = rng.range(0, 1);
        let mut harm = (rng.next_u32() % 4) as u8;
        let mut phase = rng.range(0, 511);
        let mut slot: Vec<i32> = random_slots(&mut rng, 1);
        let mut n = nrgs;
        let (mut harm2, mut phase2, mut slot2) = (harm, phase, slot.clone());
        if it % 2 == 0 {
            unsafe { cref_sbrdec_env_calc_adjust_lc(slot.as_mut_ptr(), low, ptr(&mut n), &mut harm, nsb, scale, nn, &mut phase) };
            adjust_time_slot_lc(&mut slot2, low, &nrgs, &mut harm2, nsb, scale, nn, &mut phase2);
        } else {
            let sdl = rng.range(-31, 31);
            unsafe { cref_sbrdec_env_calc_adjust_eld(slot.as_mut_ptr(), low, ptr(&mut n), &mut harm, nsb, scale, nn, &mut phase, sdl) };
            adjust_time_slot_eld_grid(&mut slot2, low, &nrgs, &mut harm2, nsb, scale, nn, &mut phase2, sdl);
        }
        assert_eq!((slot2, harm2, phase2), (slot, harm, phase), "adjustTimeSlot LC/ELD {it}");

        // high quality
        let mode = rng.range(0, 2);
        let mut env = SbrCalculateEnvelope::ZERO;
        for k in 0..MAX_FREQ_COEFFS {
            env.filt_buffer[k] = rng.i32() >> rng.range(0, 8);
            env.filt_buffer_noise[k] = rng.i32() >> rng.range(0, 8);
        }
        env.phase_index = rng.range(0, 511);
        env.harm_index = (rng.next_u32() % 4) as u8;
        let mut env2 = env;
        let mut re = random_slots(&mut rng, 1);
        let mut im = random_slots(&mut rng, 1);
        let (mut re2, mut im2) = (re.clone(), im.clone());
        let low = rng.range(0, 63);
        let nsb = rng.range(0, (64 - low).min(56));
        let smooth = if rng.next_u32() % 2 == 0 { 0 } else { rng.range(0, 0x7fff) };
        let fs = rng.range(-40, 40);
        let mut n = nrgs;
        unsafe {
            cref_sbrdec_env_calc_adjust_hq(mode, re.as_mut_ptr(), im.as_mut_ptr(), ptr(&mut env), ptr(&mut n), low, nsb, scale, smooth, nn, fs)
        };
        let (r2, i2) = (&mut re2[low as usize..], &mut im2[low as usize..]);
        match mode {
            0 => adjust_time_slot_hq(r2, i2, &mut env2, &nrgs, low, nsb, scale, smooth as i16, nn, fs),
            1 => adjust_time_slot_hq_gain_and_noise(r2, i2, &mut env2, &nrgs, low, nsb, scale, smooth as i16, nn, fs),
            _ => adjust_time_slot_hq_add_harmonics(r2, i2, &mut env2, &nrgs, low, nsb, scale),
        }
        assert_eq!((re2, im2), (re, im), "adjustTimeSlotHQ mode {mode} {it}");
        assert_eq!(pod_bytes(&env2), pod_bytes(&env), "adjustTimeSlotHQ state mode {mode} {it}");
    }
}

#[test]
fn map_sine_flags_and_limiter_bands() {
    let mut rng = Rng::new(5);
    for it in 0..20000 {
        let n_sfb = rng.range(0, 48);
        let mut table = [0u8; 64];
        // at most MAX_FREQ_COEFFS bands from the first one (tables with a defined reference
        // result, see the port)
        let first = rng.range(0, 30);
        let mut b = first;
        for t in table.iter_mut().take(n_sfb as usize + 1) {
            *t = b as u8;
            b = (b + rng.range(0, 2)).min(63).min(first + MAX_FREQ_COEFFS as i32 - 1);
        }
        let mut add = [rng.next_u32(), rng.next_u32()];
        let mut prev = [rng.next_u32(), rng.next_u32()];
        let mut act = [rng.next_u32(), rng.next_u32()];
        let tran = rng.range(-1, 8);
        let mut sm = [0i8; MAX_FREQ_COEFFS];
        let (mut prev2, mut act2, mut sm2) = (prev, act, sm);
        if it % 2 == 0 {
            unsafe { cref_sbrdec_env_calc_map_sine_flags(table.as_mut_ptr(), n_sfb, add.as_mut_ptr(), prev.as_mut_ptr(), act.as_mut_ptr(), tran, sm.as_mut_ptr()) };
            map_sine_flags(&table, n_sfb, &add, &mut prev2, &mut act2, tran, &mut sm2);
        } else {
            let mut spp = rng.range(-20, 40) as i8;
            let mut spp2 = spp;
            let pos = rng.range(0, 40);
            let tr = rng.range(0, 1);
            unsafe { cref_sbrdec_env_calc_map_sine_flags_pvc(table.as_mut_ptr(), n_sfb, prev.as_mut_ptr(), act.as_mut_ptr(), sm.as_mut_ptr(), pos, &mut spp, tr) };
            map_sine_flags_pvc(&table, n_sfb, &prev2, &act2, &mut sm2, pos, &mut spp2, tr);
            assert_eq!(spp2, spp, "mapSineFlagsPvc {it}");
        }
        assert_eq!((prev2, act2, sm2), (prev, act, sm), "mapSineFlags {it}");

        // ResetLimiterBands
        let nfb = rng.range(1, 28);
        let mut ftab = [0u8; 64];
        let mut b = rng.range(1, 30);
        for t in ftab.iter_mut().take(nfb as usize + 1) {
            *t = b as u8;
            b = (b + rng.range(1, 4)).min(64);
        }
        let n_patches = rng.range(0, MAX_NUM_PATCHES as i32);
        let mut patches = [PatchParam::ZERO; MAX_NUM_PATCHES];
        let mut g = ftab[0] as i32;
        for p in patches.iter_mut() {
            p.guard_start_band = g as u8;
            g = (g + rng.range(1, 12)).min(64);
        }
        let lb = rng.range(0, 3);
        let mode = rng.range(0, 1);
        let mut xo = [0i32; 6];
        let mut x = ftab[0] as i32;
        for v in xo.iter_mut() {
            *v = if rng.next_u32() % 5 == 0 { 0 } else { x };
            x = (x + rng.range(1, 12)).min(64);
        }
        let use_xo = rng.next_u32() % 2 == 0;
        let b41 = rng.range(0, 1);
        let mut lim = [0u8; MAX_NUM_LIMITERS + 1];
        let mut nlim = 0u8;
        let (mut lim2, mut nlim2) = (lim, nlim);
        let r = unsafe {
            cref_sbrdec_env_calc_reset_limiter_bands(
                lim.as_mut_ptr(),
                &mut nlim,
                ftab.as_mut_ptr(),
                nfb,
                patches.as_mut_ptr() as *mut c_void,
                n_patches,
                lb,
                mode,
                if use_xo { xo.as_mut_ptr() } else { core::ptr::null_mut() },
                b41,
            )
        };
        let r2 = reset_limiter_bands(&mut lim2, &mut nlim2, &ftab, nfb, &patches, n_patches, lb, mode as u8, if use_xo { Some(&xo) } else { None }, b41);
        assert_eq!((r2, lim2, nlim2), (r, lim, nlim), "ResetLimiterBands {it}");
    }
}

#[test]
fn max_rescale_inter_tes() {
    let mut rng = Rng::new(6);
    for it in 0..4000 {
        let n_slots = rng.range(2, 44) as usize;
        let mut re = random_slots(&mut rng, n_slots);
        let mut im = random_slots(&mut rng, n_slots);
        let use_im = rng.next_u32() % 2 == 0;
        let lo = rng.range(0, 64);
        let hi = rng.range(lo, 64);
        let start = rng.range(0, n_slots as i32);
        let next = rng.range(start, n_slots as i32);
        let c = unsafe {
            cref_sbrdec_env_calc_max_subband_sample(re.as_mut_ptr(), if use_im { im.as_mut_ptr() } else { core::ptr::null_mut() }, n_slots as i32, lo, hi, start, next)
        };
        let p = {
            let r = as_slots(&mut re);
            let i = as_slots(&mut im);
            max_subband_sample(&r, if use_im { Some(&i) } else { None }, lo, hi, start, next)
        };
        assert_eq!(p, c, "maxSubbandSample {it}");

        let shift = rng.range(-40, 40);
        let (mut re2, mut im2) = (re.clone(), im.clone());
        unsafe {
            cref_sbrdec_env_calc_rescale(re.as_mut_ptr(), if use_im { im.as_mut_ptr() } else { core::ptr::null_mut() }, n_slots as i32, lo, hi, start, next, shift)
        };
        {
            let mut r = as_slots(&mut re2);
            let mut i = as_slots(&mut im2);
            rescale_subband_samples(&mut r, if use_im { Some(&mut i) } else { None }, lo, hi, start, next, shift);
        }
        assert_eq!((re2, im2), (re.clone(), im.clone()), "rescaleSubbandSamples {it}");

        // apply_inter_tes
        let rate = [1, 2, 4][rng.range(0, 2) as usize];
        let start = rng.range(0, n_slots as i32 - 1);
        let stop = rng.range(start, n_slots as i32);
        let low = rng.range(1, 48);
        let nb = rng.range(1, 64 - low);
        let mut sc = [rng.range(-20, 31), rng.range(-20, 31), rng.range(-20, 31), rng.range(-20, 31)];
        let mut exp = [rng.range(-30, 30) as i8, rng.range(-30, 30) as i8];
        let gamma = rng.range(0, 3);
        let (mut re2, mut im2) = (re.clone(), im.clone());
        unsafe {
            cref_sbrdec_env_calc_inter_tes(re.as_mut_ptr(), im.as_mut_ptr(), n_slots as i32, sc.as_mut_ptr(), exp.as_mut_ptr(), rate, start, stop, low, nb, gamma)
        };
        {
            let mut r = as_slots(&mut re2);
            let mut i = as_slots(&mut im2);
            let sf = QmfScaleFactor { lb_scale: sc[0], ov_lb_scale: sc[1], hb_scale: sc[2], ov_hb_scale: sc[3] };
            apply_inter_tes(&mut r, &mut i, &sf, &exp, rate, start, stop, low, nb, gamma as u8);
        }
        assert_eq!((re2, im2), (re, im), "apply_inter_tes {it}");
    }
}

/// A random header with valid frequency band tables (None if the random header is rejected).
fn random_header(rng: &mut Rng, flags: u32) -> Option<SbrHeaderData> {
    let mut h = SbrHeaderData::ZERO;
    let rates = [16000, 22050, 24000, 32000, 44100, 48000];
    let sr_in = rates[rng.range(0, 5) as usize];
    let frame = if flags & SBRDEC_ELD_GRID != 0 { [512, 480][rng.range(0, 1) as usize] } else { [1024, 960][rng.range(0, 1) as usize] };
    if init_header_data(&mut h, sr_in, 2 * sr_in, 1, frame, flags, 1) != 0 {
        return None;
    }
    h.bs_data.start_freq = rng.range(0, 15) as u8;
    h.bs_data.stop_freq = rng.range(0, 15) as u8;
    h.bs_data.freq_scale = rng.range(0, 3) as u8;
    h.bs_data.alter_scale = rng.range(0, 1) as u8;
    h.bs_data.noise_bands = rng.range(0, 3) as u8;
    h.bs_data.limiter_bands = rng.range(0, 3) as u8;
    h.bs_data.limiter_gains = rng.range(0, 3) as u8;
    h.bs_data.interpol_freq = rng.range(0, 1) as u8;
    h.bs_data.smoothing_length = rng.range(0, 1) as u8;
    h.bs_info.xover_band = rng.range(0, 2) as u8;
    if reset_freq_band_tables(&mut h, flags) != 0 {
        return None;
    }
    Some(h)
}

/// A random frame matching the header (legacy SBR).
fn random_frame(rng: &mut Rng, h: &SbrHeaderData, eld: bool) -> SbrFrameData {
    let mut f = SbrFrameData::ZERO;
    let nts = h.number_time_slots as i32;
    let n_env = rng.range(1, 5);
    let fi = &mut f.frame_info;
    fi.n_envelopes = n_env as u8;
    let mut b = if eld { 0 } else { rng.range(0, 3) };
    for e in 0..=n_env as usize {
        fi.borders[e] = b as u8;
        if e < n_env as usize {
            let remaining = n_env - e as i32;
            let max_end = nts + if eld { 0 } else { 2 };
            let room = max_end - b - (remaining - 1);
            b += if e + 1 == n_env as usize { room.max(1) } else { rng.range(1, (room / remaining).max(1)) };
        }
    }
    for e in 0..n_env as usize {
        fi.freq_res[e] = rng.range(0, 1) as u8;
    }
    fi.tran_env = rng.range(-1, n_env) as i8;
    let n_noise = if n_env > 1 { rng.range(1, 2) } else { 1 };
    fi.n_noise_envelopes = n_noise as u8;
    fi.borders_noise[0] = fi.borders[0];
    fi.borders_noise[n_noise as usize] = fi.borders[n_env as usize];
    if n_noise == 2 {
        fi.borders_noise[1] = fi.borders[rng.range(1, n_env - 1) as usize];
    }
    for v in f.i_envelope.iter_mut() {
        // mantissa (10 bits) and exponent (6 bits) as requantizeEnvelopeData produces them
        let m = rng.range(0x200, 0x3ff) as i16;
        let e = rng.range(0, 63) as i16;
        *v = (m << 6) | e;
    }
    for v in f.sbr_noise_floor_level.iter_mut() {
        let m = rng.range(0x200, 0x3ff) as i16;
        let e = rng.range(0, 63) as i16;
        *v = (m << 6) | e;
    }
    f.add_harmonics = [rng.next_u32() & rng.next_u32(), rng.next_u32() & rng.next_u32()];
    f.i_tes_active = (rng.next_u32() % 3 == 0) as u8;
    for v in f.inter_temp_shape_mode.iter_mut() {
        *v = rng.range(0, 3) as u8;
    }
    f
}

#[test]
fn calculate_sbr_envelope_random() {
    let mut rng = Rng::new(7);
    let mut runs = 0;
    let mut traps = 0;
    for it in 0..3000 {
        let eld = it % 4 == 3;
        let flags = if eld { SBRDEC_ELD_GRID } else { 0 };
        let Some(mut h) = random_header(&mut rng, flags) else {
            continue;
        };
        let mut env = SbrCalculateEnvelope::ZERO;
        let mut h2 = h;
        let r = unsafe { cref_sbrdec_env_calc_create(ptr(&mut env), ptr(&mut h), 0, flags) };
        let mut env2 = SbrCalculateEnvelope::ZERO;
        let r2 = create_sbr_envelope_calc(&mut env2, &mut h2, 0, flags);
        assert_eq!(r2, r, "createSbrEnvelopeCalc {it}");
        assert_eq!(pod_bytes(&h2), pod_bytes(&h), "createSbrEnvelopeCalc header {it}");
        assert_eq!(pod_bytes(&env2), pod_bytes(&env), "createSbrEnvelopeCalc state {it}");
        if r != 0 {
            continue;
        }
        let use_lp = rng.next_u32() % 2 == 0;
        let n_slots = (h.number_time_slots as usize * h.time_step as usize + 12).min(64);
        let mut scale = [rng.range(0, 20), rng.range(0, 20), rng.range(0, 20), rng.range(0, 20)];
        let ites = !use_lp && !eld && rng.next_u32() % 3 == 0;
        // frames of the same configuration with state carried over
        for fr in 0..4 {
            let mut f = random_frame(&mut rng, &h, eld);
            if !ites {
                f.i_tes_active = 0;
            }
            let fe = (rng.next_u32() % 8 == 0) as i32;
            let mut re = random_slots(&mut rng, n_slots);
            let mut im = random_slots(&mut rng, n_slots);
            let mut alias: Vec<i32> = (0..64).map(|_| (rng.next_u32() >> 1) as i32).collect();
            let (mut re2, mut im2, mut alias2) = (re.clone(), im.clone(), alias.clone());
            let mut sc2 = scale;
            let mut hc = h;
            let trapped = unsafe {
                cref_sbrdec_env_calc_calculate(
                    scale.as_mut_ptr(),
                    ptr(&mut env),
                    ptr(&mut hc),
                    ptr(&mut f),
                    re.as_mut_ptr(),
                    im.as_mut_ptr(),
                    n_slots as i32,
                    use_lp as i32,
                    alias.as_mut_ptr(),
                    flags,
                    fe,
                )
            };
            if trapped != 0 {
                // (no defined reference result: skipped)
                traps += 1;
                break;
            }
            {
                let mut r = as_slots(&mut re2);
                let mut i = as_slots(&mut im2);
                let mut sf = QmfScaleFactor { lb_scale: sc2[0], ov_lb_scale: sc2[1], hb_scale: sc2[2], ov_hb_scale: sc2[3] };
                let pvc = PvcDynamicData::default();
                calculate_sbr_envelope(&mut sf, &mut env2, &h, &f, &pvc, &mut r, &mut i, use_lp, &mut alias2, flags, fe);
                sc2 = [sf.lb_scale, sf.ov_lb_scale, sf.hb_scale, sf.ov_hb_scale];
            }
            let what = format!("calculateSbrEnvelope {it}/{fr} lp {use_lp} eld {eld} ites {ites}");
            assert_eq!(sc2, scale, "{what}: scale");
            assert_eq!(pod_bytes(&env2), pod_bytes(&env), "{what}: state");
            assert!(re2 == re, "{what}: real");
            if !use_lp {
                assert!(im2 == im, "{what}: imag");
            }
            runs += 1;
        }
    }
    eprintln!("calculateSbrEnvelope: {runs} frames compared, {traps} without a defined reference result");
    assert!(runs > 1000, "too few valid configurations: {runs}");
}
