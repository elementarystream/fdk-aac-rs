// Differential tests of fdk_aac_rs_aacenc::bitenc against the C++ reference.
//
// * The file-static writers (global gain, ics_info, MS info, TNS, pulse/gain control, section
//   data, extension payloads, byte alignment) on random inputs, with and without a bitstream.
// * FDKaacEnc_writeExtensionData in counting and writing mode for every payload type and
//   syntax, written into a transport encoder (raw, ADTS), compared bit for bit.
// * FDKaacEnc_ChannelElementWrite on real encoder states (the reference encoder of
//   adj_thr_enc.rs after FDKaacEnc_QCMain): counting mode (static bits, minimal static bits,
//   from PSY only) and writing mode into raw and ADTS transport encoders with CRC.
// FDKaacEnc_WriteBitstream writes whole access units; it is compared byte by byte through the
// API (aacenc_lib.rs) and the end-to-end corpus.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use crate::adj_thr_enc::*;
use core::ffi::c_void;
use fdk_aac_rs_aacenc::aacenc_tns::TnsInfo;
use fdk_aac_rs_aacenc::bitenc::*;
use fdk_aac_rs_aacenc::dyn_bits::{SectionData, MAX_SECTIONS};
use fdk_aac_rs_aacenc::interface::*;
use fdk_aac_rs_aacenc::qc_data::*;
use fdk_aac_rs_cref::aacenc_bitenc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream, BS_WRITER};
use fdk_aac_rs_tpenc::tpenc_lib::*;

/// Callbacks for the port's transport encoder (none are called for raw and ADTS).
pub struct NoCb;
impl TpEncCallbacks for NoCb {
    fn has_sbr(&self) -> bool {
        false
    }
    fn sbr(
        &mut self,
        _: &mut FdkBitStream,
        _: i32,
        _: i32,
        _: i32,
        _: AudioObjectType,
        _: Mp4ElementId,
        _: i32,
        _: u8,
        _: u8,
        _: u8,
        _: Option<&mut u8>,
        _: i32,
    ) -> i32 {
        0
    }
    fn has_ssc(&self) -> bool {
        false
    }
    fn ssc(
        &mut self,
        _: Option<&mut FdkBitStream>,
        _: AudioObjectType,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: u8,
        _: Option<&mut u8>,
    ) -> i32 {
        0
    }
}

/// A reference and a port transport encoder with the same configuration.
pub struct Tp {
    pub c: *mut c_void,
    pub r: Box<TransportEnc>,
    buf: BitMem,
}

impl Tp {
    /// cfg = {aot, channelMode, samplingRate, samplesPerFrame, noChannels, flags, headerPeriod,
    /// bitRate}
    pub fn open(fmt: TransportType, cfg: [i32; 8]) -> Tp {
        // SAFETY: shim call.
        let ch = unsafe { c::cref_bitenc_tp_open(fmt, cfg.as_ptr()) };
        assert!(!ch.is_null(), "C transport init {fmt} {cfg:?}");
        let mut h = None;
        transport_enc_open(&mut h);
        let mut r = h.unwrap();
        let mut cc = CoderConfig::default();
        cc.aot = cfg[0];
        cc.channel_mode = cfg[1];
        cc.sampling_rate = cfg[2];
        cc.ext_sampling_rate = cfg[2];
        cc.samples_per_frame = cfg[3];
        cc.no_channels = cfg[4];
        cc.flags = cfg[5] as u32;
        cc.header_period = cfg[6] as u8;
        cc.bit_rate = cfg[7];
        cc.n_sub_frames = 1;
        let buf = BitMem::new(8192);
        let e = transport_enc_init_cb(&mut r, &buf, 8192, fmt, &cc, 0, &mut NoCb);
        assert_eq!(e, 0, "port transport init");
        Tp { c: ch, r, buf }
    }

    pub fn check(&mut self, what: &str) {
        let mut cb = vec![0u8; 8192];
        // SAFETY: shim call with an 8192-byte buffer.
        let vc = unsafe { c::cref_bitenc_tp_bits(self.c, cb.as_mut_ptr(), 8192) };
        let vr = fdk_get_valid_bits(&mut self.r.bit_stream) as i32;
        assert_eq!(vr, vc, "{what}: valid bits");
        let rb = self.buf.to_vec();
        let n = ((vc as usize).div_ceil(8) + 4).min(8192);
        if let Some(k) = (0..n).find(|&k| rb[k] != cb[k]) {
            panic!("{what}: bitstream byte {k} differs ({} vs {})", rb[k], cb[k]);
        }
    }

