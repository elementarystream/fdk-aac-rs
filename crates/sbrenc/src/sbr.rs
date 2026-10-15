// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/sbr.h.

//! Port of `sbr.h`: the main SBR encoder structures.
//!
//! The pointers between the structures become indices: an element's channels
//! (`SBR_ELEMENT::sbrChannel[ch]`) are indices into [`SbrEncoder::p_sbr_channel`], its QMF
//! analysis filter banks (`hQmfAnalysis[ch]`) indices into [`SbrEncoder::qmf_analysis`]. The
//! filter states of the QMF banks (`FilterStates`) are owned by the encoder
//! ([`SbrEncoder::qmf_analysis_states`], [`SbrEncoder::qmf_synthesis_ps_states`]).

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::common_fix::FixpDbl;
use fdk_aac_rs_fdk::qmf::QmfFilterBank;

use crate::bit_sbr::{SbrBitstreamData, SbrHeaderData};
use crate::cmondata::CommonData;
use crate::env_est::EnvChannel;
use crate::ps_main::ParametricStereo;
use crate::resampler::Downsampler;
use crate::sbr_encoder::{SbrConfigData, SbrElementInfo, MAX_PAYLOAD_SIZE};

/// `MAX_DELAY_FRAMES`: SBR bitstream delay
pub const MAX_DELAY_FRAMES: usize = 2;

/// `SBRENC_DS_TYPE`: sbr encoder downsampling type
pub type SbrencDsType = i32;
/// `SBRENC_DS_NONE`
pub const SBRENC_DS_NONE: SbrencDsType = 0;
/// `SBRENC_DS_TIME`
pub const SBRENC_DS_TIME: SbrencDsType = 1;
/// `SBRENC_DS_QMF`
pub const SBRENC_DS_QMF: SbrencDsType = 2;

/// The maximum number of elements and channels of an encoder instance (`(8)`).
pub const MAX_SBR_ELEMENTS: usize = 8;

/// `SBR_CHANNEL` (`HANDLE_SBR_CHANNEL` is an index into [`SbrEncoder::p_sbr_channel`])
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SbrChannel {
    /// `hEnvChannel`
    pub h_env_channel: EnvChannel,
    /// `downSampler`
    pub down_sampler: Downsampler,
}

/// `SBR_ELEMENT` (`HANDLE_SBR_ELEMENT` is `&mut SbrElement`). The bitstreams of `CmonData`
/// use rows of `payloadDelayLine`; a clone has its own copy of the delay line with its
/// bitstreams bound to it.
#[derive(Debug)]
pub struct SbrElement {
    /// `sbrChannel`: indices into [`SbrEncoder::p_sbr_channel`] (`None` for `NULL`)
    pub sbr_channel: [Option<usize>; 2],
    /// `hQmfAnalysis`: indices into [`SbrEncoder::qmf_analysis`] (`None` for `NULL`)
    pub h_qmf_analysis: [Option<usize>; 2],
    /// `sbrConfigData`
    pub sbr_config_data: SbrConfigData,
    /// `sbrHeaderData`
    pub sbr_header_data: SbrHeaderData,
    /// `sbrBitstreamData`
    pub sbr_bitstream_data: SbrBitstreamData,
    /// `CmonData`
    pub cmon_data: CommonData,
    /// `dynXOverFreqDelay`: to delay a frame (I don't like it that much that way - hrc)
    pub dyn_x_over_freq_delay: [i32; 5],
    /// `elInfo`
    pub el_info: SbrElementInfo,

    /// `payloadDelayLine`: `1 + MAX_DELAY_FRAMES` rows of `MAX_PAYLOAD_SIZE` bytes (row `r` at
    /// byte `r * MAX_PAYLOAD_SIZE`)
    pub payload_delay_line: BitMem,
    /// `payloadDelayLineSize`: Sizes in bits
    pub payload_delay_line_size: [u32; 1 + MAX_DELAY_FRAMES],
}

impl Default for SbrElement {
    /// The cleared element (`FDKmemclear`).
    fn default() -> Self {
        SbrElement {
            sbr_channel: [None; 2],
            h_qmf_analysis: [None; 2],
            sbr_config_data: SbrConfigData::default(),
            sbr_header_data: SbrHeaderData::default(),
            sbr_bitstream_data: SbrBitstreamData::default(),
            cmon_data: CommonData::default(),
            dyn_x_over_freq_delay: [0; 5],
            el_info: SbrElementInfo::default(),
            payload_delay_line: BitMem::new((1 + MAX_DELAY_FRAMES) * MAX_PAYLOAD_SIZE),
            payload_delay_line_size: [0; 1 + MAX_DELAY_FRAMES],
        }
    }
}

