// Differential tests of fdk_aac_rs_fdk::qmf_domain against the C++ reference.
//
// Random sequences of the QMF domain API (requested configurations as SBR, MPS and the explicit
// USAC configuration set them, Configure, ClearRequested, FreeMem, Close, InitFilterBank with
// extra flags, ClearPersistentMemory, filter bank fields set by SBR/MPS, SaveOverlap, GetSlot,
// GetWorkBuffer, WorkBuffer2ProcChannel, QmfData2HBE) run on a C++ domain and on the port side
// by side. After every call the return values, the whole configuration, every channel
// (filter banks, scaling, work buffer assignment, buffer presence), the time slot pointers and
// the complete memory (work buffers, overlap buffers, filter states) are compared.
//
// The sequences avoid inputs without a defined reference result (see `configure_safe()`; the
// port allocates the largest size, see qmf_domain.rs).

use core::ffi::c_void;
use fdk_aac_rs_cref::fdk_qmf_domain::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_domain::*;

const CH: usize = QMF_DOMAIN_MAX_CHANNELS;

struct CQd(*mut c_void);
impl Drop for CQd {
    fn drop(&mut self) {
        unsafe { cref_qd_free(self.0) }
    }
}
impl CQd {
    fn gc(&self) -> [i32; 27] {
        let mut o = [0i32; 27];
        unsafe { cref_qd_get_gc(self.0, o.as_mut_ptr()) };
        o
    }
    fn in_state(&self, ch: usize) -> [i32; 30] {
        let mut o = [0i32; 30];
        unsafe { cref_qd_in_state(self.0, ch as i32, o.as_mut_ptr()) };
        o
    }
    fn out_state(&self, ch: usize) -> [i32; 17] {
        let mut o = [0i32; 17];
        unsafe { cref_qd_out_state(self.0, ch as i32, o.as_mut_ptr()) };
        o
    }
    fn slots(&self, ch: usize, n: usize) -> Vec<i32> {
        let mut o = vec![0i32; 6 * n];
        unsafe { cref_qd_in_slots(self.0, ch as i32, n as i32, o.as_mut_ptr()) };
        o
    }
    fn mem(&self, kind: i32, idx: usize, n: usize) -> Option<Vec<i32>> {
        let mut o = vec![0i32; n];
        (unsafe { cref_qd_get_mem(self.0, kind, idx as i32, n as i32, o.as_mut_ptr()) } != 0)
            .then_some(o)
    }
}

fn gc_fields(g: &FdkQmfDomainGc) -> [i32; 21] {
    [
        g.qmf_domain_explicit_config as i32,
        g.n_input_channels as i32,
        g.n_input_channels_requested as i32,
        g.n_output_channels as i32,
        g.n_output_channels_requested as i32,
        g.park_channel as i32,
        g.park_channel_requested as i32,
        g.flags as i32,
        g.flags_requested as i32,
        g.n_bands_analysis as i32,
        g.n_bands_analysis_requested as i32,
        g.n_bands_synthesis as i32,
        g.n_bands_synthesis_requested as i32,
        g.n_qmf_time_slots as i32,
        g.n_qmf_time_slots_requested as i32,
        g.n_qmf_ov_time_slots as i32,
        g.n_qmf_ov_time_slots_requested as i32,
        g.n_qmf_proc_bands as i32,
        g.n_qmf_proc_bands_requested as i32,
        g.n_qmf_proc_channels as i32,
        g.n_qmf_proc_channels_requested as i32,
    ]
}

fn set_gc_fields(g: &mut FdkQmfDomainGc, v: &[i32]) {
    g.qmf_domain_explicit_config = v[0] as u8;
    g.n_input_channels = v[1] as u8;
    g.n_input_channels_requested = v[2] as u8;
    g.n_output_channels = v[3] as u8;
    g.n_output_channels_requested = v[4] as u8;
    g.park_channel = v[5] as u8;
    g.park_channel_requested = v[6] as u8;
    g.flags = v[7] as u32;
    g.flags_requested = v[8] as u32;
    g.n_bands_analysis = v[9] as u8;
    g.n_bands_analysis_requested = v[10] as u8;
    g.n_bands_synthesis = v[11] as u16;
    g.n_bands_synthesis_requested = v[12] as u16;
    g.n_qmf_time_slots = v[13] as u8;
    g.n_qmf_time_slots_requested = v[14] as u8;
    g.n_qmf_ov_time_slots = v[15] as u8;
    g.n_qmf_ov_time_slots_requested = v[16] as u8;
    g.n_qmf_proc_bands = v[17] as u8;
    g.n_qmf_proc_bands_requested = v[18] as u8;
    g.n_qmf_proc_channels = v[19] as u8;
    g.n_qmf_proc_channels_requested = v[20] as u8;
}

