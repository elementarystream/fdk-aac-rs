// Differential tests of fdk_aac_rs_sacdec::sac_tsd against the C++ reference.

use crate::sac_dec_util::*;
use fdk_aac_rs_cref::sacdec_sac_tsd as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacdec::sac_tsd::*;

fn to_c(t: &TsdData) -> [i32; 66] {
    let mut v = [0i32; 66];
    v[0] = t.bs_tsd_enable as i32;
    v[1] = t.num_slots as i32;
    for i in 0..64 {
        v[2 + i] = t.bs_tsd_tr_phase_data[i] as i32;
    }
    v
}

fn random_tsd(rng: &mut Rng, any: bool) -> TsdData {
    let mut t = TsdData {
        bs_tsd_enable: (rng.next_u32() % 2) as u8,
        num_slots: [32u8, 64][rng.next_u32() as usize % 2],
        ..Default::default()
    };
    for v in t.bs_tsd_tr_phase_data.iter_mut() {
        *v = if any { rng.next_u32() as i8 } else { rng.range(-1, 7) as i8 };
    }
    t
}

#[test]
fn sac_tsd_read() {
    let mut rng = Rng::new(0x75d1);
    for iter in 0..20000 {
        let num_slots = match iter % 7 {
            0..=2 => 32,
            3..=5 => 64,
            _ => [0, 16, 31, 33, 65, -1][rng.next_u32() as usize % 6],
        };
        let mut data = random_bytes(&mut rng, 16);
        if iter % 3 == 0 {
            data[0] |= 0x80; // TSD enabled
        }
        // Small transient counts and codewords at the limits
        if iter % 5 == 0 {
            data[0] &= 0x83;
        }
        if iter % 11 == 0 {
            for b in data[1..10].iter_mut() {
                *b = 0xff;
            }
        }
        let mut t = random_tsd(&mut rng, iter % 4 == 0);
        let mut ct = to_c(&t);
        let mut cbits = 0;
        let mut cbuf = data.clone();
        cbuf.resize(16, 0);
        let cr = unsafe { c::cref_tsd_read(cbuf.as_mut_ptr(), 16, num_slots, ct.as_mut_ptr(), &mut cbits) };
        let mut bs = Bs::new(&data, 16, 128);
        let r = tsd_read(&mut bs.bs, num_slots, &mut t);
        assert_eq!(r, cr, "TsdRead result ({iter})");
        assert_eq!(to_c(&t), ct, "TsdRead data ({iter})");
        assert_eq!(128 - bs.valid(), cbits, "TsdRead bits ({iter})");
    }
}

#[test]
fn sac_tsd_generate_non_tr_and_apply() {
    let mut rng = Rng::new(0x75d2);
    for iter in 0..20000 {
        let n = [71, 23, 31, 39, 64, 7, 8, 0][rng.next_u32() as usize % 8];
        let t = random_tsd(&mut rng, false);
        let ct = to_c(&t);
        let ts = rng.range(0, 63);
        let vd_re = sigs(&mut rng, 71);
        let vd_im = sigs(&mut rng, 71);
        let mut vn_re = sigs(&mut rng, 71);
        let mut vn_im = sigs(&mut rng, 71);
        let (mut cvn_re, mut cvn_im) = (vn_re.clone(), vn_im.clone());
        let (mut cvd_re, mut cvd_im) = (vd_re.clone(), vd_im.clone());
        let cw = unsafe {
            c::cref_tsd_generate_non_tr(
                n,
                ct.as_ptr(),
                ts,
                cvd_re.as_mut_ptr(),
                cvd_im.as_mut_ptr(),
                cvn_re.as_mut_ptr(),
                cvn_im.as_mut_ptr(),
            )
        };
        let w = tsd_generate_non_tr(n, &t, ts, &vd_re, &vd_im, &mut vn_re, &mut vn_im);
        assert_eq!(w == TsdDecorrInput::NonTr, cw == 1, "TsdGenerateNonTr input ({iter})");
        assert_eq!((vn_re, vn_im), (cvn_re, cvn_im), "TsdGenerateNonTr ({iter})");

        let mut d_re = sigs(&mut rng, 71);
        let mut d_im = sigs(&mut rng, 71);
        let (mut cd_re, mut cd_im) = (d_re.clone(), d_im.clone());
        let mut ts_r = ts;
        let mut ts_c = ts;
        unsafe {
            c::cref_tsd_apply(
                n,
                ct.as_ptr(),
                &mut ts_c,
                vd_re.as_ptr(),
                vd_im.as_ptr(),
                cd_re.as_mut_ptr(),
                cd_im.as_mut_ptr(),
            )
        };
        tsd_apply(n, &t, &mut ts_r, &vd_re, &vd_im, &mut d_re, &mut d_im);
        assert_eq!(ts_r, ts_c, "TsdApply ts ({iter})");
        assert_eq!((d_re, d_im), (cd_re, cd_im), "TsdApply ({iter})");
    }
}
