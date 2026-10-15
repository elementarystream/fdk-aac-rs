// Differential tests of fdk_aac_rs_drc::drc_dec_lib against the C++ reference.
//
// The public API runs end to end on both sides: a DRC decoder pair is opened, configured, fed
// with generated (and corrupted) uniDrcConfig(), loudnessInfoSet() and uniDrcGain() payloads,
// random user parameters and random audio over many frames, and the return values, the audio
// buffers and the complete state of the decoder (the memory images of all structures, including
// the gain decoder and the selection process) are compared after every call.
//
// This covers drcDec_gainDecoder, drcGainDec_init, drcGainDec_preprocess, drcGainDec_process and
// drcDec_selectionProcess through the API as the library uses them.

use core::ffi::c_void;
use fdk_aac_rs_cref::drc_drc_dec_lib as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_drc::drc_dec_lib::*;
use fdk_aac_rs_drc::drc_dec_types::*;
use fdk_aac_rs_fdk::audio::{LibInfo, FDK_MODULE_LAST};
use fdk_aac_rs_fdk::common_fix::FixpDbl;

use super::drc_dec_reader::{
    bs_state, cmp_bytes, gen_loudness_info_set, gen_uni_drc_config, gen_uni_drc_gain, set_clean,
    set_extreme, BitWriter, Payload,
};

/// A DRC decoder of the port and one of the reference.
pub struct Pair {
    pub rs: HandleDrcDecoder,
    pub c: *mut c_void,
    pub log: Vec<String>,
    /// Calls of FDK_drcDec_ProcessTime that changed the guard zones (no defined reference
    /// result).
    pub guard_writes: u32,
}

impl Drop for Pair {
    fn drop(&mut self) {
        if !self.c.is_null() {
            unsafe { c::cref_drc_lib_close(self.c) };
        }
    }
}

impl Pair {
    pub fn open(functional_range: i32) -> Pair {
        let mut rs: HandleDrcDecoder = None;
        let err = fdk_drc_dec_open(&mut rs, functional_range);
        let mut cerr = 0;
        let c = unsafe { c::cref_drc_lib_open(functional_range, &mut cerr) };
        assert_eq!(err, cerr, "FDK_drcDec_Open({functional_range})");
        assert!(!c.is_null());
        let p = Pair { rs, c, log: vec![format!("open({functional_range})")], guard_writes: 0 };
        p.check("FDK_drcDec_Open");
        p
    }

    fn rs(&mut self) -> Option<&mut DrcDecoder> {
        self.rs.as_deref_mut()
    }

    fn fail(&self, what: &str) -> String {
        let n = self.log.len();
        format!("{what} after: {}", self.log[n.saturating_sub(12)..].join("; "))
    }

