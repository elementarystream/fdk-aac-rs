// Differential tests of fdk_aac_rs_sbrenc::sbr_encoder (the whole SBR encoder) against the C++
// reference.
//
// The SBR encoder of the library and of the port are driven as the AAC encoder drives them
// (aacenc_lib.cpp): sbrEncoder_Open(), sbrEncoder_Init() on the encoder's deinterleaved input
// buffer, then per frame new input samples behind the input buffer offset,
// sbrEncoder_EncodeFrame() and sbrEncoder_UpdateBuffers(). The configurations are every entry
// of the SBR tuning table (every core coder, sample rate, channel count and bitrate range):
// HE-AAC dual-rate and downsampled SBR with core frames of 1024 and 960, HE-AACv2 (PS) for the
// mono entries, ELD with SBR dual-rate and single-rate with core frames of 512 and 480 (with
// and without AC_LD_MPS), and 5.1 (SCE, two CPEs, LFE). The input is the corpus (music and
// synthetic clips) and, for some configurations, noise, silence and full-scale signals. After
// the initialization and after every frame, the return values, the SBR payloads and their bit
// counts, the whole input buffer (with the downsampled or downmixed core input) and the
// complete encoder state (every element and channel, the QMF filter states, the dynamic RAM
// and the PS encoder) are compared; the delay and header getters, sbrEncoder_SendHeader(),
// sbrEncoder_SAPPrepare() and sbrEncoder_GetHeader() are mixed in.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::sbrenc_sbr_encoder as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_sbrenc::sbr::SbrEncoder;
use fdk_aac_rs_sbrenc::sbr_encoder::*;
use fdk_aac_rs_sbrenc::sbrenc_rom::SBR_TUNING_TABLE;

use super::sbr_encoder_dump::{assert_dump_eq, Dump};
use super::sbr_encoder_pcm::Pcm;
use super::sbr_encoder_state::sbr_encoder as dump_encoder;

type H = *mut core::ffi::c_void;

const BUF: usize = 8192;

/// The largest high resolution band count of the tested configurations (calculateSbrEnvelope()
/// has arrays of 32 bands in C).
static MAX_NSFB_HI: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

fn c_dump(h: H) -> Vec<i32> {
    let mut buf = vec![0i32; 1 << 20];
    let n = unsafe { c::cref_sbr_encoder_dump(h, buf.as_mut_ptr(), buf.len() as i32) };
    assert!((n as usize) <= buf.len());
    buf.truncate(n as usize);
    buf
}

fn r_dump(e: &SbrEncoder) -> Vec<i32> {
    let mut d = Dump::default();
    dump_encoder(&mut d, e);
    d.0
}

fn el_flat(el: &[SbrElementInfo; 8]) -> Vec<i32> {
    el.iter()
        .flat_map(|e| {
            [
                e.el_type,
                e.bit_rate,
                e.instance_tag,
                e.f_parametric_stereo as i32,
                e.f_dual_mono as i32,
                e.n_channels_in_el as i32,
                e.channel_index[0] as i32,
                e.channel_index[1] as i32,
            ]
        })
        .collect()
}

fn info_r(e: &SbrEncoder) -> [i32; 6] {
    [
        sbr_encoder_get_header_delay(Some(e)),
        sbr_encoder_get_bs_delay(Some(e)),
        sbr_encoder_get_estimate_bitrate(Some(e)),
        sbr_encoder_get_input_data_delay(Some(e)),
        sbr_encoder_get_sbr_dec_delay(Some(e)),
        sbr_encoder_contains_header(Some(e)),
    ]
}

/// One encoder configuration.
#[derive(Clone, Debug)]
struct Config {
    aot: AudioObjectType,
    in_rate: i32,
    dsf: u32,
    frame_length: i32,
    /// the elements: (type, bitrate, channels, channel indices)
    elements: Vec<(Mp4ElementId, i32, u8, [u8; 2])>,
    in_channels: usize,
    syntax_flags: u32,
    header_period: i32,
    delay: i32,
    /// 0: corpus, 1: noise, 2: silence then corpus, 3: full scale
    signal: i32,
}

