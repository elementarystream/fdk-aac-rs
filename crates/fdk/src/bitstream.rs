// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_bitstream.h.

//! Port of `FDK_bitstream.h`: the cached bitstream interface to the bit buffer routines.
//!
//! [`FdkBitStream`] is a `Clone` value whose clones share the memory (see [`crate::bitbuffer`]):
//! the C idiom of saving and restoring a position by struct assignment becomes a clone and an
//! assignment.
//!
//! Every function exists twice: as a free function with the mechanically translated C name
//! (`FDKreadBits(hBs, n)` → `fdk_read_bits(hbs, n)`) and as a method (`hbs.read_bits(n)`), which
//! calls the free function.
//!
//! Shift amounts that are out of range in C (only reachable with unusual states, e.g. a cache
//! filled to 32 bits by `FDKpushBackCache()`) are reproduced modulo 32, which is what both
//! reference builds compute (`lsl`/`lsr` on AArch64, `shl`/`shr` on x86).

use crate::bitbuffer as bb;
use crate::bitbuffer::{
    bit_mask, fdk_copy, fdk_delete_bit_buffer, fdk_feed, fdk_fetch, fdk_get, fdk_get32,
    fdk_get_bwd, fdk_init_bit_buffer_at, fdk_peek32, fdk_push_forward, fdk_put, fdk_put_bwd,
    fdk_reset_bit_buffer, BitMem, FdkBitBuf, MAX_BUFSIZE_BYTES,
};

/// `CACHE_BITS`
pub const CACHE_BITS: u32 = 32;

/// `BUFSIZE_DUMMY_VALUE`
pub const BUFSIZE_DUMMY_VALUE: u32 = MAX_BUFSIZE_BYTES;

/// `FDK_BS_CFG`
pub type FdkBsCfg = u32;
/// `BS_READER`
pub const BS_READER: FdkBsCfg = 0;
/// `BS_WRITER`
pub const BS_WRITER: FdkBsCfg = 1;

/// `FDK_BITSTREAM`
#[derive(Clone, Debug, Default)]
pub struct FdkBitStream {
    pub cache_word: u32,
    pub bits_in_cache: u32,
    pub h_bit_buf: FdkBitBuf,
    pub config_cache: u32,
}

/// `HANDLE_FDK_BITSTREAM`
pub type HandleFdkBitStream<'a> = &'a mut FdkBitStream;

/// `FDKcreateBitStream(pBuffer, bufSize, config)`: allocates and initializes a bitstream over
/// the first `buf_size` bytes of `mem`.
pub fn fdk_create_bit_stream(mem: &BitMem, buf_size: u32, config: FdkBsCfg) -> Box<FdkBitStream> {
    let mut h_bit_stream = Box::<FdkBitStream>::default();
    fdk_init_bit_stream(&mut h_bit_stream, mem, buf_size, 0, config);
    h_bit_stream
}

/// `FDKinitBitStream(hBitStream, pBuffer, bufSize, validBits, config)` with `pBuffer` the start
/// of `mem` (`buf_size` a power of two; see [`crate::bitbuffer::fdk_init_bit_buffer`]).
pub fn fdk_init_bit_stream(
    h_bit_stream: &mut FdkBitStream,
    mem: &BitMem,
    buf_size: u32,
    valid_bits: u32,
    config: FdkBsCfg,
) {
    fdk_init_bit_stream_at(h_bit_stream, mem, 0, buf_size, valid_bits, config)
}

/// `FDKinitBitStream(hBitStream, pBuffer, bufSize, validBits, config)` with `pBuffer` at byte
/// `base` of `mem`.
pub fn fdk_init_bit_stream_at(
    h_bit_stream: &mut FdkBitStream,
    mem: &BitMem,
    base: u32,
    buf_size: u32,
    valid_bits: u32,
    config: FdkBsCfg,
) {
    fdk_init_bit_buffer_at(&mut h_bit_stream.h_bit_buf, mem, base, buf_size, valid_bits);

    // init cache
    h_bit_stream.cache_word = 0;
    h_bit_stream.bits_in_cache = 0;
    h_bit_stream.config_cache = config;
}

