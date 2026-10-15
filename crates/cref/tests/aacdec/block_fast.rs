// Tests of the optimized functions of fdk_aac_rs_aacdec::block against their straight
// ports, which keep their own differential tests against C (block.rs), and against C directly.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use crate::block::{random_codebooks, random_pulse, random_tns};
use crate::block_ctx::*;
use fdk_aac_rs_aacdec::block::*;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_aacdec::aac_rom::AAC_CODE_BOOK_DESCRIPTION_TABLE;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, fdk_push_back_cache, fdk_read_bit, fdk_read_bits, FdkBitStream, BS_READER};
use fdk_aac_rs_cref::aacdec_block as c;
use fdk_aac_rs_cref::testutil::Rng;

/// A value for an inverse quantization test: quantized values, the boundaries of the lookup
/// table and of `MAX_QUANTIZED_VALUE`, and arbitrary values (which the decoder rejects before,
/// but the functions must still agree on).
fn iq_value(rng: &mut Rng) -> i32 {
    let t = IQ_TABLE_SIZE as i32;
    match rng.next_u32() % 8 {
        0..=3 => random_quant(rng),
        4 => [t - 1, t, t + 1, 1 - t, -t, -t - 1, 8191, -8191, 8192, -8192][(rng.next_u32() % 10) as usize],
        5 => rng.range(-t - 4, t + 4),
        _ => random_spec(rng),
    }
}

#[test]
fn inverse_quantize_band_fast_vs_straight() {
    let mut rng = Rng::new(0xb10c_f001);
    // Every magnitude of the table and around it, for every lsb and a range of scales.
    for lsb in 0..4u32 {
        for scale in -40..40 {
            let mut v: Vec<i32> = (-(IQ_TABLE_SIZE as i32) - 8..IQ_TABLE_SIZE as i32 + 8).collect();
            let mut w = v.clone();
            let n = v.len() as i32;
            inverse_quantize_band(&mut v, lsb, n, scale);
            inverse_quantize_band_fast(&mut w, lsb, n, scale);
            assert_eq!(v, w, "lsb {lsb} scale {scale}");
        }
    }
    for _ in 0..200000 {
        let n = [0, 1, 4, 8, 12, 16, 32, 48, 64, 96][(rng.next_u32() % 10) as usize];
        let spec: Vec<i32> = (0..n).map(|_| iq_value(&mut rng)).collect();
        let lsb = if rng.next_u32() % 16 == 0 { rng.next_u32() % 8 } else { rng.next_u32() % 4 };
        let scale = match rng.next_u32() % 8 {
            // (The straight port computes `scale + 1` with overflow checks in `checked`.)
            0 => rng.i32().min(i32::MAX - 1),
            1 => rng.range(-100, 100),
            _ => rng.range(-40, 40),
        };
        // no_lines below, at or above the slice length.
        let no_lines = match rng.next_u32() % 8 {
            0 => n as i32 - rng.range(0, 4),
            1 => n as i32 + rng.range(0, 4),
            2 => -rng.range(0, 2),
            _ => n as i32,
        };
        let mut a = spec.clone();
        inverse_quantize_band(&mut a, lsb, no_lines, scale);
        let mut b = spec.clone();
        inverse_quantize_band_fast(&mut b, lsb, no_lines, scale);
        assert_eq!(a, b, "{spec:?} lsb {lsb} no_lines {no_lines} scale {scale}");
        assert_eq!(maxabs_d(&spec, no_lines), maxabs_d_fast(&spec[..(no_lines.max(0) as usize).min(spec.len())]));
    }
    // Bands of table magnitudes only (the SIMD kernels of every group), any length, with
    // occasional values outside the table, scales around the limits of the SIMD range.
    for _ in 0..200000 {
        let n = rng.range(0, 100) as usize;
        let spec: Vec<i32> = (0..n)
            .map(|_| match rng.next_u32() % 16 {
                0..=5 => 0,
                6..=9 => rng.range(-3, 3),
                10..=14 => rng.range(1 - IQ_TABLE_SIZE as i32, IQ_TABLE_SIZE as i32 - 1),
                _ if rng.next_u32() % 8 == 0 => iq_value(&mut rng),
                _ => rng.range(-20, 20),
            })
            .collect();
        let lsb = rng.next_u32() % 4;
        let scale = rng.range(-50, 50);
        let mut a = spec.clone();
        inverse_quantize_band(&mut a, lsb, n as i32, scale);
        let mut b = spec.clone();
        inverse_quantize_band_fast(&mut b, lsb, n as i32, scale);
        assert_eq!(a, b, "{spec:?} lsb {lsb} scale {scale}");
    }
}

