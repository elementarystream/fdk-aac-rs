// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/env_est.h, libSBRenc/src/env_est.cpp.

//! Port of `env_est.h` and `env_est.cpp`: the envelope extraction (energies, envelopes, noise
//! floor quantization, stereo coding decision) of an SBR element.
//!
//! # Memory
//!
//! `SBR_EXTRACT_ENVELOPE` holds row pointers into three areas:
//!
//! - `rBuffer[i]`, `iBuffer[i]`: the QMF data in the dynamic RAM of the encoder
//!   (`GetRam_Sbr_envRBuffer(0)` and `GetRam_Sbr_envIBuffer(0)`, rows of 64 values); all
//!   channels share them;
//! - `YBuffer[0..16]`: the channel's own energy buffer `p_YBuffer` (`Ram_Sbr_envYBuffer`);
//! - `YBuffer[16..32]`: the dynamic RAM (`GetRam_Sbr_envYBuffer(chInEl)`).
//!
//! The port keeps the offsets of the dynamic parts ([`SbrExtractEnvelope::r_buffer`],
//! [`SbrExtractEnvelope::i_buffer`], [`SbrExtractEnvelope::y_buffer_dyn`]) and the owned
//! `p_YBuffer`; the functions that use them take the dynamic RAM. The dynamic RAM is shared with
//! the hybrid data of the parametric stereo encoder (see [`crate::ps_main`]), as in C.
//!
//! `SBR_ENV_TEMP_DATA::frame_info` points to the frame info of an envelope channel; it is the
//! index of that channel within the element here.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

use crate::bit_sbr::*;
use crate::cmondata::CommonData;
use crate::code_env::*;
use crate::fram_gen::*;
use crate::ps_main::ParametricStereo;
use crate::sbr_def::*;
use crate::sbr_encoder::*;
use crate::sbrenc_ram::*;
use crate::ton_corr::*;
use crate::tran_det::*;

mod tables;
pub use tables::*;

/// `QUANT_ERROR_THRES`
pub const QUANT_ERROR_THRES: i32 = 200;
/// `Y_NRG_SCALE`: noCols = 32 -> shift(5)
pub const Y_NRG_SCALE: i32 = 5;
/// `MAX_NRG_SLOTS_LD`
pub const MAX_NRG_SLOTS_LD: usize = 16;

/// The number of rows of `YBuffer`, `rBuffer` and `iBuffer`.
pub const ENV_BUFFER_ROWS: usize = 32;
/// The number of rows of `YBuffer` in `p_YBuffer` (the following ones are in the dynamic RAM).
pub const Y_BUFFER_STATIC_ROWS: usize = 32 >> 1;

/// `SBR_EXTRACT_ENVELOPE` (`HANDLE_SBR_EXTRACT_ENVELOPE` is `&mut SbrExtractEnvelope`), see the
/// [module documentation](self) for the buffers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SbrExtractEnvelope {
    /// `rBuffer[i]` is `dynamic_RAM[r_buffer + 64 * i]`
    pub r_buffer: usize,
    /// `iBuffer[i]` is `dynamic_RAM[i_buffer + 64 * i]`
    pub i_buffer: usize,

    /// `p_YBuffer` (`YBuffer[i]` for `i < 16` is `p_YBuffer[64 * i]`; empty for `NULL`)
    pub p_y_buffer: Vec<FixpDbl>,

    /// `YBuffer[16 + n]` is `dynamic_RAM[y_buffer_dyn + 64 * n]`
    pub y_buffer_dyn: usize,
    /// `YBufferScale`
    pub y_buffer_scale: [i32; 2],

    /// `envelopeCompensation`
    pub envelope_compensation: [u8; MAX_FREQ_COEFFS],
    /// `pre_transient_info`
    pub pre_transient_info: [u8; 2],

    /// `YBufferWriteOffset`
    pub y_buffer_write_offset: i32,
    /// `YBufferSzShift`
    pub y_buffer_sz_shift: i32,
    /// `rBufferReadOffset`
    pub r_buffer_read_offset: i32,

    /// `no_cols`
    pub no_cols: i32,
    /// `no_rows`
    pub no_rows: i32,
    /// `start_index`
    pub start_index: i32,

    /// `time_slots`
    pub time_slots: i32,
    /// `time_step`
    pub time_step: i32,
}

impl Default for SbrExtractEnvelope {
    /// The cleared structure (`FDKmemclear`), without `p_YBuffer`.
    fn default() -> Self {
        SbrExtractEnvelope {
            r_buffer: 0,
            i_buffer: 0,
            p_y_buffer: Vec::new(),
            y_buffer_dyn: 0,
            y_buffer_scale: [0; 2],
            envelope_compensation: [0; MAX_FREQ_COEFFS],
            pre_transient_info: [0; 2],
            y_buffer_write_offset: 0,
            y_buffer_sz_shift: 0,
            r_buffer_read_offset: 0,
            no_cols: 0,
            no_rows: 0,
            start_index: 0,
            time_slots: 0,
            time_step: 0,
        }
    }
}

impl SbrExtractEnvelope {
    /// `YBuffer[0..32]` (rows of 64 values; an array, so that no frame allocates).
    pub fn y_rows<'a>(&'a self, dyn_ram: &'a [FixpDbl]) -> [&'a [FixpDbl]; ENV_BUFFER_ROWS] {
        core::array::from_fn(|i| self.y_row(i, dyn_ram))
    }

    /// `YBuffer[i]`
    pub fn y_row<'a>(&'a self, i: usize, dyn_ram: &'a [FixpDbl]) -> &'a [FixpDbl] {
        if i < Y_BUFFER_STATIC_ROWS {
            &self.p_y_buffer[64 * i..64 * i + 64]
        } else {
            let o = self.y_buffer_dyn + 64 * (i - Y_BUFFER_STATIC_ROWS);
            &dyn_ram[o..o + 64]
        }
    }

    /// `YBuffer[i]`, writable.
    pub fn y_row_mut<'a>(&'a mut self, i: usize, dyn_ram: &'a mut [FixpDbl]) -> &'a mut [FixpDbl] {
        if i < Y_BUFFER_STATIC_ROWS {
            &mut self.p_y_buffer[64 * i..64 * i + 64]
        } else {
            let o = self.y_buffer_dyn + 64 * (i - Y_BUFFER_STATIC_ROWS);
            &mut dyn_ram[o..o + 64]
        }
    }

    /// `rBuffer[0..32]` and `iBuffer[0..32]` (arrays, so that no frame allocates).
    #[allow(clippy::type_complexity)]
    pub fn qmf_rows<'a>(
        &self,
        dyn_ram: &'a [FixpDbl],
    ) -> ([&'a [FixpDbl]; ENV_BUFFER_ROWS], [&'a [FixpDbl]; ENV_BUFFER_ROWS]) {
        let re =
            core::array::from_fn(|i| &dyn_ram[self.r_buffer + 64 * i..self.r_buffer + 64 * i + 64]);
        let im =
            core::array::from_fn(|i| &dyn_ram[self.i_buffer + 64 * i..self.i_buffer + 64 * i + 64]);
        (re, im)
    }
}

/// `struct ENV_CHANNEL` (`HANDLE_ENV_CHANNEL` is `&mut EnvChannel`)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvChannel {
    /// `sbrFastTransientDetector`
    pub sbr_fast_transient_detector: FastTranDetector,
    /// `sbrTransientDetector`
    pub sbr_transient_detector: SbrTransientDetector,
    /// `sbrCodeEnvelope`
    pub sbr_code_envelope: SbrCodeEnvelope,
    /// `sbrCodeNoiseFloor`
    pub sbr_code_noise_floor: SbrCodeEnvelope,
    /// `sbrExtractEnvelope`
    pub sbr_extract_envelope: SbrExtractEnvelope,

    /// `SbrEnvFrame`
    pub sbr_env_frame: SbrEnvelopeFrame,
    /// `TonCorr`
    pub ton_corr: SbrTonCorrEst,

    /// `encEnvData`
    pub enc_env_data: SbrEnvData,

    /// `qmfScale`
    pub qmf_scale: i32,
    /// `fLevelProtect`
    pub f_level_protect: u8,
}

/// `SBR_FRAME_TEMP_DATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrFrameTempData {
    /// `res`
    pub res: [FreqRes; MAX_NUM_NOISE_VALUES],
    /// `maxQuantError`
    pub max_quant_error: i32,
}

/// `SBR_ENV_TEMP_DATA`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbrEnvTempData {
    /// `frame_info`: the index (within the element) of the envelope channel whose
    /// `SbrEnvFrame.SbrFrameInfo` it points to (`None` for `NULL`)
    pub frame_info: Option<usize>,
    /// `noiseFloor`
    pub noise_floor: [FixpDbl; MAX_NUM_NOISE_VALUES],
    /// `sfb_nrg_coupling`: only used if stereomode = SWITCH_L_R_C
    pub sfb_nrg_coupling: [i8; MAX_NUM_ENVELOPE_VALUES],
    /// `sfb_nrg`
    pub sfb_nrg: [i8; MAX_NUM_ENVELOPE_VALUES],
    /// `noise_level_coupling`: only used if stereomode = SWITCH_L_R_C
    pub noise_level_coupling: [i8; MAX_NUM_NOISE_VALUES],
    /// `noise_level`
    pub noise_level: [i8; MAX_NUM_NOISE_VALUES],
    /// `transient_info`
    pub transient_info: [u8; 3],
    /// `nEnvelopes`
    pub n_envelopes: u8,
}

impl Default for SbrEnvTempData {
    /// The cleared structure (`FDKmemclear`).
    fn default() -> Self {
        SbrEnvTempData {
            frame_info: None,
            noise_floor: [0; MAX_NUM_NOISE_VALUES],
            sfb_nrg_coupling: [0; MAX_NUM_ENVELOPE_VALUES],
            sfb_nrg: [0; MAX_NUM_ENVELOPE_VALUES],
            noise_level_coupling: [0; MAX_NUM_NOISE_VALUES],
            noise_level: [0; MAX_NUM_NOISE_VALUES],
            transient_info: [0; 3],
            n_envelopes: 0,
        }
    }
}

/// `quotaMatrix[row][band]` of a tonality correction instance.
///
/// Bounded: indices outside the matrix read 0.
fn quota(t: &SbrTonCorrEst, row: i32, band: i32) -> FixpDbl {
    if (0..MAX_NO_OF_ESTIMATES as i32).contains(&row) && (0..64).contains(&band) {
        t.quota_matrix[64 * row as usize + band as usize]
    } else {
        0
    }
}

