// Differential tests of fdk_aac_rs_arith::ac_arith_coder against the C++ reference.
//
// Every function is compared on random input: the bitstream (state array plus its own memory on
// the C++ side, as in the fdk bitstream tests), the decoder state, the context memory and the
// spectrum are compared after every call. `CArco_DecodeArithData()` is run over sequences of
// frames (long, short and LPD-like windows, block switching, resets, unusual lengths) on
// arithmetic-coded data produced by an encoder built from the decoder's tables, on corrupted
// and truncated data, and on random data.
//
// The C++ `CArcoData` lives inside a region of zeros (`PRE` bytes in front, `POST` bytes after),
// zeroed before every call; the port reads 0 outside the structure and drops the writes (see the
// module documentation of ac_arith_coder.rs).

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_arith::ac_arith_coder::*;
use fdk_aac_rs_cref::arith_ac_arith_coder as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, FdkBitStream, BS_READER};

/// The state array of the shims.
fn state(bs: &FdkBitStream) -> [u32; 9] {
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

/// A Rust bitstream and the state of its C++ counterpart, over separate copies of the same memory.
struct Bs {
    rs: FdkBitStream,
    rbuf: BitMem,
    cst: [u32; 9],
    cbuf: Vec<u8>,
}

impl Bs {
    /// A reader over `mem` (its length must be a power of two) with `valid_bits` valid bits.
    fn new(mem: Vec<u8>, valid_bits: u32) -> Bs {
        let size = mem.len() as u32;
        assert!(size.is_power_of_two());
        let mut b = Bs { rs: FdkBitStream::default(), rbuf: BitMem::from_slice(&mem), cst: [0; 9], cbuf: mem };
        fdk_init_bit_stream(&mut b.rs, &b.rbuf, size, valid_bits, BS_READER);
        b.cst = state(&b.rs);
        b
    }

    /// A reader over random memory with a random position, valid bits and cache.
    fn random(rng: &mut Rng) -> Bs {
        let size = 1usize << (2 + rng.next_u32() % 8);
        let mem: Vec<u8> = (0..size).map(|_| rng.next_u32() as u8).collect();
        let mut b = Bs::new(mem, 0);
        let bits = size as u32 * 8;
        let h = &mut b.rs.h_bit_buf;
        h.bit_ndx = rng.next_u32() % bits;
        h.valid_bits = match rng.next_u32() % 4 {
            0 => rng.next_u32(),
            1 => rng.next_u32() % 64,
            _ => rng.next_u32() % (bits + 1),
        };
        b.rs.cache_word = rng.next_u32();
        b.rs.bits_in_cache = rng.next_u32() % 32;
        b.cst = state(&b.rs);
        b
    }

    fn c(&mut self) -> (*mut u32, *mut u8) {
        (self.cst.as_mut_ptr(), self.cbuf.as_mut_ptr())
    }

    fn check(&self, what: &str) {
        assert_eq!(state(&self.rs), self.cst, "bitstream state after {what}");
        assert_eq!(self.rbuf.to_vec(), self.cbuf, "bitstream memory after {what}");
    }
}

#[test]
fn layout_create_destroy() {
    let size = unsafe { c::cref_ac_arith_coder_sizeof_CArcoData() };
    let ofs = unsafe { c::cref_ac_arith_coder_offsetof_c_prev() };
    assert_eq!(size as usize, core::mem::size_of::<CArcoData>());
    assert_eq!(ofs as usize, core::mem::offset_of!(CArcoData, c_prev));
    assert_eq!(size, 518);
    for _ in 0..16 {
        let p = unsafe { c::cref_ac_arith_coder_CArco_Create() };
        assert!(!p.is_null());
        // SAFETY: CArco_Create returns a CArcoData of `size` bytes.
        let bytes = unsafe { core::slice::from_raw_parts(p as *const u8, size as usize) };
        assert!(bytes.iter().all(|&b| b == 0), "CArco_Create clears the state");
        unsafe { c::cref_ac_arith_coder_CArco_Destroy(p) };
        let r = carco_create();
        assert_eq!(*r, CArcoData::default());
        assert_eq!(arco_bytes(&r), vec![0u8; 518]);
        carco_destroy(r);
    }
}

#[test]
fn mul_sbc_14bits_random() {
    let mut rng = Rng::new(0xA417_0001);
    for k in 0..200_000 {
        // The range of a decoder state (-0xFFFE..=0x10000) times any SHORT.
        let r = match k % 4 {
            0 => {
                [-0xFFFE, -1, 0, 1, 0x4000, 0x8000, 0xFFFF, 0x10000][(rng.next_u32() % 8) as usize]
            }
            _ => rng.range(-0xFFFE, 0x10000),
        };
        let cf = rng.i16() as i32;
        assert_eq!(mul_sbc_14bits(r, cf), unsafe { c::cref_ac_arith_coder_mul_sbc_14bits(r, cf) });
    }
}

#[test]
fn get_pk_v2_exhaustive() {
    // Every state decode2() can produce (21 bits: 16 bit context, bit 16 for the amplitude
    // flag, three bits of escape count), then random 32 bit values.
    for s in 0..(1u32 << 21) {
        assert_eq!(get_pk_v2(s), unsafe { c::cref_ac_arith_coder_get_pk_v2(s) }, "s = {s:#x}");
    }
    let mut rng = Rng::new(0xA417_0002);
    for _ in 0..1_000_000 {
        let s = rng.next_u32();
        assert_eq!(get_pk_v2(s), unsafe { c::cref_ac_arith_coder_get_pk_v2(s) }, "s = {s:#x}");
    }
    for s in [u32::MAX, 0xFFF0_0000, 0x0010_0000, 0x0010_0001, 0x000F_FFFF, 0x8000_0000] {
        assert_eq!(get_pk_v2(s), unsafe { c::cref_ac_arith_coder_get_pk_v2(s) }, "s = {s:#x}");
    }
}

/// A random cumulative frequency table of `len` entries (non-increasing, 14 bits).
fn random_cfreq(rng: &mut Rng, len: usize) -> Vec<i16> {
    let mut v: Vec<i16> = (0..len).map(|_| (rng.next_u32() % 16385) as i16).collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    if rng.next_u32() % 2 == 0 {
        *v.last_mut().unwrap() = 0;
    }
    v
}

#[test]
fn ari_decode_14bits_random() {
    let mut rng = Rng::new(0xA417_0003);
    for k in 0..200_000 {
        let (table, cfl): (Vec<i16>, i32) = match rng.next_u32() % 8 {
            0..=2 => (ARI_PK[(rng.next_u32() % 64) as usize].to_vec(), 17),
            3..=4 => (ARI_LSB2[(rng.next_u32() % 3) as usize].to_vec(), 4),
            5 => (random_cfreq(&mut rng, 2), 2),
            6 => (random_cfreq(&mut rng, 27), 27),
            _ => {
                let cfl = [0, 1, 3, 5, 16, 18, -1][(rng.next_u32() % 7) as usize];
                (random_cfreq(&mut rng, 4), cfl)
            }
        };
        // Decoder states: mostly the states decode2() produces (16 bit values, high - low at
        // least 0x4000), sometimes any 16 bit values, including low > high.
        let mut st = if k % 4 == 0 {
            Tastat {
                low: (rng.next_u32() & 0xFFFF) as i32,
                high: (rng.next_u32() & 0xFFFF) as i32,
                vobf: (rng.next_u32() & 0xFFFF) as i32,
            }
        } else {
            let low = (rng.next_u32() % 0xC000) as i32;
            let high = rng.range(low + 0x3FFF, 0xFFFF);
            let vobf =
                if k % 3 == 0 { rng.range(low, high) } else { (rng.next_u32() & 0xFFFF) as i32 };
            Tastat { low, high, vobf }
        };
        let mut b = Bs::random(&mut rng);
        let mut cs = [st.low, st.high, st.vobf];
        let before = st;
        let r = ari_decode_14bits(&mut b.rs, &mut st, &table, cfl);
        let (s, buf) = b.c();
        let e = unsafe {
            c::cref_ac_arith_coder_ari_decode_14bits(s, buf, cs.as_mut_ptr(), table.as_ptr(), cfl)
        };
        let what = format!("ari_decode_14bits({before:?}, {table:?}, {cfl})");
        assert_eq!(r, e, "{what}");
        assert_eq!([st.low, st.high, st.vobf], cs, "{what}");
        b.check(&what);
    }
}

#[test]
fn copy_table_amrwb_arith2_random() {
    const PRE: usize = 1 << 16;
    const POST: usize = 1 << 16;
    let mut rng = Rng::new(0xA417_0004);
    let sizes = [512, 384, 256, 192, 128, 96, 64, 48, 32, 0, 1, 2, 513, 514, 1024];
    for k in 0..3000 {
        let (size_in, size_out) = match k % 4 {
            0 => (
                sizes[(rng.next_u32() as usize) % sizes.len()],
                sizes[(rng.next_u32() as usize) % sizes.len()],
            ),
            1 => (rng.range(-16384, 16383), rng.range(-16384, 30000)),
            2 => {
                let s = rng.range(-64, 1100);
                (s, s)
            }
            _ => (rng.range(-40, 600), rng.range(-40, 600)),
        };
        let len = PRE + 518 + POST;
        let mut mem: Vec<u8> = (0..len)
            .map(|_| if k % 2 == 0 { (rng.next_u32() % 16) as u8 } else { rng.next_u32() as u8 })
            .collect();
        let mut cmem = mem.clone();
        // `tab` is `c_prev`, at offset 2 of the structure.
        copy_table_amrwb_arith2(&mut mem, PRE + 2, size_in, size_out);
        unsafe {
            c::cref_ac_arith_coder_copyTableAmrwbArith2(
                cmem.as_mut_ptr().add(PRE + 2),
                size_in,
                size_out,
            )
        };
        assert!(mem == cmem, "copyTableAmrwbArith2({size_in}, {size_out})");
    }
}

/// A random context of `len` entries: 4 bit values as decode2() writes them, or any byte.
fn random_context(rng: &mut Rng, len: usize) -> Vec<u8> {
    let any = rng.next_u32() % 4 == 0;
    (0..len).map(|_| if any { rng.next_u32() as u8 } else { (rng.next_u32() % 16) as u8 }).collect()
}

#[test]
fn decode2_random() {
    let mut rng = Rng::new(0xA417_0005);
    for k in 0..4000 {
        let nt = match k % 4 {
            0 => [512, 384, 256, 192, 128, 96, 64, 48][(rng.next_u32() % 8) as usize],
            _ => rng.range(0, 520),
        };
        let n = if k % 8 == 0 { nt } else { rng.range(0, nt) };
        let ctx = random_context(&mut rng, nt as usize + 4);
        let spec: Vec<i32> =
            (0..2 * n as usize + 8).map(|_| if k % 2 == 0 { 0 } else { rng.i32() }).collect();

        // Valid data for this context (with errors sometimes), or random data.
        let mut tab = ctx.clone();
        let q = random_spectrum(&mut rng, n as usize);
        let stop = random_stop(&mut rng, n as usize);
        let mut w = BitWriter::default();
        encode_window(&mut tab, &q, n as usize, nt as usize, stop, &mut w);
        let mem = match k % 3 {
            0 => random_memory(&mut rng, 64),
            1 => stream_memory(&mut rng, &w, 0),
            _ => {
                let flips = 1 + rng.next_u32() % 4;
                stream_memory(&mut rng, &w, flips)
            }
        };
        let valid = (mem.len() * 8) as u32;
        let mut b = Bs::new(mem, valid);

        let (mut rctx, mut cctx) = (ctx.clone(), ctx.clone());
        let (mut rspec, mut cspec) = (spec.clone(), spec);
        let r = decode2(&mut b.rs, &mut rctx, &mut rspec, n, nt);
        let (s, buf) = b.c();
        let e = unsafe {
            c::cref_ac_arith_coder_decode2(s, buf, cctx.as_mut_ptr(), cspec.as_mut_ptr(), n, nt)
        };
        let what = format!("decode2(n = {n}, nt = {nt}), case {k}");
        assert_eq!(r, e, "{what}");
        assert_eq!(rctx, cctx, "context after {what}");
        assert_eq!(rspec, cspec, "spectrum after {what}");
        b.check(&what);
        if k % 3 == 1 {
            assert_eq!(r, ARITH_CODER_OK, "{what}: valid data");
            assert_eq!(rctx[..nt as usize], tab[..nt as usize], "{what}: context of the encoder");
        }
    }
}

/// The optimized `decode2()` against the straight port `decode2_straight()`: valid,
/// corrupted and random data, contexts with any byte (also values decode2() never writes),
/// `n > nt`, random bitstream positions and caches, prefilled spectra. Runs with every variant
/// (no C code).
#[test]
fn decode2_fast_matches_straight() {
    let mut rng = Rng::new(0xA417_0F57);
    for k in 0..20000 {
        let nt = match k % 4 {
            0 => [512, 384, 256, 192, 128, 96, 64, 48][(rng.next_u32() % 8) as usize],
            _ => rng.range(0, 520),
        };
        let n = match k % 16 {
            0 => nt,
            1 => nt + rng.range(1, 4),
            _ => rng.range(0, nt),
        };
        let ctx = random_context(&mut rng, n.max(nt) as usize + 4);
        let spec: Vec<i32> =
            (0..2 * n as usize + 8).map(|_| if k % 2 == 0 { 0 } else { rng.i32() }).collect();
        let mut tab = ctx.clone();
        let q = random_spectrum(&mut rng, n as usize);
        let stop = random_stop(&mut rng, n as usize);
        let mut w = BitWriter::default();
        // (the encoder of the test needs n <= nt; n > nt decodes random data)
        if n <= nt {
            encode_window(&mut tab, &q, n as usize, nt as usize, stop, &mut w);
        }
        let mem = match k % 3 {
            _ if n > nt => random_memory(&mut rng, 64),
            0 => random_memory(&mut rng, 64),
            1 => stream_memory(&mut rng, &w, 0),
            _ => {
                let flips = 1 + rng.next_u32() % 4;
                stream_memory(&mut rng, &w, flips)
            }
        };
        let mut a = if k % 5 == 0 {
            Bs::random(&mut rng)
        } else {
            let valid = (mem.len() * 8) as u32;
            Bs::new(mem, valid)
        };
        let mut b = Bs::new(a.rbuf.to_vec(), 0);
        b.rs.h_bit_buf.bit_ndx = a.rs.h_bit_buf.bit_ndx;
        b.rs.h_bit_buf.valid_bits = a.rs.h_bit_buf.valid_bits;
        b.rs.cache_word = a.rs.cache_word;
        b.rs.bits_in_cache = a.rs.bits_in_cache;
        assert_eq!(state(&a.rs), state(&b.rs));

        let (mut actx, mut bctx) = (ctx.clone(), ctx);
        let (mut aspec, mut bspec) = (spec.clone(), spec);
        let ra = decode2(&mut a.rs, &mut actx, &mut aspec, n, nt);
        let rb = decode2_straight(&mut b.rs, &mut bctx, &mut bspec, n, nt);
        let what = format!("decode2(n = {n}, nt = {nt}), case {k}");
        assert_eq!(ra, rb, "{what}");
        assert_eq!(actx, bctx, "context after {what}");
        assert_eq!(aspec, bspec, "spectrum after {what}");
        assert_eq!(state(&a.rs), state(&b.rs), "bitstream after {what}");
    }
}

// --- Encoder -------------------------------------------------------------------------------

#[derive(Default)]
struct BitWriter {
    bits: Vec<u8>,
}

impl BitWriter {
    fn put(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.bits.push(((v >> i) & 1) as u8);
        }
    }
}

