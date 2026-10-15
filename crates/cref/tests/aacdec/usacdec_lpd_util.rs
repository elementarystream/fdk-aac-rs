// Helpers shared by the differential tests of the USAC LPD modules: bitstreams over two
// copies of the same memory, random data.

#![allow(dead_code)]

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, FdkBitStream, BS_READER};

/// The state array of the shims (`st[0..9]`, see the shims).
pub fn bs_state(bs: &FdkBitStream) -> [u32; 9] {
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

/// A Rust bitstream and the state of its C++ counterpart, over separate copies of the same
/// memory.
pub struct Bs {
    pub rs: FdkBitStream,
    pub rbuf: BitMem,
    pub cst: [u32; 9],
    pub cbuf: Vec<u8>,
}

impl Bs {
    /// A reader over `mem` (its length must be a power of two) with `valid_bits` valid bits.
    pub fn new(mem: Vec<u8>, valid_bits: u32) -> Bs {
        let size = mem.len() as u32;
        assert!(size.is_power_of_two());
        let mut b = Bs { rs: FdkBitStream::default(), rbuf: BitMem::from_slice(&mem), cst: [0; 9], cbuf: mem };
        fdk_init_bit_stream(&mut b.rs, &b.rbuf, size, valid_bits, BS_READER);
        b.cst = bs_state(&b.rs);
        b
    }

    /// A reader over `size` random bytes (all bits valid), starting at a random bit.
    pub fn random(rng: &mut Rng, size: usize) -> Bs {
        let mem: Vec<u8> = (0..size).map(|_| rng.next_u32() as u8).collect();
        let mut b = Bs::new(mem, size as u32 * 8);
        let skip = rng.next_u32() % 8;
        fdk_aac_rs_fdk::bitstream::fdk_push_for(&mut b.rs, skip);
        b.cst = bs_state(&b.rs);
        b
    }

    /// A reader over a copy of `bytes` padded to a power of two (all bits of `bytes` valid).
    pub fn from_bytes(bytes: &[u8]) -> Bs {
        let size = bytes.len().max(4).next_power_of_two();
        let mut mem = vec![0u8; size];
        mem[..bytes.len()].copy_from_slice(bytes);
        Bs::new(mem, bytes.len() as u32 * 8)
    }

    pub fn c(&mut self) -> (*mut u32, *mut u8) {
        (self.cst.as_mut_ptr(), self.cbuf.as_mut_ptr())
    }

    pub fn check(&self, what: &str) {
        assert_eq!(bs_state(&self.rs), self.cst, "bitstream state after {what}");
        assert_eq!(self.rbuf.to_vec(), self.cbuf, "bitstream memory after {what}");
    }
}

/// A random `FIXP_DBL` vector with a random magnitude (edge values included).
pub fn rand_dbl_vec(rng: &mut Rng, n: usize) -> Vec<i32> {
    let mode = rng.next_u32() % 6;
    let sh = rng.next_u32() % 32;
    (0..n)
        .map(|_| match mode {
            0 => rng.i32(),
            1 => (rng.next_u32() as i32) >> sh,
            2 => [0, 1, -1, i32::MAX, i32::MIN, i32::MIN + 1][(rng.next_u32() % 6) as usize],
            3 => 0,
            _ => (rng.next_u32() as i32) >> (sh.max(8)),
        })
        .collect()
}

/// A random `FIXP_SGL` vector.
pub fn rand_sgl_vec(rng: &mut Rng, n: usize) -> Vec<i16> {
    (0..n).map(|_| rng.i16()).collect()
}

/// A sorted LSF vector (16 values, scaled by `LSF_SCALE`, between 0 and 6400 Hz).
pub fn rand_lsf(rng: &mut Rng) -> [i16; 16] {
    let mut v = [0i16; 16];
    for x in v.iter_mut() {
        *x = (rng.next_u32() % 25600) as i16;
    }
    v.sort_unstable();
    v
}

/// Asserts that two vectors are equal, reporting the first difference.
#[track_caller]
pub fn assert_vec_eq(a: &[i32], b: &[i32], what: &str) {
    if a != b {
        if a.len() != b.len() {
            panic!("{what}: lengths differ: {} != {}", a.len(), b.len());
        }
        let i = a.iter().zip(b).position(|(x, y)| x != y).unwrap();
        let lo = i.saturating_sub(3);
        let hi = (i + 4).min(a.len());
        panic!("{what}: first difference at {i}: {:?} != {:?} (from {lo})", &a[lo..hi], &b[lo..hi]);
    }
}
