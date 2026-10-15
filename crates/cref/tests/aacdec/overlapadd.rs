// Differential tests of fdk_aac_rs_aacdec::overlapadd against the C++ reference.

use fdk_aac_rs_aacdec::overlapadd::*;
use fdk_aac_rs_cref::aacdec_overlapadd as c;

#[test]
fn constants() {
    assert_eq!(OverlapBufferSize, unsafe { c::cref_ola_overlap_buffer_size() });
    assert_eq!(std::mem::size_of::<Spectrum>() / 4, unsafe { c::cref_ola_spectrum_len() } as usize);
}

/// The offsets of `SPEC_LONG`, `SPEC` and `SPEC_TCX` for the window/frame indices and granule
/// lengths of the decoder.
#[test]
fn spec_macros() {
    assert_eq!(spec_long(), unsafe { c::cref_ola_spec_long() } as usize);
    for gl in [0, 1, 32, 60, 64, 96, 120, 128, 192, 240, 256] {
        for w in 0..8 {
            assert_eq!(spec(w, gl), unsafe { c::cref_ola_spec(w, gl) } as usize, "SPEC({w}, {gl})");
        }
        for f in 0..4 {
            for fb in [0, 1, 2, -1] {
                assert_eq!(
                    spec_tcx(f, gl, fb),
                    unsafe { c::cref_ola_spec_tcx(f, gl, fb) } as usize,
                    "SPEC_TCX({f}, {gl}, {fb})"
                );
            }
        }
    }
}

#[test]
fn spec_slices() {
    let mut s = [0i32; 16];
    s[5] = 7;
    assert_eq!(spec_slice(&s, 5)[0], 7);
    assert_eq!(spec_slice(&s, 16).len(), 0);
    assert_eq!(spec_slice(&s, 17).len(), 0);
    spec_slice_mut(&mut s, 3)[0] = 9;
    assert_eq!(s[3], 9);
    assert_eq!(spec_slice_mut(&mut s, 100).len(), 0);
}
