// Differential tests of fdk_aac_rs_aacdec::block against the C++ reference.
//
// The standalone functions on random and edge-case inputs; the functions on channel state with
// the block test context (block_ctx.rs): random states, random bitstreams, sequences of frames for
// the frequency to time transforms, including CBlock_ReadSpectralData() with AC_ER_HCR and the
// LPD branch of CBlock_FrequencyToTime() (the FAC transition and the bass post-filter).

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use crate::block_ctx::*;
use crate::usacdec_lpd_util::{rand_dbl_vec, rand_lsf};
use fdk_aac_rs_aacdec::aacdec_hcr::chcr_read;
use fdk_aac_rs_aacdec::aac_rom::AAC_CODE_BOOK_DESCRIPTION_TABLE;
use fdk_aac_rs_aacdec::block::*;
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_cref::aacdec_block as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream, BS_READER};
use fdk_aac_rs_fdk::common_fix::{MAXVAL_DBL, MINVAL_DBL};

fn bs_over(buf: &mut [u8]) -> FdkBitStream {
    let mut bs = FdkBitStream::default();
    // SAFETY: the buffer outlives the bitstream in every test.
    fdk_init_bit_stream(&mut bs, &BitMem::from_slice(buf), buf.len() as u32, buf.len() as u32 * 8, BS_READER);
    bs
}

#[test]
fn get_escape() {
    let mut rng = Rng::new(0xb10c_0001);
    for _ in 0..20000 {
        let mut buf = random_bytes(&mut rng, 16);
        let q = match rng.next_u32() % 4 {
            0 => 16,
            1 => -16,
            2 => rng.range(-20, 20),
            _ => rng.i32(),
        };
        let mut cb = buf.clone();
        let mut cleft = 0;
        let ce = unsafe { c::cref_block_get_escape(cb.as_mut_ptr(), 16, q, &mut cleft) };
        let mut bs = bs_over(&mut buf);
        let re = cblock_get_escape(&mut bs, q);
        assert_eq!(re, ce, "CBlock_GetEscape({q})");
        assert_eq!(fdk_get_valid_bits(&mut bs) as i32, cleft, "CBlock_GetEscape({q}): bits");
    }
}

#[test]
fn decode_huffman_word() {
    let mut rng = Rng::new(0xb10c_0002);
    for _ in 0..40000 {
        let mut buf = random_bytes(&mut rng, 16);
        let cbn = 1 + rng.next_u32() % 12;
        let which = (rng.next_u32() % 2) as i32;
        let mut cb = buf.clone();
        let mut cleft = 0;
        let ce = unsafe { c::cref_block_decode_huffman_word(which, cb.as_mut_ptr(), 16, cbn as i32, &mut cleft) };
        let mut bs = bs_over(&mut buf);
        let hcb = &AAC_CODE_BOOK_DESCRIPTION_TABLE[cbn as usize];
        let re = if which == 0 {
            cblock_decode_huffman_word(&mut bs, hcb)
        } else {
            cblock_decode_huffman_word_cb(&mut bs, hcb.code_book)
        };
        assert_eq!(re, ce, "huffman word {which} cb {cbn}");
        assert_eq!(fdk_get_valid_bits(&mut bs) as i32, cleft, "huffman word {which} cb {cbn}: bits");
    }
}

#[test]
fn evaluate_power43_and_scale() {
    let mut rng = Rng::new(0xb10c_0003);
    let mut values: Vec<i32> = (1..=8191).collect();
    for _ in 0..4 {
        values.extend((0..2000).map(|_| rng.range(1, 8191)));
    }
    for &v in &values {
        for lsb in 0..4u32 {
            let mut cv = v;
            let ce = unsafe { c::cref_block_evaluate_power43(&mut cv, lsb) };
            let mut rv = v;
            let re = evaluate_power43(&mut rv, lsb);
            assert_eq!((re, rv), (ce, cv), "EvaluatePower43({v}, {lsb})");
            let cs = unsafe { c::cref_block_get_scale_from_value(v, lsb) };
            assert_eq!(get_scale_from_value(v, lsb), cs, "GetScaleFromValue({v}, {lsb})");
        }
    }
    for lsb in 0..4u32 {
        assert_eq!(get_scale_from_value(0, lsb), unsafe { c::cref_block_get_scale_from_value(0, lsb) });
    }
}

