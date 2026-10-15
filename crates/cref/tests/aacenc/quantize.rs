// Differential tests of fdk_aac_rs_aacenc::quantize against the C++ reference.
//
// The static helpers FDKaacEnc_quantizeLines and FDKaacEnc_invQuantizeLines are inlined into
// the three public functions and are tested through them, with and without the dead zone
// quantizer, on gains across the whole INT range (including gains that do not fit the spectrum,
// which reach the undefined shifts of the reference), on random spectra with the edge values and
// on realistic spectra (see sf_estim_data). The quantized output buffers are pre-filled with the
// same random contents on both sides and compared completely, so that lines the functions leave
// unwritten are checked as well.
//
// FDKaacEnc_calcSfbQuantEnergyAndDist gets the quantized spectrum as input. A quantized value of
// -32768, a shift by 32 or more, or a shifted value of exactly 2^31 (MINVAL_DBL, whose fixp_abs()
// is undefined) have no defined result in C, and the reference builds compile them differently
// (see `inv_quant_class`). The port reproduces ref-arm64, ref-x86_64 (Debian clang 19) and
// ref-i686; the `*_undefined` tests cover these inputs and are skipped where the reference of the
// test binary is ref-x86_64-mac (Apple clang, `APPLE_CLANG_X86`), the other tests keep the inverse
// quantization defined and run everywhere.

use crate::sf_estim_data::channel;
use fdk_aac_rs_aacenc::aac_enc_rom::{
    FDK_AAC_ENC_M_TAB_4_3_ELC, FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC,
    FDK_AAC_ENC_SPEC_EXP_TABLE_COMB, MANT_DIGITS, MANT_SIZE,
};
use fdk_aac_rs_aacenc::quantize::*;
use fdk_aac_rs_cref::aacenc_quantize as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{f_mult, FixpDbl, DFRACT_BITS, MAXVAL_DBL, MINVAL_DBL};

fn spec_value(rng: &mut Rng) -> FixpDbl {
    match rng.range(0, 9) {
        0 => [0, MINVAL_DBL, MAXVAL_DBL, 1, -1, MINVAL_DBL + 1, MAXVAL_DBL - 1, 0x4000_0000]
            [rng.range(0, 7) as usize],
        1..=3 => rng.i32() >> rng.range(0, 31),
        _ => rng.i32(),
    }
}

fn gain(rng: &mut Rng) -> i32 {
    match rng.range(0, 9) {
        0 => rng.i32(),
        1 => [i32::MIN, i32::MAX, i32::MIN + 1, -1000, 1000, 0][rng.range(0, 5) as usize],
        2 => rng.range(-400, 400),
        _ => rng.range(-120, 120),
    }
}

fn check_dist(what: &str, spec: &[FixpDbl], q0: &[i16], gain: i32, dz: i32) {
    let n = spec.len() as i32;
    let mut qr = q0.to_vec();
    let mut qc = q0.to_vec();
    let r = fdk_aac_enc_calc_sfb_dist(spec, &mut qr, n, gain, dz);
    let cc = unsafe { c::cref_quantize_calcSfbDist(spec.as_ptr(), qc.as_mut_ptr(), n, gain, dz) };
    assert_eq!((r, &qr), (cc, &qc), "{what}: calcSfbDist(n {n}, gain {gain}, dz {dz})");
}

fn check_en_dist(what: &str, spec: &[FixpDbl], q: &[i16], gain: i32) {
    let n = spec.len() as i32;
    let (mut en_r, mut dist_r) = (0x1234, 0x5678);
    let (mut en_c, mut dist_c) = (0x1234, 0x5678);
    fdk_aac_enc_calc_sfb_quant_energy_and_dist(spec, q, n, gain, &mut en_r, &mut dist_r);
    let mut spec_c = spec.to_vec();
    let mut q_c = q.to_vec();
    unsafe {
        c::cref_quantize_calcSfbQuantEnergyAndDist(
            spec_c.as_mut_ptr(),
            q_c.as_mut_ptr(),
            n,
            gain,
            &mut en_c,
            &mut dist_c,
        )
    };
    assert_eq!(
        (en_r, dist_r),
        (en_c, dist_c),
        "{what}: calcSfbQuantEnergyAndDist(n {n}, gain {gain})"
    );
}

#[test]
fn calc_sfb_dist_random() {
    let mut rng = Rng::new(0x9a17);
    for k in 0..20000 {
        let n = rng.range(0, 64);
        let spec: Vec<FixpDbl> = (0..n).map(|_| spec_value(&mut rng)).collect();
        let q0: Vec<i16> = (0..n).map(|_| rng.i16()).collect();
        check_dist(&format!("random {k}"), &spec, &q0, gain(&mut rng), rng.range(0, 1));
    }
}

