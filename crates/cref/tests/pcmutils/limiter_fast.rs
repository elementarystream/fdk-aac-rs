// Tests of the optimized pcmLimiter_Apply() against the straight port, which keeps its
// differential tests against C (limiter.rs; those run the optimized version as well).

use super::limiter::{gains, random_scaling, random_threshold, signal};
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_pcmutils::limiter::*;

/// Random sequences of pcmLimiter_Apply() calls and parameter changes on two copies of a
/// limiter, one with the optimized and one with the straight version: return value, output
/// (including the samples behind the processed ones) and the complete limiter state.
#[test]
fn pcm_limiter_apply_fast_vs_straight() {
    let sample_rates = [8000, 16000, 22050, 32000, 44100, 48000, 96000];
    let (mut limited, mut transparent) = (0, 0);
    for seed in 0..300u64 {
        let mut rng = Rng::new(0x11a1_0000 + seed);
        let max_attack_ms = 1 + rng.next_u32() % 20;
        let max_channels = 1 + rng.next_u32() % 8;
        let sr = sample_rates[(rng.next_u32() % 7) as usize];
        let release = rng.next_u32() % 400;
        let mut a = pcm_limiter_create(max_attack_ms, release, random_threshold(&mut rng), max_channels, sr).unwrap();
        let mut b = a.clone();
        let mut scaling = random_scaling(&mut rng);
        for _ in 0..80 {
            match rng.next_u32() % 16 {
                0 => {
                    let t = random_threshold(&mut rng);
                    pcm_limiter_set_threshold(Some(&mut a), t);
                    pcm_limiter_set_threshold(Some(&mut b), t);
                }
                1 => {
                    let ms = rng.next_u32() % (max_attack_ms + 3);
                    pcm_limiter_set_attack(Some(&mut a), ms);
                    pcm_limiter_set_attack(Some(&mut b), ms);
                }
                2 => {
                    // 0 channels too (no channel loop)
                    let n = rng.next_u32() % (max_channels + 1);
                    pcm_limiter_set_n_channels(Some(&mut a), n);
                    pcm_limiter_set_n_channels(Some(&mut b), n);
                }
                3 => {
                    let s = sample_rates[(rng.next_u32() % 7) as usize];
                    pcm_limiter_set_sample_rate(Some(&mut a), s);
                    pcm_limiter_set_sample_rate(Some(&mut b), s);
                }
                4 => scaling = random_scaling(&mut rng),
                _ => {
                    let ch = a.channels as usize;
                    let n = match rng.next_u32() % 4 {
                        0 => rng.next_u32() as usize % 4,
                        1 => 1024,
                        _ => 1 + rng.next_u32() as usize % 700,
                    };
                    let input = signal(&mut rng, n, ch);
                    let g = if rng.next_u32() % 3 == 0 { Some(gains(&mut rng, n)) } else { None };
                    let mut out_a = vec![0x5A5Ai16; n * ch + 4];
                    let mut out_b = out_a.clone();
                    let ea = pcm_limiter_apply_straight(Some(&mut a), &input, &mut out_a, g.as_deref(), scaling, n as u32);
                    let eb = pcm_limiter_apply(Some(&mut b), &input, &mut out_b, g.as_deref(), scaling, n as u32);
                    assert_eq!(ea, eb, "seed {seed}");
                    assert_eq!(out_a, out_b, "seed {seed}: output (n {n}, ch {ch})");
                    if a.min_gain < 0x4000_0000 {
                        limited += 1;
                    } else {
                        transparent += 1;
                    }
                }
            }
            assert_eq!(*a, *b, "seed {seed}: state");
        }
    }
    assert!(limited > 1000 && transparent > 1000, "{limited} {transparent}");
    assert_eq!(pcm_limiter_apply(None, &[], &mut [], None, 0, 1), pcm_limiter_apply_straight(None, &[], &mut [], None, 0, 1));
}