    fn write_au(&mut self, bits: i32, fullness: i32, ncc: i32) {
        // SAFETY: shim call.
        let ec = unsafe { c::cref_bitenc_tp_write_au(self.c, bits, fullness, ncc) };
        let er = transport_enc_write_access_unit_cb(Some(&mut self.r), bits, fullness, ncc, &mut NoCb);
        assert_eq!(er, ec);
    }

    fn end_au(&mut self, bits: i32) {
        // SAFETY: shim call.
        let bc = unsafe { c::cref_bitenc_tp_end_au(self.c, bits) };
        let mut br = bits;
        transport_enc_end_access_unit(&mut self.r, &mut br);
        assert_eq!(br, bc);
    }
}

impl Drop for Tp {
    fn drop(&mut self) {
        // SAFETY: opened in Tp::open.
        unsafe { c::cref_bitenc_tp_close(self.c) };
    }
}

const LC_STEREO: [i32; 8] = [2, 2, 44100, 1024, 2, 0, 0, 128000];

#[test]
fn sizes_match() {
    let mut v = [0i32; 6];
    // SAFETY: shim writes 6 values.
    unsafe { c::cref_bitenc_sizes(v.as_mut_ptr()) };
    assert_eq!(core::mem::size_of::<PsyOutChannel>() as i32, v[0], "PSY_OUT_CHANNEL prefix");
    assert_eq!(core::mem::size_of::<QcOutChannel>() as i32, v[1]);
    assert_eq!(core::mem::size_of::<ElementInfo>() as i32, v[2]);
    assert_eq!(core::mem::size_of::<ToolsInfo>() as i32, v[3]);
    assert_eq!(core::mem::size_of::<TnsInfo>() as i32, v[4]);
    assert_eq!(core::mem::size_of::<SectionData>() as i32, v[5]);
}

/// Runs one static writer on both sides; returns the C result.
#[allow(clippy::too_many_arguments)]
fn static_case(
    which: i32,
    use_bs: bool,
    a: &[i32],
    ints: &[i32],
    tns: Option<&TnsInfo>,
    sd: Option<&SectionData>,
    bytes: &[u8],
    what: &str,
) {
    let mut a8 = [0i32; 8];
    a8[..a.len()].copy_from_slice(a);
    let mut ints60 = [0i32; 64];
    ints60[..ints.len()].copy_from_slice(ints);
    let mut cbuf = vec![0u8; 1024];
    let mut cvalid = 0i32;
    let ptr: *const c_void = match (tns, sd) {
        (Some(t), _) => t as *const TnsInfo as *const c_void,
        (_, Some(s)) => s as *const SectionData as *const c_void,
        _ => core::ptr::null(),
    };
    let mut bytes_pad = bytes.to_vec();
    bytes_pad.extend_from_slice(&[0u8; 8]);
    // SAFETY: shim with valid buffers.
    let rc = unsafe {
        c::cref_bitenc_static(
            which,
            use_bs as i32,
            a8.as_ptr(),
            ints60.as_ptr(),
            ptr,
            bytes_pad.as_ptr(),
            cbuf.as_mut_ptr(),
            &mut cvalid,
        )
    };
    let rmem = BitMem::new(1024);
    let mut bs = FdkBitStream::default();
    fdk_init_bit_stream(&mut bs, &rmem, 1024, 0, BS_WRITER);
    let h = if use_bs { Some(&mut bs) } else { None };
    let rr = match which {
        0 => fdk_aac_enc_encode_global_gain(a8[0], a8[1], h, a8[2]),
        1 => fdk_aac_enc_encode_ics_info(a8[0], a8[1], a8[2], a8[3], h, a8[4] as u32),
        2 => fdk_aac_enc_encode_ms_info(a8[0], a8[1], a8[2], a8[3], &ints60, h),
        3 => fdk_aac_enc_encode_tns_data_present(tns, a8[0], h),
        4 => fdk_aac_enc_encode_tns_data(tns, a8[0], h),
        5 => fdk_aac_enc_encode_gain_control_data(h),
        6 => fdk_aac_enc_encode_pulse_data(h),
        // SAFETY: the payload is padded beyond the bytes the writer reads.
        7 => unsafe { fdk_aac_enc_write_extension_payload(h, a8[0], bytes_pad.as_ptr(), a8[1]) },
        8 => fdk_aac_enc_encode_section_data(sd, h, a8[0] as u32),
        _ => {
            if let Some(h) = h {
                fdk_aac_enc_byte_alignment(h, a8[0]);
            }
            0
        }
    };
    let rvalid = fdk_get_valid_bits(&mut bs) as i32;
    assert_eq!(rr, rc, "{what}: return");
    assert_eq!(rvalid, cvalid, "{what}: bits");
    assert_eq!(rmem.to_vec(), cbuf, "{what}: bytes");
}

