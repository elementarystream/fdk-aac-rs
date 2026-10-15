// The LPD channel state as the INT arrays of the shim aacdec_usacdec_lpd.cpp (sci_store(),
// sci_load(), ci_store(), ci_load()), shared by the differential tests of the USAC LPD modules.

#![allow(dead_code)]

use fdk_aac_rs_aacdec::channelinfo::{CAacDecoderChannelInfo, CAacDecoderStaticChannelInfo, FacDataPtr};
use fdk_aac_rs_aacdec::usacdec_acelp::CAcelpStaticMem;
use fdk_aac_rs_arith::ac_arith_coder::CArcoData;
use fdk_aac_rs_fdk::archdef::FixpWtp;
use fdk_aac_rs_fdk::tools_rom::fdk_get_window_slope;

pub const OVL_MAX: usize = 1024;
pub const ZIR_MAX: usize = 428;
const ARCO_PREV: usize = 516;

/// `WIN_CAND` of the shim: the window slopes an IMdct can point to.
pub const WIN_CAND: [(i32, i32); 32] = [
    (8, 0),
    (16, 0),
    (32, 0),
    (64, 0),
    (128, 0),
    (256, 0),
    (512, 0),
    (1024, 0),
    (120, 0),
    (240, 0),
    (480, 0),
    (960, 0),
    (12, 0),
    (24, 0),
    (48, 0),
    (96, 0),
    (192, 0),
    (384, 0),
    (768, 0),
    (20, 0),
    (40, 0),
    (160, 0),
    (128, 1),
    (256, 1),
    (1024, 1),
    (120, 1),
    (960, 1),
    (96, 1),
    (768, 1),
    (512, 1),
    (480, 1),
    (192, 1),
];

/// `win_id()` of the shim.
pub fn win_id(w: Option<&'static [FixpWtp]>) -> i32 {
    let Some(w) = w else { return -1 };
    for (i, &(l, s)) in WIN_CAND.iter().enumerate() {
        if let Some(c) = fdk_get_window_slope(l, s) {
            if core::ptr::eq(c.as_ptr(), w.as_ptr()) {
                return i as i32;
            }
        }
    }
    -2
}

/// `win_ptr()` of the shim.
pub fn win_ptr(id: i32) -> Option<&'static [FixpWtp]> {
    if id < 0 || id as usize >= WIN_CAND.len() {
        return None;
    }
    let (l, s) = WIN_CAND[id as usize];
    fdk_get_window_slope(l, s)
}

fn acelp_mem_store(m: &CAcelpStaticMem, v: &mut Vec<i32>) {
    v.extend_from_slice(&m.old_exc_mem);
    v.extend_from_slice(&m.old_syn_mem);
    v.extend(m.a.iter().map(|&x| x as i32));
    v.extend_from_slice(&[
        m.a_exp,
        m.gc_threshold,
        m.de_emph_mem,
        m.past_gpit as i32,
        m.past_gcode,
        m.old_t0 as i32,
        m.old_t0_frac as i32,
        m.deemph_mem_wsyn,
        m.wsyn_rms,
        m.seed_ace as i32,
    ]);
}

/// A reader over an INT array.
struct Rd<'a> {
    v: &'a [i32],
    n: usize,
}

impl Rd<'_> {
    fn next(&mut self) -> i32 {
        let x = self.v[self.n];
        self.n += 1;
        x
    }
    fn skip(&mut self, k: usize) {
        self.n += k;
    }
}

fn acelp_mem_load(m: &mut CAcelpStaticMem, r: &mut Rd) {
    for i in 0..428 {
        m.old_exc_mem[i] = r.next();
    }
    for i in 0..16 {
        m.old_syn_mem[i] = r.next();
    }
    for i in 0..16 {
        m.a[i] = r.next() as i16;
    }
    m.a_exp = r.next();
    m.gc_threshold = r.next();
    m.de_emph_mem = r.next();
    m.past_gpit = r.next() as i16;
    m.past_gcode = r.next();
    m.old_t0 = r.next() as u16;
    m.old_t0_frac = r.next() as u8;
    m.deemph_mem_wsyn = r.next();
    m.wsyn_rms = r.next();
    m.seed_ace = r.next() as i16;
}

