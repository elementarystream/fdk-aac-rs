// Differential tests of fdk_aac_rs_sacenc::sacenc_delay against the C++ reference.
//
// The delay calculation with the configurations of the encoder (QMF bands 32/64, the frame
// lengths, core coder delays around half a frame, time alignments) and with random values in
// every field of the configuration (also those that the encoder never sets), with and without
// time domain downmix, dmx alignment and delay minimization. The whole structure is compared.

use fdk_aac_rs_cref::sacenc_sacenc_delay as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacenc::sacenc_delay as r;
use fdk_aac_rs_sacenc::sacenc_delay::{Delay, DelayConfig};

pub fn delay_ints(d: &Delay) -> [i32; 23] {
    let c = &d.config;
    [
        c.b_dmx_align,
        c.b_time_dom_dmx,
        c.b_minimize_delay,
        c.b_sac_time_alignment_dynamic_out,
        c.n_qmf_len,
        c.n_frame_len,
        c.n_surround_delay,
        c.n_arb_dmx_delay,
        c.n_limiter_delay,
        c.n_core_coder_delay,
        c.n_sac_stream_mux_delay,
        c.n_sac_time_alignment,
        d.n_dmx_align_buffer,
        d.n_surround_analysis_buffer,
        d.n_arb_dmx_analysis_buffer,
        d.n_output_audio_buffer,
        d.n_bitstream_frame_buffer,
        d.n_output_audio_qmf_frame_buffer,
        d.n_discard_out_frames,
        d.n_bitstream_frame_buffer_size,
        d.n_info_dmx_delay,
        d.n_info_codec_delay,
        d.n_info_decoder_delay,
    ]
}

pub fn delay_from(v: &[i32; 23]) -> Delay {
    Delay {
        config: DelayConfig {
            b_dmx_align: v[0],
            b_time_dom_dmx: v[1],
            b_minimize_delay: v[2],
            b_sac_time_alignment_dynamic_out: v[3],
            n_qmf_len: v[4],
            n_frame_len: v[5],
            n_surround_delay: v[6],
            n_arb_dmx_delay: v[7],
            n_limiter_delay: v[8],
            n_core_coder_delay: v[9],
            n_sac_stream_mux_delay: v[10],
            n_sac_time_alignment: v[11],
        },
        n_dmx_align_buffer: v[12],
        n_surround_analysis_buffer: v[13],
        n_arb_dmx_analysis_buffer: v[14],
        n_output_audio_buffer: v[15],
        n_bitstream_frame_buffer: v[16],
        n_output_audio_qmf_frame_buffer: v[17],
        n_discard_out_frames: v[18],
        n_bitstream_frame_buffer_size: v[19],
        n_info_dmx_delay: v[20],
        n_info_codec_delay: v[21],
        n_info_decoder_delay: v[22],
    }
}

fn getters(d: &Delay) -> [i32; 9] {
    [
        r::fdk_sacenc_delay_get_output_audio_buffer_delay(d),
        r::fdk_sacenc_delay_get_surround_analysis_buffer_delay(d),
        r::fdk_sacenc_delay_get_arb_dmx_analysis_buffer_delay(d),
        r::fdk_sacenc_delay_get_bitstream_frame_buffer_size(d),
        r::fdk_sacenc_delay_get_dmx_align_buffer_delay(d),
        r::fdk_sacenc_delay_get_discard_out_frames(d),
        r::fdk_sacenc_delay_get_info_dmx_delay(d),
        r::fdk_sacenc_delay_get_info_codec_delay(d),
        r::fdk_sacenc_delay_get_info_decoder_delay(d),
    ]
}

fn small(rng: &mut Rng) -> i32 {
    match rng.range(0, 4) {
        0 => 0,
        1 => rng.range(-100, 100),
        2 => rng.range(-5000, 5000),
        3 => rng.range(-1, 2),
        _ => rng.range(-40000, 40000),
    }
}

