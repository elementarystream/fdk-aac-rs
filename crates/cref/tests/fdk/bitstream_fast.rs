// Tests of the optimized bit reading functions against the straight ports: the fast path
// of `FDK_get32()` and the port's peek/skip helpers of the Huffman table decoding. The straight
// functions keep their differential tests against C (bitbuffer.rs, bitstream.rs), which run
// the fast `fdk_get32()` as well.

use super::bitbuffer::{state, Pair, MEM};
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitbuffer::{fdk_get32, fdk_get32_straight};
use fdk_aac_rs_fdk::bitstream::*;

/// A random pair over any size (1 to `MEM` bytes, often a power of two), with any cache state
/// (0 to 32 bits).
fn any_pair(rng: &mut Rng) -> Pair {
    let l = rng.next_u32() % 9;
    let size = 1 + rng.next_u32() % MEM as u32;
    let mut p = if rng.next_u32() % 2 == 0 { Pair::random(rng, l) } else { Pair::random_size(rng, size) };
    p.rs.bits_in_cache = rng.next_u32() % 33;
    if rng.next_u32() % 4 == 0 {
        // positions near the end of the buffer, where the fast path ends
        let bits = p.rs.h_bit_buf.buf_bits();
        p.rs.h_bit_buf.bit_ndx = bits.saturating_sub(1 + rng.next_u32() % 72);
    }
    p
}

#[test]
fn fdk_get32_fast_vs_straight() {
    let mut rng = Rng::new(0xb175_0001);
    for _ in 0..200000 {
        let mut p = any_pair(&mut rng);
        let mut a = p.rs.h_bit_buf.clone();
        let mut b = p.rs.h_bit_buf.clone();
        for i in 0..4 {
            let x = fdk_get32_straight(&mut a);
            let y = fdk_get32(&mut b);
            assert_eq!(x, y, "FDK_get32 {i}");
            p.rs.h_bit_buf = a.clone();
            let sa = state(&p.rs);
            p.rs.h_bit_buf = b.clone();
            assert_eq!(sa, state(&p.rs), "FDK_get32 {i}: state");
        }
    }
}

#[test]
fn peek_and_skip_vs_read_bits() {
    let mut rng = Rng::new(0xb175_0002);
    for _ in 0..200000 {
        let p = any_pair(&mut rng);
        let n = 1 + rng.next_u32() % 31;
        let mut r = p.rs.clone();
        let before = state(&p.rs);
        let v = fdk_peek_bits(&p.rs, n);
        assert_eq!(state(&p.rs), before, "peek changed the state");
        let w = fdk_read_bits(&mut r, n);
        assert_eq!(v, w, "peek {n} bits");
        let m = rng.next_u32() % 33;
        let mut s1 = p.rs.clone();
        let mut s2 = p.rs.clone();
        fdk_read_bits(&mut s1, m);
        fdk_skip_cached_bits(&mut s2, m);
        assert_eq!(state(&s1), state(&s2), "skip {m} bits");
    }
}
