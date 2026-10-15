// Differential tests of fdk_aac_rs_drc::drc_dec_tools against the C++ reference.
//
// The pure functions run on random and edge-case inputs; the selection functions and
// bitstreamContainsMultibandDrc on random configurations (random memory images with counts in
// the ranges the reader produces), including gain set indices past the 12 gain sets, which the
// reference reads from the following bytes of the structure.

use fdk_aac_rs_cref::drc_drc_dec_tools as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_drc::drc_dec_tools::*;
use fdk_aac_rs_drc::drc_dec_types::*;

use super::drc_dec_reader::pod_from_bytes;

fn exponent(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 8 {
        0 => rng.range(-31, 31),
        1 => rng.range(-3, 3),
        _ => rng.range(-8, 12),
    }
}

#[test]
fn get_delta_tmin_matches() {
    let mut rng = Rng::new(0xd5c0_2001);
    for sr in (0..200_000).step_by(997).chain([999, 1000, 1001, 3000, 48000, 96000, i32::MAX]) {
        assert_eq!(get_delta_tmin(sr), unsafe { c::cref_drc_tools_get_delta_tmin(sr) }, "{sr}");
    }
    for _ in 0..10000 {
        let sr = rng.i32();
        assert_eq!(get_delta_tmin(sr), unsafe { c::cref_drc_tools_get_delta_tmin(sr) }, "{sr}");
    }
}

#[test]
fn db_conversions() {
    let mut rng = Rng::new(0xd5c0_2002);
    for _ in 0..200_000 {
        let m = rng.i32();
        let e = exponent(&mut rng);
        let (mut re, mut ce) = (0, 0);
        let r = db2lin(m, e, &mut re);
        let cr = unsafe { c::cref_drc_tools_db2lin(m, e, &mut ce) };
        assert_eq!((r, re), (cr, ce), "dB2lin({m}, {e})");
        let r = approx_db2lin(m, e, &mut re);
        let cr = unsafe { c::cref_drc_tools_approx_db2lin(m, e, &mut ce) };
        assert_eq!((r, re), (cr, ce), "approxDb2lin({m}, {e})");
        let r = lin2db(m, e, &mut re);
        let cr = unsafe { c::cref_drc_tools_lin2db(m, e, &mut ce) };
        assert_eq!((r, re), (cr, ce), "lin2dB({m}, {e})");
    }
}

#[test]
fn downmix_offset() {
    let mut rng = Rng::new(0xd5c0_2003);
    for _ in 0..50_000 {
        let mut d = DownmixInstructions::ZERO;
        d.target_channel_count = rng.range(0, 10) as u8;
        d.bs_downmix_offset = rng.range(0, 4) as u8;
        d.downmix_id = rng.next_u32() as u8;
        let base = match rng.next_u32() % 8 {
            0 => rng.i32(),
            _ => rng.range(0, 10),
        };
        let r = get_downmix_offset(&d, base);
        let cr = unsafe { c::cref_drc_tools_get_downmix_offset(pod_bytes(&d).as_ptr(), base) };
        assert_eq!(r, cr, "getDownmixOffset({}, {}, {base})", d.target_channel_count, d.bs_downmix_offset);
    }
}

/// A random configuration (random memory image) with counts in the ranges of the reader.
fn random_config(rng: &mut Rng) -> Box<UniDrcConfig> {
    let mut cfg = Box::new(UniDrcConfig::ZERO);
    if rng.next_u32() % 2 == 0 {
        for b in pod_bytes_mut(&mut *cfg).iter_mut() {
            *b = rng.next_u32() as u8;
        }
    }
    cfg.drc_coefficients_uni_drc_count = rng.range(0, 2) as u8;
    cfg.drc_instructions_uni_drc_count = rng.range(0, 12) as u8;
    cfg.drc_instructions_count_incl_virtual = rng.range(0, 19) as u8;
    cfg.downmix_instructions_count = rng.range(0, 6) as u8;
    for c in cfg.drc_coefficients_uni_drc.iter_mut() {
        c.drc_location = rng.range(0, 3) as u8;
        for g in c.gain_set.iter_mut() {
            g.band_count = rng.range(0, 6) as u8;
        }
    }
    for inst in cfg.drc_instructions_uni_drc.iter_mut() {
        inst.drc_set_id = rng.range(-3, 8) as i8;
        inst.downmix_id_count = rng.range(0, 8) as u8;
        for d in inst.downmix_id.iter_mut() {
            *d = [0, 0x7f, 1, 2, 3][rng.next_u32() as usize % 5];
        }
        inst.n_drc_channel_groups = rng.range(0, 8) as u8;
        for s in inst.gain_set_index_for_channel_group.iter_mut() {
            *s = match rng.next_u32() % 8 {
                // Past the gain sets; also before them (not produced by the reader), as far as
                // the reads stay inside the structure.
                0 => rng.range(12, 62) as i8,
                1 => rng.range(-5, -1) as i8,
                _ => rng.range(0, 11) as i8,
            };
        }
    }
    for d in cfg.downmix_instructions.iter_mut() {
        d.downmix_id = rng.range(0, 8) as u8;
    }
    cfg
}