/// `FDKresetBitbuffer(hBitStream, config)`: resets the states of the bit buffer and the cache.
pub fn fdk_reset_bitbuffer(h_bit_stream: &mut FdkBitStream, config: FdkBsCfg) {
    fdk_reset_bit_buffer(&mut h_bit_stream.h_bit_buf);

    // init cache
    h_bit_stream.cache_word = 0;
    h_bit_stream.bits_in_cache = 0;
    h_bit_stream.config_cache = config;
}

/// `FDKdeleteBitStream(hBitStream)`: frees a bitstream of `fdk_create_bit_stream()`.
#[allow(clippy::boxed_local)]
pub fn fdk_delete_bit_stream(mut h_bit_stream: Box<FdkBitStream>) {
    fdk_delete_bit_buffer(&mut h_bit_stream.h_bit_buf);
}

/// `FDKreadBits(hBitStream, numberOfBits)`: reads `number_of_bits` (0..=32) bits.
#[inline(always)]
pub fn fdk_read_bits(h_bit_stream: &mut FdkBitStream, number_of_bits: u32) -> u32 {
    let mut bits: u32 = 0;
    let missing_bits = (number_of_bits as i32).wrapping_sub(h_bit_stream.bits_in_cache as i32);

    fdk_assert!(number_of_bits <= 32);
    if missing_bits > 0 {
        if missing_bits != 32 {
            bits = h_bit_stream.cache_word.wrapping_shl(missing_bits as u32);
        }
        h_bit_stream.cache_word = fdk_get32(&mut h_bit_stream.h_bit_buf) as u32;
        h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_add(CACHE_BITS);
    }

    h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_sub(number_of_bits);

    (bits | h_bit_stream.cache_word.wrapping_shr(h_bit_stream.bits_in_cache))
        & bit_mask(number_of_bits)
}

/// The next `number_of_bits` (1..=31) bits without reading them: the bits that
/// `fdk_read_bits(number_of_bits)` would return, while the bitstream is unchanged (an
/// optimization of the port, no C function). Requires a cache of at most 32 bits, which every
/// sequence of reads and of `FDKpushBackCache()` within the asserted limits leaves.
#[inline(always)]
pub fn fdk_peek_bits(h_bit_stream: &FdkBitStream, number_of_bits: u32) -> u32 {
    let b = h_bit_stream.bits_in_cache;
    debug_assert!((1..32).contains(&number_of_bits) && b <= CACHE_BITS);
    if b >= number_of_bits {
        h_bit_stream.cache_word.wrapping_shr(b - number_of_bits) & bit_mask(number_of_bits)
    } else {
        // The low b bits of the cache followed by the first bits of the next FDK_get32() word.
        let next = fdk_peek32(&h_bit_stream.h_bit_buf) as u32;
        let missing = number_of_bits - b;
        ((h_bit_stream.cache_word << missing) | (next >> (CACHE_BITS - missing)))
            & bit_mask(number_of_bits)
    }
}

/// Skips `number_of_bits` (0..=32) bits: the state change of [`fdk_read_bits`] without its
/// value (an optimization of the port, no C function).
#[inline(always)]
pub fn fdk_skip_cached_bits(h_bit_stream: &mut FdkBitStream, number_of_bits: u32) {
    let missing_bits = (number_of_bits as i32).wrapping_sub(h_bit_stream.bits_in_cache as i32);
    if missing_bits > 0 {
        h_bit_stream.cache_word = fdk_get32(&mut h_bit_stream.h_bit_buf) as u32;
        h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_add(CACHE_BITS);
    }
    h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_sub(number_of_bits);
}

/// `FDKreadBit(hBitStream)`
#[inline(always)]
pub fn fdk_read_bit(h_bit_stream: &mut FdkBitStream) -> u32 {
    if h_bit_stream.bits_in_cache == 0 {
        h_bit_stream.cache_word = fdk_get32(&mut h_bit_stream.h_bit_buf) as u32;
        h_bit_stream.bits_in_cache = CACHE_BITS - 1;
        return h_bit_stream.cache_word >> 31;
    }
    h_bit_stream.bits_in_cache -= 1;

    h_bit_stream.cache_word.wrapping_shr(h_bit_stream.bits_in_cache) & 1
}