/// The arithmetic encoder that matches `ari_decode_14bits()`.
struct ArithEncoder {
    low: i32,
    high: i32,
    follow: u32,
}

impl ArithEncoder {
    fn new() -> Self {
        ArithEncoder { low: 0, high: 0xFFFF, follow: 0 }
    }

    fn emit(&mut self, w: &mut BitWriter, bit: u32) {
        w.put(bit, 1);
        for _ in 0..self.follow {
            w.put(bit ^ 1, 1);
        }
        self.follow = 0;
    }

    fn encode(&mut self, w: &mut BitWriter, symbol: usize, c_freq: &[i16]) {
        let range = self.high - self.low + 1;
        if symbol > 0 {
            self.high = self.low + ((range * c_freq[symbol - 1] as i32) >> 14) - 1;
        }
        self.low += (range * c_freq[symbol] as i32) >> 14;
        loop {
            if self.high < 0x8000 {
                self.emit(w, 0);
            } else if self.low >= 0x8000 {
                self.emit(w, 1);
                self.low -= 0x8000;
                self.high -= 0x8000;
            } else if self.low >= 0x4000 && self.high < 0xC000 {
                self.follow += 1;
                self.low -= 0x4000;
                self.high -= 0x4000;
            } else {
                break;
            }
            self.low *= 2;
            self.high = 2 * self.high + 1;
        }
    }

