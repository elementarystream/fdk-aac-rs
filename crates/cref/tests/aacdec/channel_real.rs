// Differential tests of the channel modules (with the HCR, RVLC and LPD functions that they
// call) on real streams: the ISO/IEC 14496-26 conformance streams (LOAS: AAC-LC with up to 6
// channels, ER AAC-LC, ER AAC-LD with 512 and 480 samples, each combination of the ER tools VCB11,
// RVLC and HCR; MP4: AAC-LC with 960 samples, ER AAC-ELD with the ER tools, USAC) and the
// ISO/IEC 23003-7 USAC streams with LPD coding and core coder transitions (Lpd_*, Cct_*) in
// work/conformance of the main checkout. The streams are not part of the repository; the tests do
// nothing if they are missing.
//
// Every access unit is parsed into its channel elements as the decoder does (element IDs of the
// raw_data_block for AAC, the implicit element order of the channel configuration for ER, the
// elements of the UsacConfig for USAC), and each element goes through CChannelElement_Read,
// CChannelElement_Decode, CConcealment_Apply, CBlock_FrequencyToTime /
// CBlock_FrequencyToTimeLowDelay / CLpd_RenderTimeSignal and CConcealment_TDFading on both
// sides, with one block test context per element that keeps the state of its channels across the
// frames. Random access units are dropped to exercise the concealment. The complete state
// (channel info, spectra, IMDCT, LPD and concealment state, output) and the bit position are
// compared after every step. The state is never copied between the sides after the start, so the
// memory that the image does not contain (the HCR/RVLC overlay of pComData, the FAC ZIR that
// IMdct.pFacZir points to) evolves on each side by its own calls.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use crate::block_ctx::*;
use fdk_aac_rs_aacdec::aac_rom::ELEMENTS_TAB;
use fdk_aac_rs_aacdec::aacdec_pns::cpns_reset_data;
use fdk_aac_rs_aacdec::block::{cblock_frequency_to_time, cblock_frequency_to_time_low_delay};
use fdk_aac_rs_aacdec::channel::*;
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_aacdec::conceal::*;
use fdk_aac_rs_aacdec::usacdec_lpd::clpd_render_time_signal;
use fdk_aac_rs_cref::aacdec_aacdec_pns as cpns;
use fdk_aac_rs_cref::aacdec_block as cblk;
use fdk_aac_rs_cref::aacdec_channel as c;
use fdk_aac_rs_cref::aacdec_conceal as ccc;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{fdk_push_for, fdk_read_bits, FdkBitStream};
use fdk_aac_rs_tpdec::tp_data::CSAudioSpecificConfig;
use fdk_aac_rs_tpdec::tpdec_lib::*;
use std::path::PathBuf;

/// The number of compared steps per kind.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    /// CChannelElement_Read
    reads: u32,
    /// ... of RVLC elements (CRvlc_Read, CRvlc_Decode)
    reads_rvlc: u32,
    /// ... of HCR elements (CHcr_Read, HcrInit, HcrDecoder)
    reads_hcr: u32,
    /// LPD channels read (CLpdChannelStream_Read)
    reads_lpd: u32,
    /// FD channels read after an LPD frame (fac_data, CLpd_FAC_Read)
    reads_fd_after_lpd: u32,
    /// CChannelElement_Decode
    decodes: u32,
    /// LPD channels decoded (CLpdChannelStream_Decode)
    decodes_lpd: u32,
    /// CConcealment_Apply
    conceals: u32,
    /// ... of bad frames in the IMDCT render mode (CLpc_Conceal)
    conceals_lpc: u32,
    /// CBlock_FrequencyToTime
    f2t: u32,
    /// ... after an LPD frame (FAC transition or IMDCT after TCX, bass post-filter)
    f2t_lpd: u32,
    /// CBlock_FrequencyToTimeLowDelay
    f2t_ld: u32,
    /// CLpd_RenderTimeSignal
    lpd_render: u32,
    /// CConcealment_TDFading
    td_fading: u32,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.reads += o.reads;
        self.reads_rvlc += o.reads_rvlc;
        self.reads_hcr += o.reads_hcr;
        self.reads_lpd += o.reads_lpd;
        self.reads_fd_after_lpd += o.reads_fd_after_lpd;
        self.decodes += o.decodes;
        self.decodes_lpd += o.decodes_lpd;
        self.conceals += o.conceals;
        self.conceals_lpc += o.conceals_lpc;
        self.f2t += o.f2t;
        self.f2t_lpd += o.f2t_lpd;
        self.f2t_ld += o.f2t_ld;
        self.lpd_render += o.lpd_render;
        self.td_fading += o.td_fading;
    }
}

/// work/ of the main checkout (the cref library lies in work/build-cref/...).
fn work_dir() -> Option<PathBuf> {
    if let Some(w) = std::env::var_os("FDK_AAC_RS_WORK_DIR") {
        return Some(PathBuf::from(w));
    }
    let lib = PathBuf::from(env!("FDK_AAC_RS_CREF_LIB"));
    lib.ancestors().find(|a| a.file_name().is_some_and(|n| n == "build-cref")).and_then(|b| b.parent()).map(|p| p.to_path_buf())
}