fn random_tns(rng: &mut Rng, short: bool) -> TnsInfo {
    let mut t = TnsInfo::default();
    let nw = if short { 8 } else { 1 };
    for w in 0..nw {
        t.num_of_filters[w] = rng.range(0, if short { 1 } else { 2 });
        t.coef_res[w] = rng.range(3, 4);
        for f in 0..t.num_of_filters[w] as usize {
            t.length[w][f] = rng.range(0, if short { 15 } else { 63 });
            t.order[w][f] = rng.range(0, if short { 7 } else { 12 });
            t.direction[w][f] = rng.range(0, 1);
            let (lo, hi) = if t.coef_res[w] == 4 {
                if rng.range(0, 1) == 0 { (-4, 3) } else { (-8, 7) }
            } else if rng.range(0, 1) == 0 {
                (-2, 1)
            } else {
                (-4, 3)
            };
            for k in 0..t.order[w][f] as usize {
                t.coef[w][f][k] = rng.range(lo, hi);
            }
        }
    }
    t
}

#[test]
fn static_writers() {
    let mut rng = Rng::new(31);
    let flags = [0u32, AC_ER, AC_ER | AC_ELD, AC_SCALABLE, AC_ELD, AC_ER | AC_LD];
    for k in 0..4000 {
        let use_bs = rng.range(0, 3) != 0;
        let what = format!("case {k}");
        match k % 10 {
            0 => static_case(0, use_bs, &[rng.range(0, 255), rng.range(-100, 255), rng.range(-5, 30)], &[], None, None, &[], &what),
            1 => static_case(
                1,
                use_bs,
                &[rng.range(0, 5), rng.range(0, 3), rng.range(0, 127), rng.range(0, 63), flags[rng.range(0, 5) as usize] as i32],
                &[],
                None,
                None,
                &[],
                &what,
            ),
            2 => {
                let grp = rng.range(1, 15);
                let ng = rng.range(1, 60 / grp);
                let js: Vec<i32> = (0..60).map(|_| rng.range(0, 3)).collect();
                static_case(2, use_bs, &[grp * ng, grp, rng.range(0, grp), rng.range(0, 3)], &js, None, None, &[], &what);
            }
            3 | 4 => {
                let short = rng.range(0, 1) == 0;
                let t = random_tns(&mut rng, short);
                let bt = if short { 2 } else { [0, 1, 3][rng.range(0, 2) as usize] };
                let none = rng.range(0, 9) == 0;
                static_case((k % 10) as i32, use_bs, &[bt], &[], if none { None } else { Some(&t) }, None, &[], &what);
            }
            5 | 6 => static_case((k % 10) as i32, use_bs, &[], &[], None, None, &[], &what),
            7 => {
                let types = [0, 1, 2, 9, 11, 13, 14, 3, 10, 12];
                let ty = types[rng.range(0, 9) as usize];
                let bits = rng.range(0, 2000);
                let bytes: Vec<u8> = (0..(bits / 8 + 2)).map(|_| rng.next_u32() as u8).collect();
                static_case(7, use_bs, &[ty, bits], &[], None, None, &bytes, &what);
            }
            8 => {
                let mut sd = SectionData::default();
                sd.block_type = rng.range(0, 3);
                sd.no_of_sections = rng.range(0, MAX_SECTIONS as i32);
                for i in 0..sd.no_of_sections as usize {
                    sd.huffsection[i].code_book = rng.range(0, 15);
                    sd.huffsection[i].sfb_cnt = rng.range(0, 60);
                }
                static_case(8, use_bs, &[rng.range(0, 1)], &[], None, Some(&sd), &[], &what);
            }
            _ => static_case(9, use_bs, &[rng.range(0, 7)], &[], None, None, &[], &what),
        }
    }
}

