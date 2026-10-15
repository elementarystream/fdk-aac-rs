// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/usacdec_fac.h, libAACdec/src/usacdec_fac.cpp.

//! Port of `usacdec_fac.h`: USAC forward aliasing cancellation (FAC).
//!
//! The preprocessed `usacdec_fac.cpp` of the `aarch64` and `x86` builds are identical apart
//! from `FDK_ASSERT`; the variants differ only in the primitives and the DCTs.
//!
//! # Memory
//!
//! - The FAC data pointer `pFac` (`data.usac.fac_data[k]`) is a slice; the callers resolve the
//!   [`FacDataPtr`] of the channel ([`fac_data_slice`]).
//! - `output` is a C pointer into a buffer that is also read before its start (the past ACELP
//!   samples, `pOut0[-i-1]`): the port passes the buffer and the index of `output[0]`.
//! - The overlap buffer of `mdct_t` is used as `overlap.time` (from the start) and
//!   `overlap.freq` (from the end) at once, as in [`fdk_aac_rs_fdk::mdct`]. The port accesses
//!   it with indices, in the order of C. Accesses outside of the buffers (inconsistent MDCT
//!   states only) read 0 and are not written in the port, as in `imlt_block`.
//! - `pFacZir` of `mdct_t` is a pointer into the ACELP excitation memory in C (set by
//!   [`clpd_fac_acelp2mdct`]); the port stores a copy of the memory from that position to the end
//!   of the excitation memory ([`fdk_aac_rs_fdk::mdct::MdctT::p_fac_zir`]). This is exact
//!   because the excitation memory is not written between setting the pointer and consuming it:
//!   it is consumed by the next transform of the channel (the rest of the same call for
//!   `nSpec > 1`, [`clpd_fac_mdct2acelp`] for a following ACELP frame, `imlt_block()` for a
//!   following TCX or FD frame), which always precedes the next write of the memory
//!   (`CLpd_AcelpPrepareInternalMem()`, `CLpd_AcelpDecode()`, `CLpd_TcxTDConceal()`, or the
//!   next `CLpd_FAC_Acelp2Mdct()`, which only follows an ACELP frame).

use fdk_aac_rs_fdk::archdef::{FixpWtb, FixpWtp};
use fdk_aac_rs_fdk::bitstream::{fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::{
    cplx_mult_div2, f_add_saturate, f_min, f_mult, f_mult_div2, fixmin_i, FixpDbl, DFRACT_BITS,
};
use fdk_aac_rs_fdk::dct::{dct_iv, dst_iii, dst_iv};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::lpc::FixpLpc;
use fdk_aac_rs_fdk::mdct::{
    imdct_adapt_parameters, imdct_gain, imdct_scale_dbl, imdct_scale_dbl_lsh1, MdctT,
};
use fdk_aac_rs_fdk::scale::{
    get_scalefactor, scale_values, scale_values_saturate, scale_values_saturate_to,
};
use fdk_aac_rs_fdk::tools_rom::{
    fdk_get_window_slope, SINE_WINDOW_128, SINE_WINDOW_192, SINE_WINDOW_256, SINE_WINDOW_64,
    SINE_WINDOW_96,
};

use crate::channelinfo::{CAacDecoderChannelInfo, FacDataPtr};
use crate::usacdec_acelp::{clpd_acelp_get_free_exc_mem, clpd_acelp_zir, CAcelpStaticMem};
use crate::usacdec_const::{LFAC, LP_FILTER_SCALE, M_LP_FILTER_ORDER, NB_DIV};
use crate::usacdec_lpc::{clpc_decode_avq, clpd_decode_gain, e_lpc_a_weight};
use crate::usacdec_rom::{
    FAC_WINDOW_SYNTH_128, FAC_WINDOW_SYNTH_32, FAC_WINDOW_SYNTH_48, FAC_WINDOW_SYNTH_64,
    FAC_WINDOW_SYNTH_96, FAC_WINDOW_ZIR_128, FAC_WINDOW_ZIR_32, FAC_WINDOW_ZIR_48,
    FAC_WINDOW_ZIR_64, FAC_WINDOW_ZIR_96,
};

const M: usize = M_LP_FILTER_ORDER as usize;

/// `SPEC_FAC(ptr, i, gl)`: the offset `i * gl` of FAC data block `i`.
#[inline(always)]
pub const fn spec_fac(i: i32, gl: i32) -> usize {
    (i * gl) as usize
}

/// The memory a [`FacDataPtr`] points to: `fac_data0` of the channel, or the channel's spectrum
/// from the given offset (`spec`, rule 1 of [`crate::channelinfo`]). `None` for `NULL` and for
/// [`FacDataPtr::Scratch`] (a buffer of the function that set it).
pub fn fac_data_slice<'a>(
    ptr: FacDataPtr,
    fac_data0: &'a mut [FixpDbl],
    spec: &'a mut [FixpDbl],
) -> Option<&'a mut [FixpDbl]> {
    match ptr {
        FacDataPtr::Null | FacDataPtr::Scratch => None,
        FacDataPtr::FacData0 => Some(fac_data0),
        FacDataPtr::Spectral(o) => Some(match spec.get_mut(o..) {
            Some(s) => s,
            None => &mut [],
        }),
    }
}

