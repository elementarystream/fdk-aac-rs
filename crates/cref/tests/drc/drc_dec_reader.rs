// Differential tests of fdk_aac_rs_drc::drc_dec_reader against the C++ reference.
//
// The parsers run on the same bitstreams in C++ and Rust, starting from the same state (the
// reader compares with and assigns to the previous structures: `diff`). After every call the
// error code, the state of the bit buffer and the complete memory image of every structure are
// compared. The bitstreams are
//
// - random bytes;
// - random but valid uniDrcConfig(), loudnessInfoSet() and uniDrcGain() payloads, written by the
//   generator below, which follows the syntax of ISO/IEC 23003-4 (including AMD 1: the V1
//   extension, loudEqInstructions(), eqCoefficients(), eqInstructions()) and the derivations of
//   the parser (channel counts, channel groups, band counts);
// - the generated payloads with random bit errors and truncations.
//
// The generator is also used by the tests of the gain decoder and the public API.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::drc_drc_dec_reader as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_drc::drc_dec_reader::*;
use fdk_aac_rs_drc::drc_dec_rom::*;
use fdk_aac_rs_drc::drc_dec_tools::{get_delta_tmin, select_drc_coefficients};
use fdk_aac_rs_drc::drc_dec_types::*;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, FdkBitStream, BS_READER};

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// A value of a [`Pod`] type from its memory image (as written by the C++ code).
pub fn pod_from_bytes<T: Pod>(b: &[u8]) -> Box<T> {
    assert_eq!(b.len(), core::mem::size_of::<T>());
    let mut v: Box<core::mem::MaybeUninit<T>> = Box::new_uninit();
    // SAFETY: T is Pod (every bit pattern is valid), the sizes match.
    unsafe {
        core::ptr::copy_nonoverlapping(b.as_ptr(), v.as_mut_ptr() as *mut u8, b.len());
        v.assume_init()
    }
}

/// Compares two memory images and reports the first difference.
pub fn cmp_bytes(what: &str, rs: &[u8], c: &[u8]) {
    assert_eq!(rs.len(), c.len(), "{what}: size");
    if let Some(i) = (0..rs.len()).find(|&i| rs[i] != c[i]) {
        let lo = i.saturating_sub(8);
        let hi = (i + 8).min(rs.len());
        panic!(
            "{what}: first difference at byte {i}: rust {:02x?} c {:02x?} (bytes {lo}..{hi})",
            &rs[lo..hi],
            &c[lo..hi]
        );
    }
}

/// MSB-first bit writer.
#[derive(Clone, Default)]
pub struct BitWriter {
    pub buf: Vec<u8>,
    pub bits: usize,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            let bit = (v >> i) & 1;
            if self.bits % 8 == 0 {
                self.buf.push(0);
            }
            if bit != 0 {
                let last = self.buf.len() - 1;
                self.buf[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }

    pub fn append(&mut self, o: &BitWriter) {
        for i in 0..o.bits {
            let bit = (o.buf[i / 8] >> (7 - i % 8)) & 1;
            self.put(bit as u32, 1);
        }
    }

    /// Flips `n` random bits.
    pub fn corrupt(&mut self, rng: &mut Rng, n: usize) {
        for _ in 0..n {
            if self.bits == 0 {
                return;
            }
            let i = (rng.next_u32() as usize) % self.bits;
            self.buf[i / 8] ^= 0x80 >> (i % 8);
        }
    }
}

/// A reader buffer: the payload in a power-of-two sized buffer.
pub struct Payload {
    pub buf: Vec<u8>,
    pub valid_bits: u32,
}

impl Payload {
    pub fn new(w: &BitWriter, valid_bits: Option<u32>) -> Self {
        let size = w.buf.len().next_power_of_two().max(64);
        let mut buf = w.buf.clone();
        buf.resize(size, 0);
        Payload { buf, valid_bits: valid_bits.unwrap_or(w.bits as u32) }
    }

    pub fn random(rng: &mut Rng) -> Self {
        let n = (rng.next_u32() % 96) as usize;
        let w = BitWriter { buf: (0..n).map(|_| rng.next_u32() as u8).collect(), bits: n * 8 };
        let mut p = Self::new(&w, None);
        if rng.next_u32() % 4 == 0 {
            p.valid_bits = rng.next_u32() % (p.buf.len() as u32 * 8 + 1);
        }
        p
    }

    /// A Rust bitstream over a copy of the buffer (the copy is returned and must outlive it).
    pub fn rs(&self) -> (Box<[u8]>, FdkBitStream) {
        let b: Box<[u8]> = self.buf.clone().into_boxed_slice();
        let mut bs = FdkBitStream::default();
        fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&b), b.len() as u32, self.valid_bits, BS_READER);
        (b, bs)
    }
}

/// `[ValidBits, ReadOffset, BitNdx]` of a Rust bitstream, like the shim returns.
pub fn bs_state(bs: &mut FdkBitStream) -> [u32; 3] {
    let v = bs.get_valid_bits();
    [v, bs.h_bit_buf.read_offset, bs.h_bit_buf.bit_ndx]
}

fn chance(rng: &mut Rng, num: u32, den: u32) -> bool {
    rng.next_u32() % den < num
}

