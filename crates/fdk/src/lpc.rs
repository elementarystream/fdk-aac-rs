// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_lpc.h, libFDK/src/FDK_lpc.cpp.

//! Port of `FDK_lpc.h`: LPC related functions.
//!
//! The C++ overloads for `FIXP_LPC_TNS` (= `FIXP_DBL`) and `FIXP_LPC` (= `FIXP_SGL`)
//! coefficients are separate functions here; the `FIXP_SGL` variants carry the `_sgl` suffix.
//! Both variants compile the same code (`FUNCTION_CLpc_SynthesisLattice_SGL` and
//! `_DBL` are not defined); they differ only in the primitives (`fMult`, `schur_div`).
//!
//! The signal is a slice and `signal_size` the number of processed values, as in C: the
//! traversal starts at `signal[signal_size - 1]` for `inc == -1` and at `signal[0]` otherwise,
//! and advances by `inc`. The arithmetic wraps around on overflow like the reference builds.

use crate::common_fix::*;
use crate::fixpoint_math::{f_div_norm_signed, schur_div};
use crate::scale::{saturate_left_shift_alt, scale_value, scale_value_saturate};

/// `LPC_MAX_ORDER`
pub const LPC_MAX_ORDER: usize = 24;

/// `FIXP_LPC_TNS`: LPC coefficient type of the TNS IIR synthesis variant.
pub type FixpLpcTns = FixpDbl;
/// `FIXP_LPC`: LPC coefficient type.
pub type FixpLpc = FixpSgl;

/// `FX_DBL2FX_LPC_TNS(x)`
#[inline(always)]
pub const fn fx_dbl2fx_lpc_tns(x: FixpDbl) -> FixpLpcTns {
    x
}

/// `FX_DBL2FXCONST_LPC_TNS(x)`
#[inline(always)]
pub const fn fx_dbl2fxconst_lpc_tns(x: FixpDbl) -> FixpLpcTns {
    x
}

/// `FX_LPC_TNS2FX_DBL(x)`
#[inline(always)]
pub const fn fx_lpc_tns2fx_dbl(x: FixpLpcTns) -> FixpDbl {
    x
}

/// `FL2FXCONST_LPC_TNS(val)`
#[inline(always)]
pub const fn fl2fxconst_lpc_tns(val: f64) -> FixpLpcTns {
    fl2fxconst_dbl(val)
}

/// `MAXVAL_LPC_TNS`
pub const MAXVAL_LPC_TNS: i32 = MAXVAL_DBL;

/// `FX_DBL2FX_LPC(x)`
#[inline(always)]
pub const fn fx_dbl2fx_lpc(x: FixpDbl) -> FixpLpc {
    fx_dbl2fx_sgl(x)
}

/// `FX_DBL2FXCONST_LPC(x)`
#[inline(always)]
pub const fn fx_dbl2fxconst_lpc(x: FixpDbl) -> FixpLpc {
    fx_dbl2fxconst_sgl(x)
}

/// `FX_LPC2FX_DBL(x)`
#[inline(always)]
pub const fn fx_lpc2fx_dbl(x: FixpLpc) -> FixpDbl {
    fx_sgl2fx_dbl(x)
}

/// `FL2FXCONST_LPC(val)`
#[inline(always)]
pub const fn fl2fxconst_lpc(val: f64) -> FixpLpc {
    fl2fxconst_sgl(val)
}

/// `MAXVAL_LPC`
pub const MAXVAL_LPC: i32 = MAXVAL_SGL;

/// `order_ld[LPC_MAX_ORDER]`: internal scaling of the LPC synthesis to avoid overflow of the
/// filter states. This depends on the LPC order, because the LPC order defines the amount of MAC
/// operations. Assume that the synthesis filter output does not clip and the filter accu does
/// change no more than 1.0 for each iteration: `ceil(0.5*log((1:24))/log(2))`.
///
/// `scripts/dump_table.py libFDK/src/FDK_lpc.cpp order_ld SCHAR --name ORDER_LD --per-line 24`
#[rustfmt::skip]
pub static ORDER_LD: [i8; 24] = [
    0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3,
];