/// `sci_store()` of the shim.
pub fn sci_to_vec(s: &CAacDecoderStaticChannelInfo) -> Vec<i32> {
    let mut v = Vec::with_capacity(3000);
    let m = &s.i_mdct;
    v.push(win_id(m.prev_wrs));
    v.extend_from_slice(&[
        m.prev_tl,
        m.prev_nr,
        m.prev_fr,
        m.ov_offset,
        m.ov_size,
        m.prev_alias_symmetry,
        m.prev_prev_alias_symmetry,
    ]);
    match &m.p_fac_zir {
        Some(z) => {
            v.push(1);
            v.push(z.len() as i32);
            for i in 0..ZIR_MAX {
                v.push(z.get(i).copied().unwrap_or(0));
            }
        }
        None => {
            v.push(0);
            v.push(0);
            v.extend(core::iter::repeat_n(0, ZIR_MAX));
        }
    }
    v.push(m.p_asym_ovlp.is_some() as i32);
    for i in 0..OVL_MAX {
        v.push(if (i as i32) < m.ov_size { m.overlap.get(i).copied().unwrap_or(0) } else { 0 });
    }
    match &s.h_ar_co {
        Some(a) => {
            v.push(1);
            v.push(a.m_number_lines_prev as i32);
            v.extend(a.c_prev.iter().map(|&x| x as i32));
        }
        None => {
            v.push(0);
            v.push(0);
            v.extend(core::iter::repeat_n(0, ARCO_PREV));
        }
    }
    v.extend_from_slice(&s.old_synth);
    v.extend_from_slice(&s.old_t_pf);
    v.extend_from_slice(&s.old_gain_pf);
    v.extend_from_slice(&s.mem_bpf);
    v.extend_from_slice(&[
        s.old_bpf_control_info as i32,
        s.last_core_mode,
        s.last_lpd_mode as i32,
        s.last_last_lpd_mode as i32,
        s.last_lpc_lost as i32,
    ]);
    v.extend(s.lpc4_lsf.iter().map(|&x| x as i32));
    v.extend(s.lsf_adaptive_mean.iter().map(|&x| x as i32));
    for k in 0..2 {
        v.extend(s.lp_coeff_old[k].iter().map(|&x| x as i32));
    }
    v.extend_from_slice(&s.lp_coeff_old_exp);
    v.push(s.old_stability as i32);
    v.push(s.num_lost_lpd_frames as i32);
    v.push(s.last_tcx_gain);
    v.push(s.last_tcx_gain_e);
    v.extend_from_slice(&s.last_alfd_gains);
    v.push(s.last_tcx_pitch as i32);
    v.push(s.last_tcx_noise_factor as i32);
    acelp_mem_store(&s.acelp, &mut v);
    v.push(s.nf_random_seed as i32);
    v.push(s.concealment_info.prev_frame_ok[0]);
    v.push(s.concealment_info.prev_frame_ok[1]);
    v
}