    /// Two bits (plus the pending ones) that select a value inside the final interval: the
    /// decoder reads 16 bits ahead and pushes 14 of them back.
    fn finish(&mut self, w: &mut BitWriter) {
        self.follow += 1;
        let bit = if self.low < 0x4000 { 0 } else { 1 };
        self.emit(w, bit);
    }
}

/// Encodes `n` 2-tuples of `q` as `decode2()` decodes them, with the context `tab`
/// (`c_prev + 2`), and updates the context as the decoder does. `stop`: index of a stop symbol.
fn encode_window(
    tab: &mut [u8],
    q: &[i32],
    n: usize,
    nt: usize,
    stop: Option<usize>,
    w: &mut BitWriter,
) {
    let mut enc = ArithEncoder::new();
    let mut state_inc = ((tab[0] as i32) << 12) as u16;
    let (mut c_1, mut c_2, mut c_3) = (0i32, 0i32, 0i32);
    let mut end = n;
    for i in 0..n {
        let mut s = (state_inc >> 8) as u32;
        s += (tab[i + 1] as u32) << 8;
        s = (s << 4) + c_1 as u32;
        state_inc = s as u16;
        if i > 3 && c_1 + c_2 + c_3 < 5 {
            s += 0x10000;
        }
        let pk = |esc: u32| &ARI_PK[get_pk_v2(s + (esc.min(7) << 17)) as usize];
        if stop == Some(i) {
            enc.encode(w, VAL_ESC as usize, pk(0));
            enc.encode(w, 0, pk(1));
            end = i;
            break;
        }
        let (a, b) = (q[2 * i].unsigned_abs(), q[2 * i + 1].unsigned_abs());
        let mut lev = 0;
        while (a >> lev) > 3 || (b >> lev) > 3 {
            lev += 1;
        }
        for e in 0..lev {
            enc.encode(w, VAL_ESC as usize, pk(e));
        }
        let m = ((a >> lev) + ((b >> lev) << 2)) as usize;
        enc.encode(w, m, pk(lev));
        for l in (0..lev).rev() {
            let (pa, pb) = (a >> (l + 1), b >> (l + 1));
            let pidx = if pa == 0 {
                1
            } else if pb == 0 {
                0
            } else {
                2
            };
            let sym = ((a >> l) & 1) | (((b >> l) & 1) << 1);
            enc.encode(w, sym as usize, &ARI_LSB2[pidx]);
        }
        let c_0 = (a + b + 1).min(15) as i32;
        c_3 = c_2;
        c_2 = c_1;
        c_1 = c_0;
        tab[i] = c_0 as u8;
    }
    enc.finish(w);
    for i in 0..end {
        for v in [q[2 * i], q[2 * i + 1]] {
            if v != 0 {
                w.put((v > 0) as u32, 1);
            }
        }
    }
    for t in &mut tab[end..nt] {
        *t = 1;
    }
}