thread_local! {
    /// Clean mode of the generators: the deliberately invalid choices are 16 times rarer.
    static CLEAN: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

thread_local! {
    /// Extreme mode of the gain generator: the largest time deltas, so that the 16 bit node
    /// times wrap.
    static EXTREME: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Sets the extreme mode of the gain generator of this thread.
pub fn set_extreme(extreme: bool) {
    EXTREME.with(|c| c.set(extreme));
}

/// Sets the clean mode of the generators of this thread (see [`rare`]).
pub fn set_clean(clean: bool) {
    CLEAN.with(|c| c.set(clean));
}

/// A deliberately invalid choice of a generator, with probability `num / den` (16 times less in
/// clean mode).
fn rare(rng: &mut Rng, num: u32, den: u32) -> bool {
    let den = if CLEAN.with(|c| c.get()) { den * 16 } else { den };
    chance(rng, num, den)
}

fn pick(rng: &mut Rng, lo: u32, hi: u32) -> u32 {
    rng.range(lo as i32, hi as i32) as u32
}

/// Writes a random codeword of a Huffman codebook (a random walk from the root to a leaf).
fn huffman_cw(rng: &mut Rng, w: &mut BitWriter, h: &[[i8; 2]]) {
    let mut index: i8 = 0;
    while index >= 0 {
        let bit = rng.next_u32() & 1;
        w.put(bit, 1);
        index = h[index as usize][bit as usize];
    }
}

/// The number of bits needed for `v`.
fn bit_len(v: u32) -> u32 {
    32 - v.leading_zeros()
}

/// Writes an extension: type, size and payload (the payload size may be off by one).
fn put_ext(rng: &mut Rng, w: &mut BitWriter, ext_type: u32, size_len_bits: u32, payload: &BitWriter) {
    w.put(ext_type, 4);
    let mut size = payload.bits.max(1) as u32 - 1;
    if rare(rng, 1, 32) {
        size = size.wrapping_add(if chance(rng, 1, 2) { 1 } else { u32::MAX });
    }
    let len = bit_len(size).max(4) - 4;
    let max_len = (1 << size_len_bits) - 1;
    let len = len.min(max_len);
    w.put(len, size_len_bits);
    w.put(size & ((1u64 << (len + 4)) - 1) as u32, len + 4);
    let mut p = payload.clone();
    if p.bits == 0 {
        p.put(0, 1);
    }
    w.append(&p);
}

// ---------------------------------------------------------------------------------------------
// Generator of uniDrcConfig()
// ---------------------------------------------------------------------------------------------

/// The state of the parsed configuration that the syntax depends on.
#[derive(Clone, Default)]
struct CfgModel {
    base_channel_count: u32,
    /// `downmixInstructions[]` as stored: (downmixId, targetChannelCount)
    downmix: [(u32, u32); 6],
    downmix_count: usize,
    downmix_count_v0: u32,
    /// `drcCoefficientsUniDrc[]` as stored: (drcLocation, bandCount of the stored gain sets)
    coef: [(u32, Vec<u32>); 2],
    coef_count: usize,
    coef_count_v0: u32,
    inst_count: usize,
}

impl CfgModel {
    fn select_coef(&self, location: u32) -> Option<&(u32, Vec<u32>)> {
        let mut c = None;
        for n in 0..self.coef_count {
            if self.coef[n].0 == location {
                c = Some(&self.coef[n]);
            }
        }
        c
    }

    fn select_downmix(&self, id: u32) -> Option<u32> {
        (0..self.downmix_count).find(|&i| self.downmix[i].0 == id).map(|i| self.downmix[i].1)
    }
}

fn gen_drc_set_effect(rng: &mut Rng) -> u32 {
    match rng.next_u32() % 16 {
        0 => 0,
        1 => 0x0400, /* EB_DUCK_OTHER */
        2 => 0x0800, /* EB_DUCK_SELF */
        3 => 0x0200, /* EB_FADE */
        4 => 0x0100, /* EB_CLIPPING */
        5 => rng.next_u32() & 0xffff,
        6..=8 => 1 << (rng.next_u32() % 8),
        _ => (1 << (rng.next_u32() % 8)) | (1 << (rng.next_u32() % 8)) | 0x20,
    }
}

fn gen_downmix_instructions(rng: &mut Rng, w: &mut BitWriter, version: u32, m: &CfgModel) -> (u32, u32) {
    let id = if chance(rng, 1, 8) { rng.next_u32() & 0x7f } else { pick(rng, 1, 6) };
    let bcc = m.base_channel_count;
    let tcc = if rare(rng, 1, 16) || bcc == 0 { rng.next_u32() & 0x7f } else { pick(rng, 1, bcc) };
    w.put(id, 7);
    w.put(tcc, 7);
    w.put(rng.next_u32() & 0xff, 8);
    let present = chance(rng, 1, 2);
    w.put(present as u32, 1);
    if present {
        let n = tcc * bcc;
        if n <= 64 {
            if version == 0 {
                for _ in 0..n {
                    w.put(rng.next_u32() & 0xf, 4);
                }
            } else {
                w.put(if chance(rng, 3, 4) { pick(rng, 0, 3) } else { rng.next_u32() & 0xf }, 4);
                for _ in 0..n {
                    w.put(rng.next_u32() & 0x1f, 5);
                }
            }
        }
    }
    (id, tcc)
}

fn gen_skip_drc_instructions_basic(rng: &mut Rng, w: &mut BitWriter) {
    w.put(rng.next_u32() & 0x3f, 6);
    w.put(rng.next_u32() & 0xf, 4);
    w.put(rng.next_u32() & 0x7f, 7);
    let add = chance(rng, 1, 4);
    w.put(add as u32, 1);
    if add {
        let n = rng.next_u32() & 7;
        w.put(n, 3);
        w.put(rng.next_u32() & ((1 << (7 * n.min(4))) - 1), 7 * n.min(4));
        w.put(0, 7 * n.saturating_sub(4));
    }
    let effect = gen_drc_set_effect(rng);
    w.put(effect, 16);
    if effect & 0x0c00 == 0 {
        let p = chance(rng, 1, 2);
        w.put(p as u32, 1);
        if p {
            w.put(rng.next_u32() & 0xff, 8);
        }
    }
    let tl = chance(rng, 1, 2);
    w.put(tl as u32, 1);
    if tl {
        w.put(rng.next_u32() & 0x3f, 6);
        let lo = chance(rng, 1, 2);
        w.put(lo as u32, 1);
        if lo {
            w.put(rng.next_u32() & 0x3f, 6);
        }
    }
}

fn gen_custom_char(rng: &mut Rng, w: &mut BitWriter) {
    let nodes = chance(rng, 1, 2);
    w.put(nodes as u32, 1);
    if !nodes {
        w.put(rng.next_u32() & 0x3f, 6);
        w.put(rng.next_u32() & 0xf, 4);
        w.put(rng.next_u32() & 0xf, 4);
        w.put(rng.next_u32() & 1, 1);
    } else {
        let n = rng.next_u32() & 3;
        w.put(n, 2);
        for _ in 0..=n {
            w.put(rng.next_u32() & 0x1f, 5);
            w.put(rng.next_u32() & 0xff, 8);
        }
    }
}

/// Writes a gainSet(); returns its band count and the last gain sequence index.
fn gen_gain_set(rng: &mut Rng, w: &mut BitWriter, version: u32, seq_index: &mut i32) -> u32 {
    let gcp = match rng.next_u32() % 20 {
        0..=9 => 0,
        10..=12 => 1,
        13..=16 => 2,
        _ => 3,
    };
    w.put(gcp, 2);
    w.put(rng.next_u32() & 1, 1); /* gainInterpolationType */
    w.put(rng.next_u32() & 1, 1); /* fullFrame */
    w.put(rng.next_u32() & 1, 1); /* timeAlignment */
    let tdmp = chance(rng, 1, 4);
    w.put(tdmp as u32, 1);
    if tdmp {
        let v = match rng.next_u32() % 4 {
            0 => rng.next_u32() & 0x7ff,
            1 => pick(rng, 0, 63),
            _ => (1 << pick(rng, 0, 6)) - 1,
        };
        w.put(v, 11);
    }
    if gcp != 3 {
        let bc = if rare(rng, 1, 20) {
            rng.next_u32() & 0xf
        } else if chance(rng, 12, 19) {
            1
        } else {
            pick(rng, 2, 4)
        };
        w.put(bc, 4);
        if bc > 4 {
            return bc;
        }
        let band_type = rng.next_u32() & 1;
        if bc > 1 {
            w.put(band_type, 1);
        }
        for _ in 0..bc {
            if version == 0 {
                *seq_index += 1;
                w.put(if chance(rng, 7, 8) { pick(rng, 0, 11) } else { rng.next_u32() & 0x7f }, 7);
            } else {
                let ip = chance(rng, 1, 6);
                w.put(ip as u32, 1);
                if ip {
                    let idx = if chance(rng, 3, 4) { pick(rng, 0, 13) } else { rng.next_u32() & 0x3f };
                    w.put(idx, 6);
                    *seq_index = idx as i32;
                } else {
                    *seq_index += 1;
                }
                let present = chance(rng, 2, 3);
                w.put(present as u32, 1);
                if present {
                    let cicp = chance(rng, 1, 2);
                    w.put(cicp as u32, 1);
                    if cicp {
                        w.put(if chance(rng, 7, 8) { pick(rng, 0, 11) } else { rng.next_u32() & 0x7f }, 7);
                    } else {
                        w.put(pick(rng, 0, 4), 4);
                        w.put(pick(rng, 0, 4), 4);
                    }
                }
            }
        }
        for _ in 1..bc {
            if band_type != 0 {
                w.put(rng.next_u32() & 0xf, 4);
            } else {
                w.put(rng.next_u32() & 0x3ff, 10);
            }
        }
        bc
    } else {
        *seq_index += 1;
        1
    }
}

/// Writes drcCoefficientsUniDrc(); returns (drcLocation, band counts of the first 12 sets).
fn gen_coefficients(rng: &mut Rng, w: &mut BitWriter, version: u32) -> (u32, Vec<u32>) {
    let loc = if !rare(rng, 1, 8) { 1 } else { rng.next_u32() & 0xf };
    w.put(loc, 4);
    let fsp = chance(rng, 1, 4);
    w.put(fsp as u32, 1);
    if fsp {
        w.put(if chance(rng, 1, 2) { 1023 } else { rng.next_u32() & 0x7fff }, 15);
    }
    let gsc = if rare(rng, 1, 16) {
        pick(rng, 5, 63)
    } else if chance(rng, 1, 16) {
        0
    } else {
        pick(rng, 1, 4)
    };
    let mut bands = vec![];
    let mut seq_index = -1;
    if version == 0 {
        w.put(gsc, 6);
        for i in 0..gsc {
            let bc = gen_gain_set(rng, w, version, &mut seq_index);
            if bc > 4 {
                break;
            }
            if i < 12 {
                bands.push(bc);
            }
        }
    } else {
        for _ in 0..2 {
            let p = chance(rng, 1, 3);
            w.put(p as u32, 1);
            if p {
                let n = pick(rng, 0, 15);
                w.put(n, 4);
                for _ in 0..n {
                    gen_custom_char(rng, w);
                }
            }
        }
        let sfp = chance(rng, 1, 4);
        w.put(sfp as u32, 1);
        if sfp {
            let n = rng.next_u32() & 0xf;
            w.put(n, 4);
            for _ in 0..n * 4 {
                let p = chance(rng, 1, 2);
                w.put(p as u32, 1);
                if p {
                    w.put(rng.next_u32() & 0x1f, 5);
                }
            }
        }
        let mut sets = BitWriter::new();
        let mut max_seq = -1;
        for i in 0..gsc {
            let bc = gen_gain_set(rng, &mut sets, version, &mut seq_index);
            max_seq = max_seq.max(seq_index);
            if bc > 4 {
                break;
            }
            if i < 12 {
                bands.push(bc);
            }
        }
        let n_seq = if !rare(rng, 1, 8) { (max_seq + 1).max(0) as u32 } else { rng.next_u32() & 0x3f };
        w.put(n_seq & 0x3f, 6);
        w.put(gsc, 6);
        w.append(&sets);
    }
    (loc, bands)
}

fn gen_gain_modification(rng: &mut Rng, w: &mut BitWriter, version: u32, band_count: u32) {
    if version > 0 {
        for _ in 0..band_count {
            for _ in 0..2 {
                let p = chance(rng, 1, 4);
                w.put(p as u32, 1);
                if p {
                    w.put(pick(rng, 0, 4), 4);
                }
            }
            let gsp = chance(rng, 1, 3);
            w.put(gsp as u32, 1);
            if gsp {
                w.put(rng.next_u32() & 0xff, 8);
            }
            let gop = chance(rng, 1, 3);
            w.put(gop as u32, 1);
            if gop {
                w.put(rng.next_u32() & 0x3f, 6);
            }
        }
        if band_count == 1 {
            let sfp = chance(rng, 1, 4);
            w.put(sfp as u32, 1);
            if sfp {
                w.put(rng.next_u32() & 0xf, 4);
            }
        }
    } else {
        let gsp = chance(rng, 1, 3);
        w.put(gsp as u32, 1);
        if gsp {
            w.put(rng.next_u32() & 0xff, 8);
        }
        let gop = chance(rng, 1, 3);
        w.put(gop as u32, 1);
        if gop {
            w.put(rng.next_u32() & 0x3f, 6);
        }
    }
}

/// A gain set index (`bsGainSetIndex - 1`) for the instructions: mostly valid.
fn gen_gain_set_index(rng: &mut Rng, m: &CfgModel, location: u32) -> u32 {
    let n = m.select_coef(location).map_or(0, |c| c.1.len() as u32);
    if rare(rng, 1, 16) {
        return rng.next_u32() & 0x3f;
    }
    match rng.next_u32() % 16 {
        0 => 0, /* -1 */
        _ => {
            if n > 0 {
                pick(rng, 1, n)
            } else {
                pick(rng, 0, 2)
            }
        }
    }
}

fn gen_instructions(rng: &mut Rng, w: &mut BitWriter, version: u32, m: &CfgModel) {
    w.put(if chance(rng, 7, 8) { pick(rng, 1, 12) } else { rng.next_u32() & 0x3f }, 6);
    if version > 0 {
        w.put(rng.next_u32() & 0xf, 4);
    }
    let loc = if !rare(rng, 1, 8) { 1 } else { rng.next_u32() & 0xf };
    w.put(loc, 4);
    let downmix_id_present = if version == 0 {
        true
    } else {
        let p = chance(rng, 3, 4);
        w.put(p as u32, 1);
        p
    };
    let mut downmix_id = 0;
    let mut apply = 0;
    let mut downmix_id_count = 1;
    if downmix_id_present {
        downmix_id = if rare(rng, 1, 10) {
            rng.next_u32() & 0x7f
        } else {
            match rng.next_u32() % 9 {
                0..=5 => 0,
                6 | 7 if m.downmix_count > 0 => {
                    m.downmix[(rng.next_u32() as usize) % m.downmix_count].0
                }
                8 => 0x7f,
                _ => 0,
            }
        };
        w.put(downmix_id, 7);
        if version == 0 {
            apply = (downmix_id != 0) as u32;
        } else {
            apply = rng.next_u32() & 1;
            w.put(apply, 1);
        }
        let add = chance(rng, 1, 8);
        w.put(add as u32, 1);
        if add {
            let n = rng.next_u32() & 7;
            w.put(n, 3);
            for _ in 0..n {
                w.put(rng.next_u32() & 0x7f, 7);
            }
            downmix_id_count = 1 + n;
        }
    }
    let effect = gen_drc_set_effect(rng);
    w.put(effect, 16);
    if effect & 0x0c00 == 0 {
        let p = chance(rng, 1, 3);
        w.put(p as u32, 1);
        if p {
            w.put(rng.next_u32() & 0xff, 8);
        }
    }
    let tl = chance(rng, 1, 3);
    w.put(tl as u32, 1);
    if tl {
        w.put(rng.next_u32() & 0x3f, 6);
        let lo = chance(rng, 1, 2);
        w.put(lo as u32, 1);
        if lo {
            w.put(rng.next_u32() & 0x3f, 6);
        }
    }
    let dep = chance(rng, 1, 6);
    w.put(dep as u32, 1);
    if dep {
        w.put(if chance(rng, 3, 4) { pick(rng, 1, 12) } else { rng.next_u32() & 0x3f }, 6);
    } else {
        w.put(chance(rng, 1, 8) as u32, 1);
    }
    if version > 0 {
        w.put(chance(rng, 1, 16) as u32, 1);
    }

    let bcc = m.base_channel_count;
    if effect & 0x0c00 != 0 {
        /* ducking */
        let cc = bcc;
        let mut c = 0;
        // EB_DUCK_OTHER needs one gain set index (>= 0) for the ducked channels and -1 for the
        // others.
        let mut seq = gen_gain_set_index(rng, m, loc).max(1);
        if rare(rng, 1, 8) {
            seq = gen_gain_set_index(rng, m, loc);
        }
        while c < cc {
            let idx = if effect & 0x0400 != 0 && !rare(rng, 1, 8) {
                if c == 0 || chance(rng, 1, 2) {
                    seq
                } else {
                    0
                }
            } else {
                gen_gain_set_index(rng, m, loc)
            };
            w.put(idx, 6);
            let p = chance(rng, 1, 2);
            w.put(p as u32, 1);
            if p {
                w.put(rng.next_u32() & 0xf, 4);
            }
            c += 1;
            let rep = c < cc && chance(rng, 1, 3);
            w.put(rep as u32, 1);
            if rep {
                let r = pick(rng, 1, cc - c).min(32);
                w.put(r - 1, 5);
                c += r;
            }
        }
        return;
    }
    let mut cc = bcc;
    let mut drc_channel_count = bcc;
    let mut derive = false;
    let v0_or_apply = version == 0 || apply != 0;
    if v0_or_apply && downmix_id != 0 && downmix_id != 0x7f && downmix_id_count == 1 {
        if m.downmix_count != 0 {
            match m.select_downmix(downmix_id) {
                Some(t) => {
                    cc = t;
                    drc_channel_count = t;
                }
                None => return, /* DE_NOT_OK */
            }
        } else {
            derive = true;
            cc = 1;
        }
    } else if v0_or_apply && (downmix_id == 0x7f || downmix_id_count > 1) {
        drc_channel_count = 8;
        cc = 1;
    }
    let mut gsi = [-1i32; 8];
    let mut c = 0;
    while c < cc {
        let idx = gen_gain_set_index(rng, m, loc);
        w.put(idx, 6);
        if c < 8 {
            gsi[c as usize] = idx as i32 - 1;
        }
        c += 1;
        let rep = if derive { chance(rng, 1, 2) } else { c < cc && chance(rng, 1, 3) };
        w.put(rep as u32, 1);
        if rep {
            let r = if derive { pick(rng, 1, 7) } else { pick(rng, 1, cc - c).min(32) };
            w.put(r - 1, 5);
            if derive {
                cc = 1 + r;
            }
            for _ in 0..r {
                if c < 8 {
                    gsi[c as usize] = idx as i32 - 1;
                }
                c += 1;
            }
        }
    }
    if derive {
        drc_channel_count = cc;
    }
    if downmix_id == 0x7f || downmix_id_count > 1 {
        for c in 1..(drc_channel_count as usize).min(8) {
            gsi[c] = gsi[0];
        }
    }
    let mut groups: Vec<i32> = vec![];
    for c in 0..(drc_channel_count as usize).min(8) {
        if gsi[c] >= 0 && !groups.contains(&gsi[c]) {
            groups.push(gsi[c]);
        }
    }
    for g in groups {
        let bc = match m.select_coef(loc) {
            Some(coef) if (g as usize) < coef.1.len() => coef.1[g as usize],
            _ => 1,
        };
        gen_gain_modification(rng, w, version, bc);
    }
}

fn gen_skip_loud_eq_instructions(rng: &mut Rng, w: &mut BitWriter) {
    w.put(rng.next_u32() & 0xff, 8);
    for bits in [7u32, 6, 6] {
        let p = chance(rng, 1, 2);
        w.put(p as u32, 1);
        if p {
            w.put(rng.next_u32() & ((1 << bits) - 1), bits);
            let a = chance(rng, 1, 4);
            w.put(a as u32, 1);
            if a {
                let n = pick(rng, 0, 3);
                w.put(n, bits);
                for _ in 0..n {
                    w.put(rng.next_u32() & ((1 << bits) - 1), bits);
                }
            }
        }
    }
    w.put(rng.next_u32() & 3, 2);
    let n = pick(rng, 0, 3);
    w.put(n, 6);
    for _ in 0..n {
        w.put(rng.next_u32() & 0x3f, 6);
        let cicp = chance(rng, 1, 2);
        w.put(cicp as u32, 1);
        w.put(rng.next_u32() & 0x7f, if cicp { 7 } else { 8 });
        w.put(rng.next_u32() & 0x3fff, 14);
    }
}

fn gen_skip_eq_coefficients(rng: &mut Rng, w: &mut BitWriter) {
    let p = chance(rng, 1, 2);
    w.put(p as u32, 1);
    if p {
        w.put(rng.next_u32() & 0xff, 8);
    }
    let n = pick(rng, 0, 2);
    w.put(n, 6);
    for _ in 0..n {
        let k = pick(rng, 0, 2);
        w.put(k, 6);
        for _ in 0..k {
            w.put(rng.next_u32() & 0x3f, 6);
            let gp = chance(rng, 1, 2);
            w.put(gp as u32, 1);
            if gp {
                w.put(rng.next_u32() & 0x3ff, 10);
            }
        }
    }
    let n = pick(rng, 0, 2);
    w.put(n, 6);
    for _ in 0..n {
        let fir = chance(rng, 1, 2);
        w.put(fir as u32, 1);
        if !fir {
            let counts = [pick(rng, 0, 2), pick(rng, 0, 2), pick(rng, 0, 2), pick(rng, 0, 2), pick(rng, 0, 2)];
            for (c, b) in counts.iter().zip([3u32, 6, 6, 4, 4]) {
                w.put(*c, b);
            }
            let bits = 2 * counts[0] + 8 * counts[1] + 14 * counts[2] + 8 * counts[3] + 14 * counts[4];
            for _ in 0..bits {
                w.put(rng.next_u32() & 1, 1);
            }
        } else {
            let order = pick(rng, 0, 8);
            w.put(order, 7);
            w.put(rng.next_u32() & 1, 1);
            for _ in 0..(order / 2 + 1) * 11 {
                w.put(rng.next_u32() & 1, 1);
            }
        }
    }
    let n = pick(rng, 0, 1);
    w.put(n, 6);
    if n > 0 {
        let rep = rng.next_u32() & 1;
        w.put(rep, 1);
        let format = pick(rng, 1, 8);
        w.put(format, 4);
        let count = match format {
            1 => 32,
            2 => 39,
            3 => 64,
            4 => 71,
            5 => 128,
            6 => 135,
            _ => {
                let c = pick(rng, 0, 20);
                w.put(c, 8);
                c + 1
            }
        };
        for _ in 0..n {
            if rep == 1 {
                let nodes = pick(rng, 0, 3);
                w.put(nodes, 5);
                for _ in 0..nodes + 2 {
                    let b = chance(rng, 1, 2);
                    w.put(b as u32, 1);
                    if !b {
                        w.put(rng.next_u32() & 0xf, 4);
                    }
                }
                w.put(0, 4 * (nodes + 1));
                let s = rng.next_u32() & 3;
                w.put(s, 2);
                w.put(0, [5, 4, 4, 3][s as usize]);
                w.put(0, 5 * (nodes + 1));
            } else {
                for _ in 0..count * 9 {
                    w.put(rng.next_u32() & 1, 1);
                }
            }
        }
    }
}

fn gen_skip_eq_instructions(rng: &mut Rng, w: &mut BitWriter, m: &CfgModel) {
    w.put(rng.next_u32() & 0x3ff, 10);
    let dp = chance(rng, 1, 2);
    w.put(dp as u32, 1);
    let mut downmix_id = 0;
    let mut apply = 0;
    let mut add_count = 0;
    if dp {
        downmix_id = match rng.next_u32() % 4 {
            0 if m.downmix_count > 0 => m.downmix[(rng.next_u32() as usize) % m.downmix_count].0,
            1 => 0x7f,
            _ => 0,
        };
        w.put(downmix_id, 7);
        apply = rng.next_u32() & 1;
        w.put(apply, 1);
        let a = chance(rng, 1, 4);
        w.put(a as u32, 1);
        if a {
            add_count = pick(rng, 0, 2);
            w.put(add_count, 7);
            w.put(0, 7 * add_count);
        }
    }
    w.put(rng.next_u32() & 0x3f, 6);
    let a = chance(rng, 1, 4);
    w.put(a as u32, 1);
    if a {
        let n = pick(rng, 0, 2);
        w.put(n, 6);
        w.put(0, 6 * n);
    }
    w.put(rng.next_u32() & 0xffff, 16);
    let dep = chance(rng, 1, 2);
    w.put(dep as u32, 1);
    w.put(rng.next_u32() & 0x3f, if dep { 6 } else { 1 });
    let mut cc = m.base_channel_count;
    if dp && apply == 1 && downmix_id != 0 && downmix_id != 0x7f && add_count == 0 {
        match m.select_downmix(downmix_id) {
            Some(t) => cc = t,
            None => return,
        }
    } else if downmix_id == 0x7f || add_count > 1 {
        cc = 1;
    }
    let mut groups = vec![];
    for _ in 0..cc.min(9) {
        let g = pick(rng, 0, 3);
        w.put(g, 7);
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    let n_groups = groups.len() as u32;
    let td = chance(rng, 1, 2);
    w.put(td as u32, 1);
    if td {
        for _ in 0..n_groups {
            let p = chance(rng, 1, 2);
            w.put(p as u32, 1);
            if p {
                w.put(rng.next_u32() & 0x3ff, 10);
            }
            let n = pick(rng, 0, 3);
            w.put(n, 4);
            w.put(0, 7 * n);
        }
        let pa = chance(rng, 1, 2);
        w.put(pa as u32, 1);
        if pa {
            for i in 0..n_groups {
                w.put(0, n_groups - i - 1);
            }
        }
    }
    let sg = chance(rng, 1, 2);
    w.put(sg as u32, 1);
    if sg {
        w.put(0, 6 * n_groups);
    }
    let t = chance(rng, 1, 2);
    w.put(t as u32, 1);
    if t {
        w.put(rng.next_u32() & 0x1f, 5);
    }
}

fn gen_extension_v1(rng: &mut Rng, m: &mut CfgModel) -> BitWriter {
    let mut w = BitWriter::new();
    let dp = chance(rng, 1, 2);
    w.put(dp as u32, 1);
    if dp {
        let n = pick(rng, 0, 3);
        w.put(n, 7);
        let offset = m.downmix_count_v0 as usize;
        m.downmix_count = (offset + n as usize).min(6);
        for i in 0..n as usize {
            let d = gen_downmix_instructions(rng, &mut w, 1, m);
            if offset + i < 6 {
                m.downmix[offset + i] = d;
            }
        }
    }
    let cp = chance(rng, 2, 3);
    w.put(cp as u32, 1);
    if cp {
        let n = pick(rng, 0, 2);
        w.put(n, 3);
        let offset = m.coef_count_v0 as usize;
        m.coef_count = (offset + n as usize).min(2);
        for i in 0..n as usize {
            let coef = gen_coefficients(rng, &mut w, 1);
            if offset + i < 2 {
                m.coef[offset + i] = coef;
            }
        }
        let ni = pick(rng, 0, 4);
        w.put(ni, 6);
        let count = (m.inst_count + ni as usize).min(12);
        m.inst_count = count;
        for _ in 0..count {
            gen_instructions(rng, &mut w, 1, m);
        }
    }
    let le = chance(rng, 1, 6);
    w.put(le as u32, 1);
    if le {
        let n = pick(rng, 0, 2);
        w.put(n, 4);
        for _ in 0..n {
            gen_skip_loud_eq_instructions(rng, &mut w);
        }
    }
    let eq = chance(rng, 1, 6);
    w.put(eq as u32, 1);
    if eq {
        gen_skip_eq_coefficients(rng, &mut w);
        let n = pick(rng, 0, 2);
        w.put(n, 4);
        for _ in 0..n {
            gen_skip_eq_instructions(rng, &mut w, m);
        }
    }
    w
}

/// A random uniDrcConfig() payload.
pub fn gen_uni_drc_config(rng: &mut Rng) -> BitWriter {
    let mut w = BitWriter::new();
    let mut m = CfgModel::default();
    let srp = chance(rng, 1, 2);
    w.put(srp as u32, 1);
    if srp {
        w.put(rng.next_u32() & 0x3ffff, 18);
    }
    let dmx_v0 = match rng.next_u32() % 10 {
        0..=3 => 0,
        4..=8 => pick(rng, 1, 3),
        _ => pick(rng, 4, 8),
    };
    w.put(dmx_v0, 7);
    let basic = chance(rng, 1, 8);
    w.put(basic as u32, 1);
    let (cb, ib) = if basic { (pick(rng, 0, 2), pick(rng, 0, 2)) } else { (0, 0) };
    if basic {
        w.put(cb, 3);
        w.put(ib, 4);
    }
    let coef_v0 = match rng.next_u32() % 16 {
        0 => 0,
        1 => pick(rng, 2, 4),
        _ => 1,
    };
    w.put(coef_v0, 3);
    let inst_v0 = match rng.next_u32() % 16 {
        0 => 0,
        1 => pick(rng, 6, 15),
        _ => pick(rng, 1, 5),
    };
    w.put(inst_v0, 6);
    let bcc = if rare(rng, 1, 64) {
        0
    } else if rare(rng, 1, 64) {
        pick(rng, 9, 127)
    } else {
        pick(rng, 1, 8)
    };
    m.base_channel_count = bcc;
    w.put(bcc, 7);
    let ls = chance(rng, 1, 4);
    w.put(ls as u32, 1);
    if ls {
        let dl = if chance(rng, 1, 2) { 0 } else { rng.next_u32() & 0xff };
        w.put(dl, 8);
        if dl == 0 {
            for _ in 0..bcc {
                w.put(rng.next_u32() & 0x7f, 7);
            }
        }
    }
    m.downmix_count_v0 = dmx_v0;
    m.downmix_count = (dmx_v0 as usize).min(6);
    for i in 0..dmx_v0 as usize {
        let d = gen_downmix_instructions(rng, &mut w, 0, &m);
        if i < 6 {
            m.downmix[i] = d;
        }
    }
    for _ in 0..cb {
        w.put(rng.next_u32() & 0x7ff, 11);
    }
    for _ in 0..ib {
        gen_skip_drc_instructions_basic(rng, &mut w);
    }
    m.coef_count_v0 = coef_v0;
    m.coef_count = (coef_v0 as usize).min(2);
    for i in 0..coef_v0 as usize {
        let coef = gen_coefficients(rng, &mut w, 0);
        if i < 2 {
            m.coef[i] = coef;
        }
    }
    m.inst_count = (inst_v0 as usize).min(12);
    for _ in 0..inst_v0 {
        gen_instructions(rng, &mut w, 0, &m);
    }
    let ext = chance(rng, 1, 2);
    w.put(ext as u32, 1);
    if ext {
        for _ in 0..pick(rng, 1, 2) {
            match rng.next_u32() % 6 {
                0 => {
                    let mut p = BitWriter::new();
                    for _ in 0..pick(rng, 1, 40) {
                        p.put(rng.next_u32() & 1, 1);
                    }
                    let t = pick(rng, 1, 15);
                put_ext(rng, &mut w, t, 4, &p);
                }
                _ => {
                    let p = gen_extension_v1(rng, &mut m);
                    put_ext(rng, &mut w, 2, 4, &p);
                }
            }
        }
        w.put(0, 4);
    }
    w
}

// ---------------------------------------------------------------------------------------------
// Generator of loudnessInfoSet()
// ---------------------------------------------------------------------------------------------

/// Writes loudnessInfo() exactly.
fn gen_loudness_info_exact(rng: &mut Rng, w: &mut BitWriter, version: u32) {
    w.put(
        match rng.next_u32() % 8 {
            0 => 0x3f,
            1 => rng.next_u32() & 0x3f,
            _ => pick(rng, 0, 6),
        },
        6,
    );
    if version >= 1 {
        w.put(rng.next_u32() & 0x3f, 6);
    }
    w.put(
        match rng.next_u32() % 8 {
            0 => 0x7f,
            1 => rng.next_u32() & 0x7f,
            2 | 3 => pick(rng, 1, 6),
            _ => 0,
        },
        7,
    );
    let sp = chance(rng, 1, 2);
    w.put(sp as u32, 1);
    if sp {
        w.put(if chance(rng, 1, 8) { 0 } else { rng.next_u32() & 0xfff }, 12);
    }
    let tp = chance(rng, 1, 2);
    w.put(tp as u32, 1);
    if tp {
        w.put(if chance(rng, 1, 8) { 0 } else { rng.next_u32() & 0xfff }, 12);
        w.put(rng.next_u32() & 0xf, 4);
        w.put(rng.next_u32() & 3, 2);
    }
    let n = match rng.next_u32() % 16 {
        0 => pick(rng, 9, 15),
        1 | 2 => 0,
        _ => pick(rng, 1, 4),
    };
    w.put(n, 4);
    for _ in 0..n {
        let md = if !rare(rng, 1, 32) { pick(rng, 0, 9) } else { pick(rng, 10, 15) };
        w.put(md, 4);
        match md {
            7 => w.put(rng.next_u32() & 0x1f, 5),
            8 => w.put(rng.next_u32() & 3, 2),
            0..=9 => w.put(rng.next_u32() & 0xff, 8),
            _ => {}
        }
        w.put(if chance(rng, 7, 8) { pick(rng, 0, 11) } else { rng.next_u32() & 0xf }, 4);
        w.put(rng.next_u32() & 3, 2);
    }
}

/// A random loudnessInfoSet() payload.
pub fn gen_loudness_info_set(rng: &mut Rng) -> BitWriter {
    let mut w = BitWriter::new();
    let count = |rng: &mut Rng| match rng.next_u32() % 16 {
        0 => pick(rng, 10, 20),
        1..=4 => 0,
        _ => pick(rng, 1, 4),
    };
    let na = count(rng);
    let ni = count(rng);
    w.put(na, 6);
    w.put(ni, 6);
    for _ in 0..na + ni {
        gen_loudness_info_exact(rng, &mut w, 0);
    }
    let ext = chance(rng, 1, 3);
    w.put(ext as u32, 1);
    if ext {
        for _ in 0..pick(rng, 1, 2) {
            if chance(rng, 3, 4) {
                let mut p = BitWriter::new();
                let na = count(rng);
                let ni = count(rng);
                p.put(na, 6);
                p.put(ni, 6);
                for _ in 0..na + ni {
                    gen_loudness_info_exact(rng, &mut p, 1);
                }
                put_ext(rng, &mut w, 1, 4, &p);
            } else {
                let mut p = BitWriter::new();
                for _ in 0..pick(rng, 1, 40) {
                    p.put(rng.next_u32() & 1, 1);
                }
                let t = pick(rng, 2, 15);
                put_ext(rng, &mut w, t, 4, &p);
            }
        }
        w.put(0, 4);
    }
    w
}

// ---------------------------------------------------------------------------------------------
// Generator of uniDrcGain()
// ---------------------------------------------------------------------------------------------

fn get_z(n: i32) -> u32 {
    let two_n = n.wrapping_mul(2);
    let mut z = 1u32;
    while z < 31 && (1i32 << z) < two_n {
        z += 1;
    }
    z
}

fn gen_gain_initial(rng: &mut Rng, w: &mut BitWriter, gcp: u32) {
    match gcp {
        0 => {
            w.put(rng.next_u32() & 1, 1);
            w.put(if chance(rng, 3, 4) { pick(rng, 0, 120) } else { rng.next_u32() & 0xff }, 8);
        }
        1 => {
            let s = chance(rng, 1, 2);
            w.put(s as u32, 1);
            if s {
                w.put(if chance(rng, 3, 4) { pick(rng, 0, 120) } else { rng.next_u32() & 0x3ff }, 10);
            }
        }
        2 => {
            let s = chance(rng, 1, 2);
            w.put(s as u32, 1);
            if s {
                w.put(if chance(rng, 3, 4) { pick(rng, 0, 120) } else { rng.next_u32() & 0xff }, 8);
            }
        }
        _ => {}
    }
}

fn gen_time_delta(rng: &mut Rng, w: &mut BitWriter, d: u32, z: u32) {
    if d <= 1 {
        w.put(0, 2);
    } else if d <= 5 {
        w.put(1, 2);
        w.put(d - 2, 2);
    } else if d <= 13 {
        w.put(2, 2);
        w.put(d - 6, 3);
    } else {
        w.put(3, 2);
        let v = (d - 14).min(((1u64 << z) - 1) as u32);
        let _ = rng;
        w.put(v, z);
    }
}

/// A random uniDrcGain() payload for the parsed configuration `cfg`.
pub fn gen_uni_drc_gain(
    rng: &mut Rng,
    cfg: &UniDrcConfig,
    frame_size: i32,
    delta_tmin_default: i32,
) -> BitWriter {
    let mut w = BitWriter::new();
    let Some(ci) = select_drc_coefficients(cfg, 1) else { return w };
    let coef = &cfg.drc_coefficients_uni_drc[ci];
    let count = coef.gain_sequence_count.min(12) as usize;
    for seq in 0..count {
        let index = coef.gain_set_index_for_gain_sequence[seq];
        if index >= coef.gain_set_count || index >= 12 {
            return w;
        }
        let gs = &coef.gain_set[index as usize];
        let tdm = if gs.time_delta_min_present != 0 { gs.time_delta_min as i32 } else { delta_tmin_default };
        let gcp = gs.gain_coding_profile as u32;
        if gcp == 3 {
            continue;
        }
        if chance(rng, 1, 4) {
            w.put(0, 1);
            gen_gain_initial(rng, &mut w, gcp);
            continue;
        }
        w.put(1, 1);
        let n = match rng.next_u32() % 32 {
            0 => 128,
            1 => pick(rng, 17, 40),
            _ => pick(rng, 1, 8),
        };
        let zeros = if n >= 128 { 127 } else { n - 1 };
        for _ in 0..zeros {
            w.put(0, 1);
        }
        if n < 128 {
            w.put(1, 1);
        }
        if gs.gain_interpolation_type == 0 {
            for _ in 0..n {
                huffman_cw(rng, &mut w, &SLOPE_STEEPNESS_HUFFMAN);
            }
        }
        let z = if tdm != 0 { get_z(frame_size.wrapping_div(tdm)) } else { 1 };
        let frame_end_flag = if gs.full_frame == 0 {
            let f = rng.next_u32() & 1;
            w.put(f, 1);
            f
        } else {
            1
        };
        let n_deltas = if frame_end_flag == 1 { n - 1 } else { n };
        let span = if tdm > 0 { (frame_size / tdm).max(1) as u32 } else { 4 };
        for _ in 0..n_deltas {
            let d = match rng.next_u32() % 8 {
                _ if EXTREME.with(|e| e.get()) => u32::MAX,
                0 => pick(rng, 1, 20),
                1 => pick(rng, 1, 2 * span),
                _ => (span / n.max(1)).max(1) + pick(rng, 0, 2),
            };
            gen_time_delta(rng, &mut w, d, z);
        }
        gen_gain_initial(rng, &mut w, gcp);
        let book: &[[i8; 2]] = if gcp == 2 {
            &DELTA_GAIN_CODING_PROFILE_2_HUFFMAN
        } else {
            &DELTA_GAIN_CODING_PROFILE_0_1_HUFFMAN
        };
        for _ in 1..n {
            huffman_cw(rng, &mut w, book);
        }
    }
    if count == coef.gain_sequence_count as usize {
        let ext = chance(rng, 1, 8);
        w.put(ext as u32, 1);
        if ext {
            for _ in 0..pick(rng, 1, 3) {
                let mut p = BitWriter::new();
                for _ in 0..pick(rng, 1, 40) {
                    p.put(rng.next_u32() & 1, 1);
                }
                let t = pick(rng, 1, 15);
                put_ext(rng, &mut w, t, 3, &p);
            }
            w.put(0, 4);
        }
    }
    w
}

// ---------------------------------------------------------------------------------------------
// Comparisons of the parsers
// ---------------------------------------------------------------------------------------------

/// Parsed state of both implementations.
pub struct ReaderState {
    pub cfg: Box<UniDrcConfig>,
    pub lis: Box<LoudnessInfoSet>,
    pub gain: Box<UniDrcGain>,
    pub c_cfg: Vec<u8>,
    pub c_lis: Vec<u8>,
    pub c_gain: Vec<u8>,
}

impl ReaderState {
    pub fn new() -> Self {
        let cfg = Box::new(UniDrcConfig::ZERO);
        let lis = Box::new(LoudnessInfoSet::ZERO);
        let gain = Box::new(UniDrcGain::ZERO);
        let c_cfg = pod_bytes(&*cfg).to_vec();
        let c_lis = pod_bytes(&*lis).to_vec();
        let c_gain = pod_bytes(&*gain).to_vec();
        ReaderState { cfg, lis, gain, c_cfg, c_lis, c_gain }
    }

    pub fn check(&self, what: &str) {
        cmp_bytes(&format!("{what}: UNI_DRC_CONFIG"), pod_bytes(&*self.cfg), &self.c_cfg);
        cmp_bytes(&format!("{what}: LOUDNESS_INFO_SET"), pod_bytes(&*self.lis), &self.c_lis);
        cmp_bytes(&format!("{what}: UNI_DRC_GAIN"), pod_bytes(&*self.gain), &self.c_gain);
    }

    pub fn read_config(&mut self, p: &Payload) {
        let (_b, mut bs) = p.rs();
        let err = drc_dec_read_uni_drc_config(&mut bs, &mut self.cfg);
        let st = bs_state(&mut bs);
        let mut buf = p.buf.clone();
        let mut cst = [0u32; 3];
        let cerr = unsafe {
            c::cref_drc_reader_read_uni_drc_config(
                buf.as_mut_ptr(),
                buf.len() as u32,
                p.valid_bits,
                self.c_cfg.as_mut_ptr(),
                cst.as_mut_ptr(),
            )
        };
        assert_eq!(err, cerr, "drcDec_readUniDrcConfig: error");
        assert_eq!(st, cst, "drcDec_readUniDrcConfig: bitstream");
        self.check("drcDec_readUniDrcConfig");
    }

    pub fn read_loudness(&mut self, p: &Payload) {
        let (_b, mut bs) = p.rs();
        let err = drc_dec_read_loudness_info_set(&mut bs, &mut self.lis);
        let st = bs_state(&mut bs);
        let mut buf = p.buf.clone();
        let mut cst = [0u32; 3];
        let cerr = unsafe {
            c::cref_drc_reader_read_loudness_info_set(
                buf.as_mut_ptr(),
                buf.len() as u32,
                p.valid_bits,
                self.c_lis.as_mut_ptr(),
                cst.as_mut_ptr(),
            )
        };
        assert_eq!(err, cerr, "drcDec_readLoudnessInfoSet: error");
        assert_eq!(st, cst, "drcDec_readLoudnessInfoSet: bitstream");
        self.check("drcDec_readLoudnessInfoSet");
    }

    pub fn read_gain(&mut self, p: &Payload, frame_size: i32, delta_tmin: i32) {
        let (_b, mut bs) = p.rs();
        let err = drc_dec_read_uni_drc_gain(&mut bs, &self.cfg, frame_size, delta_tmin, &mut self.gain);
        let st = bs_state(&mut bs);
        let mut buf = p.buf.clone();
        let mut cst = [0u32; 3];
        let cerr = unsafe {
            c::cref_drc_reader_read_uni_drc_gain(
                buf.as_mut_ptr(),
                buf.len() as u32,
                p.valid_bits,
                self.c_cfg.as_mut_ptr(),
                frame_size,
                delta_tmin,
                self.c_gain.as_mut_ptr(),
                cst.as_mut_ptr(),
            )
        };
        assert_eq!(err, cerr, "drcDec_readUniDrcGain({frame_size}, {delta_tmin}): error");
        assert_eq!(st, cst, "drcDec_readUniDrcGain: bitstream");
        self.check("drcDec_readUniDrcGain");
    }

    pub fn read_uni_drc(&mut self, p: &Payload, frame_size: i32, delta_tmin: i32) {
        let (_b, mut bs) = p.rs();
        let err = drc_dec_read_uni_drc(
            &mut bs,
            &mut self.cfg,
            &mut self.lis,
            frame_size,
            delta_tmin,
            &mut self.gain,
        );
        let st = bs_state(&mut bs);
        let mut buf = p.buf.clone();
        let mut cst = [0u32; 3];
        let cerr = unsafe {
            c::cref_drc_reader_read_uni_drc(
                buf.as_mut_ptr(),
                buf.len() as u32,
                p.valid_bits,
                self.c_cfg.as_mut_ptr(),
                self.c_lis.as_mut_ptr(),
                frame_size,
                delta_tmin,
                self.c_gain.as_mut_ptr(),
                cst.as_mut_ptr(),
            )
        };
        assert_eq!(err, cerr, "drcDec_readUniDrc: error");
        assert_eq!(st, cst, "drcDec_readUniDrc: bitstream");
        self.check("drcDec_readUniDrc");
    }
}

/// A frame size and the matching `deltaTminDefault` as the gain decoder computes them.
pub fn gen_frame_params(rng: &mut Rng) -> (i32, i32) {
    let frame_size = match rng.next_u32() % 8 {
        0 => 2048,
        1 => 960,
        2 => 512,
        3 => pick(rng, 1, 4096) as i32,
        4 => 768,
        _ => 1024,
    };
    let sample_rate = match rng.next_u32() % 8 {
        0 => 44100,
        1 => 32000,
        2 => 96000,
        3 => 8000,
        4 => pick(rng, 0, 200000) as i32,
        _ => 48000,
    };
    let mut dt = get_delta_tmin(sample_rate);
    if chance(rng, 1, 64) {
        dt = -1;
    }
    (frame_size, dt)
}

/// Generated payload, possibly corrupted or truncated.
fn mangle(rng: &mut Rng, mut w: BitWriter) -> Payload {
    if chance(rng, 1, 8) {
        let n = pick(rng, 1, 4) as usize;
        w.corrupt(rng, n);
    }
    let valid = if chance(rng, 1, 16) { Some(rng.next_u32() % (w.bits as u32 + 1)) } else { None };
    Payload::new(&w, valid)
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn random_bitstreams() {
    let mut rng = Rng::new(0xd5c0_0001);
    let mut s = ReaderState::new();
    for i in 0..3000 {
        if i % 200 == 0 {
            s = ReaderState::new();
        }
        let p = Payload::random(&mut rng);
        let (fs, dt) = gen_frame_params(&mut rng);
        match rng.next_u32() % 4 {
            0 => s.read_config(&p),
            1 => s.read_loudness(&p),
            2 => s.read_gain(&p, fs, dt),
            _ => s.read_uni_drc(&p, fs, dt),
        }
    }
}

#[test]
fn generated_config_and_loudness() {
    let mut rng = Rng::new(0xd5c0_0002);
    let mut s = ReaderState::new();
    let mut ok = 0;
    for i in 0..4000 {
        if i % 100 == 0 {
            s = ReaderState::new();
        }
        let w = gen_uni_drc_config(&mut rng);
        let p = mangle(&mut rng, w);
        s.read_config(&p);
        // Sometimes the same payload again (diff == 0).
        if chance(&mut rng, 1, 4) {
            s.read_config(&p);
        }
        if s.cfg.drc_instructions_uni_drc_count > 0 {
            ok += 1;
        }
        let w = gen_loudness_info_set(&mut rng);
        let p = mangle(&mut rng, w);
        s.read_loudness(&p);
        if chance(&mut rng, 1, 4) {
            s.read_loudness(&p);
        }
        for _ in 0..4 {
            let (fs, dt) = gen_frame_params(&mut rng);
            let w = gen_uni_drc_gain(&mut rng, &s.cfg, fs, dt);
            let p = mangle(&mut rng, w);
            s.read_gain(&p, fs, dt);
        }
    }
    assert!(ok > 1000, "too few valid configurations: {ok}");
}

#[test]
fn generated_uni_drc() {
    let mut rng = Rng::new(0xd5c0_0003);
    let mut s = ReaderState::new();
    for i in 0..2000 {
        if i % 50 == 0 {
            s = ReaderState::new();
        }
        let (fs, dt) = gen_frame_params(&mut rng);
        let mut w = BitWriter::new();
        let lp = chance(&mut rng, 1, 2);
        w.put(lp as u32, 1);
        let mut cfg = s.cfg.clone();
        if lp {
            let cp = chance(&mut rng, 1, 2);
            w.put(cp as u32, 1);
            if cp {
                let c = gen_uni_drc_config(&mut rng);
                // Parse it on a copy to generate the matching gain payload.
                let p = Payload::new(&c, None);
                let (_b, mut bs) = p.rs();
                if drc_dec_read_uni_drc_config(&mut bs, &mut cfg) != 0 {
                    *cfg = UniDrcConfig::ZERO;
                }
                w.append(&c);
            }
            w.append(&gen_loudness_info_set(&mut rng));
        }
        w.append(&gen_uni_drc_gain(&mut rng, &cfg, fs, dt));
        let p = mangle(&mut rng, w);
        s.read_uni_drc(&p, fs, dt);
    }
}
