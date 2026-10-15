// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcDec_types.h.

//! Port of `drcDec_types.h`: data structures corresponding to the static and dynamic
//! DRC/loudness payload as defined in section 7 of MPEG-D DRC, ISO/IEC 23003-4.
//!
//! # Memory layout
//!
//! The structures have the memory layout of the C structures of the reference build (LP64,
//! little endian): `#[repr(C)]`, every padding byte of the C structure as an explicit `_pad*`
//! field (always 0), and the C unions as Rust unions. `pod_struct!` checks at compile time that
//! a structure has no implicit padding, so every byte of a value is initialized and the value can
//! be viewed as bytes ([`Pod`], [`pod_bytes`]). This is needed because
//!
//! - the reader compares structures with `FDKmemcmp` (`diff`), which compares all bytes,
//!   including the union members that are not in use; the derived `PartialEq` is the same
//!   comparison here;
//! - some indices past the end of an array reach the following fields of the C structure, which
//!   the port reproduces from the bytes of the structure (C layout);
//! - the differential tests compare whole structures with the C structures byte by byte.
//!
//! The padding bytes are 0 in the reference too: all these structures are allocated with
//! `FDKcalloc` or cleared with `FDKmemclear` before they are filled, and struct assignments copy
//! the (zero) padding.

use fdk_aac_rs_fdk::common_fix::{FixpDbl, FixpSgl};

/// A plain-old-data type: `#[repr(C)]`, `Copy`, without implicit padding and valid for every bit
/// pattern, so that all bytes of a value are initialized.
///
/// # Safety
/// Implement only through `pod_struct!` and `pod_union!`, which check the absence of implicit
/// padding and that every field is `Pod`. The union fields are private to the crate, so unions
/// are created from their `ZERO` value only.
pub unsafe trait Pod: Copy + 'static {}

unsafe impl Pod for u8 {}
unsafe impl Pod for i8 {}
unsafe impl Pod for u16 {}
unsafe impl Pod for i16 {}
unsafe impl Pod for u32 {}
unsafe impl Pod for i32 {}
unsafe impl Pod for i64 {}
unsafe impl Pod for isize {}
unsafe impl Pod for usize {}
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

/// Reads the `u8` at byte offset `off` of the memory image of `v`, `None` outside of it.
#[inline]
pub fn pod_read_u8<T: Pod>(v: &T, off: usize) -> Option<u8> {
    pod_bytes(v).get(off).copied()
}

/// Reads the little endian `i16` at byte offset `off` of the memory image of `v`, `None` if it
/// is not completely inside.
#[inline]
pub fn pod_read_i16<T: Pod>(v: &T, off: usize) -> Option<i16> {
    let b = pod_bytes(v);
    Some(i16::from_le_bytes([*b.get(off)?, *b.get(off + 1)?]))
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
            const fn assert_pod<T: $crate::drc_dec_types::Pod>() {}
            $( assert_pod::<$t>(); )*
        };

        // SAFETY: repr(C), no implicit padding (checked above), all fields are Pod (checked
        // above).
        unsafe impl $crate::drc_dec_types::Pod for $name {}

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

