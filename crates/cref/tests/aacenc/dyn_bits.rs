// Differential tests of fdk_aac_rs_aacenc::dyn_bits against the C++ reference.
//
// FDKaacEnc_dynBitCount (and through it buildBitLookUp, findBestBook, the merge helpers,
// gmStage0/1/2, noiselessCounter, scfCount and noiseCount) on random layouts (long blocks, up to
// 4 groups of short blocks), spectra over all code book ranges, PNS and intensity bands, with and
// without scalefactors and VCB11, and on quantized real spectra. The complete SECTION_DATA (C
// layout) and the return value are compared. The C bit counter's look-up tables are filled with
// a different pseudo-random pattern before every call while the port's are zero, which checks
// that no stale contents of the shared dynamic RAM are read.

use crate::sf_estim_data::channel;
use fdk_aac_rs_aacenc::aacenc_pns::NO_NOISE_PNS;
use fdk_aac_rs_aacenc::dyn_bits::*;
use fdk_aac_rs_aacenc::quantize::fdk_aac_enc_quantize_spectrum;
use fdk_aac_rs_cref::aacenc_dyn_bits as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::AC_ER_VCB11;

struct Input {
    quant: Vec<i16>,
    max_value: Vec<u32>,
    scalefac: Option<Vec<i32>>,
    block_type: i32,
    sfb_cnt: i32,
    max_sfb: i32,
    sfb_per_group: i32,
    sfb_offset: Vec<i32>,
    noise_nrg: Vec<i32>,
    is_book: Vec<i32>,
    is_scale: Vec<i32>,
    flags: u32,
}

fn check(what: &str, rng: &mut Rng, inp: &Input) {
    let mut sd0 = SectionData::default();
    // random previous contents of the output structure
    let words = std::mem::size_of::<SectionData>() / 4;
    let p = &mut sd0 as *mut SectionData as *mut i32;
    for k in 0..words {
        unsafe { *p.add(k) = rng.i32() };
    }
    let mut sd_r = sd0;
    let mut sd_c = sd0;
    let mut bc = BitcntrState::default();
    fdk_aac_enc_bc_new(&mut bc);
    let r = fdk_aac_enc_dyn_bit_count(
        &mut bc,
        &inp.quant,
        &inp.max_value,
        inp.scalefac.as_deref(),
        inp.block_type,
        inp.sfb_cnt,
        inp.max_sfb,
        inp.sfb_per_group,
        &inp.sfb_offset,
        &mut sd_r,
        &inp.noise_nrg,
        &inp.is_book,
        &inp.is_scale,
        inp.flags,
    );
    let cc = unsafe {
        c::cref_dyn_bits_dynBitCount(
            rng.next_u32(),
            inp.quant.as_ptr(),
            inp.max_value.as_ptr(),
            inp.scalefac.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            inp.block_type,
            inp.sfb_cnt,
            inp.max_sfb,
            inp.sfb_per_group,
            inp.sfb_offset.as_ptr(),
            &mut sd_c as *mut SectionData as *mut _,
            inp.noise_nrg.as_ptr(),
            inp.is_book.as_ptr(),
            inp.is_scale.as_ptr(),
            inp.flags,
        )
    };
    assert_eq!(r, cc, "{what}: return value");
    assert_eq!(sd_r, sd_c, "{what}: SECTION_DATA");
    fdk_aac_enc_bc_close(&mut bc);
}

/// maxValueInSfb as FDKaacEnc_calcMaxValueInSfb computes it (other entries keep `stale`).
fn max_values(inp: &mut Input, stale: u32) {
    inp.max_value = vec![stale; 60];
    let mut sfb_offs = 0;
    while sfb_offs < inp.sfb_cnt {
        for sfb in 0..inp.max_sfb {
            let i = (sfb_offs + sfb) as usize;
            let (lo, hi) = (inp.sfb_offset[i] as usize, inp.sfb_offset[i + 1] as usize);
            inp.max_value[i] =
                inp.quant[lo..hi].iter().map(|&x| (x as i32).unsigned_abs()).max().unwrap_or(0);
        }
        sfb_offs += inp.sfb_per_group;
    }
}

