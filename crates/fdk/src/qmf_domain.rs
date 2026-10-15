// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_qmf_domain.h, libFDK/src/FDK_qmf_domain.cpp.

//! Port of `FDK_qmf_domain.h` and `FDK_qmf_domain.cpp`: the QMF domain, which holds the QMF
//! analysis and synthesis filter banks of all channels and the QMF data that SBR and MPEG
//! Surround share in the decoder.
//!
//! # Memory design
//!
//! The C module hands out raw pointers into memory it allocates: work buffer sections
//! (`globalConf.pWorkBuffer[]`), per channel overlap buffers (`pOverlapBuffer`), and arrays of
//! time slot pointers into both (`hQmfSlotsReal`, `hQmfSlotsImag`). SBR and SAC read and write
//! the QMF data through these pointers. The port replaces every pointer by an owned buffer or
//! by a location in one:
//!
//! - **QMF data memory** ([`QmfDomainMem`], field [`FdkQmfDomain::mem`]): the five work buffer
//!   sections (`globalConf.pWorkBuffer[i]` → `mem.work_buffer[i]`) and the overlap buffers of
//!   the input channels (`QmfDomainIn[ch].pOverlapBuffer` → `mem.overlap_buffer[ch]`). They are
//!   kept apart from the channel structures so that a caller can borrow a channel's filter bank
//!   and states and the QMF data at the same time (`let FdkQmfDomain { mem, qmf_domain_in, .. }
//!   = qd;`). `None` is `NULL`.
//! - **Time slot pointers** (`FIXP_DBL *` into that memory) are [`QmfSlotPtr`] values: a buffer
//!   (work buffer section or overlap buffer of a channel) and an offset in it. `hQmfSlotsReal`
//!   and `hQmfSlotsImag` are arrays of them. A pointer is resolved when it is used
//!   ([`QmfDomainMem::slot`], [`QmfDomainMem::slot_mut`], [`QmfDomainMem::slots_mut`]), so a
//!   resolved slot is a slice from the slot's start to the end of its buffer (or the requested
//!   length). [`QmfDomainMem::slots_mut`] returns the mutable slices of a whole pointer array at
//!   once, which is what the C functions taking `FIXP_DBL **` need (e.g.
//!   [`crate::qmf::qmf_analysis_filtering`]).
//! - **`QmfDomainIn[ch].pWorkBuffer`** always points to `globalConf.pWorkBuffer` (or is `NULL`):
//!   it is the flag [`FdkQmfDomainIn::p_work_buffer`]. `pGlobalConf` always points to the
//!   domain's `globalConf` (or is `NULL`): the flag [`FdkQmfDomainIn::p_global_conf`]. The
//!   functions that take a `HANDLE_FDK_QMF_DOMAIN_IN` in C take the domain and the channel
//!   index.
//! - **Filter states** (`pAnaQmfStates`, `pSynQmfStates`) are owned by the channel structures
//!   ([`FdkQmfDomainIn::p_ana_qmf_states`], [`FdkQmfDomainOut::p_syn_qmf_states`]); the filter
//!   bank `fb` only records that it has states (see [`crate::qmf`]), and callers pass the
//!   channel's states to the QMF functions.
//!
//! Allocation follows the C code: memory is allocated (zeroed, like `FDKcalloc`/`FDKaalloc`)
//! where the C code allocates it, kept where the C code keeps it (also across
//! reconfigurations) and freed where the C code frees it, so stale content is reproduced.
//!
//! Deviations where the reference has no defined result (PORTING.md, "Deviations from the
//! reference"):
//!
//! - Every persistent buffer is allocated with the largest size the C code uses for it
//!   (`AnaQmfStates`: 640 values, slot pointer arrays: 76 entries, overlap buffers: 1536
//!   values); the port works within these buffers.
//! - A time slot pointer into a work buffer section is resolved when it is used; the C code
//!   computes the address when the slot array is set up.
//! - Channel indices beyond the 9 channels (`((8) + (1))`) are ignored; a channel without slot
//!   arrays, work buffer or global configuration is not processed; copies are limited to the end
//!   of the buffers.

use crate::common_fix::*;
use crate::machine_type::ALIGNMENT_DEFAULT;
use crate::qmf::*;
use crate::scale::scale_value_saturate;

/// `QMF_DOMAIN_ERROR`
pub type QmfDomainError = i32;
/// No error occurred.
pub const QMF_DOMAIN_OK: QmfDomainError = 0x0;
/// QMF-Configuration demands for more memory than allocated on heap.
pub const QMF_DOMAIN_OUT_OF_MEMORY: QmfDomainError = 0x1;
/// An error during filterbank-setup occurred.
pub const QMF_DOMAIN_INIT_ERROR: QmfDomainError = 0x2;
/// An error during QMF-resampler-setup occurred.
pub const QMF_DOMAIN_RESAMPLER_INIT_ERROR: QmfDomainError = 0x3;

/// `CMPLX_MOD`
pub const CMPLX_MOD: i32 = 2;

/// `QMF_MAX_WB_SECTIONS`: maximum number of workbuffer sections
pub const QMF_MAX_WB_SECTIONS: usize = 5;
/// `QMF_WB_SECTION_SIZE`
pub const QMF_WB_SECTION_SIZE: i32 = 1024 * 2;

/// `QMF_DOMAIN_MAX_ANALYSIS_QMF_BANDS`
pub const QMF_DOMAIN_MAX_ANALYSIS_QMF_BANDS: i32 = 64;
/// `QMF_DOMAIN_MAX_SYNTHESIS_QMF_BANDS`
pub const QMF_DOMAIN_MAX_SYNTHESIS_QMF_BANDS: i32 = QMF_MAX_SYNTHESIS_BANDS;
/// `QMF_DOMAIN_MAX_QMF_PROC_BANDS`
pub const QMF_DOMAIN_MAX_QMF_PROC_BANDS: i32 = 64;
/// `QMF_DOMAIN_MAX_TIMESLOTS`
pub const QMF_DOMAIN_MAX_TIMESLOTS: i32 = 64;
/// `QMF_DOMAIN_MAX_OV_TIMESLOTS`
pub const QMF_DOMAIN_MAX_OV_TIMESLOTS: i32 = 12;

/// `QMF_DOMAIN_ANALYSIS_QMF_BANDS_16`
pub const QMF_DOMAIN_ANALYSIS_QMF_BANDS_16: i32 = 16;
/// `QMF_DOMAIN_ANALYSIS_QMF_BANDS_24`
pub const QMF_DOMAIN_ANALYSIS_QMF_BANDS_24: i32 = 24;
/// `QMF_DOMAIN_ANALYSIS_QMF_BANDS_32`
pub const QMF_DOMAIN_ANALYSIS_QMF_BANDS_32: i32 = 32;

/// `QMF_DOMAIN_TIMESLOTS_16`
pub const QMF_DOMAIN_TIMESLOTS_16: i32 = 16;
/// `QMF_DOMAIN_TIMESLOTS_32`
pub const QMF_DOMAIN_TIMESLOTS_32: i32 = 32;

/// `QMF_DOMAIN_OV_TIMESLOTS_16`
pub const QMF_DOMAIN_OV_TIMESLOTS_16: i32 = 3;
/// `QMF_DOMAIN_OV_TIMESLOTS_32`
pub const QMF_DOMAIN_OV_TIMESLOTS_32: i32 = 6;

/// The number of input and output channels of a QMF domain, the literal `((8) + (1))` of the
/// C code.
pub const QMF_DOMAIN_MAX_CHANNELS: usize = 8 + 1;

/// Size of an analysis states buffer (`AnaQmfStates`: `10 * QMF_DOMAIN_MAX_ANALYSIS_QMF_BANDS`).
pub const ANA_QMF_STATES_SIZE: usize = 10 * QMF_DOMAIN_MAX_ANALYSIS_QMF_BANDS as usize;
/// Size of a synthesis states buffer (`SynQmfStates`: `9 * QMF_DOMAIN_MAX_SYNTHESIS_QMF_BANDS`).
pub const SYN_QMF_STATES_SIZE: usize = 9 * QMF_DOMAIN_MAX_SYNTHESIS_QMF_BANDS as usize;
/// Size of a slot pointer array (`QmfSlotsReal`: `QMF_DOMAIN_MAX_TIMESLOTS +
/// QMF_DOMAIN_MAX_OV_TIMESLOTS`).
pub const QMF_SLOTS_SIZE: usize = (QMF_DOMAIN_MAX_TIMESLOTS + QMF_DOMAIN_MAX_OV_TIMESLOTS) as usize;
/// Size of an overlap buffer (`QmfOverlapBuffer`: `2 * QMF_DOMAIN_MAX_OV_TIMESLOTS *
/// QMF_DOMAIN_MAX_QMF_PROC_BANDS`).
pub const QMF_OVERLAP_BUFFER_SIZE: usize =
    (2 * QMF_DOMAIN_MAX_OV_TIMESLOTS * QMF_DOMAIN_MAX_QMF_PROC_BANDS) as usize;

/// A `FIXP_DBL *` into the QMF data memory of a domain ([`QmfDomainMem`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QmfSlotPtr {
    /// `NULL`
    #[default]
    Null,
    /// `&globalConf.pWorkBuffer[sect][offset]`
    Work { sect: u8, offset: u32 },
    /// `&QmfDomainIn[ch].pOverlapBuffer[offset]`
    Overlap { ch: u8, offset: u32 },
}

