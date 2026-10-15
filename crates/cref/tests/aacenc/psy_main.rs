// Differential tests of fdk_aac_rs_aacenc::psy_main against the C++ reference.
//
// The reference psychoacoustic model is set up and run as the encoder does it (shim
// aacenc_psy_main.cpp: FDKaacEnc_PsyNew/PsyOutNew/QCOutNew, channel mapping, bandwidth,
// FDKaacEnc_psyInit, FDKaacEnc_psyMainInit, and per frame FDKaacEnc_psyMain for every element);
// the port does the same with its functions. Many consecutive frames of real and synthetic
// PCM are encoded for LC, LD and ELD (all granule lengths), mono, stereo, 5.1 and 7.1, CBR and
// VBR bandwidths, with TNS/PNS/IS/MS on and off, including re-initializations with and without
// state reset. After every frame the complete state is compared: every PSY_STATIC (block
// switching, input buffer, overlap, pre-echo state, MDCT state), PSY_DYNAMIC, every
// PSY_OUT_CHANNEL and PSY_OUT_ELEMENT and every QC_OUT_CHANNEL.
//
// Exception: the spectral lines of grouped short blocks without a defined value in the
// reference (FDKaacEnc_groupShortData()) are 0 in the port and excluded from the comparison of
// QC_OUT_CHANNEL::mdctSpectrum.

use crate::block_switch::{corpus, synthetic_signals};
use fdk_aac_rs_aacenc::aacenc::*;
use fdk_aac_rs_aacenc::aacenc_pns::PnsData;
use fdk_aac_rs_aacenc::aacenc_tns::TnsData;
use fdk_aac_rs_aacenc::bandwidth::fdk_aac_enc_determine_band_width;
use fdk_aac_rs_aacenc::block_switch::BlockSwitchingControl;
use fdk_aac_rs_aacenc::channel_map::fdk_aac_enc_init_channel_mapping;
use fdk_aac_rs_aacenc::interface::*;
use fdk_aac_rs_aacenc::psy_configuration::PsyConfiguration;
use fdk_aac_rs_aacenc::psy_const::*;
use fdk_aac_rs_aacenc::psy_data::PsyData;
use fdk_aac_rs_aacenc::psy_main::*;
use fdk_aac_rs_aacenc::qc_data::*;
use fdk_aac_rs_cref::aacenc_psy_main as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;
use std::ffi::c_void;
use std::mem::MaybeUninit;

/// Window slopes (length, shape) that identify `prev_wrs`.
const SLOPES: &[(i32, i32)] = &[
    (1024, 0),
    (1024, 1),
    (960, 0),
    (960, 1),
    (512, 0),
    (512, 1),
    (480, 0),
    (480, 1),
    (128, 0),
    (128, 1),
    (120, 0),
    (120, 1),
    (64, 0),
    (60, 0),
    (256, 0),
    (240, 0),
];

/// A C struct copied into the port's type with the same layout.
fn c_struct<T: Copy>(f: impl FnOnce(*mut c_void)) -> T {
    let mut m = MaybeUninit::<T>::zeroed();
    f(m.as_mut_ptr() as *mut c_void);
    // SAFETY: T consists of integers; the C function wrote size_of::<T>() bytes.
    unsafe { m.assume_init() }
}

/// An encoder configuration of the psychoacoustic model.
#[derive(Clone, Copy, Debug)]
struct Config {
    aot: AudioObjectType,
    frame_length: i32,
    sample_rate: i32,
    mode: ChannelMode,
    order: ChannelOrder,
    bitrate: i32,
    bitrate_mode: AacencBitrateMode,
    use_tns: i32,
    use_pns: i32,
    use_is: i32,
    use_ms: i32,
    syntax_flags: u32,
    bandwidth: i32,
}

fn n_channels(mode: ChannelMode) -> i32 {
    match mode {
        MODE_1 => 1,
        MODE_2 => 2,
        MODE_1_2 => 3,
        MODE_1_2_1 => 4,
        MODE_1_2_2 => 5,
        MODE_1_2_2_1 => 6,
        MODE_6_1 => 7,
        _ => 8,
    }
}

fn n_elements(mode: ChannelMode) -> i32 {
    match mode {
        MODE_1 | MODE_2 => 1,
        MODE_1_2 => 2,
        MODE_1_2_1 | MODE_1_2_2 => 3,
        MODE_1_2_2_1 => 4,
        _ => 5,
    }
}

thread_local! {
    /// Frames, msDigest NONE/SOME/ALL, short channel frames, TNS channel frames, PNS bands, IS
    /// bands.
    static COVERAGE: std::cell::RefCell<[u64; 8]> = const { std::cell::RefCell::new([0; 8]) };
}

/// The C and the Rust psychoacoustic model.
struct Harness {
    c: *mut c_void,
    psy: Option<Box<PsyInternal>>,
    psy_out: [Option<Box<PsyOut>>; 1],
    qc: QcOut,
    cm: ChannelMapping,
    n_ch: usize,
    max_ch: usize,
    frame: usize,
    what: String,
    /// FDKaacEnc_psyMain of the reference: NULL for the library's, or the copy of
    /// cref_psy_main_amplified_fn()
    c_fn: *mut c_void,
}