/// Defines a `#[repr(C)]` union of [`Pod`] members that implements [`Pod`], `PartialEq` and
/// `Debug` on its bytes (like `FDKmemcmp`), with a compile time check that its size is the size of
/// the given largest member, and a `ZERO` constant.
macro_rules! pod_union {
    (
        $(#[$m:meta])*
        pub union $name:ident [largest = $lt:ty] {
            $( $(#[$fm:meta])* pub $f:ident : $t:ty ),* $(,)?
        }
    ) => {
        $(#[$m])*
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub union $name {
            $( $(#[$fm])* pub(crate) $f: $t ),*
        }

        const _: () = {
            const fn assert_pod<T: $crate::drc_dec_types::Pod>() {}
            $( assert_pod::<$t>(); )*
        };

        const _: () = assert!(
            core::mem::size_of::<$name>() == core::mem::size_of::<$lt>(),
            "union larger than its largest member"
        );

        // SAFETY: repr(C), the size is the size of a Pod member without padding; values are only
        // created from ZERO (all bytes initialized) and written through Pod members.
        unsafe impl $crate::drc_dec_types::Pod for $name {}

        impl $name {
            /// The cleared union.
            // SAFETY: all members are Pod, for which all-zero bytes are a valid value.
            pub const ZERO: Self = unsafe { core::mem::zeroed() };
        }

        impl Default for $name {
            fn default() -> Self {
                Self::ZERO
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                $crate::drc_dec_types::pod_bytes(self) == $crate::drc_dec_types::pod_bytes(other)
            }
        }

        impl Eq for $name {}

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_tuple(stringify!($name))
                    .field(&$crate::drc_dec_types::pod_bytes(self))
                    .finish()
            }
        }
    };
}
pub(crate) use pod_union;

/**************/
/* uniDrcGain */
/**************/

pod_struct! {
    /// `GAIN_NODE`
    pub struct GainNode {
        /// e = 7
        pub gain_db: FixpSgl,
        pub time: i16,
    }
}

pod_struct! {
    /// `UNI_DRC_GAIN_EXTENSION`: uniDrcGainExtension() (Table 56)
    pub struct UniDrcGainExtension {
        pub uni_drc_gain_ext_type: [u8; 8],
        pub ext_bit_size: [u32; 8 - 1],
    }
}

pod_struct! {
    /// `UNI_DRC_GAIN`: uniDrcGain() (Table 55)
    pub struct UniDrcGain {
        /// unsaturated value, i.e. as provided in bitstream
        pub n_nodes: [u8; 12],
        pub gain_node: [[GainNode; 16]; 12],

        pub uni_drc_gain_ext_present: u8,
        pub _pad0: [u8; 3],
        pub uni_drc_gain_extension: UniDrcGainExtension,

        /* derived data */
        pub status: u8,
        pub _pad1: [u8; 3],
    }
}

/****************/
/* uniDrcConfig */
/****************/

/// `EFFECT_BIT`
pub type EffectBit = i32;
/// `EB_NIGHT`
pub const EB_NIGHT: EffectBit = 0x0001;
/// `EB_NOISY`
pub const EB_NOISY: EffectBit = 0x0002;
/// `EB_LIMITED`
pub const EB_LIMITED: EffectBit = 0x0004;
/// `EB_LOWLEVEL`
pub const EB_LOWLEVEL: EffectBit = 0x0008;
/// `EB_DIALOG`
pub const EB_DIALOG: EffectBit = 0x0010;
/// `EB_GENERAL_COMPR`
pub const EB_GENERAL_COMPR: EffectBit = 0x0020;
/// `EB_EXPAND`
pub const EB_EXPAND: EffectBit = 0x0040;
/// `EB_ARTISTIC`
pub const EB_ARTISTIC: EffectBit = 0x0080;
/// `EB_CLIPPING`
pub const EB_CLIPPING: EffectBit = 0x0100;
/// `EB_FADE`
pub const EB_FADE: EffectBit = 0x0200;
/// `EB_DUCK_OTHER`
pub const EB_DUCK_OTHER: EffectBit = 0x0400;
/// `EB_DUCK_SELF`
pub const EB_DUCK_SELF: EffectBit = 0x0800;

/// `GAIN_CODING_PROFILE`
pub type GainCodingProfile = i32;
/// `GCP_REGULAR`
pub const GCP_REGULAR: GainCodingProfile = 0;
/// `GCP_FADING`
pub const GCP_FADING: GainCodingProfile = 1;
/// `GCP_CLIPPING_DUCKING`
pub const GCP_CLIPPING_DUCKING: GainCodingProfile = 2;
/// `GCP_CONSTANT`
pub const GCP_CONSTANT: GainCodingProfile = 3;

/// `GAIN_INTERPOLATION_TYPE`
pub type GainInterpolationType = i32;
/// `GIT_SPLINE`
pub const GIT_SPLINE: GainInterpolationType = 0;
/// `GIT_LINEAR`
pub const GIT_LINEAR: GainInterpolationType = 1;

/// `CHARACTERISTIC_SIDE`
pub type CharacteristicSide = i32;
/// `CS_LEFT`
pub const CS_LEFT: CharacteristicSide = 0;
/// `CS_RIGHT`
pub const CS_RIGHT: CharacteristicSide = 1;

/// `CHARACTERISTIC_FORMAT`
pub type CharacteristicFormat = i32;
/// `CF_SIGMOID`
pub const CF_SIGMOID: CharacteristicFormat = 0;
/// `CF_NODES`
pub const CF_NODES: CharacteristicFormat = 1;

/// `EQ_SUBBAND_GAIN_FORMAT`
pub type EqSubbandGainFormat = i32;
/// `GF_QMF32`
pub const GF_QMF32: EqSubbandGainFormat = 0x1;
/// `GF_QMFHYBRID39`
pub const GF_QMFHYBRID39: EqSubbandGainFormat = 0x2;
/// `GF_QMF64`
pub const GF_QMF64: EqSubbandGainFormat = 0x3;
/// `GF_QMFHYBRID71`
pub const GF_QMFHYBRID71: EqSubbandGainFormat = 0x4;
/// `GF_QMF128`
pub const GF_QMF128: EqSubbandGainFormat = 0x5;
/// `GF_QMFHYBRID135`
pub const GF_QMFHYBRID135: EqSubbandGainFormat = 0x6;
/// `GF_UNIFORM`
pub const GF_UNIFORM: EqSubbandGainFormat = 0x7;

pod_struct! {
    /// `DUCKING_MODIFICATION`
    pub struct DuckingModification {
        pub ducking_scaling_present: u8,
        pub _pad0: u8,
        /// e = 2
        pub ducking_scaling: FixpSgl,
    }
}

pod_struct! {
    /// `GAIN_MODIFICATION`
    pub struct GainModification {
        pub target_characteristic_left_present: u8,
        pub target_characteristic_left_index: u8,
        pub target_characteristic_right_present: u8,
        pub target_characteristic_right_index: u8,
        pub gain_scaling_present: u8,
        pub _pad0: u8,
        /// e = 2
        pub attenuation_scaling: FixpSgl,
        /// e = 2
        pub amplification_scaling: FixpSgl,
        pub gain_offset_present: u8,
        pub _pad1: u8,
        /// e = 4
        pub gain_offset: FixpSgl,
    }
}

pod_struct! {
    /// `BAND_BORDER`: union of `UCHAR crossoverFreqIndex` and `USHORT startSubBandIndex`, stored
    /// as the `USHORT` member; `crossoverFreqIndex` is its low byte (little endian).
    pub struct BandBorder {
        pub start_sub_band_index: u16,
    }
}

impl BandBorder {
    /// `crossoverFreqIndex`
    #[inline]
    pub fn crossover_freq_index(&self) -> u8 {
        self.start_sub_band_index as u8
    }

    /// Assignment to `crossoverFreqIndex` (writes the low byte only).
    #[inline]
    pub fn set_crossover_freq_index(&mut self, v: u8) {
        self.start_sub_band_index = (self.start_sub_band_index & 0xFF00) | v as u16;
    }
}

pod_struct! {
    /// `CUSTOM_INDEX`
    pub struct CustomIndex {
        pub left: u8,
        pub right: u8,
    }
}

pod_struct! {
    /// `DRC_CHARACTERISTIC`. The union of `UCHAR cicpIndex` and `CUSTOM_INDEX custom` is stored as
    /// `cicp_index` (the byte shared by `cicpIndex` and `custom.left`) and `custom_right`.
    pub struct DrcCharacteristic {
        pub present: u8,
        pub is_cicp: u8,
        /// `cicpIndex`, shares its byte with `custom.left`
        pub cicp_index: u8,
        /// `custom.right`
        pub custom_right: u8,
    }
}

impl DrcCharacteristic {
    /// `custom.left`
    #[inline]
    pub fn custom_left(&self) -> u8 {
        self.cicp_index
    }

    /// Assignment to `custom.left`
    #[inline]
    pub fn set_custom_left(&mut self, v: u8) {
        self.cicp_index = v;
    }
}

pod_struct! {
    /// `GAIN_SET`
    pub struct GainSet {
        pub gain_coding_profile: u8,
        pub gain_interpolation_type: u8,
        pub full_frame: u8,
        pub time_alignment: u8,
        pub time_delta_min_present: u8,
        pub _pad0: u8,
        pub time_delta_min: u16,
        pub band_count: u8,
        pub drc_band_type: u8,
        pub gain_sequence_index: [u8; 4],
        pub drc_characteristic: [DrcCharacteristic; 4],
        pub band_border: [BandBorder; 4],
    }
}

pod_struct! {
    /// `CUSTOM_DRC_CHAR_SIGMOID`
    pub struct CustomDrcCharSigmoid {
        /// e = 6
        pub gain: FixpSgl,
        /// e = 2
        pub io_ratio: FixpSgl,
        /// e = 5
        pub exp: FixpSgl,
        pub flip_sign: u8,
        pub _pad0: u8,
    }
}

pod_struct! {
    /// `CUSTOM_DRC_CHAR_NODES`
    pub struct CustomDrcCharNodes {
        pub characteristic_node_count: u8,
        pub _pad0: u8,
        /// e = 7
        pub node_level: [FixpSgl; 4 + 1],
        /// e = 7
        pub node_gain: [FixpSgl; 4 + 1],
    }
}

pod_union! {
    /// `CUSTOM_DRC_CHAR` (`shouldBeUnion`, a union in the reference build)
    pub union CustomDrcChar [largest = CustomDrcCharNodes] {
        pub sigmoid: CustomDrcCharSigmoid,
        pub nodes: CustomDrcCharNodes,
    }
}

impl CustomDrcChar {
    /// `sigmoid`
    #[inline]
    pub fn sigmoid(&self) -> &CustomDrcCharSigmoid {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &self.sigmoid }
    }

    /// `sigmoid`
    #[inline]
    pub fn sigmoid_mut(&mut self) -> &mut CustomDrcCharSigmoid {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &mut self.sigmoid }
    }

    /// `nodes`
    #[inline]
    pub fn nodes(&self) -> &CustomDrcCharNodes {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &self.nodes }
    }

    /// `nodes`
    #[inline]
    pub fn nodes_mut(&mut self) -> &mut CustomDrcCharNodes {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &mut self.nodes }
    }
}

