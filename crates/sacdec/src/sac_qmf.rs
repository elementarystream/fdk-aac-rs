// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_qmf.h, libSACdec/src/sac_qmf.cpp.

//! Port of `sac_qmf.h` and `sac_qmf.cpp`: one slot of QMF analysis or synthesis with the
//! `PCM_MPS` (`LONG`) sample type.
//!
//! The C functions take the filter bank (whose `FilterStates` pointer points to the states in
//! the QMF domain); the port takes the filter bank and its states (see
//! [`fdk_aac_rs_fdk::qmf`]).

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_domain::FdkQmfDomainOut;

use crate::sac_dec_interface::*;

/// `CalculateSpaceSynthesisQmf(hQmfDomainOutCh, Sr, Si, stride, timeSig)`: converts one slot
/// of QMF data into audio output data (`timeSig[i * stride]`). `h_qmf_domain_out_ch` is `None`
/// for `NULL`.
pub fn calculate_space_synthesis_qmf(
    h_qmf_domain_out_ch: Option<&mut FdkQmfDomainOut>,
    sr: &[FixpDbl],
    si: &[FixpDbl],
    stride: i32,
    time_sig: &mut [PcmMps],
) -> SacdecError {
    let Some(h) = h_qmf_domain_out_ch else {
        return MPS_INVALID_HANDLE;
    };
    let h_space_synthesis_qmf = &h.fb;
    let states = h.p_syn_qmf_states.as_deref_mut().unwrap_or(&mut []);
    // C_AALLOC_SCRATCH_START(pWorkBuffer, FIXP_DBL, (QMF_MAX_SYNTHESIS_BANDS << 1)): written
    // before it is read.
    let mut p_work_buffer = [0 as FixpDbl; (QMF_MAX_SYNTHESIS_BANDS << 1) as usize];
    qmf_synthesis_filtering_slot::<PcmMps>(
        h_space_synthesis_qmf,
        states,
        sr,
        si,
        0,
        0,
        time_sig,
        stride,
        &mut p_work_buffer,
    );
    MPS_OK
}

/// `CalculateSpaceAnalysisQmf(hSpaceAnalysisQmf, timeSig, Sr, Si)`: converts one slot of audio
/// input data into QMF data. `h_space_analysis_qmf` is the filter bank with its states, `None`
/// for `NULL`.
pub fn calculate_space_analysis_qmf(
    h_space_analysis_qmf: Option<(&QmfFilterBank, &mut [FixpDbl])>,
    time_sig: &[PcmMps],
    sr: &mut [FixpDbl],
    si: &mut [FixpDbl],
) -> SacdecError {
    let Some((fb, states)) = h_space_analysis_qmf else {
        return MPS_INVALID_HANDLE;
    };
    // C_AALLOC_SCRATCH_START(pWorkBuffer, FIXP_DBL, (64 << 1)): written before it is read.
    let mut p_work_buffer = [0 as FixpDbl; 64 << 1];
    qmf_analysis_filtering_slot::<PcmMps>(fb, states, sr, si, time_sig, 1, &mut p_work_buffer);
    MPS_OK
}
