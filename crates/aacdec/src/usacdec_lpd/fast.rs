// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized version of bass_pf_1sf_delay() and filtLP() of libAACdec/src/usacdec_lpd.cpp.

//! The bass post-filter (an optimization of the port; the straight ports are
//! [`super::bass_pf_1sf_delay_ref`] and [`super::filt_lp`]).
//!
//! [`bass_pf_1sf_delay`] does the operations of the straight port in the same order. It differs
//! only in how it reaches the synthesis signal: the straight port copies every window of
//! `syn[]` into a local array, reading 0 outside of the buffer; here a window that lies inside
//! the buffer is used in place (the same values), and only a window that does not is copied as
//! before. The loops then run over slices without per-access checks.
//!
//! [`filt_lp_fast`]: the outputs of `filtLP()` are independent (`noise[]` is only read), and its
//! sums wrap, so 4 (NEON) or 8 (AVX2) outputs are computed at once with the same products:
//! `fMultDiv2(FIXP_DBL a, FIXP_SGL b)` is `(a * b) >> 16` and `fMult(a, b)` is
//! `((a * b) >> 16) << 1` on every variant. NEON computes `(a * b) >> 16` as `sqdmulh(a, b << 15)`
//! (`b << 15` is never `INT_MIN`, so it does not saturate), AVX2 exactly in 32-bit lanes as
//! `(a >> 16) * b + (((a & 0xFFFF) * b) >> 16)` (no intermediate value exceeds 31 bits); the
//! `<< 1` wraps as in C.

use super::*;

/// `buf[start..start + n]` if it lies inside `buf`.
#[inline(always)]
fn win(buf: &[FixpDbl], start: isize, n: usize) -> Option<&[FixpDbl]> {
    if start >= 0 {
        buf.get(start as usize..(start as usize).checked_add(n)?)
    } else {
        None
    }
}

/// The values of `buf` from the signed index `start` (`tmp.len()` of them, 0 outside of the
/// buffer): the window itself if it lies inside `buf`, otherwise a copy in `tmp` as
/// [`window_copy`] makes it.
#[inline(always)]
fn win_or_copy<'a>(buf: &'a [FixpDbl], start: isize, tmp: &'a mut [FixpDbl]) -> &'a [FixpDbl] {
    match win(buf, start, tmp.len()) {
        Some(s) => s,
        None => {
            window_copy(buf, start, tmp);
            tmp
        }
    }
}

