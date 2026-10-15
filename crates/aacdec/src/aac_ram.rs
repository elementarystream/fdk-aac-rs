// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aac_ram.h, libAACdec/src/aac_ram.cpp.

//! Port of `aac_ram.h`: the static and dynamic memory of the AAC decoder.
//!
//! Every `C_ALLOC_MEM*` of `aac_ram.cpp` defines `Get<name>(n)`, `Free<name>(p)` and
//! `GetRequiredMem<name>()`. In the port (PORTING.md, "Memory"):
//!
//! - `Get<name>(n)` returns an owned, zero-filled allocation (`Box` or `Vec`), as the C
//!   functions return zero-filled memory (`FDKcalloc`, `FDKaalloc`, `FDKaalloc_L`). C returns
//!   `NULL` on allocation failure; the port aborts like any Rust allocation. `n` is only checked
//!   with `FDK_ASSERT` in C ([`fdk_assert!`]).
//! - `Free<name>(p)` takes `&mut Option<...>` and sets it to `None` (`*p = NULL`).
//! - `GetRequiredMem<name>()` is ported where the size does not depend on the C layout of a
//!   structure: the values are the byte sizes of the reference, which the decoder uses
//!   (`timeData2Size = GetRequiredMemWorkBufferCore5()`, the check of
//!   `GetRequiredMemWorkBufferCore1()` in `aacDecoder_DecodeFrame()`). The sizes of
//!   `AacDecoder`, `AacDecoderStaticChannelInfo`, `AacDecoderChannelInfo`, `DrcInfo`,
//!   `CpePersistentData`, `CplxPredictionData` and `WorkBufferCore6` are `sizeof` of C
//!   structures with pointers; the library never reads them, and they are not ported.
//!
//! `AacDecoder` (`AAC_DECODER_INSTANCE`) is allocated by the decoder instance itself;
//! `SpectralCoeffs` and `SpecScale` are declared in `aac_ram.h` but not defined.
//!
//! `WorkBufferCore6` is the `pComData` of the first channel element (`CAacDecoder_Init()`
//! casts it to `CAacDecoderCommonData *`); [`get_work_buffer_core6`] therefore returns a
//! [`CAacDecoderCommonData`]. Its C size is `max(sizeof(FIXP_DBL) * WB_SECTION_SIZE,
//! sizeof(CAacDecoderCommonData))`; nothing else uses that memory.

use fdk_aac_rs_fdk::common_fix::FixpDbl;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::machine_type::{algn_size_extres, ALIGNMENT_DEFAULT};

use crate::aac_rom::PcmDec;
use crate::aacdec_drc_types::CDrcInfo;
use crate::channelinfo::{
    CAacDecoderChannelInfo, CAacDecoderCommonData, CAacDecoderStaticChannelInfo,
    CCplxPredictionData, CWorkBufferCore1, CpePersistentData,
};
use crate::overlapadd::OverlapBufferSize;

/// `MAX_SYNCHS`
pub const MAX_SYNCHS: i32 = 10;
/// `SAMPL_FREQS`
pub const SAMPL_FREQS: i32 = 12;

/// `WORKBUFFER1_TAG`
pub const WORKBUFFER1_TAG: i32 = 0;
/// `WORKBUFFER2_TAG`
pub const WORKBUFFER2_TAG: i32 = 1;
/// `WORKBUFFER5_TAG`
pub const WORKBUFFER5_TAG: i32 = 6;
/// `WORKBUFFER6_TAG`
pub const WORKBUFFER6_TAG: i32 = 7;

/// `TIME_DATA_FLUSH_SIZE` of `aacdecoder.h`, the size of [`get_time_data_flush`].
/// `crate::aacdecoder` defines the same value.
pub const TIME_DATA_FLUSH_SIZE: i32 = 128;

/// Number of channels of the per channel allocations (`(8)` in `aac_ram.cpp`).
const NUM_CHANNELS: i32 = 8;

/// `sizeof(void *)` of the target, part of the size of the aligned allocations.
const PTR_SIZE: usize = core::mem::size_of::<*const u8>();

/// `GetRequiredMem` of `C_ALLOC_MEM2(name, type, n1, n2)`.
const fn required_mem2(bytes: usize, n2: usize) -> u32 {
    (algn_size_extres(bytes) * n2) as u32
}

