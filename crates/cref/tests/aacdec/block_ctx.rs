// The Rust side of the block test context (crates/cref/shim/aacdec_block_ctx.cpp): the structures
// of two channels of an element with the memory that the channel functions reach (spectrum, common
// data, workbuffer, joint stereo scratch, concealment parameters, output buffers), and a flat
// image of them in the same field order as the C++ side. Tests load the same image into both
// sides, run a function on both, and compare the images afterwards.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_aacdec::conceal_types::*;
use fdk_aac_rs_aacdec::usacdec_acelp::{CAcelpChannelData, CAcelpStaticMem};
use fdk_aac_rs_arith::ac_arith_coder::CArcoData;
use fdk_aac_rs_cref::aacdec_block_ctx as cx;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::archdef::FixpWtp;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream, BS_READER};
use fdk_aac_rs_fdk::common_fix::FixpDbl;
use fdk_aac_rs_fdk::mdct::MdctT;
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;

pub const CTX_SPEC_LEN: usize = 8 * 1024;
pub const CTX_SCRATCH_LEN: usize = 4096;
pub const CTX_OUT_LEN: usize = 2048;
pub const OVERLAP_LEN: usize = 768;
const ARCO_BYTES: usize = 518;
const BS_MAX: usize = 65536;

/// A flat image: one `i32` per scalar field. `load` reads the fields from `v`, otherwise they
/// are appended to `v`. `sections` names the start of each structure for error messages.
pub struct Img {
    pub v: Vec<i32>,
    pub n: usize,
    pub load: bool,
    pub sections: Vec<(usize, String)>,
}

impl Img {
    pub fn dump() -> Img {
        Img { v: Vec::new(), n: 0, load: false, sections: Vec::new() }
    }
    pub fn loader(v: Vec<i32>) -> Img {
        Img { v, n: 0, load: true, sections: Vec::new() }
    }
    fn sec(&mut self, name: &str) {
        self.sections.push((self.n, name.to_string()));
    }
    fn raw(&mut self, x: i32) -> i32 {
        let r = if self.load { self.v[self.n] } else {
            self.v.push(x);
            x
        };
        self.n += 1;
        r
    }
    pub fn i32(&mut self, x: &mut i32) {
        *x = self.raw(*x);
    }
    pub fn u32(&mut self, x: &mut u32) {
        *x = self.raw(*x as i32) as u32;
    }
    pub fn i16(&mut self, x: &mut i16) {
        *x = self.raw(*x as i32) as i16;
    }
    pub fn u16(&mut self, x: &mut u16) {
        *x = self.raw(*x as i32) as u16;
    }
    pub fn u8(&mut self, x: &mut u8) {
        *x = self.raw(*x as i32) as u8;
    }
    pub fn i8(&mut self, x: &mut i8) {
        *x = self.raw(*x as i32) as i8;
    }
    pub fn a32(&mut self, a: &mut [i32]) {
        for x in a {
            self.i32(x);
        }
    }
    pub fn a16(&mut self, a: &mut [i16]) {
        for x in a {
            self.i16(x);
        }
    }
    pub fn au16(&mut self, a: &mut [u16]) {
        for x in a {
            self.u16(x);
        }
    }
    pub fn au8(&mut self, a: &mut [u8]) {
        for x in a {
            self.u8(x);
        }
    }
    pub fn ai8(&mut self, a: &mut [i8]) {
        for x in a {
            self.i8(x);
        }
    }
    /// The name of the structure that contains index `i`.
    pub fn where_is(&self, i: usize) -> String {
        let mut name = String::from("?");
        let mut start = 0;
        for (s, n) in &self.sections {
            if *s <= i {
                name = n.clone();
                start = *s;
            }
        }
        format!("{name} + {}", i - start)
    }
}

pub fn img_ics(im: &mut Img, p: &mut CIcsInfo) {
    im.au8(&mut p.window_group_length);
    im.u8(&mut p.window_groups);
    im.u8(&mut p.valid);
    im.u8(&mut p.window_shape);
    im.i32(&mut p.window_sequence);
    im.u8(&mut p.max_sf_bands);
    im.u8(&mut p.max_sfb_ste);
    im.u8(&mut p.scale_factor_grouping);
    im.u8(&mut p.total_sf_bands);
}

