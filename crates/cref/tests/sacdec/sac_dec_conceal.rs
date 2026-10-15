// Differential tests of fdk_aac_rs_sacdec::sac_dec_conceal against the C++ reference.

use fdk_aac_rs_cref::sacdec_sac_dec_conceal as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacdec::sac_dec_conceal::*;

fn to_c(i: &SpatialDecConcealmentInfo) -> [i32; 8] {
    [
        i.conceal_params.method,
        i.conceal_params.num_keep_frames as i32,
        i.conceal_params.num_fade_out_frames as i32,
        i.conceal_params.num_fade_in_frames as i32,
        i.conceal_params.num_release_frames as i32,
        i.conceal_state,
        i.cnt_state_frames as i32,
        i.cnt_valid_frames as i32,
    ]
}

fn small(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 8 {
        0 => 0,
        1 => rng.next_u32(),
        2 => u32::MAX - 1, // (the largest value with a defined reference result)
        _ => rng.next_u32() % 12,
    }
}

fn random_info(rng: &mut Rng) -> SpatialDecConcealmentInfo {
    SpatialDecConcealmentInfo {
        conceal_params: SpatialDecConcealmentParams {
            method: rng.range(0, 1),
            num_keep_frames: small(rng),
            num_fade_out_frames: small(rng),
            num_fade_in_frames: small(rng),
            num_release_frames: small(rng),
        },
        conceal_state: rng.range(0, 5),
        cnt_state_frames: small(rng),
        cnt_valid_frames: small(rng),
    }
}

#[test]
fn sac_dec_conceal_init_update_set_param() {
    let mut rng = Rng::new(0xc0c1);
    for iter in 0..100000 {
        let mut i = random_info(&mut rng);
        let mut ci = to_c(&i);
        match iter % 3 {
            0 => {
                let flags = [0u32, 1, 2, 3, 0xff, rng.next_u32()][rng.next_u32() as usize % 6];
                spatial_dec_concealment_init(&mut i, flags);
                unsafe { c::cref_sac_dec_conceal_init(ci.as_mut_ptr(), flags) };
            }
            1 => {
                let ok = [0, 1, 2, -1][rng.next_u32() as usize % 4];
                spatial_dec_concealment_update_state(&mut i, ok);
                unsafe { c::cref_sac_dec_conceal_update_state(ci.as_mut_ptr(), ok) };
            }
            _ => {
                let param = rng.range(-1, 6);
                let value = match rng.next_u32() % 4 {
                    0 => rng.range(-2, 3),
                    1 => rng.i32(),
                    _ => rng.range(0, 20),
                };
                if rng.next_u32() % 10 == 0 {
                    let r = spatial_dec_concealment_set_param(None, param, value);
                    let cr = unsafe { c::cref_sac_dec_conceal_set_param(core::ptr::null_mut(), param, value) };
                    assert_eq!(r, cr, "SetParam(NULL) ({iter})");
                } else {
                    let r = spatial_dec_concealment_set_param(Some(&mut i), param, value);
                    let cr = unsafe { c::cref_sac_dec_conceal_set_param(ci.as_mut_ptr(), param, value) };
                    assert_eq!(r, cr, "SetParam ({iter})");
                }
            }
        }
        assert_eq!(to_c(&i), ci, "state ({iter})");
    }
}

#[test]
fn sac_dec_conceal_apply() {
    let mut rng = Rng::new(0xc0c2);
    for iter in 0..100000 {
        let i = random_info(&mut rng);
        let ci = to_c(&i);
        let mut cmp = [[0i8; 28]; 9];
        for r in cmp.iter_mut() {
            for v in r.iter_mut() {
                *v = if rng.next_u32() % 4 == 0 { rng.next_u32() as i8 } else { rng.range(-15, 15) as i8 };
            }
        }
        let mut idx_prev = [0i8; 28];
        for v in idx_prev.iter_mut() {
            *v = if rng.next_u32() % 4 == 0 { rng.next_u32() as i8 } else { rng.range(-15, 15) as i8 };
        }
        let mut mode = [0i8; 9];
        for v in mode.iter_mut() {
            *v = rng.range(-1, 3) as i8;
        }
        let with_diff = rng.next_u32() % 2 == 0;
        let mut diff = [[0i8; 28]; 9];
        for r in diff.iter_mut() {
            for v in r.iter_mut() {
                *v = rng.next_u32() as i8;
            }
        }
        let start = rng.range(0, 5);
        let stop = rng.range(start, 28);
        let def = rng.range(-20, 20);
        let nps = rng.range(1, 9);
        let ptype = rng.range(0, 2);

        let (mut cidx, mut cmode, mut cdiff) = (idx_prev, mode, diff);
        let cr = unsafe {
            c::cref_sac_dec_conceal_apply(
                ci.as_ptr(),
                cmp.as_ptr() as *const i8,
                if with_diff { cdiff.as_mut_ptr() as *mut i8 } else { core::ptr::null_mut() },
                cidx.as_mut_ptr(),
                cmode.as_mut_ptr(),
                start,
                stop,
                def,
                ptype,
                nps,
            )
        };
        let r = spatial_dec_concealment_apply(
            &i,
            &cmp,
            if with_diff { Some(&mut diff[..]) } else { None },
            &mut idx_prev,
            &mut mode,
            start,
            stop,
            def as i8,
            ptype,
            nps,
        );
        assert_eq!(r, cr, "result ({iter})");
        assert_eq!(idx_prev, cidx, "idxPrev ({iter})");
        assert_eq!(mode, cmode, "mode ({iter})");
        assert_eq!(diff, cdiff, "diff ({iter})");
    }
}