impl Drop for Harness {
    fn drop(&mut self) {
        unsafe { c::cref_psy_main_close(self.c) };
    }
}

impl Harness {
    fn new(n_el: i32, n_ch: i32) -> Self {
        let c = unsafe { c::cref_psy_main_open(n_el, n_ch) };
        assert!(!c.is_null());
        let mut psy = None;
        assert_eq!(fdk_aac_enc_psy_new(&mut psy, n_el, n_ch), AAC_ENC_OK);
        let mut psy_out = [None];
        assert_eq!(fdk_aac_enc_psy_out_new(&mut psy_out, n_el, n_ch, 1), AAC_ENC_OK);
        let qc = QcOut {
            qc_element: vec![QcOutElement::default(); n_el as usize],
            p_qc_out_channels: vec![QcOutChannel::default(); n_ch as usize],
            ..Default::default()
        };
        Harness {
            c,
            psy,
            psy_out,
            qc,
            cm: ChannelMapping::default(),
            n_ch: n_ch as usize,
            max_ch: n_ch as usize,
            frame: 0,
            what: String::new(),
            c_fn: std::ptr::null_mut(),
        }
    }

    /// `FDKaacEnc_Initialize()`'s part of the psychoacoustic model on both; returns the error
    /// (equal on both) and the bandwidth.
    fn init(&mut self, cfg: &Config, init_flags: u32, do_psy_init: bool) -> (i32, i32) {
        self.what = format!("{cfg:?} initFlags {init_flags} psyInit {do_psy_init}");
        let mut bw_c = cfg.bandwidth;
        let err_c = unsafe {
            c::cref_psy_main_init(
                self.c,
                cfg.aot,
                cfg.mode,
                cfg.order,
                cfg.sample_rate,
                cfg.frame_length,
                cfg.bitrate,
                cfg.bitrate_mode,
                cfg.use_tns,
                cfg.use_pns,
                cfg.use_is,
                cfg.use_ms,
                cfg.syntax_flags,
                init_flags,
                do_psy_init as i32,
                &mut bw_c,
            )
        };
        let err_r = (|| {
            let e = fdk_aac_enc_init_channel_mapping(cfg.mode, cfg.order, &mut self.cm);
            if e != AAC_ENC_OK {
                return (e, cfg.bandwidth);
            }
            let mut bw = cfg.bandwidth;
            let e = fdk_aac_enc_determine_band_width(
                bw,
                cfg.bitrate,
                cfg.bitrate_mode,
                cfg.sample_rate,
                cfg.frame_length,
                &self.cm,
                cfg.mode,
                &mut bw,
            );
            if e != AAC_ENC_OK {
                return (e, cfg.bandwidth);
            }
            let psy = self.psy.as_deref_mut().unwrap();
            if do_psy_init {
                let e = fdk_aac_enc_psy_init(
                    psy,
                    &mut self.psy_out,
                    1,
                    self.max_ch as i32,
                    cfg.aot,
                    &self.cm,
                );
                if e != AAC_ENC_OK {
                    return (e, bw);
                }
            }
            let e = fdk_aac_enc_psy_main_init(
                psy,
                cfg.aot,
                &self.cm,
                cfg.sample_rate,
                cfg.frame_length,
                cfg.bitrate,
                if cfg.use_tns != 0 { 0xf } else { 0 },
                bw,
                cfg.use_pns,
                cfg.use_is,
                cfg.use_ms,
                cfg.syntax_flags,
                init_flags,
            );
            if e != AAC_ENC_OK {
                return (e, bw);
            }
            // FDKaacEnc_QCOutInit
            let mut ch_inc = 0;
            for i in 0..self.cm.n_elements as usize {
                for ch in 0..self.cm.el_info[i].n_channels_in_el as usize {
                    self.qc.qc_element[i].qc_out_channel[ch] = ch_inc;
                    ch_inc += 1;
                }
            }
            (AAC_ENC_OK, bw)
        })();
        assert_eq!(err_r, (err_c, bw_c), "{}: init", self.what);
        if err_c == AAC_ENC_OK {
            let cm_c: ChannelMapping = c_struct(|p| unsafe { c::cref_psy_main_get_cm(self.c, p) });
            assert_eq!(self.cm, cm_c, "{}: channel mapping", self.what);
            let psy = self.psy.as_deref().unwrap();
            for i in 0..2 {
                let mut g = 0;
                let conf: PsyConfiguration =
                    c_struct(|p| unsafe { c::cref_psy_main_get_conf(self.c, i, p, &mut g) });
                assert!(conf == psy.psy_conf[i as usize], "{}: psyConf[{i}]", self.what);
                assert_eq!(g, psy.granule_length);
            }
        }
        (err_c, bw_c)
    }

