// Differential tests of fdk_aac_rs_tpdec::tpdec_adts against the C++ reference.
//
// adtsRead_DecodeHeader() on generated ADTS headers (all fields, CRC, raw data block
// positions, PCEs for channel configuration 0) and their mutations, with random previous
// headers and PCEs; adtsRead_GetRawDataBlockLength() and the CRC functions on random states.
// The complete STRUCT_ADTS, the ASC and the bitstream are compared.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpdec_tpdec_adts as c;
use fdk_aac_rs_fdk::bitstream::fdk_read_bits;
use fdk_aac_rs_fdk::crc::FdkCrcInfo;
use fdk_aac_rs_tpdec::tp_data::*;
use fdk_aac_rs_tpdec::tpdec_adts::*;
use fdk_aac_rs_tpdec::tpdec_asc::audio_specific_config_init;

use super::tp_data::{asc_dump, assert_dump_eq, flip_bits, BitWriter, Bs, DUMP_MAX};
use super::tpdec_asc::gen_pce;

const N: usize = 41;

fn crc_store(c: &FdkCrcInfo, s: &mut [i32]) {
    for (i, r) in c.crc_reg_data.iter().enumerate() {
        s[22 + 4 * i] = r.is_active as i32;
        s[23 + 4 * i] = r.max_bits;
        s[24 + 4 * i] = r.bit_buf_cnt_bits;
        s[25 + 4 * i] = r.valid_bits;
    }
    s[34] = c.reg_start as i32;
    s[35] = c.reg_stop as i32;
    s[36] = c.crc_value as i32;
    s[37] = c.crc_poly as i32;
    s[38] = c.crc_mask as i32;
    s[39] = c.start_value as i32;
    s[40] = c.crc_len as i32;
}

fn store(a: &StructAdts) -> [i32; N] {
    let mut s = [0i32; N];
    let b = &a.bs;
    let v = [
        b.mpeg_id as i32,
        b.layer as i32,
        b.protection_absent as i32,
        b.profile as i32,
        b.sample_freq_index as i32,
        b.private_bit as i32,
        b.channel_config as i32,
        b.original as i32,
        b.home as i32,
        b.copyright_id as i32,
        b.copyright_start as i32,
        b.frame_length as i32,
        b.adts_fullness as i32,
        b.num_raw_blocks as i32,
        b.num_pce_bits as i32,
        a.decoder_can_do_mpeg4 as i32,
        a.buffer_fullnes_start_flag as i32,
        a.crc_read_value as i32,
    ];
    s[..18].copy_from_slice(&v);
    for i in 0..4 {
        s[18 + i] = a.raw_data_block_dist[i] as i32;
    }
    crc_store(&a.crc_info, &mut s);
    s
}

fn load(s: &[i32; N]) -> StructAdts {
    let mut a = StructAdts::default();
    adts_read_crc_init(&mut a);
    let b = &mut a.bs;
    b.mpeg_id = s[0] as u8;
    b.layer = s[1] as u8;
    b.protection_absent = s[2] as u8;
    b.profile = s[3] as u8;
    b.sample_freq_index = s[4] as u8;
    b.private_bit = s[5] as u8;
    b.channel_config = s[6] as u8;
    b.original = s[7] as u8;
    b.home = s[8] as u8;
    b.copyright_id = s[9] as u8;
    b.copyright_start = s[10] as u8;
    b.frame_length = s[11] as u16;
    b.adts_fullness = s[12] as u16;
    b.num_raw_blocks = s[13] as u8;
    b.num_pce_bits = s[14] as u8;
    a.decoder_can_do_mpeg4 = s[15] as u8;
    a.buffer_fullnes_start_flag = s[16] as u8;
    a.crc_read_value = s[17] as u16;
    for i in 0..4 {
        a.raw_data_block_dist[i] = s[18 + i] as u16;
    }
    for i in 0..3 {
        let r = &mut a.crc_info.crc_reg_data[i];
        r.is_active = s[22 + 4 * i] as u8;
        r.max_bits = s[23 + 4 * i];
        r.bit_buf_cnt_bits = s[24 + 4 * i];
        r.valid_bits = s[25 + 4 * i];
    }
    a.crc_info.reg_start = s[34] as u32;
    a.crc_info.reg_stop = s[35] as u32;
    a.crc_info.crc_value = s[36] as u16;
    a
}