/// The input samples of channel `ch` for frame `frame` (`n` samples).
fn input(cfg: &Config, clips: &[Pcm], multi: &Pcm, rng: &mut Rng, k: usize, ch: usize, frame: usize, n: usize) -> Vec<i16> {
    match cfg.signal {
        1 => (0..n).map(|_| rng.i16() >> (ch as i32 * 3 % 12)).collect(),
        2 if frame < 2 => vec![0; n],
        3 => (0..n).map(|i| if (i / 37 + ch) % 2 == 0 { i16::MAX } else { i16::MIN }).collect(),
        _ => {
            if cfg.in_channels > 2 {
                multi.channel_frame(ch, 48000 * 2 + frame * n, n)
            } else {
                let clip = &clips[k % clips.len()];
                clip.channel_frame(ch, 48000 * 3 + (k * 7919) % 48000 + frame * n, n)
            }
        }
    }
}

fn run_config(k: usize, cfg: &Config, clips: &[Pcm], multi: &Pcm, frames: usize) -> bool {
    let mut rng = Rng::new(0x5b2e_0000 + k as u64);
    let n_el = cfg.elements.len() as i32;
    let n_ch_sbr = cfg.elements.iter().filter(|e| e.0 != ID_LFE).map(|e| e.2 as i32).sum::<i32>();
    let ps = (cfg.aot == AOT_PS) as i32;

    let hc = unsafe { c::cref_sbr_encoder_open(n_el, n_ch_sbr, ps) };
    let mut hr: Option<Box<SbrEncoder>> = None;
    let rr = sbr_encoder_open(Some(&mut hr), n_el, n_ch_sbr, ps);
    assert_eq!(hc.is_null(), rr != 0, "{cfg:?} open");
    if hc.is_null() {
        return false;
    }
    let er = hr.as_deref_mut().unwrap();

    let mut el = [SbrElementInfo::default(); 8];
    for (i, e) in cfg.elements.iter().enumerate() {
        el[i] = SbrElementInfo {
            el_type: e.0,
            bit_rate: e.1,
            instance_tag: i as i32,
            f_parametric_stereo: 0,
            f_dual_mono: 0,
            n_channels_in_el: e.2,
            channel_index: e.3,
        };
    }
    let mut el_c = el_flat(&el);

    let n_in = cfg.in_channels;
    let mut buf_c = vec![0i16; BUF * n_in.max(2)];
    let mut buf_r = buf_c.clone();

    // coreBandwidth, inputBufferOffset, numChannels, coreSampleRate, downSampleFactor,
    // frameLength, delay
    let mut io_c = [20000, 0, n_in as i32, cfg.in_rate, cfg.dsf as i32, cfg.frame_length, cfg.delay];
    let tf = if cfg.aot == AOT_ER_AAC_ELD { 1 } else { 8 };
    let ret_c = unsafe {
        c::cref_sbr_encoder_init(
            hc,
            el_c.as_mut_ptr(),
            n_el,
            buf_c.as_mut_ptr(),
            BUF as u32,
            io_c.as_mut_ptr(),
            cfg.syntax_flags,
            cfg.aot,
            tf,
            cfg.header_period,
            1,
        )
    };
    let (mut bw, mut off, mut nch, mut sr, mut dsf, mut fl, mut delay) =
        (20000, 0, n_in as i32, cfg.in_rate, cfg.dsf, cfg.frame_length, cfg.delay);
    let ret_r = sbr_encoder_init(
        er,
        &mut el,
        n_el,
        &mut buf_r,
        BUF as u32,
        &mut bw,
        &mut off,
        &mut nch,
        cfg.syntax_flags,
        &mut sr,
        &mut dsf,
        &mut fl,
        cfg.aot,
        &mut delay,
        tf,
        cfg.header_period,
        1,
    );
    assert_eq!(ret_c, ret_r, "{cfg:?} init");
    assert_eq!(io_c, [bw, off, nch, sr, dsf as i32, fl, delay], "{cfg:?} init outputs");
    assert_eq!(el_c, el_flat(&el), "{cfg:?} element info");
    assert_eq!(buf_c, buf_r, "{cfg:?} init buffer");
    assert_dump_eq(&c_dump(hc), &r_dump(er), &format!("{cfg:?} init state"));
    if ret_r != 0 {
        unsafe { c::cref_sbr_encoder_close(hc) };
        sbr_encoder_close(&mut hr);
        return false;
    }

    // the SBR headers of the elements (the ASC)
    for e in 0..er.no_elements {
        let send = (k % 5 != 0) as i32;
        let mut hb = vec![0u8; 256];
        let nc = unsafe { c::cref_sbr_encoder_get_header(hc, e, send, hb.as_mut_ptr()) };
        let rb = BitMem::new(256);
        let mut bs = FdkBitStream::default();
        fdk_init_bit_stream(&mut bs, &rb, 256, 0, BS_WRITER);
        sbr_encoder_get_header(er, &mut bs, e, send);
        fdk_sync_cache(&mut bs);
        assert_eq!(nc, fdk_get_valid_bits(&mut bs) as i32, "{cfg:?} header");
        assert_eq!(hb, rb.to_vec(), "{cfg:?} header bits");
    }

    for e in er.sbr_element.iter().flatten() {
        MAX_NSFB_HI.fetch_max(e.sbr_config_data.n_sfb[1], std::sync::atomic::Ordering::Relaxed);
    }
    let frame_size = er.frame_size.max(0) as usize;
    let in_off = (off / nch.max(1)).max(0) as usize;
    for frame in 0..frames {
        for ch in 0..n_in {
            let x = input(cfg, clips, multi, &mut rng, k, ch, frame, frame_size);
            let p = ch * BUF + in_off;
            buf_c[p..p + frame_size].copy_from_slice(&x);
            buf_r[p..p + frame_size].copy_from_slice(&x);
        }
        if frame % 4 == 3 {
            let a = unsafe { c::cref_sbr_encoder_send_header(hc) };
            assert_eq!(a, sbr_encoder_send_header(Some(&mut *er)));
        }
        if frame % 5 == 4 {
            let a = unsafe { c::cref_sbr_encoder_sap_prepare(hc) };
            assert_eq!(a, sbr_encoder_sap_prepare(Some(&mut *er)));
        }

        let mut bits_c = [0u32; 8];
        let mut data_c = [[0u8; MAX_PAYLOAD_SIZE]; 8];
        let rc = unsafe {
            c::cref_sbr_encoder_encode(hc, buf_c.as_mut_ptr(), BUF as u32, bits_c.as_mut_ptr(), data_c.as_mut_ptr() as *mut u8)
        };
        let mut bits_r = [0u32; 8];
        let mut data_r = [[0u8; MAX_PAYLOAD_SIZE]; 8];
        let rr = sbr_encoder_encode_frame(er, &mut buf_r, BUF as u32, &mut bits_r, &mut data_r);
        assert_eq!(rc, rr, "{cfg:?} frame {frame} encode");
        assert_eq!(bits_c, bits_r, "{cfg:?} frame {frame} payload bits");
        assert_eq!(data_c, data_r, "{cfg:?} frame {frame} payload");
        assert!(buf_c == buf_r, "{cfg:?} frame {frame} core input");
        assert_dump_eq(&c_dump(hc), &r_dump(er), &format!("{cfg:?} frame {frame} state"));
        let mut info_c = [0i32; 6];
        unsafe { c::cref_sbr_encoder_info(hc, info_c.as_mut_ptr()) };
        assert_eq!(info_c, info_r(er), "{cfg:?} frame {frame} getters");

        let uc = unsafe { c::cref_sbr_encoder_update(hc, buf_c.as_mut_ptr(), BUF as u32) };
        let ur = sbr_encoder_update_buffers(er, &mut buf_r, BUF as u32);
        assert_eq!(uc, ur);
        assert!(buf_c == buf_r, "{cfg:?} frame {frame} buffer update");
    }
    let fin = c_dump(hc);
    assert!(fin.len() > 9088, "{cfg:?} dump size {}", fin.len());
    assert_dump_eq(&fin, &r_dump(er), &format!("{cfg:?} final state"));

    unsafe { c::cref_sbr_encoder_close(hc) };
    sbr_encoder_close(&mut hr);
    assert!(hr.is_none());
    true
}

