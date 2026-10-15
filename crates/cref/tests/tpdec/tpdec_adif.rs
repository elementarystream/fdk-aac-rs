// Differential tests of fdk_aac_rs_tpdec::tpdec_adif against the C++ reference.
//
// adifRead_DecodeHeader() on generated ADIF headers (with PCEs), their mutations and random
// bytes, with random previous PCEs. The header, the PCE and the bitstream are compared.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpdec_tpdec_adif as c;
use fdk_aac_rs_tpdec::tpdec_adif::*;

use super::tp_data::{flip_bits, BitWriter, Bs};
use super::tpdec_asc::gen_pce;

fn gen_adif(rng: &mut Rng, w: &mut BitWriter) {
    for &ch in b"ADIF" {
        w.put(ch as u32, 8);
    }
    let cp = rng.next_u32() % 4 == 0;
    w.put(cp as u32, 1);
    if cp {
        w.random_bytes(9, rng);
    }
    w.put(rng.next_u32() & 1, 1);
    w.put(rng.next_u32() & 1, 1);
    let bst = rng.next_u32() & 1;
    w.put(bst, 1);
    w.put(rng.next_u32() & 0x7fffff, 23);
    let n = if rng.next_u32() % 4 == 0 { rng.next_u32() % 4 } else { 0 };
    w.put(n, 4);
    if bst == 0 {
        w.put(rng.next_u32() & 0xfffff, 20);
    }
    for _ in 0..=n {
        gen_pce(rng, w, 0);
    }
    w.align();
    w.random_bytes(20, rng);
}

#[test]
fn adif_read_decode_header_() {
    let mut rng = Rng::new(0xad1f);
    for it in 0..20000 {
        let mut w = BitWriter::new();
        if rng.next_u32() % 10 == 0 {
            w.random_bytes(1 + (rng.next_u32() % 30) as usize, &mut rng);
        } else {
            gen_adif(&mut rng, &mut w);
        }
        let mut bytes = w.bytes.clone();
        if rng.next_u32() % 4 == 0 {
            flip_bits(&mut bytes, 1 + rng.next_u32() % 3, &mut rng);
        }
        let bits = if rng.next_u32() % 5 == 0 { rng.next_u32() % (bytes.len() as u32 * 8 + 1) } else { bytes.len() as u32 * 8 };
        let mut bs = Bs::new(&bytes, bits, 256, if rng.next_u32() % 3 == 0 { Some(&mut rng) } else { None });
        let mut h = CAdifHeader {
            num_program_config_elements: rng.next_u32() as i32,
            bit_rate: rng.next_u32(),
            copyright_id_present: rng.next_u32() as u8,
            original_copy: rng.next_u32() as u8,
            home: rng.next_u32() as u8,
            bitstream_type: rng.next_u32() as u8,
        };
        let mut ch = [
            h.num_program_config_elements,
            h.bit_rate as i32,
            h.copyright_id_present as i32,
            h.original_copy as i32,
            h.home as i32,
            h.bitstream_type as i32,
        ];
        let mut p = super::tpdec_asc::random_or_valid_pce(&mut rng);
        let mut cp = p.as_bytes().to_vec();
        let r = adif_read_decode_header(&mut h, &mut p, &mut bs.rs);
        let e = unsafe { c::cref_adif_decode_header(ch.as_mut_ptr(), cp.as_mut_ptr(), bs.cst.as_mut_ptr(), bs.cbuf.as_mut_ptr()) };
        let what = format!("adifRead_DecodeHeader {it}");
        assert_eq!(r, e, "{what}");
        assert_eq!(
            [
                h.num_program_config_elements,
                h.bit_rate as i32,
                h.copyright_id_present as i32,
                h.original_copy as i32,
                h.home as i32,
                h.bitstream_type as i32
            ],
            ch,
            "{what}: header"
        );
        assert_eq!(p.as_bytes().as_slice(), cp.as_slice(), "{what}: pce");
        bs.check(&what);
    }
}