/// `FDKread2Bits(hBitStream)`: reads 2 bits (the optimized version of `FDKreadBits()` for 2
/// bits).
#[inline(always)]
pub fn fdk_read2_bits(h_bit_stream: &mut FdkBitStream) -> u32 {
    // Version corresponds to optimized FDKreadBits implementation calling FDK_get32, that keeps
    // read pointer aligned.
    let mut bits: u32 = 0;
    let missing_bits = 2i32.wrapping_sub(h_bit_stream.bits_in_cache as i32);
    if missing_bits > 0 {
        bits = h_bit_stream.cache_word << missing_bits;
        h_bit_stream.cache_word = fdk_get32(&mut h_bit_stream.h_bit_buf) as u32;
        h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_add(CACHE_BITS);
    }

    h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_sub(2);

    (bits | h_bit_stream.cache_word.wrapping_shr(h_bit_stream.bits_in_cache)) & 0x3
}

/// `FDKreadBitsBwd(hBitStream, numberOfBits)`: reads `number_of_bits` bits backwards.
#[inline]
pub fn fdk_read_bits_bwd(h_bit_stream: &mut FdkBitStream, number_of_bits: u32) -> u32 {
    let valid_mask = bit_mask(number_of_bits);

    if h_bit_stream.bits_in_cache <= number_of_bits {
        let free_bits = ((CACHE_BITS - 1) as i32).wrapping_sub(h_bit_stream.bits_in_cache as i32);

        h_bit_stream.cache_word = h_bit_stream.cache_word.wrapping_shl(free_bits as u32)
            | fdk_get_bwd(&mut h_bit_stream.h_bit_buf, free_bits as u32) as u32;
        h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_add(free_bits as u32);
    }

    h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_sub(number_of_bits);

    h_bit_stream.cache_word.wrapping_shr(h_bit_stream.bits_in_cache) & valid_mask
}

/// `escapedValue(hBitStream, nBits1, nBits2, nBits3)`: reads an integer value using a varying
/// number of bits (ISO/IEC 23003-3, Table 16).
#[inline]
pub fn escaped_value(
    h_bit_stream: &mut FdkBitStream,
    n_bits1: i32,
    n_bits2: i32,
    n_bits3: i32,
) -> u32 {
    let mut value = fdk_read_bits(h_bit_stream, n_bits1 as u32);

    if value == 1u32.wrapping_shl(n_bits1 as u32).wrapping_sub(1) {
        let value_add = fdk_read_bits(h_bit_stream, n_bits2 as u32);
        value = value.wrapping_add(value_add);
        if value_add == 1u32.wrapping_shl(n_bits2 as u32).wrapping_sub(1) {
            value = value.wrapping_add(fdk_read_bits(h_bit_stream, n_bits3 as u32));
        }
    }

    value
}

/// `FDKgetBits(hBitStream, numBits)`: reads directly from the bit buffer. The cache has to be
/// synchronized before.
#[inline]
pub fn fdk_get_bits(h_bit_stream: &mut FdkBitStream, num_bits: u32) -> u32 {
    fdk_get(&mut h_bit_stream.h_bit_buf, num_bits) as u32
}

/// `FDKwriteBits(hBitStream, value, numberOfBits)`: writes `number_of_bits` (0..=32) bits of
/// `value`. Returns the number of bits written.
#[inline]
pub fn fdk_write_bits(h_bit_stream: &mut FdkBitStream, mut value: u32, number_of_bits: u32) -> u8 {
    let valid_mask = bit_mask(number_of_bits);

    if h_bit_stream.bits_in_cache.wrapping_add(number_of_bits) < CACHE_BITS {
        h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_add(number_of_bits);
        h_bit_stream.cache_word =
            h_bit_stream.cache_word.wrapping_shl(number_of_bits) | (value & valid_mask);
    } else {
        // Put always 32 bits into memory
        // - fill cache's LSBits with MSBits of value
        // - store 32 bits in memory using subroutine
        // - fill remaining bits into cache's LSBits
        // - upper bits in cache are don't care

        // Compute number of bits to be filled into cache
        let missing_bits = (CACHE_BITS as i32).wrapping_sub(h_bit_stream.bits_in_cache as i32);
        let remaining_bits = (number_of_bits as i32).wrapping_sub(missing_bits);
        value &= valid_mask;
        // Avoid shift left by 32 positions
        let mut cache_word = if missing_bits == 32 {
            0
        } else {
            h_bit_stream.cache_word.wrapping_shl(missing_bits as u32)
        };
        cache_word |= value.wrapping_shr(remaining_bits as u32);
        fdk_put(&mut h_bit_stream.h_bit_buf, cache_word, 32);

        h_bit_stream.cache_word = value;
        h_bit_stream.bits_in_cache = remaining_bits as u32;
    }

    number_of_bits as u8
}

