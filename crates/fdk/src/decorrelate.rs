// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_decorrelate.h, libFDK/src/FDK_decorrelate.cpp.

//! Port of `FDK_decorrelate.h`: the decorrelator of MPEG Surround (`libSACdec`) and parametric
//! stereo (`libSBRdec`), with its ducker.
//!
//! Both variants compile the same code (`FUNCTION_DecorrFilterApplyREAL`,
//! `FUNCTION_DecorrFilterApplyCPLX_PS` and `FUNCTION_DuckerCalcEnergy` are not defined); they
//! differ in the primitives (`fMult`, `cplxMultDiv2`, `sqrtFixp`, `invSqrtNorm2`).
//!
//! Memory: `FDKdecorrelateOpen()` takes the state/delay buffer that the caller allocated
//! (`2 * (825 + 373)` values in the library); the port takes ownership of it
//! ([`DecorrDec::state_buffer_cplx`]). The pointers of the C++ struct into this buffer are
//! offsets ([`DecorrDec::delay_buffer_cplx`], [`DecorrFilterInstance::state_cplx`],
//! [`DecorrFilterInstance::delay_buffer_cplx`]), the pointers to constant tables are references.
//! The buffer keeps its content across `FDKdecorrelateInit()` calls without `initStatesFlag`, so
//! a new configuration reads the stale content of the previous one, as in C++.
//!
//! Deviations where the reference has no defined result (PORTING.md, "Deviations from the
//! reference"):
//!
//! - Accesses outside of the buffer read 0 and drop writes.
//! - `FDKdecorrelateApply()` on an instance without an initialized ducker returns 1.
//! - `FDKdecorrelateInit()` without a buffer returns 1.
//!
//! The data slices hold at least `nrHybBands` (in the library: 71) values.

mod tables;
pub use tables::*;

use crate::archdef::FixpStp;
use crate::common_fix::*;
use crate::cplx_mul::cplx_mult_div2;
use crate::fixpoint_math::{f_add_saturate, inv_sqrt_norm2, sqrt_fixp};
use crate::scale::{get_scalefactor, saturate_left_shift, scale_value};

/// `FIXP_MPS`
pub type FixpMps = FixpDbl;
/// `FIXP_DECORR`
pub type FixpDecorr = FixpSgl;

/// `FX_DECORR2FX_DBL(x)`
#[inline(always)]
pub const fn fx_decorr2fx_dbl(x: FixpDecorr) -> FixpDbl {
    fx_sgl2fx_dbl(x)
}

/// `FX_DBL2FX_DECORR(x)`
#[inline(always)]
pub const fn fx_dbl2fx_decorr(x: FixpDbl) -> FixpDecorr {
    fx_dbl2fx_sgl(x)
}

/// `DECORR(a)`: `FX_DBL2FXCONST_SGL(a)`
#[inline(always)]
pub const fn decorr(a: FixpDbl) -> FixpDecorr {
    fx_dbl2fxconst_sgl(a)
}

/// `FL2FXCONST_DECORR(val)`
#[inline(always)]
pub const fn fl2fxconst_decorr(val: f64) -> FixpDecorr {
    fl2fxconst_sgl(val)
}

/// `FDK_DECORR_TYPE`
pub type FdkDecorrType = i32;
/// `DECORR_MPS`: decorrelator type used by MPS LP/HQ.
pub const DECORR_MPS: FdkDecorrType = 0;
/// `DECORR_PS`: decorrelator type used by HEAACv2 and MPS LP.
pub const DECORR_PS: FdkDecorrType = 1;
/// `DECORR_USAC`: decorrelator type used by USAC.
pub const DECORR_USAC: FdkDecorrType = 2;
/// `DECORR_LD`: decorrelator type used by MPS Low Delay.
pub const DECORR_LD: FdkDecorrType = 3;

/// `FDK_DUCKER_TYPE`
pub type FdkDuckerType = i32;
/// `DUCKER_AUTOMATIC`: `FDKdecorrelateInit()` chooses the ducker type.
pub const DUCKER_AUTOMATIC: FdkDuckerType = 0;
/// `DUCKER_MPS`
pub const DUCKER_MPS: FdkDuckerType = 1;
/// `DUCKER_PS`
pub const DUCKER_PS: FdkDuckerType = 2;

/// `REVBAND_FILT_TYPE`
pub type RevbandFiltType = i32;
/// `NOT_EXIST`: non-existing reverb band (number of bands = 0).
pub const NOT_EXIST: RevbandFiltType = 0;
/// `DELAY`: delay elements only, no allpass filters.
pub const DELAY: RevbandFiltType = 1;
/// `COMMON_REAL`: real filter coefficients, common within one reverb band.
pub const COMMON_REAL: RevbandFiltType = 2;
/// `COMMON_CPLX`: complex filter coefficients, common within one reverb band.
pub const COMMON_CPLX: RevbandFiltType = 3;
/// `INDEP_CPLX`: complex filter coefficients, independent for each hybrid band.
pub const INDEP_CPLX: RevbandFiltType = 4;
/// `INDEP_CPLX_PS`: PS optimized implementation of `INDEP_CPLX`.
pub const INDEP_CPLX_PS: RevbandFiltType = 5;

/// `(28)`: maximum number of parameter bands.
pub const MAX_PARAMETER_BANDS: usize = 28;
/// `(71)`: maximum number of hybrid bands.
pub const MAX_HYBRID_BANDS: usize = 71;
/// `(4)`: number of reverb bands.
pub const NUM_REVERB_BANDS: usize = 4;
/// `(3)`: number of allpass stages of the PS decorrelator.
pub const NUM_PS_STAGES: usize = 3;
/// `(2 * ((825) + (373)))`: minimum length of the buffer of `FDKdecorrelateOpen()`.
pub const DECORR_BUFFER_LEN: i32 = 2 * (825 + 373);

const DECORR_FILTER_ORDER_PS: i32 = 12;
const MAX_DECORR_SEED_LD: i32 = 4;

const DUCKER_MAX_NRG_SCALE: i32 = 24;
const DUCKER_HEADROOM_BITS: i32 = 2;

const FILTER_SF: i32 = 2;

/// `FIXP_DUCK_GAIN`
type FixpDuckGain = FixpSgl;

/// `DUCK_ALPHA_FDK`: `FL2FXCONST_SGL(0.8f)`
const DUCK_ALPHA_FDK: FixpDuckGain = fl2fxconst_sgl(0.8f32 as f64);
/// `DUCK_ONE_MINUS_ALPHA_X4_FDK`: `FL2FXCONST_SGL(4.0f * (1.0f - 0.8f))` (in `float`)
const DUCK_ONE_MINUS_ALPHA_X4_FDK: FixpDuckGain =
    fl2fxconst_sgl((4.0f32 * (1.0f32 - 0.8f32)) as f64);
/// `DUCK_GAMMA_FDK`: `FL2FXCONST_SGL(1.5f / 2)`
const DUCK_GAMMA_FDK: FixpDuckGain = fl2fxconst_sgl((1.5f32 / 2.0) as f64);
/// `PS_DUCK_PEAK_DECAY_FACTOR_FDK`: `FL2FXCONST_SGL(0.765928338364649f)`
const PS_DUCK_PEAK_DECAY_FACTOR_FDK: FixpDuckGain = fl2fxconst_sgl(0.765928338364649f32 as f64);
/// `PS_DUCK_FILTER_COEFF_FDK`: `FL2FXCONST_SGL(0.25f)`
const PS_DUCK_FILTER_COEFF_FDK: FixpDuckGain = fl2fxconst_sgl(0.25f32 as f64);
/// `ABS_THR`: `1e-9f * 32768 * 32768` (in `float`)
const ABS_THR: f32 = 1e-9f32 * 32768.0 * 32768.0;
/// `FL2FXCONST_DBL(ABS_THR / (32768.0f * 32768.0f * 128.0f * 1.5f))` (0)
const ABS_THR_PS: FixpDbl =
    fl2fxconst_dbl((ABS_THR / (32768.0f32 * 32768.0f32 * 128.0f32 * 1.5f32)) as f64);

/// The offset of a `NULL` pointer into the buffer: every access through it is outside of the
/// buffer (see the module documentation).
const NULL_OFFSET: isize = isize::MIN / 4;

/// `DUCKER_INSTANCE`
#[derive(Clone, Debug)]
pub struct DuckerInstance {
    pub hybrid_bands: i32,
    pub parameter_bands: i32,
    pub partially_complex: i32,
    pub ducker_type: FdkDuckerType,
    /// `qs_next`: `&mapProcBands2HybBands[1]`
    pub qs_next: Option<&'static [u8]>,
    pub map_proc_bands2_hyb_bands: Option<&'static [u8]>,
    pub map_hyb_bands2_proc_bands: Option<&'static [u8]>,
    /// Interleaved `SmoothDirectNrg[]` and `SmoothReverbNrg[]`, non-interleaved
    /// `SmoothDirectNrg[]` in case of parametric stereo.
    pub smooth_dir_rev_nrg: [FixpMps; 2 * MAX_PARAMETER_BANDS],
    // parametric stereo
    pub peak_decay: [FixpMps; MAX_PARAMETER_BANDS],
    pub peak_diff: [FixpMps; MAX_PARAMETER_BANDS],
    pub max_val_direct_data: FixpDbl,
    pub max_val_reverb_data: FixpDbl,
    pub scale_direct_nrg: i8,
    pub scale_reverb_nrg: i8,
    pub scale_smooth_dir_rev_nrg: i8,
    pub headroom_smooth_dir_rev_nrg: i8,
}

impl Default for DuckerInstance {
    fn default() -> Self {
        Self {
            hybrid_bands: 0,
            parameter_bands: 0,
            partially_complex: 0,
            ducker_type: DUCKER_AUTOMATIC,
            qs_next: None,
            map_proc_bands2_hyb_bands: None,
            map_hyb_bands2_proc_bands: None,
            smooth_dir_rev_nrg: [0; 2 * MAX_PARAMETER_BANDS],
            peak_decay: [0; MAX_PARAMETER_BANDS],
            peak_diff: [0; MAX_PARAMETER_BANDS],
            max_val_direct_data: 0,
            max_val_reverb_data: 0,
            scale_direct_nrg: 0,
            scale_reverb_nrg: 0,
            scale_smooth_dir_rev_nrg: 0,
            headroom_smooth_dir_rev_nrg: 0,
        }
    }
}

/// `DECORR_FILTER_INSTANCE`
#[derive(Clone, Copy, Debug, Default)]
pub struct DecorrFilterInstance {
    /// `stateCplx`: offset into the buffer.
    pub state_cplx: Option<usize>,
    /// `DelayBufferCplx`: offset into the buffer.
    pub delay_buffer_cplx: Option<usize>,
    pub numerator_real: Option<&'static [FixpDecorr]>,
    pub coeffs_packed: Option<&'static [FixpStp; 4]>,
    /// `denominatorReal` (never set by the library).
    pub denominator_real: Option<&'static [FixpDecorr]>,
}

/// `DECORR_DEC`. The C++ struct is allocated (zero-initialized) by the caller; the port starts
/// from [`DecorrDec::default`].
#[derive(Clone, Debug)]
pub struct DecorrDec {
    pub l_state_buffer_cplx: i32,
    /// `stateBufferCplx`: the buffer of `FDKdecorrelateOpen()` (the state buffer is its start).
    pub state_buffer_cplx: Option<Box<[FixpDbl]>>,
    pub l_delay_buffer_cplx: i32,
    /// `delayBufferCplx`: offset into the buffer.
    pub delay_buffer_cplx: Option<usize>,
    pub rev_filt_type: Option<&'static [RevbandFiltType; NUM_REVERB_BANDS]>,
    pub rev_band_offset: Option<&'static [u8; NUM_REVERB_BANDS]>,
    pub rev_delay: Option<&'static [u8; NUM_REVERB_BANDS]>,
    pub rev_filter_order: Option<&'static [i8; NUM_REVERB_BANDS]>,
    pub reverb_band_delay_buffer_index: [i32; NUM_REVERB_BANDS],
    pub state_buffer_offset: [u8; NUM_PS_STAGES],
    pub filter: [DecorrFilterInstance; MAX_HYBRID_BANDS],
    pub ducker: DuckerInstance,
    pub numbins: i32,
    pub partially_complex: i32,
}

impl Default for DecorrDec {
    fn default() -> Self {
        Self {
            l_state_buffer_cplx: 0,
            state_buffer_cplx: None,
            l_delay_buffer_cplx: 0,
            delay_buffer_cplx: None,
            rev_filt_type: None,
            rev_band_offset: None,
            rev_delay: None,
            rev_filter_order: None,
            reverb_band_delay_buffer_index: [0; NUM_REVERB_BANDS],
            state_buffer_offset: [0; NUM_PS_STAGES],
            filter: [DecorrFilterInstance::default(); MAX_HYBRID_BANDS],
            ducker: DuckerInstance::default(),
            numbins: 0,
            partially_complex: 0,
        }
    }
}

/// `HANDLE_DECORR_DEC`
pub type HandleDecorrDec<'a> = &'a mut DecorrDec;

/// A pointer into the buffer as a signed offset (`NULL` is far outside).
#[inline(always)]
fn ptr(offset: Option<usize>) -> isize {
    offset.map_or(NULL_OFFSET, |o| o as isize)
}

/// `buf[i]`, 0 outside of the buffer (see the module documentation).
#[inline(always)]
fn rd(buf: &[FixpDbl], i: isize) -> FixpDbl {
    if i >= 0 {
        buf.get(i as usize).copied().unwrap_or(0)
    } else {
        0
    }
}

