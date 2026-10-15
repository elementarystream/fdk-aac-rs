// Differential tests of fdk_aac_rs_aacenc::metadata_main against the C++ reference.
//
// * The file-static helpers (profile conversion, dialnorm and program reference level, the
//   downmix level, DRC gain and compression value codings), LoadSubmittedMetadata and the two
//   payload writers on random inputs; the tables.
// * The metadata encoder as a whole (FDK_MetadataEnc_Open/Init/Process/GetDelay/Close of the
//   library): random sequences of initializations (metadata modes 0-3, resets, delays, frame
//   lengths, channel modes and orders, including the seamless channel reconfiguration) and
//   frames with random audio and metadata changes. Compared after every call: the return
//   values, the extension payloads (type, size, bytes), the matrix mixdown index (including
//   the bytes the ETSI payload writes over it), the delayed audio, the delay and the state
//   (delay lines of audio and metadata).

use fdk_aac_rs_aacenc::aacenc_lib::AacencMetaData;
use fdk_aac_rs_aacenc::metadata_main::*;
use fdk_aac_rs_cref::aacenc_metadata_main as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;

/// `AacMetadata` as the 82 integers of the shim.
fn md_to_ints(m: &AacMetadata) -> Vec<i32> {
    let d = &m.mpeg_drc;
    let e = &m.etsi_anc_data;
    let x = &e.ext_anc_data;
    let mut v = vec![d.prog_ref_level_present as i32, d.prog_ref_level as i32];
    v.extend(d.dyn_rng_sgn.iter().map(|&b| b as i32));
    v.extend(d.dyn_rng_ctl.iter().map(|&b| b as i32));
    v.push(d.drc_bands_present as i32);
    v.push(d.drc_band_incr as i32);
    v.extend(d.drc_band_top.iter().map(|&b| b as i32));
    v.extend([
        d.drc_interpolation_scheme as i32,
        d.drc_profile,
        d.drc_target_ref_level,
        d.excluded_chns_present as i32,
        d.exclude_mask[0] as i32,
        d.exclude_mask[1] as i32,
        e.compression_on as i32,
        e.compression_value as i32,
        e.comp_profile,
        e.comp_target_ref_level,
        e.timecode_coarse_status,
        e.timecode_fine_status,
        e.ext_anc_data_status as i32,
        x.ext_downmix_lvl_status as i32,
        x.ext_downmix_gain_status as i32,
        x.ext_lfe_downmix_status as i32,
        x.ext_dmix_a_idx as i32,
        x.ext_dmix_b_idx as i32,
        x.dmx_gain_5_sgn as i32,
        x.dmx_gain_5_idx as i32,
        x.dmx_gain_2_sgn as i32,
        x.dmx_gain_2_idx as i32,
        x.ext_dmix_lfe_idx as i32,
        m.center_mix_level as i32,
        m.surround_mix_level as i32,
        m.write_pce_mix_dwn_idx as i32,
        m.dmx_lvl_on as i32,
        m.dolby_surround_mode as i32,
        m.drc_presentation_mode as i32,
        m.metadata_mode as i32,
    ]);
    assert_eq!(v.len(), 82);
    v
}

/// A random `AACENC_MetaData` with the table indexes in range.
fn random_meta(rng: &mut Rng) -> AacencMetaData {
    crate::aacenc_lib::random_meta(rng)
}

#[test]
fn helpers() {
    let mut rng = Rng::new(11);
    let h = |w: i32, a: i32, b: i32| -> i32 {
        // SAFETY: shim call.
        unsafe { c::cref_metadata_main_helper(w, a, b) }
    };
    for k in 0..100000 {
        let a = if k % 3 == 0 { rng.i32() } else { rng.range(-(40 << 16), 40 << 16) };
        let b = rng.i32();
        let p = [0, 1, 2, 3, 4, 5, 256, 6, -1, 255, 257][rng.range(0, 10) as usize];
        assert_eq!(convert_profile(p), h(0, p, 0), "convertProfile({p})");
        assert_eq!(dialnorm2progreflvl(a) as i32, h(1, a, 0), "dialnorm2progreflvl({a})");
        assert_eq!(progreflvl2dialnorm(a as u8), h(2, a, 0), "progreflvl2dialnorm({a})");
        assert_eq!(encode_dmx_lvls(a as i8, b as i8) as i32, h(3, a, b), "encodeDmxLvls({a}, {b})");
        let (mut ctl, mut sgn) = (0u8, 0u8);
        encode_dynrng(a, &mut ctl, &mut sgn);
        assert_eq!(ctl as i32 | (sgn as i32) << 8, h(4, a, 0), "encodeDynrng({a})");
        assert_eq!(decode_dynrng(a as u8, b as u8), h(5, a, b), "decodeDynrng({a}, {b})");
        let g = if k % 2 == 0 { a } else { rng.range(-4_000_000, 4_000_000) };
        assert_eq!(encode_compr(g) as i32, h(6, g, 0), "encodeCompr({g})");
        assert_eq!(decode_compr(a as u8), h(7, a, 0), "decodeCompr({a})");
    }
}

