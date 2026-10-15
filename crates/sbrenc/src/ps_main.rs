// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/ps_main.h, libSBRenc/src/ps_main.cpp.

//! Port of `ps_main.h` and `ps_main.cpp`: the parametric stereo encoder wrapper and the
//! downmix.
//!
//! # Memory
//!
//! `pHybridData[HYBRID_READ_OFFSET + HYBRID_FRAMESIZE][MAX_PS_CHANNELS][2]` are pointers: rows
//! `0..HYBRID_READ_OFFSET` point to `__staticHybridData` (owned by [`ParametricStereo`]), the
//! following rows into the dynamic RAM of the SBR encoder (`GetRam_Sbr_envRBuffer(ch)` and
//! `GetRam_Sbr_envIBuffer(ch)`, rows of `MAX_HYBRID_BANDS` values, see
//! [`hybrid_dyn_offset`]). The PS processing functions therefore take the dynamic RAM, which
//! also holds the downmixed QMF data they write (the QMF buffers of the SBR envelope extraction
//! overlap the hybrid data of the left channel and are written slot by slot behind the hybrid
//! data the downmix reads, as in C).
//!
//! The hybrid analysis states (`__staticHybAnaStatesLF`, `__staticHybAnaStatesHF`) are owned
//! by the hybrid filter banks ([`fdk_aac_rs_fdk::hybrid`]).

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::hybrid::*;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::qmf::{FixpQss, QmfFilterBank};
use fdk_aac_rs_fdk::qmf_pcm::{qmf_analysis_filtering_slot, qmf_synthesis_filtering_slot};

use crate::ps_bitenc::*;
use crate::ps_const::*;
use crate::ps_encode::*;
use crate::sbr_misc::schur_div_fast;
use crate::sbrenc_ram::{ENV_I_BUFF_DBL, ENV_R_BUFF_DBL};

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

/// `PSENC_STEREO_BANDS_CONFIG`
pub type PsencStereoBandsConfig = i32;
/// `PSENC_STEREO_BANDS_INVALID`
pub const PSENC_STEREO_BANDS_INVALID: PsencStereoBandsConfig = 0;
/// `PSENC_STEREO_BANDS_10`
pub const PSENC_STEREO_BANDS_10: PsencStereoBandsConfig = 10;
/// `PSENC_STEREO_BANDS_20`
pub const PSENC_STEREO_BANDS_20: PsencStereoBandsConfig = 20;

/// `PSENC_NENV_CONFIG`
pub type PsencNenvConfig = i32;
/// `PSENC_NENV_1`
pub const PSENC_NENV_1: PsencNenvConfig = 1;
/// `PSENC_NENV_2`
pub const PSENC_NENV_2: PsencNenvConfig = 2;
/// `PSENC_NENV_4`
pub const PSENC_NENV_4: PsencNenvConfig = 4;
/// `PSENC_NENV_DEFAULT`
pub const PSENC_NENV_DEFAULT: PsencNenvConfig = PSENC_NENV_2;
/// `PSENC_NENV_MAX`
pub const PSENC_NENV_MAX: PsencNenvConfig = PSENC_NENV_4;

/// `psTuningTable_t`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsTuningTable {
    /// `bitrateFrom`: inclusive
    pub bitrate_from: u32,
    /// `bitrateTo`: exclusive
    pub bitrate_to: u32,
    /// `nStereoBands`
    pub n_stereo_bands: PsencStereoBandsConfig,
    /// `nEnvelopes`
    pub n_envelopes: PsencNenvConfig,
    /// `iidQuantErrorThreshold`: quantization threshold to switch between coarse and fine iid
    /// quantization
    pub iid_quant_error_threshold: i32,
}

/// The size of `__staticHybAnaStatesLF[ch]` in values
/// (`2 * HYBRID_FILTER_LENGTH * HYBRID_MAX_QMF_BANDS`).
pub const HYB_ANA_STATES_LF_SIZE: usize = 2 * HYBRID_FILTER_LENGTH * HYBRID_MAX_QMF_BANDS;
/// The size of `__staticHybAnaStatesHF[ch]` in values
/// (`2 * HYBRID_FILTER_DELAY * (64 - HYBRID_MAX_QMF_BANDS)`).
pub const HYB_ANA_STATES_HF_SIZE: usize = 2 * HYBRID_FILTER_DELAY * (64 - HYBRID_MAX_QMF_BANDS);