/// A random STRUCT_ADTS state (as a previous header).
fn random_state(rng: &mut Rng) -> [i32; N] {
    let mut s = [0i32; N];
    let mut ci = [0i32; N];
    unsafe { c::cref_adts_crc_init(ci.as_mut_ptr()) };
    s[0] = (rng.next_u32() & 1) as i32;
    s[1] = if rng.next_u32() % 8 == 0 { (rng.next_u32() & 3) as i32 } else { 0 };
    s[2] = (rng.next_u32() & 1) as i32;
    s[3] = (rng.next_u32() & 3) as i32;
    s[4] = (rng.next_u32() % 16) as i32;
    s[6] = (rng.next_u32() % 8) as i32;
    s[11] = (rng.next_u32() & 0x1fff) as i32;
    s[12] = (rng.next_u32() & 0x7ff) as i32;
    s[13] = (rng.next_u32() & 3) as i32;
    s[14] = (rng.next_u32() & 0xff) as i32;
    s[15] = (rng.next_u32() % 8 != 0) as i32;
    s[16] = (rng.next_u32() & 1) as i32;
    s[17] = (rng.next_u32() & 0xffff) as i32;
    for i in 0..4 {
        s[18 + i] = (rng.next_u32() & 0xffff) as i32;
    }
    s[22..].copy_from_slice(&ci[22..]);
    if rng.next_u32() % 2 == 0 {
        for i in 0..3 {
            s[22 + 4 * i] = (rng.next_u32() & 1) as i32;
            s[23 + 4 * i] = rng.next_u32() as i32 % 300;
        }
        s[34] = (rng.next_u32() % 3) as i32;
        s[35] = (rng.next_u32() % 3) as i32;
        s[36] = (rng.next_u32() & 0xffff) as i32;
    }
    s
}

/// An ADTS header after the sync word (and data following it).
fn gen_header(rng: &mut Rng, w: &mut BitWriter) {
    let prot_absent = rng.next_u32() & 1;
    let nrb = if rng.next_u32() % 4 == 0 { rng.next_u32() & 3 } else { 0 };
    let chcfg = if rng.next_u32() % 4 == 0 { 0 } else { rng.next_u32() & 7 };
    let start = w.nbits;
    w.put(rng.next_u32() & 1, 1); // id
    w.put(if rng.next_u32() % 10 == 0 { 1 + rng.next_u32() % 3 } else { 0 }, 2); // layer
    w.put(prot_absent, 1);
    w.put(if rng.next_u32() % 4 == 0 { rng.next_u32() & 3 } else { 1 }, 2); // profile
    w.put(if rng.next_u32() % 10 == 0 { rng.next_u32() & 15 } else { 3 + rng.next_u32() % 6 }, 4);
    w.put(rng.next_u32() & 1, 1);
    w.put(chcfg, 3);
    w.put(rng.next_u32() & 3, 2);
    w.put(rng.next_u32() & 3, 2);
    let fl = match rng.next_u32() % 4 {
        0 => rng.next_u32() & 0x1fff,
        1 => 7 + rng.next_u32() % 20,
        _ => 20 + rng.next_u32() % 200,
    };
    w.put(fl, 13);
    w.put(if rng.next_u32() % 2 == 0 { 0x7ff } else { rng.next_u32() & 0x7ff }, 11);
    w.put(nrb, 2);
    let _ = start;
    if prot_absent == 0 && nrb > 0 {
        for _ in 0..nrb {
            w.put(rng.next_u32() % 2000, 16);
        }
    }
    if prot_absent == 0 {
        w.put(rng.next_u32() & 0xffff, 16);
    }
    if chcfg == 0 && rng.next_u32() % 3 != 0 {
        let anchor = w.nbits;
        w.put(5, 3); // ID_PCE
        gen_pce(rng, w, anchor);
    }
    w.random_bytes((rng.next_u32() % 40) as usize, rng);
}