/// `sci_load()` of the shim (the fields that are not serialized are default).
pub fn sci_from_vec(v: &[i32]) -> CAacDecoderStaticChannelInfo {
    let mut s = CAacDecoderStaticChannelInfo::default();
    let mut r = Rd { v, n: 0 };
    {
        let m = &mut s.i_mdct;
        m.prev_wrs = win_ptr(r.next());
        m.prev_tl = r.next();
        m.prev_nr = r.next();
        m.prev_fr = r.next();
        m.ov_offset = r.next();
        m.ov_size = r.next();
        m.prev_alias_symmetry = r.next();
        m.prev_prev_alias_symmetry = r.next();
        let flag = r.next();
        let nvalid = r.next();
        let z = &v[r.n..r.n + ZIR_MAX];
        r.skip(ZIR_MAX);
        m.p_fac_zir = if flag != 0 {
            let k = if nvalid >= 0 { nvalid as usize } else { 128 };
            Some(z[..k.min(ZIR_MAX)].to_vec())
        } else {
            None
        };
        let _asym = r.next();
        m.p_asym_ovlp = None;
        let ov = (m.ov_size.max(0) as usize).min(OVL_MAX);
        m.overlap = v[r.n..r.n + ov].to_vec();
        r.skip(OVL_MAX);
    }
    if r.next() != 0 {
        let mut a = CArcoData::default();
        a.m_number_lines_prev = r.next() as i16;
        for i in 0..ARCO_PREV {
            a.c_prev[i] = r.next() as u8;
        }
        s.h_ar_co = Some(Box::new(a));
    } else {
        r.skip(1 + ARCO_PREV);
    }
    for i in 0..s.old_synth.len() {
        s.old_synth[i] = r.next();
    }
    for i in 0..s.old_t_pf.len() {
        s.old_t_pf[i] = r.next();
    }
    for i in 0..s.old_gain_pf.len() {
        s.old_gain_pf[i] = r.next();
    }
    for i in 0..s.mem_bpf.len() {
        s.mem_bpf[i] = r.next();
    }
    s.old_bpf_control_info = r.next() as u8;
    s.last_core_mode = r.next();
    s.last_lpd_mode = r.next() as u8;
    s.last_last_lpd_mode = r.next() as u8;
    s.last_lpc_lost = r.next() as u8;
    for i in 0..16 {
        s.lpc4_lsf[i] = r.next() as i16;
    }
    for i in 0..16 {
        s.lsf_adaptive_mean[i] = r.next() as i16;
    }
    for k in 0..2 {
        for i in 0..16 {
            s.lp_coeff_old[k][i] = r.next() as i16;
        }
    }
    s.lp_coeff_old_exp[0] = r.next();
    s.lp_coeff_old_exp[1] = r.next();
    s.old_stability = r.next() as i16;
    s.num_lost_lpd_frames = r.next() as u32;
    s.last_tcx_gain = r.next();
    s.last_tcx_gain_e = r.next();
    for i in 0..32 {
        s.last_alfd_gains[i] = r.next();
    }
    s.last_tcx_pitch = r.next() as i16;
    s.last_tcx_noise_factor = r.next() as u8;
    acelp_mem_load(&mut s.acelp, &mut r);
    s.nf_random_seed = r.next() as u32;
    s.concealment_info.prev_frame_ok[0] = r.next();
    s.concealment_info.prev_frame_ok[1] = r.next();
    assert_eq!(r.n, v.len(), "sci length");
    s
}

fn fac_code(p: FacDataPtr) -> i32 {
    match p {
        FacDataPtr::Null => -1,
        FacDataPtr::FacData0 => -2,
        FacDataPtr::Scratch => -3,
        FacDataPtr::Spectral(o) => o as i32,
    }
}

fn fac_ptr(code: i32) -> FacDataPtr {
    match code {
        -1 => FacDataPtr::Null,
        -2 => FacDataPtr::FacData0,
        c if c >= 0 => FacDataPtr::Spectral(c as usize),
        _ => FacDataPtr::Scratch,
    }
}