/// [`super::bass_pf_1sf_delay_ref`] without the window copies and with [`filt_lp_fast`] (see
/// the module documentation).
#[allow(clippy::too_many_arguments)]
pub(super) fn bass_pf_1sf_delay(
    syn_buf: &[FixpDbl],
    syn_off: usize,
    t_sf: &[i32],
    pit_gain: &[FixpDbl],
    frame_length: i32,
    l_frame: i32,
    l_next: i32,
    synth_out: &mut [PcmDec],
    aac_out_data_headroom: i32,
    mem_bpf: &mut [FixpDbl],
) {
    const MEM: usize = (L_FILT + L_SUBFR) as usize;
    const XN: usize = (L_SUBFR + L_EXTRA) as usize;
    let mut noise_buf = [0 as FixpDbl; (L_FILT + 2 * L_SUBFR) as usize];
    let noise = L_FILT as usize;
    let noise_in = (L_FILT + L_SUBFR) as usize;
    let syn = syn_off as isize;

    // Input scaling of the BPF memory
    scale_values(&mut mem_bpf[..MEM], 1);

    // Quirk: gain_exp is set once here and only updated for subframes with lg > 0 (see the
    // straight port).
    let mut gain_exp: i32 = 17;

    let mut sf = 0usize;
    let mut i_subfr: i32 = 0;
    while i_subfr < l_frame {
        let mut t = t_sf[sf];
        let mut gain = pit_gain[sf];

        if gain > (1 << 14) {
            gain = 1 << 14;
        }
        if gain < 0 {
            gain = 0;
        }

        if gain > 0 {
            let t2 = t >> 1;
            let mut xb = [0 as FixpDbl; XN];
            let mut yb = [0 as FixpDbl; XN];
            let x = win_or_copy(syn_buf, syn + (i_subfr - L_EXTRA) as isize, &mut xb);
            let y = win_or_copy(syn_buf, syn + (i_subfr - t2 - L_EXTRA) as isize, &mut yb);

            let headroom_x = get_scalefactor(x);
            let headroom_y = get_scalefactor(y);

            let width_shift = 7;

            let mut ener: FixpDbl = 0;
            let mut corr: FixpDbl = 0;
            let mut tmp: FixpDbl = 0;
            for (&xv, &yv) in x.iter().zip(y) {
                let xs = xv << headroom_x;
                let ys = yv << headroom_y;
                ener = ener.wrapping_add(f_pow2_div2(xs) >> width_shift);
                corr = corr.wrapping_add(f_mult_div2(xs, ys) >> width_shift);
                tmp = tmp.wrapping_add(f_pow2_div2(ys) >> width_shift);
            }

            let mut exp_ener = ((17 - headroom_x) << 1) + width_shift + 1;
            let mut exp_corr = (17 - headroom_x) + (17 - headroom_y) + width_shift + 1;
            let mut exp_tmp = ((17 - headroom_y) << 1) + width_shift + 1;

            let point_zero_one: FixpDbl = 0x51eb851f; // In Q-6.37
            ener = f_add_norm(ener, exp_ener, point_zero_one, -6, &mut exp_ener);
            corr = f_add_norm(corr, exp_corr, point_zero_one, -6, &mut exp_corr);
            tmp = f_add_norm(tmp, exp_tmp, point_zero_one, -6, &mut exp_tmp);

            let mut s1 = cnt_leading_zeros(ener) - 1;
            let mut s2 = cnt_leading_zeros(tmp) - 1;

            let mut ener_by_tmp = f_mult_div2(ener << s1, tmp << s2);
            let mut ener_by_tmp_exp = (exp_ener - s1) + (exp_tmp - s2) + 1;

            if ener_by_tmp_exp & 1 != 0 {
                ener_by_tmp <<= 1;
                ener_by_tmp_exp -= 1;
            }

            let mut temp_exp = 0;

            let temp1 = inv_sqrt_norm2(ener_by_tmp, &mut temp_exp);

            let temp1_exp = temp_exp - (ener_by_tmp_exp >> 1);

            let mut tmp_result = f_mult(corr, temp1);

            let tmp_result_exp = exp_corr + temp1_exp;

            let mut diff = tmp_result_exp - 0;
            let mut point95 = fl2fxconst_dbl(0.95f32 as f64);
            if diff >= 0 {
                diff = f_min(diff, 31);
                point95 = fl2fxconst_dbl(0.95f32 as f64) >> diff;
            } else {
                diff = f_max(diff, -31);
                tmp_result >>= -diff;
            }

            if tmp_result > point95 {
                t = t2;
            }

            let lg = (l_frame + l_next - t - i_subfr).clamp(0, L_SUBFR);
            let lgu = lg as usize;

            if lg > 0 {
                let mut ab = [0 as FixpDbl; L_SUBFR_U];
                let mut bb = [0 as FixpDbl; L_SUBFR_U];
                let a = win_or_copy(syn_buf, syn + i_subfr as isize, &mut ab[..lgu]);
                let b = win_or_copy(syn_buf, syn + (i_subfr + t) as isize, &mut bb[..lgu]);

                s1 = get_scalefactor(a);
                s2 = get_scalefactor(b);
                let s = fix_min(s1, s2);

                tmp = 0;
                ener = 0;
                for (&av, &bv) in a.iter().zip(b) {
                    tmp = tmp.wrapping_add(f_pow2_div2(av << s1) >> SF_PITCH_TRACK);
                    ener = ener.wrapping_add(f_pow2_div2(bv << s2) >> SF_PITCH_TRACK);
                }
                tmp >>= f_min(DFRACT_BITS - 1, 2 * (s1 - s));
                ener >>= f_min(DFRACT_BITS - 1, 2 * (s2 - s));

                if ener <= 0 {
                    ener = 1;
                }
                if tmp <= 0 {
                    tmp = 1;
                }
                fdk_assert!(ener > 0);

                let mut result_e = 0;
                let mut tmp1 = f_div_norm(tmp, ener, &mut result_e);
                if result_e & 1 != 0 {
                    tmp1 >>= 1;
                    result_e += 1;
                }
                tmp = sqrt_fixp(tmp1);
                result_e >>= 1;

                gain_exp = 17;

                diff = result_e - gain_exp;

                let mut gain1 = gain;

                if diff >= 0 {
                    diff = f_min(diff, 31);
                    gain1 >>= diff;
                } else {
                    result_e += -diff;
                    diff = f_max(diff, -31);
                    tmp >>= -diff;
                }

                if tmp < gain1 {
                    gain = tmp;
                    gain_exp = result_e;
                }
            }

            diff = gain_exp - 17;
            if diff >= 0 {
                gain = gain.wrapping_shl(diff as u32);
            } else {
                gain = gain.wrapping_shr((-diff) as u32);
            }

            s1 = cnt_leading_zeros(gain) - 1;
            s1 -= 16; // Leading bits for SGL

            let mut gain_sgl: FixpSgl = fx_dbl2fx_sgl(gain << 16);

            gain_sgl = (gain_sgl as i32).wrapping_shl(s1 as u32) as FixpSgl;

            {
                let it = syn + i_subfr as isize;
                let mut cb = [0 as FixpDbl; L_SUBFR_U];
                let mut mb = [0 as FixpDbl; L_SUBFR_U];
                let mut pb = [0 as FixpDbl; L_SUBFR_U];
                let c = win_or_copy(syn_buf, it, &mut cb); // syn[it + i]
                let m = win_or_copy(syn_buf, it - t as isize, &mut mb); // syn[it + i - T]
                let p = win_or_copy(syn_buf, it + t as isize, &mut pb[..lgu]); // syn[it + i + T]
                let out = &mut noise_buf[noise_in..noise_in + L_SUBFR_U];
                let (out_lo, out_hi) = out.split_at_mut(lgu);
                let (c_lo, c_hi) = c.split_at(lgu);
                let (m_lo, m_hi) = m.split_at(lgu);
                for (((o, &cv), &mv), &pv) in out_lo.iter_mut().zip(c_lo).zip(m_lo).zip(p) {
                    *o = scale_value(
                        f_mult_div2(
                            gain_sgl,
                            (cv >> 1).wrapping_sub(mv >> 2).wrapping_sub(pv >> 2),
                        ),
                        2 - s1,
                    );
                }
                for ((o, &cv), &mv) in out_hi.iter_mut().zip(c_hi).zip(m_hi) {
                    *o =
                        scale_value(f_mult_div2(gain_sgl, (cv >> 1).wrapping_sub(mv >> 1)), 2 - s1);
                }
            }
        } else {
            noise_buf[noise_in..noise_in + L_SUBFR_U].fill(0);
        }

        noise_buf[..MEM].copy_from_slice(&mem_bpf[..MEM]);
        mem_bpf[..MEM].copy_from_slice(&noise_buf[L_SUBFR_U..L_SUBFR_U + MEM]);

        {
            let mut sb = [0 as FixpDbl; L_SUBFR_U];
            let s = win_or_copy(syn_buf, syn + (i_subfr - L_SUBFR) as isize, &mut sb);
            let out_start = i_subfr as usize;
            match synth_out.get_mut(out_start..out_start + L_SUBFR_U) {
                Some(out) => filt_lp_fast(
                    s,
                    out,
                    &noise_buf,
                    noise,
                    &FDK_DEC_FILT_LP,
                    aac_out_data_headroom,
                    L_SUBFR,
                    L_FILT,
                ),
                None => {
                    // An output buffer shorter than l_frame: filtered into a scratch buffer.
                    let mut o = [0 as PcmDec; L_SUBFR_U];
                    filt_lp_fast(
                        s,
                        &mut o,
                        &noise_buf,
                        noise,
                        &FDK_DEC_FILT_LP,
                        aac_out_data_headroom,
                        L_SUBFR,
                        L_FILT,
                    );
                    for (k, v) in o.iter().enumerate() {
                        if let Some(d) = synth_out.get_mut(out_start + k) {
                            *d = *v;
                        }
                    }
                }
            }
        }

        i_subfr += L_SUBFR;
        sf += 1;
    }

    {
        // Output scaling of the BPF memory
        scale_values(&mut mem_bpf[..MEM], -1);
        // Copy the rest of the signal (after the fac)
        let n = (frame_length - l_frame).max(0) as usize;
        if n > 0 {
            let start = syn + (l_frame - L_SUBFR) as isize;
            let mut copy = Vec::new();
            let src = match win(syn_buf, start, n) {
                Some(s) => s,
                None => {
                    copy.resize(n, 0);
                    window_copy(syn_buf, start, &mut copy);
                    &copy[..]
                }
            };
            let l = l_frame as usize;
            let end = (l + n).min(synth_out.len());
            if l < end {
                let m = end - l;
                scale_values_saturate_to(
                    &mut synth_out[l..end],
                    &src[..m],
                    MDCT_OUT_HEADROOM - aac_out_data_headroom,
                );
            }
        }
    }
}