#[test]
fn load_and_write_payloads() {
    let mut rng = Rng::new(12);
    for k in 0..20000 {
        let mut m = random_meta(&mut rng);
        if k % 4 == 0 {
            // unchecked fields: any values (the payload writers mask them)
            m.drc_profile = rng.i32();
            m.comp_profile = [0, 256, rng.i32()][rng.range(0, 2) as usize];
            m.prog_ref_level = rng.i32();
            m.ext_meta_data.dmx_gain5 = rng.i32();
            m.ext_meta_data.dmx_gain2 = rng.i32();
            m.ext_meta_data.ext_anc_data_enable = rng.range(0, 3) as u8;
            m.dolby_surround_mode = rng.next_u32() as u8;
            m.drc_presentation_mode = rng.range(0, 3) as u8;
        }
        let nch = rng.range(0, 8);
        let mode = rng.range(0, 4);
        let none = k % 50 == 0;
        let mut cv = vec![0i32; 82];
        // SAFETY: AacencMetaData has the C layout (aacenc_lib::layout_matches).
        let ec = unsafe {
            c::cref_metadata_main_load(
                if none { core::ptr::null() } else { &m as *const AacencMetaData as *const core::ffi::c_void },
                nch,
                mode,
                cv.as_mut_ptr(),
            )
        };
        let mut r = AacMetadata { center_mix_level: 99, ..Default::default() };
        let er = load_submitted_metadata(if none { None } else { Some(&m) }, nch, mode, &mut r);
        assert_eq!(er, ec);
        assert_eq!(md_to_ints(&r), cv, "LoadSubmittedMetadata case {k}");
        // random DRC/compression values and bands, as the compressor would leave them
        let mut r2 = r;
        r2.mpeg_drc.dyn_rng_ctl[0] = rng.range(0, 127) as u8;
        r2.mpeg_drc.dyn_rng_sgn[0] = rng.range(0, 1) as u8;
        r2.etsi_anc_data.compression_value = rng.next_u32() as u8;
        if k % 7 == 0 {
            r2.mpeg_drc.drc_bands_present = 1;
            r2.mpeg_drc.drc_band_incr = rng.range(0, 1) as u8;
            r2.mpeg_drc.excluded_chns_present = rng.range(0, 1) as u8;
            for i in 0..16 {
                r2.mpeg_drc.drc_band_top[i] = rng.next_u32() as u8;
                r2.mpeg_drc.dyn_rng_ctl[i] = rng.range(0, 127) as u8;
                r2.mpeg_drc.dyn_rng_sgn[i] = rng.range(0, 1) as u8;
            }
        }
        if k % 5 == 0 {
            r2.etsi_anc_data.timecode_coarse_status = rng.range(0, 1);
            r2.etsi_anc_data.timecode_fine_status = rng.range(0, 1);
        }
        let iv = md_to_ints(&r2);
        for which in 0..2 {
            let fill = rng.next_u32() as u8;
            let mut cb = [0u8; 32];
            // SAFETY: shim with a 32-byte buffer.
            let cbits = unsafe { c::cref_metadata_main_write_payload(which, iv.as_ptr(), cb.as_mut_ptr(), fill as i32) };
            let mut mem = [fill; PAYLOAD_MEM_SIZE];
            let off = if which == 0 { 0 } else { 12 };
            let rbits = if which == 0 {
                write_dynamic_range_info_payload(&r2, &mut mem, off)
            } else {
                write_etsi_ancillary_data_payload(&r2, &mut mem, off)
            };
            assert_eq!(rbits, cbits, "payload {which} case {k}: bits");
            let n = (PAYLOAD_MEM_SIZE - off).min(16);
            assert_eq!(&mem[off..off + n], &cb[..n], "payload {which} case {k}: bytes");
        }
    }
}