/// `buf[i] = v`, dropped outside of the buffer (see the module documentation).
#[inline(always)]
fn wr(buf: &mut [FixpDbl], i: isize, v: FixpDbl) {
    if i >= 0 {
        if let Some(x) = buf.get_mut(i as usize) {
            *x = v;
        }
    }
}

/// `SpatialDecGetProcessingBand(hybridBand, tab)`
#[inline(always)]
fn spatial_dec_get_processing_band(hybrid_band: i32, tab: &[u8]) -> i32 {
    tab[hybrid_band as usize] as i32
}

/// `SpatialDecGetQmfBand(paramBand, tab)`
#[inline(always)]
fn spatial_dec_get_qmf_band(param_band: i32, tab: &[u8]) -> i32 {
    tab[param_band as usize] as i32
}

/// `getAddrDirectSignalMaxVal(self)`: the max value of the direct signal needed for the ducker
/// energy calculation (-1.0: not known, computed from the data).
pub fn get_addr_direct_signal_max_val(self_: &mut DecorrDec) -> &mut FixpDbl {
    &mut self_.ducker.max_val_direct_data
}

/// `DecorrFilterInit(self, pStateBufferCplx, pDelayBufferCplx, offsetStateBuffer,
/// offsetDelayBuffer, decorr_seed, reverb_band, useFractDelay, noSampleDelay, filterOrder,
/// decorrType)`; the buffers are the offsets `p_state_buffer_cplx`, `p_delay_buffer_cplx`.
fn decorr_filter_init(
    self_: &mut DecorrFilterInstance,
    p_state_buffer_cplx: usize,
    p_delay_buffer_cplx: usize,
    offset_state_buffer: &mut i32,
    offset_delay_buffer: &mut i32,
    decorr_seed: i32,
    reverb_band: i32,
    use_fract_delay: i32,
    no_sample_delay: i32,
    filter_order: i32,
    decorr_type: FdkDecorrType,
) -> i32 {
    let error_code = 0;
    match decorr_type {
        DECORR_USAC => {
            if use_fract_delay != 0 {
                return 1;
            } else {
                fdk_assert!(decorr_seed == 0, "FDK_ASSERT(decorr_seed == 0)");
                let s = decorr_seed as usize;
                match reverb_band {
                    0 => self_.numerator_real = Some(&DECORR_NUMERATOR_REAL0_USAC[s][..]),
                    1 => self_.numerator_real = Some(&DECORR_NUMERATOR_REAL1_USAC[s][..]),
                    2 => self_.numerator_real = Some(&DECORR_NUMERATOR_REAL2_USAC[s][..]),
                    3 => self_.numerator_real = Some(&DECORR_NUMERATOR_REAL3_USAC[s][..]),
                    _ => {}
                }
            }
        }
        DECORR_LD => {
            fdk_assert!(
                decorr_seed < MAX_DECORR_SEED_LD,
                "FDK_ASSERT(decorr_seed < MAX_DECORR_SEED_LD)"
            );
            let s = decorr_seed as usize;
            match reverb_band {
                0 => self_.numerator_real = None,
                1 => self_.numerator_real = Some(&DECORR_NUMERATOR_REAL1_LD[s][..]),
                2 => self_.numerator_real = Some(&DECORR_NUMERATOR_REAL2_LD[s][..]),
                3 => self_.numerator_real = Some(&DECORR_NUMERATOR_REAL3_LD[s][..]),
                _ => {}
            }
        }
        _ => return 1,
    }

    self_.state_cplx = Some((p_state_buffer_cplx as i64 + *offset_state_buffer as i64) as usize);
    *offset_state_buffer += 2 * filter_order;
    self_.delay_buffer_cplx =
        Some((p_delay_buffer_cplx as i64 + *offset_delay_buffer as i64) as usize);
    *offset_delay_buffer += 2 * no_sample_delay;

    error_code
}

/// `DecorrFilterInitPS(self, pStateBufferCplx, pDelayBufferCplx, offsetStateBuffer,
/// offsetDelayBuffer, hybridBand, reverbBand, noSampleDelay)`
fn decorr_filter_init_ps(
    self_: &mut DecorrFilterInstance,
    p_state_buffer_cplx: usize,
    p_delay_buffer_cplx: usize,
    offset_state_buffer: &mut i32,
    offset_delay_buffer: &mut i32,
    hybrid_band: i32,
    reverb_band: i32,
    no_sample_delay: i32,
) -> i32 {
    let error_code = 0;

    if reverb_band == 0 {
        self_.coeffs_packed = Some(&DECORR_PS_COEFFS_CPLX[hybrid_band as usize]);

        self_.state_cplx =
            Some((p_state_buffer_cplx as i64 + *offset_state_buffer as i64) as usize);
        *offset_state_buffer += 2 * DECORR_FILTER_ORDER_PS;
    }

    self_.delay_buffer_cplx =
        Some((p_delay_buffer_cplx as i64 + *offset_delay_buffer as i64) as usize);
    *offset_delay_buffer += 2 * no_sample_delay;

    error_code
}

/// `DecorrFilterApplyPASS(filter, dataRealIn, dataImagIn, dataRealOut, dataImagOut, start, stop,
/// reverbBandNoSampleDelay, reverbBandDelayBufferIndex)`: pure delay.
///
/// `data_imag_in == None` (C++: `NULL`) delays the real part only (never used by the library).
/// `buf` is the buffer that the filter offsets refer to. Public for the differential test of the
/// `NULL` case, which `FDKdecorrelateApply()` cannot reach.
///
/// The straight port, kept as the reference of the optimized [`decorr_filter_apply_pass`].
pub fn decorr_filter_apply_pass_straight(
    filter: &[DecorrFilterInstance],
    buf: &mut [FixpDbl],
    data_real_in: &[FixpDbl],
    data_imag_in: Option<&[FixpDbl]>,
    data_real_out: &mut [FixpDbl],
    data_imag_out: &mut [FixpDbl],
    start: i32,
    stop: i32,
    reverb_band_no_sample_delay: i32,
    reverb_band_delay_buffer_index: i32,
) -> i32 {
    let offset = 2 * reverb_band_no_sample_delay as isize;
    let mut p_delay_buffer =
        ptr(filter[start as usize].delay_buffer_cplx) + reverb_band_delay_buffer_index as isize;

    // Memory for the delayline has been allocated in a consecutive order, so we can address from
    // filter to filter with a constant length. Be aware that real and imaginary part of the
    // delayline are stored in interleaved order.
    match data_imag_in {
        None => {
            for i in start as usize..stop.max(start) as usize {
                let tmp = rd(buf, p_delay_buffer);
                wr(buf, p_delay_buffer, data_real_in[i]);
                data_real_out[i] = tmp;
                p_delay_buffer += offset;
            }
        }
        Some(data_imag_in) => {
            if stop - start != 0 {
                // do ... while (--i != 0) with i = stop - start
                let mut i = start as usize;
                let mut n = stop - start;
                loop {
                    let real = data_real_in[i];
                    let imag = data_imag_in[i];
                    let delay_re = rd(buf, p_delay_buffer);
                    let delay_im = rd(buf, p_delay_buffer + 1);
                    wr(buf, p_delay_buffer, real);
                    wr(buf, p_delay_buffer + 1, imag);
                    data_real_out[i] = delay_re;
                    data_imag_out[i] = delay_im;
                    p_delay_buffer += offset;
                    i += 1;
                    n -= 1;
                    if n == 0 {
                        break;
                    }
                }
            }
        }
    }

    0
}

/// `DecorrFilterApplyREAL(filter, dataRealIn, dataImagIn, dataRealOut, dataImagOut, start,
/// stop, reverbFilterOrder, reverbBandNoSampleDelay, reverbBandDelayBufferIndex)`: allpass
/// filters with real coefficients common to the reverb band.
///
/// The straight port, kept as the reference of the optimized [`decorr_filter_apply_real`].
pub fn decorr_filter_apply_real_straight(
    filter: &[DecorrFilterInstance],
    buf: &mut [FixpDbl],
    data_real_in: &[FixpDbl],
    data_imag_in: &[FixpDbl],
    data_real_out: &mut [FixpDbl],
    data_imag_out: &mut [FixpDbl],
    start: i32,
    stop: i32,
    reverb_filter_order: i32,
    reverb_band_no_sample_delay: i32,
    reverb_band_delay_buffer_index: i32,
) -> i32 {
    // pFilter = filter[start].numeratorReal (order + 1 coefficients; a padded copy, see the
    // module documentation)
    let mut p_filter = [0 as FixpDecorr; 21];
    if let Some(n) = filter[start as usize].numerator_real {
        let len = n.len().min(p_filter.len());
        p_filter[..len].copy_from_slice(&n[..len]);
    }

    let offset_delay_buffer = (2 * reverb_band_no_sample_delay) as isize - 1;
    let mut p_delay_buffer =
        ptr(filter[start as usize].delay_buffer_cplx) + reverb_band_delay_buffer_index as isize;

    let offset_states = (2 * reverb_filter_order) as isize;
    let mut p_states = ptr(filter[start as usize].state_cplx);

    // Memory for the delayline has been allocated in a consecutive order, so we can address from
    // filter to filter with a constant length. The same is valid for the states. Be aware that
    // real and imaginary part of the delayline and the states are stored in interleaved order.
    // All filter in a reverb band have the same filter coefficients. Exploit symmetry:
    // numeratorReal[i] = denominatorReal[reverbFilterLength-1-i]. Do not accumulate the highest
    // states which are always zero.
    let st = |buf: &[FixpDbl], p: isize, k: isize| rd(buf, p + k);
    for i in start as usize..stop.max(start) as usize {
        let x_real = rd(buf, p_delay_buffer);
        wr(buf, p_delay_buffer, data_real_in[i]);
        p_delay_buffer += 1;

        let x_imag = rd(buf, p_delay_buffer);
        wr(buf, p_delay_buffer, data_imag_in[i]);
        p_delay_buffer += offset_delay_buffer;

        let ps = p_states;
        if reverb_filter_order == 2 {
            let (n0l, n0h) = (p_filter[0], p_filter[1]);

            let y_real = st(buf, ps, 0)
                .wrapping_add(f_mult_div2(x_real, n0l))
                .wrapping_shl(FILTER_SF as u32);
            let y_imag = st(buf, ps, 1)
                .wrapping_add(f_mult_div2(x_imag, n0l))
                .wrapping_shl(FILTER_SF as u32);

            data_real_out[i] = y_real;
            data_imag_out[i] = y_imag;

            let v0 = st(buf, ps, 2)
                .wrapping_add(f_mult_div2(x_real, n0h))
                .wrapping_sub(f_mult_div2(y_real, n0h));
            wr(buf, ps, v0);
            let v1 = st(buf, ps, 3)
                .wrapping_add(f_mult_div2(x_imag, n0h))
                .wrapping_sub(f_mult_div2(y_imag, n0h));
            wr(buf, ps + 1, v1);
            wr(buf, ps + 2, (x_real >> FILTER_SF).wrapping_sub(f_mult_div2(y_real, n0l)));
            wr(buf, ps + 3, (x_imag >> FILTER_SF).wrapping_sub(f_mult_div2(y_imag, n0l)));
        } else if reverb_filter_order == 3 {
            let (n0l, n0h, n1l) = (p_filter[0], p_filter[1], p_filter[2]);

            let y_real = st(buf, ps, 0)
                .wrapping_add(f_mult_div2(x_real, n0l))
                .wrapping_shl(FILTER_SF as u32);
            let y_imag = st(buf, ps, 1)
                .wrapping_add(f_mult_div2(x_imag, n0l))
                .wrapping_shl(FILTER_SF as u32);

            data_real_out[i] = y_real;
            data_imag_out[i] = y_imag;

            let v = st(buf, ps, 2)
                .wrapping_add(f_mult_div2(x_real, n0h))
                .wrapping_sub(f_mult_div2(y_real, n1l));
            wr(buf, ps, v);
            let v = st(buf, ps, 3)
                .wrapping_add(f_mult_div2(x_imag, n0h))
                .wrapping_sub(f_mult_div2(y_imag, n1l));
            wr(buf, ps + 1, v);
            let v = st(buf, ps, 4)
                .wrapping_add(f_mult_div2(x_real, n1l))
                .wrapping_sub(f_mult_div2(y_real, n0h));
            wr(buf, ps + 2, v);
            let v = st(buf, ps, 5)
                .wrapping_add(f_mult_div2(x_imag, n1l))
                .wrapping_sub(f_mult_div2(y_imag, n0h));
            wr(buf, ps + 3, v);
            wr(buf, ps + 4, (x_real >> FILTER_SF).wrapping_sub(f_mult_div2(y_real, n0l)));
            wr(buf, ps + 5, (x_imag >> FILTER_SF).wrapping_sub(f_mult_div2(y_imag, n0l)));
        } else if reverb_filter_order == 6 {
            let (n0l, n0h, n1l, n1h, n2l, n2h) =
                (p_filter[0], p_filter[1], p_filter[2], p_filter[3], p_filter[4], p_filter[5]);

            let y_real = st(buf, ps, 0)
                .wrapping_add(f_mult_div2(x_real, n0l))
                .wrapping_shl(FILTER_SF as u32);
            let y_imag = st(buf, ps, 1)
                .wrapping_add(f_mult_div2(x_imag, n0l))
                .wrapping_shl(FILTER_SF as u32);
            data_real_out[i] = y_real;
            data_imag_out[i] = y_imag;

            // pStates[2k] = pStates[2k + 2] + fMultDiv2(x, a) - fMultDiv2(y, b)
            let coef: [(FixpDecorr, FixpDecorr); 5] =
                [(n0h, n2h), (n1l, n2l), (n1h, n1h), (n2l, n1l), (n2h, n0h)];
            for (k, &(a, b)) in coef.iter().enumerate() {
                let k = 2 * k as isize;
                let v = st(buf, ps, k + 2)
                    .wrapping_add(f_mult_div2(x_real, a))
                    .wrapping_sub(f_mult_div2(y_real, b));
                wr(buf, ps + k, v);
                let v = st(buf, ps, k + 3)
                    .wrapping_add(f_mult_div2(x_imag, a))
                    .wrapping_sub(f_mult_div2(y_imag, b));
                wr(buf, ps + k + 1, v);
            }
            wr(buf, ps + 10, (x_real >> FILTER_SF).wrapping_sub(f_mult_div2(y_real, n0l)));
            wr(buf, ps + 11, (x_imag >> FILTER_SF).wrapping_sub(f_mult_div2(y_imag, n0l)));
        } else {
            let n0l = p_filter[0];
            let y_real = st(buf, ps, 0).wrapping_add(f_mult_div2(x_real, n0l)).wrapping_shl(2);
            let y_imag = st(buf, ps, 1).wrapping_add(f_mult_div2(x_imag, n0l)).wrapping_shl(2);
            data_real_out[i] = y_real;
            data_imag_out[i] = y_imag;

            let order = reverb_filter_order.max(0) as usize;
            let mut j = 1usize;
            while j < order {
                let n0l = p_filter[j];
                let n0h = p_filter[order - j];
                let jj = j as isize;
                let v = st(buf, ps, 2 * jj)
                    .wrapping_add(f_mult_div2(x_real, n0l))
                    .wrapping_sub(f_mult_div2(y_real, n0h));
                wr(buf, ps + 2 * jj - 2, v);
                let v = st(buf, ps, 2 * jj + 1)
                    .wrapping_add(f_mult_div2(x_imag, n0l))
                    .wrapping_sub(f_mult_div2(y_imag, n0h));
                wr(buf, ps + 2 * jj - 1, v);
                j += 1;
            }
            // j == reverbFilterOrder (1 for orders below 1)
            let n0l = p_filter.get(j).copied().unwrap_or(0);
            let n0h = p_filter.get(order.wrapping_sub(j)).copied().unwrap_or(0);
            let jj = j as isize;
            wr(
                buf,
                ps + 2 * jj - 2,
                f_mult_div2(x_real, n0l).wrapping_sub(f_mult_div2(y_real, n0h)),
            );
            wr(
                buf,
                ps + 2 * jj - 1,
                f_mult_div2(x_imag, n0l).wrapping_sub(f_mult_div2(y_imag, n0h)),
            );
        }
        p_states += offset_states;
    }

    0
}