#[test]
fn inverse_quantize_band_fast_vs_c() {
    let mut rng = Rng::new(0xb10c_f002);
    for _ in 0..50000 {
        let n = [4, 8, 12, 16, 32, 48, 64, 96][(rng.next_u32() % 8) as usize];
        let spec: Vec<i32> = (0..n).map(|_| iq_value(&mut rng).clamp(-8191, 8191)).collect();
        let lsb = rng.next_u32() % 4;
        let scale = rng.range(-40, 40);
        let mut cs = spec.clone();
        unsafe { c::cref_block_inverse_quantize_band(cs.as_mut_ptr(), lsb, n as i32, scale) };
        let mut rs = spec.clone();
        inverse_quantize_band_fast(&mut rs, lsb, n as i32, scale);
        assert_eq!(rs, cs, "InverseQuantizeBand(lsb {lsb}, scale {scale})");
        let all: Vec<i32> = (0..n).map(|_| random_spec(&mut rng)).collect();
        assert_eq!(maxabs_d_fast(&all), unsafe { c::cref_block_maxabs_d(all.as_ptr(), n as i32) });
    }
}

/// The complete state of a bitstream (the buffer pointers differ between contexts).
fn bs_state(bs: &FdkBitStream) -> (u32, u32, u32, u32, u32, u32) {
    let b = &bs.h_bit_buf;
    (bs.cache_word, bs.bits_in_cache, b.valid_bits, b.read_offset, b.write_offset, b.bit_ndx)
}

/// Moves the bitstream to a random position with a random cache state, as the element parsing
/// before the spectral data leaves it (reads of random lengths, sometimes a pushed back bit).
fn random_bs_position(rng: &mut Rng, bs: &mut FdkBitStream) {
    for _ in 0..rng.next_u32() % 6 {
        let n = rng.next_u32() % 33;
        fdk_read_bits(bs, n);
        if n > 0 && rng.next_u32() % 4 == 0 && bs.bits_in_cache < 32 {
            fdk_push_back_cache(bs, 1);
        }
    }
}

/// A copy of the Rust context `r` (state image, sampling rate info and bitstream with its data).
fn clone_ctx(r: &mut RCtx, data: &[u8], buf_bytes: usize, valid_bits: u32) -> RCtx {
    let mut q = RCtx::new();
    q.usac = r.usac;
    let im = r.dump();
    q.load(&im.v);
    q.sri = r.sri;
    q.set_bs(data, buf_bytes, valid_bits);
    q
}