/// FDKaacEnc_writeExtensionData in counting mode and into transport encoders.
#[test]
fn write_extension_data() {
    let mut rng = Rng::new(77);
    let types = [EXT_FIL, EXT_FILL_DATA, EXT_DATA_ELEMENT, EXT_DYNAMIC_RANGE, EXT_SBR_DATA, EXT_SBR_DATA_CRC, EXT_LDSAC_DATA, EXT_SAC_DATA, 3, 15];
    let flags = [0u32, AC_ER, AC_ER | AC_ELD, AC_SCALABLE, AC_ER | AC_LD, AC_ELD];
    for (fmt, cfg) in [(TT_MP4_RAW, LC_STEREO), (TT_MP4_ADTS, LC_STEREO)] {
        let mut tp = Tp::open(fmt, cfg);
        let mut total = 0;
        for k in 0..3000 {
            let ty = types[rng.range(0, types.len() as i32 - 1) as usize];
            let bits = match rng.range(0, 3) {
                0 => rng.range(0, 64),
                1 => rng.range(0, 1000),
                _ => rng.range(0, 6000),
            };
            let fl = flags[rng.range(0, flags.len() as i32 - 1) as usize];
            let tag = rng.range(0, 15);
            let anchor = rng.range(0, 64) as u32;
            let mut payload: Vec<u8> = (0..(bits / 8 + 520)).map(|_| rng.next_u32() as u8).collect();
            let ext = QcOutExtension { type_: ty, n_payload_bits: bits, p_payload: payload.as_ptr() };
            let what = format!("fmt {fmt} case {k}: type {ty} bits {bits} flags {fl:#x}");
            // counting mode
            // SAFETY: shim / no bitstream.
            let cc = unsafe { c::cref_bitenc_write_ext(core::ptr::null_mut(), ty, bits, payload.as_mut_ptr(), tag, anchor, fl, 2, -1) };
            let rc = unsafe { fdk_aac_enc_write_extension_data(None, &ext, tag, anchor, fl, 2, -1) };
            assert_eq!(rc, cc, "{what}: counted bits");
            // writing mode
            if total + cc > 40000 {
                tp = Tp::open(fmt, cfg);
                total = 0;
            }
            // SAFETY: the payload holds the bytes the writer reads (a DSE reads up to 510
            // bytes from its start).
            let cw = unsafe { c::cref_bitenc_write_ext(tp.c, ty, bits, payload.as_mut_ptr(), tag, anchor, fl, 2, -1) };
            let rw = unsafe { fdk_aac_enc_write_extension_data(Some(&mut tp.r), &ext, tag, anchor, fl, 2, -1) };
            assert_eq!(rw, cw, "{what}: written bits");
            total += cw.max(0);
            tp.check(&what);
        }
    }
}