#[test]
fn inverse_quantize_band_and_maxabs() {
    let mut rng = Rng::new(0xb10c_0004);
    for _ in 0..20000 {
        let n = [4, 8, 12, 16, 32, 48, 64, 96, 4][(rng.next_u32() % 9) as usize];
        let spec: Vec<i32> = (0..n).map(|_| random_quant(&mut rng).clamp(-8191, 8191)).collect();
        let lsb = rng.next_u32() % 4;
        let scale = rng.range(-40, 40);
        let mut cs = spec.clone();
        unsafe { c::cref_block_inverse_quantize_band(cs.as_mut_ptr(), lsb, n as i32, scale) };
        let mut rs = spec.clone();
        inverse_quantize_band(&mut rs, lsb, n as i32, scale);
        assert_eq!(rs, cs, "InverseQuantizeBand(lsb {lsb}, scale {scale})");
        let all: Vec<i32> = (0..n).map(|_| random_spec(&mut rng)).collect();
        assert_eq!(maxabs_d(&all, n as i32), unsafe { c::cref_block_maxabs_d(all.as_ptr(), n as i32) });
    }
}

#[test]
fn get_gain_() {
    let mut rng = Rng::new(0xb10c_0005);
    for _ in 0..20000 {
        let n = [64, 128, 32, 48, 1, 3][(rng.next_u32() % 6) as usize];
        let sh = rng.next_u32() % 32;
        let x: Vec<i32> = (0..n).map(|_| rng.i32() >> sh).collect();
        let y: Vec<i32> = (0..n).map(|_| if rng.next_u32() % 8 == 0 { 0 } else { rng.i32() >> sh }).collect();
        let ce = unsafe { c::cref_block_get_gain(x.as_ptr(), y.as_ptr(), n as i32) };
        assert_eq!(get_gain(&x, &y, n as i32), ce, "get_gain(n {n}, sh {sh})");
    }
}

#[test]
fn get_window2_nr_() {
    for length in [0, 120, 128, 480, 512, 960, 1024, 7, -4, i32::MAX / 3] {
        for shape in [0, 1, 2, 3, -1] {
            assert_eq!(get_window2_nr(length, shape), unsafe { c::cref_block_get_window2_nr(length, shape) });
        }
    }
}

/// The ARM scaling loop on both variants, and the generic loop of the x86 build through
/// `CBlock_ScaleSpectralData()` (see `scale_spectral_data`): for every table and every number of
/// bands within the table both give the same result.
#[test]
fn scale_spectral_data_func1() {
    let mut rng = Rng::new(0xb10c_0006);
    let mut p = Pair::new();
    for _ in 0..3000 {
        let spf = random_config(&mut p, &mut rng, true);
        let _ = spf;
        let sri = p.r.sri;
        let short = sri.number_of_scale_factor_bands_short > 0 && rng.next_u32() % 2 == 0;
        let (offsets, total) = if short {
            (sri.scale_factor_bands_short, sri.number_of_scale_factor_bands_short)
        } else {
            (sri.scale_factor_bands_long, sri.number_of_scale_factor_bands_long)
        };
        let max_sfbs = rng.range(0, total as i32);
        let window = if short { rng.range(0, 7) } else { 0 };
        let spec: Vec<i32> = (0..1024).map(|_| random_spec(&mut rng)).collect();
        let sfb_scale: Vec<i16> = (0..128).map(|_| rng.range(-40, 40) as i16).collect();
        // SpecScale_window as CBlock_ScaleSpectralData() computes it: at least the maximum of
        // the scale factor band scales (and 0), plus the TNS headroom
        let mut ssw = 0;
        for b in 0..max_sfbs {
            ssw = ssw.max(sfb_scale[(window * 16 + b) as usize] as i32);
        }
        ssw += [0, 0, rng.range(0, 20)][(rng.next_u32() % 3) as usize];
        let mut cs = spec.clone();
        unsafe {
            c::cref_block_scale_func1(cs.as_mut_ptr(), max_sfbs, offsets.as_ptr(), ssw, sfb_scale.as_ptr(), window)
        };
        let mut rs = spec.clone();
        cblock_scale_spectral_data_func1(&mut rs, max_sfbs, offsets, ssw, &sfb_scale, window);
        assert_eq!(rs, cs, "CBlock_ScaleSpectralData_func1(max {max_sfbs}, ssw {ssw})");
    }
}

fn check(p: &mut Pair, what: &str) {
    p.check(what);
    p.check_bs(what);
}

