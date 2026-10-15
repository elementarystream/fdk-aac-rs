// Differential tests of fdk_aac_rs_tpdec::tpdec_asc against the C++ reference.
//
// The program config element functions run on PCEs read from generated and random bitstreams
// and on the default PCEs; AudioSpecificConfig_Parse() and the DRM configuration parsers run on
// generated configurations (all object types the parser knows, with GA, ELD and USAC specific
// configs, PCEs, SBR/PS/MPS signaling), their mutations (bit flips, truncation) and random
// bytes, with recording callbacks. After every call the return value, the complete outputs
// (CProgramConfig bytes, ASC dump), the bitstream state and the callback log are compared.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpdec_tpdec_asc as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_tpdec::tp_data::*;
use fdk_aac_rs_tpdec::tpdec_asc::*;

use super::tp_data::{asc_dump, assert_dump_eq, flip_bits, BitWriter, Bs, Recs, DUMP_MAX};

// ---------------------------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------------------------

/// CRC of the FDK CRC routines (MSB first) over the bits `[from, to)` of `w`.
pub fn crc_bits(w: &BitWriter, from: usize, to: usize, poly: u16, start: u16, len: u32) -> u16 {
    let mask: u16 = 1 << (len - 1);
    let mut crc = start;
    for i in from..to {
        let bit = ((w.bytes[i / 8] >> (7 - i % 8)) & 1) as u16;
        let flag = ((crc & mask != 0) as u16) ^ bit;
        crc <<= 1;
        if flag != 0 {
            crc ^= poly;
        }
    }
    crc & (((mask - 1) << 1) + 1)
}

/// Writes a program_config_element() (after the element id), aligned relative to bit `anchor`
/// of the writer. Mostly well-formed: unique tags, canonical element order, sometimes a height
/// extension with a correct CRC.
pub fn gen_pce(rng: &mut Rng, w: &mut BitWriter, anchor: usize) {
    let small = |rng: &mut Rng, max: u32| -> u32 {
        if rng.next_u32() % 4 == 0 {
            rng.next_u32() % (max + 1)
        } else {
            rng.next_u32() % (max.min(3) + 1)
        }
    };
    w.put(rng.next_u32() & 15, 4); // element_instance_tag
    w.put(if rng.next_u32() % 4 == 0 { rng.next_u32() & 3 } else { 1 }, 2); // object_type
    w.put(if rng.next_u32() % 4 == 0 { rng.next_u32() & 15 } else { 3 + rng.next_u32() % 6 }, 4);
    let nf = small(rng, 15);
    let ns = small(rng, 15);
    let nb = small(rng, 15);
    let nl = rng.next_u32() % 4;
    let na = small(rng, 7);
    let ncc = small(rng, 15);
    w.put(nf, 4);
    w.put(ns, 4);
    w.put(nb, 4);
    w.put(nl, 2);
    w.put(na, 3);
    w.put(ncc, 4);
    for _ in 0..3 {
        if rng.next_u32() % 4 == 0 {
            w.put(1, 1);
            w.put(rng.next_u32() & 15, 4);
        } else {
            w.put(0, 1);
        }
    }
    // A matrix mixdown has 3 bits instead of 4 in the loop above: fix by rewriting is not
    // needed, the parser reads 2 + 1 bits; the random 4 bits above only shift the stream.
    let dup = rng.next_u32() % 8 == 0;
    let mut tags = [[0u32; 3]; 2];
    let mut el = |rng: &mut Rng, w: &mut BitWriter, n: u32| {
        for i in 0..n {
            let is_cpe = if i == 0 && n % 2 == 1 { 0 } else { rng.next_u32() % 2 };
            w.put(is_cpe, 1);
            let t = if dup { rng.next_u32() & 3 } else { tags[is_cpe as usize][0] };
            tags[is_cpe as usize][0] += 1;
            w.put(t & 15, 4);
        }
    };
    el(rng, w, nf);
    el(rng, w, ns);
    el(rng, w, nb);
    for i in 0..nl {
        w.put(if dup { rng.next_u32() & 1 } else { i }, 4);
    }
    for _ in 0..na {
        w.put(rng.next_u32() & 15, 4);
    }
    for _ in 0..ncc {
        w.put(rng.next_u32() & 1, 1);
        w.put(rng.next_u32() & 15, 4);
    }
    while (w.nbits - anchor) % 8 != 0 {
        w.put(0, 1);
    }
    // comment field, optionally with a height extension
    let mut ext = BitWriter::new();
    let mut ext_bytes = 0u32;
    if rng.next_u32() % 3 == 0 {
        let crc_start = w.nbits + 8;
        let _ = crc_start;
        // Built in place after the comment_field_bytes byte (written below), so record it in a
        // separate writer whose bit positions are relative to an aligned start.
        ext.put(if rng.next_u32() % 8 == 0 { rng.next_u32() & 0xff } else { 0xAC }, 8);
        for _ in 0..(nf + ns + nb) {
            ext.put(if rng.next_u32() % 10 == 0 { 3 } else { rng.next_u32() % 3 }, 2);
        }
        ext.align();
        let crc = crc_bits(&ext, 0, ext.nbits, 0x07, 0xFF, 8);
        let crc = if rng.next_u32() % 6 == 0 { crc ^ 0x10 } else { crc };
        ext.put(crc as u32, 8);
        ext_bytes = (ext.nbits / 8) as u32;
    }
    let extra = if rng.next_u32() % 3 == 0 { rng.next_u32() % 20 } else { 0 };
    let field = (ext_bytes + extra).min(255);
    w.put(field, 8);
    for &b in ext.bytes.iter() {
        w.put(b as u32, 8);
    }
    for _ in 0..extra {
        w.put(rng.next_u32() & 0xff, 8);
    }
}