/// Index of the first processed signal value for the increment `inc`.
#[inline(always)]
fn signal_start(signal_size: i32, inc: i32) -> isize {
    if inc == -1 {
        signal_size as isize - 1
    } else {
        0
    }
}

/// `SATURATE_SHIFT(x, -lpcCoeff_e - 1, DFRACT_BITS)` of `CLpc_Synthesis()`, as both reference
/// builds compile it.
///
/// For `lpcCoeff_e >= 0` the macro is a saturating left shift by `lpcCoeff_e + 1`. The builds
/// compute the saturation limit `MAXVAL_DBL >> (lpcCoeff_e + 1)` as `0x3fffffff >> lpcCoeff_e`
/// (logical shift by a register, modulo 32; x86 also `0xc0000000 >> lpcCoeff_e` arithmetically
/// for the lower limit), which differs from the macro for `lpcCoeff_e % 32 == 31` (the macro
/// shifts by 32 there). The right shift for `lpcCoeff_e < 0` cannot saturate; both builds shift
/// by `-lpcCoeff_e - 1` modulo 32.
#[inline(always)]
fn synthesis_saturate(x: FixpDbl, lpc_coeff_e: i32) -> FixpDbl {
    if lpc_coeff_e >= 0 {
        let hi = (0x3fff_ffffu32 >> (lpc_coeff_e as u32 & 31)) as i32;
        let shl = x.wrapping_shl(lpc_coeff_e.wrapping_add(1) as u32);
        if ARCH_ARM {
            // arm/scale_arm.h: SATURATE_LEFT_SHIFT with the XOR of the sign
            let sign = x >> (DFRACT_BITS - 1);
            if (x ^ sign) > hi {
                sign ^ MAXVAL_DBL
            } else {
                shl
            }
        } else if x > hi {
            MAXVAL_DBL
        } else if x < !hi {
            MINVAL_DBL
        } else {
            shl
        }
    } else {
        x.wrapping_shr(lpc_coeff_e.wrapping_neg().wrapping_sub(1) as u32)
    }
}

/// `CLpc_SynthesisLattice(signal, signal_size, signal_e, signal_e_out, inc, coeff, order,
/// state)` with `FIXP_SGL` ParCor coefficients.
///
/// Synthesizes the signal from the residual through LPC synthesis, using ParCor coefficients, in
/// place. The algorithm assumes a filter gain of max 1.0. If the filter gain is higher, this must
/// be accounted into the values of `signal_e` and/or `signal_e_out` to avoid overflows. `coeff`
/// and `state` hold at least `order` values (`1 <= order <= LPC_MAX_ORDER`).
pub fn clpc_synthesis_lattice_sgl(
    signal: &mut [FixpDbl],
    signal_size: i32,
    signal_e: i32,
    signal_e_out: i32,
    inc: i32,
    coeff: &[FixpSgl],
    order: i32,
    state: &mut [FixpDbl],
) {
    fdk_assert!(order as usize <= LPC_MAX_ORDER, "FDK_ASSERT(order <= LPC_MAX_ORDER)");
    fdk_assert!(order > 0, "FDK_ASSERT(order > 0)");

    let mut p_signal = signal_start(signal_size, inc);

    // tmp = x(k) - K(M)*g(M);
    // for m=M-1:-1:1
    //         tmp = tmp - K(m) * g(m);
    //         g(m+1) = g(m) + K(m) * tmp;
    // endfor
    // g(1) = tmp;
    //
    // y(k) = tmp;

    let shift = -(ORDER_LD[(order - 1) as usize] as i32);

    let mut i = signal_size;
    while i != 0 {
        let mut p_state = order as usize - 1;
        let mut p_coeff = order as usize - 1;

        let mut tmp = scale_value(signal[p_signal as usize], shift.wrapping_add(signal_e))
            .wrapping_sub(f_mult_div2(coeff[p_coeff], state[p_state]));
        let mut j = order - 1;
        while j != 0 {
            // pCoeff and pState have been decremented once before the loop.
            tmp = f_mult_sub_div2(tmp, coeff[p_coeff - 1], state[p_state - 1]);
            state[p_state] = state[p_state - 1]
                .wrapping_add(f_mult_div2(coeff[p_coeff - 1], tmp).wrapping_shl(2));
            p_coeff -= 1;
            p_state -= 1;
            j -= 1;
        }

        signal[p_signal as usize] =
            scale_value_saturate(tmp, shift.wrapping_neg().wrapping_sub(signal_e_out));

        // exponent of state[] is -1
        state[p_state] = tmp.wrapping_shl(1);
        p_signal += inc as isize;
        i -= 1;
    }
}