/// `DecorrFilterApplyCPLX_PS(filter, dataRealIn, dataImagIn, dataRealOut, dataImagOut, start,
/// stop, reverbFilterOrder, reverbBandNoSampleDelay, reverbBandDelayBufferIndex,
/// stateBufferOffset)`: the PS allpass filters (three stages with independent complex
/// coefficients per hybrid band).
///
/// The straight port, kept as the reference of the optimized [`decorr_filter_apply_cplx_ps`].
pub fn decorr_filter_apply_cplx_ps_straight(
    filter: &[DecorrFilterInstance],
    buf: &mut [FixpDbl],
    data_real_in: &[FixpDbl],
    data_imag_in: &[FixpDbl],
    data_real_out: &mut [FixpDbl],
    data_imag_out: &mut [FixpDbl],
    start: i32,
    stop: i32,
    reverb_filter_order: i32,
    reverb_band_no_sample_delay: i32,
    reverb_band_delay_buffer_index: i32,
    state_buffer_offset: &mut [u8; NUM_PS_STAGES],
) -> i32 {
    // Not a table of the C++ code: the coefficients the port reads for a filter without
    // `coeffsPacked` (`None`; the reference has no defined result there).
    const NO_COEFFS: [FixpStp; 4] = [FixpStp { re: 0, im: 0 }; 4];

    // get pointer to current position in input delay buffer of filter with starting-index
    let mut p_delay_buffer =
        ptr(filter[start as usize].delay_buffer_cplx) + reverb_band_delay_buffer_index as isize;
    // determine the increment for this pointer to get to the correct position in the delay
    // buffer of the next filter
    let offset_delay_buffer = (2 * reverb_band_no_sample_delay) as isize - 1;

    // pointer to current position in state buffer
    let p_states = ptr(filter[start as usize].state_cplx);
    let p_states_increment = (2 * reverb_filter_order) as isize;

    // stateBufferOffset-pointers
    let mut p_state_buffer_offset0 = p_states + state_buffer_offset[0] as isize;
    let mut p_state_buffer_offset1 = p_states + state_buffer_offset[1] as isize;
    let mut p_state_buffer_offset2 = p_states + state_buffer_offset[2] as isize;

    // traverse all hybrid-bands inbetween start- and stop-index
    for i in start as usize..stop.max(start) as usize {
        // 1. input delay (real/imaginary values interleaved)
        let r_data_a = rd(buf, p_delay_buffer);
        wr(buf, p_delay_buffer, data_real_in[i]);
        p_delay_buffer += 1;

        let j_data_a = rd(buf, p_delay_buffer);
        wr(buf, p_delay_buffer, data_imag_in[i]);
        p_delay_buffer += offset_delay_buffer;

        // 2. Phi(k)-stage: the first two entries of the coefficient table are the
        // Phi(k)-multiplicants
        let p_coeffs = filter[i].coeffs_packed.unwrap_or(&NO_COEFFS);
        let rj = p_coeffs[0];
        let (r_data_b, j_data_b) = cplx_mult_div2(r_data_a, j_data_a, rj.re, rj.im);

        // 3. process all three filter stages

        // stage 0
        let rj = p_coeffs[1];
        let (r, j) = cplx_mult_div2(r_data_b, j_data_b, rj.re, rj.im);
        let (r_stage_mult, j_stage_mult) = (r.wrapping_shl(1), j.wrapping_shl(1));
        // read and add value from state buffer (this is the input for the next stage)
        let r_data_a = r_stage_mult.wrapping_add(rd(buf, p_state_buffer_offset0));
        let j_data_a = j_stage_mult.wrapping_add(rd(buf, p_state_buffer_offset0 + 1));
        // negate r_data_a to perform multiplication with complex conjugate of rj_coeff
        let (r, j) = cplx_mult_div2(r_data_a.wrapping_neg(), j_data_a, rj.re, rj.im);
        // add stage input to shifted result, store result to state buffer
        wr(buf, p_state_buffer_offset0, r_data_b.wrapping_add(r.wrapping_shl(1)));
        wr(buf, p_state_buffer_offset0 + 1, j_data_b.wrapping_sub(j.wrapping_shl(1)));
        p_state_buffer_offset0 += p_states_increment;

        // stage 1
        let rj = p_coeffs[2];
        let (r, j) = cplx_mult_div2(r_data_a, j_data_a, rj.re, rj.im);
        let (r_stage_mult, j_stage_mult) = (r.wrapping_shl(1), j.wrapping_shl(1));
        let r_data_b = r_stage_mult.wrapping_add(rd(buf, p_state_buffer_offset1));
        let j_data_b = j_stage_mult.wrapping_add(rd(buf, p_state_buffer_offset1 + 1));
        let (r, j) = cplx_mult_div2(r_data_b.wrapping_neg(), j_data_b, rj.re, rj.im);
        wr(buf, p_state_buffer_offset1, r_data_a.wrapping_add(r.wrapping_shl(1)));
        wr(buf, p_state_buffer_offset1 + 1, j_data_a.wrapping_sub(j.wrapping_shl(1)));
        p_state_buffer_offset1 += p_states_increment;

        // stage 2
        let rj = p_coeffs[3];
        let (r, j) = cplx_mult_div2(r_data_b, j_data_b, rj.re, rj.im);
        let (r_stage_mult, j_stage_mult) = (r.wrapping_shl(1), j.wrapping_shl(1));
        let r_data_a = r_stage_mult.wrapping_add(rd(buf, p_state_buffer_offset2));
        let j_data_a = j_stage_mult.wrapping_add(rd(buf, p_state_buffer_offset2 + 1));
        let (r, j) = cplx_mult_div2(r_data_a.wrapping_neg(), j_data_a, rj.re, rj.im);
        wr(buf, p_state_buffer_offset2, r_data_b.wrapping_add(r.wrapping_shl(1)));
        wr(buf, p_state_buffer_offset2 + 1, j_data_b.wrapping_sub(j.wrapping_shl(1)));
        p_state_buffer_offset2 += p_states_increment;

        // write filter output
        data_real_out[i] = r_data_a.wrapping_shl(1);
        data_imag_out[i] = j_data_a.wrapping_shl(1);
    } // end of band/filter loop (outer loop)

    // update stateBufferOffset with respect to ring buffer boundaries
    if state_buffer_offset[0] == 4 {
        state_buffer_offset[0] = 0;
    } else {
        state_buffer_offset[0] = state_buffer_offset[0].wrapping_add(2);
    }

    if state_buffer_offset[1] == 12 {
        state_buffer_offset[1] = 6;
    } else {
        state_buffer_offset[1] = state_buffer_offset[1].wrapping_add(2);
    }

    if state_buffer_offset[2] == 22 {
        state_buffer_offset[2] = 14;
    } else {
        state_buffer_offset[2] = state_buffer_offset[2].wrapping_add(2);
    }

    0
}

// Optimized filter kernels. Each one checks once that every buffer index of the call lies in
// the buffer and that the band range lies in the data slices; then it runs the same operations in
// the same order as the straight port, without the per-access checks of `rd()`/`wr()` (the reads
// and writes go through one pointer into the buffer, so overlapping delay lines and states behave
// as in the straight port). Otherwise (offsets outside the buffer) it calls the straight port.

/// Whether `base + k * stride + extra` lies in `0..len` for every `k < n` (with `n > 0`).
#[inline(always)]
fn span_in(len: usize, base: isize, stride: isize, n: usize, extra: isize) -> bool {
    if base < 0 || stride < 0 || extra < 0 || n == 0 {
        return false;
    }
    let last = base as i128 + (n as i128 - 1) * stride as i128 + extra as i128;
    last < len as i128
}

/// Whether `start..stop` (non-empty) lies in every one of the slice lengths `lens`.
#[inline(always)]
fn bands_in(start: i32, stop: i32, lens: &[usize]) -> bool {
    start >= 0 && stop > start && lens.iter().all(|&l| stop as usize <= l)
}

/// `DecorrFilterApplyPASS()`, optimized (see [`decorr_filter_apply_pass_straight`]).
pub fn decorr_filter_apply_pass(
    filter: &[DecorrFilterInstance],
    buf: &mut [FixpDbl],
    data_real_in: &[FixpDbl],
    data_imag_in: Option<&[FixpDbl]>,
    data_real_out: &mut [FixpDbl],
    data_imag_out: &mut [FixpDbl],
    start: i32,
    stop: i32,
    reverb_band_no_sample_delay: i32,
    reverb_band_delay_buffer_index: i32,
) -> i32 {
    let fast = match data_imag_in {
        Some(data_imag_in)
            if bands_in(
                start,
                stop,
                &[
                    filter.len(),
                    data_real_in.len(),
                    data_imag_in.len(),
                    data_real_out.len(),
                    data_imag_out.len(),
                ],
            ) =>
        {
            let n = (stop - start) as usize;
            let stride = 2 * reverb_band_no_sample_delay as isize;
            let p0 = ptr(filter[start as usize].delay_buffer_cplx)
                + reverb_band_delay_buffer_index as isize;
            if span_in(buf.len(), p0, stride, n, 1) {
                Some((data_imag_in, n, stride, p0 as usize))
            } else {
                None
            }
        }
        _ => None,
    };
    let Some((data_imag_in, n, stride, p0)) = fast else {
        return decorr_filter_apply_pass_straight(
            filter,
            buf,
            data_real_in,
            data_imag_in,
            data_real_out,
            data_imag_out,
            start,
            stop,
            reverb_band_no_sample_delay,
            reverb_band_delay_buffer_index,
        );
    };
    let s = start as usize;
    let (re_in, im_in) = (&data_real_in[s..s + n], &data_imag_in[s..s + n]);
    let (re_out, im_out) = (&mut data_real_out[s..s + n], &mut data_imag_out[s..s + n]);
    let b = buf.as_mut_ptr();
    let stride = stride as usize;
    for k in 0..n {
        let p = p0 + k * stride;
        // SAFETY: p + 1 < buf.len() for every k < n (span_in above).
        unsafe {
            let delay_re = *b.add(p);
            let delay_im = *b.add(p + 1);
            *b.add(p) = re_in[k];
            *b.add(p + 1) = im_in[k];
            re_out[k] = delay_re;
            im_out[k] = delay_im;
        }
    }
    0
}