/// `T_PARAMETRIC_STEREO` (`HANDLE_PARAMETRIC_STEREO` is `&mut ParametricStereo`)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParametricStereo {
    /// `hPsEncode` (`None` for `NULL`)
    pub h_ps_encode: Option<Box<PsEncode>>,
    /// `psOut`
    pub ps_out: [PsOut; 2],

    /// `__staticHybridData`
    pub static_hybrid_data:
        Box<[[[[FixpDbl; MAX_HYBRID_BANDS]; 2]; MAX_PS_CHANNELS]; HYBRID_READ_OFFSET]>,
    /// `qmfDelayLines`
    pub qmf_delay_lines: Box<[[[FixpDbl; 64]; 32 >> 1]; 2]>,
    /// `qmfDelayScale`
    pub qmf_delay_scale: i32,

    /// `psDelay`
    pub ps_delay: i32,
    /// `maxEnvelopes`
    pub max_envelopes: u32,
    /// `dynBandScale`
    pub dyn_band_scale: [u8; PS_MAX_BANDS],
    /// `maxBandValue`
    pub max_band_value: [FixpDbl; PS_MAX_BANDS],
    /// `dmxScale`
    pub dmx_scale: i8,
    /// `initPS`
    pub init_ps: i32,
    /// `noQmfSlots`
    pub no_qmf_slots: i32,
    /// `noQmfBands`
    pub no_qmf_bands: i32,

    /// `fdkHybAnaFilter` (their memory is `__staticHybAnaStatesLF[ch]` and
    /// `__staticHybAnaStatesHF[ch]`)
    pub fdk_hyb_ana_filter: [FdkAnaHybFilter; MAX_PS_CHANNELS],
    /// `fdkHybSynFilter`
    pub fdk_hyb_syn_filter: FdkSynHybFilter,
}

/// `T_PSENC_CONFIG` (`HANDLE_PSENC_CONFIG` is `&mut PsencConfig`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PsencConfig {
    /// `frameSize`
    pub frame_size: i32,
    /// `qmfFilterMode`
    pub qmf_filter_mode: i32,
    /// `sbrPsDelay`
    pub sbr_ps_delay: i32,
    /// `nStereoBands`
    pub n_stereo_bands: PsencStereoBandsConfig,
    /// `maxEnvelopes`
    pub max_envelopes: PsencNenvConfig,
    /// `iidQuantErrorThreshold`
    pub iid_quant_error_threshold: FixpDbl,
}

/// The offset in the dynamic RAM of the SBR encoder of `pHybridData[HYBRID_READ_OFFSET +
/// row][ch][ri]` (`GetRam_Sbr_envRBuffer(ch) + row * MAX_HYBRID_BANDS` for `ri == 0`,
/// `GetRam_Sbr_envIBuffer(ch) + ...` for `ri == 1`).
#[inline]
pub fn hybrid_dyn_offset(row: usize, ch: usize, ri: usize) -> usize {
    ch * (ENV_R_BUFF_DBL + ENV_I_BUFF_DBL) + ri * ENV_R_BUFF_DBL + row * MAX_HYBRID_BANDS
}

/// `PSEnc_Create(phParametricStereo)`: a new parametric stereo encoder (cleared), with the
/// hybrid analysis filter banks opened on their state memory.
pub fn ps_enc_create() -> Box<ParametricStereo> {
    let mut h = Box::new(ParametricStereo {
        h_ps_encode: Some(fdk_sbr_enc_create_ps_encode()),
        ps_out: [PsOut::default(); 2],
        static_hybrid_data: Box::new(
            [[[[0; MAX_HYBRID_BANDS]; 2]; MAX_PS_CHANNELS]; HYBRID_READ_OFFSET],
        ),
        qmf_delay_lines: Box::new([[[0; 64]; 32 >> 1]; 2]),
        qmf_delay_scale: 0,
        ps_delay: 0,
        max_envelopes: 0,
        dyn_band_scale: [0; PS_MAX_BANDS],
        max_band_value: [0; PS_MAX_BANDS],
        dmx_scale: 0,
        init_ps: 0,
        no_qmf_slots: 0,
        no_qmf_bands: 0,
        fdk_hyb_ana_filter: Default::default(),
        fdk_hyb_syn_filter: FdkSynHybFilter::default(),
    });

    for f in h.fdk_hyb_ana_filter.iter_mut() {
        fdk_hybrid_analysis_open(
            f,
            vec![0; HYB_ANA_STATES_LF_SIZE],
            (HYB_ANA_STATES_LF_SIZE * 4) as u32,
            vec![0; HYB_ANA_STATES_HF_SIZE],
            (HYB_ANA_STATES_HF_SIZE * 4) as u32,
        );
    }
    h
}