fn conformance(name: &str) -> Option<Vec<u8>> {
    let w = work_dir()?;
    if name.contains('/') {
        return std::fs::read(w.join("conformance").join(name)).ok();
    }
    for sub in ["compressedAss/add-opt", "compressedAss", "compressedMp4", "compressedAdts/add-opt"] {
        let p = w.join("conformance/14496-26").join(sub).join(name);
        if let Ok(d) = std::fs::read(&p) {
            return Some(d);
        }
    }
    None
}

/// The access units of a LOAS stream and its configuration (through the transport decoder).
fn loas_aus(data: &[u8], max: usize) -> Option<(CSAudioSpecificConfig, Vec<Vec<u8>>)> {
    let mut tp = transport_dec_open(TT_MP4_LOAS, TP_FLAG_MPEG4, 1)?;
    let mut aus = Vec::new();
    let mut pos = 0usize;
    let mut asc = None;
    while aus.len() < max {
        if pos < data.len() {
            let chunk = &data[pos..(pos + 4096).min(data.len())];
            let mut valid = chunk.len() as u32;
            transport_dec_fill_data(&mut tp, chunk, chunk.len() as u32, &mut valid, 0);
            pos += chunk.len() - valid as usize;
        }
        let e = transport_dec_read_access_unit(&mut tp, 0);
        if e == TRANSPORTDEC_NOT_ENOUGH_BITS {
            if pos >= data.len() {
                break;
            }
            continue;
        }
        if e != TRANSPORTDEC_OK {
            if pos >= data.len() {
                break;
            }
            continue;
        }
        let mut a = CSAudioSpecificConfig::zeroed();
        transport_dec_get_asc(&tp, 0, &mut a);
        if asc.is_none() {
            asc = Some(a);
        }
        let bits = transport_dec_get_au_bits_remaining(&mut tp, 0).max(0) as usize;
        let bs = transport_dec_get_bitstream(&mut tp, 0);
        let mut au = Vec::with_capacity(bits / 8 + 1);
        let mut left = bits;
        while left > 0 {
            let n = left.min(8);
            au.push((fdk_read_bits(bs, n as u32) << (8 - n)) as u8);
            left -= n;
        }
        aus.push(au);
        transport_dec_end_access_unit(&mut tp);
    }
    Some((asc?, aus))
}

// --- a minimal MP4 demuxer (first audio track) ---

fn be32(d: &[u8], i: usize) -> u64 {
    d.get(i..i + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64)
}

fn boxes(d: &[u8], mut i: usize, end: usize) -> Vec<(String, usize, usize)> {
    let mut v = Vec::new();
    while i + 8 <= end {
        let mut size = be32(d, i) as usize;
        let typ = String::from_utf8_lossy(&d[i + 4..i + 8]).to_string();
        let mut hdr = 8;
        if size == 1 {
            size = (be32(d, i + 8) << 32 | be32(d, i + 12)) as usize;
            hdr = 16;
        } else if size == 0 {
            size = end - i;
        }
        if size < hdr || i + size > end {
            break;
        }
        v.push((typ, i + hdr, i + size));
        i += size;
    }
    v
}

fn find<'a>(v: &'a [(String, usize, usize)], t: &str) -> Option<&'a (String, usize, usize)> {
    v.iter().find(|(n, _, _)| n == t)
}

fn descriptor(d: &[u8], i: &mut usize) -> (u8, usize) {
    let tag = d[*i];
    *i += 1;
    let mut len = 0usize;
    for _ in 0..4 {
        let b = d[*i];
        *i += 1;
        len = (len << 7) | (b & 0x7f) as usize;
        if b & 0x80 == 0 {
            break;
        }
    }
    (tag, len)
}