#[test]
fn select_and_multiband() {
    let mut rng = Rng::new(0xd5c0_2004);
    for _ in 0..20_000 {
        let cfg = random_config(&mut rng);
        let bytes = pod_bytes(&*cfg);
        let arg = rng.range(-4, 8);
        let to_i = |o: Option<usize>| o.map_or(-1, |i| i as i32);
        for which in 0..4 {
            let a = if which == 3 { [0, 0x7f, 1][rng.next_u32() as usize % 3] } else { arg };
            let r = match which {
                0 => to_i(select_drc_coefficients(&cfg, a)),
                1 => to_i(select_drc_instructions(&cfg, a)),
                2 => to_i(select_downmix_instructions(&cfg, a)),
                _ => bitstream_contains_multiband_drc(&cfg, a),
            };
            let cr = unsafe { c::cref_drc_tools_select(bytes.as_ptr(), which, a) };
            assert_eq!(r, cr, "select #{which}({a})");
        }
    }
    // The memory image round trip.
    let cfg = random_config(&mut rng);
    let back: Box<UniDrcConfig> = pod_from_bytes(pod_bytes(&*cfg));
    assert_eq!(*cfg, *back);
}

#[test]
fn derive_channel_groups() {
    let mut rng = Rng::new(0xd5c0_2005);
    for _ in 0..100_000 {
        let effect = match rng.next_u32() % 4 {
            0 => 0x0400,
            1 => 0x0800,
            2 => rng.next_u32() as i32 & 0xffff,
            _ => rng.next_u32() as i32 & 0x03ff,
        };
        let channel_count = match rng.next_u32() % 8 {
            0 => rng.range(-2, 20),
            _ => rng.range(0, 8),
        };
        let gsi: [i8; 8] = core::array::from_fn(|_| match rng.next_u32() % 4 {
            0 => -1,
            1 => rng.next_u32() as i8,
            _ => rng.range(0, 3) as i8,
        });
        let ducking = effect & 0x0c00 != 0;
        let duck: [DuckingModification; 8] = core::array::from_fn(|_| DuckingModification {
            ducking_scaling_present: rng.range(0, 1) as u8,
            _pad0: 0,
            ducking_scaling: [8192, 4096, 10240, rng.i16()][rng.next_u32() as usize % 4],
        });
        let init_groups: [i8; 8] = core::array::from_fn(|_| rng.next_u32() as i8);
        let init_unique: [i8; 8] = core::array::from_fn(|_| rng.next_u32() as i8);
        let init_dg: [DuckingModification; 8] = core::array::from_fn(|_| DuckingModification {
            ducking_scaling_present: rng.next_u32() as u8,
            _pad0: 0,
            ducking_scaling: rng.i16(),
        });

        let (mut n, mut unique, mut groups, mut dg) = (0xEEu8, init_unique, init_groups, init_dg);
        let r = derive_drc_channel_groups(
            effect,
            channel_count,
            &gsi,
            if ducking { Some(&duck) } else { None },
            &mut n,
            &mut unique,
            &mut groups,
            if ducking { Some(&mut dg) } else { None },
        );
        let (mut cn, mut cunique, mut cgroups, mut cdg) = (0xEEu8, init_unique, init_groups, init_dg);
        let cr = unsafe {
            c::cref_drc_tools_derive_drc_channel_groups(
                effect,
                channel_count,
                gsi.as_ptr(),
                pod_bytes(&duck).as_ptr(),
                ducking as i32,
                &mut cn,
                cunique.as_mut_ptr(),
                cgroups.as_mut_ptr(),
                pod_bytes_mut(&mut cdg).as_mut_ptr(),
            )
        };
        let what = format!("deriveDrcChannelGroups({effect:#x}, {channel_count}, {gsi:?})");
        assert_eq!(r, cr, "{what}");
        assert_eq!(n, cn, "{what}");
        assert_eq!(unique, cunique, "{what}");
        assert_eq!(groups, cgroups, "{what}");
        assert_eq!(dg, cdg, "{what}");
    }
}
