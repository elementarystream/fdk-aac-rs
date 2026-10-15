// Differential tests of fdk_aac_rs_aacdec::rvlcconceal against the C++ reference.
//
// The concealment functions run within CRvlc_Decode() in rvlc.rs (encoded and corrupted RVLC
// data) and aacdec_hcr_stream.rs (real streams); here they also run directly on random states:
// borders of the concealment at their initial values or at any band, the scale factors of both
// directions and of the previous frame, and codebooks of all kinds in both frames.

use super::aacdec_hcr::*;
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_aacdec::rvlc_info::*;
use fdk_aac_rs_aacdec::rvlcconceal::*;
use fdk_aac_rs_cref::aacdec_rvlc as c;
use fdk_aac_rs_cref::testutil::Rng;

fn random_cb(rng: &mut Rng) -> u8 {
    match rng.next_u32() % 6 {
        0 => ZERO_HCB,
        1 => NOISE_HCB,
        2 => INTENSITY_HCB,
        3 => INTENSITY_HCB2,
        _ => 1 + (rng.next_u32() % 11) as u8,
    }
}

/// A random concealment state on channel `ch`.
fn random_conceal_state(rng: &mut Rng, ch: usize) -> Flat {
    let mut rs = Rs::new();
    let short = rng.next_u32() % 2 == 0;
    let sri = Sri::random(rng, short);
    let mut ics = random_ics(rng, &sri.t, short);
    ics.max_sf_bands = ics.max_sf_bands.max(1);
    let groups = ics.window_groups as i32;
    let maxsfb = ics.max_sf_bands as i32;
    let p = &mut rs.ch[ch];
    p.ics_info = ics;
    let d = &mut *p.p_dyn_data;
    d.raw_data_info.global_gain = rng.next_u32() as u8;
    for i in 0..128 {
        d.a_code_book[i] = random_cb(rng);
        d.a_scale_factor[i] = (rng.next_u32() % 400) as i16 - 200;
    }
    let band = |rng: &mut Rng, init: i32| -> i16 {
        if rng.next_u32() % 4 == 0 {
            init as i16
        } else {
            (16 * (rng.next_u32() % groups as u32) + rng.next_u32() % maxsfb as u32) as i16
        }
    };
    let cmin = band(rng, CONCEAL_MIN_INIT);
    let cmax = band(rng, CONCEAL_MAX_INIT);
    let cmin_esc = band(rng, CONCEAL_MIN_INIT);
    let cmax_esc = band(rng, CONCEAL_MAX_INIT);
    let a = &mut rs.com.overlay.aac;
    let r = &mut a.er_rvlc_info;
    r.num_window_groups = groups as u8;
    r.max_sfb_transmitted = maxsfb as u8;
    r.conceal_min = cmin;
    r.conceal_max = cmax;
    r.conceal_min_esc = cmin_esc;
    r.conceal_max_esc = cmax_esc;
    r.rev_global_gain = (rng.next_u32() % 256) as i32;
    r.dpcm_is_last_position = (rng.next_u32() % 15) as i32 - 7;
    r.dpcm_noise_last_position = (rng.next_u32() % 512) as i32;
    r.dpcm_noise_nrg = (rng.next_u32() % 512) as i32;
    for i in 0..128 {
        a.a_rvlc_scf_fwd[i] = (rng.next_u32() % 400) as i16 - 200;
        a.a_rvlc_scf_bwd[i] = if rng.next_u32() % 3 == 0 {
            a.a_rvlc_scf_fwd[i]
        } else {
            (rng.next_u32() % 400) as i16 - 200
        };
        a.a_rvlc_scf_esc[i] = (rng.next_u32() % 50) as i16;
    }
    let ci = &mut rs.st[ch].concealment_info;
    for i in 0..128 {
        ci.a_rvlc_previous_codebook[i] = random_cb(rng);
        ci.a_rvlc_previous_scale_factor[i] = (rng.next_u32() % 400) as i16 - 200;
    }
    Flat::of(&mut rs)
}

