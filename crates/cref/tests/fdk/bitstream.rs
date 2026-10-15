// Differential tests of fdk_aac_rs_fdk::bitstream against the C++ reference.
//
// Same method as bitbuffer.rs: a Rust bitstream and its C++ counterpart (state array plus
// separate memory) start from the same random state; after every call the return value, the
// complete state (cache and bit buffer) and the complete memory are compared. Reads go past the
// filled data and wrap around the ring buffer.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use super::bitbuffer::{bitbuffer_op, nbits, state, Pair, BITBUFFER_OPS};
use fdk_aac_rs_cref::fdk_bitstream as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;

/// Random value for push back / forward: small, around the buffer size, or any value.
fn push_amount(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 4 {
        0 => rng.next_u32(),
        1 => rng.next_u32() % 4096,
        _ => rng.next_u32() % 64,
    }
}

/// Random value of `n` bits, sometimes with garbage above them.
fn value(rng: &mut Rng, n: u32) -> u32 {
    let v = rng.next_u32();
    if rng.next_u32() % 4 == 0 || n >= 32 {
        v
    } else {
        v & ((1u32 << n) - 1)
    }
}

/// One random call of a bitstream function `op`, on both sides, compared.
pub fn bitstream_op(p: &mut Pair, op: u32, rng: &mut Rng) {
    let bs = &mut p.rs;
    match op {
        0 => {
            let n = nbits(rng);
            let r = fdk_read_bits(bs, n);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bs_fdk_read_bits(st, buf, n) }, "FDKreadBits({n})");
            p.check(&format!("FDKreadBits({n})"));
        }
        1 => {
            let r = fdk_read_bit(bs);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bs_fdk_read_bit(st, buf) }, "FDKreadBit");
            p.check("FDKreadBit");
        }
        2 => {
            let r = fdk_read2_bits(bs);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bs_fdk_read2_bits(st, buf) }, "FDKread2Bits");
            p.check("FDKread2Bits");
        }
        3 => {
            // At most 31 bits (counts with a defined reference result).
            let n = nbits(rng).min(31);
            let r = fdk_read_bits_bwd(bs, n);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bs_fdk_read_bits_bwd(st, buf, n) }, "FDKreadBitsBwd({n})");
            p.check(&format!("FDKreadBitsBwd({n})"));
        }
        4 => {
            let n: [i32; 3] = core::array::from_fn(|_| match rng.next_u32() % 4 {
                0 => (rng.next_u32() % 32) as i32,
                _ => (rng.next_u32() % 9) as i32,
            });
            let r = escaped_value(bs, n[0], n[1], n[2]);
            let (st, buf) = p.c();
            let e = unsafe { c::cref_bs_escaped_value(st, buf, n[0], n[1], n[2]) };
            assert_eq!(r, e, "escapedValue({n:?})");
            p.check(&format!("escapedValue({n:?})"));
        }
        5 => {
            let n = nbits(rng);
            let r = fdk_get_bits(bs, n);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bs_fdk_get_bits(st, buf, n) }, "FDKgetBits({n})");
            p.check(&format!("FDKgetBits({n})"));
        }
        6 => {
            let n = nbits(rng);
            let v = value(rng, n);
            let r = fdk_write_bits(bs, v, n);
            let (st, buf) = p.c();
            let e = unsafe { c::cref_bs_fdk_write_bits(st, buf, v, n) };
            assert_eq!(r as u32, e, "FDKwriteBits({v:#x}, {n})");
            p.check(&format!("FDKwriteBits({v:#x}, {n})"));
        }
        7 => {
            let n = nbits(rng);
            let v = value(rng, n);
            let r = fdk_write_bits_opt(None, v, n);
            assert_eq!(r as u32, unsafe { c::cref_bs_fdk_write_bits_null(v, n) }, "FDKwriteBits(NULL)");
        }
        8 => {
            let n = nbits(rng);
            let v = value(rng, n);
            let r = fdk_write_bits_bwd(bs, v, n);
            let (st, buf) = p.c();
            let e = unsafe { c::cref_bs_fdk_write_bits_bwd(st, buf, v, n) };
            assert_eq!(r as u32, e, "FDKwriteBitsBwd({v:#x}, {n})");
            p.check(&format!("FDKwriteBitsBwd({v:#x}, {n})"));
        }
        9 => {
            let n1 = rng.next_u32() % 9;
            let n2 = rng.next_u32() % 17;
            let n3 = rng.next_u32() % 33;
            let v = match rng.next_u32() % 3 {
                0 => rng.next_u32() % 16,
                1 => rng.next_u32() % 70000,
                _ => value(rng, n1 + n2),
            };
            let r = fdk_write_escaped_value(bs, v, n1, n2, n3);
            let (st, buf) = p.c();
            let e = unsafe { c::cref_bs_fdk_write_escaped_value(st, buf, v, n1, n2, n3) };
            assert_eq!(r as u32, e, "FDKwriteEscapedValue({v}, {n1}, {n2}, {n3})");
            p.check(&format!("FDKwriteEscapedValue({v}, {n1}, {n2}, {n3})"));
        }
        10 => {
            fdk_sync_cache(bs);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_sync_cache(st, buf) };
            p.check("FDKsyncCache");
        }
        11 => {
            fdk_sync_cache_bwd(bs);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_sync_cache_bwd(st, buf) };
            p.check("FDKsyncCacheBwd");
        }
        12 => {
            let a = rng.next_u32() % 2048;
            fdk_byte_align(bs, a);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_byte_align(st, buf, a) };
            p.check(&format!("FDKbyteAlign({a})"));
        }
        13 => {
            // FDK_ASSERT((BitsInCache + numberOfBits) <= CACHE_BITS)
            let room = CACHE_BITS.saturating_sub(bs.bits_in_cache);
            let n = rng.next_u32() % (room + 1);
            fdk_push_back_cache(bs, n);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_push_back_cache(st, buf, n) };
            p.check(&format!("FDKpushBackCache({n})"));
        }
        14 => {
            let n = push_amount(rng);
            fdk_push_back(bs, n);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_push_back(st, buf, n) };
            p.check(&format!("FDKpushBack({n})"));
        }
        15 => {
            let n = push_amount(rng);
            fdk_push_for(bs, n);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_push_for(st, buf, n) };
            p.check(&format!("FDKpushFor({n})"));
        }
        16 => {
            let n = match rng.next_u32() % 3 {
                0 => rng.next_u32() as i32,
                _ => (rng.next_u32() % 512) as i32 - 256,
            };
            fdk_push_bi_directional(bs, n);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_push_bi_directional(st, buf, n) };
            p.check(&format!("FDKpushBiDirectional({n})"));
        }
        17 => {
            let r = fdk_get_valid_bits(bs);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bs_fdk_get_valid_bits(st, buf) }, "FDKgetValidBits");
            p.check("FDKgetValidBits");
        }
        18 => {
            let r = fdk_get_free_bits(bs);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bs_fdk_get_free_bits(st, buf) }, "FDKgetFreeBits");
            p.check("FDKgetFreeBits");
        }
        19 => {
            let len = rng.next_u32() % 600;
            let input: Vec<u8> = (0..len).map(|_| rng.next_u32() as u8).collect();
            let valid = if len == 0 { 0 } else { rng.next_u32() % (len + 1) };
            let (mut rv, mut cv) = (valid, valid);
            fdk_feed_buffer(bs, &input, len, &mut rv);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_feed_buffer(st, buf, input.as_ptr(), len, &mut cv) };
            assert_eq!(rv, cv, "FDKfeedBuffer bytesValid");
            p.check(&format!("FDKfeedBuffer({len}, {valid})"));
        }
        20 => {
            let mut src = Pair::any(rng);
            let valid = rng.next_u32() % 1200;
            let (mut rv, mut cv) = (valid, valid);
            fdk_copy_buffer(&mut p.rs, &mut src.rs, &mut rv);
            let (st, buf) = p.c();
            let (sst, sbuf) = src.c();
            unsafe { c::cref_bs_fdk_copy_buffer(st, buf, sst, sbuf, &mut cv) };
            assert_eq!(rv, cv, "FDKcopyBuffer bytesValid");
            p.check("FDKcopyBuffer (dst)");
            src.check("FDKcopyBuffer (src)");
        }
        21 => {
            let n = rng.next_u32() % 600;
            let mut rd = vec![0u8; n as usize];
            let mut cd = vec![0u8; n as usize];
            let (mut rw, mut cw) = (n, n);
            fdk_fetch_buffer(bs, &mut rd, &mut rw);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_fetch_buffer(st, buf, cd.as_mut_ptr(), &mut cw) };
            assert_eq!(rw, cw, "FDKfetchBuffer writeBytes");
            assert_eq!(rd, cd, "FDKfetchBuffer output");
            p.check(&format!("FDKfetchBuffer({n})"));
        }
        22 => {
            let size = 1u32 << (rng.next_u32() % 9);
            let valid = rng.next_u32() % (size * 8 + 1);
            let cfg = rng.next_u32() & 1;
            fdk_init_bit_stream(bs, &p.rbuf, size, valid, cfg);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_init_bit_stream(st, buf, size, valid, cfg) };
            p.check(&format!("FDKinitBitStream({size}, {valid}, {cfg})"));
        }
        23 => {
            let cfg = rng.next_u32() & 1;
            fdk_reset_bitbuffer(bs, cfg);
            let (st, buf) = p.c();
            unsafe { c::cref_bs_fdk_reset_bitbuffer(st, buf, cfg) };
            p.check(&format!("FDKresetBitbuffer({cfg})"));
        }
        24 => {
            let size = 1u32 << (rng.next_u32() % 9);
            let cfg = rng.next_u32() & 1;
            let h = fdk_create_bit_stream(&p.rbuf, size, cfg);
            assert!(h.h_bit_buf.mem().ptr_eq(&p.rbuf));
            let mut cst = [0u32; 9];
            let same = unsafe { c::cref_bs_fdk_create_bit_stream(cst.as_mut_ptr(), p.cbuf.as_mut_ptr(), size, cfg) };
            assert_eq!(same, 1, "FDKcreateBitStream buffer");
            assert_eq!(state(&h), cst, "FDKcreateBitStream({size}, {cfg})");
            fdk_delete_bit_stream(h);
        }
        _ => unreachable!(),
    }
}