/// The AudioSpecificConfig and the access units of the first audio track of an MP4 file.
fn mp4_aus(d: &[u8], max: usize) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let top = boxes(d, 0, d.len());
    let moov = find(&top, "moov")?;
    for trak in boxes(d, moov.1, moov.2).iter().filter(|b| b.0 == "trak") {
        let mdia = boxes(d, trak.1, trak.2);
        let Some(mdia) = find(&mdia, "mdia") else { continue };
        let minf = boxes(d, mdia.1, mdia.2);
        let Some(minf) = find(&minf, "minf") else { continue };
        let stbl = boxes(d, minf.1, minf.2);
        let Some(stbl) = find(&stbl, "stbl") else { continue };
        let st = boxes(d, stbl.1, stbl.2);
        let stsd = find(&st, "stsd")?;
        let entries = boxes(d, stsd.1 + 8, stsd.2);
        let Some(mp4a) = find(&entries, "mp4a") else { continue };
        let ver = (be32(d, mp4a.1 + 8) >> 16) as u16;
        let skip = match ver {
            1 => 28 + 16,
            2 => 28 + 36,
            _ => 28,
        };
        let sub = boxes(d, mp4a.1 + skip, mp4a.2);
        let esds = find(&sub, "esds")?;
        let mut i = esds.1 + 4;
        let (tag, _) = descriptor(d, &mut i);
        if tag != 3 {
            return None;
        }
        i += 2;
        let fl = d[i];
        i += 1;
        if fl & 0x80 != 0 {
            i += 2;
        }
        if fl & 0x40 != 0 {
            i += 1 + d[i] as usize;
        }
        if fl & 0x20 != 0 {
            i += 2;
        }
        let (tag, _) = descriptor(d, &mut i);
        if tag != 4 {
            return None;
        }
        i += 13;
        let (tag, len) = descriptor(d, &mut i);
        if tag != 5 {
            return None;
        }
        let asc = d[i..i + len].to_vec();

        let stsz = find(&st, "stsz")?;
        let fixed = be32(d, stsz.1 + 4) as usize;
        let count = be32(d, stsz.1 + 8) as usize;
        let sizes: Vec<usize> =
            (0..count).map(|k| if fixed != 0 { fixed } else { be32(d, stsz.1 + 12 + 4 * k) as usize }).collect();
        let offsets: Vec<usize> = if let Some(stco) = find(&st, "stco") {
            (0..be32(d, stco.1 + 4) as usize).map(|k| be32(d, stco.1 + 8 + 4 * k) as usize).collect()
        } else {
            let co = find(&st, "co64")?;
            (0..be32(d, co.1 + 4) as usize)
                .map(|k| (be32(d, co.1 + 8 + 8 * k) << 32 | be32(d, co.1 + 12 + 8 * k)) as usize)
                .collect()
        };
        let stsc = find(&st, "stsc")?;
        let runs: Vec<(usize, usize)> = (0..be32(d, stsc.1 + 4) as usize)
            .map(|k| (be32(d, stsc.1 + 8 + 12 * k) as usize, be32(d, stsc.1 + 12 + 12 * k) as usize))
            .collect();
        let mut aus = Vec::new();
        let mut sample = 0;
        for (ci, &off) in offsets.iter().enumerate() {
            let chunk = ci + 1;
            let spc = runs.iter().rev().find(|r| r.0 <= chunk).map_or(1, |r| r.1);
            let mut o = off;
            for _ in 0..spc {
                if sample >= sizes.len() || aus.len() >= max {
                    break;
                }
                let s = sizes[sample];
                if o + s > d.len() {
                    return Some((asc, aus));
                }
                aus.push(d[o..o + s].to_vec());
                o += s;
                sample += 1;
            }
        }
        return Some((asc, aus));
    }
    None
}

fn parse_asc(asc: &[u8]) -> Option<CSAudioSpecificConfig> {
    let mut tp = transport_dec_open(TT_MP4_RAW, TP_FLAG_MPEG4, 1)?;
    if transport_dec_out_of_band_config(&mut tp, asc, asc.len() as u32, 0) != TRANSPORTDEC_OK {
        return None;
    }
    let mut a = CSAudioSpecificConfig::zeroed();
    transport_dec_get_asc(&tp, 0, &mut a);
    Some(a)
}

/// A channel element of the stream.
struct Elem {
    id: Mp4ElementId,
    pair: Pair,
}

struct Dec {
    aot: i32,
    flags: u32,
    ep: i32,
    spf: u32,
    sri: u32,
    rate: u32,
    usac: bool,
    els: Vec<Elem>,
    asc: CSAudioSpecificConfig,
    name: String,
    lost: u32,
    n: Counts,
}

impl Dec {
    fn new(name: &str, asc: CSAudioSpecificConfig) -> Dec {
        let aot = asc.m_aot;
        let mut flags = 0;
        flags |= if asc.m_vcb11_flag != 0 { AC_ER_VCB11 } else { 0 };
        flags |= if asc.m_rvlc_flag != 0 { AC_ER_RVLC } else { 0 };
        flags |= if asc.m_hcr_flag != 0 { AC_ER_HCR } else { 0 };
        if aot == AOT_ER_AAC_ELD {
            flags |= AC_ELD;
        }
        if aot == AOT_ER_AAC_LD {
            flags |= AC_LD;
        }
        if asc.m_ep_config >= 0 {
            flags |= AC_ER;
        }
        if aot == AOT_USAC {
            flags |= AC_USAC;
        }
        Dec {
            aot,
            flags,
            ep: asc.m_ep_config as i32,
            spf: asc.m_samples_per_frame,
            sri: asc.m_sampling_frequency_index as u32,
            rate: asc.m_sampling_frequency,
            usac: aot == AOT_USAC,
            els: Vec::new(),
            asc,
            name: name.to_string(),
            lost: 0,
            n: Counts::default(),
        }
    }