    /// One frame of planar input (`size` samples per channel) on both; compares everything.
    fn frame(&mut self, input: &[i16], size: u32) {
        let mut input_c = input.to_vec();
        let err_c =
            unsafe { c::cref_psy_main_frame_fn(self.c, input_c.as_mut_ptr(), size, self.c_fn) };
        let err_r = self.frame_r(input, size);
        let what = format!("{} frame {}", self.what, self.frame);
        assert_eq!(err_r, err_c, "{what}: error");
        self.compare(&what);
        self.frame += 1;
        // coverage
        let psy_out = self.psy_out[0].as_deref().unwrap();
        COVERAGE.with(|cov| {
            let mut cov = cov.borrow_mut();
            cov[0] += 1;
            for el in 0..self.cm.n_elements as usize {
                let e = &psy_out.psy_out_element[el];
                let n = self.cm.el_info[el].n_channels_in_el as usize;
                if n == 2 {
                    cov[1 + e.tools_info.ms_digest as usize] += 1;
                }
                for ch in 0..n {
                    let po = &psy_out.p_psy_out_channels[e.psy_out_channel[ch]];
                    // (not for the amplified spectra of `psy_main_negative_final_shift`, which
                    // 16-bit PCM does not produce: their saturated energies give positions of 0)
                    if self.c_fn.is_null() {
                        if let Err(err) = is_positions_bounded(&po.is_book, &po.is_scale) {
                            panic!("{what}: element {el} channel {ch}: {err}");
                        }
                    }
                    cov[4] += (po.last_window_sequence == SHORT_WINDOW) as u64;
                    cov[5] += (po.tns_info.num_of_filters.iter().any(|&f| f > 0)) as u64;
                    cov[6] += po.noise_nrg.iter().filter(|&&x| x != i32::MIN).count() as u64;
                    cov[7] += po.is_book.iter().filter(|&&x| x != 0).count() as u64;
                }
            }
        });
    }

    /// One frame on the port only (FDKaacEnc_psyMain for every audio element); returns the
    /// first error.
    fn frame_r(&mut self, input: &[i16], size: u32) -> AacEncoderError {
        let psy = self.psy.as_deref_mut().unwrap();
        let psy_out = self.psy_out[0].as_deref_mut().unwrap();
        let mut err_r = AAC_ENC_OK;
        for el in 0..self.cm.n_elements as usize {
            let el_info = self.cm.el_info[el];
            if el_info.el_type == ID_SCE || el_info.el_type == ID_CPE || el_info.el_type == ID_LFE
            {
                let n = el_info.n_channels_in_el as usize;
                let s0 = psy.psy_element[el].psy_static[0];
                let p0 = psy_out.psy_out_element[el].psy_out_channel[0];
                let q0 = self.qc.qc_element[el].qc_out_channel[0];
                let e = fdk_aac_enc_psy_main(
                    el_info.n_channels_in_el,
                    &mut psy.p_static_channels[s0..s0 + n],
                    &mut psy.psy_dynamic,
                    &mut psy.psy_conf,
                    &mut psy_out.psy_out_element[el],
                    &mut psy_out.p_psy_out_channels[p0..p0 + n],
                    &mut self.qc.p_qc_out_channels[q0..q0 + n],
                    input,
                    size,
                    &el_info.channel_index,
                    self.cm.n_channels,
                );
                if e != AAC_ENC_OK {
                    err_r = e;
                    break;
                }
            }
        }
        err_r
    }

    /// Sets the lowpass lines (long, LFE) of `psyConf[i]` on both.
    fn set_lowpass(&mut self, i: usize, line: i32, line_lfe: i32) {
        unsafe { c::cref_psy_main_set_lowpass(self.c, i as i32, line, line_lfe) };
        let conf = &mut self.psy.as_deref_mut().unwrap().psy_conf[i];
        conf.lowpass_line = line;
        conf.lowpass_line_lfe = line_lfe;
    }