    /// Compares the complete state.
    pub fn check(&self, what: &str) {
        let d = self.rs.as_deref().unwrap();
        let mut head = [0i32; 6];
        let mut cfg = vec![0u8; core::mem::size_of::<UniDrcConfig>()];
        let mut lis = vec![0u8; core::mem::size_of::<LoudnessInfoSet>()];
        let mut gain = vec![0u8; core::mem::size_of::<UniDrcGain>()];
        let mut sel_out = vec![0u8; 288];
        // The sizes of DRC_GAIN_DECODER and s_drcdec_selection_process (10472 and 1496 bytes
        // with 8-byte pointers, 10436 and 1344 with 4-byte ones), as the port's types.
        let mut gain_dec = vec![0u8; core::mem::size_of::<fdk_aac_rs_drc::drc_dec_gain_decoder::DrcGainDecoder>()];
        let mut sel_proc = vec![0u8; core::mem::size_of::<fdk_aac_rs_drc::drc_dec_selection_process::DrcSelectionProcess>()];
        unsafe {
            c::cref_drc_lib_state(
                self.c,
                head.as_mut_ptr(),
                cfg.as_mut_ptr(),
                lis.as_mut_ptr(),
                gain.as_mut_ptr(),
                sel_out.as_mut_ptr(),
                gain_dec.as_mut_ptr(),
                sel_proc.as_mut_ptr(),
            )
        };
        let rs_head = [
            d.codec_mode,
            d.functional_range,
            d.status,
            d.h_gain_dec.is_some() as i32,
            d.h_selection_proc.is_some() as i32,
            d.sel_proc_input_diff,
        ];
        assert_eq!(rs_head, head, "{}", self.fail(&format!("{what}: DRC_DECODER head")));
        let ctx = |s: &str| self.fail(&format!("{what}: {s}"));
        cmp_bytes(&ctx("UNI_DRC_CONFIG"), pod_bytes(&*d.uni_drc_config), &cfg);
        cmp_bytes(&ctx("LOUDNESS_INFO_SET"), pod_bytes(&*d.loudness_info_set), &lis);
        cmp_bytes(&ctx("UNI_DRC_GAIN"), pod_bytes(&*d.uni_drc_gain), &gain);
        cmp_bytes(&ctx("SEL_PROC_OUTPUT"), pod_bytes(&d.sel_proc_output), &sel_out);
        if let Some(g) = d.h_gain_dec.as_deref() {
            cmp_bytes(&ctx("DRC_GAIN_DECODER"), pod_bytes(g), &gain_dec);
        }
        if let Some(s) = d.h_selection_proc.as_deref() {
            cmp_bytes(&ctx("s_drcdec_selection_process"), pod_bytes(s), &sel_proc);
        }
    }

    pub fn set_codec_mode(&mut self, mode: i32) -> i32 {
        self.log.push(format!("setCodecMode({mode})"));
        let r = fdk_drc_dec_set_codec_mode(self.rs(), mode);
        let cr = unsafe { c::cref_drc_lib_set_codec_mode(self.c, mode) };
        assert_eq!(r, cr, "{}", self.fail("FDK_drcDec_SetCodecMode"));
        self.check("FDK_drcDec_SetCodecMode");
        r
    }

    pub fn init(&mut self, frame_size: i32, sample_rate: i32, base_channel_count: i32) -> i32 {
        self.log.push(format!("init({frame_size}, {sample_rate}, {base_channel_count})"));
        let r = fdk_drc_dec_init(self.rs(), frame_size, sample_rate, base_channel_count);
        let cr = unsafe { c::cref_drc_lib_init(self.c, frame_size, sample_rate, base_channel_count) };
        assert_eq!(r, cr, "{}", self.fail("FDK_drcDec_Init"));
        self.check("FDK_drcDec_Init");
        r
    }

    pub fn set_param(&mut self, t: i32, v: i32) -> i32 {
        self.log.push(format!("setParam({t}, {v:#x})"));
        let r = fdk_drc_dec_set_param(self.rs(), t, v);
        let cr = unsafe { c::cref_drc_lib_set_param(self.c, t, v) };
        assert_eq!(r, cr, "{}", self.fail(&format!("FDK_drcDec_SetParam({t}, {v})")));
        self.check(&format!("FDK_drcDec_SetParam({t}, {v})"));
        r
    }

    pub fn get_param(&mut self, t: i32) -> i32 {
        let r = fdk_drc_dec_get_param(self.rs.as_deref(), t);
        let cr = unsafe { c::cref_drc_lib_get_param(self.c, t) };
        assert_eq!(r, cr, "{}", self.fail(&format!("FDK_drcDec_GetParam({t})")));
        r
    }

