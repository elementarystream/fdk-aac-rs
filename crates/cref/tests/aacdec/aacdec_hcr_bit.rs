// Differential tests of fdk_aac_rs_aacdec::aacdec_hcr_bit against the C++ reference.

use super::aacdec_hcr::Bs;
use fdk_aac_rs_aacdec::aacdec_hcr_bit::*;
use fdk_aac_rs_cref::aacdec_aacdec_hcr as c;
use fdk_aac_rs_cref::testutil::Rng;

#[test]
fn toggle_read_direction_all() {
    for d in 0..256u32 {
        let cr = unsafe { c::cref_aacdec_hcr_bit_ToggleReadDirection(d) };
        assert_eq!(cr, toggle_read_direction(d as u8) as u32, "ToggleReadDirection({d})");
    }
}

#[test]
fn hcr_get_a_bit_from_bitstream_random() {
    // Random positions around the anchor in both directions, sequences of reads on one
    // bitstream (as the HCR decoder reads the segments), with the cache in every state.
    let mut rng = Rng::new(0xb17);
    for i in 0..2000 {
        let size_log2 = 1 + rng.next_u32() % 10;
        let mut cbs = Bs::random(&mut rng, size_log2);
        let mut rbs = cbs.clone();
        let anchor = (rng.next_u32() % 20000) as i32 - 2000;
        let mut lr = [(rng.next_u32() % 4096) as i32 - 2048, (rng.next_u32() % 4096) as i32 - 2048];
        for n in 0..40 {
            let dir = rng.next_u32() % 2;
            let mut rlr = lr;
            let (st, buf) = cbs.c();
            let cr = unsafe { c::cref_aacdec_hcr_bit_HcrGetABitFromBitstream(st, buf, anchor, lr.as_mut_ptr(), dir) };
            let rr = rbs.rs(|b| {
                let (l, r) = rlr.split_at_mut(1);
                hcr_get_a_bit_from_bitstream(b, anchor, &mut l[0], &mut r[0], dir as u8)
            });
            assert_eq!((cr, lr), (rr, rlr), "HcrGetABitFromBitstream {i}/{n}");
            assert_eq!(cbs.st, rbs.st, "HcrGetABitFromBitstream {i}/{n}: bitstream");
            if rng.next_u32() % 8 == 0 {
                lr = [(rng.next_u32() % 4096) as i32 - 2048, (rng.next_u32() % 4096) as i32 - 2048];
            }
        }
    }
}
