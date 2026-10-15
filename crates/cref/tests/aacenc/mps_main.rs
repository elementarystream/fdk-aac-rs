// Differential tests of fdk_aac_rs_aacenc::mps_main against the C++ reference.
//
// * FDK_MpegsEnc_GetClosestBitRate on the whole parameter grid; the table mpsConfigTab.
// * The MPEG Surround glue as a whole (FDK_MpegsEnc_Open/Init/Process/
//   WriteSpatialSpecificConfig/GetDelay/GetDecDelay/Close of the library): initializations
//   with the parameters aacEncInit uses (ELD, the 212 sampling rates, SBR ratios, frame lengths,
//   core coder delays), valid and invalid, then frames of random stereo audio. Compared after
//   every call: return values, the payload (size, type, bytes), the downmix written over the
//   input, the SpatialSpecificConfig (bits and bytes) and the delays.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_aacenc::aacenc::AacencExtPayload;
use fdk_aac_rs_aacenc::mps_main::*;
use fdk_aac_rs_cref::aacenc_mps_main as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream, BS_WRITER};

const RATES: [u32; 13] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000, 0];

#[test]
fn closest_bit_rate_and_table() {
    let mut v = [0u32; 6 * 16];
    // SAFETY: shim writes 6 values per entry.
    let n = unsafe { c::cref_mps_main_tab(v.as_mut_ptr()) } as usize;
    let t = mps_config_tab();
    assert_eq!(t.len(), n);
    for i in 0..n {
        assert_eq!(t[i].to_vec(), v[6 * i..6 * i + 6].to_vec(), "mpsConfigTab[{i}]");
    }
    let mut rng = Rng::new(21);
    for aot in [2, 5, 23, 39, 29] {
        for mode in [1, 2, 128, 0] {
            for &sr in RATES.iter() {
                for sbr in 0..=3u32 {
                    for _ in 0..30 {
                        let br = match rng.range(0, 2) {
                            0 => rng.range(0, 250000) as u32,
                            1 => rng.next_u32(),
                            _ => [16000, 18000, 39999, 40000, 64000, 64001, 192000, 192001][rng.range(0, 7) as usize],
                        };
                        // SAFETY: shim call.
                        let ec = unsafe { c::cref_mps_main_closest(aot, mode, sr, sbr, br) };
                        assert_eq!(
                            fdk_mpegs_enc_get_closest_bit_rate(aot, mode, sr, sbr, br),
                            ec,
                            "GetClosestBitRate({aot}, {mode}, {sr}, {sbr}, {br})"
                        );
                    }
                }
            }
        }
    }
}

struct Mps {
    c: *mut core::ffi::c_void,
    r: Option<Box<MpsEncoder>>,
}

impl Mps {
    fn open() -> Mps {
        let mut h = core::ptr::null_mut();
        // SAFETY: shim call.
        let ec = unsafe { c::cref_mps_main_open(&mut h) };
        let mut r = None;
        assert_eq!(fdk_mpegs_enc_open(Some(&mut r)), ec);
        Mps { c: h, r }
    }

    fn check(&mut self, what: &str) {
        for dec in 0..2 {
            // SAFETY: shim call.
            let dc = unsafe { c::cref_mps_main_get_delay(self.c, dec) };
            let dr = if dec == 0 {
                fdk_mpegs_enc_get_delay(self.r.as_deref())
            } else {
                fdk_mpegs_enc_get_dec_delay(self.r.as_deref())
            };
            assert_eq!(dr, dc, "{what}: delay {dec}");
        }
        for use_bs in [0, 1] {
            let mut cb = [0u8; 128];
            let mut cbits = 0;
            // SAFETY: shim with a 128-byte buffer.
            let cr = unsafe { c::cref_mps_main_write_ssc(self.c, use_bs, cb.as_mut_ptr(), &mut cbits) };
            let rb = BitMem::new(128);
            let mut bs = FdkBitStream::default();
            fdk_init_bit_stream(&mut bs, &rb, 128, 0, BS_WRITER);
            let rr = fdk_mpegs_enc_write_spatial_specific_config(
                self.r.as_deref(),
                if use_bs != 0 { Some(&mut bs) } else { None },
            );
            assert_eq!(rr, cr, "{what}: SSC bits");
            assert_eq!(fdk_get_valid_bits(&mut bs) as i32, cbits, "{what}: SSC written bits");
            assert_eq!(rb.to_vec(), cb, "{what}: SSC bytes");
        }
    }
}