fn r_gc(qd: &FdkQmfDomain) -> [i32; 27] {
    let mut o = [0i32; 27];
    o[..21].copy_from_slice(&gc_fields(&qd.global_conf));
    for k in 0..5 {
        o[21 + k] = qd.mem.work_buffer[k].is_some() as i32;
    }
    o[26] = fdk_qmf_domain_is_initialized(qd);
    o
}

fn r_fb(fb: &QmfFilterBank, states: bool) -> [i32; 15] {
    [
        if !fb.filter_states {
            0
        } else if states {
            1
        } else {
            -1
        },
        fb.filter_size,
        fb.filter_scale,
        fb.no_channels,
        fb.no_col,
        fb.lsb,
        fb.usb,
        fb.syn_scalefactor,
        fb.out_scalefactor,
        fb.out_gain_m,
        fb.out_gain_e,
        fb.flags as i32,
        fb.p_stride as i32,
        !fb.p_filter.is_empty() as i32,
        !fb.t_cos.is_empty() as i32,
    ]
}

fn r_in(qd: &FdkQmfDomain, ch: usize) -> [i32; 30] {
    let q = &qd.qmf_domain_in[ch];
    let mut o = [0i32; 30];
    o[..15].copy_from_slice(&r_fb(&q.fb, q.p_ana_qmf_states.is_some()));
    o[15] = q.scaling.lb_scale;
    o[16] = q.scaling.ov_lb_scale;
    o[17] = q.scaling.hb_scale;
    o[18] = q.scaling.ov_hb_scale;
    o[19] = q.work_buf_n_time_slots as i32;
    o[20] = q.work_buf_n_bands as i32;
    o[21] = q.work_buffer_offset as i32;
    o[22] = q.work_buffer_sect_size as i32;
    o[23] = q.p_ana_qmf_states.is_some() as i32;
    o[24] = qd.mem.overlap_buffer[ch].is_some() as i32;
    o[25] = q.p_work_buffer as i32;
    o[26] = q.p_global_conf as i32;
    o[27] = q.h_qmf_slots_real.is_some() as i32;
    o[28] = q.h_qmf_slots_imag.is_some() as i32;
    o
}

fn r_out(qd: &FdkQmfDomain, ch: usize) -> [i32; 17] {
    let q = &qd.qmf_domain_out[ch];
    let mut o = [0i32; 17];
    o[..15].copy_from_slice(&r_fb(&q.fb, q.p_syn_qmf_states.is_some()));
    o[15] = q.p_syn_qmf_states.is_some() as i32;
    o
}

/// A pointer of the port in the encoding of the shim, also for unallocated buffers (0 for
/// offset 0, else -1).
fn r_ptr(qd: &FdkQmfDomain, p: QmfSlotPtr) -> [i32; 3] {
    match p {
        QmfSlotPtr::Null => [0, 0, 0],
        QmfSlotPtr::Work { sect, offset } => {
            if qd.mem.work_buffer.get(sect as usize).is_some_and(|b| b.is_some()) {
                [1, sect as i32, offset as i32]
            } else if offset == 0 {
                [0, 0, 0]
            } else {
                [-1, 0, 0]
            }
        }
        QmfSlotPtr::Overlap { ch, offset } => {
            if qd.mem.overlap_buffer[ch as usize].is_some() {
                [2, ch as i32, offset as i32]
            } else if offset == 0 {
                [0, 0, 0]
            } else {
                [-1, 0, 0]
            }
        }
    }
}

fn r_slots(qd: &FdkQmfDomain, ch: usize, n: usize) -> Vec<i32> {
    let q = &qd.qmf_domain_in[ch];
    let mut o = Vec::new();
    for ts in 0..n {
        o.extend(r_ptr(qd, q.h_qmf_slots_real.as_ref().unwrap()[ts]));
        o.extend(r_ptr(qd, q.h_qmf_slots_imag.as_ref().unwrap()[ts]));
    }
    o
}

/// The sizes of the buffers that the reference allocated.
#[derive(Clone, Copy, Default, Debug)]
struct Caps {
    ana: [Option<usize>; CH],
    slots: [Option<usize>; CH],
    ov: [Option<usize>; CH],
}

