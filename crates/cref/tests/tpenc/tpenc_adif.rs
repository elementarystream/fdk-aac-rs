// Differential tests of fdk_aac_rs_tpenc::tpenc_adif against the C++ reference.
//
// adifWrite_EncodeHeader() and adifWrite_GetHeaderBits() on random ADIF_INFO states (every
// channel mode, matrix mixdown, pseudo surround, sampling rates without index, buffer fullness
// below and above 2^20, header already written), written after a random number of bits. The
// ADIF_INFO byte image, the return values, the bitstream and the buffer are compared.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::tpenc_tpenc_adif as c;
use fdk_aac_rs_tpenc::tpenc_adif::*;

use super::tpenc_asc::{rand_mode, rand_rate, BsPair};

fn rand_info(rng: &mut Rng) -> AdifInfo {
    AdifInfo {
        cm: rand_mode(rng),
        sampling_rate: rand_rate(rng),
        bit_rate: if rng.range(0, 3) == 0 { rng.i32() } else { rng.range(0, 1 << 23) },
        profile: rng.range(-1, 4),
        b_variable_rate: rng.range(-1, 2),
        instance_tag: rng.range(-1, 17),
        header_written: if rng.range(0, 4) == 0 { rng.range(-1, 2) } else { 0 },
        matrix_mixdown_a: rng.range(-1, 6),
        pseudo_surround_enable: rng.range(-1, 2),
    }
}

/// Also compares ADIF_ID (`adifId`, function-local in adifWrite_EncodeHeader()) through the
/// function: its four bytes are the first 32 bits of every header written (when the header was
/// not written already).
#[test]
fn encode_header_and_bits() {
    let mut rng = Rng::new(0x7e21);
    assert_eq!(unsafe { c::cref_tpenc_adif_size() } as usize, ADIF_INFO_SIZE);
    for it in 0..20000 {
        let info = rand_info(&mut rng);
        let mut ci = [0u8; ADIF_INFO_SIZE];
        info.store(&mut ci);
        let mut ri = info;

        if rng.range(0, 1) == 0 {
            let hc = unsafe { c::cref_tpenc_adif_get_header_bits(ci.as_mut_ptr()) };
            let hr = adif_write_get_header_bits(&ri);
            assert_eq!(hc, hr, "{it}: adifWrite_GetHeaderBits({info:?})");
        }

        let mut bs = BsPair::new(&mut rng, 256);
        let pre = rng.range(0, 40);
        bs.pat(pre, rng.next_u32());
        let bf = match rng.range(0, 4) {
            0 => rng.i32(),
            1 => (1 << 20) + rng.range(-2, 2),
            _ => rng.range(0, 1 << 20),
        };
        let rc = unsafe { c::cref_tpenc_adif_encode_header(ci.as_mut_ptr(), bs.c, bf) };
        let rr = adif_write_encode_header(&mut ri, &mut bs.r, bf);
        assert_eq!(rc, rr, "{it}: adifWrite_EncodeHeader");
        let mut rimg = ci;
        ri.store(&mut rimg);
        assert_eq!(ci, rimg, "{it}: ADIF_INFO after adifWrite_EncodeHeader");
        assert_eq!(AdifInfo::load(&ci), ri);
        bs.check(&format!("{it}: adifWrite_EncodeHeader({info:?}, {bf})"));

        let hc = unsafe { c::cref_tpenc_adif_get_header_bits(ci.as_mut_ptr()) };
        let hr = adif_write_get_header_bits(&ri);
        assert_eq!(hc, hr, "{it}: adifWrite_GetHeaderBits after the header");
    }
}