/// CBlock_ReadSpectralData(): the fast version against the straight one on random states,
/// random bitstreams at random positions (small ring buffers that wrap around, little valid
/// data) and inconsistent band counts, comparing the complete state including the bitstream.
#[test]
fn read_spectral_data_fast_vs_straight() {
    let mut rng = Rng::new(0xb10c_f003);
    let mut p = Pair::new();
    for it in 0..6000 {
        let spf = random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let vcb11 = rng.next_u32() % 2 == 0;
        random_codebooks(&mut rng, &mut p.r.ci[ch], vcb11);
        let sri = p.r.sri;
        random_pulse(&mut rng, &mut p.r.ci[ch], &sri, spf);
        if rng.next_u32() % 8 == 0 {
            // more bands than the band table has (inconsistent state: writes outside the
            // spectrum are dropped, reads give 0)
            p.r.ci[ch].ics_info.max_sf_bands = rng.range(0, 63) as u8;
        }
        let flags = [0, AC_ER_VCB11, AC_ELD, AC_SCALABLE, 0][(rng.next_u32() % 5) as usize];
        let buf_bytes = [4096, 4096, 64, 256, 2048][(rng.next_u32() % 5) as usize];
        let data = random_bytes(&mut rng, buf_bytes);
        let valid = if rng.next_u32() % 4 == 0 { rng.next_u32() % (buf_bytes as u32 * 8) } else { buf_bytes as u32 * 8 };
        p.r.set_bs(&data, buf_bytes, valid);
        let mut q = clone_ctx(&mut p.r, &data, buf_bytes, valid);
        let seed = rng.next_u64();
        random_bs_position(&mut Rng::new(seed), &mut p.r.bs);
        random_bs_position(&mut Rng::new(seed), &mut q.bs);
        let ra = {
            let (spec, com) = (&mut p.r.spec, &mut p.r.com);
            cblock_read_spectral_data(&mut p.r.bs, &mut p.r.ci[ch], &sri, flags, &mut spec[ch * 1024..], com)
        };
        let rb = {
            let (spec, com) = (&mut q.spec, &mut q.com);
            cblock_read_spectral_data_fast(&mut q.bs, &mut q.ci[ch], &sri, flags, &mut spec[ch * 1024..], com)
        };
        assert_eq!(ra, rb, "{it}");
        assert_eq!(bs_state(&p.r.bs), bs_state(&q.bs), "{it}: bitstream");
        assert_eq!(p.r.dump().v, q.dump().v, "{it}: state");
    }
}

/// CBlock_ReadSpectralData(): the fast version against C (as `block::read_spectral_data`).
#[test]
fn read_spectral_data_fast_vs_c() {
    let mut rng = Rng::new(0xb10c_f004);
    let mut p = Pair::new();
    for it in 0..4000 {
        let spf = random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let vcb11 = rng.next_u32() % 2 == 0;
        random_codebooks(&mut rng, &mut p.r.ci[ch], vcb11);
        let sri = p.r.sri;
        random_pulse(&mut rng, &mut p.r.ci[ch], &sri, spf);
        let flags = [0, AC_ER_VCB11, AC_ELD, AC_SCALABLE, 0][(rng.next_u32() % 5) as usize];
        let data = random_bytes(&mut rng, 4096);
        p.set_bs(&data, 4096, 4096 * 8);
        p.sync();
        let ce = unsafe { c::cref_block_read_spectral_data(p.c.0, ch as i32, flags) };
        let (spec, com) = (&mut p.r.spec, &mut p.r.com);
        let re = cblock_read_spectral_data_fast(&mut p.r.bs, &mut p.r.ci[ch], &sri, flags, &mut spec[ch * 1024..], com);
        assert_eq!(re, ce, "CBlock_ReadSpectralData {it}");
        p.check(&format!("CBlock_ReadSpectralData {it} flags {flags:#x}"));
        p.check_bs(&format!("CBlock_ReadSpectralData {it} flags {flags:#x}"));
    }
}