    pub fn read(&mut self, which: i32, p: &Payload) -> i32 {
        self.log.push(format!("read{which}({} bits)", p.valid_bits));
        let (_b, mut bs) = p.rs();
        let r = match which {
            0 => fdk_drc_dec_read_uni_drc_config(self.rs(), &mut bs),
            1 => fdk_drc_dec_read_loudness_info_set(self.rs(), &mut bs),
            2 => fdk_drc_dec_read_uni_drc_gain(self.rs(), &mut bs),
            3 => fdk_drc_dec_read_uni_drc(self.rs(), &mut bs),
            4 => fdk_drc_dec_read_loudness_box(self.rs(), &mut bs),
            5 => fdk_drc_dec_read_downmix_instructions_box(self.rs(), &mut bs),
            6 => fdk_drc_dec_read_uni_drc_instructions_box(self.rs(), &mut bs),
            _ => fdk_drc_dec_read_uni_drc_coefficients_box(self.rs(), &mut bs),
        };
        let st = bs_state(&mut bs);
        let mut buf = p.buf.clone();
        let mut cst = [0u32; 3];
        let cr = unsafe {
            c::cref_drc_lib_read(
                self.c,
                which,
                buf.as_mut_ptr(),
                buf.len() as u32,
                p.valid_bits,
                cst.as_mut_ptr(),
            )
        };
        assert_eq!(r, cr, "{}", self.fail(&format!("FDK_drcDec_Read #{which}: error")));
        assert_eq!(st, cst, "{}", self.fail(&format!("FDK_drcDec_Read #{which}: bitstream")));
        self.check(&format!("FDK_drcDec_Read #{which}"));
        r
    }

    pub fn set_channel_gains(
        &mut self,
        num_channels: i32,
        frame_size: i32,
        gains: &[FixpDbl; 8],
        audio: &mut [FixpDbl],
        offset: i32,
    ) {
        self.log.push(format!("setChannelGains({num_channels}, {frame_size}, {gains:?})"));
        let mut caudio = audio.to_vec();
        let mut cg = *gains;
        fdk_drc_dec_set_channel_gains(self.rs(), num_channels, frame_size, gains, audio, offset);
        unsafe {
            c::cref_drc_lib_set_channel_gains(
                self.c,
                num_channels,
                frame_size,
                cg.as_mut_ptr(),
                caudio.as_mut_ptr(),
                offset,
            )
        };
        assert!(audio == &caudio[..], "{}", self.fail("FDK_drcDec_SetChannelGains: audio"));
        self.check("FDK_drcDec_SetChannelGains");
    }

    pub fn preprocess(&mut self) -> i32 {
        self.log.push("preprocess".into());
        let r = fdk_drc_dec_preprocess(self.rs());
        let cr = unsafe { c::cref_drc_lib_preprocess(self.c) };
        assert_eq!(r, cr, "{}", self.fail("FDK_drcDec_Preprocess"));
        self.check("FDK_drcDec_Preprocess");
        r
    }

    pub fn process_time(
        &mut self,
        delay: i32,
        loc: i32,
        ch_off: i32,
        drc_ch_off: i32,
        num: i32,
        audio: &mut [FixpDbl],
        td_off: i32,
    ) -> i32 {
        self.log.push(format!("processTime({delay}, {loc}, {ch_off}, {drc_ch_off}, {num}, {td_off})"));
        // The C buffer has guard zones: the reference has no defined result for samples
        // outside the buffer; the port drops these writes.
        const GUARD: usize = 1 << 17;
        const SENTINEL: FixpDbl = 0x5a5a_5a5a;
        let mut cbuf = vec![SENTINEL; GUARD + audio.len() + GUARD];
        cbuf[GUARD..GUARD + audio.len()].copy_from_slice(audio);
        let r = fdk_drc_dec_process_time(self.rs(), delay, loc, ch_off, drc_ch_off, num, audio, td_off);
        let cr = unsafe {
            c::cref_drc_lib_process_time(
                self.c,
                delay,
                loc,
                ch_off,
                drc_ch_off,
                num,
                cbuf.as_mut_ptr().add(GUARD),
                td_off,
            )
        };
        assert_eq!(r, cr, "{}", self.fail("FDK_drcDec_ProcessTime"));
        let n = audio.len();
        if cbuf[..GUARD].iter().chain(&cbuf[GUARD + n..]).any(|&v| v != SENTINEL) {
            self.guard_writes += 1;
        }
        let caudio = &cbuf[GUARD..GUARD + n];
        if let Some(i) = (0..audio.len()).find(|&i| audio[i] != caudio[i]) {
            panic!(
                "{}",
                self.fail(&format!(
                    "FDK_drcDec_ProcessTime: audio[{i}] rust {} c {}",
                    audio[i], caudio[i]
                ))
            );
        }
        self.check("FDK_drcDec_ProcessTime");
        r
    }