pub fn put_aot(w: &mut BitWriter, aot: u32) {
    if aot >= 31 {
        w.put(31, 5);
        w.put(aot.max(32) - 32, 6);
    } else {
        w.put(aot, 5);
    }
}

/// A sampling frequency index of `n` bits, sometimes the escape value with an explicit rate.
pub fn put_sf(rng: &mut Rng, w: &mut BitWriter, n: u32) {
    let max = (1u32 << n) - 1;
    if rng.next_u32() % 10 == 0 {
        w.put(max, n);
        let rates = [44100, 48000, 96000, 96001, 0, 12345, 384000, 400000, 0xffffff];
        w.put(rates[(rng.next_u32() as usize) % rates.len()], 24);
    } else if rng.next_u32() % 6 == 0 {
        w.put(rng.next_u32() % max, n);
    } else {
        w.put([3, 4, 5, 6, 7, 8, 11][(rng.next_u32() % 7) as usize], n);
    }
}

/// SbrDfltHeader() as skipped by skipSbrHeader() and the recorder.
fn put_sbr_header(rng: &mut Rng, w: &mut BitWriter, is_usac: bool) {
    if !is_usac {
        w.put(rng.next_u32() & 63, 6);
    }
    w.put(rng.next_u32() & 0xff, 8);
    let e1 = rng.next_u32() & 1;
    let e2 = rng.next_u32() & 1;
    w.put(e1, 1);
    w.put(e2, 1);
    w.put(rng.next_u32(), 5 * e1 + 6 * e2);
}

fn ga_specific_config(rng: &mut Rng, w: &mut BitWriter, aot: u32, chcfg: u32, anchor: usize) {
    w.put(rng.next_u32() % 4 / 3, 1); // frameLengthFlag
    let dep = rng.next_u32() % 6 == 0;
    w.put(dep as u32, 1);
    if dep {
        w.put(rng.next_u32() & 0x3fff, 14);
    }
    let ext = aot >= 17 || rng.next_u32() % 4 == 0;
    w.put(ext as u32, 1);
    if chcfg == 0 {
        gen_pce(rng, w, anchor);
    }
    if aot == 6 || aot == 20 {
        w.put(if rng.next_u32() % 3 == 0 { rng.next_u32() & 7 } else { 0 }, 3);
    }
    if ext {
        if aot == 22 {
            w.put(rng.next_u32() & 31, 5);
            w.put(rng.next_u32() & 0x7ff, 11);
        }
        if aot == 17 || aot == 19 || aot == 20 || aot == 23 {
            w.put(rng.next_u32() & 7, 3);
        }
        w.put(rng.next_u32() % 4 / 3, 1);
    }
}

fn eld_specific_config(rng: &mut Rng, w: &mut BitWriter, chcfg: u32) {
    w.put(rng.next_u32() & 1, 1); // frameLengthFlag
    w.put(rng.next_u32() & 7, 3); // resilience flags
    let sbr = rng.next_u32() % 2 == 0;
    w.put(sbr as u32, 1);
    if sbr {
        w.put(rng.next_u32() & 1, 1);
        w.put(rng.next_u32() & 1, 1);
        let n = match chcfg {
            1 | 2 => 1,
            3 => 2,
            4..=6 => 3,
            7 | 11 | 12 | 14 => 4,
            _ => 0,
        };
        for _ in 0..n {
            put_sbr_header(rng, w, false);
        }
    }
    let n_ext = match rng.next_u32() % 6 {
        0 => 0,
        1..=3 => 1,
        4 => 2,
        _ => 1 + rng.next_u32() % 17,
    };
    for _ in 0..n_ext {
        let t = match rng.next_u32() % 5 {
            0 | 1 => 3, // ELDEXT_DOWNSCALEINFO
            2 => 2,     // ELDEXT_LDSAC
            _ => 1 + rng.next_u32() % 15,
        };
        w.put(t, 4);
        if t == 3 {
            let mut body = BitWriter::new();
            put_sf(rng, &mut body, 4);
            body.put(if rng.next_u32() % 8 == 0 { 1 } else { 0 }, 4);
            body.align();
            let len = body.nbits as u32 / 8;
            w.put(len.min(14), 4);
            for &b in &body.bytes {
                w.put(b as u32, 8);
            }
        } else {
            let len = match rng.next_u32() % 8 {
                0 => 15 + rng.next_u32() % 300,
                _ => rng.next_u32() % 8,
            };
            if len < 15 {
                w.put(len, 4);
            } else {
                w.put(15, 4);
                let r = len - 15;
                if r < 255 {
                    w.put(r, 8);
                } else {
                    w.put(255, 8);
                    w.put(r - 255, 16);
                }
            }
            w.random_bytes(len as usize, rng);
        }
    }
    w.put(0, 4); // ELDEXT_TERM
}

