// Differential tests of fdk_aac_rs_sbrdec::sbrdec_drc against the C++ reference.
//
// Random DRC channel states (band counts and borders, factors and exponents, interpolation
// schemes, window sequences including short blocks, enabled/disabled) are fed frame by frame
// through sbrDecoder_drcApply (or slot by slot through sbrDecoder_drcApplySlot with a random
// maximum shift) for the slot counts of all frame lengths, with and without imaginary parts;
// slots and channel state are compared after every call, the channel state also after
// sbrDecoder_drcUpdateChannel and sbrDecoder_drcInitChannel.

use core::ffi::c_void;
use fdk_aac_rs_cref::sbrdec_sbrdec_drc::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sbrdec::sbr_ram::{pod_bytes, pod_bytes_mut};
use fdk_aac_rs_sbrdec::sbrdec_drc::*;

fn ptr(v: &mut SbrdecDrcChannel) -> *mut c_void {
    pod_bytes_mut(v).as_mut_ptr() as *mut c_void
}

/// Feeds random "next" factors (like sbrDecoder_drcFeedChannel).
fn feed(rng: &mut Rng, h: &mut SbrdecDrcChannel) {
    h.enable = if rng.next_u32() % 8 == 0 { 0 } else { 1 };
    let n = rng.range(1, SBRDEC_MAX_DRC_BANDS as i32) as usize;
    h.num_bands_next = if rng.next_u32() % 16 == 0 { rng.range(0, 16) as u32 } else { n as u32 };
    let mut t = 0i32;
    for b in 0..SBRDEC_MAX_DRC_BANDS {
        t += rng.range(0, 40);
        h.band_top_next[b] = t.min(255) as u16;
        h.next_fact_mag[b] = match rng.next_u32() % 4 {
            0 => 0x7FFF_FFFF,
            1 => 0x4000_0000,
            _ => (rng.next_u32() >> 1) as i32,
        };
    }
    h.next_fact_exp = rng.range(0, 8);
    h.drc_interpolation_scheme_next = rng.range(0, 8) as i16;
    h.win_sequence_next = rng.range(0, 3) as u8;
}

#[test]
fn drc_frames() {
    let mut rng = Rng::new(11);
    for it in 0..3000 {
        let mut c = SbrdecDrcChannel::ZERO;
        unsafe { cref_sbrdec_drc_init_channel(ptr(&mut c)) };
        let mut p = SbrdecDrcChannel::ZERO;
        sbr_decoder_drc_init_channel(Some(&mut p));
        assert_eq!(pod_bytes(&p), pod_bytes(&c), "init {it}");
        let n_col = [16, 15, 30, 32, 12, 24, 8][rng.range(0, 6) as usize];
        let use_im = rng.next_u32() % 3 != 0;
        let per_slot = rng.next_u32() % 2 == 0;
        for fr in 0..6 {
            feed(&mut rng, &mut c);
            p = c;
            let mut re: Vec<i32> = (0..64 * n_col).map(|_| rng.i32()).collect();
            let mut im: Vec<i32> = (0..64 * n_col).map(|_| rng.i32()).collect();
            let (mut re2, mut im2) = (re.clone(), im.clone());
            if per_slot {
                let max_shift = rng.range(0, 8);
                for col in 0..n_col {
                    let s = col * 64;
                    unsafe {
                        cref_sbrdec_drc_apply_slot(
                            ptr(&mut c),
                            re[s..].as_mut_ptr(),
                            if use_im { im[s..].as_mut_ptr() } else { core::ptr::null_mut() },
                            col as i32,
                            n_col as i32,
                            max_shift,
                        )
                    };
                    let (r, i) = (&mut re2[s..s + 64], &mut im2[s..s + 64]);
                    sbr_decoder_drc_apply_slot(Some(&mut p), r, if use_im { Some(i) } else { None }, col as i32, n_col as i32, max_shift);
                    assert_eq!(pod_bytes(&p), pod_bytes(&c), "apply slot state {it}/{fr}/{col}");
                }
            } else {
                let mut sf = rng.range(-8, 8);
                let mut sf2 = sf;
                unsafe {
                    cref_sbrdec_drc_apply(
                        ptr(&mut c),
                        re.as_mut_ptr(),
                        if use_im { im.as_mut_ptr() } else { core::ptr::null_mut() },
                        n_col as i32,
                        &mut sf,
                    )
                };
                let mut r: Vec<&mut [i32]> = re2.chunks_mut(64).collect();
                let mut i: Vec<&mut [i32]> = im2.chunks_mut(64).collect();
                sbr_decoder_drc_apply(Some(&mut p), &mut r, if use_im { &mut i } else { &mut [] }, n_col as i32, &mut sf2);
                assert_eq!(sf2, sf, "apply scale factor {it}/{fr}");
            }
            assert_eq!(re2, re, "real {it}/{fr}");
            if use_im {
                assert_eq!(im2, im, "imag {it}/{fr}");
            }
            assert_eq!(pod_bytes(&p), pod_bytes(&c), "apply state {it}/{fr}");
            unsafe { cref_sbrdec_drc_update_channel(ptr(&mut c)) };
            sbr_decoder_drc_update_channel(Some(&mut p));
            assert_eq!(pod_bytes(&p), pod_bytes(&c), "update {it}/{fr}");
        }
    }
}