/// Random quantized spectrum of `n` 2-tuples: mostly small values, some large ones (up to the
/// 23 escapes the decoder accepts), sometimes all zero.
fn random_spectrum(rng: &mut Rng, n: usize) -> Vec<i32> {
    let style = rng.next_u32() % 6;
    let mut q = vec![0i32; 2 * n];
    if style == 0 {
        return q;
    }
    // Decaying amplitudes over frequency, as in audio spectra.
    let top = 1 + rng.next_u32() % (2 * n as u32).max(1);
    for (i, v) in q.iter_mut().enumerate() {
        if i as u32 >= top && style != 5 {
            break;
        }
        let mag = match style {
            1 => rng.next_u32() % 3,
            2 => rng.next_u32() % 9,
            3 => (rng.next_u32() % 64) >> (rng.next_u32() % 6),
            4 => {
                if rng.next_u32() % 64 == 0 {
                    rng.next_u32() % (1 << (1 + rng.next_u32() % 25))
                } else {
                    rng.next_u32() % 5
                }
            }
            _ => rng.next_u32() % (1 << (1 + rng.next_u32() % 25)),
        };
        *v = if rng.next_u32() % 2 == 0 { mag as i32 } else { -(mag as i32) };
    }
    q
}

fn random_stop(rng: &mut Rng, n: usize) -> Option<usize> {
    if n > 0 && rng.next_u32() % 3 == 0 {
        Some((rng.next_u32() as usize) % n)
    } else {
        None
    }
}