/// `DecorrFilterApplyREAL()`, optimized (see [`decorr_filter_apply_real_straight`]).
pub fn decorr_filter_apply_real(
    filter: &[DecorrFilterInstance],
    buf: &mut [FixpDbl],
    data_real_in: &[FixpDbl],
    data_imag_in: &[FixpDbl],
    data_real_out: &mut [FixpDbl],
    data_imag_out: &mut [FixpDbl],
    start: i32,
    stop: i32,
    reverb_filter_order: i32,
    reverb_band_no_sample_delay: i32,
    reverb_band_delay_buffer_index: i32,
) -> i32 {
    let fast = if reverb_filter_order >= 1
        && bands_in(
            start,
            stop,
            &[
                filter.len(),
                data_real_in.len(),
                data_imag_in.len(),
                data_real_out.len(),
                data_imag_out.len(),
            ],
        ) {
        let n = (stop - start) as usize;
        let d0 =
            ptr(filter[start as usize].delay_buffer_cplx) + reverb_band_delay_buffer_index as isize;
        let d_stride = 2 * reverb_band_no_sample_delay as isize;
        let s0 = ptr(filter[start as usize].state_cplx);
        let s_stride = 2 * reverb_filter_order as isize;
        // the highest state index of a filter is 2 * order - 1
        span_in(buf.len(), d0, d_stride, n, 1) && span_in(buf.len(), s0, s_stride, n, s_stride - 1)
    } else {
        false
    };
    if !fast {
        return decorr_filter_apply_real_straight(
            filter,
            buf,
            data_real_in,
            data_imag_in,
            data_real_out,
            data_imag_out,
            start,
            stop,
            reverb_filter_order,
            reverb_band_no_sample_delay,
            reverb_band_delay_buffer_index,
        );
    }

    let mut p_filter = [0 as FixpDecorr; 21];
    if let Some(n) = filter[start as usize].numerator_real {
        let len = n.len().min(p_filter.len());
        p_filter[..len].copy_from_slice(&n[..len]);
    }
    let s = start as usize;
    let n = (stop - start) as usize;
    let order = reverb_filter_order as usize;
    let d_stride = 2 * reverb_band_no_sample_delay as usize;
    let s_stride = 2 * order;
    let d0 = (ptr(filter[s].delay_buffer_cplx) + reverb_band_delay_buffer_index as isize) as usize;
    let s0 = ptr(filter[s].state_cplx) as usize;
    let (re_in, im_in) = (&data_real_in[s..s + n], &data_imag_in[s..s + n]);
    let (re_out, im_out) = (&mut data_real_out[s..s + n], &mut data_imag_out[s..s + n]);
    let b = buf.as_mut_ptr();
    // SAFETY (all accesses below): d + 1 < buf.len() and ps + 2 * order - 1 < buf.len() for every
    // k < n (span_in above).
    let rd = |i: usize| unsafe { *b.add(i) };
    let wr = |i: usize, v: FixpDbl| unsafe { *b.add(i) = v };

    for k in 0..n {
        let d = d0 + k * d_stride;
        let x_real = rd(d);
        wr(d, re_in[k]);
        let x_imag = rd(d + 1);
        wr(d + 1, im_in[k]);

        let ps = s0 + k * s_stride;
        let n0l = p_filter[0];
        let y_real = rd(ps).wrapping_add(f_mult_div2(x_real, n0l)).wrapping_shl(FILTER_SF as u32);
        let y_imag =
            rd(ps + 1).wrapping_add(f_mult_div2(x_imag, n0l)).wrapping_shl(FILTER_SF as u32);
        re_out[k] = y_real;
        im_out[k] = y_imag;

        if order == 2 || order == 3 || order == 6 {
            // pStates[2j - 2] = pStates[2j] + fMultDiv2(x, num[j]) - fMultDiv2(y, num[order - j])
            // for j = 1..order-1, then the last pair from x >> FILTER_SF (the unrolled C++ code
            // of these orders uses numerator[0] for the denominator of the highest state).
            for j in 1..order {
                let a = p_filter[j];
                let c = p_filter[order - j];
                let v = rd(ps + 2 * j)
                    .wrapping_add(f_mult_div2(x_real, a))
                    .wrapping_sub(f_mult_div2(y_real, c));
                wr(ps + 2 * j - 2, v);
                let v = rd(ps + 2 * j + 1)
                    .wrapping_add(f_mult_div2(x_imag, a))
                    .wrapping_sub(f_mult_div2(y_imag, c));
                wr(ps + 2 * j - 1, v);
            }
            wr(ps + 2 * order - 2, (x_real >> FILTER_SF).wrapping_sub(f_mult_div2(y_real, n0l)));
            wr(ps + 2 * order - 1, (x_imag >> FILTER_SF).wrapping_sub(f_mult_div2(y_imag, n0l)));
        } else {
            for j in 1..order {
                let a = p_filter[j];
                let c = p_filter[order - j];
                let v = rd(ps + 2 * j)
                    .wrapping_add(f_mult_div2(x_real, a))
                    .wrapping_sub(f_mult_div2(y_real, c));
                wr(ps + 2 * j - 2, v);
                let v = rd(ps + 2 * j + 1)
                    .wrapping_add(f_mult_div2(x_imag, a))
                    .wrapping_sub(f_mult_div2(y_imag, c));
                wr(ps + 2 * j - 1, v);
            }
            // j == order
            let a = p_filter.get(order).copied().unwrap_or(0);
            let c = p_filter[0];
            wr(ps + 2 * order - 2, f_mult_div2(x_real, a).wrapping_sub(f_mult_div2(y_real, c)));
            wr(ps + 2 * order - 1, f_mult_div2(x_imag, a).wrapping_sub(f_mult_div2(y_imag, c)));
        }
    }
    0
}

/// `DecorrFilterApplyCPLX_PS()`, optimized (see [`decorr_filter_apply_cplx_ps_straight`]).
pub fn decorr_filter_apply_cplx_ps(
    filter: &[DecorrFilterInstance],
    buf: &mut [FixpDbl],
    data_real_in: &[FixpDbl],
    data_imag_in: &[FixpDbl],
    data_real_out: &mut [FixpDbl],
    data_imag_out: &mut [FixpDbl],
    start: i32,
    stop: i32,
    reverb_filter_order: i32,
    reverb_band_no_sample_delay: i32,
    reverb_band_delay_buffer_index: i32,
    state_buffer_offset: &mut [u8; NUM_PS_STAGES],
) -> i32 {
    let o = state_buffer_offset.map(|v| v as usize);
    let fast = bands_in(
        start,
        stop,
        &[
            filter.len(),
            data_real_in.len(),
            data_imag_in.len(),
            data_real_out.len(),
            data_imag_out.len(),
        ],
    ) && {
        let n = (stop - start) as usize;
        let d0 =
            ptr(filter[start as usize].delay_buffer_cplx) + reverb_band_delay_buffer_index as isize;
        let s0 = ptr(filter[start as usize].state_cplx);
        let s_stride = 2 * reverb_filter_order as isize;
        let o_max = o[0].max(o[1]).max(o[2]) as isize;
        span_in(buf.len(), d0, 2 * reverb_band_no_sample_delay as isize, n, 1)
            && span_in(buf.len(), s0, s_stride, n, o_max + 1)
    };
    if !fast {
        return decorr_filter_apply_cplx_ps_straight(
            filter,
            buf,
            data_real_in,
            data_imag_in,
            data_real_out,
            data_imag_out,
            start,
            stop,
            reverb_filter_order,
            reverb_band_no_sample_delay,
            reverb_band_delay_buffer_index,
            state_buffer_offset,
        );
    }

    const NO_COEFFS: [FixpStp; 4] = [FixpStp { re: 0, im: 0 }; 4];
    let s = start as usize;
    let n = (stop - start) as usize;
    let d_stride = 2 * reverb_band_no_sample_delay as usize;
    let s_stride = 2 * reverb_filter_order as usize;
    let d0 = (ptr(filter[s].delay_buffer_cplx) + reverb_band_delay_buffer_index as isize) as usize;
    let s0 = ptr(filter[s].state_cplx) as usize;
    let filters = &filter[s..s + n];
    let (re_in, im_in) = (&data_real_in[s..s + n], &data_imag_in[s..s + n]);
    let (re_out, im_out) = (&mut data_real_out[s..s + n], &mut data_imag_out[s..s + n]);
    let b = buf.as_mut_ptr();
    // SAFETY (all accesses below): d + 1 < buf.len() and ps + o[j] + 1 < buf.len() for every
    // k < n (span_in above).
    let rd = |i: usize| unsafe { *b.add(i) };
    let wr = |i: usize, v: FixpDbl| unsafe { *b.add(i) = v };

    // Groups of four bands with NEON (aarch64 variant: the cplxMultDiv2 of fixmadd_arm.h), if
    // the bands are independent: every band reads and writes only its own state pairs and its
    // own delay pair, and the delay lines do not overlap the states. Then the order of the
    // operations of different bands is not observable.
    #[allow(unused_mut)]
    let mut k0 = 0;
    #[cfg(target_arch = "aarch64")]
    if ARCH_ARM && n >= 4 {
        let o_max = o[0].max(o[1]).max(o[2]);
        let (d_end, s_end) = (d0 + (n - 1) * d_stride + 2, s0 + n * s_stride);
        if d_stride >= 2 && s_stride >= o_max + 2 && (d_end <= s0 || s_end <= d0) {
            k0 = n & !3;
            // SAFETY: the indices are those of the scalar loop (checked above); the coefficient
            // pointers are 4 FIXP_STP each.
            unsafe {
                neon_ps::groups(
                    b, k0, d0, d_stride, s0, s_stride, o, filters, re_in, im_in, re_out, im_out,
                )
            };
        }
    }

    // The same with AVX2 (x86, x86_32 and generic variants: the cplxMultDiv2 of the generic
    // code, individually truncated products), four bands at a time.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && n >= 4 && crate::simd_x86::avx2() {
        let o_max = o[0].max(o[1]).max(o[2]);
        let (d_end, s_end) = (d0 + (n - 1) * d_stride + 2, s0 + n * s_stride);
        if d_stride >= 2 && s_stride >= o_max + 2 && (d_end <= s0 || s_end <= d0) {
            k0 = n & !3;
            // SAFETY: the CPU has AVX2; the indices are those of the scalar loop (checked
            // above); the coefficient pointers are 4 FIXP_STP each.
            unsafe {
                x86_ps::groups(
                    b, k0, d0, d_stride, s0, s_stride, o, filters, re_in, im_in, re_out, im_out,
                )
            };
        }
    }

    for k in k0..n {
        // 1. input delay
        let d = d0 + k * d_stride;
        let r_data_a = rd(d);
        wr(d, re_in[k]);
        let j_data_a = rd(d + 1);
        wr(d + 1, im_in[k]);

        // 2. Phi(k)-stage
        let p_coeffs = filters[k].coeffs_packed.unwrap_or(&NO_COEFFS);
        let rj = p_coeffs[0];
        let (r_data_b, j_data_b) = cplx_mult_div2(r_data_a, j_data_a, rj.re, rj.im);

        let ps = s0 + k * s_stride;
        // stage 0
        let p = ps + o[0];
        let rj = p_coeffs[1];
        let (r, j) = cplx_mult_div2(r_data_b, j_data_b, rj.re, rj.im);
        let r_data_a = r.wrapping_shl(1).wrapping_add(rd(p));
        let j_data_a = j.wrapping_shl(1).wrapping_add(rd(p + 1));
        let (r, j) = cplx_mult_div2(r_data_a.wrapping_neg(), j_data_a, rj.re, rj.im);
        wr(p, r_data_b.wrapping_add(r.wrapping_shl(1)));
        wr(p + 1, j_data_b.wrapping_sub(j.wrapping_shl(1)));

        // stage 1
        let p = ps + o[1];
        let rj = p_coeffs[2];
        let (r, j) = cplx_mult_div2(r_data_a, j_data_a, rj.re, rj.im);
        let r_data_b = r.wrapping_shl(1).wrapping_add(rd(p));
        let j_data_b = j.wrapping_shl(1).wrapping_add(rd(p + 1));
        let (r, j) = cplx_mult_div2(r_data_b.wrapping_neg(), j_data_b, rj.re, rj.im);
        wr(p, r_data_a.wrapping_add(r.wrapping_shl(1)));
        wr(p + 1, j_data_a.wrapping_sub(j.wrapping_shl(1)));

        // stage 2
        let p = ps + o[2];
        let rj = p_coeffs[3];
        let (r, j) = cplx_mult_div2(r_data_b, j_data_b, rj.re, rj.im);
        let r_data_a = r.wrapping_shl(1).wrapping_add(rd(p));
        let j_data_a = j.wrapping_shl(1).wrapping_add(rd(p + 1));
        let (r, j) = cplx_mult_div2(r_data_a.wrapping_neg(), j_data_a, rj.re, rj.im);
        wr(p, r_data_b.wrapping_add(r.wrapping_shl(1)));
        wr(p + 1, j_data_b.wrapping_sub(j.wrapping_shl(1)));

        re_out[k] = r_data_a.wrapping_shl(1);
        im_out[k] = j_data_a.wrapping_shl(1);
    }

    // update stateBufferOffset with respect to ring buffer boundaries
    for (v, (last, first)) in state_buffer_offset.iter_mut().zip([(4, 0), (12, 6), (22, 14)]) {
        *v = if *v == last { first } else { v.wrapping_add(2) };
    }

    0
}

/// NEON kernel of [`decorr_filter_apply_cplx_ps`] (aarch64 variant).
#[cfg(target_arch = "aarch64")]
mod neon_ps {
    use super::{DecorrFilterInstance, FixpDbl, FixpStp};
    use core::arch::aarch64::*;

    /// `cplxMultDiv2()` of `arm/cplx_mul_arm.h` (32x16, the coefficients as `b << 16`) for four
    /// lanes: `smull`, `smlsl` / `smlal` (wrapping 64 bit sums), the high 32 bits.
    #[inline(always)]
    unsafe fn cmul_div2(
        ar: int32x4_t,
        ai: int32x4_t,
        br: int32x4_t,
        bi: int32x4_t,
    ) -> (int32x4_t, int32x4_t) {
        let (arl, ail, brl, bil) =
            (vget_low_s32(ar), vget_low_s32(ai), vget_low_s32(br), vget_low_s32(bi));
        let rl = vmlsl_s32(vmull_s32(arl, brl), ail, bil);
        let rh = vmlsl_high_s32(vmull_high_s32(ar, br), ai, bi);
        let il = vmlal_s32(vmull_s32(arl, bil), ail, brl);
        let ih = vmlal_high_s32(vmull_high_s32(ar, bi), ai, br);
        (
            vshrn_high_n_s64::<32>(vshrn_n_s64::<32>(rl), rh),
            vshrn_high_n_s64::<32>(vshrn_n_s64::<32>(il), ih),
        )
    }

