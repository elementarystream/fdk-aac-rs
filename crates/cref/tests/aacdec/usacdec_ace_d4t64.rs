// Differential tests of fdk_aac_rs_aacdec::usacdec_ace_d4t64 against the C++ reference.

use fdk_aac_rs_aacdec::usacdec_ace_d4t64::*;
use fdk_aac_rs_cref::aacdec_usacdec_ace_d4t64 as c;
use fdk_aac_rs_cref::testutil::Rng;

/// The bit widths of the 8 indices that `CLpd_AcelpRead()` reads for `nbits`.
fn widths(nbits: i32) -> [u32; 8] {
    match nbits {
        12 => [1, 5, 1, 5, 0, 0, 0, 0],
        16 => [1, 5, 5, 5, 0, 0, 0, 0],
        20 => [5, 5, 5, 5, 0, 0, 0, 0],
        28 => [9, 9, 5, 5, 0, 0, 0, 0],
        36 => [9, 9, 9, 9, 0, 0, 0, 0],
        44 => [13, 13, 9, 9, 0, 0, 0, 0],
        52 => [13, 13, 13, 13, 0, 0, 0, 0],
        64 => [2, 2, 2, 2, 14, 14, 14, 14],
        _ => [16; 8],
    }
}

fn check(index: [i16; 8], nbits: i32, code_init: &[i16; 64]) {
    let mut cr = *code_init;
    let mut cc = *code_init;
    d_acelp_decode_4t64(&index, nbits, &mut cr);
    let mut ci = index;
    unsafe { c::cref_d4t64_decode_4t64(ci.as_mut_ptr(), nbits, cc.as_mut_ptr()) };
    assert_eq!(cr, cc, "index {index:?} nbits {nbits}");
}

#[test]
fn decode_4t64_bitstream_indices() {
    let mut rng = Rng::new(0x12D4_0001);
    for nbits in [12, 16, 20, 28, 36, 44, 52, 64] {
        let w = widths(nbits);
        for _ in 0..20_000 {
            let mut index = [0i16; 8];
            for k in 0..8 {
                index[k] = if w[k] == 0 { rng.i16() } else { (rng.next_u32() & ((1u32 << w[k]) - 1)) as i16 };
            }
            let code: [i16; 64] = core::array::from_fn(|_| rng.i16());
            check(index, nbits, &code);
        }
    }
}

#[test]
fn decode_4t64_exhaustive_small_codebooks() {
    // Every index combination of the 12 and 20 bit codebooks (per track) and of a 64 bit track.
    let zero = [0i16; 64];
    for a in 0..2 {
        for b in 0..32 {
            for c2 in 0..2 {
                check([a, b, c2, (b * 7) & 31, 0, 0, 0, 0], 12, &zero);
            }
        }
    }
    for a in 0..32 {
        for b in 0..32 {
            check([a, b, (a ^ b) & 31, (a + b) & 31, 0, 0, 0, 0], 20, &zero);
        }
    }
    for hi in 0..4 {
        for lo in 0..(1i32 << 14) {
            check([hi, 3 - hi, 0, 1, lo as i16, ((lo * 5) & 0x3fff) as i16, 0, 0x3fff], 64, &zero);
        }
    }
}

#[test]
fn decode_4t64_arbitrary_indices() {
    // Arbitrary SHORT indices (the tracks of the 12 bit codebook stay inside the codevector).
    let mut rng = Rng::new(0x12D4_0002);
    for nbits in [12, 16, 20, 28, 36, 44, 52, 64, 0, 13, -1] {
        for _ in 0..5_000 {
            let mut index: [i16; 8] = core::array::from_fn(|_| rng.i16());
            if nbits == 12 {
                index[0] &= 1;
                index[2] &= 1;
            }
            let code: [i16; 64] = core::array::from_fn(|_| rng.i16());
            check(index, nbits, &code);
        }
    }
}