#[test]
fn tables_and_layout() {
    let (mut dmx, mut lfe, mut sur) = ([0i32; 8], [0i32; 16], [0i32; 8]);
    let mut dflt = [0u8; 84];
    // SAFETY: shim with arrays of the table sizes.
    unsafe { c::cref_metadata_main_tables(dmx.as_mut_ptr(), lfe.as_mut_ptr(), sur.as_mut_ptr(), dflt.as_mut_ptr()) };
    let (rd, rl, rs, rdf) = tables();
    assert_eq!(rd, &dmx);
    assert_eq!(rl, &lfe);
    assert_eq!(rs.iter().map(|&x| x as i32).collect::<Vec<_>>(), sur.to_vec());
    // SAFETY: AacencMetaData is repr(C) with explicit padding fields (52 bytes).
    let rb = unsafe {
        std::slice::from_raw_parts(rdf as *const AacencMetaData as *const u8, core::mem::size_of::<AacencMetaData>())
    };
    assert_eq!(rb, &dflt[..rb.len()]);
    assert_eq!(rb.len(), 52);
    let mut v = [0i32; 4];
    // SAFETY: shim writes 4 values.
    unsafe { c::cref_metadata_main_layout(v.as_mut_ptr()) };
    // drcInfoPayload[12], drcDsePayload[8], matrix_mixdown_idx consecutive (the port's byte
    // image), exPayload after them
    assert_eq!(v[1] - v[0], 12);
    assert_eq!(v[2] - v[0], 20);
    assert!(v[3] - v[0] >= 24);
}

/// The reference and the port's metadata encoder.
struct Md {
    c: *mut core::ffi::c_void,
    r: Option<Box<FdkMetadataEncoder>>,
    max_ch: usize,
}

impl Md {
    fn open(max_ch: u32) -> Md {
        let mut h = core::ptr::null_mut();
        // SAFETY: shim call.
        let ec = unsafe { c::cref_metadata_main_open(&mut h, max_ch) };
        let mut r = None;
        let er = fdk_metadata_enc_open(Some(&mut r), max_ch);
        assert_eq!(er, ec);
        Md { c: h, r, max_ch: max_ch as usize }
    }