/// The configurations of every tuning table entry.
fn configs() -> Vec<Config> {
    let mut v = Vec::new();
    for (i, t) in SBR_TUNING_TABLE.iter().enumerate() {
        let br = (t.bitrate_from + (t.bitrate_to.min(t.bitrate_from + 64000) - t.bitrate_from) / 2) as i32;
        let eld = t.core_coder == CODEC_AACLD;
        let sr = t.sample_rate as i32;
        let (el_type, idx) = if t.num_channels == 1 { (ID_SCE, [0, 0]) } else { (ID_CPE, [0, 1]) };
        let base = Config {
            aot: if eld { AOT_ER_AAC_ELD } else { AOT_SBR },
            in_rate: 2 * sr,
            dsf: 2,
            frame_length: if eld { [512, 480][i % 2] } else { [1024, 960][i % 2] },
            elements: vec![(el_type, br, t.num_channels, idx)],
            in_channels: t.num_channels as usize,
            syntax_flags: if eld && i % 3 == 0 { AC_LD_MPS } else { 0 },
            header_period: if i % 4 == 0 { -1 } else { (i % 10) as i32 + 1 },
            delay: if eld { [0, 256, 1000][i % 3] } else { [0, 1600, 2600][i % 3] },
            signal: if i % 7 == 3 { (i % 4) as i32 } else { 0 },
        };
        if base.in_rate <= 96000 {
            v.push(base.clone());
        }
        // downsampled (single-rate) SBR
        v.push(Config { in_rate: sr, dsf: 1, ..base.clone() });
        // PS on the mono tunings of AAC
        if !eld && t.num_channels == 1 && base.in_rate <= 96000 {
            v.push(Config {
                aot: AOT_PS,
                elements: vec![(ID_CPE, br, 2, [0, 1])],
                in_channels: 2,
                frame_length: 1024,
                ..base.clone()
            });
        }
    }
    // 5.1: C, L/R, Ls/Rs, LFE
    for (rate, br1, br2) in [(48000, 24000, 48000), (44100, 20000, 40000), (32000, 18000, 36000)] {
        for dsf in [2u32, 1] {
            let core = if dsf == 2 { rate / 2 } else { rate };
            v.push(Config {
                aot: AOT_SBR,
                in_rate: rate,
                dsf,
                frame_length: 1024,
                elements: vec![
                    (ID_SCE, br1, 1, [0, 0]),
                    (ID_CPE, br2, 2, [1, 2]),
                    (ID_CPE, br2, 2, [3, 4]),
                    (ID_LFE, 8000, 1, [5, 0]),
                ],
                in_channels: 6,
                syntax_flags: 0,
                header_period: -1,
                delay: 1600,
                signal: (core % 3) as i32,
            });
        }
    }
    v
}

