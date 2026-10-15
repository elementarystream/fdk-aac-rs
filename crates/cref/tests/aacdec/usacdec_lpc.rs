// Differential tests of fdk_aac_rs_aacdec::usacdec_lpc against the C++ reference.

use crate::usacdec_lpd_util::{rand_dbl_vec, rand_lsf, Bs};
use fdk_aac_rs_aacdec::usacdec_lpc::*;
use fdk_aac_rs_cref::aacdec_usacdec_lpc as c;
use fdk_aac_rs_cref::testutil::Rng;

/// A random point near the lattice (small integers with a fraction, in ZF format), or any value.
fn rand_zf(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 4 {
        0 => rng.i32(),
        1 => (rng.range(-40, 40) << 16).wrapping_add((rng.next_u32() & 0xffff) as i32),
        _ => rng.range(-(1 << 22), 1 << 22),
    }
}

#[test]
fn nearest_neighbor_2d8_and_re8_ppv_random() {
    let mut rng = Rng::new(0x12C0_0001);
    for _ in 0..200_000 {
        let x: [i32; 8] = core::array::from_fn(|_| rand_zf(&mut rng));
        let mut yr = [0i32; 8];
        let mut yc = [0i32; 8];
        nearest_neighbor_2d8(&x, &mut yr);
        let mut xc = x;
        unsafe { c::cref_lpc_nearest_neighbor_2d8(xc.as_mut_ptr(), yc.as_mut_ptr()) };
        assert_eq!(yr, yc, "x {x:?}");

        let r = rng.range(0, 16);
        let mut vr = [0i16; 8];
        let mut vc = [0i16; 8];
        re8_ppv(&x, &mut vr, r);
        unsafe { c::cref_lpc_re8_ppv(xc.as_mut_ptr(), vc.as_mut_ptr(), r) };
        assert_eq!(vr, vc, "x {x:?} r {r}");
    }
}

#[test]
fn re8_decode_base_index_exhaustive() {
    // Every index the bitstream can carry for Q0, Q2 (8 bits), Q3 (12 bits) and Q4 (16 bits).
    for (n, bits) in [(0, 4), (2, 8), (3, 12), (4, 16)] {
        for index in 0..(1u32 << bits) {
            let mut nr = n;
            let mut yr = [0i16; 8];
            re8_decode_base_index(&mut nr, index, &mut yr);
            let mut yc = [0i16; 8];
            let nc = unsafe { c::cref_lpc_re8_decode_base_index(n, index, yc.as_mut_ptr()) };
            assert_eq!((nr, yr), (nc, yc), "n {n} index {index}");
        }
    }
}

#[test]
fn re8_k2y_and_re8_dec_random() {
    let mut rng = Rng::new(0x12C0_0002);
    for _ in 0..100_000 {
        let r = rng.range(1, 16);
        let k: [i32; 8] = core::array::from_fn(|_| (rng.next_u32() & ((1 << r) - 1)) as i32);
        let mut yr = [0i16; 8];
        let mut yc = [0i16; 8];
        re8_k2y(&k, r, &mut yr);
        let mut kc = k;
        unsafe { c::cref_lpc_re8_k2y(kc.as_mut_ptr(), r, yc.as_mut_ptr()) };
        assert_eq!(yr, yc, "k {k:?} r {r}");

        // RE8_dec with the codebook numbers decode_qn() can produce
        let n = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 20, 30, 35, 36, 37, 40][(rng.next_u32() % 17) as usize];
        let base_n = if n > 4 { n - 2 * ((n - 3) >> 1) } else { n };
        let i_idx = (rng.next_u32() & ((1u32 << (4 * base_n.max(1))) - 1)) as i32;
        let nk = if n > 4 { (n - 3) >> 1 } else { 0 };
        let kv: [i32; 8] = core::array::from_fn(|_| if nk > 0 { (rng.next_u32() & ((1u32 << nk.min(31)) - 1)) as i32 } else { 0 });
        let mut or = [0i32; 8];
        let mut oc = [0i32; 8];
        let er = re8_dec(n, i_idx, &kv, &mut or);
        let mut kvc = kv;
        let ec = unsafe { c::cref_lpc_re8_dec(n, i_idx, kvc.as_mut_ptr(), oc.as_mut_ptr()) };
        assert_eq!((er, or), (ec, oc), "n {n} I {i_idx} kv {kv:?}");
    }
}

