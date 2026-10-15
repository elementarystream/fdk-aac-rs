// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/usacdec_rom.h, libAACdec/src/usacdec_rom.cpp.

//! Port of `usacdec_rom.h`: the tables of the USAC LPD decoder (RE8 lattice quantiser, LPC,
//! ACELP gains, bass post filter, FAC windows).
//!
//! The tables are generated from the C++ sources by `usacdec_rom/gen_tables.py` with
//! `scripts/dump_table.py` ([`tables`]). The preprocessed `usacdec_rom.cpp` of the `aarch64` and
//! `x86` builds are identical.

#[rustfmt::skip]
pub mod tables;

pub use tables::*;

/* RE8 lattice quantiser constants */
/// `NB_SPHERE`
pub const NB_SPHERE: i32 = 32;
/// `NB_LEADER`
pub const NB_LEADER: i32 = 37;
/// `NB_LDSIGN`
pub const NB_LDSIGN: i32 = 226;
/// `NB_LDQ3`
pub const NB_LDQ3: i32 = 9;
/// `NB_LDQ4`
pub const NB_LDQ4: i32 = 28;

/// `LSF_SCALE`
pub const LSF_SCALE: i32 = 13;

/* ACELP tables */
/// `SF_QUA_GAIN7B`
pub const SF_QUA_GAIN7B: i32 = 4;

/// `L_FILT`: delay of up-sampling filter (bass post filter); also defined in
/// `usacdec_const.h` with the same value.
pub use crate::usacdec_const::L_FILT;