#[test]
fn whole_encoder() {
    let clips = Pcm::all_stereo();
    let multi = Pcm::multichannel();
    let mut ok = 0;
    let mut ps = 0;
    let mut eld = 0;
    let mut multich = 0;
    let all = configs();
    for (k, cfg) in all.iter().enumerate() {
        let frames = if k % 10 == 0 { 60 } else if cfg.in_channels > 2 { 12 } else { 20 };
        if run_config(k, cfg, &clips, &multi, frames) {
            ok += 1;
            ps += (cfg.aot == AOT_PS) as i32;
            eld += (cfg.aot == AOT_ER_AAC_ELD) as i32;
            multich += (cfg.in_channels > 2) as i32;
        }
    }
    eprintln!("{} clips; {} configurations, {ok} encoded ({ps} PS, {eld} ELD, {multich} 5.1)", clips.len(), all.len());
    let nsfb = MAX_NSFB_HI.load(std::sync::atomic::Ordering::Relaxed);
    eprintln!("largest nSfb[HI]: {nsfb}");
    assert!(nsfb <= 32);
    assert!(ok > 150 && ps > 10 && eld > 10 && multich > 2);
}

#[test]
fn api_helpers() {
    for ch in 0..9 {
        assert_eq!(unsafe { c::cref_sbr_encoder_in_buffer_size(ch) }, sbr_encoder_get_in_buffer_size(ch));
    }
    for aot in [AOT_SBR, AOT_PS, AOT_ER_AAC_ELD, AOT_AAC_LC, AOT_ER_AAC_LD] {
        assert_eq!(unsafe { c::cref_sbr_encoder_single_rate_possible(aot) }, sbr_encoder_is_single_rate_possible(aot));
        for nch in 1..=2 {
            for sr in [8000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 96000] {
                for br in (0..300_000u32).step_by(997).chain([0, 1, 4_999_999, 5_000_000, 6_000_000, u32::MAX]) {
                    let a = unsafe { c::cref_sbr_encoder_limit_bit_rate(br, nch, sr, aot) };
                    assert_eq!(a, sbr_encoder_limit_bit_rate(br, nch, sr, aot), "{br} {nch} {sr} {aot}");
                }
            }
        }
    }
    let mut null_c = [0i32; 8];
    unsafe { c::cref_sbr_encoder_info_null(null_c.as_mut_ptr()) };
    assert_eq!(
        null_c,
        [
            sbr_encoder_get_header_delay(None),
            sbr_encoder_get_bs_delay(None),
            sbr_encoder_get_estimate_bitrate(None),
            sbr_encoder_get_input_data_delay(None),
            sbr_encoder_get_sbr_dec_delay(None),
            sbr_encoder_contains_header(None),
            sbr_encoder_send_header(None),
            sbr_encoder_sap_prepare(None),
        ]
    );
    // open with the most and the fewest elements and channels (inputs without a defined
    // reference result are left out; the port fails for them, see sbrEncoder_Open())
    for (ne, nc) in [(8, 8), (0, 0), (1, 1), (4, 6)] {
        let hc = unsafe { c::cref_sbr_encoder_open(ne, nc, 1) };
        let mut hr = None;
        let rr = sbr_encoder_open(Some(&mut hr), ne, nc, 1);
        assert_eq!(hc.is_null(), rr != 0, "open {ne} {nc}");
        if !hc.is_null() {
            unsafe { c::cref_sbr_encoder_close(hc) };
        }
        sbr_encoder_close(&mut hr);
    }
}

