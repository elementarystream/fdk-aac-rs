// Differential tests of fdk_aac_rs_sbrdec::env_extr against the C++ reference.
//
// initHeaderData for all sample rates, frame sizes and flags; sbrGetHeaderData and
// sbrGetChannelElement on random bitstreams (partly with a valid header prefix) for the syntaxes
// of AAC, ELD, USAC (independent or not, PVC, inter-TES), scalable/DRM and the overlaps of the
// configurations, mono and stereo (all coupling modes), with random previous frame data; the
// return value, the position in the bitstream and the complete header and frame data are
// compared after every call. Real data is covered by tests/sbrdec/e2e.rs.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use core::ffi::c_void;
use fdk_aac_rs_cref::sbrdec_env_extr::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_sbrdec::env_extr::*;
use fdk_aac_rs_sbrdec::sbr_ram::{pod_bytes, pod_bytes_mut, Pod};

pub(crate) fn ptr<T: Pod>(v: &mut T) -> *mut c_void {
    pod_bytes_mut(v).as_mut_ptr() as *mut c_void
}

/// A bitstream buffer of 1024 bytes: random bits, optionally starting with `prefix`
/// (`(value, bits)` pairs).
pub(crate) fn random_bits(rng: &mut Rng, prefix: &[(u32, u32)]) -> (Vec<u8>, i32) {
    let mut buf = vec![0u8; 1024];
    for b in buf.iter_mut() {
        *b = rng.next_u32() as u8;
    }
    let mut n = 0usize;
    for &(v, bits) in prefix {
        for k in (0..bits).rev() {
            let bit = (v >> k) & 1;
            let byte = &mut buf[n / 8];
            let m = 0x80 >> (n % 8);
            *byte = if bit != 0 { *byte | m } else { *byte & !m };
            n += 1;
        }
    }
    let valid = match rng.next_u32() % 4 {
        0 => rng.range(0, 64),
        1 => rng.range(0, 1024),
        _ => 8 * 1024,
    };
    (buf, valid)
}

/// The port's bitstream on a copy of `buf`.
pub(crate) fn port_bs(buf: &mut [u8], valid: i32) -> FdkBitStream {
    let mut bs = FdkBitStream::default();
    fdk_init_bit_stream(&mut bs, &BitMem::from_slice(buf), buf.len() as u32, valid as u32, BS_READER);
    bs
}

/// Flags of a random SBR syntax and an overlap that fits it.
pub(crate) fn random_flags(rng: &mut Rng) -> (u32, i32) {
    match rng.next_u32() % 7 {
        0 | 1 => (0, 6),
        2 => (SBRDEC_ELD_GRID, 0),
        3 => (SBRDEC_SYNTAX_USAC | if rng.next_u32() % 2 == 0 { SBRDEC_USAC_INDEP } else { 0 }, 6),
        4 => {
            let mut f = SBRDEC_SYNTAX_USAC | SBRDEC_QUAD_RATE;
            if rng.next_u32() % 2 == 0 {
                f |= SBRDEC_USAC_INDEP;
            }
            if rng.next_u32() % 2 == 0 {
                f |= SBRDEC_USAC_ITES;
            }
            if rng.next_u32() % 2 == 0 {
                f |= SBRDEC_USAC_HARMONICSBR;
            }
            (f, 12)
        }
        5 => (SBRDEC_SYNTAX_SCAL | if rng.next_u32() % 2 == 0 { SBRDEC_SYNTAX_DRM } else { 0 }, 6),
        _ => (SBRDEC_SYNTAX_RSVD50 | SBRDEC_USAC_ITES, 6),
    }
}