    fn el_flags(&self, idx: usize) -> u32 {
        if !self.usac {
            return 0;
        }
        let e = &self.asc.m_sc.m_usac_config.element[idx];
        let mut f = 0;
        if e.m_noise_filling != 0 {
            f |= AC_EL_USAC_NOISE;
        }
        if e.m_stereo_config_index > 0 {
            f |= AC_EL_USAC_MPS212;
        }
        if e.m_inter_tes != 0 {
            f |= AC_EL_USAC_ITES;
        }
        if e.m_pvc != 0 {
            f |= AC_EL_USAC_PVC;
        }
        if e.usac_element_type == ID_USAC_LFE {
            f |= AC_EL_USAC_LFE | AC_EL_LFE;
        }
        if e.usac_element_type == ID_USAC_CPE && e.m_stereo_config_index == 0 {
            f |= AC_EL_USAC_CP_POSSIBLE;
        }
        f
    }

    /// The context of element `idx` (created as the decoder initializes its channels).
    fn elem(&mut self, idx: usize, id: Mp4ElementId, el_flags: u32) -> &mut Pair {
        while self.els.len() <= idx {
            let mut p = Pair::new();
            p.r.usac = self.usac;
            assert_eq!(p.set_sri(self.spf, self.sri, self.rate), 0, "{}: sampling rate info", self.name);
            init_channels(&mut p, self.spf);
            cconcealment_init_common_data(Some(&mut p.r.params));
            let rm = if self.flags & AC_ELD != 0 { AACDEC_RENDER_ELDFB } else { AACDEC_RENDER_IMDCT };
            for ch in 0..2 {
                cconcealment_init_channel_data(&mut p.r.sci[ch].concealment_info, rm, self.spf as i32);
                p.r.ci[ch].render_mode = rm;
                if self.usac {
                    p.r.sci[ch].h_ar_co = Some(Box::default());
                    p.r.sci[ch].last_lpd_mode = 255;
                }
            }
            let cp = el_flags & AC_EL_USAC_CP_POSSIBLE != 0;
            p.r.sci[0].p_cpe_static_data = if cp { Some(0) } else { None };
            p.r.cplx_present = cp;
            p.sync();
            self.els.push(Elem { id, pair: p });
        }
        &mut self.els[idx].pair
    }

    /// Reads element `idx` of type `id` at bit `pos` of the access unit; returns the error and the
    /// bits consumed.
    fn read(&mut self, idx: usize, id: Mp4ElementId, au: &[u8], pos: usize, what: &str) -> (i32, usize) {
        let el_flags = self.el_flags(idx);
        let (aot, flags, ep, spf, usac) = (self.aot, self.flags, self.ep, self.spf, self.usac);
        let nch: u8 = if id == ID_CPE || id == ID_USAC_CPE { 2 } else { 1 };
        let p = self.elem(idx, id, el_flags);
        let buf = au.len().next_power_of_two().max(64);
        p.set_bs(au, buf, (au.len() * 8) as u32);
        fdk_push_for(&mut p.r.bs, pos as u32);
        unsafe { c::cref_channel_bs_skip(p.c.0, pos as i32) };
        if !usac {
            for ch in 0..nch as usize {
                unsafe { cpns::cref_pns_reset_data(p.c.0, ch as i32) };
                cpns_reset_data(&mut p.r.ci[ch].data.aac.pns_data, &mut p.r.com.pns_inter_channel_data);
            }
        }
        let after_lpd: Vec<bool> = (0..nch as usize).map(|ch| p.r.sci[ch].last_core_mode == LPD).collect();
        let ce = unsafe { c::cref_channel_element_read(p.c.0, aot, flags, el_flags, spf, nch as i32, ep, 0) };
        let re = {
            let sri = p.r.sri;
            let cpe_present = p.r.sci[0].p_cpe_static_data.is_some();
            let mut com_static = p.r.take_com_static();
            let RCtx { ci, sci, com, cpe, spec, bs, .. } = &mut p.r;
            let [c0, c1] = ci;
            let [s0, s1] = sci;
            let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
            let mut statics: [&mut CAacDecoderStaticChannelInfo; 2] = [&mut **s0, &mut **s1];
            let r = c_channel_element_read(
                bs,
                &mut chans[..nch as usize],
                Some(&mut statics[..nch as usize]),
                aot,
                &sri,
                flags,
                el_flags,
                spf,
                nch,
                ep as i8,
                None,
                spec,
                com,
                None,
                &mut com_static,
                if cpe_present { Some(&mut **cpe) } else { None },
            );
            p.r.put_com_static(com_static);
            r
        };
        let cvalid = p.c.bs_valid();
        let consumed = (au.len() * 8) as i64 - cvalid as i64;
        assert_eq!(re, ce, "{what}: CChannelElement_Read");
        p.check(&format!("{what}: CChannelElement_Read"));
        p.check_bs(&format!("{what}: CChannelElement_Read"));
        let lpd: Vec<bool> = (0..nch as usize).map(|ch| p.r.ci[ch].render_mode == AACDEC_RENDER_LPD).collect();
        let n = &mut self.n;
        n.reads += 1;
        n.reads_rvlc += (flags & AC_ER_RVLC != 0) as u32;
        n.reads_hcr += (flags & AC_ER_HCR != 0) as u32;
        if usac {
            for (&lpd, &after) in lpd.iter().zip(after_lpd.iter()) {
                if lpd {
                    n.reads_lpd += 1;
                } else if after {
                    n.reads_fd_after_lpd += 1;
                }
            }
        }
        (ce, consumed.max(0) as usize)
    }