impl Clone for SbrElement {
    /// The C struct copy: the delay line is copied, and the copied bitstreams use the copy.
    fn clone(&self) -> Self {
        let payload_delay_line = self.payload_delay_line.deep_copy();
        let mut cmon_data = self.cmon_data.clone();
        cmon_data.sbr_bitbuf.h_bit_buf.rebind(&self.payload_delay_line, &payload_delay_line);
        cmon_data.tmp_write_bitbuf.h_bit_buf.rebind(&self.payload_delay_line, &payload_delay_line);
        SbrElement {
            sbr_channel: self.sbr_channel,
            h_qmf_analysis: self.h_qmf_analysis,
            sbr_config_data: self.sbr_config_data,
            sbr_header_data: self.sbr_header_data,
            sbr_bitstream_data: self.sbr_bitstream_data,
            cmon_data,
            dyn_x_over_freq_delay: self.dyn_x_over_freq_delay,
            el_info: self.el_info,
            payload_delay_line,
            payload_delay_line_size: self.payload_delay_line_size,
        }
    }
}

/// `SBR_ENCODER` (`HANDLE_SBR_ENCODER` is `&mut SbrEncoder`). `Clone` (not in C) takes the
/// snapshots of [`SbrEncoder::restore_from`].
#[derive(Clone, Debug)]
pub struct SbrEncoder {
    /// `sbrElement`
    pub sbr_element: [Option<Box<SbrElement>>; MAX_SBR_ELEMENTS],
    /// `pSbrChannel`
    pub p_sbr_channel: [Option<Box<SbrChannel>>; MAX_SBR_ELEMENTS],
    /// `QmfAnalysis`
    pub qmf_analysis: [QmfFilterBank; MAX_SBR_ELEMENTS],
    /// `QmfAnalysis[i].FilterStates` (`Ram_Sbr_QmfStatesAnalysis`; empty for `NULL`)
    pub qmf_analysis_states: [Vec<i16>; MAX_SBR_ELEMENTS],
    /// `lfeDownSampler`
    pub lfe_down_sampler: Downsampler,
    /// `lfeChIdx`: -1 default for no lfe, else assign channel index.
    pub lfe_ch_idx: i32,
    /// `noElements`: Number of elements.
    pub no_elements: i32,
    /// `nChannels`: Total channel count across all elements.
    pub n_channels: i32,
    /// `frameSize`: SBR framelength.
    pub frame_size: i32,
    /// `bufferOffset`: Offset for SBR parameter extraction in time domain input buffer.
    pub buffer_offset: i32,
    /// `downsampledOffset`: Offset of downsampled/mixed output for core encoder.
    pub downsampled_offset: i32,
    /// `downmixSize`: Size in samples of downsampled/mixed output for core encoder.
    pub downmix_size: i32,
    /// `downSampleFactor`: Sampling rate relation between the SBR and the core encoder.
    pub down_sample_factor: i32,
    /// `downsamplingMethod`: Method of downsmapling, time-domain, QMF or none.
    pub downsampling_method: SbrencDsType,
    /// `nBitstrDelay`: Amount of SBR frames to be delayed in bitstream domain.
    pub n_bitstr_delay: i32,
    /// `sbrDecDelay`: SBR decoder delay in samples
    pub sbr_dec_delay: i32,
    /// `estimateBitrate`: Estimate bitrate of SBR encoder.
    pub estimate_bitrate: i32,
    /// `inputDataDelay`: Delay caused by downsampler, in/out buffer at sbrEncoder_EncodeFrame.
    pub input_data_delay: i32,

    /// `dynamicRam` (and `pSBRdynamic_RAM`, the same allocation): `Ram_SbrDynamic_RAM`
    pub dynamic_ram: Vec<FixpDbl>,

    /// `hParametricStereo`
    pub h_parametric_stereo: Option<Box<ParametricStereo>>,
    /// `qmfSynthesisPS`
    pub qmf_synthesis_ps: QmfFilterBank,
    /// `qmfSynthesisPS.FilterStates` (`Ram_PsQmfStatesSynthesis`; empty for `NULL`)
    pub qmf_synthesis_ps_states: Vec<FixpDbl>,

