// Differential tests of fdk_aac_rs_aacenc::line_pe against the C++ reference.
//
// FDKaacEnc_prepareSfbPe and FDKaacEnc_calcSfbPe are run in sequence on the same
// PE_CHANNEL_DATA (prepared with random contents, compared completely after each call), on
// realistic channels (see sf_estim_data) and on random inputs including the edge values.

use crate::sf_estim_data::{channel, ChannelData};
use fdk_aac_rs_aacenc::line_pe::*;
use fdk_aac_rs_cref::aacenc_line_pe as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{FixpDbl, MAXVAL_DBL, MINVAL_DBL};

fn random_pe(rng: &mut Rng) -> PeChannelData {
    let mut p = PeChannelData::default();
    for i in 0..60 {
        p.sfb_n_lines[i] = rng.i32();
        p.sfb_pe[i] = rng.i32();
        p.sfb_const_part[i] = rng.i32();
        p.sfb_n_active_lines[i] = rng.i32();
    }
    p.pe = rng.i32();
    p.const_part = rng.i32();
    p.n_active_lines = rng.i32();
    p
}

#[allow(clippy::too_many_arguments)]
fn check(
    what: &str,
    pe: &PeChannelData,
    energy_ld: &[FixpDbl; 60],
    thr_ld: &[FixpDbl; 60],
    form_ld: &[FixpDbl; 60],
    offsets: &[i32; 61],
    (sfb_cnt, sfb_per_group, max_sfb): (i32, i32, i32),
    is_book: &[i32; 60],
    is_scale: &[i32; 60],
    skip_prepare: bool,
) {
    let mut r = *pe;
    let mut cc = *pe;
    if !skip_prepare {
        fdk_aac_enc_prepare_sfb_pe(
            &mut r, energy_ld, thr_ld, form_ld, offsets, sfb_cnt, sfb_per_group, max_sfb,
        );
        unsafe {
            c::cref_line_pe_prepareSfbPe(
                &mut cc as *mut PeChannelData as *mut _,
                energy_ld.as_ptr(),
                thr_ld.as_ptr(),
                form_ld.as_ptr(),
                offsets.as_ptr(),
                sfb_cnt,
                sfb_per_group,
                max_sfb,
            )
        };
        assert_eq!(r, cc, "{what}: prepareSfbPe");
    }
    fdk_aac_enc_calc_sfb_pe(
        &mut r, energy_ld, thr_ld, sfb_cnt, sfb_per_group, max_sfb, is_book, is_scale,
    );
    unsafe {
        c::cref_line_pe_calcSfbPe(
            &mut cc as *mut PeChannelData as *mut _,
            energy_ld.as_ptr(),
            thr_ld.as_ptr(),
            sfb_cnt,
            sfb_per_group,
            max_sfb,
            is_book.as_ptr(),
            is_scale.as_ptr(),
        )
    };
    assert_eq!(r, cc, "{what}: calcSfbPe");
}

fn intensity(rng: &mut Rng, on: bool) -> ([i32; 60], [i32; 60]) {
    let mut is_book = [0; 60];
    let mut is_scale = [0; 60];
    if on {
        for i in 0..60 {
            is_book[i] = if rng.range(0, 3) == 0 { 14 + rng.range(0, 1) } else { 0 };
            is_scale[i] = rng.range(-200, 200);
        }
    }
    (is_book, is_scale)
}

#[test]
fn realistic() {
    let mut rng = Rng::new(0x11e_9e);
    for k in 0..400 {
        let d: ChannelData = channel(&mut rng, k % 3 == 2);
        let pe = random_pe(&mut rng);
        let (is_book, is_scale) = intensity(&mut rng, k % 4 == 0);
        check(
            &format!("realistic {k}"),
            &pe,
            &d.energy_ld,
            &d.thr_ld,
            &d.form_factor_ld,
            &d.sfb_offsets,
            (d.sfb_cnt, d.sfb_per_group, d.max_sfb_per_group),
            &is_book,
            &is_scale,
            false,
        );
    }
}

#[test]
fn random() {
    let mut rng = Rng::new(0x11e_9e2);
    let edge = [0, MINVAL_DBL, MAXVAL_DBL, 1, -1, MINVAL_DBL + 1, MAXVAL_DBL - 1];
    for k in 0..3000 {
        let mut e = [0i32; 60];
        let mut t = [0i32; 60];
        let mut f = [0i32; 60];
        for i in 0..60 {
            e[i] = if rng.range(0, 7) == 0 { edge[rng.range(0, 6) as usize] } else { rng.i32() };
            t[i] = if rng.range(0, 7) == 0 { edge[rng.range(0, 6) as usize] } else { rng.i32() };
            f[i] = if rng.range(0, 7) == 0 { edge[rng.range(0, 6) as usize] } else { rng.i32() };
        }
        // increasing offsets with widths 0..=1024 (the C code accepts any)
        let mut o = [0i32; 61];
        for i in 1..61 {
            let w = match rng.range(0, 5) {
                0 => rng.range(0, 1024),
                1 => 0,
                _ => 4 * rng.range(1, 24),
            };
            o[i] = o[i - 1] + w;
        }
        let groups = rng.range(1, 4);
        let per = rng.range(1, 60 / groups);
        let layout = (groups * per, per, rng.range(0, per));
        let pe = random_pe(&mut rng);
        let (is_book, is_scale) = intensity(&mut rng, k % 2 == 0);
        // calcSfbPe alone with random sfbNLines too
        check(&format!("random {k}"), &pe, &e, &t, &f, &o, layout, &is_book, &is_scale, k % 3 == 0);
    }
}