    fn compare(&self, what: &str) {
        let psy = self.psy.as_deref().unwrap();
        let psy_out = self.psy_out[0].as_deref().unwrap();
        let lens: Vec<i32> = SLOPES.iter().map(|s| s.0).collect();
        let shapes: Vec<i32> = SLOPES.iter().map(|s| s.1).collect();

        // element mapping
        for el in 0..self.cm.n_elements as usize {
            let (mut st, mut po, mut qo) = ([0i32; 2], [0i32; 2], [0i32; 2]);
            unsafe {
                c::cref_psy_main_get_element(
                    self.c,
                    el as i32,
                    st.as_mut_ptr(),
                    po.as_mut_ptr(),
                    qo.as_mut_ptr(),
                )
            };
            for ch in 0..self.cm.el_info[el].n_channels_in_el as usize {
                assert_eq!(psy.psy_element[el].psy_static[ch] as i32, st[ch], "{what}: static");
                assert_eq!(psy_out.psy_out_element[el].psy_out_channel[ch] as i32, po[ch]);
                assert_eq!(self.qc.qc_element[el].qc_out_channel[ch] as i32, qo[ch]);
            }
            let mut cw = 0;
            let ti: ToolsInfo = c_struct(|p| unsafe {
                c::cref_psy_main_get_psy_out_element(self.c, el as i32, &mut cw, p)
            });
            let pe = &psy_out.psy_out_element[el];
            assert_eq!((pe.common_window, pe.tools_info), (cw, ti), "{what}: element {el}");
        }

        // statics
        for i in 0..self.max_ch {
            let s = &psy.p_static_channels[i];
            let mut inbuf = vec![0i16; MAX_INPUT_BUFFER_SIZE];
            let mut ola = vec![0i32; s.overlap_add_buffer.len()];
            let mut thr = vec![0i32; MAX_SFB];
            let mut misc = [0i32; 11];
            let bsc: BlockSwitchingControl = c_struct(|p| unsafe {
                c::cref_psy_main_get_static(
                    self.c,
                    i as i32,
                    p,
                    inbuf.as_mut_ptr(),
                    ola.as_mut_ptr(),
                    thr.as_mut_ptr(),
                    misc.as_mut_ptr(),
                    lens.as_ptr(),
                    shapes.as_ptr(),
                    SLOPES.len() as i32,
                )
            });
            assert_eq!(s.block_switching_control, bsc, "{what}: static {i} block switching");
            assert!(s.psy_input_buffer == inbuf, "{what}: static {i} input buffer");
            assert!(s.overlap_add_buffer[..] == ola[..], "{what}: static {i} overlap add");
            assert_eq!(s.sfb_thresholdnm1[..], thr[..], "{what}: static {i} sfbThresholdnm1");
            let m = &s.mdct_pers;
            let wrs = match m.prev_wrs {
                None => -1,
                Some(w) => SLOPES
                    .iter()
                    .position(|&(l, sh)| {
                        fdk_get_window_slope(l, sh).is_some_and(|x| x.as_ptr() == w.as_ptr())
                    })
                    .map_or(-2, |k| k as i32),
            };
            let misc_r = [
                s.mdct_scalenm1,
                s.calc_pre_echo,
                s.is_lfe,
                m.prev_tl,
                m.prev_nr,
                m.prev_fr,
                m.ov_offset,
                m.ov_size,
                m.prev_alias_symmetry,
                m.prev_prev_alias_symmetry,
                wrs,
            ];
            assert_eq!(misc_r, misc, "{what}: static {i} misc");
            assert!(wrs != -2);
        }

        // dynamic
        for ch in 0..2 {
            let mut tns = MaybeUninit::<TnsData>::zeroed();
            let mut pns = MaybeUninit::<PnsData>::zeroed();
            let pd: PsyData = c_struct(|p| unsafe {
                c::cref_psy_main_get_dynamic(
                    self.c,
                    ch as i32,
                    p,
                    tns.as_mut_ptr() as *mut c_void,
                    pns.as_mut_ptr() as *mut c_void,
                )
            });
            let (tns, pns) = unsafe { (tns.assume_init(), pns.assume_init()) };
            assert!(psy.psy_dynamic.psy_data[ch] == pd, "{what}: psyData[{ch}]");
            assert!(psy.psy_dynamic.tns_data[ch] == tns, "{what}: tnsData[{ch}]");
            assert!(psy.psy_dynamic.pns_data[ch] == pns, "{what}: pnsData[{ch}]");
        }

        // psy out and qc out channels
        for i in 0..self.n_ch {
            let po: PsyOutChannel = c_struct(|p| unsafe {
                c::cref_psy_main_get_psy_out_channel(self.c, i as i32, p)
            });
            let r = &psy_out.p_psy_out_channels[i];
            assert!(*r == po, "{what}: psyOutChannel[{i}]\n{r:?}\n{po:?}");
            let mut qc: QcOutChannel =
                c_struct(|p| unsafe { c::cref_psy_main_get_qc_out_channel(self.c, i as i32, p) });
            let mut rq = self.qc.p_qc_out_channels[i];
            // Lines of grouped short blocks without a defined value in C (0 in the port).
            if r.last_window_sequence == SHORT_WINDOW {
                let cnt = r.sfb_per_group as usize;
                let active = psy.psy_conf[1].sfb_active as usize;
                let groups = r.sfb_cnt as usize / cnt.max(1);
                for g in 0..groups {
                    let a = r.sfb_offsets[g * cnt + active] as usize;
                    let b = r.sfb_offsets[(g + 1) * cnt] as usize;
                    for k in a..b.min(1024) {
                        assert_eq!(rq.mdct_spectrum[k], 0);
                        qc.mdct_spectrum[k] = 0;
                    }
                }
            }
            for k in 0..1024 {
                assert_eq!(rq.mdct_spectrum[k], qc.mdct_spectrum[k], "{what}: qc {i} line {k}");
            }
            rq.mdct_spectrum = qc.mdct_spectrum;
            assert!(rq == qc, "{what}: qcOutChannel[{i}]");
        }
    }
}

/// The intensity positions of a channel (in the order of the coding in dyn_bits.cpp,
/// bitenc.cpp and line_pe.cpp) all have the same sign and lie in 1..=60, so that no delta
/// between them (or from 0 to the first) leaves -60..=60 (see intensity::is_positions).
fn is_positions_bounded(is_book: &[i32], is_scale: &[i32]) -> Result<(), String> {
    let pos: Vec<i32> =
        is_book.iter().zip(is_scale).filter(|(&b, _)| b != 0).map(|(_, &p)| p).collect();
    let ok = pos.iter().all(|&p| (1..=60).contains(&p)) || pos.iter().all(|&p| (-60..=-1).contains(&p));
    if ok {
        Ok(())
    } else {
        Err(format!("intensity positions {pos:?}"))
    }
}

