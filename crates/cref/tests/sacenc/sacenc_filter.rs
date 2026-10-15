// Differential tests of fdk_aac_rs_sacenc::sacenc_filter against the C++ reference.
//
// The DC filter coefficient for every sampling rate the encoder accepts (0..=48000) and some
// beyond; the filter on random and realistic PCM in blocks of the QMF band counts, in place
// and out of place, with the state carried over and compared after every block.

use fdk_aac_rs_cref::sacenc_sacenc_filter as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacenc::sacenc_filter as r;
use fdk_aac_rs_sacenc::sacenc_filter::DcFilter;

/// A PCM signal: noise, a sine with DC offset, silence or full scale.
pub fn pcm(rng: &mut Rng, n: usize) -> Vec<i16> {
    let kind = rng.range(0, 4);
    let amp = [32767, 16000, 1000, 30][rng.range(0, 3) as usize];
    let dc = rng.range(-3000, 3000);
    let f = (rng.range(1, 2000) as f64) / 48000.0;
    let ph = rng.range(0, 1000) as f64;
    (0..n)
        .map(|i| match kind {
            0 => rng.i16(),
            1 => ((amp as f64 * (2.0 * std::f64::consts::PI * f * (i as f64 + ph)).sin()) as i32 + dc)
                .clamp(-32768, 32767) as i16,
            2 => 0,
            3 => if (i / 7) % 2 == 0 { 32767 } else { -32768 },
            _ => (rng.range(-amp, amp) + dc).clamp(-32768, 32767) as i16,
        })
        .collect()
}

#[test]
fn init() {
    let mut rng = Rng::new(1);
    let h = unsafe { c::cref_sacenc_filter_create() };
    let mut rates: Vec<u32> = vec![1, 8000, 16000, 22050, 24000, 32000, 44100, 48000, 96000, u32::MAX];
    rates.extend((0..3000).map(|_| rng.range(1, 48000) as u32));
    // A sampling rate of 0: no defined x86 reference result (x86 division by zero).
    if !cfg!(any(target_arch = "x86_64", target_arch = "x86")) {
        rates.push(0);
    }
    for sr in rates {
        let ec = unsafe { c::cref_sacenc_filter_init(h, sr) };
        let mut f = DcFilter { c_fdk: 5, state_fdk: 7 };
        let er = r::fdk_sacenc_init_dc_filter(&mut f, sr);
        let (mut cc, mut sc) = (0, 0);
        unsafe { c::cref_sacenc_filter_get(h, &mut cc, &mut sc) };
        assert_eq!((ec, cc, sc), (er, f.c_fdk, f.state_fdk), "sample rate {sr}");
    }
    assert_eq!(unsafe { c::cref_sacenc_filter_destroy(h) }, 0);
    assert_eq!(unsafe { c::cref_sacenc_filter_create_null() }, r::fdk_sacenc_create_dc_filter(None));
}

#[test]
fn apply() {
    let mut rng = Rng::new(2);
    for it in 0..400 {
        let h = unsafe { c::cref_sacenc_filter_create() };
        let sr = [16000, 22050, 24000, 32000, 44100, 48000, rng.range(1, 48000)][rng.range(0, 6) as usize] as u32;
        let mut f = DcFilter::default();
        unsafe { c::cref_sacenc_filter_init(h, sr) };
        r::fdk_sacenc_init_dc_filter(&mut f, sr);
        if rng.range(0, 3) == 0 {
            let (cf, st) = (rng.i32(), rng.i32());
            unsafe { c::cref_sacenc_filter_set(h, cf, st) };
            f = DcFilter { c_fdk: cf, state_fdk: st };
        }
        let blk = [32, 64, rng.range(0, 70)][rng.range(0, 2) as usize] as usize;
        for b in 0..30 {
            let x = pcm(&mut rng, blk.max(1));
            let in_place = rng.range(0, 1);
            let mut oc = if in_place != 0 { x.clone() } else { vec![0x1234; blk.max(1)] };
            let ec = unsafe { c::cref_sacenc_filter_apply(h, x.as_ptr(), oc.as_mut_ptr(), blk as i32, in_place) };
            let mut or = vec![0x1234i16; blk.max(1)];
            let er = if in_place != 0 {
                or = x.clone();
                r::fdk_sacenc_apply_dc_filter_in_place(&mut f, &mut or, blk as i32)
            } else {
                r::fdk_sacenc_apply_dc_filter(&mut f, &x, &mut or, blk as i32)
            };
            let (mut cc, mut sc) = (0, 0);
            unsafe { c::cref_sacenc_filter_get(h, &mut cc, &mut sc) };
            assert_eq!((ec, cc, sc), (er, f.c_fdk, f.state_fdk), "it {it} block {b}");
            assert_eq!(oc, or, "it {it} block {b}");
        }
        unsafe { c::cref_sacenc_filter_destroy(h) };
    }
}