/// Whether the C++ reference of this test binary was compiled by Apple clang for x86-64
/// (`--target x86_64-apple-darwin`, ref-x86_64-mac). Its distortion of `MINVAL_DBL` differs from
/// ref-x86_64 (Debian clang 19), which the port's `x86` variant follows.
const APPLE_CLANG_X86: bool = cfg!(all(target_arch = "x86_64", target_vendor = "apple"));

/// What `FDKaacEnc_invQuantizeLines()` does with the quantized value `q` and `gain`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InvQuant {
    /// Defined, with a result other than `MINVAL_DBL`.
    Defined,
    /// `q == -32768`: no defined result in C; the port reproduces the value of the reference
    /// builds from the object layout of the tables.
    Minus32768,
    /// The shift `accu <<= -(-iquantizershift - specExp)` / `accu >>= -iquantizershift -
    /// specExp` is by 32 or more, or a left shift leaves the 32 bits (C++17).
    BadShift,
    /// A result of `MINVAL_DBL` (exactly 2^31, or an undefined shift that wraps to it): its
    /// negation (`q < 0`) and its `fixp_abs()` in the distortion are undefined. ref-arm64
    /// computes the distortion as `|a - b|` (wrapping) normalized with `lsr`, ref-x86_64 and
    /// ref-i686 (Debian clang 19) as `|a - b|` (wrapping) with an arithmetic right shift by 31,
    /// ref-x86_64-mac (Apple clang) as `a > b ? a - b : b - a` normalized with a left shift.
    Minval,
}

/// Classifies `FDKaacEnc_invQuantizeLines()` of `q` with `gain` (see [`InvQuant`]).
fn inv_quant_class(q: i16, gain: i32) -> InvQuant {
    if q == 0 {
        return InvQuant::Defined;
    }
    if q == i16::MIN {
        return InvQuant::Minus32768;
    }
    if (q as i32).abs() > MAX_QUANT {
        // rejected before the inverse quantization
        return InvQuant::Defined;
    }
    let mut accu = (q as i32).abs();
    let ex = accu.leading_zeros() as i32 - 1;
    accu <<= ex;
    let spec_exp = ((DFRACT_BITS - 1) - ex) as usize;
    let tab_index = (accu >> (DFRACT_BITS - 2 - MANT_DIGITS)) & !MANT_SIZE;
    let m = (gain & 3) as usize;
    let s = FDK_AAC_ENC_M_TAB_4_3_ELC[tab_index as usize];
    let t = FDK_AAC_ENC_SPEC_EXP_MANT_TABLE_COMB_ELC[m][spec_exp];
    // fMult of two positive values (its last bit depends on the variant)
    let accu = f_mult(s, t);
    let st = (gain >> 2).wrapping_add(FDK_AAC_ENC_SPEC_EXP_TABLE_COMB[m][spec_exp] as i32);
    // the result as the reference builds compute it (register shifts, amounts modulo 32)
    let wrapped = if st >= 2 {
        accu.wrapping_shl(st.wrapping_sub(1) as u32)
    } else {
        accu.wrapping_shr(1i32.wrapping_sub(st) as u32)
    };
    if wrapped == MINVAL_DBL {
        return InvQuant::Minval;
    }
    let shift = 1 - st as i64;
    let accu = accu as i64;
    let v = if shift < 0 {
        if -shift >= 32 {
            return InvQuant::BadShift;
        }
        accu << -shift
    } else {
        if shift >= 32 {
            return InvQuant::BadShift;
        }
        accu >> shift
    };
    if v > u32::MAX as i64 {
        InvQuant::BadShift
    } else {
        InvQuant::Defined
    }
}

/// Whether `FDKaacEnc_invQuantizeLines()` of `q` with `gain` is defined and gives a value other
/// than `MINVAL_DBL`.
fn inv_quant_defined(q: i16, gain: i32) -> bool {
    inv_quant_class(q, gain) == InvQuant::Defined
}

/// Whether FDKaacEnc_calcSfbQuantEnergyAndDist() with `q` and `gain` keeps the inverse
/// quantization defined (up to the first line it rejects with `MAX_QUANT`).
fn en_dist_defined(q: &[i16], gain: i32) -> bool {
    for &v in q {
        if v != i16::MIN && (v as i32).abs() > MAX_QUANT {
            return true;
        }
        if !inv_quant_defined(v, gain) {
            return false;
        }
    }
    true
}