/// `CLpd_FAC_GetMemory(pAacDecoderChannelInfo, mod, pState)`: get the address of a memory area
/// of the spectral data memory where the FAC data can be stored into. `p_state` must be 0 for
/// the first call and not changed externally.
///
/// Two FAC data blocks fit into each spectral data block of an ACELP division; if none is free,
/// `fac_data0` is used. The decoder never asks for more blocks than are free (`*pState` stays
/// at most 9).
pub fn clpd_fac_get_memory(
    p_aac_decoder_channel_info: &CAacDecoderChannelInfo,
    mod_: &[u8; NB_DIV as usize],
    p_state: &mut i32,
) -> FacDataPtr {
    let k = 0;
    let max_windows = 8;

    fdk_assert!(*p_state >= 0 && *p_state < max_windows);

    // Look for free space to store FAC data. 2 FAC data blocks fit into each TCX spectral data
    // block.
    let mut i = *p_state;
    while i < max_windows {
        if mod_[(i >> 1) as usize] == 0 {
            break;
        }
        i += 1;
    }

    *p_state = i + 1;

    if i == max_windows {
        FacDataPtr::FacData0
    } else {
        fdk_assert!(mod_.get((i >> 1) as usize).copied() == Some(0));
        FacDataPtr::Spectral(spec_fac(i, p_aac_decoder_channel_info.granule_length << k))
    }
}

/// `CLpd_FAC_Read(hBs, pFac, pFacScale, length, use_gain, frame)`: read a fac bitstream data
/// block into `p_fac` (`length` values) and its scale into `p_fac_scale[frame]`. If `use_gain`
/// is set, the FAC gain is read from the bitstream and applied. Returns 0 on success, -1 on
/// error.
pub fn clpd_fac_read(
    h_bs: &mut FdkBitStream,
    p_fac: &mut [FixpDbl],
    p_fac_scale: &mut [i8],
    length: i32,
    use_gain: i32,
    frame: i32,
) -> i32 {
    let mut fac_gain: FixpDbl = 0;
    let mut fac_gain_e: i32 = 0;

    if use_gain != 0 {
        let code = fdk_read_bits(h_bs, 7) as i32;
        clpd_decode_gain(&mut fac_gain, &mut fac_gain_e, code);
    }

    if clpc_decode_avq(h_bs, p_fac, 1, 1, length) != 0 {
        return -1;
    }

    // p_fac holds at least `length` values and frame is 0..3 in the decoder (otherwise no
    // defined reference result; the port stops at the ends of the arrays).
    let n = (length.max(0) as usize).min(p_fac.len());
    let mut fac_scale: i32;
    {
        let scale = get_scalefactor(&p_fac[..n]);
        scale_values(&mut p_fac[..n], scale);
        fac_scale = ((DFRACT_BITS - 1 - scale) as i8) as i32;
    }

    if use_gain != 0 {
        fac_scale = ((fac_scale + fac_gain_e) as i8) as i32;

        for i in 0..n {
            p_fac[i] = f_mult(p_fac[i], fac_gain);
        }
    }
    if let Some(sc) = p_fac_scale.get_mut(frame as usize) {
        *sc = fac_scale as i8;
    }
    0
}

/// `Syn_filt_zero(a, a_exp, length, x)`: apply the synthesis filter with zero input to `x` in
/// place. The overall filter gain is 1.0.
pub fn syn_filt_zero(a: &[FixpLpc], a_exp: i32, length: i32, x: &mut [FixpDbl]) {
    for i in 0..length.max(0) as usize {
        let mut l_tmp: FixpDbl = 0;

        for j in 0..f_min(i, M) {
            l_tmp = l_tmp.wrapping_sub(f_mult_div2(a[j], x[i - (j + 1)]) >> (LP_FILTER_SCALE - 1));
        }

        l_tmp = fdk_aac_rs_fdk::scale::scale_value(l_tmp, a_exp + LP_FILTER_SCALE);
        x[i] = f_add_saturate(x[i], l_tmp);
    }
}

/// `gainFac[4]`: the table is also correct for coreCoderFrameLength = 768. Factor 3/4 is
/// canceled out: gainFac = 0.5 * sqrt(fac_length/lFrame)
///
/// `scripts/dump_table.py libAACdec/src/usacdec_fac.cpp gainFac LONG --hex`
pub static GAIN_FAC: [FixpDbl; 4] = [0x40000000, 0x2D413CCD, 0x20000000, 0x16A09E66];

