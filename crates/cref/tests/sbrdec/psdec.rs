// Differential tests of fdk_aac_rs_sbrdec::psdec and psbitdec against the C++ reference.
//
// One PS decoder (960 and 1024 frames) runs through random frame sequences as in the SBR
// decoder: ReadPsData on structured random PS payloads (valid and invalid IID/ICC modes, fixed
// and variable borders, extensions), DecodePs with random frame errors, slots and previous
// states (the coefficient scratch starts with the same random content on both sides), then the
// PS part of sbr_dec() (PreparePsProcessing, initSlotBasedRotation, ApplyPsSlot) with random
// QMF data, scale factors and band limits, and ResetPsDec; the return values, the bit
// positions, the coefficients, the QMF output and the complete PS state (the state dump of
// tests/sbrdec/e2e.rs) are compared after every call. The file-static helpers of psbitdec.cpp
// (Huffman decoding, limiting, delta decoding, 34-to-20 mapping) are tested directly. Real data:
// tests/sbrdec/e2e.rs (HE-AAC v2 streams).

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use super::e2e::{dump_ps_reference, first_diff, Dump};
use core::ffi::c_void;
use fdk_aac_rs_cref::sbrdec_psbitdec::*;
use fdk_aac_rs_cref::sbrdec_psdec::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_sbrdec::psbitdec::*;
use fdk_aac_rs_sbrdec::psdec::*;
use fdk_aac_rs_sbrdec::sbr_ram::{pod_bytes, pod_bytes_mut};
use fdk_aac_rs_sbrdec::sbr_rom::*;

struct Bits {
    bytes: Vec<u8>,
    n: usize,
}

