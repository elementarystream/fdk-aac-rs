// Differential tests of fdk_aac_rs_aacdec::conceal_types against the C++ reference.

use fdk_aac_rs_aacdec::conceal_types::*;
use fdk_aac_rs_cref::aacdec_conceal_types as c;

#[test]
fn constants() {
    let mut out = [0i32; 32];
    let n = unsafe { c::cref_cnclt_constants(out.as_mut_ptr()) } as usize;
    let r = [
        CONCEAL_MAX_NUM_FADE_FACTORS,
        std::mem::size_of::<FixpCncl>() as i32,
        CNCL_FRACT_BITS,
        ConcealMethodNone,
        ConcealMethodMute,
        ConcealMethodNoise,
        ConcealMethodInter,
        ConcealMethodTonal,
        ConcealState_Ok,
        ConcealState_Single,
        ConcealState_FadeIn,
        ConcealState_Mute,
        ConcealState_FadeOut,
        FADE_TIMEDOMAIN_TOSPECTRALMUTE,
        FADE_TIMEDOMAIN_FROMSPECTRALMUTE,
        FADE_TIMEDOMAIN,
    ];
    assert_eq!(&out[..n], &r[..]);
}

#[test]
fn array_lengths() {
    let mut out = [0i32; 32];
    let n = unsafe { c::cref_cnclt_array_lengths(out.as_mut_ptr()) } as usize;
    let p = CConcealParams::default();
    let i = CConcealmentInfo::default();
    let r = [
        p.fade_out_factor.len(),
        p.fade_in_factor.len(),
        i.spectral_coefficient.len(),
        i.spec_scale.len(),
        i.prev_frame_ok.len(),
        i.win_grp_offset.len(),
        i.att_grp_offset.len(),
        i.a_rvlc_previous_scale_factor.len(),
        i.a_rvlc_previous_codebook.len(),
        i.lsf4.len(),
        i.td_noise_states.len(),
        i.td_noise_coef.len(),
    ];
    let r: Vec<i32> = r.iter().map(|&x| x as i32).collect();
    assert_eq!(&out[..n], &r[..]);
}