/// `FDKwriteBits(hBitStream, value, numberOfBits)` with a possibly null handle: without a
/// bitstream, only the number of bits is returned (used to count bits).
#[inline]
pub fn fdk_write_bits_opt(
    h_bit_stream: Option<&mut FdkBitStream>,
    value: u32,
    number_of_bits: u32,
) -> u8 {
    match h_bit_stream {
        Some(h) => fdk_write_bits(h, value, number_of_bits),
        None => number_of_bits as u8,
    }
}

/// `FDKwriteBitsBwd(hBitStream, value, numberOfBits)`: writes `number_of_bits` bits of `value`
/// backwards. Returns the number of bits written.
#[inline]
pub fn fdk_write_bits_bwd(h_bit_stream: &mut FdkBitStream, value: u32, number_of_bits: u32) -> u8 {
    let valid_mask = bit_mask(number_of_bits);

    if h_bit_stream.bits_in_cache.wrapping_add(number_of_bits) <= CACHE_BITS {
        h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_add(number_of_bits);
        // `CacheWord << numberOfBits` is a shift by 32 for an empty cache and 32 bits: modulo 32.
        h_bit_stream.cache_word =
            h_bit_stream.cache_word.wrapping_shl(number_of_bits) | (value & valid_mask);
    } else {
        fdk_put_bwd(
            &mut h_bit_stream.h_bit_buf,
            h_bit_stream.cache_word,
            h_bit_stream.bits_in_cache,
        );
        h_bit_stream.bits_in_cache = number_of_bits;
        h_bit_stream.cache_word = value & valid_mask;
    }

    number_of_bits as u8
}

/// `FDKwriteEscapedValue(hBitStream, value, nBits1, nBits2, nBits3)`: writes an integer value
/// using a varying number of bits (ISO/IEC 23003-3, Table 16). Returns the number of bits
/// written.
pub fn fdk_write_escaped_value(
    h_bit_stream: &mut FdkBitStream,
    mut value: u32,
    n_bits1: u32,
    n_bits2: u32,
    n_bits3: u32,
) -> u8 {
    let mut nbits: u8 = 0;
    let mut tmp = 1u32.wrapping_shl(n_bits1).wrapping_sub(1);

    if value < tmp {
        nbits = nbits.wrapping_add(fdk_write_bits(h_bit_stream, value, n_bits1));
    } else {
        nbits = nbits.wrapping_add(fdk_write_bits(h_bit_stream, tmp, n_bits1));
        value -= tmp;
        tmp = 1u32.wrapping_shl(n_bits2).wrapping_sub(1);

        if value < tmp {
            nbits = nbits.wrapping_add(fdk_write_bits(h_bit_stream, value, n_bits2));
        } else {
            nbits = nbits.wrapping_add(fdk_write_bits(h_bit_stream, tmp, n_bits2));
            value -= tmp;

            nbits = nbits.wrapping_add(fdk_write_bits(h_bit_stream, value, n_bits3));
        }
    }

    nbits
}

/// `FDKsyncCache(hBitStream)`: clears the cache after reading forward (reader) or flushes it
/// (writer).
#[inline]
pub fn fdk_sync_cache(h_bit_stream: &mut FdkBitStream) {
    if h_bit_stream.config_cache == BS_READER {
        bb::fdk_push_back(
            &mut h_bit_stream.h_bit_buf,
            h_bit_stream.bits_in_cache,
            h_bit_stream.config_cache as u8,
        );
    } else if h_bit_stream.bits_in_cache != 0 {
        // BS_WRITER
        fdk_put(&mut h_bit_stream.h_bit_buf, h_bit_stream.cache_word, h_bit_stream.bits_in_cache);
    }

    h_bit_stream.bits_in_cache = 0;
    h_bit_stream.cache_word = 0;
}