/// `n` frames of `len` samples for `n_ch` channels from signal `sig` (0..: corpus files,
/// synthetic signals), planar with `size` samples per channel (`size >= len`).
fn signal(sig: usize, n_ch: usize, total: usize) -> Option<Vec<Vec<i16>>> {
    let files = [
        ("c51_48k.raw", 6, 48000 * 20),
        ("music_c_48k.raw", 2, 48000 * 40),
        ("music_f_44k.raw", 2, 44100 * 20),
        ("music_g_48k.raw", 2, 48000 * 30),
        ("music_b_mono_44k.raw", 1, 44100 * 5),
        ("syn_clicks.raw", 2, 0),
        ("syn_sweep.raw", 2, 0),
    ];
    let base: Vec<Vec<i16>> = if sig < files.len() {
        let (name, ch, start) = files[sig];
        corpus(name, ch, start, total)?
    } else {
        let s = synthetic_signals(total, sig as u64);
        s[(sig - files.len()) % s.len()].1.clone()
    };
    // Channel c: channel c of the source, or a delayed, attenuated copy of one of its channels.
    Some(
        (0..n_ch)
            .map(|c| {
                if c < base.len() {
                    base[c].clone()
                } else {
                    let src = &base[c % base.len()];
                    let d = 17 * c;
                    (0..total).map(|i| if i >= d { src[i - d] >> (c % 3) } else { 0 }).collect()
                }
            })
            .collect(),
    )
}

/// Runs `frames` frames of signal `sig` through the harness.
fn run(h: &mut Harness, cfg: &Config, sig: usize, frames: usize, r: &mut Rng) -> bool {
    let len = cfg.frame_length as usize;
    let n_ch = h.n_ch;
    let Some(s) = signal(sig, n_ch, len * frames) else { return false };
    let size = len + (r.next_u32() % 3) as usize * 7;
    for f in 0..frames {
        let mut planar = vec![0i16; size * n_ch];
        for ch in 0..n_ch {
            planar[ch * size..ch * size + len].copy_from_slice(&s[ch][f * len..(f + 1) * len]);
        }
        h.frame(&planar, size as u32);
    }
    true
}

fn configs(r: &mut Rng) -> Vec<Config> {
    let mut v = Vec::new();
    let mut push = |aot, frame_length, sample_rate, mode, bitrate, bitrate_mode, r: &mut Rng| {
        let syntax_flags = match aot {
            AOT_ER_AAC_LD => AC_ER | AC_LD,
            AOT_ER_AAC_ELD => AC_ER | AC_ELD | if r.next_u32() % 2 == 0 { AC_SBR_PRESENT } else { 0 },
            _ => if r.next_u32() % 4 == 0 { AC_SBR_PRESENT } else { 0 },
        };
        v.push(Config {
            aot,
            frame_length,
            sample_rate,
            mode,
            order: (r.next_u32() % 2) as i32,
            bitrate,
            bitrate_mode,
            use_tns: (r.next_u32() % 4 != 0) as i32,
            use_pns: (r.next_u32() % 3 != 0) as i32,
            use_is: (r.next_u32() % 3 != 0) as i32,
            use_ms: (r.next_u32() % 4 != 0) as i32,
            syntax_flags,
            bandwidth: [0, 0, 8000, 15000, 20000][(r.next_u32() % 5) as usize],
        });
    };
    for &(sr, br) in &[
        (8000, 16000),
        (16000, 24000),
        (22050, 32000),
        (24000, 48000),
        (32000, 64000),
        (44100, 64000),
        (44100, 128000),
        (48000, 96000),
        (48000, 256000),
        (96000, 192000),
    ] {
        for &mode in &[MODE_1, MODE_2] {
            let b = if mode == MODE_1 { br / 2 } else { br };
            push(AOT_AAC_LC, 1024, sr, mode, b, AACENC_BR_MODE_CBR, r);
        }
    }
    for &(mode, br) in &[(MODE_1_2_2_1, 320000), (MODE_1_2_2_1, 128000), (MODE_7_1_BACK, 448000)]
    {
        push(AOT_AAC_LC, 1024, 48000, mode, br, AACENC_BR_MODE_CBR, r);
    }
    for brm in 1..=5 {
        push(AOT_AAC_LC, 1024, 44100, MODE_2, 128000, brm, r);
    }
    for &fl in &[512, 480] {
        for &(sr, br) in &[(22050, 32000), (32000, 48000), (48000, 64000), (48000, 128000)] {
            push(AOT_ER_AAC_LD, fl, sr, MODE_2, br, AACENC_BR_MODE_CBR, r);
            push(AOT_ER_AAC_LD, fl, sr, MODE_1, br / 2, AACENC_BR_MODE_CBR, r);
        }
    }
    for &(fl, sr) in &[
        (512, 48000),
        (512, 32000),
        (480, 48000),
        (480, 22050),
        (256, 24000),
        (240, 24000),
        (256, 16000),
        (128, 12000),
        (120, 12000),
    ] {
        push(AOT_ER_AAC_ELD, fl, sr, MODE_2, 64000, AACENC_BR_MODE_CBR, r);
        push(AOT_ER_AAC_ELD, fl, sr, MODE_1, 24000, AACENC_BR_MODE_CBR, r);
    }
    push(AOT_ER_AAC_ELD, 512, 48000, MODE_1_2_2_1, 256000, AACENC_BR_MODE_CBR, r);
    v
}