    fn decode(&mut self, idx: usize, what: &str) {
        let el_flags = self.el_flags(idx);
        let (flags, usac) = (self.flags, self.usac);
        let id = self.els[idx].id;
        let nch = if id == ID_CPE || id == ID_USAC_CPE { 2 } else { 1 };
        let p = &mut self.els[idx].pair;
        unsafe { c::cref_channel_element_decode(p.c.0, flags, el_flags, nch, usac as i32) };
        {
            let sri = p.r.sri;
            let cpe_present = p.r.sci[0].p_cpe_static_data.is_some();
            let mut com_static = p.r.take_com_static();
            let RCtx { ci, sci, com, cpe, spec, scratch, .. } = &mut p.r;
            let [c0, c1] = ci;
            let [s0, s1] = sci;
            let mut chans: [&mut CAacDecoderChannelInfo; 2] = [&mut **c0, &mut **c1];
            let mut statics: [&mut CAacDecoderStaticChannelInfo; 2] = [&mut **s0, &mut **s1];
            let mut seed = statics[0].pns_current_seed;
            c_channel_element_decode(
                &mut chans[..nch as usize],
                &mut statics[..nch as usize],
                &sri,
                flags,
                el_flags,
                nch,
                spec,
                com,
                None,
                &mut com_static,
                if cpe_present { Some(&mut **cpe) } else { None },
                scratch,
                &mut seed,
            );
            statics[0].pns_current_seed = seed;
            p.r.put_com_static(com_static);
        }
        p.check(&format!("{what}: CChannelElement_Decode"));
        self.n.decodes += 1;
        self.n.decodes_lpd +=
            p.r.ci[..nch as usize].iter().filter(|c| c.render_mode == AACDEC_RENDER_LPD).count() as u32;
    }

    /// Concealment, frequency to time transform and time domain fading of the channels of
    /// element `idx`.
    fn render(&mut self, idx: usize, frame_ok: bool, what: &str) {
        let (spf, flags) = (self.spf, self.flags);
        let id = self.els[idx].id;
        let nch = if id == ID_CPE || id == ID_USAC_CPE { 2 } else { 1 };
        let headroom = 1;
        for ch in 0..nch {
            let p = &mut self.els[idx].pair;
            let mut fok = frame_ok as i32;
            {
                // aacdecoder.cpp: an invalid ics_info that the fade out would use marks the frame
                // as erroneous
                let ics = &p.r.ci[ch].ics_info;
                let ci = &p.r.sci[ch].concealment_info;
                let mute_release_active = fok != 0
                    && ci.conceal_state >= ConcealState_Mute
                    && ci.cnt_valid_frames + 1 <= p.r.params.num_mute_release_frames;
                let invalid = ics.max_sf_bands > ics.total_sf_bands;
                let used = !(p.r.ci[ch].render_mode == AACDEC_RENDER_LPD && p.r.sci[ch].last_lpd_mode == 0);
                if used && invalid && !mute_release_active {
                    fok = 0;
                }
            }
            let last_lpd = p.r.sci[ch].last_lpd_mode;
            let ce = unsafe { ccc::cref_conceal_apply(p.c.0, ch as i32, spf as i32, last_lpd as i32, fok, flags) };
            let re = {
                let sri = p.r.sri;
                let RCtx { ci, sci, params, spec, .. } = &mut p.r;
                cconcealment_apply(&mut ci[ch], &mut sci[ch], &sri, spf as i32, last_lpd, fok, flags, params, &mut spec[ch * 1024..])
            };
            assert_eq!(re, ce, "{what} ch {ch}: CConcealment_Apply");
            p.check(&format!("{what} ch {ch}: CConcealment_Apply"));
            let rm = p.r.ci[ch].render_mode;
            self.n.conceals += 1;
            self.n.conceals_lpc += (fok == 0 && rm == AACDEC_RENDER_IMDCT) as u32;

            let fo = (fok != 0 && ce != -1) as i32;
            if rm == AACDEC_RENDER_IMDCT {
                self.n.f2t += 1;
                self.n.f2t_lpd += (p.r.sci[ch].last_core_mode == LPD) as u32;
                unsafe { cblk::cref_block_frequency_to_time(p.c.0, ch as i32, spf as i32, fo, headroom, 0, ch as i32) };
                let RCtx { ci, sci, spec, wb1, out, .. } = &mut p.r;
                cblock_frequency_to_time(&mut sci[ch], &mut ci[ch], &mut out[ch], spf as i16, fo, &mut wb1.buf, headroom, 0, ch as i32, &mut spec[ch * 1024..]);
            } else if rm == AACDEC_RENDER_ELDFB {
                self.n.f2t_ld += 1;
                unsafe { cblk::cref_block_frequency_to_time_ld(p.c.0, ch as i32, spf as i32) };
                let RCtx { ci, sci, spec, out, .. } = &mut p.r;
                cblock_frequency_to_time_low_delay(&mut sci[ch], &mut ci[ch], &mut out[ch], spf as i16, &mut spec[ch * 1024..]);
            } else {
                // AACDEC_RENDER_LPD (the API flags of the frame are 0)
                self.n.lpd_render += 1;
                let ce = unsafe {
                    cblk::cref_block_lpd_render_time_signal(p.c.0, ch as i32, spf as i32, fo as u32, headroom, 0, flags)
                };
                let re = {
                    let sri = p.r.sri;
                    let RCtx { ci, sci, spec, wb1, out, .. } = &mut p.r;
                    clpd_render_time_signal(
                        &mut sci[ch],
                        &mut ci[ch],
                        &mut out[ch],
                        spf as i32,
                        &sri,
                        fo as u32,
                        headroom,
                        0,
                        flags,
                        &mut spec[ch * 1024..],
                        wb1,
                    )
                };
                assert_eq!(re, ce, "{what} ch {ch}: CLpd_RenderTimeSignal");
            }
            p.check(&format!("{what} ch {ch}: frequency to time"));

            let ce = unsafe { ccc::cref_conceal_td_fading(p.c.0, ch as i32, spf as i32, headroom) };
            let re = {
                let RCtx { sci, params, out, .. } = &mut p.r;
                cconcealment_td_fading(spf as i32, &mut sci[ch], params, headroom, &mut out[ch])
            };
            assert_eq!(re, ce, "{what} ch {ch}: CConcealment_TDFading");
            p.check(&format!("{what} ch {ch}: CConcealment_TDFading"));
            self.n.td_fading += 1;
        }
    }
}