#[test]
fn read_section_data() {
    let mut rng = Rng::new(0xb10c_0010);
    let mut p = Pair::new();
    for it in 0..4000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        p.r.ci[ch].p_dyn_data.raw_data_info.common_window = (rng.next_u32() % 2) as u8;
        let flags = [0, AC_ER_VCB11, AC_ER_HCR, AC_ER_VCB11 | AC_ER_HCR, AC_ER | AC_LD][(rng.next_u32() % 5) as usize];
        let mut data = random_bytes(&mut rng, 64);
        // Inputs that reach a reference site without a defined result are avoided (see
        // cblock_read_section_data).
        data[63] &= 0x7F;
        data[62] |= 0x01;
        p.set_bs(&data, 64, 64 * 8);
        p.sync();
        let ce = unsafe { c::cref_block_read_section_data(p.c.0, ch as i32, flags) };
        let sri = p.r.sri;
        let re = cblock_read_section_data(&mut p.r.bs, &mut p.r.ci[ch], &sri, flags);
        assert_eq!(re, ce, "CBlock_ReadSectionData {it}");
        check(&mut p, &format!("CBlock_ReadSectionData {it} flags {flags:#x}"));
    }
}

/// Random section data as CBlock_ReadSectionData() can give it.
pub fn random_codebooks(rng: &mut Rng, ci: &mut CAacDecoderChannelInfo, vcb11: bool) {
    let set: &[u8] = if vcb11 {
        &[0, 1, 2, 5, 8, 11, 13, 14, 15, 16, 17, 20, 31, 0, 0]
    } else {
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 14, 15, 0, 0, 11, 11]
    };
    for g in 0..8 {
        for b in 0..16 {
            ci.p_dyn_data.a_code_book[g * 16 + b] = set[(rng.next_u32() % set.len() as u32) as usize];
        }
    }
    if ci.ics_info.window_sequence != 2 {
        for b in 0..64 {
            ci.p_dyn_data.a_code_book[b] = set[(rng.next_u32() % set.len() as u32) as usize];
        }
    }
}

#[test]
fn read_scale_factor_data() {
    let mut rng = Rng::new(0xb10c_0011);
    let mut p = Pair::new();
    for it in 0..4000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let vcb = rng.next_u32() % 2 == 0;
        random_codebooks(&mut rng, &mut p.r.ci[ch], vcb);
        p.r.ci[ch].data.aac.pns_data.pns_active = (rng.next_u32() % 2) as u8;
        let flags = [0, AC_USAC, AC_MPEGD_RES, AC_ER_VCB11, 0][(rng.next_u32() % 5) as usize];
        let data = random_bytes(&mut rng, 256);
        p.set_bs(&data, 256, 256 * 8);
        p.sync();
        let ce = unsafe { c::cref_block_read_scale_factor_data(p.c.0, ch as i32, flags) };
        let re = cblock_read_scale_factor_data(&mut p.r.ci[ch], &mut p.r.bs, flags);
        assert_eq!(re, ce, "CBlock_ReadScaleFactorData {it}");
        check(&mut p, &format!("CBlock_ReadScaleFactorData {it} flags {flags:#x}"));
    }
}

/// Valid pulse data for the long block of the channel (or none).
pub fn random_pulse(rng: &mut Rng, ci: &mut CAacDecoderChannelInfo, sri: &SamplingRateInfo, spf: u32) {
    let pd = &mut ci.p_dyn_data.specific_to.aac.pulse_data;
    pd.pulse_data_present = 0;
    if ci.ics_info.window_sequence != 2 && ci.ics_info.max_sf_bands > 0 && rng.next_u32() % 3 == 0 {
        loop {
            pd.number_pulse = (rng.next_u32() % 4) as u8;
            pd.pulse_start_band = (rng.next_u32() % ci.ics_info.max_sf_bands as u32) as u8;
            let mut k = sri.scale_factor_bands_long[pd.pulse_start_band as usize] as u32;
            for i in 0..4 {
                pd.pulse_offset[i] = (rng.next_u32() % 32) as u8;
                pd.pulse_amp[i] = (rng.next_u32() % 16) as u8;
                if i <= pd.number_pulse as usize {
                    k += pd.pulse_offset[i] as u32;
                }
            }
            if k < spf {
                pd.pulse_data_present = 1;
                break;
            }
        }
    }
}

#[test]
fn read_spectral_data() {
    let mut rng = Rng::new(0xb10c_0012);
    let mut p = Pair::new();
    for it in 0..4000 {
        let spf = random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let vcb11 = rng.next_u32() % 2 == 0;
        random_codebooks(&mut rng, &mut p.r.ci[ch], vcb11);
        let sri = p.r.sri;
        random_pulse(&mut rng, &mut p.r.ci[ch], &sri, spf);
        let flags = [0, AC_ER_VCB11, AC_ELD, AC_SCALABLE, 0][(rng.next_u32() % 5) as usize];
        let data = random_bytes(&mut rng, 4096);
        p.set_bs(&data, 4096, 4096 * 8);
        p.sync();
        let ce = unsafe { c::cref_block_read_spectral_data(p.c.0, ch as i32, flags) };
        let (spec, com) = (&mut p.r.spec, &mut p.r.com);
        let re = cblock_read_spectral_data(&mut p.r.bs, &mut p.r.ci[ch], &sri, flags, &mut spec[ch * 1024..], com);
        assert_eq!(re, ce, "CBlock_ReadSpectralData {it}");
        check(&mut p, &format!("CBlock_ReadSpectralData {it} flags {flags:#x}"));
    }
}

