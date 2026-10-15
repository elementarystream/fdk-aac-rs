// Differential tests of fdk_aac_rs_aacdec::aacdec_drc against the C++ reference.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_aacdec::aacdec_drc::*;
use fdk_aac_rs_aacdec::channelinfo::CAacDecoderChannelInfo;
use fdk_aac_rs_cref::aacdec_aacdec_drc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, fdk_push_for, FdkBitStream, BS_READER};
use fdk_aac_rs_fdk::mdct::BlockType;

const NST: usize = 37;
const NCH: usize = 35;

fn to_vec(d: &CDrcInfo) -> [i32; NST] {
    let p = &d.params;
    let mut o = [0i32; NST];
    let v: Vec<i32> = [
        p.usr_cut,
        p.usr_boost,
        p.usr_apply_heavy_compression as i32,
        p.cut,
        p.boost,
        p.target_ref_level as i32,
        p.apply_heavy_compression as i32,
        p.expiry_frame as i32,
        p.bs_delay_enable as i32,
        p.default_presentation_mode,
        p.encoder_target_level as i32,
        d.enable as i32,
        d.digital_norm as i32,
        d.update as i32,
        d.num_out_channels,
        d.prev_aac_num_channels,
        d.num_payloads as i32,
        d.num_threads as i32,
        d.prog_ref_level as i32,
        d.prog_ref_level_present as i32,
        d.prl_expiry_count as i32,
        d.pres_mode as i32,
        d.dvb_anc_data_available as i32,
        d.dvb_anc_data_position as i32,
    ]
    .into_iter()
    .chain(d.drc_payload_position.iter().map(|&x| x as i32))
    .chain([
        d.apply_ext_gain as i32,
        d.additional_gain_prev,
        d.additional_gain_filter_state,
        d.additional_gain_filter_state1,
    ])
    .collect();
    o.copy_from_slice(&v);
    o
}

fn from_vec(o: &[i32; NST]) -> CDrcInfo {
    let mut d = CDrcInfo::default();
    let mut n = 0;
    let mut nx = || {
        n += 1;
        o[n - 1]
    };
    d.params.usr_cut = nx();
    d.params.usr_boost = nx();
    d.params.usr_apply_heavy_compression = nx() as u8;
    d.params.cut = nx();
    d.params.boost = nx();
    d.params.target_ref_level = nx() as i8;
    d.params.apply_heavy_compression = nx() as u8;
    d.params.expiry_frame = nx() as u32;
    d.params.bs_delay_enable = nx() as u8;
    d.params.default_presentation_mode = nx();
    d.params.encoder_target_level = nx() as u8;
    d.enable = nx() as u8;
    d.digital_norm = nx() as u8;
    d.update = nx() as u8;
    d.num_out_channels = nx();
    d.prev_aac_num_channels = nx();
    d.num_payloads = nx() as u16;
    d.num_threads = nx() as u16;
    d.prog_ref_level = nx() as i8;
    d.prog_ref_level_present = nx() as u8;
    d.prl_expiry_count = nx() as u32;
    d.pres_mode = nx() as i8;
    d.dvb_anc_data_available = nx() as u8;
    d.dvb_anc_data_position = nx() as u32;
    for i in 0..MAX_DRC_THREADS as usize {
        d.drc_payload_position[i] = nx() as u32;
    }
    d.apply_ext_gain = nx() as u8;
    d.additional_gain_prev = nx();
    d.additional_gain_filter_state = nx();
    d.additional_gain_filter_state1 = nx();
    d
}

fn ch_to(c: &CDrcChannelData) -> [i32; NCH] {
    let mut o = [0i32; NCH];
    o[0] = c.expiry_count as i32;
    o[1] = c.num_bands as i32;
    for i in 0..16 {
        o[2 + i] = c.band_top[i] as i32;
    }
    o[18] = c.drc_interpolation_scheme as i32;
    for i in 0..16 {
        o[19 + i] = c.drc_value[i] as i32;
    }
    o
}

fn ch_from(o: &[i32]) -> CDrcChannelData {
    let mut c = CDrcChannelData { expiry_count: o[0] as u32, num_bands: o[1] as u32, ..Default::default() };
    for i in 0..16 {
        c.band_top[i] = o[2 + i] as u16;
    }
    c.drc_interpolation_scheme = o[18] as i16;
    for i in 0..16 {
        c.drc_value[i] = o[19 + i] as u8;
    }
    c.drc_data_type = o[35] as i8;
    c
}

