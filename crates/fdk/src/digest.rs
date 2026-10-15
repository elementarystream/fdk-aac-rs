// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the state digest of the chunked parallel decoding.

//! Digest of the complete decoder state (not in C).
//!
//! The decoder is deterministic: its future output depends only on its state and its future
//! input. The chunked parallel decoding (`fdk_aac_rs::chunked`) decodes ranges of a file in
//! parallel, each range with its own decoder that starts some access units earlier (warm-up); a
//! range boundary is accepted only if the state of the decoder of the next range equals the state
//! of the decoder that decoded up to the boundary, which is checked by comparing their digests.
//!
//! [`StateDigest`] feeds a value to a [`Digester`]. It is implemented for the primitive types,
//! arrays, slices, `Vec`, `Box`, `Option`, tuples and references (by address: a reference in the
//! decoder state points to a constant table), and for the decoder structures in a `digest`
//! module of each crate, with `impl_state_digest!`: the structure is destructured exhaustively,
//! so a new field does not compile until it is listed (fed) or skipped (with a comment why it
//! cannot influence the future output). Types holding raw pointers or ring buffers implement the
//! trait by hand.
//!
//! The digest is a 128 bit hash (two independent chains of the `wyhash` multiply-fold mixing over
//! 16 byte blocks), not a cryptographic hash: two different states have the same digest with a
//! probability of about 2^-128 for unrelated inputs (collisions would have to be constructed).
//!
//! [`Digester::traced`] additionally records the digest of every field (up to a depth), so that the
//! fields in which two decoder states differ can be listed ([`diff_traces`]).

use crate::bitbuffer::FdkBitBuf;
use crate::bitstream::FdkBitStream;

/// Secrets of the mixing function (the `wyhash` default secret).
const S: [u64; 4] =
    [0xa076_1d64_78bd_642f, 0xe703_7ed1_a0b4_28db, 0x8ebc_6af0_9c88_c6e3, 0x5899_65cc_7537_4cc3];

#[inline(always)]
fn mum(a: u64, b: u64) -> u64 {
    let r = (a as u128).wrapping_mul(b as u128);
    (r as u64) ^ ((r >> 64) as u64)
}

/// Hash chain state.
#[derive(Clone, Copy)]
struct Lanes {
    h0: u64,
    h1: u64,
    pending: [u8; 16],
    npending: usize,
    total: u64,
}

impl Lanes {
    const fn new() -> Lanes {
        Lanes { h0: S[0], h1: S[1], pending: [0; 16], npending: 0, total: 0 }
    }

    #[inline(always)]
    fn block(&mut self, x0: u64, x1: u64) {
        self.h0 = mum(x0 ^ S[1], x1 ^ self.h0);
        self.h1 = mum(x0 ^ S[2], x1 ^ self.h1 ^ S[3]);
    }

    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        self.total = self.total.wrapping_add(bytes.len() as u64);
        if self.npending > 0 {
            let n = (16 - self.npending).min(bytes.len());
            self.pending[self.npending..self.npending + n].copy_from_slice(&bytes[..n]);
            self.npending += n;
            bytes = &bytes[n..];
            if self.npending < 16 {
                return;
            }
            let p = self.pending;
            self.block(
                u64::from_le_bytes(p[..8].try_into().unwrap()),
                u64::from_le_bytes(p[8..].try_into().unwrap()),
            );
            self.npending = 0;
        }
        let mut chunks = bytes.chunks_exact(16);
        for c in &mut chunks {
            self.block(
                u64::from_le_bytes(c[..8].try_into().unwrap()),
                u64::from_le_bytes(c[8..].try_into().unwrap()),
            );
        }
        let r = chunks.remainder();
        self.pending[..r.len()].copy_from_slice(r);
        self.npending = r.len();
    }

    fn finish(mut self) -> [u64; 2] {
        let mut p = [0u8; 16];
        p[..self.npending].copy_from_slice(&self.pending[..self.npending]);
        self.block(
            u64::from_le_bytes(p[..8].try_into().unwrap()),
            u64::from_le_bytes(p[8..].try_into().unwrap()),
        );
        self.block(self.total, self.npending as u64);
        [mum(self.h0 ^ S[0], self.h1 ^ S[3]), mum(self.h1 ^ S[1], self.h0 ^ S[2])]
    }
}