/// CBlock_ReadSpectralData() with AC_ER_HCR: the section data of CBlock_ReadSectionData() and
/// the side info of CHcr_Read() (both run on the Rust side, then copied to the C++ side) on random
/// bits, then the HCR decoding of random spectral data on both sides. The HCR state
/// (`pComData->overlay.aac.erHcrInfo`) is not part of the image; it starts zeroed on both sides
/// and is only changed by the compared calls.
#[test]
fn read_spectral_data_hcr() {
    let mut rng = Rng::new(0xb10c_001a);
    let mut p = Pair::new();
    let (mut compared, mut section_errors, mut hcr_errors) = (0, 0, 0);
    for it in 0..6000 {
        let spf = random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let ld = spf == 512 || spf == 480;
        let mut flags = AC_ER | AC_ER_HCR;
        if rng.next_u32() % 2 == 0 {
            flags |= AC_ER_VCB11;
        }
        if ld {
            flags |= if rng.next_u32() % 2 == 0 { AC_ELD } else { AC_LD };
        }
        let sri = p.r.sri;
        random_pulse(&mut rng, &mut p.r.ci[ch], &sri, spf);
        // section data (sets aCodeBooks4Hcr, aNumLineInSec4Hcr and numberSection for HCR)
        let mut side = random_bytes(&mut rng, 256);
        side[255] &= 0x7F;
        side[254] |= 0x01;
        let mut bs = bs_over(&mut side);
        if cblock_read_section_data(&mut bs, &mut p.r.ci[ch], &sri, flags) != AAC_DEC_OK {
            section_errors += 1;
            continue;
        }
        // lenOfReorderedSpectralData and lenOfLongestCodeword
        let id = if rng.next_u32() % 2 == 0 { fdk_aac_rs_fdk::audio::ID_SCE } else { fdk_aac_rs_fdk::audio::ID_CPE };
        let mut si = random_bytes(&mut rng, 8);
        let mut bs = bs_over(&mut si);
        chcr_read(&mut bs, &mut p.r.ci[ch], id);
        if rng.next_u32() % 2 == 0 {
            // shorter reordered data than the 14-bit field usually gives
            let aac = &mut p.r.ci[ch].p_dyn_data.specific_to.aac;
            aac.len_of_reordered_spectral_data = rng.range(0, 3000) as i16;
        }
        let data = random_bytes(&mut rng, 4096);
        p.set_bs(&data, 4096, 4096 * 8);
        p.sync();
        let ce = unsafe { c::cref_block_read_spectral_data(p.c.0, ch as i32, flags) };
        let (spec, com) = (&mut p.r.spec, &mut p.r.com);
        let re = cblock_read_spectral_data(&mut p.r.bs, &mut p.r.ci[ch], &sri, flags, &mut spec[ch * 1024..], com);
        assert_eq!(re, ce, "CBlock_ReadSpectralData (HCR) {it}");
        if re != AAC_DEC_OK {
            hcr_errors += 1;
        }
        compared += 1;
        check(&mut p, &format!("CBlock_ReadSpectralData (HCR) {it} flags {flags:#x}"));
    }
    eprintln!("CBlock_ReadSpectralData (HCR): {compared} compared ({hcr_errors} with an error), {section_errors} section data errors");
    assert!(compared > 2000, "compared {compared}");
}

#[test]
fn inverse_quantize_spectral_data() {
    let mut rng = Rng::new(0xb10c_0013);
    let mut p = Pair::new();
    for it in 0..4000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        let vcb = rng.next_u32() % 2 == 0;
        random_codebooks(&mut rng, &mut p.r.ci[ch], vcb);
        let big = rng.next_u32() % 8 == 0;
        for i in 0..1024 {
            p.r.spec[ch * 1024 + i] = if big { random_spec(&mut rng) } else { random_quant(&mut rng) };
        }
        for i in 0..128 {
            p.r.ci[ch].p_dyn_data.a_scale_factor[i] = rng.range(-100, 155) as i16;
        }
        let active = (rng.next_u32() % 2) as u8;
        p.sync();
        let ce = unsafe { c::cref_block_inverse_quantize(p.c.0, ch as i32, active as i32) };
        let sri = p.r.sri;
        let re = cblock_inverse_quantize_spectral_data(&mut p.r.ci[ch], &sri, active, &mut p.r.spec[ch * 1024..]);
        assert_eq!(re, ce, "CBlock_InverseQuantizeSpectralData {it}");
        check(&mut p, &format!("CBlock_InverseQuantizeSpectralData {it}"));
    }
}