pod_struct! {
    /// `DRC_COEFFICIENTS_UNI_DRC`: drcCoefficientsUniDrc() (Table 67)
    pub struct DrcCoefficientsUniDrc {
        pub drc_location: u8,
        pub drc_frame_size_present: u8,
        pub drc_frame_size: u16,
        pub characteristic_left_count: u8,
        pub characteristic_left_format: [u8; 16],
        pub _pad0: u8,
        pub custom_characteristic_left: [CustomDrcChar; 16],
        pub characteristic_right_count: u8,
        pub characteristic_right_format: [u8; 16],
        pub _pad1: u8,
        pub custom_characteristic_right: [CustomDrcChar; 16],
        /// unsaturated value, i.e. as provided in bitstream
        pub gain_sequence_count: u8,
        /// saturated to 12
        pub gain_set_count: u8,
        pub gain_set: [GainSet; 12],
        /* derived data */
        pub gain_set_index_for_gain_sequence: [u8; 12],
    }
}

pod_union! {
    /// The union of `gainModificationForChannelGroup` and `duckingModificationForChannel` in
    /// `DRC_INSTRUCTIONS_UNI_DRC` (`shouldBeUnion`, a union in the reference build).
    pub union GainDuckingModification [largest = [[GainModification; 4]; 8]] {
        pub gain_modification_for_channel_group: [[GainModification; 4]; 8],
        pub ducking_modification_for_channel: [DuckingModification; 8],
    }
}