/// The digest of a decoder state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Digest(pub [u64; 2]);

impl core::fmt::Display for Digest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:016x}{:016x}", self.0[0], self.0[1])
    }
}

struct Trace {
    max_depth: usize,
    path: String,
    /// Parent states and path lengths of the entered fields.
    stack: Vec<(Lanes, usize)>,
    entries: Vec<(String, Digest)>,
}

/// Collects the digest of a state (see the [module documentation](self)).
pub struct Digester {
    lanes: Lanes,
    trace: Option<Box<Trace>>,
}

impl Default for Digester {
    fn default() -> Self {
        Digester::new()
    }
}

impl Digester {
    /// A digester computing the digest only.
    pub const fn new() -> Digester {
        Digester { lanes: Lanes::new(), trace: None }
    }

    /// A digester that also records the digest of every field up to `max_depth` levels (see
    /// [`Digester::finish_traced`]). The digest differs from the one of [`Digester::new`].
    pub fn traced(max_depth: usize) -> Digester {
        Digester {
            lanes: Lanes::new(),
            trace: Some(Box::new(Trace {
                max_depth,
                path: String::new(),
                stack: Vec::new(),
                entries: Vec::new(),
            })),
        }
    }

    /// Number of bytes fed so far (at the current level of a traced digester).
    pub fn bytes_fed(&self) -> u64 {
        self.lanes.total
    }

    /// The digest of everything fed so far.
    pub fn finish(self) -> Digest {
        Digest(self.lanes.finish())
    }

    /// The digest and the recorded digests of the fields: `(path, digest)` in depth-first
    /// order, each field after its children.
    pub fn finish_traced(self) -> (Digest, Vec<(String, Digest)>) {
        let entries = self.trace.map(|t| t.entries).unwrap_or_default();
        (Digest(self.lanes.finish()), entries)
    }

    /// Feeds raw bytes.
    #[inline]
    pub fn bytes(&mut self, b: &[u8]) {
        self.lanes.write(b);
    }

    /// Feeds a `u32` (e.g. an enum variant index).
    #[inline]
    pub fn u32(&mut self, v: u32) {
        self.lanes.write(&v.to_le_bytes());
    }

    /// Feeds a `u64` (e.g. a length).
    #[inline]
    pub fn u64(&mut self, v: u64) {
        self.lanes.write(&v.to_le_bytes());
    }

    #[inline]
    fn enter(&mut self, name: &dyn core::fmt::Display) -> bool {
        let Some(t) = self.trace.as_deref_mut() else { return false };
        if t.stack.len() >= t.max_depth {
            return false;
        }
        let len = t.path.len();
        if len > 0 {
            t.path.push('.');
        }
        use core::fmt::Write;
        let _ = write!(t.path, "{name}");
        t.stack.push((self.lanes, len));
        self.lanes = Lanes::new();
        true
    }

    #[inline]
    fn leave(&mut self) {
        let t = self.trace.as_deref_mut().expect("trace");
        let (parent, len) = t.stack.pop().expect("entered field");
        let d = Digest(core::mem::replace(&mut self.lanes, parent).finish());
        t.entries.push((t.path.clone(), d));
        t.path.truncate(len);
        self.lanes.write(&d.0[0].to_le_bytes());
        self.lanes.write(&d.0[1].to_le_bytes());
    }

    /// Feeds the field `name` of a structure.
    #[inline]
    pub fn field<T: StateDigest + ?Sized>(&mut self, name: &'static str, v: &T) {
        if self.trace.is_none() {
            v.digest(self);
        } else {
            let e = self.enter(&name);
            v.digest(self);
            if e {
                self.leave();
            }
        }
    }

    /// Feeds the field `name` with the function `f`.
    #[inline]
    pub fn field_with(&mut self, name: &'static str, f: impl FnOnce(&mut Digester)) {
        if self.trace.is_none() {
            f(self);
        } else {
            let e = self.enter(&name);
            f(self);
            if e {
                self.leave();
            }
        }
    }