/// A header initialized for a random configuration (both sides), with random header bitstream
/// data applied; None if the configuration is rejected.
pub(crate) fn random_header(rng: &mut Rng, flags: u32) -> Option<SbrHeaderData> {
    let usac = flags & SBRDEC_SYNTAX_USAC != 0;
    let rates = [8000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 64000];
    let sr_in = rates[rng.range(0, rates.len() as i32 - 1) as usize];
    // (the overlap of random_flags() matches: 4:1 with SBRDEC_QUAD_RATE only)
    let sr_out = if flags & SBRDEC_QUAD_RATE != 0 {
        4 * sr_in
    } else if rng.next_u32() % 4 == 0 {
        sr_in
    } else {
        2 * sr_in
    };
    let frame = if flags & SBRDEC_ELD_GRID != 0 {
        [512, 480, 256, 240][rng.range(0, 3) as usize]
    } else if usac {
        [1024, 768][rng.range(0, 1) as usize]
    } else {
        [1024, 960][rng.range(0, 1) as usize]
    };
    let downscale = if flags & SBRDEC_ELD_GRID != 0 { [1, 2, 4][rng.range(0, 2) as usize] } else { 1 };
    let mut c = SbrHeaderData::ZERO;
    let mut p = SbrHeaderData::ZERO;
    let rc = unsafe { cref_sbrdec_env_extr_init_header(ptr(&mut c), sr_in, sr_out, downscale, frame, flags, 1) };
    let rp = init_header_data(&mut p, sr_in, sr_out, downscale, frame, flags, 1);
    assert_eq!((rp, pod_bytes(&p)), (rc, pod_bytes(&c)), "initHeaderData {sr_in} {sr_out} {frame} {flags:#x}");
    if rc != 0 {
        return None;
    }
    p.bs_data.start_freq = rng.range(0, 15) as u8;
    p.bs_data.stop_freq = rng.range(0, 15) as u8;
    p.bs_data.freq_scale = rng.range(0, 3) as u8;
    p.bs_data.alter_scale = rng.range(0, 1) as u8;
    p.bs_data.noise_bands = rng.range(0, 3) as u8;
    p.bs_data.limiter_bands = rng.range(0, 3) as u8;
    p.bs_data.limiter_gains = rng.range(0, 3) as u8;
    p.bs_data.interpol_freq = rng.range(0, 1) as u8;
    p.bs_data.smoothing_length = rng.range(0, 1) as u8;
    p.bs_info.xover_band = rng.range(0, 3) as u8;
    p.bs_info.amp_resolution = rng.range(0, 1) as u8;
    let mut c = p;
    let rc = unsafe { cref_sbrdec_env_extr_reset_tables(ptr(&mut c), flags) };
    let rp = fdk_aac_rs_sbrdec::sbrdec_freq_sca::reset_freq_band_tables(&mut p, flags);
    assert_eq!((rp, pod_bytes(&p)), (rc, pod_bytes(&c)), "resetFreqBandTables {flags:#x}");
    if rc != 0 {
        return None;
    }
    Some(p)
}

#[test]
fn init_header_and_map_rate() {
    let mut rng = Rng::new(21);
    for _ in 0..3000 {
        let (flags, _) = random_flags(&mut rng);
        let sr_in = rng.range(0, 100000);
        let sr_out = [0, sr_in, 2 * sr_in, 4 * sr_in, rng.range(0, 200000)][rng.range(0, 4) as usize];
        let frame = [1024, 960, 768, 512, 480, 256, 240, 120, 0][rng.range(0, 8) as usize];
        // (downscale factors 1 to 4; the decoder passes 1, 2 or 4)
        let ds = rng.range(1, 4);
        let dflt = rng.range(0, 1);
        let mut c = SbrHeaderData::ZERO;
        let mut p = c;
        let rc = unsafe { cref_sbrdec_env_extr_init_header(ptr(&mut c), sr_in, sr_out, ds, frame, flags, dflt) };
        let rp = init_header_data(&mut p, sr_in, sr_out, ds, frame, flags, dflt);
        assert_eq!((rp, pod_bytes(&p)), (rc, pod_bytes(&c)), "initHeaderData {sr_in} {sr_out} {frame} {ds} {flags:#x}");
        let fs = rng.range(0, 200000) as u32;
        let usac = rng.range(0, 1) as u32;
        assert_eq!(sbrdec_map_to_std_sample_rate(fs, usac), unsafe { cref_sbrdec_env_extr_map_rate(fs, usac) });
        let ts = rng.range(0, 20);
        let mut pc = SbrPrevFrameData::ZERO;
        for b in pod_bytes_mut(&mut pc).iter_mut() {
            *b = rng.next_u32() as u8;
        }
        let mut pp = pc;
        unsafe { cref_sbrdec_env_extr_init_prev(ptr(&mut pc), ts) };
        init_sbr_prev_frame_data(&mut pp, ts);
        assert_eq!(pod_bytes(&pp), pod_bytes(&pc), "initSbrPrevFrameData");
    }
}

