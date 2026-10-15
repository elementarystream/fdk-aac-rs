// Differential tests of fdk_aac_rs_fdk::bitbuffer against the C++ reference.
//
// A bit buffer (inside a bitstream, see `Pair`) is kept twice: as the Rust value and as a state
// array plus its own memory for the C++ shims. After every call, the return value, the complete
// state and the complete memory of both are compared. The states are random (position, offsets,
// valid bits, cache), so reads and writes go past the filled data and wrap around the ring
// buffer.

use fdk_aac_rs_cref::fdk_bitbuffer as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitbuffer::*;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, FdkBitStream};

/// Size of the memory behind every test buffer (the bit buffer may use less).
pub const MEM: usize = 256;

/// A Rust bitstream and the state of its C++ counterpart, with separate memory.
pub struct Pair {
    pub rs: FdkBitStream,
    pub rbuf: BitMem,
    pub cst: [u32; 9],
    pub cbuf: Vec<u8>,
}

/// The state array of the shims (see `shim/fdk_bitstream.cpp`).
pub fn state(bs: &FdkBitStream) -> [u32; 9] {
    let b = &bs.h_bit_buf;
    [
        bs.cache_word,
        bs.bits_in_cache,
        bs.config_cache,
        b.valid_bits,
        b.read_offset,
        b.write_offset,
        b.bit_ndx,
        b.buf_size(),
        b.buf_bits(),
    ]
}

impl Pair {
    /// A bitstream over `1 << size_log2` bytes of random memory, with a random position, random
    /// offsets, a random number of valid bits and a random cache.
    pub fn random(rng: &mut Rng, size_log2: u32) -> Pair {
        Pair::random_size(rng, 1u32 << size_log2)
    }

    /// As [`Pair::random`], over `size` bytes (1 to `MEM`), which need not be a power of two.
    pub fn random_size(rng: &mut Rng, size: u32) -> Pair {
        let mem: Vec<u8> = (0..MEM).map(|_| rng.next_u32() as u8).collect();
        let mut p = Pair { rs: FdkBitStream::default(), rbuf: BitMem::from_slice(&mem), cst: [0; 9], cbuf: mem };
        let config = rng.next_u32() & 1;
        fdk_init_bit_stream(&mut p.rs, &p.rbuf, size, 0, config);
        let bits = size * 8;
        let b = &mut p.rs.h_bit_buf;
        b.bit_ndx = rng.next_u32() % bits;
        b.read_offset = rng.next_u32() % size;
        b.write_offset = rng.next_u32() % size;
        b.valid_bits = match rng.next_u32() % 4 {
            0 => rng.next_u32(),
            1 => rng.next_u32() % 64,
            _ => rng.next_u32() % (bits + 1),
        };
        p.rs.cache_word = rng.next_u32();
        p.rs.bits_in_cache = rng.next_u32() % 32;
        p.cst = state(&p.rs);
        p
    }

    /// A random pair with a random size of 1 to 256 bytes (small sizes are frequent).
    pub fn any(rng: &mut Rng) -> Pair {
        let l = match rng.next_u32() % 3 {
            0 => rng.next_u32() % 3,
            _ => rng.next_u32() % 9,
        };
        Pair::random(rng, l)
    }

    /// Pointers for the C++ shims.
    pub fn c(&mut self) -> (*mut u32, *mut u8) {
        (self.cst.as_mut_ptr(), self.cbuf.as_mut_ptr())
    }

    pub fn check(&self, what: &str) {
        assert_eq!(state(&self.rs), self.cst, "state after {what}");
        assert_eq!(self.rbuf.to_vec(), self.cbuf, "memory after {what}");
    }
}

/// Random bit count: 0..=32, with the edges more frequent.
pub fn nbits(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 8 {
        0 => [0, 1, 7, 8, 24, 25, 31, 32][(rng.next_u32() % 8) as usize],
        _ => rng.next_u32() % 33,
    }
}

/// Random value for push back / forward: small, around the buffer size, or any `UINT`.
fn push_amount(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 4 {
        0 => rng.next_u32(),
        1 => rng.next_u32() % 4096,
        _ => rng.next_u32() % 64,
    }
}

