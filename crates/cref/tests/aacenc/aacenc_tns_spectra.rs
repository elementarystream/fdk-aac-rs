// Realistic MDCT spectra for the TNS and PNS tests: real PCM of the corpus (work/corpus, read at
// test time, never committed) transformed with the ported fdk_aac_rs_fdk::mdct::mdct_block the
// way FDKaacEnc_Transform_Real calls it, with the block switching sequences of the encoder.
// Without the corpus, synthetic signals are used.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::mdct::{mdct_block, mdct_init, MdctT};
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// `work/` of the main checkout (the reference library lives in `work/build-cref/<v>/<key>/`).
pub fn work_dir() -> PathBuf {
    let lib = PathBuf::from(env!("FDK_AAC_RS_CREF_LIB"));
    lib.ancestors().nth(4).unwrap().to_path_buf()
}

/// Corpus files (name, channels).
const FILES: &[(&str, usize)] = &[
    ("music_a_44k.raw", 2),
    ("music_e_48k.raw", 2),
    ("music_g_48k.raw", 2),
    ("music_b_mono_44k.raw", 1),
    ("music_e_96k.raw", 2),
    ("broadcast_a_48k.raw", 2),
    ("syn_sweep.raw", 2),
    ("syn_white.raw", 2),
    ("syn_pink.raw", 2),
    ("syn_clicks.raw", 2),
];

/// `n` samples of channel 0 of corpus file `f` starting at a random position, or a synthetic
/// signal if the file is missing.
pub fn pcm(r: &mut Rng, f: usize, n: usize) -> Vec<i16> {
    let (name, ch) = FILES[f % FILES.len()];
    let path = work_dir().join("corpus").join(name);
    if let Ok(mut file) = File::open(&path) {
        let len = file.metadata().map(|m| m.len()).unwrap_or(0) as usize / (2 * ch);
        if len > n {
            let start = (r.next_u64() as usize) % (len - n);
            let mut buf = vec![0u8; n * 2 * ch];
            if file.seek(SeekFrom::Start((start * 2 * ch) as u64)).is_ok()
                && file.read_exact(&mut buf).is_ok()
            {
                return (0..n)
                    .map(|i| i16::from_le_bytes([buf[i * 2 * ch], buf[i * 2 * ch + 1]]))
                    .collect();
            }
        }
    }
    synthetic(r, f, n)
}

/// A uniform random value in [-1, 1).
fn uniform(r: &mut Rng) -> f64 {
    (r.next_u32() as f64) / 2147483648.0 - 1.0
}