impl QmfSlotPtr {
    /// `p + n` (pointer arithmetic within the same buffer).
    pub fn advance(self, n: i32) -> QmfSlotPtr {
        match self {
            QmfSlotPtr::Null => QmfSlotPtr::Null,
            QmfSlotPtr::Work { sect, offset } => {
                QmfSlotPtr::Work { sect, offset: offset.wrapping_add(n as u32) }
            }
            QmfSlotPtr::Overlap { ch, offset } => {
                QmfSlotPtr::Overlap { ch, offset: offset.wrapping_add(n as u32) }
            }
        }
    }
}

/// The QMF data memory of a domain: the work buffer sections and the overlap buffers. See the
/// [module documentation](self).
#[derive(Clone, Debug, Default)]
pub struct QmfDomainMem {
    /// `globalConf.pWorkBuffer[]`: the work buffer sections (`QMF_WB_SECTION_SIZE` values each).
    pub work_buffer: [Option<Box<[FixpDbl]>>; QMF_MAX_WB_SECTIONS],
    /// `QmfDomainIn[ch].pOverlapBuffer`: the overlap buffers of the input channels.
    pub overlap_buffer: [Option<Box<[FixpDbl]>>; QMF_DOMAIN_MAX_CHANNELS],
}

impl QmfDomainMem {
    /// The buffer of a pointer and the offset in it.
    fn locate(&self, p: QmfSlotPtr) -> Option<(&[FixpDbl], usize)> {
        let (buf, off) = match p {
            QmfSlotPtr::Null => return None,
            QmfSlotPtr::Work { sect, offset } => (self.work_buffer.get(sect as usize)?, offset),
            QmfSlotPtr::Overlap { ch, offset } => (self.overlap_buffer.get(ch as usize)?, offset),
        };
        let buf = buf.as_deref()?;
        let off = off as usize;
        (off <= buf.len()).then_some((buf, off))
    }

    /// The slot that `p` points to, up to the end of its buffer (empty for `NULL`, an
    /// unallocated buffer or an offset past its end).
    pub fn slot(&self, p: QmfSlotPtr) -> &[FixpDbl] {
        match self.locate(p) {
            Some((buf, off)) => &buf[off..],
            None => &[],
        }
    }

    /// The mutable slot that `p` points to, up to the end of its buffer (empty for `NULL`, an
    /// unallocated buffer or an offset past its end).
    pub fn slot_mut(&mut self, p: QmfSlotPtr) -> &mut [FixpDbl] {
        let (buf, off) = match p {
            QmfSlotPtr::Null => return &mut [],
            QmfSlotPtr::Work { sect, offset } => (self.work_buffer.get_mut(sect as usize), offset),
            QmfSlotPtr::Overlap { ch, offset } => {
                (self.overlap_buffer.get_mut(ch as usize), offset)
            }
        };
        match buf.and_then(|b| b.as_deref_mut()) {
            Some(buf) if off as usize <= buf.len() => &mut buf[off as usize..],
            _ => &mut [],
        }
    }

    /// The mutable slots of a pointer array (`FIXP_DBL **`): slice `i` starts at `ptrs[i]` and
    /// has `len` values (fewer at the end of its buffer, none for `NULL`).
    ///
    /// The slots must not overlap (they never do in the library: the slots of a channel are
    /// consecutive blocks of `nQmfProcBands` values); a slot that overlaps an earlier one (in
    /// the order of the buffers) is returned empty.
    pub fn slots_mut(&mut self, ptrs: &[QmfSlotPtr], len: usize) -> Vec<&mut [FixpDbl]> {
        let mut cut = SlotCutter::new(self, len);
        // The slices of different regions are independent, so only the order within a region
        // matters. The slots of a channel are consecutive in the library, so the pointers of a
        // region are often in ascending order of the offsets already: then the slices are cut
        // in the order of the pointers (the order of the sort below, which breaks ties by the
        // index), without sorting.
        if in_region_order(ptrs.iter().copied(), false) {
            return ptrs.iter().map(|&p| cut.cut(p)).collect();
        }

        let mut order: Vec<(usize, usize, usize)> = ptrs
            .iter()
            .enumerate()
            .filter_map(|(i, &p)| region_of(p).map(|(r, o)| (r, o, i)))
            .collect();
        order.sort_unstable();
        let mut out: Vec<Option<&mut [FixpDbl]>> = (0..ptrs.len()).map(|_| None).collect();
        for (r, off, i) in order {
            out[i] = cut.cut_at(r, off);
        }
        out.into_iter().map(|s| s.unwrap_or(&mut [])).collect()
    }

    /// The slots of a channel without heap allocation: `slots_mut(re ++ im, len)`, the slots of
    /// `re` appended to `out_re` and those of `im` to `out_im` (`re` and `im` of equal length).
    ///
    /// The QMF domain places the real and imaginary slot of every time slot next to each other
    /// (`re[0]`, `im[0]`, `re[1]`, ...). If the offsets of every region are strictly ascending
    /// in this order, it is the order of the sort of [`QmfDomainMem::slots_mut`] within each
    /// region (no ties), and the slices are cut in this order; otherwise `slots_mut` computes
    /// them.
    pub fn channel_slots_into<'a>(
        &'a mut self,
        re: &[QmfSlotPtr],
        im: &[QmfSlotPtr],
        len: usize,
        out_re: &mut SlotList<'a>,
        out_im: &mut SlotList<'a>,
    ) {
        let n = re.len();
        assert!(im.len() == n);
        if in_region_order(re.iter().zip(im).flat_map(|(&r, &i)| [r, i]), true) {
            let mut cut = SlotCutter::new(self, len);
            for (&r, &i) in re.iter().zip(im) {
                out_re.push(cut.cut(r));
                out_im.push(cut.cut(i));
            }
        } else {
            let all: Vec<QmfSlotPtr> = re.iter().chain(im).copied().collect();
            let mut slots = self.slots_mut(&all, len).into_iter();
            out_re.extend(slots.by_ref().take(n));
            out_im.extend(slots);
        }
    }

    /// `FDKmemcpy(dst, src, n * sizeof(FIXP_DBL))` between two slots (limited to the ends of
    /// the buffers).
    ///
    /// The values are moved directly (`copy_within` in one buffer, which is `memmove` as the
    /// copy through a temporary is), without the temporary vector of
    /// [`QmfDomainMem::copy_straight`].
    pub fn copy(&mut self, dst: QmfSlotPtr, src: QmfSlotPtr, n: usize) {
        let (Some((rd, od)), Some((rs, os))) = (region_of(dst), region_of(src)) else {
            return;
        };
        let w = QMF_MAX_WB_SECTIONS;
        // the buffers of the two regions (None: not allocated)
        let (d, s): (Option<&mut [FixpDbl]>, Option<&[FixpDbl]>) = if rd == rs {
            let Some(buf) = self.region_mut(rd) else { return };
            // in one buffer: offsets past its end give empty slots (no copy)
            if od > buf.len() || os > buf.len() {
                return;
            }
            let k = n.min(buf.len() - os).min(buf.len() - od);
            buf.copy_within(os..os + k, od);
            return;
        } else if rd < w && rs < w {
            let (lo, hi) = self.work_buffer.split_at_mut(rd.max(rs));
            let (a, b) = (lo[rd.min(rs)].as_deref_mut(), hi[0].as_deref_mut());
            if rd < rs {
                (a, b.map(|b| &*b))
            } else {
                (b, a.map(|a| &*a))
            }
        } else if rd >= w && rs >= w {
            let (lo, hi) = self.overlap_buffer.split_at_mut(rd.max(rs) - w);
            let (a, b) = (lo[rd.min(rs) - w].as_deref_mut(), hi[0].as_deref_mut());
            if rd < rs {
                (a, b.map(|b| &*b))
            } else {
                (b, a.map(|a| &*a))
            }
        } else if rd < w {
            (self.work_buffer[rd].as_deref_mut(), self.overlap_buffer[rs - w].as_deref())
        } else {
            (self.overlap_buffer[rd - w].as_deref_mut(), self.work_buffer[rs].as_deref())
        };
        let (Some(d), Some(s)) = (d, s) else { return };
        if od > d.len() || os > s.len() {
            return;
        }
        let k = n.min(s.len() - os).min(d.len() - od);
        d[od..od + k].copy_from_slice(&s[os..os + k]);
    }

    /// The buffer of region `r` of [`region_of`] (`None`: not allocated).
    fn region_mut(&mut self, r: usize) -> Option<&mut [FixpDbl]> {
        if r < QMF_MAX_WB_SECTIONS {
            self.work_buffer[r].as_deref_mut()
        } else {
            self.overlap_buffer[r - QMF_MAX_WB_SECTIONS].as_deref_mut()
        }
    }

    /// [`QmfDomainMem::copy`] through a temporary vector (the straight port, kept as its
    /// reference).
    #[doc(hidden)]
    pub fn copy_straight(&mut self, dst: QmfSlotPtr, src: QmfSlotPtr, n: usize) {
        let tmp: Vec<FixpDbl> = {
            let s = self.slot(src);
            s[..n.min(s.len())].to_vec()
        };
        let d = self.slot_mut(dst);
        let n = tmp.len().min(d.len());
        d[..n].copy_from_slice(&tmp[..n]);
    }
}