#[test]
fn lsf_weight_reorder_a_weight_random() {
    let mut rng = Rng::new(0x12C0_0003);
    for it in 0..100_000 {
        let lsf: [i16; 16] = if it % 2 == 0 { rand_lsf(&mut rng) } else { core::array::from_fn(|_| rng.i16()) };
        let xq: Vec<i32> = (0..16).map(|_| if it % 3 == 0 { rng.i32() } else { rng.range(-200, 200) }).collect();
        let nk_mode = rng.range(0, 4);
        let mut r = lsf;
        let mut cc = lsf;
        lsf_weight_2st(&mut r, &xq, nk_mode);
        let mut xqc = xq.clone();
        unsafe { c::cref_lpc_lsf_weight_2st(cc.as_mut_ptr(), xqc.as_mut_ptr(), nk_mode) };
        assert_eq!(r, cc, "lsf_weight_2st {lsf:?} {xq:?} {nk_mode}");

        let min_dist = if it % 5 == 0 { rng.i16() } else { 200 };
        let n = if it % 7 == 0 { rng.range(0, 16) as usize } else { 16 };
        let mut r = lsf;
        let mut cc = lsf;
        reorder_lsf(&mut r, min_dist, n);
        unsafe { c::cref_lpc_reorder_lsf(cc.as_mut_ptr(), min_dist, n as i32) };
        assert_eq!(r, cc, "reorder_lsf");

        let a: [i16; 16] = core::array::from_fn(|_| rng.i16());
        let m = if it % 4 == 0 { rng.range(0, 16) } else { 16 };
        let mut wr = [0i16; 16];
        let mut wc = [0i16; 16];
        e_lpc_a_weight(&mut wr, &a, m);
        unsafe { c::cref_lpc_e_lpc_a_weight(wc.as_mut_ptr(), a.as_ptr(), m) };
        assert_eq!(wr, wc, "E_LPC_a_weight");
        let mut ir = a;
        e_lpc_a_weight_in_place(&mut ir, m);
        let mut ic = a;
        unsafe { c::cref_lpc_e_lpc_a_weight(ic.as_mut_ptr(), ic.as_ptr(), m) };
        assert_eq!(ir, ic, "E_LPC_a_weight in place");
    }
}

#[test]
fn decode_gain_all_codes() {
    let mut rng = Rng::new(0x12C0_0004);
    let codes: Vec<i32> = (-200..400).chain((0..10_000).map(|_| rng.i32())).collect();
    for code in codes {
        let (mut gr, mut er) = (0, 0);
        clpd_decode_gain(&mut gr, &mut er, code);
        let (mut gc, mut ec) = (0, 0);
        unsafe { c::cref_lpc_decode_gain(&mut gc, &mut ec, code) };
        assert_eq!((gr, er), (gc, ec), "gain_code {code}");
    }
}

/// LSF values (scaled by LSF_SCALE) to the LSP domain, like CLpc_Read().
fn lsf_to_lsp_c(lsf: &[i16; 16]) -> [i16; 16] {
    // The LSP of a LSF set through CLpc_Conceal(first_lpd_flag = 0) would need more state; use
    // the cosine directly.
    core::array::from_fn(|i| {
        let x = (lsf[i] as f64) / 8192.0 * core::f64::consts::PI / 6400.0 * 8192.0;
        (x.cos() * 32767.0) as i16
    })
}

#[test]
fn lsp_a_conversion_random() {
    let mut rng = Rng::new(0x12C0_0005);
    for it in 0..100_000 {
        let lsp: [i16; 16] = match it % 3 {
            0 => core::array::from_fn(|_| rng.i16()),
            1 => lsf_to_lsp_c(&rand_lsf(&mut rng)),
            _ => {
                let mut v: [i16; 16] = core::array::from_fn(|_| rng.i16());
                v.sort_unstable_by(|a, b| b.cmp(a));
                v
            }
        };
        let mut ar = [0i16; 16];
        let mut er = 0;
        e_lpc_f_lsp_a_conversion(&lsp, &mut ar, &mut er);
        let mut lc = lsp;
        let mut ac = [0i16; 16];
        let mut ec = 0;
        unsafe { c::cref_lpc_lsp_a_conversion(lc.as_mut_ptr(), ac.as_mut_ptr(), &mut ec) };
        assert_eq!((ar, er), (ac, ec), "lsp {lsp:?}");
    }
}

