// Differential tests of fdk_aac_rs_fdk::lpc against the C++ reference.
//
// The filters run over several consecutive calls with their state; after every call the signal,
// the complete state and the state index are compared.

use fdk_aac_rs_cref::fdk_lpc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::lpc::*;

/// A signal value: random, small (realistic headroom) or an edge case.
fn sample(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 4 {
        0 => rng.i32(),
        _ => rng.i32() >> (2 + rng.next_u32() % 12),
    }
}

/// An exponent: usually small, sometimes far out of range (shift amounts of 32 and more).
fn exponent(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 8 {
        0 => rng.range(-40, 40),
        1 => rng.range(28, 34),
        _ => rng.range(-3, 6),
    }
}

/// A ParCor coefficient: |k| < 1 mostly, any value sometimes.
fn parcor_dbl(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 4 {
        0 => rng.i32(),
        _ => (rng.next_u32() as i32) >> 1,
    }
}

#[test]
fn order_ld() {
    let mut len = 0;
    let p = unsafe { c::cref_lpc_order_ld(&mut len) };
    assert_eq!(unsafe { std::slice::from_raw_parts(p, len as usize) }, &ORDER_LD[..]);
}

#[test]
fn synthesis_lattice() {
    let mut rng = Rng::new(10);
    for iter in 0..4000 {
        let order = rng.range(1, LPC_MAX_ORDER as i32);
        let dbl = iter % 2 == 1;
        let coeff_s: Vec<i16> = (0..order).map(|_| rng.i16()).collect();
        let coeff_d: Vec<i32> = (0..order).map(|_| parcor_dbl(&mut rng)).collect();
        let mut st_r: Vec<i32> = (0..order).map(|_| sample(&mut rng)).collect();
        let mut st_c = st_r.clone();
        for call in 0..4 {
            let size = rng.range(1, 40);
            let inc = if rng.next_u32() % 2 == 0 { 1 } else { -1 };
            let (e, e_out) = (exponent(&mut rng), exponent(&mut rng));
            let mut sig_r: Vec<i32> = (0..size).map(|_| sample(&mut rng)).collect();
            let mut sig_c = sig_r.clone();
            if dbl {
                clpc_synthesis_lattice(&mut sig_r, size, e, e_out, inc, &coeff_d, order, &mut st_r);
                unsafe {
                    c::cref_lpc_synthesis_lattice(
                        sig_c.as_mut_ptr(),
                        size,
                        e,
                        e_out,
                        inc,
                        coeff_d.as_ptr(),
                        order,
                        st_c.as_mut_ptr(),
                    )
                };
            } else {
                clpc_synthesis_lattice_sgl(&mut sig_r, size, e, e_out, inc, &coeff_s, order, &mut st_r);
                unsafe {
                    c::cref_lpc_synthesis_lattice_sgl(
                        sig_c.as_mut_ptr(),
                        size,
                        e,
                        e_out,
                        inc,
                        coeff_s.as_ptr(),
                        order,
                        st_c.as_mut_ptr(),
                    )
                };
            }
            let what = format!("iter {iter} call {call} dbl {dbl} order {order} e {e} e_out {e_out} inc {inc}");
            assert_eq!(sig_r, sig_c, "signal, {what}");
            assert_eq!(st_r, st_c, "state, {what}");
        }
    }
}

