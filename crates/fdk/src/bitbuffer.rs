// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_bitbuffer.h, libFDK/src/FDK_bitbuffer.cpp.

//! Port of `FDK_bitbuffer.h`: common bit buffer read/write routines.
//!
//! In C a bit buffer holds a pointer to a ring buffer whose size is a power of two, and the C
//! code saves and restores bitstream states by struct assignment while several such copies share
//! the memory. Here the memory is a [`BitMem`]: a reference-counted array of bytes that every
//! clone of a bit buffer shares. Clones of a bit buffer (the C struct assignments) read and write
//! the same bytes, and the memory lives as long as any bit buffer refers to it, so no safe use of
//! a bit buffer can reach freed memory. The bytes are held in atomic words (accessed with relaxed
//! ordering), so bit buffers can be sent between threads.
//!
//! All accesses to the memory are masked with the buffer size, as in C, so reads past the valid
//! data wrap around the ring buffer and return what the C code returns. In addition, every access
//! checks the index against the buffer size and the memory: this never triggers for a buffer
//! initialized with a power-of-two size that fits its memory (all indices are masked), but it
//! makes a default (uninitialized) bit buffer harmless (reads give 0, writes are dropped).
//!
//! The functions of `FDK_bitbuffer.cpp` are identical in the `aarch64` and `x86` reference builds
//! (the preprocessed translation units differ only in `FDK_ASSERT`), so there is one version.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

/// `MAX_BUFSIZE_BYTES`: leave 3 bits headroom so MAX_BUFSIZE can be represented in bits as well.
pub const MAX_BUFSIZE_BYTES: u32 = 0x1000_0000;

/// `BitMask[32 + 1]`: `BitMask[n] = (1 << n) - 1` for `n = 0..=32`.
///
/// Generated with `scripts/dump_table.py libFDK/src/FDK_bitbuffer.cpp BitMask UINT --hex
/// --name BIT_MASK`.
pub static BIT_MASK: [u32; 33] = [
    0x00000000, 0x00000001, 0x00000003, 0x00000007, 0x0000000F, 0x0000001F, 0x0000003F, 0x0000007F,
    0x000000FF, 0x000001FF, 0x000003FF, 0x000007FF, 0x00000FFF, 0x00001FFF, 0x00003FFF, 0x00007FFF,
    0x0000FFFF, 0x0001FFFF, 0x0003FFFF, 0x0007FFFF, 0x000FFFFF, 0x001FFFFF, 0x003FFFFF, 0x007FFFFF,
    0x00FFFFFF, 0x01FFFFFF, 0x03FFFFFF, 0x07FFFFFF, 0x0FFFFFFF, 0x1FFFFFFF, 0x3FFFFFFF, 0x7FFFFFFF,
    0xFFFFFFFF,
];

/// `BitMask[n]` for any `n`.
///
/// The C code indexes `BitMask` with the bit count unchecked; all callers pass at most 32 (the
/// bitstream functions assert it). A larger count has no defined result in C; the port returns
/// 0 there instead of panicking (PORTING.md, "Deviations from the reference").
#[inline(always)]
pub fn bit_mask(n: u32) -> u32 {
    match BIT_MASK.get(n as usize) {
        Some(&m) => m,
        None => 0,
    }
}

/// The memory of bit buffers: a fixed-size array of bytes shared by every clone (see the module
/// documentation). Accesses outside the array read 0 and drop the write.
///
/// The bytes are stored big-endian in 64-bit atomic words (byte `i` in bits `56 - 8 * (i % 8)`
/// and up of word `i / 8`), followed by one word that holds no bytes, so that the eight bytes
/// from any index inside the memory are two word loads ([`BitMem::get_be`]). A write of single
/// bytes ([`BitMem::set`], [`BitMem::update`]) replaces its word with a compare-and-exchange,
/// so concurrent writes to different bytes of a word do not undo each other.
#[derive(Clone)]
pub struct BitMem {
    words: Arc<[AtomicU64]>,
    len: usize,
}

impl BitMem {
    /// A zeroed memory of `len` bytes.
    pub fn new(len: usize) -> Self {
        Self { words: (0..len / 8 + 2).map(|_| AtomicU64::new(0)).collect(), len }
    }

    /// A memory holding a copy of `data`.
    pub fn from_slice(data: &[u8]) -> Self {
        let m = Self::new(data.len());
        m.write(0, data);
        m
    }

    /// The size in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the memory has no bytes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The word `k` (0 past the words).
    #[inline(always)]
    fn word(&self, k: usize) -> u64 {
        match self.words.get(k) {
            Some(w) => w.load(Relaxed),
            None => 0,
        }
    }

    /// Sets word `k` to `(word & keep) | set`.
    #[inline(always)]
    fn modify(&self, k: usize, keep: u64, set: u64) {
        if let Some(w) = self.words.get(k) {
            let mut old = w.load(Relaxed);
            while let Err(cur) = w.compare_exchange_weak(old, (old & keep) | set, Relaxed, Relaxed)
            {
                old = cur;
            }
        }
    }

    /// The byte at `idx` (0 outside the memory).
    #[inline(always)]
    pub fn get(&self, idx: usize) -> u8 {
        if idx < self.len {
            (self.word(idx / 8) >> (56 - 8 * (idx % 8))) as u8
        } else {
            0
        }
    }

    /// Writes the byte at `idx` (dropped outside the memory).
    #[inline(always)]
    pub fn set(&self, idx: usize, v: u8) {
        if idx < self.len {
            let s = 56 - 8 * (idx % 8);
            self.modify(idx / 8, !(0xFF << s), (v as u64) << s);
        }
    }