#[test]
fn conceal_random() {
    let mut rng = Rng::new(0x12C0_0006);
    for it in 0..50_000 {
        let mut lsp_r = [[0i16; 16]; 5];
        for row in lsp_r.iter_mut() {
            *row = core::array::from_fn(|_| rng.i16());
        }
        let lpc4: [i16; 16] = if it % 2 == 0 { rand_lsf(&mut rng) } else { core::array::from_fn(|_| rng.i16()) };
        let mean: [i16; 16] = if it % 3 == 0 { rand_lsf(&mut rng) } else { core::array::from_fn(|_| rng.i16()) };
        let first = (rng.next_u32() % 2) as i32;
        let mut lsp_c = lsp_r;
        let mut lpc4_r = lpc4;
        let mut lpc4_c = lpc4;
        clpc_conceal(&mut lsp_r, &mut lpc4_r, &mean, first);
        let mut mean_c = mean;
        unsafe { c::cref_lpc_conceal(lsp_c.as_mut_ptr() as *mut i16, lpc4_c.as_mut_ptr(), mean_c.as_mut_ptr(), first) };
        assert_eq!((lsp_r, lpc4_r), (lsp_c, lpc4_c));
    }
}

#[test]
fn decode_avq_random_bitstreams() {
    let mut rng = Rng::new(0x12C0_0007);
    for it in 0..60_000 {
        let mut b = Bs::random(&mut rng, 64);
        let (nk_mode, no_qn, length) = match it % 5 {
            0 => (1, 1, [32, 48, 64, 96, 128][(rng.next_u32() % 5) as usize]),
            _ => (rng.range(0, 3), 2, 8),
        };
        let init = rand_dbl_vec(&mut rng, 128);
        let mut or = init.clone();
        let mut oc = init;
        let er = clpc_decode_avq(&mut b.rs, &mut or, nk_mode, no_qn, length);
        let (st, buf) = b.c();
        let ec = unsafe { c::cref_lpc_decode_avq(st, buf, oc.as_mut_ptr(), nk_mode, no_qn, length) };
        assert_eq!(er, ec, "error");
        assert_eq!(or, oc, "output nk_mode {nk_mode} no_qn {no_qn} length {length}");
        b.check("CLpc_DecodeAVQ");
    }
}

#[test]
fn read_random_bitstreams() {
    let mut rng = Rng::new(0x12C0_0008);
    for it in 0..60_000 {
        let mut b = Bs::random(&mut rng, 256);
        let mut lsp_r = [[0i16; 16]; 5];
        for row in lsp_r.iter_mut() {
            *row = core::array::from_fn(|_| rng.i16());
        }
        let mut lsp_c = lsp_r;
        let lpc4 = rand_lsf(&mut rng);
        let (mut lpc4_r, mut lpc4_c) = (lpc4, lpc4);
        let mean: [i16; 16] = core::array::from_fn(|_| rng.i16());
        let (mut mean_r, mut mean_c) = (mean, mean);
        let stab: [i16; 4] = core::array::from_fn(|_| rng.i16());
        let (mut stab_r, mut stab_c) = (stab, stab);
        let mut mod_: [u8; 4] = core::array::from_fn(|_| (rng.next_u32() % 4) as u8);
        if it % 4 == 0 {
            mod_ = [[3, 3, 3, 3], [2, 2, 2, 2], [0, 0, 0, 0], [1, 1, 1, 1]][(rng.next_u32() % 4) as usize];
        }
        let first = (rng.next_u32() % 2) as i32;
        let lost = (rng.next_u32() % 2) as i32;
        let ok = (rng.next_u32() % 2) as i32;
        let er = clpc_read(&mut b.rs, &mut lsp_r, &mut lpc4_r, &mut mean_r, &mut stab_r, &mod_, first, lost, ok);
        let (st, buf) = b.c();
        let mut mod_c = mod_;
        let ec = unsafe {
            c::cref_lpc_read(
                st,
                buf,
                lsp_c.as_mut_ptr() as *mut i16,
                lpc4_c.as_mut_ptr(),
                mean_c.as_mut_ptr(),
                stab_c.as_mut_ptr(),
                mod_c.as_mut_ptr(),
                first,
                lost,
                ok,
            )
        };
        assert_eq!(er, ec, "error");
        assert_eq!((lsp_r, lpc4_r, mean_r, stab_r), (lsp_c, lpc4_c, mean_c, stab_c), "mod {mod_:?}");
        b.check("CLpc_Read");
    }
}