/// `CLpc_SynthesisLattice(signal, signal_size, signal_e, signal_e_out, inc, coeff, order,
/// state)` with `FIXP_DBL` ParCor coefficients. See [`clpc_synthesis_lattice_sgl`].
///
/// An optimization of the port (the straight port is [`clpc_synthesis_lattice_straight`]): for
/// `inc == ±1`, `1 <= order <= LPC_MAX_ORDER` and buffers that hold what the C code accesses,
/// the same operations in the same order run on local copies of the coefficients and the state
/// (fixed-size arrays without per-access checks, the state written back at the end), with the
/// primitives of the variant. The filter recursion `tmp = SATURATE_LEFT_SHIFT_ALT((tmp >> 1) -
/// fMultDiv2(k, s), 1)` is latency bound: the selects of the saturation sit on it. Here both
/// values of a tap are first computed as plain doublings and one predictable branch checks that
/// neither saturates (otherwise the tap is recomputed with the saturation, see `sat_alt1`);
/// `tmp >> 1` is then the unshifted value, so the recursion waits for one subtraction per tap.
/// The taps are unrolled by four and order 1 keeps the state in a register. Other arguments
/// take the straight port.
pub fn clpc_synthesis_lattice(
    signal: &mut [FixpDbl],
    signal_size: i32,
    signal_e: i32,
    signal_e_out: i32,
    inc: i32,
    coeff: &[FixpDbl],
    order: i32,
    state: &mut [FixpDbl],
) {
    let ord = order as usize;
    let n = signal_size as usize;
    if !(1..=LPC_MAX_ORDER as i32).contains(&order)
        || (inc != 1 && inc != -1)
        || signal_size <= 0
        || signal.len() < n
        || coeff.len() < ord
        || state.len() < ord
    {
        return clpc_synthesis_lattice_straight(
            signal,
            signal_size,
            signal_e,
            signal_e_out,
            inc,
            coeff,
            order,
            state,
        );
    }
    let mut c = [0 as FixpDbl; LPC_MAX_ORDER];
    let mut st = [0 as FixpDbl; LPC_MAX_ORDER];
    c[..ord].copy_from_slice(&coeff[..ord]);
    st[..ord].copy_from_slice(&state[..ord]);
    let e_in = signal_e.wrapping_sub(1);
    let e_out = signal_e_out.wrapping_neg();
    // The traversal order of the C code (`inc == -1` starts at the end); every value is read
    // once and written once, at its own position.
    if ord == 1 {
        // No inner taps: state[0] stays in a register from one value to the next.
        let (k, mut s) = (c[0], st[0]);
        let mut one = |x: &mut FixpDbl| {
            let (_, tmp) = sat_alt1(f_mult_sub_div2(scale_value(*x, e_in), k, s));
            *x = scale_value(tmp, e_out);
            s = tmp;
        };
        if inc == 1 {
            signal[..n].iter_mut().for_each(&mut one);
        } else {
            signal[..n].iter_mut().rev().for_each(&mut one);
        }
        st[0] = s;
    } else if inc == 1 {
        for x in signal[..n].iter_mut() {
            lattice_sample(x, &mut st, &c, ord, e_in, e_out);
        }
    } else {
        for x in signal[..n].iter_mut().rev() {
            lattice_sample(x, &mut st, &c, ord, e_in, e_out);
        }
    }
    state[..ord].copy_from_slice(&st[..ord]);
}