    /// The `n` (at most 8) bytes at `idx` as a big-endian number, if they are all inside the
    /// memory.
    #[inline(always)]
    pub fn get_be(&self, idx: usize, n: usize) -> Option<u64> {
        if n == 0 || n > 8 || idx.checked_add(n)? > self.len {
            return None;
        }
        let (k, s) = (idx / 8, 8 * (idx % 8) as u32);
        let w = self.words.get(k..k + 2)?;
        let (a, b) = (w[0].load(Relaxed), w[1].load(Relaxed));
        // the eight bytes from idx: the rest of word k, then the start of word k + 1 (shifted
        // in two steps, as `s` may be 0)
        let w = (a << s) | ((b >> 1) >> (63 - s));
        Some(w >> (64 - 8 * n as u32))
    }

    /// Sets the `n` (at most 8) bytes at `idx` to `(bytes & keep) | set` (big-endian numbers of
    /// `n` bytes), if they are all inside the memory; returns whether they are.
    #[inline(always)]
    pub fn update(&self, idx: usize, n: usize, keep: u64, set: u64) -> bool {
        match idx.checked_add(n) {
            Some(end) if n > 0 && n <= 8 && end <= self.len => {}
            _ => return false,
        }
        let (k, s) = (idx / 8, 8 * (idx % 8) as u32);
        // as 8-byte windows from idx: the bytes past n are kept
        let r = 64 - 8 * n as u32;
        let (keep, set) = ((keep << r) | ((1u64 << r) - 1), set << r);
        // word k: bytes idx % 8 .. 8 of the window's start
        self.modify(k, (keep >> s) | !(u64::MAX >> s), set >> s);
        // word k + 1: the window's last idx % 8 bytes, if the n bytes reach into it
        if s + 8 * n as u32 > 64 {
            self.modify(k + 1, (keep << (64 - s)) | (u64::MAX >> s), set << (64 - s));
        }
        true
    }

    /// Copies the bytes from `idx` on into `out` (bytes outside the memory read as 0).
    pub fn read(&self, idx: usize, out: &mut [u8]) {
        let n = self.len.saturating_sub(idx).min(out.len());
        let (head, tail) = out.split_at_mut(n);
        let mut chunks = head.chunks_exact_mut(8);
        let mut i = idx;
        for c in &mut chunks {
            c.copy_from_slice(&self.get_be(i, 8).unwrap_or(0).to_be_bytes());
            i += 8;
        }
        for o in chunks.into_remainder() {
            *o = self.get(i);
            i += 1;
        }
        tail.fill(0);
    }

    /// Writes `data` from `idx` on (bytes outside the memory are dropped).
    pub fn write(&self, idx: usize, data: &[u8]) {
        let n = self.len.saturating_sub(idx).min(data.len());
        let data = &data[..n];
        // bytes up to the next word boundary, whole words, the rest
        let head = ((8 - idx % 8) % 8).min(n);
        for (i, &v) in data[..head].iter().enumerate() {
            self.set(idx + i, v);
        }
        let mut chunks = data[head..].chunks_exact(8);
        let mut k = (idx + head) / 8;
        for c in &mut chunks {
            if let Some(w) = self.words.get(k) {
                w.store(u64::from_be_bytes(c.try_into().unwrap()), Relaxed);
            }
            k += 1;
        }
        for (i, &v) in chunks.remainder().iter().enumerate() {
            self.set(k * 8 + i, v);
        }
    }

    /// Sets the `len` bytes from `idx` on to `v` (bytes outside the memory are skipped).
    pub fn fill(&self, idx: usize, len: usize, v: u8) {
        let n = self.len.saturating_sub(idx).min(len);
        let chunk = [v; 64];
        let mut i = 0;
        while i < n {
            let c = (n - i).min(64);
            self.write(idx + i, &chunk[..c]);
            i += c;
        }
    }

    /// A copy of the whole memory.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut v = vec![0; self.len];
        self.read(0, &mut v);
        v
    }

    /// Whether `self` and `other` are the same memory.
    #[inline]
    pub fn ptr_eq(&self, other: &BitMem) -> bool {
        Arc::ptr_eq(&self.words, &other.words)
    }

    /// A new memory with a copy of the bytes (the C struct copy of an array that bit buffers
    /// point into; see [`FdkBitBuf::rebind`]).
    pub fn deep_copy(&self) -> BitMem {
        Self {
            words: self.words.iter().map(|w| AtomicU64::new(w.load(Relaxed))).collect(),
            len: self.len,
        }
    }

    /// `memmove(&mem[dst], &mem[src], len)` (bytes outside the memory read as 0 and are dropped).
    pub fn copy_within(&self, src: usize, dst: usize, len: usize) {
        let mut tmp = vec![0u8; len];
        self.read(src, &mut tmp);
        self.write(dst, &tmp);
    }
}

impl Default for BitMem {
    fn default() -> Self {
        Self::new(0)
    }
}

impl core::fmt::Debug for BitMem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "BitMem({} bytes)", self.len())
    }
}

/// `FDK_BITBUF`
///
/// `Buffer` is the window of `buf_size` bytes of `mem` that starts at `base`.
#[derive(Clone, Debug, Default)]
pub struct FdkBitBuf {
    pub valid_bits: u32,
    pub read_offset: u32,
    pub write_offset: u32,
    pub bit_ndx: u32,

    mem: BitMem,
    base: u32,
    buf_size: u32,
    buf_bits: u32,
}

/// `HANDLE_FDK_BITBUF`
pub type HandleFdkBitBuf<'a> = &'a mut FdkBitBuf;

