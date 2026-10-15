// Differential tests of fdk_aac_rs_aacenc::bit_cnt against the C++ reference.
//
// FDKaacEnc_bitCount (and through it every static count* helper, selected by maxVal),
// FDKaacEnc_countValues and FDKaacEnc_codeValues for every code book with values over the whole
// range of the book (escape values up to the SHORT range), FDKaacEnc_codeScalefactorDelta and
// FDKaacEnc_bitCountScalefactorDelta. The coding functions write through a bit writer; the
// written bytes and the bit position are compared, after a random number of leading bits to vary
// the alignment. Realistic data: quantized real spectra (sf_estim_data + the ported quantizer).

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use crate::sf_estim_data::channel;
use fdk_aac_rs_aacenc::bit_cnt::*;
use fdk_aac_rs_aacenc::quantize::fdk_aac_enc_quantize_spectrum;
use fdk_aac_rs_cref::aacenc_bit_cnt as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;

/// The largest absolute value of a code book (`codeBookLav`, unbounded for ESC).
fn lav(cb: i32) -> i32 {
    [0, 1, 1, 2, 2, 4, 4, 7, 7, 12, 12, 32767][cb as usize]
}

/// Values in `-max..=max`; for the escape book -32768 as well if `min` is set.
fn values_min(rng: &mut Rng, n: usize, max: i32, min: bool) -> Vec<i16> {
    let mut v: Vec<i16> = (0..n)
        .map(|_| match rng.range(0, 5) {
            0 => 0,
            1 => [max, -max][rng.range(0, 1) as usize] as i16,
            2 if max > 16 => rng.range(-16, 16) as i16,
            _ => rng.range(-max, max) as i16,
        })
        .collect();
    if min && max == 32767 && rng.range(0, 3) == 0 && n > 0 {
        let k = rng.range(0, n as i32 - 1) as usize;
        v[k] = -32768;
    }
    v
}

/// Values in `-max..=max`. -32768 is excluded (no defined reference result).
fn values(rng: &mut Rng, n: usize, max: i32) -> Vec<i16> {
    values_min(rng, n, max, false)
}

fn check_bit_count(what: &str, v: &[i16], width: i32, max_val: i32) {
    let mut r = [0x55i32; 12];
    let mut cc = [0x55i32; 12];
    let rr = fdk_aac_enc_bit_count(v, width, max_val, &mut r);
    let rc = unsafe { c::cref_bit_cnt_bitCount(v.as_ptr(), width, max_val, cc.as_mut_ptr()) };
    assert_eq!((rr, r), (rc, cc), "{what}: bitCount(width {width}, maxVal {max_val}) {v:?}");
}

fn max_abs(v: &[i16]) -> i32 {
    v.iter().map(|&x| (x as i32).abs()).max().unwrap_or(0)
}

#[test]
fn bit_count_random() {
    let mut rng = Rng::new(0xb17);
    for k in 0..30000 {
        let width = 4 * rng.range(0, 24);
        let max = match rng.range(0, 4) {
            0 => rng.range(0, 16),
            1 => rng.range(17, 8191),
            2 => 32767,
            _ => rng.range(0, 3),
        };
        let mut v = values(&mut rng, width as usize + 4, max);
        // the counters with maxVal < 16 must only see values up to maxVal
        let m = max_abs(&v[..width as usize]);
        if m < 16 && rng.range(0, 1) == 0 {
            // maxVal larger than the actual maximum selects a counter for more books
            let mv = rng.range(m, 16);
            check_bit_count(&format!("random {k}"), &v, width, mv);
        } else {
            for x in v[width as usize..].iter_mut() {
                *x = 0;
            }
            check_bit_count(&format!("random {k}"), &v, width, m);
        }
    }
}

fn check_count_values(what: &str, v: &[i16], width: i32, cb: i32) {
    let r = fdk_aac_enc_count_values(v, width, cb);
    let mut vc = v.to_vec();
    let rc = unsafe { c::cref_bit_cnt_countValues(vc.as_mut_ptr(), width, cb) };
    assert_eq!(r, rc, "{what}: countValues(width {width}, cb {cb}) {v:?}");
}

#[test]
fn count_values_random() {
    let mut rng = Rng::new(0xb18);
    for k in 0..30000 {
        let cb = rng.range(0, 15);
        let width = 4 * rng.range(0, 24);
        let v = values(&mut rng, width as usize, if cb <= 11 { lav(cb) } else { 0 });
        check_count_values(&format!("random {k}"), &v, width, cb);
    }
}

