// Differential tests of the encoder's run-time floating point expressions outside
// x86/fixpoint_math_x86.h, with the inputs where the x87 unit of the x86_32 variant (ref-i686)
// and the SSE code of the x86 variant differ (PORTING.md, "Architecture variants"): operands
// and products that need more than the 24 bits of a `float`. They run on every variant.

use fdk_aac_rs_aacenc::aacenc_tns::fdk_aac_enc_calc_gauss_window;
use fdk_aac_rs_aacenc::adj_thr::{fdk_aac_enc_bits2pe2, fdk_aac_enc_calc_pe_correction};
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_cref::{aacenc_aacenc_tns as tns, aacenc_adj_thr as adj};

/// `(INT)(transformResolution * 1000.f)` with transform resolutions above 2^24 / 1000.
#[test]
fn calc_gauss_window_large_transform_resolution() {
    let mut r = Rng::new(0x7837_0001);
    let check = |r: &mut Rng, res: i32| {
        let sr = r.range(1, 400_000);
        let tr = r.range(1, i32::MAX);
        let tre = r.range(-3, 4);
        let size = r.range(1, 15);
        let fill = r.i32();
        let (mut wc, mut wr) = ([fill; 16], [fill; 16]);
        unsafe { tns::cref_aacenc_tns_CalcGaussWindow(wc.as_mut_ptr(), size, sr, res, tr, tre) };
        fdk_aac_enc_calc_gauss_window(&mut wr, size, sr, res, tr, tre);
        assert_eq!(wr, wc, "size {size} sr {sr} res {res} tr {tr:#x} e {tre}");
    };
    // The product stays below 2^31 up to 2147483; it needs more than 24 bits from 16778 on.
    for res in [16_777, 16_778, 16_779, 100_001, 1_000_003, 2_147_483] {
        check(&mut r, res);
    }
    for _ in 0..20_000 {
        let res = r.range(16_778, 2_147_483);
        check(&mut r, res);
    }
}

/// `peAct < 1.5f * peLast && peAct > 0.7f * peLast` next to the bounds, where the `float`
/// products of the 64-bit builds round and the x87 products of ref-i686 are exact. The bit
/// counts are chosen so that the other conditions of the correction hold.
#[test]
fn calc_pe_correction_bounds() {
    use fdk_aac_rs_fdk::common_fix::{f_mult, fl2fxconst_dbl};
    let mut rng = Rng::new(0x7837_0002);
    let mut sensitive = 0;
    for _ in 0..300_000 {
        let pe_last = rng.range(1, 200_000);
        let be = rng.range(1, 3);
        let bm = rng.range(0x0800_0000, 0x6000_0000);
        // bits * 0.65 * factor < peLast < bits * 1.2 * factor
        let factor = bm as f64 / 2147483648.0 * (1 << be) as f64;
        let bits_last = ((pe_last as f64 / (factor * rng.range(66, 119) as f64 / 100.0)) as i32).clamp(1, 16383);
        // Integers next to 0.7f * peLast and 1.5f * peLast.
        let anchor = if rng.next_u32() % 2 == 0 {
            (pe_last as f64 * 0.7f32 as f64).round() as i32
        } else {
            pe_last * 3 / 2
        };
        let pe_act = anchor + rng.range(-1, 1);
        let float_in = (pe_act as f32) < 1.5f32 * (pe_last as f32) && (pe_act as f32) > 0.7f32 * (pe_last as f32);
        // 0.7f = 11744051 / 2^24
        let exact_in = 2 * (pe_act as i64) < 3 * pe_last as i64 && ((pe_act as i64) << 24) > 11_744_051 * pe_last as i64;
        let others = fdk_aac_enc_bits2pe2(bits_last, f_mult(fl2fxconst_dbl((1.2f32 / 2.0f32) as f64), bm), be + 1) > pe_last
            && fdk_aac_enc_bits2pe2(bits_last, f_mult(fl2fxconst_dbl(0.65f32 as f64), bm), be) < pe_last;
        if float_in != exact_in && others {
            sensitive += 1;
        }
        let mut m0 = rng.range(0x3000_0000, 0x4a00_0000);
        let mut e0 = rng.range(0, 3);
        let (mut m1, mut e1) = (m0, e0);
        fdk_aac_enc_calc_pe_correction(&mut m0, &mut e0, pe_act, pe_last, bits_last, bm, be);
        unsafe { adj::cref_adj_thr_calcPeCorrection(&mut m1, &mut e1, pe_act, pe_last, bits_last, bm, be) };
        assert_eq!((m0, e0), (m1, e1), "{pe_act} {pe_last} {bits_last} {bm:#x} {be}");
    }
    // The inputs where float and exact comparisons disagree must be reached.
    assert!(sensitive > 0);
    eprintln!("calc_pe_correction_bounds: {sensitive} inputs where float and x87 disagree");
}