impl FdkBitBuf {
    /// `Buffer[idx]`
    #[inline(always)]
    fn byte(&self, idx: u32) -> u32 {
        if idx < self.buf_size {
            self.mem.get(self.base as usize + idx as usize) as u32
        } else {
            0
        }
    }

    /// `Buffer[idx] = v`
    #[inline(always)]
    fn set_byte(&mut self, idx: u32, v: u8) {
        if idx < self.buf_size {
            self.mem.set(self.base as usize + idx as usize, v)
        }
    }

    /// The memory of `hBitBuf->Buffer`.
    #[inline]
    pub fn mem(&self) -> &BitMem {
        &self.mem
    }

    /// The offset of `hBitBuf->Buffer` in [`FdkBitBuf::mem`].
    #[inline]
    pub fn base(&self) -> u32 {
        self.base
    }

    /// `hBitBuf->bufSize`
    #[inline]
    pub fn buf_size(&self) -> u32 {
        self.buf_size
    }

    /// `hBitBuf->bufBits`
    #[inline]
    pub fn buf_bits(&self) -> u32 {
        self.buf_bits
    }

    /// Points the bit buffer to `new` if it uses `old` (same window). An owner that copies its
    /// memory with [`BitMem::deep_copy`] rebinds its bit buffers to the copy, which gives the
    /// copy's bit buffers the state of C after the copied struct has been moved back to the
    /// original's address.
    pub fn rebind(&mut self, old: &BitMem, new: &BitMem) {
        if self.mem.ptr_eq(old) {
            self.mem = new.clone();
        }
    }

    /// `Buffer[0..bufSize]` as a copy (bytes outside the memory read as 0).
    pub fn buffer_to_vec(&self) -> Vec<u8> {
        let mut v = vec![0u8; self.buf_size as usize];
        self.mem.read(self.base as usize, &mut v);
        v
    }
}

/// `FDK_CreateBitBuffer(hBitBuffer, pBuffer, bufSize)`: initializes the bit buffer over the first
/// `buf_size` bytes of `mem` and clears them.
pub fn fdk_create_bit_buffer(h_bit_buf: &mut FdkBitBuf, mem: &BitMem, buf_size: u32) {
    fdk_init_bit_buffer(h_bit_buf, mem, buf_size, 0);
    mem.fill(0, buf_size as usize, 0);
}

/// `FDK_DeleteBitBuffer(hBitBuffer)`: does nothing.
pub fn fdk_delete_bit_buffer(_h_bit_buf: &mut FdkBitBuf) {}

/// `FDK_InitBitBuffer(hBitBuffer, pBuffer, bufSize, validBits)` with `pBuffer` the start of
/// `mem`.
///
/// `buf_size` must be a power of two (asserted in C) and should not exceed the memory; bytes of
/// the window outside the memory read as 0 and drop writes.
pub fn fdk_init_bit_buffer(
    h_bit_buf: &mut FdkBitBuf,
    mem: &BitMem,
    buf_size: u32,
    valid_bits: u32,
) {
    fdk_init_bit_buffer_at(h_bit_buf, mem, 0, buf_size, valid_bits)
}

/// `FDK_InitBitBuffer(hBitBuffer, pBuffer, bufSize, validBits)` with `pBuffer` at byte `base` of
/// `mem` (see [`fdk_init_bit_buffer`]).
pub fn fdk_init_bit_buffer_at(
    h_bit_buf: &mut FdkBitBuf,
    mem: &BitMem,
    base: u32,
    buf_size: u32,
    valid_bits: u32,
) {
    h_bit_buf.valid_bits = valid_bits;
    h_bit_buf.read_offset = 0;
    h_bit_buf.write_offset = 0;
    h_bit_buf.bit_ndx = 0;

    h_bit_buf.mem = mem.clone();
    h_bit_buf.base = base;
    h_bit_buf.buf_size = buf_size;
    h_bit_buf.buf_bits = buf_size << 3;
    // assure bufsize (2^n)
    fdk_assert!(h_bit_buf.valid_bits <= h_bit_buf.buf_bits);
    fdk_assert!(buf_size > 0 && buf_size <= MAX_BUFSIZE_BYTES);
    fdk_assert!(buf_size.is_power_of_two());
}

/// `FDK_ResetBitBuffer(hBitBuffer)`
pub fn fdk_reset_bit_buffer(h_bit_buf: &mut FdkBitBuf) {
    h_bit_buf.valid_bits = 0;
    h_bit_buf.read_offset = 0;
    h_bit_buf.write_offset = 0;
    h_bit_buf.bit_ndx = 0;
}

/// `FDK_get(hBitBuffer, numberOfBits)`: reads `number_of_bits` (0..=32) bits.
#[inline]
pub fn fdk_get(h: &mut FdkBitBuf, number_of_bits: u32) -> i32 {
    let byte_offset = h.bit_ndx >> 3;
    let bit_offset = h.bit_ndx & 0x07;

    h.bit_ndx = h.bit_ndx.wrapping_add(number_of_bits) & h.buf_bits.wrapping_sub(1);
    h.valid_bits = h.valid_bits.wrapping_sub(number_of_bits);

    let byte_mask = h.buf_size.wrapping_sub(1);

    // Where the five bytes are inside a power-of-two buffer, the masks change nothing: they are
    // loaded at once (the fifth is shifted out for bit_offset == 0).
    if h.buf_size.is_power_of_two() && byte_offset + 4 < h.buf_size {
        if let Some(w40) = h.mem.get_be(h.base as usize + byte_offset as usize, 5) {
            let tx = ((w40 << bit_offset) >> 8) as u32;
            return tx.wrapping_shr(32u32.wrapping_sub(number_of_bits)) as i32;
        }
    }

    let mut tx = (h.byte(byte_offset & byte_mask) << 24)
        | (h.byte(byte_offset.wrapping_add(1) & byte_mask) << 16)
        | (h.byte(byte_offset.wrapping_add(2) & byte_mask) << 8)
        | h.byte(byte_offset.wrapping_add(3) & byte_mask);

    if bit_offset != 0 {
        tx <<= bit_offset;
        tx |= h.byte(byte_offset.wrapping_add(4) & byte_mask) >> (8 - bit_offset);
    }

    // `tx >> (32 - numberOfBits)`: for numberOfBits == 0 the shift amount is 32 (undefined in
    // C); both reference builds shift by the amount modulo 32 (`lsr`, `shr`) and return `tx`.
    tx.wrapping_shr(32u32.wrapping_sub(number_of_bits)) as i32
}