/// `PSEnc_Init(hParametricStereo, hPsEncConfig, noQmfSlots, noQmfBands, dynamic_RAM)`: the
/// pointers into the dynamic RAM are implicit in the port (see [`hybrid_dyn_offset`]).
pub fn ps_enc_init(
    h_parametric_stereo: Option<&mut ParametricStereo>,
    h_ps_enc_config: Option<&mut PsencConfig>,
    no_qmf_slots: i32,
    no_qmf_bands: i32,
) -> FdkPsencError {
    let (Some(h), Some(cfg)) = (h_parametric_stereo, h_ps_enc_config) else {
        return PSENC_INVALID_HANDLE;
    };

    h.init_ps = 1;
    h.no_qmf_slots = no_qmf_slots;
    h.no_qmf_bands = no_qmf_bands;

    // clear delay lines
    *h.qmf_delay_lines = [[[0; 64]; 32 >> 1]; 2];

    h.qmf_delay_scale = FRACT_BITS - 1;

    // create configuration for hybrid filter bank
    for ch in 0..MAX_PS_CHANNELS {
        fdk_hybrid_analysis_init(&mut h.fdk_hyb_ana_filter[ch], THREE_TO_TEN, 64, 64, 1);
    } // ch

    fdk_hybrid_synthesis_init(&mut h.fdk_hyb_syn_filter, THREE_TO_TEN, 64, 64);

    // determine average delay
    h.ps_delay = HYBRID_FILTER_DELAY as i32 * h.no_qmf_bands;

    if cfg.max_envelopes < PSENC_NENV_1 || cfg.max_envelopes > PSENC_NENV_MAX {
        cfg.max_envelopes = PSENC_NENV_DEFAULT;
    }
    h.max_envelopes = cfg.max_envelopes as u32;

    let error = fdk_sbr_enc_init_ps_encode(
        h.h_ps_encode.as_deref_mut(),
        cfg.n_stereo_bands,
        cfg.iid_quant_error_threshold,
    );
    if error != PSENC_OK {
        return error;
    }

    // clear static hybrid buffer
    *h.static_hybrid_data = [[[[0; MAX_HYBRID_BANDS]; 2]; MAX_PS_CHANNELS]; HYBRID_READ_OFFSET];

    // clear bs buffer
    h.ps_out = [PsOut::default(); 2];

    h.ps_out[0].enable_ps_header = 1; // write ps header in first frame

    // clear scaling buffer
    h.dyn_band_scale = [0; PS_MAX_BANDS];
    h.max_band_value = [0; PS_MAX_BANDS];

    PSENC_OK
}

/// `PSEnc_Destroy(phParametricStereo)`
pub fn ps_enc_destroy(ph_parametric_stereo: &mut Option<Box<ParametricStereo>>) -> FdkPsencError {
    *ph_parametric_stereo = None;
    PSENC_OK
}

/// `ExtractPSParameters(hParametricStereo, sendHeader, hybridData)`
fn extract_ps_parameters(
    h: &mut ParametricStereo,
    send_header: i32,
    dyn_ram: &[FixpDbl],
) -> FdkPsencError {
    // call ps encode function
    if h.init_ps != 0 {
        h.ps_out[1] = h.ps_out[0];
    }
    h.ps_out[0] = h.ps_out[1];

    let view = HybridView { stat: &h.static_hybrid_data, dyn_ram, base: 0 };
    let Some(enc) = h.h_ps_encode.as_deref_mut() else {
        return PSENC_INVALID_HANDLE;
    };
    let error = fdk_sbr_enc_ps_encode(
        enc,
        &mut h.ps_out[1],
        &h.dyn_band_scale,
        h.max_envelopes,
        &view,
        h.no_qmf_slots,
        send_header,
    );
    if error != PSENC_OK {
        return error;
    }

    if h.init_ps != 0 {
        h.ps_out[0] = h.ps_out[1];
        h.init_ps = 0;
    }

    error
}