/// `ci_store()` of the shim.
pub fn ci_to_vec(c: &CAacDecoderChannelInfo) -> Vec<i32> {
    let mut v = Vec::with_capacity(700);
    let u = &c.data.usac;
    v.extend_from_slice(&u.fac_data0);
    v.extend(u.fac_data_e.iter().map(|&x| x as i32));
    v.extend(u.fac_data.iter().map(|&p| fac_code(p)));
    v.push(u.core_mode as i32);
    v.push(u.core_mode_last);
    v.push(u.lpd_mode_last as i32);
    v.extend(u.r#mod.iter().map(|&x| x as i32));
    v.push(u.bpf_control_info as i32);
    for k in 0..5 {
        v.extend(u.lsp_coeff[k].iter().map(|&x| x as i32));
    }
    for k in 0..5 {
        v.extend(u.lp_coeff[k].iter().map(|&x| x as i32));
    }
    v.extend_from_slice(&u.lp_coeff_exp);
    v.extend(u.lsf_adaptive_mean_cand.iter().map(|&x| x as i32));
    v.extend(u.a_stability.iter().map(|&x| x as i32));
    for d in &u.acelp {
        v.push(d.acelp_core_mode as i32);
        v.push(d.mean_energy as i32);
        v.extend(d.t0.iter().map(|&x| x as i32));
        v.extend(d.t0_frac.iter().map(|&x| x as i32));
        v.extend(d.ltp_filtering_flag.iter().map(|&x| x as i32));
        for row in &d.icb_index {
            v.extend(row.iter().map(|&x| x as i32));
        }
        v.extend(d.gains.iter().map(|&x| x as i32));
    }
    v.extend_from_slice(&u.tcx_gain);
    v.extend(u.tcx_gain_e.iter().map(|&x| x as i32));
    v.extend(c.spec_scale.iter().map(|&x| x as i32));
    v.push(c.ics_info.window_shape as i32);
    v.push(c.ics_info.window_sequence);
    v.push(c.granule_length);
    v.push(c.curr_aliasing_symmetry);
    v.push(c.render_mode);
    let du = &c.p_dyn_data.specific_to.usac;
    v.extend(du.tcx_noise_factor.iter().map(|&x| x as i32));
    v.extend(du.tcx_global_gain.iter().map(|&x| x as i32));
    v
}

/// `ci_load()` of the shim (the fields that are not serialized are default).
pub fn ci_from_vec(v: &[i32]) -> CAacDecoderChannelInfo {
    let mut c = CAacDecoderChannelInfo::default();
    let mut r = Rd { v, n: 0 };
    {
        let u = &mut c.data.usac;
        for i in 0..u.fac_data0.len() {
            u.fac_data0[i] = r.next();
        }
        for i in 0..4 {
            u.fac_data_e[i] = r.next() as i8;
        }
        for i in 0..4 {
            u.fac_data[i] = fac_ptr(r.next());
        }
        u.core_mode = r.next() as u8;
        u.core_mode_last = r.next();
        u.lpd_mode_last = r.next() as u8;
        for i in 0..4 {
            u.r#mod[i] = r.next() as u8;
        }
        u.bpf_control_info = r.next() as u8;
        for k in 0..5 {
            for i in 0..16 {
                u.lsp_coeff[k][i] = r.next() as i16;
            }
        }
        for k in 0..5 {
            for i in 0..16 {
                u.lp_coeff[k][i] = r.next() as i16;
            }
        }
        for k in 0..5 {
            u.lp_coeff_exp[k] = r.next();
        }
        for i in 0..16 {
            u.lsf_adaptive_mean_cand[i] = r.next() as i16;
        }
        for i in 0..4 {
            u.a_stability[i] = r.next() as i16;
        }
        for d in u.acelp.iter_mut() {
            d.acelp_core_mode = r.next() as u8;
            d.mean_energy = r.next() as u8;
            for i in 0..4 {
                d.t0[i] = r.next() as u16;
            }
            for i in 0..4 {
                d.t0_frac[i] = r.next() as u8;
            }
            for i in 0..4 {
                d.ltp_filtering_flag[i] = r.next() as u8;
            }
            for i in 0..4 {
                for j in 0..8 {
                    d.icb_index[i][j] = r.next() as i16;
                }
            }
            for i in 0..4 {
                d.gains[i] = r.next() as u8;
            }
        }
        for i in 0..4 {
            u.tcx_gain[i] = r.next();
        }
        for i in 0..4 {
            u.tcx_gain_e[i] = r.next() as i8;
        }
    }
    for i in 0..8 {
        c.spec_scale[i] = r.next() as i16;
    }
    c.ics_info.window_shape = r.next() as u8;
    c.ics_info.window_sequence = r.next();
    c.granule_length = r.next();
    c.curr_aliasing_symmetry = r.next();
    c.render_mode = r.next();
    for i in 0..4 {
        c.p_dyn_data.specific_to.usac.tcx_noise_factor[i] = r.next() as u8;
    }
    for i in 0..4 {
        c.p_dyn_data.specific_to.usac.tcx_global_gain[i] = r.next() as u8;
    }
    assert_eq!(r.n, v.len(), "ci length");
    c
}