/// `GetRequiredMem` of `C_AALLOC_MEM*` and `C_ALLOC_MEM_OVERLAY` (`FDKaalloc` adds the
/// alignment and a pointer).
const fn required_amem(bytes: usize, n2: usize) -> u32 {
    (algn_size_extres(bytes + ALIGNMENT_DEFAULT + PTR_SIZE) * n2) as u32
}

/* StaticAacData: static memory areas, must not be overwritten in other sections of the decoder */

/// `GetAacDecoderStaticChannelInfo(n)` (`C_ALLOC_MEM2(..., 1, (8))`): the static sideinfo
/// which is needed for the decoding of one aac channel.
pub fn get_aac_decoder_static_channel_info(n: i32) -> Box<CAacDecoderStaticChannelInfo> {
    fdk_assert!(n < NUM_CHANNELS);
    Box::default()
}

/// `FreeAacDecoderStaticChannelInfo(p)`
pub fn free_aac_decoder_static_channel_info(p: &mut Option<Box<CAacDecoderStaticChannelInfo>>) {
    *p = None;
}

/// `GetAacDecoderChannelInfo(n)` (`C_AALLOC_MEM2(..., 1, (8))`): the dynamic sideinfo which is
/// needed for the decoding of one aac channel.
pub fn get_aac_decoder_channel_info(n: i32) -> Box<CAacDecoderChannelInfo> {
    fdk_assert!(n < NUM_CHANNELS);
    Box::default()
}

/// `FreeAacDecoderChannelInfo(p)`
pub fn free_aac_decoder_channel_info(p: &mut Option<Box<CAacDecoderChannelInfo>>) {
    *p = None;
}

/// `GetOverlapBuffer(n)` (`C_AALLOC_MEM2(OverlapBuffer, FIXP_DBL, OverlapBufferSize, (8))`).
/// The decoder moves it into `IMdct` ([`fdk_aac_rs_fdk::mdct::MdctT::overlap`]).
pub fn get_overlap_buffer(n: i32) -> Vec<FixpDbl> {
    fdk_assert!(n < NUM_CHANNELS);
    vec![0; OverlapBufferSize as usize]
}

/// `FreeOverlapBuffer(p)`
pub fn free_overlap_buffer(p: &mut Option<Vec<FixpDbl>>) {
    *p = None;
}

/// `GetRequiredMemOverlapBuffer()`
pub const fn get_required_mem_overlap_buffer() -> u32 {
    required_amem(OverlapBufferSize as usize * 4, NUM_CHANNELS as usize)
}

/// `GetDrcInfo(n)` (`C_ALLOC_MEM(DrcInfo, CDrcInfo, 1)`)
pub fn get_drc_info(n: i32) -> Box<CDrcInfo> {
    fdk_assert!(n == 0);
    Box::default()
}

/// `FreeDrcInfo(p)`
pub fn free_drc_info(p: &mut Option<Box<CDrcInfo>>) {
    *p = None;
}

/// `GetCpePersistentData(n)` (`C_ALLOC_MEM(CpePersistentData, CpePersistentData, 1)`): the
/// persistent data shared by both channels of a CPE.
pub fn get_cpe_persistent_data(n: i32) -> Box<CpePersistentData> {
    fdk_assert!(n == 0);
    Box::default()
}

/// `FreeCpePersistentData(p)`
pub fn free_cpe_persistent_data(p: &mut Option<Box<CpePersistentData>>) {
    *p = None;
}

/// `GetCplxPredictionData(n)` (`C_ALLOC_MEM(CplxPredictionData, CCplxPredictionData, 1)`):
/// data for complex stereo prediction.
pub fn get_cplx_prediction_data(n: i32) -> Box<CCplxPredictionData> {
    fdk_assert!(n == 0);
    Box::default()
}

/// `FreeCplxPredictionData(p)`
pub fn free_cplx_prediction_data(p: &mut Option<Box<CCplxPredictionData>>) {
    *p = None;
}