/// CBlock_DecodeHuffmanWordCB(): the table version against the straight walk, for every
/// spectral codebook, on random bitstreams at random positions and cache states (including a
/// full cache after a pushed back bit), small ring buffers that wrap around and runs of 0x00
/// and 0xFF (the longest codewords); value and complete bitstream state after each word.
#[test]
fn decode_huffman_word_lut_vs_straight() {
    let mut rng = Rng::new(0xb10c_f005);
    for it in 0..20000 {
        let buf_bytes = [16usize, 64, 256, 4096][(rng.next_u32() % 4) as usize];
        let a = random_bytes(&mut rng, buf_bytes);
        let b = a.clone();
        let valid = if rng.next_u32() % 4 == 0 { rng.next_u32() % (buf_bytes as u32 * 8) } else { buf_bytes as u32 * 8 };
        let (mut x, mut y) = (FdkBitStream::default(), FdkBitStream::default());
        fdk_init_bit_stream(&mut x, &BitMem::from_slice(&a), buf_bytes as u32, valid, BS_READER);
        fdk_init_bit_stream(&mut y, &BitMem::from_slice(&b), buf_bytes as u32, valid, BS_READER);
        let seed = rng.next_u64();
        random_bs_position(&mut Rng::new(seed), &mut x);
        random_bs_position(&mut Rng::new(seed), &mut y);
        let cb = 1 + (rng.next_u32() % 11) as usize;
        let hcb = &AAC_CODE_BOOK_DESCRIPTION_TABLE[cb];
        for w in 0..64 {
            let r1 = cblock_decode_huffman_word_cb(&mut x, hcb.code_book);
            let r2 = cblock_decode_huffman_word_lut(&mut y, hcb.code_book, &HUFF_LUT[cb]);
            assert_eq!(r1, r2, "{it} word {w} cb {cb}");
            assert_eq!(bs_state(&x), bs_state(&y), "{it} word {w} cb {cb}: bitstream");
        }
    }
}

/// One codeword of codebook `cb` as the straight CBlock_ReadSpectralData() reads it: the walk,
/// then per value (`(idx & mask) - offset`) its sign bit if the codebook is unsigned and the
/// value nonzero.
fn straight_word_signs(bs: &mut FdkBitStream, cb: usize) -> [i32; 4] {
    let hcb = &AAC_CODE_BOOK_DESCRIPTION_TABLE[cb];
    let (bits, offset) = (hcb.num_bits as i32, hcb.offset as i32);
    let mut idx = cblock_decode_huffman_word_cb(bs, hcb.code_book);
    let mut v = [0; 4];
    for x in v.iter_mut().take(hcb.dimension as usize) {
        let mut t = (idx & ((1 << bits) - 1)) - offset;
        if offset == 0 && t != 0 && fdk_read_bit(bs) != 0 {
            t = -t;
        }
        *x = t;
        idx >>= bits;
    }
    v
}

/// [`decode_huffman_word_signs`] for codebook `cb` (the parameters of the decoder's dispatch).
fn fast_word_signs(bs: &mut FdkBitStream, cb: usize) -> [i32; 4] {
    let hcb = &AAC_CODE_BOOK_DESCRIPTION_TABLE[cb];
    let lut = &HUFF_SIGN_LUT[cb];
    let cbk = hcb.code_book;
    let mut v = [0; 4];
    match (hcb.dimension, hcb.num_bits, hcb.offset) {
        (4, 2, 1) => v = decode_huffman_word_signs::<4, 2, 1>(bs, cbk, lut),
        (4, 2, 0) => v = decode_huffman_word_signs::<4, 2, 0>(bs, cbk, lut),
        (2, 4, 4) => v[..2].copy_from_slice(&decode_huffman_word_signs::<2, 4, 4>(bs, cbk, lut)),
        (2, 4, 0) => v[..2].copy_from_slice(&decode_huffman_word_signs::<2, 4, 0>(bs, cbk, lut)),
        (2, 5, 0) => v[..2].copy_from_slice(&decode_huffman_word_signs::<2, 5, 0>(bs, cbk, lut)),
        p => panic!("codebook {cb}: parameters {p:?}"),
    }
    v
}

