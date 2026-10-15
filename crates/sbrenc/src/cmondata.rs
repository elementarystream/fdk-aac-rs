// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/cmondata.h.

//! Port of `cmondata.h`: the data structure shared by the core coder and SBR.

use fdk_aac_rs_fdk::bitstream::FdkBitStream;

/// `struct COMMON_DATA` (`HANDLE_COMMON_DATA` is `&mut CommonData`).
///
/// Both bitstreams use the payload delay line of their SBR element
/// ([`crate::sbr::SbrElement::payload_delay_line`]), as in C.
#[derive(Clone, Debug, Default)]
pub struct CommonData {
    /// `sbrHdrBits`: number of SBR header bits
    pub sbr_hdr_bits: i32,
    /// `sbrDataBits`: number of SBR data bits
    pub sbr_data_bits: i32,
    /// `sbrFillBits`: number of SBR fill bits
    pub sbr_fill_bits: i32,
    /// `sbrBitbuf`: the SBR data bitbuffer
    pub sbr_bitbuf: FdkBitStream,
    /// `tmpWriteBitbuf`: helper var for writing header
    pub tmp_write_bitbuf: FdkBitStream,
    /// `xOverFreq`: the SBR crossover frequency
    pub x_over_freq: i32,
    /// `dynBwEnabled`: indicates if dynamic bandwidth is enabled
    pub dyn_bw_enabled: i32,
    /// `sbrNumChannels`: number of channels (meaning mono or stereo)
    pub sbr_num_channels: i32,
    /// `dynXOverFreqEnc`: encoder dynamic crossover frequency
    pub dyn_x_over_freq_enc: i32,
}
