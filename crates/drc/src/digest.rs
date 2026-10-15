// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the state digest of the MPEG-D DRC decoder (see fdk_aac_rs_fdk::digest).

//! [`StateDigest`] of the MPEG-D DRC decoder (not in C; see `fdk_aac_rs_fdk::digest`).
//!
//! The structures of the DRC decoder are plain old data with the memory image of the C
//! structures ([`Pod`](crate::drc_dec_types::Pod)): they are fed as their bytes, which include
//! the union members that are not in use and the (zero) padding, as the reader compares them
//! (`FDKmemcmp`). The pointers of the C structures are indices here (`pInst`, `pCoef`).

use fdk_aac_rs_fdk::digest::{Digester, StateDigest};

use crate::drc_dec_gain_decoder::DrcGainDecoder;
use crate::drc_dec_lib::DrcDecoder;
use crate::drc_dec_selection_process::{DrcSelectionProcess, SelProcOutput};
use crate::drc_dec_types::{pod_bytes, LoudnessInfoSet, UniDrcConfig, UniDrcGain};

macro_rules! pod_digest {
    ($($t:ty),*) => {$(
        impl StateDigest for $t {
            fn digest(&self, d: &mut Digester) {
                d.bytes(pod_bytes(self));
            }
        }
    )*};
}

pod_digest!(
    DrcGainDecoder,
    DrcSelectionProcess,
    SelProcOutput,
    UniDrcConfig,
    LoudnessInfoSet,
    UniDrcGain
);

impl StateDigest for DrcDecoder {
    fn digest(&self, d: &mut Digester) {
        let DrcDecoder {
            codec_mode,
            functional_range,
            status,
            h_gain_dec,
            h_selection_proc,
            sel_proc_input_diff,
            uni_drc_config,
            loudness_info_set,
            uni_drc_gain,
            sel_proc_output,
        } = self;
        d.field("codec_mode", codec_mode);
        d.field("functional_range", functional_range);
        d.field("status", status);
        d.field("h_gain_dec", h_gain_dec);
        d.field("h_selection_proc", h_selection_proc);
        d.field("sel_proc_input_diff", sel_proc_input_diff);
        d.field("uni_drc_config", uni_drc_config);
        d.field("loudness_info_set", loudness_info_set);
        d.field("uni_drc_gain", uni_drc_gain);
        d.field("sel_proc_output", sel_proc_output);
    }
}