/// One output value of [`clpc_synthesis_lattice`] (`1 <= ord <= LPC_MAX_ORDER`).
#[inline(always)]
fn lattice_sample(
    x: &mut FixpDbl,
    st: &mut [FixpDbl; LPC_MAX_ORDER],
    c: &[FixpDbl; LPC_MAX_ORDER],
    ord: usize,
    e_in: i32,
    e_out: i32,
) {
    let accu = f_mult_sub_div2(scale_value(*x, e_in), c[ord - 1], st[ord - 1]);
    let (mut half, mut tmp) = sat_alt1(accu);
    // Taps p = order - 1 .. 1: pState[0] = state[p - 1], pState[1] = state[p]. One tap: `fMultSubDiv2(tmp >> 1, k, s)` (`half` is `tmp >> 1`) and the state update. Both
    // values are first computed as if neither saturated; if one of them is outside the range
    // where SATURATE_LEFT_SHIFT_ALT is the plain doubling, the tap is recomputed with the
    // saturation.
    let tap =
        |p: usize, half: &mut FixpDbl, tmp: &mut FixpDbl, st: &mut [FixpDbl; LPC_MAX_ORDER]| {
            let s = st[p - 1];
            let k = c[p - 1];
            let x1 = half.wrapping_sub(f_mult_div2(k, s));
            let t1 = x1.wrapping_shl(1);
            let x2 = f_mult_add_div2(s >> 1, k, t1);
            if in_alt1_range(x1) & in_alt1_range(x2) {
                *half = x1;
                *tmp = t1;
                st[p] = x2 << 1;
            } else {
                (*half, *tmp) = sat_alt1(x1);
                st[p] = sat_alt1(f_mult_add_div2(s >> 1, k, *tmp)).1;
            }
        };
    let mut p = ord - 1;
    while p >= 4 {
        tap(p, &mut half, &mut tmp, st);
        tap(p - 1, &mut half, &mut tmp, st);
        tap(p - 2, &mut half, &mut tmp, st);
        tap(p - 3, &mut half, &mut tmp, st);
        p -= 4;
    }
    while p >= 1 {
        tap(p, &mut half, &mut tmp, st);
        p -= 1;
    }
    *x = scale_value(tmp, e_out);
    st[0] = tmp;
}

/// `-2^30 < x < 2^30`, where `SATURATE_LEFT_SHIFT_ALT(x, 1, DFRACT_BITS)` is `2 x` (above, it
/// is `MAXVAL_DBL`, at and below `-2^30` it is `-MAXVAL_DBL`). `x + 2^30` wraps to a value
/// `<= 0` exactly outside this range.
#[inline(always)]
fn in_alt1_range(x: FixpDbl) -> bool {
    x.wrapping_add(0x4000_0000) > 0
}

/// `(y >> 1, y)` with `y = SATURATE_LEFT_SHIFT_ALT(x, 1, DFRACT_BITS)`: `(x, 2 x)` in the range
/// of [`in_alt1_range`], otherwise the saturated value through a call that is not inlined.
#[inline(always)]
fn sat_alt1(x: FixpDbl) -> (FixpDbl, FixpDbl) {
    #[cold]
    #[inline(never)]
    fn saturated(x: FixpDbl) -> (FixpDbl, FixpDbl) {
        let y = saturate_left_shift_alt(x, 1, DFRACT_BITS);
        (y >> 1, y)
    }
    if !in_alt1_range(x) {
        saturated(x)
    } else {
        (x, x << 1)
    }
}