    pub fn process_freq(
        &mut self,
        delay: i32,
        loc: i32,
        ch_off: i32,
        drc_ch_off: i32,
        num: i32,
        single: i32,
        re: &mut [FixpDbl],
        im: &mut [FixpDbl],
        stride: usize,
    ) -> i32 {
        self.log.push(format!("processFreq({delay}, {loc}, {ch_off}, {drc_ch_off}, {num}, {single})"));
        let mut cre = re.to_vec();
        let mut cim = im.to_vec();
        let r = {
            let mut rr: Vec<&mut [FixpDbl]> = re.chunks_mut(stride).collect();
            let mut ii: Vec<&mut [FixpDbl]> = im.chunks_mut(stride).collect();
            fdk_drc_dec_process_freq(
                self.rs(),
                delay,
                loc,
                ch_off,
                drc_ch_off,
                num,
                single,
                &mut rr,
                &mut ii,
            )
        };
        let cr = unsafe {
            c::cref_drc_lib_process_freq(
                self.c,
                delay,
                loc,
                ch_off,
                drc_ch_off,
                num,
                single,
                cre.as_mut_ptr(),
                cim.as_mut_ptr(),
                stride as i32,
            )
        };
        assert_eq!(r, cr, "{}", self.fail("FDK_drcDec_ProcessFreq"));
        assert!(re == &cre[..], "{}", self.fail("FDK_drcDec_ProcessFreq: real"));
        assert!(im == &cim[..], "{}", self.fail("FDK_drcDec_ProcessFreq: imag"));
        self.check("FDK_drcDec_ProcessFreq");
        r
    }

    pub fn apply_downmix(&mut self, audio: &mut [FixpDbl], n_channels: &mut i32) -> i32 {
        self.log.push(format!("applyDownmix({n_channels})"));
        let map: [i32; 8] = core::array::from_fn(|i| i as i32);
        let mut caudio = audio.to_vec();
        let mut cmap_in = map;
        let mut cmap_out = map;
        let mut cn = *n_channels;
        let r = fdk_drc_dec_apply_downmix(self.rs(), Some(&map), Some(&map), Some(audio), n_channels);
        let cr = unsafe {
            c::cref_drc_lib_apply_downmix(
                self.c,
                cmap_in.as_mut_ptr(),
                cmap_out.as_mut_ptr(),
                caudio.as_mut_ptr(),
                &mut cn,
            )
        };
        assert_eq!(r, cr, "{}", self.fail("FDK_drcDec_ApplyDownmix"));
        assert_eq!(*n_channels, cn, "{}", self.fail("FDK_drcDec_ApplyDownmix: channels"));
        assert!(audio == &caudio[..], "{}", self.fail("FDK_drcDec_ApplyDownmix: audio"));
        self.check("FDK_drcDec_ApplyDownmix");
        r
    }
}

fn chance(rng: &mut Rng, num: u32, den: u32) -> bool {
    rng.next_u32() % den < num
}

fn pick(rng: &mut Rng, lo: i32, hi: i32) -> i32 {
    rng.range(lo, hi)
}