/// Valid TNS data for the channel (as CTns_Read() gives it).
pub fn random_tns(rng: &mut Rng, ci: &mut CAacDecoderChannelInfo) {
    let t = &mut ci.p_dyn_data.tns_data;
    let ics = ci.ics_info;
    let long = ics.window_sequence != 2;
    t.active = (rng.next_u32() % 2) as u8;
    t.gain_ld = [0, 4][(rng.next_u32() % 2) as usize];
    let nwin = if long { 1 } else { 8 };
    for w in 0..8 {
        t.number_of_filters[w] = 0;
    }
    for w in 0..nwin {
        let nf = rng.next_u32() % (if long { 4 } else { 2 });
        t.number_of_filters[w] = nf as u8;
        let mut next = ics.total_sf_bands;
        for f in 0..nf as usize {
            let fl = &mut t.filter[w][f];
            let len = (rng.next_u32() % (next as u32 + 1)) as u8;
            fl.start_band = next - len;
            fl.stop_band = next;
            next = fl.start_band;
            fl.order = (rng.next_u32() % 13) as u8;
            fl.direction = if rng.next_u32() % 2 == 0 { 1 } else { -1 };
            fl.resolution = (3 + rng.next_u32() % 2) as i8;
            for i in 0..20 {
                fl.coeff[i] = if fl.resolution == 3 { rng.range(-4, 3) } else { rng.range(-8, 7) } as i8;
            }
        }
    }
}

#[test]
fn scale_spectral_data() {
    let mut rng = Rng::new(0xb10c_0014);
    let mut p = Pair::new();
    for it in 0..4000 {
        random_state(&mut p, &mut rng, false, true, true);
        let ch = (rng.next_u32() % 2) as usize;
        random_tns(&mut rng, &mut p.r.ci[ch]);
        for i in 0..128 {
            p.r.ci[ch].p_dyn_data.a_sfb_scale[i] = rng.range(-30, 40) as i16;
        }
        for i in 0..1024 {
            p.r.spec[ch * 1024 + i] = random_spec(&mut rng);
        }
        let max_sfbs = rng.range(0, p.r.ci[ch].ics_info.max_sf_bands as i32) as u8;
        p.sync();
        unsafe { c::cref_block_scale_spectral_data(p.c.0, ch as i32, max_sfbs as i32) };
        let sri = p.r.sri;
        cblock_scale_spectral_data(&mut p.r.ci[ch], max_sfbs, &sri, &mut p.r.spec[ch * 1024..]);
        p.check(&format!("CBlock_ScaleSpectralData {it}"));
    }
}

#[test]
fn apply_noise() {
    let mut rng = Rng::new(0xb10c_0015);
    let mut p = Pair::new();
    for it in 0..3000 {
        // USAC frame lengths (1024, 768)
        let spf = loop {
            let spf = random_state(&mut p, &mut rng, true, false, true);
            if spf == 1024 || spf == 768 {
                break spf;
            }
        };
        let _ = spf;
        let ch = (rng.next_u32() % 2) as usize;
        for i in 0..128 {
            p.r.ci[ch].p_dyn_data.band_is_noise[i] = (rng.next_u32() % 2) as u8;
            p.r.ci[ch].p_dyn_data.a_scale_factor[i] = rng.range(-100, 155) as i16;
            p.r.ci[ch].p_dyn_data.a_sfb_scale[i] = rng.range(-30, 40) as i16;
        }
        for i in 0..1024 {
            p.r.spec[ch * 1024 + i] = if rng.next_u32() % 2 == 0 { 0 } else { random_spec(&mut rng) };
        }
        p.sync();
        unsafe { c::cref_block_apply_noise(p.c.0, ch as i32) };
        let sri = p.r.sri;
        let (ci, sci) = (&mut p.r.ci[ch], &mut p.r.sci[ch]);
        cblock_apply_noise(ci, &sri, &mut sci.nf_random_seed, &mut p.r.spec[ch * 1024..]);
        p.check(&format!("CBlock_ApplyNoise {it}"));
    }
}

