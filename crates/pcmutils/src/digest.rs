// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the state digest of the PCM utilities (see fdk_aac_rs_fdk::digest).

//! [`StateDigest`] of the limiter and the downmix (not in C; see `fdk_aac_rs_fdk::digest`).

use fdk_aac_rs_fdk::digest::{Digester, StateDigest};
use fdk_aac_rs_fdk::impl_state_digest;

use crate::limiter::TDLimiter;
use crate::pcmdmx_lib::{DmxBsMetaData, PcmDmxInstance, PcmDmxUserParams};

/// The limiter keeps two ring buffers whose positions advance with every sample: the running
/// maximum window `maxBuf[0..=attack]` (next write position `maxBufIdx`) and the lookahead delay
/// line `delayBuf` (`attack` slots of `channels` values, next slot `delayBufIdx`). Every access
/// is relative to the positions, except the search of the maximum over the whole window (order
/// independent) and the rescaling of both windows (element-wise), so they are fed in logical
/// order, starting at the next write position. The values beyond the windows (left by a longer
/// attack time) and the physical layout only matter when `attack` or `channels` change without
/// `pcmLimiter_Reset()`, which the chunked decoding treats as a hazard
/// ([`TDLimiter::digest_layout`]). A position outside its window (only after such a change) is
/// fed with the buffers in physical order.
impl StateDigest for TDLimiter {
    fn digest(&self, d: &mut Digester) {
        let TDLimiter {
            attack,
            attack_const,
            release_const,
            attack_ms,
            release_ms,
            max_attack_ms,
            threshold,
            channels,
            max_channels,
            sample_rate,
            max_sample_rate,
            cor,
            max,
            max_buf,
            delay_buf,
            max_buf_idx,
            delay_buf_idx,
            smooth_state0,
            min_gain,
            scaling,
        } = self;
        d.field("attack", attack);
        d.field("attack_const", attack_const);
        d.field("release_const", release_const);
        d.field("attack_ms", attack_ms);
        d.field("release_ms", release_ms);
        d.field("max_attack_ms", max_attack_ms);
        d.field("threshold", threshold);
        d.field("channels", channels);
        d.field("max_channels", max_channels);
        d.field("sample_rate", sample_rate);
        d.field("max_sample_rate", max_sample_rate);
        d.field("cor", cor);
        d.field("max", max);
        d.field("smooth_state0", smooth_state0);
        d.field("min_gain", min_gain);
        d.field("scaling", scaling);
        let win = *attack as usize + 1;
        let slots = *attack as usize;
        let ch = *channels as usize;
        let rotatable = (*max_buf_idx as usize) < win
            && win <= max_buf.len()
            && ((*delay_buf_idx as usize) < slots || slots == 0)
            && slots.checked_mul(ch).is_some_and(|n| n <= delay_buf.len());
        d.field("rotatable", &rotatable);
        if rotatable {
            let i = *max_buf_idx as usize;
            d.field_with("max_buf", |d| {
                d.u64(win as u64);
                for v in max_buf[i..win].iter().chain(&max_buf[..i]) {
                    v.digest(d);
                }
            });
            let j = *delay_buf_idx as usize * ch;
            let n = slots * ch;
            d.field_with("delay_buf", |d| {
                d.u64(n as u64);
                if n > 0 {
                    for v in delay_buf[j..n].iter().chain(&delay_buf[..j]) {
                        v.digest(d);
                    }
                }
            });
        } else {
            d.field("max_buf_idx", max_buf_idx);
            d.field("delay_buf_idx", delay_buf_idx);
            d.field("max_buf", max_buf);
            d.field("delay_buf", delay_buf);
        }
    }
}

impl TDLimiter {
    /// Not in C: what decides how the ring buffers are laid out (see the [`StateDigest`]
    /// implementation): the chunked decoding signals a hazard when this changes during a decoder
    /// call.
    pub fn digest_layout(&self) -> (u32, u32, usize, usize) {
        (self.attack, self.channels, self.max_buf.len(), self.delay_buf.len())
    }
}

impl_state_digest!(DmxBsMetaData {
    type_flags,
    c_lev_idx,
    s_lev_idx,
    dmix_idx_a,
    dmix_idx_b,
    dmix_idx_lfe,
    dmx_gain_idx2,
    dmx_gain_idx5,
    matrix_mixdown_idx,
    pseudo_surround,
    expiry_count,
});
impl_state_digest!(PcmDmxUserParams {
    dmx_profile,
    expiry_frame,
    dual_channel_mode,
    pseudo_surr_mode,
    num_out_channels_min,
    num_out_channels_max,
    frame_delay,
});
impl_state_digest!(PcmDmxInstance { bs_meta_data, user_params, apply_processing });

#[cfg(test)]
mod tests {
    use crate::limiter::*;
    use fdk_aac_rs_fdk::digest::digest_of;

    /// Two limiters whose ring buffers hold the same values at different positions have equal
    /// digests and process the same input identically.
    #[test]
    fn limiter_digest_is_independent_of_the_ring_positions() {
        let new = || pcm_limiter_create(15, 50, 0x7fff_0000 >> 2, 2, 48_000).unwrap();
        let (mut a, mut b) = (new(), new());
        let signal = |n: usize, amp: f64, ph: usize| -> Vec<i32> {
            (0..2 * n)
                .map(|i| (amp * ((i / 2 + ph) as f64 * 0.031).sin() * 2147483647.0) as i32)
                .collect()
        };
        let run = |l: &mut TDLimiter, x: &[i32]| -> Vec<i16> {
            let mut out = vec![0i16; x.len()];
            pcm_limiter_apply(Some(l), x, &mut out, None, 0, (x.len() / 2) as u32);
            out
        };
        // a starts with 300 extra samples: its positions differ from b's from then on
        run(&mut a, &signal(300, 0.0, 0));
        for x in [signal(4000, 0.9, 7), signal(40_000, 0.01, 3)] {
            run(&mut a, &x);
            run(&mut b, &x);
        }
        assert_ne!(a.max_buf_idx, b.max_buf_idx);
        assert_eq!(digest_of(&*a), digest_of(&*b));
        for x in [signal(3000, 0.95, 11), signal(5000, 0.3, 5)] {
            assert_eq!(run(&mut a, &x), run(&mut b, &x));
        }
        assert_eq!(digest_of(&*a), digest_of(&*b));
        // a different delay line content gives a different digest
        let i = a.delay_buf_idx as usize * 2;
        a.delay_buf[i] ^= 1;
        assert_ne!(digest_of(&*a), digest_of(&*b));
    }
}
