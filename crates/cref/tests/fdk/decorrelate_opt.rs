// Tests of the optimized decorrelator kernels of fdk_aac_rs_fdk::decorrelate against the
// straight port kept next to them (`*_straight`), on random and extreme inputs: valid layouts of
// the library (which take the fast paths, on aarch64 the NEON kernel), shifted and overlapping
// layouts, and offsets outside the buffer (which fall back to the straight port). The complete
// decorrelator is compared with C++ in `decorrelate.rs`.

use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::archdef::FixpStp;
use fdk_aac_rs_fdk::common_fix::FixpSpk;
use fdk_aac_rs_fdk::decorrelate::*;

/// Length of the data arrays (71 hybrid bands plus one).
const N: usize = 72;

fn data(rng: &mut Rng) -> Vec<i32> {
    let shift = rng.next_u32() % 8;
    (0..N).map(|_| if shift == 0 { rng.i32() } else { rng.i32() >> shift }).collect()
}

fn buffer(rng: &mut Rng, len: usize) -> Vec<i32> {
    (0..len).map(|_| rng.i32()).collect()
}

/// A buffer offset: mostly inside, sometimes `None` (`NULL`) or past the end.
fn offset(rng: &mut Rng, len: usize, valid: usize) -> Option<usize> {
    match rng.next_u32() % 16 {
        0 => None,
        1 => Some(len + rng.range(0, 8) as usize),
        2 => Some(rng.range(0, len as i32) as usize),
        _ => Some(valid),
    }
}

fn random_coeffs(rng: &mut Rng) -> &'static [FixpStp; 4] {
    Box::leak(Box::new([(); 4].map(|_| FixpSpk { re: rng.i16(), im: rng.i16() })))
}

#[test]
fn cplx_ps_fast_matches_straight() {
    let mut rng = Rng::new(0x0301);
    for iter in 0..20000 {
        // the PS layout of the library: 30 bands of order 12, delay 2, states first
        let valid_layout = iter % 4 != 3;
        let order = if valid_layout { 12 } else { rng.range(-2, 14) };
        let delay = if valid_layout { 2 } else { rng.range(0, 16) };
        let (start, stop) = match rng.next_u32() % 4 {
            0 => (rng.range(0, 30), rng.range(0, 31)),
            1 => (rng.range(0, 70), rng.range(0, 71)),
            _ => (0, 30),
        };
        let n_bands = (stop - start).max(0) as usize;
        let l_state = 2 * 12 * 30usize;
        let len = l_state + 2 * 2 * 30 + rng.range(0, 64) as usize;
        let mut filter = [DecorrFilterInstance::default(); 71];
        let s_valid = if valid_layout { 0 } else { rng.range(0, 64) as usize };
        let d_valid =
            if rng.next_u32() % 8 == 0 { rng.range(0, len as i32) as usize } else { l_state };
        filter[start.clamp(0, 70) as usize].state_cplx =
            offset(&mut rng, len, s_valid + 24 * start.max(0) as usize);
        filter[start.clamp(0, 70) as usize].delay_buffer_cplx =
            offset(&mut rng, len, d_valid + 4 * start.max(0) as usize);
        for (i, f) in filter.iter_mut().enumerate() {
            f.coeffs_packed = match rng.next_u32() % 32 {
                0 => None,
                1 => Some(random_coeffs(&mut rng)),
                _ => Some(&DECORR_PS_COEFFS_CPLX[i % 30]),
            };
        }
        let index = if rng.next_u32() % 8 == 0 {
            rng.range(0, 40)
        } else {
            2 * rng.range(0, delay.max(1) - 1)
        };
        let mut sbo = if rng.next_u32() % 8 == 0 {
            [rng.range(0, 30) as u8, rng.range(0, 30) as u8, rng.range(0, 30) as u8]
        } else {
            [
                2 * rng.range(0, 2) as u8,
                6 + 2 * rng.range(0, 3) as u8,
                14 + 2 * rng.range(0, 4) as u8,
            ]
        };
        let (re, im) = (data(&mut rng), data(&mut rng));
        let (mut buf_a, mut out_re_a, mut out_im_a) =
            (buffer(&mut rng, len), data(&mut rng), data(&mut rng));
        let (mut buf_b, mut out_re_b, mut out_im_b) =
            (buf_a.clone(), out_re_a.clone(), out_im_a.clone());
        let mut sbo_b = sbo;
        let ra = decorr_filter_apply_cplx_ps(
            &filter,
            &mut buf_a,
            &re,
            &im,
            &mut out_re_a,
            &mut out_im_a,
            start,
            stop,
            order,
            delay,
            index,
            &mut sbo,
        );
        let rb = decorr_filter_apply_cplx_ps_straight(
            &filter,
            &mut buf_b,
            &re,
            &im,
            &mut out_re_b,
            &mut out_im_b,
            start,
            stop,
            order,
            delay,
            index,
            &mut sbo_b,
        );
        let what = format!(
            "iter {iter} start {start} stop {stop} bands {n_bands} order {order} delay {delay}"
        );
        assert_eq!(ra, rb, "{what}");
        assert_eq!(out_re_a, out_re_b, "{what}");
        assert_eq!(out_im_a, out_im_b, "{what}");
        assert_eq!(buf_a, buf_b, "{what}");
        assert_eq!(sbo, sbo_b, "{what}");
    }
}

