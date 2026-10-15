// Differential tests of fdk_aac_rs_sbrenc::resampler against the C++ reference.
//
// The filter tables; FDKaacEnc_InitDownsampler() for cutoff frequencies around every
// parameter set; FDKaacEnc_Downsample() in place (input behind the output, as the SBR encoder
// calls it) on sequences of frames of random, full-scale, alternating and realistic PCM, with
// the downsampler state compared after every call.

use fdk_aac_rs_cref::sbrenc_resampler as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrenc::resampler::*;

use super::sbr_encoder_pcm::Pcm;

fn state(d: &Downsampler) -> Vec<i32> {
    let mut v = Vec::new();
    for s in d.down_filter.states.iter() {
        v.extend_from_slice(s);
    }
    v.push(match d.down_filter.coeffa {
        None => -1,
        Some(p) => SOS_TABLES.iter().position(|s| std::ptr::eq(*s, p)).map_or(-2, |i| i as i32),
    });
    v.extend_from_slice(&[
        d.down_filter.gain,
        d.down_filter.wc,
        d.down_filter.no_coeffs,
        d.down_filter.ptr,
        d.ratio,
        d.delay,
        d.pending,
    ]);
    v
}

/// The SOS tables in the order of the shim's index.
static SOS_TABLES: [&[i16]; 5] = [&SOS48, &SOS45, &SOS41, &SOS35, &SOS25];

#[test]
fn tables() {
    let mut out = vec![0i32; 4096];
    for (id, t) in SOS_TABLES.iter().enumerate() {
        let n = unsafe { c::cref_resampler_table(id as i32, out.as_mut_ptr()) };
        let v: Vec<i32> = t.iter().map(|&x| x as i32).collect();
        assert_eq!(&out[..n as usize], &v[..], "sos {id}");
    }
    let sets = [&PARAM_SET48, &PARAM_SET45, &PARAM_SET41, &PARAM_SET35, &PARAM_SET25];
    for (k, p) in sets.iter().enumerate() {
        let n = unsafe { c::cref_resampler_table(5 + k as i32, out.as_mut_ptr()) };
        let idx = SOS_TABLES.iter().position(|s| std::ptr::eq(*s, p.coeffa)).unwrap() as i32;
        assert_eq!(&out[..n as usize], &[idx, p.g, p.wc, p.no_coeffs, p.delay], "param_set {k}");
    }
    let n = unsafe { c::cref_resampler_table(10, out.as_mut_ptr()) };
    let idx: Vec<i32> = FILTER_PARAM_SET
        .iter()
        .map(|p| sets.iter().position(|s| std::ptr::eq(*s, *p)).unwrap() as i32)
        .collect();
    assert_eq!(&out[..n as usize], &idx[..], "filter_paramSet");
}

struct Pair {
    c: *mut core::ffi::c_void,
    r: Downsampler,
}

impl Pair {
    fn new() -> Self {
        Pair { c: unsafe { c::cref_resampler_new() }, r: Downsampler::default() }
    }
    fn init(&mut self, wc: i32, ratio: i32) {
        let rc = unsafe { c::cref_resampler_init(self.c, wc, ratio) };
        let rr = fdk_aac_enc_init_downsampler(&mut self.r, wc, ratio);
        assert_eq!(rc, rr);
        self.check("init");
    }
    fn check(&self, what: &str) {
        let mut v = vec![0i32; 40];
        unsafe { c::cref_resampler_state(self.c, v.as_mut_ptr()) };
        assert_eq!(v, state(&self.r), "{what}");
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        unsafe { c::cref_resampler_free(self.c) };
    }
}

fn run(pair: &mut Pair, frames: &[Vec<i16>], frame_len: usize, offset: usize) {
    for (f, input) in frames.iter().enumerate() {
        let mut bc = vec![0i16; offset + frame_len];
        bc[offset..].copy_from_slice(&input[..frame_len]);
        let mut br = bc.clone();
        let (mut nc, mut nr) = (0, 0);
        let rc = unsafe {
            c::cref_resampler_downsample(pair.c, bc.as_mut_ptr(), offset as i32, frame_len as i32, 0, &mut nc)
        };
        let rr = fdk_aac_enc_downsample(&mut pair.r, &mut br, offset, frame_len as i32, 0, &mut nr);
        assert_eq!((rc, nc), (rr, nr));
        assert_eq!(bc, br, "frame {f}");
        pair.check(&format!("frame {f}"));
    }
}

#[test]
fn init_all_cutoffs() {
    let mut p = Pair::new();
    for wc in [-1, 0, 100, 249, 250, 251, 349, 350, 351, 409, 410, 411, 449, 450, 451, 479, 480, 481, 500, 1000] {
        for ratio in [1, 2, 3] {
            p.init(wc, ratio);
        }
    }
}

#[test]
fn downsample_random() {
    let mut rng = Rng::new(0x5eed_0101);
    for &wc in &[0, 250, 350, 410, 450, 500] {
        let mut p = Pair::new();
        p.init(wc, 2);
        let frames: Vec<Vec<i16>> = (0..40)
            .map(|k| {
                (0..2048)
                    .map(|i| match k % 4 {
                        0 => rng.i16(),
                        1 => if i % 2 == 0 { i16::MAX } else { i16::MIN },
                        2 => if (i / 7) % 2 == 0 { i16::MIN } else { i16::MAX },
                        _ => (rng.i16() >> 4) as i16,
                    })
                    .collect()
            })
            .collect();
        run(&mut p, &frames, 2048, 1537);
    }
}

#[test]
fn downsample_pcm() {
    let mut p = Pair::new();
    p.init(500, 2);
    for pcm in Pcm::all_stereo().iter().take(4) {
        let frames: Vec<Vec<i16>> = (0..60).map(|f| pcm.channel_frame(0, f * 2048, 2048)).collect();
        run(&mut p, &frames, 2048, 1000);
    }
}
