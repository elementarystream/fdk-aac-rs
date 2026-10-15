// Differential tests of fdk_aac_rs_aacdec::aacdec_hcr against the C++ reference, and the helpers
// of the HCR and RVLC tests (aacdec_hcrs.rs, aacdec_hcr_bit.rs, rvlc*.rs, aacdec_hcr_stream.rs).
//
// Method: the C++ side works on the context of shim/aacdec_aacdec_hcr.cpp (two channels of an
// element with their dynamic data in the element's common data, their static channel infos and a
// spectrum of 2 * 1024 values). The state is transferred as flat arrays ([`Flat`]): the channel
// fields that HCR and RVLC use, the complete HCR state, the RVLC state with its scale factor
// arrays, the RVLC fields of the concealment infos and the spectrum. Every call runs on both sides
// from the same flat state, and the return value, the complete flat state after the call and the
// bitstream state are compared.
//
// The HCR functions are driven by random side info in the form CBlock_ReadSectionData() produces
// (all window sequences, groupings, codebooks including the virtual codebooks of VCB11, section
// lengths), random lengths and random bitstreams, with corrupt side info mixed in, in sequences of
// frames on one persistent HCR state; aacdec_hcr_stream.rs replays the calls of the reference
// decoder on real ER streams.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use core::ffi::c_void;
use std::sync::OnceLock;

use fdk_aac_rs_aacdec::aac_rom::{A_HUFF_TABLE, A_QUANT_TABLE, A_DIM_CB};
use fdk_aac_rs_aacdec::aacdec_hcr::*;
use fdk_aac_rs_aacdec::block::get_scale_from_value;
use fdk_aac_rs_aacdec::aacdec_hcr_types::*;
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_cref::aacdec_aacdec_hcr as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::{ID_CCE, ID_CPE, ID_DSE, ID_LFE, ID_SCE};
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::mdct::{BLOCK_LONG, BLOCK_SHORT, BLOCK_START, BLOCK_STOP};

// ---------------------------------------------------------------------------------------------
// Flat state transfer (the same order as in shim/aacdec_aacdec_hcr.cpp)
// ---------------------------------------------------------------------------------------------

/// An integer field of the flat state.
pub trait Iv: Copy {
    fn get(self) -> i32;
    fn put(v: i32) -> Self;
}

macro_rules! iv {
    ($($t:ty),*) => {
        $(impl Iv for $t {
            fn get(self) -> i32 {
                self as i32
            }
            fn put(v: i32) -> Self {
                v as $t
            }
        })*
    };
}
iv!(i8, u8, i16, u16, i32, u32);

/// Visitor that saves (`load == false`) or loads the fields in order, optionally recording the
/// field names.
pub struct Fl<'a> {
    v: &'a mut [i32],
    pos: usize,
    load: bool,
    names: Option<&'a mut Vec<(usize, &'static str)>>,
}

impl<'a> Fl<'a> {
    pub fn new(v: &'a mut [i32], load: bool) -> Self {
        Fl { v, pos: 0, load, names: None }
    }

    fn name(&mut self, n: &'static str) {
        let pos = self.pos;
        if let Some(nm) = self.names.as_deref_mut() {
            nm.push((pos, n));
        }
    }

    pub fn s<T: Iv>(&mut self, n: &'static str, x: &mut T) {
        self.name(n);
        if self.load {
            *x = T::put(self.v[self.pos]);
        } else {
            self.v[self.pos] = x.get();
        }
        self.pos += 1;
    }

    pub fn a<T: Iv>(&mut self, n: &'static str, x: &mut [T]) {
        self.name(n);
        for e in x.iter_mut() {
            if self.load {
                *e = T::put(self.v[self.pos]);
            } else {
                self.v[self.pos] = e.get();
            }
            self.pos += 1;
        }
    }
}

pub fn xfer_ch(f: &mut Fl, p: &mut CAacDecoderChannelInfo) {
    let ics = &mut p.ics_info;
    f.a("ics.WindowGroupLength", &mut ics.window_group_length);
    f.s("ics.WindowGroups", &mut ics.window_groups);
    f.s("ics.Valid", &mut ics.valid);
    f.s("ics.WindowShape", &mut ics.window_shape);
    f.s("ics.WindowSequence", &mut ics.window_sequence);
    f.s("ics.MaxSfBands", &mut ics.max_sf_bands);
    f.s("ics.max_sfb_ste", &mut ics.max_sfb_ste);
    f.s("ics.ScaleFactorGrouping", &mut ics.scale_factor_grouping);
    f.s("ics.TotalSfBands", &mut ics.total_sf_bands);
    f.s("granuleLength", &mut p.granule_length);
    let d = &mut *p.p_dyn_data;
    f.a("aScaleFactor", &mut d.a_scale_factor);
    f.a("aSfbScale", &mut d.a_sfb_scale);
    f.a("aCodeBook", &mut d.a_code_book);
    f.s("GlobalGain", &mut d.raw_data_info.global_gain);
    f.s("CommonWindow", &mut d.raw_data_info.common_window);
    let a = &mut d.specific_to.aac;
    f.a("aNumLineInSec4Hcr", &mut a.a_num_line_in_sec4_hcr);
    f.a("aCodeBooks4Hcr", &mut a.a_code_books4_hcr);
    f.s("lenOfReorderedSpectralData", &mut a.len_of_reordered_spectral_data);
    f.s("lenOfLongestCodeword", &mut a.len_of_longest_codeword);
    f.s("numberSection", &mut a.number_section);
    f.s("rvlcCurrentScaleFactorOK", &mut a.rvlc_current_scale_factor_ok);
    f.s("rvlcIntensityUsed", &mut a.rvlc_intensity_used);
    f.a("pnsUsed", &mut p.data.aac.pns_data.pns_used);
    f.s("PnsActive", &mut p.data.aac.pns_data.pns_active);
}

