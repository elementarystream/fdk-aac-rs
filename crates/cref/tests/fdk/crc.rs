// Differential tests of fdk_aac_rs_fdk::crc against the C++ reference.
//
// Random sequences of CRC region starts and ends, interleaved with bitstream reads (reader) or
// writes (writer) on a random bitstream (see bitbuffer.rs). After every step the complete CRC
// state, the CRC value, the bitstream state and its memory are compared.

use super::bitbuffer::{state, Pair};
use super::bitstream::bitstream_op;
use fdk_aac_rs_cref::fdk_crc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::{BS_READER, BS_WRITER};
use fdk_aac_rs_fdk::crc::*;

fn rs_state(h: &FdkCrcInfo) -> [i32; 20] {
    let mut s = [0i32; 20];
    for (i, r) in h.crc_reg_data.iter().enumerate() {
        s[4 * i] = r.is_active as i32;
        s[4 * i + 1] = r.max_bits;
        s[4 * i + 2] = r.bit_buf_cnt_bits;
        s[4 * i + 3] = r.valid_bits;
    }
    s[12] = match h.p_crc_lookup {
        None => 0,
        Some(t) if core::ptr::eq(t, &CRC_LOOKUP_16_15_5_0) => 1,
        Some(t) if core::ptr::eq(t, &CRC_LOOKUP_16_15_2_0) => 2,
        Some(t) if core::ptr::eq(t, &CRC_LOOKUP_16_12_5_0) => 3,
        Some(_) => -1,
    };
    s[13] = h.crc_poly as i32;
    s[14] = h.crc_mask as i32;
    s[15] = h.start_value as i32;
    s[16] = h.crc_len as i32;
    s[17] = h.reg_start as i32;
    s[18] = h.reg_stop as i32;
    s[19] = h.crc_value as i32;
    s
}

struct Crc {
    rs: FdkCrcInfo,
    c: *mut core::ffi::c_void,
}

impl Crc {
    fn new() -> Crc {
        Crc { rs: FdkCrcInfo::default(), c: unsafe { c::cref_crc_new() } }
    }

    fn check(&self, what: &str) {
        let mut cs = [0i32; 20];
        unsafe { c::cref_crc_state(self.c, cs.as_mut_ptr()) };
        assert_eq!(rs_state(&self.rs), cs, "CRC state after {what}");
        assert_eq!(fdk_crc_get_crc(&self.rs) as u32, unsafe { c::cref_crc_get_crc(self.c) }, "FDKcrcGetCRC after {what}");
    }
}

impl Drop for Crc {
    fn drop(&mut self) {
        unsafe { c::cref_crc_free(self.c) };
    }
}

/// A random CRC configuration: the three polynomials with lookup tables, the 8-bit DRM
/// polynomial, and random polynomials and lengths.
fn config(rng: &mut Rng) -> (u32, u32, u32) {
    let start = match rng.next_u32() % 3 {
        0 => 0,
        1 => 0xFFFF,
        _ => rng.next_u32() & 0xFFFF,
    };
    let (poly, len) = match rng.next_u32() % 7 {
        0 => (0x8021, 16),
        1 => (0x8005, 16),
        2 => (0x1021, 16),
        3 => (0x001d, 8),
        4 => (rng.next_u32() & 0xFFFF, 16),
        5 => (rng.next_u32() & 0xFFFF, rng.next_u32() % 17),
        _ => ([0x8021, 0x8005, 0x1021][(rng.next_u32() % 3) as usize], rng.next_u32() % 17),
    };
    (poly, start, len)
}

fn m_bits(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 4 {
        0 => 0,
        1 => (rng.next_u32() % 300) as i32,
        2 => -((rng.next_u32() % 300) as i32),
        _ => (rng.next_u32() % 4000) as i32 - 2000,
    }
}