    /// Reads the complex pairs at `p[0]`, `p[0] + step`, ... (four bands): (re, im).
    #[inline(always)]
    unsafe fn gather(b: *const i32, p: usize, step: usize) -> (int32x4_t, int32x4_t) {
        let q01 = vcombine_s32(vld1_s32(b.add(p)), vld1_s32(b.add(p + step)));
        let q23 = vcombine_s32(vld1_s32(b.add(p + 2 * step)), vld1_s32(b.add(p + 3 * step)));
        (vuzp1q_s32(q01, q23), vuzp2q_s32(q01, q23))
    }

    /// Writes the complex pairs of four bands (the inverse of [`gather`]).
    #[inline(always)]
    unsafe fn scatter(b: *mut i32, p: usize, step: usize, re: int32x4_t, im: int32x4_t) {
        let lo = vzip1q_s32(re, im);
        let hi = vzip2q_s32(re, im);
        vst1_s32(b.add(p), vget_low_s32(lo));
        vst1_s32(b.add(p + step), vget_high_s32(lo));
        vst1_s32(b.add(p + 2 * step), vget_low_s32(hi));
        vst1_s32(b.add(p + 3 * step), vget_high_s32(hi));
    }

    /// One all-pass stage: returns the stage output, updates the state pairs at `p`.
    #[inline(always)]
    unsafe fn stage(
        b: *mut i32,
        p: usize,
        step: usize,
        xr: int32x4_t,
        xi: int32x4_t,
        cr: int32x4_t,
        ci: int32x4_t,
    ) -> (int32x4_t, int32x4_t) {
        let (r, j) = cmul_div2(xr, xi, cr, ci);
        let (sr, si) = gather(b, p, step);
        let yr = vaddq_s32(vshlq_n_s32::<1>(r), sr);
        let yi = vaddq_s32(vshlq_n_s32::<1>(j), si);
        let (r, j) = cmul_div2(vnegq_s32(yr), yi, cr, ci);
        scatter(b, p, step, vaddq_s32(xr, vshlq_n_s32::<1>(r)), vsubq_s32(xi, vshlq_n_s32::<1>(j)));
        (yr, yi)
    }

    /// Bands `0..k_end` (a multiple of 4) of the filter loop, four at a time.
    ///
    /// Safety: every index of the scalar loop for these bands lies in the buffer at `b`.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn groups(
        b: *mut i32,
        k_end: usize,
        d0: usize,
        d_stride: usize,
        s0: usize,
        s_stride: usize,
        o: [usize; 3],
        filters: &[DecorrFilterInstance],
        re_in: &[FixpDbl],
        im_in: &[FixpDbl],
        re_out: &mut [FixpDbl],
        im_out: &mut [FixpDbl],
    ) {
        const NO_COEFFS: [FixpStp; 4] = [FixpStp { re: 0, im: 0 }; 4];
        let mask = vdupq_n_s32(0xFFFF_0000u32 as i32);
        let mut k = 0;
        while k < k_end {
            // input delay
            let d = d0 + k * d_stride;
            let (r_a, j_a) = gather(b, d, d_stride);
            scatter(
                b,
                d,
                d_stride,
                vld1q_s32(re_in.as_ptr().add(k)),
                vld1q_s32(im_in.as_ptr().add(k)),
            );

            // coefficients: 4 x (re, im) per band; a 4x4 transpose of the 32 bit pairs gives
            // the pairs of one stage for the four bands (re in the low half: re << 16 and
            // im << 16 as in the C++ code)
            let c = |i: usize| {
                let p = filters[k + i].coeffs_packed.unwrap_or(&NO_COEFFS);
                vreinterpretq_s32_s16(vld1q_s16(p.as_ptr() as *const i16))
            };
            let (c0, c1, c2, c3) = (c(0), c(1), c(2), c(3));
            let t01l = vtrn1q_s32(c0, c1);
            let t01h = vtrn2q_s32(c0, c1);
            let t23l = vtrn1q_s32(c2, c3);
            let t23h = vtrn2q_s32(c2, c3);
            let st = [
                vreinterpretq_s32_s64(vtrn1q_s64(
                    vreinterpretq_s64_s32(t01l),
                    vreinterpretq_s64_s32(t23l),
                )),
                vreinterpretq_s32_s64(vtrn1q_s64(
                    vreinterpretq_s64_s32(t01h),
                    vreinterpretq_s64_s32(t23h),
                )),
                vreinterpretq_s32_s64(vtrn2q_s64(
                    vreinterpretq_s64_s32(t01l),
                    vreinterpretq_s64_s32(t23l),
                )),
                vreinterpretq_s32_s64(vtrn2q_s64(
                    vreinterpretq_s64_s32(t01h),
                    vreinterpretq_s64_s32(t23h),
                )),
            ];
            let cr = |j: usize| vshlq_n_s32::<16>(st[j]);
            let ci = |j: usize| vandq_s32(st[j], mask);

            // Phi(k)-stage
            let (r_b, j_b) = cmul_div2(r_a, j_a, cr(0), ci(0));

            let ps = s0 + k * s_stride;
            let (r_a, j_a) = stage(b, ps + o[0], s_stride, r_b, j_b, cr(1), ci(1));
            let (r_b, j_b) = stage(b, ps + o[1], s_stride, r_a, j_a, cr(2), ci(2));
            let (r_a, j_a) = stage(b, ps + o[2], s_stride, r_b, j_b, cr(3), ci(3));

            vst1q_s32(re_out.as_mut_ptr().add(k), vshlq_n_s32::<1>(r_a));
            vst1q_s32(im_out.as_mut_ptr().add(k), vshlq_n_s32::<1>(j_a));
            k += 4;
        }
    }
}

/// AVX2 kernel of [`decorr_filter_apply_cplx_ps`] (x86, x86_32 and generic variants).
///
/// Four bands at a time, each complex pair `(re, im)` in two adjacent 32 bit lanes. The
/// `cplxMultDiv2()` of these variants is `(fMultDiv2(ar, br) - fMultDiv2(ai, bi),
/// fMultDiv2(ar, bi) + fMultDiv2(ai, br))` with `fMultDiv2(FIXP_DBL x, FIXP_SGL a) = (x * a) >>
/// 16`, computed exactly in 32 bit lanes as `(x >> 16) * a + (((x & 0xFFFF) * a) >> 16)`; on
/// the pairs it is `M(v, br) + (-1, 1) * M(swap(v), bi)` (wrapping, like the C code).
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86_ps {
    use super::{DecorrFilterInstance, FixpDbl, FixpStp};
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    #[inline(always)]
    unsafe fn md2(x: __m256i, a: __m256i) -> __m256i {
        let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(x), a);
        let lo = _mm256_and_si256(x, _mm256_set1_epi32(0xFFFF));
        _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, a)))
    }

    /// `cplxMultDiv2()` of the pairs `v` with the coefficients `br`, `bi` (each duplicated in the
    /// two lanes of a pair).
    #[inline(always)]
    unsafe fn cmul_div2(v: __m256i, br: __m256i, bi: __m256i) -> __m256i {
        let swap = _mm256_shuffle_epi32::<0b1011_0001>(v);
        let t = _mm256_sign_epi32(md2(swap, bi), _mm256_setr_epi32(-1, 1, -1, 1, -1, 1, -1, 1));
        _mm256_add_epi32(md2(v, br), t)
    }

    /// The complex pairs at `p`, `p + step`, `p + 2 step`, `p + 3 step`.
    #[inline(always)]
    unsafe fn gather(b: *const i32, p: usize, step: usize) -> __m256i {
        let g = |q: usize| (b.add(q) as *const i64).read_unaligned();
        _mm256_set_epi64x(g(p + 3 * step), g(p + 2 * step), g(p + step), g(p))
    }

    /// Writes the complex pairs of four bands (the inverse of [`gather`]).
    #[inline(always)]
    unsafe fn scatter(b: *mut i32, p: usize, step: usize, v: __m256i) {
        let lo = _mm256_castsi256_si128(v);
        let hi = _mm256_extracti128_si256::<1>(v);
        _mm_storel_epi64(b.add(p) as *mut __m128i, lo);
        _mm_storel_epi64(b.add(p + step) as *mut __m128i, _mm_unpackhi_epi64(lo, lo));
        _mm_storel_epi64(b.add(p + 2 * step) as *mut __m128i, hi);
        _mm_storel_epi64(b.add(p + 3 * step) as *mut __m128i, _mm_unpackhi_epi64(hi, hi));
    }

    /// One all-pass stage: returns the stage output, updates the state pairs at `p`.
    #[inline(always)]
    unsafe fn stage(
        b: *mut i32,
        p: usize,
        step: usize,
        x: __m256i,
        cr: __m256i,
        ci: __m256i,
    ) -> __m256i {
        // (r, j) = cplxMultDiv2(x, c); y = 2 (r, j) + state
        let y = _mm256_add_epi32(_mm256_slli_epi32::<1>(cmul_div2(x, cr, ci)), gather(b, p, step));
        // (r, j) = cplxMultDiv2((-y.re, y.im), c); state = (x.re + 2 r, x.im - 2 j)
        let neg_re = _mm256_setr_epi32(-1, 1, -1, 1, -1, 1, -1, 1);
        let rj = _mm256_slli_epi32::<1>(cmul_div2(_mm256_sign_epi32(y, neg_re), cr, ci));
        let neg_im = _mm256_setr_epi32(1, -1, 1, -1, 1, -1, 1, -1);
        scatter(b, p, step, _mm256_add_epi32(x, _mm256_sign_epi32(rj, neg_im)));
        y
    }

    /// Bands `0..k_end` (a multiple of 4) of the filter loop, four at a time.
    ///
    /// # Safety
    /// The CPU must have AVX2; every index of the scalar loop for these bands lies in the
    /// buffer at `b`.
    #[allow(clippy::too_many_arguments)]
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn groups(
        b: *mut i32,
        k_end: usize,
        d0: usize,
        d_stride: usize,
        s0: usize,
        s_stride: usize,
        o: [usize; 3],
        filters: &[DecorrFilterInstance],
        re_in: &[FixpDbl],
        im_in: &[FixpDbl],
        re_out: &mut [FixpDbl],
        im_out: &mut [FixpDbl],
    ) {
        const NO_COEFFS: [FixpStp; 4] = [FixpStp { re: 0, im: 0 }; 4];
        assert!(k_end <= filters.len() && k_end <= re_in.len() && k_end <= im_in.len());
        assert!(k_end <= re_out.len() && k_end <= im_out.len());
        let deint = _mm256_setr_epi32(0, 2, 4, 6, 1, 3, 5, 7);
        let mut k = 0;
        while k < k_end {
            // input delay
            let d = d0 + k * d_stride;
            let a = gather(b, d, d_stride);
            let r_in = _mm_loadu_si128(re_in.as_ptr().add(k) as *const __m128i);
            let i_in = _mm_loadu_si128(im_in.as_ptr().add(k) as *const __m128i);
            let new =
                _mm256_set_m128i(_mm_unpackhi_epi32(r_in, i_in), _mm_unpacklo_epi32(r_in, i_in));
            scatter(b, d, d_stride, new);

            // the coefficients of stage j for the four bands, each duplicated in a pair
            let co: [&[FixpStp; 4]; 4] =
                core::array::from_fn(|i| filters[k + i].coeffs_packed.unwrap_or(&NO_COEFFS));
            let cr = |j: usize| {
                let v: [i32; 4] = core::array::from_fn(|i| co[i][j].re as i32);
                _mm256_setr_epi32(v[0], v[0], v[1], v[1], v[2], v[2], v[3], v[3])
            };
            let ci = |j: usize| {
                let v: [i32; 4] = core::array::from_fn(|i| co[i][j].im as i32);
                _mm256_setr_epi32(v[0], v[0], v[1], v[1], v[2], v[2], v[3], v[3])
            };

            // Phi(k)-stage
            let bb = cmul_div2(a, cr(0), ci(0));

            let ps = s0 + k * s_stride;
            let a = stage(b, ps + o[0], s_stride, bb, cr(1), ci(1));
            let bb = stage(b, ps + o[1], s_stride, a, cr(2), ci(2));
            let a = stage(b, ps + o[2], s_stride, bb, cr(3), ci(3));

            let out = _mm256_permutevar8x32_epi32(_mm256_slli_epi32::<1>(a), deint);
            _mm_storeu_si128(
                re_out.as_mut_ptr().add(k) as *mut __m128i,
                _mm256_castsi256_si128(out),
            );
            _mm_storeu_si128(
                im_out.as_mut_ptr().add(k) as *mut __m128i,
                _mm256_extracti128_si256::<1>(out),
            );
            k += 4;
        }
    }
}

