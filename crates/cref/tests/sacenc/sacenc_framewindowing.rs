// Differential tests of fdk_aac_rs_sacenc::sacenc_framewindowing against the C++ reference.
//
// calcTaperWin for every length, random sequences of list operations, the initialization for
// every number of time slots, the windows and the framing for every frame length with and
// without frame keeping, with no transient (as the encoder uses it) and with transients in
// either half, and the analysis windowing in both dimensions and in place.

use fdk_aac_rs_cref::sacenc_sacenc_framewindowing as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::common_fix::FixpDpk;
use fdk_aac_rs_sacenc::sacenc_bitstream::FramingInfo;
use fdk_aac_rs_sacenc::sacenc_framewindowing as r;
use fdk_aac_rs_sacenc::sacenc_framewindowing::*;

use super::sacenc_vectorfunctions::{cmat, flat};

pub fn fw_ints(f: &FrameWindow) -> Vec<i32> {
    let mut v = vec![
        f.n_time_slots_max,
        f.b_frame_keep,
        f.start_slope,
        f.stop_slope,
        f.start_rect,
        f.stop_rect,
        f.taper_ana_len,
        f.taper_syn_len,
    ];
    v.extend_from_slice(&f.p_taper_ana_fdk);
    v.extend_from_slice(&f.p_taper_syn_fdk);
    v
}

fn fw_from(v: &[i32]) -> FrameWindow {
    let mut f = FrameWindow {
        n_time_slots_max: v[0],
        b_frame_keep: v[1],
        start_slope: v[2],
        stop_slope: v[3],
        start_rect: v[4],
        stop_rect: v[5],
        taper_ana_len: v[6],
        taper_syn_len: v[7],
        ..Default::default()
    };
    f.p_taper_ana_fdk.copy_from_slice(&v[8..40]);
    f.p_taper_syn_fdk.copy_from_slice(&v[40..72]);
    f
}

pub fn list_ints(l: &FrameWinList) -> [i32; 5] {
    [l.dat[0].slot, l.dat[0].hold, l.dat[1].slot, l.dat[1].hold, l.n]
}

fn list_from(v: &[i32; 5]) -> FrameWinList {
    FrameWinList {
        dat: [FrameWinData { slot: v[0], hold: v[1] }, FrameWinData { slot: v[2], hold: v[3] }],
        n: v[4],
    }
}

#[test]
fn taper_and_list() {
    for ts in 0..32 {
        let mut wc = [0x1234i32; 33];
        let mut wr = wc;
        unsafe { c::cref_sacenc_framewindowing_calcTaperWin(wc.as_mut_ptr(), ts) };
        r::calc_taper_win(&mut wr, ts);
        assert_eq!(wc, wr, "calcTaperWin {ts}");
    }
    let mut rng = Rng::new(1);
    for it in 0..20000 {
        let mut lc = [rng.range(-3, 20), rng.range(0, 1), rng.range(-3, 20), rng.range(0, 1), rng.range(0, 2)];
        let mut lr = list_from(&lc);
        for _ in 0..6 {
            let op = rng.range(0, 3);
            let (a, b) = (rng.range(-3, 20), rng.range(-3, 20));
            let b = if op == 1 { b & 1 } else { b };
            let ec = unsafe { c::cref_sacenc_framewindowing_list(lc.as_mut_ptr(), op, a, b) };
            let er = match op {
                0 => r::frame_win_list_reset(&mut lr),
                1 => r::frame_window_list_add(&mut lr, a, b),
                2 => r::frame_window_list_remove(&mut lr, a),
                _ => r::frame_window_list_limit(&mut lr, a, b),
            };
            assert_eq!((ec, lc), (er, list_ints(&lr)), "list it {it} op {op} {a} {b}");
        }
    }
}