/// `FDKsyncCacheBwd(hBitStream)`: clears the cache after reading backwards.
#[inline]
pub fn fdk_sync_cache_bwd(h_bit_stream: &mut FdkBitStream) {
    if h_bit_stream.config_cache == BS_READER {
        fdk_push_forward(
            &mut h_bit_stream.h_bit_buf,
            h_bit_stream.bits_in_cache,
            h_bit_stream.config_cache as u8,
        );
    } else {
        // BS_WRITER
        fdk_put_bwd(
            &mut h_bit_stream.h_bit_buf,
            h_bit_stream.cache_word,
            h_bit_stream.bits_in_cache,
        );
    }

    h_bit_stream.bits_in_cache = 0;
    h_bit_stream.cache_word = 0;
}

/// `FDKbyteAlign(hBitStream, alignmentAnchor)`: byte alignment with respect to the bit position
/// `alignment_anchor`.
#[inline]
pub fn fdk_byte_align(h_bit_stream: &mut FdkBitStream, alignment_anchor: u32) {
    fdk_sync_cache(h_bit_stream);
    if h_bit_stream.config_cache == BS_READER {
        let n = 8i32.wrapping_sub(
            (alignment_anchor as i32)
                .wrapping_sub(bb::fdk_get_valid_bits(&h_bit_stream.h_bit_buf) as i32)
                & 0x07,
        ) as u32
            & 0x07;
        fdk_push_forward(&mut h_bit_stream.h_bit_buf, n, h_bit_stream.config_cache as u8);
    } else {
        let n = 8u32.wrapping_sub(
            bb::fdk_get_valid_bits(&h_bit_stream.h_bit_buf).wrapping_sub(alignment_anchor) & 0x07,
        ) & 0x07;
        fdk_put(&mut h_bit_stream.h_bit_buf, 0, n);
    }
}

/// `FDKpushBackCache(hBitStream, numberOfBits)`: ungets bits read by the last read call.
#[inline]
pub fn fdk_push_back_cache(h_bit_stream: &mut FdkBitStream, number_of_bits: u32) {
    fdk_assert!(h_bit_stream.bits_in_cache.wrapping_add(number_of_bits) <= CACHE_BITS);
    h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_add(number_of_bits);
}

/// `FDKpushBack(hBitStream, numberOfBits)`: ungets bits (combines cache and bit buffer indices).
#[inline]
pub fn fdk_push_back(h_bit_stream: &mut FdkBitStream, number_of_bits: u32) {
    if h_bit_stream.bits_in_cache.wrapping_add(number_of_bits) < CACHE_BITS
        && h_bit_stream.config_cache == BS_READER
    {
        h_bit_stream.bits_in_cache = h_bit_stream.bits_in_cache.wrapping_add(number_of_bits);
        fdk_sync_cache(h_bit_stream); // sync cache to avoid invalid cache
    } else {
        fdk_sync_cache(h_bit_stream);
        bb::fdk_push_back(
            &mut h_bit_stream.h_bit_buf,
            number_of_bits,
            h_bit_stream.config_cache as u8,
        );
    }
}

/// `FDKpushFor(hBitStream, numberOfBits)`: skips bits (combines cache and bit buffer indices).
#[inline]
pub fn fdk_push_for(h_bit_stream: &mut FdkBitStream, number_of_bits: u32) {
    if h_bit_stream.bits_in_cache > number_of_bits && h_bit_stream.config_cache == BS_READER {
        h_bit_stream.bits_in_cache -= number_of_bits;
    } else {
        fdk_sync_cache(h_bit_stream);
        fdk_push_forward(
            &mut h_bit_stream.h_bit_buf,
            number_of_bits,
            h_bit_stream.config_cache as u8,
        );
    }
}