/// `FDK_get32(hBitBuf)`: reads 32 bits.
///
/// The value is [`fdk_peek32`]; the state change is that of the C function on both of its paths.
#[inline(always)]
pub fn fdk_get32(h: &mut FdkBitBuf) -> i32 {
    let v = fdk_peek32(h);
    h.bit_ndx = h.bit_ndx.wrapping_add(32) & h.buf_bits.wrapping_sub(1);
    h.valid_bits = (h.valid_bits as i32).wrapping_sub(32) as u32;
    v
}

/// The value that `FDK_get32(hBitBuf)` returns, without its state change (no C function).
///
/// Where all five bytes that [`fdk_get32_straight`] may read lie inside the buffer and its fast
/// path applies (`bit_ndx <= buf_bits`, no byte mask), they are loaded directly and combined
/// without a branch on the bit offset; otherwise the straight computation runs. Both give the
/// same value for every bit buffer (tested on random states).
#[inline(always)]
pub fn fdk_peek32(h: &FdkBitBuf) -> i32 {
    let bit_ndx = h.bit_ndx.wrapping_add(32);
    let byte_offset = bit_ndx.wrapping_sub(1) >> 3;
    if bit_ndx <= h.buf_bits && byte_offset >= 4 && byte_offset < h.buf_size {
        // Bytes byte_offset - 4 ..= byte_offset are inside the buffer (checked above).
        if let Some(w40) = h.mem.get_be(h.base as usize + byte_offset as usize - 4, 5) {
            // (byte0:byte1..4) >> (8 - bitNdx % 8), or the last four bytes for bitNdx % 8 == 0.
            return (w40 >> ((8 - (bit_ndx & 7)) & 7)) as u32 as i32;
        }
    }
    peek32_straight(h)
}

/// `FDK_get32(hBitBuf)` as ported (see [`fdk_get32`]).
#[inline]
pub fn fdk_get32_straight(h: &mut FdkBitBuf) -> i32 {
    let v = peek32_straight(h);
    h.bit_ndx = h.bit_ndx.wrapping_add(32) & h.buf_bits.wrapping_sub(1);
    h.valid_bits = (h.valid_bits as i32).wrapping_sub(32) as u32;
    v
}

/// The value of [`fdk_get32_straight`].
#[inline]
fn peek32_straight(h: &FdkBitBuf) -> i32 {
    let bit_ndx = h.bit_ndx.wrapping_add(32);
    let byte_offset = bit_ndx.wrapping_sub(1) >> 3;
    // The C code indexes without the byte mask on its fast path (`BitNdx <= bufBits`) and with
    // it on the wrap-around path. Both give the same bytes only for a buffer size that is a power
    // of two, which the C code asserts but does not enforce (NDEBUG), so the two paths are kept.
    let byte_mask = if bit_ndx <= h.buf_bits { u32::MAX } else { h.buf_size.wrapping_sub(1) };
    let mut cache = (h.byte(byte_offset.wrapping_sub(3) & byte_mask) << 24)
        | (h.byte(byte_offset.wrapping_sub(2) & byte_mask) << 16)
        | (h.byte(byte_offset.wrapping_sub(1) & byte_mask) << 8)
        | h.byte(byte_offset & byte_mask);

    let bit_ndx = bit_ndx & 7;
    if bit_ndx != 0 {
        cache = (cache >> (8 - bit_ndx))
            | (h.byte(byte_offset.wrapping_sub(4) & byte_mask) << (24 + bit_ndx));
    }
    cache as i32
}

/// `FDK_getBwd(hBitBuffer, numberOfBits)`: reads `number_of_bits` bits backwards.
pub fn fdk_get_bwd(h: &mut FdkBitBuf, number_of_bits: u32) -> i32 {
    let byte_offset = h.bit_ndx >> 3;
    let bit_offset = h.bit_ndx & 0x07;
    let byte_mask = h.buf_size.wrapping_sub(1);

    h.bit_ndx = h.bit_ndx.wrapping_sub(number_of_bits) & h.buf_bits.wrapping_sub(1);
    h.valid_bits = h.valid_bits.wrapping_add(number_of_bits);

    let mut tx = (h.byte(byte_offset.wrapping_sub(3) & byte_mask) << 24)
        | (h.byte(byte_offset.wrapping_sub(2) & byte_mask) << 16)
        | (h.byte(byte_offset.wrapping_sub(1) & byte_mask) << 8)
        | h.byte(byte_offset & byte_mask);
    let mut txa: u32 = 0x0;

    tx >>= 8 - bit_offset;

    if bit_offset != 0 && number_of_bits > 24 {
        tx |= h.byte(byte_offset.wrapping_sub(4) & byte_mask) << (24 + bit_offset);
    }

    // in place turn around
    for i in 0..16u32 {
        let bit_mask_r = 0x0000_0001u32 << i;
        let bit_mask_l = 0x8000_0000u32 >> i;

        txa |= (tx & bit_mask_r) << (31 - (i << 1));
        txa |= (tx & bit_mask_l) >> (31 - (i << 1));
    }

    // Shift by 32 for numberOfBits == 0: modulo 32 as in fdk_get().
    txa.wrapping_shr(32u32.wrapping_sub(number_of_bits)) as i32
}