/// `FDKsbrEnc_GetTonality(quotaMatrix, noEstPerFrame, startIndex, Energies, startBand,
/// stopBand, numberCols)`: the mean tonality of the 5 bands with the highest energy, scaled by
/// 2^(RELAXATION_SHIFT+2)*RELAXATION_FRACT. `energies` are the rows `Energies[0..16]`.
fn fdk_sbr_enc_get_tonality(
    ton_corr: &SbrTonCorrEst,
    no_est_per_frame: i32,
    start_index: i32,
    energies: &[&[FixpDbl]],
    start_band: u8,
    stop_band: i32,
    number_cols: i32,
) -> FixpDbl {
    let mut no_en_max_band = [-1i32; SBR_MAX_ENERGY_VALUES];
    let mut energy_max: [FixpDbl; SBR_MAX_ENERGY_VALUES] = [0; SBR_MAX_ENERGY_VALUES];
    let mut energy_max_min: FixpDbl; // min. energy in energyMax array
    let mut pos_energy_max_min: u8; // min. energy in energyMax array position
    let mut tonality_band: [FixpDbl; SBR_MAX_ENERGY_VALUES] = [0; SBR_MAX_ENERGY_VALUES];
    let mut global_tonality: FixpDbl = 0;
    let mut energy_band = [0 as FixpDbl; 64];

    let start = start_band as usize;
    let stop = stop_band.clamp(0, 64) as usize;
    let row = |k: usize| -> &[FixpDbl] { energies.get(k).copied().unwrap_or(&[0; 64]) };

    // Sum up energies for each band
    fdk_assert!(number_cols == 15 || number_cols == 16);
    // numberCols is always 15 or 16 for ELD. In case of 16 bands, the energyBands are
    // initialized with the [15]th column. The rest of the column energies are added in the
    // next step.
    if number_cols == 15 {
        for b in start..stop {
            energy_band[b] = 0;
        }
    } else {
        for b in start..stop {
            energy_band[b] = row(15)[b] >> 4;
        }
    }

    for k in 0..15 {
        for b in start..stop {
            energy_band[b] = energy_band[b].wrapping_add(row(k)[b] >> 4);
        }
    }

    // Determine 5 highest band-energies
    let max_n_energy_values = f_min(SBR_MAX_ENERGY_VALUES as i32, stop_band - start_band as i32);

    // Get min. value in energyMax array. energyBand[startBand] is not defined by the reference
    // for startBand >= stopBand; the value is never used then.
    energy_max[0] = energy_band.get(start).copied().unwrap_or(0);
    energy_max_min = energy_max[0];
    no_en_max_band[0] = start_band as i32;
    pos_energy_max_min = 0;
    for k in 1..max_n_energy_values.max(0) as usize {
        energy_max[k] = energy_band[start + k];
        no_en_max_band[k] = (start + k) as i32;
        if energy_max_min > energy_max[k] {
            energy_max_min = energy_max[k];
            pos_energy_max_min = k as u8;
        }
    }

    // C: for (b = startBand + maxNEnergyValues; b < stopBand; b++) with a UCHAR b
    let mut b = (start_band as i32 + max_n_energy_values) as u8 as usize;
    while (b as i32) < stop_band {
        if energy_band[b] > energy_max_min {
            energy_max[pos_energy_max_min as usize] = energy_band[b];
            no_en_max_band[pos_energy_max_min as usize] = b as i32;

            // Again, get min. value in energyMax array
            energy_max_min = energy_max[0];
            pos_energy_max_min = 0;
            for k in 1..max_n_energy_values.max(0) as usize {
                if energy_max_min > energy_max[k] {
                    energy_max_min = energy_max[k];
                    pos_energy_max_min = k as u8;
                }
            }
        }
        b += 1;
    }
    // End determine 5 highest band-energies

    // Get tonality values for 5 highest energies
    for e in 0..max_n_energy_values.max(0) as usize {
        tonality_band[e] = 0;
        for k in 0..no_est_per_frame.max(0) {
            tonality_band[e] = tonality_band[e]
                .wrapping_add(quota(ton_corr, start_index + k, no_en_max_band[e]) >> 1);
        }
        global_tonality = global_tonality.wrapping_add(tonality_band[e] >> 2); // headroom of 2+1 (max. 5 additions)
    }

    global_tonality
}

/// The energy loops of [`get_energy_from_cplx_qmf_data`] on the real and imaginary rows `re`,
/// `im` (rows of 64 values, at least `ncols` rows, rounded up to pairs for `!full`): shifts the
/// first `nb` values of every row left by `sh` (modulo 32), writes the energies of the slots
/// (`full`) or slot pairs to `nrg` (`nb` per row) and returns their maximum (at least 0). The
/// same operations as the straight loops, on slices; on x86 with AVX2 compiled for AVX2 (the
/// 64-bit products of `fPow2Div2()` vectorize there).
#[inline]
fn qmf_energies(
    re: &mut [FixpDbl],
    im: &mut [FixpDbl],
    nb: usize,
    ncols: usize,
    sh: u32,
    full: bool,
    nrg: &mut [FixpDbl; 32 * 64],
) -> FixpDbl {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if crate::simd::avx2() {
        #[target_feature(enable = "avx2")]
        unsafe fn avx2(
            re: &mut [FixpDbl],
            im: &mut [FixpDbl],
            nb: usize,
            ncols: usize,
            sh: u32,
            full: bool,
            nrg: &mut [FixpDbl; 32 * 64],
        ) -> FixpDbl {
            qmf_energies_impl(re, im, nb, ncols, sh, full, nrg)
        }
        // SAFETY: the CPU supports AVX2.
        return unsafe { avx2(re, im, nb, ncols, sh, full, nrg) };
    }
    qmf_energies_impl(re, im, nb, ncols, sh, full, nrg)
}

#[inline(always)]
fn qmf_energies_impl(
    re: &mut [FixpDbl],
    im: &mut [FixpDbl],
    nb: usize,
    ncols: usize,
    sh: u32,
    full: bool,
    nrg: &mut [FixpDbl; 32 * 64],
) -> FixpDbl {
    let nb = nb.min(64);
    let mut max_val: FixpDbl = 0;
    let mut out = nrg.chunks_exact_mut(nb.max(1));
    if full {
        for (r, i) in re.chunks_exact_mut(64).zip(im.chunks_exact_mut(64)).take(ncols) {
            let Some(e) = out.next() else { break };
            for ((e, r), i) in e.iter_mut().zip(&mut r[..nb]).zip(&mut i[..nb]) {
                let tr0 = r.wrapping_shl(sh);
                let ti0 = i.wrapping_shl(sh);
                let energy = f_pow2_add_div2(f_pow2_div2(tr0), ti0);
                *e = energy;
                max_val = fix_max(max_val, energy);
                *r = tr0;
                *i = ti0;
            }
        }
    } else {
        for (r, i) in re.chunks_exact_mut(128).zip(im.chunks_exact_mut(128)).take(ncols.div_ceil(2))
        {
            let Some(e) = out.next() else { break };
            let (r0, r1) = r.split_at_mut(64);
            let (i0, i1) = i.split_at_mut(64);
            for ((((e, r0), r1), i0), i1) in e
                .iter_mut()
                .zip(&mut r0[..nb])
                .zip(&mut r1[..nb])
                .zip(&mut i0[..nb])
                .zip(&mut i1[..nb])
            {
                // Read QMF values of 2 timeslots, scale them and calc energy average of both
                let tr0 = r0.wrapping_shl(sh);
                let ti0 = i0.wrapping_shl(sh);
                let mut energy = f_pow2_add_div2(f_pow2_div2(tr0), ti0) >> 1;
                let tr1 = r1.wrapping_shl(sh);
                let ti1 = i1.wrapping_shl(sh);
                energy = energy.wrapping_add(f_pow2_add_div2(f_pow2_div2(tr1), ti1) >> 1);
                *e = energy;
                max_val = fix_max(max_val, energy);
                *r0 = tr0;
                *r1 = tr1;
                *i0 = ti0;
                *i1 = ti1;
            }
        }
    }
    max_val
}

/// `FDKsbrEnc_getEnergyFromCplxQmfData()` (`YBufferSzShift == 1`, energies of slot pairs) and
/// `FDKsbrEnc_getEnergyFromCplxQmfDataFull()` (`full`, energies of slots): the energies of the
/// QMF data into `YBuffer[YBufferWriteOffset..]`; the QMF data (`rBuffer[rBufferReadOffset..]`,
/// `iBuffer[..]`) is scaled up in place.
fn get_energy_from_cplx_qmf_data(
    ex: &mut SbrExtractEnvelope,
    dyn_ram: &mut [FixpDbl],
    number_bands: i32,
    number_cols: i32,
    qmf_scale: &mut i32,
    energy_scale_out: &mut i32,
    full: bool,
) {
    let nb = number_bands.clamp(0, 64) as usize;
    let ncols = number_cols.clamp(0, ENV_BUFFER_ROWS as i32) as usize;
    let ro = ex.r_buffer_read_offset.max(0) as usize;
    let mut max_val: FixpDbl = 0;

    if full {
        fdk_assert!(number_cols <= MAX_NRG_SLOTS_LD as i32);
        fdk_assert!(number_bands <= 64);
    }

    let re_off = |k: usize| ex.r_buffer + 64 * (ro + k);
    let im_off = |k: usize| ex.i_buffer + 64 * (ro + k);

    // Get max possible scaling of QMF data
    let mut scale = DFRACT_BITS;
    for k in 0..ncols {
        scale = fix_min(
            scale,
            fix_min(
                get_scalefactor(&dyn_ram[re_off(k)..re_off(k) + nb]),
                get_scalefactor(&dyn_ram[im_off(k)..im_off(k) + nb]),
            ),
        );
    }

    // Tweak scaling stability for zero signal to non-zero signal transitions
    if scale >= DFRACT_BITS - 1 {
        scale = FRACT_BITS - 1 - *qmf_scale;
    }
    // prevent scaling of QMF values to -1.f
    scale = fix_max(0, scale - 1);

    // Update QMF scale
    *qmf_scale += scale;

    // Calculate energy of each time slot (pair), max energy and shift QMF values as far as
    // possible to the left. C: `tr0 <<= scale`; the scale can exceed 31 only for a silent
    // signal (values 0 and -1), where both reference builds shift by the amount modulo 32
    // (lsl, shl), as wrapping_shl does.
    let mut tmp_nrg = [0 as FixpDbl; 32 * 64];
    let mut n = 0usize;
    let sh = scale as u32;
    // The rows read and written: the real rows end before the imaginary ones start (the layout
    // of the dynamic RAM), so the rows can be processed as separate slices (`qmf_energies`).
    let rows = if full { ncols } else { ncols.div_ceil(2) * 2 };
    let (re0, im0) = (re_off(0), im_off(0));
    if rows > 0 && re0 + 64 * rows <= im0 && im0 + 64 * rows <= dyn_ram.len() {
        let (lo, hi) = dyn_ram.split_at_mut(im0);
        max_val = qmf_energies(
            &mut lo[re0..re0 + 64 * rows],
            &mut hi[..64 * rows],
            nb,
            ncols,
            sh,
            full,
            &mut tmp_nrg,
        );
    } else if full {
        for k in 0..ncols {
            for j in 0..nb {
                let tr0 = dyn_ram[re_off(k) + j].wrapping_shl(sh);
                let ti0 = dyn_ram[im_off(k) + j].wrapping_shl(sh);
                let energy = f_pow2_add_div2(f_pow2_div2(tr0), ti0);
                tmp_nrg[n] = energy;
                n += 1;
                max_val = fix_max(max_val, energy);
                dyn_ram[re_off(k) + j] = tr0;
                dyn_ram[im_off(k) + j] = ti0;
            }
        }
    } else {
        let mut k = 0;
        while k < ncols {
            for j in 0..nb {
                // Read QMF values of 2 timeslots, scale them and calc energy average of both
                let tr0 = dyn_ram[re_off(k) + j].wrapping_shl(sh);
                let ti0 = dyn_ram[im_off(k) + j].wrapping_shl(sh);
                let mut energy = f_pow2_add_div2(f_pow2_div2(tr0), ti0) >> 1;

                let tr1 = dyn_ram[re_off(k + 1) + j].wrapping_shl(sh);
                let ti1 = dyn_ram[im_off(k + 1) + j].wrapping_shl(sh);
                energy = energy.wrapping_add(f_pow2_add_div2(f_pow2_div2(tr1), ti1) >> 1);

                // Write timeslot pair energy to scratch
                tmp_nrg[n] = energy;
                n += 1;
                max_val = fix_max(max_val, energy);

                // Write back scaled QMF values
                dyn_ram[re_off(k) + j] = tr0;
                dyn_ram[re_off(k + 1) + j] = tr1;
                dyn_ram[im_off(k) + j] = ti0;
                dyn_ram[im_off(k + 1) + j] = ti1;
            }
            k += 2;
        }
    }

    // energyScale: scalefactor energies of current frame
    *energy_scale_out = 2 * *qmf_scale - 1; // if qmfScale > 0: nr of right shifts otherwise nr of left shifts

    // Scale timeslot (pair) energies and write to output buffer
    let scale = count_leading_bits(max_val);
    let rows = if full { ncols } else { ncols >> 1 };
    let wo = ex.y_buffer_write_offset.max(0) as usize;
    for k in 0..rows {
        let idx = wo + k;
        fdk_assert!(idx < ENV_BUFFER_ROWS);
        if idx >= ENV_BUFFER_ROWS {
            break;
        }
        let row = ex.y_row_mut(idx, dyn_ram);
        scale_values_to(&mut row[..nb], &tmp_nrg[k * nb..k * nb + nb], scale);
    }
    *energy_scale_out += scale;
}