#[test]
fn read_ac_spectral_data() {
    let mut rng = Rng::new(0xb10c_0016);
    let mut p = Pair::new();
    for it in 0..3000 {
        let spf = loop {
            let spf = random_state(&mut p, &mut rng, true, false, true);
            if spf == 1024 || spf == 768 {
                break spf;
            }
        };
        let ch = (rng.next_u32() % 2) as usize;
        if rng.next_u32() % 4 != 0 {
            p.r.sci[ch].h_ar_co = Some(Box::default());
            // a previous line count of this frame length (other counts have no defined result
            // in the reference)
            p.r.arco[ch].m_number_lines_prev = [0, spf as i16, spf as i16 / 8][(rng.next_u32() % 3) as usize];
            for x in p.r.arco[ch].c_prev.iter_mut() {
                *x = (rng.next_u32() % 16) as u8;
            }
            // load the state into the box
            let a = p.r.arco[ch].clone();
            p.r.sci[ch].h_ar_co = Some(a);
        } else {
            p.r.sci[ch].h_ar_co = None;
            p.sync();
            continue; // no defined result in the reference
        }
        let flags = AC_USAC | if rng.next_u32() % 2 == 0 { AC_INDEP } else { 0 };
        let data = random_bytes(&mut rng, 8192);
        p.set_bs(&data, 8192, 8192 * 8);
        p.sync();
        let ce = unsafe { c::cref_block_read_ac_spectral_data(p.c.0, ch as i32, spf, flags) };
        let sri = p.r.sri;
        let (ci, sci) = (&mut p.r.ci[ch], &mut p.r.sci[ch]);
        let re = cblock_read_ac_spectral_data(&mut p.r.bs, ci, sci, &sri, spf, flags, &mut p.r.spec[ch * 1024..]);
        assert_eq!(re, ce, "CBlock_ReadAcSpectralData {it}");
        check(&mut p, &format!("CBlock_ReadAcSpectralData {it}"));
    }
}

#[test]
fn apply_tools_() {
    let mut rng = Rng::new(0xb10c_0017);
    let mut p = Pair::new();
    for it in 0..3000 {
        let usac = rng.next_u32() % 4 == 0;
        random_state(&mut p, &mut rng, usac, true, true);
        let channel = (rng.next_u32() % 2) as i32;
        // the second channel of an element shares the ICS of the first one in most cases
        if rng.next_u32() % 2 == 0 {
            p.r.ci[1].ics_info = p.r.ci[0].ics_info;
        }
        for ch in 0..2 {
            random_tns(&mut rng, &mut p.r.ci[ch]);
            if !usac {
                let pd = &mut p.r.ci[ch].data.aac.pns_data;
                pd.pns_active = (rng.next_u32() % 2) as u8;
                for x in pd.pns_used.iter_mut() {
                    *x = (rng.next_u32() % 4 == 0) as u8;
                }
            }
            for i in 0..128 {
                p.r.ci[ch].p_dyn_data.a_scale_factor[i] = rng.range(-100, 155) as i16;
            }
            for i in 0..8 {
                p.r.ci[ch].spec_scale[i] = rng.range(-5, 30) as i16;
            }
            for i in 0..1024 {
                p.r.spec[ch * 1024 + i] = random_spec(&mut rng) >> 6;
            }
        }
        for x in p.r.com.pns_inter_channel_data.correlated.iter_mut() {
            *x = (rng.next_u32() % 4) as u8;
        }
        let flags = if usac { AC_USAC } else { [0, AC_ER, AC_LD][(rng.next_u32() % 3) as usize] };
        let el_flags = if rng.next_u32() % 4 == 0 { AC_EL_ENHANCED_NOISE } else { 0 };
        p.sync();
        unsafe { c::cref_block_apply_tools(p.c.0, flags, el_flags, channel, 1, usac as i32) };
        let sri = p.r.sri;
        let RCtx { ci, sci, com, spec, .. } = &mut p.r;
        let [c0, c1] = ci;
        let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
        apply_tools(&mut chans, &sri, flags, el_flags, channel, 1, spec, com, &mut sci[0].pns_current_seed);
        p.check(&format!("ApplyTools {it}"));
    }
}

