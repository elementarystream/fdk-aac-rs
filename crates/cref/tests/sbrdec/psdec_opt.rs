// Tests of the optimized PS kernels of fdk_aac_rs_sbrdec::psdec against the straight port
// kept next to them (`*_straight`), on random and extreme inputs. The complete PS processing is
// compared with C++ in psdec.rs and e2e.rs.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::psdec::*;

fn data(rng: &mut Rng, n: usize) -> Vec<i32> {
    let shift = rng.next_u32() % 8;
    (0..n).map(|_| if shift == 0 { rng.i32() } else { rng.i32() >> shift }).collect()
}

#[test]
fn slot_based_rotation_fast_matches_straight() {
    let mut rng = Rng::new(0x0307);
    for iter in 0..20000 {
        let mut ca = PsDecCoefficients::default();
        for g in 0..NO_IID_GROUPS {
            ca.h11r[g] = rng.i32();
            ca.h12r[g] = rng.i32();
            ca.h21r[g] = rng.i32();
            ca.h22r[g] = rng.i32();
            ca.delta_h11r[g] = rng.i32() >> (rng.next_u32() % 32);
            ca.delta_h12r[g] = rng.i32() >> (rng.next_u32() % 32);
            ca.delta_h21r[g] = rng.i32() >> (rng.next_u32() % 32);
            ca.delta_h22r[g] = rng.i32() >> (rng.next_u32() % 32);
        }
        let mut cb = ca;
        let mut a: Vec<Vec<i32>> = (0..4).map(|_| data(&mut rng, 71)).collect();
        let mut b = a.clone();
        let [lr, li, rr, ri] = &mut a[..] else { unreachable!() };
        apply_slot_based_rotation(&mut ca, lr, li, rr, ri);
        let [lr, li, rr, ri] = &mut b[..] else { unreachable!() };
        apply_slot_based_rotation_straight(&mut cb, lr, li, rr, ri);
        assert_eq!(a, b, "iter {iter}");
        assert_eq!((ca.h11r, ca.h12r, ca.h21r, ca.h22r), (cb.h11r, cb.h12r, cb.h21r, cb.h22r));
    }
}