/// `FDKpushBiDirectional(hBitStream, numberOfBits)`: skips (positive) or ungets (negative) bits.
#[inline]
pub fn fdk_push_bi_directional(h_bit_stream: &mut FdkBitStream, number_of_bits: i32) {
    if number_of_bits >= 0 {
        fdk_push_for(h_bit_stream, number_of_bits as u32);
    } else {
        fdk_push_back(h_bit_stream, number_of_bits.wrapping_neg() as u32);
    }
}

/// `FDKgetValidBits(hBitStream)`: synchronizes the cache and returns the number of valid bits.
#[inline]
pub fn fdk_get_valid_bits(h_bit_stream: &mut FdkBitStream) -> u32 {
    fdk_sync_cache(h_bit_stream);
    bb::fdk_get_valid_bits(&h_bit_stream.h_bit_buf)
}

/// `FDKgetFreeBits(hBitStream)`: the number of bits that can still be written.
#[inline]
pub fn fdk_get_free_bits(h_bit_stream: &FdkBitStream) -> i32 {
    bb::fdk_get_free_bits(&h_bit_stream.h_bit_buf)
}

/// `FDKfeedBuffer(hBitStream, inputBuffer, bufferSize, bytesValid)`: fills the bit buffer from
/// the last `*bytes_valid` bytes of the first `buffer_size` bytes of `input_buffer`.
#[inline]
pub fn fdk_feed_buffer(
    h_bit_stream: &mut FdkBitStream,
    input_buffer: &[u8],
    buffer_size: u32,
    bytes_valid: &mut u32,
) {
    fdk_sync_cache(h_bit_stream);
    fdk_feed(&mut h_bit_stream.h_bit_buf, input_buffer, buffer_size, bytes_valid);
}

/// `FDKcopyBuffer(hBSDst, hBSSrc, bytesValid)`: copies bytes from `h_bs_src` into `h_bs_dst`.
#[inline]
pub fn fdk_copy_buffer(
    h_bs_dst: &mut FdkBitStream,
    h_bs_src: &mut FdkBitStream,
    bytes_valid: &mut u32,
) {
    fdk_sync_cache(h_bs_src);
    fdk_copy(&mut h_bs_dst.h_bit_buf, &mut h_bs_src.h_bit_buf, bytes_valid);
}

/// `FDKfetchBuffer(hBitStream, outputBuffer, writeBytes)`: copies all valid bytes to
/// `output_buffer`.
#[inline]
pub fn fdk_fetch_buffer(
    h_bit_stream: &mut FdkBitStream,
    output_buffer: &mut [u8],
    write_bytes: &mut u32,
) {
    fdk_sync_cache(h_bit_stream);
    fdk_fetch(&mut h_bit_stream.h_bit_buf, output_buffer, write_bytes);
}

impl FdkBitStream {
    /// `FDKinitBitStream(hBitStream, pBuffer, bufSize, validBits, config)` (see
    /// [`fdk_init_bit_stream`]).
    #[inline]
    pub fn init(&mut self, mem: &BitMem, buf_size: u32, valid_bits: u32, config: FdkBsCfg) {
        fdk_init_bit_stream(self, mem, buf_size, valid_bits, config)
    }

    /// `FDKresetBitbuffer(hBitStream, config)`
    #[inline]
    pub fn reset_bitbuffer(&mut self, config: FdkBsCfg) {
        fdk_reset_bitbuffer(self, config)
    }

    /// `FDKreadBits(hBitStream, numberOfBits)`
    #[inline(always)]
    pub fn read_bits(&mut self, number_of_bits: u32) -> u32 {
        fdk_read_bits(self, number_of_bits)
    }

    /// `FDKreadBit(hBitStream)`
    #[inline(always)]
    pub fn read_bit(&mut self) -> u32 {
        fdk_read_bit(self)
    }

    /// `FDKread2Bits(hBitStream)`
    #[inline(always)]
    pub fn read_2bits(&mut self) -> u32 {
        fdk_read2_bits(self)
    }

    /// `FDKreadBitsBwd(hBitStream, numberOfBits)`
    #[inline]
    pub fn read_bits_bwd(&mut self, number_of_bits: u32) -> u32 {
        fdk_read_bits_bwd(self, number_of_bits)
    }