/// `FDK_put(hBitBuffer, value, numberOfBits)`: writes `number_of_bits` (0..=32) bits of `value`.
pub fn fdk_put(h: &mut FdkBitBuf, value: u32, number_of_bits: u32) {
    if number_of_bits != 0 {
        let byte_offset0 = h.bit_ndx >> 3;
        let bit_offset = h.bit_ndx & 0x7;

        h.bit_ndx = h.bit_ndx.wrapping_add(number_of_bits) & h.buf_bits.wrapping_sub(1);
        h.valid_bits = h.valid_bits.wrapping_add(number_of_bits);

        let byte_mask = h.buf_size.wrapping_sub(1);

        let byte_offset1 = byte_offset0.wrapping_add(1) & byte_mask;
        let byte_offset2 = byte_offset0.wrapping_add(2) & byte_mask;
        let byte_offset3 = byte_offset0.wrapping_add(3) & byte_mask;

        // Create tmp containing free bits at the left border followed by bits to write, LSB's
        // are cleared, if available. Create mask to apply upon all buffer bytes.
        let tmp = value.wrapping_shl(32u32.wrapping_sub(number_of_bits)) >> bit_offset;
        let mask = !(bit_mask(number_of_bits).wrapping_shl(32u32.wrapping_sub(number_of_bits))
            >> bit_offset);

        // Where the five bytes are inside a power-of-two buffer, the masks change nothing: the
        // four bytes and the fifth (below) are updated at once.
        if h.buf_size.is_power_of_two() && byte_offset0 + 4 < h.buf_size {
            let (keep4, set4) = if bit_offset + number_of_bits > 32 {
                let bits = (bit_offset + number_of_bits) & 7;
                (!(bit_mask(bits) << (8 - bits)) as u8, (value << (8 - bits)) as u8)
            } else {
                (0xFF, 0)
            };
            let keep = ((mask as u64) << 8) | keep4 as u64;
            let set = ((tmp as u64) << 8) | set4 as u64;
            if h.mem.update(h.base as usize + byte_offset0 as usize, 5, keep, set) {
                return;
            }
        }

        // read all 4 bytes from buffer and create a 32-bit cache
        let mut cache = (h.byte(byte_offset0) << 24)
            | (h.byte(byte_offset1) << 16)
            | (h.byte(byte_offset2) << 8)
            | h.byte(byte_offset3);

        cache = (cache & mask) | tmp;
        h.set_byte(byte_offset0, (cache >> 24) as u8);
        h.set_byte(byte_offset1, (cache >> 16) as u8);
        h.set_byte(byte_offset2, (cache >> 8) as u8);
        h.set_byte(byte_offset3, cache as u8);

        if bit_offset + number_of_bits > 32 {
            let byte_offset4 = byte_offset0.wrapping_add(4) & byte_mask;
            // remaining bits: in range 1..7
            // replace MSBits of next byte in buffer by LSBits of "value"
            let bits = (bit_offset + number_of_bits) & 7;
            let mut cache = h.byte(byte_offset4) & !(bit_mask(bits) << (8 - bits));
            cache |= value << (8 - bits);
            h.set_byte(byte_offset4, cache as u8);
        }
    }
}

/// `FDK_putBwd(hBitBuffer, value, numberOfBits)`: writes `number_of_bits` bits of `value`
/// backwards.
pub fn fdk_put_bwd(h: &mut FdkBitBuf, mut value: u32, number_of_bits: u32) {
    let byte_offset = h.bit_ndx >> 3;
    let bit_offset = 7 - (h.bit_ndx & 0x07);
    let byte_mask = h.buf_size.wrapping_sub(1);

    let mask = !(bit_mask(number_of_bits) << bit_offset);
    let mut tmp: u32 = 0x0000;

    h.bit_ndx = h.bit_ndx.wrapping_sub(number_of_bits) & h.buf_bits.wrapping_sub(1);
    h.valid_bits = h.valid_bits.wrapping_sub(number_of_bits);

    // in place turn around
    for i in 0..16u32 {
        let bit_mask_r = 0x0000_0001u32 << i;
        let bit_mask_l = 0x8000_0000u32 >> i;

        tmp |= (value & bit_mask_r) << (31 - (i << 1));
        tmp |= (value & bit_mask_l) >> (31 - (i << 1));
    }
    value = tmp;
    // `value >> (32 - numberOfBits)`: shift by 32 for numberOfBits == 0, modulo 32 as in
    // fdk_get(). The bytes are then rewritten with an all-ones mask (a quirk of the C code).
    tmp = value.wrapping_shr(32u32.wrapping_sub(number_of_bits)) << bit_offset;

    let i0 = byte_offset & byte_mask;
    let i1 = byte_offset.wrapping_sub(1) & byte_mask;
    let i2 = byte_offset.wrapping_sub(2) & byte_mask;
    let i3 = byte_offset.wrapping_sub(3) & byte_mask;
    h.set_byte(i0, ((h.byte(i0) & mask) | (tmp as u8 as u32)) as u8);
    h.set_byte(i1, ((h.byte(i1) & (mask >> 8)) | ((tmp >> 8) as u8 as u32)) as u8);
    h.set_byte(i2, ((h.byte(i2) & (mask >> 16)) | ((tmp >> 16) as u8 as u32)) as u8);
    h.set_byte(i3, ((h.byte(i3) & (mask >> 24)) | ((tmp >> 24) as u8 as u32)) as u8);

    if bit_offset + number_of_bits > 32 {
        let i4 = byte_offset.wrapping_sub(4) & byte_mask;
        let v = (value.wrapping_shr(64u32.wrapping_sub(number_of_bits).wrapping_sub(bit_offset))
            as u8 as u32)
            | (h.byte(i4)
                & !(bit_mask(bit_offset).wrapping_shr(32u32.wrapping_sub(number_of_bits))));
        h.set_byte(i4, v as u8);
    }
}