fn sequences(seed: u64, cases: usize, steps: usize) {
    let mut rng = Rng::new(seed);
    let mut ends = 0;
    for _ in 0..cases {
        let mut p = Pair::any(&mut rng);
        if p.rs.config_cache != BS_READER {
            // A writer that fits into its buffer (FDKinitBitStream() in crcCalc() asserts
            // ValidBits <= bufBits).
            p = Pair::random(&mut rng, 8);
            p.rs.config_cache = BS_WRITER;
            p.rs.h_bit_buf.valid_bits = rng.next_u32() % 256;
            p.cst = state(&p.rs);
        }
        let mut crc = Crc::new();
        let (poly, start, len) = config(&mut rng);
        fdk_crc_init(&mut crc.rs, poly, start, len);
        unsafe { c::cref_crc_init(crc.c, poly, start, len) };
        crc.check(&format!("FDKcrcInit({poly:#x}, {start:#x}, {len})"));
        for _ in 0..steps {
            let r = rng.next_u32() % 100;
            if r < 25 {
                let m = m_bits(&mut rng);
                let reg = fdk_crc_start_reg(&mut crc.rs, &mut p.rs, m);
                let (st, buf) = p.c();
                let e = unsafe { c::cref_crc_start_reg(crc.c, st, buf, m) };
                assert_eq!(reg, e, "FDKcrcStartReg({m})");
                p.check("FDKcrcStartReg");
                crc.check(&format!("FDKcrcStartReg({m})"));
            } else if r < 70 {
                // Reads on a reader, writes on a writer (no large jumps of the position, so that
                // the regions stay short).
                let full = p.rs.h_bit_buf.valid_bits as u64 + p.rs.bits_in_cache as u64 + 160 > p.rs.h_bit_buf.buf_bits() as u64;
                let ops: &[u32] = if p.rs.config_cache == BS_READER {
                    &[0, 0, 1, 2, 4, 12]
                } else if full {
                    continue;
                } else {
                    &[6, 6, 9, 12]
                };
                let op = ops[(rng.next_u32() as usize) % ops.len()];
                bitstream_op(&mut p, op, &mut rng);
            } else if r < 95 {
                let reg = crc.rs.reg_stop as i32;
                if crc.rs.crc_reg_data[reg as usize].is_active == 0 {
                    continue;
                }
                let ret = fdk_crc_end_reg(&mut crc.rs, &mut p.rs, reg);
                let (st, buf) = p.c();
                let e = unsafe { c::cref_crc_end_reg(crc.c, st, buf, reg) };
                assert_eq!(ret, e, "FDKcrcEndReg({reg})");
                ends += 1;
                p.check("FDKcrcEndReg");
                crc.check(&format!("FDKcrcEndReg({reg})"));
            } else {
                fdk_crc_reset(&mut crc.rs);
                unsafe { c::cref_crc_reset(crc.c) };
                crc.check("FDKcrcReset");
            }
        }
    }
    assert!(ends > cases * steps / 10, "only {ends} regions ended");
}

#[test]
fn crc_lookup_tables() {
    for (i, t) in [&CRC_LOOKUP_16_15_2_0, &CRC_LOOKUP_16_12_5_0, &CRC_LOOKUP_16_15_5_0].iter().enumerate() {
        let mut n = 0;
        let p = unsafe { c::cref_crc_table(i as i32, &mut n) };
        let ct = unsafe { std::slice::from_raw_parts(p, n as usize) };
        assert_eq!(t.as_slice(), ct, "table {i}");
    }
}

#[test]
fn fdk_crc_init_reset_get_crc() {
    let mut rng = Rng::new(201);
    let crc = &mut Crc::new();
    for _ in 0..5000 {
        let (poly, start, len) = config(&mut rng);
        fdk_crc_init(&mut crc.rs, poly, start, len);
        unsafe { c::cref_crc_init(crc.c, poly, start, len) };
        crc.check(&format!("FDKcrcInit({poly:#x}, {start:#x}, {len})"));
        fdk_crc_reset(&mut crc.rs);
        unsafe { c::cref_crc_reset(crc.c) };
        crc.check("FDKcrcReset");
    }
}

#[test]
fn fdk_crc_regions() {
    sequences(202, 3000, 40);
}

/// Longer regions and more steps per stream.
#[test]
fn fdk_crc_long_sequences() {
    sequences(203, 300, 400);
}