#[test]
fn get_window() {
    assert_eq!(unsafe { c::cref_sacenc_framewindowing_size() }, 72);
    let mut rng = Rng::new(2);
    let h = unsafe { c::cref_sacenc_framewindowing_create() };
    for it in 0..30000 {
        let max = if rng.range(0, 10) == 0 { rng.range(-1, 24) } else { rng.range(1, 16) };
        let keep = rng.range(0, 1);
        let mut f = FrameWindow::default();
        // Start from a random structure.
        let img: Vec<i32> = (0..72).map(|_| rng.i32()).collect();
        unsafe { c::cref_sacenc_framewindowing_set(h, img.as_ptr()) };
        f = if it % 2 == 0 { fw_from(&img) } else { f };
        if it % 2 != 0 {
            unsafe { c::cref_sacenc_framewindowing_set(h, fw_ints(&f).as_ptr()) };
        }
        let ec = unsafe { c::cref_sacenc_framewindowing_init(h, max, keep) };
        let er = r::fdk_sacenc_frame_window_init(&mut f, &FrameWindowConfig { n_time_slots_max: max, b_frame_keep: keep });
        let mut v = vec![0i32; 72];
        unsafe { c::cref_sacenc_framewindowing_get(h, v.as_mut_ptr()) };
        assert_eq!((ec, &v), (er, &fw_ints(&f)), "init it {it} max {max}");
        // The windows hold 3 * 16 values (the encoder's maximum of 16 time slots).
        if er != 0 || max < 1 || max > 16 {
            continue;
        }

        for k in 0..4 {
            let ts = if rng.range(0, 3) == 0 { rng.range(-1, max + 1) } else { max };
            let t = ts.max(0);
            let tr = match if t == 0 { 0 } else { rng.range(0, 3) } {
                0 => [-1, -1],
                1 => [rng.range(-1, t - 1), -1],
                2 => [-1, rng.range(t, 2 * t - 1)],
                _ => [rng.range(-1, t - 1), rng.range(t, 2 * t - 1)],
            };
            let avoid = rng.range(0, 3);
            let mut fic = [rng.next_u32() as u8, rng.next_u32() as u8, rng.next_u32() as u8, rng.next_u32() as u8];
            let mut fir = FramingInfo { bs_framing_type: fic[0], num_param_sets: fic[1], bs_param_slots: [fic[2], fic[3]] };
            let w0: Vec<i32> = (0..48).map(|_| rng.i32()).collect();
            let w1: Vec<i32> = (0..48).map(|_| rng.i32()).collect();
            let (mut w0c, mut w1c) = (w0.clone(), w1.clone());
            let mut wr = [w0, w1];
            let mut lc = [rng.i32(), 0, rng.i32(), 1, rng.range(0, 2)];
            let mut lr = list_from(&lc);
            let mut trc = tr;
            let ec = unsafe {
                c::cref_sacenc_framewindowing_getWindow(h, trc.as_mut_ptr(), ts, fic.as_mut_ptr(), w0c.as_mut_ptr(), w1c.as_mut_ptr(), lc.as_mut_ptr(), avoid)
            };
            let er = r::fdk_sacenc_frame_window_get_window(&f, &tr, ts, &mut fir, &mut wr, &mut lr, avoid);
            let what = format!("getWindow it {it} k {k} max {max} ts {ts} tr {tr:?} keep {keep}");
            assert_eq!(ec, er, "{what}");
            assert_eq!(fic, [fir.bs_framing_type, fir.num_param_sets, fir.bs_param_slots[0], fir.bs_param_slots[1]], "{what}");
            assert_eq!(lc, list_ints(&lr), "{what}");
            assert_eq!((w0c, w1c), (wr[0].clone(), wr[1].clone()), "{what}");
        }
    }
    unsafe { c::cref_sacenc_framewindowing_destroy(h) };
    assert_eq!(unsafe { c::cref_sacenc_framewindowing_create_null() }, r::fdk_sacenc_frame_window_create(None));
}

#[test]
fn analysis_windowing() {
    let mut rng = Rng::new(3);
    for it in 0..5000 {
        let slots = rng.range(1, 48) as usize;
        let bands = rng.range(1, 64) as usize;
        let win: Vec<i32> = (0..48)
            .map(|_| match rng.range(0, 3) {
                0 => 0x7FFF_FFFF,
                1 => 0,
                _ => rng.i32(),
            })
            .collect();
        let nts = rng.range(0, slots as i32);
        let st = rng.range(0, nts.max(0));
        let nb = rng.range(0, bands as i32);
        let dim = rng.range(0, 1);
        let inp = cmat(&mut rng, slots, bands);
        let (orows, ocols) = if dim == FW_CHANGE_DIM { (bands, slots) } else { (slots, bands) };
        let out0 = cmat(&mut rng, orows, ocols);
        let mut wc = win.clone();
        let mut oc = flat(&out0);
        let ec = unsafe {
            c::cref_sacenc_framewindowing_analysisWindowing(
                nts, st, wc.as_mut_ptr(), flat(&inp).as_ptr(), slots as i32, bands as i32,
                oc.as_mut_ptr(), orows as i32, ocols as i32, nb, dim, 0,
            )
        };
        let mut or = out0.clone();
        let er = r::fdk_sacenc_analysis_windowing(nts, st, &win, &inp, &mut or, nb, dim);
        assert_eq!((ec, &oc), (er, &flat(&or)), "it {it}");

        // In place (FW_LEAVE_DIM)
        let mut oc = flat(&inp);
        let ec = unsafe {
            c::cref_sacenc_framewindowing_analysisWindowing(
                nts, st, wc.as_mut_ptr(), oc.as_ptr(), slots as i32, bands as i32,
                oc.as_mut_ptr(), slots as i32, bands as i32, nb, FW_LEAVE_DIM, 1,
            )
        };
        let mut or: Vec<Vec<FixpDpk>> = inp.clone();
        let er = r::fdk_sacenc_analysis_windowing_in_place(nts, st, &win, &mut or, nb);
        assert_eq!((ec, &oc), (er, &flat(&or)), "in place it {it}");
    }
}