/// Sequences of frames through CBlock_FrequencyToTime() (non-LPD) per frame length, window
/// sequence and shape.
#[test]
fn frequency_to_time() {
    let mut rng = Rng::new(0xb10c_0018);
    let mut p = Pair::new();
    for seq in 0..200 {
        let spf = random_state(&mut p, &mut rng, false, true, true);
        let ld = spf == 512 || spf == 480;
        for ch in 0..2 {
            p.r.sci[ch].last_core_mode = FD_LONG;
            p.r.ci[ch].curr_aliasing_symmetry = (rng.next_u32() % 8 == 0) as i32;
        }
        let headroom = [0, 1, 2, 3][(rng.next_u32() % 4) as usize];
        p.sync();
        for frame in 0..12 {
            let ch = (rng.next_u32() % 2) as usize;
            // next window sequence: random (also invalid transitions), mostly valid ones
            let sri = p.r.sri;
            {
                let ics = &mut p.r.ci[ch].ics_info;
                random_ics(&mut rng, ics, &sri, !ld);
                if ld {
                    ics.window_sequence = 0;
                    ics.window_shape = [0, 2][(rng.next_u32() % 2) as usize];
                }
            }
            for i in 0..1024 {
                p.r.spec[ch * 1024 + i] = random_spec(&mut rng) >> (rng.next_u32() % 8);
            }
            for i in 0..8 {
                p.r.ci[ch].spec_scale[i] = rng.range(-2, 20) as i16;
            }
            let frame_ok = (rng.next_u32() % 4 != 0) as i32;
            p.sync();
            unsafe { c::cref_block_frequency_to_time(p.c.0, ch as i32, spf as i32, frame_ok, headroom, 0, ch as i32) };
            let RCtx { ci, sci, spec, wb1, out, .. } = &mut p.r;
            cblock_frequency_to_time(
                &mut sci[ch],
                &mut ci[ch],
                &mut out[ch],
                spf as i16,
                frame_ok,
                &mut wb1.buf,
                headroom,
                0,
                ch as i32,
                &mut spec[ch * 1024..],
            );
            p.check(&format!("CBlock_FrequencyToTime seq {seq} frame {frame} spf {spf}"));
        }
    }
}

/// CBlock_FrequencyToTime() of an FD frame after an LPD frame (`last_core_mode == LPD`): the FAC
/// transition after ACELP (`last_lpd_mode` 0 or 4, `CLpd_FAC_Acelp2Mdct()`) or the IMDCT after
/// TCX, then the bass post-filter. Random LPD states with the IMDCT state of a decoder after an
/// ACELP frame (no buffered output), valid pitch lags and sorted LSFs.
#[test]
fn frequency_to_time_lpd() {
    let mut rng = Rng::new(0xb10c_001b);
    let mut p = Pair::new();
    let (mut fac, mut imdct) = (0, 0);
    for it in 0..3000 {
        let spf = loop {
            let spf = random_state(&mut p, &mut rng, true, false, true);
            if spf == 1024 || spf == 768 {
                break spf;
            }
        };
        let ch = (rng.next_u32() % 2) as usize;
        {
            let RCtx { ci, sci, spec, wb1, .. } = &mut p.r;
            let (c, s) = (&mut ci[ch], &mut sci[ch]);
            s.last_core_mode = LPD;
            s.last_lpd_mode = [0, 0, 1, 2, 3, 4][(rng.next_u32() % 6) as usize];
            s.last_lpc_lost = (rng.next_u32() % 4 == 0) as u8;
            s.lpc4_lsf = rand_lsf(&mut rng);
            let sh = 2 + rng.next_u32() % 12;
            for x in s.old_synth.iter_mut().chain(s.mem_bpf.iter_mut()) {
                *x = random_spec(&mut rng) >> sh;
            }
            for (t, g) in s.old_t_pf.iter_mut().zip(s.old_gain_pf.iter_mut()) {
                *t = if rng.next_u32() % 4 == 0 { 64 } else { rng.range(34, 256) };
                *g = match rng.next_u32() % 4 {
                    0 => 0,
                    1 => -(rng.range(0, 1 << 20)),
                    _ => rng.range(0, 0x4000_0000),
                };
            }
            let a = &mut s.acelp;
            for x in a.old_exc_mem.iter_mut().chain(a.old_syn_mem.iter_mut()) {
                *x = random_spec(&mut rng) >> sh;
            }
            a.de_emph_mem = random_spec(&mut rng) >> sh;
            a.deemph_mem_wsyn = random_spec(&mut rng) >> sh;
            // the IMDCT after an ACELP frame: no buffered output
            s.i_mdct.ov_offset = 0;
            for x in s.i_mdct.overlap.iter_mut() {
                *x = random_spec(&mut rng) >> sh;
            }
            if rng.next_u32() % 2 == 0 {
                s.i_mdct.p_fac_zir = None;
            }
            let u = &mut c.data.usac;
            // (not FacDataPtr::Scratch, which has no defined target in C here; see the module
            // documentation of usacdec_lpd)
            u.fac_data[0] = [FacDataPtr::FacData0, FacDataPtr::FacData0, FacDataPtr::Null][(rng.next_u32() % 3) as usize];
            u.fac_data_e[0] = rng.range(-2, 24) as i8;
            let v = rand_dbl_vec(&mut rng, u.fac_data0.len());
            u.fac_data0.copy_from_slice(&v);
            c.curr_aliasing_symmetry = (rng.next_u32() % 8 == 0) as i32;
            for i in 0..1024 {
                spec[ch * 1024 + i] = random_spec(&mut rng) >> (rng.next_u32() % 8);
            }
            for x in c.spec_scale.iter_mut() {
                *x = rng.range(-2, 20) as i16;
            }
            for x in wb1.buf.iter_mut() {
                *x = random_spec(&mut rng) >> sh;
            }
        }
        let el_flags = if rng.next_u32() % 8 == 0 { AC_EL_LPDSTEREOIDX } else { 0 };
        let frame_ok = (rng.next_u32() % 4 != 0) as i32;
        let headroom = [0, 1, 2, 3][(rng.next_u32() % 4) as usize];
        let last = p.r.sci[ch].last_lpd_mode;
        p.sync();
        unsafe { c::cref_block_frequency_to_time(p.c.0, ch as i32, spf as i32, frame_ok, headroom, el_flags, ch as i32) };
        let RCtx { ci, sci, spec, wb1, out, .. } = &mut p.r;
        cblock_frequency_to_time(
            &mut sci[ch],
            &mut ci[ch],
            &mut out[ch],
            spf as i16,
            frame_ok,
            &mut wb1.buf,
            headroom,
            el_flags,
            ch as i32,
            &mut spec[ch * 1024..],
        );
        if last == 0 || last == 4 {
            fac += 1;
        } else {
            imdct += 1;
        }
        p.check(&format!("CBlock_FrequencyToTime (LPD) {it} spf {spf} last_lpd_mode {last} elFlags {el_flags:#x}"));
    }
    eprintln!("CBlock_FrequencyToTime (LPD): {fac} FAC transitions, {imdct} IMDCT after TCX");
}