/// `FDK_pushBack(hBitBuffer, numberOfBits, config)`: moves the position back.
#[inline]
pub fn fdk_push_back(h: &mut FdkBitBuf, number_of_bits: u32, config: u8) {
    h.valid_bits = if config == 0 {
        (h.valid_bits as i32).wrapping_add(number_of_bits as i32) as u32
    } else {
        (h.valid_bits as i32).wrapping_sub(number_of_bits as i32) as u32
    };
    h.bit_ndx = ((h.bit_ndx as i32).wrapping_sub(number_of_bits as i32) as u32)
        & h.buf_bits.wrapping_sub(1);
}

/// `FDK_pushForward(hBitBuffer, numberOfBits, config)`: moves the position forward.
#[inline]
pub fn fdk_push_forward(h: &mut FdkBitBuf, number_of_bits: u32, config: u8) {
    h.valid_bits = if config == 0 {
        (h.valid_bits as i32).wrapping_sub(number_of_bits as i32) as u32
    } else {
        (h.valid_bits as i32).wrapping_add(number_of_bits as i32) as u32
    };
    h.bit_ndx = ((h.bit_ndx as i32).wrapping_add(number_of_bits as i32) as u32)
        & h.buf_bits.wrapping_sub(1);
}

/// `FDK_getValidBits(hBitBuffer)`
#[inline]
pub fn fdk_get_valid_bits(h: &FdkBitBuf) -> u32 {
    h.valid_bits
}

/// `FDK_getFreeBits(hBitBuffer)`
#[inline]
pub fn fdk_get_free_bits(h: &FdkBitBuf) -> i32 {
    h.buf_bits.wrapping_sub(h.valid_bits) as i32
}

/// `FDK_Feed(hBitBuffer, inputBuffer, bufferSize, bytesValid)`: copies the last `*bytes_valid`
/// bytes of the first `buffer_size` bytes of `input_buffer` into the bit buffer (as much as
/// fits) and decrements `*bytes_valid` by the number of bytes copied.
///
/// `input_buffer` must hold at least `buffer_size` bytes and `*bytes_valid <= buffer_size`, as in
/// C. Otherwise the port reads 0 outside of the input.
pub fn fdk_feed(h: &mut FdkBitBuf, input_buffer: &[u8], buffer_size: u32, bytes_valid: &mut u32) {
    let mut pos = buffer_size.wrapping_sub(*bytes_valid) as usize;

    let mut b_total: u32 = 0;

    let free = (h.buf_bits as i32).wrapping_sub(h.valid_bits as i32).max(0) as u32;
    let mut b_to_read = h.buf_bits.min(free) >> 3;
    let mut no_of_bytes = b_to_read.min(*bytes_valid);

    while no_of_bytes > 0 {
        // split read to buffer size
        b_to_read = h.buf_size.wrapping_sub(h.read_offset);
        b_to_read = b_to_read.min(no_of_bytes);
        if b_to_read == 0 {
            // Only for an uninitialized bit buffer (`ReadOffset == bufSize == 0`): stop.
            break;
        }

        // copy 'bToRead' bytes from 'ptr' to inputbuffer
        let dst_end = h.read_offset as usize + b_to_read as usize;
        if dst_end <= h.buf_size as usize
            && pos.checked_add(b_to_read as usize).is_some_and(|e| e <= input_buffer.len())
        {
            // FDKmemcpy: every destination byte is inside the buffer and every source byte
            // inside the input, which is what the byte loop below checks per byte.
            let src = &input_buffer[pos..pos + b_to_read as usize];
            h.mem.write(h.base as usize + h.read_offset as usize, src);
        } else {
            for i in 0..b_to_read {
                let v = input_buffer.get(pos + i as usize).copied().unwrap_or(0);
                h.set_byte(h.read_offset.wrapping_add(i), v);
            }
        }

        // add noOfBits to number of valid bits in buffer
        h.valid_bits = (h.valid_bits as i32).wrapping_add((b_to_read << 3) as i32) as u32;
        b_total = b_total.wrapping_add(b_to_read);
        pos += b_to_read as usize;

        h.read_offset = h.read_offset.wrapping_add(b_to_read) & h.buf_size.wrapping_sub(1);
        no_of_bytes -= b_to_read;
    }

    *bytes_valid = bytes_valid.wrapping_sub(b_total);
}

/// `CopyAlignedBlock(h_BitBufSrc, dstBuffer, bToRead)`: copies `b_to_read` bytes from the byte
/// aligned read position of `h_bit_buf_src` to `dst_buffer` and advances the position.
///
/// `dst_buffer` must hold `b_to_read` bytes (bytes beyond its end are dropped).
pub fn copy_aligned_block(
    h_bit_buf_src: &mut FdkBitBuf,
    dst_buffer: &mut [u8],
    mut b_to_read: u32,
) {
    let byte_offset = h_bit_buf_src.bit_ndx >> 3;
    let byte_mask = h_bit_buf_src.buf_size.wrapping_sub(1);

    for i in 0..b_to_read {
        let v = h_bit_buf_src.byte(byte_offset.wrapping_add(i) & byte_mask) as u8;
        if let Some(d) = dst_buffer.get_mut(i as usize) {
            *d = v;
        }
    }

    b_to_read <<= 3;

    h_bit_buf_src.bit_ndx =
        h_bit_buf_src.bit_ndx.wrapping_add(b_to_read) & h_bit_buf_src.buf_bits.wrapping_sub(1);
    h_bit_buf_src.valid_bits = h_bit_buf_src.valid_bits.wrapping_sub(b_to_read);
}