#[test]
fn pass_fast_matches_straight() {
    let mut rng = Rng::new(0x0302);
    for iter in 0..20000 {
        let delay = rng.range(0, 16);
        let start = rng.range(0, 70);
        // (stop < start: the do-while loop of the straight port runs until it panics)
        let stop = rng.range(start, 71);
        let len = (2 * delay.max(1) * 72 + rng.range(0, 32)) as usize;
        let mut filter = [DecorrFilterInstance::default(); 71];
        let valid = rng.range(0, 8) as usize;
        filter[start as usize].delay_buffer_cplx = offset(&mut rng, len, valid);
        let index = if rng.next_u32() % 8 == 0 {
            rng.range(-4, 40)
        } else {
            2 * rng.range(0, delay.max(1) - 1)
        };
        let (re, im) = (data(&mut rng), data(&mut rng));
        let (mut buf_a, mut out_re_a, mut out_im_a) =
            (buffer(&mut rng, len), data(&mut rng), data(&mut rng));
        let (mut buf_b, mut out_re_b, mut out_im_b) =
            (buf_a.clone(), out_re_a.clone(), out_im_a.clone());
        let ra = decorr_filter_apply_pass(
            &filter,
            &mut buf_a,
            &re,
            Some(&im),
            &mut out_re_a,
            &mut out_im_a,
            start,
            stop,
            delay,
            index,
        );
        let rb = decorr_filter_apply_pass_straight(
            &filter,
            &mut buf_b,
            &re,
            Some(&im),
            &mut out_re_b,
            &mut out_im_b,
            start,
            stop,
            delay,
            index,
        );
        let what = format!("iter {iter} start {start} stop {stop} delay {delay} index {index}");
        assert_eq!(ra, rb, "{what}");
        assert_eq!(out_re_a, out_re_b, "{what}");
        assert_eq!(out_im_a, out_im_b, "{what}");
        assert_eq!(buf_a, buf_b, "{what}");
    }
}

#[test]
fn real_fast_matches_straight() {
    let mut rng = Rng::new(0x0303);
    let numerators: [&'static [FixpDecorr]; 7] = [
        &DECORR_NUMERATOR_REAL0_USAC[0],
        &DECORR_NUMERATOR_REAL1_USAC[0],
        &DECORR_NUMERATOR_REAL2_USAC[0],
        &DECORR_NUMERATOR_REAL3_USAC[0],
        &DECORR_NUMERATOR_REAL1_LD[1],
        &DECORR_NUMERATOR_REAL2_LD[2],
        &DECORR_NUMERATOR_REAL3_LD[3],
    ];
    for iter in 0..20000 {
        // orders of the library: 20, 15, 6, 3 (MPS), 10, 8, 3, 2 (USAC), 15, 6, 3 (LD)
        let order = match rng.next_u32() % 4 {
            0 => rng.range(-1, 21),
            _ => [2, 3, 6, 8, 10, 15, 20][(rng.next_u32() % 7) as usize],
        };
        let delay = rng.range(0, 14);
        let start = rng.range(0, 70);
        let stop = if rng.next_u32() % 8 == 0 { rng.range(0, 71) } else { rng.range(start, 71) };
        let o = order.max(1) as usize;
        let len = 2 * o * 72 + 2 * delay.max(1) as usize * 72 + rng.range(0, 32) as usize;
        let mut filter = [DecorrFilterInstance::default(); 71];
        let s_valid = rng.range(0, 8) as usize;
        filter[start as usize].state_cplx = offset(&mut rng, len, s_valid);
        filter[start as usize].delay_buffer_cplx = offset(&mut rng, len, s_valid + 2 * o * 71);
        filter[start as usize].numerator_real = match rng.next_u32() % 8 {
            0 => None,
            1 => Some(Box::leak(
                (0..rng.range(0, 24)).map(|_| rng.i16()).collect::<Vec<_>>().into_boxed_slice(),
            )),
            _ => Some(numerators[(rng.next_u32() % 7) as usize]),
        };
        let index = if rng.next_u32() % 8 == 0 {
            rng.range(-4, 40)
        } else {
            2 * rng.range(0, delay.max(1) - 1)
        };
        let (re, im) = (data(&mut rng), data(&mut rng));
        let (mut buf_a, mut out_re_a, mut out_im_a) =
            (buffer(&mut rng, len), data(&mut rng), data(&mut rng));
        let (mut buf_b, mut out_re_b, mut out_im_b) =
            (buf_a.clone(), out_re_a.clone(), out_im_a.clone());
        let ra = decorr_filter_apply_real(
            &filter,
            &mut buf_a,
            &re,
            &im,
            &mut out_re_a,
            &mut out_im_a,
            start,
            stop,
            order,
            delay,
            index,
        );
        let rb = decorr_filter_apply_real_straight(
            &filter,
            &mut buf_b,
            &re,
            &im,
            &mut out_re_b,
            &mut out_im_b,
            start,
            stop,
            order,
            delay,
            index,
        );
        let what = format!(
            "iter {iter} start {start} stop {stop} order {order} delay {delay} index {index}"
        );
        assert_eq!(ra, rb, "{what}");
        assert_eq!(out_re_a, out_re_b, "{what}");
        assert_eq!(out_im_a, out_im_b, "{what}");
        assert_eq!(buf_a, buf_b, "{what}");
    }
}