/// `CLpc_SynthesisLattice()` with `FIXP_DBL` ParCor coefficients, the straight port (the
/// reference of the optimized [`clpc_synthesis_lattice`]).
pub fn clpc_synthesis_lattice_straight(
    signal: &mut [FixpDbl],
    signal_size: i32,
    signal_e: i32,
    signal_e_out: i32,
    inc: i32,
    coeff: &[FixpDbl],
    order: i32,
    state: &mut [FixpDbl],
) {
    fdk_assert!(order as usize <= LPC_MAX_ORDER, "FDK_ASSERT(order <= LPC_MAX_ORDER)");
    fdk_assert!(order > 0, "FDK_ASSERT(order > 0)");

    let mut p_signal = signal_start(signal_size, inc);

    fdk_assert!(signal_size > 0, "FDK_ASSERT(signal_size > 0)");
    let mut i = signal_size;
    while i != 0 {
        let mut p_state = order as usize - 1;
        let mut p_coeff = order as usize - 1;

        let accu = f_mult_sub_div2(
            scale_value(signal[p_signal as usize], signal_e.wrapping_sub(1)),
            coeff[p_coeff],
            state[p_state],
        );
        let mut tmp = saturate_left_shift_alt(accu, 1, DFRACT_BITS);

        let mut j = order - 1;
        while j != 0 {
            // pCoeff and pState have been decremented once before the loop.
            let accu = f_mult_sub_div2(tmp >> 1, coeff[p_coeff - 1], state[p_state - 1]);
            tmp = saturate_left_shift_alt(accu, 1, DFRACT_BITS);

            let accu = f_mult_add_div2(state[p_state - 1] >> 1, coeff[p_coeff - 1], tmp);
            state[p_state] = saturate_left_shift_alt(accu, 1, DFRACT_BITS);

            p_coeff -= 1;
            p_state -= 1;
            j -= 1;
        }

        signal[p_signal as usize] = scale_value(tmp, signal_e_out.wrapping_neg());

        // exponent of state[] is 0
        state[p_state] = tmp;
        p_signal += inc as isize;
        i -= 1;
    }
}

/// `CLpc_Synthesis(signal, signal_size, signal_e, inc, lpcCoeff_m, lpcCoeff_e, order, state,
/// pStateIndex)` with `FIXP_LPC_TNS` (`FIXP_DBL`) coefficients (the `LPC_SYNTHESIS_IIR`
/// version).
///
/// Synthesizes the signal from the residual through LPC synthesis, using LP coefficients, in
/// place. `state` holds at least `order` values, `p_state_index` is the index of the newest
/// state (`0 <= *p_state_index < order`).
pub fn clpc_synthesis(
    signal: &mut [FixpDbl],
    signal_size: i32,
    signal_e: i32,
    inc: i32,
    lpc_coeff_m: &[FixpLpcTns],
    lpc_coeff_e: i32,
    order: i32,
    state: &mut [FixpDbl],
    p_state_index: &mut i32,
) {
    let mut state_index = *p_state_index;
    let ord = order.max(0) as usize;

    let mut coeff = [0 as FixpLpcTns; 2 * LPC_MAX_ORDER];
    coeff[..ord].copy_from_slice(&lpc_coeff_m[..ord]);
    coeff[ord..2 * ord].copy_from_slice(&lpc_coeff_m[..ord]);

    fdk_assert!(order as usize <= LPC_MAX_ORDER, "FDK_ASSERT(order <= LPC_MAX_ORDER)");
    fdk_assert!(state_index < order, "FDK_ASSERT(stateIndex < order)");

    let mut p_signal = signal_start(signal_size, inc);

    // y(n) = x(n) - lpc[1]*y(n-1) - ... - lpc[order]*y(n-order)

    for _ in 0..signal_size {
        let p_coeff = &coeff[(order - state_index) as usize..];

        let mut x =
            scale_value(signal[p_signal as usize], lpc_coeff_e.wrapping_add(1).wrapping_neg());
        for j in 0..ord {
            x = x.wrapping_sub(f_mult_div2(state[j], p_coeff[j]));
        }
        x = synthesis_saturate(x, lpc_coeff_e);

        // Update states
        state_index = if state_index - 1 < 0 { order - 1 } else { state_index - 1 };
        state[state_index as usize] = x;

        signal[p_signal as usize] = scale_value(x, signal_e);
        p_signal += inc as isize;
    }

    *p_state_index = state_index;
}