/// The codeword and sign table decoding against the straight walk and sign reads, for
/// every spectral codebook, on random bitstreams at every cache state (0..=32 bits, also a full
/// cache after a pushed back bit), small ring buffers that wrap around, little valid data and
/// runs of 0x00 and 0xFF; values and complete bitstream state after each word. Also counts the
/// words where the straight walk refills and pushes the last bit back into the full cache.
#[test]
fn decode_huffman_word_signs_vs_straight() {
    let mut rng = Rng::new(0xb10c_f00a);
    let mut edge = 0;
    for it in 0..40000 {
        let buf_bytes = [16usize, 64, 256, 4096][(rng.next_u32() % 4) as usize];
        let mut a = random_bytes(&mut rng, buf_bytes);
        if rng.next_u32() % 4 == 0 {
            let fill = [0x00, 0xFF, 0x55, 0xAA][(rng.next_u32() % 4) as usize];
            let start = (rng.next_u32() as usize) % buf_bytes;
            let len = (rng.next_u32() as usize) % 16;
            for b in a.iter_mut().skip(start).take(len) {
                *b = fill;
            }
        }
        let b = a.clone();
        let valid = if rng.next_u32() % 4 == 0 { rng.next_u32() % (buf_bytes as u32 * 8) } else { buf_bytes as u32 * 8 };
        let (mut x, mut y) = (FdkBitStream::default(), FdkBitStream::default());
        fdk_init_bit_stream(&mut x, &BitMem::from_slice(&a), buf_bytes as u32, valid, BS_READER);
        fdk_init_bit_stream(&mut y, &BitMem::from_slice(&b), buf_bytes as u32, valid, BS_READER);
        // every cache state: one read of 0..=32 bits, sometimes a pushed back bit
        let n = it as u32 % 33;
        for bs in [&mut x, &mut y] {
            fdk_read_bits(bs, 32);
            fdk_read_bits(bs, n);
        }
        if n > 0 && it % 7 == 0 {
            fdk_push_back_cache(&mut x, 1);
            fdk_push_back_cache(&mut y, 1);
        }
        let cb = 1 + (rng.next_u32() % 11) as usize;
        for w in 0..64 {
            let c = x.bits_in_cache;
            let r1 = straight_word_signs(&mut x, cb);
            let r2 = fast_word_signs(&mut y, cb);
            assert_eq!(r1, r2, "{it} word {w} cb {cb}");
            assert_eq!(bs_state(&x), bs_state(&y), "{it} word {w} cb {cb}: bitstream");
            edge += (c < 32 && x.bits_in_cache == 32) as u32;
        }
    }
    assert!(edge > 100, "refill and push back into the full cache: {edge} words");
}

/// CBlock_ReadScaleFactorData(): the table version against the straight one on random states,
/// codebooks (with PNS and intensity) and bitstream positions and cache states (including a
/// cache of more than 32 bits, which takes the straight decoding), and against C.
#[test]
fn read_scale_factor_data_fast() {
    let mut rng = Rng::new(0xb10c_f006);
    let mut p = Pair::new();
    for it in 0..6000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let vcb = rng.next_u32() % 2 == 0;
        random_codebooks(&mut rng, &mut p.r.ci[ch], vcb);
        p.r.ci[ch].data.aac.pns_data.pns_active = (rng.next_u32() % 2) as u8;
        let flags = [0, AC_USAC, AC_MPEGD_RES, AC_ER_VCB11, 0][(rng.next_u32() % 5) as usize];
        let buf_bytes = [256usize, 64, 16][(rng.next_u32() % 3) as usize];
        let data = random_bytes(&mut rng, buf_bytes);
        if it % 2 == 0 {
            // against C, from the start of the buffer
            p.set_bs(&data, buf_bytes, buf_bytes as u32 * 8);
            p.sync();
            let ce = unsafe { c::cref_block_read_scale_factor_data(p.c.0, ch as i32, flags) };
            let re = cblock_read_scale_factor_data_fast(&mut p.r.ci[ch], &mut p.r.bs, flags);
            assert_eq!(re, ce, "CBlock_ReadScaleFactorData {it}");
            p.check(&format!("CBlock_ReadScaleFactorData {it} flags {flags:#x}"));
            p.check_bs(&format!("CBlock_ReadScaleFactorData {it} flags {flags:#x}"));
        } else {
            p.r.set_bs(&data, buf_bytes, buf_bytes as u32 * 8);
            let mut q = clone_ctx(&mut p.r, &data, buf_bytes, buf_bytes as u32 * 8);
            let seed = rng.next_u64();
            random_bs_position(&mut Rng::new(seed), &mut p.r.bs);
            random_bs_position(&mut Rng::new(seed), &mut q.bs);
            if rng.next_u32() % 16 == 0 && p.r.bs.bits_in_cache == 32 {
                // a cache beyond 32 bits (assertion violated by FDKpushBackCache())
                fdk_push_back_cache(&mut p.r.bs, 1);
                fdk_push_back_cache(&mut q.bs, 1);
            }
            let ra = cblock_read_scale_factor_data(&mut p.r.ci[ch], &mut p.r.bs, flags);
            let rb = cblock_read_scale_factor_data_fast(&mut q.ci[ch], &mut q.bs, flags);
            assert_eq!(ra, rb, "{it}");
            assert_eq!(bs_state(&p.r.bs), bs_state(&q.bs), "{it}: bitstream");
            assert_eq!(p.r.dump().v, q.dump().v, "{it}: state");
        }
    }
}

