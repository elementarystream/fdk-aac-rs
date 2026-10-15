// Differential tests of fdk_aac_rs_sbrenc::tran_det against the C++ reference.
//
// The transient detector, the fast transient detector of low delay SBR and the frame splitter
// of the library and of the port, on sequences of frames of random energies (all magnitudes,
// steady, decaying, bursts and silence, the scale factors of both halves of the energy buffer
// from 0 to beyond 31), for the HE-AAC (32 and 64 bands, 16 and 15 time slots) and low delay
// configurations; the detector state is compared after every frame. The whole encoder test
// (sbr_encoder.rs) runs them on the energies of real audio.

use fdk_aac_rs_cref::sbrenc_tran_det as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrenc::sbr_encoder::{SbrConfiguration, SBR_SYNTAX_LOW_DELAY};
use fdk_aac_rs_sbrenc::tran_det::*;

use super::sbr_encoder_dump::{assert_dump_eq, fast_tran_det, tran_det, Dump};

type H = *mut core::ffi::c_void;

/// 32 rows of 64 energies. `style` selects the signal.
fn energies(rng: &mut Rng, style: i32, frame: usize) -> Vec<i32> {
    let mut v = vec![0i32; 32 * 64];
    let base = rng.range(0, 30);
    for r in 0..32 {
        for b in 0..64 {
            v[r * 64 + b] = match style {
                0 => (rng.next_u32() >> 1) as i32 >> rng.range(0, 30),
                1 => (0x7fff_ffff >> (base + (b as i32 / 8)).min(31)) - rng.range(0, 1000),
                2 => {
                    if (r + frame) % 11 == 3 {
                        0x7fff_ffff - rng.range(0, 0xffff)
                    } else {
                        rng.range(0, 0xffff) >> (b as i32 / 4)
                    }
                }
                3 => 0,
                4 => rng.range(0, 3),
                _ => ((rng.next_u32() >> 1) as i32) >> (base + rng.range(0, 3)).min(31),
            }
            .max(0);
        }
    }
    v
}

fn rows(v: &[i32]) -> Vec<&[i32]> {
    v.chunks(64).collect()
}

fn check(hc: H, r: &SbrTransientDetector, what: &str) {
    let mut buf = vec![0i32; 512];
    let n = unsafe { c::cref_tran_det_dump(hc, buf.as_mut_ptr(), 512) };
    let mut d = Dump::default();
    tran_det(&mut d, r);
    assert_dump_eq(&buf[..n as usize], &d.0, what);
}

fn check_fast(hc: H, r: &FastTranDetector, what: &str) {
    let mut buf = vec![0i32; 1024];
    let n = unsafe { c::cref_tran_det_fast_dump(hc, buf.as_mut_ptr(), 1024) };
    let mut d = Dump::default();
    fast_tran_det(&mut d, r);
    assert_dump_eq(&buf[..n as usize], &d.0, what);
}

/// A random increasing high resolution frequency band table starting at `k0` within `rows`.
fn freq_band_table(rng: &mut Rng, no_rows: i32) -> (Vec<u8>, i32) {
    let mut t = vec![rng.range(4, no_rows / 2) as u8];
    while t.len() < 48 {
        let next = *t.last().unwrap() as i32 + rng.range(1, 4);
        if next > no_rows {
            break;
        }
        t.push(next as u8);
    }
    let n = t.len() as i32 - 1;
    t.resize(49, 0);
    (t, n)
}

#[allow(clippy::too_many_arguments)]
fn init_pair(
    rng: &mut Rng,
    flags: u32,
    frame_size: i32,
    fs: i32,
    no_cols: i32,
    no_rows: i32,
    wo: i32,
    sz: i32,
    frame_shift: i32,
    tran_off: i32,
) -> (H, SbrTransientDetector) {
    let mut params = [
        [6000, 13000][rng.range(0, 1) as usize],
        [24000, 0, 64000][rng.range(0, 2) as usize],
        rng.range(1, 2),
        [0, 8000, 24000, 64000, 128000, 256000][rng.range(0, 5) as usize],
        rng.range(0, 1),
    ];
    let tran_fc = rng.range(0, 40);
    let hc = unsafe { c::cref_tran_det_new() };
    let rc = unsafe {
        c::cref_tran_det_init(hc, flags, frame_size, fs, params.as_mut_ptr(), tran_fc, no_cols, no_rows, wo, sz, frame_shift, tran_off)
    };
    let mut cfg = SbrConfiguration::default();
    cfg.tran_thr = params[0];
    cfg.codec_settings.standard_bitrate = params[1];
    cfg.codec_settings.n_channels = params[2];
    cfg.codec_settings.bit_rate = params[3];
    cfg.tran_det_mode = params[4];
    let mut r = SbrTransientDetector::default();
    let rr = fdk_sbr_enc_init_sbr_transient_detector(
        &mut r, flags, frame_size, fs, &cfg, tran_fc, no_cols, no_rows, wo, sz, frame_shift, tran_off,
    );
    assert_eq!(rc, rr);
    check(hc, &r, "init");
    (hc, r)
}