#[test]
fn adts_read_decode_header_() {
    let mut rng = Rng::new(0xad7);
    for it in 0..30000 {
        let mut w = BitWriter::new();
        if rng.next_u32() % 10 == 0 {
            w.random_bytes(1 + (rng.next_u32() % 30) as usize, &mut rng);
        } else {
            gen_header(&mut rng, &mut w);
        }
        let mut bytes = w.bytes.clone();
        if rng.next_u32() % 4 == 0 {
            flip_bits(&mut bytes, 1 + rng.next_u32() % 3, &mut rng);
        }
        let bits = match rng.next_u32() % 5 {
            0 => rng.next_u32() % (bytes.len() as u32 * 8 + 1),
            1 => bytes.len() as u32 * 8 + rng.next_u32() % 20000,
            _ => bytes.len() as u32 * 8,
        };
        let mut bs = Bs::new(&bytes, bits, 2048, if rng.next_u32() % 3 == 0 { Some(&mut rng) } else { None });
        let mut st = random_state(&mut rng);
        let mut a = load(&st);
        let pce = super::tpdec_asc::random_or_valid_pce(&mut rng);
        let mut asc = CSAudioSpecificConfig::zeroed();
        audio_specific_config_init(&mut asc);
        asc.m_progr_config_element = pce;
        let ign = (rng.next_u32() % 2) as i32;
        let r = adts_read_decode_header(&mut a, &mut asc, &mut bs.rs, ign);
        let mut cd = vec![0i32; DUMP_MAX];
        let mut n = 0;
        let e = unsafe {
            c::cref_adts_decode_header(st.as_mut_ptr(), pce.as_bytes().as_ptr(), bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr(), ign, cd.as_mut_ptr(), &mut n)
        };
        let what = format!("adtsRead_DecodeHeader {it}");
        assert_eq!(r, e, "{what}");
        assert_eq!(store(&a), st, "{what}: state");
        bs.check(&what);
        assert_dump_eq(&asc_dump(&asc), &cd[..n as usize], &what);

        for block in -2..6 {
            assert_eq!(
                adts_read_get_raw_data_block_length(&a, block),
                unsafe { c::cref_adts_get_raw_data_block_length(st.as_ptr(), block) },
                "{what}: GetRawDataBlockLength({block})"
            );
        }
    }
}

#[test]
fn adts_crc() {
    let mut rng = Rng::new(0xad8);
    let mut ci = [0i32; N];
    unsafe { c::cref_adts_crc_init(ci.as_mut_ptr()) };
    let mut a = StructAdts::default();
    adts_read_crc_init(&mut a);
    assert_eq!(store(&a), ci, "adtsRead_CrcInit");
    for it in 0..3000 {
        let mut st = random_state(&mut rng);
        let mut a = load(&st);
        let data: Vec<u8> = (0..64).map(|_| rng.next_u32() as u8).collect();
        let mut bs = Bs::new(&data, 512, 64, if rng.next_u32() % 2 == 0 { Some(&mut rng) } else { None });
        for step in 0..8 {
            let what = format!("adts crc {it}/{step}");
            match rng.next_u32() % 4 {
                0 => {
                    let m = [0, 16, -8, 192][(rng.next_u32() % 4) as usize];
                    let r = adts_read_crc_start_reg(&mut a, &mut bs.rs, m);
                    let e = unsafe { c::cref_adts_crc_start_reg(st.as_mut_ptr(), bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr(), m) };
                    assert_eq!(r, e, "{what}: start");
                }
                1 => {
                    let reg = (rng.next_u32() % 3) as i32;
                    adts_read_crc_end_reg(&mut a, &mut bs.rs, reg);
                    unsafe { c::cref_adts_crc_end_reg(st.as_mut_ptr(), bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr(), reg) };
                }
                2 => {
                    let r = adts_read_crc_check(&mut a);
                    let e = unsafe { c::cref_adts_crc_check(st.as_mut_ptr()) };
                    assert_eq!(r, e, "{what}: check");
                }
                _ => {
                    // read some bits (Rust), mirrored to C by copying the state
                    let n = 1 + rng.next_u32() % 32;
                    fdk_read_bits(&mut bs.rs, n);
                    bs.sync_c();
                }
            }
            assert_eq!(store(&a), st, "{what}: state");
            bs.check(&what);
        }
    }
}