fn random_memory(rng: &mut Rng, min_bytes: usize) -> Vec<u8> {
    let size = min_bytes.next_power_of_two() << (rng.next_u32() % 3);
    (0..size).map(|_| rng.next_u32() as u8).collect()
}

/// The bits of `w` followed by random bits, in a power-of-two buffer; `flips` random bit errors
/// in the coded bits.
fn stream_memory(rng: &mut Rng, w: &BitWriter, flips: u32) -> Vec<u8> {
    let mut bits = w.bits.clone();
    for _ in 0..flips {
        if !bits.is_empty() {
            let i = (rng.next_u32() as usize) % bits.len();
            bits[i] ^= 1;
        }
    }
    let size = (bits.len() / 8 + 16).next_power_of_two();
    let mut mem: Vec<u8> = (0..size).map(|_| rng.next_u32() as u8).collect();
    for (i, &b) in bits.iter().enumerate() {
        let (byte, sh) = (i / 8, 7 - (i % 8));
        mem[byte] = (mem[byte] & !(1 << sh)) | (b << sh);
    }
    mem
}

// --- CArco_DecodeArithData -------------------------------------------------------------------

/// The memory image of a `CArcoData` (little endian, as both reference builds).
fn arco_bytes(a: &CArcoData) -> Vec<u8> {
    let mut v = a.m_number_lines_prev.to_le_bytes().to_vec();
    v.extend_from_slice(&a.c_prev);
    v
}