/// The region of a slot pointer (0..5: work buffer sections, 5..14: overlap buffers) and the
/// offset in it; `None` for `NULL` or an invalid index.
#[inline]
fn region_of(p: QmfSlotPtr) -> Option<(usize, usize)> {
    match p {
        QmfSlotPtr::Null => None,
        QmfSlotPtr::Work { sect, offset } => {
            ((sect as usize) < QMF_MAX_WB_SECTIONS).then_some((sect as usize, offset as usize))
        }
        QmfSlotPtr::Overlap { ch, offset } => ((ch as usize) < QMF_DOMAIN_MAX_CHANNELS)
            .then_some((QMF_MAX_WB_SECTIONS + ch as usize, offset as usize)),
    }
}

/// Number of regions of [`region_of`].
const N_REGIONS: usize = QMF_MAX_WB_SECTIONS + QMF_DOMAIN_MAX_CHANNELS;

/// Whether the offsets of the pointers of every region are ascending in the order of `ptrs`
/// (`strict`: strictly ascending); `NULL` and invalid pointers are skipped.
fn in_region_order(ptrs: impl Iterator<Item = QmfSlotPtr>, strict: bool) -> bool {
    let mut last = [None::<usize>; N_REGIONS];
    for p in ptrs {
        if let Some((r, o)) = region_of(p) {
            match last[r] {
                Some(l) if o < l || (strict && o == l) => return false,
                _ => last[r] = Some(o),
            }
        }
    }
    true
}

/// Cuts the slices of slots from the buffers of a [`QmfDomainMem`], the slots of each region in
/// ascending order of their offsets: each slice is cut from the rest of its region after the
/// previous one of the region, so the slices do not overlap; a slot that starts before the end
/// of the previous one of its region (or past the end of the buffer) is `None`. The regions are
/// independent of each other.
struct SlotCutter<'a> {
    rest: [&'a mut [FixpDbl]; N_REGIONS],
    consumed: [usize; N_REGIONS],
    len: usize,
}

impl<'a> SlotCutter<'a> {
    fn new(mem: &'a mut QmfDomainMem, len: usize) -> Self {
        let mut rest: [&'a mut [FixpDbl]; N_REGIONS] = Default::default();
        for (r, b) in
            rest.iter_mut().zip(mem.work_buffer.iter_mut().chain(mem.overlap_buffer.iter_mut()))
        {
            *r = b.as_deref_mut().unwrap_or(&mut []);
        }
        SlotCutter { rest, consumed: [0; N_REGIONS], len }
    }

    #[inline]
    fn cut_at(&mut self, r: usize, off: usize) -> Option<&'a mut [FixpDbl]> {
        let consumed = self.consumed[r];
        if off < consumed || off > consumed + self.rest[r].len() {
            return None;
        }
        let tail = core::mem::take(&mut self.rest[r]);
        let (_, tail) = tail.split_at_mut(off - consumed);
        let n = self.len.min(tail.len());
        let (slot, tail) = tail.split_at_mut(n);
        self.rest[r] = tail;
        self.consumed[r] = off + n;
        Some(slot)
    }

    /// The slice of `p` (empty for `NULL` or a slot that cannot be cut).
    #[inline]
    fn cut(&mut self, p: QmfSlotPtr) -> &'a mut [FixpDbl] {
        region_of(p).and_then(|(r, o)| self.cut_at(r, o)).unwrap_or(&mut [])
    }
}

/// The capacity of a [`SlotList`]: three times the slots of a channel.
pub const SLOT_LIST_CAP: usize = 3 * QMF_SLOTS_SIZE;

/// A list of at most [`SLOT_LIST_CAP`] slot slices without heap allocation and without
/// initializing its storage (dereferences to `[&mut [FixpDbl]]`).
pub struct SlotList<'a> {
    slots: [core::mem::MaybeUninit<&'a mut [FixpDbl]>; SLOT_LIST_CAP],
    len: usize,
}

impl<'a> SlotList<'a> {
    /// An empty list.
    #[inline]
    pub fn new() -> Self {
        SlotList { slots: [const { core::mem::MaybeUninit::uninit() }; SLOT_LIST_CAP], len: 0 }
    }

    /// Appends a slice. Panics if the list is full.
    #[inline]
    pub fn push(&mut self, s: &'a mut [FixpDbl]) {
        self.slots[self.len].write(s);
        self.len += 1;
    }
}

impl Default for SlotList<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Extend<&'a mut [FixpDbl]> for SlotList<'a> {
    fn extend<I: IntoIterator<Item = &'a mut [FixpDbl]>>(&mut self, iter: I) {
        for s in iter {
            self.push(s);
        }
    }
}

impl<'a> core::ops::Deref for SlotList<'a> {
    type Target = [&'a mut [FixpDbl]];
    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: the first `len` elements are initialized (`push`); `MaybeUninit<T>` has the
        // layout of `T`.
        unsafe {
            core::slice::from_raw_parts(self.slots.as_ptr() as *const &'a mut [FixpDbl], self.len)
        }
    }
}

impl core::ops::DerefMut for SlotList<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: as in `deref`.
        unsafe {
            core::slice::from_raw_parts_mut(
                self.slots.as_mut_ptr() as *mut &mut [FixpDbl],
                self.len,
            )
        }
    }
}

/// `FDK_QMF_DOMAIN_GC`: the configuration of a QMF domain. `pWorkBuffer` is
/// [`QmfDomainMem::work_buffer`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FdkQmfDomainGc {
    /// `qmfDomainExplicitConfig`: the QMF domain is set explicitly instead of by the SBR and
    /// MPS init routines.
    pub qmf_domain_explicit_config: u8,
    /// `nInputChannels`
    pub n_input_channels: u8,
    /// `nInputChannels_requested`
    pub n_input_channels_requested: u8,
    /// `nOutputChannels`
    pub n_output_channels: u8,
    /// `nOutputChannels_requested`
    pub n_output_channels_requested: u8,
    /// `parkChannel`: allocate additional memory to park a channel if only one processing
    /// channel is available.
    pub park_channel: u8,
    /// `parkChannel_requested`
    pub park_channel_requested: u8,
    /// `flags`: flags of all QMF analysis/synthesis filter banks
    pub flags: u32,
    /// `flags_requested`
    pub flags_requested: u32,
    /// `nBandsAnalysis`
    pub n_bands_analysis: u8,
    /// `nBandsAnalysis_requested`
    pub n_bands_analysis_requested: u8,
    /// `nBandsSynthesis`
    pub n_bands_synthesis: u16,
    /// `nBandsSynthesis_requested`
    pub n_bands_synthesis_requested: u16,
    /// `nQmfTimeSlots`: number of QMF time slots (stored in work buffer memory)
    pub n_qmf_time_slots: u8,
    /// `nQmfTimeSlots_requested`
    pub n_qmf_time_slots_requested: u8,
    /// `nQmfOvTimeSlots`: number of QMF overlap/delay time slots (persistent memory)
    pub n_qmf_ov_time_slots: u8,
    /// `nQmfOvTimeSlots_requested`
    pub n_qmf_ov_time_slots_requested: u8,
    /// `nQmfProcBands`: number of QMF bands processed by the decoder
    pub n_qmf_proc_bands: u8,
    /// `nQmfProcBands_requested`
    pub n_qmf_proc_bands_requested: u8,
    /// `nQmfProcChannels`: number of complete QMF channels that coexist in memory
    pub n_qmf_proc_channels: u8,
    /// `nQmfProcChannels_requested`
    pub n_qmf_proc_channels_requested: u8,
}

/// `FDK_QMF_DOMAIN_IN`: one QMF input channel. `pOverlapBuffer` is
/// [`QmfDomainMem::overlap_buffer`]`[ch]`.
#[derive(Clone, Debug, Default)]
pub struct FdkQmfDomainIn {
    /// `pGlobalConf != NULL` (it always points to the domain's `globalConf`)
    pub p_global_conf: bool,
    /// `fb`: QMF analysis filter bank
    pub fb: QmfFilterBank,
    /// `scaling`
    pub scaling: QmfScaleFactor,
    /// `workBuf_nTimeSlots`
    pub work_buf_n_time_slots: u8,
    /// `workBuf_nBands`
    pub work_buf_n_bands: u8,
    /// `workBufferOffset`: offset within the work buffer
    pub work_buffer_offset: u16,
    /// `workBufferSectSize`: size of a work buffer section
    pub work_buffer_sect_size: u16,
    /// `pAnaQmfStates`: QMF analysis states (persistent memory)
    pub p_ana_qmf_states: Option<Box<[FixpDbl]>>,
    /// `pWorkBuffer != NULL` (it always points to `globalConf.pWorkBuffer`)
    pub p_work_buffer: bool,
    /// `hQmfSlotsReal`: pointers to the real QMF data of each time slot
    pub h_qmf_slots_real: Option<Box<[QmfSlotPtr]>>,
    /// `hQmfSlotsImag`: pointers to the imaginary QMF data of each time slot
    pub h_qmf_slots_imag: Option<Box<[QmfSlotPtr]>>,
}

/// `FDK_QMF_DOMAIN_OUT`: one QMF output channel.
#[derive(Clone, Debug, Default)]
pub struct FdkQmfDomainOut {
    /// `fb`: QMF synthesis filter bank
    pub fb: QmfFilterBank,
    /// `pSynQmfStates`: QMF synthesis states (persistent memory)
    pub p_syn_qmf_states: Option<Box<[FixpQss]>>,
}

/// `FDK_QMF_DOMAIN`
#[derive(Clone, Debug, Default)]
pub struct FdkQmfDomain {
    /// `globalConf`
    pub global_conf: FdkQmfDomainGc,
    /// `QmfDomainIn[]`
    pub qmf_domain_in: [FdkQmfDomainIn; QMF_DOMAIN_MAX_CHANNELS],
    /// `QmfDomainOut[]`
    pub qmf_domain_out: [FdkQmfDomainOut; QMF_DOMAIN_MAX_CHANNELS],
    /// The QMF data memory (`globalConf.pWorkBuffer[]` and `QmfDomainIn[].pOverlapBuffer`).
    pub mem: QmfDomainMem,
}