/// `mapPanorama(nrgVal, ampRes, quantError)`: quantization of the panorama value (balance).
fn map_panorama(mut nrg_val: i32, amp_res: i32, quant_error: &mut i32) -> i32 {
    let ar = amp_res.clamp(0, 1) as usize;
    let sign = if nrg_val > 0 { 1 } else { -1 };

    nrg_val = nrg_val.wrapping_mul(sign);

    let mut min_val = i32::MAX;
    let mut pan_index = 0usize;
    for i in 0..MAX_INDEX[ar] as usize {
        let val = fixp_abs(nrg_val.wrapping_sub(PAN_TABLE[ar][i] as i32));

        if val < min_val {
            min_val = val;
            pan_index = i;
        }
    }

    *quant_error = min_val;

    PAN_TABLE[ar][MAX_INDEX[ar] as usize - 1] as i32 + sign * PAN_TABLE[ar][pan_index] as i32
}

/// `sbrNoiseFloorLevelsQuantisation(iNoiseLevels, NoiseLevels, coupling)`
fn sbr_noise_floor_levels_quantisation(
    i_noise_levels: &mut [i8; MAX_NUM_NOISE_VALUES],
    noise_levels: &[FixpDbl; MAX_NUM_NOISE_VALUES],
    coupling: i32,
) {
    let mut dummy = 0;

    // Quantisation, similar to sfb quant...
    for i in 0..MAX_NUM_NOISE_VALUES {
        // 30>>LD_DATA_SHIFT = 0.46875
        let mut tmp = if noise_levels[i] > fl2fxconst_dbl(0.46875f32 as f64) {
            30
        } else {
            // rounding is done by shifting one bit less than necessary to the right, adding '1'
            // and then shifting the final bit
            let mut t = noise_levels[i] >> (DFRACT_BITS - 1 - LD_DATA_SHIFT); // conversion to integer
            if t != 0 {
                t += 1;
            }
            t
        };

        if coupling != 0 {
            tmp = if tmp < -30 { -30 } else { tmp };
            tmp = map_panorama(tmp, 1, &mut dummy);
        }
        i_noise_levels[i] = tmp as i8;
    }
}

/// `coupleNoiseFloor(noise_level_left, noise_level_right)`: the noise floor for coupling.
fn couple_noise_floor(
    noise_level_left: &mut [FixpDbl; MAX_NUM_NOISE_VALUES],
    noise_level_right: &mut [FixpDbl; MAX_NUM_NOISE_VALUES],
) {
    // C: temp << (DFRACT_BITS - 1 - LD_DATA_SHIFT - 1), a signed shift that overflows for noise
    // levels below -0.0625 (both reference builds shift with lsl/shl, as wrapping_shl).
    const INT_TO_FRACT: u32 = (DFRACT_BITS - 1 - LD_DATA_SHIFT - 1) as u32;
    let c7: FixpDbl = fl2fxconst_dbl(0.109375f32 as f64); // scaled with 7/64

    for i in 0..MAX_NUM_NOISE_VALUES {
        // Calculation of the power function using ld64:
        //   z  = x^y;
        //   z' = CalcLd64(z) = y*CalcLd64(x)/64;
        //   z  = CalcInvLd64(z');
        let cmp_val_left = NOISE_FLOOR_OFFSET_64.wrapping_sub(noise_level_left[i]);
        let cmp_val_right = NOISE_FLOOR_OFFSET_64.wrapping_sub(noise_level_right[i]);

        let mut temp1 = calc_inv_ld_data(NOISE_FLOOR_OFFSET_64.wrapping_sub(noise_level_right[i]));
        if cmp_val_right >= 0 {
            // INT to fract conversion of result, if input of CalcInvLdData is positiv
            temp1 = temp1.wrapping_shl(INT_TO_FRACT);
        }

        let mut temp2 = calc_inv_ld_data(NOISE_FLOOR_OFFSET_64.wrapping_sub(noise_level_left[i]));
        if cmp_val_left >= 0 {
            temp2 = temp2.wrapping_shl(INT_TO_FRACT);
        }

        if cmp_val_left < 0 && cmp_val_right < 0 {
            noise_level_left[i] = NOISE_FLOOR_OFFSET_64
                .wrapping_sub(calc_ld_data((temp1 >> 1).wrapping_add(temp2 >> 1))); // no scaling needed! both values are dfract
            noise_level_right[i] = calc_ld_data(temp2).wrapping_sub(calc_ld_data(temp1));
        }

        if cmp_val_left >= 0 && cmp_val_right >= 0 {
            noise_level_left[i] = NOISE_FLOOR_OFFSET_64
                .wrapping_sub(calc_ld_data((temp1 >> 1).wrapping_add(temp2 >> 1)).wrapping_add(c7));
            noise_level_right[i] = calc_ld_data(temp2).wrapping_sub(calc_ld_data(temp1));
        }

        if cmp_val_left >= 0 && cmp_val_right < 0 {
            noise_level_left[i] = NOISE_FLOOR_OFFSET_64.wrapping_sub(
                calc_ld_data((temp1 >> (7 + 1)).wrapping_add(temp2 >> 1)).wrapping_add(c7),
            );
            noise_level_right[i] =
                calc_ld_data(temp2).wrapping_add(c7).wrapping_sub(calc_ld_data(temp1));
        }

        if cmp_val_left < 0 && cmp_val_right >= 0 {
            noise_level_left[i] = NOISE_FLOOR_OFFSET_64.wrapping_sub(
                calc_ld_data((temp1 >> 1).wrapping_add(temp2 >> (7 + 1))).wrapping_add(c7),
            );
            noise_level_right[i] =
                calc_ld_data(temp2).wrapping_sub(calc_ld_data(temp1).wrapping_add(c7));
        }
    }
}

/// `getEnvSfbEnergy(li, ui, start_pos, stop_pos, border_pos, YBuffer, YBufferSzShift,
/// scaleNrg0, scaleNrg1)`: the energy from band `li` up to band `ui` over the slots `start_pos`
/// to `stop_pos`.
#[allow(clippy::too_many_arguments)]
fn get_env_sfb_energy(
    li: i32,
    ui: i32,
    start_pos: i32,
    stop_pos: i32,
    border_pos: i32,
    y_buffer: &[&[FixpDbl]],
    y_buffer_sz_shift: i32,
    scale_nrg0: i32,
    scale_nrg1: i32,
) -> FixpDbl {
    // use dynamic scaling for outer energy loop; energies are critical and every bit is
    // important
    let dyn_scale = if ui - li == 0 {
        DFRACT_BITS - 1
    } else {
        calc_ld_int(ui - li) >> (DFRACT_BITS - 1 - LD_DATA_SHIFT)
    };

    let sc0 = fix_min(scale_nrg0, Y_NRG_SCALE);
    let sc1 = fix_min(scale_nrg1, Y_NRG_SCALE);
    // dynScale{1,2} is set such that the right shift below is positive
    let dyn_scale1 = fix_min(scale_nrg0 - sc0, dyn_scale);
    let dyn_scale2 = fix_min(scale_nrg1 - sc1, dyn_scale);
    let mut nrg_sum: FixpDbl = 0;
    let mut accu1: FixpDbl = 0;
    let mut accu2: FixpDbl = 0;

    let y = |l: i32, k: i32| -> FixpDbl {
        // YBuffer[l >> YBufferSzShift][k]
        let r = l.wrapping_shr(y_buffer_sz_shift as u32);
        y_buffer
            .get(r.max(0) as usize)
            .and_then(|row| row.get(k.max(0) as usize))
            .copied()
            .unwrap_or(0)
    };

    // The shift amounts are >= 0 for the scales of the encoder (FDK_ASSERT in
    // calculateSbrEnvelope()); both reference builds take them modulo 32 (asr, sar).
    for k in li..ui {
        let mut nrg1: FixpDbl = 0;
        let mut nrg2: FixpDbl = 0;
        let mut l = start_pos;
        while l < border_pos {
            nrg1 = nrg1.wrapping_add(y(l, k).wrapping_shr(sc0 as u32));
            l += 1;
        }
        while l < stop_pos {
            nrg2 = nrg2.wrapping_add(y(l, k).wrapping_shr(sc1 as u32));
            l += 1;
        }
        accu1 = f_add_saturate(accu1, nrg1.wrapping_shr(dyn_scale1 as u32));
        accu2 = f_add_saturate(accu2, nrg2.wrapping_shr(dyn_scale2 as u32));
    }
    // This shift factor is always positive. See comment above.
    nrg_sum = nrg_sum.wrapping_add(
        accu1
            .wrapping_shr(fix_min(scale_nrg0 - sc0 - dyn_scale1, DFRACT_BITS - 1) as u32)
            .wrapping_add(
                accu2.wrapping_shr(fix_min(scale_nrg1 - sc1 - dyn_scale2, DFRACT_BITS - 1) as u32),
            ),
    );

    nrg_sum
}

/// `mhLoweringEnergy(nrg, M)`: energy compensation in missing harmonic mode.
fn mh_lowering_energy(mut nrg: FixpDbl, m: i32) -> FixpDbl {
    // Compensating for the fact that we in the decoder map the "average energy to every QMF
    // band, and use this when we calculate the boost-factor. Since the mapped energy isn't the
    // average energy but the maximum energy in case of missing harmonic creation, we will in
    // the boost function calculate that too much limiting has been applied and hence we will
    // boost the signal although it isn't called for. Hence we need to compensate for this by
    // lowering the transmitted energy values for the sines so they will get the correct level
    // after the boost is applied.
    if m > 2 {
        let tmp_scale = count_leading_bits(nrg);
        nrg <<= tmp_scale;
        // The maximum boost is 1.584893, so the maximum attenuation should be
        // square(1/1.584893) = 0.398107267
        nrg = f_mult(nrg, fl2fxconst_dbl(0.398107267f32 as f64));
        nrg >>= tmp_scale;
    } else if m > 1 {
        nrg >>= 1;
    }

    nrg
}

/// `nmhLoweringEnergy(nrg, nrgSum, nrgSum_scale, M)`: energy compensation in none missing
/// harmonic mode.
fn nmh_lowering_energy(mut nrg: FixpDbl, nrg_sum: FixpDbl, nrg_sum_scale: i32, m: i32) -> FixpDbl {
    if nrg > 0 {
        let mut sc = 0;
        // gain = nrgSum / (nrg*(M+1))
        let gain = f_mult(f_div_norm(nrg_sum, nrg, &mut sc), get_inv_int(m + 1));
        sc += nrg_sum_scale;

        // reduce nrg if gain smaller 1.f
        // C: (FIXP_DBL)MAXVAL_DBL >> sc for sc >= 0 (sc < 32 for the scales of the encoder)
        if !(sc >= 0 && gain > MAXVAL_DBL.wrapping_shr(sc as u32)) {
            nrg = f_mult(scale_value(gain, sc), nrg);
        }
    }
    nrg
}