fn usac_config(rng: &mut Rng, w: &mut BitWriter) {
    put_sf(rng, w, 5);
    let csfli = if rng.next_u32() % 8 == 0 { rng.next_u32() & 7 } else { rng.next_u32() % 5 };
    w.put(csfli, 3);
    let chcfg = if rng.next_u32() % 10 == 0 { rng.next_u32() & 31 } else { 1 + rng.next_u32() % 2 };
    w.put(chcfg, 5);
    let sbr = [0, 0, 2, 3, 1, 0, 0, 0][(csfli & 7) as usize] > 0;
    // elements: the channel elements of the configuration plus extension elements
    let mut els: Vec<u32> = match chcfg {
        1 => vec![0],
        2 => vec![1],
        _ => vec![rng.next_u32() & 3],
    };
    for _ in 0..rng.next_u32() % 3 {
        let pos = (rng.next_u32() as usize) % (els.len() + 1);
        els.insert(pos, 3);
    }
    if rng.next_u32() % 10 == 0 {
        els.push(rng.next_u32() & 3);
    }
    // Usually an AudioPreRoll() extension element first (as USAC streams with pre-roll have).
    let preroll_first = rng.next_u32() % 2 == 0;
    if preroll_first {
        els.insert(0, 3);
    }
    w.escaped(els.len() as u32 - 1, 4, 8, 16);
    for (k, &e) in els.iter().enumerate() {
        w.put(e, 2);
        match e {
            0 | 1 => {
                w.put((rng.next_u32() % 10 == 0) as u32, 1); // tw_mdct
                w.put(rng.next_u32() & 1, 1); // noiseFilling
                if sbr {
                    w.put(rng.next_u32() & 7, 3); // harmonicSBR, interTes, pvc
                    put_sbr_header(rng, w, true);
                    if e == 1 {
                        w.put(if rng.next_u32() % 2 == 0 { 0 } else { rng.next_u32() & 3 }, 2);
                    }
                }
            }
            2 => {}
            _ => {
                let t = match if k == 0 && preroll_first { 1 } else { rng.next_u32() % 6 } {
                    0 => 0, // fill
                    1 => 3, // audio pre-roll
                    2 => 4, // uniDrc
                    3 => 1,
                    _ => rng.next_u32() % 20,
                };
                w.escaped(t, 4, 8, 16);
                let len = rng.next_u32() % 12;
                w.escaped(len, 4, 8, 16);
                let dl = rng.next_u32() % 2;
                w.put(dl, 1);
                if dl != 0 {
                    w.escaped(rng.next_u32() % 300, 8, 16, 0);
                }
                w.put(rng.next_u32() & 1, 1);
                w.random_bytes(len as usize, rng);
            }
        }
    }
    let cext = rng.next_u32() % 3 == 0;
    w.put(cext as u32, 1);
    if cext {
        let n = rng.next_u32() % 3;
        w.escaped(n, 2, 4, 8);
        for _ in 0..=n {
            let t = [0, 2, 2, 7][(rng.next_u32() % 4) as usize];
            w.escaped(t, 4, 8, 16);
            let len = rng.next_u32() % 6;
            w.escaped(len, 4, 8, 16);
            for _ in 0..len {
                w.put(if t == 0 && rng.next_u32() % 8 != 0 { 0xa5 } else { rng.next_u32() & 0xff }, 8);
            }
        }
    }
}

/// Writes an AudioSpecificConfig of a random object type.
pub fn gen_asc(rng: &mut Rng, w: &mut BitWriter) {
    let anchor = w.nbits;
    let aots = [2, 2, 5, 29, 23, 39, 39, 42, 42, 17, 19, 20, 6, 22, 1, 4, 30, 24, 8];
    let aot = if rng.next_u32() % 16 == 0 { rng.next_u32() % 64 } else { aots[(rng.next_u32() as usize) % aots.len()] };
    put_aot(w, aot);
    if aot == 42 {
        // AOT_USAC: the sampling frequency is part of UsacConfig()? No: AudioSpecificConfig
        // has samplingFrequencyIndex and channelConfiguration first for every object type.
    }
    put_sf(rng, w, 4);
    let chcfg = match rng.next_u32() % 8 {
        0 => 0,
        1 => rng.next_u32() & 15,
        2 => 1,
        _ => 2,
    };
    w.put(chcfg, 4);
    let mut real = aot;
    if aot == 5 || aot == 29 {
        put_sf(rng, w, 4);
        real = if rng.next_u32() % 6 == 0 { [22, 23, 1][(rng.next_u32() % 3) as usize] } else { 2 };
        put_aot(w, real);
        if real == 22 {
            w.put(rng.next_u32() & 15, 4);
        }
    }
    match real {
        1 | 2 | 6 | 17 | 23 | 20 | 22 => ga_specific_config(rng, w, real, chcfg, anchor),
        39 => eld_specific_config(rng, w, chcfg),
        42 => usac_config(rng, w),
        30 => w.random_bytes(4, rng),
        _ => {}
    }
    if [17, 19, 20, 22, 23, 24, 25, 39].contains(&real) {
        w.put(if rng.next_u32() % 8 == 0 { rng.next_u32() & 3 } else { rng.next_u32() & 1 }, 2);
    }
    // backward compatible extensions
    if rng.next_u32() % 3 == 0 {
        for _ in 0..1 + rng.next_u32() % 2 {
            match rng.next_u32() % 5 {
                0 | 1 => {
                    w.put(0x2b7, 11);
                    put_aot(w, if rng.next_u32() % 5 == 0 { 22 } else { 5 });
                    let sbr = rng.next_u32() % 4 != 0;
                    w.put(sbr as u32, 1);
                    if sbr {
                        put_sf(rng, w, 4);
                    }
                    if rng.next_u32() % 2 == 0 {
                        w.put(0x548, 11);
                        w.put(rng.next_u32() & 1, 1);
                    }
                }
                2 => {
                    w.put(if rng.next_u32() % 2 == 0 { 0x76a } else { 0x7cc }, 11);
                    let p = rng.next_u32() & 1;
                    w.put(p, 1);
                    if p != 0 {
                        let len = if rng.next_u32() % 8 == 0 { 255 } else { rng.next_u32() % 40 };
                        w.put(len, 8);
                        if len == 255 {
                            w.put(rng.next_u32() % 100, 16);
                        }
                        w.random_bytes(6, rng);
                    }
                }
                3 => {
                    w.put(0x7cb, 11);
                    let p = rng.next_u32() & 1;
                    w.put(p, 1);
                    if p != 0 {
                        w.put(rng.next_u32() % 40, 8);
                        w.random_bytes(6, rng);
                    }
                }
                _ => w.put(rng.next_u32() & 0x7ff, 11),
            }
        }
    }
}

