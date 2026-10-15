// Realistic input data for the differential tests of the quantization and noiseless coding
// modules (line_pe, sf_estim, quantize, bit_cnt, dyn_bits).
//
// A channel is produced like the encoder produces it before the quantization stage:
// - PCM from the corpus (`work/corpus`, never committed; a synthetic signal if it is missing),
// - the forward MDCT of the port (`fdk_aac_rs_fdk::mdct::mdct_block`) with the window sequences
//   of the encoder (LONG, or LONG -> START -> SHORT),
// - the scalefactor band layout of a real configuration (C `FDKaacEnc_InitPsyConfiguration`),
// - the lowpass and the block floating point normalization of psy_main,
// - band energies and grouping with the reference's own helpers
//   (`FDKaacEnc_CalcBandEnergyOptimLong/Short`, `FDKaacEnc_groupShortData`, `LdDataVector`),
// - thresholds as random fractions of the energies (as the threshold adjustment leaves them).

use fdk_aac_rs_cref::aacenc_sf_estim_data as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::{fl2fxconst_dbl, FixpDbl};
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::mdct::{mdct_block, mdct_init, MdctT};
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// The quantizer input of one channel, in the layout of `PSY_OUT_CHANNEL`/`QC_OUT_CHANNEL`.
#[derive(Clone)]
pub struct ChannelData {
    pub short: bool,
    pub sfb_cnt: i32,
    pub sfb_per_group: i32,
    pub max_sfb_per_group: i32,
    pub sfb_offsets: [i32; 61],
    pub no_of_groups: i32,
    pub group_len: [i32; 4],
    pub spectrum: [FixpDbl; 1024],
    pub energy: [FixpDbl; 60],
    pub energy_ld: [FixpDbl; 60],
    pub thr: [FixpDbl; 60],
    pub thr_ld: [FixpDbl; 60],
    pub min_snr_ld: [FixpDbl; 60],
    pub spread_energy: [FixpDbl; 60],
    /// `sfbFormFactorLdData` (C `FDKaacEnc_CalcFormFactor`)
    pub form_factor_ld: [FixpDbl; 60],
}

/// (sample rate, bit rate per channel, bandwidth)
const CONFIGS: &[(i32, i32, i32)] = &[
    (48000, 64000, 17000),
    (48000, 128000, 20000),
    (44100, 48000, 14000),
    (44100, 96000, 19000),
    (32000, 32000, 11000),
    (24000, 24000, 8000),
    (22050, 40000, 10000),
    (16000, 16000, 6000),
    (11025, 12000, 5000),
    (8000, 12000, 3900),
    (96000, 128000, 20000),
    (64000, 96000, 20000),
];

/// Groupings of the 8 short windows (group lengths).
const GROUPINGS: &[&[i32]] = &[&[8], &[1, 7], &[2, 2, 2, 2], &[4, 1, 3], &[1, 1, 1, 5], &[3, 5]];

fn work_dir() -> PathBuf {
    // FDK_AAC_RS_CREF_LIB = <work>/build-cref/<variant>/<key>/libfdkaac_ref.a
    let lib = PathBuf::from(env!("FDK_AAC_RS_CREF_LIB"));
    lib.ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build-cref"))
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
}

const CORPUS: &[(&str, usize)] = &[
    ("music_c_48k.raw", 2),
    ("music_a_44k.raw", 2),
    ("music_b_mono_44k.raw", 1),
    ("broadcast_a_48k.raw", 2),
    ("music_g_48k.raw", 2),
    ("syn_clicks.raw", 2),
    ("syn_sweep.raw", 2),
    ("syn_pink.raw", 2),
    ("syn_white.raw", 2),
    ("syn_square_fs.raw", 2),
];