/// The stereo inputs of [`calculate_sbr_envelope`] for `SBR_COUPLING`.
struct CouplingInput<'a> {
    /// `YBufferRight`
    y_buffer_right: [&'a [FixpDbl]; ENV_BUFFER_ROWS],
    /// `YBufferScaleRight`
    y_buffer_scale_right: [i32; 2],
    /// `sfb_nrgRight`
    sfb_nrg_right: &'a mut [i8; MAX_NUM_ENVELOPE_VALUES],
    /// `maxQuantError`
    max_quant_error: &'a mut i32,
}

/// `calculateSbrEnvelope(YBufferLeft, YBufferRight, YBufferScaleLeft, YBufferScaleRight,
/// frame_info, sfb_nrgLeft, sfb_nrgRight, h_con, h_sbr, stereoMode, maxQuantError,
/// YBufferSzShift)`: the envelope values from the energies. `coupling` is `None` for
/// `SBR_MONO`.
#[allow(clippy::too_many_arguments)]
fn calculate_sbr_envelope(
    y_buffer_left: &[&[FixpDbl]],
    y_buffer_scale_left: &[i32; 2],
    mut coupling: Option<CouplingInput>,
    frame_info: &SbrFrameInfo,
    sfb_nrg_left: &mut [i8; MAX_NUM_ENVELOPE_VALUES],
    h_con: &SbrConfigData,
    h_sbr: &EnvChannel,
    y_buffer_sz_shift: i32,
) {
    let mut m = 0usize;

    let ca = 2 - h_sbr.enc_env_data.init_sbr_amp_res;
    let one_bit_less = if ca == 2 { 1 } else { 0 }; // LD_DATA_SHIFT => ld64 scaling; one bit less for rounding

    let mut quant_error = 0;
    let n_envelopes = frame_info.n_envelopes;
    let short_env = frame_info.short_env - 1;
    let time_step = h_sbr.sbr_extract_envelope.time_step;
    let mut scale_right0 = 0;
    let mut scale_right1 = 0;

    let mut common_scale = fix_min(y_buffer_scale_left[0], y_buffer_scale_left[1]);

    if let Some(c) = &coupling {
        common_scale = fix_min(common_scale, c.y_buffer_scale_right[0]);
        common_scale = fix_min(common_scale, c.y_buffer_scale_right[1]);
    }

    common_scale -= 7;

    let scale_left0 = y_buffer_scale_left[0] - common_scale;
    let scale_left1 = y_buffer_scale_left[1] - common_scale;
    fdk_assert!(scale_left0 >= 0 && scale_left1 >= 0);

    if let Some(c) = &mut coupling {
        scale_right0 = c.y_buffer_scale_right[0] - common_scale;
        scale_right1 = c.y_buffer_scale_right[1] - common_scale;
        fdk_assert!(scale_right0 >= 0 && scale_right1 >= 0);
        *c.max_quant_error = 0;
    }

    let fbt_hi = h_con.freq_band_table(HI);
    let fbt_lo = h_con.freq_band_table(LO);
    let fbt = |res: FreqRes, j: i32| -> i32 {
        let t = if res == FREQ_RES_HIGH { fbt_hi } else { fbt_lo };
        t.get(j.max(0) as usize).copied().unwrap_or(0) as i32
    };

    for env in 0..n_envelopes.clamp(0, MAX_ENVELOPES as i32) as usize {
        // C: pNrgLeft[32], pNrgRight[32], missingHarmonic[32], count[32]; the band count of the
        // frequency tables of the encoder's configurations stays below 32 (bounded: the port
        // clamps the band count).
        let mut p_nrg_left = [0 as FixpDbl; MAX_FREQ_COEFFS];
        let mut p_nrg_right = [0 as FixpDbl; MAX_FREQ_COEFFS];
        let mut env_nrg_left: FixpDbl = 0;
        let mut env_nrg_right: FixpDbl = 0;
        let mut missing_harmonic = [0i32; MAX_FREQ_COEFFS];
        let mut count = [0i32; MAX_FREQ_COEFFS];

        let start_pos = time_step.wrapping_mul(frame_info.borders[env]);
        let mut stop_pos = time_step.wrapping_mul(frame_info.borders[env + 1]);
        let freq_res = frame_info.freq_res[env];
        let no_of_bands = h_con.n_sfb[(freq_res == FREQ_RES_HIGH) as usize];
        let env_nrg_scale = DFRACT_BITS - f_normz(no_of_bands as FixpDbl);
        if env as i32 == short_env {
            let j = f_max(2, time_step); // consider at least 2 QMF slots less for short envelopes (envelopes just before transients)
            if stop_pos - start_pos - j > 0 {
                stop_pos -= j;
            }
        }
        let nb = no_of_bands.clamp(0, MAX_FREQ_COEFFS as i32) as usize;
        for j in 0..nb {
            let mut nrg_left: FixpDbl;
            let mut nrg_right: FixpDbl = 0;

            let mut li = fbt(freq_res, j as i32);
            let ui = fbt(freq_res, j as i32 + 1);

            if freq_res == FREQ_RES_HIGH {
                if j == 0 && ui - li > 1 {
                    li += 1;
                }
            } else if j == 0 && ui - li > 2 {
                li += 1;
            }

            // Find out whether a sine will be missing in the scale-factor band that we're
            // currently processing.
            missing_harmonic[j] = 0;

            if h_sbr.enc_env_data.add_harmonic_flag != 0 {
                if freq_res == FREQ_RES_HIGH {
                    if h_sbr.enc_env_data.add_harmonic[j] != 0 {
                        // A missing sine in the current band
                        missing_harmonic[j] = 1;
                    }
                } else {
                    let mut start_band_high = 0i32;
                    let mut stop_band_high = 0i32;

                    // Bounded: the searches stop at the end of the table.
                    while (start_band_high as usize) < fbt_hi.len()
                        && (fbt_hi[start_band_high as usize] as i32) < fbt(FREQ_RES_LOW, j as i32)
                    {
                        start_band_high += 1;
                    }
                    while (stop_band_high as usize) < fbt_hi.len()
                        && (fbt_hi[stop_band_high as usize] as i32)
                            < fbt(FREQ_RES_LOW, j as i32 + 1)
                    {
                        stop_band_high += 1;
                    }

                    for i in start_band_high..stop_band_high {
                        if h_sbr.enc_env_data.add_harmonic.get(i as usize).copied().unwrap_or(0)
                            != 0
                        {
                            missing_harmonic[j] = 1;
                        }
                    }
                }
            }

            // If a sine is missing in a scalefactorband, with more than one qmf channel use
            // the nrg from the channel with the largest nrg rather than the mean. Compensate
            // for the boost calculation in the decdoder.
            let border_pos = fix_min(
                stop_pos,
                h_sbr
                    .sbr_extract_envelope
                    .y_buffer_write_offset
                    .wrapping_shl(y_buffer_sz_shift as u32),
            );

            if missing_harmonic[j] != 0 {
                count[j] = stop_pos - start_pos;
                nrg_left = 0;

                for k in li..ui {
                    let tmp_nrg = get_env_sfb_energy(
                        k,
                        k + 1,
                        start_pos,
                        stop_pos,
                        border_pos,
                        y_buffer_left,
                        y_buffer_sz_shift,
                        scale_left0,
                        scale_left1,
                    );

                    nrg_left = fix_max(nrg_left, tmp_nrg);
                }

                // Energy lowering compensation
                nrg_left = mh_lowering_energy(nrg_left, ui - li);

                if let Some(c) = &coupling {
                    nrg_right = 0;

                    for k in li..ui {
                        let tmp_nrg = get_env_sfb_energy(
                            k,
                            k + 1,
                            start_pos,
                            stop_pos,
                            border_pos,
                            &c.y_buffer_right,
                            y_buffer_sz_shift,
                            scale_right0,
                            scale_right1,
                        );

                        nrg_right = fix_max(nrg_right, tmp_nrg);
                    }

                    // Energy lowering compensation
                    nrg_right = mh_lowering_energy(nrg_right, ui - li);
                }
            } else {
                // end missingHarmonic
                count[j] = (stop_pos - start_pos).wrapping_mul(ui - li);

                nrg_left = get_env_sfb_energy(
                    li,
                    ui,
                    start_pos,
                    stop_pos,
                    border_pos,
                    y_buffer_left,
                    y_buffer_sz_shift,
                    scale_left0,
                    scale_left1,
                );

                if let Some(c) = &coupling {
                    nrg_right = get_env_sfb_energy(
                        li,
                        ui,
                        start_pos,
                        stop_pos,
                        border_pos,
                        &c.y_buffer_right,
                        y_buffer_sz_shift,
                        scale_right0,
                        scale_right1,
                    );
                }
            } // !missingHarmonic

            // save energies
            p_nrg_left[j] = nrg_left;
            p_nrg_right[j] = nrg_right;
            env_nrg_left = env_nrg_left.wrapping_add(nrg_left.wrapping_shr(env_nrg_scale as u32));
            env_nrg_right =
                env_nrg_right.wrapping_add(nrg_right.wrapping_shr(env_nrg_scale as u32));
        } // j

        for j in 0..nb {
            let mut nrg_left2: FixpDbl = 0;
            let mut nrg_left = p_nrg_left[j];
            let mut nrg_right = p_nrg_right[j];

            // None missing harmonic Energy lowering compensation
            if missing_harmonic[j] == 0 && h_sbr.f_level_protect != 0 {
                // in case of missing energy in base band, reduce reference energy to prevent
                // overflows in decoder output
                nrg_left = nmh_lowering_energy(nrg_left, env_nrg_left, env_nrg_scale, no_of_bands);
                if coupling.is_some() {
                    nrg_right =
                        nmh_lowering_energy(nrg_right, env_nrg_right, env_nrg_scale, no_of_bands);
                }
            }

            if coupling.is_some() {
                // calc operation later with log
                nrg_left2 = nrg_left;
                nrg_left = nrg_right.wrapping_add(nrg_left) >> 1;
            }

            // nrgLeft = f20_log2(nrgLeft / (PFLOAT)(count * 64))+(PFLOAT)44;
            // If nrgLeft == 0 then the Log calculations below do fail.
            if nrg_left > 0 {
                let tmp_scale = count_leading_bits(nrg_left);
                nrg_left <<= tmp_scale;

                let tmp0 = calc_ld_data(nrg_left); // scaled by 1/64
                                                   // C: ((FIXP_DBL)(commonScale + tmpScale)) << (DFRACT_BITS - 1 - LD_DATA_SHIFT - 1)
                let tmp1 = (common_scale + tmp_scale)
                    .wrapping_shl((DFRACT_BITS - 1 - LD_DATA_SHIFT - 1) as u32); // scaled by 1/64
                                                                                 // C: ((FIXP_DBL)(count[j] * 64)) << (DFRACT_BITS - 1 - 14 - 1); the shift
                                                                                 // overflows for counts of 2^14 / 64 and more (wide bands of long envelopes); both
                                                                                 // reference builds shift with lsl/shl, as wrapping_shl.
                let tmp2 =
                    count[j].wrapping_mul(64).wrapping_shl((DFRACT_BITS - 1 - 14 - 1) as u32);
                let tmp2 = calc_ld_data(tmp2); // scaled by 1/64
                let tmp3: FixpDbl =
                    fl2fxconst_dbl((0.6875f32 - 0.21875f32 - 0.015625f32) as f64) >> 1; // scaled by 1/64

                nrg_left = (tmp0.wrapping_sub(tmp2) >> 1).wrapping_add(tmp3.wrapping_sub(tmp1));
            } else {
                nrg_left = fl2fxconst_dbl(-1.0);
            }

            // ld64 to integer conversion
            nrg_left = fix_min(fix_max(nrg_left, 0), fl2fxconst_dbl(0.5) >> one_bit_less);
            nrg_left >>= DFRACT_BITS - 1 - LD_DATA_SHIFT - 1 - one_bit_less - 1;
            sfb_nrg_left[m] = (nrg_left.wrapping_add(1) >> 1) as i8; // rounding

            if let Some(c) = &mut coupling {
                nrg_left2 = fix_max(0x1, nrg_left2);
                nrg_right = fix_max(0x1, nrg_right);

                let sc0 = count_leading_bits(nrg_left2);
                let sc1 = count_leading_bits(nrg_right);

                // scale value in ld64 representation; C: ((FIXP_DBL)(sc0 - sc1)) << 25
                let scale_fract =
                    (sc0 - sc1).wrapping_shl((DFRACT_BITS - 1 - LD_DATA_SHIFT) as u32);
                nrg_right = calc_ld_data(nrg_left2 << sc0)
                    .wrapping_sub(calc_ld_data(nrg_right << sc1))
                    .wrapping_sub(scale_fract);

                // ld64 to integer conversion
                nrg_right >>= DFRACT_BITS - 1 - LD_DATA_SHIFT - 1 - one_bit_less;
                nrg_right = nrg_right.wrapping_add(1) >> 1; // rounding

                c.sfb_nrg_right[m] =
                    map_panorama(nrg_right, h_sbr.enc_env_data.init_sbr_amp_res, &mut quant_error)
                        as i8;

                *c.max_quant_error = fix_max(quant_error, *c.max_quant_error);
            }

            m += 1;
        } // j

        // Do energy compensation for sines that are present in two QMF-bands in the
        // original, but will only occur in one band in the decoder due to the synthetic sine
        // coding.
        if h_con.use_parametric_coding != 0 {
            m -= nb;
            for j in 0..nb {
                if freq_res == FREQ_RES_HIGH
                    && h_sbr.sbr_extract_envelope.envelope_compensation[j] != 0
                {
                    // C: sfb_nrgLeft[m] -= (ca * fixp_abs((INT)envelopeCompensation[j])) in
                    // int, stored to SCHAR
                    let v = sfb_nrg_left[m] as i32
                        - ca * fixp_abs(h_sbr.sbr_extract_envelope.envelope_compensation[j] as i32);
                    sfb_nrg_left[m] = v as i8;
                }
                sfb_nrg_left[m] = fix_max(0, sfb_nrg_left[m] as i32) as i8;
                m += 1;
            }
        } // useParametricCoding
    } // env loop
}