/// `CLpc_Synthesis(signal, signal_size, signal_e, inc, coeff, coeff_e, order, filtState,
/// pStateIndex)` with `FIXP_LPC` (`FIXP_SGL`) coefficients (the default version). See
/// [`clpc_synthesis`].
pub fn clpc_synthesis_sgl(
    signal: &mut [FixpDbl],
    signal_size: i32,
    signal_e: i32,
    inc: i32,
    lpc_coeff_m: &[FixpLpc],
    lpc_coeff_e: i32,
    order: i32,
    state: &mut [FixpDbl],
    p_state_index: &mut i32,
) {
    let mut state_index = *p_state_index;
    let ord = order.max(0) as usize;

    let mut coeff = [0 as FixpLpc; 2 * LPC_MAX_ORDER];
    coeff[..ord].copy_from_slice(&lpc_coeff_m[..ord]);
    coeff[ord..2 * ord].copy_from_slice(&lpc_coeff_m[..ord]);

    fdk_assert!(order as usize <= LPC_MAX_ORDER, "FDK_ASSERT(order <= LPC_MAX_ORDER)");
    fdk_assert!(state_index < order, "FDK_ASSERT(stateIndex < order)");

    let mut p_signal = signal_start(signal_size, inc);

    // y(n) = x(n) - lpc[1]*y(n-1) - ... - lpc[order]*y(n-order)

    for _ in 0..signal_size {
        let p_coeff = &coeff[(order - state_index) as usize..];

        let mut x =
            scale_value(signal[p_signal as usize], lpc_coeff_e.wrapping_add(1).wrapping_neg());
        for j in 0..ord {
            x = x.wrapping_sub(f_mult_div2(state[j], p_coeff[j]));
        }
        x = synthesis_saturate(x, lpc_coeff_e);

        // Update states
        state_index = if state_index - 1 < 0 { order - 1 } else { state_index - 1 };
        state[state_index as usize] = x;

        signal[p_signal as usize] = scale_value(x, signal_e);
        p_signal += inc as isize;
    }

    *p_state_index = state_index;
}

/// `CLpc_Analysis(signal, signal_size, lpcCoeff_m, lpcCoeff_e, order, filtState,
/// filtStateIndex)`: FIR filtering of `signal` in place with the LPC coefficients (the residual
/// of the analysis filter).
///
/// `filt_state` holds at least `order` values; `filt_state_index` is the state index storage
/// (`NULL` in C: start at index 0 and do not store the index). `lpc_coeff_e + 1 >= 0`.
pub fn clpc_analysis(
    signal: &mut [FixpDbl],
    signal_size: i32,
    lpc_coeff_m: &[FixpLpc],
    lpc_coeff_e: i32,
    order: i32,
    filt_state: &mut [FixpDbl],
    filt_state_index: Option<&mut i32>,
) {
    let shift = lpc_coeff_e.wrapping_add(1); // +1, because fMultDiv2

    if order <= 0 {
        return;
    }
    let mut state_index = match &filt_state_index {
        Some(i) => **i,
        None => 0,
    };
    let ord = order as usize;

    // keep filter coefficients twice and save memory copy operation in modulo state buffer
    let mut coeff = [0 as FixpLpc; 2 * LPC_MAX_ORDER];
    coeff[..ord].copy_from_slice(&lpc_coeff_m[..ord]);
    coeff[ord..2 * ord].copy_from_slice(&lpc_coeff_m[..ord]);

    // # Analysis filter, obtain residual.
    // for k = 0:BL-1
    //         err(i-BL+k) = a * inputSignal(i-BL+k:-1:i-BL-M+k);
    // endfor

    fdk_assert!(shift >= 0, "FDK_ASSERT(shift >= 0)");

    for j in 0..signal_size.max(0) as usize {
        let p_coeff = &coeff[(order - state_index) as usize..];

        let mut tmp = signal[j].wrapping_shr(shift as u32);
        for i in 0..ord {
            tmp = f_mult_add_div2(tmp, p_coeff[i], filt_state[i]);
        }

        state_index = if state_index - 1 < 0 { state_index - 1 + order } else { state_index - 1 };
        filt_state[state_index as usize] = signal[j];

        signal[j] = tmp.wrapping_shl(shift as u32);
    }

    if let Some(i) = filt_state_index {
        *i = state_index;
    }
}

