// Helpers shared by the differential tests of fdk_aac_rs_sacdec.

#![allow(dead_code)]

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;

/// A bitstream over an owned buffer (the buffer must stay alive and unmoved while the
/// bitstream is used; it is boxed for that).
pub struct Bs {
    pub buf: Box<[u8]>,
    pub bs: FdkBitStream,
}

impl Bs {
    /// A reader over `data` padded with zeros to `size` bytes (a power of two), with `valid_bits`
    /// valid bits.
    pub fn new(data: &[u8], size: usize, valid_bits: u32) -> Self {
        let mut buf = vec![0u8; size].into_boxed_slice();
        let n = data.len().min(size);
        buf[..n].copy_from_slice(&data[..n]);
        let mut bs = FdkBitStream::default();
        fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&buf), size as u32, valid_bits, BS_READER);
        Bs { buf, bs }
    }

    /// The number of bits read so far relative to `valid_bits` at creation.
    pub fn valid(&mut self) -> i32 {
        fdk_get_valid_bits(&mut self.bs) as i32
    }
}

/// Random bytes.
pub fn random_bytes(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng.next_u32() as u8).collect()
}

/// A random FIXP_DBL signal value with a random magnitude.
pub fn sig(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 8 {
        0 => rng.i32(),
        1 => 0,
        _ => (rng.next_u32() as i32) >> (rng.next_u32() % 24 + 1),
    }
}

/// A vector of random signal values.
pub fn sigs(rng: &mut Rng, n: usize) -> Vec<i32> {
    (0..n).map(|_| sig(rng)).collect()
}