#[test]
fn psy_main_sequences() {
    let mut r = Rng::new(0x5059_4d41);
    let mut ran = 0;
    for (k, cfg) in configs(&mut r).iter().enumerate() {
        let n_ch = n_channels(cfg.mode);
        let n_el = n_elements(cfg.mode);
        // maxChannels as aacenc_lib: the configured channels (sometimes more, to reach the
        // chInc logic of FDKaacEnc_psyInit)
        let max_ch = if k % 5 == 4 { 8 } else { n_ch };
        let mut h = Harness::new(n_el.max(5), max_ch);
        h.n_ch = n_ch as usize;
        let (err, _) = h.init(cfg, 1, true);
        if err != AAC_ENC_OK {
            continue;
        }
        let frames = if cfg.frame_length >= 1024 { 24 } else { 40 };
        let n_sig = 7 + 7;
        let sig = k % n_sig;
        if !run(&mut h, cfg, sig, frames, &mut r) {
            run(&mut h, cfg, 7 + k % 7, frames, &mut r);
        }
        // Re-initialization as FDKaacEnc_Initialize with another bit rate (states kept),
        // then a full reset.
        let mut cfg2 = *cfg;
        cfg2.bitrate = cfg.bitrate * 3 / 2;
        cfg2.use_pns ^= 1;
        let (err, _) = h.init(&cfg2, 0, false);
        if err == AAC_ENC_OK {
            run(&mut h, &cfg2, 7 + (k + 3) % 7, frames / 2, &mut r);
            let (err, _) = h.init(cfg, 1, true);
            if err == AAC_ENC_OK {
                run(&mut h, cfg, (k + 1) % n_sig, frames / 2, &mut r);
            }
        }
        ran += 1;
    }
    assert!(ran > 40, "only {ran} configurations ran");
    let cov = COVERAGE.with(|c| *c.borrow());
    eprintln!(
        "psy_main coverage: {} frames, msDigest {:?}, short {}, TNS {}, PNS bands {}, IS bands {}",
        cov[0],
        &cov[1..4],
        cov[4],
        cov[5],
        cov[6],
        cov[7]
    );
    assert!(cov[1..8].iter().all(|&x| x > 0), "coverage {cov:?}");
}

/// Channel mode changes between stereo and mono with more allocated channels (the
/// `resetChannels` / `chInc` logic of FDKaacEnc_psyInit keeps some states).
#[test]
fn psy_main_mode_switches() {
    let mut r = Rng::new(0x4d4f_4445);
    let base = Config {
        aot: AOT_AAC_LC,
        frame_length: 1024,
        sample_rate: 44100,
        mode: MODE_2,
        order: CH_ORDER_MPEG,
        bitrate: 96000,
        bitrate_mode: AACENC_BR_MODE_CBR,
        use_tns: 1,
        use_pns: 1,
        use_is: 1,
        use_ms: 1,
        syntax_flags: 0,
        bandwidth: 0,
    };
    for max_ch in [2, 6, 8] {
        let mut h = Harness::new(5, max_ch);
        for (step, &mode) in [MODE_2, MODE_1, MODE_2, MODE_1_2_2_1, MODE_2, MODE_1].iter().enumerate()
        {
            let n_ch = n_channels(mode);
            if n_ch > max_ch {
                continue;
            }
            let cfg = Config { mode, ..base };
            let (err, _) = h.init(&cfg, 0, true);
            assert_eq!(err, AAC_ENC_OK);
            h.n_ch = n_ch as usize;
            run(&mut h, &cfg, step % 4 + 1, 12, &mut r);
            h.compare("after switch");
        }
    }
}