#[test]
fn calc_sfb_quant_energy_and_dist_random() {
    let mut rng = Rng::new(0x9a18);
    let mut skipped = 0;
    for k in 0..20000 {
        let n = rng.range(0, 64);
        let spec: Vec<FixpDbl> = (0..n).map(|_| spec_value(&mut rng)).collect();
        let q: Vec<i16> = (0..n)
            .map(|_| match rng.range(0, 6) {
                0 => [0, 8191, -8191, 8192, -8192, -32767, 32767, 1, -1][rng.range(0, 8) as usize],
                1 => match rng.i16() {
                    i16::MIN => 0,
                    v => v,
                },
                _ => rng.range(-200, 200) as i16,
            })
            .collect();
        // A gain for which the inverse quantization stays defined (see inv_quant_defined).
        match (0..16).map(|_| gain(&mut rng)).find(|&g| en_dist_defined(&q, g)) {
            Some(g) => check_en_dist(&format!("random {k}"), &spec, &q, g),
            None => skipped += 1,
        }
    }
    eprintln!("calcSfbQuantEnergyAndDist random: {skipped} cases without a defined gain");
    assert!(skipped < 1000);
}

/// The gains around the edge of the defined domain of the inverse quantization, for every
/// quantized magnitude class.
#[test]
fn calc_sfb_quant_energy_and_dist_gain_edges() {
    let mut rng = Rng::new(0x9a19);
    let mut n = 0;
    for e in 0..13 {
        for q in [1i16 << e, (1i16 << e) | 1, ((2i32 << e) - 1).min(8191) as i16] {
            for g in -400..400 {
                for qq in [q, -q] {
                    if en_dist_defined(&[qq, -1], g) {
                        check_en_dist("gain edges", &[rng.i32(), 3], &[qq, -1], g);
                        n += 1;
                    }
                }
            }
        }
    }
    assert!(n > 5000, "{n}");
}

/// `calc_sfb_quant_energy_and_dist_random` without the restriction to a defined inverse
/// quantization: quantized values of -32768 and every gain.
#[test]
fn calc_sfb_quant_energy_and_dist_random_undefined() {
    if APPLE_CLANG_X86 {
        eprintln!("skipped: the reference is ref-x86_64-mac (Apple clang)");
        return;
    }
    let mut rng = Rng::new(0x9a1c);
    let mut classes = [0usize; 4];
    for k in 0..20000 {
        let n = rng.range(0, 64);
        let spec: Vec<FixpDbl> = (0..n).map(|_| spec_value(&mut rng)).collect();
        let q: Vec<i16> = (0..n)
            .map(|_| match rng.range(0, 6) {
                0 => [0, 8191, -8191, 8192, -8192, -32768, 32767, 1, -1][rng.range(0, 8) as usize],
                1 => rng.i16(),
                _ => rng.range(-200, 200) as i16,
            })
            .collect();
        let g = gain(&mut rng);
        for &v in &q {
            classes[inv_quant_class(v, g) as usize] += 1;
        }
        check_en_dist(&format!("random undefined {k}"), &spec, &q, g);
    }
    eprintln!("calcSfbQuantEnergyAndDist random undefined: lines per class {classes:?}");
    assert!(classes.iter().all(|&c| c > 0), "{classes:?}");
}

/// The inverse quantization of every quantized magnitude class (and -32768) for the gains
/// -400..400 where it is undefined: `MINVAL_DBL` results, shifts by 32 or more and
/// `InvQuant::Minus32768` (reproduced from the object layout).
#[test]
fn calc_sfb_quant_energy_and_dist_undefined() {
    if APPLE_CLANG_X86 {
        eprintln!("skipped: the reference is ref-x86_64-mac (Apple clang)");
        return;
    }
    let mut rng = Rng::new(0x9a1d);
    let mut classes = [0usize; 4];
    for e in 0..13 {
        for q in [1i16 << e, (1i16 << e) | 1, ((2i32 << e) - 1).min(8191) as i16] {
            for g in -400..400 {
                for qq in [q, -q] {
                    let class = inv_quant_class(qq, g);
                    if class != InvQuant::Defined {
                        classes[class as usize] += 1;
                        check_en_dist("undefined", &[rng.i32(), 3], &[qq, -1], g);
                        check_en_dist("undefined", &[spec_value(&mut rng)], &[qq], g);
                    }
                }
            }
        }
    }
    // A quantized value of -32768 (specExp 16) with every gain residue.
    for g in -400..400 {
        classes[InvQuant::Minus32768 as usize] += 1;
        check_en_dist("q -32768", &[rng.i32()], &[-32768], g);
        check_en_dist("q -32768", &[rng.i32(), 3, -5], &[-32768, 32767, -32767], g);
    }
    eprintln!("calcSfbQuantEnergyAndDist undefined: cases per class {classes:?}");
    assert!(classes[InvQuant::Minval as usize] > 0, "{classes:?}");
    assert!(classes[InvQuant::BadShift as usize] > 0, "{classes:?}");
}