/// Decodes the access units of a stream on both sides; returns the compared steps.
fn run(name: &str, asc: CSAudioSpecificConfig, aus: &[Vec<u8>], seed: u64) -> Counts {
    let mut d = Dec::new(name, asc);
    let mut rng = Rng::new(seed);
    for (f, au) in aus.iter().enumerate() {
        let lost = rng.next_u32() % 16 == 0;
        let mut frame_ok = !lost;
        let mut decoded: Vec<usize> = Vec::new();
        if lost {
            d.lost += 1;
        } else {
            // the elements of the access unit
            let mut pos = 0usize;
            let mut idx = 0usize;
            let total = au.len() * 8;
            if d.usac {
                let mut bs = FdkBitStream::default();
                let mut b = au.clone();
                b.resize(b.len().next_power_of_two().max(64), 0);
                // SAFETY: b outlives bs.
                fdk_aac_rs_fdk::bitstream::fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&b), b.len() as u32, total as u32, 0);
                let indep = fdk_read_bits(&mut bs, 1);
                pos = 1;
                d.flags = (d.flags & !AC_INDEP) | if indep != 0 { AC_INDEP } else { 0 };
                let n = d.asc.m_sc.m_usac_config.m_usac_num_elements as usize;
                for e in 0..n {
                    let el = d.asc.m_sc.m_usac_config.element[e];
                    let id = el.usac_element_type;
                    if id == ID_USAC_EXT {
                        let present = fdk_read_bits(&mut bs, 1);
                        pos += 1;
                        if present != 0 {
                            let dflt = fdk_read_bits(&mut bs, 1);
                            pos += 1;
                            let mut len = el.ext_element.usac_ext_element_default_length as usize;
                            if dflt == 0 {
                                len = fdk_read_bits(&mut bs, 8) as usize;
                                pos += 8;
                                if len == 255 {
                                    len += fdk_read_bits(&mut bs, 16) as usize - 2;
                                    pos += 16;
                                }
                            }
                            fdk_push_for(&mut bs, (len * 8) as u32);
                            pos += len * 8;
                        }
                        continue;
                    }
                    let what = format!("{name} frame {f} element {e}");
                    let (err, consumed) = d.read(e, id, au, pos, &what);
                    decoded.push(e);
                    if err != AAC_DEC_OK {
                        frame_ok = false;
                        break;
                    }
                    let el_flags = d.el_flags(e);
                    if el.m_stereo_config_index > 0 || el.m_harmonic_sbr != 0 || d.asc.m_sbr_present_flag != 0 || el_flags & AC_EL_USAC_MPS212 != 0 {
                        // SBR / MPS payloads follow inside the element (not parsed here)
                        break;
                    }
                    let _ = consumed;
                    break; // the following elements need the SBR/extension parsers; one is enough
                }
            } else {
                let er = d.ep >= 0;
                let cc = d.asc.m_channel_configuration as usize;
                loop {
                    let id: Mp4ElementId = if er {
                        if cc == 0 || cc > 14 {
                            break;
                        }
                        let t = ELEMENTS_TAB[cc - 1][idx];
                        if t != ID_SCE && t != ID_CPE && t != ID_LFE {
                            break;
                        }
                        t
                    } else {
                        if pos + 3 > total {
                            break;
                        }
                        let mut bs = FdkBitStream::default();
                        let mut b = au.clone();
                        b.resize(b.len().next_power_of_two().max(64), 0);
                        // SAFETY: b outlives bs.
                        fdk_aac_rs_fdk::bitstream::fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&b), b.len() as u32, total as u32, 0);
                        fdk_push_for(&mut bs, pos as u32);
                        let id = fdk_read_bits(&mut bs, 3) as i32;
                        pos += 3;
                        match id {
                            ID_SCE | ID_CPE | ID_LFE => id,
                            ID_FIL => {
                                let mut cnt = fdk_read_bits(&mut bs, 4) as usize;
                                pos += 4;
                                if cnt == 15 {
                                    cnt += fdk_read_bits(&mut bs, 8) as usize - 1;
                                    pos += 8;
                                }
                                pos += cnt * 8;
                                continue;
                            }
                            ID_DSE => {
                                fdk_read_bits(&mut bs, 4);
                                let align = fdk_read_bits(&mut bs, 1);
                                let mut cnt = fdk_read_bits(&mut bs, 8) as usize;
                                pos += 13;
                                if cnt == 255 {
                                    cnt += fdk_read_bits(&mut bs, 8) as usize;
                                    pos += 8;
                                }
                                if align != 0 {
                                    pos = pos.div_ceil(8) * 8;
                                }
                                pos += cnt * 8;
                                continue;
                            }
                            _ => break, // END, PCE, CCE
                        }
                    };
                    let what = format!("{name} frame {f} element {idx}");
                    let (err, consumed) = d.read(idx, id, au, pos, &what);
                    decoded.push(idx);
                    if err != AAC_DEC_OK {
                        frame_ok = false;
                        break;
                    }
                    pos = consumed;
                    idx += 1;
                    if pos >= total {
                        break;
                    }
                }
            }
        }
        if frame_ok {
            for &e in &decoded {
                d.decode(e, &format!("{name} frame {f} element {e}"));
            }
        }
        for e in 0..d.els.len() {
            d.render(e, frame_ok, &format!("{name} frame {f} element {e} ok {frame_ok}"));
        }
    }
    d.n
}