pod_struct! {
    /// `DRC_INSTRUCTIONS_UNI_DRC`: drcInstructionsUniDrc() (Table 72)
    pub struct DrcInstructionsUniDrc {
        pub drc_set_id: i8,
        pub drc_set_complexity_level: u8,
        pub drc_location: u8,
        pub drc_apply_to_downmix: u8,
        pub downmix_id_count: u8,
        pub downmix_id: [u8; 8],
        pub _pad0: u8,
        pub drc_set_effect: u16,
        pub limiter_peak_target_present: u8,
        pub _pad1: u8,
        /// e = 5
        pub limiter_peak_target: FixpSgl,
        pub drc_set_target_loudness_present: u8,
        pub drc_set_target_loudness_value_upper: i8,
        pub drc_set_target_loudness_value_lower: i8,
        pub depends_on_drc_set_present: u8,
        /// `dependsOnDrcSet`, union with `UCHAR noIndependentUse`
        pub depends_on_drc_set: i8,
        pub requires_eq: u8,
        /// `gainModificationForChannelGroup` / `duckingModificationForChannel`
        pub gain_ducking: GainDuckingModification,
        pub gain_set_index: [i8; 8],

        /* derived data */
        pub drc_channel_count: u8,
        pub n_drc_channel_groups: u8,
        pub gain_set_index_for_channel_group: [i8; 8],
    }
}

impl DrcInstructionsUniDrc {
    /// `noIndependentUse` (union with `dependsOnDrcSet`)
    #[inline]
    pub fn no_independent_use(&self) -> u8 {
        self.depends_on_drc_set as u8
    }