/// CBlock_GetEscape(): the peeking version against the straight one (every prefix length,
/// random positions and cache states, small wrapping buffers).
#[test]
fn get_escape_fast_vs_straight() {
    let mut rng = Rng::new(0xb10c_f007);
    for it in 0..100000 {
        let buf_bytes = [8usize, 16, 64][(rng.next_u32() % 3) as usize];
        let mut a = random_bytes(&mut rng, buf_bytes);
        if rng.next_u32() % 2 == 0 {
            // a prefix of k ones at a random place
            let k = rng.next_u32() % 11;
            let at = rng.next_u32() as usize % (buf_bytes * 8);
            for j in 0..=k as usize {
                let bit = (at + j) % (buf_bytes * 8);
                if j < k as usize { a[bit / 8] |= 0x80 >> (bit % 8) } else { a[bit / 8] &= !(0x80 >> (bit % 8)) }
            }
        }
        let b = a.clone();
        let (mut x, mut y) = (FdkBitStream::default(), FdkBitStream::default());
        fdk_init_bit_stream(&mut x, &BitMem::from_slice(&a), buf_bytes as u32, buf_bytes as u32 * 8, BS_READER);
        fdk_init_bit_stream(&mut y, &BitMem::from_slice(&b), buf_bytes as u32, buf_bytes as u32 * 8, BS_READER);
        let seed = rng.next_u64();
        random_bs_position(&mut Rng::new(seed), &mut x);
        random_bs_position(&mut Rng::new(seed), &mut y);
        for w in 0..8 {
            let q = match rng.next_u32() % 4 {
                0 => 16,
                1 => -16,
                2 => rng.range(-20, 20),
                _ => rng.i32(),
            };
            assert_eq!(cblock_get_escape(&mut x, q), cblock_get_escape_fast(&mut y, q), "{it} {w} q {q}");
            assert_eq!(bs_state(&x), bs_state(&y), "{it} {w} q {q}: bitstream");
        }
    }
}