    /// Feeds the element `index` of a sequence.
    #[inline]
    pub fn element<T: StateDigest + ?Sized>(&mut self, index: usize, v: &T) {
        if self.trace.is_none() {
            v.digest(self);
        } else {
            let e = self.enter(&format_args!("[{index}]"));
            v.digest(self);
            if e {
                self.leave();
            }
        }
    }
}

/// The paths of the fields whose digests differ in two traces of [`Digester::finish_traced`],
/// only the innermost ones (a differing field is not listed if one of its fields differs), with
/// the fields that exist in only one of the traces.
pub fn diff_traces(a: &[(String, Digest)], b: &[(String, Digest)]) -> Vec<String> {
    let mb: std::collections::HashMap<&str, Digest> =
        b.iter().map(|(p, d)| (p.as_str(), *d)).collect();
    let ma: std::collections::HashSet<&str> = a.iter().map(|(p, _)| p.as_str()).collect();
    let mut diff: Vec<&str> = Vec::new();
    for (p, d) in a {
        match mb.get(p.as_str()) {
            Some(x) if x == d => {}
            _ => diff.push(p),
        }
    }
    for (p, _) in b {
        if !ma.contains(p.as_str()) {
            diff.push(p);
        }
    }
    let is_parent = |p: &str, q: &str| {
        q.len() > p.len() && q.starts_with(p) && matches!(q.as_bytes()[p.len()], b'.')
    };
    let set: Vec<&str> = diff.clone();
    diff.retain(|p| !set.iter().any(|q| is_parent(p, q)));
    diff.sort_unstable();
    diff.dedup();
    diff.into_iter().map(String::from).collect()
}

/// A value that is part of the decoder state (see the [module documentation](self)).
pub trait StateDigest {
    /// Feeds the value to `d`.
    fn digest(&self, d: &mut Digester);

    /// Feeds the elements of a slice (primitive types feed the memory at once).
    #[inline]
    fn digest_slice(s: &[Self], d: &mut Digester)
    where
        Self: Sized,
    {
        for (i, x) in s.iter().enumerate() {
            d.element(i, x);
        }
    }
}

/// The digest of `v`.
pub fn digest_of<T: StateDigest + ?Sized>(v: &T) -> Digest {
    let mut d = Digester::new();
    v.digest(&mut d);
    d.finish()
}

/// Implements [`StateDigest`] for a structure: feeds the listed fields in order (by their name in
/// a trace); the fields after `skip` are not fed (each with a comment at the invocation why it
/// cannot influence the future output). The structure is destructured exhaustively, so every
/// field must be listed in one of the two lists.
#[macro_export]
macro_rules! impl_state_digest {
    ($t:ident { $($f:ident),* $(,)? }) => {
        $crate::impl_state_digest!($t { $($f),* } skip {});
    };
    ($t:ident { $($f:ident),* $(,)? } skip { $($s:ident),* $(,)? }) => {
        impl $crate::digest::StateDigest for $t {
            fn digest(&self, d: &mut $crate::digest::Digester) {
                let $t { $($f,)* $($s: _,)* } = self;
                $( d.field(stringify!($f), $f); )*
            }
        }
    };
}

macro_rules! primitive {
    ($($t:ty),*) => {$(
        impl StateDigest for $t {
            #[inline]
            fn digest(&self, d: &mut Digester) {
                d.bytes(&self.to_le_bytes());
            }

            #[inline]
            fn digest_slice(s: &[Self], d: &mut Digester) {
                // SAFETY: the primitive integer types have no padding and every bit pattern is
                // initialized, so their memory can be read as bytes. (Little endian: the bytes
                // are the same as `to_le_bytes()`; on a big endian target only the digest value
                // differs, which is compared within one process.)
                let b = unsafe {
                    core::slice::from_raw_parts(s.as_ptr().cast::<u8>(), core::mem::size_of_val(s))
                };
                d.bytes(b);
            }
        }
    )*};
}

primitive!(u8, i8, u16, i16, u32, i32, u64, i64);

impl StateDigest for usize {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        d.u64(*self as u64);
    }
}