fn run_loas(name: &str, frames: usize, seed: u64) -> Counts {
    let Some(data) = conformance(name) else {
        eprintln!("{name}: not available, skipped");
        return Counts::default();
    };
    let (asc, aus) = loas_aus(&data, frames).unwrap_or_else(|| panic!("{name}: no access units"));
    let r = run(name, asc, &aus, seed);
    eprintln!("{name}: {} access units, {r:?}", aus.len());
    r
}

fn run_mp4(name: &str, frames: usize, seed: u64) -> Counts {
    let Some(data) = conformance(name) else {
        eprintln!("{name}: not available, skipped");
        return Counts::default();
    };
    let (asc_bytes, aus) = mp4_aus(&data, frames).unwrap_or_else(|| panic!("{name}: no audio track"));
    let asc = parse_asc(&asc_bytes).unwrap_or_else(|| panic!("{name}: AudioSpecificConfig"));
    let r = run(name, asc, &aus, seed);
    eprintln!("{name}: {} access units, {r:?}", aus.len());
    r
}


#[test]
fn loas_aac_lc() {
    for (i, n) in ["al00_08_amv0.ass", "al05_08_amv0.ass", "al06_08_amv0.ass", "al07_08_amv0.ass"].iter().enumerate() {
        run_loas(n, 200, 0x3ea1_0000 + i as u64);
    }
}

#[test]
fn loas_er_aac() {
    for (i, n) in [
        "er_al10_08_ep0_amv0.ass",
        "er_al21_08_ep0_amv0.ass",
        "er_al12_08_ep0_amv0.ass",
        "er_ad1102np_22_ep0_amv0.ass",
        "er_ad1000np_22_ep0_amv0.ass",
        "er_ad2040np_22_ep0_amv0.ass",
    ]
    .iter()
    .enumerate()
    {
        run_loas(n, 200, 0x3ea2_0000 + i as u64);
    }
}