const BITSTREAM_OPS: u32 = 25;

/// `steps` calls of `op` on each of `pairs` random bitstreams.
fn run(op: u32, seed: u64, pairs: usize, steps: usize) {
    let mut rng = Rng::new(seed);
    for _ in 0..pairs {
        let mut p = Pair::any(&mut rng);
        for _ in 0..steps {
            bitstream_op(&mut p, op, &mut rng);
        }
    }
}

#[test]
fn fdk_read_bits_() {
    run(0, 101, 2000, 40);
}

#[test]
fn fdk_read_bit_() {
    run(1, 102, 1000, 80);
}

#[test]
fn fdk_read2_bits_() {
    run(2, 103, 1000, 80);
}

#[test]
fn fdk_read_bits_bwd_() {
    run(3, 104, 2000, 40);
}

#[test]
fn escaped_value_() {
    run(4, 105, 2000, 20);
}

#[test]
fn fdk_get_bits_() {
    run(5, 106, 1000, 20);
}

#[test]
fn fdk_write_bits_() {
    run(6, 107, 2000, 40);
}

#[test]
fn fdk_write_bits_null() {
    run(7, 108, 100, 20);
}

#[test]
fn fdk_write_bits_bwd_() {
    run(8, 109, 2000, 40);
}

#[test]
fn fdk_write_escaped_value_() {
    run(9, 110, 2000, 20);
}