/// `FDKsbrEnc_extractSbrEnvelope1(h_con, sbrHeaderData, sbrBitstreamData, hEnvChan, hCmonData,
/// eData, fData)`: extracts the features of the QMF data (energies, tonality, transients);
/// afterwards the QMF data is not required anymore.
pub fn fdk_sbr_enc_extract_sbr_envelope1(
    h_con: &SbrConfigData,
    h_env_chan: &mut EnvChannel,
    e_data: &mut SbrEnvTempData,
    dyn_ram: &mut [FixpDbl],
) {
    let full = h_env_chan.sbr_extract_envelope.y_buffer_sz_shift == 0;
    {
        let ex = &mut h_env_chan.sbr_extract_envelope;
        let mut scale1 = ex.y_buffer_scale[1];
        let no_cols = ex.no_cols;
        get_energy_from_cplx_qmf_data(
            ex,
            dyn_ram,
            h_con.no_qmf_bands,
            no_cols,
            &mut h_env_chan.qmf_scale,
            &mut scale1,
            full,
        );
        ex.y_buffer_scale[1] = scale1;
    }

    // Energie values = YBuffer[YBufferWriteOffset][x].floatVal * (1<<2*7-YBufferScale[1])

    // Precalculation of Tonality Quotas  COEFF Transform OK
    {
        let (re, im) = h_env_chan.sbr_extract_envelope.qmf_rows(dyn_ram);
        let usb = h_con
            .freq_band_table(HI)
            .get(h_con.n_sfb[HI].clamp(0, MAX_FREQ_COEFFS as i32) as usize)
            .copied()
            .unwrap_or(0) as i32;
        fdk_sbr_enc_calculate_tonality_quotas(
            &mut h_env_chan.ton_corr,
            &re,
            &im,
            usb,
            h_env_chan.qmf_scale,
        );
    }

    let ex = &h_env_chan.sbr_extract_envelope;
    let rows = ex.y_rows(dyn_ram);
    let wo = ex.y_buffer_write_offset.clamp(0, ENV_BUFFER_ROWS as i32) as usize;

    if h_con.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        let tonality = fdk_sbr_enc_get_tonality(
            &h_env_chan.ton_corr,
            h_env_chan.ton_corr.number_of_estimates_per_frame as i32,
            h_env_chan.ton_corr.start_index_matrix,
            &rows[wo..],
            h_con.freq_band_table(HI)[0].wrapping_add(1),
            h_con.no_qmf_bands,
            ex.no_cols,
        );

        let ed = &mut h_env_chan.enc_env_data;
        ed.ton_hf[1] = ed.ton_hf[0];
        ed.ton_hf[0] = tonality;

        // tonality is scaled by 2^19/0.524288f (fract part of RELAXATION)
        ed.global_tonality = (ed.ton_hf[0] >> 1).wrapping_add(ed.ton_hf[1] >> 1);
    }

    // Transient detection COEFF Transform OK
    if h_con.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        fdk_sbr_enc_fast_transient_detect(
            &mut h_env_chan.sbr_fast_transient_detector,
            &rows,
            &ex.y_buffer_scale,
            ex.y_buffer_write_offset,
            &mut e_data.transient_info,
        );
    } else {
        fdk_sbr_enc_transient_detect(
            &mut h_env_chan.sbr_transient_detector,
            &rows,
            &ex.y_buffer_scale,
            &mut e_data.transient_info,
            ex.y_buffer_write_offset,
            ex.y_buffer_sz_shift,
            ex.time_step,
            h_env_chan.sbr_env_frame.frame_middle_slot,
        );
    }

    // Generate flags for 2 env in a FIXFIX-frame. Remove this function to get always 1 env
    // per FIXFIX-frame.

    // frame Splitter COEFF Transform OK
    fdk_sbr_enc_frame_splitter(
        &rows,
        &ex.y_buffer_scale,
        &mut h_env_chan.sbr_transient_detector,
        h_con.freq_band_table(1),
        &mut e_data.transient_info,
        ex.y_buffer_write_offset,
        ex.y_buffer_sz_shift,
        h_con.n_sfb[1],
        ex.time_step,
        ex.no_cols,
        &mut h_env_chan.enc_env_data.global_tonality,
    );
}

/// The envelope data and bitstream grid of channel `ch` of an element.
fn env_ref<'a>(chans: &'a [&mut EnvChannel], ch: usize) -> EnvRef<'a> {
    let g = chans[ch].enc_env_data.h_sbr_bs_grid.unwrap_or(ch).min(chans.len() - 1);
    EnvRef { data: &chans[ch].enc_env_data, grid: &chans[g].sbr_env_frame.sbr_grid }
}

/// `FDKsbrEnc_codeEnvelope()` on the noise floor of channel `ch`.
fn code_noise(
    chan: &mut EnvChannel,
    levels: &mut [i8; MAX_NUM_NOISE_VALUES],
    res: &[FreqRes; MAX_NUM_NOISE_VALUES],
    coupling: i32,
    n_env: i32,
    channel: i32,
    header_active: i32,
) {
    fdk_sbr_enc_code_envelope(
        levels,
        res,
        &mut chan.sbr_code_noise_floor,
        &mut chan.enc_env_data.domain_vec_noise,
        coupling,
        if n_env > 1 { 2 } else { 1 },
        channel,
        header_active,
    );
}

/// `FDKsbrEnc_codeEnvelope()` on the envelope of channel `ch`.
fn code_env(
    chan: &mut EnvChannel,
    sfb_nrg: &mut [i8; MAX_NUM_ENVELOPE_VALUES],
    frame_info: &SbrFrameInfo,
    coupling: i32,
    channel: i32,
    header_active: i32,
) {
    fdk_sbr_enc_code_envelope(
        sfb_nrg,
        &frame_info.freq_res,
        &mut chan.sbr_code_envelope,
        &mut chan.enc_env_data.domain_vec,
        coupling,
        frame_info.n_envelopes,
        channel,
        header_active,
    );
}