/// A test configuration: generated, mutated or random bytes. Returns the bytes and the number of
/// valid bits.
pub fn config_bytes(rng: &mut Rng, gen: fn(&mut Rng, &mut BitWriter)) -> (Vec<u8>, u32) {
    let mut w = BitWriter::new();
    match rng.next_u32() % 8 {
        0 => {
            w.random_bytes(1 + (rng.next_u32() % 64) as usize, rng);
        }
        _ => gen(rng, &mut w),
    }
    let mut bytes = w.bytes.clone();
    let mut bits = w.nbits as u32;
    match rng.next_u32() % 6 {
        0 => {
            let n = 1 + rng.next_u32() % 4;
            flip_bits(&mut bytes, n, rng);
        }
        1 => {
            // truncate
            if bits > 0 {
                bits = rng.next_u32() % bits;
            }
        }
        2 => {
            // trailing data
            for _ in 0..rng.next_u32() % 8 {
                bytes.push(rng.next_u32() as u8);
            }
            bits = bytes.len() as u32 * 8;
        }
        _ => {}
    }
    (bytes, bits)
}

// ---------------------------------------------------------------------------------------------
// Program config element
// ---------------------------------------------------------------------------------------------

fn pce_from(bytes: &[u8]) -> CProgramConfig {
    let mut p = CProgramConfig::zeroed();
    // SAFETY: CProgramConfig is repr(C) of u8 only (see tp_data.rs); any byte pattern is valid.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), &mut p as *mut _ as *mut u8, CPROGRAMCONFIG_SIZE)
    };
    p
}

/// Random PCE bytes; the height values are kept below 3 (larger values have no defined result
/// in the reference).
fn random_pce(rng: &mut Rng) -> CProgramConfig {
    let mut b: Vec<u8> = (0..CPROGRAMCONFIG_SIZE).map(|_| rng.next_u32() as u8).collect();
    let mut p = pce_from(&b);
    p.num_front_channel_elements &= 15;
    p.num_side_channel_elements &= 15;
    p.num_back_channel_elements &= 15;
    p.num_lfe_channel_elements &= 3;
    p.num_assoc_data_elements &= 7;
    p.num_valid_cc_elements &= 15;
    for h in p
        .front_element_height_info
        .iter_mut()
        .chain(p.side_element_height_info.iter_mut())
        .chain(p.back_element_height_info.iter_mut())
    {
        *h %= 3;
    }
    for v in p
        .front_element_is_cpe
        .iter_mut()
        .chain(p.side_element_is_cpe.iter_mut())
        .chain(p.back_element_is_cpe.iter_mut())
    {
        if rng.next_u32() % 4 != 0 {
            *v &= 1;
        }
    }
    b.clear();
    p
}

/// Random PCE bytes or a PCE of `some_pce()`.
pub fn random_or_valid_pce(rng: &mut Rng) -> CProgramConfig {
    if rng.next_u32() % 2 == 0 {
        random_pce(rng)
    } else {
        some_pce(rng)
    }
}

/// A PCE read from a generated (or random) bitstream, or a default PCE, or random bytes.
fn some_pce(rng: &mut Rng) -> CProgramConfig {
    match rng.next_u32() % 6 {
        0 => random_pce(rng),
        1 => {
            let mut p = CProgramConfig::zeroed();
            cprogram_config_get_default(&mut p, [0, 1, 2, 3, 4, 5, 6, 7, 11, 12, 14, 32][(rng.next_u32() % 12) as usize]);
            p
        }
        _ => {
            let mut w = BitWriter::new();
            gen_pce(rng, &mut w, 0);
            let mut p = CProgramConfig::zeroed();
            cprogram_config_init(&mut p);
            let mut bs = Bs::new(&w.bytes, w.nbits as u32, 64, None);
            let anchor = w.nbits as u32;
            cprogram_config_read(&mut p, &mut bs.rs, anchor);
            p
        }
    }
}

