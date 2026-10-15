// Differential tests of fdk_aac_rs_tpdec::tpdec_drm against the C++ reference.
//
// The DRM CRC functions on random states and random sequences of region starts, reads, region
// ends and checks. The complete STRUCT_DRM and the bitstream are compared.
// (DrmRawSdcAudioConfig_Parse() is tested in tpdec_asc.rs.)

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpdec_tpdec_drm as c;
use fdk_aac_rs_fdk::bitstream::fdk_read_bits;
use fdk_aac_rs_tpdec::tpdec_drm::*;

use super::tp_data::Bs;

fn store(d: &StructDrm) -> [i32; 20] {
    let mut s = [0i32; 20];
    for (i, r) in d.crc_info.crc_reg_data.iter().enumerate() {
        s[4 * i] = r.is_active as i32;
        s[4 * i + 1] = r.max_bits;
        s[4 * i + 2] = r.bit_buf_cnt_bits;
        s[4 * i + 3] = r.valid_bits;
    }
    s[12] = d.crc_info.reg_start as i32;
    s[13] = d.crc_info.reg_stop as i32;
    s[14] = d.crc_info.crc_value as i32;
    s[15] = d.crc_read_value as i32;
    s[16] = d.crc_info.crc_poly as i32;
    s[17] = d.crc_info.crc_mask as i32;
    s[18] = d.crc_info.start_value as i32;
    s[19] = d.crc_info.crc_len as i32;
    s
}

fn load(s: &[i32; 20]) -> StructDrm {
    let mut d = StructDrm::default();
    drm_read_crc_init(&mut d);
    for i in 0..3 {
        let r = &mut d.crc_info.crc_reg_data[i];
        r.is_active = s[4 * i] as u8;
        r.max_bits = s[4 * i + 1];
        r.bit_buf_cnt_bits = s[4 * i + 2];
        r.valid_bits = s[4 * i + 3];
    }
    d.crc_info.reg_start = s[12] as u32;
    d.crc_info.reg_stop = s[13] as u32;
    d.crc_info.crc_value = s[14] as u16;
    d.crc_read_value = s[15] as u16;
    d
}

#[test]
fn drm_crc() {
    let mut rng = Rng::new(0xd7);
    let mut ci = [0i32; 20];
    unsafe { c::cref_drm_crc_init(ci.as_mut_ptr()) };
    let mut d = StructDrm::default();
    drm_read_crc_init(&mut d);
    assert_eq!(store(&d), ci, "drmRead_CrcInit");
    for it in 0..4000 {
        let mut st = ci;
        if rng.next_u32() % 2 == 0 {
            for i in 0..3 {
                st[4 * i] = (rng.next_u32() & 1) as i32;
                st[4 * i + 1] = rng.next_u32() as i32 % 300;
            }
            st[12] = (rng.next_u32() % 3) as i32;
            st[13] = (rng.next_u32() % 3) as i32;
            st[14] = (rng.next_u32() & 0xffff) as i32;
            st[15] = (rng.next_u32() & 0xff) as i32;
        }
        let mut d = load(&st);
        let data: Vec<u8> = (0..64).map(|_| rng.next_u32() as u8).collect();
        let mut bs = Bs::new(&data, 512, 64, if rng.next_u32() % 2 == 0 { Some(&mut rng) } else { None });
        for step in 0..8 {
            let what = format!("drm crc {it}/{step}");
            match rng.next_u32() % 4 {
                0 => {
                    let m = [0, 16, -8, 192, 400][(rng.next_u32() % 5) as usize];
                    let r = drm_read_crc_start_reg(&mut d, &mut bs.rs, m);
                    let e = unsafe { c::cref_drm_crc_start_reg(st.as_mut_ptr(), bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr(), m) };
                    assert_eq!(r, e, "{what}: start");
                }
                1 => {
                    let reg = (rng.next_u32() % 3) as i32;
                    drm_read_crc_end_reg(&mut d, &mut bs.rs, reg);
                    unsafe { c::cref_drm_crc_end_reg(st.as_mut_ptr(), bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr(), reg) };
                }
                2 => {
                    let r = drm_read_crc_check(&mut d);
                    let e = unsafe { c::cref_drm_crc_check(st.as_mut_ptr()) };
                    assert_eq!(r, e, "{what}: check");
                }
                _ => {
                    let n = 1 + rng.next_u32() % 32;
                    fdk_read_bits(&mut bs.rs, n);
                    bs.sync_c();
                }
            }
            assert_eq!(store(&d), st, "{what}: state");
            bs.check(&what);
        }
    }
}