/// Zeroed memory in front of and after the C++ structure (see the file comment).
const PRE: usize = 1 << 16;
const POST: usize = 1 << 16;

/// The decoder state on both sides.
struct ArcoPair {
    rs: CArcoData,
    cmem: Vec<u8>,
}

impl ArcoPair {
    fn new(rs: CArcoData) -> Self {
        let mut cmem = vec![0u8; PRE + 518 + POST];
        cmem[PRE..PRE + 518].copy_from_slice(&arco_bytes(&rs));
        ArcoPair { rs, cmem }
    }

    fn check(&self, what: &str) {
        assert_eq!(arco_bytes(&self.rs), self.cmem[PRE..PRE + 518], "CArcoData after {what}");
    }

    /// Calls both sides and compares everything.
    fn decode(
        &mut self,
        b: &mut Bs,
        rng: &mut Rng,
        lg: i32,
        lg_max: i32,
        reset: i32,
        what: &str,
    ) -> (i32, Vec<i32>) {
        self.cmem[..PRE].fill(0);
        self.cmem[PRE + 518..].fill(0);
        let guard = 8;
        let len = lg_max.max(0) as usize + guard;
        let spec: Vec<i32> = (0..len).map(|_| rng.i32()).collect();
        let (mut rspec, mut cspec) = (spec.clone(), spec);
        let r = carco_decode_arith_data(&mut self.rs, &mut b.rs, &mut rspec, lg, lg_max, reset);
        let (s, buf) = b.c();
        let e = unsafe {
            c::cref_ac_arith_coder_CArco_DecodeArithData(
                s,
                buf,
                self.cmem.as_mut_ptr().add(PRE) as *mut core::ffi::c_void,
                cspec.as_mut_ptr(),
                lg,
                lg_max,
                reset,
            )
        };
        let what =
            format!("CArco_DecodeArithData(lg = {lg}, lg_max = {lg_max}, reset = {reset}), {what}");
        assert_eq!(r, e, "{what}");
        assert_eq!(rspec, cspec, "spectrum after {what}");
        self.check(&what);
        b.check(&what);
        (r, rspec)
    }
}