pub fn xfer_hcr(f: &mut Fl, h: &mut CErHcrInfo) {
    let io = &mut h.dec_in_out;
    f.s("errorLog", &mut io.error_log);
    f.s("quantizedSpectralCoefficientsIdx", &mut io.quantized_spectral_coefficients_idx);
    f.s("lengthOfReorderedSpectralData", &mut io.length_of_reordered_spectral_data);
    f.s("numSection", &mut io.num_section);
    f.s("bitstreamAnchor", &mut io.bitstream_anchor);
    f.s("lengthOfLongestCodeword", &mut io.length_of_longest_codeword);
    let sg = &mut h.segment_info;
    f.s("numSegment", &mut sg.num_segment);
    f.a("pSegmentBitfield", &mut sg.p_segment_bitfield);
    f.a("pCodewordBitfield", &mut sg.p_codeword_bitfield);
    f.s("segmentOffset", &mut sg.segment_offset);
    f.a("pLeftStartOfSegment", &mut sg.p_left_start_of_segment);
    f.a("pRightStartOfSegment", &mut sg.p_right_start_of_segment);
    f.a("pRemainingBitsInSegment", &mut sg.p_remaining_bits_in_segment);
    f.s("readDirection", &mut sg.read_direction);
    f.s("numWordForBitfield", &mut sg.num_word_for_bitfield);
    f.s("pNumBitValidInLastWord", &mut sg.p_num_bit_valid_in_last_word);
    let sc = &mut h.section_info;
    f.s("numCodeword", &mut sc.num_codeword);
    f.s("numSortedSection", &mut sc.num_sorted_section);
    f.a("pNumCodewordInSection", &mut sc.p_num_codeword_in_section);
    f.a("pNumSortedCodewordInSection", &mut sc.p_num_sorted_codeword_in_section);
    f.a("pNumExtendedSortedCodewordInSection", &mut sc.p_num_extended_sorted_codeword_in_section);
    f.s("numExtendedSortedCodewordInSectionIdx", &mut sc.num_extended_sorted_codeword_in_section_idx);
    f.a("pNumExtendedSortedSectionsInSets", &mut sc.p_num_extended_sorted_sections_in_sets);
    f.s("numExtendedSortedSectionsInSetsIdx", &mut sc.num_extended_sorted_sections_in_sets_idx);
    f.a("pReorderOffset", &mut sc.p_reorder_offset);
    f.a("pSortedCodebook", &mut sc.p_sorted_codebook);
    f.a("pExtendedSortedCodebook", &mut sc.p_extended_sorted_codebook);
    f.s("extendedSortedCodebookIdx", &mut sc.extended_sorted_codebook_idx);
    f.a("pMaxLenOfCbInExtSrtSec", &mut sc.p_max_len_of_cb_in_ext_srt_sec);
    f.s("maxLenOfCbInExtSrtSecIdx", &mut sc.max_len_of_cb_in_ext_srt_sec_idx);
    f.a("pCodebookSwitch", &mut sc.p_codebook_switch);
    let np = &mut h.non_pcw_sideinfo;
    f.a("iNode", &mut np.i_node);
    f.a("iResultPointer", &mut np.i_result_pointer);
    f.a("pEscapeSequenceInfo", &mut np.p_escape_sequence_info);
    f.s("codewordOffset", &mut np.codeword_offset);
    f.s("pState", &mut np.p_state);
    f.a("pCodebook", &mut np.p_codebook);
    f.a("pCntSign", &mut np.p_cnt_sign);
    f.a("pSta", &mut np.p_sta);
}

pub fn xfer_rv(f: &mut Fl, com: &mut CAacDecoderCommonData) {
    use fdk_aac_rs_aacdec::aac_rom::{A_HUFF_TREE_RVLC_ESCAPE, A_HUFF_TREE_RVL_CODEWDS};
    let r = &mut com.overlay.aac.er_rvlc_info;
    f.s("sf_concealment", &mut r.sf_concealment);
    f.s("rev_global_gain", &mut r.rev_global_gain);
    f.s("length_of_rvlc_sf", &mut r.length_of_rvlc_sf);
    f.s("dpcm_noise_nrg", &mut r.dpcm_noise_nrg);
    f.s("sf_escapes_present", &mut r.sf_escapes_present);
    f.s("length_of_rvlc_escapes", &mut r.length_of_rvlc_escapes);
    f.s("dpcm_noise_last_position", &mut r.dpcm_noise_last_position);
    f.s("dpcm_is_last_position", &mut r.dpcm_is_last_position);
    f.s("length_of_rvlc_sf_fwd", &mut r.length_of_rvlc_sf_fwd);
    f.s("length_of_rvlc_sf_bwd", &mut r.length_of_rvlc_sf_bwd);
    f.s("pRvlBitCnt_RVL", &mut r.p_rvl_bit_cnt_rvl);
    f.s("pBitstrIndxRvl_RVL", &mut r.p_bitstr_indx_rvl_rvl);
    f.s("numWindowGroups", &mut r.num_window_groups);
    f.s("maxSfbTransmitted", &mut r.max_sfb_transmitted);
    f.s("first_noise_group", &mut r.first_noise_group);
    f.s("first_noise_band", &mut r.first_noise_band);
    f.s("direction", &mut r.direction);
    f.s("bsAnchor", &mut r.bs_anchor);
    f.s("bitstreamIndexRvlFwd", &mut r.bitstream_index_rvl_fwd);
    f.s("bitstreamIndexRvlBwd", &mut r.bitstream_index_rvl_bwd);
    f.s("bitstreamIndexEsc", &mut r.bitstream_index_esc);
    let mut t0: i32 = !r.p_huff_tree_rvl_codewds.is_empty() as i32;
    let mut t1: i32 = !r.p_huff_tree_rvlc_escape.is_empty() as i32;
    f.s("pHuffTreeRvlCodewds", &mut t0);
    f.s("pHuffTreeRvlcEscape", &mut t1);
    if f.load {
        r.p_huff_tree_rvl_codewds = if t0 != 0 { &A_HUFF_TREE_RVL_CODEWDS } else { &[] };
        r.p_huff_tree_rvlc_escape = if t1 != 0 { &A_HUFF_TREE_RVLC_ESCAPE } else { &[] };
    }
    f.s("numDecodedEscapeWordsFwd", &mut r.num_decoded_escape_words_fwd);
    f.s("numDecodedEscapeWordsBwd", &mut r.num_decoded_escape_words_bwd);
    f.s("numDecodedEscapeWordsEsc", &mut r.num_decoded_escape_words_esc);
    f.s("noise_used", &mut r.noise_used);
    f.s("intensity_used", &mut r.intensity_used);
    f.s("sf_used", &mut r.sf_used);
    f.s("firstScf", &mut r.first_scf);
    f.s("lastScf", &mut r.last_scf);
    f.s("firstNrg", &mut r.first_nrg);
    f.s("lastNrg", &mut r.last_nrg);
    f.s("firstIs", &mut r.first_is);
    f.s("lastIs", &mut r.last_is);
    f.s("errorLogRvlc", &mut r.error_log_rvlc);
    f.s("conceal_min", &mut r.conceal_min);
    f.s("conceal_max", &mut r.conceal_max);
    f.s("conceal_min_esc", &mut r.conceal_min_esc);
    f.s("conceal_max_esc", &mut r.conceal_max_esc);
    let a = &mut com.overlay.aac;
    f.a("aRvlcScfEsc", &mut a.a_rvlc_scf_esc);
    f.a("aRvlcScfFwd", &mut a.a_rvlc_scf_fwd);
    f.a("aRvlcScfBwd", &mut a.a_rvlc_scf_bwd);
    f.s("MsMaskPresent", &mut com.joint_stereo_data.ms_mask_present);
}