fn alloc(n: usize) -> Option<Box<[FixpDbl]>> {
    Some(vec![0; n].into_boxed_slice())
}

/// `FDK_QmfDomain_FreePersistentMemory()`
fn fdk_qmf_domain_free_persistent_memory(qd: &mut FdkQmfDomain) -> i32 {
    let err = 0;

    for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
        // The C code selects the Free function by the configuration; all of them free.
        qd.qmf_domain_in[ch].p_ana_qmf_states = None;
        qd.mem.overlap_buffer[ch] = None;
        qd.qmf_domain_in[ch].h_qmf_slots_real = None;
        qd.qmf_domain_in[ch].h_qmf_slots_imag = None;
    }

    for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
        qd.qmf_domain_out[ch].p_syn_qmf_states = None;
    }

    err
}

/// `FDK_QmfDomain_AllocatePersistentMemory()`
fn fdk_qmf_domain_allocate_persistent_memory(qd: &mut FdkQmfDomain) -> i32 {
    let gc = qd.global_conf;

    if gc.n_input_channels as usize > QMF_DOMAIN_MAX_CHANNELS
        || gc.n_output_channels as usize > QMF_DOMAIN_MAX_CHANNELS
    {
        return 1;
    }
    for ch in 0..gc.n_input_channels as usize {
        let qd_in = &mut qd.qmf_domain_in[ch];

        let size = gc.n_bands_analysis as i32 * 10;
        if size > 0 {
            // AnaQmfStates16/24/32 or AnaQmfStates (see the module documentation)
            if qd_in.p_ana_qmf_states.is_none() {
                qd_in.p_ana_qmf_states = alloc(ANA_QMF_STATES_SIZE);
            }
        } else {
            qd_in.p_ana_qmf_states = None;
        }

        let size = gc.n_qmf_ov_time_slots as i32 + gc.n_qmf_time_slots as i32;
        if size > 0 {
            // QmfSlotsReal16/32 or QmfSlotsReal (FDKcalloc: NULL pointers)
            if qd_in.h_qmf_slots_real.is_none() {
                qd_in.h_qmf_slots_real = Some(vec![QmfSlotPtr::Null; QMF_SLOTS_SIZE].into());
            }
            if qd_in.h_qmf_slots_imag.is_none() {
                qd_in.h_qmf_slots_imag = Some(vec![QmfSlotPtr::Null; QMF_SLOTS_SIZE].into());
            }
        } else {
            qd_in.h_qmf_slots_real = None;
            qd_in.h_qmf_slots_imag = None;
        }

        let size = gc.n_qmf_ov_time_slots as i32 * gc.n_qmf_proc_bands as i32 * CMPLX_MOD;
        if size > 0 {
            // QmfOverlapBuffer16/32 or QmfOverlapBuffer
            if qd.mem.overlap_buffer[ch].is_none() {
                qd.mem.overlap_buffer[ch] = alloc(QMF_OVERLAP_BUFFER_SIZE);
            }
        } else {
            qd.mem.overlap_buffer[ch] = None;
        }
    }

    for ch in 0..gc.n_output_channels as usize {
        let size = gc.n_bands_synthesis as i32 * 9;
        if size > 0 {
            if qd.qmf_domain_out[ch].p_syn_qmf_states.is_none() {
                qd.qmf_domain_out[ch].p_syn_qmf_states = alloc(SYN_QMF_STATES_SIZE);
            }
        } else {
            qd.qmf_domain_out[ch].p_syn_qmf_states = None;
        }
    }

    // The allocations cannot fail (the C code frees everything and returns -1 if one fails).
    0
}

/// `FDK_QmfDomain_ClearPersistentMemory(hqd)`: clears the QMF overlap buffers and the filter
/// bank states. `hqd = NULL` (`None`) returns `QMF_DOMAIN_INIT_ERROR`.
pub fn fdk_qmf_domain_clear_persistent_memory(hqd: Option<&mut FdkQmfDomain>) -> QmfDomainError {
    let mut err = QMF_DOMAIN_OK;
    if let Some(hqd) = hqd {
        let gc = hqd.global_conf;

        let size =
            gc.n_qmf_ov_time_slots as usize * gc.n_qmf_proc_bands as usize * CMPLX_MOD as usize;
        for ch in 0..(gc.n_input_channels as usize).min(QMF_DOMAIN_MAX_CHANNELS) {
            if let Some(ov) = hqd.mem.overlap_buffer[ch].as_deref_mut() {
                let n = size.min(ov.len());
                ov[..n].fill(0);
            }
        }
        if fdk_qmf_domain_init_filter_bank(hqd, 0) != 0 {
            err = QMF_DOMAIN_INIT_ERROR;
        }
    } else {
        err = QMF_DOMAIN_INIT_ERROR;
    }
    err
}

/// `FDK_getWorkBuffer(pWorkBuffer, workBufferOffset, workBufferSectSize, memSize)`: the address
/// of the requested memory in the work buffer, which is divided into `QMF_MAX_WB_SECTIONS`
/// sections of `workBufferSectSize` values.
///
/// A section size of 0 has no defined result in C; the port computes section 0 and the offset
/// (the `aarch64` result).
pub fn fdk_get_work_buffer(
    work_buffer_offset: u16,
    work_buffer_sect_size: u16,
    mem_size: u16,
) -> QmfSlotPtr {
    // a section must be a multiple of the number of processing bands (currently always 64)
    fdk_assert!((work_buffer_sect_size % 64) == 0);

    let (idx1, idx2) = if work_buffer_sect_size == 0 {
        (0, work_buffer_offset as i32)
    } else {
        // calculate offset within the section
        let idx2 = work_buffer_offset as i32 % work_buffer_sect_size as i32;
        // calculate section number
        let idx1 = (work_buffer_offset as i32 - idx2) / work_buffer_sect_size as i32;
        (idx1, idx2)
    };
    // maximum sectionnumber is QMF_MAX_WB_SECTIONS
    fdk_assert!(idx1 < QMF_MAX_WB_SECTIONS as i32);

    // check, whether buffer fits into selected section
    fdk_assert!((idx2 + mem_size as i32) <= work_buffer_sect_size as i32);

    // get requested address to workbuffer
    QmfSlotPtr::Work { sect: idx1 as u8, offset: idx2 as u32 }
}

/// `FDK_QmfDomain_FeedWorkBuffer()`: assigns the work buffer to channel `ch`.
fn fdk_qmf_domain_feed_work_buffer(
    qd: &mut FdkQmfDomain,
    ch: usize,
    work_buffer_offset: u16,
    work_buffer_sect_size: u16,
    size: i32,
) -> i32 {
    let qd_in = &mut qd.qmf_domain_in[ch];
    let mem_needed = qd_in.work_buf_n_bands as i32 * qd_in.work_buf_n_time_slots as i32 * CMPLX_MOD;
    if mem_needed > size {
        return 1;
    }
    qd_in.p_work_buffer = true;
    qd_in.work_buffer_offset = work_buffer_offset;
    qd_in.work_buffer_sect_size = work_buffer_sect_size;

    0
}

/// `FDK_QmfDomain_IsInitialized(qd)`: 1 if the analysis or synthesis filter bank states of the
/// first channel exist, 0 otherwise.
pub fn fdk_qmf_domain_is_initialized(qd: &FdkQmfDomain) -> i32 {
    if qd.qmf_domain_in[0].p_ana_qmf_states.is_none()
        && qd.qmf_domain_out[0].p_syn_qmf_states.is_none()
    {
        0
    } else {
        1
    }
}

