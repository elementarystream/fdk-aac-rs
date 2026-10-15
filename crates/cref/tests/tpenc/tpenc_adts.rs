// Differential tests of fdk_aac_rs_tpenc::tpenc_adts against the C++ reference.
//
// adtsWrite_Init() on random configurations, and frame sequences as the AAC encoder writes
// them: adtsWrite_GetHeaderBits(), adtsWrite_EncodeHeader() with random buffer fullness and
// frame lengths, payloads with overlapping CRC regions (adtsWrite_CrcStartReg/EndReg with
// fixed, zero-padded and unlimited lengths), adtsWrite_EndRawDataBlock() for 1 to 4 raw data
// blocks, with and without protection. STRUCT_ADTS is compared as its byte image (starting
// from random bytes), together with the bitstream, the buffer and the bit counts.

use core::mem::size_of;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpenc_tpenc_adts as c;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::fdk_get_valid_bits;
use fdk_aac_rs_tpenc::tpenc_adts::*;

use super::tpenc_asc::{cc_to, rand_cc, BsPair};

/// The byte images of STRUCT_ADTS on both sides.
struct Img {
    c: [u8; STRUCT_ADTS_SIZE],
    r: [u8; STRUCT_ADTS_SIZE],
}

impl Img {
    fn new(rng: &mut Rng) -> Self {
        let mut c = [0u8; STRUCT_ADTS_SIZE];
        if rng.range(0, 1) == 0 {
            for b in c.iter_mut() {
                *b = rng.next_u32() as u8;
            }
            // pCrcLookup holds a table index (a pointer in C: 8 bytes on LP64, 4 on 32-bit x86).
            let id = rng.range(0, 3) as usize;
            c[88..88 + size_of::<usize>()].copy_from_slice(&id.to_le_bytes());
        }
        Img { c, r: c }
    }

    fn rust<R>(&mut self, f: impl FnOnce(&mut StructAdts) -> R) -> R {
        let mut a = StructAdts::load(&self.r);
        let x = f(&mut a);
        a.store(&mut self.r);
        x
    }

    fn check(&self, what: &str) {
        if self.c != self.r {
            let i = (0..STRUCT_ADTS_SIZE).find(|&i| self.c[i] != self.r[i]).unwrap();
            panic!("{what}: STRUCT_ADTS differs at byte {i}: C {:?}, Rust {:?}", self.c, self.r);
        }
    }
}

fn init(rng: &mut Rng, img: &mut Img, cc: &CoderConfig) -> i32 {
    let (mut v, mut raw) = cc_to(cc);
    let rc = unsafe { c::cref_tpenc_adts_init(img.c.as_mut_ptr(), v.as_mut_ptr(), raw.as_mut_ptr()) };
    let rr = img.rust(|a| adts_write_init(a, cc));
    assert_eq!(rc, rr, "adtsWrite_Init");
    img.check("adtsWrite_Init");
    let _ = rng;
    rr
}

#[test]
fn init_random() {
    let mut rng = Rng::new(0x7e11);
    assert_eq!(unsafe { c::cref_tpenc_adts_size() } as usize, STRUCT_ADTS_SIZE);
    for _ in 0..20000 {
        let mut img = Img::new(&mut rng);
        let mut cc = rand_cc(&mut rng);
        if rng.range(0, 1) == 0 {
            cc.aot = rng.range(-1, 6);
        }
        init(&mut rng, &mut img, &cc);
        let hc = unsafe { c::cref_tpenc_adts_get_header_bits(img.c.as_mut_ptr()) };
        let hr = img.rust(adts_write_get_header_bits);
        assert_eq!(hc, hr, "adtsWrite_GetHeaderBits");
        img.check("adtsWrite_GetHeaderBits");
    }
}