/// `GetTimeDataFlush(n)` (`C_ALLOC_MEM2(TimeDataFlush, PCM_DEC, TIME_DATA_FLUSH_SIZE, (8))`):
/// time samples for the crossfade in case of an USAC DASH IPF config change.
pub fn get_time_data_flush(n: i32) -> Vec<PcmDec> {
    fdk_assert!(n < NUM_CHANNELS);
    vec![0; TIME_DATA_FLUSH_SIZE as usize]
}

/// `FreeTimeDataFlush(p)`
pub fn free_time_data_flush(p: &mut Option<Vec<PcmDec>>) {
    *p = None;
}

/// `GetRequiredMemTimeDataFlush()`
pub const fn get_required_mem_time_data_flush() -> u32 {
    required_mem2(TIME_DATA_FLUSH_SIZE as usize * 4, NUM_CHANNELS as usize)
}

/* DynamicAacData: dynamic memory areas, might be reused in other algorithm sections, e.g. the
sbr decoder */

/// `GetWorkBufferCore2(n)` (`C_ALLOC_MEM_OVERLAY(WorkBufferCore2, FIXP_DBL, ((8) * 1024),
/// SECT_DATA_L2, WORKBUFFER2_TAG)`): the spectra of all channels (`pSpectralCoefficient` of
/// channel `ch` is `&workBufferCore2[ch * 1024]`).
pub fn get_work_buffer_core2(n: i32) -> Vec<FixpDbl> {
    fdk_assert!(n == 0);
    vec![0; (NUM_CHANNELS * 1024) as usize]
}

/// `FreeWorkBufferCore2(p)`
pub fn free_work_buffer_core2(p: &mut Option<Vec<FixpDbl>>) {
    *p = None;
}

/// `GetRequiredMemWorkBufferCore2()`
pub const fn get_required_mem_work_buffer_core2() -> u32 {
    required_amem((NUM_CHANNELS * 1024) as usize * 4, 1)
}

/// `GetWorkBufferCore6(n)` (`C_ALLOC_MEM_OVERLAY(WorkBufferCore6, SCHAR, ...)`): the
/// `pComData` of the first channel element (see the [module documentation](self)).
pub fn get_work_buffer_core6(n: i32) -> Box<CAacDecoderCommonData> {
    fdk_assert!(n == 0);
    Box::default()
}

/// `FreeWorkBufferCore6(p)`
pub fn free_work_buffer_core6(p: &mut Option<Box<CAacDecoderCommonData>>) {
    *p = None;
}

/// `GetWorkBufferCore1(n)` (`C_ALLOC_MEM_OVERLAY(WorkBufferCore1, CWorkBufferCore1, 1,
/// SECT_DATA_L1, WORKBUFFER1_TAG)`)
pub fn get_work_buffer_core1(n: i32) -> Box<CWorkBufferCore1> {
    fdk_assert!(n == 0);
    Box::default()
}

/// `FreeWorkBufferCore1(p)`
pub fn free_work_buffer_core1(p: &mut Option<Box<CWorkBufferCore1>>) {
    *p = None;
}

/// `GetRequiredMemWorkBufferCore1()`: `sizeof(CWorkBufferCore1)` is the size of its largest
/// member, `workBuffer` (`WB_SECTION_SIZE` values).
pub const fn get_required_mem_work_buffer_core1() -> u32 {
    required_amem(core::mem::size_of::<CWorkBufferCore1>(), 1)
}

/// `GetWorkBufferCore5(n)` (`C_ALLOC_MEM_OVERLAY(WorkBufferCore5, PCM_DEC, (8) * (1024 * 4) *
/// 2, SECT_DATA_EXTERN, WORKBUFFER5_TAG)`): double buffer size needed for de-/interleaving
/// (`pTimeData2`).
pub fn get_work_buffer_core5(n: i32) -> Vec<PcmDec> {
    fdk_assert!(n == 0);
    vec![0; (NUM_CHANNELS * (1024 * 4) * 2) as usize]
}

/// `FreeWorkBufferCore5(p)`
pub fn free_work_buffer_core5(p: &mut Option<Vec<PcmDec>>) {
    *p = None;
}

/// `GetRequiredMemWorkBufferCore5()`
pub const fn get_required_mem_work_buffer_core5() -> u32 {
    required_amem((NUM_CHANNELS * (1024 * 4) * 2) as usize * 4, 1)
}