impl StateDigest for isize {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        d.u64(*self as u64);
    }
}

impl StateDigest for bool {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        d.bytes(&[*self as u8]);
    }

    #[inline]
    fn digest_slice(s: &[Self], d: &mut Digester) {
        // SAFETY: `bool` is one byte with the value 0 or 1.
        let b = unsafe { core::slice::from_raw_parts(s.as_ptr().cast::<u8>(), s.len()) };
        d.bytes(b);
    }
}

impl<T: StateDigest, const N: usize> StateDigest for [T; N] {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        T::digest_slice(self, d);
    }
}

impl<T: StateDigest> StateDigest for [T] {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        d.u64(self.len() as u64);
        T::digest_slice(self, d);
    }
}

impl<T: StateDigest> StateDigest for Vec<T> {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        self.as_slice().digest(d);
    }
}

impl<T: StateDigest + ?Sized> StateDigest for Box<T> {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        (**self).digest(d);
    }
}

impl<T: StateDigest> StateDigest for Option<T> {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        match self {
            None => d.bytes(&[0]),
            Some(v) => {
                d.bytes(&[1]);
                v.digest(d);
            }
        }
    }
}

/// References in the decoder state point to constant tables: fed by address and size (the same
/// table gives the same digest; the digests are only compared within one process).
impl<T: ?Sized> StateDigest for &T {
    #[inline]
    fn digest(&self, d: &mut Digester) {
        d.u64((*self as *const T).cast::<u8>() as usize as u64);
        d.u64(core::mem::size_of_val(*self) as u64);
    }
}

macro_rules! tuple {
    ($($n:tt $t:ident),*) => {
        impl<$($t: StateDigest),*> StateDigest for ($($t,)*) {
            #[inline]
            fn digest(&self, d: &mut Digester) {
                $(d.element($n, &self.$n);)*
            }
        }
    };
}

tuple!(0 A, 1 B);
tuple!(0 A, 1 B, 2 C);

/* ---- the state structures of this crate --------------------------------------------------- */

/// The bit buffer is a ring buffer whose reads and writes are all relative to its positions
/// (`bit_ndx` for reading, `read_offset` for `FDK_Feed()`; every index is masked with the
/// power-of-two size). It is fed in its logical order: the positions relative to the read
/// position `bit_ndx` (and its bit offset in the byte), and the whole memory starting at the byte
/// of the read position, including the bytes that are not valid (they are read on corrupt input).
/// Two decoders that were fed the same bytes from different starting points of a file have equal
/// states once the ring buffer has been overwritten completely by both. `write_offset` is not
/// fed: only `FDK_Fetch()` reads it, which the decoder does not call.
impl StateDigest for FdkBitBuf {
    fn digest(&self, d: &mut Digester) {
        let FdkBitBuf { valid_bits, read_offset, write_offset: _, bit_ndx, .. } = self;
        let size = self.buf_size();
        let bits = self.buf_bits();
        d.field("buf_size", &size);
        d.field("valid_bits", valid_bits);
        d.field("bit_offset", &(bit_ndx & 7));
        let mask = bits.wrapping_sub(1);
        d.field("read_offset", &((read_offset << 3).wrapping_sub(*bit_ndx) & mask));
        d.field("ptr_is_null", &self.mem().is_empty());
        let buf = if self.mem().is_empty() { Vec::new() } else { self.buffer_to_vec() };
        if !buf.is_empty() {
            let start = ((bit_ndx >> 3) as usize) & (buf.len() - 1);
            d.field_with("buffer", |d| {
                d.bytes(&buf[start..]);
                d.bytes(&buf[..start]);
            });
        }
    }
}

impl_state_digest!(FdkBitStream { cache_word, bits_in_cache, h_bit_buf, config_cache });

use crate::common_fix::FixpSpk;
use crate::crc::{CCrcRegData, FdkCrcInfo};
use crate::decorrelate::{DecorrDec, DecorrFilterInstance, DuckerInstance};
use crate::hybrid::{FdkAnaHybFilter, FdkSynHybFilter};
use crate::mdct::MdctT;
use crate::qmf::{QmfFilterBank, QmfScaleFactor};
use crate::qmf_domain::{
    FdkQmfDomain, FdkQmfDomainGc, FdkQmfDomainIn, FdkQmfDomainOut, QmfDomainMem, QmfSlotPtr,
};
use crate::syslib_channel_map_descr::FdkChannelMapDescr;