    /// Assignment to `noIndependentUse`
    #[inline]
    pub fn set_no_independent_use(&mut self, v: u8) {
        self.depends_on_drc_set = v as i8;
    }

    /// `gainModificationForChannelGroup`
    #[inline]
    pub fn gain_modification_for_channel_group(&self) -> &[[GainModification; 4]; 8] {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &self.gain_ducking.gain_modification_for_channel_group }
    }

    /// `gainModificationForChannelGroup`
    #[inline]
    pub fn gain_modification_for_channel_group_mut(&mut self) -> &mut [[GainModification; 4]; 8] {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &mut self.gain_ducking.gain_modification_for_channel_group }
    }

    /// `duckingModificationForChannel`
    #[inline]
    pub fn ducking_modification_for_channel(&self) -> &[DuckingModification; 8] {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &self.gain_ducking.ducking_modification_for_channel }
    }

    /// `duckingModificationForChannel`
    #[inline]
    pub fn ducking_modification_for_channel_mut(&mut self) -> &mut [DuckingModification; 8] {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &mut self.gain_ducking.ducking_modification_for_channel }
    }
}

pod_struct! {
    /// `CHANNEL_LAYOUT`: channelLayout() (Table 62)
    pub struct ChannelLayout {
        pub base_channel_count: u8,
        pub layout_signaling_present: u8,
        pub defined_layout: u8,
        pub speaker_position: [u8; 8],
    }
}

pod_struct! {
    /// `DOWNMIX_INSTRUCTIONS`: downmixInstructions() (Table 63)
    pub struct DownmixInstructions {
        pub downmix_id: u8,
        pub target_channel_count: u8,
        pub target_layout: u8,
        pub downmix_coefficients_present: u8,
        pub bs_downmix_offset: u8,
        pub _pad0: [u8; 3],
        /// e = 2
        pub downmix_coefficient: [FixpDbl; 8 * 8],
    }
}

pod_struct! {
    /// `UNI_DRC_CONFIG_EXTENSION`
    pub struct UniDrcConfigExtension {
        pub uni_drc_config_ext_type: [u8; 8],
        pub ext_bit_size: [u32; 8 - 1],
    }
}

pod_struct! {
    /// `UNI_DRC_CONFIG`: uniDrcConfig() (Table 57)
    pub struct UniDrcConfig {
        pub sample_rate_present: u8,
        pub _pad0: [u8; 3],
        pub sample_rate: u32,
        pub downmix_instructions_count_v0: u8,
        pub downmix_instructions_count_v1: u8,
        /// saturated to 6
        pub downmix_instructions_count: u8,
        pub drc_coefficients_uni_drc_count_v0: u8,
        pub drc_coefficients_uni_drc_count_v1: u8,
        /// saturated to 2
        pub drc_coefficients_uni_drc_count: u8,
        pub drc_instructions_uni_drc_count_v0: u8,
        pub drc_instructions_uni_drc_count_v1: u8,
        /// saturated to (12 + 1 + 6)
        pub drc_instructions_uni_drc_count: u8,
        pub channel_layout: ChannelLayout,
        pub downmix_instructions: [DownmixInstructions; 6],
        pub drc_coefficients_uni_drc: [DrcCoefficientsUniDrc; 2],
        pub drc_instructions_uni_drc: [DrcInstructionsUniDrc; 12 + 1 + 6],
        pub uni_drc_config_ext_present: u8,
        pub _pad1: [u8; 3],
        pub uni_drc_config_ext: UniDrcConfigExtension,

        /* derived data */
        pub drc_instructions_count_incl_virtual: u8,
        pub diff: u8,
        pub _pad2: [u8; 2],
    }
}

/*******************/
/* loudnessInfoSet */
/*******************/