/// `DuckerInit(self, hybridBands, partiallyComplex, duckerType, nParamBands, initStatesFlag)`
fn ducker_init(
    self_: &mut DuckerInstance,
    hybrid_bands: i32,
    partially_complex: i32,
    ducker_type: FdkDuckerType,
    n_param_bands: i32,
    init_states_flag: i32,
) -> i32 {
    let error_code = 0;

    match n_param_bands {
        20 => {
            fdk_assert!(hybrid_bands == 71, "FDK_ASSERT(hybridBands == 71)");
            self_.map_hyb_bands2_proc_bands = Some(&KERNELS_20_TO_71_PS[..]);
            self_.map_proc_bands2_hyb_bands = Some(&KERNELS_20_TO_71_OFFSET_PS[..]);
            self_.parameter_bands = 20;
        }
        28 => {
            self_.map_hyb_bands2_proc_bands = Some(&KERNELS_28_TO_71[..]);
            self_.map_proc_bands2_hyb_bands = Some(&KERNELS_28_TO_71_OFFSET[..]);
            self_.parameter_bands = 28;
        }
        23 => {
            fdk_assert!(
                hybrid_bands == 64 || hybrid_bands == 32,
                "FDK_ASSERT(hybridBands == 64 || hybridBands == 32)"
            );
            self_.map_hyb_bands2_proc_bands = Some(&KERNELS_23_TO_64[..]);
            self_.map_proc_bands2_hyb_bands = Some(&KERNELS_23_TO_64_OFFSET[..]);
            self_.parameter_bands = 23;
        }
        _ => return 1,
    }
    self_.qs_next = self_.map_proc_bands2_hyb_bands.map(|t| &t[1..]);

    self_.max_val_direct_data = fl2fxconst_dbl(-1.0);
    self_.max_val_reverb_data = fl2fxconst_dbl(-1.0);
    self_.scale_direct_nrg = (2 * DUCKER_MAX_NRG_SCALE) as i8;
    self_.scale_reverb_nrg = (2 * DUCKER_MAX_NRG_SCALE) as i8;
    self_.scale_smooth_dir_rev_nrg = (2 * DUCKER_MAX_NRG_SCALE) as i8;
    self_.headroom_smooth_dir_rev_nrg = (2 * DUCKER_MAX_NRG_SCALE) as i8;
    self_.hybrid_bands = hybrid_bands;
    self_.partially_complex = partially_complex;

    if init_states_flag != 0 && ducker_type == DUCKER_PS {
        for pb in 0..self_.parameter_bands as usize {
            self_.smooth_dir_rev_nrg[pb] = 0;
        }
    }

    error_code
}

/// `&v[start]` with `len` values (the C++ code only forms the pointer for `len == 0`).
#[inline(always)]
fn sub(v: &[FixpDbl], start: i32, len: i32) -> &[FixpDbl] {
    if len <= 0 {
        &[]
    } else {
        &v[start as usize..(start + len) as usize]
    }
}

/// `DuckerCalcEnergy(self, inputReal, inputImag, energy, inputMaxVal, nrgScale, mode,
/// startHybBand)`; `mode` 1: PS, 0: else.
///
/// The straight port, kept as the reference of the optimized [`ducker_calc_energy`].
pub fn ducker_calc_energy_straight(
    self_: &DuckerInstance,
    input_real: &[FixpDbl],
    input_imag: &[FixpDbl],
    energy: &mut [FixpDbl; MAX_PARAMETER_BANDS],
    input_max_val: FixpDbl,
    nrg_scale: &mut i8,
    mode: i32,
    start_hyb_band: i32,
) -> i32 {
    let err = 0;
    let max_hybrid_band = self_.hybrid_bands - 1;
    let map = self_.map_hyb_bands2_proc_bands.unwrap_or(&[]);

    let max_hyb_band = max_hybrid_band;

    energy.fill(0);

    let mut clz: i32;
    let max_val = if mode == 1 { fl2fxconst_dbl(-1.0) } else { input_max_val };
    if max_val == fl2fxconst_dbl(-1.0) {
        clz = f_min(
            get_scalefactor(sub(
                input_real,
                start_hyb_band,
                f_max(0, max_hybrid_band - start_hyb_band + 1),
            )),
            get_scalefactor(sub(
                input_imag,
                start_hyb_band,
                f_max(0, max_hyb_band - start_hyb_band + 1),
            )),
        );
    } else {
        clz = cnt_leading_zeros(max_val) - 1;
    }

    clz = f_min(f_max(0, clz - DUCKER_HEADROOM_BITS), DUCKER_MAX_NRG_SCALE);
    // (SCHAR)clz << 1, stored to SCHAR
    *nrg_scale = ((clz as i8 as i32) << 1) as i8;

    let sh = clz as u32;
    let band_nrg = |e: FixpDbl, qs: usize| -> FixpDbl {
        saturate_left_shift(
            (e >> 1)
                .wrapping_add(f_pow2_div2(input_real[qs].wrapping_shl(sh)) >> 1)
                .wrapping_add(f_pow2_div2(input_imag[qs].wrapping_shl(sh)) >> 1),
            1,
            DFRACT_BITS,
        )
    };

    if mode == 1 {
        // Initialize pb since it would stay uninitialized for the case startHybBand >
        // maxHybBand.
        let mut pb = spatial_dec_get_processing_band(max_hyb_band, map);
        let mut qs = start_hyb_band;
        while qs <= max_hyb_band {
            pb = spatial_dec_get_processing_band(qs, map);
            energy[pb as usize] = band_nrg(energy[pb as usize], qs as usize);
            qs += 1;
        }
        pb += 1;

        // Not reached: pb > SpatialDecGetProcessingBand(maxHybridBand) after the loop above.
        let qs_next_tab = self_.qs_next.unwrap_or(&[]);
        while pb <= spatial_dec_get_processing_band(max_hybrid_band, map) {
            let qs_next = qs_next_tab[pb as usize] as i32;
            let mut nrg: FixpDbl = 0;
            while qs < qs_next {
                nrg = f_add_saturate(nrg, f_pow2_div2(input_real[qs as usize].wrapping_shl(sh)));
                qs += 1;
            }
            energy[pb as usize] = nrg;
            pb += 1;
        }
    } else {
        let mut qs = start_hyb_band;
        while qs <= max_hyb_band {
            let pb = spatial_dec_get_processing_band(qs, map) as usize;
            energy[pb] = band_nrg(energy[pb], qs as usize);
            qs += 1;
        }

        // Not reached: qs > maxHybridBand after the loop above.
        while qs <= max_hybrid_band {
            let pb = spatial_dec_get_processing_band(qs, map) as usize;
            energy[pb] =
                f_add_saturate(energy[pb], f_pow2_div2(input_real[qs as usize].wrapping_shl(sh)));
            qs += 1;
        }
    }

    // Catch overflows which have been observed in erred bitstreams to avoid assertion failures
    // later.
    for e in energy.iter_mut() {
        *e &= MAXVAL_DBL;
    }
    err
}

/// `DuckerCalcEnergy()`, optimized (see [`ducker_calc_energy_straight`]).
///
/// With `startHybBand <= maxHybridBand` (always in the library) the trailing loops of the
/// straight port are not reached. The band energies are then computed in two passes: the terms
/// `(fPow2Div2(re << clz) >> 1) + (fPow2Div2(im << clz) >> 1)` of every hybrid band
/// (independent, vectorizable), then the saturating accumulation per parameter band in closed
/// form (see below). The additions do not wrap, so adding the sum of the two terms equals adding
/// them one by one. A parameter band outside `energy` panics as in the straight port.
pub fn ducker_calc_energy(
    self_: &DuckerInstance,
    input_real: &[FixpDbl],
    input_imag: &[FixpDbl],
    energy: &mut [FixpDbl; MAX_PARAMETER_BANDS],
    input_max_val: FixpDbl,
    nrg_scale: &mut i8,
    mode: i32,
    start_hyb_band: i32,
) -> i32 {
    let max_hybrid_band = self_.hybrid_bands - 1;
    let s = start_hyb_band;
    let fast = match self_.map_hyb_bands2_proc_bands {
        Some(map) if s >= 0 && s <= max_hybrid_band => {
            let m = max_hybrid_band as usize;
            m <= MAX_HYBRID_BANDS && m < map.len() && m < input_real.len() && m < input_imag.len()
        }
        _ => false,
    };
    if !fast {
        return ducker_calc_energy_straight(
            self_,
            input_real,
            input_imag,
            energy,
            input_max_val,
            nrg_scale,
            mode,
            start_hyb_band,
        );
    }
    let (s, m) = (s as usize, max_hybrid_band as usize);
    let map = &self_.map_hyb_bands2_proc_bands.unwrap_or(&[])[s..=m];
    let re = &input_real[s..=m];
    let im = &input_imag[s..=m];
    let n = m + 1 - s;

    let max_val = if mode == 1 { fl2fxconst_dbl(-1.0) } else { input_max_val };
    let mut clz = if max_val == fl2fxconst_dbl(-1.0) {
        // min(getScalefactor(re), getScalefactor(im)) is getScalefactor() of both together
        let mut max_val: FixpDbl = 0;
        for (&r, &i) in re.iter().zip(im) {
            max_val |= (r ^ (r >> (DFRACT_BITS - 1))) | (i ^ (i >> (DFRACT_BITS - 1)));
        }
        fix_max(0, cnt_leading_zeros(max_val) - 1)
    } else {
        cnt_leading_zeros(max_val) - 1
    };
    clz = f_min(f_max(0, clz - DUCKER_HEADROOM_BITS), DUCKER_MAX_NRG_SCALE);
    // (SCHAR)clz << 1, stored to SCHAR
    *nrg_scale = ((clz as i8 as i32) << 1) as i8;
    let sh = clz as u32;

    let mut terms = [0 as FixpDbl; MAX_HYBRID_BANDS + 1];
    ducker_terms(&mut terms[..n], re, im, sh);

    // Per run of hybrid bands of the same parameter band (start value e0 = energy[pb]), the
    // straight port iterates e = SATURATE_LEFT_SHIFT((e >> 1) + t, 1). The terms are in
    // 0..=2^30 and e stays in 0..=MAXVAL_DBL, so nothing wraps; with P = (e0 >> 1) + sum(t) the
    // result is 2 * P if P <= MAXVAL_DBL >> 1 (no step saturates, P only grows), otherwise
    // MAXVAL_DBL if the last term is not 0, else MAXVAL_DBL - 1 (after the first saturating
    // step, which has a term > 0, a term > 0 saturates again and a term 0 gives
    // (MAXVAL_DBL >> 1) << 1). Both saturation formulas (aarch64, x86) give these values.
    let fin = |p: i64, t: FixpDbl| -> FixpDbl {
        if p <= (MAXVAL_DBL >> 1) as i64 {
            (2 * p) as FixpDbl
        } else if t != 0 {
            MAXVAL_DBL
        } else {
            MAXVAL_DBL - 1
        }
    };
    // A parameter band can occur in several runs (not in the maps of the library): each run
    // starts from its energy.
    energy.fill(0);
    let mut acc: i64 = 0;
    for k in 0..n {
        let t = terms[k];
        acc += t as i64;
        let pb = map[k];
        if k + 1 == n || map[k + 1] != pb {
            let e = &mut energy[pb as usize];
            *e = fin((*e >> 1) as i64 + acc, t);
            acc = 0;
        }
    }

    // Catch overflows which have been observed in erred bitstreams to avoid assertion failures
    // later.
    for e in energy.iter_mut() {
        *e &= MAXVAL_DBL;
    }
    0
}

crate::avx2_clone! {
/// The terms `(fPow2Div2(re << sh) >> 1) + (fPow2Div2(im << sh) >> 1)` of
/// [`ducker_calc_energy`] (vectorized; on x86 also compiled with AVX2, whose `vpmuldq` gives the
/// 64-bit products, the same values).
fn ducker_terms(terms: &mut [FixpDbl], re: &[FixpDbl], im: &[FixpDbl], sh: u32) {
    for ((t, &r), &i) in terms.iter_mut().zip(re).zip(im) {
        *t = (f_pow2_div2(r.wrapping_shl(sh)) >> 1)
            .wrapping_add(f_pow2_div2(i.wrapping_shl(sh)) >> 1);
    }
}
}

/// `fMax(fMin(x, DFRACT_BITS - 1), -(DFRACT_BITS - 1))`
#[inline(always)]
fn clamp_shift(x: i32) -> i32 {
    f_max(f_min(x, DFRACT_BITS - 1), -(DFRACT_BITS - 1))
}