impl Bits {
    fn put(&mut self, v: u32, bits: u32) {
        for k in (0..bits).rev() {
            if self.n % 8 == 0 {
                self.bytes.push(0);
            }
            if (v >> k) & 1 != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
    }
}

/// A PS payload: a header and frame info that are mostly valid, then random bits.
fn payload(rng: &mut Rng) -> (Vec<u8>, i32) {
    let mut b = Bits { bytes: Vec::new(), n: 0 };
    let header = rng.next_u32() % 4 != 0;
    b.put(header as u32, 1);
    if header {
        let iid = rng.next_u32() % 5 != 0;
        b.put(iid as u32, 1);
        if iid {
            b.put(if rng.next_u32() % 10 == 0 { rng.range(6, 7) } else { rng.range(0, 5) } as u32, 3);
        }
        let icc = rng.next_u32() % 5 != 0;
        b.put(icc as u32, 1);
        if icc {
            b.put(if rng.next_u32() % 10 == 0 { rng.range(6, 7) } else { rng.range(0, 5) } as u32, 3);
        }
        b.put((rng.next_u32() % 4 == 0) as u32, 1); // extension
    }
    let var = rng.next_u32() % 2;
    b.put(var, 1);
    if var == 0 {
        b.put(rng.range(0, 3) as u32, 2);
    } else {
        let n = rng.range(0, 3);
        b.put(n as u32, 2);
        let mut t = 0;
        for _ in 0..=n {
            t = if rng.next_u32() % 8 == 0 { rng.range(0, 31) } else { (t + rng.range(1, 10)).min(31) };
            b.put(t as u32, 5);
        }
    }
    while b.bytes.len() < 128 {
        b.put(rng.next_u32() & 0xFF, 8);
    }
    b.bytes.truncate(128);
    let valid = if rng.next_u32() % 4 == 0 { rng.range(0, 1024) } else { 128 * 8 };
    (b.bytes, valid)
}

fn compare_state(ps: &PsDec, what: &str) {
    let mut d = Dump(Vec::new());
    d.ps(Some(ps));
    let c = dump_ps_reference(unsafe { cref_sbrdec_ps_handle() });
    if let Some(e) = first_diff(&d.0, &c) {
        panic!("{what}: state: {e}");
    }
}

fn set(ps: &mut PsDec, last: u8, read: u8, process: u8, prv: u8, frame_based: i8) {
    ps.bs_last_slot = last;
    ps.bs_read_slot = read;
    ps.process_slot = process;
    ps.ps_decoded_prv = prv;
    ps.proc_frame_based = frame_based;
    unsafe { cref_sbrdec_ps_set(last as i32, read as i32, process as i32, prv as i32, frame_based as i32) };
}

#[test]
fn ps_frames() {
    // (the state dump of shim/sbrdec_e2e.cpp is a global buffer)
    let _lock = super::e2e::harness_lock();
    let mut rng = Rng::new(0x9500);
    let mut applied = 0;
    for it in 0..200 {
        let spf = match rng.next_u32() % 8 {
            0 => [0, 512, 2048][rng.range(0, 2) as usize],
            k => [1024, 960][(k % 2) as usize],
        };
        let mut port: Option<Box<PsDec>> = None;
        let rp = create_ps_dec(&mut port, spf);
        let rc = unsafe { cref_sbrdec_ps_create(spf) };
        assert_eq!(rp, rc, "{it}: CreatePsDec({spf})");
        if rc != 0 {
            unsafe { cref_sbrdec_ps_delete() };
            delete_ps_dec(&mut port);
            continue;
        }
        let ps = port.as_deref_mut().unwrap();
        compare_state(ps, &format!("{it}: create"));
        let no_cols = if spf == 960 { 30 } else { 32 };
        let mut read_slot = 0u8;
        let mut prv = 0u8;
        let mut frame_based: i8 = -1;
        for f in 0..40 {
            let what = format!("{it}/{f}");
            let last = read_slot;
            read_slot = rng.range(0, 1) as u8;
            let process = if rng.next_u32() % 4 == 0 { rng.range(0, 1) as u8 } else { read_slot };
            set(ps, last, read_slot, process, prv, frame_based);
            if rng.next_u32() % 10 != 0 {
                let (buf, valid) = payload(&mut rng);
                let n_left = if rng.next_u32() % 4 == 0 { rng.range(0, 200) } else { valid };
                let mut cbuf = buf.clone();
                let mut left_c = 0;
                let with = rng.next_u32() % 50 != 0;
                let rc = unsafe { cref_sbrdec_ps_read(with as i32, cbuf.as_mut_ptr(), 128, valid, n_left, &mut left_c) };
                let pbuf = buf.clone();
                let mut bs = FdkBitStream::default();
                fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&pbuf), 128, valid as u32, BS_READER);
                let rp = read_ps_data(if with { Some(&mut *ps) } else { None }, &mut bs, n_left);
                let left_p = fdk_get_valid_bits(&mut bs) as i32;
                assert_eq!((rp, left_p), (rc, left_c), "{what}: ReadPsData");
                compare_state(ps, &format!("{what}: ReadPsData"));
            }
            let fe = (rng.next_u32() % 6 == 0) as u8;
            let mut cc = PsDecCoefficients::ZERO;
            for b in pod_bytes_mut(&mut cc).iter_mut() {
                *b = rng.next_u32() as u8;
            }
            let mut cp = cc;
            let rc = unsafe { cref_sbrdec_ps_decode(fe as i32, pod_bytes_mut(&mut cc).as_mut_ptr() as *mut c_void) };
            let rp = decode_ps(ps, fe, &mut cp);
            assert_eq!(rp, rc, "{what}: DecodePs");
            assert_eq!(pod_bytes(&cp), pod_bytes(&cc), "{what}: DecodePs coefficients");
            compare_state(ps, &format!("{what}: DecodePs"));
            if rc != 0 {
                applied += 1;
                let n = (no_cols + HYBRID_FILTER_DELAY) * 64;
                let shift = rng.range(0, 12);
                let re: Vec<i32> = (0..n).map(|_| rng.i32() >> shift).collect();
                let im: Vec<i32> = (0..n).map(|_| rng.i32() >> shift).collect();
                let (sf_ov, sf_no_ov, sf_high) = (rng.range(-31, 31), rng.range(-31, 31), rng.range(-31, 31));
                let lsb = rng.range(3, 40);
                let usb = rng.range(lsb, 64);
                let high = if rng.next_u32() % 4 == 0 { rng.range(lsb, 64) } else { usb };
                let ov = HYBRID_FILTER_DELAY as i32;
                let (mut rc_, mut ic_) = (re.clone(), im.clone());
                let mut orc = vec![0i32; no_cols * 64];
                let mut oic = vec![0i32; no_cols * 64];
                unsafe {
                    cref_sbrdec_ps_frame(
                        rc_.as_mut_ptr(),
                        ic_.as_mut_ptr(),
                        no_cols as i32,
                        sf_ov,
                        sf_no_ov,
                        sf_high,
                        high,
                        lsb,
                        usb,
                        ov,
                        orc.as_mut_ptr(),
                        oic.as_mut_ptr(),
                    )
                };
                let (mut rp_, mut ip_) = (re.clone(), im.clone());
                let mut orp = vec![0i32; no_cols * 64];
                let mut oip = vec![0i32; no_cols * 64];
                {
                    let mut lr: Vec<&mut [i32]> = rp_.chunks_mut(64).collect();
                    let mut li: Vec<&mut [i32]> = ip_.chunks_mut(64).collect();
                    prepare_ps_processing(ps, &lr, &li, sf_ov);
                    let mut env = 0usize;
                    for i in 0..no_cols {
                        let ess = ps.bs_data[ps.process_slot as usize & 1].a_env_start_stop;
                        if ess.get(env).is_some_and(|&v| v as usize == i) {
                            init_slot_based_rotation(ps, &mut cp, env as i32, high);
                            env += 1;
                        }
                        let (or, oi) = (&mut orp[i * 64..i * 64 + 64], &mut oip[i * 64..i * 64 + 64]);
                        apply_ps_slot(
                            ps,
                            &mut cp,
                            &mut lr[i..],
                            &mut li[i..],
                            or,
                            oi,
                            sf_no_ov,
                            if (i as i32) < ov { sf_ov } else { sf_no_ov },
                            sf_high,
                            lsb,
                            usb,
                        );
                    }
                }
                let w = format!("{what}: PS frame lsb {lsb} usb {usb} high {high} sf {sf_ov} {sf_no_ov} {sf_high}");
                assert_eq!(orp, orc, "{w}: right real");
                assert_eq!(oip, oic, "{w}: right imag");
                assert_eq!(rp_, rc_, "{w}: left real");
                assert_eq!(ip_, ic_, "{w}: left imag");
                compare_state(ps, &w);
            }
            prv = (rc != 0) as u8;
            frame_based = match rng.next_u32() % 4 {
                0 => 1,
                1 => -1,
                _ => ps.proc_frame_based,
            };
            if rng.next_u32() % 30 == 0 {
                let rc = unsafe { cref_sbrdec_ps_reset() };
                let rp = reset_ps_dec(ps);
                assert_eq!(rp, rc, "{what}: ResetPsDec");
                compare_state(ps, &format!("{what}: ResetPsDec"));
            }
        }
        unsafe { cref_sbrdec_ps_delete() };
        delete_ps_dec(&mut port);
    }
    assert!(applied > 1000, "{applied} PS frames");
}