#[test]
fn header_data_random_bits() {
    let mut rng = Rng::new(22);
    for it in 0..20000 {
        let (flags, _) = random_flags(&mut rng);
        let Some(mut h) = random_header(&mut rng, flags) else {
            continue;
        };
        let null = rng.next_u32() % 8 == 0;
        let cfg = if null { 1 } else { [0u8, 0, 2][rng.range(0, 2) as usize] };
        let is_data = rng.range(0, 1);
        let (mut buf, valid) = random_bits(&mut rng, &[]);
        let mut buf2 = buf.clone();
        let mut hc = h;
        let mut left = 0;
        let rc = unsafe {
            cref_sbrdec_env_extr_get_header(
                if null { core::ptr::null_mut() } else { ptr(&mut hc) },
                buf.as_mut_ptr(),
                buf.len() as i32,
                valid,
                flags,
                is_data,
                cfg as i32,
                &mut left,
            )
        };
        let mut bs = port_bs(&mut buf2, valid);
        let rp = sbr_get_header_data(if null { None } else { Some(&mut h) }, &mut bs, flags, is_data, cfg);
        assert_eq!(rp, rc, "sbrGetHeaderData {it}");
        assert_eq!(fdk_get_valid_bits(&mut bs) as i32, left, "sbrGetHeaderData bits {it}");
        assert_eq!(pod_bytes(&h), pod_bytes(&hc), "sbrGetHeaderData header {it}");
    }
}

/// Random previous frame data (as left behind by earlier frames).
pub(crate) fn random_prev(rng: &mut Rng) -> SbrPrevFrameData {
    let mut p = SbrPrevFrameData::ZERO;
    init_sbr_prev_frame_data(&mut p, 16);
    if rng.next_u32() % 2 == 0 {
        for v in p.sfb_nrg_prev.iter_mut() {
            *v = (rng.next_u32() as i16) & !0x3f | rng.range(0, 63) as i16;
        }
        for v in p.prev_noise_level.iter_mut() {
            *v = (rng.next_u32() as i16) & !0x3f | rng.range(0, 63) as i16;
        }
        p.coupling = rng.range(0, 2);
        p.amp_res = rng.range(0, 1) as u8;
        p.stop_pos = rng.range(0, 20) as u8;
        p.frame_error_flag = rng.range(0, 1) as u8;
        p.prev_frame_info.n_envelopes = rng.range(1, 5) as u8;
        p.prev_frame_info.borders[p.prev_frame_info.n_envelopes as usize] = rng.range(8, 18) as u8;
        p.prev_frame_info.n_noise_envelopes = rng.range(1, 2) as u8;
    }
    p
}

/// A valid-looking start of sbr_channel_pair_element / sbr_single_channel_element: data extra
/// bits and coupling, then random grid bits.
fn element_prefix(rng: &mut Rng, stereo: bool, flags: u32) -> Vec<(u32, u32)> {
    let mut v = Vec::new();
    if flags & SBRDEC_SYNTAX_USAC == 0 {
        v.push((0, 1)); // bs_data_extra
        if flags & SBRDEC_SYNTAX_SCAL != 0 {
            v.push((0, 4));
        }
    }
    if stereo {
        v.push((rng.next_u32() & 1, 1)); // bs_coupling
    }
    // FIXFIX grid with a random number of envelopes
    if rng.next_u32() % 2 == 0 && flags & SBRDEC_ELD_GRID == 0 {
        v.push((0, 2));
        v.push((rng.next_u32() & 3, 2));
        v.push((rng.next_u32() & 1, 1));
    }
    v
}

