// Tests of the optimized envelope adjustor kernels of fdk_aac_rs_sbrdec::env_calc against
// the straight port kept next to them (`*_straight`), on random and extreme inputs. The complete
// envelope adjustor is compared with C++ in env_calc.rs and e2e.rs.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::env_calc::*;
use fdk_aac_rs_sbrdec::sbr_ram::pod_bytes;

const ROWS: usize = 40;

/// QMF slots with a random level per subband (some silent), sometimes extreme values.
fn slots(rng: &mut Rng) -> Vec<Vec<i32>> {
    let levels: Vec<u32> = (0..64)
        .map(|_| match rng.next_u32() % 8 {
            0 => 32, // silent
            1 => 0,  // full scale
            _ => rng.next_u32() % 31,
        })
        .collect();
    let edge = rng.next_u32() % 4 == 0;
    (0..ROWS)
        .map(|_| {
            (0..64)
                .map(|k| {
                    if edge && rng.next_u32() % 16 == 0 {
                        [i32::MIN, i32::MAX, -1, 1][(rng.next_u32() % 4) as usize]
                    } else if levels[k] == 32 {
                        0
                    } else {
                        (rng.next_u32() as i32) >> levels[k]
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn calc_nrg_per_subband_fast_matches_straight() {
    let mut rng = Rng::new(0x030a);
    for iter in 0..20000 {
        let lp = rng.next_u32() % 2 == 0;
        let mut re = slots(&mut rng);
        let mut im = slots(&mut rng);
        let lo = if rng.next_u32() % 8 == 0 { rng.range(-3, 64) } else { rng.range(0, 40) };
        let hi = if rng.next_u32() % 8 == 0 { rng.range(lo, 70) } else { rng.range(lo, 64) };
        let start = if rng.next_u32() % 8 == 0 { rng.range(-2, 38) } else { rng.range(0, 30) };
        let next = rng.range(start + 1, ROWS as i32);
        let frame_exp = rng.range(-30, 30) as i8;
        let len = if rng.next_u32() % 4 == 0 { rng.range(0, 64) as usize } else { 64 };
        let (mut na, mut ea) = (vec![0x5555i32; len], vec![0x55i8; len]);
        let (mut nb, mut eb) = (na.clone(), ea.clone());
        let r: Vec<&mut [i32]> = re.iter_mut().map(|v| &mut v[..]).collect();
        let i: Vec<&mut [i32]> = im.iter_mut().map(|v| &mut v[..]).collect();
        let imag = if lp { None } else { Some(&i[..]) };
        calc_nrg_per_subband(&r, imag, lo, hi, start, next, frame_exp, &mut na, &mut ea);
        calc_nrg_per_subband_straight(&r, imag, lo, hi, start, next, frame_exp, &mut nb, &mut eb);
        let what =
            format!("iter {iter} lp {lp} lo {lo} hi {hi} start {start} next {next} len {len}");
        assert_eq!(na, nb, "{what}");
        assert_eq!(ea, eb, "{what}");
    }
}

#[test]
fn max_subband_sample_fast_matches_straight() {
    let mut rng = Rng::new(0x030b);
    for iter in 0..40000 {
        let lp = rng.next_u32() % 2 == 0;
        let mut re = slots(&mut rng);
        let mut im = slots(&mut rng);
        // short rows and missing slots (the C code would read outside)
        if rng.next_u32() % 8 == 0 {
            let l = rng.range(0, ROWS as i32 - 1) as usize;
            re[l].truncate(rng.range(0, 64) as usize);
        }
        let n_rows =
            if rng.next_u32() % 8 == 0 { rng.range(0, ROWS as i32) as usize } else { ROWS };
        let lo = rng.range(-3, 66);
        let hi = if rng.next_u32() % 8 == 0 { rng.range(-3, 70) } else { rng.range(lo, 70) };
        let start = rng.range(-2, ROWS as i32);
        let next = rng.range(-2, ROWS as i32 + 4);
        let r: Vec<&mut [i32]> = re.iter_mut().take(n_rows).map(|v| &mut v[..]).collect();
        let i: Vec<&mut [i32]> = im.iter_mut().take(n_rows).map(|v| &mut v[..]).collect();
        let imag = if lp { None } else { Some(&i[..]) };
        let a = max_subband_sample(&r, imag, lo, hi, start, next);
        let b = max_subband_sample_straight(&r, imag, lo, hi, start, next);
        assert_eq!(a, b, "iter {iter} lp {lp} lo {lo} hi {hi} start {start} next {next}");
    }
}

/// Random gains, noise levels and sine levels (many sines are 0), and a random smoothing state.
fn adj_state(rng: &mut Rng) -> (EnvCalcNrgs, SbrCalculateEnvelope) {
    let mut nrgs = EnvCalcNrgs::default();
    let mut env = SbrCalculateEnvelope::default();
    let sh = rng.next_u32() % 8;
    for k in 0..nrgs.nrg_gain.len() {
        nrgs.nrg_gain[k] = rng.i32() >> sh;
        nrgs.noise_level[k] = rng.i32() >> sh;
        nrgs.nrg_sine[k] = if rng.next_u32() % 4 == 0 { rng.i32() >> sh } else { 0 };
        env.filt_buffer[k] = rng.i32();
        env.filt_buffer_noise[k] = rng.i32() >> (rng.next_u32() % 32);
    }
    env.phase_index = if rng.next_u32() % 8 == 0 { rng.range(0, 2000) } else { rng.range(0, 511) };
    env.harm_index =
        if rng.next_u32() % 8 == 0 { rng.next_u32() as u8 } else { rng.range(0, 3) as u8 };
    (nrgs, env)
}

#[test]
fn adjust_time_slot_hq_fast_matches_straight() {
    let mut rng = Rng::new(0x030c);
    for iter in 0..40000 {
        let (nrgs, mut env_a) = adj_state(&mut rng);
        let mut env_b = env_a;
        let lo = rng.range(0, 63);
        let no_subbands = rng.range(-2, (64 - lo).min(56));
        let scale_change = rng.range(-3, 31);
        let smooth_ratio = match rng.next_u32() % 4 {
            0 => 0,
            1 => rng.i16(),
            _ => (rng.next_u32() >> 17) as i16,
        };
        let no_noise_flag = rng.range(0, 1);
        let shift = rng.range(-40, 40);
        let sh = rng.next_u32() % 8;
        let mut re_a: Vec<i32> = (0..64 - lo).map(|_| rng.i32() >> sh).collect();
        let mut im_a: Vec<i32> = (0..64 - lo).map(|_| rng.i32() >> sh).collect();
        let (mut re_b, mut im_b) = (re_a.clone(), im_a.clone());
        let gain_and_noise = iter % 2 == 1;
        let args = (lo, no_subbands, scale_change, smooth_ratio, no_noise_flag, shift);
        if gain_and_noise {
            adjust_time_slot_hq_gain_and_noise(
                &mut re_a,
                &mut im_a,
                &mut env_a,
                &nrgs,
                lo,
                no_subbands,
                scale_change,
                smooth_ratio,
                no_noise_flag,
                shift,
            );
            adjust_time_slot_hq_gain_and_noise_straight(
                &mut re_b,
                &mut im_b,
                &mut env_b,
                &nrgs,
                lo,
                no_subbands,
                scale_change,
                smooth_ratio,
                no_noise_flag,
                shift,
            );
        } else {
            adjust_time_slot_hq(
                &mut re_a,
                &mut im_a,
                &mut env_a,
                &nrgs,
                lo,
                no_subbands,
                scale_change,
                smooth_ratio,
                no_noise_flag,
                shift,
            );
            adjust_time_slot_hq_straight(
                &mut re_b,
                &mut im_b,
                &mut env_b,
                &nrgs,
                lo,
                no_subbands,
                scale_change,
                smooth_ratio,
                no_noise_flag,
                shift,
            );
        }
        let what = format!("iter {iter} gain_and_noise {gain_and_noise} {args:?}");
        assert_eq!(re_a, re_b, "{what}");
        assert_eq!(im_a, im_b, "{what}");
        assert_eq!(pod_bytes(&env_a), pod_bytes(&env_b), "{what}");
    }
}

#[test]
fn adjust_time_slot_lc_fast_matches_straight() {
    let mut rng = Rng::new(0x030d);
    for iter in 0..60000 {
        let (mut nrgs, env) = adj_state(&mut rng);
        // runs of many sines (the 16 tone limit)
        if rng.next_u32() % 4 == 0 {
            for k in 0..nrgs.nrg_sine.len() {
                if rng.next_u32() % 4 != 0 {
                    nrgs.nrg_sine[k] = rng.i32() >> (rng.next_u32() % 8);
                }
            }
        }
        let lo = if rng.next_u32() % 16 == 0 { rng.range(0, 63) } else { rng.range(0, 40) };
        let no_subbands =
            if rng.next_u32() % 8 == 0 { rng.range(-2, 60) } else { rng.range(3, 56) };
        let len = if rng.next_u32() % 8 == 0 { rng.range(0, 64) as usize } else { 64 };
        // (slot[lowSubband - 1] must exist: the straight port indexes it)
        let len = if (len as i32) < lo + 1 { 64 } else { len };
        let scale_change = rng.range(-3, 31);
        let no_noise_flag = rng.range(0, 1);
        let sh = rng.next_u32() % 8;
        let mut slot_a: Vec<i32> = (0..len).map(|_| rng.i32() >> sh).collect();
        let mut slot_b = slot_a.clone();
        let (mut harm_a, mut phase_a) = (env.harm_index, env.phase_index);
        let (mut harm_b, mut phase_b) = (harm_a, phase_a);
        adjust_time_slot_lc(
            &mut slot_a,
            lo,
            &nrgs,
            &mut harm_a,
            no_subbands,
            scale_change,
            no_noise_flag,
            &mut phase_a,
        );
        adjust_time_slot_lc_straight(
            &mut slot_b,
            lo,
            &nrgs,
            &mut harm_b,
            no_subbands,
            scale_change,
            no_noise_flag,
            &mut phase_b,
        );
        let what = format!(
            "iter {iter} lo {lo} ns {no_subbands} len {len} sc {scale_change} nn {no_noise_flag} \
             harm {} phase {}",
            env.harm_index, env.phase_index
        );
        assert_eq!(slot_a, slot_b, "{what}");
        assert_eq!((harm_a, phase_a), (harm_b, phase_b), "{what}");
    }
}

#[test]
fn adjust_time_slot_eld_grid_fast_matches_straight() {
    let mut rng = Rng::new(0x030e);
    for iter in 0..60000 {
        let (mut nrgs, env) = adj_state(&mut rng);
        if rng.next_u32() % 4 == 0 {
            for k in 0..nrgs.nrg_sine.len() {
                if rng.next_u32() % 4 != 0 {
                    nrgs.nrg_sine[k] = rng.i32() >> (rng.next_u32() % 8);
                }
            }
        } else if rng.next_u32() % 3 == 0 {
            // 14 to 18 non-zero sines (around the limit of 16 for the neighbour sines)
            nrgs.nrg_sine = [0; 56];
            for _ in 0..rng.range(14, 18) {
                let k = rng.range(0, 55) as usize;
                nrgs.nrg_sine[k] = rng.i32() | 1;
            }
        }
        let lo = rng.range(0, 63);
        let no_subbands = if rng.next_u32() % 8 == 0 {
            rng.range(-2, 3.min(64 - lo))
        } else {
            rng.range(0, (64 - lo).min(56))
        };
        let scale_change = rng.range(-3, 31);
        let no_noise_flag = rng.range(0, 1);
        let scale_diff_low = rng.range(-31, 40);
        let sh = rng.next_u32() % 8;
        let mut slot_a: Vec<i32> = (0..64).map(|_| rng.i32() >> sh).collect();
        let mut slot_b = slot_a.clone();
        let (mut harm_a, mut phase_a) = (env.harm_index, env.phase_index);
        let (mut harm_b, mut phase_b) = (harm_a, phase_a);
        adjust_time_slot_eld_grid(
            &mut slot_a,
            lo,
            &nrgs,
            &mut harm_a,
            no_subbands,
            scale_change,
            no_noise_flag,
            &mut phase_a,
            scale_diff_low,
        );
        adjust_time_slot_eld_grid_straight(
            &mut slot_b,
            lo,
            &nrgs,
            &mut harm_b,
            no_subbands,
            scale_change,
            no_noise_flag,
            &mut phase_b,
            scale_diff_low,
        );
        let what = format!(
            "iter {iter} lo {lo} ns {no_subbands} sc {scale_change} nn {no_noise_flag} harm {} \
             phase {}",
            env.harm_index, env.phase_index
        );
        assert_eq!(slot_a, slot_b, "{what}");
        assert_eq!((harm_a, phase_a), (harm_b, phase_b), "{what}");
    }
}