fn side_bands(rng: &mut Rng, inp: &mut Input, pns: bool, is: bool) {
    inp.noise_nrg = vec![NO_NOISE_PNS; 60];
    inp.is_book = vec![0; 60];
    inp.is_scale = (0..60).map(|_| rng.range(-60, 60)).collect();
    for i in 0..60 {
        if pns && rng.range(0, 5) == 0 {
            inp.noise_nrg[i] = rng.range(-100, 100);
        }
        if is && rng.range(0, 4) == 0 {
            inp.is_book[i] = rng.range(14, 15);
        }
    }
}

#[test]
fn random() {
    let mut rng = Rng::new(0xd7b);
    let lavs = [0, 1, 2, 4, 7, 12, 15, 16, 100, 8191];
    for k in 0..6000 {
        let short = rng.range(0, 1) == 1;
        let (groups, per) =
            if short { (rng.range(1, 4), rng.range(1, 15)) } else { (1, rng.range(1, 51)) };
        let sfb_cnt = groups * per;
        let max_sfb = if rng.range(0, 9) == 0 { 0 } else { rng.range(1, per) };
        let mut sfb_offset = vec![0i32; 61];
        for i in 1..61 {
            sfb_offset[i] = (sfb_offset[i - 1] + 4 * rng.range(1, if short { 4 } else { 8 }))
                .min(1024 + 4 * i as i32);
        }
        let mut quant = vec![0i16; 1024 + 512];
        for i in 0..60 {
            let m = lavs[rng.range(0, lavs.len() as i32 - 1) as usize];
            for l in sfb_offset[i]..sfb_offset[i + 1] {
                if rng.range(0, 2) != 0 {
                    quant[l as usize] = rng.range(-m, m) as i16;
                }
            }
        }
        let scalefac = match rng.range(0, 5) {
            0 => None,
            1 => Some((0..60).map(|_| rng.range(-80, 80)).collect()),
            _ => Some((0..60).map(|_| rng.range(0, 60)).collect()),
        };
        let mut inp = Input {
            quant,
            max_value: vec![],
            scalefac,
            block_type: if short { 2 } else { [0, 1, 3][rng.range(0, 2) as usize] },
            sfb_cnt,
            max_sfb,
            sfb_per_group: per,
            sfb_offset,
            noise_nrg: vec![],
            is_book: vec![],
            is_scale: vec![],
            flags: if rng.range(0, 1) == 1 { AC_ER_VCB11 } else { 0 },
        };
        max_values(&mut inp, rng.next_u32() % 20);
        let (pns, is) = (rng.range(0, 2) == 0, rng.range(0, 2) == 0);
        side_bands(&mut rng, &mut inp, pns, is);
        check(&format!("random {k}"), &mut rng, &inp);
    }
}

/// Quantized real spectra with scalefactors in the range the scalefactor estimation produces.
#[test]
fn realistic() {
    let mut rng = Rng::new(0xd7c);
    for k in 0..600 {
        let d = channel(&mut rng, k % 3 == 2);
        let scf: Vec<i32> = (0..60).map(|_| rng.range(0, 20)).collect();
        let mut quant = vec![0i16; 1024];
        let mut gg = rng.range(-20, 60);
        loop {
            fdk_aac_enc_quantize_spectrum(
                d.sfb_cnt,
                d.max_sfb_per_group,
                d.sfb_per_group,
                &d.sfb_offsets,
                &d.spectrum,
                gg,
                &scf,
                &mut quant,
                rng.range(0, 1),
            );
            if quant.iter().all(|&q| (q as i32).abs() <= 8191) {
                break;
            }
            gg += 4;
        }
        let mut inp = Input {
            quant,
            max_value: vec![],
            scalefac: Some(scf),
            block_type: if d.short { 2 } else { 0 },
            sfb_cnt: d.sfb_cnt,
            max_sfb: d.max_sfb_per_group,
            sfb_per_group: d.sfb_per_group,
            sfb_offset: d.sfb_offsets.to_vec(),
            noise_nrg: vec![],
            is_book: vec![],
            is_scale: vec![],
            flags: 0,
        };
        max_values(&mut inp, 0);
        side_bands(&mut rng, &mut inp, k % 5 == 0, k % 7 == 0);
        check(&format!("realistic {k}"), &mut rng, &inp);
    }
}