/// CDrcChannelData as the 36 values of the shim (with drcDataType).
fn ch_full(c: &CDrcChannelData) -> [i32; 36] {
    let mut o = [0i32; 36];
    o[..NCH].copy_from_slice(&ch_to(c));
    o[35] = c.drc_data_type as i32;
    o
}

/// A random but plausible DRC state.
fn random_state(rng: &mut Rng) -> CDrcInfo {
    let mut d = CDrcInfo::default();
    aac_decoder_drc_init(Some(&mut d));
    let step = (rng.next_u32() % 128) as i32;
    d.params.usr_cut = step * 8456010 / 2;
    d.params.usr_boost = (rng.next_u32() % 128) as i32 * 8456010 / 2;
    d.params.usr_apply_heavy_compression = (rng.next_u32() % 2) as u8;
    d.params.target_ref_level = [(-1i32), 40, 64, 96, 100, 124, 127][(rng.next_u32() % 7) as usize] as i8;
    d.params.default_presentation_mode = (rng.next_u32() % 4) as i32 - 1;
    d.params.encoder_target_level = (rng.next_u32() % 128) as u8;
    d.params.expiry_frame = [0, 1, 5, 100][(rng.next_u32() % 4) as usize];
    d.params.bs_delay_enable = (rng.next_u32() % 2) as u8;
    d.num_out_channels = (rng.next_u32() % 9) as i32 - 1;
    d.prev_aac_num_channels = (rng.next_u32() % 9) as i32;
    d.prog_ref_level = (rng.next_u32() % 128) as i8;
    d.prog_ref_level_present = (rng.next_u32() % 2) as u8;
    d.pres_mode = (rng.next_u32() % 5) as i8 - 1;
    d.update = (rng.next_u32() % 2) as u8;
    d.prl_expiry_count = rng.next_u32() % 200;
    d.additional_gain_prev = rng.i32() >> (rng.next_u32() % 8);
    d.additional_gain_filter_state = rng.i32() >> (rng.next_u32() % 8);
    d.additional_gain_filter_state1 = rng.i32() >> (rng.next_u32() % 8);
    d.enable = (rng.next_u32() % 2) as u8;
    d.apply_ext_gain = (rng.next_u32() % 2) as u8;
    d
}

#[test]
fn convert_drc_param_matches() {
    let mut rng = Rng::new(1);
    for k in 0..20000 {
        let v = if k < 256 { k * 4194304 } else { rng.i32() };
        assert_eq!(convert_drc_param(v), unsafe { c::cref_drc_convert_param(v) }, "{v}");
    }
}

#[test]
fn init_disable_reset_get_info() {
    let mut rng = Rng::new(2);
    for k in 0..2000 {
        let d = random_state(&mut rng);
        let mut st = to_vec(&d);
        if k % 3 == 0 {
            unsafe { c::cref_drc_init(st.as_mut_ptr()) };
            let mut p = d;
            aac_decoder_drc_init(Some(&mut p));
            assert_eq!(st, to_vec(&p));
        } else {
            let which = (k % 3 == 1) as i32;
            unsafe { c::cref_drc_disable_reset(st.as_mut_ptr(), which) };
            let mut p = d;
            if which != 0 {
                aac_decoder_drc_reset(&mut p);
            } else {
                aac_decoder_drc_disable(&mut p);
            }
            assert_eq!(st, to_vec(&p));
        }
        let mut st = to_vec(&d);
        let (mut pm, mut prl) = (0, 0);
        unsafe { c::cref_drc_get_info(st.as_mut_ptr(), &mut pm, &mut prl) };
        let (mut a, mut b) = (-5i8, -5i8);
        aac_decoder_drc_get_info(Some(&d), Some(&mut a), Some(&mut b));
        assert_eq!((pm, prl), (a as i32, b as i32));
    }
    let mut ch = CDrcChannelData { num_bands: 5, expiry_count: 9, drc_data_type: 1, ..Default::default() };
    ch.band_top[0] = 77;
    ch.drc_value[0] = 9;
    let mut v = ch_full(&ch);
    unsafe { c::cref_drc_init_channel_data(v.as_mut_ptr()) };
    aac_decoder_drc_init_channel_data(Some(&mut ch));
    assert_eq!(v, ch_full(&ch));
}