impl_state_digest!(FixpSpk { re, im });

/// A CRC region: its positions are written by `FDKcrcStartReg()` and read by `FDKcrcEndReg()` of
/// the same region (the callers pass the ID that the start returned), so an inactive region
/// contributes only that it is inactive.
impl StateDigest for CCrcRegData {
    fn digest(&self, d: &mut Digester) {
        let CCrcRegData { is_active, max_bits, bit_buf_cnt_bits, valid_bits } = self;
        d.field("is_active", is_active);
        if *is_active != 0 {
            d.field("max_bits", max_bits);
            d.field("bit_buf_cnt_bits", bit_buf_cnt_bits);
            d.field("valid_bits", valid_bits);
        }
    }
}

/// The IMDCT state of a channel whose overlap buffer is used only by [`imlt_block`],
/// [`imdct_drain`] and [`imdct_copy_ov_and_nr`] (`crate::mdct`; the AAC filter bank, not the
/// ELD filter bank or USAC, which use the buffer otherwise): every field, but of the overlap
/// buffer only the values these functions read before they write them: the buffered output
/// samples `overlap[0..ov_offset]` and the spectral overlap of the previous transform, the last
/// `prev_tl / 2` values (`imlt_block` reads `prev_nr + fl / 2` values backwards from the end for
/// its first window, which is `prev_tl / 2` after `imdct_adapt_parameters()`). The rest holds
/// values of earlier frames that are overwritten before they are read. After `mdct_init()`
/// (`prev_tl == 0`), and for positions outside the buffer, the whole buffer is fed.
///
/// [`imlt_block`]: crate::mdct::imlt_block
/// [`imdct_drain`]: crate::mdct::imdct_drain
/// [`imdct_copy_ov_and_nr`]: crate::mdct::imdct_copy_ov_and_nr
pub fn digest_mdct_imlt(m: &MdctT, d: &mut Digester) {
    let MdctT {
        overlap,
        prev_wrs,
        prev_tl,
        prev_nr,
        prev_fr,
        ov_offset,
        ov_size,
        prev_alias_symmetry,
        prev_prev_alias_symmetry,
        p_fac_zir,
        p_asym_ovlp,
    } = m;
    let n = overlap.len();
    let live = (*ov_size as usize == n && *ov_offset >= 0 && *prev_tl > 0)
        .then_some((*ov_offset as usize, (*prev_tl / 2) as usize))
        .filter(|&(a, b)| a <= n && b <= n);
    d.field_with("overlap", |d| match live {
        Some((a, b)) => {
            d.u64(n as u64);
            d.u64(a as u64);
            overlap[..a].digest(d);
            overlap[n - b..].digest(d);
        }
        None => overlap.digest(d),
    });
    d.field("prev_wrs", prev_wrs);
    d.field("prev_tl", prev_tl);
    d.field("prev_nr", prev_nr);
    d.field("prev_fr", prev_fr);
    d.field("ov_offset", ov_offset);
    d.field("ov_size", ov_size);
    d.field("prev_alias_symmetry", prev_alias_symmetry);
    d.field("prev_prev_alias_symmetry", prev_prev_alias_symmetry);
    d.field("p_fac_zir", p_fac_zir);
    d.field("p_asym_ovlp", p_asym_ovlp);
}