/// Lowpass lines at the bounds of the window (0 and the window length, for the long, short and
/// LFE configurations) against the reference, and lowpass lines outside them on the port only:
/// the reference has no defined result for them, the port returns AAC_ENC_UNKNOWN without
/// changing any state, which the next frame (with valid lines again, on both) checks against the
/// reference.
#[test]
fn psy_main_lowpass_bounds() {
    let mut r = Rng::new(0x4c50_4153);
    let base = Config {
        aot: AOT_AAC_LC,
        frame_length: 1024,
        sample_rate: 44100,
        mode: MODE_2,
        order: CH_ORDER_MPEG,
        bitrate: 128000,
        bitrate_mode: AACENC_BR_MODE_CBR,
        use_tns: 1,
        use_pns: 1,
        use_is: 1,
        use_ms: 1,
        syntax_flags: 0,
        bandwidth: 0,
    };
    let cfgs = [
        base,
        Config { mode: MODE_1, sample_rate: 48000, bitrate: 64000, ..base },
        Config { mode: MODE_1_2_2_1, sample_rate: 48000, bitrate: 320000, ..base },
        Config { aot: AOT_ER_AAC_LD, frame_length: 512, sample_rate: 48000, bitrate: 64000, syntax_flags: AC_ER | AC_LD, ..base },
        Config { aot: AOT_ER_AAC_ELD, frame_length: 480, sample_rate: 48000, mode: MODE_1, bitrate: 32000, syntax_flags: AC_ER | AC_ELD, ..base },
    ];
    let mut rejected = 0;
    for (k, cfg) in cfgs.iter().enumerate() {
        let lc = cfg.aot == AOT_AAC_LC;
        for setting in 0..3 {
            let n_ch = n_channels(cfg.mode);
            let mut h = Harness::new(n_elements(cfg.mode).max(5), n_ch);
            let (err, _) = h.init(cfg, 1, true);
            assert_eq!(err, AAC_ENC_OK, "{cfg:?}");
            let psy = h.psy.as_deref().unwrap();
            let gl = psy.granule_length;
            let short_lfe = psy.psy_conf[1].lowpass_line_lfe;
            // (long, LFE, short)
            let lines = match setting {
                0 => (psy.psy_conf[0].lowpass_line, psy.psy_conf[0].lowpass_line_lfe, gl / TRANS_FAC),
                1 => (gl, gl, psy.psy_conf[1].lowpass_line),
                _ => (0, 0, 0),
            };
            let set = |h: &mut Harness, l: (i32, i32, i32)| {
                h.set_lowpass(0, l.0, l.1);
                if lc {
                    h.set_lowpass(1, l.2, short_lfe);
                }
            };
            set(&mut h, lines);
            h.what = format!("{cfg:?} lowpass {lines:?}");
            // clicks (short blocks), then other signals
            if !run(&mut h, cfg, 5, 6, &mut r) {
                run(&mut h, cfg, 7 + k % 7, 6, &mut r);
            }
            run(&mut h, cfg, 7 + (k + setting) % 7, 4, &mut r);

            // out of range: the port only
            let mut bad = vec![(gl + 1, lines.1, lines.2), (-1, lines.1, lines.2), (lines.0, gl + 1, lines.2), (lines.0, -1, lines.2)];
            if lc {
                bad.extend([(lines.0, lines.1, gl / TRANS_FAC + 1), (lines.0, lines.1, -1)]);
            }
            let len = cfg.frame_length as usize;
            let s = signal(7 + k % 7, n_ch as usize, len).unwrap();
            let mut planar = vec![0i16; len * n_ch as usize];
            for ch in 0..n_ch as usize {
                planar[ch * len..(ch + 1) * len].copy_from_slice(&s[ch]);
            }
            for &b in &bad {
                set(&mut h, b);
                assert_eq!(h.frame_r(&planar, len as u32), AAC_ENC_UNKNOWN, "{cfg:?} lowpass {b:?}");
                set(&mut h, lines);
                h.what = format!("{cfg:?} lowpass {lines:?} after {b:?}");
                h.compare(&h.what.clone());
                run(&mut h, cfg, 7 + (k + 1) % 7, 2, &mut r);
                rejected += 1;
            }
        }
    }
    assert!(rejected >= 60);
    let cov = COVERAGE.with(|c| *c.borrow());
    eprintln!("psy_main lowpass bounds: {} frames, short {}, TNS {}", cov[0], cov[4], cov[5]);
    assert!(cov[4] > 0, "no short blocks");
}

/// FADE_OUT_FACTOR (`fadeOutFactor`, file-static in psy_main.cpp; a copy compiled into the shim),
/// element by element.
#[test]
fn fade_out_factor() {
    let cname = std::ffi::CString::new("fadeOutFactor").unwrap();
    let mut bytes = 0;
    let p = unsafe { fdk_aac_rs_cref::aacenc_psy_main_tables::cref_psy_main_table(cname.as_ptr(), &mut bytes) };
    assert_eq!(bytes as usize, 4 * fdk_aac_rs_aacenc::psy_main::FADE_OUT_FACTOR.len());
    let c = unsafe { core::slice::from_raw_parts(p as *const i32, bytes as usize / 4) };
    assert_eq!(c, &fdk_aac_rs_aacenc::psy_main::FADE_OUT_FACTOR[..]);
}

/// The gain of [`amplify_spectrum`] in 1/256 (the same as cref_psy_main_amplified_set_gain()).
static AMPLIFY_GAIN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(256);

/// The spectrum hook of `psy_main_negative_final_shift`: `x * gain / 256` (rounded down),
/// saturated, as amplifySpectrum() in shim/aacenc_psy_main_amplified.cpp.
fn amplify_spectrum(x: &mut [i32]) {
    let g = AMPLIFY_GAIN.load(std::sync::atomic::Ordering::Relaxed) as i64;
    for v in x.iter_mut() {
        *v = ((*v as i64 * g) >> 8).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    }
}

