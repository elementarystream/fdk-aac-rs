// Differential tests of fdk_aac_rs_sacdec::sac_calc_m1_and_m2 (and of the OPD smoothing it
// calls) against the C++ reference, through the library's SpatialDecCalculateM1andM2() on a
// spatial decoder in the 212 configurations of USAC (with and without residual and phase
// coding) and LD. Every combination of the CLD, ICC and IPD indices is computed in each mode,
// for bands inside and outside of the IPD and residual bands.

use crate::sac_dec::*;
use crate::sac_dec_state::*;
use crate::sac_dec_util::*;
use fdk_aac_rs_cref::sacdec_sac_dec as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_sacdec::sac_bitdec::*;
use fdk_aac_rs_sacdec::sac_calc_m1_and_m2::*;
use fdk_aac_rs_sacdec::sac_dec_interface::*;

/// Packs the bits of a USAC Mps212Config and parses it (the parser is tested in sac_bitdec).
pub fn usac_config(
    fr_idx: u32,
    tsc: u32,
    decorr: u32,
    high_rate: u32,
    phase: u32,
    otb_phase: Option<u32>,
    res: Option<(u32, u32)>,
    sci: i32,
    csfli: i32,
) -> SpatialSpecificConfig {
    let mut bits: u64 = 0;
    let mut n = 0;
    let mut put = |v: u32, w: u32| {
        bits = (bits << w) | (v as u64 & ((1 << w) - 1));
        n += w;
    };
    put(fr_idx, 3);
    put(0, 3); // bsFixedGainDMX
    put(tsc, 2);
    put(decorr, 2);
    put(high_rate, 1);
    put(phase, 1);
    put(otb_phase.is_some() as u32, 1);
    if let Some(o) = otb_phase {
        put(o, 5);
    }
    if sci > 1 {
        let (nres, pseudo) = res.unwrap_or((0, 0));
        put(nres, 5);
        put(pseudo, 1);
    }
    if tsc == 2 {
        put(1, 1);
    }
    let data = (bits << (64 - n)).to_be_bytes();
    let mut bs = Bs::new(&data, 8, 64);
    let mut ssc = SpatialSpecificConfig::default();
    let r = spatial_dec_parse_mps212_config(&mut bs.bs, &mut ssc, 44100, AOT_USAC, sci, csfli);
    assert_eq!(r, MPS_OK, "usac_config");
    ssc
}

fn calc(p: &mut Pair, ps: i32, what: &str) {
    let r = spatial_dec_calculate_m1_and_m2(&mut p.r.sd, ps, &p.r.frame);
    let cr = unsafe { c::cref_sacinst_calc_m1m2(p.c.0, ps) };
    assert_eq!(r, cr, "{what}: SpatialDecCalculateM1andM2");
    p.check(what);
}

#[test]
fn sac_calc_m1_and_m2_all_indices() {
    let mut rng = Rng::new(0xca1);
    // (phase coding bit, stereoConfigIndex, residual bands)
    let modes: [(u32, i32, u32); 5] = [(0, 1, 0), (1, 1, 0), (0, 2, 7), (1, 2, 10), (1, 2, 0)];
    for (mi, &(phase, sci, nres)) in modes.iter().enumerate() {
        for fr_idx in [1u32, 2, 5] {
            let fr = [0u32, 28, 20, 14, 10, 7, 5, 4][fr_idx as usize];
            let ssc = usac_config(fr_idx, 0, 0, 1, phase, None, Some((nres.min(fr), 0)), sci, 3);
            let (mut p, r) = init_pair(&ssc, 64, 32 * 64, INIT_ALL);
            assert_eq!(r, MPS_OK);
            let npb = p.r.sd.num_parameter_bands as usize;
            let mut combo = 0usize;
            let total = 31 * 8 * 16;
            let mut iter = 0;
            while combo < total {
                let ps = rng.range(0, 8);
                let ipd_bands = [0, 3, 10, npb as i32, rng.range(0, 28)][rng.next_u32() as usize % 5];
                let res_bands = [0, 2, 7, rng.range(0, 28)][rng.next_u32() as usize % 4];
                let mut combos = Vec::new();
                for _ in 0..npb {
                    let k = combo % total;
                    combos.push(((k / 128) as i8, ((k / 16) % 8) as i8, (k % 16) as i8));
                    combo += 1;
                }
                p.set_state(|s| {
                    for (b, &(cld, icc, ipd)) in combos.iter().enumerate() {
                        s.ott_cld__fdk[0][ps as usize][b] = cld;
                        s.ott_icc__fdk[0][ps as usize][b] = icc;
                        s.ott_ipd__fdk[0][ps as usize][b] = ipd;
                    }
                    s.num_ott_bands_ipd = ipd_bands;
                    s.residual_bands[0] = res_bands as u8;
                    for v in s.phase_left__fdk.iter_mut().chain(s.phase_right__fdk.iter_mut()) {
                        *v = (rng.next_u32() >> 3) as i32;
                    }
                    for v in s
                        .smooth_state
                        .opd_left_state__fdk
                        .iter_mut()
                        .chain(s.smooth_state.opd_right_state__fdk.iter_mut())
                    {
                        *v = (rng.next_u32() >> 3) as i32;
                    }
                });
                p.set_frame(|f, _| {
                    f.phase_mode = rng.range(0, 1);
                    f.opd_smoothing_mode = rng.range(0, 1);
                    for (i, v) in f.param_slot.iter_mut().enumerate() {
                        *v = 3 * i as i32 + rng.range(0, 2);
                    }
                    for v in f.ipd_lossless_data[0].bs_quant_coarse_xxx.iter_mut() {
                        *v = rng.range(0, 1) as i8;
                    }
                });
                calc(&mut p, ps, &format!("mode {mi} freqRes {fr_idx} ({iter})"));
                iter += 1;
            }
        }
    }
}

#[test]
fn sac_calc_m1_and_m2_ld_and_init() {
    let mut rng = Rng::new(0xca2);
    for iter in 0..200 {
        let Some((ssc, qmf, frame)) = random_config(&mut rng) else { continue };
        let (mut p, r) = init_pair(&ssc, qmf, frame, INIT_ALL);
        if r != MPS_OK {
            continue;
        }
        for k in 0..20 {
            let ps = rng.range(0, 8);
            p.set_state(|s| {
                for b in 0..28 {
                    s.ott_cld__fdk[0][ps as usize][b] = rng.range(0, 30) as i8;
                    s.ott_icc__fdk[0][ps as usize][b] = rng.range(0, 7) as i8;
                    s.ott_ipd__fdk[0][ps as usize][b] = rng.range(0, 15) as i8;
                }
                s.arbitrary_downmix = rng.range(0, 1);
                s.arbdmx_alpha__fdk[0] = rng.i32();
            });
            calc(&mut p, ps, &format!("LD {iter}/{k}"));
            let (isf, cc) = (rng.range(0, 1), rng.range(0, 1));
            let r = init_m1_and_m2(&mut p.r.sd, isf, cc);
            let cr = unsafe { c::cref_sacinst_init_m1m2(p.c.0, isf, cc) };
            assert_eq!(r, cr);
            p.check(&format!("initM1andM2 {iter}/{k}"));
            for row in 0..2 {
                assert_eq!(
                    spatial_dec_get_residual_index(&p.r.sd, row),
                    unsafe { c::cref_sacinst_get_residual_index(p.c.0, row) }
                );
            }
        }
    }
}
