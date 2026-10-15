// Differential tests of the decoder (through its public API, see aacdecoder_lib.rs) for channel
// elements of the bitstream that do not match the element layout of the configuration. The
// decoder allocates its per element memory (pComData) for the elements of the configuration, and
// a CPE of the bitstream may start at the channel of an SCE (the program config element lists
// the CPE's tag, and all elements of the PCE are present). Found by the decode_adif fuzz target.
//
// - A CPE decoded into the channels of an SCE and of the next element has a different pComData
//   per channel: the PNS correlation flags and random seeds of the second channel are its own
//   (C reaches them through pAacDecoderChannelInfo[ch]->pComData).
// - The PNS seed pointer PnsData.currentSeed of the second channel of a CPE is set to the first
//   channel's and kept: a later element that starts at that channel draws its noise from the
//   seed of the CPE's first channel, not from its own.
//
// The streams are synthesized here: raw access units after ConfigRaw with an AudioSpecificConfig
// (AAC-LC, channel configuration 0, PCE with the front elements SCE 0 and CPE 0).

use crate::aacdecoder_lib::{run_packets, Pair};
use fdk_aac_rs_fdk::audio::TT_MP4_RAW;

/// A bit writer (MSB first).
#[derive(Default)]
struct Bits {
    v: Vec<bool>,
}

impl Bits {
    fn put(&mut self, x: u32, n: u32) {
        for i in (0..n).rev() {
            self.v.push((x >> i) & 1 != 0);
        }
    }
    fn align(&mut self) {
        while self.v.len() % 8 != 0 {
            self.v.push(false);
        }
    }
    fn bytes(mut self) -> Vec<u8> {
        self.align();
        self.v.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | b as u8)).collect()
    }
}

/// Sampling frequency index 3 (48 kHz).
const SFI: u32 = 3;

/// AudioSpecificConfig: AAC-LC, 48 kHz, channel configuration 0 and a PCE with the front
/// elements SCE (tag 0) and CPE (tag 0).
fn asc() -> Vec<u8> {
    let mut b = Bits::default();
    b.put(2, 5); // audioObjectType: AAC-LC
    b.put(SFI, 4);
    b.put(0, 4); // channelConfiguration
    b.put(0, 1); // frameLengthFlag
    b.put(0, 1); // dependsOnCoreCoder
    b.put(0, 1); // extensionFlag
    // program_config_element()
    b.put(0, 4); // element_instance_tag
    b.put(1, 2); // object_type: LC
    b.put(SFI, 4);
    b.put(2, 4); // num_front_channel_elements
    b.put(0, 4); // num_side_channel_elements
    b.put(0, 4); // num_back_channel_elements
    b.put(0, 2); // num_lfe_channel_elements
    b.put(0, 3); // num_assoc_data_elements
    b.put(0, 4); // num_valid_cc_elements
    b.put(0, 3); // mono, stereo, matrix mixdown not present
    b.put(0, 1); // front_element_is_cpe
    b.put(0, 4); // front_element_tag_select
    b.put(1, 1); // front_element_is_cpe
    b.put(0, 4); // front_element_tag_select
    b.align(); // byte_alignment() (relative to the start of the ASC)
    b.put(0, 8); // comment_field_bytes
    b.bytes()
}

/// ics_info() of a long block with `max_sfb` bands.
fn ics_info(b: &mut Bits, max_sfb: u32) {
    b.put(0, 1); // ics_reserved_bit
    b.put(0, 2); // window_sequence: ONLY_LONG_SEQUENCE
    b.put(0, 1); // window_shape
    b.put(max_sfb, 6);
    b.put(0, 1); // predictor_data_present
}

/// individual_channel_stream() whose `max_sfb` bands are all noise bands (PNS, codebook 13) with
/// the energy `nrg`, or all zero bands (codebook 0) if `nrg` is `None`; with ics_info() if
/// `with_ics_info` (not in a CPE with a common window).
fn ics(b: &mut Bits, max_sfb: u32, nrg: Option<i32>, with_ics_info: bool) {
    let global_gain = 100;
    b.put(global_gain, 8);
    if with_ics_info {
        ics_info(b, max_sfb);
    }
    // section_data(): one section over all bands
    b.put(if nrg.is_some() { 13 } else { 0 }, 4);
    let mut len = max_sfb;
    while len >= 31 {
        b.put(31, 5); // sect_esc_val
        len -= 31;
    }
    b.put(len, 5);
    // scale_factor_data(): the first noise band codes the energy in 9 bits, the others a
    // Huffman coded difference (index 60, difference 0: the code "0"); nothing for zero bands
    if let Some(nrg) = nrg {
        b.put((nrg - (global_gain as i32 - 90) + 256) as u32, 9);
        for _ in 1..max_sfb {
            b.put(0, 1);
        }
    }
    b.put(0, 1); // pulse_data_present
    b.put(0, 1); // tns_data_present
    b.put(0, 1); // gain_control_data_present
    // spectral_data(): nothing for noise and zero bands
}