/// Writes a payload with CRC regions on both sides (as bitenc.cpp does for ADTS with
/// protection: regions of 192 and 128 bits for a CPE, unlimited regions otherwise).
fn payload(rng: &mut Rng, img: &mut Img, bs: &mut BsPair, bits: i32) {
    let mut open: Vec<(i32, i32)> = Vec::new();
    let mut left = bits;
    while left > 0 {
        let n = rng.range(1, left.min(300));
        bs.pat(n, rng.next_u32());
        left -= n;
        match rng.range(0, 3) {
            0 if open.len() < 3 => {
                let m = match rng.range(0, 4) {
                    0 => 192,
                    1 => 128,
                    2 => -rng.range(1, 200),
                    3 => rng.range(1, 400),
                    _ => 0,
                };
                let rc = unsafe { c::cref_tpenc_adts_crc_start_reg(img.c.as_mut_ptr(), bs.c, m) };
                let rr = img.rust(|a| adts_write_crc_start_reg(a, &mut bs.r, m));
                assert_eq!(rc, rr, "adtsWrite_CrcStartReg({m})");
                img.check("adtsWrite_CrcStartReg");
                bs.check("adtsWrite_CrcStartReg");
                open.push((rr, m));
            }
            1 if !open.is_empty() => {
                // The encoder ends the regions in order of their start.
                let (reg, _) = open.remove(0);
                unsafe { c::cref_tpenc_adts_crc_end_reg(img.c.as_mut_ptr(), bs.c, reg) };
                img.rust(|a| adts_write_crc_end_reg(a, &mut bs.r, reg));
                img.check("adtsWrite_CrcEndReg");
                bs.check("adtsWrite_CrcEndReg");
            }
            _ => {}
        }
    }
    for (reg, _) in open {
        unsafe { c::cref_tpenc_adts_crc_end_reg(img.c.as_mut_ptr(), bs.c, reg) };
        img.rust(|a| adts_write_crc_end_reg(a, &mut bs.r, reg));
        img.check("adtsWrite_CrcEndReg");
        bs.check("adtsWrite_CrcEndReg");
    }
}

#[test]
fn frames() {
    let mut rng = Rng::new(0x7e12);
    for it in 0..1500 {
        let mut img = Img::new(&mut rng);
        let mut cc = rand_cc(&mut rng);
        cc.aot = if rng.range(0, 5) == 0 { rng.range(1, 4) } else { AOT_AAC_LC };
        cc.n_sub_frames = rng.range(1, 4);
        if rng.range(0, 3) == 0 {
            cc.sampling_rate = super::tpenc_asc::RATES[rng.range(0, 12) as usize];
        }
        if init(&mut rng, &mut img, &cc) != 0 {
            continue;
        }
        let mut bs = BsPair::new(&mut rng, 8192);
        let nraw = cc.n_sub_frames;
        for frame in 0..rng.range(1, 6) {
            for block in 0..nraw {
                let what = format!("{it}/{frame}/{block}");
                if rng.range(0, 1) == 0 {
                    let hc = unsafe { c::cref_tpenc_adts_get_header_bits(img.c.as_mut_ptr()) };
                    let hr = img.rust(adts_write_get_header_bits);
                    assert_eq!(hc, hr, "{what}: adtsWrite_GetHeaderBits");
                    img.check(&what);
                }
                let au_bits = rng.range(0, 2000) & !7;
                let bf = match rng.range(0, 3) {
                    0 => 0x7FF,
                    1 => rng.i32(),
                    _ => rng.range(0, 0x7FF),
                };
                let fl = match rng.range(0, 4) {
                    0 => rng.i32(),
                    _ => au_bits + rng.range(-8, 8),
                };
                let rc = unsafe { c::cref_tpenc_adts_encode_header(img.c.as_mut_ptr(), bs.c, bf, fl) };
                let rr = img.rust(|a| adts_write_encode_header(a, &mut bs.r, bf, fl));
                assert_eq!(rc, rr, "{what}: adtsWrite_EncodeHeader");
                img.check(&what);
                bs.check(&what);

                payload(&mut rng, &mut img, &mut bs, au_bits);

                let vb = unsafe { fdk_aac_rs_cref::tpenc_tpenc_asc::cref_tpenc_bs_valid_bits(bs.c) };
                assert_eq!(vb, fdk_get_valid_bits(&mut bs.r), "{what}: valid bits");
                let bits0 = match rng.range(0, 4) {
                    0 => rng.i32(),
                    _ => vb as i32,
                };
                let mut bc = bits0;
                let mut br = bits0;
                unsafe { c::cref_tpenc_adts_end_raw_data_block(img.c.as_mut_ptr(), bs.c, &mut bc) };
                img.rust(|a| adts_write_end_raw_data_block(a, &mut bs.r, &mut br));
                assert_eq!(bc, br, "{what}: adtsWrite_EndRawDataBlock bits");
                img.check(&what);
                bs.check(&what);
            }
            // transportEnc_GetFrame() starts the next frame.
            img.c[32..36].copy_from_slice(&0i32.to_le_bytes());
            img.r[32..36].copy_from_slice(&0i32.to_le_bytes());
        }
    }
}