/// A random user parameter request.
fn random_param(rng: &mut Rng) -> (i32, i32) {
    let t = if chance(rng, 1, 16) { pick(rng, -1, 20) } else { pick(rng, 0, 17) };
    let v = match t {
        DRC_DEC_BOOST | DRC_DEC_COMPRESS => match rng.next_u32() % 4 {
            0 => rng.i32(),
            _ => pick(rng, 0, 0x4000_0000),
        },
        DRC_DEC_LOUDNESS_NORMALIZATION_ON | DRC_DEC_ALBUM_MODE => pick(rng, -1, 2),
        DRC_DEC_TARGET_LOUDNESS => match rng.next_u32() % 4 {
            0 => rng.i32(),
            /* (INT)-defaultTargetLoudness * FL2FXCONST_DBL(1.0f / (float)(1 << 9)) */
            _ => -pick(rng, 0, 127) * (1 << 22),
        },
        DRC_DEC_EFFECT_TYPE => pick(rng, -2, 10),
        DRC_DEC_LOUDNESS_MEASUREMENT_METHOD => pick(rng, -1, 3),
        DRC_DEC_DOWNMIX_ID => pick(rng, -1, 8),
        DRC_DEC_TARGET_CHANNEL_COUNT_REQUESTED => pick(rng, 0, 9),
        DRC_DEC_BASE_CHANNEL_COUNT => pick(rng, -1, 9),
        DRC_DEC_FRAME_SIZE => [1024, 2048, 960, 512, 768, -1, 256][rng.next_u32() as usize % 7],
        DRC_DEC_SAMPLE_RATE => {
            [48000, 44100, 32000, 96000, 22050, 500, -1][rng.next_u32() as usize % 7]
        }
        _ => rng.i32(),
    };
    (t, v)
}

fn random_audio(rng: &mut Rng, n: usize) -> Vec<FixpDbl> {
    let level = rng.next_u32() % 4;
    (0..n)
        .map(|_| match level {
            0 => rng.i32(),
            1 => (rng.next_u32() as i32) >> 4,
            2 => (rng.next_u32() as i32) >> 12,
            _ => 0,
        })
        .collect()
}

/// A channel gain in dB (e = 7) as aacdec passes them (mostly 0).
fn random_channel_gains(rng: &mut Rng) -> [FixpDbl; 8] {
    let mode = rng.next_u32() % 4;
    core::array::from_fn(|_| match mode {
        0 | 1 => 0,
        2 => match rng.next_u32() % 8 {
            0 => FixpDbl::MIN,
            _ => -(pick(rng, 0, 40) << 24),
        },
        _ => rng.i32() >> pick(rng, 0, 8),
    })
}