#[test]
fn set_param_and_parameter_handling() {
    let mut rng = Rng::new(3);
    for k in 0..20000 {
        let d = random_state(&mut rng);
        let param = (rng.next_u32() % 11) as i32 - 1;
        let value = match rng.next_u32() % 4 {
            0 => rng.i32(),
            1 => (rng.next_u32() % 300) as i32 - 150,
            2 => (rng.next_u32() % 3) as i32 - 1,
            _ => (rng.next_u32() % 130) as i32,
        };
        let null = (k % 50 == 0) as i32;
        let mut st = to_vec(&d);
        let rr = unsafe { c::cref_drc_set_param(st.as_mut_ptr(), null, param, value) };
        let mut p = d;
        let rp = aac_decoder_drc_set_param(if null != 0 { None } else { Some(&mut p) }, param, value);
        assert_eq!(rr, rp, "set_param({param}, {value})");
        assert_eq!(st, to_vec(&p), "set_param({param}, {value}) state");

        let d = random_state(&mut rng);
        let n = (rng.next_u32() % 9) as i32;
        let prl = (rng.next_u32() % 130) as i32 - 1;
        let pm = (rng.next_u32() % 5) as i32 - 1;
        let mut st = to_vec(&d);
        unsafe { c::cref_drc_parameter_handling(st.as_mut_ptr(), n, prl, pm) };
        let mut p = d;
        aac_decoder_drc_parameter_handling(&mut p, n, prl as i8, pm as i8);
        assert_eq!(st, to_vec(&p), "parameter handling");
    }
}

#[test]
fn bitstream_functions() {
    let mut rng = Rng::new(4);
    for k in 0..30000 {
        let len = 4 + (rng.next_u32() % 60) as usize;
        let mut buf: Vec<u8> = (0..64).map(|i| if i < len { rng.next_u32() as u8 } else { 0 }).collect();
        if k % 4 == 1 {
            buf[0] = 0xBC; // DVB sync byte
            buf[1] = 0xC0 | (rng.next_u32() as u8 & 0x3F);
        }
        let skip = (rng.next_u32() % 8) as i32;
        let which = (k % 5) as i32;
        let mut d = random_state(&mut rng);
        d.num_payloads = (rng.next_u32() % 10) as u16;
        d.dvb_anc_data_available = (rng.next_u32() % 2) as u8;
        let pos = (len * 8) as u32 - (rng.next_u32() % 9);
        let mut pl0 = [0i32; 40];
        pl0[0] = rng.next_u32() as i32 & 0xff;
        pl0[1] = -1;
        pl0[3] = -1;
        pl0[4 + 3] = 255;
        let mut st = to_vec(&d);
        let mut pl = pl0;
        let mut bits = 0;
        let mut cbuf = buf.clone();
        let rr = unsafe {
            c::cref_drc_bitstream(which, cbuf.as_mut_ptr(), len as i32, skip, pos, st.as_mut_ptr(), pl.as_mut_ptr(), &mut bits)
        };

        let mut bs = FdkBitStream::default();
        let pbuf = buf.clone();
        fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&pbuf), 64, (len * 8) as u32, BS_READER);
        fdk_push_for(&mut bs, skip as u32);
        let mut p = d;
        let mut payload = CDrcPayload {
            excluded_chns_mask: pl0[0] as u32,
            prog_ref_level: pl0[1] as i8,
            pres_mode: pl0[2] as i8,
            pce_instance_tag: pl0[3] as i8,
            channel_data: ch_from(&pl0[4..40]),
        };
        let rp = match which {
            0 => aac_decoder_drc_mark_payload(Some(&mut p), &mut bs, MPEG_DRC_EXT_DATA),
            1 => aac_decoder_drc_mark_payload(Some(&mut p), &mut bs, DVB_DRC_ANC_DATA),
            2 => aac_decoder_drc_parse(&mut bs, &mut payload, pos),
            3 => aac_decoder_drc_read_compression(&mut bs, &mut payload, pos),
            _ => parse_excluded_channels(&mut payload.excluded_chns_mask, &mut bs),
        };
        assert_eq!(rr, rp, "bitstream {which}");
        assert_eq!(bits, fdk_get_valid_bits(&mut bs) as i32, "bits {which}");
        assert_eq!(st, to_vec(&p), "state {which}");
        let mut plp = [0i32; 40];
        plp[0] = payload.excluded_chns_mask as i32;
        plp[1] = payload.prog_ref_level as i32;
        plp[2] = payload.pres_mode as i32;
        plp[3] = payload.pce_instance_tag as i32;
        plp[4..40].copy_from_slice(&ch_full(&payload.channel_data));
        assert_eq!(pl, plp, "payload {which}");
    }
}