/// `FDKsbrEnc_extractSbrEnvelope2(h_con, sbrHeaderData, hParametricStereo, sbrBitstreamData,
/// h_envChan0, h_envChan1, hCmonData, eData, fData, clearOutput)`: the time/frequency division,
/// the tonality correction parameters, the envelopes, the noise floor and the stereo coding
/// decision, and writes the SBR data. `h_env_chan` holds the channels of the element
/// (`h_envChan1` is `h_env_chan[1]`).
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_extract_sbr_envelope2(
    h_con: &SbrConfigData,
    sbr_header_data: &mut SbrHeaderData,
    h_parametric_stereo: Option<&ParametricStereo>,
    sbr_bitstream_data: &SbrBitstreamData,
    h_env_chan: &mut [&mut EnvChannel],
    h_cmon_data: &mut CommonData,
    e_data: &mut [SbrEnvTempData; MAX_NUM_CHANNELS],
    f_data: &mut SbrFrameTempData,
    clear_output: i32,
    dyn_ram: &mut [FixpDbl],
) {
    let y_sz_shift = h_env_chan[0].sbr_extract_envelope.y_buffer_sz_shift;

    let mut stereo_mode = h_con.stereo_mode;
    let n_channels = (h_con.n_channels.max(0) as usize).min(h_env_chan.len());

    let v_tuning: &'static [i32] = if h_con.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        &V_TUNING_ELD
    } else {
        &V_TUNING_HEAAC
    };

    // Select stereo mode.
    if stereo_mode == SBR_COUPLING {
        if e_data[0].transient_info[1] != 0 && e_data[1].transient_info[1] != 0 {
            e_data[0].transient_info[0] =
                fix_min(e_data[1].transient_info[0], e_data[0].transient_info[0]);
            e_data[1].transient_info[0] = e_data[0].transient_info[0];
        } else if e_data[0].transient_info[1] != 0 && e_data[1].transient_info[1] == 0 {
            e_data[1].transient_info[0] = e_data[0].transient_info[0];
        } else if e_data[0].transient_info[1] == 0 && e_data[1].transient_info[1] != 0 {
            e_data[0].transient_info[0] = e_data[1].transient_info[0];
        } else {
            e_data[0].transient_info[0] =
                fix_max(e_data[1].transient_info[0], e_data[0].transient_info[0]);
            e_data[1].transient_info[0] = e_data[0].transient_info[0];
        }
    }

    // Determine time/frequency division of current granule
    {
        let c0 = &mut *h_env_chan[0];
        let ld = c0.enc_env_data.ld_grid as i32;
        let pre = c0.sbr_extract_envelope.pre_transient_info;
        fdk_sbr_enc_frame_info_generator(
            &mut c0.sbr_env_frame,
            &e_data[0].transient_info,
            sbr_bitstream_data.right_border_fix,
            &pre,
            ld,
            v_tuning,
        );
        e_data[0].frame_info = Some(0);

        c0.enc_env_data.h_sbr_bs_grid = Some(0);

        // AAC LD patch for transient prediction
        if c0.enc_env_data.ld_grid != 0 && e_data[0].transient_info[2] != 0 {
            // if next frame will start with transient, set shortEnv to
            // numEnvelopes(shortend Envelope = shortEnv-1)
            c0.sbr_env_frame.sbr_frame_info.short_env = c0.sbr_env_frame.sbr_frame_info.n_envelopes;
        }
    }

    match stereo_mode {
        SBR_LEFT_RIGHT | SBR_SWITCH_LRC => {
            {
                let c1 = &mut *h_env_chan[1];
                let ld = c1.enc_env_data.ld_grid as i32;
                let pre = c1.sbr_extract_envelope.pre_transient_info;
                fdk_sbr_enc_frame_info_generator(
                    &mut c1.sbr_env_frame,
                    &e_data[1].transient_info,
                    sbr_bitstream_data.right_border_fix,
                    &pre,
                    ld,
                    v_tuning,
                );
                e_data[1].frame_info = Some(1);

                c1.enc_env_data.h_sbr_bs_grid = Some(1);

                if c1.enc_env_data.ld_grid != 0 && e_data[1].transient_info[2] != 0 {
                    // if next frame will start with transient, set shortEnv to
                    // numEnvelopes(shortend Envelope = shortEnv-1)
                    c1.sbr_env_frame.sbr_frame_info.short_env =
                        c1.sbr_env_frame.sbr_frame_info.n_envelopes;
                }
            }

            // compare left and right frame_infos
            let f0 = h_env_chan[0].sbr_env_frame.sbr_frame_info;
            let f1 = h_env_chan[1].sbr_env_frame.sbr_frame_info;
            if f0.n_envelopes != f1.n_envelopes {
                stereo_mode = SBR_LEFT_RIGHT;
            } else {
                for i in 0..(f0.n_envelopes + 1).clamp(0, MAX_ENVELOPES as i32 + 1) as usize {
                    if f0.borders[i] != f1.borders[i] {
                        stereo_mode = SBR_LEFT_RIGHT;
                        break;
                    }
                }
                for i in 0..f0.n_envelopes.clamp(0, MAX_ENVELOPES as i32) as usize {
                    if f0.freq_res[i] != f1.freq_res[i] {
                        stereo_mode = SBR_LEFT_RIGHT;
                        break;
                    }
                }
                if f0.short_env != f1.short_env {
                    stereo_mode = SBR_LEFT_RIGHT;
                }
            }
        }
        SBR_COUPLING => {
            e_data[1].frame_info = e_data[0].frame_info;
            h_env_chan[1].enc_env_data.h_sbr_bs_grid = Some(0);
        }
        SBR_MONO => {
            // nothing to do
        }
        _ => {
            fdk_assert!(false);
        }
    }

    // The frame info of channel ch (`eData[ch].frame_info`).
    let frame_info_of = |chans: &[&mut EnvChannel], ed: &SbrEnvTempData| -> SbrFrameInfo {
        chans[ed.frame_info.unwrap_or(0).min(chans.len() - 1)].sbr_env_frame.sbr_frame_info
    };

    for ch in 0..n_channels {
        let fi = frame_info_of(h_env_chan, &e_data[ch]);
        let grid_ch =
            h_env_chan[ch].enc_env_data.h_sbr_bs_grid.unwrap_or(ch).min(h_env_chan.len() - 1);
        let grid_class = h_env_chan[grid_ch].sbr_env_frame.sbr_grid.frame_class;
        let h_env = &mut *h_env_chan[ch];
        let ed = &mut e_data[ch];

        // Send transient info to bitstream and store for next call
        h_env.sbr_extract_envelope.pre_transient_info[0] = ed.transient_info[0]; // tran_pos
        h_env.sbr_extract_envelope.pre_transient_info[1] = ed.transient_info[1]; // tran_flag
        ed.n_envelopes = fi.n_envelopes as u8;
        h_env.enc_env_data.no_of_envelopes = ed.n_envelopes as i32; // number of envelopes of current frame
        h_env.enc_env_data.current_amp_res_ff = h_con.init_amp_res_ff as AmpRes;

        // Check if the current frame is divided into one envelope only. If so, set the
        // amplitude resolution to 1.5 dB, otherwise may set back to chosen value
        if grid_class == FIXFIX && ed.n_envelopes == 1 {
            let mut current_amp_res_ff = SBR_AMP_RES_1_5;
            if h_con.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
                // decision of SBR_AMP_RES
                if fdk_aac_rs_fdk::fixpoint_math::f_is_less_than(
                    // global_tonality > threshold ?
                    h_con.threshold_amp_res_ff_m,
                    h_con.threshold_amp_res_ff_e as i32,
                    h_env.enc_env_data.global_tonality,
                    RELAXATION_SHIFT + 2,
                ) != 0
                {
                    h_env.enc_env_data.current_amp_res_ff = SBR_AMP_RES_1_5;
                } else {
                    h_env.enc_env_data.current_amp_res_ff = SBR_AMP_RES_3_0;
                }
                current_amp_res_ff = h_env.enc_env_data.current_amp_res_ff;
            }

            if current_amp_res_ff != h_env.enc_env_data.init_sbr_amp_res {
                fdk_sbr_enc_init_sbr_huffman_tables(
                    &mut h_env.enc_env_data,
                    &mut h_env.sbr_code_envelope,
                    &mut h_env.sbr_code_noise_floor,
                    current_amp_res_ff,
                );
            }
        } else if sbr_header_data.sbr_amp_res != h_env.enc_env_data.init_sbr_amp_res {
            fdk_sbr_enc_init_sbr_huffman_tables(
                &mut h_env.enc_env_data,
                &mut h_env.sbr_code_envelope,
                &mut h_env.sbr_code_noise_floor,
                sbr_header_data.sbr_amp_res,
            );
        }

        if clear_output == 0 {
            // Tonality correction parameter extraction (inverse filtering level, noise floor
            // additional sines).
            let tail = h_env.enc_env_data.add_harmonic_tail();
            let xpos_mode = h_env.enc_env_data.sbr_xpos_mode;
            fdk_sbr_enc_ton_corr_param_extr(
                &mut h_env.ton_corr,
                &mut h_env.enc_env_data.sbr_invf_mode_vec,
                &mut ed.noise_floor,
                &mut h_env.enc_env_data.add_harmonic_flag,
                &mut h_env.enc_env_data.add_harmonic,
                &mut h_env.sbr_extract_envelope.envelope_compensation,
                &fi,
                &ed.transient_info,
                h_con.freq_band_table(HI),
                h_con.n_sfb[HI],
                xpos_mode,
                h_con.sbr_syntax_flags,
                &tail,
            );
        }

        // Low energy in low band fix
        if h_env.sbr_transient_detector.prev_low_band_energy < h_env.sbr_transient_detector.prev_high_band_energy
            // C: FL2FX_DBL(0.03), with the double argument computed in double (0x3D70A3D)
            && h_env.sbr_transient_detector.prev_high_band_energy > fl2fxconst_dbl(0.03)
            // The fix needs the non-fast transient detector running. It sets
            // prevLowBandEnergy and prevHighBandEnergy.
            && h_con.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY == 0
        {
            h_env.f_level_protect = 1;

            for i in 0..MAX_NUM_NOISE_VALUES {
                h_env.enc_env_data.sbr_invf_mode_vec[i] = INVF_HIGH_LEVEL;
            }
        } else {
            h_env.f_level_protect = 0;
        }

        h_env.enc_env_data.sbr_invf_mode = h_env.enc_env_data.sbr_invf_mode_vec[0];

        h_env.enc_env_data.no_ofnoisebands = h_env.ton_corr.sbr_noise_floor_estimate.no_noise_bands;
    } // ch

    // Save number of scf bands per envelope
    for ch in 0..n_channels {
        let fi = frame_info_of(h_env_chan, &e_data[ch]);
        for i in 0..(e_data[ch].n_envelopes as usize).min(MAX_ENVELOPES) {
            h_env_chan[ch].enc_env_data.no_scf_bands[i] = if fi.freq_res[i] == FREQ_RES_HIGH {
                h_con.n_sfb[FREQ_RES_HIGH as usize]
            } else {
                h_con.n_sfb[FREQ_RES_LOW as usize]
            };
        }
    }

    if h_con.sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0
        && stereo_mode == SBR_SWITCH_LRC
        && h_env_chan[0].enc_env_data.current_amp_res_ff
            != h_env_chan[1].enc_env_data.current_amp_res_ff
    {
        stereo_mode = SBR_LEFT_RIGHT;
    }

    let fi0 = frame_info_of(h_env_chan, &e_data[0]);
    let fi1 = if n_channels > 1 { frame_info_of(h_env_chan, &e_data[1]) } else { fi0 };

    // Extract envelope of current frame.
    {
        let [ed0, ed1] = &mut *e_data;
        let mono = |chan: &EnvChannel,
                    fi: &SbrFrameInfo,
                    sfb: &mut [i8; MAX_NUM_ENVELOPE_VALUES],
                    ram: &[FixpDbl]| {
            let ex = &chan.sbr_extract_envelope;
            calculate_sbr_envelope(
                &ex.y_rows(ram),
                &ex.y_buffer_scale,
                None,
                fi,
                sfb,
                h_con,
                chan,
                y_sz_shift,
            );
        };
        let coupled = |c0: &EnvChannel,
                       c1: &EnvChannel,
                       fi: &SbrFrameInfo,
                       sfb0: &mut [i8; MAX_NUM_ENVELOPE_VALUES],
                       sfb1: &mut [i8; MAX_NUM_ENVELOPE_VALUES],
                       mqe: &mut i32,
                       ram: &[FixpDbl]| {
            let ex0 = &c0.sbr_extract_envelope;
            let ex1 = &c1.sbr_extract_envelope;
            let cpl = CouplingInput {
                y_buffer_right: ex1.y_rows(ram),
                y_buffer_scale_right: ex1.y_buffer_scale,
                sfb_nrg_right: sfb1,
                max_quant_error: mqe,
            };
            calculate_sbr_envelope(
                &ex0.y_rows(ram),
                &ex0.y_buffer_scale,
                Some(cpl),
                fi,
                sfb0,
                h_con,
                c0,
                y_sz_shift,
            );
        };
        match stereo_mode {
            SBR_MONO => mono(&*h_env_chan[0], &fi0, &mut ed0.sfb_nrg, dyn_ram),
            SBR_LEFT_RIGHT => {
                mono(&*h_env_chan[0], &fi0, &mut ed0.sfb_nrg, dyn_ram);
                mono(&*h_env_chan[1], &fi1, &mut ed1.sfb_nrg, dyn_ram);
            }
            SBR_COUPLING => coupled(
                &*h_env_chan[0],
                &*h_env_chan[1],
                &fi0,
                &mut ed0.sfb_nrg,
                &mut ed1.sfb_nrg,
                &mut f_data.max_quant_error,
                dyn_ram,
            ),
            SBR_SWITCH_LRC => {
                mono(&*h_env_chan[0], &fi0, &mut ed0.sfb_nrg, dyn_ram);
                mono(&*h_env_chan[1], &fi1, &mut ed1.sfb_nrg, dyn_ram);
                coupled(
                    &*h_env_chan[0],
                    &*h_env_chan[1],
                    &fi0,
                    &mut ed0.sfb_nrg_coupling,
                    &mut ed1.sfb_nrg_coupling,
                    &mut f_data.max_quant_error,
                    dyn_ram,
                );
            }
            _ => {}
        }
    }

    let header_active = sbr_bitstream_data.header_active;
    let res = f_data.res;

    // Noise floor quantisation and coding.
    match stereo_mode {
        SBR_MONO => {
            let ed = &mut e_data[0];
            sbr_noise_floor_levels_quantisation(&mut ed.noise_level, &ed.noise_floor, 0);
            code_noise(
                &mut *h_env_chan[0],
                &mut ed.noise_level,
                &res,
                0,
                fi0.n_envelopes,
                0,
                header_active,
            );
        }
        SBR_LEFT_RIGHT => {
            let ed = &mut e_data[0];
            sbr_noise_floor_levels_quantisation(&mut ed.noise_level, &ed.noise_floor, 0);
            code_noise(
                &mut *h_env_chan[0],
                &mut ed.noise_level,
                &res,
                0,
                fi0.n_envelopes,
                0,
                header_active,
            );

            let ed = &mut e_data[1];
            sbr_noise_floor_levels_quantisation(&mut ed.noise_level, &ed.noise_floor, 0);
            code_noise(
                &mut *h_env_chan[1],
                &mut ed.noise_level,
                &res,
                0,
                fi1.n_envelopes,
                0,
                header_active,
            );
        }
        SBR_COUPLING => {
            {
                let [ed0, ed1] = &mut *e_data;
                couple_noise_floor(&mut ed0.noise_floor, &mut ed1.noise_floor);
            }

            let ed = &mut e_data[0];
            sbr_noise_floor_levels_quantisation(&mut ed.noise_level, &ed.noise_floor, 0);
            code_noise(
                &mut *h_env_chan[0],
                &mut ed.noise_level,
                &res,
                1,
                fi0.n_envelopes,
                0,
                header_active,
            );

            let ed = &mut e_data[1];
            sbr_noise_floor_levels_quantisation(&mut ed.noise_level, &ed.noise_floor, 1);
            code_noise(
                &mut *h_env_chan[1],
                &mut ed.noise_level,
                &res,
                1,
                fi1.n_envelopes,
                1,
                header_active,
            );
        }
        SBR_SWITCH_LRC => {
            let [ed0, ed1] = &mut *e_data;
            sbr_noise_floor_levels_quantisation(&mut ed0.noise_level, &ed0.noise_floor, 0);
            sbr_noise_floor_levels_quantisation(&mut ed1.noise_level, &ed1.noise_floor, 0);
            couple_noise_floor(&mut ed0.noise_floor, &mut ed1.noise_floor);
            sbr_noise_floor_levels_quantisation(&mut ed0.noise_level_coupling, &ed0.noise_floor, 0);
            sbr_noise_floor_levels_quantisation(&mut ed1.noise_level_coupling, &ed1.noise_floor, 1);
        }
        _ => {}
    }

    // Encode envelope of current frame.
    match stereo_mode {
        SBR_MONO => {
            sbr_header_data.coupling = 0;
            h_env_chan[0].enc_env_data.balance = 0;
            code_env(
                &mut *h_env_chan[0],
                &mut e_data[0].sfb_nrg,
                &fi0,
                sbr_header_data.coupling,
                0,
                header_active,
            );
        }
        SBR_LEFT_RIGHT => {
            sbr_header_data.coupling = 0;

            h_env_chan[0].enc_env_data.balance = 0;
            h_env_chan[1].enc_env_data.balance = 0;

            code_env(
                &mut *h_env_chan[0],
                &mut e_data[0].sfb_nrg,
                &fi0,
                sbr_header_data.coupling,
                0,
                header_active,
            );
            code_env(
                &mut *h_env_chan[1],
                &mut e_data[1].sfb_nrg,
                &fi1,
                sbr_header_data.coupling,
                0,
                header_active,
            );
        }
        SBR_COUPLING => {
            sbr_header_data.coupling = 1;
            h_env_chan[0].enc_env_data.balance = 0;
            h_env_chan[1].enc_env_data.balance = 1;

            code_env(
                &mut *h_env_chan[0],
                &mut e_data[0].sfb_nrg,
                &fi0,
                sbr_header_data.coupling,
                0,
                header_active,
            );
            code_env(
                &mut *h_env_chan[1],
                &mut e_data[1].sfb_nrg,
                &fi1,
                sbr_header_data.coupling,
                1,
                header_active,
            );
        }
        SBR_SWITCH_LRC => {
            let mut sfb_nrg_prev_temp = [[0i8; MAX_FREQ_COEFFS]; MAX_NUM_CHANNELS];
            let mut noise_prev_temp = [[0i8; MAX_NUM_NOISE_COEFFS]; MAX_NUM_CHANNELS];
            let mut up_date_nrg_temp = [0i32; MAX_NUM_CHANNELS];
            let mut up_date_noise_temp = [0i32; MAX_NUM_CHANNELS];
            let mut domain_vec_temp = [[0i32; MAX_ENVELOPES]; MAX_NUM_CHANNELS];
            let mut domain_vec_noise_temp = [[0i32; MAX_ENVELOPES]; MAX_NUM_CHANNELS];

            // Store previous values, in order to be able to "undo" what is being done.
            for ch in 0..n_channels {
                let c = &mut *h_env_chan[ch];
                sfb_nrg_prev_temp[ch] = c.sbr_code_envelope.sfb_nrg_prev;
                noise_prev_temp[ch]
                    .copy_from_slice(&c.sbr_code_noise_floor.sfb_nrg_prev[..MAX_NUM_NOISE_COEFFS]);

                up_date_nrg_temp[ch] = c.sbr_code_envelope.up_date;
                up_date_noise_temp[ch] = c.sbr_code_noise_floor.up_date;

                // forbid time coding in the first envelope in case of a different previous
                // stereomode
                if sbr_header_data.prev_coupling != 0 {
                    c.sbr_code_envelope.up_date = 0;
                    c.sbr_code_noise_floor.up_date = 0;
                }
            } // ch

            // Code ordinary Left/Right stereo
            code_env(&mut *h_env_chan[0], &mut e_data[0].sfb_nrg, &fi0, 0, 0, header_active);
            code_env(&mut *h_env_chan[1], &mut e_data[1].sfb_nrg, &fi1, 0, 0, header_active);

            copy_ienvelope(h_env_chan, e_data, false);

            code_noise(
                &mut *h_env_chan[0],
                &mut e_data[0].noise_level,
                &res,
                0,
                fi0.n_envelopes,
                0,
                header_active,
            );

            h_env_chan[0].enc_env_data.sbr_noise_levels[..MAX_NUM_NOISE_VALUES]
                .copy_from_slice(&e_data[0].noise_level);

            code_noise(
                &mut *h_env_chan[1],
                &mut e_data[1].noise_level,
                &res,
                0,
                fi1.n_envelopes,
                0,
                header_active,
            );

            h_env_chan[1].enc_env_data.sbr_noise_levels[..MAX_NUM_NOISE_VALUES]
                .copy_from_slice(&e_data[1].noise_level);

            sbr_header_data.coupling = 0;
            h_env_chan[0].enc_env_data.balance = 0;
            h_env_chan[1].enc_env_data.balance = 0;

            let payloadbits_lr = {
                let chans: &[&mut EnvChannel] = h_env_chan;
                fdk_sbr_enc_count_sbr_channel_pair_element(
                    sbr_header_data,
                    h_parametric_stereo,
                    sbr_bitstream_data,
                    Some(env_ref(chans, 0)),
                    Some(env_ref(chans, 1)),
                    h_cmon_data,
                    h_con.sbr_syntax_flags,
                )
            };

            // swap saved stored with current values
            for ch in 0..n_channels {
                let c = &mut *h_env_chan[ch];
                // swap sfb energies
                core::mem::swap(&mut c.sbr_code_envelope.sfb_nrg_prev, &mut sfb_nrg_prev_temp[ch]);
                // swap noise energies
                for i in 0..MAX_NUM_NOISE_COEFFS {
                    core::mem::swap(
                        &mut c.sbr_code_noise_floor.sfb_nrg_prev[i],
                        &mut noise_prev_temp[ch][i],
                    );
                }
                // swap update flags
                core::mem::swap(&mut c.sbr_code_envelope.up_date, &mut up_date_nrg_temp[ch]);
                core::mem::swap(&mut c.sbr_code_noise_floor.up_date, &mut up_date_noise_temp[ch]);

                // save domain vecs
                domain_vec_temp[ch] = c.enc_env_data.domain_vec;
                domain_vec_noise_temp[ch] = c.enc_env_data.domain_vec_noise;

                // forbid time coding in the first envelope in case of a different previous
                // stereomode
                if sbr_header_data.prev_coupling == 0 {
                    c.sbr_code_envelope.up_date = 0;
                    c.sbr_code_noise_floor.up_date = 0;
                }
            } // ch

            // Coupling
            code_env(
                &mut *h_env_chan[0],
                &mut e_data[0].sfb_nrg_coupling,
                &fi0,
                1,
                0,
                header_active,
            );
            code_env(
                &mut *h_env_chan[1],
                &mut e_data[1].sfb_nrg_coupling,
                &fi1,
                1,
                1,
                header_active,
            );

            copy_ienvelope(h_env_chan, e_data, true);

            code_noise(
                &mut *h_env_chan[0],
                &mut e_data[0].noise_level_coupling,
                &res,
                1,
                fi0.n_envelopes,
                0,
                header_active,
            );

            h_env_chan[0].enc_env_data.sbr_noise_levels[..MAX_NUM_NOISE_VALUES]
                .copy_from_slice(&e_data[0].noise_level_coupling);

            code_noise(
                &mut *h_env_chan[1],
                &mut e_data[1].noise_level_coupling,
                &res,
                1,
                fi1.n_envelopes,
                1,
                header_active,
            );

            h_env_chan[1].enc_env_data.sbr_noise_levels[..MAX_NUM_NOISE_VALUES]
                .copy_from_slice(&e_data[1].noise_level_coupling);

            sbr_header_data.coupling = 1;

            h_env_chan[0].enc_env_data.balance = 0;
            h_env_chan[1].enc_env_data.balance = 1;

            let temp_flag_left = h_env_chan[0].enc_env_data.add_harmonic_flag;
            let temp_flag_right = h_env_chan[1].enc_env_data.add_harmonic_flag;

            let payloadbits_coupling = {
                let chans: &[&mut EnvChannel] = h_env_chan;
                fdk_sbr_enc_count_sbr_channel_pair_element(
                    sbr_header_data,
                    h_parametric_stereo,
                    sbr_bitstream_data,
                    Some(env_ref(chans, 0)),
                    Some(env_ref(chans, 1)),
                    h_cmon_data,
                    h_con.sbr_syntax_flags,
                )
            };

            h_env_chan[0].enc_env_data.add_harmonic_flag = temp_flag_left;
            h_env_chan[1].enc_env_data.add_harmonic_flag = temp_flag_right;

            if payloadbits_coupling < payloadbits_lr {
                // copy coded coupling envelope and noise data to l/r
                for ed in e_data.iter_mut().take(n_channels) {
                    ed.sfb_nrg = ed.sfb_nrg_coupling;
                    ed.noise_level = ed.noise_level_coupling;
                }

                sbr_header_data.coupling = 1;
                h_env_chan[0].enc_env_data.balance = 0;
                h_env_chan[1].enc_env_data.balance = 1;
            } else {
                // restore saved l/r items
                for ch in 0..n_channels {
                    let c = &mut *h_env_chan[ch];
                    c.sbr_code_envelope.sfb_nrg_prev = sfb_nrg_prev_temp[ch];

                    c.sbr_code_envelope.up_date = up_date_nrg_temp[ch];

                    c.sbr_code_noise_floor.sfb_nrg_prev[..MAX_NUM_NOISE_COEFFS]
                        .copy_from_slice(&noise_prev_temp[ch]);

                    c.enc_env_data.domain_vec = domain_vec_temp[ch];
                    c.enc_env_data.domain_vec_noise = domain_vec_noise_temp[ch];

                    c.sbr_code_noise_floor.up_date = up_date_noise_temp[ch];
                }

                sbr_header_data.coupling = 0;
                h_env_chan[0].enc_env_data.balance = 0;
                h_env_chan[1].enc_env_data.balance = 0;
            }
        }
        _ => {}
    } // switch

    // tell the envelope encoders how long it has been, since we last sent a frame starting
    // with a dF-coded envelope
    if stereo_mode == SBR_MONO {
        if h_env_chan[0].enc_env_data.domain_vec[0] == TIME {
            h_env_chan[0].sbr_code_envelope.d_f_edge_incr_fac += 1;
        } else {
            h_env_chan[0].sbr_code_envelope.d_f_edge_incr_fac = 0;
        }
    } else if h_env_chan[0].enc_env_data.domain_vec[0] == TIME
        || h_env_chan[1].enc_env_data.domain_vec[0] == TIME
    {
        h_env_chan[0].sbr_code_envelope.d_f_edge_incr_fac += 1;
        h_env_chan[1].sbr_code_envelope.d_f_edge_incr_fac += 1;
    } else {
        h_env_chan[0].sbr_code_envelope.d_f_edge_incr_fac = 0;
        h_env_chan[1].sbr_code_envelope.d_f_edge_incr_fac = 0;
    }

    // Send the encoded data to the bitstream
    for ch in 0..n_channels {
        let ed = &e_data[ch];
        let c = &mut *h_env_chan[ch];
        let mut k = 0usize;
        for i in 0..(ed.n_envelopes as usize).min(MAX_ENVELOPES) {
            for j in 0..c.enc_env_data.no_scf_bands[i].clamp(0, MAX_FREQ_COEFFS as i32) as usize {
                c.enc_env_data.ienvelope[i][j] = ed.sfb_nrg.get(k).copied().unwrap_or(0) as i32;
                k += 1;
            }
        }
        c.enc_env_data.sbr_noise_levels[..MAX_NUM_NOISE_VALUES].copy_from_slice(&ed.noise_level);
    } // ch

    // Write bitstream
    {
        let chans: &[&mut EnvChannel] = h_env_chan;
        if n_channels == 2 {
            fdk_sbr_enc_write_env_channel_pair_element(
                sbr_header_data,
                h_parametric_stereo,
                sbr_bitstream_data,
                Some(env_ref(chans, 0)),
                Some(env_ref(chans, 1)),
                h_cmon_data,
                h_con.sbr_syntax_flags,
            );
        } else {
            fdk_sbr_enc_write_env_single_channel_element(
                sbr_header_data,
                h_parametric_stereo,
                sbr_bitstream_data,
                Some(env_ref(chans, 0)),
                h_cmon_data,
                h_con.sbr_syntax_flags,
            );
        }
    }

    // Update buffers.
    for ch in 0..n_channels {
        let ex = &mut h_env_chan[ch].sbr_extract_envelope;
        let y_buffer_length = (ex.no_cols >> ex.y_buffer_sz_shift).max(0) as usize;
        for i in 0..ex.y_buffer_write_offset.max(0) as usize {
            let src = i + y_buffer_length;
            if src >= ENV_BUFFER_ROWS {
                break;
            }
            let mut row = [0 as FixpDbl; 64];
            row.copy_from_slice(ex.y_row(src, dyn_ram));
            ex.y_row_mut(i, dyn_ram).copy_from_slice(&row);
        }
        ex.y_buffer_scale[0] = ex.y_buffer_scale[1];
    }

    sbr_header_data.prev_coupling = sbr_header_data.coupling;
}