#[test]
fn psbitdec_helpers() {
    let mut rng = Rng::new(0xb17);
    let books: [&[[i8; 2]]; 6] = [
        &A_BOOK_PS_IID_TIME_DECODE,
        &A_BOOK_PS_IID_FREQ_DECODE,
        &A_BOOK_PS_ICC_TIME_DECODE,
        &A_BOOK_PS_ICC_FREQ_DECODE,
        &A_BOOK_PS_IID_FINE_TIME_DECODE,
        &A_BOOK_PS_IID_FINE_FREQ_DECODE,
    ];
    for it in 0..20_000 {
        let k = rng.range(0, 5);
        let buf: Vec<u8> = (0..64).map(|_| rng.next_u32() as u8).collect();
        let valid = rng.range(0, 512);
        let n = 30;
        let mut out = vec![0i32; 2 * n];
        let mut cbuf = buf.clone();
        unsafe { cref_sbrdec_psbitdec_huff(k, cbuf.as_mut_ptr(), 64, valid, n as i32, out.as_mut_ptr()) };
        let mut bs = FdkBitStream::default();
        fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&buf), 64, valid as u32, BS_READER);
        for i in 0..n {
            let v = decode_huff_cw(books[k as usize], &mut bs) as i32;
            let left = fdk_get_valid_bits(&mut bs) as i32;
            assert_eq!([v, left], [out[2 * i], out[2 * i + 1]], "{it}: book {k} code word {i}");
        }

        let (a, lo, hi) = (rng.next_u32() as i8, rng.range(-20, 0) as i8, rng.range(0, 20) as i8);
        assert_eq!(limit_min_max(a, lo, hi), unsafe { cref_sbrdec_psbitdec_limit(a, lo, hi) });

        let stride = rng.range(1, 2) as u8;
        let nr = rng.range(0, (NO_HI_RES_BINS / stride as usize) as i32) as u8;
        let mut idx = [0i8; NO_HI_RES_BINS];
        let mut prev = [0i8; NO_HI_RES_BINS];
        for v in idx.iter_mut().chain(prev.iter_mut()) {
            *v = rng.range(-30, 30) as i8;
        }
        let (enable, dt_df) = ([0, 1, 1, 1, 2][rng.range(0, 4) as usize] as i8, rng.range(0, 1) as i8);
        let (mut ic, mut pc) = (idx, prev);
        let mut ip = idx;
        unsafe { cref_sbrdec_psbitdec_delta_decode(enable, ic.as_mut_ptr(), pc.as_mut_ptr(), dt_df, nr, stride, lo, hi) };
        delta_decode_array(enable, &mut ip, &prev, dt_df, nr, stride, lo, hi);
        assert_eq!(ip, ic, "{it}: deltaDecodeArray {enable} {dt_df} {nr} {stride}");

        let bins = [20u8, 34][rng.range(0, 1) as usize];
        let (mut mc, mut mp) = (idx, idx);
        unsafe { cref_sbrdec_psbitdec_map34(mc.as_mut_ptr(), bins) };
        map34_index_to20(&mut mp, bins);
        assert_eq!(mp, mc, "{it}: map34IndexTo20 {bins}");
    }
}