/// `METHOD_DEFINITION`
pub type MethodDefinition = i32;
/// `MD_UNKNOWN_OTHER`
pub const MD_UNKNOWN_OTHER: MethodDefinition = 0;
/// `MD_PROGRAM_LOUDNESS`
pub const MD_PROGRAM_LOUDNESS: MethodDefinition = 1;
/// `MD_ANCHOR_LOUDNESS`
pub const MD_ANCHOR_LOUDNESS: MethodDefinition = 2;
/// `MD_MAX_OF_LOUDNESS_RANGE`
pub const MD_MAX_OF_LOUDNESS_RANGE: MethodDefinition = 3;
/// `MD_MOMENTARY_LOUDNESS_MAX`
pub const MD_MOMENTARY_LOUDNESS_MAX: MethodDefinition = 4;
/// `MD_SHORT_TERM_LOUDNESS_MAX`
pub const MD_SHORT_TERM_LOUDNESS_MAX: MethodDefinition = 5;
/// `MD_LOUDNESS_RANGE`
pub const MD_LOUDNESS_RANGE: MethodDefinition = 6;
/// `MD_MIXING_LEVEL`
pub const MD_MIXING_LEVEL: MethodDefinition = 7;
/// `MD_ROOM_TYPE`
pub const MD_ROOM_TYPE: MethodDefinition = 8;
/// `MD_SHORT_TERM_LOUDNESS`
pub const MD_SHORT_TERM_LOUDNESS: MethodDefinition = 9;

/// `MEASUREMENT_SYSTEM`
pub type MeasurementSystem = i32;
/// `MS_UNKNOWN_OTHER`
pub const MS_UNKNOWN_OTHER: MeasurementSystem = 0;
/// `MS_EBU_R_128`
pub const MS_EBU_R_128: MeasurementSystem = 1;
/// `MS_BS_1770_4`
pub const MS_BS_1770_4: MeasurementSystem = 2;
/// `MS_BS_1770_4_PRE_PROCESSING`
pub const MS_BS_1770_4_PRE_PROCESSING: MeasurementSystem = 3;
/// `MS_USER`
pub const MS_USER: MeasurementSystem = 4;
/// `MS_EXPERT_PANEL`
pub const MS_EXPERT_PANEL: MeasurementSystem = 5;
/// `MS_BS_1771_1`
pub const MS_BS_1771_1: MeasurementSystem = 6;
/// `MS_RESERVED_A`
pub const MS_RESERVED_A: MeasurementSystem = 7;
/// `MS_RESERVED_B`
pub const MS_RESERVED_B: MeasurementSystem = 8;
/// `MS_RESERVED_C`
pub const MS_RESERVED_C: MeasurementSystem = 9;
/// `MS_RESERVED_D`
pub const MS_RESERVED_D: MeasurementSystem = 10;
/// `MS_RESERVED_E`
pub const MS_RESERVED_E: MeasurementSystem = 11;

/// `RELIABILITY`
pub type Reliability = i32;
/// `R_UKNOWN`
pub const R_UKNOWN: Reliability = 0;
/// `R_UNVERIFIED`
pub const R_UNVERIFIED: Reliability = 1;
/// `R_CEILING`
pub const R_CEILING: Reliability = 2;
/// `R_ACCURATE`
pub const R_ACCURATE: Reliability = 3;

pod_struct! {
    /// `LOUDNESS_MEASUREMENT`
    pub struct LoudnessMeasurement {
        pub method_definition: u8,
        pub _pad0: [u8; 3],
        /// e = 7 for all methodDefinitions
        pub method_value: FixpDbl,
        pub measurement_system: u8,
        pub reliability: u8,
        pub _pad1: [u8; 2],
    }
}

pod_struct! {
    /// `LOUDNESS_INFO`: loudnessInfo() (Table 59)
    pub struct LoudnessInfo {
        pub drc_set_id: i8,
        pub eq_set_id: u8,
        pub downmix_id: u8,
        pub sample_peak_level_present: u8,
        /// e = 7
        pub sample_peak_level: FixpDbl,
        pub true_peak_level_present: u8,
        pub _pad0: [u8; 3],
        /// e = 7
        pub true_peak_level: FixpDbl,
        pub true_peak_level_measurement_system: u8,
        pub true_peak_level_reliability: u8,
        /// saturated to 8
        pub measurement_count: u8,
        pub _pad1: u8,
        pub loudness_measurement: [LoudnessMeasurement; 8],
    }
}

pod_struct! {
    /// `LOUDNESS_INFO_SET_EXTENSION`: loudnessInfoSetExtension() (Table 61)
    pub struct LoudnessInfoSetExtension {
        pub loudness_info_set_ext_type: [u8; 8],
        pub ext_bit_size: [u32; 8 - 1],
    }
}