fn img_filter(im: &mut Img, f: &mut CFilter) {
    im.ai8(&mut f.coeff);
    im.u8(&mut f.start_band);
    im.u8(&mut f.stop_band);
    im.i8(&mut f.direction);
    im.i8(&mut f.resolution);
    im.u8(&mut f.order);
}

pub fn img_tns(im: &mut Img, t: &mut CTnsData) {
    for w in t.filter.iter_mut() {
        for f in w.iter_mut() {
            img_filter(im, f);
        }
    }
    im.au8(&mut t.number_of_filters);
    im.u8(&mut t.data_present);
    im.u8(&mut t.active);
    im.u8(&mut t.gain_ld);
}

pub fn img_pulse(im: &mut Img, p: &mut CPulseData) {
    im.u8(&mut p.pulse_data_present);
    im.u8(&mut p.number_pulse);
    im.u8(&mut p.pulse_start_band);
    im.au8(&mut p.pulse_offset);
    im.au8(&mut p.pulse_amp);
}

fn img_dyn(im: &mut Img, d: &mut CAacDecoderDynamicData, usac: bool) {
    im.a16(&mut d.a_scale_factor);
    im.a16(&mut d.a_sfb_scale);
    im.au8(&mut d.a_code_book);
    im.au8(&mut d.band_is_noise);
    img_tns(im, &mut d.tns_data);
    im.u8(&mut d.raw_data_info.common_window);
    im.u8(&mut d.raw_data_info.global_gain);
    if !usac {
        let a = &mut d.specific_to.aac;
        img_pulse(im, &mut a.pulse_data);
        im.a16(&mut a.a_num_line_in_sec4_hcr);
        im.au8(&mut a.a_code_books4_hcr);
        im.i16(&mut a.len_of_reordered_spectral_data);
        im.i8(&mut a.len_of_longest_codeword);
        im.i8(&mut a.number_section);
        im.i8(&mut a.rvlc_current_scale_factor_ok);
        im.i8(&mut a.rvlc_intensity_used);
    } else {
        let u = &mut d.specific_to.usac;
        im.u8(&mut u.fd_noise_level_and_offset);
        im.u8(&mut u.tns_active);
        im.u8(&mut u.tns_on_lr);
        im.au8(&mut u.tcx_noise_factor);
        im.au8(&mut u.tcx_global_gain);
    }
}

pub fn img_pns(im: &mut Img, p: &mut CPnsData) {
    im.au8(&mut p.pns_used);
    im.i32(&mut p.current_energy);
    im.u8(&mut p.pns_active);
}

fn img_acelp_ch(im: &mut Img, a: &mut CAcelpChannelData) {
    im.u8(&mut a.acelp_core_mode);
    im.u8(&mut a.mean_energy);
    im.au16(&mut a.t0);
    im.au8(&mut a.t0_frac);
    im.au8(&mut a.ltp_filtering_flag);
    for r in a.icb_index.iter_mut() {
        im.a16(r);
    }
    im.au8(&mut a.gains);
}

fn img_fac_ptr(im: &mut Img, p: &mut FacDataPtr) {
    let code = match *p {
        FacDataPtr::Null => 0,
        FacDataPtr::FacData0 => 1,
        FacDataPtr::Scratch => 2,
        FacDataPtr::Spectral(o) => 1000 + o as i32,
    };
    let code = im.raw(code);
    if im.load {
        *p = match code {
            0 => FacDataPtr::Null,
            1 => FacDataPtr::FacData0,
            c if (1000..1000 + 1024).contains(&c) => FacDataPtr::Spectral((c - 1000) as usize),
            _ => FacDataPtr::Scratch,
        };
    }
}