/// The envelope values of both channels into `encEnvData.ienvelope` (`coupling`: from
/// `sfb_nrg_coupling`), as the `SBR_SWITCH_LRC` branch of `FDKsbrEnc_extractSbrEnvelope2()`
/// does: the band counts of channel 0 are used for both channels.
fn copy_ienvelope(
    h_env_chan: &mut [&mut EnvChannel],
    e_data: &[SbrEnvTempData; MAX_NUM_CHANNELS],
    coupling: bool,
) {
    let mut c = 0usize;
    for i in 0..(e_data[0].n_envelopes as usize).min(MAX_ENVELOPES) {
        for j in
            0..h_env_chan[0].enc_env_data.no_scf_bands[i].clamp(0, MAX_FREQ_COEFFS as i32) as usize
        {
            let (v0, v1) = if coupling {
                (e_data[0].sfb_nrg_coupling[c], e_data[1].sfb_nrg_coupling[c])
            } else {
                (e_data[0].sfb_nrg[c], e_data[1].sfb_nrg[c])
            };
            h_env_chan[0].enc_env_data.ienvelope[i][j] = v0 as i32;
            h_env_chan[1].enc_env_data.ienvelope[i][j] = v1 as i32;
            c += 1;
        }
    }
}

/// `FDKsbrEnc_CreateExtractSbrEnvelope(hSbrCut, channel, chInEl, dynamic_RAM)`: clears the
/// extractor and allocates `p_YBuffer`. Returns 0.
pub fn fdk_sbr_enc_create_extract_sbr_envelope(
    h_sbr_cut: &mut SbrExtractEnvelope,
    _channel: i32,
    ch_in_el: i32,
) -> i32 {
    *h_sbr_cut = SbrExtractEnvelope::default();

    h_sbr_cut.p_y_buffer = vec![0; ENV_Y_BUFFER_STATIC_DBL];
    h_sbr_cut.y_buffer_dyn = get_ram_sbr_env_y_buffer(ch_in_el.max(0) as usize);

    h_sbr_cut.r_buffer = get_ram_sbr_env_r_buffer(0);
    h_sbr_cut.i_buffer = get_ram_sbr_env_i_buffer(0);

    0
}