#[test]
fn fdk_sync_cache_() {
    run(10, 111, 2000, 2);
}

#[test]
fn fdk_sync_cache_bwd_() {
    run(11, 112, 2000, 2);
}

#[test]
fn fdk_byte_align_() {
    run(12, 113, 2000, 3);
}

#[test]
fn fdk_push_back_cache_() {
    run(13, 114, 2000, 3);
}

#[test]
fn fdk_push_back_() {
    run(14, 115, 2000, 5);
}

#[test]
fn fdk_push_for_() {
    run(15, 116, 2000, 5);
}

#[test]
fn fdk_push_bi_directional_() {
    run(16, 117, 2000, 5);
}

#[test]
fn fdk_get_valid_bits_() {
    run(17, 118, 1000, 2);
}

#[test]
fn fdk_get_free_bits_() {
    run(18, 119, 1000, 2);
}

#[test]
fn fdk_feed_buffer_() {
    run(19, 120, 1000, 5);
}

#[test]
fn fdk_copy_buffer_() {
    run(20, 121, 1000, 5);
}

#[test]
fn fdk_fetch_buffer_() {
    run(21, 122, 1000, 5);
}

#[test]
fn fdk_init_bit_stream_() {
    run(22, 123, 500, 3);
}

#[test]
fn fdk_reset_bitbuffer_() {
    run(23, 124, 500, 3);
}

