// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/aacEnc_ram.h, libAACenc/src/aacEnc_ram.cpp.

//! Port of `aacEnc_ram.h`: memory layout of the AAC encoder.
//!
//! The static memory pools of `aacEnc_ram.cpp` (`C_ALLOC_MEM`, `C_ALLOC_MEM2`, `C_AALLOC_MEM`)
//! become owned allocations of the structures that use them:
//!
//! | C | Port |
//! |---|---|
//! | `Ram_aacEnc_PsyInternal`, `Ram_aacEnc_PsyElement`, `Ram_aacEnc_PsyStatic`, `Ram_aacEnc_PsyInputBuffer` | [`PsyInternal`] and its vectors |
//! | `GetRam_aacEnc_PsyDynamic` (dynamic RAM, `P_BUF_1`) | `PsyInternal::psy_dynamic` |
//! | `Ram_aacEnc_PsyOut`, `Ram_aacEnc_PsyOutElements`, `Ram_aacEnc_PsyOutChannel` | [`PsyOut`] |
//! | `Ram_aacEnc_QCstate`, `Ram_aacEnc_AdjustThreshold`, `Ram_aacEnc_AdjThrStateElement`, `Ram_aacEnc_ElementBits`, `Ram_aacEnc_BitCntrState` | [`QcState`] |
//! | `GetRam_aacEnc_BitLookUp`, `GetRam_aacEnc_MergeGainLookUp` (dynamic RAM, `P_BUF_1`) | `BitcntrState` buffers |
//! | `Ram_aacEnc_QCout`, `Ram_aacEnc_QCelement`, `GetRam_aacEnc_QCchannel` (dynamic RAM, `P_BUF_0`) | [`QcOut`] |
//! | `dynMem_Ah_Flag`, `dynMem_Thr_Exp`, `dynMem_SfbNActiveLinesLdData` (dynamic RAM, `P_BUF_1`) | buffers of the threshold adjustment |
//!
//! In C, `PSY_DYNAMIC`, the bit look-up tables and the threshold adjustment buffers share the
//! same memory (`P_BUF_1`). The port keeps them apart; code that reads values of one of them that
//! another one left there is documented at the reading site.

use crate::aacenc::{AacencBitrateMode, AacencConfig};
use crate::interface::PsyOut;
use crate::psy_main::PsyInternal;
use crate::qc_data::{ChannelMapping, QcOut, QcState};
use fdk_aac_rs_fdk::audio::{AudioObjectType, ChannelMode};

/// `OUTPUTBUFFER_SIZE`: output buffer size has to be at least 6144 bits per channel (768
/// bytes). FDK bitbuffer implementation expects buffer of size 2^n.
pub const OUTPUTBUFFER_SIZE: usize = 8192;

/// `struct AAC_ENC`
///
/// In C, `config` points to the configuration owned by the caller (`aacenc_lib.cpp`), and the
/// other members point to the static memory pools; here the members are owned (`config` is a
/// copy that `FDKaacEnc_Initialize()` takes).
#[derive(Clone, Debug, Default)]
pub struct AacEnc {
    pub config: AacencConfig,
    /// ancillary bits per frame calculated from ancillary rate
    pub ancillary_bits_per_frame: i32,
    pub channel_mapping: ChannelMapping,
    pub qc_kernel: Option<Box<QcState>>,
    pub qc_out: [Option<Box<QcOut>>; 1],
    pub psy_out: [Option<Box<PsyOut>>; 1],
    pub psy_kernel: Option<Box<PsyInternal>>,
    // lifetime vars
    pub encoder_mode: ChannelMode,
    pub bandwidth90d_b: i32,
    pub bitrate_mode: AacencBitrateMode,
    /// use: write ADIF header only before 1st frame
    pub dont_write_adif: i32,
    /// used while allocation
    pub max_channels: i32,
    pub max_elements: i32,
    pub max_frames: i32,
    /// AOT to be used while encoding.
    pub aot: AudioObjectType,
}