/// `CFac_ApplyGains(fac_data, fac_length, tcx_gain, alfd_gains, mod)`: apply the TCX and ALFD
/// gains to the FAC data. `mod_` is the mode (1,2,3) of the TCX frame where the FAC signal needs
/// to be applied.
pub fn cfac_apply_gains(
    fac_data: &mut [FixpDbl],
    fac_length: i32,
    tcx_gain: FixpDbl,
    alfd_gains: &[FixpDbl],
    mod_: i32,
) {
    fdk_assert!((fac_length == 128) || (fac_length == 96));

    // 2) Apply gain factor to FAC data
    // mod is 1, 2 or 3; other values read 0 (no defined reference result).
    let fac_factor = f_mult(GAIN_FAC.get(mod_ as usize).copied().unwrap_or(0), tcx_gain);
    for i in 0..fac_length.max(0) as usize {
        fac_data[i] = f_mult(fac_data[i], fac_factor);
    }

    // 3) Apply spectrum deshaping using alfd_gains
    for i in 0..(fac_length / 4).max(0) as usize {
        let k = i >> (3 - mod_);
        // alfd_gains is scaled by one bit.
        fac_data[i] = f_mult(fac_data[i], alfd_gains[k]) << 1;
    }
}

/// `CFac_CalcFacSignal(pOut, pFac, fac_scale, fac_length, A, A_exp, fAddZir, isFdFac)`:
/// compute the FAC signal (`2 * fac_length` values of `p_out`: the signal followed by its
/// zero input response). `p_fac` is transformed in place (DCT-IV).
pub fn cfac_calc_fac_signal(
    p_out: &mut [FixpDbl],
    p_fac: &mut [FixpDbl],
    fac_scale: i32,
    fac_length: i32,
    a: &[FixpLpc],
    a_exp: i32,
    _f_add_zir: i32,
    is_fd_fac: i32,
) {
    let mut wa = [0 as FixpLpc; M];
    let mut tf_gain: FixpDbl = 0;
    let mut scale = fac_scale;
    let n = fac_length as usize;

    // obtain tranform gain.
    imdct_gain(&mut tf_gain, &mut scale, if is_fd_fac != 0 { 0 } else { fac_length });

    // 4) Compute inverse DCT-IV of FAC data. Output scale of DCT IV is 16 bits.
    dct_iv(p_fac, fac_length, &mut scale);
    // dct_IV scale = log2(fac_length). "- 7" is a factor of 2/128
    if tf_gain != 0 {
        // non-radix 2 transform gain
        for i in 0..n {
            p_fac[i] = f_mult(tf_gain, p_fac[i]);
        }
    }
    scale_values_saturate_to(&mut p_out[..n], &p_fac[..n], scale); // Avoid overflow issues and saturate.

    e_lpc_a_weight(&mut wa, a, M_LP_FILTER_ORDER);

    // We need the output of the IIR filter to be longer than "fac_length". For this reason we
    // run it with zero input appended to the end of the input sequence, i.e. we generate its
    // ZIR and extend the output signal.
    p_out[n..2 * n].fill(0);
    let wlength = 2 * fac_length;

    // 5) Apply weighted synthesis filter to FAC data, including optional Zir (5. item 4).
    syn_filt_zero(&wa, a_exp, wlength, p_out);
}

/// The buffers a C output pointer of the transforms can point into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Buf {
    /// `output` (index relative to `output[0]`)
    Out,
    /// `hMdct->overlap`
    Ovl,
    /// the spectrum `_pSpec`
    Spec,
}

/// A C pointer: a buffer and an index into it.
#[derive(Clone, Copy, Debug)]
struct Ptr {
    buf: Buf,
    idx: isize,
}

impl Ptr {
    #[inline(always)]
    fn new(buf: Buf, idx: isize) -> Ptr {
        Ptr { buf, idx }
    }
    #[inline(always)]
    fn at(self, d: isize) -> Ptr {
        Ptr { buf: self.buf, idx: self.idx + d }
    }
}

/// The memory of the transforms; accesses outside of a buffer read 0 and are dropped.
struct Bufs<'a> {
    out: &'a mut [FixpDbl],
    out_off: isize,
    ovl: &'a mut [FixpDbl],
    spec: &'a mut [FixpDbl],
}

impl Bufs<'_> {
    #[inline(always)]
    fn slot(&mut self, p: Ptr) -> Option<&mut FixpDbl> {
        let (b, i): (&mut [FixpDbl], isize) = match p.buf {
            Buf::Out => (&mut *self.out, self.out_off + p.idx),
            Buf::Ovl => (&mut *self.ovl, p.idx),
            Buf::Spec => (&mut *self.spec, p.idx),
        };
        if i >= 0 {
            b.get_mut(i as usize)
        } else {
            None
        }
    }
    #[inline(always)]
    fn get(&mut self, p: Ptr) -> FixpDbl {
        self.slot(p).map_or(0, |v| *v)
    }
    #[inline(always)]
    fn set(&mut self, p: Ptr, v: FixpDbl) {
        if let Some(s) = self.slot(p) {
            *s = v;
        }
    }
}