// The x87 forms of the three encoder expressions (PORTING.md, "Floating point and the x87
// variant") against the software model `fdk_aac_rs_fdk::x87::soft` on every host, and against
// the x87 instructions `fdk_aac_rs_fdk::x87::hw` on x86 and x86-64 hosts, independently of the
// variant.

/// Operands near the bounds where products need more than 24 bits, and random magnitudes.
fn x87_operands(r: &mut Rng, n: usize) -> Vec<i32> {
    let mut v = vec![0, 1, -1, 16_777, 16_778, 2_147_483, 2_147_484, -2_147_483, -2_147_484];
    v.extend([i32::MAX, i32::MIN, i32::MAX - 1, i32::MIN + 1, 1 << 24, (1 << 24) + 1]);
    for _ in 0..n {
        let m = (r.next_u32() >> 1) >> (r.next_u32() % 31);
        v.push(if r.next_u32() % 2 == 0 { m as i32 } else { (m as i32).wrapping_neg() });
    }
    v
}

/// `(INT)(transformResolution * 1000.f)` of `FDKaacEnc_CalcGaussWindow` (x87: `fildl`, `fmuls`,
/// `fistpl`; the integer product, `INT_MIN` out of range).
#[test]
fn calc_gauss_window_product_x87() {
    use fdk_aac_rs_aacenc::aacenc_tns::transform_resolution_ms_x87;
    use fdk_aac_rs_fdk::x87::soft;
    let mut r = Rng::new(0x7837_0003);
    for op in x87_operands(&mut r, 200_000) {
        let v = transform_resolution_ms_x87(op);
        assert_eq!(v, soft::mul_trunc(op, 1000.0), "{op}");
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        assert_eq!(v, fdk_aac_rs_fdk::x87::hw::mul_trunc(op, 1000.0), "{op}");
    }
}

/// `peAct < 1.5f * peLast && peAct > 0.7f * peLast` of `calcPeCorrection` (x87: `fildl`,
/// `fmuls`, `fucomip`; the exact comparisons).
#[test]
fn calc_pe_correction_range_x87() {
    use fdk_aac_rs_aacenc::adj_thr::pe_correction_in_range_x87;
    use fdk_aac_rs_fdk::x87::soft;
    let mut r = Rng::new(0x7837_0004);
    for l in x87_operands(&mut Rng::new(0x7837_0006), 50_000) {
        let a0 = r.i32();
        for anchor in [(l as f64 * 0.7f32 as f64) as i32, (l as f64 * 1.5) as i32, a0] {
            for d in -2..=2 {
                let a = anchor.wrapping_add(d);
                let v = pe_correction_in_range_x87(a, l);
                assert_eq!(v, soft::between_products(a, l, 0.7, 1.5), "{a} {l}");
                #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
                assert_eq!(v, fdk_aac_rs_fdk::x87::hw::between_products(a, l, 0.7, 1.5), "{a} {l}");
            }
        }
    }
}

/// The DRC `limDecay` of `FDK_DRC_Generator_Initialize`:
/// `FL2FXCONST_DBL(((0.006f / 256) * blockLength) / METADATA_INT_SCALE)` on the x87 unit.
#[test]
fn drc_lim_decay_x87() {
    use fdk_aac_rs_fdk::x87::{self, soft};
    let (c1, c2) = (0.006f32 / 256.0, 1.0f32 / 1024.0);
    let mut r = Rng::new(0x7837_0005);
    let mut ops: Vec<i32> = (-5000..=5000).collect();
    ops.extend(x87_operands(&mut r, 50_000));
    for op in ops {
        let v = soft::fl2fxconst_dbl_mul2(op, c1, c2);
        assert_eq!(x87::fl2fxconst_dbl_mul2(op, c1, c2), v, "{op}");
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        assert_eq!(x87::hw::fl2fxconst_dbl_mul2(op, c1, c2), v, "{op}");
    }
}