/// The hybrid bands of one slot of `DownmixPSQmfData()`: `rows` are the left real, left
/// imaginary, right real and right imaginary hybrid samples, `out_re`/`out_im` the downmix.
/// On AArch64 (`aarch64` variant) the NEON kernel `ps_main/neon.rs` computes groups of four
/// bands, on x86 with AVX2 (`x86` and `x86_32` variants) `ps_main/x86.rs` groups of eight; other
/// variants and targets use `downmix_band` for every band.
#[inline]
pub fn downmix_bands(
    rows: [&[FixpDbl; 71]; 4],
    out_re: &mut [FixpDbl; 71],
    out_im: &mut [FixpDbl; 71],
    dyn_qmf_scale: i32,
) {
    #[cfg(target_arch = "aarch64")]
    if matches!(fdk_aac_rs_fdk::ARCH, fdk_aac_rs_fdk::Arch::Aarch64) {
        return neon::downmix_bands(rows, out_re, out_im, dyn_qmf_scale);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if matches!(fdk_aac_rs_fdk::ARCH, fdk_aac_rs_fdk::Arch::X86 | fdk_aac_rs_fdk::Arch::X86_32)
        && crate::simd::avx2()
    {
        // SAFETY: the CPU supports AVX2.
        return unsafe { x86::downmix_bands(rows, out_re, out_im, dyn_qmf_scale) };
    }
    downmix_bands_straight(rows, out_re, out_im, dyn_qmf_scale);
}

/// [`downmix_bands`] with [`downmix_band`] for every band (straight port).
#[doc(hidden)]
pub fn downmix_bands_straight(
    rows: [&[FixpDbl; 71]; 4],
    out_re: &mut [FixpDbl; 71],
    out_im: &mut [FixpDbl; 71],
    dyn_qmf_scale: i32,
) {
    let [lr, li, rr, ri] = rows;
    for k in 0..71usize {
        (out_re[k], out_im[k]) = downmix_band(lr[k], li[k], rr[k], ri[k], dyn_qmf_scale);
    }
}

/// One hybrid band of `DownmixPSQmfData()` (the body of the loop over `k`): the downmix of the
/// left and right samples `(tmpLeftReal, tmpLeftImag, tmpRightReal, tmpRightImag)`.
#[inline(always)]
fn downmix_band(
    mut tmp_left_real: FixpDbl,
    mut tmp_left_imag: FixpDbl,
    mut tmp_right_real: FixpDbl,
    mut tmp_right_imag: FixpDbl,
    dyn_qmf_scale: i32,
) -> (FixpDbl, FixpDbl) {
    let max_stereo_scale_factor = MAXVAL_DBL; // 2.f/2.f
                                              // fixMax() of the four fixp_abs() values as compiled: an unsigned maximum
    let mut sc = fix_max(
        0,
        cnt_leading_zeros(umax_abs(
            umax_abs(fixp_abs(tmp_left_real), fixp_abs(tmp_left_imag)),
            umax_abs(fixp_abs(tmp_right_real), fixp_abs(tmp_right_imag)),
        )) - 2,
    );

    tmp_left_real = tmp_left_real.wrapping_shl(sc as u32);
    tmp_left_imag = tmp_left_imag.wrapping_shl(sc as u32);
    tmp_right_real = tmp_right_real.wrapping_shl(sc as u32);
    tmp_right_imag = tmp_right_imag.wrapping_shl(sc as u32);
    let dyn_scale = fix_min(sc - dyn_qmf_scale, DFRACT_BITS - 1);

    // calc stereo scale factor to avoid loss of energy in bands
    // stereo scale factor = min(2.0f, sqrt( (abs(l(k, n)^2 + abs(r(k, n)^2
    // )))/(0.5f*abs(l(k, n) + r(k, n))) ))
    let mut stereo_scale_factor = f_pow2_div2(tmp_left_real)
        .wrapping_add(f_pow2_div2(tmp_left_imag))
        .wrapping_add(f_pow2_div2(tmp_right_real))
        .wrapping_add(f_pow2_div2(tmp_right_imag));

    // might be that tmpScaleFactor becomes negative, so fabs(.)
    let mut tmp_scale_factor = fixp_abs(
        stereo_scale_factor
            .wrapping_add(f_mult(tmp_left_real, tmp_right_real))
            .wrapping_add(f_mult(tmp_left_imag, tmp_right_imag)),
    );

    // min(2.0f, sqrt(stereoScaleFactor/(0.5f*tmpScaleFactor)))
    if (stereo_scale_factor >> 1) < f_mult(max_stereo_scale_factor, tmp_scale_factor) {
        let sc_num = count_leading_bits(stereo_scale_factor);
        // CountLeadingBits(fixp_abs(...)) as compiled: no sign fold
        let sc_denum = clb_abs(tmp_scale_factor);
        sc = -(sc_num - sc_denum);

        tmp_scale_factor = schur_div_fast(
            stereo_scale_factor.wrapping_shl(sc_num as u32) >> 1,
            tmp_scale_factor.wrapping_shl(sc_denum as u32),
            16,
        );

        // prevent odd scaling for next sqrt calculation
        if sc & 0x1 != 0 {
            sc += 1;
            tmp_scale_factor >>= 1;
        }

        stereo_scale_factor = sqrt_fixp(tmp_scale_factor);
        // C: stereoScaleFactor <<= (sc >> 1)
        stereo_scale_factor = stereo_scale_factor.wrapping_shl((sc >> 1) as u32);
    } else {
        stereo_scale_factor = max_stereo_scale_factor;
    }

    // write data to hybrid output
    (
        f_mult_div2(stereo_scale_factor, tmp_left_real.wrapping_add(tmp_right_real))
            .wrapping_shr(dyn_scale as u32),
        f_mult_div2(stereo_scale_factor, tmp_left_imag.wrapping_add(tmp_right_imag))
            .wrapping_shr(dyn_scale as u32),
    )
}

/// `DownmixPSQmfData(hParametricStereo, sbrSynthQmf, mixRealQmfData, mixImagQmfData,
/// downsampledOutSignal, downsampledOutSignalBufSize, hybridData, noQmfSlots, psQmfScale,
/// qmfScale)`: the downmix and hybrid and QMF synthesis. The mixed QMF data rows are
/// `dyn_ram[mix_real + n * 64]` and `dyn_ram[mix_imag + n * 64]`; the hybrid data is
/// `pHybridData[HYBRID_READ_OFFSET..]`; the downsampled time signal goes to
/// `pcm[out_pos..]`.
///
/// `fAbs()` negates with signed overflow, so both reference builds take its results as not
/// negative: the maximum of the four absolute values (`sc`) is an unsigned maximum (arm64
/// `abs` + `umaxv`, x86 `cmova`), and `CountLeadingBits(tmpScaleFactor)` drops the sign fold
/// (`clz`/`bsr` of the absolute value itself). Both differ from the signed code only for
/// `MINVAL_DBL` (see `umax_abs`, `clb_abs`).
#[allow(clippy::too_many_arguments)]
pub fn downmix_ps_qmf_data(
    h: &mut ParametricStereo,
    sbr_synth_qmf: &QmfFilterBank,
    syn_states: &mut [FixpQss],
    dyn_ram: &mut [FixpDbl],
    mix_real: usize,
    mix_imag: usize,
    pcm: &mut [IntPcm],
    out_pos: usize,
    no_qmf_slots: i32,
    ps_qmf_scale: &[i32; MAX_PS_CHANNELS],
    qmf_scale: &mut i8,
) -> FdkPsencError {
    let mut p_work_buffer = [0 as FixpDbl; 2 * 64];

    // define scalings
    let dyn_qmf_scale = fix_max(0, h.dmx_scale as i32 - 1); // scale one bit more for addition of left and right
    let downmix_scale = ps_qmf_scale[0] - dyn_qmf_scale;

    let no_ch = sbr_synth_qmf.no_channels.max(0) as usize;

    for n in 0..no_qmf_slots.max(0) as usize {
        let mut tmp_hybrid = [[0 as FixpDbl; MAX_HYBRID_BANDS]; 2];

        // hybridData[n][ch][ri][k]: pHybridData[HYBRID_READ_OFFSET + n]
        let row = |ch: usize, ri: usize| -> &[FixpDbl; 71] {
            let o = hybrid_dyn_offset(n, ch, ri);
            dyn_ram[o..o + 71].try_into().unwrap()
        };
        let [out_re, out_im] = &mut tmp_hybrid;
        downmix_bands(
            [row(0, 0), row(0, 1), row(1, 0), row(1, 1)],
            (&mut out_re[..71]).try_into().unwrap(),
            (&mut out_im[..71]).try_into().unwrap(),
            dyn_qmf_scale,
        );

        {
            let (lo, hi) = dyn_ram.split_at_mut(mix_imag);
            let re = &mut lo[mix_real + n * 64..mix_real + n * 64 + 64];
            let im = &mut hi[n * 64..n * 64 + 64];
            fdk_hybrid_synthesis_apply(
                &h.fdk_hyb_syn_filter,
                &tmp_hybrid[0],
                &tmp_hybrid[1],
                re,
                im,
            );
        }

        let (lo, hi) = dyn_ram.split_at(mix_imag);
        let re = &lo[mix_real + n * 64..mix_real + n * 64 + 64];
        let im = &hi[n * 64..n * 64 + 64];
        let out = out_pos + n * no_ch;
        if out < pcm.len() {
            qmf_synthesis_filtering_slot::<IntPcm>(
                sbr_synth_qmf,
                syn_states,
                re,
                im,
                downmix_scale - 7,
                downmix_scale - 7,
                &mut pcm[out..],
                1,
                &mut p_work_buffer,
            );
        }
    } // slots

    *qmf_scale = (-downmix_scale + 7) as i8;

    {
        let no_qmf_slots2 = (h.no_qmf_slots >> 1).max(0) as usize;
        let no_qmf_bands = (h.no_qmf_bands.max(0) as usize).min(64);

        for i in 0..no_qmf_slots2 {
            let tmp0 = h.qmf_delay_lines[0][i];
            let tmp1 = h.qmf_delay_lines[1][i];

            let r_hi = mix_real + (i + no_qmf_slots2) * 64;
            let i_hi = mix_imag + (i + no_qmf_slots2) * 64;
            let r_lo = mix_real + i * 64;
            let i_lo = mix_imag + i * 64;

            h.qmf_delay_lines[0][i][..no_qmf_bands]
                .copy_from_slice(&dyn_ram[r_hi..r_hi + no_qmf_bands]);
            h.qmf_delay_lines[1][i][..no_qmf_bands]
                .copy_from_slice(&dyn_ram[i_hi..i_hi + no_qmf_bands]);

            dyn_ram.copy_within(r_lo..r_lo + no_qmf_bands, r_hi);
            dyn_ram.copy_within(i_lo..i_lo + no_qmf_bands, i_hi);

            dyn_ram[r_lo..r_lo + no_qmf_bands].copy_from_slice(&tmp0[..no_qmf_bands]);
            dyn_ram[i_lo..i_lo + no_qmf_bands].copy_from_slice(&tmp1[..no_qmf_bands]);
        }

        let scale;
        let slot_offset;
        if h.qmf_delay_scale > *qmf_scale as i32 {
            scale = h.qmf_delay_scale - *qmf_scale as i32;
            slot_offset = 0;
        } else {
            scale = *qmf_scale as i32 - h.qmf_delay_scale;
            slot_offset = no_qmf_slots2;
        }

        for i in 0..no_qmf_slots2 {
            for j in 0..no_qmf_bands {
                // C: >>= scale (the scales of the library keep it below 32)
                let r = mix_real + (i + slot_offset) * 64 + j;
                dyn_ram[r] = dyn_ram[r].wrapping_shr(scale as u32);
                let im = mix_imag + (i + slot_offset) * 64 + j;
                dyn_ram[im] = dyn_ram[im].wrapping_shr(scale as u32);
            }
        }

        let scale = *qmf_scale as i32;
        *qmf_scale = f_min(*qmf_scale as i32, h.qmf_delay_scale) as i8;
        h.qmf_delay_scale = scale;
    }

    PSENC_OK
}

/// `FDKsbrEnc_PSEnc_WritePSData(hParametricStereo, hBitstream)`: writes ps_data() (or counts
/// its bits for `None`) and returns the number of bits.
pub fn fdk_sbr_enc_ps_enc_write_ps_data(
    h_parametric_stereo: Option<&ParametricStereo>,
    h_bitstream: Option<&mut fdk_aac_rs_fdk::bitstream::FdkBitStream>,
) -> i32 {
    match h_parametric_stereo {
        Some(h) => fdk_sbr_enc_write_ps_bitstream(Some(&h.ps_out[0]), h_bitstream),
        None => 0,
    }
}

/// The QMF analysis filter banks of the two channels (`QMF_FILTER_BANK **hQmfAnalysis`) with
/// their filter states.
pub struct PsQmfAnalysis<'a> {
    /// `hQmfAnalysis[ch]`
    pub bank: [&'a QmfFilterBank; MAX_PS_CHANNELS],
    /// `hQmfAnalysis[ch]->FilterStates`
    pub states: [&'a mut [FixpSgl]; MAX_PS_CHANNELS],
}

/// `FDKsbrEnc_PSEnc_ParametricStereoProcessing(hParametricStereo, samples, timeInStride,
/// hQmfAnalysis, downmixedRealQmfData, downmixedImagQmfData, downsampledOutSignal,
/// sbrSynthQmf, qmfScale, sendHeader)`: stereo QMF and hybrid analysis, PS parameter
/// extraction, downmix and hybrid synthesis.
///
/// `samples[ch]` and `downsampled_out` are positions in `pcm` (the encoder's input buffer);
/// the downmixed QMF data rows are `dyn_ram[downmixed_real + n * 64]` and
/// `dyn_ram[downmixed_imag + n * 64]`.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_ps_enc_parametric_stereo_processing(
    h: &mut ParametricStereo,
    pcm: &mut [IntPcm],
    samples: [usize; 2],
    _samples_buf_size: u32,
    h_qmf_analysis: &mut PsQmfAnalysis,
    dyn_ram: &mut [FixpDbl],
    downmixed_real: usize,
    downmixed_imag: usize,
    downsampled_out: usize,
    sbr_synth_qmf: &QmfFilterBank,
    syn_states: &mut [FixpQss],
    qmf_scale: &mut i8,
    send_header: i32,
) -> FdkPsencError {
    let mut ps_qmf_scale = [0i32; MAX_PS_CHANNELS];
    let mut p_work_buffer = [0 as FixpDbl; 4 * 64];

    for ps_ch in 0..MAX_PS_CHANNELS {
        let bank = h_qmf_analysis.bank[ps_ch];
        let no_ch = bank.no_channels.max(0) as usize;
        for i in 0..bank.no_col.max(0) as usize {
            let (w01, w23) = p_work_buffer.split_at_mut(2 * 64);
            let (qmf_real, qmf_imag) = w23.split_at_mut(64);
            let pos = samples[ps_ch] + i * no_ch;
            if pos <= pcm.len() {
                qmf_analysis_filtering_slot::<IntPcm>(
                    bank,
                    h_qmf_analysis.states[ps_ch],
                    qmf_real,
                    qmf_imag,
                    &pcm[pos..],
                    1,
                    w01,
                );
            }

            let (lo, hi) = dyn_ram.split_at_mut(hybrid_dyn_offset(i, ps_ch, 1));
            let re = &mut lo
                [hybrid_dyn_offset(i, ps_ch, 0)..hybrid_dyn_offset(i, ps_ch, 0) + MAX_HYBRID_BANDS];
            let im = &mut hi[..MAX_HYBRID_BANDS];
            fdk_hybrid_analysis_apply(&mut h.fdk_hyb_ana_filter[ps_ch], qmf_real, qmf_imag, re, im);
        } // no_col loop  i
        ps_qmf_scale[ps_ch] = bank.out_scalefactor;
    } // for psCh

    // find best scaling in new QMF and Hybrid data
    {
        let view = HybridView { stat: &h.static_hybrid_data, dyn_ram, base: HYBRID_READ_OFFSET };
        let Some(enc) = h.h_ps_encode.as_deref() else {
            return PSENC_INVALID_HANDLE;
        };
        let mut dyn_band_scale = h.dyn_band_scale;
        let mut max_band_value = h.max_band_value;
        let mut dmx_scale = h.dmx_scale;
        ps_find_best_scaling(
            enc,
            h.no_qmf_slots,
            &view,
            &mut dyn_band_scale,
            &mut max_band_value,
            &mut dmx_scale,
        );
        h.dyn_band_scale = dyn_band_scale;
        h.max_band_value = max_band_value;
        h.dmx_scale = dmx_scale;
    }

    // extract the ps parameters
    let error = extract_ps_parameters(h, send_header, dyn_ram);
    if error != PSENC_OK {
        return error;
    }

    // save hybrid date for next frame
    for i in 0..HYBRID_READ_OFFSET {
        let row = h.no_qmf_slots.max(0) as usize + i;
        for ch in 0..MAX_PS_CHANNELS {
            for ri in 0..2 {
                // pHybridData[noQmfSlots + i] is in the dynamic RAM (noQmfSlots >= 10)
                let v: [FixpDbl; MAX_HYBRID_BANDS] = if row < HYBRID_READ_OFFSET {
                    h.static_hybrid_data[row][ch][ri]
                } else {
                    let off = hybrid_dyn_offset(row - HYBRID_READ_OFFSET, ch, ri);
                    let mut v = [0; MAX_HYBRID_BANDS];
                    v.copy_from_slice(&dyn_ram[off..off + MAX_HYBRID_BANDS]);
                    v
                };
                h.static_hybrid_data[i][ch][ri] = v;
            }
        }
    }

    // downmix and hybrid synthesis
    downmix_ps_qmf_data(
        h,
        sbr_synth_qmf,
        syn_states,
        dyn_ram,
        downmixed_real,
        downmixed_imag,
        pcm,
        downsampled_out,
        h.no_qmf_slots,
        &ps_qmf_scale,
        qmf_scale,
    )
}

/// `psFindBestScaling(hParametricStereo, hybridData, dynBandScale, maxBandValue, dmxScale)`
/// (file-static).
///
/// The four `fixMax(tmp, fixp_abs(...))` of the inner loop are compiled as in
/// `findPeakLevels()` of the metadata compressor: `fAbs()` negates with signed
/// overflow, so the compilers take its results as not negative and, after the first (signed)
/// maximum of an iteration, `tmp` as well; the second to fourth maxima are unsigned (arm64
/// `csel hi`, x86 `ja`). They differ from signed maxima when a value is `MINVAL_DBL`.
pub fn ps_find_best_scaling(
    h_ps_encode: &PsEncode,
    frame_size: i32,
    hybrid_data: &HybridView,
    dyn_band_scale: &mut [u8; PS_MAX_BANDS],
    max_band_value: &mut [FixpDbl; PS_MAX_BANDS],
    dmx_scale: &mut i8,
) {
    let ps_bands = h_ps_encode.ps_enc_mode;
    let n_iid_groups = h_ps_encode.n_qmf_iid_groups + h_ps_encode.n_sub_qmf_iid_groups;

    // group wise scaling
    let mut max_val = [[0 as FixpDbl; PS_MAX_BANDS]; 2];
    let mut max_value: FixpDbl = 0;

    // start with hybrid data
    for group in 0..n_iid_groups.max(0) as usize {
        // Translate group to bin
        let mut bin = h_ps_encode.subband2parameter_index[group] as usize;

        // Translate from 20 bins to 10 bins
        if h_ps_encode.ps_enc_mode == PS_BANDS_COARSE {
            bin >>= 1;
        }

        // QMF downmix scaling
        for col in 0..frame_size.max(0) {
            let section = if col < frame_size - HYBRID_READ_OFFSET as i32 { 0 } else { 1 };
            let col = col as usize;
            let mut tmp = max_val[section][bin];
            let lr = hybrid_data.row(col, 0, 0);
            let li = hybrid_data.row(col, 0, 1);
            let rr = hybrid_data.row(col, 1, 0);
            let ri = hybrid_data.row(col, 1, 1);
            for i in h_ps_encode.iid_group_borders[group]..h_ps_encode.iid_group_borders[group + 1]
            {
                let i = i as usize;
                tmp = fix_max(tmp, fixp_abs(lr[i]));
                tmp = umax_abs(tmp, fixp_abs(li[i]));
                tmp = umax_abs(tmp, fixp_abs(rr[i]));
                tmp = umax_abs(tmp, fixp_abs(ri[i]));
            }
            max_val[section][bin] = tmp;
        }
    } // nIidGroups

    // convert maxSpec to maxScaling, find scaling space
    for band in 0..ps_bands.max(0) as usize {
        dyn_band_scale[band] =
            count_leading_bits(fix_max(max_val[0][band], max_band_value[band])) as u8;
        max_value = fix_max(max_value, fix_max(max_val[0][band], max_val[1][band]));
        max_band_value[band] = fix_max(max_val[0][band], max_val[1][band]);
    }

    // calculate maximal scaling for QMF downmix
    *dmx_scale = fix_min(DFRACT_BITS, count_leading_bits(max_value)) as i8;
}
