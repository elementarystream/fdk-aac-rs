// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/usacdec_lpd.h, libAACdec/src/usacdec_lpd.cpp.

//! Port of `usacdec_lpd.h`: USAC linear prediction domain (LPD) coding: `lpd_channel_stream()`
//! parsing, TCX decoding (noise filling, FDNS, gains), the rendering of the time signal (ACELP,
//! TCX, FAC transitions, concealment) and the bass post-filter.
//!
//! The preprocessed `usacdec_lpd.cpp` of the `aarch64` and `x86` builds differ only in
//! `FDK_ASSERT` and in the form of the `SATURATE_RIGHT_SHIFT` macro of `filtLP()` (both give the
//! same values); the variants differ in the primitives.
//!
//! # Memory
//!
//! The functions follow the rules of [`crate::channelinfo`]: the spectrum
//! (`pSpectralCoefficient`, `spec`) and the work buffer (`pComStaticData->pWorkBufferCore1`)
//! are additional parameters. The FAC data pointers `data.usac.fac_data[k]` are resolved with
//! [`fac_data_slice`]; [`FacDataPtr::Scratch`] (a pointer to the `fac_buf` of a previous call in
//! C) is only compared with `NULL` by the decoder, as in C (`CLpdChannelStream_Read()` resets
//! the pointers of every frame index it visits before `CLpdChannelStream_Decode()` and
//! `CLpd_RenderTimeSignal()` use them); otherwise the port reads its zero buffer.
//!
//! `AACDEC_FLUSH` (`aacdecoder_lib.h`) is a private constant here.