/// The ER tools RVLC and HCR (with and without VCB11): every combination of ER AAC-LD (LOAS; the
/// fourth digit of the name is VCB11/RVLC/HCR as a 3-bit number, the second 512/480 samples) and
/// ER AAC-ELD (MP4), and the ER AAC-LC setups with RVLC or HCR (LOAS).
#[test]
fn er_tools_rvlc_hcr() {
    let mut total = Counts::default();
    for (i, n) in [
        "er_ad1111np_22_ep0_amv0.ass", // HCR, 480
        "er_ad1020np_22_ep0_amv0.ass", // RVLC, 512
        "er_ad1030np_22_ep0_amv0.ass", // RVLC + HCR, 512
        "er_ad2050np_22_ep0_amv0.ass", // VCB11 + HCR, stereo, 512
        "er_ad2160np_22_ep0_amv0.ass", // VCB11 + RVLC, stereo, 480
        "er_ad2170np_22_ep0_amv0.ass", // VCB11 + RVLC + HCR, stereo, 480
        "er_al15_08_ep0_amv0.ass",     // ER AAC-LC HCR
        "er_al18_08_ep0_amv0.ass",     // ER AAC-LC VCB11 + RVLC + HCR
        "er_al23_08_ep0_amv0.ass",     // ER AAC-LC RVLC, stereo
        "er_al26_08_ep0_amv0.ass",     // ER AAC-LC VCB11 + HCR, stereo
    ]
    .iter()
    .enumerate()
    {
        total.add(&run_loas(n, 200, 0x3ea4_0000 + i as u64));
    }
    for (i, n) in [
        "er_eld1010np_22_ep0.mp4", // HCR
        "er_eld1020np_22_ep0.mp4", // RVLC
        "er_eld1030np_22_ep0.mp4", // RVLC + HCR
        "er_eld1050np_22_ep0.mp4", // VCB11 + HCR
        "er_eld1060np_22_ep0.mp4", // VCB11 + RVLC
        "er_eld1170np_22_ep0.mp4", // VCB11 + RVLC + HCR, 480
    ]
    .iter()
    .enumerate()
    {
        total.add(&run_mp4(n, 200, 0x3ea5_0000 + i as u64));
    }
    eprintln!("ER tools total: {total:?}");
}

#[test]
fn mp4_960_eld_usac() {
    let w = work_dir();
    let mut names: Vec<String> = vec![
        "al00sf_08.mp4".into(),
        "al05sf_08.mp4".into(),
        "er_eld1100np_22_ep0.mp4".into(),
        "er_eld2000np_22_ep0.mp4".into(),
    ];
    // a few USAC streams of the index (mono, stereo, 768 and 1024 core frame lengths)
    if let Some(w) = &w {
        if let Ok(idx) = std::fs::read_to_string(w.join("conformance/index.json")) {
            let mut taken = std::collections::HashSet::new();
            for part in idx.split("\"file\": \"").skip(1) {
                let file = part.split('"').next().unwrap_or("");
                if !part.contains("\"aot\": 42") || !part.contains("\"role\": \"positive\"") || !file.ends_with(".mp4") {
                    continue;
                }
                let fl = if part.contains("\"frame_length\": 768") { 768 } else { 1024 };
                let chn = if part.contains("\"channels\": 1,") { 1 } else { 2 };
                if taken.insert((fl, chn)) {
                    names.push(file.to_string());
                }
            }
        }
    }
    for (i, n) in names.iter().enumerate() {
        run_mp4(n, 120, 0x3ea3_0000 + i as u64);
    }
}

/// The ISO/IEC 23003-7 streams of a prefix in compressedMp4 (sorted by name).
fn usac_streams(prefix: &str) -> Vec<String> {
    let Some(w) = work_dir() else { return Vec::new() };
    let dir = w.join("conformance/23003-7/compressedMp4");
    let mut v: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with(prefix) && n.ends_with(".mp4"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The USAC LPD streams (ACELP, TCX, LPC, AVQ, bass post-filter; mono and stereo, both core
/// frame lengths): LPD channels through CLpdChannelStream_Read/Decode and CLpd_RenderTimeSignal.
#[test]
fn mp4_usac_lpd() {
    let mut total = Counts::default();
    for (i, n) in usac_streams("Lpd_").iter().enumerate() {
        total.add(&run_mp4(&format!("23003-7/compressedMp4/{n}"), 120, 0x3ea6_0000 + i as u64));
    }
    eprintln!("Lpd_* total: {total:?}");
}

/// The USAC core coder transition streams (one of each setup): FD/LPD switches with FAC
/// (CLpd_FAC_Read, CLpd_FAC_Acelp2Mdct in CBlock_FrequencyToTime, CLpd_FAC_Mdct2Acelp).
#[test]
fn mp4_usac_cct() {
    let mut total = Counts::default();
    let mut taken = std::collections::HashSet::new();
    for (i, n) in usac_streams("Cct_").iter().enumerate() {
        // Cct_<ch>_<c0|c1>_<setup>_<config>.mp4: one stream per setup
        let setup: String = n.rsplit_once('_').map(|x| x.0).unwrap_or(n).to_string();
        if !taken.insert(setup) {
            continue;
        }
        total.add(&run_mp4(&format!("23003-7/compressedMp4/{n}"), 120, 0x3ea7_0000 + i as u64));
    }
    eprintln!("Cct_* total: {total:?}");
}