impl Drop for Mps {
    fn drop(&mut self) {
        // SAFETY: opened in Mps::open.
        let ec = unsafe { c::cref_mps_main_close(self.c) };
        assert_eq!(fdk_mpegs_enc_close(Some(&mut self.r)), ec);
    }
}

#[test]
fn encoder_sequences() {
    let mut rng = Rng::new(22);
    let bufsize = 2 * 1024 + 100 + 1537;
    let mut frames = 0;
    let mut inits_ok = 0;
    for k in 0..120 {
        let mut m = Mps::open();
        m.check(&format!("sequence {k} open"));
        let mut ok = false;
        let mut fl = 512u32;
        for step in 0..14 {
            let what = format!("sequence {k} step {step}");
            if !ok || rng.range(0, 6) == 0 {
                let sr = [16000u32, 22050, 24000, 32000, 44100, 48000, 64000, 8000][rng.range(0, 7) as usize];
                let sbr = [0u32, 0, 1, 2][rng.range(0, 3) as usize];
                fl = [512u32, 480, 256, 240, 1024, 960, 500][rng.range(0, 6) as usize];
                if sbr != 0 && rng.range(0, 3) != 0 {
                    fl *= sbr;
                }
                let br = if rng.range(0, 3) == 0 {
                    rng.range(8000, 200000) as u32
                } else {
                    fdk_mpegs_enc_get_closest_bit_rate(AOT_ER_AAC_ELD, MODE_212, sr, sbr, rng.range(10000, 100000) as u32)
                        .max(0) as u32
                };
                let delay = fl / 2 + [0u32, 0, 1, 5, 6, 100][rng.range(0, 5) as usize];
                let aot = if rng.range(0, 10) == 0 { AOT_AAC_LC } else { AOT_ER_AAC_ELD };
                // SAFETY: shim call.
                let ec = unsafe { c::cref_mps_main_init(m.c, aot, sr, br, sbr, fl, bufsize as u32, delay) };
                let er = fdk_mpegs_enc_init(m.r.as_deref_mut(), aot, sr, br, sbr, fl, bufsize as u32, delay);
                assert_eq!(er, ec, "{what}: init({aot}, {sr}, {br}, {sbr}, {fl}, .., {delay})");
                m.check(&what);
                ok = ec == MPS_ENCODER_OK;
                if ok {
                    inits_ok += 1;
                }
                // C runs Process only after a successful initialization (aacEncInit fails
                // otherwise; after a failed one the reference has no defined result)
                if !ok {
                    continue;
                }
            }
            let mut audio = vec![0i16; 2 * bufsize];
            let amp = [0, 50, 4000, 32767][rng.range(0, 3) as usize];
            for ch in 0..2 {
                for i in 0..fl as usize {
                    audio[ch * bufsize + i] = if amp == 0 { 0 } else { (rng.next_u32() as i32 % amp) as i16 };
                }
            }
            let mut ca = audio.clone();
            let mut pl = [0i32; 4];
            let mut cb = [0u8; 128];
            // SAFETY: two channels of `fl` samples, `bufsize` apart.
            let ec = unsafe { c::cref_mps_main_process(m.c, ca.as_mut_ptr(), 2 * fl as i32, pl.as_mut_ptr(), cb.as_mut_ptr()) };
            let mut ra = audio.clone();
            let mut p = AacencExtPayload { associated_ch_element: -77, ..Default::default() };
            // SAFETY: as above.
            let er = unsafe { fdk_mpegs_enc_process(m.r.as_deref_mut(), ra.as_mut_ptr(), 2 * fl as i32, &mut p) };
            assert_eq!(er, ec, "{what}: process");
            let is_buf = p.p_data == m.r.as_ref().unwrap().sac_out_buffer.as_ptr();
            assert_eq!([is_buf as i32, p.data_size as i32, p.data_type, p.associated_ch_element], pl, "{what}: payload");
            if !p.p_data.is_null() {
                let rb = &m.r.as_ref().unwrap().sac_out_buffer[..128];
                assert_eq!(rb, &cb[..], "{what}: payload bytes");
            }
            assert!(ra == ca, "{what}: downmix");
            m.check(&what);
            frames += 1;
        }
    }
    eprintln!("mps_main: {inits_ok} initializations, {frames} frames");
    assert!(frames > 0);
}