/// `filtLP(syn, syn_out, noise, filt, aacOutDataHeadroom, stop, len)` with several outputs at
/// once (see the module documentation); the same arguments and results as [`filt_lp`], which it
/// calls where an access would leave the buffers.
#[allow(clippy::too_many_arguments)]
pub fn filt_lp_fast(
    syn: &[FixpDbl],
    syn_out: &mut [PcmDec],
    noise_buf: &[FixpDbl],
    noise_off: usize,
    filt: &[FixpSgl],
    aac_out_data_headroom: i32,
    stop: i32,
    len: i32,
) {
    let n = stop.max(0) as usize;
    let l = len.max(0) as usize;
    if n > L_SUBFR_U
        || l >= filt.len()
        || noise_off < l
        || noise_off + n + l > noise_buf.len()
        || syn.len() < n
        || syn_out.len() < n
    {
        return filt_lp(syn, syn_out, noise_buf, noise_off, filt, aac_out_data_headroom, stop, len);
    }
    fdk_assert!((aac_out_data_headroom - 1) >= -MDCT_OUTPUT_SCALE);
    // nb[l + i] = noise[i] for -l <= i < n + l
    let nb = &noise_buf[noise_off - l..noise_off + n + l];
    let filt = &filt[..=l];
    let mut tmp = [0 as FixpDbl; L_SUBFR_U];
    let done = filt_lp_tmp_simd(nb, filt, n, &mut tmp);
    for i in done..n {
        let mut t = f_mult_div2(nb[l + i], filt[0]); // Filt in Q-1.16
        for j in 1..=l {
            t = t.wrapping_add(f_mult(
                (nb[l + i - j] >> 1).wrapping_add(nb[l + i + j] >> 1),
                filt[j],
            ));
        }
        tmp[i] = t;
    }
    for ((o, &s), &t) in syn_out[..n].iter_mut().zip(&syn[..n]).zip(&tmp[..n]) {
        *o = imdct_scale((s >> 1).wrapping_sub(t >> 1), aac_out_data_headroom - 1) as PcmDec;
    }
}