#[test]
fn apply() {
    let mut rng = Rng::new(5);
    for k in 0..6000 {
        let mut d = random_state(&mut rng);
        if k % 4 != 0 {
            d.enable = 1;
        }
        let mut ch = CDrcChannelData {
            expiry_count: rng.next_u32() % 8,
            num_bands: 1 + rng.next_u32() % 16,
            drc_interpolation_scheme: (rng.next_u32() % 16) as i16,
            drc_data_type: (rng.next_u32() % 3) as i8,
            ..Default::default()
        };
        let mut top = 0u32;
        for i in 0..16 {
            top += 1 + rng.next_u32() % 40;
            ch.band_top[i] = top.min(255) as u16;
            ch.drc_value[i] = rng.next_u32() as u8;
        }
        if k % 7 == 0 {
            ch.drc_value[0] = 0x7F;
        }
        let frame = [1024, 960, 512, 480, 768][(rng.next_u32() % 5) as usize];
        let spec: Vec<i32> = (0..1024).map(|_| rng.i32() >> (rng.next_u32() % 12)).collect();
        let scale: [i16; 8] = core::array::from_fn(|_| (rng.next_u32() % 20) as i16);
        let win = (rng.next_u32() % 4) as i32;
        let gain_in: i32 = if k % 9 == 0 { -1000 } else { (rng.next_u32() % 33) as i32 };

        let mut st = to_vec(&d);
        let mut chv = ch_full(&ch);
        let mut spec_r = spec.clone();
        let mut scale_r = scale;
        let mut g_r = gain_in;
        unsafe {
            c::cref_drc_apply(st.as_mut_ptr(), chv.as_mut_ptr(), spec_r.as_mut_ptr(), scale_r.as_mut_ptr(), win, &mut g_r, 0, frame)
        };

        let mut ci = Box::<CAacDecoderChannelInfo>::default();
        ci.spec_scale = scale;
        ci.ics_info.window_sequence = win as BlockType;
        let mut spec_p = spec.clone();
        let mut g_p = gain_in;
        aac_decoder_drc_apply(
            &mut d,
            None,
            &mut ci,
            &mut ch,
            if gain_in == -1000 { None } else { Some(&mut g_p) },
            0,
            frame,
            0,
            &mut spec_p,
        );
        assert_eq!(st, to_vec(&d), "state {k}");
        assert_eq!(chv, ch_full(&ch), "channel data {k}");
        assert_eq!(spec_r, spec_p, "spectrum {k}");
        assert_eq!(scale_r, ci.spec_scale, "spec scale {k}");
        assert_eq!(g_r, g_p, "ext gain {k}");
    }
}

#[test]
fn apply_level_normalization() {
    let mut rng = Rng::new(6);
    for k in 0..6000 {
        let d = random_state(&mut rng);
        let channels = 1 + rng.next_u32() % 8;
        let n = [1024u32, 960, 512, 480, 2048, 64][(rng.next_u32() % 6) as usize];
        let samples: Vec<i32> = (0..(channels * n) as usize).map(|_| rng.i32() >> (rng.next_u32() % 10)).collect();
        let gain = match k % 4 {
            0 => AACDEC_DRC_GAIN_INIT_VALUE,
            1 => rng.i32() >> (rng.next_u32() % 16),
            2 => (rng.next_u32() >> 1) as i32,
            _ => rng.i32(),
        };
        let gain_scale = [0, 3, 8, 11, 19, 31][(rng.next_u32() % 6) as usize];
        let delay = [0, 1, n / 2, n][(rng.next_u32() % 4) as usize];
        let limiter = (rng.next_u32() % 2) as u32;
        let stride = if limiter != 0 || k % 3 == 0 { 1 } else { n };

        let mut st = to_vec(&d);
        let mut sr = samples.clone();
        let mut gr = vec![0i32; n as usize];
        let rr = unsafe {
            c::cref_drc_apply_level_normalization(st.as_mut_ptr(), sr.as_mut_ptr(), gain, gr.as_mut_ptr(), gain_scale, delay, n, channels, stride, limiter)
        };
        let mut p = d;
        let mut sp = samples.clone();
        let mut gp = vec![0i32; n as usize];
        let rp = apply_drc_level_normalization(
            &mut p,
            &mut sp,
            &[gain],
            if limiter != 0 { Some(&mut gp) } else { None },
            gain_scale,
            delay,
            n,
            channels,
            stride,
            limiter,
        );
        assert_eq!(rr, rp);
        assert_eq!(st, to_vec(&p), "state {k}");
        assert_eq!(sr, sp, "samples {k}");
        assert_eq!(gr, gp, "gains {k}");
    }
}