use fdk_aac_rs_fdk::bitstream::{fdk_read_bit, fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::{
    cnt_leading_zeros, f_add_norm, f_div_norm, f_max, f_min, f_mult, f_mult_div2, f_norm, f_normz,
    f_pow2, f_pow2_div2, fix_max, fix_min, fl2fx_dbl, fl2fxconst_dbl, fx_dbl2fx_sgl,
    inv_sqrt_norm2, sqrt_fixp, FixpDbl, FixpSgl, DFRACT_BITS, MAXVAL_DBL,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fft::fft;
use fdk_aac_rs_fdk::lpc::FixpLpc;
use fdk_aac_rs_fdk::mdct::{
    imdct_copy_ov_and_nr, imdct_drain, imdct_scale, imlt_block, MDCT_OUTPUT_SCALE,
    MDCT_OUT_HEADROOM, MLT_FLAG_CURR_ALIAS_SYMMETRY,
};
use fdk_aac_rs_fdk::scale::{
    get_scalefactor, scale_value, scale_value_saturate, scale_values, scale_values_saturate_to,
};
use fdk_aac_rs_fdk::tools_rom::{fdk_get_window_slope, SINE_TABLE_384, SINE_TABLE_512};

use fdk_aac_rs_arith::ac_arith_coder::{carco_decode_arith_data, ARITH_CODER_ERROR};
use fdk_aac_rs_fdk::audio::AC_INDEP;

use crate::aac_rom::PcmDec;
use crate::block::get_gain;
use crate::channelinfo::{
    get_window_shape, AacDecoderError, CAacDecoderChannelInfo, CAacDecoderStaticChannelInfo,
    CWorkBufferCore1, FacDataPtr, SamplingRateInfo, AAC_DEC_OK, AAC_DEC_PARSE_ERROR,
    AAC_DEC_UNKNOWN, FD_LONG, FD_SHORT, LPD,
};
use crate::conceal::cconcealment_get_last_frame_ok;
use crate::overlapadd::spec_tcx;
use crate::usacdec_acelp::{
    acelp_post_processing, acelp_pre_processing, clpd_acelp_decode,
    clpd_acelp_prepare_internal_mem, clpd_acelp_read, clpd_acelp_reset, clpd_tcx_td_conceal,
};
use crate::usacdec_const::{
    PIT_MIN_12k8, BPF_DELAY, BPF_SFD, FAC_FSCALE_MAX, FAC_FSCALE_MIN, FDNS_NPTS, FSCALE_DENOM,
    LFAC, L_DIV, L_EXTRA, L_FILT, L_SUBFR, M_LP_FILTER_ORDER, NB_DIV, NB_SUBFR_SUPERFR,
    PIT_MAX_MAX, SF_EXC, SYN_SFD,
};
use crate::usacdec_fac::{
    cfac_apply_gains, clpd_fac_acelp2mdct, clpd_fac_get_memory, clpd_fac_mdct2acelp, clpd_fac_read,
    fac_data_slice,
};
use crate::usacdec_lpc::{clpc_conceal, clpc_read, clpd_decode_gain, e_lpc_f_lsp_a_conversion};
use crate::usacdec_rom::{FDK_DEC_FILT_LP, FDK_DEC_LSF_INIT};

mod fast;

/// `OPTIMIZE_AVG_PERFORMANCE`
pub const OPTIMIZE_AVG_PERFORMANCE: bool = true;

/// `AACDEC_FLUSH` of `aacdecoder_lib.h` (the same value as the constant of
/// `crate::aacdecoder_lib`).
const AACDEC_FLUSH: u32 = 2;

const M: usize = M_LP_FILTER_ORDER as usize;
const SF_PITCH_TRACK: i32 = 6;
const L_SUBFR_U: usize = L_SUBFR as usize;

/// `CLpd_FAC_getLength(fNotShortBlock, fac_length_long)`
#[inline]
pub fn clpd_fac_get_length(f_not_short_block: i32, fac_length_long: i32) -> i32 {
    if f_not_short_block != 0 {
        fac_length_long
    } else {
        fac_length_long / 2
    }
}

/// `UsacRandomSign(seed)`: random sign generator for FD and TCX noise filling. If the return
/// value is zero use the positive sign. (This code is also implemented as a copy in
/// `block.cpp`.)
#[inline]
pub fn usac_random_sign(seed: &mut u32) -> i32 {
    *seed = (*seed as u64).wrapping_mul(69069).wrapping_add(5) as u32;

    (*seed & 0x10000) as i32
}

/// A value of a buffer at a signed index; 0 outside of the buffer.
#[inline(always)]
fn rd(buf: &[FixpDbl], idx: isize) -> FixpDbl {
    if idx >= 0 {
        buf.get(idx as usize).copied().unwrap_or(0)
    } else {
        0
    }
}

/// `n` values of `buf` from the signed index `start`; 0 outside of the buffer.
fn window_copy(buf: &[FixpDbl], start: isize, out: &mut [FixpDbl]) {
    for (i, o) in out.iter_mut().enumerate() {
        *o = rd(buf, start + i as isize);
    }
}

/// `filtLP(syn, syn_out, noise, filt, aacOutDataHeadroom, stop, len)`: subtract the low-pass
/// filtered noise (`noise[-len..stop+len]`, `noise_buf[noise_off..]`) from the synthesis
/// signal.
pub fn filt_lp(
    syn: &[FixpDbl],
    syn_out: &mut [PcmDec],
    noise_buf: &[FixpDbl],
    noise_off: usize,
    filt: &[FixpSgl],
    aac_out_data_headroom: i32,
    stop: i32,
    len: i32,
) {
    fdk_assert!((aac_out_data_headroom - 1) >= -MDCT_OUTPUT_SCALE);

    for i in 0..stop.max(0) as usize {
        let n = noise_off as isize + i as isize;
        let mut tmp = f_mult_div2(rd(noise_buf, n), filt[0]); // Filt in Q-1.16
        for j in 1..=len.max(0) as usize {
            tmp = tmp.wrapping_add(f_mult(
                (rd(noise_buf, n - j as isize) >> 1)
                    .wrapping_add(rd(noise_buf, n + j as isize) >> 1),
                filt[j],
            ));
        }
        syn_out[i] =
            imdct_scale((syn[i] >> 1).wrapping_sub(tmp >> 1), aac_out_data_headroom - 1) as PcmDec;
    }
}

/// `bass_pf_1sf_delay(syn, T_sf, pit_gain, frame_length, l_frame, l_next, synth_out,
/// aacOutDataHeadroom, mem_bpf)`: perform a low-frequency pitch enhancement on the time domain
/// signal.
///
/// `syn` (`syn_buf[syn_off..]`): 12.8kHz synthesis to postfilter (the past samples before it are
/// read); `t_sf`: pitch period for all subframes; `pit_gain`: pitch gains; `frame_length`: frame
/// length (should be 768|1024); `l_frame`: length of filtering, must be a multiple of
/// `L_SUBFR`; `l_next`: length of allowed look ahead on `syn[i]`, `i < l_frame+l_next`;
/// `synth_out`: filtered synthesis (with delay of 1 subfr); `mem_bpf`: filter memory
/// (`L_FILT+L_SUBFR`).
///
/// The optimized form `fast::bass_pf_1sf_delay` gives the same results as the straight port
/// [`bass_pf_1sf_delay_ref`].
#[allow(clippy::too_many_arguments)]
pub fn bass_pf_1sf_delay(
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
    fast::bass_pf_1sf_delay(
        syn_buf,
        syn_off,
        t_sf,
        pit_gain,
        frame_length,
        l_frame,
        l_next,
        synth_out,
        aac_out_data_headroom,
        mem_bpf,
    )
}

/// The straight port of `bass_pf_1sf_delay()` (see [`bass_pf_1sf_delay`]).
#[allow(clippy::too_many_arguments)]
pub fn bass_pf_1sf_delay_ref(
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
    // L_FILT = 12, L_SUBFR = 64 => 140
    let mut noise_buf = [0 as FixpDbl; (L_FILT + 2 * L_SUBFR) as usize];
    let noise = L_FILT as usize; // L_FILT = 12 delay of upsampling filter
    let noise_in = (L_FILT + L_SUBFR) as usize;
    let syn = syn_off as isize;

    // Input scaling of the BPF memory
    scale_values(&mut mem_bpf[..MEM], 1);

    // Quirk: gain_exp is set once here and only updated for subframes with lg > 0; a subframe
    // with lg == 0 uses the value of the previous subframe (which is always 17, see the tests).
    let mut gain_exp: i32 = 17;

    let mut sf = 0usize;
    let mut i_subfr: i32 = 0;
    while i_subfr < l_frame {
        let mut t = t_sf[sf];
        let mut gain = pit_gain[sf];

        // Gain is in Q17.14
        // If gain > 1 set to 1
        if gain > (1 << 14) {
            gain = 1 << 14;
        }

        // If gain < 0 set to 0
        if gain < 0 {
            gain = 0;
        }

        if gain > 0 {
            // pitch tracker: test pitch/2 to avoid continuous pitch doubling
            // Note: pitch is limited to PIT_MIN (34 = 376Hz) at the encoder
            let t2 = t >> 1;
            let mut x = [0 as FixpDbl; (L_SUBFR + L_EXTRA) as usize];
            let mut y = [0 as FixpDbl; (L_SUBFR + L_EXTRA) as usize];
            window_copy(syn_buf, syn + (i_subfr - L_EXTRA) as isize, &mut x);
            window_copy(syn_buf, syn + (i_subfr - t2 - L_EXTRA) as isize, &mut y);

            let mut ener: FixpDbl = 0;
            let mut corr: FixpDbl = 0;
            let mut tmp: FixpDbl = 0;

            let headroom_x = get_scalefactor(&x);
            let headroom_y = get_scalefactor(&y);

            let width_shift = 7;

            for i in 0..(L_SUBFR + L_EXTRA) as usize {
                ener = ener.wrapping_add(f_pow2_div2(x[i] << headroom_x) >> width_shift);
                corr = corr.wrapping_add(
                    f_mult_div2(x[i] << headroom_x, y[i] << headroom_y) >> width_shift,
                );
                tmp = tmp.wrapping_add(f_pow2_div2(y[i] << headroom_y) >> width_shift);
            }

            let mut exp_ener = ((17 - headroom_x) << 1) + width_shift + 1;
            let mut exp_corr = (17 - headroom_x) + (17 - headroom_y) + width_shift + 1;
            let mut exp_tmp = ((17 - headroom_y) << 1) + width_shift + 1;

            // Add 0.01 to "ener". Adjust exponents
            let point_zero_one: FixpDbl = 0x51eb851f; // In Q-6.37
            ener = f_add_norm(ener, exp_ener, point_zero_one, -6, &mut exp_ener);
            corr = f_add_norm(corr, exp_corr, point_zero_one, -6, &mut exp_corr);
            tmp = f_add_norm(tmp, exp_tmp, point_zero_one, -6, &mut exp_tmp);

            // use T2 if normalized correlation > 0.95
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

            // prevent that noise calculation below reaches into not defined signal parts at the
            // end of the synth_buf or in other words restrict the below used index
            // (i+i_subfr+T) < l_frame + l_next
            let mut lg = l_frame + l_next - t - i_subfr;

            if lg > L_SUBFR {
                lg = L_SUBFR;
            } else if lg < 0 {
                lg = 0;
            }

            // limit gain to avoid problem on burst
            if lg > 0 {
                // max(lg) = 64 => scale with 6 bits minus 1 (fPow2Div2)
                let lgu = lg as usize;
                let mut a = [0 as FixpDbl; L_SUBFR_U];
                let mut b = [0 as FixpDbl; L_SUBFR_U];
                window_copy(syn_buf, syn + i_subfr as isize, &mut a[..lgu]);
                window_copy(syn_buf, syn + (i_subfr + t) as isize, &mut b[..lgu]);

                s1 = get_scalefactor(&a[..lgu]);
                s2 = get_scalefactor(&b[..lgu]);
                let s = fix_min(s1, s2);

                tmp = 0;
                ener = 0;
                for i in 0..lgu {
                    tmp = tmp.wrapping_add(f_pow2_div2(a[i] << s1) >> SF_PITCH_TRACK);
                    ener = ener.wrapping_add(f_pow2_div2(b[i] << s2) >> SF_PITCH_TRACK);
                }
                tmp >>= f_min(DFRACT_BITS - 1, 2 * (s1 - s));
                ener >>= f_min(DFRACT_BITS - 1, 2 * (s2 - s));

                // error robustness: for the specific case syn[...] == -1.0f for all 64 samples
                // ener or tmp might overflow and become negative. For all sane cases we have
                // enough headroom.
                if ener <= 0 {
                    ener = 1;
                }
                if tmp <= 0 {
                    tmp = 1;
                }
                fdk_assert!(ener > 0);

                // tmp = sqrt(tmp/ener)
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

            // calculate noise based on voiced pitch
            // fMultDiv2() replaces weighting of gain with 0.5
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
                for i in 0..lg.max(0) as isize {
                    // scaled with SF_SYNTH + gain_sf + 1; composition of scalefactor 2: one
                    // additional shift of syn values + fMult => fMultDiv2
                    noise_buf[noise_in + i as usize] = scale_value(
                        f_mult_div2(
                            gain_sgl,
                            (rd(syn_buf, it + i) >> 1)
                                .wrapping_sub(rd(syn_buf, it + i - t as isize) >> 2)
                                .wrapping_sub(rd(syn_buf, it + i + t as isize) >> 2),
                        ),
                        2 - s1,
                    );
                }

                for i in lg.max(0) as isize..L_SUBFR as isize {
                    // scaled with SF_SYNTH + gain_sf + 1; composition of scalefactor 2: one
                    // additional shift of syn values + fMult => fMultDiv2
                    noise_buf[noise_in + i as usize] = scale_value(
                        f_mult_div2(
                            gain_sgl,
                            (rd(syn_buf, it + i) >> 1)
                                .wrapping_sub(rd(syn_buf, it + i - t as isize) >> 1),
                        ),
                        2 - s1,
                    );
                }
            }
        } else {
            noise_buf[noise_in..noise_in + L_SUBFR_U].fill(0);
        }

        {
            noise_buf[..MEM].copy_from_slice(&mem_bpf[..MEM]);

            mem_bpf[..MEM].copy_from_slice(&noise_buf[L_SUBFR_U..L_SUBFR_U + MEM]);
        }

        // substract from voiced speech low-pass filtered noise
        // filter coefficients are scaled with factor SF_FILT_LP (1)
        {
            let mut s = [0 as FixpDbl; L_SUBFR_U];
            window_copy(syn_buf, syn + (i_subfr - L_SUBFR) as isize, &mut s);
            let out_start = i_subfr as usize;
            match synth_out.get_mut(out_start..out_start + L_SUBFR_U) {
                Some(out) => filt_lp(
                    &s,
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
                    filt_lp(
                        &s,
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

    // To be determined (info from Ben)
    {
        // Output scaling of the BPF memory
        scale_values(&mut mem_bpf[..MEM], -1);
        // Copy the rest of the signal (after the fac)
        let n = (frame_length - l_frame).max(0) as usize;
        if n > 0 {
            let mut src = vec![0 as FixpDbl; n];
            window_copy(syn_buf, syn + (l_frame - L_SUBFR) as isize, &mut src);
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

/*
 * Frequency Domain Noise Shaping
 */

/// `ALFDPOW2_SCALE`
const ALFDPOW2_SCALE: i32 = 3;

/// `CLpd_AdaptLowFreqDeemph(x, lg, alfd_gains, s)`: adaptive low frequencies deemphasis of the
/// spectral coefficients `x` (`lg` values, 1 bit headroom required, scale shift factor `s`).
///
/// Ensure quantization of low frequencies in case where the signal dynamic is higher than the
/// LPC noise shaping. This is the inverse operation of `adap_low_freq_emph()`. Outputs the gain
/// of every block (`alfd_gains`).
pub fn clpd_adapt_low_freq_deemph(x: &mut [FixpDbl], lg: i32, alfd_gains: &mut [FixpDbl], s: i32) {
    // Note: This stack array saves temporary accumulation results to be used in a second run.
    // The size should be limited to (1024/4)/8=32
    let mut tmp_pow2 = [0 as FixpDbl; 32];

    let mut s = s.wrapping_mul(2).wrapping_add(ALFDPOW2_SCALE);
    s = f_min(31, s);

    let k = 8;
    // lg is at most 1024 (ALFD range = 1600Hz (lg = 6400Hz))
    let i_max = (lg / 4).min(256);

    // find spectral peak
    // s is negative only for unusual spectrum scales (not produced by the TCX reader); the
    // shift then takes the amount modulo 32 like the reference builds.
    let floor = fl2fx_dbl(0.01f32).wrapping_shr(s as u32);
    let mut max = floor;
    let mut i = 0;
    while i < i_max {
        let mut tmp: FixpDbl = 0;
        let mut px = i as usize;

        let mut j = 8;
        loop {
            let mut x0 = x[px];
            px += 1;
            let mut x1 = x[px];
            px += 1;
            x0 = f_pow2_div2(x0);
            x1 = f_pow2_div2(x1);
            tmp = tmp.wrapping_add(x0 >> (ALFDPOW2_SCALE - 1));
            tmp = tmp.wrapping_add(x1 >> (ALFDPOW2_SCALE - 1));
            j -= 2;
            if j == 0 {
                break;
            }
        }
        tmp = f_max(tmp, floor);
        tmp_pow2[(i >> 3) as usize] = tmp;
        if tmp > max {
            max = tmp;
        }
        i += k;
    }

    // deemphasis of all blocks below the peak
    let mut fac = fl2fx_dbl(0.1f32) >> 1;
    let mut g = 0usize;
    i = 0;
    while i < i_max {
        let mut tmp = tmp_pow2[(i >> 3) as usize];
        let mut shifti = 0;

        // tmp = (float)sqrt(tmp/max);

        // value of tmp is between 8/2*max^2 and max^2 / 2.
        // required shift factor of division can grow up to 27 (grows exponentially for values
        // toward zero) thus using normalized division to assure valid result.
        {
            let mut sd = 0;

            if tmp != 0 {
                tmp = f_div_norm(max, tmp, &mut sd);
                if sd & 1 != 0 {
                    sd += 1;
                    tmp >>= 1;
                }
            } else {
                tmp = MAXVAL_DBL;
                sd = 0;
            }
            tmp = inv_sqrt_norm2(tmp, &mut shifti);
            tmp = scale_value(tmp, shifti - 1 - (sd / 2));
        }
        if tmp > fac {
            fac = tmp;
        }
        let mut px = i as usize;

        let mut j = 8;
        loop {
            let mut x0 = x[px];
            let mut x1 = x[px + 1];
            x0 = f_mult_div2(x0, fac);
            x1 = f_mult_div2(x1, fac);
            x0 <<= 2;
            x1 <<= 2;
            x[px] = x0;
            px += 1;
            x[px] = x1;
            px += 1;

            j -= 2;
            if j == 0 {
                break;
            }
        }
        // Store gains for FAC
        alfd_gains[g] = fac;
        g += 1;
        i += k;
    }
}

/// `NSHAPE_SCALE`
const NSHAPE_SCALE: i32 = 4;

/// `lpc2mdctAndNoiseShaping(r, pScale, lg, fdns_npts, A1, A1_exp, A2, A2_exp)`: interpolated
/// noise shaping for MDCT coefficients.
///
/// This algorithm shapes temporally the spectral noise between the two spectral noise
/// representations (`fdns_npts` of resolution). The noise is shaped monotonically between the
/// two points using a curved shape to favor the lower gain in mid-frame. ODFT and amplitude
/// calculation are applied to the 2 LPC coefficients first. `A1`, `A2`: old and new input LPC
/// coefficients (exponents `A1_exp`, `A2_exp`).
#[allow(clippy::too_many_arguments)]
pub fn lpc2mdct_and_noise_shaping(
    r: &mut [FixpDbl],
    p_scale: &mut i16,
    lg: i32,
    fdns_npts: i32,
    a1: &[FixpLpc],
    a1_exp: i32,
    a2: &[FixpLpc],
    a2_exp: i32,
) {
    let mut tmp1_buf = [0 as FixpDbl; (FDNS_NPTS * 8) as usize];
    let s;

    {
        let (tmp1, tmp2) = tmp1_buf.split_at_mut((fdns_npts.clamp(0, FDNS_NPTS) * 4) as usize);
        let npts = fdns_npts as usize;

        // lpc2mdct()

        // ODFT. E_LPC_a_weight() for A1 and A2 vectors is included into the loop below.
        let mut f = fl2fxconst_dbl(0.92f32 as f64);

        // needed values: sin(phi), cos(phi); phi = i*PI/(2*fdns_npts), i = 0 ...
        // M_LP_FILTER_ORDER
        let (sin_tab, k_step): (&[_], usize) = match fdns_npts {
            64 => (&SINE_TABLE_512[..], 512 / 64),
            48 => (&SINE_TABLE_384[..], 384 / 48),
            _ => {
                fdk_assert!(false);
                return;
            }
        };

        let mut k = k_step;
        for i in 0..M {
            let cs = sin_tab[k];

            let wa1 = f_mult(a1[i], f);
            let wa2 = f_mult(a2[i], f);

            // r[i] = A[i]*cos()
            tmp1[2 + i * 2] = f_mult(wa1, cs.re);
            tmp2[2 + i * 2] = f_mult(wa2, cs.re);
            // i[i] = A[i]*sin()
            tmp1[3 + i * 2] = f_mult(wa1, cs.im).wrapping_neg();
            tmp2[3 + i * 2] = f_mult(wa2, cs.im).wrapping_neg();

            f = f_mult(f, fl2fxconst_dbl(0.92f32 as f64));
            k += k_step;
        }

        // Guarantee at least 2 bits of headroom for the FFT
        // "3" stands for 1.0 with 2 bits of headroom; (A1_exp + 2) guarantess 2 bits of headroom
        // if A1_exp > 1
        //
        // `fMax(3, A_exp + 2)` and `(ULONG)0x80000000 >> A_exp_fix` as all three reference builds
        // compile them (clang assumes that neither the addition overflows nor the shift amount
        // reaches 32): `fMax(1, A_exp) + 2` and `0x20000000 >> fMax(1, A_exp)`, with the shift
        // amount taken modulo 32 by the instruction (`lsr`, `shr`). The two forms differ only for
        // `A_exp % 32` in 30..=31 (1.0 becomes 0 instead of `0x80000000 >> 0` or `>> 1`) and for
        // `A_exp` near `INT_MAX`; `E_LPC_f_lsp_a_conversion()` gives small exponents.
        let a1_exp_fix = f_max(1, a1_exp).wrapping_add(2);
        let a2_exp_fix = f_max(1, a2_exp).wrapping_add(2);

        // Set 1.0 in the proper format
        tmp1[0] = 0x20000000u32.wrapping_shr(f_max(1, a1_exp) as u32) as i32;
        tmp2[0] = 0x20000000u32.wrapping_shr(f_max(1, a2_exp) as u32) as i32;

        tmp1[1] = 0;
        tmp2[1] = 0;

        // Clear the resto of the array
        let c0 = 2 * (M + 1);
        let c1 = c0 + 2 * (npts * 2 - (M + 1));
        tmp1[c0..c1].fill(0);
        tmp2[c0..c1].fill(0);

        // Guarantee 2 bits of headroom for FFT
        scale_values(&mut tmp1[2..2 + 2 * M], a1_exp.wrapping_sub(a1_exp_fix));
        scale_values(&mut tmp2[2..2 + 2 * M], a2_exp.wrapping_sub(a2_exp_fix));

        let mut s1 = a1_exp_fix;
        let mut s2 = a2_exp_fix;

        fft(2 * fdns_npts, tmp1, &mut s1);
        fft(2 * fdns_npts, tmp2, &mut s2);

        // Adjust the exponents of both fft outputs if necessary
        if s1 > s2 {
            scale_values(&mut tmp2[..2 * npts], s2.wrapping_sub(s1));
            s2 = s1;
        } else if s1 < s2 {
            scale_values(&mut tmp1[..2 * npts], s1.wrapping_sub(s2));
            s1 = s2;
        }

        fdk_assert!(s1 == s2);
        s = s1;
    }

    let (tmp1, tmp2) = tmp1_buf.split_at((fdns_npts * 4) as usize);

    // Get amplitude and apply gains
    let step = lg / fdns_npts;
    let mut rr_minus_one: FixpDbl = 0;

    for k in 0..fdns_npts as usize {
        let mut g1;
        let mut g2;
        let inv_g1_g2;
        let mut inv_g1_g2_e = 0;
        let g_e;
        let shift;

        {
            let mut si1 = 0;
            let mut si2 = 0;

            let mut real = tmp1[k * 2];
            let mut imag = tmp1[k * 2 + 1];
            let mut s_input = f_max(f_min(f_norm(real), f_norm(imag)) - 1, 0);
            real <<= s_input;
            imag <<= s_input;
            // g1_e = si1 - 2*s/2
            g1 = inv_sqrt_norm2(f_pow2(real).wrapping_add(f_pow2(imag)), &mut si1);
            si1 += s_input;

            real = tmp2[k * 2];
            imag = tmp2[k * 2 + 1];
            s_input = f_max(f_min(f_norm(real), f_norm(imag)) - 1, 0);
            real <<= s_input;
            imag <<= s_input;
            // g2_e = si2 - 2*s/2
            g2 = inv_sqrt_norm2(f_pow2(real).wrapping_add(f_pow2(imag)), &mut si2);
            si2 += s_input;

            // Pick a common scale factor for g1 and g2
            if si1 > si2 {
                g2 = g2.wrapping_shr((si1 - si2) as u32);
                g_e = si1 - s;
            } else {
                g1 = g1.wrapping_shr((si2 - si1) as u32);
                g_e = si2 - s;
            }
        }

        // end of lpc2mdct()

        fdk_assert!(g1 >= 0);
        fdk_assert!(g2 >= 0);

        // mdct_IntNoiseShaping()
        {
            let a;
            let b;
            // inv_g1_g2 * 2^inv_g1_g2_e = 1/(g1+g2)
            let sum = (g1 >> 1).wrapping_add(g2 >> 1);
            if sum != 0 {
                inv_g1_g2 = f_div_norm(fl2fxconst_dbl(0.5), sum, &mut inv_g1_g2_e);
                inv_g1_g2_e -= g_e;
            } else {
                inv_g1_g2 = MAXVAL_DBL;
                inv_g1_g2_e = 0;
            }

            if g_e < 0 {
                // a_e = g_e + inv_g1_g2_e + 1
                a = scale_value(f_mult(f_mult(g1, g2), inv_g1_g2), g_e);
                // b_e = g_e + inv_g1_g2_e
                b = f_mult(g2.wrapping_sub(g1), inv_g1_g2);
                shift = g_e + inv_g1_g2_e + 1 - NSHAPE_SCALE;
            } else {
                // a_e = (g_e+g_e) + inv_g1_g2_e + 1
                a = f_mult(f_mult(g1, g2), inv_g1_g2);
                // b_e = (g_e+g_e) + inv_g1_g2_e
                b = scale_value(f_mult(g2.wrapping_sub(g1), inv_g1_g2), -g_e);
                shift = (g_e + g_e) + inv_g1_g2_e + 1 - NSHAPE_SCALE;
            }

            let st = step.max(0) as usize;
            for i in k * st..(k + 1) * st {
                // rr[i] = 2*a*r[i] + b*rr[i-1]
                let mut tmp = f_mult(a, r[i]);
                tmp = tmp.wrapping_add(scale_value(f_mult_div2(b, rr_minus_one), NSHAPE_SCALE));
                tmp = scale_value_saturate(tmp, shift);
                rr_minus_one = tmp;
                r[i] = tmp;
            }
        }
    }

    // end of mdct_IntNoiseShaping()
    *p_scale = (*p_scale as i32 + NSHAPE_SCALE) as i16;
}

/// `calcEnergy(r, rs, lg, rms_e)`: calculate the energy of the spectrum `r` (scale factor
/// `rs`, `lg` values). Returns the mantissa, `rms_e` receives the exponent.
pub fn calc_energy(r: &[FixpDbl], rs: i16, lg: i32, rms_e: &mut i32) -> FixpDbl {
    let lgu = lg.max(0) as usize;
    let headroom = get_scalefactor(&r[..lgu]);

    let mut rms_m: FixpDbl;

    // Calculate number of growth bits due to addition
    let mut shift = f_normz(lg);
    shift = 31 - shift;

    // Generate 1e-2 in Q-6.37
    let value0_01: FixpDbl = 0x51eb851e;
    let value0_01_exp: i32 = -6;

    // Find the exponent of the resulting energy value
    *rms_e = ((rs as i32 - headroom) << 1) + shift + 1;

    let mut delta = *rms_e - value0_01_exp;
    if delta > 0 {
        // Limit shift_to 31
        delta = f_min(31, delta);
        rms_m = value0_01 >> delta;
    } else {
        rms_m = value0_01;
        *rms_e = value0_01_exp;
        shift -= delta;
        // Limit shift_to 31
        shift = f_min(31, shift);
    }

    for i in 0..lgu {
        rms_m = rms_m.wrapping_add(f_pow2_div2(r[i] << headroom) >> shift);
    }

    rms_m
}

/// `calcTCXGain(pAacDecoderChannelInfo, r, rms_m, rms_e, frame, lg)`: TCX gain calculation.
pub fn calc_tcx_gain(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    _r: &[FixpDbl],
    mut rms_m: FixpDbl,
    mut rms_e: i32,
    frame: usize,
    lg: i32,
) {
    if rms_m != 0 {
        let mut tcx_gain_m: FixpDbl = 0;
        let mut tcx_gain_e: i32 = 0;

        clpd_decode_gain(
            &mut tcx_gain_m,
            &mut tcx_gain_e,
            p_aac_decoder_channel_info.p_dyn_data.specific_to.usac.tcx_global_gain[frame] as i32,
        );

        // rms * 2^rms_e = lg/sqrt(sum(spec^2))
        if rms_e & 1 != 0 {
            rms_m >>= 1;
            rms_e += 1;
        }

        {
            let mut inv_e = 0;

            // lg = fx_lg * 2^fx_lg_e
            let s = f_norm(lg);
            let fx_lg = lg << s;
            let fx_lg_e = DFRACT_BITS - 1 - s;
            // 1/sqrt(rms)
            rms_m = inv_sqrt_norm2(rms_m, &mut inv_e);
            rms_m = f_mult(rms_m, fx_lg);
            rms_e = inv_e - (rms_e >> 1) + fx_lg_e;
        }

        {
            let s = f_norm(tcx_gain_m);
            tcx_gain_m <<= s;
            tcx_gain_e -= s;
        }

        tcx_gain_m = f_mult_div2(tcx_gain_m, rms_m);
        tcx_gain_e += rms_e;

        // global_gain * 2^(global_gain_e+rms_e) = (10^(global_gain/28)) * rms * 2^rms_e
        tcx_gain_e += 1;

        let usac = &mut p_aac_decoder_channel_info.data.usac;
        usac.tcx_gain[frame] = tcx_gain_m;
        usac.tcx_gain_e[frame] = tcx_gain_e as i8;

        p_aac_decoder_channel_info.spec_scale[frame] =
            (p_aac_decoder_channel_info.spec_scale[frame] as i32 + tcx_gain_e) as i16;
    }
}

/// `CLpd_FdnsDecode(pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo, r, lg, frame, pScale,
/// A1, A1_exp, A2, A2_exp, pAlfdGains, fdns_npts)`: FDNS decoding of the spectrum `r` (`lg`
/// values) of TCX frame `frame`; the scale of `r` is `specScale[frame]`. `alfd_gains`: ALFD
/// gains output (scaled by 1).
#[allow(clippy::too_many_arguments)]
pub fn clpd_fdns_decode(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    r: &mut [FixpDbl],
    lg: i32,
    frame: usize,
    a1: &[FixpLpc],
    a1_exp: i32,
    a2: &[FixpLpc],
    a2_exp: i32,
    p_alfd_gains: &mut [FixpDbl],
    fdns_npts: i32,
) {
    // Weight LPC coefficients using Rm values
    clpd_adapt_low_freq_deemph(
        r,
        lg,
        p_alfd_gains,
        p_aac_decoder_channel_info.spec_scale[frame] as i32,
    );

    let mut rms_e = 0;
    // Calculate Energy
    let rms_m = calc_energy(r, p_aac_decoder_channel_info.spec_scale[frame], lg, &mut rms_e);

    calc_tcx_gain(p_aac_decoder_channel_info, r, rms_m, rms_e, frame, lg);

    // Apply ODFT and Noise Shaping. LP coefficient (A1, A2) weighting is done inside on the
    // fly.
    let mut sc = p_aac_decoder_channel_info.spec_scale[frame];
    lpc2mdct_and_noise_shaping(r, &mut sc, lg, fdns_npts, a1, a1_exp, a2, a2_exp);
    p_aac_decoder_channel_info.spec_scale[frame] = sc;
}

/// `find_mpitch(xri, lg)`: find the pitch for TCX20 (time domain) concealment.
pub fn find_mpitch(xri: &[FixpDbl], lg: i32) -> i32 {
    let mut pitch_e = 0;

    let mut max: FixpDbl = 0;
    let mut n = 2;

    // find maximum below 400Hz
    let mut i = 2;
    while i < (lg >> 4) {
        let tmp = f_pow2_div2(xri[i as usize]).wrapping_add(f_pow2_div2(xri[i as usize + 1]));
        if tmp > max {
            max = tmp;
            n = i;
        }
        i += 2;
    }

    // pitch = ((float)lg<<1)/(float)n;
    let mut pitch = f_div_norm(lg << 1, n, &mut pitch_e);
    pitch = pitch.wrapping_shr(fix_max(0, DFRACT_BITS - 1 - pitch_e - 16) as u32);

    // find pitch multiple under 20ms
    if pitch >= ((256 << 16) - 1) {
        // 231.0f
        n = 256;
    } else {
        let mut mpitch = pitch;
        // pitch is at least 2*lg/(lg/16) = 32 (in Q16) for the transform lengths of the decoder;
        // the loop runs only for a positive pitch.
        if pitch > 0 {
            while mpitch < (255 << 16) {
                mpitch += pitch;
            }
        }
        n = (mpitch - pitch) >> 16;
    }

    n
}

/// `lg_table_ccfl[2][4]`: number of spectral coefficients / time domain samples using frame
/// mode as index.
static LG_TABLE_CCFL: [[i32; 4]; 2] = [
    [256, 256, 512, 1024], // coreCoderFrameLength = 1024
    [192, 192, 384, 768],  // coreCoderFrameLength = 768
];

/// `CLpd_TcxDecode(pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo, flags, mod, last_mod,
/// frame, frameOk)`: decode and render one MDCT-TCX frame (`spec`: the channel's spectrum).
#[allow(clippy::too_many_arguments)]
pub fn clpd_tcx_decode(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    _flags: u32,
    mod_: i32,
    last_mod: i32,
    frame: i32,
    frame_ok: i32,
    spec: &mut [FixpDbl],
) {
    let ci = p_aac_decoder_channel_info;
    let sci = p_aac_decoder_static_channel_info;
    let gl = ci.granule_length;
    // mod is 1, 2 or 3 here
    let mu = mod_.clamp(0, 3) as usize;
    let lg = if gl == 128 { LG_TABLE_CCFL[0][mu] } else { LG_TABLE_CCFL[1][mu] };
    let next_frame = (frame + (1 << (mod_ - 1))) as usize;
    let is_full_band_lpd = 0;
    let frame = frame as usize;
    let coeffs_off = spec_tcx(frame as i32, gl, is_full_band_lpd);

    // Obtain r[] vector by combining the quant[] and noise[] vectors
    {
        let coeffs = &mut spec[coeffs_off..];
        let scale = ci.spec_scale[frame] as i32;
        let mut tcx_noise_factor = ci.p_dyn_data.specific_to.usac.tcx_noise_factor[frame];

        // find pitch for bfi case
        sci.last_tcx_pitch = find_mpitch(coeffs, lg) as i16;

        if frame_ok != 0 {
            // store for concealment
            sci.last_tcx_noise_factor = tcx_noise_factor;
        } else {
            // restore last frames value
            tcx_noise_factor = sci.last_tcx_noise_factor;
        }

        let mut noise_level: FixpDbl =
            fl2fxconst_dbl(0.0625f32 as f64).wrapping_mul(8 - tcx_noise_factor as i32);
        // -scale can be -33 (an extreme spectrum scale); scale_value() shifts by the amount
        // modulo 32 like the reference builds.
        noise_level = scale_value(noise_level, -scale);

        let neg_noise_level = noise_level.wrapping_neg();

        let nf_bgn = lg / 6;
        let nf_end = lg;

        let seed = &mut sci.nf_random_seed;
        let mut i = nf_bgn;
        while i < nf_end - 7 {
            let iu = i as usize;
            // Fill all 8 consecutive zero coeffs with noise
            let tmp = coeffs[iu]
                | coeffs[iu + 1]
                | coeffs[iu + 2]
                | coeffs[iu + 3]
                | coeffs[iu + 4]
                | coeffs[iu + 5]
                | coeffs[iu + 6]
                | coeffs[iu + 7];

            if tmp == 0 {
                for k in iu..iu + 8 {
                    coeffs[k] =
                        if usac_random_sign(seed) != 0 { neg_noise_level } else { noise_level };
                }
            }
            i += 8;
        }
        if (nf_end - i) > 0 {
            // noise filling for last "band" with less than 8 bins
            let iu = i as usize;
            let mut tmp = coeffs[iu];

            fdk_assert!((nf_end - i) < 8);
            for k in 1..(nf_end - i) as usize {
                tmp |= coeffs[iu + k];
            }
            if tmp == 0 {
                for k in iu..nf_end as usize {
                    coeffs[k] =
                        if usac_random_sign(seed) != 0 { neg_noise_level } else { noise_level };
                }
            }
        }
    }

    {
        // Convert LPC to LP domain
        let usac = &mut ci.data.usac;
        if last_mod == 0 {
            // Note: The case where last_mod == 255 is handled by other means in
            // CLpdChannelStream_Read()
            let lsp = usac.lsp_coeff[frame];
            e_lpc_f_lsp_a_conversion(
                &lsp,
                &mut usac.lp_coeff[frame],
                &mut usac.lp_coeff_exp[frame],
            );
        }

        let lsp = usac.lsp_coeff[next_frame];
        e_lpc_f_lsp_a_conversion(
            &lsp,
            &mut usac.lp_coeff[next_frame],
            &mut usac.lp_coeff_exp[next_frame],
        );

        let a1 = usac.lp_coeff[frame];
        let a1_exp = usac.lp_coeff_exp[frame];
        let a2 = usac.lp_coeff[next_frame];
        let a2_exp = usac.lp_coeff_exp[next_frame];

        // FDNS decoding
        clpd_fdns_decode(
            ci,
            &mut spec[coeffs_off..],
            lg,
            frame,
            &a1,
            a1_exp,
            &a2,
            a2_exp,
            &mut sci.last_alfd_gains,
            gl / 2, // == FDNS_NPTS(ccfl)
        );
    }
}

/// `CLpd_TCX_Read(hBs, pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo, lg,
/// first_tcx_flag, frame, flags)`: read the `tcx_coding` bitstream part.
#[allow(clippy::too_many_arguments)]
pub fn clpd_tcx_read(
    h_bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    lg: i32,
    first_tcx_flag: i32,
    frame: i32,
    flags: u32,
    spec: &mut [FixpDbl],
) -> AacDecoderError {
    let mut error_aac = AAC_DEC_OK;
    let mut arith_reset_flag = 0;
    let is_full_band_lpd = 0;
    let ci = p_aac_decoder_channel_info;
    let f = frame as usize;

    let p_spec_off = spec_tcx(frame, ci.granule_length, is_full_band_lpd);

    // TCX noise level
    ci.p_dyn_data.specific_to.usac.tcx_noise_factor[f] = fdk_read_bits(h_bs, 3) as u8;
    // TCX global gain
    ci.p_dyn_data.specific_to.usac.tcx_global_gain[f] = fdk_read_bits(h_bs, 7) as u8;

    // Arithmetic coded residual/spectrum
    if first_tcx_flag != 0 {
        if flags & AC_INDEP != 0 {
            arith_reset_flag = 1;
        } else {
            arith_reset_flag = fdk_read_bits(h_bs, 1) as i32;
        }
    }

    let p_spec = match spec.get_mut(p_spec_off..) {
        Some(s) => s,
        None => &mut [],
    };

    // CArco_DecodeArithData() output scale of "pSpec" is DFRACT_BITS-1
    let error = match p_aac_decoder_static_channel_info.h_ar_co.as_deref_mut() {
        Some(arco) => carco_decode_arith_data(arco, h_bs, p_spec, lg, lg, arith_reset_flag),
        // Without hArCo the reference has no defined result.
        None => ARITH_CODER_ERROR,
    };

    // Rescale residual/spectrum
    {
        let n = (lg.max(0) as usize).min(p_spec.len());
        let mut scale = get_scalefactor(&p_spec[..n]) - 2; // Leave 2 bits headroom

        // Exponent of CArco_DecodeArithData() output is DFRACT_BITS; integer values.
        scale_values(&mut p_spec[..n], scale);
        scale = DFRACT_BITS - 1 - scale;

        ci.spec_scale[f] = scale as i16;
    }

    if error == ARITH_CODER_ERROR {
        error_aac = AAC_DEC_UNKNOWN;
    }

    error_aac
}

/// `CLpd_ReadAndMapLpdModeToModArray(mod, hBs, elFlags)`: translate `lpd_mode` into the
/// `mod[]` array which describes the mode of each LPD frame.
pub fn clpd_read_and_map_lpd_mode_to_mod_array(
    mod_: &mut [u8; 4],
    h_bs: &mut FdkBitStream,
    _el_flags: u32,
) -> AacDecoderError {
    let lpd_mode = fdk_read_bits(h_bs, 5) as i32;

    if lpd_mode > 25 || lpd_mode < 0 {
        return AAC_DEC_PARSE_ERROR;
    }

    match lpd_mode {
        25 => {
            // 1 80MS frame
            *mod_ = [3; 4];
        }
        24 => {
            // 2 40MS frames
            *mod_ = [2; 4];
        }
        _ => match lpd_mode >> 2 {
            4 => {
                // lpd_mode 19 - 16  => 1 40MS and 2 20MS frames
                mod_[0] = 2;
                mod_[1] = 2;
                mod_[2] = if lpd_mode & 1 != 0 { 1 } else { 0 };
                mod_[3] = if lpd_mode & 2 != 0 { 1 } else { 0 };
            }
            5 => {
                // lpd_mode 23 - 20 => 2 20MS and 1 40MS frames
                mod_[2] = 2;
                mod_[3] = 2;
                mod_[0] = if lpd_mode & 1 != 0 { 1 } else { 0 };
                mod_[1] = if lpd_mode & 2 != 0 { 1 } else { 0 };
            }
            _ => {
                // lpd_mode < 16 => 4 20MS frames
                mod_[0] = if lpd_mode & 1 != 0 { 1 } else { 0 };
                mod_[1] = if lpd_mode & 2 != 0 { 1 } else { 0 };
                mod_[2] = if lpd_mode & 4 != 0 { 1 } else { 0 };
                mod_[3] = if lpd_mode & 8 != 0 { 1 } else { 0 };
            }
        },
    }
    AAC_DEC_OK
}

/// `CLpd_Reset(pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo, keep_past_signal)`
pub fn clpd_reset(
    _p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    sci: &mut CAacDecoderStaticChannelInfo,
    keep_past_signal: i32,
) {
    // Reset TCX / ACELP common memory
    if keep_past_signal == 0 {
        sci.old_synth.fill(0);
    }

    // Initialize the LSFs
    sci.lpc4_lsf.copy_from_slice(&FDK_DEC_LSF_INIT);

    // Reset memory needed by bass post-filter
    sci.mem_bpf.fill(0);

    sci.old_bpf_control_info = 0;
    for i in 0..SYN_SFD as usize {
        sci.old_t_pf[i] = 64;
        sci.old_gain_pf[i] = 0;
    }

    // Reset ACELP memory
    clpd_acelp_reset(&mut sci.acelp);

    sci.last_lpc_lost = 0; // prev_lpc_lost
    sci.last_tcx_pitch = L_DIV as i16; // pitch_tcx
    sci.num_lost_lpd_frames = 0; // nbLostCmpt
}

/*
 * Externally visible functions
 */

/// `CLpdChannelStream_Read(hBs, pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo,
/// pSamplingRateInfo, flags)`: read a `lpd_channel_stream`. `spec` is the channel's spectrum
/// (`pSpectralCoefficient`, 1024 values).
pub fn clpd_channel_stream_read(
    h_bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    flags: u32,
    spec: &mut [FixpDbl],
) -> AacDecoderError {
    let ci = p_aac_decoder_channel_info;
    let sci = p_aac_decoder_static_channel_info;
    let mut error: AacDecoderError;
    let mut fac_get_mem_state = 0;
    let lg_table_offset = 0;
    let lg_table = if ci.granule_length == 128 {
        &LG_TABLE_CCFL[0][lg_table_offset..]
    } else {
        &LG_TABLE_CCFL[1][lg_table_offset..]
    };
    let mut last_lpc_lost = sci.last_lpc_lost as i32;

    let last_frame_ok = cconcealment_get_last_frame_ok(Some(&sci.concealment_info), 1);

    let sampling_rate = p_sampling_rate_info.sampling_rate;

    let i_offset = (sampling_rate
        .wrapping_mul(PIT_MIN_12k8 as u32)
        .wrapping_add((FSCALE_DENOM / 2) as u32) as i32)
        / FSCALE_DENOM
        - PIT_MIN_12k8;

    'bail: {
        if (sampling_rate < FAC_FSCALE_MIN as u32) || (sampling_rate > FAC_FSCALE_MAX as u32) {
            error = AAC_DEC_PARSE_ERROR;
            break 'bail;
        }

        let acelp_core_mode = fdk_read_bits(h_bs, 3) as i32;

        // lpd_mode
        error = clpd_read_and_map_lpd_mode_to_mod_array(&mut ci.data.usac.r#mod, h_bs, 0);
        if error != AAC_DEC_OK {
            break 'bail;
        }
        let mod_ = ci.data.usac.r#mod;

        // bpf_control_info
        ci.data.usac.bpf_control_info = fdk_read_bit(h_bs) as u8;

        // last_core_mode
        let prev_frame_was_lpd = fdk_read_bit(h_bs);
        // fac_data_present
        let f_fac_data_present = fdk_read_bit(h_bs) as i32;

        // Set valid values from pAacDecoderStaticChannelInfo->{last_core_mode,last_lpd_mode}
        ci.data.usac.core_mode_last = sci.last_core_mode;
        ci.data.usac.lpd_mode_last = sci.last_lpd_mode;
        let lpd_mode_last0 = sci.last_lpd_mode;

        if prev_frame_was_lpd == 0 {
            // Last frame was FD
            ci.data.usac.core_mode_last = FD_LONG;
            ci.data.usac.lpd_mode_last = 255;
        } else {
            // Last frame was LPD
            ci.data.usac.core_mode_last = LPD;
            if ((mod_[0] == 0) && f_fac_data_present != 0)
                || ((mod_[0] != 0) && f_fac_data_present == 0)
            {
                // Currend mod is ACELP, fac data present -> TCX, current mod TCX, no fac data ->
                // TCX
                if lpd_mode_last0 == 0 {
                    // Bit stream interruption detected. Assume last TCX mode as TCX20.
                    ci.data.usac.lpd_mode_last = 1;
                }
                // Else assume that remembered TCX mode is correct.
            } else {
                ci.data.usac.lpd_mode_last = 0;
            }
        }

        // Depends on bitstream configuration
        let first_lpd_flag = (ci.data.usac.core_mode_last != LPD) as i32;
        let mut first_tcx_flag = 1;

        if sci.last_core_mode != LPD {
            // ATTENTION: Reset depends on what we rendered before!
            clpd_reset(ci, sci, 0);

            if last_frame_ok == 0 {
                // If last rendered frame was not LPD and first lpd flag is not set, this must
                // be an error - set last_lpc_lost flag
                last_lpc_lost |= if first_lpd_flag != 0 { 0 } else { 1 };
            }
        }

        let mut core_mode_last = ci.data.usac.core_mode_last;
        let mut lpd_mode_last = ci.data.usac.lpd_mode_last as i32;

        let nb_div = NB_DIV as usize;

        // k is the frame index. If a frame is of size 40MS or 80MS, this frame index is
        // incremented 2 or 4 instead of 1 respectively.

        let mut k = 0usize;
        while k < nb_div {
            // Reset FAC data pointers in order to avoid applying old random FAC data.
            ci.data.usac.fac_data[k] = FacDataPtr::Null;

            if (k == 0 && core_mode_last == LPD && f_fac_data_present != 0)
                || (lpd_mode_last == 0 && mod_[k] > 0)
                || ((lpd_mode_last != 255) && lpd_mode_last > 0 && mod_[k] == 0)
            {
                // Assign FAC memory
                ci.data.usac.fac_data[k] = clpd_fac_get_memory(ci, &mod_, &mut fac_get_mem_state);

                // FAC for (ACELP -> TCX) or (TCX -> ACELP)
                let gl = ci.granule_length;
                let usac = &mut ci.data.usac;
                let ptr = usac.fac_data[k];
                let mut empty: [FixpDbl; 0] = [];
                let p_fac = fac_data_slice(ptr, &mut usac.fac_data0, spec).unwrap_or(&mut empty);
                let err = clpd_fac_read(
                    h_bs,
                    p_fac,
                    &mut usac.fac_data_e,
                    gl, // == fac_length
                    0,
                    k as i32,
                );
                if err != 0 {
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
            }

            if mod_[k] == 0 {
                // acelp-mode
                let gl = ci.granule_length;
                let err = clpd_acelp_read(
                    h_bs,
                    &mut ci.data.usac.acelp[k],
                    acelp_core_mode,
                    gl * 8, // coreCoderFrameLength
                    i_offset,
                );
                if err != 0 {
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }

                lpd_mode_last = 0;
                k += 1;
            } else {
                // mode != 0  =>  TCX
                error = clpd_tcx_read(
                    h_bs,
                    ci,
                    sci,
                    lg_table[mod_[k] as usize],
                    first_tcx_flag,
                    k as i32,
                    flags,
                    spec,
                );

                lpd_mode_last = mod_[k] as i32;
                first_tcx_flag = 0;
                k += 1 << (mod_[k] - 1);
            }
            if error != AAC_DEC_OK {
                error = AAC_DEC_PARSE_ERROR;
                break 'bail;
            }
        }

        {
            // Read LPC coefficients
            let usac = &mut ci.data.usac;
            let err = clpc_read(
                h_bs,
                &mut usac.lsp_coeff,
                &mut sci.lpc4_lsf,
                &mut usac.lsf_adaptive_mean_cand,
                &mut usac.a_stability,
                &mod_,
                first_lpd_flag,
                // if last lpc4 is available from concealment do not extrapolate lpc0 from lpc2
                if mod_[0] & 0x3 != 0 {
                    0
                } else {
                    (last_lpc_lost != 0 && sci.last_core_mode != LPD) as i32
                },
                last_frame_ok,
            );
            if err != 0 {
                error = AAC_DEC_PARSE_ERROR;
                break 'bail;
            }
        }

        // adjust old lsp[] following to a bad frame (to avoid overshoot) (ref: dec_LPD.c)
        if last_lpc_lost != 0 && last_frame_ok == 0 {
            let mut k = 0usize;
            while k < nb_div {
                let k_next = k + if (mod_[k] & 0x3) == 0 { 1 } else { 1 << (mod_[k] - 1) };
                let lsp = &mut ci.data.usac.lsp_coeff;
                let lsp_new = lsp[k_next];
                let lsp_old = &mut lsp[k];

                for i in 0..M {
                    if lsp_new[i] < lsp_old[i] {
                        lsp_old[i] = lsp_new[i];
                    }
                }
                k = k_next;
            }
        }

        let usac = &mut ci.data.usac;
        if cconcealment_get_last_frame_ok(Some(&sci.concealment_info), 1) == 0 {
            let lsp = usac.lsp_coeff[0];
            e_lpc_f_lsp_a_conversion(&lsp, &mut usac.lp_coeff[0], &mut usac.lp_coeff_exp[0]);
        } else if sci.last_lpd_mode != 0 {
            if sci.last_lpd_mode == 255 {
                // We need it for TCX decoding or ACELP excitation update
                let lsp = usac.lsp_coeff[0];
                e_lpc_f_lsp_a_conversion(&lsp, &mut usac.lp_coeff[0], &mut usac.lp_coeff_exp[0]);
            } else {
                // last_lpd_mode was TCX
                // Copy old LPC4 LP domain coefficients to LPC0 LP domain buffer (to avoid
                // converting LSP coefficients again).
                usac.lp_coeff[0] = sci.lp_coeff_old[0];
                usac.lp_coeff_exp[0] = sci.lp_coeff_old_exp[0];
            }
        } // case last_lpd_mode was ACELP is handled by CLpd_TcxDecode()

        if f_fac_data_present != 0 && (core_mode_last != LPD) {
            let prev_frame_was_short = fdk_read_bit(h_bs);

            if prev_frame_was_short != 0 {
                core_mode_last = FD_SHORT;
                ci.data.usac.core_mode_last = FD_SHORT;
                ci.data.usac.lpd_mode_last = 255;

                if (sci.last_core_mode != FD_SHORT)
                    && cconcealment_get_last_frame_ok(Some(&sci.concealment_info), 1) != 0
                {
                    // USAC Conformance document:
                    // short_fac_flag   shall be encoded with a value of 1 if the
                    // window_sequence of the previous frame was 2 (EIGHT_SHORT_SEQUENCE).
                    // Otherwise short_fac_flag shall be encoded with a value of 0.
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
            }

            // Assign memory
            ci.data.usac.fac_data[0] = clpd_fac_get_memory(ci, &mod_, &mut fac_get_mem_state);

            {
                // FAC for FD -> ACELP
                let length =
                    clpd_fac_get_length((core_mode_last != FD_SHORT) as i32, ci.granule_length);
                let usac = &mut ci.data.usac;
                let ptr = usac.fac_data[0];
                let mut empty: [FixpDbl; 0] = [];
                let p_fac = fac_data_slice(ptr, &mut usac.fac_data0, spec).unwrap_or(&mut empty);
                let err = clpd_fac_read(h_bs, p_fac, &mut usac.fac_data_e, length, 1, 0);
                if err != 0 {
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
            }
        }
    }

    // bail:
    if error == AAC_DEC_OK {
        // check consitency of last core/lpd mode values
        if (ci.data.usac.core_mode_last != sci.last_core_mode) && (sci.last_lpc_lost == 0) {
            // Something got wrong!
            // error = AAC_DEC_PARSE_ERROR; // Throwing errors does not help
        } else if (ci.data.usac.core_mode_last == LPD)
            && (ci.data.usac.lpd_mode_last != sci.last_lpd_mode)
            && (sci.last_lpc_lost == 0)
        {
            // Something got wrong!
            // error = AAC_DEC_PARSE_ERROR; // Throwing errors does not help
        }
    }

    error
}

/// `CLpdChannelStream_Decode(pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo, flags)`:
/// decode one `lpd_channel_stream` (`spec`: the channel's spectrum).
pub fn clpd_channel_stream_decode(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    flags: u32,
    spec: &mut [FixpDbl],
) {
    let ci = p_aac_decoder_channel_info;
    let sci = p_aac_decoder_static_channel_info;
    let mod_ = ci.data.usac.r#mod;
    let nb_div = NB_DIV as usize;

    // k is the frame index. If a frame is of size 40MS or 80MS, this frame index is
    // incremented 2 or 4 instead of 1 respectively.
    let mut k = 0usize;
    // could be different to what has been rendered
    let mut last_lpd_mode = ci.data.usac.lpd_mode_last;
    while k < nb_div {
        if mod_[k] == 0 {
            // ACELP

            // If FAC (fac_data[k] != NULL), and previous frame was TCX, apply (TCX) gains to
            // FAC data
            if last_lpd_mode > 0
                && last_lpd_mode != 255
                && ci.data.usac.fac_data[k] != FacDataPtr::Null
            {
                let gl = ci.granule_length;
                let usac = &mut ci.data.usac;
                let mut zero = [0 as FixpDbl; LFAC as usize];
                let ptr = usac.fac_data[k];
                let p = fac_data_slice(ptr, &mut usac.fac_data0, spec).unwrap_or(&mut zero);
                cfac_apply_gains(
                    p,
                    gl,
                    sci.last_tcx_gain,
                    &sci.last_alfd_gains,
                    if last_lpd_mode < 4 { last_lpd_mode as i32 } else { 3 },
                );

                usac.fac_data_e[k] = (usac.fac_data_e[k] as i32 + sci.last_tcx_gain_e) as i8;
            }
        } else {
            // TCX
            clpd_tcx_decode(
                ci,
                sci,
                flags,
                mod_[k] as i32,
                last_lpd_mode as i32,
                k as i32,
                1,
                spec,
            );

            // Store TCX gain scale for next possible FAC transition.
            sci.last_tcx_gain = ci.data.usac.tcx_gain[k];
            sci.last_tcx_gain_e = ci.data.usac.tcx_gain_e[k] as i32;

            // If FAC (fac_data[k] != NULL), apply gains
            if last_lpd_mode == 0 && ci.data.usac.fac_data[k] != FacDataPtr::Null {
                let gl = ci.granule_length;
                let usac = &mut ci.data.usac;
                let tcx_gain = usac.tcx_gain[k];
                let mut zero = [0 as FixpDbl; LFAC as usize];
                let ptr = usac.fac_data[k];
                let p = fac_data_slice(ptr, &mut usac.fac_data0, spec).unwrap_or(&mut zero);
                cfac_apply_gains(
                    p,
                    gl, // == fac_length
                    tcx_gain,
                    &sci.last_alfd_gains,
                    mod_[k] as i32,
                );

                usac.fac_data_e[k] = (usac.fac_data_e[k] as i32 + usac.tcx_gain_e[k] as i32) as i8;
            }
        }

        // remember previous mode
        last_lpd_mode = mod_[k];

        // Increase k to next frame
        k += if mod_[k] == 0 { 1 } else { 1 << (mod_[k] - 1) };
    }
}

/// The FAC data of `fac_data[k]` copied into `buf` (`len` values; zero for `NULL` and
/// [`FacDataPtr::Scratch`]).
fn fac_copy_in(ptr: FacDataPtr, fac_data0: &[FixpDbl], spec: &[FixpDbl], buf: &mut [FixpDbl]) {
    buf.fill(0);
    let src: &[FixpDbl] = match ptr {
        FacDataPtr::Null | FacDataPtr::Scratch => &[],
        FacDataPtr::FacData0 => fac_data0,
        FacDataPtr::Spectral(o) => spec.get(o..).unwrap_or(&[]),
    };
    let n = src.len().min(buf.len());
    buf[..n].copy_from_slice(&src[..n]);
}

/// Writes the FAC data `buf` back to the memory of `fac_data[k]`.
fn fac_copy_out(ptr: FacDataPtr, fac_data0: &mut [FixpDbl], spec: &mut [FixpDbl], buf: &[FixpDbl]) {
    let dst: &mut [FixpDbl] = match ptr {
        FacDataPtr::Null | FacDataPtr::Scratch => return,
        FacDataPtr::FacData0 => fac_data0,
        FacDataPtr::Spectral(o) => match spec.get_mut(o..) {
            Some(s) => s,
            None => return,
        },
    };
    let n = dst.len().min(buf.len());
    dst[..n].copy_from_slice(&buf[..n]);
}

/// `CLpd_RenderTimeSignal(pAacDecoderStaticChannelInfo, pAacDecoderChannelInfo, pTimeData,
/// lFrame, pSamplingRateInfo, frameOk, aacOutDataHeadroom, flags, strmFlags)`: generate the
/// time domain output signal (`lFrame` values of `p_time_data`) of an LPD channel stream.
///
/// `spec` is the channel's spectrum (`pSpectralCoefficient`), `work_buffer_core1` the decoder's
/// `pComStaticData->pWorkBufferCore1` (the synthesis buffer `synth_buf`).
#[allow(clippy::too_many_arguments)]
pub fn clpd_render_time_signal(
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_time_data: &mut [PcmDec],
    l_frame: i32,
    p_sampling_rate_info: &SamplingRateInfo,
    frame_ok: u32,
    aac_out_data_headroom: i32,
    flags: u32,
    _strm_flags: u32,
    spec: &mut [FixpDbl],
    work_buffer_core1: &mut CWorkBufferCore1,
) -> AacDecoderError {
    let sci = p_aac_decoder_static_channel_info;
    let ci = p_aac_decoder_channel_info;
    let error = AAC_DEC_OK;
    let mut frame_ok = frame_ok;
    let mut i_offset = 0;
    let mut last_k: i32;
    let mut nr_samples: i32 = 0;
    let fac_fb = 1;
    let nb_div = NB_DIV;
    let l_div = l_frame / nb_div; // length of division (acelp or tcx20 frame)
    let l_fac = l_div / 2;
    let nb_subfr = l_frame / (nb_div * L_SUBFR); // number of subframes per division
    let nb_subfr_superfr = nb_div * nb_subfr;
    let syn_sfd = (nb_subfr_superfr / 2) - BPF_SFD;
    let syn_delay = syn_sfd * L_SUBFR;
    let aac_delay = l_frame / 2;

    // In respect to the reference software, the synth pointer here is lagging by aacDelay
    // ( == SYN_DELAY + BPF_DELAY ) samples. The corresponding old synthesis samples are handled
    // by the IMDCT overlap.

    let synth_buf = work_buffer_core1.synth_buf();
    let synth = (PIT_MAX_MAX - BPF_DELAY) as usize; // synth = synth_buf + PIT_MAX_MAX - BPF_DELAY

    let mut pitch = [0i32; (NB_SUBFR_SUPERFR + SYN_SFD) as usize];
    let mut pit_gain = [0 as FixpDbl; (NB_SUBFR_SUPERFR + SYN_SFD) as usize];

    let lg_table_offset = 0;

    let sampling_rate = p_sampling_rate_info.sampling_rate;

    // FDKmemclear(pitch, ...): zero-initialized.

    if flags & AACDEC_FLUSH != 0 {
        clpd_reset(ci, sci, (flags & AACDEC_FLUSH) as i32);
        frame_ok = 0;
    }

    let lg_table: &[i32] = match l_frame {
        1024 => &LG_TABLE_CCFL[0][lg_table_offset..],
        768 => &LG_TABLE_CCFL[1][lg_table_offset..],
        _ => {
            fdk_assert!(false);
            return AAC_DEC_UNKNOWN;
        }
    };

    let mut last_frame_lost =
        (cconcealment_get_last_frame_ok(Some(&sci.concealment_info), 0) == 0) as u8;

    // Maintain LPD mode from previous frame
    if (sci.last_core_mode == FD_LONG) || (sci.last_core_mode == FD_SHORT) {
        sci.last_lpd_mode = 255;
    }

    if frame_ok == 0 {
        let last_lpd_mode = sci.last_lpd_mode;
        let old_tcx_gain = sci.last_tcx_gain;
        let old_tcx_gain_e = sci.last_tcx_gain_e as i8;
        let old_stab = sci.old_stability;
        let mut n_lost_sf = sci.num_lost_lpd_frames;

        // patch the last LPD mode
        ci.data.usac.lpd_mode_last = last_lpd_mode;

        // Do mode extrapolation and repeat the previous mode:
        //   if previous mode = ACELP        -> ACELP
        //   if previous mode = TCX-20/40    -> TCX-20
        //   if previous mode = TCX-80       -> TCX-80
        // notes:
        //   - ACELP is not allowed after TCX (no pitch information to reuse)
        //   - TCX-40 is not allowed in the mode repetition to keep the logic simple
        let m: u8 = match last_lpd_mode {
            0 => 0, // -> ACELP concealment
            3 => 3, // -> TCX FD concealment
            2 => 2, // -> TCX FD concealment
            _ => 4, // -> TCX TD concealment
        };
        ci.data.usac.r#mod = [m; 4];

        // LPC extrapolation
        clpc_conceal(
            &mut ci.data.usac.lsp_coeff,
            &mut sci.lpc4_lsf,
            &sci.lsf_adaptive_mean,
            // (pAacDecoderStaticChannelInfo->numLostLpdFrames == 0) ||
            (last_lpd_mode == 255) as i32,
        );

        if (last_lpd_mode > 0) && (last_lpd_mode < 255) {
            // Copy old LPC4 LP domain coefficients to LPC0 LP domain buffer (to avoid converting
            // LSP coefficients again).
            ci.data.usac.lp_coeff[0] = sci.lp_coeff_old[0];
            ci.data.usac.lp_coeff_exp[0] = sci.lp_coeff_old_exp[0];
        } // case last_lpd_mode was ACELP is handled by CLpd_TcxDecode()
          // case last_lpd_mode was Time domain TCX concealment is handled after this
          // "if (!frameOk)"-block

        // k is the frame index. If a frame is of size 40MS or 80MS, this frame index is
        // incremented 2 or 4 instead of 1 respectively.
        let mod_ = ci.data.usac.r#mod;
        let mut k = 0usize;
        while k < nb_div as usize {
            ci.data.usac.tcx_gain[k] = old_tcx_gain;
            ci.data.usac.tcx_gain_e[k] = old_tcx_gain_e;

            // restore stability value from last frame
            ci.data.usac.a_stability[k] = old_stab;

            // Increase k to next frame
            k += if (mod_[k] & 0x3) == 0 { 1 } else { 1 << ((mod_[k] & 0x3) - 1) };

            n_lost_sf = n_lost_sf.wrapping_add(1);
        }
        let _ = n_lost_sf;
    } else {
        if (sci.last_lpd_mode == 4) && (ci.data.usac.r#mod[0] > 0) {
            // Copy old LPC4 LP domain coefficients to LPC0 LP domain buffer (to avoid converting
            // LSP coefficients again).
            ci.data.usac.lp_coeff[0] = sci.lp_coeff_old[0];
            ci.data.usac.lp_coeff_exp[0] = sci.lp_coeff_old_exp[0];
        }
    }

    acelp_pre_processing(
        synth_buf,
        &sci.old_synth,
        &mut pitch,
        &sci.old_t_pf,
        &mut pit_gain,
        &sci.old_gain_pf,
        sampling_rate as i32,
        &mut i_offset,
        l_frame,
        syn_sfd,
        nb_subfr_superfr,
    );

    let mod_ = ci.data.usac.r#mod;

    // k is the frame index. If a frame is of size 40MS or 80MS, this frame index is
    // incremented 2 or 4 instead of 1 respectively.
    let mut k: i32 = 0;
    last_k = -1; // mark invalid
    let mut last_lpd_mode: u8 = sci.last_lpd_mode;
    let mut last_last_lpd_mode: u8 = sci.last_last_lpd_mode;
    let mut last_lpc_lost: u8 = sci.last_lpc_lost | last_frame_lost;

    // This buffer must be avalable for the case of FD->ACELP transition. The beginning of the
    // buffer is used after the BPF to overwrite the output signal. Only the FAC area must be
    // affected by the BPF

    while k < nb_div {
        let ku = k as usize;
        if frame_ok == 0 {
            sci.num_lost_lpd_frames = sci.num_lost_lpd_frames.wrapping_add(1);
        } else {
            last_frame_lost |= if sci.num_lost_lpd_frames > 0 { 1 } else { 0 };
            sci.num_lost_lpd_frames = 0;
        }
        if mod_[ku] == 0 || mod_[ku] == 4 {
            // ACELP or TCX time domain concealment

            // FAC management
            if (last_lpd_mode != 0) && (last_lpd_mode != 4) {
                // TCX TD concealment
                let fac_length = clpd_fac_get_length(
                    ((sci.last_core_mode != FD_SHORT) || (k > 0)) as i32,
                    l_fac,
                );
                let mut fac_buf = [0 as FixpDbl; LFAC as usize];
                let p_fac_data = if frame_ok != 0 && last_frame_lost == 0 {
                    ci.data.usac.fac_data[ku]
                } else {
                    FacDataPtr::Null
                };
                fac_copy_in(p_fac_data, &ci.data.usac.fac_data0, spec, &mut fac_buf);
                let fac_opt: Option<&mut [FixpDbl]> =
                    if p_fac_data != FacDataPtr::Null { Some(&mut fac_buf[..]) } else { None };

                nr_samples += clpd_fac_mdct2acelp(
                    &mut sci.i_mdct,
                    synth_buf,
                    synth + nr_samples as usize,
                    fac_opt,
                    ci.data.usac.fac_data_e[ku] as i32,
                    &ci.data.usac.lp_coeff[ku],
                    ci.data.usac.lp_coeff_exp[ku],
                    l_frame - nr_samples,
                    fac_length,
                    ((sci.last_core_mode != LPD) && (k == 0)) as i32,
                    0,
                );
                if p_fac_data != FacDataPtr::Null {
                    fac_copy_out(
                        p_fac_data,
                        &mut ci.data.usac.fac_data0,
                        spec,
                        &fac_buf[..fac_length.clamp(0, LFAC) as usize],
                    );
                }

                {
                    let o = synth + nr_samples as usize;
                    let n = sci.i_mdct.ov_offset.max(0) as usize;
                    let n = n.min(synth_buf.len().saturating_sub(o)).min(sci.i_mdct.overlap.len());
                    synth_buf[o..o + n].copy_from_slice(&sci.i_mdct.overlap[..n]);
                }
                {
                    // init value does not real matter
                    let mut lp_prev: [FixpLpc; M] = ci.data.usac.lp_coeff[0];
                    let mut lp_prev_exp = ci.data.usac.lp_coeff_exp[0];

                    if last_lpd_mode != 255 {
                        // last mode was tcx
                        last_k = k - (1 << (last_lpd_mode - 1));
                        if last_k < 0 {
                            lp_prev = sci.lp_coeff_old[1];
                            lp_prev_exp = sci.lp_coeff_old_exp[1];
                        } else {
                            lp_prev = ci.data.usac.lp_coeff[last_k as usize];
                            lp_prev_exp = ci.data.usac.lp_coeff_exp[last_k as usize];
                        }
                    }

                    clpd_acelp_prepare_internal_mem(
                        synth_buf,
                        synth + (aac_delay + k * l_div) as usize,
                        last_lpd_mode,
                        if last_last_lpd_mode == 4 { 0 } else { last_last_lpd_mode },
                        &ci.data.usac.lp_coeff[ku],
                        ci.data.usac.lp_coeff_exp[ku],
                        &lp_prev,
                        lp_prev_exp,
                        &mut sci.acelp,
                        l_frame,
                        (last_frame_lost != 0 && k < 2) as i32,
                        mod_[ku],
                    );
                }
            } else {
                if k == 0 && sci.i_mdct.ov_offset != l_frame / fac_fb / 2 {
                    sci.i_mdct.ov_offset = l_frame / fac_fb / 2;
                }
                let o = synth + nr_samples as usize;
                nr_samples += imdct_drain(
                    &mut sci.i_mdct,
                    &mut synth_buf[o..],
                    l_frame / fac_fb - nr_samples,
                );
            }

            let to_overlap = nr_samples >= l_frame / fac_fb;
            let acelp_out_off;
            if to_overlap {
                // Write ACELP time domain samples into IMDCT overlap buffer at
                // pAacDecoderStaticChannelInfo->IMdct.overlap.time +
                // pAacDecoderStaticChannelInfo->IMdct.ov_offset
                acelp_out_off = sci.i_mdct.ov_offset;

                // Account ACELP time domain output samples to overlap buffer
                sci.i_mdct.ov_offset += l_div;
            } else {
                // Write ACELP time domain samples into output buffer at pTimeData + nrSamples
                acelp_out_off = nr_samples;

                // Account ACELP time domain output samples to output buffer
                nr_samples += l_div;
            }

            // The ACELP output is written only; it is computed into a local buffer and copied.
            let mut acelp_out = [0 as FixpDbl; L_DIV as usize];
            let ld = l_div.clamp(0, L_DIV) as usize;

            if mod_[ku] == 4 {
                sci.acelp.wsyn_rms = scale_value(
                    ci.data.usac.tcx_gain[ku],
                    fix_min(0, ci.data.usac.tcx_gain_e[ku] as i32 - SF_EXC),
                );
                clpd_tcx_td_conceal(
                    &mut sci.acelp,
                    &sci.last_tcx_pitch,
                    &ci.data.usac.lsp_coeff[ku],
                    &ci.data.usac.lsp_coeff[ku + 1],
                    ci.data.usac.a_stability[ku],
                    sci.num_lost_lpd_frames as i32,
                    &mut acelp_out[..ld],
                    l_frame,
                    sci.last_tcx_noise_factor,
                );
            } else {
                fdk_assert!(ci.data.usac.a_stability[ku] >= 0);
                let ps = (k * nb_subfr + syn_sfd) as usize;
                clpd_acelp_decode(
                    &mut sci.acelp,
                    i_offset,
                    &ci.data.usac.lsp_coeff[ku],
                    &ci.data.usac.lsp_coeff[ku + 1],
                    ci.data.usac.a_stability[ku],
                    &ci.data.usac.acelp[ku],
                    sci.num_lost_lpd_frames as i32,
                    last_lpc_lost as i32,
                    k,
                    &mut acelp_out[..ld],
                    &mut pitch[ps..],
                    &mut pit_gain[ps..],
                    l_frame,
                );
            }

            {
                let dst: &mut [FixpDbl] =
                    if to_overlap { &mut sci.i_mdct.overlap[..] } else { &mut synth_buf[synth..] };
                let o = acelp_out_off.max(0) as usize;
                let n = ld.min(dst.len().saturating_sub(o));
                dst[o..o + n].copy_from_slice(&acelp_out[..n]);
            }

            if mod_[ku] != 4 {
                if last_lpd_mode != 0 && ci.data.usac.bpf_control_info != 0 {
                    // FD/TCX -> ACELP transition
                    // bass post-filter past FAC area (past two (one for FD short) subframes)
                    let current_sf = (syn_sfd + k * nb_subfr) as usize;

                    if (k > 0) || (sci.last_core_mode != FD_SHORT) {
                        // TCX or FD long -> ACELP
                        pitch[current_sf - 2] = pitch[current_sf];
                        pitch[current_sf - 1] = pitch[current_sf];
                        pit_gain[current_sf - 2] = pit_gain[current_sf];
                        pit_gain[current_sf - 1] = pit_gain[current_sf];
                    } else {
                        // FD short -> ACELP
                        pitch[current_sf - 1] = pitch[current_sf];
                        pit_gain[current_sf - 1] = pit_gain[current_sf];
                    }
                }
            }
        } else {
            // TCX
            let lg = lg_table[mod_[ku].min(3) as usize];
            let is_full_band_lpd = 0;
            let spec_off = spec_tcx(k, ci.granule_length, is_full_band_lpd);

            // FAC management
            if (last_lpd_mode == 0) || (last_lpd_mode == 4) {
                // TCX TD concealment

                // pAacDecoderChannelInfo->data.usac.fac_data[k] == NULL means no FAC data
                // available.
                if last_frame_lost == 1 || ci.data.usac.fac_data[ku] == FacDataPtr::Null {
                    // FDKmemclear(fac_buf, 1024 / 8 * sizeof(FIXP_DBL))
                    ci.data.usac.fac_data[ku] = FacDataPtr::Scratch;
                    ci.data.usac.fac_data_e[ku] = 0;
                }

                let gl = ci.granule_length;
                let ptr = ci.data.usac.fac_data[ku];
                let mut fac_buf = [0 as FixpDbl; LFAC as usize];
                fac_copy_in(ptr, &ci.data.usac.fac_data0, spec, &mut fac_buf);

                let wrs = fdk_get_window_slope(l_div, get_window_shape(&ci.ics_info) as i32);
                let a = ci.data.usac.lp_coeff[ku];
                let a_exp = ci.data.usac.lp_coeff_exp[ku];
                let tcx_gain = ci.data.usac.tcx_gain[ku];
                let fac_e = ci.data.usac.fac_data_e[ku] as i32;
                let mut empty: [FixpDbl; 0] = [];
                let p_spec = match spec.get_mut(spec_off..) {
                    Some(s) => s,
                    None => &mut empty,
                };
                nr_samples += clpd_fac_acelp2mdct(
                    &mut sci.i_mdct,
                    synth_buf,
                    synth + nr_samples as usize,
                    p_spec,
                    &ci.spec_scale[ku..],
                    1,
                    &mut fac_buf[..gl.clamp(0, LFAC) as usize],
                    fac_e,
                    gl, // == fac_length
                    l_frame - nr_samples,
                    lg,
                    wrs,
                    l_div,
                    &a,
                    a_exp,
                    &mut sci.acelp,
                    tcx_gain,
                    (last_frame_lost != 0 || frame_ok == 0) as i32,
                    0, // is not FD FAC
                    last_lpd_mode,
                    k,
                    // Note: The current aliasing symmetry for a TCX (i.e. LPD) frame must always
                    // be 0
                    ci.curr_aliasing_symmetry,
                );
                fac_copy_out(
                    ptr,
                    &mut ci.data.usac.fac_data0,
                    spec,
                    &fac_buf[..gl.clamp(0, LFAC) as usize],
                );

                let b = (k * nb_subfr + syn_sfd) as usize;
                pitch[b + 1] = pitch[b - 1];
                pitch[b] = pitch[b - 1];
                pit_gain[b + 1] = pit_gain[b - 1];
                pit_gain[b] = pit_gain[b - 1];
            } else {
                let tl = lg;
                let fl = l_div;
                let fr = l_div;

                let shape = get_window_shape(&ci.ics_info) as i32;
                let o = synth + nr_samples as usize;
                let mut empty: [FixpDbl; 0] = [];
                let p_spec = match spec.get_mut(spec_off..) {
                    Some(s) => s,
                    None => &mut empty,
                };
                nr_samples += imlt_block(
                    &mut sci.i_mdct,
                    &mut synth_buf[o..],
                    p_spec,
                    &ci.spec_scale[ku..],
                    1,
                    l_frame - nr_samples,
                    tl,
                    fdk_get_window_slope(fl, shape),
                    fl,
                    fdk_get_window_slope(fr, shape),
                    fr,
                    ci.data.usac.tcx_gain[ku],
                    if ci.curr_aliasing_symmetry != 0 { MLT_FLAG_CURR_ALIAS_SYMMETRY } else { 0 },
                );
            }
        }
        // remember previous mode
        last_last_lpd_mode = last_lpd_mode;
        last_lpd_mode = mod_[ku];
        last_lpc_lost = if frame_ok == 0 { 1 } else { 0 };

        // Increase k to next frame
        last_k = k;
        k += if (mod_[ku] & 0x3) == 0 { 1 } else { 1 << (mod_[ku] - 1) };
    }

    if frame_ok != 0 {
        // assume data was ok => store for concealment
        fdk_assert!(ci.data.usac.a_stability[last_k as usize] >= 0);
        sci.old_stability = ci.data.usac.a_stability[last_k as usize];
        sci.lsf_adaptive_mean = ci.data.usac.lsf_adaptive_mean_cand;
    }

    // store past lp coeffs for next superframe (they are only valid and needed if
    // last_lpd_mode was tcx)
    if last_lpd_mode > 0 {
        sci.lp_coeff_old[0] = ci.data.usac.lp_coeff[nb_div as usize];
        sci.lp_coeff_old_exp[0] = ci.data.usac.lp_coeff_exp[nb_div as usize];
        sci.lp_coeff_old[1] = ci.data.usac.lp_coeff[last_k as usize];
        sci.lp_coeff_old_exp[1] = ci.data.usac.lp_coeff_exp[last_k as usize];
    }

    fdk_assert!(nr_samples == l_frame);

    // check whether usage of bass postfilter was de-activated in the bitstream; if yes, set
    // pitch gain to 0
    if ci.data.usac.bpf_control_info == 0 {
        if mod_[0] != 0 && sci.old_bpf_control_info != 0 {
            for i in 2..nb_subfr_superfr as usize {
                pit_gain[syn_sfd as usize + i] = 0;
            }
        } else {
            for i in 0..nb_subfr_superfr as usize {
                pit_gain[syn_sfd as usize + i] = 0;
            }
        }
    }

    // for bass postfilter
    for n in 0..syn_sfd as usize {
        sci.old_t_pf[n] = pitch[nb_subfr_superfr as usize + n];
        sci.old_gain_pf[n] = pit_gain[nb_subfr_superfr as usize + n];
    }

    sci.old_bpf_control_info = ci.data.usac.bpf_control_info;

    {
        let mut lookahead = -BPF_DELAY;
        let copy_samp =
            if mod_[(nb_div - 1) as usize] == 0 { aac_delay } else { aac_delay - l_fac };

        // Copy enough time domain samples from MDCT to synthesis buffer as needed by the bass
        // postfilter

        let o = synth + nr_samples as usize;
        lookahead += imdct_copy_ov_and_nr(&sci.i_mdct, &mut synth_buf[o..], copy_samp);

        fdk_assert!(lookahead == copy_samp - BPF_DELAY);

        let p2_synth = synth + BPF_DELAY as usize;

        // recalculate pitch gain to allow postfilering on FAC area
        for i in 0..nb_subfr_superfr as usize {
            let t = pitch[i];
            let gain = pit_gain[i];

            if gain > 0 {
                let mut x = [0 as FixpDbl; L_SUBFR_U];
                let mut y = [0 as FixpDbl; L_SUBFR_U];
                let base = (p2_synth + i * L_SUBFR_U) as isize;
                window_copy(synth_buf, base, &mut x);
                window_copy(synth_buf, base - t as isize, &mut y);
                pit_gain[i] = get_gain(&x, &y, L_SUBFR);
            }
        }

        {
            bass_pf_1sf_delay(
                synth_buf,
                p2_synth,
                &pitch,
                &pit_gain,
                l_frame,
                l_frame / fac_fb,
                if mod_[(nb_div - 1) as usize] != 0 { syn_delay - (l_div / 2) } else { syn_delay },
                p_time_data,
                aac_out_data_headroom,
                &mut sci.mem_bpf,
            );
        }
    }

    acelp_post_processing(
        synth_buf,
        &mut sci.old_synth,
        &pitch,
        &mut sci.old_t_pf,
        l_frame,
        syn_sfd,
        nb_subfr_superfr,
    );

    // Store last mode for next super frame
    sci.last_core_mode = LPD;
    sci.last_lpd_mode = last_lpd_mode;
    sci.last_last_lpd_mode = last_last_lpd_mode;
    sci.last_lpc_lost = last_lpc_lost;

    error
}
