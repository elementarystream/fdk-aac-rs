// Differential tests of fdk_aac_rs_aacdec::rvlcbit against the C++ reference.

use super::aacdec_hcr::Bs;
use fdk_aac_rs_aacdec::rvlcbit::*;
use fdk_aac_rs_cref::aacdec_rvlc as c;
use fdk_aac_rs_cref::testutil::Rng;

#[test]
fn rvlc_read_bit_from_bitstream_random() {
    // Sequences of reads in both directions from random positions around the anchor, on bit
    // buffers of 2 to 1024 bytes, with the cache in every state.
    let mut rng = Rng::new(0x7b1);
    for i in 0..2000 {
        let size_log2 = 1 + rng.next_u32() % 10;
        let mut cbs = Bs::random(&mut rng, size_log2);
        let mut rbs = cbs.clone();
        let anchor = (rng.next_u32() % 20000) as i32 - 2000;
        let mut pos = (rng.next_u32() % 4096) as i32 - 2048;
        let dir = rng.next_u32() % 2;
        for n in 0..40 {
            let mut rpos = pos;
            let (st, buf) = cbs.c();
            let cr = unsafe { c::cref_rvlcbit_rvlcReadBitFromBitstream(st, buf, anchor, &mut pos, dir) };
            let rr = rbs.rs(|b| rvlc_read_bit_from_bitstream(b, anchor, &mut rpos, dir as u8));
            assert_eq!((cr, pos), (rr as u32, rpos), "rvlcReadBitFromBitstream {i}/{n}");
            assert_eq!(cbs.st, rbs.st, "rvlcReadBitFromBitstream {i}/{n}: bitstream");
            if rng.next_u32() % 8 == 0 {
                pos = (rng.next_u32() % 4096) as i32 - 2048;
            }
        }
    }
}