#[test]
fn synthesis() {
    let mut rng = Rng::new(11);
    for iter in 0..4000 {
        let order = rng.range(1, LPC_MAX_ORDER as i32);
        let dbl = iter % 2 == 1;
        let coeff_s: Vec<i16> = (0..order).map(|_| rng.i16()).collect();
        let coeff_d: Vec<i32> = (0..order).map(|_| parcor_dbl(&mut rng)).collect();
        let coeff_e = exponent(&mut rng);
        let mut st_r: Vec<i32> = (0..order).map(|_| sample(&mut rng)).collect();
        let mut st_c = st_r.clone();
        let mut idx_r = rng.range(0, order - 1);
        let mut idx_c = idx_r;
        for call in 0..4 {
            let size = rng.range(0, 40);
            let inc = if rng.next_u32() % 2 == 0 { 1 } else { -1 };
            let e = exponent(&mut rng);
            let mut sig_r: Vec<i32> = (0..size.max(1)).map(|_| sample(&mut rng)).collect();
            let mut sig_c = sig_r.clone();
            if dbl {
                clpc_synthesis(&mut sig_r, size, e, inc, &coeff_d, coeff_e, order, &mut st_r, &mut idx_r);
                unsafe {
                    c::cref_lpc_synthesis(
                        sig_c.as_mut_ptr(),
                        size,
                        e,
                        inc,
                        coeff_d.as_ptr(),
                        coeff_e,
                        order,
                        st_c.as_mut_ptr(),
                        &mut idx_c,
                    )
                };
            } else {
                clpc_synthesis_sgl(&mut sig_r, size, e, inc, &coeff_s, coeff_e, order, &mut st_r, &mut idx_r);
                unsafe {
                    c::cref_lpc_synthesis_sgl(
                        sig_c.as_mut_ptr(),
                        size,
                        e,
                        inc,
                        coeff_s.as_ptr(),
                        coeff_e,
                        order,
                        st_c.as_mut_ptr(),
                        &mut idx_c,
                    )
                };
            }
            let what = format!("iter {iter} call {call} dbl {dbl} order {order} coeff_e {coeff_e} e {e}");
            assert_eq!(sig_r, sig_c, "signal, {what}");
            assert_eq!(st_r, st_c, "state, {what}");
            assert_eq!(idx_r, idx_c, "state index, {what}");
        }
    }
}

#[test]
fn analysis() {
    let mut rng = Rng::new(12);
    for iter in 0..4000 {
        let order = rng.range(-1, LPC_MAX_ORDER as i32);
        let n = order.max(1) as usize;
        let coeff: Vec<i16> = (0..n).map(|_| rng.i16()).collect();
        // lpcCoeff_e + 1 >= 0 (FDK_ASSERT(shift >= 0)).
        let coeff_e = match rng.next_u32() % 4 {
            0 => rng.range(-1, 40),
            _ => rng.range(-1, 6),
        };
        let with_index = rng.next_u32() % 4 != 0;
        let mut st_r: Vec<i32> = (0..n).map(|_| sample(&mut rng)).collect();
        let mut st_c = st_r.clone();
        let mut idx_r = rng.range(0, n as i32 - 1);
        let mut idx_c = idx_r;
        for call in 0..4 {
            let size = rng.range(0, 40);
            let mut sig_r: Vec<i32> = (0..size.max(1)).map(|_| sample(&mut rng)).collect();
            let mut sig_c = sig_r.clone();
            clpc_analysis(
                &mut sig_r,
                size,
                &coeff,
                coeff_e,
                order,
                &mut st_r,
                if with_index { Some(&mut idx_r) } else { None },
            );
            unsafe {
                c::cref_lpc_analysis(
                    sig_c.as_mut_ptr(),
                    size,
                    coeff.as_ptr(),
                    coeff_e,
                    order,
                    st_c.as_mut_ptr(),
                    if with_index { &mut idx_c } else { std::ptr::null_mut() },
                )
            };
            let what = format!("iter {iter} call {call} order {order} coeff_e {coeff_e} index {with_index}");
            assert_eq!(sig_r, sig_c, "signal, {what}");
            assert_eq!(st_r, st_c, "state, {what}");
            assert_eq!(idx_r, idx_c, "state index, {what}");
        }
    }
}