/// One decoder run: configuration, payloads, parameters and `frames` frames of audio.
fn run(rng: &mut Rng, frames: usize, stats: &mut Stats) {
    set_clean(chance(rng, 3, 4));
    let range = match rng.next_u32() % 16 {
        0 => DRC_DEC_GAIN,
        1 => DRC_DEC_SELECTION,
        _ => DRC_DEC_ALL,
    };
    let mut p = Pair::open(range);
    let mode = match rng.next_u32() % 16 {
        0 => DRC_DEC_MPEG_4_AAC,
        1 => DRC_DEC_TEST_TIME_DOMAIN,
        2 => DRC_DEC_TEST_STFT_DOMAIN,
        3 => DRC_DEC_MPEG_H_3DA,
        4 => DRC_DEC_TEST_QMF_DOMAIN,
        5 => pick(rng, -101, 3),
        _ => DRC_DEC_MPEG_D_USAC,
    };
    p.set_codec_mode(mode);
    if chance(rng, 1, 8) {
        p.set_codec_mode(pick(rng, -101, 3));
    }
    let mut frame_size = match rng.next_u32() % 8 {
        0 => 2048,
        1 => 960,
        2 => 512,
        3 => 768,
        4 => 256 * pick(rng, 1, 16),
        _ => 1024,
    };
    let sample_rate = [48000, 44100, 32000, 96000, 16000, 24000, 7350][rng.next_u32() as usize % 7];
    let base_channel_count = pick(rng, 1, 8);
    for _ in 0..pick(rng, 0, 2) {
        let (t, v) = random_param(rng);
        p.set_param(t, v);
    }
    if chance(rng, 1, 16) {
        p.init(if chance(rng, 1, 2) { 0 } else { frame_size }, 0, base_channel_count);
    }
    p.init(frame_size, sample_rate, base_channel_count);
    if chance(rng, 1, 8) {
        let (t, v) = random_param(rng);
        p.set_param(t, v);
    }
    // Request a DRC effect (without, the virtual "no DRC" set wins the selection).
    if chance(rng, 2, 3) {
        p.set_param(DRC_DEC_EFFECT_TYPE, pick(rng, 1, 8));
    }

    let n_ch = 8usize;
    for frame in 0..frames {
        // Configuration and loudness payloads, at the start and now and then.
        if frame == 0 || chance(rng, 1, 16) {
            let w = gen_uni_drc_config(rng);
            let mut w2 = w.clone();
            if chance(rng, 1, 16) {
                w2.corrupt(rng, 2);
            }
            if chance(rng, 3, 4) {
                p.read(0, &Payload::new(&w2, None));
            } else {
                // uniDrc() with loudnessInfoSet and uniDrcConfig present.
                let mut u = BitWriter::new();
                u.put(1, 1);
                u.put(1, 1);
                u.append(&w2);
                u.append(&gen_loudness_info_set(rng));
                let cfg = p.rs.as_deref().unwrap().uni_drc_config.clone();
                let (fs, dt) = gain_params(&p);
                u.append(&gen_uni_drc_gain(rng, &cfg, fs, dt));
                p.read(3, &Payload::new(&u, None));
            }
        }
        if frame == 0 || chance(rng, 1, 16) {
            let w = gen_loudness_info_set(rng);
            p.read(1, &Payload::new(&w, None));
        }
        if chance(rng, 1, 32) {
            let w = gen_loudness_info_set(rng);
            p.read(pick(rng, 4, 7), &Payload::new(&w, None));
        }
        if chance(rng, 1, 12) {
            let (t, v) = random_param(rng);
            p.set_param(t, v);
            if t == DRC_DEC_FRAME_SIZE && v > 0 {
                frame_size = v;
            }
        }
        for t in [13, 14, 15, 16, 17, 0, 1, pick(rng, -1, 19)] {
            // Only queries with a defined reference result.
            if t == DRC_DEC_IS_ACTIVE && range == DRC_DEC_GAIN {
                continue;
            }
            p.get_param(t);
        }

        // Gain payload (sometimes missing: concealment).
        if chance(rng, 7, 8) {
            let cfg = p.rs.as_deref().unwrap().uni_drc_config.clone();
            let (fs, dt) = gain_params(&p);
            let mut w = gen_uni_drc_gain(rng, &cfg, fs, dt);
            if chance(rng, 1, 32) {
                w.corrupt(rng, 1);
            }
            p.read(2, &Payload::new(&w, None));
        }

        // The buffers follow the frame size of the gain decoder (set by FDK_drcDec_Init and
        // FDK_drcDec_SetParam).
        let (gfs, _) = gain_params(&p);
        if gfs > 0 {
            frame_size = gfs;
        }
        let fs = frame_size.max(1) as usize;
        let num_channels = if chance(rng, 7, 8) { base_channel_count } else { pick(rng, 1, 8) };
        let gains = random_channel_gains(rng);
        let mut audio = random_audio(rng, n_ch * fs);
        // Only calls with a defined reference result.
        if range != DRC_DEC_SELECTION {
            p.set_channel_gains(num_channels, frame_size, &gains, &mut audio, frame_size);
        }
        p.preprocess();
        {
            let d = p.rs.as_deref().unwrap();
            stats.frames += 1;
            if let Some(g) = d.h_gain_dec.as_deref() {
                let cfg = &d.uni_drc_config;
                for a in 0..(g.n_active_drcs.clamp(0, 3) as usize) {
                    let Some(i) = g.active_drc[a].inst() else { continue };
                    let inst = &cfg.drc_instructions_uni_drc[i];
                    if inst.drc_set_id > 0 {
                        stats.real_drc += 1;
                        if inst.drc_set_effect & 0x0c00 != 0 {
                            stats.ducking += 1;
                        }
                        if g.active_drc[a].band_count_for_channel_group.iter().any(|&b| b > 1) {
                            stats.multiband += 1;
                        }
                    }
                }
                if g.channel_gain_active_drc_index < 0 {
                    stats.smoothing += 1;
                }
            }
            if d.uni_drc_gain.status != 0 {
                stats.gain_payloads += 1;
            }
        }

        let stft = matches!(mode, DRC_DEC_TEST_STFT_DOMAIN | DRC_DEC_MPEG_H_3DA)
            && chance(rng, 1, 2);
        let delay = match rng.next_u32() % 4 {
            0 => 257,
            1 => pick(rng, 0, 3 * frame_size),
            _ => 0,
        };
        let ch_off = if chance(rng, 7, 8) { 0 } else { pick(rng, 0, 3) };
        let drc_ch_off = if chance(rng, 7, 8) { 0 } else { pick(rng, -2, 3) };
        if !stft {
            p.process_time(delay, DRC_DEC_DRC1, ch_off, drc_ch_off, num_channels, &mut audio, frame_size);
            let mut n = if chance(rng, 7, 8) { num_channels } else { pick(rng, 0, 8) };
            p.apply_downmix(&mut audio, &mut n);
            let loc = if chance(rng, 3, 4) { DRC_DEC_DRC2_DRC3 } else { pick(rng, -1, 5) };
            p.process_time(delay, loc, ch_off, drc_ch_off, n.clamp(0, 8), &mut audio, frame_size);
        } else {
            let stride = (fs / 256).max(1) * 512;
            let mut re = random_audio(rng, n_ch * stride);
            let mut im = random_audio(rng, n_ch * stride);
            let single = if chance(rng, 1, 2) { -1 } else { pick(rng, -1, (fs / 256) as i32) };
            let loc = if chance(rng, 3, 4) { DRC_DEC_DRC1 } else { pick(rng, -1, 5) };
            if p.process_freq(delay, loc, ch_off, drc_ch_off, num_channels, single, &mut re, &mut im, stride) == 0 {
                stats.freq_ok += 1;
            }
            if single >= 0 {
                p.process_freq(delay, loc, ch_off, drc_ch_off, num_channels, single, &mut re, &mut im, stride);
            }
        }
    }
    stats.guard_writes += p.guard_writes;
}