/// A single_channel_element() with tag 0.
fn sce(b: &mut Bits, max_sfb: u32, nrg: Option<i32>) {
    b.put(0, 3); // ID_SCE
    b.put(0, 4); // element_instance_tag
    ics(b, max_sfb, nrg, true);
}

/// A channel_pair_element() with tag 0, a common window and M/S in all bands.
fn cpe(b: &mut Bits, max_sfb: u32, nrg: Option<(i32, i32)>) {
    b.put(1, 3); // ID_CPE
    b.put(0, 4); // element_instance_tag
    b.put(1, 1); // common_window
    ics_info(b, max_sfb);
    b.put(2, 2); // ms_mask_present: all bands
    ics(b, max_sfb, nrg.map(|n| n.0), false);
    ics(b, max_sfb, nrg.map(|n| n.1), false);
}

/// The raw_data_block() of the configuration: SCE (channel 0), CPE (channels 1 and 2), with
/// noise bands.
fn au_sce_cpe(max_sfb: u32) -> Vec<u8> {
    let mut b = Bits::default();
    sce(&mut b, max_sfb, Some(38));
    cpe(&mut b, max_sfb, Some((40, 44)));
    b.put(7, 3); // ID_END
    b.bytes()
}

/// A raw_data_block() with the elements swapped: CPE (channels 0 and 1: the SCE's channel and
/// the first channel of the CPE of the configuration), SCE (channel 2), with noise bands or
/// (`pns` false) zero bands. The decoder accepts it: the PCE lists both tags, and all its
/// elements are present.
fn au_cpe_sce(max_sfb: u32, pns: bool) -> Vec<u8> {
    let mut b = Bits::default();
    cpe(&mut b, max_sfb, pns.then_some((40, 44)));
    sce(&mut b, max_sfb, pns.then_some(38));
    b.put(7, 3); // ID_END
    b.bytes()
}

/// Decodes the access units on both decoders (comparing every call); returns the number of
/// valid frames.
fn run(ctx: &str, aus: &[Vec<u8>]) -> usize {
    let mut pair = Pair::open(TT_MP4_RAW, 1, ctx).unwrap();
    assert_eq!(pair.config_raw(&asc()), 0, "{ctx}: ConfigRaw");
    let aus: Vec<&[u8]> = aus.iter().map(|a| a.as_slice()).collect();
    run_packets(&mut pair, &aus, usize::MAX, |_| 0);
    pair.n_valid
}

/// A CPE decoded into the channels of the SCE and of the CPE of the configuration: the second
/// channel has its own PNS correlation flags and random seeds (in C it draws its own noise; the
/// M/S mask does not make it correlated).
#[test]
fn cpe_on_sce_channel_com_data() {
    for max_sfb in [1, 8, 20, 49] {
        let ctx = format!("CPE on the SCE channel, {max_sfb} PNS bands");
        let aus = vec![au_cpe_sce(max_sfb, true); 4];
        assert_eq!(run(&ctx, &aus), aus.len(), "{ctx}: valid frames");
    }
}

/// After a CPE at channels 0 and 1, the CPE of the configuration (channels 1 and 2) draws its
/// noise from the PNS seed of channel 0: C keeps PnsData.currentSeed of channel 1 pointing there.
/// The swapped frame has no noise bands, so only the seed pointer matters.
#[test]
fn pns_seed_pointer_after_cpe() {
    for max_sfb in [4, 20, 49] {
        let ctx = format!("PNS seed after a CPE on the SCE channel, {max_sfb} bands");
        let (normal, swapped) = (au_sce_cpe(max_sfb), au_cpe_sce(max_sfb, false));
        let aus = vec![normal.clone(), swapped, normal.clone(), normal];
        assert_eq!(run(&ctx, &aus), aus.len(), "{ctx}: valid frames");
    }
}