/// FDKaacEnc_psyMain() with a negative `finalShift`, where `spectrum[line] <<=
/// finalShift` is undefined and the reference builds' vector and scalar shift instructions
/// disagree (the port's `shift_spectrum_left`). `nrgShift` is never negative: the energy that
/// FDKaacEnc_CheckBandEnergyOptim() returns is a Q31 value below 1.0 scaled by
/// 2^(2 * (minSpecShift - 4 - scale)), so its leading bits make up for `minSpecShift - 4`. Only
/// the decrement for short blocks makes `finalShift` negative, and then only -1: when the band
/// with the largest energy has `sfbMaxScaleSpec <= 4` and an energy of at least 0.25 (Q31),
/// which the transform of 16-bit PCM does not produce (`transform_energy_bound`). The
/// reference therefore runs a copy of psy_main.cpp whose transform calls amplify the spectrum
/// (shim/aacenc_psy_main_amplified.cpp; its shift loop compiles to the same instructions as the
/// library's), and the port applies the same amplification through
/// `psy_main_set_spectrum_hook`. Every configuration of `psy_main_sequences` (LC long and short
/// blocks, LD, ELD, 1 to 8 channels) runs frames with gains of 1 (none) and of 2^0 to 2^14
/// times 1 to 2 (log-uniform); the complete state is compared after every frame.
#[test]
fn psy_main_negative_final_shift() {
    use fdk_aac_rs_cref::aacenc_psy_main_amplified as amp;
    let mut r = Rng::new(0x4832_3535);
    let fn_c = unsafe { amp::cref_psy_main_amplified_fn() };
    psy_main_set_spectrum_hook(Some(amplify_spectrum));
    let before = psy_main_negative_final_shifts();
    let mut ran = 0;
    for (k, cfg) in configs(&mut r).iter().enumerate() {
        let n_ch = n_channels(cfg.mode);
        let mut h = Harness::new(n_elements(cfg.mode).max(5), n_ch);
        h.n_ch = n_ch as usize;
        h.c_fn = fn_c;
        let (err, _) = h.init(cfg, 1, true);
        if err != AAC_ENC_OK {
            continue;
        }
        for part in 0..8 {
            let gain = if part == 0 { 256 } else { r.range(256, 511) << r.range(0, 14) };
            AMPLIFY_GAIN.store(gain, std::sync::atomic::Ordering::Relaxed);
            unsafe { amp::cref_psy_main_amplified_set_gain(gain) };
            let sig = (k + part) % 14;
            if !run(&mut h, cfg, sig, 6, &mut r) {
                run(&mut h, cfg, 7 + (k + part) % 7, 6, &mut r);
            }
        }
        ran += 1;
    }
    psy_main_set_spectrum_hook(None);
    let negative = psy_main_negative_final_shifts() - before;
    eprintln!("psy_main_negative_final_shift: {ran} configurations, {negative} frames with finalShift < 0");
    assert!(ran > 40 && negative > 100, "{ran} configurations, {negative} negative shifts");
}

/// An upper bound of the band energies (sum of the squared spectral values, Q31)
/// that mdct_block() produces from 16-bit PCM in the AAC-LC filter bank, where short blocks make
/// `finalShift` -1 from a band energy of 0.25 on (`psy_main_negative_final_shift`). The
/// folded input of dct_IV() is at most 0.5 * (|w_a| + |w_b|) on the window slopes (fMultDiv2()
/// of two samples of at most 1.0 in magnitude with the two window coefficients) and 0.5 on the
/// flat parts, and dct_IV() is a scaled orthogonal transform: the sum of the squared outputs is
/// G(L) times the sum of the squared inputs (G measured here on random inputs, the largest ratio
/// plus a margin of 1%). So every band energy of a block is at most G(L) * sum(c_n^2), for every
/// combination of window shapes and slope lengths of a block of length L.
#[test]
fn transform_energy_bound() {
    use fdk_aac_rs_fdk::dct::dct_iv;
    let mut r = Rng::new(0x4832_3542);
    // sum of the squared bounds of one slope (fl / 2 coefficient pairs)
    let slope = |len: i32, shape: i32| -> f64 {
        fdk_get_window_slope(len, shape)
            .unwrap()
            .iter()
            .take(len as usize / 2)
            .map(|w| {
                let c = 0.5 * ((w.re as f64).abs() + (w.im as f64).abs()) / 32768.0;
                c * c
            })
            .sum()
    };
    for &(tl, slopes, threshold) in
        &[(1024, &[1024, 128][..], 0.25), (960, &[960, 120][..], 0.25), (128, &[128][..], 0.25), (120, &[120][..], 0.25)]
    {
        // G(tl): the largest energy gain of dct_IV() on random inputs of the magnitude of the
        // folded input
        let mut g: f64 = 0.0;
        for _ in 0..200 {
            let mut x: Vec<i32> = (0..tl).map(|_| r.i32() >> r.range(1, 3)).collect();
            if r.next_u32() % 2 == 0 {
                // a tone, which concentrates the energy in a few outputs
                let f = r.range(0, tl - 1) as f64 + 0.5;
                for (n, v) in x.iter_mut().enumerate() {
                    *v = (1.4e9 * (std::f64::consts::PI / tl as f64 * (n as f64 + 0.5) * f).cos()) as i32;
                }
            }
            let ein: f64 = x.iter().map(|&v| (v as f64 / 2f64.powi(31)).powi(2)).sum();
            let mut e = 0;
            dct_iv(&mut x, tl, &mut e);
            let eout: f64 = x.iter().map(|&v| (v as f64 / 2f64.powi(31)).powi(2)).sum();
            g = g.max(eout / ein);
        }
        let g = g * 1.01;
        // the largest sum of squared folded-input bounds over the slopes and shapes
        let mut s_max: f64 = 0.0;
        for &fl in slopes {
            for &fr in slopes {
                for sl in 0..2 {
                    for sr in 0..2 {
                        let flat = 0.25 * (((tl - fl) / 2 + (tl - fr) / 2) as f64);
                        s_max = s_max.max(flat + slope(fl, sl) + slope(fr, sr));
                    }
                }
            }
        }
        let bound = g * s_max;
        eprintln!("transform_energy_bound: L {tl}: G {g:.6e}, sum c^2 {s_max:.3}, band energy <= {bound:.4}");
        assert!(bound < threshold, "L {tl}: bound {bound}");
    }
}