#[test]
fn conceal_functions_random() {
    let mut rng = Rng::new(0xc0c);
    for i in 0..20000 {
        let ch = (rng.next_u32() % 2) as usize;
        let f = random_conceal_state(&mut rng, ch);
        let k = rng.next_u32() % 4;
        let names = [
            "BidirectionalEstimation_UseLowerScfOfCurrentFrame",
            "BidirectionalEstimation_UseScfOfPrevFrameAsReference",
            "StatisticalEstimation",
            "PredictiveInterpolation",
        ];
        both(
            &format!("{} {i}", names[k as usize]),
            &f,
            None,
            |ctx, _| {
                let chi = ch as i32;
                unsafe {
                    match k {
                        0 => c::cref_rvlcconceal_BidirectionalEstimation_UseLowerScfOfCurrentFrame(ctx, chi),
                        1 => c::cref_rvlcconceal_BidirectionalEstimation_UseScfOfPrevFrameAsReference(ctx, chi),
                        2 => c::cref_rvlcconceal_StatisticalEstimation(ctx, chi),
                        _ => c::cref_rvlcconceal_PredictiveInterpolation(ctx, chi),
                    }
                };
                0
            },
            |rs, _| {
                let Rs { ch: chs, st, com, .. } = rs;
                match k {
                    0 => bidirectional_estimation_use_lower_scf_of_current_frame(&mut chs[ch], com),
                    1 => bidirectional_estimation_use_scf_of_prev_frame_as_reference(
                        &mut chs[ch],
                        &mut st[ch],
                        com,
                    ),
                    2 => statistical_estimation(&mut chs[ch], com),
                    _ => predictive_interpolation(&mut chs[ch], &mut st[ch], com),
                }
                0
            },
        );
    }
}

#[test]
fn calc_ref_val_random() {
    let mut rng = Rng::new(0xcf);
    for i in 0..20000 {
        let ch = (rng.next_u32() % 2) as usize;
        let mut f = random_conceal_state(&mut rng, ch);
        // calcRefValFwd/Bwd run after the borders were replaced (not at their initial values)
        let mut rs = f.to_rs();
        let r = &mut rs.com.overlay.aac.er_rvlc_info;
        if r.conceal_min as i32 == CONCEAL_MIN_INIT {
            r.conceal_min = 0;
        }
        if r.conceal_max as i32 == CONCEAL_MAX_INIT {
            r.conceal_max = ((r.num_window_groups as i32 - 1) * 16 + r.max_sfb_transmitted as i32 - 1) as i16;
        }
        f = Flat::of(&mut rs);
        let dir = (rng.next_u32() % 2) as i32;
        let mut out = [0i32; 3];
        let ctx = Ctx::new();
        ctx.set(&f);
        unsafe { c::cref_rvlcconceal_calcRefVal(ctx.ptr(), ch as i32, dir, out.as_mut_ptr()) };
        let rs = f.to_rs();
        let a = &rs.com.overlay.aac;
        let mut r = [0i32; 3];
        let (r0, rest) = r.split_at_mut(1);
        let (r1, r2) = rest.split_at_mut(1);
        if dir == 0 {
            calc_ref_val_fwd(&a.er_rvlc_info, &rs.ch[ch], &a.a_rvlc_scf_fwd, &mut r0[0], &mut r1[0], &mut r2[0]);
        } else {
            calc_ref_val_bwd(&a.er_rvlc_info, &rs.ch[ch], &a.a_rvlc_scf_bwd, &mut r0[0], &mut r1[0], &mut r2[0]);
        }
        assert_eq!(out, r, "calcRefVal{} {i}", if dir == 0 { "Fwd" } else { "Bwd" });
        cmp_flat(&format!("calcRefVal {i}"), &ctx.get(), &f);
    }
}

