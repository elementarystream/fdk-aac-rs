// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/scramble.h, libFDK/include/arm/scramble_arm.h.

//! Port of `scramble.h`: bit reversal of interleaved complex data.
//!
//! `arm/scramble_arm.h` holds an ARMv5-v7 assembly version that is compiled only if
//! `FUNCTION_scramble` is defined before the header is included, which no reference build does:
//! both the `aarch64` and the `x86` translation units compile the generic C function.

use crate::common_fix::FixpDbl;

/// `scramble(x, length)`: bit reversal of the `length` complex values (interleaved real and
/// imaginary parts) of `x`.
///
/// `length` must be a power of two; `x` must hold `2 * length` values.
pub fn scramble(x: &mut [FixpDbl], length: i32) {
    let mut j: i32 = 0;
    let mut m: i32 = 1;
    while m < length - 1 {
        // for (k = length >> 1; (!((j ^= k) & k)); k >>= 1);
        // With a power-of-two `length` the loop ends before `k` reaches 0; for other lengths
        // (never used by the library) the C loop would not terminate, the port stops at 0.
        let mut k = length >> 1;
        loop {
            j ^= k;
            if (j & k) != 0 || k == 0 {
                break;
            }
            k >>= 1;
        }

        if j > m {
            let (m2, j2) = (2 * m as usize, 2 * j as usize);
            x.swap(m2, j2);
            x.swap(m2 + 1, j2 + 1);
        }
        m += 1;
    }
}
