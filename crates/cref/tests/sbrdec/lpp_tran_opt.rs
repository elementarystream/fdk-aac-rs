// Tests of the optimized filter kernels of fdk_aac_rs_sbrdec::lpp_tran against the straight
// port kept next to them (`*_straight`), on random and extreme inputs. The complete transposer
// is compared with C++ in lpp_tran.rs and e2e.rs.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::lpp_tran::*;

const LEN: usize = MAX_COLS + MAX_OV_COLS + LPC_ORDER;

fn data(rng: &mut Rng, n: usize) -> Vec<i32> {
    let shift = rng.next_u32() % 8;
    (0..n).map(|_| if shift == 0 { rng.i32() } else { rng.i32() >> shift }).collect()
}

fn slots(rng: &mut Rng) -> Vec<Vec<i32>> {
    (0..MAX_COLS + MAX_OV_COLS).map(|_| data(rng, 64)).collect()
}

fn range(rng: &mut Rng) -> (i32, i32) {
    let a = rng.range(0, (MAX_COLS + MAX_OV_COLS) as i32);
    let b = rng.range(0, (MAX_COLS + MAX_OV_COLS) as i32);
    match rng.next_u32() % 4 {
        0 => (b, a),
        _ => (a.min(b), a.max(b)),
    }
}

#[test]
fn lpp_filter_cplx_fast_matches_straight() {
    let mut rng = Rng::new(0x0308);
    for iter in 0..20000 {
        let (start, stop) = range(&mut rng);
        let hb = rng.range(0, 63) as usize;
        let a = [rng.i16(), rng.i16(), rng.i16(), rng.i16()];
        let dynamic_scale =
            if rng.next_u32() % 8 == 0 { rng.range(-40, 31) } else { rng.range(0, 30) };
        let pw = match rng.next_u32() % 2 {
            0 => None,
            _ => Some((rng.i32(), rng.range(-40, 40))),
        };
        let (lr, li) = (data(&mut rng, LEN), data(&mut rng, LEN));
        let (mut ra, mut ia) = (slots(&mut rng), slots(&mut rng));
        let (mut rb, mut ib) = (ra.clone(), ia.clone());
        {
            let mut r: Vec<&mut [i32]> = ra.iter_mut().map(|v| &mut v[..]).collect();
            let mut i: Vec<&mut [i32]> = ia.iter_mut().map(|v| &mut v[..]).collect();
            lpp_filter_cplx(&mut r, &mut i, &lr, &li, start, stop, hb, a, dynamic_scale, pw);
            let mut r: Vec<&mut [i32]> = rb.iter_mut().map(|v| &mut v[..]).collect();
            let mut i: Vec<&mut [i32]> = ib.iter_mut().map(|v| &mut v[..]).collect();
            lpp_filter_cplx_straight(
                &mut r,
                &mut i,
                &lr,
                &li,
                start,
                stop,
                hb,
                a,
                dynamic_scale,
                pw,
            );
        }
        assert_eq!(ra, rb, "iter {iter}");
        assert_eq!(ia, ib, "iter {iter}");
    }
}

#[test]
fn calc_qmf_buffer_real_fast_matches_straight() {
    let mut rng = Rng::new(0x0309);
    for iter in 0..20000 {
        let (start, stop) = range(&mut rng);
        let hb = rng.range(0, 63) as u8;
        let (a0r, a1r) = (rng.i16(), rng.i16());
        let dynamic_scale =
            if rng.next_u32() % 8 == 0 { rng.range(-40, 31) } else { rng.range(0, 30) };
        let lr = data(&mut rng, LEN);
        let off = (start.max(0) as usize).min(LEN - 2);
        let mut ra = slots(&mut rng);
        let mut rb = ra.clone();
        {
            let mut r: Vec<&mut [i32]> = ra.iter_mut().map(|v| &mut v[..]).collect();
            calc_qmf_buffer_real(&mut r, &lr[off..], start, stop, hb, dynamic_scale, a0r, a1r);
            let mut r: Vec<&mut [i32]> = rb.iter_mut().map(|v| &mut v[..]).collect();
            calc_qmf_buffer_real_straight(
                &mut r,
                &lr[off..],
                start,
                stop,
                hb,
                dynamic_scale,
                a0r,
                a1r,
            );
        }
        assert_eq!(ra, rb, "iter {iter}");
    }
}