    fn check_state(&self, what: &str) {
        let mut v = [0i32; 8];
        let mut delay = vec![0i16; self.max_ch * 2048];
        let mut md = vec![0i32; 82 * 3];
        // SAFETY: shim with buffers of the encoder's sizes.
        unsafe { c::cref_metadata_main_state(self.c, v.as_mut_ptr(), delay.as_mut_ptr(), md.as_mut_ptr()) };
        let r = self.r.as_ref().unwrap();
        assert_eq!(
            [
                r.n_audio_data_delay,
                r.n_meta_data_delay,
                r.n_channels,
                r.channel_mode,
                r.metadata_mode,
                r.meta_data_delay_idx,
                r.finalize_meta_data,
                r.initialize_meta_data
            ],
            v,
            "{what}: state"
        );
        assert!(r.p_audio_delay_buffer == delay, "{what}: audio delay buffer");
        for i in 0..3 {
            assert_eq!(md_to_ints(&r.meta_data_buffer[i]), md[82 * i..82 * (i + 1)].to_vec(), "{what}: metaDataBuffer[{i}]");
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn init(&mut self, reset: i32, mode: i32, delay: i32, fl: u32, sr: u32, nch: u32, cm: ChannelMode, co: i32, what: &str) -> i32 {
        // SAFETY: shim call.
        let ec = unsafe { c::cref_metadata_main_init(self.c, reset, mode, delay, fl, sr, nch, cm, co) };
        let er = fdk_metadata_enc_init(self.r.as_deref_mut(), reset, mode, delay, fl, sr, nch, cm, co);
        assert_eq!(er, ec, "{what}: init");
        self.check_state(what);
        ec
    }

    fn process(&mut self, audio: &[i16], buf: u32, n: i32, meta: Option<&AacencMetaData>, what: &str) {
        let mut ca = audio.to_vec();
        let mut cn = 0;
        let mut ext = [0i32; 6];
        let mut bytes = [0u8; 32];
        let mut cmm = 0;
        // SAFETY: the audio buffer holds the channels with the stride `buf`.
        let ec = unsafe {
            c::cref_metadata_main_process(
                self.c,
                ca.as_mut_ptr(),
                buf,
                n,
                meta.map_or(core::ptr::null(), |m| m as *const AacencMetaData as *const core::ffi::c_void),
                &mut cn,
                ext.as_mut_ptr(),
                bytes.as_mut_ptr(),
                &mut cmm,
            )
        };
        let mut ra = audio.to_vec();
        let mut have = false;
        let mut rn: u32 = (-77i32) as u32;
        let mut rmm = -77;
        let r = self.r.as_deref_mut().unwrap();
        let er = fdk_metadata_enc_process(r, &mut ra, buf, n, meta, &mut have, &mut rn, &mut rmm);
        assert_eq!(er, ec, "{what}: process");
        assert_eq!(rn as i32, cn, "{what}: nMetaDataExtensions");
        assert_eq!(rmm, cmm, "{what}: matrix_mixdown_idx");
        for i in 0..(cn.clamp(0, 2) as usize) {
            let p = r.ex_payload[i];
            assert_eq!([p.data_type, p.data_size as i32, p.associated_ch_element], ext[3 * i..3 * i + 3], "{what}: payload {i}");
            let nb = (p.data_size as usize).div_ceil(8);
            // SAFETY: the payload points into the encoder's payload memory.
            let rb = unsafe { std::slice::from_raw_parts(p.p_data, nb) };
            assert_eq!(rb, &bytes[16 * i..16 * i + nb], "{what}: payload {i} bytes");
        }
        assert!(ra == ca, "{what}: delayed audio");
        // SAFETY: shim call.
        let dc = unsafe { c::cref_metadata_main_get_delay(self.c) };
        assert_eq!(fdk_metadata_enc_get_delay(self.r.as_deref()), dc, "{what}: delay");
        self.check_state(what);
    }
}

impl Drop for Md {
    fn drop(&mut self) {
        // SAFETY: opened in Md::open.
        let ec = unsafe { c::cref_metadata_main_close(self.c) };
        assert_eq!(fdk_metadata_enc_close(Some(&mut self.r)), ec);
    }
}

#[test]
fn encoder_sequences() {
    let mut rng = Rng::new(13);
    // (channel mode, channels)
    let modes: [(ChannelMode, u32); 9] = [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6), (11, 7), (12, 8), (7, 8)];
    let mut payload_bytes = 0;
    let mut overwritten = 0;
    for k in 0..200 {
        let max_ch = rng.range(1, 8) as u32;
        let mut md = Md::open(max_ch);
        let mut inited = false;
        let mut cur_ch = 1u32;
        let buf = 2048 + 100 + 1537;
        let mut meta = random_meta(&mut rng);
        for step in 0..30 {
            let what = format!("sequence {k} step {step}");
            if !inited || rng.range(0, 7) == 0 {
                let (cm, nch) = modes[rng.range(0, 8) as usize];
                let fl = [1024u32, 2048, 512, 480, 960, 256][rng.range(0, 5) as usize];
                // delays of the encoder (DELAY_AAC, with SBR) and random ones, never a
                // multiple of the frame length (a remaining delay of 0 makes the seamless
                // reconfiguration of C read before the delay buffer)
                let mut delay = [1600, 2 * 1600 + 290, 3200 + 545, 1024 + 576][rng.range(0, 3) as usize];
                if rng.range(0, 3) == 0 {
                    delay = rng.range(1, 3 * fl as i32);
                }
                if delay % fl as i32 == 0 {
                    delay += 1;
                }
                let e = md.init(
                    if rng.range(0, 3) == 0 { 1 } else { 0 },
                    rng.range(0, 3),
                    delay,
                    fl,
                    [32000u32, 44100, 48000, 22050][rng.range(0, 3) as usize],
                    nch,
                    cm,
                    rng.range(0, 2),
                    &what,
                );
                if e == METADATA_OK {
                    inited = true;
                    cur_ch = nch;
                } else if !inited {
                    continue;
                }
                let fl = md.r.as_ref().unwrap().n_audio_data_delay;
                let _ = fl;
            }
            if !inited {
                continue;
            }
            if rng.range(0, 4) == 0 {
                meta = random_meta(&mut rng);
            }
            let fl = 1024usize.max(md.r.as_ref().unwrap().n_audio_data_delay as usize + 1).min(2048);
            let mut audio = vec![0i16; buf * cur_ch as usize];
            for ch in 0..cur_ch as usize {
                let amp = [0, 100, 3000, 32767][rng.range(0, 3) as usize];
                for i in 0..fl {
                    audio[ch * buf + i] = if amp == 0 { 0 } else { (rng.next_u32() as i32 % amp) as i16 };
                }
            }
            let pass = rng.range(0, 3) != 0;
            md.process(&audio, buf as u32, (fl * cur_ch as usize) as i32, if pass { Some(&meta) } else { None }, &what);
            let r = md.r.as_ref().unwrap();
            for i in 0..r.n_extensions.clamp(0, 2) as usize {
                payload_bytes += r.ex_payload[i].data_size.div_ceil(8);
                if r.ex_payload[i].data_type == EXT_DATA_ELEMENT && r.ex_payload[i].data_size > 64 {
                    overwritten += 1;
                }
            }
        }
    }
    eprintln!("metadata_main: {payload_bytes} payload bytes, {overwritten} ETSI payloads over 8 bytes (overwriting matrix_mixdown_idx)");
}