/// `CopyAlignedBlock()` into the memory of another bit buffer, at `dst_offset`.
fn copy_aligned_block_to(
    src: &mut FdkBitBuf,
    dst: &mut FdkBitBuf,
    dst_offset: u32,
    b_to_read: u32,
) {
    let byte_offset = src.bit_ndx >> 3;
    let byte_mask = src.buf_size.wrapping_sub(1);

    for i in 0..b_to_read {
        let v = src.byte(byte_offset.wrapping_add(i) & byte_mask) as u8;
        dst.set_byte(dst_offset.wrapping_add(i), v);
    }

    let bits = b_to_read << 3;

    src.bit_ndx = src.bit_ndx.wrapping_add(bits) & src.buf_bits.wrapping_sub(1);
    src.valid_bits = src.valid_bits.wrapping_sub(bits);
}

/// `FDK_Copy(hBitBufDst, hBitBufSrc, bytesValid)`: copies bytes from `h_bit_buf_src` into
/// `h_bit_buf_dst`.
///
/// Quirk of the C code: the number of bytes is limited by the number of free *bits* of the
/// destination.
pub fn fdk_copy(
    h_bit_buf_dst: &mut FdkBitBuf,
    h_bit_buf_src: &mut FdkBitBuf,
    bytes_valid: &mut u32,
) {
    let mut b_total: i32 = 0;

    // limit noOfBytes to valid bytes in src buffer and available bytes in dst buffer
    let mut b_to_read = h_bit_buf_src.valid_bits >> 3;
    let mut no_of_bytes = b_to_read.min(*bytes_valid);
    b_to_read = fdk_get_free_bits(h_bit_buf_dst) as u32;
    no_of_bytes = b_to_read.min(no_of_bytes);

    while no_of_bytes > 0 {
        // Split Read to buffer size
        b_to_read = h_bit_buf_dst.buf_size.wrapping_sub(h_bit_buf_dst.read_offset);
        b_to_read = no_of_bytes.min(b_to_read);
        if b_to_read == 0 {
            // Only for an uninitialized destination: stop.
            break;
        }

        // copy 'bToRead' bytes from buffer to buffer
        if h_bit_buf_src.bit_ndx & 0x07 == 0 {
            let dst_offset = h_bit_buf_dst.read_offset;
            copy_aligned_block_to(h_bit_buf_src, h_bit_buf_dst, dst_offset, b_to_read);
        } else {
            for i in 0..b_to_read {
                let v = fdk_get(h_bit_buf_src, 8) as u8;
                let idx = h_bit_buf_dst.read_offset.wrapping_add(i);
                h_bit_buf_dst.set_byte(idx, v);
            }
        }

        // add noOfBits to number of valid bits in buffer
        h_bit_buf_dst.valid_bits = h_bit_buf_dst.valid_bits.wrapping_add(b_to_read << 3);
        b_total = b_total.wrapping_add(b_to_read as i32);

        h_bit_buf_dst.read_offset = h_bit_buf_dst.read_offset.wrapping_add(b_to_read)
            & h_bit_buf_dst.buf_size.wrapping_sub(1);
        no_of_bytes -= b_to_read;
    }

    *bytes_valid = bytes_valid.wrapping_sub(b_total as u32);
}