/// `DuckerApply(self, directNrg, outputReal, outputImag, startHybBand)`: the MPS ducker.
fn ducker_apply(
    self_: &mut DuckerInstance,
    direct_nrg: &[FixpDbl; MAX_PARAMETER_BANDS],
    output_real: &mut [FixpDbl],
    output_imag: &mut [FixpDbl],
    start_hyb_band: i32,
) -> i32 {
    let err = 0;
    let mut qs = start_hyb_band;
    let hybrid_bands = self_.hybrid_bands;

    let mut reverb_nrg = [0 as FixpDbl; MAX_PARAMETER_BANDS];

    let mut do_scale_nrg = false;
    let mut scale_direct_nrg = 0;
    let mut scale_reverb_nrg = 0;
    let mut scale_smooth_dir_rev_nrg = 0;
    let mut max_dir_rev_nrg: FixpDbl = 0;

    let hyb_bands = hybrid_bands;
    let map = self_.map_hyb_bands2_proc_bands.unwrap_or(&[]);
    let qs_next_tab = self_.qs_next.unwrap_or(&[]);

    let start_param_band = spatial_dec_get_processing_band(start_hyb_band, map);

    let mut nrg_scale = self_.scale_reverb_nrg;
    ducker_calc_energy(
        self_,
        output_real,
        output_imag,
        &mut reverb_nrg,
        self_.max_val_reverb_data,
        &mut nrg_scale,
        0,
        start_hyb_band,
    );
    self_.scale_reverb_nrg = nrg_scale;

    if (self_.scale_direct_nrg != self_.scale_reverb_nrg)
        || (self_.scale_direct_nrg != self_.scale_smooth_dir_rev_nrg)
        || (self_.headroom_smooth_dir_rev_nrg == 0)
    {
        let mut scale = fix_min(
            self_.scale_direct_nrg as i32,
            self_.scale_smooth_dir_rev_nrg as i32 + self_.headroom_smooth_dir_rev_nrg as i32 - 1,
        );
        scale = fix_min(scale, self_.scale_reverb_nrg as i32);

        scale_direct_nrg = clamp_shift(self_.scale_direct_nrg as i32 - scale);
        scale_reverb_nrg = clamp_shift(self_.scale_reverb_nrg as i32 - scale);
        scale_smooth_dir_rev_nrg = clamp_shift(self_.scale_smooth_dir_rev_nrg as i32 - scale);

        self_.scale_smooth_dir_rev_nrg = scale as i8;

        do_scale_nrg = true;
    }
    let mut pb = start_param_band;
    while pb < self_.parameter_bands {
        let p = pb as usize;
        let mut s: i32 = 0;
        let duck_gain: FixpDuckGain;

        // smoothDirRevNrg[2*pb  ] = fMult(smoothDirRevNrg[2*pb  ],DUCK_ALPHA_FDK) +
        //   fMultDiv2(directNrg[pb],DUCK_ONE_MINUS_ALPHA_X4_FDK);
        // smoothDirRevNrg[2*pb+1] = fMult(smoothDirRevNrg[2*pb+1],DUCK_ALPHA_FDK) +
        //   fMultDiv2(reverbNrg[pb],DUCK_ONE_MINUS_ALPHA_X4_FDK);
        // tmp1 = fMult(smoothDirRevNrg[2*pb],DUCK_GAMMA_FDK);
        // tmp2 = smoothDirRevNrg[2*pb+1] >> 1;
        let mut tmp1 = self_.smooth_dir_rev_nrg[2 * p];
        let mut tmp2 = self_.smooth_dir_rev_nrg[2 * p + 1];
        tmp1 = f_mult(tmp1, DUCK_ALPHA_FDK);
        tmp2 = f_mult(tmp2, DUCK_ALPHA_FDK);

        if do_scale_nrg {
            let scale_smooth_dir_rev_nrg_as_exponent = -scale_smooth_dir_rev_nrg;

            tmp1 = scale_value(tmp1, scale_smooth_dir_rev_nrg_as_exponent);
            tmp2 = scale_value(tmp2, scale_smooth_dir_rev_nrg_as_exponent);
            tmp1 = f_mult_add_div2(
                tmp1,
                scale_value(direct_nrg[p], -scale_direct_nrg),
                DUCK_ONE_MINUS_ALPHA_X4_FDK,
            );
            tmp2 = f_mult_add_div2(
                tmp2,
                scale_value(reverb_nrg[p], -scale_reverb_nrg),
                DUCK_ONE_MINUS_ALPHA_X4_FDK,
            );
        } else {
            tmp1 = f_mult_add_div2(tmp1, direct_nrg[p], DUCK_ONE_MINUS_ALPHA_X4_FDK);
            tmp2 = f_mult_add_div2(tmp2, reverb_nrg[p], DUCK_ONE_MINUS_ALPHA_X4_FDK);
        }

        self_.smooth_dir_rev_nrg[2 * p] = tmp1;
        self_.smooth_dir_rev_nrg[2 * p + 1] = tmp2;

        max_dir_rev_nrg |= f_abs(tmp1);
        max_dir_rev_nrg |= f_abs(tmp2);

        tmp1 = f_mult(tmp1, DUCK_GAMMA_FDK);
        tmp2 >>= 1;

        let qs_next = f_min(qs_next_tab[p] as i32, self_.hybrid_bands);

        if tmp2 > tmp1 {
            // true for about 20%
            // gain smaller than 1.0
            tmp1 = sqrt_fixp(tmp1);
            tmp2 = inv_sqrt_norm2(tmp2, &mut s);
            duck_gain = fx_dbl2fx_sgl(f_mult_div2(tmp1, tmp2).wrapping_shl(s as u32));
        } else {
            // true for about 80 %
            tmp2 = self_.smooth_dir_rev_nrg[2 * p] >> 1;
            tmp1 = f_mult(self_.smooth_dir_rev_nrg[2 * p + 1], DUCK_GAMMA_FDK);
            if tmp2 > tmp1 {
                // true for about 20%
                if tmp1 <= (tmp2 >> 2) {
                    // limit gain to 2.0
                    if qs < hyb_bands {
                        while qs < qs_next {
                            output_real[qs as usize] = output_real[qs as usize].wrapping_shl(1);
                            output_imag[qs as usize] = output_imag[qs as usize].wrapping_shl(1);
                            qs += 1;
                        }
                    } else {
                        while qs < qs_next {
                            output_real[qs as usize] = output_real[qs as usize].wrapping_shl(1);
                            qs += 1;
                        }
                    }
                    // skip general gain*output section
                    pb += 1;
                    continue;
                } else {
                    // gain from 1.0 to 2.0
                    tmp2 = sqrt_fixp(tmp2 >> 2);
                    tmp1 = inv_sqrt_norm2(tmp1, &mut s);
                    duck_gain = fx_dbl2fx_sgl(f_mult(tmp1, tmp2).wrapping_shl(s as u32));
                }
            } else {
                // true for about 60%
                // gain = 1.0; output does not change; update qs index
                qs = qs_next;
                pb += 1;
                continue;
            }
        }

        // general gain*output section
        if qs < hyb_bands {
            // true for about 39%
            while qs < qs_next {
                // runs about 2 times
                output_real[qs as usize] =
                    f_mult_div2(output_real[qs as usize], duck_gain).wrapping_shl(2);
                output_imag[qs as usize] =
                    f_mult_div2(output_imag[qs as usize], duck_gain).wrapping_shl(2);
                qs += 1;
            }
        } else {
            while qs < qs_next {
                output_real[qs as usize] =
                    f_mult_div2(output_real[qs as usize], duck_gain).wrapping_shl(2);
                qs += 1;
            }
        }
        pb += 1;
    } // pb

    self_.headroom_smooth_dir_rev_nrg = fix_max(0, cnt_leading_zeros(max_dir_rev_nrg) - 1) as i8;

    err
}

/// `DuckerApplyPS(self, directNrg, outputReal, outputImag, startHybBand)`: the PS ducker.
fn ducker_apply_ps(
    self_: &mut DuckerInstance,
    direct_nrg: &[FixpDbl; MAX_PARAMETER_BANDS],
    output_real: &mut [FixpDbl],
    output_imag: &mut [FixpDbl],
    start_hyb_band: i32,
) -> i32 {
    let mut qs = start_hyb_band;
    let map = self_.map_hyb_bands2_proc_bands.unwrap_or(&[]);
    let map2 = self_.map_proc_bands2_hyb_bands.unwrap_or(&[]);
    let qs_next_tab = self_.qs_next.unwrap_or(&[]);
    let start_param_band = spatial_dec_get_processing_band(start_hyb_band, map);

    let mut do_scale_nrg = false;
    let mut scale_direct_nrg = 0;
    let mut scale_smooth_dir_rev_nrg = 0;
    let mut max_dir_rev_nrg: FixpDbl = 0;

    if (self_.scale_direct_nrg != self_.scale_smooth_dir_rev_nrg)
        || (self_.headroom_smooth_dir_rev_nrg == 0)
    {
        let scale = fix_min(
            self_.scale_direct_nrg as i32,
            self_.scale_smooth_dir_rev_nrg as i32 + self_.headroom_smooth_dir_rev_nrg as i32 - 2,
        );

        scale_direct_nrg = clamp_shift(self_.scale_direct_nrg as i32 - scale);
        scale_smooth_dir_rev_nrg = clamp_shift(self_.scale_smooth_dir_rev_nrg as i32 - scale);

        self_.scale_smooth_dir_rev_nrg = scale as i8;

        do_scale_nrg = true;
    }

    let hyb_bands = self_.hybrid_bands;

    fdk_assert!(
        (self_.parameter_bands == 28) || (self_.parameter_bands == 20),
        "FDK_ASSERT((self->parameterBands == (28)) || (self->parameterBands == (20)))"
    );
    for pb in start_param_band.max(0) as usize..self_.parameter_bands.max(0) as usize {
        let mut direct_nrg2 = direct_nrg[pb];

        if do_scale_nrg {
            direct_nrg2 = scale_value(direct_nrg2, -scale_direct_nrg);
            self_.peak_diff[pb] = scale_value(self_.peak_diff[pb], -scale_smooth_dir_rev_nrg);
            self_.peak_decay[pb] = scale_value(self_.peak_decay[pb], -scale_smooth_dir_rev_nrg);
            self_.smooth_dir_rev_nrg[pb] =
                scale_value(self_.smooth_dir_rev_nrg[pb], -scale_smooth_dir_rev_nrg);
        }
        self_.peak_decay[pb] =
            fix_max(direct_nrg2, f_mult(self_.peak_decay[pb], PS_DUCK_PEAK_DECAY_FACTOR_FDK));
        self_.peak_diff[pb] = self_.peak_diff[pb].wrapping_add(f_mult(
            PS_DUCK_FILTER_COEFF_FDK,
            self_.peak_decay[pb].wrapping_sub(direct_nrg2).wrapping_sub(self_.peak_diff[pb]),
        ));
        self_.smooth_dir_rev_nrg[pb] = fix_max(
            self_.smooth_dir_rev_nrg[pb].wrapping_add(f_mult(
                PS_DUCK_FILTER_COEFF_FDK,
                direct_nrg2.wrapping_sub(self_.smooth_dir_rev_nrg[pb]),
            )),
            0,
        );

        max_dir_rev_nrg |= f_abs(self_.peak_diff[pb]);
        max_dir_rev_nrg |= f_abs(self_.smooth_dir_rev_nrg[pb]);

        if (self_.peak_diff[pb] == 0) && (self_.smooth_dir_rev_nrg[pb] == 0) {
            qs = f_max(qs, spatial_dec_get_qmf_band(pb as i32, map2));
            let qs_next = f_min(qs_next_tab[pb] as i32, self_.hybrid_bands);

            if qs < hyb_bands {
                while qs < qs_next {
                    output_real[qs as usize] = 0;
                    output_imag[qs as usize] = 0;
                    qs += 1;
                }
            } else {
                while qs < qs_next {
                    output_real[qs as usize] = 0;
                    qs += 1;
                }
            }
        } else if self_.peak_diff[pb] != 0 {
            let multiplication = f_mult(fl2fxconst_sgl(0.75f32 as f64), self_.peak_diff[pb]);
            if multiplication > (self_.smooth_dir_rev_nrg[pb] >> 1) {
                let mut scale = 0;

                // implement x/y as (sqrt(x)*invSqrt(y))^2
                let num = sqrt_fixp(self_.smooth_dir_rev_nrg[pb] >> 1);
                let mut denom = self_.peak_diff[pb].wrapping_add(ABS_THR_PS);
                denom = inv_sqrt_norm2(denom, &mut scale);

                // duck output whether duckGain != 1.f
                qs = f_max(qs, spatial_dec_get_qmf_band(pb as i32, map2));
                let qs_next = f_min(qs_next_tab[pb] as i32, self_.hybrid_bands);

                let mut duck_gain = f_mult(num, denom);
                duck_gain = f_pow2_div2(duck_gain.wrapping_shl(scale as u32));
                duck_gain = f_mult_div2(fl2fxconst_sgl((2.0f32 / 3.0f32) as f64), duck_gain)
                    .wrapping_shl(3);

                if qs < hyb_bands {
                    while qs < qs_next {
                        output_real[qs as usize] = f_mult(output_real[qs as usize], duck_gain);
                        output_imag[qs as usize] = f_mult(output_imag[qs as usize], duck_gain);
                        qs += 1;
                    }
                } else {
                    while qs < qs_next {
                        output_real[qs as usize] = f_mult(output_real[qs as usize], duck_gain);
                        qs += 1;
                    }
                }
            }
        }
    } // pb

    self_.headroom_smooth_dir_rev_nrg = fix_max(0, cnt_leading_zeros(max_dir_rev_nrg) - 1) as i8;

    0
}

/// `FDKdecorrelateOpen(hDecorrDec, bufferCplx, bufLen)`: creates one instance of the
/// decorrelator with the externally allocated buffer `buffer_cplx` (`bufLen` is its length,
/// at least [`DECORR_BUFFER_LEN`]). Returns 0 on success.
pub fn fdk_decorrelate_open(h_decorr_dec: &mut DecorrDec, buffer_cplx: Box<[FixpDbl]>) -> i32 {
    let self_ = h_decorr_dec;

    if (buffer_cplx.len() as i64) < DECORR_BUFFER_LEN as i64 {
        return 1;
    }

    // assign all memory to stateBufferCplx. It is reassigned during FDKdecorrelateInit()
    self_.state_buffer_cplx = Some(buffer_cplx);
    self_.l_state_buffer_cplx = 0;

    self_.delay_buffer_cplx = None;
    self_.l_delay_buffer_cplx = 0;

    0
}

/// `distributeBuffer(self, L_stateBuf, L_delayBuf)`
fn distribute_buffer(self_: &mut DecorrDec, l_state_buf: i32, l_delay_buf: i32) -> i32 {
    // factor 2 because of complex values
    if DECORR_BUFFER_LEN < 2 * (l_state_buf + l_delay_buf) {
        return 1;
    }

    self_.l_state_buffer_cplx = 2 * l_state_buf;
    self_.delay_buffer_cplx = Some(2 * l_state_buf as usize);
    self_.l_delay_buffer_cplx = 2 * l_delay_buf;

    0
}