/// A synthetic stand-in for corpus file `f` when the corpus is missing: signals with the temporal
/// structure that makes TNS work (note onsets with decaying partials, percussive noise bursts,
/// clicks), and stationary ones (a sweep, white and pink noise), so that the tests see HIFILT and
/// LOFILT filters, merged filters and frames with two filters without the corpus as well.
fn synthetic(r: &mut Rng, f: usize, n: usize) -> Vec<i16> {
    let amp = 2000.0 + (r.next_u32() % 14000) as f64;
    let mut x = vec![0f64; n];
    match f % FILES.len() {
        // "music": notes (attack, exponential decay, a few partials) and percussive noise bursts
        0..=5 => {
            let mut t = 0usize;
            while t < n {
                let w = 0.005 + (r.next_u32() % 1000) as f64 * 1.5e-4;
                let attack = 10 + (r.next_u32() % 300) as usize;
                let decay = 500.0 + (r.next_u32() % 20000) as f64;
                let partials = 1 + (r.next_u32() % 6) as usize;
                let a = 0.2 + uniform(r).abs();
                for (i, v) in x[t..].iter_mut().enumerate().take((decay * 6.0) as usize) {
                    let env = if i < attack {
                        i as f64 / attack as f64
                    } else {
                        (-((i - attack) as f64) / decay).exp()
                    };
                    let mut s = 0.0;
                    for k in 1..=partials {
                        s += (w * k as f64 * i as f64).sin() / k as f64;
                    }
                    *v += a * env * s;
                }
                if r.next_u32() % 2 == 0 {
                    let decay = 30.0 + (r.next_u32() % 1500) as f64;
                    let a = 0.3 + uniform(r).abs();
                    let start = t + (r.next_u32() % 4000) as usize;
                    for (i, v) in x.iter_mut().skip(start).enumerate().take((decay * 8.0) as usize)
                    {
                        *v += a * (-(i as f64) / decay).exp() * uniform(r);
                    }
                }
                t += 800 + (r.next_u32() % 12000) as usize;
            }
        }
        // sweep
        6 => {
            let (w0, w1) = (0.001 + uniform(r).abs() * 0.01, 0.5 + uniform(r).abs() * 2.5);
            for (i, v) in x.iter_mut().enumerate() {
                let p = i as f64 / n as f64;
                *v = (i as f64 * (w0 + (w1 - w0) * p / 2.0)).sin();
            }
        }
        // white noise
        7 => x.iter_mut().for_each(|v| *v = uniform(r)),
        // pink noise (Paul Kellet's economy filter)
        8 => {
            let (mut b0, mut b1, mut b2) = (0.0, 0.0, 0.0);
            for v in x.iter_mut() {
                let w = uniform(r);
                b0 = 0.99765 * b0 + w * 0.0990460;
                b1 = 0.96300 * b1 + w * 0.2965164;
                b2 = 0.57000 * b2 + w * 1.0526913;
                *v = (b0 + b1 + b2 + w * 0.1848) * 0.25;
            }
        }
        // clicks: impulses with a short ringing decay at irregular distances
        _ => {
            let mut t = (r.next_u32() % 2000) as usize;
            while t < n {
                let w = 0.3 + uniform(r).abs() * 2.5;
                let decay = 3.0 + (r.next_u32() % 200) as f64;
                let a = uniform(r);
                for (i, v) in x[t..].iter_mut().enumerate().take((decay * 10.0) as usize) {
                    *v += a * (-(i as f64) / decay).exp() * (w * i as f64).cos();
                }
                t += 200 + (r.next_u32() % 5000) as usize;
            }
        }
    }
    x.iter().map(|&v| (amp * v).round().clamp(-32768.0, 32767.0) as i16).collect()
}

/// One transformed frame: block type, spectrum (`frame_len` lines; 8 windows of
/// `frame_len / 8` for short blocks) and the exponents.
pub struct Frame {
    pub block_type: i32,
    pub spectrum: Vec<i32>,
}

/// `frames` consecutive frames of a corpus file, transformed as FDKaacEnc_Transform_Real does
/// (LD: `ld` with the low-overlap window shape 2, only long blocks).
pub fn spectra(r: &mut Rng, file: usize, frame_len: i32, ld: bool, frames: usize) -> Vec<Frame> {
    let n = frame_len as usize;
    let input = pcm(r, file, n * (frames + 1));
    let mut mdct = MdctT::default();
    mdct_init(&mut mdct, 0);
    let mut out = Vec::new();
    let mut block_type = 0;
    let shapes: &[i32] = if ld { &[0, 2] } else { &[0, 1] };
    let shape = shapes[r.range(0, 1) as usize];
    for frame in 0..frames {
        // the transitions of the encoder's block switching
        block_type = if ld {
            0
        } else {
            match block_type {
                0 | 3 => [0, 0, 1][r.range(0, 2) as usize],
                _ => [2, 3][r.range(0, 1) as usize],
            }
        };
        let (n_spec, tl) = if block_type == 2 { (8, frame_len >> 3) } else { (1, frame_len) };
        let offset = if shape == 2 { (frame_len * 3) >> 2 } else { 0 };
        let fr = match block_type {
            0 | 3 => frame_len - offset,
            _ => frame_len >> 3,
        };
        let time = &input[frame * n..frame * n + 2 * n];
        let mut spec = vec![0i32; 2 * n];
        let mut e = [0i16; 8];
        mdct_block(
            &mut mdct,
            time,
            frame_len,
            &mut spec,
            n_spec,
            tl,
            fdk_get_window_slope(fr, shape),
            fr,
            &mut e,
        );
        spec.truncate(n);
        out.push(Frame { block_type, spectrum: spec });
    }
    out
}