fn ana_class(bands: i32) -> usize {
    match bands {
        16 => 160,
        24 => 240,
        32 => 320,
        _ => 640,
    }
}
fn slots_class(ts: i32) -> usize {
    match ts {
        16 => 19,
        32 => 38,
        _ => 76,
    }
}
fn ov_class(ov: i32) -> usize {
    match ov {
        3 => 2 * 3 * 64,
        6 => 2 * 6 * 64,
        _ => 2 * 12 * 64,
    }
}

impl Caps {
    /// Records the buffers that the reference allocated or freed in the last call.
    fn update(&mut self, c: &CQd) {
        let g = c.gc();
        for ch in 0..CH {
            let s = c.in_state(ch);
            self.ana[ch] = (s[23] != 0).then(|| self.ana[ch].unwrap_or(ana_class(g[9])));
            self.ov[ch] = (s[24] != 0).then(|| self.ov[ch].unwrap_or(ov_class(g[15])));
            self.slots[ch] = (s[27] != 0).then(|| self.slots[ch].unwrap_or(slots_class(g[13])));
        }
    }
}

/// Whether `FDK_QmfDomain_Configure()` with the configuration `g` (the 21 fields) has a defined
/// reference result.
fn configure_safe(g: &[i32; 27], caps: &Caps) -> bool {
    let (n_in, n_in_r, n_out, n_out_r) = (g[1], g[2], g[3], g[4]);
    let (park, park_r) = (g[5], g[6]);
    let (ba, ba_r, bs, bs_r) = (g[9], g[10], g[11], g[12]);
    let (ts, ts_r, ov, ov_r) = (g[13], g[14], g[15], g[16]);
    let (pb, pb_r, pc, pc_r) = (g[17], g[18], g[19], g[20]);
    if pc_r > 0 && pb_r != 64 {
        return true; // QMF_DOMAIN_INIT_ERROR before any change
    }
    if ba_r > pb_r {
        return true; // QMF_DOMAIN_INIT_ERROR before any change
    }
    if pc_r as usize > CH {
        return false; // no defined result in the reference
    }
    if pc_r == 0 && ts_r != 0 {
        return false; // no defined result in the reference
    }
    let step1 = pc != pc_r || pb != pb_r || ts != ts_r;
    let pb_eff = if step1 && pc_r > 0 { pb_r } else { pb };
    let step2 = n_in != n_in_r
        || ba != ba_r
        || ts != ts_r
        || ov != ov_r
        || n_out != n_out_r
        || bs != bs_r
        || park != park_r;
    if step2 {
        if n_in_r as usize > CH || n_out_r as usize > CH {
            return true; // QMF_DOMAIN_OUT_OF_MEMORY, freed
        }
        for ch in 0..n_in_r as usize {
            let ana = caps.ana[ch].unwrap_or(ana_class(ba_r));
            if ba_r * 10 > 0 && (ba_r * 10) as usize > ana {
                return false;
            }
            let sl = caps.slots[ch].unwrap_or(slots_class(ts_r));
            if ov_r + ts_r > 0 && (ov_r + ts_r) as usize > sl {
                return false;
            }
            let o = caps.ov[ch].unwrap_or(ov_class(ov_r));
            if ov_r * pb_eff * 2 > 0 && (ov_r * pb_eff * 2) as usize > o {
                return false;
            }
        }
    } else {
        for ch in 0..(n_in as usize).min(CH) {
            if let Some(o) = caps.ov[ch] {
                if (ov * pb_eff * 2) as usize > o {
                    return false;
                }
            }
        }
    }
    true
}

/// Compares the complete state of both domains.
fn compare(qd: &FdkQmfDomain, c: &CQd, caps: &Caps, w: &str) {
    let g = c.gc();
    assert_eq!(r_gc(qd), g, "{w}: configuration");
    for ch in 0..CH {
        let cs = c.in_state(ch);
        assert_eq!(r_in(qd, ch), cs, "{w}: input channel {ch}");
        assert_eq!(r_out(qd, ch), c.out_state(ch), "{w}: output channel {ch}");
        if cs[27] != 0 {
            let n = ((g[15] + g[13]) as usize).min(caps.slots[ch].unwrap_or(0));
            assert_eq!(r_slots(qd, ch, n), c.slots(ch, n), "{w}: slots of channel {ch}");
        }
        if let Some(cap) = caps.ana[ch] {
            let cm = c.mem(2, ch, cap).unwrap();
            assert_eq!(&qd.qmf_domain_in[ch].p_ana_qmf_states.as_ref().unwrap()[..cap], &cm[..], "{w}: analysis states {ch}");
        }
        if let Some(cap) = caps.ov[ch] {
            let cm = c.mem(1, ch, cap).unwrap();
            assert_eq!(&qd.mem.overlap_buffer[ch].as_ref().unwrap()[..cap], &cm[..], "{w}: overlap {ch}");
        }
        match c.mem(3, ch, 576) {
            Some(cm) => assert_eq!(&qd.qmf_domain_out[ch].p_syn_qmf_states.as_ref().unwrap()[..], &cm[..], "{w}: synthesis states {ch}"),
            None => assert!(qd.qmf_domain_out[ch].p_syn_qmf_states.is_none()),
        }
    }
    for k in 0..5 {
        match c.mem(0, k, 2048) {
            Some(cm) => assert_eq!(&qd.mem.work_buffer[k].as_ref().unwrap()[..], &cm[..], "{w}: work buffer {k}"),
            None => assert!(qd.mem.work_buffer[k].is_none()),
        }
    }
}

