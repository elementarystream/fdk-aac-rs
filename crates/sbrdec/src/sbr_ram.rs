// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/sbr_ram.h, libSBRdec/src/sbr_ram.cpp.

//! Port of `sbr_ram.h` and `sbr_ram.cpp`: the memory of the SBR decoder.
//!
//! # Allocation
//!
//! `sbr_ram.cpp` defines the allocators of the decoder instance, its elements and channels, and
//! the PS decoder (`GetRam_SbrDecoder`, `GetRam_SbrDecElement`, `GetRam_SbrDecChannel`,
//! `GetRam_ps_dec`). In the reference build they are `FDKcalloc` calls, so every new structure is
//! zero-initialized; `FreeRam_*` frees it. The port allocates zeroed boxes in the same places
//! (`Box<T>` with `T::default()` = the cleared structure) and drops them where the C code frees
//! them. The structures `SBR_DECODER_ELEMENT` and `struct SBR_DECODER_INSTANCE` themselves are
//! defined in [`crate::sbrdecoder`].
//!
//! # Memory layout of the plain data structures
//!
//! The plain data structures of the decoder (frame and header data, envelope calculator and
//! transposer states, PVC and DRC states, PS bitstream data) are `#[repr(C)]` with the memory
//! layout of the reference build (LP64, little endian): every padding byte of the C structure is
//! an explicit `_pad*` field and every pointer an explicit `u64` placeholder (always 0 in the
//! port, never read). `pod_struct!` checks at compile time that a structure has no implicit
//! padding, so every byte is initialized and a value can be viewed as bytes ([`Pod`],
//! [`pod_bytes`]). The decoder itself does not depend on the layout; the differential tests use it
//! to copy whole states between the reference and the port and to compare them byte by byte.

/// A plain-old-data type: `#[repr(C)]`, `Copy`, without implicit padding and valid for every bit
/// pattern, so that all bytes of a value are initialized.
///
/// # Safety
/// Implement only through `pod_struct!`, which checks the absence of implicit padding.
pub unsafe trait Pod: Copy + 'static {}

// SAFETY: primitive integers have no padding and are valid for every bit pattern.
unsafe impl Pod for u8 {}
// SAFETY: as above.
unsafe impl Pod for i8 {}
// SAFETY: as above.
unsafe impl Pod for u16 {}
// SAFETY: as above.
unsafe impl Pod for i16 {}
// SAFETY: as above.
unsafe impl Pod for u32 {}
// SAFETY: as above.
unsafe impl Pod for i32 {}
// SAFETY: as above.
unsafe impl Pod for u64 {}
// SAFETY: as above.
unsafe impl Pod for isize {}
// SAFETY: as above.
unsafe impl Pod for usize {}
// SAFETY: arrays of Pod values have no padding between the elements.
unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}

/// The bytes of a [`Pod`] value (the memory image of the corresponding C structure).
#[inline]
pub fn pod_bytes<T: Pod>(v: &T) -> &[u8] {
    // SAFETY: `T: Pod` has no uninitialized bytes; the slice covers exactly the value.
    unsafe { core::slice::from_raw_parts(v as *const T as *const u8, core::mem::size_of::<T>()) }
}

/// The bytes of a [`Pod`] value, writable (every bit pattern is a valid value).
#[inline]
pub fn pod_bytes_mut<T: Pod>(v: &mut T) -> &mut [u8] {
    // SAFETY: `T: Pod` has no uninitialized bytes and is valid for every bit pattern; the slice
    // covers exactly the value.
    unsafe { core::slice::from_raw_parts_mut(v as *mut T as *mut u8, core::mem::size_of::<T>()) }
}

/// Defines a `#[repr(C)]` structure that implements [`Pod`], with a compile time check that the
/// sum of the field sizes is the size of the structure (no implicit padding), a `ZERO` constant
/// (the cleared C structure) and `Default`.
macro_rules! pod_struct {
    (
        $(#[$m:meta])*
        pub struct $name:ident {
            $( $(#[$fm:meta])* pub $f:ident : $t:ty ),* $(,)?
        }
    ) => {
        $(#[$m])*
        #[repr(C)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name {
            $( $(#[$fm])* pub $f: $t ),*
        }

        const _: () = assert!(
            core::mem::size_of::<$name>() == 0 $( + core::mem::size_of::<$t>() )*,
            "implicit padding"
        );

        const _: () = {
            const fn assert_pod<T: $crate::sbr_ram::Pod>() {}
            $( assert_pod::<$t>(); )*
        };

        // SAFETY: repr(C), no implicit padding (checked above), all fields are Pod (checked
        // above).
        unsafe impl $crate::sbr_ram::Pod for $name {}

        impl $name {
            /// The cleared structure (`FDKmemclear`, `FDKcalloc`).
            // SAFETY: all fields are Pod, for which all-zero bytes are a valid value.
            pub const ZERO: Self = unsafe { core::mem::zeroed() };
        }

        impl Default for $name {
            fn default() -> Self {
                Self::ZERO
            }
        }
    };
}
pub(crate) use pod_struct;

/// `SBRDEC_MAX_CH_PER_ELEMENT`
pub const SBRDEC_MAX_CH_PER_ELEMENT: usize = 2;

/// `FRAME_OK`
pub const FRAME_OK: u8 = 0;
/// `FRAME_ERROR`
pub const FRAME_ERROR: u8 = 1;
/// `FRAME_ERROR_ALLSLOTS`
pub const FRAME_ERROR_ALLSLOTS: u8 = 2;