pub fn xfer_st(f: &mut Fl, p: &mut CAacDecoderStaticChannelInfo) {
    let ci = &mut p.concealment_info;
    f.a("aRvlcPreviousScaleFactor", &mut ci.a_rvlc_previous_scale_factor);
    f.a("aRvlcPreviousCodebook", &mut ci.a_rvlc_previous_codebook);
    f.s("rvlcPreviousScaleFactorOK", &mut ci.rvlc_previous_scale_factor_ok);
    f.s("rvlcPreviousBlockType", &mut ci.rvlc_previous_block_type);
}

/// Lengths and field names of the flat arrays.
pub struct Layout {
    pub n: [usize; 4],
    pub names: [Vec<(usize, &'static str)>; 4],
}

pub fn layout() -> &'static Layout {
    static L: OnceLock<Layout> = OnceLock::new();
    L.get_or_init(|| {
        let mut rs = Rs::new();
        let mut buf = vec![0i32; 1 << 16];
        let mut names: [Vec<(usize, &'static str)>; 4] = Default::default();
        let mut n = [0usize; 4];
        for (k, nm) in names.iter_mut().enumerate() {
            let mut f = Fl { v: &mut buf, pos: 0, load: false, names: Some(nm) };
            match k {
                0 => xfer_ch(&mut f, &mut rs.ch[0]),
                1 => xfer_hcr(&mut f, &mut rs.com.overlay.aac.er_hcr_info),
                2 => xfer_rv(&mut f, &mut rs.com),
                _ => xfer_st(&mut f, &mut rs.st[0]),
            }
            n[k] = f.pos;
        }
        Layout { n, names }
    })
}

// ---------------------------------------------------------------------------------------------
// States on both sides
// ---------------------------------------------------------------------------------------------

/// The Rust state of the context: two channels, their static channel infos, the element's common
/// data and the spectrum (`2 * 1024` values).
pub struct Rs {
    pub ch: Vec<CAacDecoderChannelInfo>,
    pub st: Vec<CAacDecoderStaticChannelInfo>,
    pub com: Box<CAacDecoderCommonData>,
    pub spec: Vec<i32>,
}

impl Rs {
    pub fn new() -> Rs {
        Rs {
            ch: (0..2).map(|_| CAacDecoderChannelInfo::default()).collect(),
            st: (0..2).map(|_| CAacDecoderStaticChannelInfo::default()).collect(),
            com: Box::default(),
            spec: vec![0; 2048],
        }
    }

    /// The HCR state and the channel `ch` with its spectrum.
    pub fn hcr(&mut self, ch: usize) -> (&mut CErHcrInfo, &mut CAacDecoderChannelInfo, &mut [i32]) {
        (&mut self.com.overlay.aac.er_hcr_info, &mut self.ch[ch], &mut self.spec[ch * 1024..])
    }
}

/// The flat state.
#[derive(Clone, Debug, PartialEq)]
pub struct Flat {
    pub ch: [Vec<i32>; 2],
    pub hcr: Vec<i32>,
    pub rv: Vec<i32>,
    pub st: [Vec<i32>; 2],
    pub spec: Vec<i32>,
}

impl Flat {
    pub fn of(rs: &mut Rs) -> Flat {
        let l = layout();
        let mut f = Flat {
            ch: [vec![0; l.n[0]], vec![0; l.n[0]]],
            hcr: vec![0; l.n[1]],
            rv: vec![0; l.n[2]],
            st: [vec![0; l.n[3]], vec![0; l.n[3]]],
            spec: rs.spec.clone(),
        };
        for k in 0..2 {
            xfer_ch(&mut Fl::new(&mut f.ch[k], false), &mut rs.ch[k]);
            xfer_st(&mut Fl::new(&mut f.st[k], false), &mut rs.st[k]);
        }
        xfer_hcr(&mut Fl::new(&mut f.hcr, false), &mut rs.com.overlay.aac.er_hcr_info);
        xfer_rv(&mut Fl::new(&mut f.rv, false), &mut rs.com);
        f
    }

    pub fn to_rs(&self) -> Rs {
        let mut rs = Rs::new();
        let mut f = self.clone();
        for k in 0..2 {
            xfer_ch(&mut Fl::new(&mut f.ch[k], true), &mut rs.ch[k]);
            xfer_st(&mut Fl::new(&mut f.st[k], true), &mut rs.st[k]);
        }
        xfer_hcr(&mut Fl::new(&mut f.hcr, true), &mut rs.com.overlay.aac.er_hcr_info);
        xfer_rv(&mut Fl::new(&mut f.rv, true), &mut rs.com);
        rs.spec.copy_from_slice(&self.spec);
        rs
    }

    pub fn zero() -> Flat {
        Flat::of(&mut Rs::new())
    }
}

/// The first difference of two flat arrays, with the field name.
fn first_diff(what: &str, names: &[(usize, &'static str)], c: &[i32], r: &[i32]) -> Option<String> {
    let i = c.iter().zip(r).position(|(a, b)| a != b)?;
    let (p, n) = names.iter().rev().find(|(p, _)| *p <= i).copied().unwrap_or((0, "?"));
    Some(format!("{what}.{n}[{}]: C {} != Rust {}", i - p, c[i], r[i]))
}

/// Panics with the first difference of two flat states.
pub fn cmp_flat(what: &str, c: &Flat, r: &Flat) {
    let l = layout();
    let spec_names = [(0usize, "spec")];
    let checks: [(&str, &[(usize, &'static str)], &[i32], &[i32]); 7] = [
        ("ch0", &l.names[0], &c.ch[0], &r.ch[0]),
        ("ch1", &l.names[0], &c.ch[1], &r.ch[1]),
        ("hcr", &l.names[1], &c.hcr, &r.hcr),
        ("rv", &l.names[2], &c.rv, &r.rv),
        ("st0", &l.names[3], &c.st[0], &r.st[0]),
        ("st1", &l.names[3], &c.st[1], &r.st[1]),
        ("", &spec_names, &c.spec, &r.spec),
    ];
    for (k, names, a, b) in checks {
        if let Some(d) = first_diff(k, names, a, b) {
            panic!("{what}: {d}");
        }
    }
}

/// The C++ context.
pub struct Ctx(*mut c_void);

impl Ctx {
    pub fn new() -> Ctx {
        Ctx(unsafe { c::cref_aacdec_hcr_ctx_new() })
    }

    pub fn ptr(&self) -> *mut c_void {
        self.0
    }

    pub fn set(&self, f: &Flat) {
        let mut f = f.clone();
        unsafe {
            c::cref_aacdec_hcr_ctx_xfer(
                self.0,
                f.ch[0].as_mut_ptr(),
                f.ch[1].as_mut_ptr(),
                f.hcr.as_mut_ptr(),
                f.rv.as_mut_ptr(),
                f.st[0].as_mut_ptr(),
                f.st[1].as_mut_ptr(),
                f.spec.as_mut_ptr(),
                1,
            )
        };
    }

    pub fn get(&self) -> Flat {
        let mut f = Flat::zero();
        unsafe {
            c::cref_aacdec_hcr_ctx_xfer(
                self.0,
                f.ch[0].as_mut_ptr(),
                f.ch[1].as_mut_ptr(),
                f.hcr.as_mut_ptr(),
                f.rv.as_mut_ptr(),
                f.st[0].as_mut_ptr(),
                f.st[1].as_mut_ptr(),
                f.spec.as_mut_ptr(),
                0,
            )
        };
        f
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        unsafe { c::cref_aacdec_hcr_ctx_free(self.0) };
    }
}

/// A bitstream state (see shim/fdk_bitstream.cpp) and the memory of its bit buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct Bs {
    pub st: [u32; 9],
    pub buf: Vec<u8>,
}

impl Bs {
    /// A reader over `buf` (a power of two of bytes) with all bits valid, positioned at `pos`
    /// bits.
    pub fn new(buf: Vec<u8>, pos: u32) -> Bs {
        let mut b = Bs { st: [0; 9], buf };
        let n = b.buf.len() as u32;
        let mut bs = FdkBitStream::default();
        // SAFETY: the buffer outlives the temporary bitstream.
        fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&b.buf), n, n * 8, BS_READER);
        fdk_push_for(&mut bs, pos);
        b.st = state(&bs);
        b
    }

    /// Random memory of `1 << size_log2` bytes, at a random position.
    pub fn random(rng: &mut Rng, size_log2: u32) -> Bs {
        let n = 1usize << size_log2;
        let buf: Vec<u8> = (0..n).map(|_| rng.next_u32() as u8).collect();
        let pos = rng.next_u32() % (n as u32 * 8);
        Bs::new(buf, pos)
    }

    /// Runs `f` on a Rust bitstream with this state and stores the state afterwards.
    pub fn rs<R>(&mut self, f: impl FnOnce(&mut FdkBitStream) -> R) -> R {
        let mut bs = FdkBitStream::default();
        let mem = BitMem::from_slice(&self.buf);
        fdk_init_bit_stream(&mut bs, &mem, self.st[7], 0, self.st[2]);
        bs.cache_word = self.st[0];
        bs.bits_in_cache = self.st[1];
        bs.h_bit_buf.valid_bits = self.st[3];
        bs.h_bit_buf.read_offset = self.st[4];
        bs.h_bit_buf.write_offset = self.st[5];
        bs.h_bit_buf.bit_ndx = self.st[6];
        assert_eq!(bs.h_bit_buf.buf_bits(), self.st[8]);
        let r = f(&mut bs);
        self.st = state(&bs);
        r
    }

    /// The pointers for the shims.
    pub fn c(&mut self) -> (*mut u32, *mut u8) {
        (self.st.as_mut_ptr(), self.buf.as_mut_ptr())
    }
}

pub fn state(bs: &FdkBitStream) -> [u32; 9] {
    let b = &bs.h_bit_buf;
    [
        bs.cache_word,
        bs.bits_in_cache,
        bs.config_cache,
        b.valid_bits,
        b.read_offset,
        b.write_offset,
        b.bit_ndx,
        b.buf_size(),
        b.buf_bits(),
    ]
}

/// Runs one call on both sides from the same state (`flat`, `bs`) and compares the return values,
/// the states and the bitstreams. Returns the C++ state afterwards (for chaining).
pub fn both(
    what: &str,
    flat: &Flat,
    bs: Option<&Bs>,
    cf: impl FnOnce(*mut c_void, Option<&mut Bs>) -> i64,
    rf: impl FnOnce(&mut Rs, Option<&mut FdkBitStream>) -> i64,
) -> (Flat, Option<Bs>, i64) {
    let ctx = Ctx::new();
    ctx.set(flat);
    let mut cbs = bs.cloned();
    let cr = cf(ctx.ptr(), cbs.as_mut());
    let cout = ctx.get();

    let mut rs = flat.to_rs();
    let mut rbs = bs.cloned();
    let rr = match rbs.as_mut() {
        Some(b) => b.rs(|bs| rf(&mut rs, Some(bs))),
        None => rf(&mut rs, None),
    };
    let rout = Flat::of(&mut rs);

    assert_eq!(cr, rr, "{what}: return value");
    if let (Some(a), Some(b)) = (&cbs, &rbs) {
        assert_eq!(a.st, b.st, "{what}: bitstream state");
    }
    cmp_flat(what, &cout, &rout);
    (cout, cbs, cr)
}

// ---------------------------------------------------------------------------------------------
// Random side info
// ---------------------------------------------------------------------------------------------

pub const SAMPLE_RATES: [u32; 12] =
    [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000];

/// A `SamplingRateInfo` and the arguments of `getSamplingRateInfo()` that give it.
#[derive(Clone, Copy, Debug)]
pub struct Sri {
    pub spf: u32,
    pub idx: u32,
    pub rate: u32,
    pub t: SamplingRateInfo,
}

impl Sri {
    pub fn new(spf: u32, idx: u32, rate: u32) -> Option<Sri> {
        let mut t = SamplingRateInfo::default();
        (get_sampling_rate_info(&mut t, spf, idx, rate) == AAC_DEC_OK)
            .then_some(Sri { spf, idx, rate, t })
    }

    /// A random configuration; with `short`, one with short blocks (frame lengths 1024 and 960).
    pub fn random(rng: &mut Rng, short: bool) -> Sri {
        loop {
            let spf = if short {
                [1024, 960][(rng.next_u32() % 2) as usize]
            } else {
                [1024, 960, 512, 480][(rng.next_u32() % 4) as usize]
            };
            let idx = rng.next_u32() % 12;
            if let Some(s) = Sri::new(spf, idx, SAMPLE_RATES[idx as usize]) {
                if !short || !s.t.scale_factor_bands_short.is_empty() {
                    return s;
                }
            }
        }
    }
}

/// Random individual channel stream info (as IcsRead() produces it).
pub fn random_ics(rng: &mut Rng, t: &SamplingRateInfo, short: bool) -> CIcsInfo {
    let mut ics = CIcsInfo { valid: 1, ..Default::default() };
    if short {
        ics.window_sequence = BLOCK_SHORT;
        ics.total_sf_bands = t.number_of_scale_factor_bands_short;
        ics.scale_factor_grouping = (rng.next_u32() & 0x7F) as u8;
        ics.window_groups = 0;
        for i in 0..7 {
            ics.window_group_length[i] = 1;
            if ics.scale_factor_grouping as u32 & (1 << (6 - i)) != 0 {
                ics.window_group_length[ics.window_groups as usize] += 1;
            } else {
                ics.window_groups += 1;
            }
        }
        ics.window_group_length[7] = 1;
        ics.window_groups += 1;
    } else {
        ics.window_sequence = [BLOCK_LONG, BLOCK_START, BLOCK_STOP][(rng.next_u32() % 3) as usize];
        ics.total_sf_bands = t.number_of_scale_factor_bands_long;
        ics.window_groups = 1;
        ics.window_group_length[0] = 1;
    }
    ics.max_sf_bands = match rng.next_u32() % 4 {
        0 => (rng.next_u32() % (ics.total_sf_bands as u32 + 1)) as u8,
        _ => ics.total_sf_bands - (rng.next_u32() % 3).min(ics.total_sf_bands as u32) as u8,
    };
    ics.window_shape = (rng.next_u32() % 2) as u8;
    ics
}

/// Random section data as CBlock_ReadSectionData() stores it for HCR (`aCodeBook` and, for
/// HCR, `aNumLineInSec4Hcr`, `aCodeBooks4Hcr` and `numberSection`), with `vcb11` codebooks 16 to
/// 31 and with intensity codebooks only for a common window.
pub fn random_sections(rng: &mut Rng, p: &mut CAacDecoderChannelInfo, t: &SamplingRateInfo, vcb11: bool) {
    let ics = p.ics_info;
    let d = &mut *p.p_dyn_data;
    d.a_code_book = [0; 128];
    d.specific_to.aac.number_section = 0;
    let offsets = get_scale_factor_band_offsets(&ics, t);
    let total = get_number_of_scale_factor_bands(&ics, t) as i32;
    let few = rng.next_u32() % 2 == 0; /* few or many codebooks */
    let mut idx = 0usize;
    for group in 0..ics.window_groups as usize {
        let mut band = 0i32;
        while band < ics.max_sf_bands as i32 {
            let cb = loop {
                let cb = if few {
                    [0u8, 1, 5, 11, 7][(rng.next_u32() % 5) as usize]
                } else if vcb11 {
                    (rng.next_u32() % 32) as u8
                } else {
                    (rng.next_u32() % 16) as u8
                };
                if cb == BOOKSCL {
                    continue;
                }
                if (cb == INTENSITY_HCB || cb == INTENSITY_HCB2) && d.raw_data_info.common_window == 0
                {
                    continue;
                }
                break cb;
            };
            let sect_len = if vcb11 && (cb == 11 || cb >= 16) {
                1
            } else {
                match rng.next_u32() % 3 {
                    0 => 1 + (rng.next_u32() % (total - band) as u32) as i32,
                    _ => 1 + (rng.next_u32() % 4) as i32,
                }
                .min(total - band)
            };
            let top = band + sect_len;
            if idx < MAX_SFB_HCR as usize {
                d.specific_to.aac.a_num_line_in_sec4_hcr[idx] =
                    offsets[top as usize] - offsets[band as usize];
                d.specific_to.aac.a_code_books4_hcr[idx] = cb;
                idx += 1;
                d.specific_to.aac.number_section = d.specific_to.aac.number_section.wrapping_add(1);
            }
            for b in band..top {
                d.a_code_book[group * 16 + b as usize] = cb;
            }
            band = top;
        }
    }
}

/// One random HCR frame: side info (sometimes corrupt), lengths, scale factors and spectrum, on
/// channel `ch` of `rs`.
pub fn random_hcr_frame(rng: &mut Rng, rs: &mut Rs, ch: usize, vcb11: bool) -> Sri {
    let short = rng.next_u32() % 3 == 0;
    let sri = Sri::random(rng, short);
    let p = &mut rs.ch[ch];
    p.ics_info = random_ics(rng, &sri.t, short);
    p.granule_length = (sri.spf / 8) as i32;
    p.p_dyn_data.raw_data_info.common_window = (rng.next_u32() % 2) as u8;
    p.p_dyn_data.raw_data_info.global_gain = rng.next_u32() as u8;
    random_sections(rng, p, &sri.t, vcb11);
    let d = &mut *p.p_dyn_data;
    for sf in d.a_scale_factor.iter_mut() {
        *sf = (rng.next_u32() % 300) as i16 - 100;
    }
    let a = &mut d.specific_to.aac;
    a.len_of_reordered_spectral_data = match rng.next_u32() % 4 {
        0 => (rng.next_u32() % 6145) as i16,
        1 => (rng.next_u32() % 64) as i16,
        _ => (rng.next_u32() % 3000) as i16,
    };
    a.len_of_longest_codeword = match rng.next_u32() % 4 {
        0 => (rng.next_u32() % 50) as i8,
        _ => 49 - (rng.next_u32() % 20) as i8,
    };
    /* side info that HcrInit() rejects (or, for numberSection, fewer sections). The line counts
    are only decreased (inputs with a defined reference result). */
    let ns = a.number_section.max(1) as u32;
    match rng.next_u32() % 12 {
        0 => a.number_section = (rng.next_u32() % (ns + 1)) as i8,
        1 => {
            let i = (rng.next_u32() % ns) as usize;
            a.a_code_books4_hcr[i] = [12, 32 + rng.next_u32() % 224][(rng.next_u32() % 2) as usize] as u8;
        }
        2 => {
            let i = (rng.next_u32() % ns) as usize;
            let v = a.a_num_line_in_sec4_hcr[i] as i32;
            a.a_num_line_in_sec4_hcr[i] = ((rng.next_u32() % (v.max(0) as u32 + 21)) as i32 - 20) as i16;
        }
        3 => {
            let i = (rng.next_u32() % 128) as usize;
            d.a_code_book[i] = rng.next_u32() as u8;
        }
        _ => {}
    }
    for v in rs.spec[ch * 1024..ch * 1024 + 1024].iter_mut() {
        *v = match rng.next_u32() % 4 {
            0 => rng.i32(),
            _ => 0,
        };
    }
    sri
}

// ---------------------------------------------------------------------------------------------
// Calls on both sides
// ---------------------------------------------------------------------------------------------

pub fn call_hcr_init(what: &str, flat: &Flat, bs: &Bs, ch: usize, sri: &Sri) -> (Flat, Bs, u32) {
    let (f, b, r) = both(
        what,
        flat,
        Some(bs),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            (unsafe { c::cref_aacdec_hcr_HcrInit(ctx, ch as i32, sri.spf, sri.idx, sri.rate, st, buf) })
                as i64
        },
        |rs, b| {
            let (h, p, _) = rs.hcr(ch);
            hcr_init(h, p, &sri.t, b.unwrap()) as i64
        },
    );
    (f, b.unwrap(), r as u32)
}

pub fn call_hcr_decoder(what: &str, flat: &Flat, bs: &Bs, ch: usize, sri: &Sri) -> (Flat, Bs, u32) {
    let (f, b, r) = both(
        what,
        flat,
        Some(bs),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            (unsafe {
                c::cref_aacdec_hcr_HcrDecoder(ctx, ch as i32, sri.spf, sri.idx, sri.rate, st, buf)
            }) as i64
        },
        |rs, b| {
            let (h, p, s) = rs.hcr(ch);
            hcr_decoder(h, p, &sri.t, b.unwrap(), s) as i64
        },
    );
    (f, b.unwrap(), r as u32)
}

pub fn call_hcr_mute(what: &str, flat: &Flat, ch: usize) -> Flat {
    both(
        what,
        flat,
        None,
        |ctx, _| {
            unsafe { c::cref_aacdec_hcr_HcrMuteErroneousLines(ctx, ch as i32) };
            0
        },
        |rs, _| {
            let (h, _, s) = rs.hcr(ch);
            hcr_mute_erroneous_lines(h, s);
            0
        },
    )
    .0
}

pub fn call_chcr_read(what: &str, flat: &Flat, bs: &Bs, ch: usize, ty: i32) -> (Flat, Bs) {
    let (f, b, _) = both(
        what,
        flat,
        Some(bs),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            unsafe { c::cref_aacdec_hcr_CHcr_Read(ctx, ch as i32, st, buf, ty) };
            0
        },
        |rs, b| {
            chcr_read(b.unwrap(), &mut rs.ch[ch], ty);
            0
        },
    );
    (f, b.unwrap())
}

/// The steps of HcrDecoder() one by one, each on both sides from the C++ state after the previous
/// step (`flat` is the state before HcrDecoder()).
pub fn hcr_decoder_steps(what: &str, flat: &Flat, bs: &Bs, ch: usize, sri: &Sri) {
    let chi = ch as i32;
    macro_rules! step {
        ($name:expr, $f:expr, $cfn:ident, $rfn:expr) => {{
            both(
                &format!("{what}: {}", $name),
                $f,
                None,
                |ctx, _| {
                    unsafe { c::$cfn(ctx, chi) };
                    0
                },
                $rfn,
            )
            .0
        }};
    }
    let f = step!("HcrCalcNumCodeword", flat, cref_aacdec_hcr_HcrCalcNumCodeword, |rs, _| {
        let a = rs.ch[ch].p_dyn_data.specific_to.aac.clone();
        hcr_calc_num_codeword(
            &mut rs.com.overlay.aac.er_hcr_info,
            &a.a_code_books4_hcr,
            &a.a_num_line_in_sec4_hcr,
        );
        0
    });
    let f = step!(
        "HcrSortCodebookAndNumCodewordInSection",
        &f,
        cref_aacdec_hcr_HcrSortCodebookAndNumCodewordInSection,
        |rs, _| {
            let a = rs.ch[ch].p_dyn_data.specific_to.aac.clone();
            hcr_sort_codebook_and_num_codeword_in_section(
                &mut rs.com.overlay.aac.er_hcr_info,
                &a.a_code_books4_hcr,
            );
            0
        }
    );
    let f = step!(
        "HcrPrepareSegmentationGrid",
        &f,
        cref_aacdec_hcr_HcrPrepareSegmentationGrid,
        |rs, _| {
            hcr_prepare_segmentation_grid(&mut rs.com.overlay.aac.er_hcr_info);
            0
        }
    );
    let f = step!("HcrExtendedSectionInfo", &f, cref_aacdec_hcr_HcrExtendedSectionInfo, |rs, _| {
        hcr_extended_section_info(&mut rs.com.overlay.aac.er_hcr_info);
        0
    });
    let hi = f.clone().to_rs().com.overlay.aac.er_hcr_info.dec_in_out.error_log;
    if hi & HCR_FATAL_PCW_ERROR_MASK != 0 {
        return;
    }
    let f = step!(
        "DeriveNumberOfExtendedSortedSectionsInSets",
        &f,
        cref_aacdec_hcr_DeriveNumberOfExtendedSortedSectionsInSets,
        |rs, _| {
            let h = &mut rs.com.overlay.aac.er_hcr_info;
            let si = &mut h.section_info;
            derive_number_of_extended_sorted_sections_in_sets(
                h.segment_info.num_segment,
                &si.p_num_extended_sorted_codeword_in_section,
                si.num_extended_sorted_codeword_in_section_idx,
                &mut si.p_num_extended_sorted_sections_in_sets,
                si.num_extended_sorted_sections_in_sets_idx,
            );
            0
        }
    );
    let (f, b, _) = both(
        &format!("{what}: DecodePCWs"),
        &f,
        Some(bs),
        |ctx, b| {
            let (st, buf) = b.unwrap().c();
            unsafe { c::cref_aacdec_hcr_DecodePCWs(ctx, chi, st, buf) };
            0
        },
        |rs, b| {
            let (h, _, s) = rs.hcr(ch);
            decode_pcws(b.unwrap(), h, s);
            0
        },
    );
    let b = b.unwrap();
    let hi = f.clone().to_rs().com.overlay.aac.er_hcr_info.dec_in_out.error_log;
    let f = if hi & HCR_FATAL_PCW_ERROR_MASK == 0 {
        super::aacdec_hcrs::call_decode_non_pcws(&format!("{what}: DecodeNonPCWs"), &f, &b, ch).0
    } else {
        f
    };
    let f = step!(
        "errDetectWithinSegmentationFinal",
        &f,
        cref_aacdec_hcr_errDetectWithinSegmentationFinal,
        |rs, _| {
            err_detect_within_segmentation_final(&mut rs.com.overlay.aac.er_hcr_info);
            0
        }
    );
    both(
        &format!("{what}: HcrReorderQuantizedSpectralCoefficients"),
        &f,
        None,
        |ctx, _| {
            unsafe {
                c::cref_aacdec_hcr_HcrReorderQuantizedSpectralCoefficients(
                    ctx, chi, sri.spf, sri.idx, sri.rate,
                )
            };
            0
        },
        |rs, _| {
            let (h, p, s) = rs.hcr(ch);
            hcr_reorder_quantized_spectral_coefficients(h, p, &sri.t, s);
            0
        },
    );
}

/// A sequence of random frames on one HCR state: HcrInit(), then HcrDecoder() (and
/// HcrMuteErroneousLines() on errors) as CBlock_ReadSpectralData() calls them, each compared,
/// plus the steps of HcrDecoder().
pub fn hcr_random_sequence(rng: &mut Rng, frames: usize, vcb11: bool, steps: bool) -> (u32, u32) {
    let mut rs = Rs::new();
    let mut flat = Flat::of(&mut rs);
    let (mut decoded, mut errors) = (0, 0);
    for fr in 0..frames {
        let ch = (rng.next_u32() % 2) as usize;
        let mut rs = flat.to_rs();
        let sri = random_hcr_frame(rng, &mut rs, ch, vcb11);
        let f = Flat::of(&mut rs);
        let bs = Bs::random(rng, 12);
        let what = format!("frame {fr}");
        let (f, bs, r) = call_hcr_init(&format!("{what}: HcrInit"), &f, &bs, ch, &sri);
        flat = f;
        if r != 0 {
            continue;
        }
        if steps {
            hcr_decoder_steps(&what, &flat, &bs, ch, &sri);
        }
        let (f, _, r) = call_hcr_decoder(&format!("{what}: HcrDecoder"), &flat, &bs, ch, &sri);
        flat = f;
        decoded += 1;
        if r != 0 {
            errors += 1;
            flat = call_hcr_mute(&format!("{what}: HcrMuteErroneousLines"), &flat, ch);
        }
    }
    (decoded, errors)
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn layout_matches() {
    let mut out = [0i32; 8];
    let n = unsafe { c::cref_aacdec_hcr_layout(out.as_mut_ptr()) } as usize;
    assert_eq!(n, 5);
    let l = layout();
    assert_eq!(&out[..4], &l.n.map(|x| x as i32)[..], "flat array lengths");
    // the C layout, which the port reproduces (write_code_books4_hcr)
    assert_eq!(out[4], MAX_SFB_HCR, "lenOfReorderedSpectralData follows aCodeBooks4Hcr");
}

#[test]
fn flat_roundtrip() {
    // Random flat states survive C -> flat -> C and Rust -> flat -> Rust.
    let mut rng = Rng::new(0x11c);
    for _ in 0..20 {
        let mut f = Flat::zero();
        for v in f.ch.iter_mut().chain(f.st.iter_mut()) {
            for x in v.iter_mut() {
                *x = (rng.next_u32() % 100) as i32;
            }
        }
        for x in f.hcr.iter_mut() {
            *x = (rng.next_u32() % 7) as i32;
        }
        for x in f.rv.iter_mut() {
            *x = (rng.next_u32() % 2) as i32;
        }
        let ctx = Ctx::new();
        ctx.set(&f);
        cmp_flat("C roundtrip", &ctx.get(), &f);
        cmp_flat("Rust roundtrip", &Flat::of(&mut f.to_rs()), &f);
    }
}

#[test]
fn chcr_read_random() {
    let mut rng = Rng::new(0xc4c);
    let mut rs = Rs::new();
    for i in 0..2000 {
        let ch = (rng.next_u32() % 2) as usize;
        let a = &mut rs.ch[ch].p_dyn_data.specific_to.aac;
        a.len_of_reordered_spectral_data = rng.i16();
        a.len_of_longest_codeword = rng.next_u32() as i8;
        let ty = [ID_SCE, ID_CPE, ID_LFE, ID_CCE, ID_DSE, -1][(rng.next_u32() % 6) as usize];
        let bs = Bs::random(&mut rng, 6);
        let (f, _) = call_chcr_read(&format!("CHcr_Read {i}"), &Flat::of(&mut rs), &bs, ch, ty);
        rs = f.to_rs();
    }
}

#[test]
fn hcr_init_and_decoder_random() {
    let mut rng = Rng::new(0x4c2);
    let (mut decoded, mut errors) = (0, 0);
    for seq in 0..40 {
        let (d, e) = hcr_random_sequence(&mut rng, 25, seq % 2 == 1, false);
        decoded += d;
        errors += e;
    }
    // Both the error-free and the error paths are reached.
    assert!(decoded > 100 && errors > 20 && errors < decoded, "{decoded} decoded, {errors} errors");
}

#[test]
fn hcr_decoder_steps_random() {
    let mut rng = Rng::new(0x57e9);
    for seq in 0..12 {
        hcr_random_sequence(&mut rng, 15, seq % 2 == 0, true);
    }
}

#[test]
fn hcr_init_codebooks_layout() {
    // HcrInit() with the C layout of aCodeBooks4Hcr and lenOfReorderedSpectralData, which the
    // port reproduces.
    let mut rng = Rng::new(0x256);
    let sri = Sri::new(1024, 4, 44100).unwrap();
    for i in 0..20 {
        let mut rs = Rs::new();
        let p = &mut rs.ch[0];
        p.ics_info = random_ics(&mut rng, &sri.t, true);
        p.ics_info.scale_factor_grouping = 0;
        p.ics_info.window_groups = 8;
        p.ics_info.window_group_length = [1; 8];
        p.ics_info.max_sf_bands = sri.t.number_of_scale_factor_bands_short;
        p.granule_length = 128;
        for g in 0..8 {
            for b in 0..16 {
                // alternating codebooks in every window: a change in every unit
                p.p_dyn_data.a_code_book[g * 16 + b] = if g % 2 == 0 { 1 + (i % 5) as u8 } else { 7 };
            }
        }
        let a = &mut p.p_dyn_data.specific_to.aac;
        a.len_of_reordered_spectral_data = 0x1234 + i as i16;
        a.len_of_longest_codeword = 20;
        let bs = Bs::random(&mut rng, 12);
        let (f, bs, r) = call_hcr_init(&format!("256 sections {i}"), &Flat::of(&mut rs), &bs, 0, &sri);
        let rsf = f.to_rs();
        assert_eq!(rsf.com.overlay.aac.er_hcr_info.dec_in_out.num_section, 256);
        assert_eq!(rsf.ch[0].p_dyn_data.specific_to.aac.len_of_reordered_spectral_data & 0xFF, 7);
        if r == 0 {
            hcr_decoder_steps("256 sections", &f, &bs, 0, &sri);
            call_hcr_decoder("256 sections", &f, &bs, 0, &sri);
        }
    }
}

#[test]
fn hcr_mute_erroneous_lines_random() {
    let mut rng = Rng::new(0x3a7e);
    for i in 0..200 {
        let mut f = Flat::zero();
        for v in f.spec.iter_mut() {
            *v = match rng.next_u32() % 4 {
                0 => Q_VALUE_INVALID,
                1 => -Q_VALUE_INVALID,
                2 => rng.i32(),
                _ => 0,
            };
        }
        call_hcr_mute(&format!("mute {i}"), &f, (rng.next_u32() % 2) as usize);
    }
}

#[test]
fn carry_bit_to_branch_value_random() {
    let mut rng = Rng::new(0xcb);
    for _ in 0..10000 {
        let carry = rng.next_u32() % 3;
        let node = rng.next_u32();
        let mut out = [0u32; 2];
        unsafe { c::cref_aacdec_hcr_CarryBitToBranchValue(carry, node, out.as_mut_ptr()) };
        let (mut bv, mut bn) = (0, 0);
        carry_bit_to_branch_value(carry as u8, node, &mut bv, &mut bn);
        assert_eq!(out, [bv, bn], "CarryBitToBranchValue({carry}, {node:#x})");
    }
}

#[test]
fn err_detectors() {
    let mut rng = Rng::new(0xed);
    for _ in 0..10000 {
        let cb = rng.next_u32() as i8;
        let nl = rng.i16();
        let e0 = rng.next_u32();
        let mut e = e0;
        err_detector_in_hcr_sideinfo_shrt(cb, nl, &mut e);
        let ce = unsafe { c::cref_aacdec_hcr_errDetectorInHcrSideinfoShrt(cb as i32, nl as i32, e0) };
        assert_eq!(ce, e, "errDetectorInHcrSideinfoShrt({cb}, {nl})");
        let lolc = rng.next_u32() as i8;
        let lorsd = rng.i16();
        let mut e = e0;
        err_detector_in_hcr_lengths(lolc, lorsd, &mut e);
        let ce = unsafe { c::cref_aacdec_hcr_errDetectorInHcrLengths(lolc as i32, lorsd as i32, e0) };
        assert_eq!(ce, e, "errDetectorInHcrLengths({lolc}, {lorsd})");
    }
}

#[test]
fn err_detect_pcw_segmentation_random() {
    let mut rng = Rng::new(0xe5);
    for i in 0..500 {
        let mut f = Flat::zero();
        for v in f.spec.iter_mut() {
            *v = rng.i32();
        }
        let rem = rng.next_u32() as i8;
        let kind = (rng.next_u32() % 4) as i32;
        let dim = [1u8, 2, 4][(rng.next_u32() % 3) as usize];
        let off = (rng.next_u32() % (1024 - 4)) as i32;
        let ch = (rng.next_u32() % 2) as usize;
        both(
            &format!("errDetectPcwSegmentation {i}"),
            &f,
            None,
            |ctx, _| unsafe {
                c::cref_aacdec_hcr_errDetectPcwSegmentation(ctx, ch as i32, rem as i32, kind, off, dim as i32)
                    as i64
            },
            |rs, _| {
                let (h, _, s) = rs.hcr(ch);
                err_detect_pcw_segmentation(rem, h, kind, s, off as isize, dim) as i64
            },
        );
    }
}

#[test]
fn decode_pcw_body_sign_escape_random() {
    let mut rng = Rng::new(0xb0d7);
    for i in 0..3000 {
        let mut bs = Bs::random(&mut rng, 8);
        let anchor = (rng.next_u32() % 4096) as i32;
        let cb = loop {
            let cb = (rng.next_u32() % 32) as usize;
            if cb != 12 {
                break cb;
            }
        };
        let left = (rng.next_u32() % 2048) as i32 - 1024;
        let rem = (rng.next_u32() % 100) as i8 - 10;
        let ndb = (rng.next_u32() % 600) as i32;
        // DecodePCW_Body
        let mut io = [left, rem as i32, ndb];
        let mut cbs = bs.clone();
        let (st, buf) = cbs.c();
        let cq = unsafe { c::cref_aacdec_hcr_DecodePCW_Body(st, buf, anchor, cb as i32, io.as_mut_ptr()) };
        let (mut l, mut r, mut n) = (left, rem, ndb);
        let rq = bs.rs(|b| decode_pcw_body(b, anchor, A_HUFF_TABLE[cb], &mut l, &mut r, &mut n));
        assert_eq!((cq as usize, io), (rq, [l, r as i32, n]), "DecodePCW_Body {i}");
        assert_eq!(cbs.st, bs.st, "DecodePCW_Body {i}: bitstream");
        // DecodePCW_Sign on the decoded values
        let dim = A_DIM_CB[cb] as usize;
        let qv: Vec<i32> = (0..4).map(|k| A_QUANT_TABLE[cb].get(rq + k).copied().unwrap_or(0) as i32).collect();
        let mut cspec: Vec<i32> = (0..1024).map(|_| rng.i32()).collect();
        let mut rspec = cspec.clone();
        let idx = (rng.next_u32() % 1030) as i32 - 3;
        let idx = idx.clamp(0, 1023);
        let mut io = [l, r as i32, n, idx];
        let cr = unsafe {
            c::cref_aacdec_hcr_DecodePCW_Sign(st, buf, anchor, dim as i32, qv.as_ptr(), cspec.as_mut_ptr(), io.as_mut_ptr())
        };
        let mut ri = idx;
        let qvs: Vec<i8> = qv.iter().map(|&v| v as i8).collect();
        let rr = bs.rs(|b| decode_pcw_sign(b, anchor, dim as u32, &qvs, &mut rspec, &mut ri, &mut l, &mut r, &mut n));
        assert_eq!((cr, io), (rr, [l, r as i32, n, ri]), "DecodePCW_Sign {i}");
        assert_eq!(cspec, rspec, "DecodePCW_Sign {i}: spectrum");
        assert_eq!(cbs.st, bs.st, "DecodePCW_Sign {i}: bitstream");
        // DecodeEscapeSequence
        let q = [16, -16, 0, 5][(rng.next_u32() % 4) as usize];
        let err0 = rng.next_u32();
        let mut io = [l, r as i32, n, err0 as i32];
        let cr = unsafe { c::cref_aacdec_hcr_DecodeEscapeSequence(st, buf, anchor, q, io.as_mut_ptr()) };
        let mut e = err0;
        let rr = bs.rs(|b| decode_escape_sequence(b, anchor, q, &mut l, &mut r, &mut n, &mut e));
        assert_eq!((cr, io), (rr, [l, r as i32, n, e as i32]), "DecodeEscapeSequence {i}");
        assert_eq!(cbs.st, bs.st, "DecodeEscapeSequence {i}: bitstream");
    }
}

#[test]
fn get_scale_from_value_all() {
    // HCR passes values of 0 to 8191 (MAX_QUANTIZED_VALUE) and lsb 0 to 3.
    for lsb in 0..4u32 {
        for v in 0..=MAX_QUANTIZED_VALUE {
            let cr = unsafe { c::cref_aacdec_hcr_GetScaleFromValue(v, lsb) };
            assert_eq!(cr, get_scale_from_value(v, lsb), "GetScaleFromValue({v}, {lsb})");
        }
    }
}

#[test]
fn huffman_trees_stay_inside_tables() {
    // Every branch of the HCR trees stays inside its tree and every leaf (plus the codebook
    // dimension) inside the quantized value table, so the direct table indexing of the port
    // cannot panic.
    for cb in 0..32 {
        let tree = A_HUFF_TABLE[cb];
        for &node in tree {
            for br in [(node & 0xFFF000) >> 12, node & 0xFFF] {
                let v = (br & 0x3FF) as usize;
                if br & 0x400 != 0 {
                    assert!(v + A_DIM_CB[cb] as usize <= A_QUANT_TABLE[cb].len(), "cb {cb} leaf {v}");
                } else {
                    assert!(v < tree.len(), "cb {cb} node {v}");
                }
            }
        }
    }
}