/// `drcDec_GainDecoder_GetFrameSize`, `drcDec_GainDecoder_GetDeltaTminDefault` of the port.
fn gain_params(p: &Pair) -> (i32, i32) {
    match p.rs.as_deref().unwrap().h_gain_dec.as_deref() {
        Some(g) => (g.frame_size, g.delta_tmin_default),
        None => (-1, -1),
    }
}

/// Coverage counters of [`run`] (per frame).
#[derive(Debug, Default)]
struct Stats {
    frames: u32,
    real_drc: u32,
    ducking: u32,
    multiband: u32,
    smoothing: u32,
    gain_payloads: u32,
    freq_ok: u32,
    guard_writes: u32,
}

#[test]
fn api_end_to_end() {
    let mut stats = Stats::default();
    let mut rng = Rng::new(0xd5c0_1001);
    for _ in 0..800 {
        let frames = pick(&mut rng, 1, 30) as usize;
        run(&mut rng, frames, &mut stats);
    }
    eprintln!("{stats:?}");
    assert!(stats.real_drc > 1000, "{stats:?}");
    assert!(stats.ducking > 50 && stats.multiband > 50, "{stats:?}");
    assert!(stats.smoothing > 200 && stats.gain_payloads > 2000, "{stats:?}");
    assert!(stats.freq_ok > 100, "{stats:?}");
    assert_eq!(stats.guard_writes, 0, "{stats:?}");
}

/// Node times that wrap in 16 bits (largest time deltas, large frames).
#[test]
fn api_extreme_times() {
    let mut rng = Rng::new(0xd5c0_1004);
    let mut stats = Stats::default();
    set_extreme(true);
    for _ in 0..300 {
        run(&mut rng, 10, &mut stats);
    }
    set_extreme(false);
    eprintln!("{stats:?}");
    assert!(stats.real_drc > 100, "{stats:?}");
}

