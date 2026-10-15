// Differential tests of fdk_aac_rs_sbrenc::ps_bitenc against the C++ reference.
//
// The IID, ICC, IPD and OPD coders on random parameters of every resolution and coding
// direction (including values beyond the Huffman tables and invalid modes), and
// FDKsbrEnc_WritePSBitstream() on random PS_OUT structures (headers, all iid/icc modes, both
// frame classes, 0 to 4 envelopes, IPD/OPD extension), written and only counted. The bit counts,
// error flags and written bits are compared. Realistic parameters are covered by the PS test
// (ps_main.rs) and the whole-encoder test.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::sbrenc_ps_bitenc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_sbrenc::ps_bitenc::*;
use fdk_aac_rs_sbrenc::ps_const::*;

use super::sbr_encoder_dump::{ps_out, Dump};

/// Runs `f` on a writer over a 1024 byte buffer (or without one); returns the buffer and the
/// valid bits.
fn with_bs(count: bool, f: impl FnOnce(Option<&mut FdkBitStream>) -> i32) -> (i32, Vec<u8>, i32) {
    let mem = BitMem::new(1024);
    let mut bs = FdkBitStream::default();
    fdk_init_bit_stream(&mut bs, &mem, 1024, 0, BS_WRITER);
    let n = f(if count { None } else { Some(&mut bs) });
    fdk_sync_cache(&mut bs);
    let valid = fdk_get_valid_bits(&mut bs) as i32;
    (n, mem.to_vec(), valid)
}

fn random_vals(rng: &mut Rng, lo: i32, hi: i32, wide: bool) -> [i32; PS_MAX_BANDS] {
    let mut v = [0i32; PS_MAX_BANDS];
    for x in v.iter_mut() {
        *x = if wide { rng.range(lo - 40, hi + 40) } else { rng.range(lo, hi) };
    }
    v
}

#[test]
fn encoders() {
    let mut rng = Rng::new(0x9_b17e);
    let mut errors = 0;
    for case in 0..6000 {
        let which = case % 4;
        let count = rng.range(0, 3) == 0;
        let wide = rng.range(0, 4) == 0;
        let (lo, hi) = match which {
            0 => (-15, 15),
            1 => (0, 7),
            _ => (0, 7),
        };
        let val = random_vals(&mut rng, lo, hi, wide);
        let last = random_vals(&mut rng, lo, hi, wide);
        let n_bands = [10, 20, rng.range(0, 20)][rng.range(0, 2) as usize];
        let res = rng.range(0, 2);
        let mode = rng.range(0, 2);

        let mut val_c = val;
        let mut last_c = last;
        let mut err_c = 0;
        let mut buf_c = vec![0u8; 1024];
        let mut valid_c = 0;
        let n_c = unsafe {
            c::cref_ps_bitenc_encode(
                which,
                count as i32,
                val_c.as_mut_ptr(),
                last_c.as_mut_ptr(),
                n_bands,
                res,
                mode,
                &mut err_c,
                buf_c.as_mut_ptr(),
                &mut valid_c,
            )
        };
        let mut err_r = 0;
        let (n_r, buf_r, valid_r) = with_bs(count, |bs| match which {
            0 => fdk_sbr_enc_encode_iid(bs, &val, &last, n_bands, res, mode, &mut err_r),
            1 => fdk_sbr_enc_encode_icc(bs, &val, &last, n_bands, mode, &mut err_r),
            2 => fdk_sbr_enc_encode_ipd(bs, &val, &last, n_bands, mode, &mut err_r),
            _ => fdk_sbr_enc_encode_opd(bs, &val, &last, n_bands, mode, &mut err_r),
        });
        assert_eq!((n_c, err_c, valid_c), (n_r, err_r, valid_r), "case {case} which {which}");
        assert_eq!(buf_c, buf_r, "case {case} bits");
        errors += err_r;
    }
    assert!(errors > 100);
}

#[test]
fn write_ps_bitstream() {
    let mut rng = Rng::new(0x9_5_0ff);
    for case in 0..4000 {
        let mut o = PsOut {
            enable_ps_header: rng.range(0, 1),
            enable_iid: rng.range(0, 1),
            iid_mode: rng.range(0, 5),
            enable_icc: rng.range(0, 1),
            icc_mode: rng.range(0, 5),
            enable_ipd_opd: if rng.range(0, 3) == 0 { rng.range(0, 2) } else { 0 },
            frame_class: rng.range(0, 1),
            n_envelopes: rng.range(0, 4),
            ..Default::default()
        };
        for b in o.frame_border.iter_mut() {
            *b = rng.range(0, 40);
        }
        let wide = rng.range(0, 5) == 0;
        let fine = o.iid_mode >= 3;
        let iid_range = if fine { 15 } else { 7 };
        for e in 0..PS_MAX_ENVELOPES {
            o.delta_iid[e] = rng.range(0, if wide { 2 } else { 1 });
            o.delta_icc[e] = rng.range(0, if wide { 2 } else { 1 });
            o.delta_ipd[e] = rng.range(0, 1);
            o.delta_opd[e] = rng.range(0, 1);
            o.iid[e] = random_vals(&mut rng, -iid_range, iid_range, wide);
            o.icc[e] = random_vals(&mut rng, 0, 7, wide);
            o.ipd[e] = random_vals(&mut rng, 0, 7, wide);
            o.opd[e] = random_vals(&mut rng, 0, 7, wide);
        }
        o.iid_last = random_vals(&mut rng, -iid_range, iid_range, wide);
        o.icc_last = random_vals(&mut rng, 0, 7, wide);
        o.ipd_last = random_vals(&mut rng, 0, 7, wide);
        o.opd_last = random_vals(&mut rng, 0, 7, wide);

        let mut d = Dump::default();
        ps_out(&mut d, &o);
        for count in [false, true] {
            let mut buf_c = vec![0u8; 1024];
            let mut valid_c = 0;
            let n_c = unsafe {
                c::cref_ps_bitenc_write(d.0.as_mut_ptr(), buf_c.as_mut_ptr(), count as i32, &mut valid_c)
            };
            let (n_r, buf_r, valid_r) = with_bs(count, |bs| fdk_sbr_enc_write_ps_bitstream(Some(&o), bs));
            assert_eq!((n_c, valid_c), (n_r, valid_r), "case {case} count {count}");
            assert_eq!(buf_c, buf_r, "case {case} bits");
        }
    }
    let (n, _, _) = with_bs(false, |bs| fdk_sbr_enc_write_ps_bitstream(None, bs));
    assert_eq!(n, 0);
}