#[test]
fn quantize_spectrum_random() {
    let mut rng = Rng::new(0x9a1a);
    for k in 0..3000 {
        let groups = rng.range(1, 4);
        let per = rng.range(1, 60 / groups);
        let sfb_cnt = groups * per;
        let max_sfb = rng.range(0, per);
        let mut offsets = vec![0i32; 61];
        for i in 1..61 {
            offsets[i] = (offsets[i - 1] + 4 * rng.range(0, 6)).min(1024);
        }
        let spec: Vec<FixpDbl> = (0..1024).map(|_| spec_value(&mut rng)).collect();
        let scf: Vec<i32> = (0..60)
            .map(|_| if rng.range(0, 9) == 0 { rng.i32() } else { rng.range(-60, 60) })
            .collect();
        let g = gain(&mut rng);
        let dz = rng.range(0, 1);
        let q0: Vec<i16> = (0..1024).map(|_| rng.i16()).collect();
        let mut qr = q0.clone();
        let mut qc = q0.clone();
        fdk_aac_enc_quantize_spectrum(sfb_cnt, max_sfb, per, &offsets, &spec, g, &scf, &mut qr, dz);
        unsafe {
            c::cref_quantize_QuantizeSpectrum(
                sfb_cnt,
                max_sfb,
                per,
                offsets.as_ptr(),
                spec.as_ptr(),
                g,
                scf.as_ptr(),
                qc.as_mut_ptr(),
                dz,
            )
        };
        assert_eq!(qr, qc, "random {k}");
    }
}

/// Real spectra, with the scalefactors around the values the scalefactor estimation finds
/// (the gain that maps the band maximum to about 8191, and finer/coarser values).
#[test]
fn realistic() {
    let mut rng = Rng::new(0x9a1b);
    for k in 0..300 {
        let d = channel(&mut rng, k % 3 == 2);
        let mut scf = [0i32; 60];
        let mut sfb_offs = 0;
        while sfb_offs < d.sfb_cnt {
            for sfb in 0..d.max_sfb_per_group {
                let i = (sfb_offs + sfb) as usize;
                let (lo, hi) = (d.sfb_offsets[i] as usize, d.sfb_offsets[i + 1] as usize);
                let max = d.spectrum[lo..hi].iter().map(|&x| (x as i64).abs()).max().unwrap_or(0);
                // minSfMaxQuant of sf_estim: about -69.33 + 4 * log2(maxSpec)
                let l2 = 63 - (max.max(1) as u64).leading_zeros() as i32;
                let base = 4 * (l2 - 31) - 69;
                let g = base + rng.range(-8, 30);
                scf[i] = g;
                for dz in 0..2 {
                    let q0: Vec<i16> = (lo..hi).map(|_| rng.i16()).collect();
                    check_dist(&format!("realistic {k} sfb {i}"), &d.spectrum[lo..hi], &q0, g, dz);
                    let mut q = vec![0i16; hi - lo];
                    fdk_aac_enc_calc_sfb_dist(&d.spectrum[lo..hi], &mut q, (hi - lo) as i32, g, dz);
                    check_en_dist(
                        &format!("realistic {k} sfb {i}"),
                        &d.spectrum[lo..hi],
                        &q,
                        g + rng.range(-3, 0),
                    );
                }
            }
            sfb_offs += d.sfb_per_group;
        }
        let global_gain = *scf.iter().max().unwrap();
        let scf_rel: Vec<i32> = scf.iter().map(|&s| global_gain - s).collect();
        for dz in 0..2 {
            let mut qr = vec![7i16; 1024];
            let mut qc = qr.clone();
            fdk_aac_enc_quantize_spectrum(
                d.sfb_cnt,
                d.max_sfb_per_group,
                d.sfb_per_group,
                &d.sfb_offsets,
                &d.spectrum,
                global_gain,
                &scf_rel,
                &mut qr,
                dz,
            );
            unsafe {
                c::cref_quantize_QuantizeSpectrum(
                    d.sfb_cnt,
                    d.max_sfb_per_group,
                    d.sfb_per_group,
                    d.sfb_offsets.as_ptr(),
                    d.spectrum.as_ptr(),
                    global_gain,
                    scf_rel.as_ptr(),
                    qc.as_mut_ptr(),
                    dz,
                )
            };
            assert_eq!(qr, qc, "realistic {k} QuantizeSpectrum");
        }
    }
}