/// Writes `ops` (as in `cref_bit_cnt_write`) with the port and the reference and compares the
/// return values, the bit positions and the written bytes.
fn check_write(what: &str, pre: u32, n_pre: i32, values: &[i16], ops: &[[i32; 4]]) {
    const SIZE: usize = 8192;
    let mem_r = BitMem::new(SIZE);
    let mut buf_c = vec![0u8; SIZE];
    let mut bs = FdkBitStream::default();
    fdk_init_bit_stream(&mut bs, &mem_r, SIZE as u32, 0, BS_WRITER);
    fdk_write_bits(&mut bs, pre, n_pre as u32);
    let mut ret_r = 0;
    for op in ops {
        ret_r += if op[0] == 0 {
            fdk_aac_enc_code_values(&values[op[1] as usize..], op[2], op[3], &mut bs)
        } else {
            fdk_aac_enc_code_scalefactor_delta(op[1], &mut bs)
        };
    }
    fdk_sync_cache(&mut bs);
    let valid_r = fdk_get_valid_bits(&mut bs);
    let buf_r = mem_r.to_vec();

    let flat: Vec<i32> = ops.iter().flatten().copied().collect();
    let mut vc = values.to_vec();
    let mut valid_c = 0;
    let ret_c = unsafe {
        c::cref_bit_cnt_write(
            buf_c.as_mut_ptr(),
            SIZE as u32,
            pre,
            n_pre,
            vc.as_mut_ptr(),
            flat.as_ptr(),
            ops.len() as i32,
            &mut valid_c,
        )
    };
    assert_eq!((ret_r, valid_r), (ret_c, valid_c), "{what}: return value, valid bits");
    assert!(buf_r == buf_c, "{what}: written bytes");
}

#[test]
fn code_values_random() {
    let mut rng = Rng::new(0xb19);
    for k in 0..3000 {
        let n = 1024;
        let mut v = vec![0i16; n + 4];
        let mut ops = vec![];
        let mut pos = 0usize;
        while pos < n {
            let cb = rng.range(0, 12);
            let width = (4 * rng.range(0, 12) as usize).min(n - pos);
            let nv = values_min(&mut rng, width, if cb <= 11 { lav(cb) } else { 0 }, true);
            v[pos..pos + width].copy_from_slice(&nv);
            if rng.range(0, 3) == 0 {
                ops.push([1, rng.range(-62, 62), 0, 0]);
            }
            ops.push([0, pos as i32, width as i32, cb]);
            pos += width;
        }
        let n_pre = rng.range(0, 32);
        check_write(&format!("random {k}"), rng.next_u32(), n_pre, &v, &ops);
    }
}

#[test]
fn code_scalefactor_delta() {
    let mut rng = Rng::new(0xb1a);
    for d in -70..=70 {
        check_write(&format!("delta {d}"), 0, rng.range(0, 32), &[0; 4], &[[1, d, 0, 0]]);
    }
    for k in 0..200 {
        let d = rng.i32();
        if d == i32::MIN {
            continue; // no defined result in the reference (see the port's documentation)
        }
        check_write(&format!("delta random {k}"), 0, 3, &[0; 4], &[[1, d, 0, 0]]);
    }
}

/// The offsets of the Huffman tables in the linked test binary (relative to
/// FDKaacEnc_huff_ltab1_2) equal the port's model of aacEnc_rom.o for this variant.
#[test]
fn huff_rom_layout_matches() {
    let mut c_off = [0i32; HUFF_ROM_TABLES + 1];
    unsafe { c::cref_bit_cnt_huff_layout(c_off.as_mut_ptr()) };
    let (off, end) = huff_rom_layout();
    let r: Vec<i64> = off.iter().copied().chain([end]).collect();
    assert_eq!(c_off.iter().map(|&x| x as i64).collect::<Vec<_>>(), r);
}

/// Every delta whose result the port reproduces from the layout of the Huffman tables of
/// aacEnc_rom.o. For the others the reference has no defined result; the port returns
/// INVALID_BITCOUNT.
#[test]
fn bit_count_scalefactor_delta() {
    let (off, end) = huff_rom_layout();
    let scf = off[6]; // FDKaacEnc_huff_ltabscf
    let (lo, hi) = (-scf - 60, end - scf - 1 - 60);
    for d in lo..=hi {
        let r = fdk_aac_enc_bit_count_scalefactor_delta(d as i32);
        let cc = unsafe { c::cref_bit_cnt_bitCountScalefactorDelta(d as i32) };
        assert_eq!(r, cc, "delta {d}");
    }
    let before = huff_rom_outside_reads();
    for d in [lo - 1, lo - 1000, hi + 1, hi + 1000, i32::MIN as i64, i32::MAX as i64] {
        assert_eq!(fdk_aac_enc_bit_count_scalefactor_delta(d as i32), INVALID_BITCOUNT, "delta {d}");
    }
    assert_eq!(huff_rom_outside_reads(), before + 6);
}