pod_struct! {
    /// `LOUDNESS_INFO_SET`: loudnessInfoSet() (Table 58)
    pub struct LoudnessInfoSet {
        pub loudness_info_album_count_v0: u8,
        pub loudness_info_album_count_v1: u8,
        /// saturated to 12
        pub loudness_info_album_count: u8,
        pub loudness_info_count_v0: u8,
        pub loudness_info_count_v1: u8,
        /// saturated to 12
        pub loudness_info_count: u8,
        pub _pad0: [u8; 2],
        pub loudness_info_album: [LoudnessInfo; 12],
        pub loudness_info: [LoudnessInfo; 12],
        pub loudness_info_set_ext_present: u8,
        pub _pad1: [u8; 3],
        pub loudness_info_set_ext: LoudnessInfoSetExtension,
        /* derived data */
        pub diff: u8,
        pub _pad2: [u8; 3],
    }
}

/// Byte offsets of fields in the C structures, for the reads that the reference makes past the
/// end of an array (see the module documentation).
pub mod offsets {
    use super::*;
    use core::mem::{offset_of, size_of};

    /// `offsetof(UNI_DRC_CONFIG, drcCoefficientsUniDrc[c])`
    pub const fn drc_coefficients_uni_drc(c: usize) -> usize {
        offset_of!(UniDrcConfig, drc_coefficients_uni_drc) + c * size_of::<DrcCoefficientsUniDrc>()
    }

    /// `offsetof(DRC_COEFFICIENTS_UNI_DRC, gainSet[s].bandCount)`
    pub const fn gain_set_band_count(s: usize) -> usize {
        offset_of!(DrcCoefficientsUniDrc, gain_set)
            + s * size_of::<GainSet>()
            + offset_of!(GainSet, band_count)
    }

    /// `offsetof(UNI_DRC_GAIN, nNodes[s])`
    pub const fn uni_drc_gain_n_nodes(s: usize) -> usize {
        offset_of!(UniDrcGain, n_nodes) + s
    }

    /// `offsetof(UNI_DRC_GAIN, gainNode[s][i])`
    pub const fn uni_drc_gain_gain_node(s: usize, i: usize) -> usize {
        offset_of!(UniDrcGain, gain_node) + (s * 16 + i) * size_of::<GainNode>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    /// Sizes and offsets of the C structures of the reference build (LP64).
    #[test]
    fn layout() {
        assert_eq!(size_of::<UniDrcGain>(), 824);
        assert_eq!(offset_of!(UniDrcGain, uni_drc_gain_extension), 784);
        assert_eq!(offset_of!(UniDrcGain, status), 820);
        assert_eq!(size_of::<GainModification>(), 14);
        assert_eq!(size_of::<GainSet>(), 38);
        assert_eq!(offset_of!(GainSet, band_border), 30);
        assert_eq!(size_of::<CustomDrcChar>(), 22);
        assert_eq!(size_of::<DrcCoefficientsUniDrc>(), 1214);
        assert_eq!(offset_of!(DrcCoefficientsUniDrc, custom_characteristic_left), 22);
        assert_eq!(offset_of!(DrcCoefficientsUniDrc, custom_characteristic_right), 392);
        assert_eq!(offset_of!(DrcCoefficientsUniDrc, gain_set), 746);
        assert_eq!(size_of::<DrcInstructionsUniDrc>(), 492);
        assert_eq!(offset_of!(DrcInstructionsUniDrc, drc_set_effect), 14);
        assert_eq!(offset_of!(DrcInstructionsUniDrc, gain_ducking), 26);
        assert_eq!(offset_of!(DrcInstructionsUniDrc, gain_set_index), 474);
        assert_eq!(size_of::<ChannelLayout>(), 11);
        assert_eq!(size_of::<DownmixInstructions>(), 264);
        assert_eq!(size_of::<UniDrcConfig>(), 13432);
        assert_eq!(offset_of!(UniDrcConfig, downmix_instructions), 28);
        assert_eq!(offset_of!(UniDrcConfig, drc_coefficients_uni_drc), 1612);
        assert_eq!(offset_of!(UniDrcConfig, drc_instructions_uni_drc), 4040);
        assert_eq!(offset_of!(UniDrcConfig, uni_drc_config_ext), 13392);
        assert_eq!(offset_of!(UniDrcConfig, diff), 13429);
        assert_eq!(size_of::<LoudnessMeasurement>(), 12);
        assert_eq!(size_of::<LoudnessInfo>(), 116);
        assert_eq!(size_of::<LoudnessInfoSet>(), 2836);
        assert_eq!(offset_of!(LoudnessInfoSet, loudness_info), 1400);
        assert_eq!(offset_of!(LoudnessInfoSet, diff), 2832);
    }
}