/// Sequences of frames through CBlock_FrequencyToTimeLowDelay() (AAC-ELD) per frame length.
#[test]
fn frequency_to_time_low_delay() {
    let mut rng = Rng::new(0xb10c_0019);
    let mut p = Pair::new();
    for seq in 0..300 {
        random_state(&mut p, &mut rng, false, true, true);
        let spf = [512, 480, 256, 240, 160, 128, 120][(rng.next_u32() % 7) as usize];
        for ch in 0..2 {
            p.r.sci[ch].i_mdct.overlap.iter_mut().for_each(|x| *x = 0);
        }
        p.sync();
        for frame in 0..8 {
            let ch = (rng.next_u32() % 2) as usize;
            let sh = rng.next_u32() % 12;
            for i in 0..1024 {
                p.r.spec[ch * 1024 + i] = random_spec(&mut rng) >> sh;
            }
            p.r.ci[ch].spec_scale[0] = rng.range(-4, 24) as i16;
            p.sync();
            unsafe { c::cref_block_frequency_to_time_ld(p.c.0, ch as i32, spf) };
            let RCtx { ci, sci, spec, out, .. } = &mut p.r;
            cblock_frequency_to_time_low_delay(&mut sci[ch], &mut ci[ch], &mut out[ch], spf as i16, &mut spec[ch * 1024..]);
            p.check(&format!("CBlock_FrequencyToTimeLowDelay seq {seq} frame {frame} spf {spf}"));
        }
    }
}

#[test]
fn limits_of_get_escape() {
    // the escape prefix limit and the values around it
    for q in [16, -16, MAXVAL_DBL, MINVAL_DBL + 1, 15, 17] {
        for fill in [0xFFu8, 0xFE, 0x80, 0x00, 0xF0] {
            let mut buf = vec![fill; 16];
            let mut cb = buf.clone();
            let mut cleft = 0;
            let ce = unsafe { c::cref_block_get_escape(cb.as_mut_ptr(), 16, q, &mut cleft) };
            let mut bs = bs_over(&mut buf);
            assert_eq!(cblock_get_escape(&mut bs, q), ce);
            assert_eq!(fdk_get_valid_bits(&mut bs) as i32, cleft);
        }
    }
}

/// The file-static table `name` of the C++ source (a copy compiled into the shim `f`).
fn c_table<T: Copy>(f: unsafe extern "C" fn(*const i8, *mut i32) -> *const core::ffi::c_void, name: &str) -> Vec<T> {
    let cname = std::ffi::CString::new(name).unwrap();
    let mut bytes = 0;
    let p = unsafe { f(cname.as_ptr(), &mut bytes) };
    assert!(!p.is_null(), "{name}");
    assert_eq!(bytes as usize % core::mem::size_of::<T>(), 0, "{name}");
    unsafe { core::slice::from_raw_parts(p as *const T, bytes as usize / core::mem::size_of::<T>()) }.to_vec()
}

/// NOISE_LEVEL_TAB (`noise_level_tab`, file-static in block.cpp), element by element.
#[test]
fn noise_level_tab() {
    assert_eq!(c_table::<i16>(c::cref_block_table, "noise_level_tab"), NOISE_LEVEL_TAB.to_vec());
}
