// Tests of the optimized FDKaacEnc_Downsample (the NEON wavefront cascade on AArch64, the AVX2
// one on x86) against its straight port (PORTING.md, "Optimization rules"); resampler.rs
// tests the public function against the C++ reference.
//
// Random filter states (including the extremes, where the wrapping sums overflow), both ring
// buffer phases, every parameter set and section count, ratios 1 to 3, block lengths that are
// not multiples of the ratio or of the chunk size, and input/output positions at the edges of
// the buffer. The whole buffer, the state and the returned values are compared.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrenc::resampler::*;

fn compare(what: &str, d: &Downsampler, buf: &[i16], in_pos: usize, n: i32, out_pos: usize) {
    let (mut ds, mut df) = (*d, *d);
    let (mut bs, mut bf) = (buf.to_vec(), buf.to_vec());
    let (mut ns, mut nf) = (-7, -7);
    let rs = fdk_aac_enc_downsample_straight(&mut ds, &mut bs, in_pos, n, out_pos, &mut ns);
    let rf = fdk_aac_enc_downsample(&mut df, &mut bf, in_pos, n, out_pos, &mut nf);
    assert_eq!((rs, ns), (rf, nf), "{what}");
    assert_eq!(bs, bf, "{what}: buffer");
    assert_eq!(ds, df, "{what}: state");
}

fn sample(rng: &mut Rng, kind: i32) -> i16 {
    match kind {
        0 => rng.i16(),
        1 => [i16::MIN, i16::MAX][rng.range(0, 1) as usize],
        _ => rng.i16() >> rng.range(0, 15),
    }
}

#[test]
fn downsample_fast_vs_straight() {
    let mut rng = Rng::new(0x5eed_0f01);
    let mut sections = [0usize; 16];
    for k in 0..3000 {
        let mut d = Downsampler::default();
        let wc = [0, 250, 350, 410, 450, 500][rng.range(0, 5) as usize];
        let ratio = rng.range(1, 3);
        fdk_aac_enc_init_downsampler(&mut d, wc, ratio);
        // fewer sections than the parameter set (the vector code pads the lanes)
        if rng.range(0, 3) == 0 {
            d.down_filter.no_coeffs = rng.range(0, d.down_filter.no_coeffs);
        }
        sections[d.down_filter.no_coeffs as usize] += 1;
        if rng.range(0, 1) == 0 {
            for s in d.down_filter.states.iter_mut().flatten() {
                *s = match rng.range(0, 3) {
                    0 => rng.i32(),
                    1 => [i32::MIN, i32::MAX, 0][rng.range(0, 2) as usize],
                    _ => rng.i32() >> rng.range(0, 31),
                };
            }
        }
        d.down_filter.ptr = rng.range(0, 1);
        let kind = rng.range(0, 2);
        let n = match rng.range(0, 3) {
            0 => rng.range(0, 40),
            1 => 2048,
            _ => rng.range(1, 3000),
        };
        let offset = rng.range(0, 1600) as usize;
        let len = offset + n.max(0) as usize + rng.range(0, 4) as usize;
        let buf: Vec<i16> = (0..len).map(|_| sample(&mut rng, kind)).collect();
        // out_pos <= in_pos as the SBR encoder calls it, also the other way round and past the end
        let (in_pos, out_pos) = match rng.range(0, 9) {
            0 => (0, offset),
            1 => (offset, len),
            _ => (offset, rng.range(0, offset as i32) as usize),
        };
        let what = format!("{k}: wc {wc} ratio {ratio} n {n} in {in_pos} out {out_pos} len {len}");
        compare(&what, &d, &buf, in_pos, n, out_pos);
    }
    assert!(sections.iter().filter(|&&c| c > 0).count() >= 12, "{sections:?}");
}

/// Consecutive frames with the state carried over, as the SBR encoder uses it.
#[test]
fn downsample_fast_vs_straight_frames() {
    let mut rng = Rng::new(0x5eed_0f02);
    for &wc in &[250, 350, 410, 450, 500] {
        for ratio in 1..=3 {
            let mut ds = Downsampler::default();
            fdk_aac_enc_init_downsampler(&mut ds, wc, ratio);
            let mut df = ds;
            for f in 0..30 {
                let n = if f % 5 == 4 { rng.range(1, 2100) } else { 2048 };
                let offset = 1537;
                let kind = f % 3;
                let buf: Vec<i16> =
                    (0..offset + n as usize).map(|_| sample(&mut rng, kind)).collect();
                let (mut bs, mut bf) = (buf.clone(), buf);
                let (mut ns, mut nf) = (0, 0);
                fdk_aac_enc_downsample_straight(&mut ds, &mut bs, offset, n, 0, &mut ns);
                fdk_aac_enc_downsample(&mut df, &mut bf, offset, n, 0, &mut nf);
                assert_eq!((&bs, ns), (&bf, nf), "wc {wc} ratio {ratio} frame {f}");
                assert_eq!(ds, df, "wc {wc} ratio {ratio} frame {f}: state");
            }
        }
    }
}

/// A coefficient of -32768 (none of the tables has one).
#[test]
fn downsample_fast_vs_straight_minval_coefficient() {
    let mut rng = Rng::new(0x5eed_0f03);
    let mut coeffs = FILTER_PARAM_SET[0].coeffa.to_vec();
    for k in 0..200 {
        let i = rng.range(0, coeffs.len() as i32 - 1) as usize;
        coeffs[i] = i16::MIN;
        let coeffa: &'static [i16] = Box::leak(coeffs.clone().into_boxed_slice());
        let mut d = Downsampler::default();
        fdk_aac_enc_init_downsampler(&mut d, 500, 2);
        d.down_filter.coeffa = Some(coeffa);
        for s in d.down_filter.states.iter_mut().flatten() {
            *s = [i32::MIN, rng.i32()][rng.range(0, 1) as usize];
        }
        let buf: Vec<i16> = (0..600).map(|_| sample(&mut rng, k % 3)).collect();
        compare(&format!("minval {k}"), &d, &buf, 100, 500, 0);
    }
}
