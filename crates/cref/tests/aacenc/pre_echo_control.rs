// Differential tests of fdk_aac_rs_aacenc::pre_echo_control against the C++ reference.
//
// The pre echo control is stateful (the thresholds and the MDCT exponent of the last block).
// It is compared on random states and parameters (including exponent differences of 16 or more,
// which shift by 32 or more, and wrapping products), and over sequences of frames with the
// parameters of real configurations and thresholds of MDCT spectra of real and synthetic PCM,
// with the resets of FDKaacEnc_psyMain() around start and stop windows.

use crate::band_nrg::{configs, frames, normalize};
use fdk_aac_rs_aacenc::band_nrg::{
    fdk_aac_enc_calc_band_energy_optim_long, fdk_aac_enc_calc_band_energy_optim_short,
};
use fdk_aac_rs_aacenc::pre_echo_control::*;
use fdk_aac_rs_aacenc::psy_configuration::C_RATIO;
use fdk_aac_rs_cref::aacenc_pre_echo_control as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::*;

/// The state of the pre echo control (`sfbThresholdnm1`, `calcPreEcho`, `mdctScalenm1`).
#[derive(Clone, PartialEq, Debug)]
struct State {
    nm1: Vec<i32>,
    calc: i32,
    scale_nm1: i32,
}

fn init(s: &mut State, t: &mut State, num_pb: i32, pcm_thr: &[i32], what: &str) {
    fdk_aac_enc_init_pre_echo_control(&mut s.nm1, &mut s.calc, num_pb, pcm_thr, &mut s.scale_nm1);
    let mut thr = pcm_thr.to_vec();
    unsafe {
        c::cref_pre_echo_control_InitPreEchoControl(
            t.nm1.as_mut_ptr(),
            &mut t.calc,
            num_pb,
            thr.as_mut_ptr(),
            &mut t.scale_nm1,
        )
    };
    assert_eq!(s, t, "{what}: InitPreEchoControl");
}

#[allow(clippy::too_many_arguments)]
fn control(
    s: &mut State,
    t: &mut State,
    num_pb: i32,
    max_inc: i32,
    min_rem: i16,
    thr: &[i32],
    mdct_scale: i32,
    what: &str,
) -> Vec<i32> {
    let mut a = thr.to_vec();
    let mut b = thr.to_vec();
    fdk_aac_enc_pre_echo_control(&mut s.nm1, s.calc, num_pb, max_inc, min_rem, &mut a, mdct_scale, &mut s.scale_nm1);
    unsafe {
        c::cref_pre_echo_control_PreEchoControl(
            t.nm1.as_mut_ptr(),
            t.calc,
            num_pb,
            max_inc,
            min_rem,
            b.as_mut_ptr(),
            mdct_scale,
            &mut t.scale_nm1,
        )
    };
    assert_eq!((&a, &*s), (&b, &*t), "{what}: PreEchoControl");
    a
}

fn random_thr(r: &mut Rng) -> Vec<i32> {
    let shift = r.next_u32() % 32;
    (0..64)
        .map(|_| match r.next_u32() % 6 {
            0 => r.i32(),
            1 => [0, MAXVAL_DBL, MINVAL_DBL, 1][(r.next_u32() % 4) as usize],
            _ => ((r.next_u32() >> 1) >> shift) as i32,
        })
        .collect()
}

#[test]
fn pre_echo_control_random() {
    let mut r = Rng::new(51);
    for it in 0..20000 {
        let num_pb = r.range(0, 51);
        let pcm_thr = random_thr(&mut r);
        let mut s = State { nm1: random_thr(&mut r), calc: 7, scale_nm1: 9 };
        let mut t = s.clone();
        let what = format!("iteration {it}");
        init(&mut s, &mut t, num_pb, &pcm_thr, &what);
        // a random state
        s.nm1 = random_thr(&mut r);
        s.calc = r.range(0, 1);
        s.scale_nm1 = if r.next_u32() % 8 == 0 { r.range(-40, 40) } else { r.range(-2, 20) };
        let t2 = s.clone();
        t = t2;
        let max_inc = match r.next_u32() % 4 {
            0 => r.i32(),
            _ => r.range(0, 8),
        };
        let min_rem = r.i16();
        let mdct_scale = if r.next_u32() % 8 == 0 { r.range(-40, 40) } else { r.range(-2, 20) };
        let thr = random_thr(&mut r);
        control(&mut s, &mut t, num_pb, max_inc, min_rem, &thr, mdct_scale, &what);
    }
}

#[test]
fn pre_echo_control_realistic() {
    let mut count = 0;
    for conf in configs() {
        let fr = frames(&conf, 12);
        let pc_long = conf.long;
        // two channels; the state persists over the frames
        let mut st: Vec<(State, State)> = (0..2)
            .map(|_| {
                let s = State { nm1: vec![0; 64], calc: 0, scale_nm1: 0 };
                (s.clone(), s)
            })
            .collect();
        for (s, t) in st.iter_mut() {
            init(s, t, pc_long.sfb_cnt, &pc_long.sfb_pcm_quant_threshold, &conf.name);
        }
        let mut prev_short = false;
        for mut frame in fr {
            let pc = *frame.conf(&conf);
            let (nw, wl) = frame.windows(&conf);
            let scale = normalize(&mut frame, &conf);
            let max_sfb = if frame.short { 15 } else { 60 };
            for ch in 0..2 {
                let (s, t) = &mut st[ch];
                for w in 0..nw {
                    let spec = &frame.spectrum[ch][w * wl..];
                    let sc = &scale[ch][w * max_sfb..];
                    let mut nrg = vec![0i32; 64];
                    let mut ld = vec![0i32; 64];
                    if frame.short {
                        fdk_aac_enc_calc_band_energy_optim_short(spec, sc, &pc.sfb_offset, pc.sfb_active, &mut nrg);
                    } else {
                        fdk_aac_enc_calc_band_energy_optim_long(spec, sc, &pc.sfb_offset, pc.sfb_active, &mut nrg, &mut ld);
                    }
                    let thr: Vec<i32> = nrg.iter().map(|&e| f_mult(e, C_RATIO) >> 4).collect();
                    // psyMain: a stop window (long after short) resets the state before, a start
                    // window (approximated: the last long block before short ones) after
                    if !frame.short && prev_short {
                        for i in 0..pc.sfb_active as usize {
                            s.nm1[i] = MAXVAL_DBL;
                            t.nm1[i] = MAXVAL_DBL;
                        }
                        s.scale_nm1 = 0;
                        t.scale_nm1 = 0;
                        s.calc = 0;
                        t.calc = 0;
                    }
                    let what = format!("{} short {} channel {ch} window {w}", conf.name, frame.short);
                    control(
                        s,
                        t,
                        pc.sfb_active,
                        pc.max_allowed_increase_factor,
                        pc.min_remaining_threshold_factor,
                        &thr,
                        frame.mdct_scale[ch],
                        &what,
                    );
                    s.calc = 1;
                    t.calc = 1;
                    count += 1;
                }
            }
            prev_short = frame.short;
        }
    }
    assert!(count > 1000);
}