#[test]
fn transient_detector_and_splitter() {
    let mut rng = Rng::new(0x5eed_0301);
    // (frame size, fs, no_cols, no_rows, YBufferWriteOffset, tran_off) of HE-AAC
    let configs = [
        (2048, 44100, 32, 64, 16, 16),
        (2048, 48000, 32, 64, 16, 16),
        (2048, 32000, 32, 64, 16, 16),
        (1024, 24000, 32, 32, 16, 16),
        (1920, 48000, 30, 64, 15, 15),
    ];
    for round in 0..30 {
        for &(fsz, fs, no_cols, no_rows, wo, tran_off) in &configs {
            let (hc, mut r) = init_pair(&mut rng, 0, fsz, fs, no_cols, no_rows, wo, 1, 0, tran_off);
            let (fbt, n_sfb) = freq_band_table(&mut rng, no_rows);
            let mut fbt_c = fbt.clone();
            let style = rng.range(0, 5);
            for f in 0..40 {
                let st = if rng.range(0, 7) == 0 { rng.range(0, 5) } else { style };
                let mut nrg = energies(&mut rng, st, f);
                let mut scale = [rng.range(0, 40), rng.range(0, 40)];
                if rng.range(0, 3) != 0 {
                    scale[1] = (scale[0] + rng.range(-3, 3)).max(0);
                }
                let mut info_c = [9u8; 3];
                let mut info_r = [9u8; 3];
                unsafe { c::cref_tran_det_detect(hc, nrg.as_mut_ptr(), scale.as_mut_ptr(), info_c.as_mut_ptr(), wo, 1, 2, 4) };
                fdk_sbr_enc_transient_detect(&mut r, &rows(&nrg), &scale, &mut info_r, wo, 1, 2, 4);
                assert_eq!(info_c, info_r, "transientDetect info, round {round} frame {f}");
                check(hc, &r, &format!("transientDetect round {round} frame {f}"));

                let mut ton_c = rng.i32();
                let mut ton_r = ton_c;
                if rng.range(0, 1) == 0 {
                    info_c[1] = 0;
                    info_r[1] = 0;
                }
                unsafe {
                    c::cref_tran_det_split(hc, nrg.as_mut_ptr(), scale.as_mut_ptr(), fbt_c.as_mut_ptr(), info_c.as_mut_ptr(), wo, 1, n_sfb, 2, no_cols, &mut ton_c)
                };
                fdk_sbr_enc_frame_splitter(&rows(&nrg), &scale, &mut r, &fbt, &mut info_r, wo, 1, n_sfb, 2, no_cols, &mut ton_r);
                assert_eq!((info_c, ton_c), (info_r, ton_r), "frameSplitter, round {round} frame {f}");
                check(hc, &r, &format!("frameSplitter round {round} frame {f}"));
            }
            unsafe { c::cref_tran_det_free(hc) };
        }
    }
}

#[test]
fn low_delay() {
    let mut rng = Rng::new(0x5eed_0302);
    // (frame size (SBR), fs (SBR), slots, no_rows)
    let configs = [(1024, 48000, 16, 64), (1024, 44100, 16, 64), (960, 48000, 15, 64), (512, 32000, 16, 32), (1024, 32000, 16, 64), (1024, 24000, 16, 64)];
    for round in 0..30 {
        for &(fsz, fs, slots, no_rows) in &configs {
            let wo = slots / 2;
            let tran_off = 3 + 4;
            let (hc, mut r) =
                init_pair(&mut rng, SBR_SYNTAX_LOW_DELAY, fsz, fs, slots, no_rows, wo, 0, 3, tran_off);
            let (fbt, n_sfb) = freq_band_table(&mut rng, no_rows);
            let mut fbt_c = fbt.clone();
            let bw = (fs >> 1) / no_rows;
            let hf = unsafe { c::cref_tran_det_fast_new() };
            let rc = unsafe { c::cref_tran_det_fast_init(hf, slots, bw, no_rows, fbt[0] as i32) };
            let mut rf = FastTranDetector::default();
            let rr = fdk_sbr_enc_init_sbr_fast_transient_detector(&mut rf, slots, bw, no_rows, fbt[0] as i32);
            assert_eq!(rc, rr);
            check_fast(hf, &rf, "fast init");
            let style = rng.range(0, 5);
            for f in 0..40 {
                let st = if rng.range(0, 7) == 0 { rng.range(0, 5) } else { style };
                let mut nrg = energies(&mut rng, st, f);
                let mut scale = [rng.range(0, 40), rng.range(0, 40)];
                if rng.range(0, 3) != 0 {
                    scale[1] = (scale[0] + rng.range(-3, 3)).max(0);
                }
                let mut info_c = [9u8; 3];
                let mut info_r = [9u8; 3];
                unsafe { c::cref_tran_det_fast_detect(hf, nrg.as_mut_ptr(), scale.as_mut_ptr(), wo, info_c.as_mut_ptr()) };
                fdk_sbr_enc_fast_transient_detect(&mut rf, &rows(&nrg), &scale, wo, &mut info_r);
                assert_eq!(info_c, info_r, "fastTransientDetect, round {round} frame {f}");
                check_fast(hf, &rf, &format!("fastTransientDetect round {round} frame {f}"));

                let mut ton_c = rng.i32();
                let mut ton_r = ton_c;
                unsafe {
                    c::cref_tran_det_split(hc, nrg.as_mut_ptr(), scale.as_mut_ptr(), fbt_c.as_mut_ptr(), info_c.as_mut_ptr(), wo, 0, n_sfb, 1, slots, &mut ton_c)
                };
                fdk_sbr_enc_frame_splitter(&rows(&nrg), &scale, &mut r, &fbt, &mut info_r, wo, 0, n_sfb, 1, slots, &mut ton_r);
                assert_eq!((info_c, ton_c), (info_r, ton_r), "LD frameSplitter, round {round} frame {f}");
                check(hc, &r, &format!("LD frameSplitter round {round} frame {f}"));
            }
            unsafe {
                c::cref_tran_det_free(hc);
                c::cref_tran_det_free(hf);
            }
        }
    }
}