/// The context mapping of `CArco_DecodeArithData()` before `decode2()`, on a copy of the state,
/// for the encoder: the byte image of the structure, extended to `lg_max / 2 + 2` entries after
/// `c_prev + 2`. `None` if the call fails before decoding.
fn mapped_image(a: &CArcoData, lg: i32, lg_max: i32, reset: i32) -> Option<Vec<u8>> {
    if lg_max < lg || lg_max < 0 {
        return None;
    }
    let prev = a.m_number_lines_prev as i32;
    let nt = (lg_max >> 1) as usize;
    let mut mem = vec![0u8; 518.max(4 + nt + 2)];
    mem[..518].copy_from_slice(&arco_bytes(a));
    if reset != 0 {
        let end = (2 + (lg_max / 2 + 4) as usize).min(mem.len());
        mem[2..end].fill(0);
    } else if lg_max != prev {
        if prev == 0 {
            return None;
        }
        copy_table_amrwb_arith2(&mut mem, 2, prev >> 1, lg_max >> 1);
    }
    Some(mem)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Data {
    /// Arithmetic-coded data of the encoder.
    Valid,
    /// The same with random bit errors.
    Errors,
    /// The same, but with fewer valid bits than coded.
    Truncated,
    /// Random bits.
    Random,
}

/// One frame: the windows `(lg, lg_max, reset)` are decoded one after the other from one
/// bitstream, as `CBlock_ReadAcSpectralData()` does (it stops at the first error). Returns
/// (windows decoded, windows that reproduced the encoder's spectrum and context).
fn frame(
    p: &mut ArcoPair,
    rng: &mut Rng,
    windows: &[(i32, i32, i32)],
    data: Data,
    what: &str,
) -> (u32, u32) {
    // Encode all windows on a copy of the state; keep the expected spectrum and state.
    let mut w = BitWriter::default();
    let mut sim = p.rs.clone();
    let mut expected: Vec<(Vec<i32>, CArcoData)> = Vec::new();
    for &(lg, lg_max, reset) in windows {
        let Some(mut mem) = mapped_image(&sim, lg, lg_max, reset) else {
            break;
        };
        let nt = (lg_max >> 1) as usize;
        let mut spec = vec![0i32; lg_max as usize];
        if lg > 0 {
            let n = (lg >> 1) as usize;
            let q = random_spectrum(rng, n);
            let stop = random_stop(rng, n);
            encode_window(&mut mem[4..], &q, n, nt, stop, &mut w);
            let end = stop.unwrap_or(n);
            spec[..2 * end].copy_from_slice(&q[..2 * end]);
        } else {
            mem[4..4 + nt].fill(1);
        }
        sim.c_prev.copy_from_slice(&mem[2..518]);
        sim.m_number_lines_prev = lg_max as i16;
        expected.push((spec, sim.clone()));
    }

    let mem = match data {
        Data::Valid | Data::Truncated => stream_memory(rng, &w, 0),
        Data::Errors => {
            let flips = 1 + rng.next_u32() % 3;
            stream_memory(rng, &w, flips)
        }
        Data::Random => random_memory(rng, w.bits.len() / 8 + 4),
    };
    let size_bits = (mem.len() * 8) as u32;
    let valid_bits = match data {
        Data::Truncated => rng.next_u32() % (w.bits.len() as u32 + 1),
        _ => size_bits,
    };
    let mut b = Bs::new(mem, valid_bits);
    let (mut decoded, mut exact) = (0, 0);
    for (wi, &(lg, lg_max, reset)) in windows.iter().enumerate() {
        let (r, spec) =
            p.decode(&mut b, rng, lg, lg_max, reset, &format!("{what}, window {wi}, {data:?}"));
        decoded += 1;
        if data == Data::Valid {
            if let Some((es, ea)) = expected.get(wi) {
                if r == ARITH_CODER_OK && spec[..es.len()] == es[..] && p.rs == *ea {
                    exact += 1;
                }
            }
        }
        if r != ARITH_CODER_OK {
            break;
        }
    }
    (decoded, exact)
}

/// The windows of one frame of `frame_length`: long, eight short, LPD-like (TCX windows of a
/// quarter, half or the whole frame), or unusual lengths.
fn random_windows(rng: &mut Rng, frame_length: i32, independent: bool) -> Vec<(i32, i32, i32)> {
    // CBlock_ReadAcSpectralData: arith_reset_flag is 1 in independent frames, else read from the
    // bitstream; it applies to the first window only.
    let reset = if independent || rng.next_u32() % 8 == 0 { 1 } else { 0 };
    // lg: the end of the transmitted scale factor bands (any value up to lg_max here).
    let lg_of = |rng: &mut Rng, lg_max: i32| match rng.next_u32() % 4 {
        0 => lg_max,
        1 => rng.range(0, lg_max) & !1,
        2 => rng.range(0, lg_max),
        _ => rng.range(0, lg_max.min(32)),
    };
    match rng.next_u32() % 16 {
        0..=6 => vec![(lg_of(rng, frame_length), frame_length, reset)],
        7..=10 => {
            let lg_max = frame_length / 8;
            let lg = lg_of(rng, lg_max);
            (0..8).map(|win| (lg, lg_max, (reset != 0 && win == 0) as i32)).collect()
        }
        11..=13 => {
            // LPD: lg == lg_max.
            let mut v = Vec::new();
            let mut left = 4;
            let mut first = true;
            while left > 0 {
                let mut q = [1, 2, 4][(rng.next_u32() % 3) as usize];
                while q > left {
                    q /= 2;
                }
                let lg = frame_length / 4 * q;
                v.push((lg, lg, (first && reset != 0) as i32));
                first = false;
                left -= q;
            }
            v
        }
        _ => {
            // Unusual lengths: odd, above 1024, zero, lg > lg_max, negative lg.
            let n = 1 + rng.next_u32() % 3;
            (0..n)
                .map(|_| {
                    let lg_max = match rng.next_u32() % 4 {
                        0 => rng.range(0, 1100),
                        1 => rng.range(1020, 4096),
                        2 => rng.range(0, 8),
                        _ => [0, 1, 2, 3, 1023, 1025, 1026, 1027, 1028]
                            [(rng.next_u32() % 9) as usize],
                    };
                    let lg = match rng.next_u32() % 8 {
                        0 => lg_max + 1 + rng.range(0, 3),
                        1 => -rng.range(0, 5),
                        _ => lg_of(rng, lg_max),
                    };
                    (lg, lg_max, (rng.next_u32() % 4 == 0) as i32)
                })
                .collect()
        }
    }
}

/// A random initial state: a fresh one, a realistic one, or random bytes.
fn random_state(rng: &mut Rng) -> CArcoData {
    let mut a = CArcoData::default();
    match rng.next_u32() % 4 {
        0 => {}
        1 => {
            a.m_number_lines_prev = [1024, 128, 768, 96, 256, 512][(rng.next_u32() % 6) as usize];
            for v in a.c_prev.iter_mut() {
                *v = (rng.next_u32() % 16) as u8;
            }
        }
        2 => {
            a.m_number_lines_prev = rng.range(-8, 2100) as i16;
            for v in a.c_prev.iter_mut() {
                *v = (rng.next_u32() % 16) as u8;
            }
        }
        _ => {
            a.m_number_lines_prev = rng.i16();
            for v in a.c_prev.iter_mut() {
                *v = rng.next_u32() as u8;
            }
        }
    }
    a
}

fn run_sequences(seed: u64, sequences: u32, frames: u32) {
    let mut rng = Rng::new(seed);
    let (mut valid_windows, mut exact_windows, mut calls) = (0u32, 0u32, 0u32);
    for seq in 0..sequences {
        let frame_length = if rng.next_u32() % 4 == 0 { 768 } else { 1024 };
        let mut p = ArcoPair::new(random_state(&mut rng));
        for f in 0..frames {
            let independent = f == 0 || rng.next_u32() % 16 == 0;
            let windows = random_windows(&mut rng, frame_length, independent);
            let data = match rng.next_u32() % 16 {
                0..=9 => Data::Valid,
                10..=11 => Data::Errors,
                12..=13 => Data::Truncated,
                _ => Data::Random,
            };
            let what = format!("seed {seed:#x}, sequence {seq}, frame {f}");
            let (decoded, exact) = frame(&mut p, &mut rng, &windows, data, &what);
            calls += decoded;
            if data == Data::Valid {
                valid_windows += decoded;
                exact_windows += exact;
            }
        }
    }
    // Check the encoder: most valid windows decode to the encoded spectrum and context (the
    // others are windows with errors by design, e.g. lg > lg_max or a missing context).
    eprintln!("{calls} calls, {exact_windows} of {valid_windows} valid windows decoded exactly");
    assert!(exact_windows * 10 >= valid_windows * 8, "the test encoder does not work");
}

#[test]
fn carco_decode_arith_data_sequences_a() {
    run_sequences(0xA417_1001, 150, 40);
}

#[test]
fn carco_decode_arith_data_sequences_b() {
    run_sequences(0xA417_1002, 150, 40);
}

#[test]
fn carco_decode_arith_data_sequences_c() {
    run_sequences(0xA417_1003, 150, 40);
}

#[test]
fn carco_decode_arith_data_edge_cases() {
    let mut rng = Rng::new(0xA417_1004);
    // Every combination of a previous length and a new length from a set of edge values, with
    // and without reset, on valid, random and truncated data.
    // (Inputs without a defined reference result are not compared here; see
    // CArco_DecodeArithData.)
    let prevs = [-1, -2048, -32768, 32767];
    let lens = [
        0, 1, 2, 3, 7, 8, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1023, 1024, 1025, 1026, 1027,
        1028, 1029, 2048, 4095,
    ];
    let mut k = 0u32;
    for &prev in lens.iter().chain(&prevs) {
        for &lg_max in &lens {
            for reset in [0, 1] {
                for data in [Data::Valid, Data::Random, Data::Truncated] {
                    let mut a = random_state(&mut rng);
                    a.m_number_lines_prev = prev as i16;
                    let mut p = ArcoPair::new(a);
                    let lg = match k % 3 {
                        0 => lg_max,
                        1 => lg_max / 2,
                        _ => 0,
                    };
                    let what = format!("prev {prev}, edge case {k}");
                    frame(&mut p, &mut rng, &[(lg, lg_max, reset), (lg, lg_max, 0)], data, &what);
                    k += 1;
                }
            }
        }
    }
}

/// The file-static table `name` of ac_arith_coder.cpp (a copy compiled into the shim).
fn c_table<T: Copy>(name: &str) -> Vec<T> {
    let cname = std::ffi::CString::new(name).unwrap();
    let mut bytes = 0;
    let p = unsafe { c::cref_ac_arith_coder_table(cname.as_ptr(), &mut bytes) };
    assert!(!p.is_null(), "{name}");
    assert_eq!(bytes as usize % core::mem::size_of::<T>(), 0, "{name}");
    unsafe {
        core::slice::from_raw_parts(p as *const T, bytes as usize / core::mem::size_of::<T>())
    }
    .to_vec()
}

/// ARI_MERGED_HASH_PS (`ari_merged_hash_ps`, `ULONG` = 32 bits), ARI_PK (`ari_pk`) and ARI_LSB2
/// (`ari_lsb2`), element by element.
#[test]
fn tables() {
    assert_eq!(c_table::<u32>("ari_merged_hash_ps"), ARI_MERGED_HASH_PS.to_vec());
    assert_eq!(c_table::<i16>("ari_pk"), ARI_PK.iter().flatten().copied().collect::<Vec<_>>());
    assert_eq!(c_table::<i16>("ari_lsb2"), ARI_LSB2.iter().flatten().copied().collect::<Vec<_>>());
}