/// Fills all allocated memory of both domains with the same random values.
fn fill_memory(qd: &mut FdkQmfDomain, c: &CQd, caps: &Caps, rng: &mut Rng) {
    let fill = |kind: i32, idx: usize, buf: &mut [i32], n: usize, rng: &mut Rng| {
        let v: Vec<i32> = (0..n).map(|_| rng.i32()).collect();
        buf[..n].copy_from_slice(&v);
        unsafe { cref_qd_set_mem(c.0, kind, idx as i32, n as i32, v.as_ptr()) };
    };
    for k in 0..5 {
        if let Some(b) = qd.mem.work_buffer[k].as_deref_mut() {
            fill(0, k, b, 2048, rng);
        }
    }
    for ch in 0..CH {
        if let (Some(b), Some(cap)) = (qd.mem.overlap_buffer[ch].as_deref_mut(), caps.ov[ch]) {
            fill(1, ch, b, cap, rng);
        }
        if let (Some(b), Some(cap)) = (qd.qmf_domain_in[ch].p_ana_qmf_states.as_deref_mut(), caps.ana[ch]) {
            fill(2, ch, b, cap, rng);
        }
        if let Some(b) = qd.qmf_domain_out[ch].p_syn_qmf_states.as_deref_mut() {
            // Moderate values: KEEP_STATES re-initializations scale them.
            let v: Vec<i32> = (0..576).map(|_| rng.i32() >> 4).collect();
            b.copy_from_slice(&v);
            unsafe { cref_qd_set_mem(c.0, 3, ch as i32, 576, v.as_ptr()) };
        }
    }
}

/// A random requested configuration in the style of one of the users of the QMF domain.
fn random_request(rng: &mut Rng, g: &mut [i32; 27]) {
    let pick = |rng: &mut Rng, v: &[i32]| v[(rng.next_u32() as usize) % v.len()];
    match rng.next_u32() % 4 {
        0 => {
            // SBR (sbrDecoder_InitElement): analysis bands of the header, 64 or 32 synthesis
            // bands, time slots, overlap, 64 processing bands, one processing channel.
            g[8] |= pick(rng, &[0, 1, 4, 64]); // flags_requested |= qmfFlags
            g[10] = pick(rng, &[16, 24, 32, 64]);
            g[12] = pick(rng, &[64, 32, 16]);
            g[14] = pick(rng, &[16, 32]);
            g[16] = pick(rng, &[3, 6]);
            g[18] = 64;
            g[20] = 1;
            g[2] = rng.range(1, 8);
            g[4] = g[4].max(g[2]);
        }
        1 => {
            // MPS: CLDFB or standard bands, more output channels, processing channels.
            g[8] = pick(rng, &[0, 4, 1, 16, 20]);
            g[10] = pick(rng, &[32, 64, 16]);
            g[12] = pick(rng, &[64, 32]);
            g[14] = pick(rng, &[16, 32, 64]);
            g[16] = pick(rng, &[0, 3, 6, 12]);
            g[18] = 64;
            g[20] = rng.range(1, 3);
            g[2] = rng.range(1, 3);
            g[4] = rng.range(1, 8);
        }
        2 => {
            // Explicit configuration (USAC, aacdecoder.cpp): also sets the configured
            // parkChannel.
            g[0] = 1;
            g[2] = rng.range(1, 2);
            g[4] = g[2];
            g[8] = 0;
            g[10] = pick(rng, &[16, 24, 32]);
            g[12] = 64;
            g[14] = pick(rng, &[16, 32]);
            g[16] = pick(rng, &[3, 6]);
            g[18] = 64;
            g[20] = 1;
            let park = rng.range(0, 1);
            g[5] = park;
            g[6] = park;
        }
        _ => {
            // Random values, including invalid ones.
            g[2] = rng.range(0, 10);
            g[4] = rng.range(0, 10);
            g[6] = rng.range(0, 1);
            g[8] = rng.next_u32() as i32 & 0x7f;
            g[10] = pick(rng, &[0, 8, 12, 16, 20, 24, 32, 40, 48, 64, 65]);
            g[12] = pick(rng, &[0, 8, 16, 24, 32, 40, 64, 128, 300]);
            g[14] = pick(rng, &[0, 1, 15, 16, 32, 48, 64]);
            g[16] = pick(rng, &[0, 1, 3, 6, 12]);
            g[18] = pick(rng, &[0, 32, 64, 64, 64, 96]);
            g[20] = rng.range(0, 4);
        }
    }
}

