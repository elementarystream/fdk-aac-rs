// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPDec/src/tpdec_drm.h, libMpegTPDec/src/tpdec_drm.cpp.

//! Port of `tpdec_drm.h`: the DRM CRC. `DrmRawSdcAudioConfig_Parse()`, declared in
//! `tpdec_drm.h`, is defined in `tpdec_asc.cpp` and re-exported here.
//! (`drmRead_DecodeHeader()` is declared but not defined in the reference.)
//!
//! The code of this file is identical in the `aarch64` and `x86` reference builds (apart from
//! `FDK_ASSERT`).

use fdk_aac_rs_fdk::bitstream::{fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::crc::{
    fdk_crc_end_reg, fdk_crc_get_crc, fdk_crc_init, fdk_crc_reset, fdk_crc_start_reg, FdkCrcInfo,
};

use crate::tpdec_lib::*;

pub use crate::tpdec_asc::drm_raw_sdc_audio_config_parse;

/// `STRUCT_DRM`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StructDrm {
    /// CRC state info
    pub crc_info: FdkCrcInfo,
    /// CRC value read from bitstream data
    pub crc_read_value: u16,
}

/// `HANDLE_DRM`
pub type HandleDrm<'a> = &'a mut StructDrm;

/// `drmRead_CrcInit(pDrm)`: initializes the CRC info.
pub fn drm_read_crc_init(p_drm: &mut StructDrm) {
    fdk_crc_init(&mut p_drm.crc_info, 0x001d, 0xFFFF, 8);
}

/// `drmRead_CrcStartReg(pDrm, hBs, mBits)`: resets the CRC, reads the CRC value (8 bits) and
/// starts a CRC region. Returns the region ID.
pub fn drm_read_crc_start_reg(p_drm: &mut StructDrm, h_bs: &mut FdkBitStream, m_bits: i32) -> i32 {
    fdk_crc_reset(&mut p_drm.crc_info);

    p_drm.crc_read_value = fdk_read_bits(h_bs, 8) as u16;

    fdk_crc_start_reg(&mut p_drm.crc_info, h_bs, m_bits)
}

/// `drmRead_CrcEndReg(pDrm, hBs, reg)`: ends a CRC region.
pub fn drm_read_crc_end_reg(p_drm: &mut StructDrm, h_bs: &mut FdkBitStream, reg: i32) {
    fdk_crc_end_reg(&mut p_drm.crc_info, h_bs, reg);
}

/// `drmRead_CrcCheck(pDrm)`: compares the calculated CRC (inverted) with the one read from the
/// bitstream.
pub fn drm_read_crc_check(p_drm: &mut StructDrm) -> TransportDecError {
    let error_status = TRANSPORTDEC_OK;

    let crc = fdk_crc_get_crc(&p_drm.crc_info) ^ 0xFF;
    if crc != p_drm.crc_read_value {
        return TRANSPORTDEC_CRC_ERROR;
    }

    error_status
}