#[test]
fn calculate() {
    assert_eq!(unsafe { c::cref_sacenc_delay_size() }, 23);
    let mut rng = Rng::new(1);
    for it in 0..100000 {
        let h = unsafe { c::cref_sacenc_delay_open() };
        let mut d: Option<Box<Delay>> = None;
        assert_eq!(r::fdk_sacenc_delay_open(Some(&mut d)), 0);
        let mut d = d.unwrap();
        // Start from a random structure.
        let img: [i32; 23] = core::array::from_fn(|_| small(&mut rng));
        unsafe { c::cref_sacenc_delay_set(h, img.as_ptr()) };
        *d = delay_from(&img);

        // Divisors of 0 and -1 (with INT_MIN) have no defined result in the x86 reference (see
        // the module documentation); they are not tested.
        let nz = |v: i32| if v == 0 || v == -1 { 7 } else { v };
        let q = nz([32, 64, 32, 64, small(&mut rng)][rng.range(0, 4) as usize]);
        let nfts = rng.range(1, 16);
        let f = nz(if rng.range(0, 5) == 0 { small(&mut rng) } else { q * nfts });
        let core = match rng.range(0, 3) {
            0 => f / 2 + rng.range(0, 5),
            1 => f / 2 + rng.range(-10, 10),
            2 => small(&mut rng),
            _ => rng.i32() >> rng.range(0, 31),
        };
        let mux = if rng.range(0, 1) == 0 { 0 } else { small(&mut rng) };
        let ec = unsafe { c::cref_sacenc_delay_init(h, q, f, core, mux) };
        let er = r::fdk_sacenc_delay_init(&mut d, q, f, core, mux);
        assert_eq!(ec, er);

        for _ in 0..rng.range(0, 6) {
            let which = rng.range(0, 4);
            let v = match rng.range(0, 2) {
                0 => rng.range(-1, 2),
                1 => rng.range(-40000, 40000),
                _ => small(&mut rng),
            };
            let ec = unsafe { c::cref_sacenc_delay_setter(h, which, v) };
            let er = match which {
                0 => r::fdk_sacenc_delay_set_dmx_align(&mut d, v),
                1 => r::fdk_sacenc_delay_set_time_dom_dmx(&mut d, v),
                2 => r::fdk_sacenc_delay_set_sac_time_alignment_dynamic_out(&mut d, v),
                3 => r::fdk_sacenc_delay_set_n_sac_time_alignment(&mut d, v),
                _ => r::fdk_sacenc_delay_set_minimize_delay(&mut d, v),
            };
            assert_eq!(ec, er);
        }
        if rng.range(0, 2) == 0 {
            // Fields that the encoder does not set.
            let mut v = [0i32; 23];
            unsafe { c::cref_sacenc_delay_get(h, v.as_mut_ptr()) };
            for k in [6, 7, 8] {
                v[k] = small(&mut rng);
            }
            unsafe { c::cref_sacenc_delay_set(h, v.as_ptr()) };
            *d = delay_from(&v);
        }

        let ec = unsafe { c::cref_sacenc_delay_calc(h) };
        let er = r::fdk_sacenc_delay_sub_calulate_buffer_delays(&mut d);
        let mut v = [0i32; 23];
        unsafe { c::cref_sacenc_delay_get(h, v.as_mut_ptr()) };
        assert_eq!((ec, v), (er, delay_ints(&d)), "it {it}");
        let mut g = [0i32; 9];
        unsafe { c::cref_sacenc_delay_getters(h, g.as_mut_ptr()) };
        assert_eq!(g, getters(&d));

        assert_eq!(unsafe { c::cref_sacenc_delay_close(h) }, r::fdk_sacenc_delay_close(Some(&mut Some(d))));
    }
    assert_eq!(unsafe { c::cref_sacenc_delay_open_null() }, r::fdk_sacenc_delay_open(None));
    assert_eq!(unsafe { c::cref_sacenc_delay_close_null() }, r::fdk_sacenc_delay_close(None));
}