/// `FDK_Fetch(hBitBuffer, outBuf, writeBytes)`: copies up to `*write_bytes` valid bytes to
/// `out_buf` and sets `*write_bytes` to the number of bytes copied.
///
/// `out_buf` must hold `*write_bytes` bytes (bytes beyond its end are dropped).
pub fn fdk_fetch(h: &mut FdkBitBuf, out_buf: &mut [u8], write_bytes: &mut u32) {
    let mut pos = 0usize;
    let mut b_total: u32 = 0;

    let mut b_to_write = h.valid_bits >> 3;
    let mut no_of_bytes = b_to_write.min(*write_bytes);

    while no_of_bytes > 0 {
        // split write to buffer size
        b_to_write = h.buf_size.wrapping_sub(h.write_offset);
        b_to_write = b_to_write.min(no_of_bytes);
        if b_to_write == 0 {
            // Only for an uninitialized bit buffer: stop.
            break;
        }

        // copy 'bToWrite' bytes from bitbuffer to outputbuffer
        for i in 0..b_to_write {
            let v = h.byte(h.write_offset.wrapping_add(i)) as u8;
            if let Some(d) = out_buf.get_mut(pos + i as usize) {
                *d = v;
            }
        }

        // sub noOfBits from number of valid bits in buffer
        h.valid_bits = h.valid_bits.wrapping_sub(b_to_write << 3);
        b_total = b_total.wrapping_add(b_to_write);
        pos += b_to_write as usize;

        h.write_offset = h.write_offset.wrapping_add(b_to_write) & h.buf_size.wrapping_sub(1);
        no_of_bytes -= b_to_write;
    }

    *write_bytes = b_total;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A byte model of a memory: reads outside give 0, writes outside are dropped.
    struct Model(Vec<u8>);

    impl Model {
        fn get(&self, i: usize) -> u8 {
            self.0.get(i).copied().unwrap_or(0)
        }
        fn set(&mut self, i: usize, v: u8) {
            if let Some(b) = self.0.get_mut(i) {
                *b = v;
            }
        }
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// The bit buffer functions on a window at `base` of a larger memory equal those on a memory
    /// of its own, and leave the bytes around the window unchanged.
    #[test]
    fn window_at_base_equals_own_memory() {
        let mut r = Rng(0x5eed_ba5e);
        for _ in 0..2000 {
            let size = 1u32 << (r.next() % 8 + 1);
            let (base, tail) = ((r.next() % 40) as usize, (r.next() % 9) as usize);
            let end = base + size as usize;
            let around: Vec<u8> = (0..end + tail).map(|_| r.next() as u8).collect();
            let big = BitMem::from_slice(&around);
            let own = BitMem::from_slice(&around[base..end]);
            let (mut a, mut b) = (FdkBitBuf::default(), FdkBitBuf::default());
            fdk_init_bit_buffer_at(&mut a, &big, base as u32, size, size * 8);
            fdk_init_bit_buffer(&mut b, &own, size, size * 8);
            for _ in 0..50 {
                let pos = (r.next() % (size as u64 * 8)) as u32;
                (a.bit_ndx, b.bit_ndx) = (pos, pos);
                let n = (r.next() % 33) as u32;
                match r.next() % 4 {
                    0 => assert_eq!(fdk_get(&mut a, n), fdk_get(&mut b, n)),
                    1 => assert_eq!(fdk_get32(&mut a), fdk_get32(&mut b)),
                    2 => assert_eq!(fdk_peek32(&a), fdk_peek32(&b)),
                    _ => {
                        let v = r.next() as u32;
                        fdk_put(&mut a, v, n);
                        fdk_put(&mut b, v, n);
                    }
                }
                assert_eq!((a.bit_ndx, a.valid_bits), (b.bit_ndx, b.valid_bits));
                let all = big.to_vec();
                assert_eq!(all[..base], around[..base]);
                assert_eq!(all[end..], around[end..]);
                assert_eq!(all[base..end], own.to_vec()[..]);
            }
        }
    }

    /// Every accessor of BitMem against the byte model, at all alignments, across and beyond the
    /// end, for memories of 0 to 40 bytes.
    #[test]
    fn bitmem_equals_byte_model() {
        let mut r = Rng(0x5eed_b17e);
        let starts = |len: usize| (0..len + 10).chain([usize::MAX - 3, usize::MAX]);
        for len in 0..=40usize {
            let init: Vec<u8> = (0..len).map(|_| r.next() as u8).collect();
            let m = BitMem::from_slice(&init);
            let mut model = Model(init.clone());
            assert_eq!(m.len(), len);
            assert_eq!(m.to_vec(), init);
            for i in starts(len) {
                assert_eq!(m.get(i), model.get(i));
                for n in 0..=9 {
                    let exp = (n >= 1 && n <= 8 && i.checked_add(n).is_some_and(|e| e <= len))
                        .then(|| (0..n).fold(0u64, |a, j| (a << 8) | model.get(i + j) as u64));
                    assert_eq!(m.get_be(i, n), exp, "get_be {len} {i} {n}");
                }
            }
            for _ in 0..300 {
                let i = match r.next() % 8 {
                    0 => usize::MAX - (r.next() % 9) as usize,
                    _ => (r.next() % (len as u64 + 12)) as usize,
                };
                let n = (r.next() % 20) as usize;
                let data: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
                let op = r.next() % 5;
                match op {
                    0 => {
                        m.set(i, data.first().copied().unwrap_or(0x5a));
                        model.set(i, data.first().copied().unwrap_or(0x5a));
                    }
                    1 => {
                        m.write(i, &data);
                        for (j, &v) in data.iter().enumerate() {
                            if let Some(k) = i.checked_add(j) {
                                model.set(k, v);
                            }
                        }
                    }
                    2 => {
                        let v = r.next() as u8;
                        m.fill(i, n, v);
                        for k in i..i.saturating_add(n) {
                            model.set(k, v);
                        }
                    }
                    3 => {
                        let w = (n % 10).min(9);
                        let (keep, set) = (r.next(), r.next());
                        let ok = w >= 1 && w <= 8 && i.checked_add(w).is_some_and(|e| e <= len);
                        assert_eq!(m.update(i, w, keep, set), ok);
                        if ok {
                            for j in 0..w {
                                let sh = 8 * (w - 1 - j);
                                let (kb, sb) = ((keep >> sh) as u8, (set >> sh) as u8);
                                model.set(i + j, (model.get(i + j) & kb) | sb);
                            }
                        }
                    }
                    _ => {
                        let src = (r.next() % (len as u64 + 4)) as usize;
                        m.copy_within(src, i, n);
                        let tmp: Vec<u8> = (0..n).map(|j| model.get(src + j)).collect();
                        for (j, &v) in tmp.iter().enumerate() {
                            if let Some(k) = i.checked_add(j) {
                                model.set(k, v);
                            }
                        }
                    }
                }
                assert_eq!(m.to_vec(), model.0, "len {len} op {op} i {i} n {n}");
                let mut out = vec![0x77; n];
                m.read(i, &mut out);
                let exp: Vec<u8> =
                    (0..n).map(|j| i.checked_add(j).map_or(0, |k| model.get(k))).collect();
                assert_eq!(out, exp, "read {len} {i} {n}");
            }
            let c = m.deep_copy();
            assert!(!c.ptr_eq(&m) && m.clone().ptr_eq(&m));
            assert_eq!(c.to_vec(), m.to_vec());
        }
    }
}