/// FDKaacEnc_bitCount and FDKaacEnc_countValues on inputs that the encoder does not produce.
/// Compared with the reference whenever the port reproduces the result from the layout of the
/// Huffman tables of aacEnc_rom.o; the other cases (no defined reference result) only run on the
/// port, which must not panic. A negative maxVal gives INVALID_BITCOUNT for every book.
#[test]
fn out_of_range_counts() {
    let mut rng = Rng::new(0xb1c);
    let (mut compared, mut outside) = (0, 0);
    for k in 0..40000 {
        let width = 4 * rng.range(1, 16);
        let n = width as usize;
        let max = [2, 3, 4, 5, 8, 13, 17, 24, 40, 300][rng.range(0, 9) as usize];
        let mut v: Vec<i16> = (0..n).map(|_| rng.range(-max, max) as i16).collect();
        if rng.range(0, 15) == 0 {
            v[rng.range(0, n as i32 - 1) as usize] = i16::MIN;
        }
        let m = max_abs(&v);
        let mv = if rng.range(0, 3) == 0 { rng.range(0, m) } else { rng.range(0, m.min(16)) };
        let what = format!("{k}");
        let before = huff_rom_outside_reads();
        let mut r = [0x55i32; 12];
        fdk_aac_enc_bit_count(&v, width, mv, &mut r);
        if huff_rom_outside_reads() == before {
            check_bit_count(&what, &v, width, mv);
            compared += 1;
        } else {
            outside += 1;
        }
        for cb in 0..=11 {
            let before = huff_rom_outside_reads();
            fdk_aac_enc_count_values(&v, width, cb);
            if huff_rom_outside_reads() == before {
                check_count_values(&what, &v, width, cb);
                compared += 1;
            } else {
                outside += 1;
            }
        }
        let mut r = [0x55i32; 12];
        assert_eq!(fdk_aac_enc_bit_count(&v, width, -1 - rng.range(0, 100), &mut r), 0);
        assert!(r.iter().all(|&x| x == INVALID_BITCOUNT));
    }
    eprintln!("out-of-range counts: {compared} compared, {outside} without a defined reference result");
    assert!(compared > 200_000 && outside > 50_000);
}

/// FDKaacEnc_codeValues on inputs that the encoder does not produce, where the port reproduces
/// the result from the layout of the Huffman tables of aacEnc_rom.o.
#[test]
fn out_of_range_code_values() {
    let mut rng = Rng::new(0xb1d);
    let range = |cb: i32| [0, 3, 3, 4, 4, 8, 8, 21, 21, 12, 12, 32767][cb as usize];
    for k in 0..3000 {
        let n = 1024;
        let mut v = vec![0i16; n + 4];
        let mut ops = vec![];
        let mut pos = 0usize;
        while pos < n {
            let cb = rng.range(0, 11);
            let width = (4 * rng.range(0, 12) as usize).min(n - pos);
            let r = range(cb);
            let lo = if cb <= 2 { -1 } else { -r };
            for x in v[pos..pos + width].iter_mut() {
                *x = rng.range(lo, r) as i16;
            }
            ops.push([0, pos as i32, width as i32, cb]);
            pos += width;
        }
        let before = huff_rom_outside_reads();
        check_write(&format!("out of range {k}"), rng.next_u32(), rng.range(0, 32), &v, &ops);
        assert_eq!(huff_rom_outside_reads(), before, "{k}: no defined reference result");
    }
}

/// Quantized real spectra: bitCount per band with its maximum, countValues and codeValues with
/// every code book that can code the band.
#[test]
fn realistic() {
    let mut rng = Rng::new(0xb1b);
    for k in 0..200 {
        let d = channel(&mut rng, k % 3 == 2);
        let scf: Vec<i32> = (0..60).map(|_| rng.range(0, 30)).collect();
        let gg = rng.range(100, 220) - 100 - 20 * (k % 4);
        let mut q = vec![0i16; 1024 + 4];
        fdk_aac_enc_quantize_spectrum(
            d.sfb_cnt,
            d.max_sfb_per_group,
            d.sfb_per_group,
            &d.sfb_offsets,
            &d.spectrum,
            gg,
            &scf,
            &mut q,
            rng.range(0, 1),
        );
        let mut ops = vec![];
        let mut sfb_offs = 0;
        while sfb_offs < d.sfb_cnt {
            for sfb in 0..d.max_sfb_per_group {
                let i = (sfb_offs + sfb) as usize;
                let (lo, hi) = (d.sfb_offsets[i] as usize, d.sfb_offsets[i + 1] as usize);
                let w = (hi - lo) as i32;
                let m = max_abs(&q[lo..hi]);
                check_bit_count(&format!("realistic {k} sfb {i}"), &q[lo..], w, m);
                for cb in 0..=11 {
                    if m <= lav(cb) || cb == 11 {
                        check_count_values(&format!("realistic {k} sfb {i}"), &q[lo..], w, cb);
                    }
                }
                let cb = (0..=11).find(|&cb| m <= lav(cb)).unwrap();
                ops.push([1, rng.range(-60, 60), 0, 0]);
                ops.push([0, lo as i32, w, cb + rng.range(0, 1).min(11 - cb) * (cb != 0) as i32]);
            }
            sfb_offs += d.sfb_per_group;
        }
        check_write(&format!("realistic {k}"), 0, rng.range(0, 32), &q, &ops);
    }
}