#[test]
fn cprogram_config_read_() {
    let mut rng = Rng::new(0x5ce);
    for it in 0..20000 {
        let mut w = BitWriter::new();
        let lead = rng.next_u32() % 8;
        w.put(rng.next_u32(), lead);
        if rng.next_u32() % 8 == 0 {
            w.random_bytes(1 + (rng.next_u32() % 100) as usize, &mut rng);
        } else {
            gen_pce(&mut rng, &mut w, lead as usize * 0);
        }
        let mut bytes = w.bytes.clone();
        if rng.next_u32() % 4 == 0 {
            let n = 1 + rng.next_u32() % 3;
            flip_bits(&mut bytes, n, &mut rng);
        }
        let valid = if rng.next_u32() % 8 == 0 { rng.next_u32() % (w.nbits as u32 + 1) } else { w.nbits as u32 };
        let mut bs = Bs::new(&bytes, valid, 64, if rng.next_u32() % 2 == 0 { Some(&mut rng) } else { None });
        // skip the leading bits
        fdk_aac_rs_fdk::bitstream::fdk_read_bits(&mut bs.rs, lead);
        bs.sync_c();
        let anchor = match rng.next_u32() % 3 {
            0 => rng.next_u32(),
            _ => bs.rs.h_bit_buf.valid_bits.wrapping_add(bs.rs.bits_in_cache).wrapping_add(lead * (rng.next_u32() % 2)),
        };

        let mut p = if rng.next_u32() % 2 == 0 { random_pce(&mut rng) } else { some_pce(&mut rng) };
        let mut cb = p.as_bytes().to_vec();
        cprogram_config_read(&mut p, &mut bs.rs, anchor);
        unsafe { c::cref_asc_pce_read(cb.as_mut_ptr(), bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr(), anchor) };
        assert_eq!(p.as_bytes().as_slice(), cb.as_slice(), "CProgramConfig_Read {it}");
        bs.check(&format!("CProgramConfig_Read {it}"));
        assert_eq!(cprogram_config_is_valid(&p), unsafe { c::cref_asc_pce_is_valid(cb.as_ptr()) });
    }
}

#[test]
fn cprogram_config_init_reset_default() {
    let mut rng = Rng::new(7);
    for it in 0..200 {
        let mut p = random_pce(&mut rng);
        let mut cb = p.as_bytes().to_vec();
        if it % 2 == 0 {
            cprogram_config_init(&mut p);
            unsafe { c::cref_asc_pce_init(cb.as_mut_ptr()) };
        } else {
            cprogram_config_reset(&mut p);
            unsafe { c::cref_asc_pce_reset(cb.as_mut_ptr()) };
        }
        assert_eq!(p.as_bytes().as_slice(), cb.as_slice(), "init/reset {it}");
    }
    for cfg in 0..=40u32 {
        let mut p = random_pce(&mut rng);
        let mut cb = p.as_bytes().to_vec();
        cprogram_config_get_default(&mut p, cfg);
        unsafe { c::cref_asc_pce_get_default(cb.as_mut_ptr(), cfg) };
        assert_eq!(p.as_bytes().as_slice(), cb.as_slice(), "GetDefault({cfg})");
    }
}

#[test]
fn cprogram_config_compare_() {
    let mut rng = Rng::new(11);
    for it in 0..20000 {
        let a = some_pce(&mut rng);
        let mut b = match rng.next_u32() % 4 {
            0 => some_pce(&mut rng),
            _ => a,
        };
        // small changes
        let mut bytes = b.as_bytes().to_vec();
        for _ in 0..rng.next_u32() % 3 {
            let i = (rng.next_u32() as usize) % CPROGRAMCONFIG_SIZE;
            bytes[i] = bytes[i].wrapping_add(1 + (rng.next_u32() % 3) as u8);
        }
        b = pce_from(&bytes);
        // keep counts and heights in range
        b.num_front_channel_elements &= 15;
        b.num_side_channel_elements &= 15;
        b.num_back_channel_elements &= 15;
        for h in b.front_element_height_info.iter_mut().chain(b.side_element_height_info.iter_mut()).chain(b.back_element_height_info.iter_mut()) {
            *h %= 3;
        }
        let r = cprogram_config_compare(&a, &b);
        let e = unsafe { c::cref_asc_pce_compare(a.as_bytes().as_ptr(), b.as_bytes().as_ptr()) };
        assert_eq!(r, e, "CProgramConfig_Compare {it}");
    }
}

#[test]
fn cprogram_config_tables() {
    let mut rng = Rng::new(13);
    for it in 0..20000 {
        let p = some_pce(&mut rng);
        // GetChannelDescription
        let ch_config = match rng.next_u32() % 3 {
            0 => 0,
            1 => rng.next_u32() % 16,
            _ => rng.next_u32() % 40,
        };
        let with_pce = rng.next_u32() % 4 != 0;
        let mut rt = [0x55i32; 256];
        let mut ri = [0xaau8; 256];
        let mut ct = rt;
        let mut ci = ri;
        cprogram_config_get_channel_description(ch_config, if with_pce { Some(&p) } else { None }, &mut rt, &mut ri);
        unsafe {
            c::cref_asc_pce_get_channel_description(
                ch_config,
                if with_pce { p.as_bytes().as_ptr() } else { core::ptr::null() },
                ct.as_mut_ptr(),
                ci.as_mut_ptr(),
            )
        };
        assert_eq!(rt, ct, "GetChannelDescription types {it}");
        assert_eq!(ri, ci, "GetChannelDescription indices {it}");

        // GetPceChMap
        let len = rng.next_u32() % 64;
        let mut rm = [0x33u8; 256];
        let mut cm = rm;
        let r = cprogram_config_get_pce_ch_map(&p, &mut rm, len);
        let e = unsafe { c::cref_asc_pce_get_pce_ch_map(p.as_bytes().as_ptr(), cm.as_mut_ptr(), len) };
        assert_eq!(r, e, "GetPceChMap {it}");
        assert_eq!(rm, cm, "GetPceChMap map {it}");

        // GetElementTable
        let size = rng.next_u32() as i32 % 80 - 10;
        let mut rl = [0x77i32; 256];
        let mut cl = rl;
        let mut ridx = 0x99u8;
        let mut cidx = 0x99u8;
        let r = cprogram_config_get_element_table(&p, &mut rl, size, &mut ridx);
        let e = unsafe { c::cref_asc_pce_get_element_table(p.as_bytes().as_ptr(), cl.as_mut_ptr(), size, &mut cidx) };
        assert_eq!(r, e, "GetElementTable {it}");
        assert_eq!(rl, cl, "GetElementTable list {it}");
        assert_eq!(ridx, cidx, "GetElementTable chMapIdx {it}");
    }
}

