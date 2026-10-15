// Differential tests of fdk_aac_rs_aacdec::aacdec_pns against the C++ reference.
//
// GenerateRandomVector() and ScaleBand() on random inputs; CPns_ResetData, CPns_IsPnsUsed,
// CPns_SetCorrelation, CPns_Read and CPns_Apply on random states of the block test context
// (block_ctx.rs), with the seeds of the element as CChannelElement_Decode() passes them.

use crate::block_ctx::*;
use fdk_aac_rs_aacdec::aac_rom::AAC_CODE_BOOK_DESCRIPTION_TABLE;
use fdk_aac_rs_aacdec::aacdec_pns::*;
use fdk_aac_rs_aacdec::channelinfo::BOOKSCL;
use fdk_aac_rs_cref::aacdec_aacdec_pns as c;
use fdk_aac_rs_cref::testutil::Rng;

#[test]
fn generate_random_vector_and_scale_band() {
    let mut rng = Rng::new(0x9a5_0001);
    for _ in 0..20000 {
        let size = [4, 8, 12, 16, 32, 64, 96, 0, 1][(rng.next_u32() % 9) as usize];
        let seed0 = rng.i32();
        let mut cs = vec![0; size];
        let mut cseed = seed0;
        let ce = unsafe { c::cref_pns_generate_random_vector(cs.as_mut_ptr(), size as i32, &mut cseed) };
        let mut rs = vec![0; size];
        let mut rseed = seed0;
        let re = generate_random_vector(&mut rs, size as i32, &mut rseed);
        assert_eq!((re, rseed, &rs), (ce, cseed, &cs), "GenerateRandomVector({size})");

        let sf = rng.range(-200, 300);
        let ss = rng.range(-10, 40);
        let ne = rng.range(-40, 10);
        let oop = (rng.next_u32() % 2) as i32;
        let spec: Vec<i32> = (0..size).map(|_| rng.i32()).collect();
        let mut cs = spec.clone();
        unsafe { c::cref_pns_scale_band(cs.as_mut_ptr(), size as i32, sf, ss, ne, oop) };
        let mut rs = spec.clone();
        scale_band(&mut rs, size as i32, sf, ss, ne, oop);
        assert_eq!(rs, cs, "ScaleBand(sf {sf}, ss {ss}, ne {ne})");
    }
}

#[test]
fn state_functions() {
    let mut rng = Rng::new(0x9a5_0002);
    let mut p = Pair::new();
    for it in 0..2000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        p.sync();
        match rng.next_u32() % 4 {
            0 => {
                unsafe { c::cref_pns_reset_data(p.c.0, ch as i32) };
                cpns_reset_data(&mut p.r.ci[ch].data.aac.pns_data, &mut p.r.com.pns_inter_channel_data);
                p.check(&format!("CPns_ResetData {it}"));
            }
            1 => {
                let (g, b) = (rng.range(0, 7), rng.range(0, 15));
                let ce = unsafe { c::cref_pns_is_pns_used(p.c.0, ch as i32, g, b) };
                assert_eq!(cpns_is_pns_used(&p.r.ci[ch].data.aac.pns_data, g, b), ce, "CPns_IsPnsUsed {it}");
            }
            2 => {
                let (g, b, o) = (rng.range(0, 7), rng.range(0, 15), (rng.next_u32() % 2) as i32);
                unsafe { c::cref_pns_set_correlation(p.c.0, ch as i32, g, b, o) };
                cpns_set_correlation(&mut p.r.com.pns_inter_channel_data, g, b, o);
                p.check(&format!("CPns_SetCorrelation {it}"));
            }
            _ => {
                let (g, b) = (rng.range(0, 7), rng.range(0, 15));
                let gg = (rng.next_u32() % 256) as i32;
                p.r.ci[ch].data.aac.pns_data.pns_active = (rng.next_u32() % 2) as u8;
                p.r.ci[ch].data.aac.pns_data.current_energy = rng.range(-300, 300);
                let data = random_bytes(&mut rng, 16);
                p.set_bs(&data, 16, 128);
                p.sync();
                unsafe { c::cref_pns_read(p.c.0, ch as i32, gg, b, g) };
                let hcb = &AAC_CODE_BOOK_DESCRIPTION_TABLE[BOOKSCL as usize];
                let ci = &mut p.r.ci[ch];
                cpns_read(&mut ci.data.aac.pns_data, &mut p.r.bs, hcb, &mut ci.p_dyn_data.a_scale_factor, gg as u8, b, g);
                p.check(&format!("CPns_Read {it}"));
                p.check_bs(&format!("CPns_Read {it}"));
            }
        }
    }
}

#[test]
fn apply() {
    let mut rng = Rng::new(0x9a5_0003);
    let mut p = Pair::new();
    for it in 0..3000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        {
            let ci = &mut p.r.ci[ch];
            ci.data.aac.pns_data.pns_active = (rng.next_u32() % 4 != 0) as u8;
            for x in ci.data.aac.pns_data.pns_used.iter_mut() {
                *x = (rng.next_u32() % 3 == 0) as u8 | ((rng.next_u32() % 4) as u8 & 2);
            }
            for i in 0..128 {
                ci.p_dyn_data.a_scale_factor[i] = rng.range(-100, 200) as i16;
            }
            for i in 0..8 {
                ci.spec_scale[i] = rng.range(-5, 30) as i16;
            }
        }
        for x in p.r.com.pns_inter_channel_data.correlated.iter_mut() {
            *x = (rng.next_u32() % 4) as u8;
        }
        p.sync();
        unsafe { c::cref_pns_apply(p.c.0, ch as i32) };
        let sri = p.r.sri;
        let RCtx { ci, sci, com, spec, .. } = &mut p.r;
        let pc = &ci[ch];
        cpns_apply(
            &pc.data.aac.pns_data,
            &pc.ics_info,
            &mut spec[ch * 1024..],
            &pc.spec_scale,
            &pc.p_dyn_data.a_scale_factor,
            &sri,
            pc.granule_length,
            ch as i32,
            &com.pns_inter_channel_data,
            &mut sci[0].pns_current_seed,
            &mut com.pns_random_seed,
        );
        p.check(&format!("CPns_Apply {it}"));
    }
}