/// `FDK_QmfDomain_InitFilterBank(qd, extra_flags)`: initializes the QMF analysis and synthesis
/// filter banks and sets up the QMF data representation. Returns 0 on success.
pub fn fdk_qmf_domain_init_filter_bank(qd: &mut FdkQmfDomain, extra_flags: u32) -> i32 {
    let mut err = 0;
    let gc = &mut qd.global_conf;
    let no_cols = gc.n_qmf_time_slots as i32;
    let lsb = gc.n_bands_analysis as i32;
    let usb = f_min(gc.n_bands_synthesis as i32, 64);
    let n_proc_bands = gc.n_qmf_proc_bands as i32;
    fdk_assert!(n_proc_bands % ALIGNMENT_DEFAULT as i32 == 0);

    if extra_flags & QMF_FLAG_MPSLDFB != 0 {
        gc.flags &= !QMF_FLAG_CLDFB;
        gc.flags |= QMF_FLAG_MPSLDFB;
    }
    let gc = *gc;
    for ch in 0..(gc.n_input_channels as usize).min(QMF_DOMAIN_MAX_CHANNELS) {
        // distribute memory to slots array
        let has_ov = qd.mem.overlap_buffer[ch].is_some(); // persistent memory for overlap
        if !has_ov && gc.n_qmf_ov_time_slots != 0 {
            err = 1;
            return err;
        }
        let mut ptr_ov =
            if has_ov { QmfSlotPtr::Overlap { ch: ch as u8, offset: 0 } } else { QmfSlotPtr::Null };
        // This assumes the workbuffer defined for ch0 is the big one being used to hold one
        // full frame of QMF data.
        let src = f_min(ch as i32, f_max(gc.n_qmf_proc_channels as i32 - 1, 0)) as usize;
        let ptr = qd.qmf_domain_in[src].p_work_buffer; // non-persistent workbuffer
        let mut work_buffer_offset = qd.qmf_domain_in[src].work_buffer_offset;
        let work_buffer_sect_size = qd.qmf_domain_in[src].work_buffer_sect_size;

        if !ptr && gc.n_qmf_time_slots != 0 {
            err = 1;
            return err;
        }

        let qd_in = &mut qd.qmf_domain_in[ch];
        qd_in.p_global_conf = true;
        let n_ov = gc.n_qmf_ov_time_slots as usize;
        let n_all = n_ov + gc.n_qmf_time_slots as usize;
        if let (Some(re), Some(im)) =
            (qd_in.h_qmf_slots_real.as_deref_mut(), qd_in.h_qmf_slots_imag.as_deref_mut())
        {
            for ts in 0..n_ov.min(re.len()) {
                re[ts] = ptr_ov;
                ptr_ov = ptr_ov.advance(n_proc_bands);
                im[ts] = ptr_ov;
                ptr_ov = ptr_ov.advance(n_proc_bands);
            }
            for ts in n_ov..n_all.min(re.len()) {
                re[ts] = fdk_get_work_buffer(
                    work_buffer_offset,
                    work_buffer_sect_size,
                    n_proc_bands as u16,
                );
                work_buffer_offset = work_buffer_offset.wrapping_add(n_proc_bands as u16);
                im[ts] = fdk_get_work_buffer(
                    work_buffer_offset,
                    work_buffer_sect_size,
                    n_proc_bands as u16,
                );
                work_buffer_offset = work_buffer_offset.wrapping_add(n_proc_bands as u16);
            }
        }
        let lsb_ch = if qd_in.fb.lsb == 0 { lsb } else { qd_in.fb.lsb };
        let usb_ch = if qd_in.fb.usb == 0 { usb } else { qd_in.fb.usb };
        err |= qmf_init_analysis_filter_bank(
            &mut qd_in.fb,
            qd_in.p_ana_qmf_states.as_deref_mut(),
            no_cols,
            lsb_ch,
            usb_ch,
            gc.n_bands_analysis as i32,
            gc.flags | extra_flags,
        );
    }

    for ch in 0..(gc.n_output_channels as usize).min(QMF_DOMAIN_MAX_CHANNELS) {
        let qd_out = &mut qd.qmf_domain_out[ch];
        let out_gain_m = qd_out.fb.out_gain_m;
        let out_gain_e = qd_out.fb.out_gain_e;
        let out_scale = qmf_get_out_scalefactor(&qd_out.fb);
        let lsb_ch = if qd_out.fb.lsb == 0 { lsb } else { qd_out.fb.lsb };
        let usb_ch = if qd_out.fb.usb == 0 { usb } else { qd_out.fb.usb };
        err |= qmf_init_synthesis_filter_bank(
            &mut qd_out.fb,
            qd_out.p_syn_qmf_states.as_deref_mut(),
            no_cols,
            lsb_ch,
            usb_ch,
            gc.n_bands_synthesis as i32,
            gc.flags | extra_flags,
        );
        if out_gain_m != 0 {
            qmf_change_out_gain(&mut qd_out.fb, out_gain_m, out_gain_e);
        }
        if out_scale != 0 {
            let states = qd_out.p_syn_qmf_states.as_deref_mut().unwrap_or(&mut []);
            qmf_change_out_scalefactor(&mut qd_out.fb, states, out_scale);
        }
    }

    err
}

/// `FDK_QmfDomain_SaveOverlap(qd_ch, offset)`: copies the overlap/delay part of the QMF data of
/// input channel `ch` into the persistent memory, for the next frame.
pub fn fdk_qmf_domain_save_overlap(qd: &mut FdkQmfDomain, ch: usize, offset: i32) {
    let FdkQmfDomain { global_conf: gc, qmf_domain_in, mem, .. } = qd;
    // (guarded: a channel index beyond the array does nothing)
    let Some(qd_ch) = qmf_domain_in.get_mut(ch) else { return };
    if !qd_ch.p_global_conf {
        return;
    }
    let ov_slots = gc.n_qmf_ov_time_slots as usize;
    let n_cols = gc.n_qmf_time_slots as usize;
    let n_proc_bands = gc.n_qmf_proc_bands as usize;
    let empty: &[QmfSlotPtr] = &[];
    let qmf_real = qd_ch.h_qmf_slots_real.as_deref().unwrap_or(empty);
    let get = |a: &[QmfSlotPtr], i: usize| a.get(i).copied().unwrap_or_default();

    // for high part it would be enough to save only used part of overlap area
    if let Some(qmf_imag) = qd_ch.h_qmf_slots_imag.as_deref() {
        for ts in (offset.max(0) as usize)..ov_slots {
            mem.copy(get(qmf_real, ts), get(qmf_real, n_cols + ts), n_proc_bands);
            mem.copy(get(qmf_imag, ts), get(qmf_imag, n_cols + ts), n_proc_bands);
        }
    } else {
        for ts in 0..ov_slots {
            mem.copy(get(qmf_real, ts), get(qmf_real, n_cols + ts), n_proc_bands);
        }
    }
    qd_ch.scaling.ov_lb_scale = qd_ch.scaling.lb_scale;
}

/// `SCALE2EXP(s)`: converts headroom bits to an exponent.
#[inline]
pub const fn scale2exp(s: i32) -> i32 {
    15i32.wrapping_sub(s)
}

/// `EXP2SCALE(e)`
#[inline]
pub const fn exp2scale(e: i32) -> i32 {
    15i32.wrapping_sub(e)
}

/// `FDK_QmfDomain_GetSlot(qd_ch, ts, start_band, stop_band, pQmfOutReal, pQmfOutImag,
/// exp_out)`: one slot of QMF data of input channel `ch`, scaled to the exponent `exp_out`.
/// `p_qmf_out_imag` is `None` for `NULL`.
pub fn fdk_qmf_domain_get_slot(
    qd: &FdkQmfDomain,
    ch: usize,
    ts: i32,
    start_band: i32,
    stop_band: i32,
    p_qmf_out_real: &mut [FixpDbl],
    p_qmf_out_imag: Option<&mut [FixpDbl]>,
    exp_out: i32,
) {
    // (guarded: a channel index beyond the array does nothing)
    let Some(qd_ch) = qd.qmf_domain_in.get(ch) else { return };
    if !qd_ch.p_global_conf {
        return;
    }
    let gc = &qd.global_conf;
    let ptr_at = |a: &Option<Box<[QmfSlotPtr]>>| {
        a.as_deref().and_then(|a| a.get(ts as usize).copied()).unwrap_or_default()
    };
    let real = qd.mem.slot(ptr_at(&qd_ch.h_qmf_slots_real));
    let imag = qd.mem.slot(ptr_at(&qd_ch.h_qmf_slots_imag));
    let ov_slots = gc.n_qmf_ov_time_slots as i32;
    let exp_lb =
        scale2exp(if ts < ov_slots { qd_ch.scaling.ov_lb_scale } else { qd_ch.scaling.lb_scale });
    let exp_hb = scale2exp(qd_ch.scaling.hb_scale);
    let lsb = qd_ch.fb.lsb;
    let usb = qd_ch.fb.usb;
    let mut b = start_band;

    let mut target_exp = ALGORITHMIC_SCALING_IN_ANALYSIS_FILTERBANK + qd_ch.fb.filter_scale;

    fdk_assert!(ts < (gc.n_qmf_time_slots as i32 + gc.n_qmf_ov_time_slots as i32));
    fdk_assert!(start_band >= 0);
    fdk_assert!(stop_band <= gc.n_qmf_proc_bands as i32);

    if qd_ch.fb.no_channels == 24 {
        target_exp -= 1;
    }

    // Limit scaling factors to maximum negative value to avoid faulty behaviour due to
    // right-shifts. Corresponding asserts were observed during robustness testing.
    let lb_sf = f_max(exp_lb.wrapping_sub(target_exp).wrapping_sub(exp_out), -31);
    fdk_assert!(lb_sf < 32);
    let hb_sf = f_max(exp_hb.wrapping_sub(target_exp).wrapping_sub(exp_out), -31);
    fdk_assert!(hb_sf < 32);

    // Bounded: reads of `real[b]`/`imag[b]` past the end of their buffers read 0.
    let at = |s: &[FixpDbl], b: i32| s.get(b as usize).copied().unwrap_or(0);
    // Writes outside the output slices are dropped.
    let put = |s: &mut [FixpDbl], b: i32, v: FixpDbl| {
        if let Some(x) = s.get_mut(b as usize) {
            *x = v;
        }
    };
    match p_qmf_out_imag {
        None => {
            while b < f_min(lsb, stop_band) {
                put(p_qmf_out_real, b, scale_value_saturate(at(real, b), lb_sf));
                b += 1;
            }
            while b < f_min(usb, stop_band) {
                put(p_qmf_out_real, b, scale_value_saturate(at(real, b), hb_sf));
                b += 1;
            }
            while b < stop_band {
                put(p_qmf_out_real, b, 0);
                b += 1;
            }
        }
        Some(p_qmf_out_imag) => {
            while b < f_min(lsb, stop_band) {
                put(p_qmf_out_real, b, scale_value_saturate(at(real, b), lb_sf));
                put(p_qmf_out_imag, b, scale_value_saturate(at(imag, b), lb_sf));
                b += 1;
            }
            while b < f_min(usb, stop_band) {
                put(p_qmf_out_real, b, scale_value_saturate(at(real, b), hb_sf));
                put(p_qmf_out_imag, b, scale_value_saturate(at(imag, b), hb_sf));
                b += 1;
            }
            while b < stop_band {
                put(p_qmf_out_real, b, 0);
                put(p_qmf_out_imag, b, 0);
                b += 1;
            }
        }
    }
}