#[test]
fn cprogram_config_lookup_element_() {
    let mut rng = Rng::new(17);
    for it in 0..3000 {
        let mut p = some_pce(&mut rng);
        p.el_counter = if rng.next_u32() % 4 == 0 { (rng.next_u32() % 40) as u8 } else { 0 };
        let mut cpce = p.as_bytes().to_vec();
        let mut rmap = [0x11u8; 256];
        let mut rty = [0x22i32; 256];
        let mut rix = [0x33u8; 256];
        let mut rel = [0i32; 256];
        for (i, e) in rel.iter_mut().enumerate() {
            *e = if i < 40 { [ID_SCE, ID_CPE, ID_LFE, ID_NONE, ID_CCE][(rng.next_u32() % 5) as usize] } else { ID_NONE };
        }
        let (mut cmap, mut cty, mut cix, mut cel) = (rmap, rty, rix, rel);
        let ch_config = match rng.next_u32() % 4 {
            0 | 1 => 0,
            2 => [1, 2, 3, 4, 5, 6, 7, 11, 12, 14][(rng.next_u32() % 10) as usize],
            _ => rng.next_u32() % 20,
        };
        let mut channel_idx = 0u32;
        for step in 0..20 {
            let el_type = [ID_SCE, ID_CPE, ID_LFE, ID_CCE, ID_DSE, ID_PCE, ID_FIL, ID_USAC_SCE, ID_USAC_CPE, ID_USAC_LFE, ID_USAC_EXT]
                [(rng.next_u32() % 11) as usize];
            let tag = if rng.next_u32() % 8 == 0 { rng.next_u32() % 20 } else { rng.next_u32() % 4 };
            let descr_len = if rng.next_u32() % 4 == 0 { rng.next_u32() % 16 } else { 8 };
            let mut rmapping = 0x44u8;
            let mut cmapping = 0x44u8;
            let r = cprogram_config_lookup_element(
                &mut p, ch_config, tag, channel_idx, &mut rmap, &mut rty, &mut rix, descr_len,
                &mut rmapping, &mut rel, el_type,
            );
            let e = unsafe {
                c::cref_asc_pce_lookup_element(
                    cpce.as_mut_ptr(), ch_config, tag, channel_idx, cmap.as_mut_ptr(), cty.as_mut_ptr(),
                    cix.as_mut_ptr(), descr_len, &mut cmapping, cel.as_mut_ptr(), el_type,
                )
            };
            let what = format!("LookupElement {it}/{step}");
            assert_eq!(r, e, "{what}");
            assert_eq!(p.as_bytes().as_slice(), cpce.as_slice(), "{what}: pce");
            assert_eq!(rmapping, cmapping, "{what}: elMapping");
            assert_eq!(rmap, cmap, "{what}: chMapping");
            assert_eq!(rty, cty, "{what}: chType");
            assert_eq!(rix, cix, "{what}: chIndex");
            assert_eq!(rel, cel, "{what}: elList");
            if r != 0 && (el_type == ID_CPE || el_type == ID_USAC_CPE) {
                channel_idx += 2;
            } else if r != 0 {
                channel_idx += 1;
            }
            channel_idx %= 200;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AudioSpecificConfig
// ---------------------------------------------------------------------------------------------

#[test]
fn audio_specific_config_init_() {
    let mut cd = vec![0i32; DUMP_MAX];
    let n = unsafe { c::cref_asc_init(cd.as_mut_ptr()) } as usize;
    let mut a = CSAudioSpecificConfig::zeroed();
    a.m_aot = 77;
    a.config[3] = 9;
    audio_specific_config_init(&mut a);
    assert_dump_eq(&asc_dump(&a), &cd[..n], "AudioSpecificConfig_Init");
}

fn asc_parse_case(rng: &mut Rng, it: usize, bytes: &[u8], bits: u32) {
    let mut recs = Recs::random(if rng.next_u32() % 3 == 0 { 4 } else { 0 }, rng);
    let mut bs = Bs::new(bytes, bits, 64, if rng.next_u32() % 3 == 0 { Some(rng) } else { None });
    let f_explicit = (rng.next_u32() % 4 != 0) as i32;
    let config_mode = [AC_CM_DET_CFG_CHANGE, AC_CM_ALLOC_MEM, 0, 3][(rng.next_u32() % 4) as usize];
    let config_changed = (rng.next_u32() % 2) as u8;
    let aot = match rng.next_u32() % 10 {
        0 => AOT_USAC,
        1 => [AOT_AAC_LC, AOT_ER_AAC_ELD, AOT_ER_AAC_LD, AOT_MPEGS, AOT_SBR, 77][(rng.next_u32() % 6) as usize],
        _ => AOT_NULL_OBJECT,
    };

    let mut a = CSAudioSpecificConfig::zeroed();
    let r = audio_specific_config_parse(&mut a, &mut bs.rs, f_explicit, &mut recs.rs, config_mode, config_changed, aot);
    let mut cd = vec![0i32; DUMP_MAX];
    let mut n = 0i32;
    let e = unsafe {
        c::cref_asc_parse(
            bs.cst.as_mut_ptr(),
            bs.cbuf.as_mut_ptr(),
            f_explicit,
            recs.c,
            config_mode,
            config_changed,
            aot,
            cd.as_mut_ptr(),
            &mut n,
        )
    };
    let what = format!("AudioSpecificConfig_Parse {it} ({bits} bits {:02x?})", &bytes[..bytes.len().min(16)]);
    assert_eq!(r, e, "{what}: result");
    bs.check(&what);
    assert_dump_eq(&asc_dump(&a), &cd[..n as usize], &what);
    recs.check(&what);
}

#[test]
fn audio_specific_config_parse_() {
    let mut rng = Rng::new(0xa5c);
    for it in 0..30000 {
        let (bytes, bits) = config_bytes(&mut rng, gen_asc);
        asc_parse_case(&mut rng, it, &bytes, bits);
    }
}

/// Configurations of the reference encoder (all object types and transports it supports).
#[test]
fn audio_specific_config_parse_encoder() {
    let mut rng = Rng::new(0xe1c);
    for (i, (bytes, _)) in super::tpdec_lib::encoder_configs().iter().enumerate() {
        for k in 0..20 {
            let mut b = bytes.clone();
            let mut bits = b.len() as u32 * 8;
            if k > 0 {
                match rng.next_u32() % 3 {
                    0 => flip_bits(&mut b, 1 + rng.next_u32() % 3, &mut rng),
                    1 => bits = rng.next_u32() % (bits + 1),
                    _ => {}
                }
            }
            asc_parse_case(&mut rng, i * 100 + k, &b, bits);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// DRM
// ---------------------------------------------------------------------------------------------

fn gen_drm_sdc(rng: &mut Rng, w: &mut BitWriter) {
    let coding = if rng.next_u32() % 3 == 0 { rng.next_u32() & 3 } else { [0, 3][(rng.next_u32() % 2) as usize] };
    w.put(coding, 2);
    w.put(rng.next_u32() & 1, 1); // SBR
    let mode = if rng.next_u32() % 5 == 0 { 1 + 2 * (rng.next_u32() & 1) } else { [0, 2][(rng.next_u32() % 2) as usize] };
    w.put(mode, 2);
    w.put(rng.next_u32() & 7, 3);
    w.put(rng.next_u32() & 3, 2);
    w.put(rng.next_u32() & 31, 5);
    w.put(rng.next_u32() & 1, 1);
    if coding == 3 {
        w.put(rng.next_u32() & 3, 2); // coreSbrFrameLengthIndex - 1
        w.put(rng.next_u32() & 1, 1); // noiseFilling
        w.put(rng.next_u32() & 7, 3); // harmonicSBR, interTes, pvc
        put_sbr_header(rng, w, true);
        w.put(rng.next_u32() & 3, 2); // stereoConfigIndex
        w.random_bytes(4, rng);
    }
}

#[test]
fn drm_raw_sdc_audio_config_parse_() {
    let mut rng = Rng::new(0xd0);
    for it in 0..20000 {
        let (bytes, bits) = config_bytes(&mut rng, gen_drm_sdc);
        let with_cb = rng.next_u32() % 4 != 0;
        let mut recs = Recs::random(if rng.next_u32() % 3 == 0 { 3 } else { 0 }, &mut rng);
        let mut bs = Bs::new(&bytes, bits, 64, if rng.next_u32() % 3 == 0 { Some(&mut rng) } else { None });
        let config_mode = [AC_CM_DET_CFG_CHANGE, AC_CM_ALLOC_MEM][(rng.next_u32() % 2) as usize];
        let config_changed = (rng.next_u32() % 2) as u8;
        let mut a = CSAudioSpecificConfig::zeroed();
        let r = drm_raw_sdc_audio_config_parse(
            &mut a,
            &mut bs.rs,
            if with_cb { Some(&mut recs.rs) } else { None },
            config_mode,
            config_changed,
        );
        let mut cd = vec![0i32; DUMP_MAX];
        let mut n = 0i32;
        let e = unsafe {
            c::cref_asc_drm_parse(
                bs.cst.as_mut_ptr(),
                bs.cbuf.as_mut_ptr(),
                if with_cb { recs.c } else { core::ptr::null_mut() },
                config_mode,
                config_changed,
                cd.as_mut_ptr(),
                &mut n,
            )
        };
        let what = format!("DrmRawSdcAudioConfig_Parse {it}");
        assert_eq!(r, e, "{what}");
        bs.check(&what);
        assert_dump_eq(&asc_dump(&a), &cd[..n as usize], &what);
        recs.check(&what);
    }
}

#[test]
fn drm_xhe_aac_static_config_() {
    let mut rng = Rng::new(0xd1);
    for it in 0..20000 {
        let mut w = BitWriter::new();
        w.put(rng.next_u32() & 3, 2);
        w.put(rng.next_u32() & 1, 1);
        w.put(rng.next_u32() & 7, 3);
        put_sbr_header(&mut rng, &mut w, true);
        w.random_bytes(6, &mut rng);
        let mut bytes = w.bytes.clone();
        if rng.next_u32() % 3 == 0 {
            flip_bits(&mut bytes, 2, &mut rng);
        }
        let with_cb = rng.next_u32() % 4 != 0;
        let mut recs = Recs::random(if rng.next_u32() % 3 == 0 { 3 } else { 0 }, &mut rng);
        let mut bs = Bs::new(&bytes, bytes.len() as u32 * 8, 64, None);
        let audio_mode = [0, 2, 1, 3][(rng.next_u32() % 4) as usize];
        // The library calls it with m_aot == AOT_USAC only (the USAC member of the m_sc union is
        // written; with another object type the C dump would read another union member).
        let aot = [AOT_USAC, AOT_DRM_USAC][(rng.next_u32() % 2) as usize];
        let sfi = (rng.next_u32() % 32) as u8;
        let sf = SAMPLING_RATE_TABLE[sfi as usize];
        let config_mode = (1 + rng.next_u32() % 2) as u8;

        let mut a = CSAudioSpecificConfig::zeroed();
        audio_specific_config_init(&mut a);
        a.m_aot = aot;
        a.m_sampling_frequency = sf;
        a.m_sampling_frequency_index = sfi;
        a.config_mode = config_mode;
        let r = drm_xhe_aac_static_config(
            &mut a,
            &mut bs.rs,
            audio_mode,
            if with_cb { Some(&mut recs.rs) } else { None },
        );
        let mut cd = vec![0i32; DUMP_MAX];
        let mut n = 0i32;
        let e = unsafe {
            c::cref_asc_drm_xheaac_static_config(
                bs.cst.as_mut_ptr(),
                bs.cbuf.as_mut_ptr(),
                audio_mode,
                if with_cb { recs.c } else { core::ptr::null_mut() },
                aot,
                sf,
                sfi,
                config_mode,
                cd.as_mut_ptr(),
                &mut n,
            )
        };
        let what = format!("Drm_xHEAACStaticConfig {it}");
        assert_eq!(r, e, "{what}");
        bs.check(&what);
        assert_dump_eq(&asc_dump(&a), &cd[..n as usize], &what);
        recs.check(&what);
    }
}

#[test]
fn map_sr2_mpeg_idx() {
    // The table is file-local in C (const); its values are checked through
    // DrmRawSdcAudioConfig_Parse() above. Here: it maps to the DRM xHE-AAC rates.
    let rates: Vec<u32> = MAP_SR2_MPEG_IDX.iter().map(|&i| SAMPLING_RATE_TABLE[i as usize]).collect();
    assert_eq!(rates, [9600, 12000, 16000, 19200, 24000, 32000, 38400, 48000]);
}

/// The file-static table `name` of tpdec_asc.cpp (a copy compiled into the shim).
fn c_asc_table<T: Copy>(name: &str) -> Vec<T> {
    let cname = std::ffi::CString::new(name).unwrap();
    let mut bytes = 0;
    let p = unsafe { fdk_aac_rs_cref::tpdec_tpdec_asc_tables::cref_tpdec_asc_table(cname.as_ptr(), &mut bytes) };
    assert!(!p.is_null(), "{name}");
    assert_eq!(bytes as usize % core::mem::size_of::<T>(), 0, "{name}");
    unsafe { core::slice::from_raw_parts(p as *const T, bytes as usize / core::mem::size_of::<T>()) }.to_vec()
}

/// The file-static tables of tpdec_asc.cpp, element by element: CHANNEL_CONFIGURATION_0 ...
/// CHANNEL_CONFIGURATION_14 (`channel_configuration_0` ...), the entries of
/// CHANNEL_CONFIGURATION_ARRAY (the list each entry points to, by name), SC_CHAN_CONFIG_TAB
/// (`sc_chan_config_tab`), USAC_FRAME_LENGTH (`usacFrameLength`) and SBR_RATIO_INDEX
/// (`sbrRatioIndex`).
#[test]
fn asc_tables() {
    use fdk_aac_rs_tpdec::tpdec_asc as r;
    let lists: [&[i32]; 15] = [
        &r::CHANNEL_CONFIGURATION_0,
        &r::CHANNEL_CONFIGURATION_1,
        &r::CHANNEL_CONFIGURATION_2,
        &r::CHANNEL_CONFIGURATION_3,
        &r::CHANNEL_CONFIGURATION_4,
        &r::CHANNEL_CONFIGURATION_5,
        &r::CHANNEL_CONFIGURATION_6,
        &r::CHANNEL_CONFIGURATION_7,
        &r::CHANNEL_CONFIGURATION_8,
        &r::CHANNEL_CONFIGURATION_9,
        &r::CHANNEL_CONFIGURATION_10,
        &r::CHANNEL_CONFIGURATION_11,
        &r::CHANNEL_CONFIGURATION_12,
        &r::CHANNEL_CONFIGURATION_13,
        &r::CHANNEL_CONFIGURATION_14,
    ];
    for (k, l) in lists.iter().enumerate() {
        assert_eq!(c_asc_table::<i32>(&format!("channel_configuration_{k}")), l.to_vec(), "channel_configuration_{k}");
    }
    assert_eq!(r::CHANNEL_CONFIGURATION_ARRAY.len(), 15);
    for (i, e) in r::CHANNEL_CONFIGURATION_ARRAY.iter().enumerate() {
        let mut len = 0;
        let name = unsafe { fdk_aac_rs_cref::tpdec_tpdec_asc_tables::cref_tpdec_asc_channel_configuration_array(i as i32, &mut len) };
        assert_eq!(len, 15);
        assert!(!name.is_null(), "channel_configuration_array[{i}]");
        let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_str().unwrap();
        let k: usize = name.strip_prefix("channel_configuration_").unwrap().parse().unwrap();
        assert!(core::ptr::eq(*e, lists[k]), "channel_configuration_array[{i}]: {name}");
    }
    let sc: Vec<i8> = r::SC_CHAN_CONFIG_TAB.iter().flat_map(|e| [e.n_ch, e.n_sce, e.n_cpe, e.n_lfe]).collect();
    assert_eq!(c_asc_table::<i8>("sc_chan_config_tab"), sc);
    assert_eq!(c_asc_table::<u16>("usacFrameLength"), r::USAC_FRAME_LENGTH.to_vec());
    assert_eq!(c_asc_table::<u8>("sbrRatioIndex"), r::SBR_RATIO_INDEX.to_vec());
}