impl_state_digest!(FdkCrcInfo {
    crc_reg_data,
    p_crc_lookup,
    crc_poly,
    crc_mask,
    start_value,
    crc_len,
    reg_start,
    reg_stop,
    crc_value,
});
impl_state_digest!(MdctT {
    overlap,
    prev_wrs,
    prev_tl,
    prev_nr,
    prev_fr,
    ov_offset,
    ov_size,
    prev_alias_symmetry,
    prev_prev_alias_symmetry,
    p_fac_zir,
    p_asym_ovlp,
});
impl_state_digest!(QmfFilterBank {
    p_filter,
    filter_states,
    filter_size,
    t_cos,
    t_sin,
    filter_scale,
    no_channels,
    no_col,
    lsb,
    usb,
    syn_scalefactor,
    out_scalefactor,
    out_gain_m,
    out_gain_e,
    flags,
    p_stride,
});
impl_state_digest!(QmfScaleFactor { lb_scale, ov_lb_scale, hb_scale, ov_hb_scale });
impl_state_digest!(FdkQmfDomainGc {
    qmf_domain_explicit_config,
    n_input_channels,
    n_input_channels_requested,
    n_output_channels,
    n_output_channels_requested,
    park_channel,
    park_channel_requested,
    flags,
    flags_requested,
    n_bands_analysis,
    n_bands_analysis_requested,
    n_bands_synthesis,
    n_bands_synthesis_requested,
    n_qmf_time_slots,
    n_qmf_time_slots_requested,
    n_qmf_ov_time_slots,
    n_qmf_ov_time_slots_requested,
    n_qmf_proc_bands,
    n_qmf_proc_bands_requested,
    n_qmf_proc_channels,
    n_qmf_proc_channels_requested,
});

impl StateDigest for QmfSlotPtr {
    fn digest(&self, d: &mut Digester) {
        match *self {
            QmfSlotPtr::Null => d.u32(0),
            QmfSlotPtr::Work { sect, offset } => {
                d.u32(1);
                d.bytes(&[sect]);
                d.u32(offset);
            }
            QmfSlotPtr::Overlap { ch, offset } => {
                d.u32(2);
                d.bytes(&[ch]);
                d.u32(offset);
            }
        }
    }
}

impl_state_digest!(FdkQmfDomainIn {
    p_global_conf,
    fb,
    scaling,
    work_buf_n_time_slots,
    work_buf_n_bands,
    work_buffer_offset,
    work_buffer_sect_size,
    p_ana_qmf_states,
    p_work_buffer,
    h_qmf_slots_real,
    h_qmf_slots_imag,
});
impl_state_digest!(FdkQmfDomainOut { fb, p_syn_qmf_states });
impl_state_digest!(QmfDomainMem { work_buffer, overlap_buffer });
impl_state_digest!(FdkQmfDomain { global_conf, qmf_domain_in, qmf_domain_out, mem });