/// `FDK_QmfDomain_GetWorkBuffer(qd_ch, ts, &pQmfReal, &pQmfImag)`: direct access to the work
/// buffer of input channel `ch` (no time slot pointer array is used). Returns the pointers to
/// the real and the imaginary slot `ts` (resolve them with [`QmfDomainMem::slot_mut`]).
pub fn fdk_qmf_domain_get_work_buffer(
    qd: &FdkQmfDomain,
    ch: usize,
    ts: i32,
) -> (QmfSlotPtr, QmfSlotPtr) {
    let qd_ch = &qd.qmf_domain_in[ch];
    let bands = qd_ch.work_buf_n_bands as i32;
    let work_buffer_offset = qd_ch.work_buffer_offset as i32;
    let work_buffer_sect_size = qd_ch.work_buffer_sect_size;

    fdk_assert!(bands > 0);
    fdk_assert!(ts < qd_ch.work_buf_n_time_slots as i32);

    if !qd_ch.p_work_buffer {
        return (QmfSlotPtr::Null, QmfSlotPtr::Null);
    }
    (
        fdk_get_work_buffer(
            (work_buffer_offset + (ts * CMPLX_MOD) * bands) as u16,
            work_buffer_sect_size,
            bands as u16,
        ),
        fdk_get_work_buffer(
            (work_buffer_offset + (ts * CMPLX_MOD + 1) * bands) as u16,
            work_buffer_sect_size,
            bands as u16,
        ),
    )
}

/// The pointer `hQmfSlots[i]` (`NULL` without array or past its end).
fn slot_ptr(a: &Option<Box<[QmfSlotPtr]>>, i: usize) -> QmfSlotPtr {
    a.as_deref().and_then(|a| a.get(i).copied()).unwrap_or_default()
}

/// `FDK_QmfDomain_WorkBuffer2ProcChannel(qd_ch)`: if the work buffer of input channel `ch` is not
/// the work buffer of the processing channel, copies its QMF data into the processing channel.
pub fn fdk_qmf_domain_work_buffer2_proc_channel(qd: &mut FdkQmfDomain, ch: usize) {
    let FdkQmfDomain { global_conf: gc, qmf_domain_in, mem, .. } = qd;
    let qd_ch = &qmf_domain_in[ch];
    if !qd_ch.p_global_conf || !qd_ch.p_work_buffer {
        return;
    }
    let mut work_buffer_offset = qd_ch.work_buffer_offset;
    let work_buffer_sect_size = qd_ch.work_buffer_sect_size;
    let n_ov = gc.n_qmf_ov_time_slots as usize;

    if fdk_get_work_buffer(work_buffer_offset, work_buffer_sect_size, qd_ch.work_buf_n_bands as u16)
        == slot_ptr(&qd_ch.h_qmf_slots_real, n_ov)
    {
        // work buffer is part of processing channel => nothing to do
    } else {
        // copy parked new QMF data to processing channel
        let bands = qd_ch.work_buf_n_bands as u16;
        let slots = qd_ch.work_buf_n_time_slots as usize;
        for ts in 0..slots {
            // parkBuf_to_anaMatrix
            mem.copy(
                slot_ptr(&qd_ch.h_qmf_slots_real, n_ov + ts),
                fdk_get_work_buffer(work_buffer_offset, work_buffer_sect_size, bands),
                bands as usize,
            );
            work_buffer_offset = work_buffer_offset.wrapping_add(bands);
            mem.copy(
                slot_ptr(&qd_ch.h_qmf_slots_imag, n_ov + ts),
                fdk_get_work_buffer(work_buffer_offset, work_buffer_sect_size, bands),
                bands as usize,
            );
            work_buffer_offset = work_buffer_offset.wrapping_add(bands);
        }
    }
}

/// `FDK_QmfDomain_QmfData2HBE(qd_ch, ppQmfReal, ppQmfImag)`: for stereoConfigIndex 3 with HBE,
/// exchanges the QMF data of input channel `ch` with the HBE buffers `pp_qmf_real` and
/// `pp_qmf_imag` (one slot per element).
pub fn fdk_qmf_domain_qmf_data2_hbe<R: AsMut<[FixpDbl]>>(
    qd: &mut FdkQmfDomain,
    ch: usize,
    pp_qmf_real: &mut [R],
    pp_qmf_imag: &mut [R],
) {
    let FdkQmfDomain { global_conf: gc, qmf_domain_in, mem, .. } = qd;
    let qd_ch = &qmf_domain_in[ch];
    if !qd_ch.p_global_conf || !qd_ch.p_work_buffer {
        return;
    }
    let mut work_buffer_offset = qd_ch.work_buffer_offset;
    let work_buffer_sect_size = qd_ch.work_buffer_sect_size;
    let n_ov = gc.n_qmf_ov_time_slots as usize;

    // Copies of at most `n` values between a slot and an HBE buffer (limited to their ends).
    fn to_slot(mem: &mut QmfDomainMem, dst: QmfSlotPtr, src: &[FixpDbl], n: usize) {
        let d = mem.slot_mut(dst);
        let n = n.min(d.len()).min(src.len());
        d[..n].copy_from_slice(&src[..n]);
    }
    fn from_slot(mem: &QmfDomainMem, dst: &mut [FixpDbl], src: QmfSlotPtr, n: usize) {
        let s = mem.slot(src);
        let n = n.min(dst.len()).min(s.len());
        dst[..n].copy_from_slice(&s[..n]);
    }

    if fdk_get_work_buffer(work_buffer_offset, work_buffer_sect_size, qd_ch.work_buf_n_bands as u16)
        == slot_ptr(&qd_ch.h_qmf_slots_real, n_ov)
    {
        // left channel (anaMatrix)
        let bands = gc.n_bands_analysis as usize;
        let slots = qd_ch.work_buf_n_time_slots as usize;
        fdk_assert!(bands <= 64);
        // Deviation: more than 64 bands have no defined result in C; the port copies 64.
        let bands = bands.min(64);
        for ts in 0..slots {
            // copy current data of processing channel
            let mut tmp = [0 as FixpDbl; 64]; // one slot
                                              // real
            let p = slot_ptr(&qd_ch.h_qmf_slots_real, n_ov + ts);
            from_slot(mem, &mut tmp, p, bands); // anaMatrix_to_tmp
            to_slot(mem, p, pp_qmf_real[ts].as_mut(), bands); // HBE_to_anaMatrix
            copy_to(pp_qmf_real[ts].as_mut(), &tmp, bands); // tmp_to_HBE
                                                            // imag
            let p = slot_ptr(&qd_ch.h_qmf_slots_imag, n_ov + ts);
            from_slot(mem, &mut tmp, p, bands);
            to_slot(mem, p, pp_qmf_imag[ts].as_mut(), bands);
            copy_to(pp_qmf_imag[ts].as_mut(), &tmp, bands);
        }
    } else {
        // right channel (parkBuf)
        let bands = qd_ch.work_buf_n_bands as u16;
        let slots = qd_ch.work_buf_n_time_slots as usize;
        fdk_assert!(qd_ch.work_buf_n_bands == gc.n_bands_analysis);
        for ts in 0..slots {
            // copy HBE QMF data buffer to processing channel
            // HBE_to_anaMatrix
            to_slot(
                mem,
                slot_ptr(&qd_ch.h_qmf_slots_real, n_ov + ts),
                pp_qmf_real[ts].as_mut(),
                bands as usize,
            );
            to_slot(
                mem,
                slot_ptr(&qd_ch.h_qmf_slots_imag, n_ov + ts),
                pp_qmf_imag[ts].as_mut(),
                bands as usize,
            );
            // copy parked new QMF data to HBE QMF data buffer
            // parkBuf_to_HBE
            from_slot(
                mem,
                pp_qmf_real[ts].as_mut(),
                fdk_get_work_buffer(work_buffer_offset, work_buffer_sect_size, bands),
                bands as usize,
            );
            work_buffer_offset = work_buffer_offset.wrapping_add(bands);
            from_slot(
                mem,
                pp_qmf_imag[ts].as_mut(),
                fdk_get_work_buffer(work_buffer_offset, work_buffer_sect_size, bands),
                bands as usize,
            );
            work_buffer_offset = work_buffer_offset.wrapping_add(bands);
        }
    }
}

/// `FDKmemcpy(dst, src, n * sizeof(FIXP_DBL))`, limited to the ends of both.
fn copy_to(dst: &mut [FixpDbl], src: &[FixpDbl], n: usize) {
    let n = n.min(dst.len()).min(src.len());
    dst[..n].copy_from_slice(&src[..n]);
}

/// `FDK_QmfDomain_ClearRequested(hgc)`: sets all requested parameters of the configuration to
/// 0.
pub fn fdk_qmf_domain_clear_requested(hgc: &mut FdkQmfDomainGc) {
    hgc.qmf_domain_explicit_config = 0;
    hgc.flags_requested = 0;
    hgc.n_input_channels_requested = 0;
    hgc.n_output_channels_requested = 0;
    hgc.park_channel_requested = 0;
    hgc.n_bands_analysis_requested = 0;
    hgc.n_bands_synthesis_requested = 0;
    hgc.n_qmf_time_slots_requested = 0;
    hgc.n_qmf_ov_time_slots_requested = 0;
    hgc.n_qmf_proc_bands_requested = 0;
    hgc.n_qmf_proc_channels_requested = 0;
}

