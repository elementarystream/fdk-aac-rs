// Tests of the optimized block switching of a channel pair (fdk_aac_enc_block_switching_pair,
// window_energies_pair) against two calls of the straight port (PORTING.md, "Optimization
// rules"); block_switch.rs tests the straight port against the C++ reference.

use crate::block_switch::random_state;
use fdk_aac_rs_aacenc::block_switch::*;
use fdk_aac_rs_cref::testutil::Rng;

fn pcm(r: &mut Rng) -> Vec<i16> {
    match r.next_u32() % 4 {
        0 => (0..1024).map(|_| r.i16()).collect(),
        1 => (0..1024).map(|_| if r.next_u32() % 2 == 0 { i16::MAX } else { i16::MIN }).collect(),
        2 => (0..1024).map(|_| 0).collect(),
        _ => {
            let shift = r.next_u32() % 16;
            (0..1024).map(|_| (r.next_u32() as i16) >> shift).collect()
        }
    }
}

#[test]
fn block_switching_pair_vs_straight() {
    let mut r = Rng::new(0xb5f1);
    let granules = [1024, 960, 512, 480, 256, 240, 128, 120, 0, -8];
    for it in 0..30000 {
        // usually the same configuration for both channels (as in a channel pair), sometimes
        // not (mixed AAC-LC/LD statics), and LFE flags
        let ld = r.range(0, 1);
        let ld1 = if r.next_u32() % 8 == 0 { 1 - ld } else { ld };
        let (a0, a1) = (random_state(&mut r, ld), random_state(&mut r, ld1));
        let granule = granules[(r.next_u32() % granules.len() as u32) as usize];
        let is_lfe = [(r.next_u32() % 16 == 0) as i32, (r.next_u32() % 16 == 0) as i32];
        let (p0, p1) = (pcm(&mut r), pcm(&mut r));
        let (mut s0, mut s1) = (a0, a1);
        let rs = [
            fdk_aac_enc_block_switching(&mut s0, granule, is_lfe[0], &p0),
            fdk_aac_enc_block_switching(&mut s1, granule, is_lfe[1], &p1),
        ];
        let (mut f0, mut f1) = (a0, a1);
        let rf = fdk_aac_enc_block_switching_pair([&mut f0, &mut f1], granule, is_lfe, [&p0, &p1]);
        assert_eq!((rs, s0, s1), (rf, f0, f1), "iteration {it}");
    }
}

/// Consecutive frames of a pair, the states carried over.
#[test]
fn block_switching_pair_sequences() {
    let mut r = Rng::new(0xb5f2);
    for ld in 0..2 {
        let granule = if ld == 0 { 1024 } else { 480 };
        let mut s = [BlockSwitchingControl::default(); 2];
        fdk_aac_enc_init_block_switching(&mut s[0], ld);
        fdk_aac_enc_init_block_switching(&mut s[1], ld);
        let mut f = s;
        for frame in 0..3000 {
            let (p0, p1) = (pcm(&mut r), pcm(&mut r));
            let [s0, s1] = &mut s;
            let rs = [
                fdk_aac_enc_block_switching(s0, granule, 0, &p0),
                fdk_aac_enc_block_switching(s1, granule, 0, &p1),
            ];
            let [f0, f1] = &mut f;
            let rf = fdk_aac_enc_block_switching_pair([f0, f1], granule, [0, 0], [&p0, &p1]);
            assert_eq!((rs, s), (rf, f), "ld {ld} frame {frame}");
        }
    }
}