#[test]
fn fdk_create_delete_bit_stream() {
    run(24, 125, 500, 2);
}

/// Random sequences of all bitstream and bit buffer operations, mostly reads or mostly writes.
#[test]
fn bitstream_sequences() {
    let mut rng = Rng::new(126);
    const READS: [u32; 6] = [0, 1, 2, 3, 4, 5];
    const WRITES: [u32; 3] = [6, 8, 9];
    for i in 0..4000 {
        let mut p = Pair::any(&mut rng);
        for _ in 0..40 {
            let r = rng.next_u32() % 100;
            if r < 10 {
                let op = rng.next_u32() % BITBUFFER_OPS;
                bitbuffer_op(&mut p, op, &mut rng);
            } else if r < 50 {
                let ops: &[u32] = if i % 2 == 0 { &READS } else { &WRITES };
                let op = ops[(rng.next_u32() as usize) % ops.len()];
                bitstream_op(&mut p, op, &mut rng);
            } else {
                let op = rng.next_u32() % BITSTREAM_OPS;
                bitstream_op(&mut p, op, &mut rng);
            }
        }
    }
}

/// Write a random sequence with a writer and read it back with a reader, through the Rust code
/// only; together with the differential tests above this checks the port end to end.
#[test]
fn write_then_read() {
    let mut rng = Rng::new(127);
    for _ in 0..200 {
        let mem = BitMem::new(1024);
        let mut w = FdkBitStream::default();
        fdk_init_bit_stream(&mut w, &mem, 1024, 0, BS_WRITER);
        let items: Vec<(u32, u32)> = (0..200)
            .map(|_| {
                let n = 1 + rng.next_u32() % 32;
                (value(&mut rng, n) & bit_mask_n(n), n)
            })
            .collect();
        let total: u32 = items.iter().map(|&(_, n)| n).sum();
        if total > 1024 * 8 {
            continue;
        }
        for &(v, n) in &items {
            fdk_write_bits(&mut w, v, n);
        }
        fdk_sync_cache(&mut w);
        assert_eq!(fdk_get_valid_bits(&mut w), total);
        let mut r = FdkBitStream::default();
        fdk_init_bit_stream(&mut r, &mem, 1024, total, BS_READER);
        for &(v, n) in &items {
            assert_eq!(fdk_read_bits(&mut r, n), v);
        }
    }
}

fn bit_mask_n(n: u32) -> u32 {
    if n >= 32 {
        u32::MAX
    } else {
        (1u32 << n) - 1
    }
}