/// One random call of a bit buffer function `op`, on both sides, compared.
pub fn bitbuffer_op(p: &mut Pair, op: u32, rng: &mut Rng) {
    let b = &mut p.rs.h_bit_buf;
    match op {
        0 => {
            let n = nbits(rng);
            let r = fdk_get(b, n);
            let (st, buf) = p.c();
            let e = unsafe { c::cref_bb_fdk_get(st, buf, n) };
            assert_eq!(r, e, "FDK_get({n})");
            p.check("FDK_get");
        }
        1 => {
            let r = fdk_get32(b);
            let (st, buf) = p.c();
            let e = unsafe { c::cref_bb_fdk_get32(st, buf) };
            assert_eq!(r, e, "FDK_get32");
            p.check("FDK_get32");
        }
        2 => {
            let n = nbits(rng);
            let r = fdk_get_bwd(b, n);
            let (st, buf) = p.c();
            let e = unsafe { c::cref_bb_fdk_get_bwd(st, buf, n) };
            assert_eq!(r, e, "FDK_getBwd({n})");
            p.check("FDK_getBwd");
        }
        3 => {
            let (v, n) = (rng.next_u32(), nbits(rng));
            fdk_put(b, v, n);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_put(st, buf, v, n) };
            p.check(&format!("FDK_put({v:#x}, {n})"));
        }
        4 => {
            let (v, n) = (rng.next_u32(), nbits(rng));
            fdk_put_bwd(b, v, n);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_put_bwd(st, buf, v, n) };
            p.check(&format!("FDK_putBwd({v:#x}, {n})"));
        }
        5 => {
            let (n, cfg) = (push_amount(rng), (rng.next_u32() % 3) as u8);
            fdk_push_back(b, n, cfg);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_push_back(st, buf, n, cfg) };
            p.check(&format!("FDK_pushBack({n}, {cfg})"));
        }
        6 => {
            let (n, cfg) = (push_amount(rng), (rng.next_u32() % 3) as u8);
            fdk_push_forward(b, n, cfg);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_push_forward(st, buf, n, cfg) };
            p.check(&format!("FDK_pushForward({n}, {cfg})"));
        }
        7 => {
            let r = fdk_get_valid_bits(b);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bb_fdk_get_valid_bits(st, buf) }, "FDK_getValidBits");
            let r = fdk_get_free_bits(&p.rs.h_bit_buf);
            let (st, buf) = p.c();
            assert_eq!(r, unsafe { c::cref_bb_fdk_get_free_bits(st, buf) }, "FDK_getFreeBits");
            p.check("FDK_getValidBits");
        }
        8 => {
            let len = rng.next_u32() % 600;
            let input: Vec<u8> = (0..len).map(|_| rng.next_u32() as u8).collect();
            let valid = if len == 0 { 0 } else { rng.next_u32() % (len + 1) };
            let (mut rv, mut cv) = (valid, valid);
            fdk_feed(b, &input, len, &mut rv);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_feed(st, buf, input.as_ptr(), len, &mut cv) };
            assert_eq!(rv, cv, "FDK_Feed bytesValid");
            p.check(&format!("FDK_Feed({len}, {valid})"));
        }
        9 => {
            let n = rng.next_u32() % 600;
            let mut rd = vec![0u8; n as usize];
            let mut cd = vec![0u8; n as usize];
            copy_aligned_block(b, &mut rd, n);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_copy_aligned_block(st, buf, cd.as_mut_ptr(), n) };
            assert_eq!(rd, cd, "CopyAlignedBlock output");
            p.check(&format!("CopyAlignedBlock({n})"));
        }
        10 => {
            let n = rng.next_u32() % 600;
            let mut rd = vec![0u8; n as usize];
            let mut cd = vec![0u8; n as usize];
            let (mut rw, mut cw) = (n, n);
            fdk_fetch(b, &mut rd, &mut rw);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_fetch(st, buf, cd.as_mut_ptr(), &mut cw) };
            assert_eq!(rw, cw, "FDK_Fetch writeBytes");
            assert_eq!(rd, cd, "FDK_Fetch output");
            p.check(&format!("FDK_Fetch({n})"));
        }
        11 => {
            let mut src = Pair::any(rng);
            if rng.next_u32() % 2 == 0 {
                // byte aligned source (CopyAlignedBlock path)
                src.rs.h_bit_buf.bit_ndx &= !7;
                src.cst = state(&src.rs);
            }
            let valid = rng.next_u32() % 1200;
            let (mut rv, mut cv) = (valid, valid);
            fdk_copy(&mut p.rs.h_bit_buf, &mut src.rs.h_bit_buf, &mut rv);
            let (st, buf) = p.c();
            let (sst, sbuf) = src.c();
            unsafe { c::cref_bb_fdk_copy(st, buf, sst, sbuf, &mut cv) };
            assert_eq!(rv, cv, "FDK_Copy bytesValid");
            p.check("FDK_Copy (dst)");
            src.check("FDK_Copy (src)");
        }
        12 => {
            fdk_reset_bit_buffer(b);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_reset_bit_buffer(st, buf) };
            p.check("FDK_ResetBitBuffer");
        }
        13 => {
            fdk_delete_bit_buffer(b);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_delete_bit_buffer(st, buf) };
            p.check("FDK_DeleteBitBuffer");
        }
        14 => {
            let size = 1u32 << (rng.next_u32() % 9);
            let valid = rng.next_u32() % (size * 8 + 1);
            fdk_init_bit_buffer(b, &p.rbuf, size, valid);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_init_bit_buffer(st, buf, size, valid) };
            p.check(&format!("FDK_InitBitBuffer({size}, {valid})"));
        }
        15 => {
            let size = 1u32 << (rng.next_u32() % 9);
            fdk_create_bit_buffer(b, &p.rbuf, size);
            let (st, buf) = p.c();
            unsafe { c::cref_bb_fdk_create_bit_buffer(st, buf, size) };
            p.check(&format!("FDK_CreateBitBuffer({size})"));
        }
        _ => unreachable!(),
    }
}