#[test]
fn qmf_domain_null_handle() {
    assert_eq!(fdk_qmf_domain_clear_persistent_memory(None), unsafe {
        cref_qd_clear_persistent_memory(core::ptr::null_mut())
    });
}

#[test]
fn qmf_domain_sequences() {
    let mut rng = Rng::new(0x9d0);
    let mut configured_ops = 0;
    let mut stats = std::collections::BTreeMap::<String, usize>::new();
    for run in 0..600 {
        let c = CQd(unsafe { cref_qd_new() });
        let mut qd = Box::<FdkQmfDomain>::default();
        let mut caps = Caps::default();
        let mut ok = false;
        compare(&qd, &c, &caps, "new");
        for step in 0..40 {
            let w = format!("run {run} step {step}");
            let g = c.gc();
            let n_in = (g[1] as usize).min(CH);
            let n_all = (g[13] + g[15]) as usize;
            let op = rng.next_u32() % 20;
            match op {
                0..=4 => {
                    // Request a configuration and configure.
                    let mut g2 = g;
                    if rng.next_u32() % 3 == 0 {
                        // ClearRequested first, as the decoder does for a new configuration.
                        unsafe { cref_qd_clear_requested(c.0) };
                        fdk_qmf_domain_clear_requested(&mut qd.global_conf);
                        g2 = c.gc();
                    }
                    random_request(&mut rng, &mut g2);
                    if !configure_safe(&g2, &caps) {
                        continue;
                    }
                    unsafe { cref_qd_set_gc(c.0, g2.as_ptr()) };
                    set_gc_fields(&mut qd.global_conf, &g2);
                    let ec = unsafe { cref_qd_configure(c.0) };
                    let er = fdk_qmf_domain_configure(&mut qd);
                    caps.update(&c);
                    assert_eq!(er, ec, "{w}: configure {g2:?}");
                    *stats.entry(format!("configure {er}")).or_default() += 1;
                    ok = er == 0;
                }
                5 => {
                    unsafe { cref_qd_clear_requested(c.0) };
                    fdk_qmf_domain_clear_requested(&mut qd.global_conf);
                }
                6 => {
                    unsafe { cref_qd_free_mem(c.0) };
                    fdk_qmf_domain_free_mem(&mut qd);
                    caps.update(&c);
                    ok = false;
                }
                7 => {
                    // Close, then FreeMem.
                    unsafe { cref_qd_close(c.0) };
                    fdk_qmf_domain_close(&mut qd);
                    caps.update(&c);
                    compare(&qd, &c, &caps, &format!("{w}: close"));
                    unsafe { cref_qd_free_mem(c.0) };
                    fdk_qmf_domain_free_mem(&mut qd);
                    ok = false;
                }
                8 if ok => {
                    let extra = [0, QMF_FLAG_KEEP_STATES, QMF_FLAG_MPSLDFB, QMF_FLAG_LP, QMF_FLAG_KEEP_STATES | QMF_FLAG_LP]
                        [(rng.next_u32() % 5) as usize];
                    fill_memory(&mut qd, &c, &caps, &mut rng);
                    let ec = unsafe { cref_qd_init_filter_bank(c.0, extra) };
                    let er = fdk_qmf_domain_init_filter_bank(&mut qd, extra);
                    assert_eq!(er, ec, "{w}: init filter bank {extra:#x}");
                }
                9 if ok => {
                    fill_memory(&mut qd, &c, &caps, &mut rng);
                    let ec = unsafe { cref_qd_clear_persistent_memory(c.0) };
                    let er = fdk_qmf_domain_clear_persistent_memory(Some(&mut qd));
                    assert_eq!(er, ec, "{w}: clear persistent memory");
                }
                10 => {
                    // Filter bank fields as SBR and MPS set them.
                    let out = rng.range(0, 1);
                    let ch = rng.range(0, CH as i32 - 1) as usize;
                    let lsb = rng.range(0, 64);
                    let usb = rng.range(lsb, 64);
                    let gm = if rng.next_u32() % 2 == 0 { i32::MIN } else { rng.i32() >> rng.range(0, 8) };
                    let ge = rng.range(-3, 3);
                    unsafe { cref_qd_set_fb(c.0, out, ch as i32, lsb, usb, gm, ge) };
                    let fb = if out != 0 { &mut qd.qmf_domain_out[ch].fb } else { &mut qd.qmf_domain_in[ch].fb };
                    fb.lsb = lsb;
                    fb.usb = usb;
                    fb.out_gain_m = gm;
                    fb.out_gain_e = ge;
                }
                11..=14 if ok && n_in > 0 && n_all > 0 => {
                    let ch = rng.range(0, n_in as i32 - 1) as usize;
                    let cs = c.in_state(ch);
                    if cs[26] != 1 || cs[27] == 0 {
                        continue;
                    }
                    fill_memory(&mut qd, &c, &caps, &mut rng);
                    let sc = [rng.range(-60, 60), rng.range(-60, 60), rng.range(-60, 60), rng.range(-60, 60)];
                    unsafe { cref_qd_set_scaling(c.0, ch as i32, sc.as_ptr()) };
                    let s = &mut qd.qmf_domain_in[ch].scaling;
                    s.lb_scale = sc[0];
                    s.ov_lb_scale = sc[1];
                    s.hb_scale = sc[2];
                    s.ov_hb_scale = sc[3];
                    if op == 11 {
                        let offset = rng.range(0, g[15]);
                        unsafe { cref_qd_save_overlap(c.0, ch as i32, offset) };
                        fdk_qmf_domain_save_overlap(&mut qd, ch, offset);
                    } else {
                        let pb = g[17].min(64);
                        for _ in 0..8 {
                            let ts = rng.range(0, n_all as i32 - 1);
                            let start = rng.range(0, pb);
                            let stop = rng.range(start, pb);
                            let exp_out = rng.range(-20, 20);
                            let with_imag = rng.next_u32() % 3 != 0;
                            let init: Vec<i32> = (0..64).map(|_| rng.i32()).collect();
                            let (mut cre, mut cim) = (init.clone(), init.clone());
                            let (mut rre, mut rim) = (init.clone(), init);
                            unsafe {
                                cref_qd_get_slot(
                                    c.0,
                                    ch as i32,
                                    ts,
                                    start,
                                    stop,
                                    cre.as_mut_ptr(),
                                    if with_imag { cim.as_mut_ptr() } else { core::ptr::null_mut() },
                                    exp_out,
                                )
                            };
                            fdk_qmf_domain_get_slot(
                                &qd,
                                ch,
                                ts,
                                start,
                                stop,
                                &mut rre,
                                if with_imag { Some(&mut rim[..]) } else { None },
                                exp_out,
                            );
                            assert_eq!(rre, cre, "{w}: get slot ch {ch} ts {ts} {start}..{stop} exp {exp_out} scaling {sc:?}");
                            assert_eq!(rim, cim, "{w}: get slot imag");
                        }
                    }
                    configured_ops += 1;
                }
                15..=18 if ok && n_in > 0 => {
                    let ch = rng.range(0, (n_in as i32).min(3) - 1) as usize;
                    let cs = c.in_state(ch);
                    let (wb_ts, wb_bands, off, sect) = (cs[19], cs[20], cs[21], cs[22]);
                    if cs[25] != 1 || sect == 0 || wb_bands == 0 {
                        continue;
                    }
                    let sections = (21..26).filter(|&k| g[k] != 0).count() as i32;
                    if op == 15 {
                        for _ in 0..4 {
                            let ts = rng.range(0, wb_ts.max(1) - 1);
                            let mut co = [0i32; 6];
                            unsafe { cref_qd_get_work_buffer(c.0, ch as i32, ts, co.as_mut_ptr()) };
                            let (re, im) = fdk_qmf_domain_get_work_buffer(&qd, ch, ts);
                            let mut ro = [0i32; 6];
                            ro[..3].copy_from_slice(&r_ptr(&qd, re));
                            ro[3..].copy_from_slice(&r_ptr(&qd, im));
                            assert_eq!(ro, co, "{w}: get work buffer ch {ch} ts {ts}");
                        }
                        continue;
                    }
                    // Only states for which WorkBuffer2ProcChannel and QmfData2HBE have a defined
                    // reference result.
                    let bands = if op == 16 { wb_bands } else { g[9].max(wb_bands) };
                    let fits = cs[26] == 1
                        && cs[27] != 0
                        && g[9] <= 64
                        && wb_bands <= 64
                        && wb_ts <= g[13]
                        && (g[15] + wb_ts) as usize <= caps.slots[ch].unwrap_or(0)
                        && off + 2 * wb_ts * wb_bands <= sections * 2048
                        && (0..2 * wb_ts).all(|k| (off + k * wb_bands) % 2048 + bands <= 2048);
                    if !fits {
                        continue;
                    }
                    fill_memory(&mut qd, &c, &caps, &mut rng);
                    let proc = fdk_get_work_buffer(off as u16, sect as u16, wb_bands as u16)
                        == qd.qmf_domain_in[ch].h_qmf_slots_real.as_ref().unwrap()[g[15] as usize];
                    *stats.entry(format!("op {op} processing channel {proc}")).or_default() += 1;
                    if op == 16 {
                        unsafe { cref_qd_work_buffer2proc_channel(c.0, ch as i32) };
                        fdk_qmf_domain_work_buffer2_proc_channel(&mut qd, ch);
                    } else {
                        let hbe_re: Vec<i32> = (0..64 * 64).map(|_| rng.i32()).collect();
                        let hbe_im: Vec<i32> = (0..64 * 64).map(|_| rng.i32()).collect();
                        let (mut cre, mut cim) = (hbe_re.clone(), hbe_im.clone());
                        let (mut rre, mut rim) = (hbe_re, hbe_im);
                        unsafe { cref_qd_qmf_data2hbe(c.0, ch as i32, cre.as_mut_ptr(), cim.as_mut_ptr()) };
                        let mut re_slots: Vec<&mut [i32]> = rre.chunks_mut(64).collect();
                        let mut im_slots: Vec<&mut [i32]> = rim.chunks_mut(64).collect();
                        fdk_qmf_domain_qmf_data2_hbe(&mut qd, ch, &mut re_slots, &mut im_slots);
                        assert_eq!(rre, cre, "{w}: HBE real");
                        assert_eq!(rim, cim, "{w}: HBE imag");
                    }
                    configured_ops += 1;
                }
                19 if ok && n_in > 0 && g[13] > 0 => {
                    // Analysis into the slots of input channel ch and synthesis from them, as
                    // the SBR decoder uses the domain (through QmfDomainMem::slots_mut).
                    let ch = rng.range(0, n_in as i32 - 1) as usize;
                    let cs = c.in_state(ch);
                    let n_out = (g[3] as usize).min(CH);
                    if cs[26] != 1 || cs[27] == 0 || cs[23] == 0 || cs[3] == 0 || ch >= n_out {
                        continue;
                    }
                    let fb = qd.qmf_domain_in[ch].fb;
                    let sfb = qd.qmf_domain_out[ch].fb;
                    let lp = fb.flags & QMF_FLAG_LP != 0;
                    // Configurations whose tables the reference has (see qmf.rs).
                    let ana_ok = matches!(fb.no_channels, 16 | 24 | 32 | 64) && (lp || fb.no_channels == 64 || !fb.t_cos.is_empty() || fb.flags & QMF_FLAG_MPSLDFB != 0);
                    let syn_ok = !sfb.p_filter.is_empty() && matches!(sfb.no_channels, 16 | 32 | 64) && (sfb.flags & QMF_FLAG_CLDFB == 0 || !sfb.t_cos.is_empty()) && sfb.flags & QMF_FLAG_LP == fb.flags & QMF_FLAG_LP;
                    // Only prototype filters that match QMF_FLAG_NONSYMMETRIC.
                    let own_filter = |f: &QmfFilterBank| {
                        f.flags & QMF_FLAG_NONSYMMETRIC == 0 || f.flags & (QMF_FLAG_CLDFB | QMF_FLAG_MPSLDFB) != 0
                    };
                    if !ana_ok || !syn_ok || !own_filter(&fb) || !own_filter(&sfb) || fb.no_col != g[13] {
                        continue;
                    }
                    fill_memory(&mut qd, &c, &caps, &mut rng);
                    let n = (fb.no_col * fb.no_channels) as usize;
                    let kind = rng.next_u32() % 3;
                    let time_in: Vec<i32> = (0..n).map(|k| match kind {
                        0 => rng.i32() >> 4,
                        1 => 0,
                        _ => ((k as f64 * 0.1).sin() * 1e8) as i32,
                    }).collect();
                    let e = rng.range(-2, 2);
                    unsafe { cref_qd_analysis(c.0, ch as i32, time_in.as_ptr(), e) };
                    {
                        let FdkQmfDomain { mem, qmf_domain_in, .. } = &mut *qd;
                        let q = &mut qmf_domain_in[ch];
                        let ov = g[15] as usize;
                        let re = q.h_qmf_slots_real.as_ref().unwrap()[ov..].to_vec();
                        let im = q.h_qmf_slots_imag.as_ref().unwrap()[ov..].to_vec();
                        let mut ptrs = re.clone();
                        ptrs.extend(im.iter().copied());
                        let mut slots = mem.slots_mut(&ptrs, g[17] as usize);
                        let (mut re_s, mut im_s) = {
                            let im_s = slots.split_off(re.len());
                            (slots, im_s)
                        };
                        let mut work = [0i32; 128];
                        let states = q.p_ana_qmf_states.as_deref_mut().unwrap();
                        qmf_analysis_filtering(&q.fb, states, &mut re_s, &mut im_s, &mut q.scaling, &time_in, e, 1, &mut work);
                    }
                    compare(&qd, &c, &caps, &format!("{w}: analysis"));
                    let n_syn = (sfb.no_col * sfb.no_channels) as usize;
                    let mut cout = vec![0i32; n_syn];
                    let mut rout = vec![0i32; n_syn];
                    unsafe { cref_qd_synthesis(c.0, ch as i32, cout.as_mut_ptr()) };
                    {
                        let FdkQmfDomain { mem, qmf_domain_in, qmf_domain_out, .. } = &mut *qd;
                        let q = &qmf_domain_in[ch];
                        let re: Vec<&[i32]> = q.h_qmf_slots_real.as_ref().unwrap().iter().map(|&p| mem.slot(p)).collect();
                        let im: Vec<&[i32]> = q.h_qmf_slots_imag.as_ref().unwrap().iter().map(|&p| mem.slot(p)).collect();
                        let o = &mut qmf_domain_out[ch];
                        let mut work = [0i32; 128];
                        qmf_synthesis_filtering(&o.fb, o.p_syn_qmf_states.as_deref_mut().unwrap(), &re, &im, &q.scaling, g[15], &mut rout, 1, &mut work);
                    }
                    assert_eq!(rout, cout, "{w}: synthesis");
                    *stats.entry("analysis/synthesis".into()).or_default() += 1;
                }
                _ => continue,
            }
            *stats.entry(format!("op {op}")).or_default() += 1;
            compare(&qd, &c, &caps, &format!("{w} op {op}"));
        }
    }
    eprintln!("{stats:?}");
    assert!(configured_ops > 2000, "only {configured_ops} data operations");
}