/// `CLpc_ParcorToLpc(reflCoeff, LpcCoeff, numOfCoeff, workBuffer)` with `FIXP_LPC_TNS`
/// (`FIXP_DBL`) coefficients (for the `LPC_SYNTHESIS_IIR` version): converts ParCor
/// coefficients to LPC coefficients.
///
/// `work_buffer` holds at least `max(num_of_coeff, 1)` values. Returns the exponent of the LPC
/// coefficients.
pub fn clpc_parcor_to_lpc(
    refl_coeff: &[FixpLpcTns],
    lpc_coeff: &mut [FixpLpcTns],
    num_of_coeff: i32,
    work_buffer: &mut [FixpDbl],
) -> i32 {
    let par2_lpc_shift_val = 6; // 6 should be enough, bec. max(numOfCoeff) = 20
    let mut max_val: FixpDbl = 0;
    let n = num_of_coeff.max(0) as usize;

    work_buffer[0] = fx_lpc_tns2fx_dbl(refl_coeff[0]) >> par2_lpc_shift_val;
    for i in 1..n {
        let mut j = 0;
        while j < i / 2 {
            let tmp1 = work_buffer[j];
            let tmp2 = work_buffer[i - 1 - j];
            work_buffer[j] = work_buffer[j].wrapping_add(f_mult(refl_coeff[i], tmp2));
            work_buffer[i - 1 - j] =
                work_buffer[i - 1 - j].wrapping_add(f_mult(refl_coeff[i], tmp1));
            j += 1;
        }
        if i & 1 != 0 {
            work_buffer[j] = work_buffer[j].wrapping_add(f_mult(refl_coeff[i], work_buffer[j]));
        }

        work_buffer[i] = fx_lpc_tns2fx_dbl(refl_coeff[i]) >> par2_lpc_shift_val;
    }

    // calculate exponent
    for i in 0..n {
        max_val = f_max(max_val, f_abs(work_buffer[i]));
    }

    let shiftval = f_min(f_norm(max_val), par2_lpc_shift_val);

    for i in 0..n {
        lpc_coeff[i] = fx_dbl2fx_lpc_tns(work_buffer[i].wrapping_shl(shiftval as u32));
    }

    par2_lpc_shift_val - shiftval
}

/// `CLpc_ParcorToLpc(reflCoeff, LpcCoeff, numOfCoeff, workBuffer)` with `FIXP_LPC` (`FIXP_SGL`)
/// coefficients (the default version). See [`clpc_parcor_to_lpc`].
pub fn clpc_parcor_to_lpc_sgl(
    refl_coeff: &[FixpLpc],
    lpc_coeff: &mut [FixpLpc],
    num_of_coeff: i32,
    work_buffer: &mut [FixpDbl],
) -> i32 {
    let par2_lpc_shift_val = 6; // 6 should be enough, bec. max(numOfCoeff) = 20
    let mut max_val: FixpDbl = 0;
    let n = num_of_coeff.max(0) as usize;

    work_buffer[0] = fx_lpc2fx_dbl(refl_coeff[0]) >> par2_lpc_shift_val;
    for i in 1..n {
        let mut j = 0;
        while j < i / 2 {
            let tmp1 = work_buffer[j];
            let tmp2 = work_buffer[i - 1 - j];
            work_buffer[j] = work_buffer[j].wrapping_add(f_mult(refl_coeff[i], tmp2));
            work_buffer[i - 1 - j] =
                work_buffer[i - 1 - j].wrapping_add(f_mult(refl_coeff[i], tmp1));
            j += 1;
        }
        if i & 1 != 0 {
            work_buffer[j] = work_buffer[j].wrapping_add(f_mult(refl_coeff[i], work_buffer[j]));
        }

        work_buffer[i] = fx_lpc2fx_dbl(refl_coeff[i]) >> par2_lpc_shift_val;
    }

    // calculate exponent
    for i in 0..n {
        max_val = f_max(max_val, f_abs(work_buffer[i]));
    }

    let shiftval = f_min(f_norm(max_val), par2_lpc_shift_val);

    for i in 0..n {
        lpc_coeff[i] = fx_dbl2fx_lpc(work_buffer[i].wrapping_shl(shiftval as u32));
    }

    par2_lpc_shift_val - shiftval
}