#[test]
fn lib_info() {
    for used in [0, 1, 5, 38, 39] {
        for null in [0, 1] {
            let mut out = [0i32; 5];
            let mut s = [0u8; 64];
            unsafe { c::cref_sbr_encoder_lib_info(used, null, out.as_mut_ptr(), s.as_mut_ptr() as *mut _) };
            let mut info = [LibInfo::default(); FDK_MODULE_LAST as usize];
            fdk_init_lib_info(&mut info);
            for (i, e) in info.iter_mut().enumerate().take(used as usize) {
                e.module_id = i as i32 + 1;
            }
            let r = sbr_encoder_get_lib_info(if null != 0 { None } else { Some(&mut info) });
            assert_eq!(out[0], r, "used {used} null {null}");
            let idx = info.iter().rposition(|e| e.module_id == FDK_SBRENC).map(|i| i as i32).unwrap_or(-1);
            assert_eq!(out[1], idx);
            // C does not set the other entries (FDKinitLibInfo() sets module_id only)
            if idx >= 0 && r == 0 {
                let e = &info[idx as usize];
                assert_eq!([out[2], out[3], out[4]], [e.module_id, e.version, e.flags as i32]);
                // C does not set the bytes behind the terminating NUL
                let n = s.iter().position(|&b| b == 0).unwrap() + 1;
                assert_eq!(&s[..n], &e.version_str[..n]);
                let t = std::ffi::CStr::from_bytes_until_nul(&s[32..]).unwrap().to_str().unwrap();
                assert_eq!(t, e.title);
            }
        }
    }
}