/// `FDK_QmfDomain_ClearConfigured()`
fn fdk_qmf_domain_clear_configured(hgc: &mut FdkQmfDomainGc) {
    hgc.flags = 0;
    hgc.n_input_channels = 0;
    hgc.n_output_channels = 0;
    hgc.park_channel = 0;
    hgc.n_bands_analysis = 0;
    hgc.n_bands_synthesis = 0;
    hgc.n_qmf_time_slots = 0;
    hgc.n_qmf_ov_time_slots = 0;
    hgc.n_qmf_proc_bands = 0;
    hgc.n_qmf_proc_channels = 0;
}

/// `FDK_QmfDomain_ClearFilterBank()`
fn fdk_qmf_domain_clear_filter_bank(hqd: &mut FdkQmfDomain) {
    for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
        hqd.qmf_domain_in[ch].fb = QmfFilterBank::default();
    }

    for ch in 0..QMF_DOMAIN_MAX_CHANNELS {
        hqd.qmf_domain_out[ch].fb = QmfFilterBank::default();
    }
}

/// `FDK_QmfDomain_Configure(hqd)`: checks the requested parameters of the configuration and
/// (re-)configures the QMF domain accordingly.
pub fn fdk_qmf_domain_configure(hqd: &mut FdkQmfDomain) -> QmfDomainError {
    let mut err = QMF_DOMAIN_OK;
    let mut size_temp = 0i32;

    let mut has_changed = false;

    {
        let hgc = &hqd.global_conf;
        if hgc.n_qmf_proc_channels_requested > 0 && hgc.n_qmf_proc_bands_requested != 64 {
            return QMF_DOMAIN_INIT_ERROR;
        }
        if hgc.n_bands_analysis_requested > hgc.n_qmf_proc_bands_requested {
            // In general the output of the qmf analysis is written to QMF memory slots which
            // size is defined by nQmfProcBands. nBandsSynthesis may be larger than
            // nQmfProcBands. This is e.g. the case if the QMF based resampler is used.
            return QMF_DOMAIN_INIT_ERROR;
        }
    }

    'bail: {
        // 1. adjust change of processing channels by comparison of current and requested
        // parameters
        let hgc = &mut hqd.global_conf;
        if hgc.n_qmf_proc_channels != hgc.n_qmf_proc_channels_requested
            || hgc.n_qmf_proc_bands != hgc.n_qmf_proc_bands_requested
            || hgc.n_qmf_time_slots != hgc.n_qmf_time_slots_requested
        {
            // Deviation: channels past QmfDomainIn[] are skipped.
            for i in 0..(hgc.n_qmf_proc_channels_requested as usize).min(QMF_DOMAIN_MAX_CHANNELS) {
                hqd.qmf_domain_in[i].work_buf_n_bands = hgc.n_qmf_proc_bands_requested;
                hgc.n_qmf_proc_bands = hgc.n_qmf_proc_bands_requested;

                hqd.qmf_domain_in[i].work_buf_n_time_slots = hgc.n_qmf_time_slots_requested;
            }

            // keep highest value encountered so far as allocated
            hgc.n_qmf_proc_channels = hgc.n_qmf_proc_channels_requested;

            has_changed = true;
        }

        // 2. reallocate persistent memory if necessary (analysis state-buffers,
        // timeslot-pointer-array, overlap-buffers, synthesis state-buffers)
        if hgc.n_input_channels != hgc.n_input_channels_requested
            || hgc.n_bands_analysis != hgc.n_bands_analysis_requested
            || hgc.n_qmf_time_slots != hgc.n_qmf_time_slots_requested
            || hgc.n_qmf_ov_time_slots != hgc.n_qmf_ov_time_slots_requested
            || hgc.n_output_channels != hgc.n_output_channels_requested
            || hgc.n_bands_synthesis != hgc.n_bands_synthesis_requested
            || hgc.park_channel != hgc.park_channel_requested
        {
            hgc.n_input_channels = hgc.n_input_channels_requested;
            hgc.n_bands_analysis = hgc.n_bands_analysis_requested;
            hgc.n_qmf_time_slots = hgc.n_qmf_time_slots_requested;
            hgc.n_qmf_ov_time_slots = hgc.n_qmf_ov_time_slots_requested;
            hgc.n_output_channels = hgc.n_output_channels_requested;
            hgc.n_bands_synthesis = hgc.n_bands_synthesis_requested;
            hgc.park_channel = hgc.park_channel_requested;

            if fdk_qmf_domain_allocate_persistent_memory(hqd) != 0 {
                err = QMF_DOMAIN_OUT_OF_MEMORY;
                break 'bail;
            }

            // 3. set request-flag for downsampled SBR
            let hgc = &mut hqd.global_conf;
            if hgc.n_bands_analysis == 32
                && hgc.n_bands_synthesis == 32
                && hgc.flags & (QMF_FLAG_CLDFB | QMF_FLAG_MPSLDFB) == 0
            {
                hgc.flags_requested |= QMF_FLAG_DOWNSAMPLED;
            }

            has_changed = true;
        }

        // 4. initialize tables and buffer for QMF-resampler

        // 5. set requested flags
        let hgc = &mut hqd.global_conf;
        if hgc.flags != hgc.flags_requested {
            if hgc.flags_requested & QMF_FLAG_MPSLDFB != 0
                && hgc.flags_requested & QMF_FLAG_CLDFB != 0
            {
                hgc.flags_requested &= !QMF_FLAG_CLDFB;
            }
            hgc.flags = hgc.flags_requested;
            has_changed = true;
        }

        if has_changed {
            // 6. recalculate and check size of required workbuffer-space

            if hqd.global_conf.park_channel != 0 && hqd.global_conf.n_qmf_proc_channels == 1 {
                // configure temp QMF buffer for parking right channel MPS212 output,
                // (USAC stereoConfigIndex 3 only)
                hqd.qmf_domain_in[1].work_buf_n_bands = hqd.global_conf.n_bands_analysis;
                hqd.qmf_domain_in[1].work_buf_n_time_slots = hqd.global_conf.n_qmf_time_slots;
                size_temp = hqd.qmf_domain_in[1].work_buf_n_bands as i32
                    * hqd.qmf_domain_in[1].work_buf_n_time_slots as i32
                    * CMPLX_MOD;
            }

            let size_main = hqd.qmf_domain_in[0].work_buf_n_bands as i32
                * hqd.qmf_domain_in[0].work_buf_n_time_slots as i32
                * CMPLX_MOD;

            let hgc = hqd.global_conf;
            let size = size_main * hgc.n_qmf_proc_channels as i32 + size_temp;

            if size > QMF_MAX_WB_SECTIONS as i32 * QMF_WB_SECTION_SIZE {
                err = QMF_DOMAIN_OUT_OF_MEMORY;
                break 'bail;
            }

            // 7. allocate additional workbuffer if necessary
            // (QmfWorkBufferCore6, 1, 3, 4, 7 of QMF_WB_SECTION_SIZE values each)
            let wb = &mut hqd.mem.work_buffer;
            for (k, w) in wb.iter_mut().enumerate() {
                if size > k as i32 * QMF_WB_SECTION_SIZE && w.is_none() {
                    // get work buffer of size QMF_WB_SECTION_SIZE
                    *w = alloc(QMF_WB_SECTION_SIZE as usize);
                }
            }

            // 8. distribute workbuffer over processing channels
            let mut i = 0usize;
            while i < (hgc.n_qmf_proc_channels as usize).min(QMF_DOMAIN_MAX_CHANNELS) {
                fdk_qmf_domain_feed_work_buffer(
                    hqd,
                    i,
                    (size_main * i as i32) as u16,
                    QMF_WB_SECTION_SIZE as u16,
                    size_main,
                );
                i += 1;
            }
            i = i.max(hgc.n_qmf_proc_channels as usize);
            if hgc.park_channel != 0 {
                while i < hgc.n_input_channels as usize {
                    fdk_qmf_domain_feed_work_buffer(
                        hqd,
                        1,
                        (size_main * hgc.n_qmf_proc_channels as i32) as u16,
                        QMF_WB_SECTION_SIZE as u16,
                        size_temp,
                    );
                    i += 1;
                }
            }

            // 9. (re-)init filterbank
            for i in 0..(hgc.n_output_channels as usize).min(QMF_DOMAIN_MAX_CHANNELS) {
                let fb = &mut hqd.qmf_domain_out[i].fb;
                if fb.lsb == 0 && fb.usb == 0 {
                    // Although lsb and usb are set in the SBR module, they are initialized at
                    // this point due to the case of using MPS without SBR.
                    fb.lsb = hgc.n_bands_analysis_requested as i32;
                    fb.usb = f_min(hgc.n_bands_synthesis_requested as i32, 64);
                }
            }
            if fdk_qmf_domain_init_filter_bank(hqd, 0) != 0 {
                err = QMF_DOMAIN_INIT_ERROR;
            }
        }
    }

    // bail:
    if err != 0 {
        fdk_qmf_domain_free_mem(hqd);
    }
    err
}

/// `FDK_QmfDomain_FreeWorkBuffer()`
fn fdk_qmf_domain_free_work_buffer(hqd: &mut FdkQmfDomain) {
    for w in hqd.mem.work_buffer.iter_mut() {
        *w = None;
    }
}