/// `CLpc_AutoToParcor(acorr, acorr_e, reflCoeff, numOfCoeff, pPredictionGain_m,
/// pPredictionGain_e)`: calculates ParCor (Partial autoCorrelation, reflection) coefficients
/// from autocorrelation coefficients using the Schur algorithm (instead of Levinson Durbin).
///
/// `acorr` holds `num_of_coeff + 1` autocorrelation coefficients (modified). `refl_coeff`
/// receives the `num_of_coeff` reflection coefficients; the first coefficient, which is always
/// 1.0, is omitted. `p_prediction_gain` receives the prediction gain mantissa and exponent if
/// given (`pPredictionGain_m != NULL` in C).
pub fn clpc_auto_to_parcor(
    acorr: &mut [FixpDbl],
    _acorr_e: i32,
    refl_coeff: &mut [FixpLpc],
    num_of_coeff: i32,
    p_prediction_gain: Option<(&mut FixpDbl, &mut i32)>,
) {
    let n = num_of_coeff.max(0) as usize;
    let mut parcor_work_buffer = [0 as FixpDbl; LPC_MAX_ORDER];
    // workBuffer points into parcorWorkBuffer
    let mut wb = 0usize;
    let auto_corr_0 = acorr[0];

    refl_coeff[..n].fill(0);

    if auto_corr_0 == fl2fxconst_dbl(0.0) {
        if let Some((gain_m, gain_e)) = p_prediction_gain {
            *gain_m = fl2fxconst_dbl(0.5);
            *gain_e = 1;
        }
        return;
    }

    parcor_work_buffer[..n].copy_from_slice(&acorr[1..n + 1]);
    for i in 0..n {
        let sign = parcor_work_buffer[wb] >> (DFRACT_BITS - 1);
        let tmp = parcor_work_buffer[wb] ^ sign;

        // Check preconditions for division function: num<=denum
        // For 1st iteration acorr[0] cannot be 0, it is checked before loop
        // Due to exor operation with "sign", num(=tmp) is greater/equal 0
        if acorr[0] < tmp {
            break;
        }

        // tmp = div(num, denum, 16)
        let tmp = schur_div(tmp, acorr[0], FRACT_BITS) ^ !sign;

        refl_coeff[i] = fx_dbl2fx_lpc(tmp);

        for j in (0..n - i).rev() {
            let accu1 = f_mult(tmp, acorr[j]);
            let accu2 = f_mult(tmp, parcor_work_buffer[wb + j]);
            parcor_work_buffer[wb + j] = parcor_work_buffer[wb + j].wrapping_add(accu1);
            acorr[j] = acorr[j].wrapping_add(accu2);
        }
        // Check preconditions for division function: denum (=acorr[0]) > 0
        if acorr[0] == 0 {
            break;
        }

        wb += 1;
    }

    if let Some((gain_m, gain_e)) = p_prediction_gain {
        if acorr[0] > 0 {
            // prediction gain = signal power / error (residual) power
            let mut scale = 0;
            *gain_m = f_div_norm_signed(auto_corr_0, acorr[0], &mut scale);
            *gain_e = scale;
        } else {
            *gain_m = 0;
            *gain_e = 0;
        }
    }
}