#[test]
fn api_long_runs() {
    let mut rng = Rng::new(0xd5c0_1002);
    for _ in 0..8 {
        run(&mut rng, 300, &mut Stats::default());
    }
}

#[test]
fn api_null_handles() {
    let mut bs = fdk_aac_rs_fdk::bitstream::FdkBitStream::default();
    assert_eq!(fdk_drc_dec_set_codec_mode(None, 1), DRC_DEC_NOT_OPENED);
    assert_eq!(fdk_drc_dec_init(None, 1024, 48000, 2), DRC_DEC_OK);
    assert_eq!(fdk_drc_dec_close(None), DRC_DEC_OK);
    let mut h: HandleDrcDecoder = None;
    assert_eq!(fdk_drc_dec_close(Some(&mut h)), DRC_DEC_NOT_OPENED);
    assert_eq!(fdk_drc_dec_set_param(None, 0, 0), DRC_DEC_NOT_OPENED);
    assert_eq!(fdk_drc_dec_get_param(None, 0), DRC_DEC_NOT_OPENED);
    assert_eq!(fdk_drc_dec_read_uni_drc_config(None, &mut bs), DRC_DEC_NOT_OPENED);
    assert_eq!(fdk_drc_dec_read_uni_drc(None, &mut bs), DRC_DEC_NOT_OPENED);
    assert_eq!(fdk_drc_dec_preprocess(None), DRC_DEC_NOT_OPENED);
    assert!(fdk_drc_dec_get_selection_process_output(None).is_none());
    let mut r = 0;
    assert_eq!(fdk_drc_dec_get_group_loudness(None, 0, &mut r), 0);
    assert_eq!(fdk_drc_dec_set_interface_parameters(None), DRC_DEC_UNSUPPORTED_FUNCTION);
}

#[test]
fn get_lib_info() {
    let mut rng = Rng::new(0xd5c0_1003);
    for case in 0..64 {
        let n = FDK_MODULE_LAST as usize;
        let free = if case == 0 { n } else { (rng.next_u32() as usize) % (n + 1) };
        let ids_in: Vec<i32> =
            (0..n).map(|i| if i >= free { 0 } else { 1 + (rng.next_u32() % 38) as i32 }).collect();
        let mut info: Vec<LibInfo> = (0..n)
            .map(|i| LibInfo {
                module_id: ids_in[i],
                flags: 0x5a5a_0000 + i as u32,
                version_str: [b'x'; 32],
                ..LibInfo::default()
            })
            .collect();
        let r = fdk_drc_dec_get_lib_info(Some(&mut info));
        let mut ids = vec![0i32; n];
        let mut version = vec![0i32; n];
        let mut flags = vec![0u32; n];
        let mut vs = vec![0u8; 32 * n];
        let mut title = vec![0u8; 32 * n];
        let cr = unsafe {
            c::cref_drc_lib_get_lib_info(
                0,
                ids_in.as_ptr(),
                ids.as_mut_ptr(),
                version.as_mut_ptr(),
                flags.as_mut_ptr(),
                vs.as_mut_ptr() as *mut i8,
                title.as_mut_ptr() as *mut i8,
            )
        };
        assert_eq!(r, cr);
        for i in 0..n {
            assert_eq!(info[i].module_id, ids[i], "{i}");
            assert_eq!(info[i].version, version[i], "{i}");
            assert_eq!(info[i].flags, flags[i], "{i}");
            assert_eq!(&info[i].version_str[..], &vs[32 * i..32 * i + 32], "{i}");
            let t = &title[32 * i..32 * i + 32];
            let tl = t.iter().position(|&b| b == 0).unwrap_or(31);
            if ids_in[i] == 0 && i == free {
                assert_eq!(info[i].title.as_bytes(), &t[..tl], "{i}");
            }
        }
    }
    assert_eq!(fdk_drc_dec_get_lib_info(None), DRC_DEC_INVALID_PARAM);
}