fn img_ci(im: &mut Img, c: &mut CAacDecoderChannelInfo, usac: bool) {
    if usac {
        let u = &mut c.data.usac;
        im.a32(&mut u.fac_data0);
        im.ai8(&mut u.fac_data_e);
        for k in 0..4 {
            img_fac_ptr(im, &mut u.fac_data[k]);
        }
        im.u8(&mut u.core_mode);
        im.i32(&mut u.core_mode_last);
        im.u8(&mut u.lpd_mode_last);
        im.au8(&mut u.r#mod);
        im.u8(&mut u.bpf_control_info);
        im.sec("ci.data.usac.lsp_coeff");
        for r in u.lsp_coeff.iter_mut() {
            im.a16(r);
        }
        im.sec("ci.data.usac.lp_coeff..");
        for r in u.lp_coeff.iter_mut() {
            im.a16(r);
        }
        im.a32(&mut u.lp_coeff_exp);
        im.a16(&mut u.lsf_adaptive_mean_cand);
        im.a16(&mut u.a_stability);
        for a in u.acelp.iter_mut() {
            img_acelp_ch(im, a);
        }
        im.a32(&mut u.tcx_gain);
        im.ai8(&mut u.tcx_gain_e);
    } else {
        img_pns(im, &mut c.data.aac.pns_data);
    }
    im.a16(&mut c.spec_scale);
    img_ics(im, &mut c.ics_info);
    im.i32(&mut c.granule_length);
    im.u8(&mut c.element_instance_tag);
    im.i32(&mut c.render_mode);
    im.i32(&mut c.curr_aliasing_symmetry);
    img_dyn(im, &mut c.p_dyn_data, usac);
}

/// The distinct window slope tables in the order of the C++ side (`FDKgetWindowSlope(len,
/// shape)` for the lengths below, `shape` 0..=2).
pub fn slopes() -> Vec<&'static [FixpWtp]> {
    const LENS: [i32; 23] = [
        4, 8, 12, 15, 16, 24, 30, 32, 48, 60, 64, 96, 120, 128, 192, 240, 256, 384, 480, 512, 768,
        960, 1024,
    ];
    let mut v: Vec<&'static [FixpWtp]> = Vec::new();
    for len in LENS {
        for shape in 0..3 {
            if let Some(w) = fdk_get_window_slope(len, shape) {
                if !v.iter().any(|x| x.as_ptr() == w.as_ptr()) {
                    v.push(w);
                }
            }
        }
    }
    v
}