/// `pWindow[i].v.re` / `.v.im`; 0 for a missing window slope or an index outside of it.
#[inline(always)]
fn win(w: Option<&'static [FixpWtp]>, i: usize) -> (i16, i16) {
    w.and_then(|w| w.get(i)).map_or((0, 0), |w| (w.re, w.im))
}

/// `CLpd_FAC_Mdct2Acelp(hMdct, output, pFac, fac_scale, A, A_exp, nrOutSamples, fac_length,
/// isFdFac, prevWindowShape)`: do the FAC transition from the frequency domain to the ACELP
/// domain. Returns the number of output samples.
///
/// `output` is `out_buf[out_off..]`; `p_fac` is the FAC data (`None` for `NULL`), transformed
/// in place. Up to `2 * fac_length` values are written behind the output position of the FAC
/// signal, as in C.
#[allow(clippy::too_many_arguments)]
pub fn clpd_fac_mdct2acelp(
    h_mdct: &mut MdctT,
    out_buf: &mut [FixpDbl],
    out_off: usize,
    p_fac: Option<&mut [FixpDbl]>,
    fac_scale: i32,
    a: &[FixpLpc],
    a_exp: i32,
    nr_out_samples: i32,
    fac_length: i32,
    is_fd_fac: i32,
    prev_window_shape: u8,
) -> i32 {
    let mut nr_samples = 0;

    fdk_assert!(fac_length <= 1024 / (4 * 2));

    let mut fl = fac_length * 2;

    let p_window = fdk_get_window_slope(fl, prev_window_shape as i32);

    // Adapt window slope length in case of frame loss.
    if h_mdct.prev_fr != fl {
        let mut nl = 0;
        imdct_adapt_parameters(h_mdct, &mut fl, &mut nl, fac_length, p_window, nr_out_samples);
        fdk_assert!(nl == 0);
    }

    let mut ovl_vec = core::mem::take(&mut h_mdct.overlap);
    let mut spec_dummy: [FixpDbl; 0] = [];
    let mut b =
        Bufs { out: out_buf, out_off: out_off as isize, ovl: &mut ovl_vec, spec: &mut spec_dummy };

    if nr_samples < nr_out_samples {
        nr_samples += h_mdct.ov_offset;
        // Purge buffered output.
        for i in 0..h_mdct.ov_offset.max(0) as isize {
            let v = b.get(Ptr::new(Buf::Ovl, i));
            b.set(Ptr::new(Buf::Out, i), v);
        }
        h_mdct.ov_offset = 0;
    }

    let mut p_ovl = Ptr::new(Buf::Ovl, h_mdct.ov_size as isize - 1);

    let mut p_out0;
    if nr_samples >= nr_out_samples {
        p_out0 = Ptr::new(Buf::Ovl, h_mdct.ov_offset as isize);
        h_mdct.ov_offset += h_mdct.prev_nr + fl / 2;
    } else {
        p_out0 = Ptr::new(Buf::Out, nr_samples as isize);
        nr_samples += h_mdct.prev_nr + fl / 2;
    }
    if h_mdct.prev_prev_alias_symmetry == 0 {
        for _ in 0..h_mdct.prev_nr {
            let x = b.get(p_ovl).wrapping_neg();
            p_ovl = p_ovl.at(-1);
            b.set(p_out0, imdct_scale_dbl(x));
            p_out0 = p_out0.at(1);
        }
    } else {
        for _ in 0..h_mdct.prev_nr {
            let x = b.get(p_ovl);
            p_ovl = p_ovl.at(-1);
            b.set(p_out0, imdct_scale_dbl(x));
            p_out0 = p_out0.at(1);
        }
    }
    h_mdct.prev_nr = 0;

    {
        let n = fac_length.max(0) as usize;
        if let Some(p_fac) = p_fac {
            // Note: The FAC gain might have been applied directly after bit stream parse in
            // this case.
            let mut tmp = [0 as FixpDbl; 2 * LFAC as usize];
            cfac_calc_fac_signal(
                &mut tmp[..2 * n],
                p_fac,
                fac_scale,
                fac_length,
                a,
                a_exp,
                0,
                is_fd_fac,
            );
            for i in 0..2 * n {
                b.set(p_out0.at(i as isize), tmp[i]);
            }
        } else {
            // Clear buffer because of the overlap and ADD!
            for i in 0..n {
                b.set(p_out0.at(i as isize), 0);
            }
        }
    }

    if h_mdct.prev_prev_alias_symmetry == 0 {
        for i in 0..(fl / 2).max(0) as usize {
            // Overlap Add
            let x0 = f_mult(b.get(p_ovl), win(p_window, i).0).wrapping_neg();
            p_ovl = p_ovl.at(-1);

            let v = f_add_saturate(b.get(p_out0), imdct_scale_dbl(x0));
            b.set(p_out0, v);
            p_out0 = p_out0.at(1);
        }
    } else {
        for i in 0..(fl / 2).max(0) as usize {
            // Overlap Add
            let x0 = f_mult(b.get(p_ovl), win(p_window, i).0);
            p_ovl = p_ovl.at(-1);

            let v = f_add_saturate(b.get(p_out0), imdct_scale_dbl(x0));
            b.set(p_out0, v);
            p_out0 = p_out0.at(1);
        }
    }
    if let Some(zir) = h_mdct.p_fac_zir.take() {
        // this should only happen for ACELP -> TCX20 -> ACELP transition
        let p_out = p_out0.at(-(fl / 2) as isize); // fl/2 == fac_length
        for i in 0..(fl / 2).max(0) as usize {
            let q = p_out.at(i as isize);
            let v = f_add_saturate(b.get(q), imdct_scale_dbl(zir.get(i).copied().unwrap_or(0)));
            b.set(q, v);
        }
        // hMdct->pFacZir = NULL (taken above)
    }

    h_mdct.overlap = ovl_vec;
    h_mdct.prev_fr = 0;
    h_mdct.prev_nr = 0;
    h_mdct.prev_tl = 0;
    h_mdct.prev_prev_alias_symmetry = h_mdct.prev_alias_symmetry;

    nr_samples
}