/// FDKaacEnc_ChannelElementWrite on real encoder states.
#[test]
fn channel_element_write_real() {
    let mut counted = 0;
    let mut written = 0;
    for (k, cfg) in config_subset(61, 60, |_| true).iter().enumerate() {
        let Ok(mut e) = CEnc::open(cfg) else { continue };
        let cm = e.channel_mapping();
        let sig = SIGNALS[k % SIGNALS.len()];
        let pcm = pcm_frames(sig, cfg.sample_rate, e.n_channels(), e.frame_length(), 6);
        for (f, frame) in pcm.iter().enumerate() {
            e.begin_frame(frame);
            if e.run_to(STAGE_FINALIZE, 0) != 0 || e.next().0 != STAGE_FINALIZE {
                break;
            }
            let qc = e.qc_out();
            let psy = e.psy_out();
            for el in 0..cm.n_elements as usize {
                let info = cm.el_info[el];
                if !matches!(info.el_type, ID_SCE | ID_CPE | ID_LFE) {
                    continue;
                }
                let what = format!("{cfg:?} {sig:?} frame {f} element {el}");
                let n = info.n_channels_in_el as usize;
                let pe = psy.psy_out_element[el];
                let p0 = pe.psy_out_channel[0];
                let q0 = qc.qc_element[el].qc_out_channel[0];
                let mut pch = [PsyOutChannel::default(); 2];
                let mut qch = [QcOutChannel::default(); 2];
                pch[..n].copy_from_slice(&psy.p_psy_out_channels[p0..p0 + n]);
                qch[..n].copy_from_slice(&qc.p_qc_out_channels[q0..q0 + n]);
                let psize = core::mem::size_of::<PsyOutChannel>();
                // SAFETY: PsyOutChannel is repr(C) without padding.
                let pbytes: Vec<u8> = unsafe {
                    std::slice::from_raw_parts(pch.as_ptr() as *const u8, 2 * psize)
                }
                .to_vec();
                let (aot, flags, ep) = (e.aot(), e.syntax_flags(), e.ep_config());
                let call_c = |t: *mut c_void, with_qc: bool, min_cnt: i32, qch: &mut [QcOutChannel; 2]| {
                    let mut bd = -1;
                    // SAFETY: byte images with the C layouts (sizes_match).
                    let r = unsafe {
                        c::cref_bitenc_element_write(
                            t,
                            &info as *const ElementInfo as *const c_void,
                            if with_qc { qch.as_mut_ptr() as *mut c_void } else { core::ptr::null_mut() },
                            pe.common_window,
                            &pe.tools_info as *const ToolsInfo as *const c_void,
                            pbytes.as_ptr(),
                            psize as i32,
                            flags,
                            aot,
                            ep as i32,
                            &mut bd,
                            min_cnt,
                        )
                    };
                    (r, bd)
                };
                // counting modes
                for (with_qc, min_cnt) in [(false, 0), (false, 1), (true, 0), (true, 1)] {
                    let (ec, bc) = call_c(core::ptr::null_mut(), with_qc, min_cnt, &mut qch.clone());
                    let mut br = -1;
                    let er = fdk_aac_enc_channel_element_write(
                        None,
                        &info,
                        if with_qc { Some(&qch[..n.max(1)]) } else { None },
                        &pe,
                        &pch[..n.max(1)],
                        flags,
                        aot,
                        ep,
                        Some(&mut br),
                        min_cnt as u8,
                    );
                    assert_eq!((er, br), (ec, bc), "{what}: counting qc {with_qc} minCnt {min_cnt}");
                    counted += 1;
                }
                // writing mode into raw and ADTS (with CRC) transport encoders
                if aot == AOT_AAC_LC || aot == AOT_ER_AAC_LD || aot == AOT_ER_AAC_ELD {
                    for (fmt, prot) in [(TT_MP4_RAW, 0), (TT_MP4_ADTS, 1)] {
                        if fmt == TT_MP4_ADTS && aot != AOT_AAC_LC {
                            continue;
                        }
                        let cfg8 = [AOT_AAC_LC, MODE_2, 44100, 1024, 2, if prot != 0 { CC_PROTECTION as i32 } else { 0 }, 0, 128000];
                        let mut tp = Tp::open(fmt, cfg8);
                        tp.write_au(8 * 700, 2000, 2);
                        let (ec, bc) = call_c(tp.c, true, 0, &mut qch.clone());
                        let mut br = -1;
                        let er = fdk_aac_enc_channel_element_write(
                            Some(&mut tp.r),
                            &info,
                            Some(&qch[..n.max(1)]),
                            &pe,
                            &pch[..n.max(1)],
                            flags,
                            aot,
                            ep,
                            Some(&mut br),
                            0,
                        );
                        assert_eq!((er, br), (ec, bc), "{what}: writing fmt {fmt}");
                        tp.check(&what);
                        let bits = fdk_get_valid_bits(&mut tp.r.bit_stream) as i32;
                        tp.end_au(bits);
                        tp.check(&format!("{what} after the end of the access unit"));
                        written += 1;
                    }
                }
            }
            if e.finish().is_err() {
                break;
            }
        }
    }
    eprintln!("bitenc: {counted} counted, {written} written elements");
    assert!(counted > 0 && written > 0);
}