/// `FDKsbrEnc_InitExtractSbrEnvelope(hSbrCut, no_cols, no_rows, start_index, time_slots,
/// time_step, tran_off, statesInitFlag, chInEl, dynamic_RAM, sbrSyntaxFlags)`. Returns 0.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_init_extract_sbr_envelope(
    h_sbr_cut: &mut SbrExtractEnvelope,
    no_cols: i32,
    no_rows: i32,
    start_index: i32,
    time_slots: i32,
    time_step: i32,
    tran_off: i32,
    states_init_flag: u32,
    ch_in_el: i32,
    dyn_ram: &mut [FixpDbl],
    sbr_syntax_flags: u32,
) -> i32 {
    let h = h_sbr_cut;
    if sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        let off = TRANSIENT_OFFSET_LD;
        h.y_buffer_write_offset = (no_cols >> 1) + off * time_step;
    } else {
        h.y_buffer_write_offset = tran_off * time_step;
    }
    h.r_buffer_read_offset = 0;

    let mut y_buffer_length = h.y_buffer_write_offset + no_cols;
    let r_buffer_length = no_cols;

    h.pre_transient_info = [0; 2];

    h.no_cols = no_cols;
    h.no_rows = no_rows;
    h.start_index = start_index;

    h.time_slots = time_slots;
    h.time_step = time_step;

    fdk_assert!(no_rows <= 64);

    // Use half the Energy values if time step is 2 or greater
    h.y_buffer_sz_shift = if time_step >= 2 { 1 } else { 0 };

    y_buffer_length >>= h.y_buffer_sz_shift;
    h.y_buffer_write_offset >>= h.y_buffer_sz_shift;

    fdk_assert!(y_buffer_length <= 32);

    h.y_buffer_dyn = get_ram_sbr_env_y_buffer(ch_in_el.max(0) as usize);

    if states_init_flag != 0 {
        for i in 0..y_buffer_length.clamp(0, ENV_BUFFER_ROWS as i32) as usize {
            h.y_row_mut(i, dyn_ram).fill(0);
        }
    }

    for i in 0..r_buffer_length.clamp(0, ENV_BUFFER_ROWS as i32) as usize {
        dyn_ram[h.r_buffer + 64 * i..h.r_buffer + 64 * i + 64].fill(0);
        dyn_ram[h.i_buffer + 64 * i..h.i_buffer + 64 * i + 64].fill(0);
    }

    h.envelope_compensation = [0; MAX_FREQ_COEFFS];

    if states_init_flag != 0 {
        h.y_buffer_scale = [FRACT_BITS - 1; 2];
    }

    0
}

/// `FDKsbrEnc_deleteExtractSbrEnvelope(hSbrCut)`: frees `p_YBuffer`.
pub fn fdk_sbr_enc_delete_extract_sbr_envelope(h_sbr_cut: Option<&mut SbrExtractEnvelope>) {
    if let Some(h) = h_sbr_cut {
        h.p_y_buffer = Vec::new();
    }
}

/// `FDKsbrEnc_GetEnvEstDelay(hSbr)`
pub fn fdk_sbr_enc_get_env_est_delay(h_sbr: &SbrExtractEnvelope) -> i32 {
    // mult 2 because nrg's are grouped half; in reference hold half spec and calc nrg's on
    // overlapped spec
    h_sbr.no_rows * (h_sbr.y_buffer_write_offset * 2 - h_sbr.r_buffer_read_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The straight energy loops of `get_energy_from_cplx_qmf_data` (rows of 64 values).
    fn energies_straight(
        re: &mut [FixpDbl],
        im: &mut [FixpDbl],
        nb: usize,
        ncols: usize,
        sh: u32,
        full: bool,
        nrg: &mut [FixpDbl],
    ) -> FixpDbl {
        let mut max_val = 0;
        let mut n = 0;
        let mut k = 0;
        while k < ncols {
            for j in 0..nb {
                let tr0 = re[64 * k + j].wrapping_shl(sh);
                let ti0 = im[64 * k + j].wrapping_shl(sh);
                let mut energy = f_pow2_add_div2(f_pow2_div2(tr0), ti0);
                re[64 * k + j] = tr0;
                im[64 * k + j] = ti0;
                if !full {
                    energy >>= 1;
                    let tr1 = re[64 * (k + 1) + j].wrapping_shl(sh);
                    let ti1 = im[64 * (k + 1) + j].wrapping_shl(sh);
                    energy = energy.wrapping_add(f_pow2_add_div2(f_pow2_div2(tr1), ti1) >> 1);
                    re[64 * (k + 1) + j] = tr1;
                    im[64 * (k + 1) + j] = ti1;
                }
                nrg[n] = energy;
                n += 1;
                max_val = fix_max(max_val, energy);
            }
            k += if full { 1 } else { 2 };
        }
        max_val
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 32) as u32
        }
        fn val(&mut self, kind: u32) -> FixpDbl {
            match kind {
                0 => self.next() as i32,
                1 => [i32::MIN, i32::MAX, 0, -1][(self.next() % 4) as usize],
                _ => (self.next() as i32) >> (self.next() % 32),
            }
        }
    }

    #[test]
    fn qmf_energies_vs_straight() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for it in 0..20_000 {
            let full = it % 2 == 0;
            let ncols = (rng.next() % 33) as usize;
            let nb = (rng.next() % 65) as usize;
            let sh = match rng.next() % 4 {
                0 => rng.next() % 40,
                _ => rng.next() % 8,
            };
            let kind = rng.next() % 4;
            let rows = ncols.div_ceil(2) * 2;
            let re: Vec<FixpDbl> = (0..64 * rows).map(|_| rng.val(kind)).collect();
            let im: Vec<FixpDbl> = (0..64 * rows).map(|_| rng.val(kind)).collect();
            let (mut rs, mut is, mut ns) = (re.clone(), im.clone(), [7; 32 * 64]);
            let (mut rf, mut if_, mut nf) = (re, im, [7; 32 * 64]);
            let ms = energies_straight(&mut rs, &mut is, nb, ncols, sh, full, &mut ns);
            let mf = qmf_energies(&mut rf, &mut if_, nb, ncols, sh, full, &mut nf);
            assert_eq!((ms, &rs, &is), (mf, &rf, &if_), "{it}");
            assert_eq!(ns, nf, "{it}");
        }
    }
}