/// CBlock_ScaleSpectralData_func1(): the slice version against the straight one on library band
/// tables and on random band offset tables (widths that are not multiples of 4, negative or
/// zero), short spectra and any number of bands.
#[test]
fn scale_spectral_data_func1_fast_vs_straight() {
    let mut rng = Rng::new(0xb10c_f008);
    let mut p = Pair::new();
    for it in 0..20000 {
        random_config(&mut p, &mut rng, true);
        let sri = p.r.sri;
        let table: Vec<i16> = match rng.next_u32() % 3 {
            0 => sri.scale_factor_bands_short.to_vec(),
            1 => sri.scale_factor_bands_long.to_vec(),
            _ => {
                let mut o = 0i16;
                let n = rng.range(0, 60);
                let mut t = Vec::new();
                for _ in 0..n {
                    t.push(o);
                    o += rng.range(-8, 40) as i16;
                }
                t
            }
        };
        let max_sfbs = rng.range(-2, 64);
        let window = rng.range(0, 7);
        let len = [1024usize, 128, 1000, 7, 0][(rng.next_u32() % 5) as usize];
        let spec: Vec<i32> = (0..len).map(|_| random_spec(&mut rng)).collect();
        let sfb_scale: Vec<i16> = (0..128).map(|_| rng.range(-40, 40) as i16).collect();
        let ssw = rng.range(-10, 60);
        let mut a = spec.clone();
        let mut b = spec.clone();
        cblock_scale_spectral_data_func1(&mut a, max_sfbs, &table, ssw, &sfb_scale, window);
        cblock_scale_spectral_data_func1_fast(&mut b, max_sfbs, &table, ssw, &sfb_scale, window);
        assert_eq!(a, b, "{it}: max {max_sfbs}, ssw {ssw}");
    }
}

/// CBlock_ScaleSpectralData(): the fast version against C (valid states) and against the
/// straight one (also more bands than the table has, which reads offsets of 0).
#[test]
fn scale_spectral_data_fast() {
    let mut rng = Rng::new(0xb10c_f009);
    let mut p = Pair::new();
    for it in 0..6000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        random_tns(&mut rng, &mut p.r.ci[ch]);
        for i in 0..128 {
            p.r.ci[ch].p_dyn_data.a_sfb_scale[i] = rng.range(-30, 40) as i16;
        }
        for i in 0..1024 {
            p.r.spec[ch * 1024 + i] = random_spec(&mut rng);
        }
        let sri = p.r.sri;
        if it % 2 == 0 {
            let max_sfbs = rng.range(0, p.r.ci[ch].ics_info.max_sf_bands as i32) as u8;
            p.sync();
            unsafe { c::cref_block_scale_spectral_data(p.c.0, ch as i32, max_sfbs as i32) };
            cblock_scale_spectral_data_fast(&mut p.r.ci[ch], max_sfbs, &sri, &mut p.r.spec[ch * 1024..]);
            p.check(&format!("CBlock_ScaleSpectralData {it}"));
        } else {
            let max_sfbs = rng.range(0, 63) as u8;
            let mut q = clone_ctx(&mut p.r, &[], 16, 0);
            cblock_scale_spectral_data(&mut p.r.ci[ch], max_sfbs, &sri, &mut p.r.spec[ch * 1024..]);
            cblock_scale_spectral_data_fast(&mut q.ci[ch], max_sfbs, &sri, &mut q.spec[ch * 1024..]);
            assert_eq!(p.r.dump().v, q.dump().v, "{it}: state");
        }
    }
}