/// `FDK_QmfDomain_FreeMem(hqd)`: frees the QMF work buffer, the persistent memory and the
/// configuration.
pub fn fdk_qmf_domain_free_mem(hqd: &mut FdkQmfDomain) {
    fdk_qmf_domain_free_work_buffer(hqd);

    fdk_qmf_domain_free_persistent_memory(hqd);

    fdk_qmf_domain_clear_filter_bank(hqd);

    fdk_qmf_domain_clear_configured(&mut hqd.global_conf);

    fdk_qmf_domain_clear_requested(&mut hqd.global_conf);
}

/// `FDK_QmfDomain_Close(hqd)`: frees the QMF work buffer and the persistent memory.
pub fn fdk_qmf_domain_close(hqd: &mut FdkQmfDomain) {
    fdk_qmf_domain_free_work_buffer(hqd);

    fdk_qmf_domain_free_persistent_memory(hqd);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::testutil::Rng;

    /// The previous `slots_mut` (always sorting), as the reference of the test below.
    fn slots_mut_sorting(
        mem: &mut QmfDomainMem,
        ptrs: &[QmfSlotPtr],
        len: usize,
    ) -> Vec<(usize, usize)> {
        let region_of = |p: QmfSlotPtr| -> Option<(usize, usize)> {
            match p {
                QmfSlotPtr::Null => None,
                QmfSlotPtr::Work { sect, offset } => ((sect as usize) < QMF_MAX_WB_SECTIONS)
                    .then_some((sect as usize, offset as usize)),
                QmfSlotPtr::Overlap { ch, offset } => ((ch as usize) < QMF_DOMAIN_MAX_CHANNELS)
                    .then_some((QMF_MAX_WB_SECTIONS + ch as usize, offset as usize)),
            }
        };
        let mut order: Vec<(usize, usize, usize)> = ptrs
            .iter()
            .enumerate()
            .filter_map(|(i, &p)| region_of(p).map(|(r, o)| (r, o, i)))
            .collect();
        order.sort_unstable();
        let lens: Vec<usize> = mem
            .work_buffer
            .iter()
            .chain(mem.overlap_buffer.iter())
            .map(|b| b.as_ref().map_or(usize::MAX, |b| b.len()))
            .collect();
        let bases: Vec<usize> = mem
            .work_buffer
            .iter()
            .chain(mem.overlap_buffer.iter())
            .map(|b| b.as_ref().map_or(0, |b| b.as_ptr() as usize))
            .collect();
        let mut out = vec![(usize::MAX, 0usize); ptrs.len()];
        let mut current = usize::MAX;
        let (mut consumed, mut avail) = (0usize, 0usize);
        for (r, off, i) in order {
            if r != current {
                current = r;
                consumed = 0;
                avail = if lens[r] == usize::MAX { 0 } else { lens[r] };
            }
            if off < consumed || off > avail {
                continue;
            }
            let n = len.min(avail - off);
            out[i] = (bases[r] + 4 * off, n);
            consumed = off + n;
        }
        out
    }

    #[test]
    fn slots_mut_equals_sorting_version() {
        let mut r = Rng(0x5eed_51a7);
        for case in 0..3000 {
            let mut mem = QmfDomainMem::default();
            for b in mem.work_buffer.iter_mut().chain(mem.overlap_buffer.iter_mut()) {
                if r.next() % 3 != 0 {
                    *b = Some(vec![0; r.range(0, 700) as usize].into_boxed_slice());
                }
            }
            let n = r.range(0, 40) as usize;
            let len = r.range(0, 80) as usize;
            let sorted = r.next() % 2 == 0;
            let mut off = 0u32;
            let ptrs: Vec<QmfSlotPtr> = (0..n)
                .map(|_| {
                    if sorted {
                        off += r.range(0, 90) as u32;
                        if r.next() % 9 == 0 {
                            QmfSlotPtr::Null
                        } else {
                            QmfSlotPtr::Work { sect: (off / 600) as u8, offset: off % 600 }
                        }
                    } else {
                        match r.next() % 4 {
                            0 => QmfSlotPtr::Null,
                            1 => QmfSlotPtr::Overlap {
                                ch: r.range(0, 10) as u8,
                                offset: r.range(0, 720) as u32,
                            },
                            _ => QmfSlotPtr::Work {
                                sect: r.range(0, 5) as u8,
                                offset: r.range(0, 720) as u32,
                            },
                        }
                    }
                })
                .collect();
            let want = slots_mut_sorting(&mut mem, &ptrs, len);
            let got: Vec<(usize, usize)> = mem
                .slots_mut(&ptrs, len)
                .iter()
                .map(
                    |s| if s.is_empty() { (usize::MAX, 0) } else { (s.as_ptr() as usize, s.len()) },
                )
                .collect();
            let want: Vec<(usize, usize)> =
                want.into_iter().map(|w| if w.1 == 0 { (usize::MAX, 0) } else { w }).collect();
            assert_eq!(got, want, "case {case}");
        }
    }
}

#[cfg(test)]
mod tests_channel {
    use super::*;
    use crate::fft::testutil::Rng;

    /// `channel_slots_into(re, im)` against `slots_mut(re ++ im)` for the layouts of the QMF
    /// domain (interleaved ascending slots, also with ties and NULL) and random pointers.
    #[test]
    fn channel_slots_into_equals_slots_mut() {
        let mut r = Rng(0x5eed_c4a5);
        for case in 0..4000 {
            let mut mem = QmfDomainMem::default();
            for b in mem.work_buffer.iter_mut().chain(mem.overlap_buffer.iter_mut()) {
                if r.next() % 4 != 0 {
                    *b = Some(vec![0; r.range(0, 2048) as usize].into_boxed_slice());
                }
            }
            let n = r.range(0, QMF_SLOTS_SIZE as i32) as usize;
            let len = [64, 32, r.range(0, 80) as usize][r.range(0, 2) as usize];
            let mode = r.next() % 3;
            let (mut re, mut im) = (vec![QmfSlotPtr::Null; n], vec![QmfSlotPtr::Null; n]);
            let mut ptr = QmfSlotPtr::Overlap { ch: r.range(0, 8) as u8, offset: 0 };
            let step = r.range(0, 70);
            let n_ov = r.range(0, n as i32) as usize;
            for i in 0..n {
                if i == n_ov {
                    ptr = QmfSlotPtr::Work { sect: r.range(0, 4) as u8, offset: 0 };
                }
                let mut next = || {
                    let p = match mode {
                        0 | 1 => ptr,
                        _ => match r.next() % 3 {
                            0 => QmfSlotPtr::Null,
                            1 => QmfSlotPtr::Overlap {
                                ch: r.range(0, 9) as u8,
                                offset: r.range(0, 2100) as u32,
                            },
                            _ => QmfSlotPtr::Work {
                                sect: r.range(0, 5) as u8,
                                offset: r.range(0, 2100) as u32,
                            },
                        },
                    };
                    let st = if mode == 1 && r.next() % 8 == 0 { 0 } else { step };
                    ptr = ptr.advance(st);
                    p
                };
                re[i] = next();
                im[i] = next();
            }
            let all: Vec<QmfSlotPtr> = re.iter().chain(&im).copied().collect();
            let key = |s: &&mut [FixpDbl]| (s.as_ptr() as usize * !s.is_empty() as usize, s.len());
            let want: Vec<(usize, usize)> = mem.slots_mut(&all, len).iter().map(key).collect();
            let (mut a, mut b) = (SlotList::new(), SlotList::new());
            mem.channel_slots_into(&re, &im, len, &mut a, &mut b);
            let got: Vec<(usize, usize)> = a.iter().chain(b.iter()).map(key).collect();
            assert_eq!(got, want, "case {case} mode {mode}");
        }
    }

    /// `copy` against the copy through a temporary vector: random buffers (some missing), slot
    /// pointers into the same and other buffers (overlapping ranges, offsets at and past the
    /// ends, invalid sections and channels, NULL) and counts.
    #[test]
    fn copy_equals_straight() {
        let mut r = Rng(0xc0b1);
        for case in 0..20000 {
            let mut mem = QmfDomainMem::default();
            for b in mem.work_buffer.iter_mut().chain(mem.overlap_buffer.iter_mut()) {
                if r.next() % 4 != 0 {
                    let n = r.range(0, 300) as usize;
                    *b = Some((0..n).map(|_| r.next() as i32).collect());
                }
            }
            let ptr = |r: &mut Rng| -> QmfSlotPtr {
                let offset = if r.next() % 8 == 0 {
                    (r.next() % 400) as u32
                } else {
                    r.range(0, 310) as u32
                };
                match r.next() % 9 {
                    0 => QmfSlotPtr::Null,
                    1..=4 => QmfSlotPtr::Work { sect: r.range(0, 6) as u8, offset },
                    _ => QmfSlotPtr::Overlap { ch: r.range(0, 10) as u8, offset },
                }
            };
            let (dst, mut src) = (ptr(&mut r), ptr(&mut r));
            if r.next() % 3 == 0 {
                // the same buffer, overlapping ranges
                src = dst.advance(r.range(-40, 40));
            }
            let n = r.range(0, 320) as usize;
            let mut a = mem.clone();
            mem.copy_straight(dst, src, n);
            a.copy(dst, src, n);
            assert_eq!(a.work_buffer, mem.work_buffer, "case {case}");
            assert_eq!(a.overlap_buffer, mem.overlap_buffer, "case {case}");
        }
    }
}
