// Differential tests of fdk_aac_rs_aacdec::aacdec_tns against the C++ reference.
//
// CTns_Reset, CTns_ReadDataPresentFlag, CTns_Read, CTns_ReadDataPresentUsac on random states
// and bitstreams, and CTns_Apply on valid TNS data (as CTns_Read() gives it) and random spectra,
// with the block test context (block_ctx.rs).

use crate::block::random_tns;
use crate::block_ctx::*;
use fdk_aac_rs_aacdec::aacdec_tns::*;
use fdk_aac_rs_cref::aacdec_aacdec_tns as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;

#[test]
fn reset_and_read() {
    let mut rng = Rng::new(0x7a5_0001);
    let mut p = Pair::new();
    for it in 0..6000 {
        let usac = rng.next_u32() % 3 == 0;
        random_state(&mut p, &mut rng, usac, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let data = random_bytes(&mut rng, 64);
        p.set_bs(&data, 64, 512);
        let flags = if usac { AC_USAC } else { [0, AC_ER | AC_LD, AC_RSVD50][(rng.next_u32() % 3) as usize] };
        let what = rng.next_u32() % 4;
        if rng.next_u32() % 2 == 0 {
            p.r.ci[ch].p_dyn_data.tns_data.data_present = 1;
        }
        p.sync();
        match what {
            0 => {
                unsafe { c::cref_tns_reset(p.c.0, ch as i32) };
                ctns_reset(&mut p.r.ci[ch].p_dyn_data.tns_data);
            }
            1 => {
                unsafe { c::cref_tns_read_data_present_flag(p.c.0, ch as i32) };
                ctns_read_data_present_flag(&mut p.r.bs, &mut p.r.ci[ch].p_dyn_data.tns_data);
            }
            2 => {
                let ce = unsafe { c::cref_tns_read(p.c.0, ch as i32, flags) };
                let ci = &mut p.r.ci[ch];
                let re = ctns_read(&mut p.r.bs, &mut ci.p_dyn_data.tns_data, &ci.ics_info, flags);
                assert_eq!(re, ce, "CTns_Read {it}");
            }
            _ => {
                if !usac {
                    p.r.usac = true;
                    p.sync();
                }
                let cw = (rng.next_u32() % 2) as i32;
                unsafe { c::cref_tns_read_data_present_usac(p.c.0, flags, 0, cw) };
                let RCtx { ci, bs, .. } = &mut p.r;
                let (a, b) = ci.split_at_mut(1);
                let (p0, p1) = (&mut *a[0], &mut *b[0]);
                let ics = p0.ics_info;
                let mut tol = p0.p_dyn_data.specific_to.usac.tns_on_lr;
                ctns_read_data_present_usac(bs, &mut p0.p_dyn_data.tns_data, &mut p1.p_dyn_data.tns_data, &mut tol, &ics, flags, 0, cw);
                p0.p_dyn_data.specific_to.usac.tns_on_lr = tol;
            }
        }
        p.check(&format!("TNS {what} {it}"));
        p.check_bs(&format!("TNS {what} {it}"));
    }
}

#[test]
fn apply() {
    let mut rng = Rng::new(0x7a5_0002);
    let mut p = Pair::new();
    for it in 0..4000 {
        let usac = rng.next_u32() % 4 == 0;
        random_state(&mut p, &mut rng, usac, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        random_tns(&mut rng, &mut p.r.ci[ch]);
        let sh = rng.next_u32() % 16;
        for i in 0..1024 {
            p.r.spec[ch * 1024 + i] = random_spec(&mut rng) >> sh;
        }
        let nbands = rng.range(0, p.r.ci[ch].ics_info.max_sf_bands as i32) as u8;
        let igf = (rng.next_u32() % 4 == 0) as u8;
        let flags = if usac { AC_USAC } else { 0 };
        p.sync();
        unsafe { c::cref_tns_apply(p.c.0, ch as i32, nbands as i32, igf as i32, flags) };
        let sri = p.r.sri;
        let RCtx { ci, spec, .. } = &mut p.r;
        let pc = &ci[ch];
        ctns_apply(&pc.p_dyn_data.tns_data, &pc.ics_info, &mut spec[ch * 1024..], &sri, pc.granule_length, nbands, igf, flags);
        p.check(&format!("CTns_Apply {it}"));
    }
}