/// The FAC window tables of a FAC length: `(pWindow, FacWindowZir, FacWindowSynth)`.
fn fac_windows(
    fac_length: i32,
) -> Option<(&'static [FixpWtp], &'static [FixpWtb], &'static [FixpWtb])> {
    Some(match fac_length {
        // coreCoderFrameLength = 1024
        128 => (&SINE_WINDOW_256[..], &FAC_WINDOW_ZIR_128[..], &FAC_WINDOW_SYNTH_128[..]),
        64 => (&SINE_WINDOW_128[..], &FAC_WINDOW_ZIR_64[..], &FAC_WINDOW_SYNTH_64[..]),
        32 => (&SINE_WINDOW_64[..], &FAC_WINDOW_ZIR_32[..], &FAC_WINDOW_SYNTH_32[..]),
        // coreCoderFrameLength = 768
        96 => (&SINE_WINDOW_192[..], &FAC_WINDOW_ZIR_96[..], &FAC_WINDOW_SYNTH_96[..]),
        48 => (&SINE_WINDOW_96[..], &FAC_WINDOW_ZIR_48[..], &FAC_WINDOW_SYNTH_48[..]),
        _ => return None,
    })
}

/// `CLpd_FAC_Acelp2Mdct(hMdct, output, pSpec, spec_scale, nSpec, pFac, fac_scale, fac_length,
/// nrSamples, tl, wrs, fr, A, A_exp, acelp_mem, gain, last_frame_lost, isFdFac, last_lpd_mode,
/// k, currAliasingSymmetry)`: do the FAC transition from the ACELP domain to the frequency
/// domain. Returns the number of output samples.
///
/// `output` is `out_buf[out_off..]` (the past ACELP samples before it are read); `p_spec`:
/// `n_spec` contiguous MDCT spectra of `tl` values (transformed in place); `spec_scale`: their
/// exponents; `p_fac`: FAC MDCT domain data (`fac_length` values, overwritten with the ACELP
/// zero input response as in C); `wrs`, `fr`: right MDCT window slope and its length; `gain`:
/// gain applied to the spectrum; `is_fd_fac`: FAC processing from or to FD.
#[allow(clippy::too_many_arguments)]
pub fn clpd_fac_acelp2mdct(
    h_mdct: &mut MdctT,
    out_buf: &mut [FixpDbl],
    out_off: usize,
    p_spec: &mut [FixpDbl],
    spec_scale: &[i16],
    n_spec: i32,
    p_fac: &mut [FixpDbl],
    fac_scale: i32,
    fac_length: i32,
    no_out_samples: i32,
    tl: i32,
    wrs: Option<&'static [FixpWtp]>,
    fr: i32,
    a: &[FixpLpc],
    a_exp: i32,
    acelp_mem: &mut CAcelpStaticMem,
    gain: FixpDbl,
    last_frame_lost: i32,
    is_fd_fac: i32,
    last_lpd_mode: u8,
    k: i32,
    curr_aliasing_symmetry: i32,
) -> i32 {
    let mut do_fac_zir_conceal = false;
    let mut do_deemph = 1;
    let mut nr_samples = 0;
    let s = 0;

    fdk_assert!(fac_length <= 1024 / (4 * 2));
    let Some((p_window, mut fac_window_zir, mut fac_window_synth)) = fac_windows(fac_length) else {
        fdk_assert!(false);
        return 0;
    };
    // The cleared excitation memory that C reinterprets as a window (all zero).
    static ZERO_WINDOW: [FixpWtb; LFAC as usize] = [0; LFAC as usize];

    let fac_window_zir_conceal = fac_window_synth;
    // Derive NR and NL
    let mut fl = fac_length * 2;
    let mut nl = (tl - fl) >> 1;
    let nr = (tl - fr) >> 1;

    let mut ovl_vec = core::mem::take(&mut h_mdct.overlap);
    let mut b = Bufs { out: out_buf, out_off: out_off as isize, ovl: &mut ovl_vec, spec: p_spec };

    // pOut0 = output (the purge below copies to output[0..ov_offset])
    let mut p_out0;
    let mut p_out1;

    if no_out_samples > nr_samples {
        // Purge buffered output.
        for i in 0..h_mdct.ov_offset.max(0) as isize {
            let v = b.get(Ptr::new(Buf::Ovl, i));
            b.set(Ptr::new(Buf::Out, i), v);
        }
        nr_samples = h_mdct.ov_offset;
        h_mdct.ov_offset = 0;
    }

    if nr_samples >= no_out_samples {
        p_out1 = Ptr::new(Buf::Ovl, h_mdct.ov_offset as isize);
        if h_mdct.ov_offset < fac_length {
            p_out0 = Ptr::new(Buf::Out, nr_samples as isize);
        } else {
            p_out0 = p_out1;
        }
        h_mdct.ov_offset += fac_length + nl;
    } else {
        p_out1 = Ptr::new(Buf::Out, nr_samples as isize);
        p_out0 = Ptr::new(Buf::Out, nr_samples as isize);
    }

    let n = fac_length.max(0) as usize;
    {
        // pFAC_and_FAC_ZIR = CLpd_ACELP_GetFreeExcMem(acelp_mem, 2 * fac_length)
        clpd_acelp_get_free_exc_mem(acelp_mem, 2 * fac_length);
        {
            do_fac_zir_conceal |= (last_frame_lost != 0) && (k == 0);
            do_deemph &= (last_lpd_mode != 4) as i32;
            if do_fac_zir_conceal {
                // ACELP contribution in concealment case: Use ZIR with a modified ZIR window to
                // preserve some more energy. Dont use FAC, which contains wrong information for
                // concealed frame. Dont use last ACELP samples, but double ZIR, instead
                // (afterwards)
                acelp_mem.old_exc_mem[..2 * n].fill(0);
                fac_window_synth = &ZERO_WINDOW[..];
                fac_window_zir = fac_window_zir_conceal;
            } else {
                cfac_calc_fac_signal(
                    &mut acelp_mem.old_exc_mem[..2 * n],
                    p_fac,
                    fac_scale + s,
                    fac_length,
                    a,
                    a_exp,
                    1,
                    is_fd_fac,
                );
            }
            // 6) Get windowed past ACELP samples and ACELP ZIR signal

            // Get ACELP ZIR (pFac[]) and ACELP past samples (pOut0[]) and add them to the FAC
            // synth signal contribution on pOut1[].
            {
                clpd_acelp_zir(a, a_exp, acelp_mem, fac_length, p_fac, do_deemph);

                let p_tmp1 = p_out0;

                for i in 0..n {
                    // Div2 is compensated by table scaling
                    let mut x = f_mult_div2(p_fac[i], fac_window_zir[i]);
                    x = x.wrapping_add(f_mult_div2(
                        b.get(p_tmp1.at(-(i as isize) - 1)),
                        fac_window_synth[i],
                    ));
                    b.set(p_out1.at(i as isize), f_add_saturate(x, acelp_mem.old_exc_mem[i]));
                }
            }

            if do_fac_zir_conceal {
                // ZIR is the only ACELP contribution, so double it
                for i in 0..n {
                    let q = p_out1.at(i as isize);
                    let v = b.get(q) << 1;
                    b.set(q, v);
                }
            }
        }
    }

    if nr_samples < no_out_samples {
        nr_samples += fac_length + nl;
    }

    // Obtain transform gain
    let mut total_gain = gain;
    let mut total_gain_e = 0;
    imdct_gain(&mut total_gain, &mut total_gain_e, tl);

    // IMDCT overlap add
    let mut scale = total_gain_e;
    let tlu = tl.max(0) as usize;

    // Note:when comming from an LPD frame (TCX/ACELP) the previous alisaing symmetry must
    // always be 0
    if curr_aliasing_symmetry == 0 {
        dct_iv(&mut b.spec[..tlu], tl, &mut scale);
    } else {
        let mut tmp = [0 as FixpDbl; 1024];
        dst_iii(&mut b.spec[..tlu], &mut tmp, tl, &mut scale);
    }

    // Optional scaling of time domain - no yet windowed - of current spectrum
    if total_gain != 0 {
        for i in 0..tlu {
            b.spec[i] = f_mult(b.spec[i], total_gain);
        }
    }
    let mut loc_scale = fixmin_i(spec_scale[0] as i32 + scale, DFRACT_BITS - 1);
    scale_values_saturate(&mut b.spec[..tlu], loc_scale);

    p_out1 = p_out1.at((fl / 2 - 1) as isize);
    let mut p_curr = Ptr::new(Buf::Spec, (tl - fl / 2) as isize);

    for i in 0..(fl / 2).max(0) as usize {
        // FAC signal is already on pOut1, because of that the += operator.
        let x1 = f_mult(b.get(p_curr), win(Some(p_window), i).0);
        p_curr = p_curr.at(1);
        let v = f_add_saturate(b.get(p_out1), imdct_scale_dbl(x1.wrapping_neg()));
        b.set(p_out1, v);
        p_out1 = p_out1.at(-1);
    }

    // NL output samples TL/2+FL/2..TL. - current[FL/2..0]
    p_out1 = p_out1.at((fl / 2 + 1) as isize);

    // pFAC_and_FAC_ZIR += fac_length: set pointer to beginning of FAC ZIR
    if nl == 0 {
        // save pointer to write FAC ZIR data later (a copy of the memory, see the module
        // documentation)
        h_mdct.p_fac_zir = Some(acelp_mem.old_exc_mem[n..].to_vec());
    } else {
        fdk_assert!(nl >= fac_length);
        // FAC ZIR will be added now ...
        h_mdct.p_fac_zir = None;
    }

    let mut p_f = n;
    let f_len = fac_length;

    p_curr = Ptr::new(Buf::Spec, (tl - fl / 2 - 1) as isize);
    for i in 0..nl.max(0) {
        let mut x = b.get(p_curr).wrapping_neg();
        p_curr = p_curr.at(-1);
        // 5) (item 4) Synthesis filter Zir component, FAC ZIR (another one).
        if i < f_len {
            x = f_add_saturate(x, acelp_mem.old_exc_mem.get(p_f).copied().unwrap_or(0));
            p_f += 1;
        }

        b.set(p_out1, imdct_scale_dbl(x));
        p_out1 = p_out1.at(1);
    }

    h_mdct.prev_nr = nr;
    h_mdct.prev_fr = fr;
    h_mdct.prev_wrs = wrs;
    h_mdct.prev_tl = tl;
    h_mdct.prev_prev_alias_symmetry = h_mdct.prev_alias_symmetry;
    h_mdct.prev_alias_symmetry = curr_aliasing_symmetry;
    fl = fr;
    nl = nr;

    let mut p_ovl = Ptr::new(Buf::Spec, (tl / 2 - 1) as isize);
    p_out0 = p_out1;

    let mut w = 1;
    while w < n_spec {
        // for ACELP -> FD short

        // Setup window pointers
        let p_window_prev = h_mdct.prev_wrs;

        // Current spectrum
        let p_spec_w = (w * tl) as usize;

        scale = total_gain_e;

        // For the second, third, etc. short frames the alisaing symmetry is equal, either (0,0)
        // or (1,1)
        {
            let cur = match b.spec.get_mut(p_spec_w..p_spec_w + tlu) {
                Some(s) => s,
                None => &mut [],
            };
            if curr_aliasing_symmetry == 0 {
                // DCT IV of current spectrum
                dct_iv(cur, tl, &mut scale);
            } else {
                dst_iv(cur, tl, &mut scale);
            }

            // Optional scaling of time domain - no yet windowed - of current spectrum
            // and de-scale current spectrum signal (time domain, no yet windowed)
            if total_gain != 0 {
                for v in cur.iter_mut() {
                    *v = f_mult(*v, total_gain);
                }
            }
            loc_scale = fixmin_i(
                spec_scale.get(w as usize).copied().unwrap_or(0) as i32 + scale,
                DFRACT_BITS - 1,
            );
            scale_values_saturate(cur, loc_scale);
        }

        if no_out_samples <= nr_samples {
            // Divert output first half to overlap buffer if we already got enough output
            // samples.
            p_out0 = Ptr::new(Buf::Ovl, h_mdct.ov_offset as isize);
            h_mdct.ov_offset += h_mdct.prev_nr + fl / 2;
        } else {
            // Account output samples
            nr_samples += h_mdct.prev_nr + fl / 2;
        }

        // NR output samples 0 .. NR. -overlap[TL/2..TL/2-NR]
        for _ in 0..h_mdct.prev_nr {
            let x = b.get(p_ovl).wrapping_neg();
            p_ovl = p_ovl.at(-1);
            b.set(p_out0, imdct_scale_dbl(x));
            p_out0 = p_out0.at(1);
        }

        if no_out_samples <= nr_samples {
            // Divert output second half to overlap buffer if we already got enough output
            // samples.
            p_out1 = Ptr::new(Buf::Ovl, (h_mdct.ov_offset + fl / 2 - 1) as isize);
            h_mdct.ov_offset += fl / 2 + nl;
        } else {
            p_out1 = p_out0.at((fl - 1) as isize);
            nr_samples += fl / 2 + nl;
        }

        // output samples before window crossing point NR .. TL/2.
        // -overlap[TL/2-NR..TL/2-NR-FL/2] + current[NR..TL/2]
        // output samples after window crossing point TL/2 .. TL/2+FL/2.
        // -overlap[0..FL/2] - current[TL/2..FL/2]
        p_curr = Ptr::new(Buf::Spec, (p_spec_w as i32 + tl - fl / 2) as isize);
        if curr_aliasing_symmetry == 0 {
            for i in 0..(fl / 2).max(0) as usize {
                let (wr, wi) = win(p_window_prev, i);
                let (x1, x0) = cplx_mult_div2(b.get(p_curr), b.get(p_ovl).wrapping_neg(), wr, wi);
                p_curr = p_curr.at(1);
                p_ovl = p_ovl.at(-1);
                b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                b.set(p_out1, imdct_scale_dbl_lsh1(x1.wrapping_neg()));
                p_out0 = p_out0.at(1);
                p_out1 = p_out1.at(-1);
            }
        } else {
            if h_mdct.prev_prev_alias_symmetry == 0 {
                // Jump DST II -> DST IV for the second window
                for i in 0..(fl / 2).max(0) as usize {
                    let (wr, wi) = win(p_window_prev, i);
                    let (x1, x0) =
                        cplx_mult_div2(b.get(p_curr), b.get(p_ovl).wrapping_neg(), wr, wi);
                    p_curr = p_curr.at(1);
                    p_ovl = p_ovl.at(-1);
                    b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                    b.set(p_out1, imdct_scale_dbl_lsh1(x1));
                    p_out0 = p_out0.at(1);
                    p_out1 = p_out1.at(-1);
                }
            } else {
                // Jump DST IV -> DST IV from the second window on
                for i in 0..(fl / 2).max(0) as usize {
                    let (wr, wi) = win(p_window_prev, i);
                    let (x1, x0) = cplx_mult_div2(b.get(p_curr), b.get(p_ovl), wr, wi);
                    p_curr = p_curr.at(1);
                    p_ovl = p_ovl.at(-1);
                    b.set(p_out0, imdct_scale_dbl_lsh1(x0));
                    b.set(p_out1, imdct_scale_dbl_lsh1(x1));
                    p_out0 = p_out0.at(1);
                    p_out1 = p_out1.at(-1);
                }
            }
        }

        if let Some(zir) = h_mdct.p_fac_zir.take() {
            // add FAC ZIR of previous ACELP -> mdct transition
            let p_out = p_out0.at(-(fl / 2) as isize);
            fdk_assert!(fl / 2 <= 128);
            for i in 0..(fl / 2).max(0) as usize {
                let q = p_out.at(i as isize);
                let v = f_add_saturate(b.get(q), imdct_scale_dbl(zir.get(i).copied().unwrap_or(0)));
                b.set(q, v);
            }
            // hMdct->pFacZir = NULL (taken above)
        }
        p_out0 = p_out0.at((fl / 2) as isize);

        // NL output samples TL/2+FL/2..TL. - current[FL/2..0]
        p_out1 = p_out1.at((fl / 2 + 1) as isize);
        p_curr = Ptr::new(Buf::Spec, (p_spec_w as i32 + tl - fl / 2 - 1) as isize);
        for _ in 0..nl.max(0) {
            let x = b.get(p_curr).wrapping_neg();
            p_curr = p_curr.at(-1);
            b.set(p_out1, imdct_scale_dbl(x));
            p_out1 = p_out1.at(1);
        }

        // Set overlap source pointer for next window pOvl = pSpec + tl/2 - 1;
        p_ovl = Ptr::new(Buf::Spec, (p_spec_w as i32 + tl / 2 - 1) as isize);

        // Previous window values.
        h_mdct.prev_nr = nr;
        h_mdct.prev_fr = fr;
        h_mdct.prev_tl = tl;
        h_mdct.prev_wrs = p_window_prev;
        h_mdct.prev_prev_alias_symmetry = h_mdct.prev_alias_symmetry;
        h_mdct.prev_alias_symmetry = curr_aliasing_symmetry;

        w += 1;
    }

    // Save overlap

    let p_ovl = h_mdct.ov_size as isize - (tl / 2) as isize;
    fdk_assert!(p_ovl >= h_mdct.ov_offset as isize);
    fdk_assert!(tl / 2 <= h_mdct.ov_size);
    for i in 0..(tl / 2).max(0) as isize {
        let v = b.get(Ptr::new(Buf::Spec, i + ((w - 1) * tl) as isize));
        b.set(Ptr::new(Buf::Ovl, p_ovl + i), v);
    }

    h_mdct.overlap = ovl_vec;

    nr_samples
}