    /// `escapedValue(hBitStream, nBits1, nBits2, nBits3)`
    #[inline]
    pub fn escaped_value(&mut self, n_bits1: i32, n_bits2: i32, n_bits3: i32) -> u32 {
        escaped_value(self, n_bits1, n_bits2, n_bits3)
    }

    /// `FDKgetBits(hBitStream, numBits)`
    #[inline]
    pub fn get_bits(&mut self, num_bits: u32) -> u32 {
        fdk_get_bits(self, num_bits)
    }

    /// `FDKwriteBits(hBitStream, value, numberOfBits)`
    #[inline]
    pub fn write_bits(&mut self, value: u32, number_of_bits: u32) -> u8 {
        fdk_write_bits(self, value, number_of_bits)
    }

    /// `FDKwriteBitsBwd(hBitStream, value, numberOfBits)`
    #[inline]
    pub fn write_bits_bwd(&mut self, value: u32, number_of_bits: u32) -> u8 {
        fdk_write_bits_bwd(self, value, number_of_bits)
    }

    /// `FDKwriteEscapedValue(hBitStream, value, nBits1, nBits2, nBits3)`
    #[inline]
    pub fn write_escaped_value(
        &mut self,
        value: u32,
        n_bits1: u32,
        n_bits2: u32,
        n_bits3: u32,
    ) -> u8 {
        fdk_write_escaped_value(self, value, n_bits1, n_bits2, n_bits3)
    }

    /// `FDKsyncCache(hBitStream)`
    #[inline]
    pub fn sync_cache(&mut self) {
        fdk_sync_cache(self)
    }

    /// `FDKsyncCacheBwd(hBitStream)`
    #[inline]
    pub fn sync_cache_bwd(&mut self) {
        fdk_sync_cache_bwd(self)
    }

    /// `FDKbyteAlign(hBitStream, alignmentAnchor)`
    #[inline]
    pub fn byte_align(&mut self, alignment_anchor: u32) {
        fdk_byte_align(self, alignment_anchor)
    }

    /// `FDKpushBackCache(hBitStream, numberOfBits)`
    #[inline]
    pub fn push_back_cache(&mut self, number_of_bits: u32) {
        fdk_push_back_cache(self, number_of_bits)
    }

    /// `FDKpushBack(hBitStream, numberOfBits)`
    #[inline]
    pub fn push_back(&mut self, number_of_bits: u32) {
        fdk_push_back(self, number_of_bits)
    }

    /// `FDKpushFor(hBitStream, numberOfBits)`
    #[inline]
    pub fn push_for(&mut self, number_of_bits: u32) {
        fdk_push_for(self, number_of_bits)
    }

    /// `FDKpushBiDirectional(hBitStream, numberOfBits)`
    #[inline]
    pub fn push_bi_directional(&mut self, number_of_bits: i32) {
        fdk_push_bi_directional(self, number_of_bits)
    }

    /// `FDKgetValidBits(hBitStream)` (synchronizes the cache)
    #[inline]
    pub fn get_valid_bits(&mut self) -> u32 {
        fdk_get_valid_bits(self)
    }

    /// `FDKgetFreeBits(hBitStream)`
    #[inline]
    pub fn get_free_bits(&self) -> i32 {
        fdk_get_free_bits(self)
    }

    /// `FDKfeedBuffer(hBitStream, inputBuffer, bufferSize, bytesValid)`
    #[inline]
    pub fn feed_buffer(&mut self, input_buffer: &[u8], buffer_size: u32, bytes_valid: &mut u32) {
        fdk_feed_buffer(self, input_buffer, buffer_size, bytes_valid)
    }

    /// `FDKcopyBuffer(hBSDst, hBSSrc, bytesValid)` with `self` as the destination.
    #[inline]
    pub fn copy_buffer(&mut self, src: &mut FdkBitStream, bytes_valid: &mut u32) {
        fdk_copy_buffer(self, src, bytes_valid)
    }

    /// `FDKfetchBuffer(hBitStream, outputBuffer, writeBytes)`
    #[inline]
    pub fn fetch_buffer(&mut self, output_buffer: &mut [u8], write_bytes: &mut u32) {
        fdk_fetch_buffer(self, output_buffer, write_bytes)
    }
}