/// The sums `tmp[i]` of `filtLP()` for the first outputs in SIMD groups; returns their number
/// (0 without SIMD). `nb[l + i] = noise[i]` (`nb.len() >= n + 2 l`, `l = filt.len() - 1`).
#[inline(always)]
fn filt_lp_tmp_simd(
    nb: &[FixpDbl],
    filt: &[FixpSgl],
    n: usize,
    tmp: &mut [FixpDbl; L_SUBFR_U],
) -> usize {
    debug_assert!(n <= L_SUBFR_U && !filt.is_empty() && nb.len() >= n + 2 * (filt.len() - 1));
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is part of AArch64; nb holds n + 2 l values.
        unsafe { neon::filt_lp_tmp(nb, filt, n, tmp) }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if fdk_aac_rs_fdk::simd::avx2() {
            // SAFETY: the CPU has AVX2; nb holds n + 2 l values.
            unsafe { x86::filt_lp_tmp(nb, filt, n, tmp) }
        } else {
            0
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    {
        let _ = (nb, filt, n, tmp);
        0
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use super::*;
    use core::arch::aarch64::*;

    /// The sums `tmp[i]` of `filtLP()` for the groups of 4 outputs below `n`; returns their
    /// number. `nb[l + i] = noise[i]`, `l = filt.len() - 1`.
    ///
    /// # Safety
    /// `nb.len() >= n + 2 l`, `n <= 64`.
    pub(super) unsafe fn filt_lp_tmp(
        nb: &[FixpDbl],
        filt: &[FixpSgl],
        n: usize,
        tmp: &mut [FixpDbl; L_SUBFR_U],
    ) -> usize {
        let l = filt.len() - 1;
        let n4 = n & !3;
        let p = nb.as_ptr();
        let f0 = vdupq_n_s32((filt[0] as i32) << 15);
        let mut i0 = 0;
        while i0 < n4 {
            let c = p.add(l + i0);
            let mut t = vqdmulhq_s32(vld1q_s32(c), f0);
            for (j, &f) in filt.iter().enumerate().skip(1) {
                let v = vaddq_s32(
                    vshrq_n_s32::<1>(vld1q_s32(c.sub(j))),
                    vshrq_n_s32::<1>(vld1q_s32(c.add(j))),
                );
                let pr = vqdmulhq_s32(v, vdupq_n_s32((f as i32) << 15));
                t = vaddq_s32(t, vshlq_n_s32::<1>(pr));
            }
            vst1q_s32(tmp.as_mut_ptr().add(i0), t);
            i0 += 4;
        }
        n4
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86 {
    use super::*;
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    /// `fMultDiv2(a, b)` for 16-bit `b` (see the module documentation).
    #[inline(always)]
    unsafe fn md2v(a: __m256i, b: __m256i) -> __m256i {
        let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(a), b);
        let lo = _mm256_and_si256(a, _mm256_set1_epi32(0xFFFF));
        _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, b)))
    }

    /// The AVX2 form of [`super::neon::filt_lp_tmp`] (groups of 8 outputs).
    ///
    /// # Safety
    /// AVX2; `nb.len() >= n + 2 l`, `n <= 64`.
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn filt_lp_tmp(
        nb: &[FixpDbl],
        filt: &[FixpSgl],
        n: usize,
        tmp: &mut [FixpDbl; L_SUBFR_U],
    ) -> usize {
        let l = filt.len() - 1;
        let n8 = n & !7;
        let p = nb.as_ptr();
        let f0 = _mm256_set1_epi32(filt[0] as i32);
        let mut i0 = 0;
        while i0 < n8 {
            let c = p.add(l + i0);
            let mut t = md2v(_mm256_loadu_si256(c as *const __m256i), f0);
            for (j, &f) in filt.iter().enumerate().skip(1) {
                let v = _mm256_add_epi32(
                    _mm256_srai_epi32::<1>(_mm256_loadu_si256(c.sub(j) as *const __m256i)),
                    _mm256_srai_epi32::<1>(_mm256_loadu_si256(c.add(j) as *const __m256i)),
                );
                let pr = md2v(v, _mm256_set1_epi32(f as i32));
                t = _mm256_add_epi32(t, _mm256_slli_epi32::<1>(pr));
            }
            _mm256_storeu_si256(tmp.as_mut_ptr().add(i0) as *mut __m256i, t);
            i0 += 8;
        }
        n8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u32) as i32
        }
        fn val(&mut self, mode: u32) -> i32 {
            match mode {
                0 => [i32::MIN, i32::MIN + 1, i32::MAX, i32::MAX - 1, 0, -1, 1 << 30, -(1 << 30)]
                    [(self.next() % 8) as usize],
                1 => self.next() as i32,
                n => (self.next() as i32) >> (n * 3),
            }
        }
        fn vec(&mut self, n: usize) -> Vec<i32> {
            let mode = self.next() % 7;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                let m = if mode == 6 { self.next() % 6 } else { mode };
                v.push(self.val(m));
            }
            v
        }
    }

    /// The vector forms of filtLP() against the straight port: every output count, filter
    /// length and noise offset, random and extreme values, random filters.
    #[test]
    fn filt_lp_matches_straight() {
        let mut r = Rng(0x0bad_cafe_1234_5678);
        for it in 0..40_000 {
            let stop = if it % 3 == 0 { 64 } else { r.range(-2, 70) };
            let len = if it % 2 == 0 { 12 } else { r.range(-1, 14) };
            let noise = r.vec(140);
            let noise_off = if it % 4 == 0 { 12 } else { r.range(0, 140) as usize };
            let syn_len = if it % 8 == 0 { r.range(0, 64) as usize } else { 64 };
            let syn = r.vec(syn_len);
            let filt: Vec<i16> = if it % 2 == 0 {
                crate::usacdec_rom::FDK_DEC_FILT_LP.to_vec()
            } else {
                (0..r.range(0, 16)).map(|_| r.next() as i16).collect()
            };
            let headroom = r.range(-1, 12);
            let olen = if it % 8 == 1 { r.range(0, 64) as usize } else { 64 };
            // the straight port panics where the reference has no defined result; the fast form
            // must then panic as well
            let fast = std::panic::catch_unwind(|| {
                let mut o = vec![0; olen];
                filt_lp_fast(&syn, &mut o, &noise, noise_off, &filt, headroom, stop, len);
                o
            });
            let straight = std::panic::catch_unwind(|| {
                let mut o = vec![0; olen];
                filt_lp(&syn, &mut o, &noise, noise_off, &filt, headroom, stop, len);
                o
            });
            match (fast, straight) {
                (Ok(x), Ok(y)) => assert_eq!(x, y, "it {it} stop {stop} len {len} off {noise_off}"),
                (Err(_), Err(_)) => {}
                (x, y) => panic!("it {it}: fast {:?} straight {:?}", x.is_ok(), y.is_ok()),
            }
        }
    }

    /// The bass post-filter against the straight port with the parameters of the decoder
    /// (CLpd_RenderTimeSignal(), CBlock_FrequencyToTime()), random pitch lags and gains, and
    /// short buffers and offsets where windows leave the synthesis buffer.
    #[test]
    fn bass_pf_matches_straight() {
        let mut r = Rng(0x1357_9bdf_2468_ace0);
        for it in 0..6_000 {
            let headroom = [3, 5, 8, 11][(r.next() % 4) as usize];
            let l_frame = if it % 2 == 0 { 1024 } else { 768 };
            let (frame_length, lf, l_next) = match it % 3 {
                0 => (l_frame, l_frame, (l_frame / 128 - 1) * 64),
                1 => (l_frame, l_frame, (l_frame / 128 - 1) * 64 - l_frame / 8),
                _ => {
                    let lpd_sfd = l_frame / 128;
                    (l_frame, (lpd_sfd + 2) * 64 + 64, l_frame - (lpd_sfd + 4) * 64)
                }
            };
            let short = it % 10 == 7;
            let len = if short { r.range(200, 1883) as usize } else { 1883 };
            let syn_off = if short { r.range(0, 600) as usize } else { 411 };
            let syn = r.vec(len);
            let t_sf: Vec<i32> = (0..23)
                .map(|_| match r.next() % 8 {
                    0 => 64,
                    1 => r.range(-40, 600),
                    _ => r.range(17, 411),
                })
                .collect();
            let gains: Vec<i32> = (0..23)
                .map(|_| match r.next() % 4 {
                    0 => 0,
                    1 => r.next() as i32,
                    _ => r.range(0, 1 << 15),
                })
                .collect();
            let mem = r.vec(76);
            let olen = if it % 16 == 5 { r.range(0, 1024) as usize } else { 1024 };
            let (mut ma, mut mb) = (mem.clone(), mem);
            let (mut oa, mut ob) = (vec![0; olen], vec![0; olen]);
            bass_pf_1sf_delay(
                &syn,
                syn_off,
                &t_sf,
                &gains,
                frame_length,
                lf,
                l_next,
                &mut oa,
                headroom,
                &mut ma,
            );
            bass_pf_1sf_delay_ref(
                &syn,
                syn_off,
                &t_sf,
                &gains,
                frame_length,
                lf,
                l_next,
                &mut ob,
                headroom,
                &mut mb,
            );
            assert_eq!((oa, ma), (ob, mb), "it {it}");
        }
    }
}