#[test]
fn parcor_to_lpc() {
    let mut rng = Rng::new(13);
    for iter in 0..20000 {
        let n = rng.range(0, LPC_MAX_ORDER as i32);
        let len = n.max(1) as usize;
        let mut wb_r: Vec<i32> = (0..len).map(|_| rng.i32()).collect();
        let mut wb_c = wb_r.clone();
        if iter % 2 == 0 {
            let refl: Vec<i32> = (0..len).map(|_| parcor_dbl(&mut rng)).collect();
            let mut lpc_r = vec![0x1234_5678i32; len];
            let mut lpc_c = lpc_r.clone();
            let r = clpc_parcor_to_lpc(&refl, &mut lpc_r, n, &mut wb_r);
            let e = unsafe {
                c::cref_lpc_parcor_to_lpc(refl.as_ptr(), lpc_c.as_mut_ptr(), n, wb_c.as_mut_ptr())
            };
            assert_eq!(r, e, "exponent, n {n} refl {refl:?}");
            assert_eq!(lpc_r, lpc_c, "lpc, n {n} refl {refl:?}");
        } else {
            let refl: Vec<i16> = (0..len).map(|_| rng.i16()).collect();
            let mut lpc_r = vec![0x1234i16; len];
            let mut lpc_c = lpc_r.clone();
            let r = clpc_parcor_to_lpc_sgl(&refl, &mut lpc_r, n, &mut wb_r);
            let e = unsafe {
                c::cref_lpc_parcor_to_lpc_sgl(refl.as_ptr(), lpc_c.as_mut_ptr(), n, wb_c.as_mut_ptr())
            };
            assert_eq!(r, e, "exponent, n {n} refl {refl:?}");
            assert_eq!(lpc_r, lpc_c, "lpc, n {n} refl {refl:?}");
        }
        assert_eq!(wb_r, wb_c, "work buffer, n {n}");
    }
}

#[test]
fn auto_to_parcor() {
    let mut rng = Rng::new(14);
    for iter in 0..20000 {
        let n = rng.range(0, LPC_MAX_ORDER as i32);
        let len = n as usize + 1;
        let mut acorr: Vec<i32> = match iter % 3 {
            // the autocorrelation of a random signal
            0 | 1 => {
                let sig: Vec<i64> = (0..64).map(|_| (sample(&mut rng) >> 8) as i64).collect();
                let mut ac: Vec<i64> = (0..len)
                    .map(|l| (0..64 - l.min(63)).map(|i| sig[i] * sig[i + l]).sum::<i64>())
                    .collect();
                let max = ac.iter().map(|v| v.abs()).max().unwrap().max(1);
                let shift = (64 - max.leading_zeros() as i32 - 31 + rng.range(0, 8)).max(0);
                for v in ac.iter_mut() {
                    *v >>= shift;
                }
                ac.iter().map(|&v| v as i32).collect()
            }
            _ => (0..len).map(|_| rng.i32()).collect(),
        };
        if iter % 50 == 0 {
            acorr[0] = 0;
        }
        let mut acorr_c = acorr.clone();
        let mut refl_r = vec![0x1234i16; len];
        let mut refl_c = refl_r.clone();
        let with_gain = iter % 4 != 0;
        let (mut gm_r, mut ge_r) = (0x1111, 0x2222);
        let (mut gm_c, mut ge_c) = (0x1111, 0x2222);
        clpc_auto_to_parcor(
            &mut acorr,
            0,
            &mut refl_r,
            n,
            if with_gain { Some((&mut gm_r, &mut ge_r)) } else { None },
        );
        unsafe {
            c::cref_lpc_auto_to_parcor(
                acorr_c.as_mut_ptr(),
                0,
                refl_c.as_mut_ptr(),
                n,
                if with_gain { &mut gm_c } else { std::ptr::null_mut() },
                &mut ge_c,
            )
        };
        assert_eq!(acorr, acorr_c, "acorr, iter {iter} n {n}");
        assert_eq!(refl_r, refl_c, "refl, iter {iter} n {n}");
        assert_eq!((gm_r, ge_r), (gm_c, ge_c), "gain, iter {iter} n {n}");
    }
}