    // parameters describing allocation volume of present instance
    /// `maxElements`
    pub max_elements: i32,
    /// `maxChannels`
    pub max_channels: i32,
    /// `supportPS`
    pub support_ps: i32,
}

impl Default for SbrEncoder {
    /// The cleared encoder (`FDKmemclear`), without allocations.
    fn default() -> Self {
        SbrEncoder {
            sbr_element: Default::default(),
            p_sbr_channel: Default::default(),
            qmf_analysis: [QmfFilterBank::default(); MAX_SBR_ELEMENTS],
            qmf_analysis_states: Default::default(),
            lfe_down_sampler: Downsampler::default(),
            lfe_ch_idx: 0,
            no_elements: 0,
            n_channels: 0,
            frame_size: 0,
            buffer_offset: 0,
            downsampled_offset: 0,
            downmix_size: 0,
            down_sample_factor: 0,
            downsampling_method: SBRENC_DS_NONE,
            n_bitstr_delay: 0,
            sbr_dec_delay: 0,
            estimate_bitrate: 0,
            input_data_delay: 0,
            dynamic_ram: Vec::new(),
            h_parametric_stereo: None,
            qmf_synthesis_ps: QmfFilterBank::default(),
            qmf_synthesis_ps_states: Vec::new(),
            max_elements: 0,
            max_channels: 0,
            support_ps: 0,
        }
    }
}

/// Replaces the content of an optional box by `src` in place (the box keeps its address).
fn restore_box<T>(dst: &mut Option<Box<T>>, src: Option<Box<T>>) {
    match (dst.as_deref_mut(), src) {
        (Some(d), Some(s)) => *d = *s,
        (_, s) => *dst = s,
    }
}

impl SbrEncoder {
    /// Restores the state of `snapshot`, a clone of this encoder taken earlier (not in C: the
    /// opt-in frame pipelining of the AAC encoder undoes a speculative frame with it). The
    /// boxed elements, channels and parametric stereo state keep their addresses, so that the
    /// bitstream pointers of [`CommonData`] (which the clone copied) stay valid.
    pub fn restore_from(&mut self, snapshot: SbrEncoder) {
        let SbrEncoder {
            sbr_element,
            p_sbr_channel,
            qmf_analysis,
            qmf_analysis_states,
            lfe_down_sampler,
            lfe_ch_idx,
            no_elements,
            n_channels,
            frame_size,
            buffer_offset,
            downsampled_offset,
            downmix_size,
            down_sample_factor,
            downsampling_method,
            n_bitstr_delay,
            sbr_dec_delay,
            estimate_bitrate,
            input_data_delay,
            dynamic_ram,
            h_parametric_stereo,
            qmf_synthesis_ps,
            qmf_synthesis_ps_states,
            max_elements,
            max_channels,
            support_ps,
        } = snapshot;
        for (d, s) in self.sbr_element.iter_mut().zip(sbr_element) {
            restore_box(d, s);
        }
        for (d, s) in self.p_sbr_channel.iter_mut().zip(p_sbr_channel) {
            restore_box(d, s);
        }
        restore_box(&mut self.h_parametric_stereo, h_parametric_stereo);
        self.qmf_analysis = qmf_analysis;
        for (d, s) in self.qmf_analysis_states.iter_mut().zip(qmf_analysis_states) {
            if d.len() == s.len() {
                d.copy_from_slice(&s);
            } else {
                *d = s;
            }
        }
        self.lfe_down_sampler = lfe_down_sampler;
        self.lfe_ch_idx = lfe_ch_idx;
        self.no_elements = no_elements;
        self.n_channels = n_channels;
        self.frame_size = frame_size;
        self.buffer_offset = buffer_offset;
        self.downsampled_offset = downsampled_offset;
        self.downmix_size = downmix_size;
        self.down_sample_factor = down_sample_factor;
        self.downsampling_method = downsampling_method;
        self.n_bitstr_delay = n_bitstr_delay;
        self.sbr_dec_delay = sbr_dec_delay;
        self.estimate_bitrate = estimate_bitrate;
        self.input_data_delay = input_data_delay;
        self.dynamic_ram = dynamic_ram;
        self.qmf_synthesis_ps = qmf_synthesis_ps;
        self.qmf_synthesis_ps_states = qmf_synthesis_ps_states;
        self.max_elements = max_elements;
        self.max_channels = max_channels;
        self.support_ps = support_ps;
    }
}