/// `FDKdecorrelateInit(hDecorrDec, nrHybBands, decorrType, duckerType, decorrConfig, seed,
/// partiallyComplex, useFractDelay, isLegacyPS, initStatesFlag)`: initializes and configures the
/// decorrelator. Returns 0 on success.
pub fn fdk_decorrelate_init(
    h_decorr_dec: &mut DecorrDec,
    nr_hyb_bands: i32,
    decorr_type: FdkDecorrType,
    ducker_type: FdkDuckerType,
    decorr_config: i32,
    seed: i32,
    partially_complex: i32,
    use_fract_delay: i32,
    is_legacy_ps: i32,
    init_states_flag: i32,
) -> i32 {
    let mut error_code;
    let mut n_param_bands = 28;

    let mut offset_state_buffer = 0;
    let mut offset_delay_buffer = 0;

    h_decorr_dec.partially_complex = partially_complex;
    h_decorr_dec.numbins = nr_hyb_bands;

    match decorr_type {
        DECORR_PS => {
            // ignore decorrConfig, seed
            if partially_complex != 0 {
                h_decorr_dec.rev_band_offset = Some(&REV_BAND_OFFSET_PS_LP);
                h_decorr_dec.rev_delay = Some(&REV_DELAY_PS_LP);
                error_code = distribute_buffer(h_decorr_dec, 168, 533);
            } else {
                h_decorr_dec.rev_band_offset = Some(&REV_BAND_OFFSET_PS_HQ);
                h_decorr_dec.rev_delay = Some(&REV_DELAY_PS_HQ);
                error_code = distribute_buffer(h_decorr_dec, 360, 257);
            }
            h_decorr_dec.rev_filter_order = Some(&REV_FILTER_ORDER_PS);
            h_decorr_dec.rev_filt_type = Some(&REV_FILT_TYPE_PS);

            // Initialize ring buffer offsets for PS specific filter implementation.
            h_decorr_dec.state_buffer_offset = STATE_BUFFER_OFFSET_INIT;
        }
        DECORR_USAC => {
            if partially_complex != 0 {
                return 1;
            }
            if seed != 0 {
                return 1;
            }
            // reverb band layout is inherited from MPS standard. Deviation: an index outside
            // of the table returns 1.
            let Some(band_offset) = REV_BAND_OFFSET_MPS_HQ.get(decorr_config as usize) else {
                return 1;
            };
            h_decorr_dec.rev_band_offset = Some(band_offset);
            h_decorr_dec.rev_filter_order = Some(&REV_FILTER_ORDER_USAC);
            h_decorr_dec.rev_delay = Some(&REV_DELAY_USAC);
            if use_fract_delay != 0 {
                return 1; // not yet supported
            } else {
                // the filter types are inherited from MPS standard
                h_decorr_dec.rev_filt_type = Some(&REV_FILT_TYPE_MPS);
            }
            // bsDecorrConfig == 1 is worst case
            error_code = distribute_buffer(h_decorr_dec, 509, 643);
        }
        DECORR_LD => {
            if partially_complex != 0 {
                return 1;
            }
            if use_fract_delay != 0 {
                return 1;
            }
            if decorr_config > 2 {
                return 1;
            }
            if seed > (MAX_DECORR_SEED_LD - 1) {
                return 1;
            }
            if !(nr_hyb_bands == 64 || nr_hyb_bands == 32) {
                return 1; // actually just qmf bands and no hybrid bands
            }
            // Deviation: a negative decorrConfig returns 1.
            let Some(band_offset) = REV_BAND_OFFSET_LD.get(decorr_config as usize) else {
                return 1;
            };
            h_decorr_dec.rev_band_offset = Some(band_offset);
            // the filter orders are inherited from MPS standard
            h_decorr_dec.rev_filter_order = Some(&REV_FILTER_ORDER_MPS);
            // the delays in each reverb band are inherited from MPS standard
            h_decorr_dec.rev_delay = Some(&REV_DELAY_MPS);
            h_decorr_dec.rev_filt_type = Some(&REV_FILT_TYPE_LD);
            error_code = distribute_buffer(h_decorr_dec, 825, 373);
        }
        _ => return 1,
    }

    if error_code != 0 {
        return error_code;
    }

    // Deviation: without a buffer (no defined result in C++) the port returns 1.
    let Some(buf) = h_decorr_dec.state_buffer_cplx.as_deref_mut() else {
        return 1;
    };
    let delay_base = h_decorr_dec.delay_buffer_cplx.unwrap_or(0);

    if init_states_flag != 0 {
        let l_state = h_decorr_dec.l_state_buffer_cplx as usize;
        let l_delay = h_decorr_dec.l_delay_buffer_cplx as usize;
        buf[..l_state].fill(0);
        buf[delay_base..delay_base + l_delay].fill(0);
        h_decorr_dec.reverb_band_delay_buffer_index = [0; NUM_REVERB_BANDS];
    }

    let rev_band_offset = h_decorr_dec.rev_band_offset.unwrap_or(&[0; NUM_REVERB_BANDS]);
    let rev_filter_order = h_decorr_dec.rev_filter_order.unwrap_or(&[0; NUM_REVERB_BANDS]);
    let rev_delay = h_decorr_dec.rev_delay.unwrap_or(&[0; NUM_REVERB_BANDS]);

    let mut i_start = 0;
    for rb in 0..NUM_REVERB_BANDS {
        let i_stop = rev_band_offset[rb] as i32;

        if i_stop <= i_start {
            continue;
        }

        for i in i_start..i_stop {
            // The C++ code assigns errorCode, which is overwritten below without being read.
            let _error_code = match decorr_type {
                DECORR_PS => decorr_filter_init_ps(
                    &mut h_decorr_dec.filter[i as usize],
                    0,
                    delay_base,
                    &mut offset_state_buffer,
                    &mut offset_delay_buffer,
                    i,
                    rb as i32,
                    rev_delay[rb] as i32,
                ),
                _ => decorr_filter_init(
                    &mut h_decorr_dec.filter[i as usize],
                    0,
                    delay_base,
                    &mut offset_state_buffer,
                    &mut offset_delay_buffer,
                    seed,
                    rb as i32,
                    use_fract_delay,
                    rev_delay[rb] as i32,
                    rev_filter_order[rb] as i32,
                    decorr_type,
                ),
            };
        }

        i_start = i_stop;
    } // loop over reverbBands

    if !(offset_state_buffer <= h_decorr_dec.l_state_buffer_cplx)
        || !(offset_delay_buffer <= h_decorr_dec.l_delay_buffer_cplx)
    {
        return 1;
    }

    if ducker_type == DUCKER_AUTOMATIC {
        // Choose correct ducker type according to standards:
        match decorr_type {
            DECORR_PS => {
                h_decorr_dec.ducker.ducker_type = DUCKER_PS;
                n_param_bands = if is_legacy_ps != 0 { 20 } else { 28 };
            }
            DECORR_USAC => {
                h_decorr_dec.ducker.ducker_type = DUCKER_MPS;
                n_param_bands = 28;
            }
            DECORR_LD => {
                h_decorr_dec.ducker.ducker_type = DUCKER_MPS;
                n_param_bands = 23;
            }
            _ => return 1,
        }
    }

    let ducker_type = h_decorr_dec.ducker.ducker_type;
    error_code = ducker_init(
        &mut h_decorr_dec.ducker,
        h_decorr_dec.numbins,
        h_decorr_dec.partially_complex,
        ducker_type,
        n_param_bands,
        init_states_flag,
    );

    error_code
}

/// `FDKdecorrelateClose(hDecorrDec)`: releases the buffer. Returns 0.
pub fn fdk_decorrelate_close(h_decorr_dec: &mut DecorrDec) -> i32 {
    let err = 0;

    h_decorr_dec.state_buffer_cplx = None;
    h_decorr_dec.l_state_buffer_cplx = 0;
    h_decorr_dec.delay_buffer_cplx = None;
    h_decorr_dec.l_delay_buffer_cplx = 0;

    err
}

/// `FDKdecorrelateApply(hDecorrDec, dataRealIn, dataImagIn, dataRealOut, dataImagOut,
/// startHybBand)` with separate input and output buffers: applies the decorrelator and the
/// ducker. Returns 0 on success.
///
/// For the in-place call of the C++ API (`dataRealOut == dataRealIn`), use
/// [`fdk_decorrelate_apply_in_place`].
pub fn fdk_decorrelate_apply(
    h_decorr_dec: &mut DecorrDec,
    data_real_in: &[FixpDbl],
    data_imag_in: &[FixpDbl],
    data_real_out: &mut [FixpDbl],
    data_imag_out: &mut [FixpDbl],
    start_hyb_band: i32,
) -> i32 {
    let self_ = h_decorr_dec;
    let mut err;

    // Deviation: no defined result in C++ (see the module documentation).
    let (Some(rev_band_offset), Some(rev_filt_type), Some(rev_delay), Some(rev_filter_order)) =
        (self_.rev_band_offset, self_.rev_filt_type, self_.rev_delay, self_.rev_filter_order)
    else {
        return 1;
    };
    if self_.ducker.map_hyb_bands2_proc_bands.is_none() {
        return 1;
    }

    // copy new samples
    let n_hyb_bands = self_.numbins;

    let mut direct_nrg = [0 as FixpDbl; MAX_PARAMETER_BANDS];

    let mut nrg_scale = self_.ducker.scale_direct_nrg;
    ducker_calc_energy(
        &self_.ducker,
        data_real_in,
        data_imag_in,
        &mut direct_nrg,
        self_.ducker.max_val_direct_data,
        &mut nrg_scale,
        if self_.ducker.ducker_type == DUCKER_PS { 1 } else { 0 },
        start_hyb_band,
    );
    self_.ducker.scale_direct_nrg = nrg_scale;

    let mut empty: [FixpDbl; 0] = [];
    let buf: &mut [FixpDbl] = match self_.state_buffer_cplx.as_deref_mut() {
        Some(b) => b,
        None => &mut empty,
    };

    // complex-valued hybrid bands
    let mut stop = 0;
    for rb in 0..NUM_REVERB_BANDS {
        let start = f_max(stop, start_hyb_band);
        // fMin(UCHAR, (UCHAR)nHybBands)
        stop = f_min(rev_band_offset[rb], n_hyb_bands as u8) as i32;

        if start < stop {
            err = match rev_filt_type[rb] {
                DELAY => decorr_filter_apply_pass(
                    &self_.filter,
                    buf,
                    data_real_in,
                    Some(data_imag_in),
                    data_real_out,
                    data_imag_out,
                    start,
                    stop,
                    rev_delay[rb] as i32,
                    self_.reverb_band_delay_buffer_index[rb],
                ),
                INDEP_CPLX_PS => decorr_filter_apply_cplx_ps(
                    &self_.filter,
                    buf,
                    data_real_in,
                    data_imag_in,
                    data_real_out,
                    data_imag_out,
                    start,
                    stop,
                    rev_filter_order[rb] as i32,
                    rev_delay[rb] as i32,
                    self_.reverb_band_delay_buffer_index[rb],
                    &mut self_.state_buffer_offset,
                ),
                COMMON_REAL => decorr_filter_apply_real(
                    &self_.filter,
                    buf,
                    data_real_in,
                    data_imag_in,
                    data_real_out,
                    data_imag_out,
                    start,
                    stop,
                    rev_filter_order[rb] as i32,
                    rev_delay[rb] as i32,
                    self_.reverb_band_delay_buffer_index[rb],
                ),
                _ => 1,
            };
            if err != 0 {
                return err;
            }
        } // if start < stop
    } // loop over reverb bands

    for rb in 0..NUM_REVERB_BANDS {
        self_.reverb_band_delay_buffer_index[rb] += 2;
        if self_.reverb_band_delay_buffer_index[rb] >= 2 * rev_delay[rb] as i32 {
            self_.reverb_band_delay_buffer_index[rb] = 0;
        }
    }

    match self_.ducker.ducker_type {
        DUCKER_PS => {
            err = ducker_apply_ps(
                &mut self_.ducker,
                &direct_nrg,
                data_real_out,
                data_imag_out,
                start_hyb_band,
            );
        }
        _ => {
            err = ducker_apply(
                &mut self_.ducker,
                &direct_nrg,
                data_real_out,
                data_imag_out,
                start_hyb_band,
            );
        }
    }

    err
}

/// `FDKdecorrelateApply(hDecorrDec, dataReal, dataImag, dataReal, dataImag, startHybBand)`: the
/// in-place call (output buffers equal to the input buffers).
///
/// Equal to [`fdk_decorrelate_apply`] with a copy of the input: every value of the input is read
/// before the output value of the same band is written, and no input value is read after that
/// (the ducker energies are computed first, each filter reads band `i` before it writes band
/// `i`, and the ducker works on the output).
pub fn fdk_decorrelate_apply_in_place(
    h_decorr_dec: &mut DecorrDec,
    data_real: &mut [FixpDbl],
    data_imag: &mut [FixpDbl],
    start_hyb_band: i32,
) -> i32 {
    let n = data_real.len().min(MAX_HYBRID_BANDS + 1);
    let m = data_imag.len().min(MAX_HYBRID_BANDS + 1);
    let mut real_in = [0 as FixpDbl; MAX_HYBRID_BANDS + 1];
    let mut imag_in = [0 as FixpDbl; MAX_HYBRID_BANDS + 1];
    real_in[..n].copy_from_slice(&data_real[..n]);
    imag_in[..m].copy_from_slice(&data_imag[..m]);
    fdk_decorrelate_apply(
        h_decorr_dec,
        &real_in[..n],
        &imag_in[..m],
        data_real,
        data_imag,
        start_hyb_band,
    )
}