fn img_mdct(im: &mut Img, m: &mut MdctT, slopes: &[&'static [FixpWtp]]) {
    m.overlap.resize(OVERLAP_LEN, 0);
    im.a32(&mut m.overlap[..OVERLAP_LEN]);
    let code = match m.prev_wrs {
        None => 0,
        Some(w) => slopes.iter().position(|x| x.as_ptr() == w.as_ptr()).map_or(-1, |i| i as i32 + 1),
    };
    let code = im.raw(code);
    if im.load {
        m.prev_wrs = if code > 0 && code as usize <= slopes.len() {
            Some(slopes[code as usize - 1])
        } else {
            None
        };
    }
    im.i32(&mut m.prev_tl);
    im.i32(&mut m.prev_nr);
    im.i32(&mut m.prev_fr);
    im.i32(&mut m.ov_offset);
    im.i32(&mut m.ov_size);
    im.i32(&mut m.prev_alias_symmetry);
    im.i32(&mut m.prev_prev_alias_symmetry);
    let fz = im.raw(m.p_fac_zir.is_some() as i32);
    let ao = im.raw(m.p_asym_ovlp.is_some() as i32);
    if im.load {
        m.p_fac_zir = if fz != 0 { Some(vec![0; 256]) } else { None };
        let _ = ao;
        m.p_asym_ovlp = None;
    }
}

fn img_acelp_static(im: &mut Img, a: &mut CAcelpStaticMem) {
    im.a32(&mut a.old_exc_mem);
    im.a32(&mut a.old_syn_mem);
    im.a16(&mut a.a);
    im.i32(&mut a.a_exp);
    im.i32(&mut a.gc_threshold);
    im.i32(&mut a.de_emph_mem);
    im.i16(&mut a.past_gpit);
    im.i32(&mut a.past_gcode);
    im.u16(&mut a.old_t0);
    im.u8(&mut a.old_t0_frac);
    im.i32(&mut a.deemph_mem_wsyn);
    im.i32(&mut a.wsyn_rms);
    im.i16(&mut a.seed_ace);
}

pub fn img_conceal(im: &mut Img, c: &mut CConcealmentInfo) {
    im.a32(&mut c.spectral_coefficient);
    im.a16(&mut c.spec_scale);
    im.i32(&mut c.i_random_phase);
    im.a32(&mut c.prev_frame_ok);
    im.i32(&mut c.cnt_valid_frames);
    im.i32(&mut c.cnt_fade_frames);
    im.a32(&mut c.win_grp_offset);
    im.a32(&mut c.att_grp_offset);
    im.i8(&mut c.last_render_mode);
    im.u8(&mut c.window_shape);
    im.i32(&mut c.window_sequence);
    im.u8(&mut c.last_win_grp_len);
    im.i32(&mut c.conceal_state);
    im.i32(&mut c.conceal_state_old);
    im.i32(&mut c.fade_old);
    im.i32(&mut c.last_fading_type);
    im.sec("conceal.rvlc");
    im.a16(&mut c.a_rvlc_previous_scale_factor);
    im.au8(&mut c.a_rvlc_previous_codebook);
    im.i8(&mut c.rvlc_previous_scale_factor_ok);
    im.i8(&mut c.rvlc_previous_block_type);
    im.sec("conceal.lsf4");
    im.a16(&mut c.lsf4);
    im.sec("conceal.last_tcx_gain..");
    im.i32(&mut c.last_tcx_gain);
    im.i32(&mut c.last_tcx_gain_e);
    im.u32(&mut c.td_noise_seed);
    im.a32(&mut c.td_noise_states);
    im.a16(&mut c.td_noise_coef);
    im.i16(&mut c.td_noise_att);
}

fn arco_bytes(a: &CArcoData) -> [u8; ARCO_BYTES] {
    let mut b = [0u8; ARCO_BYTES];
    b[..2].copy_from_slice(&a.m_number_lines_prev.to_le_bytes());
    b[2..].copy_from_slice(&a.c_prev);
    b
}

fn img_sci(im: &mut Img, s: &mut CAacDecoderStaticChannelInfo, arco: &mut CArcoData, slopes: &[&'static [FixpWtp]]) {
    img_mdct(im, &mut s.i_mdct, slopes);
    // hArCo: the arithmetic coder state of the context (held separately while it is NULL)
    let present = im.raw(s.h_ar_co.is_some() as i32);
    if let Some(a) = s.h_ar_co.take() {
        *arco = *a;
    }
    let mut b = arco_bytes(arco);
    im.au8(&mut b);
    arco.m_number_lines_prev = i16::from_le_bytes([b[0], b[1]]);
    arco.c_prev.copy_from_slice(&b[2..]);
    if im.load {
        s.h_ar_co = if present != 0 { Some(Box::new(arco.clone())) } else { None };
    } else if present != 0 {
        s.h_ar_co = Some(Box::new(arco.clone()));
    }
    im.i32(&mut s.pns_current_seed);
    im.a32(&mut s.old_synth);
    im.a32(&mut s.old_t_pf);
    im.a32(&mut s.old_gain_pf);
    im.a32(&mut s.mem_bpf);
    im.u8(&mut s.old_bpf_control_info);
    im.i32(&mut s.last_core_mode);
    im.u8(&mut s.last_lpd_mode);
    im.u8(&mut s.last_last_lpd_mode);
    im.u8(&mut s.last_lpc_lost);
    im.sec("sci.lpc4_lsf+lsf_adaptive_mean");
    im.a16(&mut s.lpc4_lsf);
    im.a16(&mut s.lsf_adaptive_mean);
    im.sec("sci.lp_coeff_old..");
    for r in s.lp_coeff_old.iter_mut() {
        im.a16(r);
    }
    im.a32(&mut s.lp_coeff_old_exp);
    im.i16(&mut s.old_stability);
    im.u32(&mut s.num_lost_lpd_frames);
    im.i32(&mut s.last_tcx_gain);
    im.i32(&mut s.last_tcx_gain_e);
    im.a32(&mut s.last_alfd_gains);
    im.i16(&mut s.last_tcx_pitch);
    im.u8(&mut s.last_tcx_noise_factor);
    img_acelp_static(im, &mut s.acelp);
    im.u32(&mut s.nf_random_seed);
    im.sec("sci.concealmentInfo");
    img_conceal(im, &mut s.concealment_info);
    let cpe = im.raw(s.p_cpe_static_data.is_some() as i32);
    if im.load {
        s.p_cpe_static_data = if cpe != 0 { Some(0) } else { None };
    }
}

pub fn img_jsd(im: &mut Img, j: &mut CJointStereoData) {
    im.u8(&mut j.ms_mask_present);
    im.au8(&mut j.ms_used);
    im.u8(&mut j.igf_ms_mask_present);
    im.u8(&mut j.cplx_pred_flag);
    im.u8(&mut j.igf_cplx_pred_flag);
    im.a32(&mut j.store_dmx_re_prev);
    im.i16(&mut j.store_dmx_re_prev_e);
}

pub fn img_cplx(im: &mut Img, c: &mut CCplxPredictionData) {
    im.u8(&mut c.pred_dir);
    im.u8(&mut c.igf_pred_dir);
    im.u8(&mut c.complex_coef);
    im.u8(&mut c.use_prev_frame);
    for r in c.alpha_q_re.iter_mut() {
        im.a16(r);
    }
    for r in c.alpha_q_im.iter_mut() {
        im.a16(r);
    }
}

pub fn img_jsp(im: &mut Img, j: &mut CJointStereoPersistentData) {
    im.u8(&mut j.clear_spectral_coeffs);
    for r in j.alpha_q_re_prev.iter_mut() {
        im.a16(r);
    }
    for r in j.alpha_q_im_prev.iter_mut() {
        im.a16(r);
    }
    im.u8(&mut j.win_seq_prev);
    im.u8(&mut j.win_shape_prev);
    im.u8(&mut j.win_groups_prev);
}

pub fn img_params(im: &mut Img, p: &mut CConcealParams) {
    im.a16(&mut p.fade_out_factor);
    im.a16(&mut p.fade_in_factor);
    im.i32(&mut p.method);
    im.i32(&mut p.num_fade_out_frames);
    im.i32(&mut p.num_fade_in_frames);
    im.i32(&mut p.num_mute_release_frames);
    im.i32(&mut p.comfort_noise_level);
}

/// The Rust context (see the module documentation).
pub struct RCtx {
    pub ci: [Box<CAacDecoderChannelInfo>; 2],
    pub sci: [Box<CAacDecoderStaticChannelInfo>; 2],
    pub com: Box<CAacDecoderCommonData>,
    /// `pComStaticData->cplxPredictionData` (`cplx` while `cplx_present`)
    pub cplx: Box<CCplxPredictionData>,
    pub cplx_present: bool,
    pub wb1: Box<CWorkBufferCore1>,
    pub cpe: Box<CpePersistentData>,
    pub params: Box<CConcealParams>,
    pub sri: SamplingRateInfo,
    pub spec: Vec<FixpDbl>,
    pub scratch: Vec<FixpDbl>,
    pub out: [Vec<FixpDbl>; 2],
    pub arco: [Box<CArcoData>; 2],
    pub usac: bool,
    pub bsbuf: Vec<u8>,
    pub bs: FdkBitStream,
    pub slopes: Vec<&'static [FixpWtp]>,
}

impl RCtx {
    pub fn new() -> RCtx {
        let mut r = RCtx {
            ci: [Box::default(), Box::default()],
            sci: [Box::default(), Box::default()],
            com: Box::default(),
            cplx: Box::default(),
            cplx_present: true,
            wb1: Box::default(),
            cpe: Box::default(),
            params: Box::default(),
            sri: SamplingRateInfo::default(),
            spec: vec![0; CTX_SPEC_LEN],
            scratch: vec![0; CTX_SCRATCH_LEN],
            out: [vec![0; CTX_OUT_LEN], vec![0; CTX_OUT_LEN]],
            arco: [Box::default(), Box::default()],
            usac: false,
            bsbuf: vec![0; BS_MAX],
            bs: FdkBitStream::default(),
            slopes: slopes(),
        };
        for s in r.sci.iter_mut() {
            s.i_mdct.overlap = vec![0; OVERLAP_LEN];
        }
        r
    }

    /// Loads (`im.load`) or dumps the image of the context.
    pub fn image(&mut self, im: &mut Img) {
        let usac = self.usac;
        for ch in 0..2 {
            im.sec(&format!("ci[{ch}]"));
            img_ci(im, &mut self.ci[ch], usac);
        }
        for ch in 0..2 {
            im.sec(&format!("sci[{ch}]"));
            let (s, a) = (&mut self.sci[ch], &mut self.arco[ch]);
            img_sci(im, s, a, &self.slopes);
        }
        im.sec("com.pnsInterChannelData");
        im.au8(&mut self.com.pns_inter_channel_data.correlated);
        im.sec("com.pnsRandomSeed");
        im.a32(&mut self.com.pns_random_seed);
        im.sec("com.jointStereoData");
        img_jsd(im, &mut self.com.joint_stereo_data);
        let p = im.raw(self.cplx_present as i32);
        self.cplx_present = p != 0;
        im.sec("cplx");
        img_cplx(im, &mut self.cplx);
        im.sec("cpe");
        img_jsp(im, &mut self.cpe.joint_stereo_persistent_data);
        im.sec("wb1");
        im.a32(&mut self.wb1.buf);
        im.sec("spec");
        im.a32(&mut self.spec);
        im.sec("params");
        img_params(im, &mut self.params);
        im.sec("scratch");
        im.a32(&mut self.scratch);
        im.sec("out[0]");
        im.a32(&mut self.out[0]);
        im.sec("out[1]");
        im.a32(&mut self.out[1]);
    }

    pub fn dump(&mut self) -> Img {
        let mut im = Img::dump();
        self.image(&mut im);
        im
    }

    pub fn load(&mut self, v: &[i32]) {
        let mut im = Img::loader(v.to_vec());
        self.image(&mut im);
    }

    /// `getSamplingRateInfo()` into `sri`.
    pub fn set_sri(&mut self, spf: u32, sri: u32, sr: u32) -> i32 {
        self.sri = SamplingRateInfo::default();
        get_sampling_rate_info(&mut self.sri, spf, sri, sr)
    }

    pub fn set_bs(&mut self, data: &[u8], buf_bytes: usize, valid_bits: u32) {
        self.bsbuf.iter_mut().for_each(|b| *b = 0);
        self.bsbuf[..data.len()].copy_from_slice(data);
        fdk_init_bit_stream(&mut self.bs, &BitMem::from_slice(&self.bsbuf[..]), buf_bytes as u32, valid_bits, BS_READER);
    }

    pub fn bs_valid(&mut self) -> i32 {
        fdk_get_valid_bits(&mut self.bs) as i32
    }

    /// `pComStaticData` for a call (`cplxPredictionData` is `NULL` unless `cplx_present`).
    pub fn take_com_static(&mut self) -> CAacDecoderCommonStaticData {
        CAacDecoderCommonStaticData {
            cplx_prediction_data: if self.cplx_present {
                Some(std::mem::take(&mut self.cplx))
            } else {
                None
            },
        }
    }

    pub fn put_com_static(&mut self, cs: CAacDecoderCommonStaticData) {
        if let Some(c) = cs.cplx_prediction_data {
            self.cplx = c;
        }
    }
}

/// The C++ context.
pub struct CCtx(pub *mut core::ffi::c_void);

impl CCtx {
    pub fn new() -> CCtx {
        CCtx(unsafe { cx::cref_u10ctx_new() })
    }
    pub fn load(&mut self, v: &[i32], usac: bool) {
        let mut v = v.to_vec();
        unsafe { cx::cref_u10ctx_image(self.0, v.as_mut_ptr(), 1, usac as i32) };
    }
    pub fn dump(&mut self, usac: bool) -> Vec<i32> {
        let n = unsafe { cx::cref_u10ctx_image(self.0, std::ptr::null_mut(), 0, usac as i32) } as usize;
        let mut v = vec![0; n];
        unsafe { cx::cref_u10ctx_image(self.0, v.as_mut_ptr(), 0, usac as i32) };
        v
    }
    pub fn set_sri(&mut self, spf: u32, sri: u32, sr: u32) -> i32 {
        unsafe { cx::cref_u10ctx_sri(self.0, spf, sri, sr) }
    }
    pub fn set_bs(&mut self, data: &[u8], buf_bytes: usize, valid_bits: u32) {
        unsafe { cx::cref_u10ctx_bs(self.0, data.as_ptr(), data.len() as i32, buf_bytes as i32, valid_bits as i32) }
    }
    pub fn bs_valid(&mut self) -> i32 {
        unsafe { cx::cref_u10ctx_bs_valid(self.0) }
    }
}

impl Drop for CCtx {
    fn drop(&mut self) {
        unsafe { cx::cref_u10ctx_free(self.0) };
    }
}

/// A pair of contexts with the same state.
pub struct Pair {
    pub r: RCtx,
    pub c: CCtx,
}

impl Pair {
    pub fn new() -> Pair {
        Pair { r: RCtx::new(), c: CCtx::new() }
    }

    /// Copies the state of the Rust context into the C++ context.
    pub fn sync(&mut self) {
        let im = self.r.dump();
        self.c.load(&im.v, self.r.usac);
    }

    /// Loads `v` into the Rust context and then copies its (truncated) state to the C++ context.
    pub fn load(&mut self, v: &[i32]) {
        self.r.load(v);
        self.sync();
    }

    pub fn set_sri(&mut self, spf: u32, sri: u32, sr: u32) -> i32 {
        let a = self.r.set_sri(spf, sri, sr);
        let b = self.c.set_sri(spf, sri, sr);
        assert_eq!(a, b, "getSamplingRateInfo({spf}, {sri}, {sr})");
        a
    }

    pub fn set_bs(&mut self, data: &[u8], buf_bytes: usize, valid_bits: u32) {
        self.r.set_bs(data, buf_bytes, valid_bits);
        self.c.set_bs(data, buf_bytes, valid_bits);
    }

    /// Compares the complete state; `what` describes the case.
    pub fn check(&mut self, what: &str) {
        let im = self.r.dump();
        let cv = self.c.dump(self.r.usac);
        assert_eq!(im.v.len(), cv.len(), "{what}: image length");
        if let Some(i) = (0..cv.len()).find(|&i| im.v[i] != cv[i]) {
            panic!(
                "{what}: state differs at {} (index {i}): rust {} c {}",
                im.where_is(i),
                im.v[i],
                cv[i]
            );
        }
    }

    pub fn check_bs(&mut self, what: &str) {
        assert_eq!(self.r.bs_valid(), self.c.bs_valid(), "{what}: bitstream position");
    }
}

/// A random image of the context (every field random within its type).
pub fn random_image(rng: &mut Rng, len: usize) -> Vec<i32> {
    (0..len).map(|_| rng.i32()).collect()
}

#[test]
fn image_roundtrip() {
    // The image of a loaded random state, truncated by the field types of both sides, must give
    // the same values on both sides; this checks the field order of the two images.
    let mut p = Pair::new();
    let mut rng = Rng::new(0x10_c7c0);
    for usac in [false, true] {
        p.r.usac = usac;
        let n = p.c.dump(usac).len();
        assert_eq!(p.r.dump().v.len(), n, "image length (usac {usac})");
        for _ in 0..20 {
            let v = random_image(&mut rng, n);
            p.r.load(&v);
            p.c.load(&v, usac);
            p.check(&format!("roundtrip usac {usac}"));
        }
    }
}

#[test]
fn layout_spec_scale_before_ics_info() {
    // The C layout, which the port reproduces: the byte in front of icsInfo (read by
    // CConcealment_Store()) is the high byte of specScale[7], without padding in between.
    assert_eq!(unsafe { cx::cref_u10ctx_ci_specscale_ics_gap() }, 0);
    let p = Pair::new();
    assert_eq!(unsafe { cx::cref_u10ctx_num_slopes(p.c.0) } as usize, p.r.slopes.len());
}

// ---------------------------------------------------------------------------------------------
// Random states for the tests of the channel modules
// ---------------------------------------------------------------------------------------------

/// (samples per frame, sampling rate index) combinations with scale factor band tables.
pub const CONFIGS: [(u32, u32); 14] = [
    (1024, 3),
    (1024, 4),
    (1024, 8),
    (1024, 11),
    (1024, 0),
    (960, 3),
    (960, 6),
    (768, 3),
    (768, 7),
    (512, 3),
    (512, 6),
    (480, 3),
    (480, 5),
    (1024, 12),
];

/// The sampling rate of a sampling rate index.
pub fn rate_of(sri: u32) -> u32 {
    const R: [u32; 13] =
        [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    R[sri as usize]
}

/// A random configuration of `CONFIGS`, set on both sides; returns the frame length.
pub fn random_config(p: &mut Pair, rng: &mut Rng, allow_ld: bool) -> u32 {
    loop {
        let (spf, sri) = CONFIGS[(rng.next_u32() % CONFIGS.len() as u32) as usize];
        if !allow_ld && (spf == 512 || spf == 480) {
            continue;
        }
        assert_eq!(p.set_sri(spf, sri, rate_of(sri)), 0);
        return spf;
    }
}

/// A random valid `CIcsInfo` as `IcsRead()` produces it (`short_ok`: short blocks allowed).
/// `WindowGroupLength` entries behind the groups keep their (random) values.
pub fn random_ics(rng: &mut Rng, ics: &mut CIcsInfo, sri: &SamplingRateInfo, short_ok: bool) {
    let short = short_ok && sri.number_of_scale_factor_bands_short > 0 && rng.next_u32() % 3 == 0;
    ics.valid = 1;
    ics.window_shape = (rng.next_u32() % 2) as u8;
    if short {
        ics.window_sequence = 2;
        ics.total_sf_bands = sri.number_of_scale_factor_bands_short;
        ics.max_sf_bands = (rng.next_u32() % (ics.total_sf_bands as u32 + 1)) as u8;
        ics.scale_factor_grouping = (rng.next_u32() % 128) as u8;
        ics.window_groups = 0;
        for i in 0..7 {
            let mask = 1u32 << (6 - i);
            ics.window_group_length[i] = 1;
            if ics.scale_factor_grouping as u32 & mask != 0 {
                ics.window_group_length[ics.window_groups as usize] += 1;
            } else {
                ics.window_groups += 1;
            }
        }
        ics.window_group_length[7] = 1;
        ics.window_groups += 1;
    } else if sri.number_of_scale_factor_bands_short == 0 {
        // AAC-LD/ELD (frame lengths 512 and 480): long blocks only, low overlap instead of KBD
        ics.window_sequence = 0;
        ics.window_shape = [0, 2][(rng.next_u32() % 2) as usize];
        ics.total_sf_bands = sri.number_of_scale_factor_bands_long;
        ics.max_sf_bands = (rng.next_u32() % (ics.total_sf_bands as u32 + 1)) as u8;
        ics.window_groups = 1;
        ics.window_group_length[0] = 1;
    } else {
        ics.window_sequence = [0, 1, 3][(rng.next_u32() % 3) as usize];
        ics.total_sf_bands = sri.number_of_scale_factor_bands_long;
        ics.max_sf_bands = match rng.next_u32() % 4 {
            0 => ics.total_sf_bands,
            _ => (rng.next_u32() % (ics.total_sf_bands as u32 + 1)) as u8,
        };
        ics.window_groups = 1;
        ics.window_group_length[0] = 1;
    }
}

/// A random quantized spectral value (mostly small, sometimes escapes and the limits).
pub fn random_quant(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 16 {
        0..=5 => 0,
        6..=11 => rng.range(-8, 8),
        12 | 13 => rng.range(-300, 300),
        14 => rng.range(-8191, 8191),
        _ => [8191, -8191, 8192, -8192, 1, -1][(rng.next_u32() % 6) as usize],
    }
}

/// A random spectral value of a decoded spectrum (any magnitude).
pub fn random_spec(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 8 {
        0 => 0,
        1 => rng.i32(),
        _ => (rng.next_u32() as i32) >> (rng.next_u32() % 32),
    }
}

/// Random bytes, with runs of 0x00 and 0xFF.
pub fn random_bytes(rng: &mut Rng, n: usize) -> Vec<u8> {
    let mode = rng.next_u32() % 6;
    (0..n)
        .map(|_| match mode {
            0 => 0xFF,
            1 => 0x00,
            2 => {
                if rng.next_u32() % 4 == 0 {
                    rng.next_u32() as u8
                } else {
                    0
                }
            }
            _ => rng.next_u32() as u8,
        })
        .collect()
}

/// Sets the per-channel defaults of a decoder for frame length `spf` in the Rust context: the
/// granule length and an initialized IMDCT (`mdct_init` with `OverlapBufferSize`, cleared).
pub fn init_channels(p: &mut Pair, spf: u32) {
    for ch in 0..2 {
        p.r.ci[ch].granule_length = spf as i32 / 8;
        let m = &mut p.r.sci[ch].i_mdct;
        fdk_aac_rs_fdk::mdct::mdct_init(m, OVERLAP_LEN as i32);
        m.overlap.iter_mut().for_each(|x| *x = 0);
        m.prev_wrs = None;
        m.ov_offset = 0;
    }
}

/// A state of the context with random contents, the configuration of a random frame length
/// (set on both sides), valid `CIcsInfo`s and initialized channels; returns the frame length.
/// The C++ side is synchronized by the caller after further adjustments ([`Pair::sync`]).
pub fn random_state(p: &mut Pair, rng: &mut Rng, usac: bool, allow_ld: bool, short_ok: bool) -> u32 {
    p.r.usac = usac;
    let n = p.r.dump().v.len();
    let v = random_image(rng, n);
    p.r.load(&v);
    let spf = random_config(p, rng, allow_ld);
    init_channels(p, spf);
    let sri = p.r.sri;
    for ch in 0..2 {
        random_ics(rng, &mut p.r.ci[ch].ics_info, &sri, short_ok);
    }
    spf
}