/// `n` samples of one channel of a corpus file at a random position (s16le, interleaved), or a
/// synthetic signal if the corpus is not available.
pub fn pcm(rng: &mut Rng, n: usize) -> Vec<IntPcm> {
    let (name, chans) = CORPUS[rng.range(0, CORPUS.len() as i32 - 1) as usize];
    let path = work_dir().join("corpus").join(name);
    if let Ok(mut f) = std::fs::File::open(&path) {
        let len = f.metadata().map(|m| m.len()).unwrap_or(0) as usize;
        let frame = 2 * chans;
        if len >= (n + 1) * frame {
            let pos = (rng.next_u64() as usize % (len / frame - n)) * frame;
            let mut buf = vec![0u8; n * frame];
            if f.seek(SeekFrom::Start(pos as u64)).is_ok() && f.read_exact(&mut buf).is_ok() {
                let ch = rng.range(0, chans as i32 - 1) as usize;
                return (0..n)
                    .map(|i| i16::from_le_bytes([buf[i * frame + 2 * ch], buf[i * frame + 2 * ch + 1]]))
                    .collect();
            }
        }
    }
    // Synthetic fallback: a few partials plus noise.
    let f1 = rng.range(50, 8000) as f64 / 48000.0;
    let f2 = rng.range(50, 15000) as f64 / 48000.0;
    let amp = rng.range(100, 20000) as f64;
    (0..n)
        .map(|i| {
            let t = i as f64;
            let v = amp * (0.6 * (std::f64::consts::TAU * f1 * t).sin()
                + 0.3 * (std::f64::consts::TAU * f2 * t).sin())
                + (rng.range(-1000, 1000) as f64) * amp / 20000.0;
            v.clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

/// Forward MDCT of frame 2 of a window sequence (`short`: LONG, START, SHORT; else LONG, LONG,
/// LONG), as `FDKaacEnc_Transform_Real` calls it.
fn spectrum(rng: &mut Rng, short: bool) -> [FixpDbl; 1024] {
    let pcm = pcm(rng, 4 * 1024);
    let shape = rng.range(0, 1);
    let mut m = MdctT::default();
    mdct_init(&mut m, 0);
    let mut out = [0; 1024];
    let seq: [i32; 3] = if short { [0, 1, 2] } else { [0, 0, 0] };
    for (k, &bt) in seq.iter().enumerate() {
        let (n_spec, tl, fr) = match bt {
            0 => (1, 1024, 1024),
            1 => (1, 1024, 128),
            _ => (8, 128, 128),
        };
        let mut e = [0i16; 8];
        mdct_block(
            &mut m,
            &pcm[k * 1024..k * 1024 + 2048],
            1024,
            &mut out,
            n_spec,
            tl,
            fdk_get_window_slope(fr, shape),
            fr,
            &mut e,
        );
    }
    out
}

/// A random fraction in `lo..hi` as `FIXP_DBL`.
fn frac(rng: &mut Rng, lo: f64, hi: f64) -> FixpDbl {
    fl2fxconst_dbl(lo + (hi - lo) * (rng.next_u32() as f64 / 4294967296.0))
}

/// A realistic channel (long or grouped short blocks).
pub fn channel(rng: &mut Rng, short: bool) -> ChannelData {
    let (sr, br, bw) = CONFIGS[rng.range(0, CONFIGS.len() as i32 - 1) as usize];
    let mut sfb_offset = [0i32; 52];
    let mut min_snr = [0i32; 51];
    let (mut sfb_cnt, mut sfb_active) = (0, 0);
    let err = unsafe {
        c::cref_sf_estim_data_psy_conf(
            br,
            sr,
            bw,
            if short { 2 } else { 0 },
            1024,
            sfb_offset.as_mut_ptr(),
            min_snr.as_mut_ptr(),
            &mut sfb_cnt,
            &mut sfb_active,
        )
    };
    assert_eq!(err, 0);
    let mut spec = spectrum(rng, short);

    // Lowpass and block floating point normalization (psy_main).
    let win_len = if short { 128 } else { 1024 };
    let lowpass = sfb_offset[sfb_active as usize] as usize;
    for w in 0..1024 / win_len {
        for l in lowpass..win_len {
            spec[w * win_len + l] = 0;
        }
    }
    let max = spec.iter().map(|&x| (x as i64).abs()).max().unwrap_or(0);
    if max != 0 {
        let headroom = (max as u64).leading_zeros() as i32 - 33; // bits of headroom in i32
        let shift = headroom - rng.range(0, 4) - short as i32;
        for x in spec.iter_mut() {
            *x = if shift >= 0 { *x << shift } else { *x >> -shift };
        }
    }

    let mut d = ChannelData {
        short,
        sfb_cnt: 0,
        sfb_per_group: 0,
        max_sfb_per_group: 0,
        sfb_offsets: [0; 61],
        no_of_groups: 1,
        group_len: [0; 4],
        spectrum: spec,
        energy: [0; 60],
        energy_ld: [0; 60],
        thr: [0; 60],
        thr_ld: [0; 60],
        min_snr_ld: [0; 60],
        spread_energy: [0; 60],
        form_factor_ld: [0; 60],
    };
    let mut ratio = [0i32; 120];
    let mut spr_ratio = [0i32; 120];
    for r in ratio.iter_mut() {
        *r = match rng.range(0, 3) {
            0 => frac(rng, 0.9, 1.0),
            1 => frac(rng, 0.0, 0.01),
            _ => frac(rng, 0.0, 0.3),
        };
    }
    for r in spr_ratio.iter_mut() {
        *r = frac(rng, 0.0, 1.0);
    }
    if !short {
        d.sfb_cnt = sfb_active;
        d.sfb_per_group = sfb_active;
        d.sfb_offsets[..52].copy_from_slice(&sfb_offset);
        d.min_snr_ld[..51].copy_from_slice(&min_snr);
        d.group_len[0] = 1;
        unsafe {
            c::cref_sf_estim_data_long(
                d.spectrum.as_ptr(),
                d.sfb_offsets.as_ptr(),
                sfb_active,
                ratio.as_ptr(),
                spr_ratio.as_ptr(),
                d.energy.as_mut_ptr(),
                d.energy_ld.as_mut_ptr(),
                d.thr.as_mut_ptr(),
                d.thr_ld.as_mut_ptr(),
                d.spread_energy.as_mut_ptr(),
            )
        };
        // maxSfbPerGroup for long blocks (psy_main)
        let mut sfb = sfb_active - 1;
        while sfb >= 0 {
            let lo = sfb_offset[sfb as usize];
            let mut line = sfb_offset[sfb as usize + 1] - 1;
            while line >= lo && d.spectrum[line as usize] == 0 {
                line -= 1;
            }
            if line > lo {
                break;
            }
            sfb -= 1;
        }
        d.max_sfb_per_group = (sfb + 1).max(5.min(sfb_active));
    } else {
        let g = GROUPINGS[rng.range(0, GROUPINGS.len() as i32 - 1) as usize];
        d.no_of_groups = g.len() as i32;
        d.group_len[..g.len()].copy_from_slice(g);
        d.sfb_cnt = d.no_of_groups * sfb_cnt;
        d.sfb_per_group = sfb_cnt;
        unsafe {
            c::cref_sf_estim_data_short(
                d.spectrum.as_mut_ptr(),
                sfb_offset.as_ptr(),
                sfb_cnt,
                sfb_active,
                min_snr.as_ptr(),
                d.no_of_groups,
                d.group_len.as_ptr(),
                ratio.as_ptr(),
                spr_ratio.as_ptr(),
                d.sfb_offsets.as_mut_ptr(),
                &mut d.max_sfb_per_group,
                d.min_snr_ld.as_mut_ptr(),
                d.energy.as_mut_ptr(),
                d.energy_ld.as_mut_ptr(),
                d.thr.as_mut_ptr(),
                d.thr_ld.as_mut_ptr(),
                d.spread_energy.as_mut_ptr(),
            )
        };
    }
    unsafe {
        c::cref_sf_estim_data_form_factor(
            d.spectrum.as_mut_ptr(),
            d.sfb_offsets.as_ptr(),
            d.sfb_cnt,
            d.sfb_per_group,
            d.max_sfb_per_group,
            d.form_factor_ld.as_mut_ptr(),
        )
    };
    // Some bands with a threshold above the energy (as after the threshold adjustment).
    for i in 0..60 {
        if rng.range(0, 15) == 0 {
            d.thr_ld[i] = d.energy_ld[i].saturating_add(rng.range(0, 1 << 22));
        }
    }
    d
}

#[test]
fn realistic_data_is_plausible() {
    let mut rng = Rng::new(1);
    let mut nonzero = 0;
    for k in 0..40 {
        let d = channel(&mut rng, k % 2 == 1);
        assert!(d.max_sfb_per_group > 0 && d.max_sfb_per_group <= d.sfb_per_group);
        assert!(d.sfb_cnt <= 60);
        nonzero += d.spectrum.iter().any(|&x| x != 0) as i32;
    }
    // (silent passages of the corpus give zero spectra)
    assert!(nonzero >= 30, "{nonzero}");
}