/// Number of bit buffer operations of `bitbuffer_op`.
pub const BITBUFFER_OPS: u32 = 16;

/// `steps` calls of `op` on each of `pairs` random bit buffers.
fn run(op: u32, seed: u64, pairs: usize, steps: usize) {
    let mut rng = Rng::new(seed);
    for _ in 0..pairs {
        let mut p = Pair::any(&mut rng);
        for _ in 0..steps {
            bitbuffer_op(&mut p, op, &mut rng);
        }
    }
}

/// The C code asserts that the buffer size is a power of two, but the assertion is compiled out
/// (NDEBUG), so other sizes must behave as in the reference, including the fast path of
/// `FDK_get32`.
#[test]
fn non_power_of_two_sizes() {
    let mut rng = Rng::new(0x6e70_6f74);
    for op in 0..BITBUFFER_OPS {
        for _ in 0..300 {
            let size = 1 + rng.next_u32() % MEM as u32;
            let mut p = Pair::random_size(&mut rng, size);
            for _ in 0..20 {
                bitbuffer_op(&mut p, op, &mut rng);
            }
        }
    }
}

#[test]
fn bit_mask_table() {
    let mut n = 0;
    let p = unsafe { c::cref_bb_bit_mask(&mut n) };
    let cm = unsafe { std::slice::from_raw_parts(p, n as usize) };
    assert_eq!(BIT_MASK.as_slice(), cm);
    for i in 0..=40 {
        assert_eq!(bit_mask(i), if i <= 32 { cm[i as usize] } else { 0 });
    }
}

#[test]
fn fdk_get_() {
    run(0, 1, 2000, 20);
}

#[test]
fn fdk_get32_() {
    run(1, 2, 2000, 20);
}

#[test]
fn fdk_get_bwd_() {
    run(2, 3, 2000, 20);
}

#[test]
fn fdk_put_() {
    run(3, 4, 2000, 20);
}

#[test]
fn fdk_put_bwd_() {
    run(4, 5, 2000, 20);
}

#[test]
fn fdk_push_back_() {
    run(5, 6, 1000, 10);
}

#[test]
fn fdk_push_forward_() {
    run(6, 7, 1000, 10);
}

#[test]
fn fdk_get_valid_free_bits() {
    run(7, 8, 1000, 2);
}

#[test]
fn fdk_feed_() {
    run(8, 9, 1000, 5);
}

#[test]
fn copy_aligned_block_() {
    run(9, 10, 1000, 5);
}

#[test]
fn fdk_fetch_() {
    run(10, 11, 1000, 5);
}

#[test]
fn fdk_copy_() {
    run(11, 12, 1000, 5);
}

#[test]
fn fdk_reset_delete_bit_buffer() {
    run(12, 13, 200, 1);
    run(13, 14, 200, 1);
}

#[test]
fn fdk_init_create_bit_buffer() {
    run(14, 15, 500, 3);
    run(15, 16, 500, 3);
}

/// Random sequences of all bit buffer operations.
#[test]
fn bitbuffer_sequences() {
    let mut rng = Rng::new(17);
    for _ in 0..2000 {
        let mut p = Pair::any(&mut rng);
        for _ in 0..30 {
            let op = rng.next_u32() % BITBUFFER_OPS;
            bitbuffer_op(&mut p, op, &mut rng);
        }
    }
}

/// An uninitialized bit buffer does not touch memory.
#[test]
fn default_bit_buffer() {
    let mut b = FdkBitBuf::default();
    let _ = fdk_get(&mut b, 17);
    let _ = fdk_get32(&mut b);
    let _ = fdk_get_bwd(&mut b, 9);
    fdk_put(&mut b, 0x1234, 13);
    fdk_put_bwd(&mut b, 0x1234, 13);
    let mut out = [0u8; 4];
    let mut w = 4;
    fdk_fetch(&mut b, &mut out, &mut w);
}
