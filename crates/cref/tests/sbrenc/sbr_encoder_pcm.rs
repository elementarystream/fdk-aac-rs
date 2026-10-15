// Realistic PCM for the differential tests of fdk-aac-rs-sbrenc: the music and synthetic clips
// of the corpus (`work/corpus/*.raw`, interleaved s16le, see scripts/corpus.json). The files are
// local test material and are never committed; a test that finds none of them runs on the
// generated signals of `work/corpus-gen/` or, failing that, on a synthetic signal.

use std::path::PathBuf;

/// The work directory of the main checkout: the reference library of the cref build lives in
/// `<work>/build-cref/<variant>/<key>/`.
pub fn work_dir() -> Option<PathBuf> {
    let lib = PathBuf::from(env!("FDK_AAC_RS_CREF_LIB"));
    let mut p = lib.as_path();
    while let Some(parent) = p.parent() {
        if p.file_name().is_some_and(|n| n == "build-cref") {
            return Some(parent.to_path_buf());
        }
        p = parent;
    }
    None
}

/// Interleaved PCM of one clip.
pub struct Pcm {
    pub name: String,
    pub channels: usize,
    pub rate: i32,
    pub data: Vec<i16>,
}

/// At most this many samples per channel are loaded of every clip (the tests use a few seconds).
const MAX_SAMPLES: usize = 48000 * 12;

impl Pcm {
    fn load(path: &PathBuf, name: &str, channels: usize, rate: i32, skip_seconds: usize) -> Option<Pcm> {
        let bytes = std::fs::read(path).ok()?;
        let frame_bytes = 2 * channels;
        let skip = (skip_seconds * rate as usize * frame_bytes).min(bytes.len() / 2 / frame_bytes * frame_bytes);
        let bytes = &bytes[skip..];
        let n = (bytes.len() / frame_bytes).min(MAX_SAMPLES);
        let data = bytes[..n * frame_bytes]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        Some(Pcm { name: name.to_string(), channels, rate, data })
    }

    /// The music clips of the corpus (stereo), skipping the first 30 seconds (often quiet).
    pub fn all_stereo() -> Vec<Pcm> {
        let mut v = Vec::new();
        if let Some(w) = work_dir() {
            for (f, rate) in [
                ("music_e_48k", 48000),
                ("music_a_44k", 44100),
                ("music_g_48k", 48000),
                ("broadcast_a_48k", 48000),
                ("music_h_48k", 48000),
                ("music_d_44k", 44100),
                ("syn_sweep", 48000),
                ("syn_clicks", 48000),
                ("syn_white", 48000),
                ("syn_square_fs", 48000),
            ] {
                let skip = if f.starts_with("syn_") { 0 } else { 30 };
                if let Some(p) = Pcm::load(&w.join("corpus").join(format!("{f}.raw")), f, 2, rate, skip) {
                    v.push(p);
                }
            }
        }
        if v.is_empty() {
            v.push(Pcm::synthetic(2));
        }
        v
    }

    /// The 5.1 clip of the corpus, or a synthetic 6 channel signal.
    pub fn multichannel() -> Pcm {
        work_dir()
            .and_then(|w| Pcm::load(&w.join("corpus/c51_48k.raw"), "c51_48k", 6, 48000, 0))
            .unwrap_or_else(|| Pcm::synthetic(6))
    }

    /// A mixture of sines, chirps and noise bursts.
    pub fn synthetic(channels: usize) -> Pcm {
        let n = 48000 * 8;
        let mut data = Vec::with_capacity(n * channels);
        let mut seed = 0x1234_5678u32;
        for i in 0..n {
            let t = i as f64 / 48000.0;
            for ch in 0..channels {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = ((seed >> 16) as i32 - 32768) as f64 / 32768.0;
                let burst = if (i / 4800) % 5 == 0 { 0.3 * noise } else { 0.01 * noise };
                let f = 440.0 * (1.0 + ch as f64) + 3000.0 * t;
                let s = 0.4 * (2.0 * std::f64::consts::PI * f * t).sin()
                    + 0.2 * (2.0 * std::f64::consts::PI * 9000.0 * t).sin()
                    + burst;
                data.push((s * 32767.0).clamp(-32768.0, 32767.0) as i16);
            }
        }
        Pcm { name: format!("synthetic{channels}"), channels, rate: 48000, data }
    }

    pub fn samples_per_channel(&self) -> usize {
        self.data.len() / self.channels
    }

    /// `len` samples of channel `ch` starting at `start`, wrapping around at the end.
    pub fn channel_frame(&self, ch: usize, start: usize, len: usize) -> Vec<i16> {
        let n = self.samples_per_channel();
        (0..len).map(|i| self.data[((start + i) % n) * self.channels + ch % self.channels]).collect()
    }
}