/// Parses one random channel element with both implementations (compared) and returns the
/// result, the parsed frames (left, right) and the previous frame data used.
pub(crate) fn parse_element(
    rng: &mut Rng,
    h: &mut SbrHeaderData,
    flags: u32,
    overlap: i32,
    stereo: bool,
    what: &str,
) -> (i32, SbrFrameData, SbrFrameData, SbrPrevFrameData) {
    // initial frame data: zero or partly random (left over from earlier frames)
    let mut l0 = SbrFrameData::ZERO;
    let mut r0 = SbrFrameData::ZERO;
    for f in [&mut l0, &mut r0] {
        if rng.next_u32() % 2 == 0 {
            for b in pod_bytes_mut(f).iter_mut() {
                *b = if rng.next_u32() % 8 == 0 { rng.next_u32() as u8 } else { 0 };
            }
            f.frame_info.n_envelopes %= 9;
            f.frame_info.n_noise_envelopes %= 3;
        }
    }
    let prev = random_prev(rng);
    let pvc_last = rng.range(0, 2);
    let prefix = element_prefix(rng, stereo, flags);
    let (mut buf, valid) = random_bits(rng, &prefix);
    let mut buf2 = buf.clone();

    let (mut hc, mut lc, mut rc_, mut prev_c) = (*h, l0, r0, prev);
    let mut left = 0;
    let ret_c = unsafe {
        cref_sbrdec_env_extr_get_channel_element(
            ptr(&mut hc),
            ptr(&mut lc),
            if stereo { ptr(&mut rc_) } else { core::ptr::null_mut() },
            ptr(&mut prev_c),
            pvc_last,
            buf.as_mut_ptr(),
            buf.len() as i32,
            valid,
            flags,
            overlap,
            &mut left,
        )
    };
    let (mut lp, mut rp_) = (l0, r0);
    let mut bs = port_bs(&mut buf2, valid);
    let ret_p = sbr_get_channel_element(
        h,
        &mut lp,
        if stereo { Some(&mut rp_) } else { None },
        &prev,
        pvc_last as u8,
        &mut bs,
        None,
        flags,
        overlap,
    );
    assert_eq!(ret_p, ret_c, "{what}: sbrGetChannelElement");
    assert_eq!(fdk_get_valid_bits(&mut bs) as i32, left, "{what}: bits");
    assert_eq!(pod_bytes(h), pod_bytes(&hc), "{what}: header");
    assert_eq!(pod_bytes(&lp), pod_bytes(&lc), "{what}: left frame");
    if stereo {
        assert_eq!(pod_bytes(&rp_), pod_bytes(&rc_), "{what}: right frame");
    }
    (ret_c, lp, rp_, prev)
}

#[test]
fn channel_element_random_bits() {
    let mut rng = Rng::new(23);
    let mut ok = 0;
    for it in 0..30000 {
        let (flags, overlap) = random_flags(&mut rng);
        let Some(mut h) = random_header(&mut rng, flags) else {
            continue;
        };
        if flags & SBRDEC_SYNTAX_USAC != 0 && rng.next_u32() % 2 == 0 {
            h.bs_info.pvc_mode = rng.range(1, 2) as u8;
        }
        let stereo = rng.next_u32() % 2 == 0 && h.bs_info.pvc_mode == 0;
        let (r, ..) = parse_element(&mut rng, &mut h, flags, overlap, stereo, &format!("element {it}"));
        ok += (r != 0) as i32;
    }
    assert!(ok > 1000, "too few successfully parsed elements: {ok}");
}