/// `sbrEncoder_EncodeFrame()` and `sbrEncoder_UpdateBuffers()` on an encoder with 0 channels.
/// x86 division by zero: the port gives the arm64 result on every variant (nothing changes, both
/// return 0); it is compared with the reference on arm64.
#[test]
fn zero_channels() {
    let mut rng = Rng::new(0x5b2e_00c0);
    let cases: [(i32, i32, i32, Option<(AudioObjectType, i32, i32)>); 6] = [
        (1, 1, 0, None),
        (2, 4, 1, None),
        (0, 0, 0, None),
        // (aot, input sample rate, bitrate): no SBR tuning for these bitrates
        (1, 1, 0, Some((AOT_SBR, 44100, 1000))),
        (1, 2, 1, Some((AOT_PS, 48000, 100))),
        (1, 1, 0, Some((AOT_ER_AAC_ELD, 48000, 1000))),
    ];
    for (k, &(ne, nc, ps, init)) in cases.iter().enumerate() {
        let hc = unsafe { c::cref_sbr_encoder_open(ne, nc, ps) };
        let mut hr: Option<Box<SbrEncoder>> = None;
        assert_eq!(sbr_encoder_open(Some(&mut hr), ne, nc, ps), 0, "case {k} open");
        assert!(!hc.is_null(), "case {k} open");
        let er = hr.as_deref_mut().unwrap();
        let mut buf_c: Vec<i16> = (0..BUF * 2).map(|_| rng.i16()).collect();
        let mut buf_r = buf_c.clone();
        if let Some((aot, rate, br)) = init {
            let n = if aot == AOT_PS { 2 } else { 1 };
            let mut el = [SbrElementInfo::default(); 8];
            el[0] = SbrElementInfo {
                el_type: if n == 2 { ID_CPE } else { ID_SCE },
                bit_rate: br,
                n_channels_in_el: n as u8,
                channel_index: [0, 1],
                ..Default::default()
            };
            let mut el_c = el_flat(&el);
            let fl = if aot == AOT_ER_AAC_ELD { 512 } else { 1024 };
            let tf = if aot == AOT_ER_AAC_ELD { 1 } else { 8 };
            let mut io_c = [20000, 0, n, rate, 2, fl, 0];
            let ret_c = unsafe {
                c::cref_sbr_encoder_init(hc, el_c.as_mut_ptr(), 1, buf_c.as_mut_ptr(), BUF as u32, io_c.as_mut_ptr(), 0, aot, tf, -1, 1)
            };
            let (mut bw, mut off, mut nch, mut sr, mut dsf, mut fl_r, mut delay) = (20000, 0, n, rate, 2u32, fl, 0);
            let ret_r = sbr_encoder_init(
                er, &mut el, 1, &mut buf_r, BUF as u32, &mut bw, &mut off, &mut nch, 0, &mut sr, &mut dsf, &mut fl_r, aot,
                &mut delay, tf, -1, 1,
            );
            assert_eq!(ret_c, ret_r, "case {k} init");
            assert_ne!(ret_r, 0, "case {k}: the init must fail");
            assert_eq!(io_c, [bw, off, nch, sr, dsf as i32, fl_r, delay], "case {k} init outputs");
            assert_eq!(el_c, el_flat(&el), "case {k} element info");
        }
        assert_eq!(er.n_channels, 0, "case {k}");
        let before_r = r_dump(er);
        let before_buf = buf_r.clone();
        assert!(buf_c == buf_r, "case {k}: buffer");
        assert_dump_eq(&c_dump(hc), &before_r, &format!("case {k} state"));
        for frame in 0..3 {
            let what = format!("case {k} frame {frame}");
            let mut bits_r = [0u32; 8];
            let mut data_r = [[0u8; MAX_PAYLOAD_SIZE]; 8];
            let rr = sbr_encoder_encode_frame(er, &mut buf_r, BUF as u32, &mut bits_r, &mut data_r);
            let ur = sbr_encoder_update_buffers(er, &mut buf_r, BUF as u32);
            if cfg!(target_arch = "aarch64") {
                let mut bits_c = [0u32; 8];
                let mut data_c = [[0u8; MAX_PAYLOAD_SIZE]; 8];
                let rc = unsafe {
                    c::cref_sbr_encoder_encode(hc, buf_c.as_mut_ptr(), BUF as u32, bits_c.as_mut_ptr(), data_c.as_mut_ptr() as *mut u8)
                };
                let uc = unsafe { c::cref_sbr_encoder_update(hc, buf_c.as_mut_ptr(), BUF as u32) };
                assert_eq!((rc, uc), (rr, ur), "{what}: return values");
                assert_eq!(bits_c, bits_r, "{what}: payload bits");
                assert_eq!(data_c, data_r, "{what}: payload");
                assert!(buf_c == buf_r, "{what}: buffer");
                assert_dump_eq(&c_dump(hc), &r_dump(er), &format!("{what} state"));
            }
            // the arm64 result (compared above): nothing is processed
            assert_eq!((rr, ur), (0, 0), "{what}");
            assert_eq!(bits_r, [0; 8], "{what}");
            assert!(buf_r == before_buf, "{what}: buffer");
            assert!(r_dump(er) == before_r, "{what}: state");
        }
        unsafe { c::cref_sbr_encoder_close(hc) };
        sbr_encoder_close(&mut hr);
    }
}