/// A ducker of one of the library's band maps with random state.
fn ducker(rng: &mut Rng, ps: bool) -> DuckerInstance {
    let mut d = DuckerInstance::default();
    let (map, offs, pb, hb): (&'static [u8], &'static [u8], i32, i32) = match rng.next_u32() % 3 {
        0 => (&KERNELS_20_TO_71_PS, &KERNELS_20_TO_71_OFFSET_PS, 20, 71),
        1 => (&KERNELS_28_TO_71, &KERNELS_28_TO_71_OFFSET, 28, 71),
        _ => (
            &KERNELS_23_TO_64,
            &KERNELS_23_TO_64_OFFSET,
            23,
            [64, 32][(rng.next_u32() % 2) as usize],
        ),
    };
    let (pb, map, offs, hb) = if ps && pb == 23 {
        (20, &KERNELS_20_TO_71_PS[..], &KERNELS_20_TO_71_OFFSET_PS[..], 71)
    } else {
        (pb, map, offs, hb)
    };
    d.map_hyb_bands2_proc_bands = Some(map);
    d.map_proc_bands2_hyb_bands = Some(offs);
    d.qs_next = Some(&offs[1..]);
    d.parameter_bands = pb;
    d.hybrid_bands = if rng.next_u32() % 8 == 0 { rng.range(1, hb) } else { hb };
    d.ducker_type = if ps { DUCKER_PS } else { DUCKER_MPS };
    // energies are not negative in the library (negative values make sqrtFixp()/invSqrtNorm2()
    // fail their assertions, which the checked profile turns into panics)
    let e = |rng: &mut Rng| ((rng.next_u32() >> 1) as i32) >> (rng.next_u32() % 32);
    for i in 0..2 * MAX_PARAMETER_BANDS {
        d.smooth_dir_rev_nrg[i] = e(rng);
    }
    for i in 0..MAX_PARAMETER_BANDS {
        d.peak_decay[i] = e(rng);
        d.peak_diff[i] = if rng.next_u32() % 4 == 0 { 0 } else { e(rng) };
        if rng.next_u32() % 4 == 0 {
            d.smooth_dir_rev_nrg[i] = 0;
        }
    }
    d.scale_direct_nrg = rng.range(0, 48) as i8;
    d.scale_reverb_nrg = rng.range(0, 48) as i8;
    d.scale_smooth_dir_rev_nrg = rng.range(0, 48) as i8;
    d.headroom_smooth_dir_rev_nrg = rng.range(0, 48) as i8;
    d.max_val_direct_data = if rng.next_u32() % 2 == 0 { -1 << 31 } else { rng.i32() };
    d
}

#[test]
fn ducker_calc_energy_fast_matches_straight() {
    let mut rng = Rng::new(0x0304);
    for iter in 0..40000 {
        let ps = rng.next_u32() % 2 == 0;
        let d = ducker(&mut rng, ps);
        let (re, im) = (data(&mut rng), data(&mut rng));
        let mode = rng.range(0, 1);
        let start = if rng.next_u32() % 4 == 0 { rng.range(0, d.hybrid_bands + 2) } else { 0 };
        let max_val = match rng.next_u32() % 4 {
            0 => -1 << 31,
            1 => rng.i32(),
            _ => re.iter().chain(&im).fold(0, |m: i32, &v| m.max(v.saturating_abs())),
        };
        let (mut ea, mut eb) = ([rng.i32(); MAX_PARAMETER_BANDS], [rng.i32(); MAX_PARAMETER_BANDS]);
        let mut sa = rng.i32() as i8;
        let mut sb = sa;
        let ra = ducker_calc_energy(&d, &re, &im, &mut ea, max_val, &mut sa, mode, start);
        let rb = ducker_calc_energy_straight(&d, &re, &im, &mut eb, max_val, &mut sb, mode, start);
        let what = format!("iter {iter} mode {mode} start {start} hybrid {}", d.hybrid_bands);
        assert_eq!(ra, rb, "{what}");
        assert_eq!(ea, eb, "{what}");
        assert_eq!(sa, sb, "{what}");
    }
}