impl_state_digest!(DecorrFilterInstance {
    state_cplx,
    delay_buffer_cplx,
    numerator_real,
    coeffs_packed,
    denominator_real,
});
impl_state_digest!(DuckerInstance {
    hybrid_bands,
    parameter_bands,
    partially_complex,
    ducker_type,
    qs_next,
    map_proc_bands2_hyb_bands,
    map_hyb_bands2_proc_bands,
    smooth_dir_rev_nrg,
    peak_decay,
    peak_diff,
    max_val_direct_data,
    max_val_reverb_data,
    scale_direct_nrg,
    scale_reverb_nrg,
    scale_smooth_dir_rev_nrg,
    headroom_smooth_dir_rev_nrg,
});
impl_state_digest!(DecorrDec {
    l_state_buffer_cplx,
    state_buffer_cplx,
    l_delay_buffer_cplx,
    delay_buffer_cplx,
    rev_filt_type,
    rev_band_offset,
    rev_delay,
    rev_filter_order,
    reverb_band_delay_buffer_index,
    state_buffer_offset,
    filter,
    ducker,
    numbins,
    partially_complex,
});
impl_state_digest!(FdkSynHybFilter { nr_bands, cplx_bands, p_setup });
impl_state_digest!(FdkAnaHybFilter {
    buffer_lf_real,
    buffer_lf_imag,
    buffer_hf_real,
    buffer_hf_imag,
    buffer_lf_pos,
    buffer_hf_pos,
    nr_bands,
    cplx_bands,
    hf_mode,
    p_lf_memory,
    p_hf_memory,
    lf_memory_size,
    hf_memory_size,
    p_setup,
});
impl_state_digest!(FdkChannelMapDescr { map_info_tab, map_info_tab_len, f_pass_through });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_distinguishes_values_and_layout() {
        assert_eq!(digest_of(&[1i32, 2, 3]), digest_of(&[1i32, 2, 3]));
        assert_ne!(digest_of(&[1i32, 2, 3]), digest_of(&[1i32, 2, 4]));
        /* lengths are part of the digest of slices */
        assert_ne!(digest_of(&vec![0u8; 3]), digest_of(&vec![0u8; 4]));
        assert_ne!(digest_of(&(vec![1u8], vec![2u8, 3])), digest_of(&(vec![1u8, 2], vec![3u8])));
        assert_ne!(digest_of(&Some(0u8)), digest_of(&None::<u8>));
        /* the digest does not depend on how the bytes are split */
        let mut a = Digester::new();
        a.bytes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]);
        let mut b = Digester::new();
        b.bytes(&[1, 2, 3]);
        b.bytes(&[4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(a.finish(), b.finish());
        /* every bit of a large buffer matters */
        let mut v = vec![0u32; 4096];
        let d0 = digest_of(&v);
        for i in [0, 1, 17, 4095] {
            for bit in [0, 13, 31] {
                v[i] ^= 1 << bit;
                assert_ne!(digest_of(&v), d0);
                v[i] ^= 1 << bit;
            }
        }
        assert_eq!(digest_of(&v), d0);
    }

    #[test]
    fn traces_locate_differences() {
        let t = |v: &(Vec<u8>, [i16; 2], Option<u32>)| {
            let mut d = Digester::traced(8);
            v.digest(&mut d);
            d.finish_traced().1
        };
        let a = (vec![1u8, 2], [3i16, 4], Some(5u32));
        let mut b = a.clone();
        b.1[1] = 9;
        assert_eq!(diff_traces(&t(&a), &t(&b)), vec!["[1]".to_string()]);
        assert!(diff_traces(&t(&a), &t(&a)).is_empty());
    }

    struct Pair {
        a: u8,
        b: [i32; 2],
        scratch: u32,
    }
    impl_state_digest!(Pair { a, b } skip { scratch });

    #[test]
    fn struct_macro_feeds_the_listed_fields() {
        let p = Pair { a: 1, b: [2, 3], scratch: 4 };
        let q = Pair { a: 1, b: [2, 3], scratch: 5 };
        assert_eq!(digest_of(&p), digest_of(&q));
        let r = Pair { a: 1, b: [2, 4], scratch: 4 };
        assert_ne!(digest_of(&p), digest_of(&r));
        let _ = (p.scratch, q.scratch, r.scratch);
    }

    /// The bit buffer digest is independent of the physical position of the data in the ring.
    #[test]
    fn bit_buffer_digest_is_relative_to_the_read_position() {
        use crate::bitbuffer::*;
        let data: Vec<u8> = (0..64u32).map(|i| (i * 37 + 5) as u8).collect();
        let mut a = FdkBitBuf::default();
        let mut b = FdkBitBuf::default();
        fdk_init_bit_buffer(&mut a, &BitMem::new(64), 64, 0);
        fdk_init_bit_buffer(&mut b, &BitMem::new(64), 64, 0);
        // a: 64 bytes, then 40 read and 40 more fed; b: 24 bytes fed and read first, then the
        // same 64 + 40 bytes (b's ring is rotated by 24 bytes against a's).
        let feed = |h: &mut FdkBitBuf, bytes: &[u8]| {
            let mut v = bytes.len() as u32;
            fdk_feed(h, bytes, bytes.len() as u32, &mut v);
            assert_eq!(v, 0);
        };
        let skip = |h: &mut FdkBitBuf, n: u32| {
            for _ in 0..n {
                fdk_get(h, 8);
            }
        };
        feed(&mut b, &data[..24]);
        skip(&mut b, 24);
        for h in [&mut a, &mut b] {
            feed(h, &data);
            skip(h, 40);
            feed(h, &data[..40]);
            fdk_get(h, 3);
        }
        assert_ne!(a.bit_ndx, b.bit_ndx);
        assert_eq!(digest_of(&a), digest_of(&b));
        fdk_get(&mut a, 1);
        assert_ne!(digest_of(&a), digest_of(&b));
    }
}