/// `QmfDomainMem::slots_mut` (a helper of the port without C counterpart): the slices start at
/// the pointers, have the requested length (less at the end of a buffer), are empty for `NULL`,
/// unallocated buffers and a slot that overlaps an earlier one, and alias nothing.
#[test]
fn qmf_domain_slots_mut() {
    let mut mem = QmfDomainMem::default();
    mem.work_buffer[0] = Some((0..2048).collect::<Vec<i32>>().into());
    mem.overlap_buffer[2] = Some((0..1536).map(|x| -x).collect::<Vec<i32>>().into());
    let ptrs = [
        QmfSlotPtr::Work { sect: 0, offset: 128 },
        QmfSlotPtr::Overlap { ch: 2, offset: 64 },
        QmfSlotPtr::Null,
        QmfSlotPtr::Work { sect: 0, offset: 0 },
        QmfSlotPtr::Work { sect: 1, offset: 0 },
        QmfSlotPtr::Work { sect: 0, offset: 2000 },
        QmfSlotPtr::Work { sect: 0, offset: 160 },
    ];
    let slots = mem.slots_mut(&ptrs, 64);
    assert_eq!(slots[0], (128..192).collect::<Vec<i32>>());
    assert_eq!(slots[1], (64..128).map(|x| -x).collect::<Vec<i32>>());
    assert!(slots[2].is_empty());
    assert_eq!(slots[3], (0..64).collect::<Vec<i32>>());
    assert!(slots[4].is_empty());
    assert_eq!(slots[5], (2000..2048).collect::<Vec<i32>>());
    assert!(slots[6].is_empty(), "overlaps slot 0");
    for s in slots {
        s.fill(7);
    }
    assert_eq!(mem.slot(QmfSlotPtr::Work { sect: 0, offset: 128 })[..64], [7; 64]);
    assert_eq!(mem.slot(QmfSlotPtr::Work { sect: 0, offset: 192 })[0], 192);
}