/// CBlock_ReadSectionData(): the fast version against C (as `block::read_section_data`) and
/// against the straight one (random positions, inconsistent window groups of long blocks).
#[test]
fn read_section_data_fast() {
    let mut rng = Rng::new(0xb10c_f00a);
    let mut p = Pair::new();
    for it in 0..8000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        p.r.ci[ch].p_dyn_data.raw_data_info.common_window = (rng.next_u32() % 2) as u8;
        let flags = [0, AC_ER_VCB11, AC_ER_HCR, AC_ER_VCB11 | AC_ER_HCR, AC_ER | AC_LD][(rng.next_u32() % 5) as usize];
        let mut data = random_bytes(&mut rng, 64);
        // Inputs that reach a reference site without a defined result are avoided (see
        // cblock_read_section_data).
        data[63] &= 0x7F;
        data[62] |= 0x01;
        let sri = p.r.sri;
        if it % 2 == 0 {
            p.set_bs(&data, 64, 64 * 8);
            p.sync();
            let ce = unsafe { c::cref_block_read_section_data(p.c.0, ch as i32, flags) };
            let re = cblock_read_section_data_fast(&mut p.r.bs, &mut p.r.ci[ch], &sri, flags);
            assert_eq!(re, ce, "CBlock_ReadSectionData {it}");
            p.check(&format!("CBlock_ReadSectionData {it} flags {flags:#x}"));
            p.check_bs(&format!("CBlock_ReadSectionData {it} flags {flags:#x}"));
        } else {
            if rng.next_u32() % 4 == 0 {
                p.r.ci[ch].ics_info.window_groups = rng.range(0, 8) as u8;
            }
            p.r.set_bs(&data, 64, 64 * 8);
            let mut q = clone_ctx(&mut p.r, &data, 64, 64 * 8);
            let seed = rng.next_u64();
            random_bs_position(&mut Rng::new(seed), &mut p.r.bs);
            random_bs_position(&mut Rng::new(seed), &mut q.bs);
            let ra = cblock_read_section_data(&mut p.r.bs, &mut p.r.ci[ch], &sri, flags);
            let rb = cblock_read_section_data_fast(&mut q.bs, &mut q.ci[ch], &sri, flags);
            assert_eq!(ra, rb, "{it}");
            assert_eq!(bs_state(&p.r.bs), bs_state(&q.bs), "{it}: bitstream");
            assert_eq!(p.r.dump().v, q.dump().v, "{it}: state");
        }
    }
}

/// CBlock_InverseQuantizeSpectralData(): the fast version against C (as
/// `block::inverse_quantize_spectral_data`) and against the straight one (also any values, which
/// the straight port reads through its flattened tables, and inconsistent band counts).
#[test]
fn inverse_quantize_spectral_data_fast() {
    let mut rng = Rng::new(0xb10c_f00b);
    let mut p = Pair::new();
    for it in 0..8000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let vcb = rng.next_u32() % 2 == 0;
        random_codebooks(&mut rng, &mut p.r.ci[ch], vcb);
        let mode = rng.next_u32() % 8;
        for i in 0..1024 {
            p.r.spec[ch * 1024 + i] = match mode {
                0 => random_spec(&mut rng),
                1 => rng.range(-1100, 1100),
                _ => random_quant(&mut rng),
            };
        }
        for i in 0..128 {
            p.r.ci[ch].p_dyn_data.a_scale_factor[i] = rng.range(-100, 155) as i16;
        }
        let active = (rng.next_u32() % 2) as u8;
        let sri = p.r.sri;
        if it % 2 == 0 {
            p.sync();
            let ce = unsafe { c::cref_block_inverse_quantize(p.c.0, ch as i32, active as i32) };
            let re = cblock_inverse_quantize_spectral_data_fast(&mut p.r.ci[ch], &sri, active, &mut p.r.spec[ch * 1024..]);
            assert_eq!(re, ce, "CBlock_InverseQuantizeSpectralData {it}");
            p.check(&format!("CBlock_InverseQuantizeSpectralData {it}"));
        } else {
            if rng.next_u32() % 4 == 0 {
                p.r.ci[ch].ics_info.max_sf_bands = rng.range(0, 63) as u8;
            }
            let mut q = clone_ctx(&mut p.r, &[], 16, 0);
            let ra = cblock_inverse_quantize_spectral_data(&mut p.r.ci[ch], &sri, active, &mut p.r.spec[ch * 1024..]);
            let rb = cblock_inverse_quantize_spectral_data_fast(&mut q.ci[ch], &sri, active, &mut q.spec[ch * 1024..]);
            assert_eq!(ra, rb, "{it}");
            assert_eq!(p.r.dump().v, q.dump().v, "{it}: state");
        }
    }
}
