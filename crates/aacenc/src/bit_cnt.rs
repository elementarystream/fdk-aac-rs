// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/bit_cnt.h, libAACenc/src/bit_cnt.cpp.

//! Port of `bit_cnt.h`: Huffman bit counting and coding.
//!
//! The counting and coding functions index the Huffman tables with the spectral values; the
//! callers (`dyn_bits`, `bitenc`, `qc_main`) derive `maxVal` and the code books from the same
//! quantized spectrum. The port reproduces the layout of the Huffman tables of `aacEnc_rom.o`,
//! modelled per variant, and reads a defined value where the reference has no defined result
//! (see `huff_rom_read`). Nothing panics.

use crate::aac_enc_rom::*;
use fdk_aac_rs_fdk::bitstream::{fdk_write_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::fix_min;
use fdk_aac_rs_fdk::generic_stds::FDK_INT_MAX;

/// `INVALID_BITCOUNT`
pub const INVALID_BITCOUNT: i32 = FDK_INT_MAX / 4;

// enum codeBookNo
pub const CODE_BOOK_ZERO_NO: i32 = 0;
pub const CODE_BOOK_1_NO: i32 = 1;
pub const CODE_BOOK_2_NO: i32 = 2;
pub const CODE_BOOK_3_NO: i32 = 3;
pub const CODE_BOOK_4_NO: i32 = 4;
pub const CODE_BOOK_5_NO: i32 = 5;
pub const CODE_BOOK_6_NO: i32 = 6;
pub const CODE_BOOK_7_NO: i32 = 7;
pub const CODE_BOOK_8_NO: i32 = 8;
pub const CODE_BOOK_9_NO: i32 = 9;
pub const CODE_BOOK_10_NO: i32 = 10;
pub const CODE_BOOK_ESC_NO: i32 = 11;
pub const CODE_BOOK_RES_NO: i32 = 12;
pub const CODE_BOOK_PNS_NO: i32 = 13;
pub const CODE_BOOK_IS_OUT_OF_PHASE_NO: i32 = 14;
pub const CODE_BOOK_IS_IN_PHASE_NO: i32 = 15;

// enum codeBookNdx
pub const CODE_BOOK_ZERO_NDX: i32 = 0;
pub const CODE_BOOK_1_NDX: i32 = 1;
pub const CODE_BOOK_2_NDX: i32 = 2;
pub const CODE_BOOK_3_NDX: i32 = 3;
pub const CODE_BOOK_4_NDX: i32 = 4;
pub const CODE_BOOK_5_NDX: i32 = 5;
pub const CODE_BOOK_6_NDX: i32 = 6;
pub const CODE_BOOK_7_NDX: i32 = 7;
pub const CODE_BOOK_8_NDX: i32 = 8;
pub const CODE_BOOK_9_NDX: i32 = 9;
pub const CODE_BOOK_10_NDX: i32 = 10;
pub const CODE_BOOK_ESC_NDX: i32 = 11;
pub const CODE_BOOK_RES_NDX: i32 = 12;
pub const CODE_BOOK_PNS_NDX: i32 = 13;
pub const CODE_BOOK_IS_OUT_OF_PHASE_NDX: i32 = 14;
pub const CODE_BOOK_IS_IN_PHASE_NDX: i32 = 15;
pub const NUMBER_OF_CODE_BOOKS: i32 = 16;

// enum codeBookLav
pub const CODE_BOOK_ZERO_LAV: i32 = 0;
pub const CODE_BOOK_1_LAV: i32 = 1;
pub const CODE_BOOK_2_LAV: i32 = 1;
pub const CODE_BOOK_3_LAV: i32 = 2;
pub const CODE_BOOK_4_LAV: i32 = 2;
pub const CODE_BOOK_5_LAV: i32 = 4;
pub const CODE_BOOK_6_LAV: i32 = 4;
pub const CODE_BOOK_7_LAV: i32 = 7;
pub const CODE_BOOK_8_LAV: i32 = 7;
pub const CODE_BOOK_9_LAV: i32 = 12;
pub const CODE_BOOK_10_LAV: i32 = 12;
pub const CODE_BOOK_ESC_LAV: i32 = 16;
pub const CODE_BOOK_SCF_LAV: i32 = 60;
pub const CODE_BOOK_PNS_LAV: i32 = 60;

/// `FDKaacEnc_bitCountScalefactorDelta(delta)`: number of bits of the Huffman code of a
/// scalefactor delta.
///
/// `FDKaacEnc_huff_ltabscf[delta + 60]` under the reproduced table layout (see `huff_rom_read`);
/// where the reference has no defined result the port returns `INVALID_BITCOUNT`.
#[inline]
pub fn fdk_aac_enc_bit_count_scalefactor_delta(delta: i32) -> i32 {
    let idx = delta.wrapping_add(CODE_BOOK_SCF_LAV);
    fdk_aac_rs_fdk::fdk_assert!((0 <= idx) && (idx < FDK_AAC_ENC_HUFF_LTABSCF.len() as i32));
    match FDK_AAC_ENC_HUFF_LTABSCF.get(idx as usize) {
        Some(&v) => v as i32,
        None => huff_rom_read(ROM_LTABSCF, idx).map_or(INVALID_BITCOUNT, |v| v as i32),
    }
}

// ---------------------------------------------------------------------------------------------
// Table reads with C's address arithmetic
// ---------------------------------------------------------------------------------------------
//
// The counting and coding functions compute the address of a table element with the row-major
// arithmetic of the declared dimensions. The Huffman tables lie one after the other in the
// constant section of `aacEnc_rom.o`, in the order of the `ROM_*` indices below
// (`FDKaacEnc_huff_ltab1_2` at the start of the section, `FDKaacEnc_huff_ctabscf` last):
// - arm64: each table at the next multiple of its element size (no gaps);
// - x86-64: each table 16-byte aligned, the gaps zero.
// The port reads the same bytes (little endian) from its own tables. Where the reference has no
// defined result, the port reads 0 (`INVALID_BITCOUNT` in `bitCountScalefactorDelta`) and
// counts the call (`huff_rom_outside_reads()`). In-range reads cost one bounds check.

const ROM_LTAB1_2: usize = 0;
const ROM_LTAB3_4: usize = 1;
const ROM_LTAB5_6: usize = 2;
const ROM_LTAB7_8: usize = 3;
const ROM_LTAB9_10: usize = 4;
const ROM_LTAB11: usize = 5;
const ROM_LTABSCF: usize = 6;
const ROM_CTAB1: usize = 7;
const ROM_CTAB2: usize = 8;
const ROM_CTAB3: usize = 9;
const ROM_CTAB4: usize = 10;
const ROM_CTAB5: usize = 11;
const ROM_CTAB6: usize = 12;
const ROM_CTAB7: usize = 13;
const ROM_CTAB8: usize = 14;
const ROM_CTAB9: usize = 15;
const ROM_CTAB10: usize = 16;
const ROM_CTAB11: usize = 17;
const ROM_CTABSCF: usize = 18;
/// The number of Huffman tables in `aacEnc_rom.o` (`ltab1_2` .. `ctabscf`).
pub const HUFF_ROM_TABLES: usize = 19;

/// A Huffman table as the flat array of its elements.
#[derive(Clone, Copy)]
enum RomTab {
    U8(&'static [u8]),
    U16(&'static [u16]),
    U32(&'static [u32]),
}

impl RomTab {
    fn elem_size(self) -> i64 {
        match self {
            RomTab::U8(_) => 1,
            RomTab::U16(_) => 2,
            RomTab::U32(_) => 4,
        }
    }

    fn bytes(self) -> i64 {
        self.elem_size()
            * match self {
                RomTab::U8(t) => t.len(),
                RomTab::U16(t) => t.len(),
                RomTab::U32(t) => t.len(),
            } as i64
    }

    /// Byte `b` of the table in memory (little endian).
    fn byte(self, b: usize) -> u8 {
        match self {
            RomTab::U8(t) => t[b],
            RomTab::U16(t) => t[b / 2].to_le_bytes()[b % 2],
            RomTab::U32(t) => t[b / 4].to_le_bytes()[b % 4],
        }
    }
}

/// Huffman table `id` (`ROM_*`).
fn rom_tab(id: usize) -> RomTab {
    use RomTab::*;
    match id {
        ROM_LTAB1_2 => U32(FDK_AAC_ENC_HUFF_LTAB_1_2.as_flattened().as_flattened().as_flattened()),
        ROM_LTAB3_4 => U32(FDK_AAC_ENC_HUFF_LTAB_3_4.as_flattened().as_flattened().as_flattened()),
        ROM_LTAB5_6 => U32(FDK_AAC_ENC_HUFF_LTAB_5_6.as_flattened()),
        ROM_LTAB7_8 => U32(FDK_AAC_ENC_HUFF_LTAB_7_8.as_flattened()),
        ROM_LTAB9_10 => U32(FDK_AAC_ENC_HUFF_LTAB_9_10.as_flattened()),
        ROM_LTAB11 => U8(FDK_AAC_ENC_HUFF_LTAB_11.as_flattened()),
        ROM_LTABSCF => U8(&FDK_AAC_ENC_HUFF_LTABSCF),
        ROM_CTAB1 => U16(FDK_AAC_ENC_HUFF_CTAB_1.as_flattened().as_flattened().as_flattened()),
        ROM_CTAB2 => U16(FDK_AAC_ENC_HUFF_CTAB_2.as_flattened().as_flattened().as_flattened()),
        ROM_CTAB3 => U16(FDK_AAC_ENC_HUFF_CTAB_3.as_flattened().as_flattened().as_flattened()),
        ROM_CTAB4 => U16(FDK_AAC_ENC_HUFF_CTAB_4.as_flattened().as_flattened().as_flattened()),
        ROM_CTAB5 => U16(FDK_AAC_ENC_HUFF_CTAB_5.as_flattened()),
        ROM_CTAB6 => U16(FDK_AAC_ENC_HUFF_CTAB_6.as_flattened()),
        ROM_CTAB7 => U16(FDK_AAC_ENC_HUFF_CTAB_7.as_flattened()),
        ROM_CTAB8 => U16(FDK_AAC_ENC_HUFF_CTAB_8.as_flattened()),
        ROM_CTAB9 => U16(FDK_AAC_ENC_HUFF_CTAB_9.as_flattened()),
        ROM_CTAB10 => U16(FDK_AAC_ENC_HUFF_CTAB_10.as_flattened()),
        ROM_CTAB11 => U16(FDK_AAC_ENC_HUFF_CTAB_11.as_flattened()),
        _ => U32(&FDK_AAC_ENC_HUFF_CTABSCF),
    }
}

/// The byte offsets of the Huffman tables in the constant section of `aacEnc_rom.o` (relative to
/// `FDKaacEnc_huff_ltab1_2`, in the order `ltab1_2` .. `ctabscf`) and the end of the last one,
/// for the variant of this build.
pub fn huff_rom_layout() -> ([i64; HUFF_ROM_TABLES], i64) {
    use fdk_aac_rs_fdk::{Arch, ARCH};
    // clang aligns global arrays of 16 bytes or more to 16 bytes on x86-64 only; the arm64 and
    // the i686 objects pack the tables with their natural alignment (measured with
    // `objdump -t aacEnc_rom.cpp.o` of ref-x86_64 and ref-i686).
    let x86 = matches!(ARCH, Arch::X86);
    let mut off = [0i64; HUFF_ROM_TABLES];
    let mut pos = 0i64;
    for (id, o) in off.iter_mut().enumerate() {
        let t = rom_tab(id);
        let align = if x86 { 16 } else { t.elem_size() };
        pos = (pos + align - 1) / align * align;
        *o = pos;
        pos += t.bytes();
    }
    (off, pos)
}

thread_local! {
    static OUTSIDE_READS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// Counts calls without a defined reference result (test support).
#[doc(hidden)]
pub fn huff_rom_outside_reads() -> u64 {
    OUTSIDE_READS.with(|c| c.get())
}

/// Adds to the counter of [`huff_rom_outside_reads`] (the carry of the thread pool).
pub(crate) fn add_huff_rom_outside_reads(n: u64) {
    OUTSIDE_READS.with(|c| c.set(c.get().wrapping_add(n)));
}

/// Element `idx` (flat and row-major) of the Huffman table `id` under the reproduced layout of
/// the table block; `None` where the reference has no defined result.
#[cold]
#[inline(never)]
fn huff_rom_read(id: usize, idx: i32) -> Option<u32> {
    let (off, end) = huff_rom_layout();
    let es = rom_tab(id).elem_size();
    let start = off[id] + idx as i64 * es;
    if start < 0 || start + es > end {
        OUTSIDE_READS.with(|c| c.set(c.get() + 1));
        return None;
    }
    let mut v = 0u32;
    for k in 0..es {
        let b = start + k;
        let byte = (0..HUFF_ROM_TABLES)
            .find_map(|j| {
                let t = rom_tab(j);
                (b >= off[j] && b < off[j] + t.bytes()).then(|| t.byte((b - off[j]) as usize))
            })
            .unwrap_or(0); // alignment gap
        v |= (byte as u32) << (8 * k);
    }
    Some(v)
}

/// Element `idx` of a `ULONG` table under the reproduced layout (0 where the reference has no
/// defined result).
#[inline(always)]
fn rd32(t: &[u32], id: usize, idx: i32) -> u32 {
    match t.get(idx as usize) {
        Some(&v) => v,
        None => huff_rom_read(id, idx).unwrap_or(0),
    }
}

/// Element `idx` of a `USHORT` table under the reproduced layout (0 where the reference has no
/// defined result).
#[inline(always)]
fn rd16(t: &[u16], id: usize, idx: i32) -> u16 {
    match t.get(idx as usize) {
        Some(&v) => v,
        None => huff_rom_read(id, idx).unwrap_or(0) as u16,
    }
}

/// Element `idx` of a `UCHAR` table under the reproduced layout (0 where the reference has no
/// defined result).
#[inline(always)]
fn rd8(t: &[u8], id: usize, idx: i32) -> u8 {
    match t.get(idx as usize) {
        Some(&v) => v,
        None => huff_rom_read(id, idx).unwrap_or(0) as u8,
    }
}

// The indices are `SHORT` values plus small offsets: the flat index cannot overflow.

/// `FDKaacEnc_huff_ltab1_2[t0][t1][t2][t3]`
#[inline(always)]
fn ltab1_2(t0: i32, t1: i32, t2: i32, t3: i32) -> u32 {
    let t = FDK_AAC_ENC_HUFF_LTAB_1_2.as_flattened().as_flattened().as_flattened();
    rd32(t, ROM_LTAB1_2, ((t0 * 3 + t1) * 3 + t2) * 3 + t3)
}
/// `FDKaacEnc_huff_ltab3_4[t0][t1][t2][t3]`
#[inline(always)]
fn ltab3_4(t0: i32, t1: i32, t2: i32, t3: i32) -> u32 {
    let t = FDK_AAC_ENC_HUFF_LTAB_3_4.as_flattened().as_flattened().as_flattened();
    rd32(t, ROM_LTAB3_4, ((t0 * 3 + t1) * 3 + t2) * 3 + t3)
}
/// `FDKaacEnc_huff_ltab5_6[t0][t1]`
#[inline(always)]
fn ltab5_6(t0: i32, t1: i32) -> u32 {
    rd32(FDK_AAC_ENC_HUFF_LTAB_5_6.as_flattened(), ROM_LTAB5_6, t0 * 9 + t1)
}
/// `FDKaacEnc_huff_ltab7_8[t0][t1]`
#[inline(always)]
fn ltab7_8(t0: i32, t1: i32) -> u32 {
    rd32(FDK_AAC_ENC_HUFF_LTAB_7_8.as_flattened(), ROM_LTAB7_8, t0 * 8 + t1)
}
/// `FDKaacEnc_huff_ltab9_10[t0][t1]`
#[inline(always)]
fn ltab9_10(t0: i32, t1: i32) -> u32 {
    rd32(FDK_AAC_ENC_HUFF_LTAB_9_10.as_flattened(), ROM_LTAB9_10, t0 * 13 + t1)
}
/// `(INT)FDKaacEnc_huff_ltab11[t0][t1]`
#[inline(always)]
fn ltab11(t0: i32, t1: i32) -> i32 {
    rd8(FDK_AAC_ENC_HUFF_LTAB_11.as_flattened(), ROM_LTAB11, t0 * 17 + t1) as i32
}
/// `FDKaacEnc_huff_ctab{1,2,3,4}[t0][t1][t2][t3]` (`id`: `ROM_CTAB1` .. `ROM_CTAB4`)
#[inline(always)]
fn ctab1_4(id: usize, t0: i32, t1: i32, t2: i32, t3: i32) -> i32 {
    let t = match id {
        ROM_CTAB1 => &FDK_AAC_ENC_HUFF_CTAB_1,
        ROM_CTAB2 => &FDK_AAC_ENC_HUFF_CTAB_2,
        ROM_CTAB3 => &FDK_AAC_ENC_HUFF_CTAB_3,
        _ => &FDK_AAC_ENC_HUFF_CTAB_4,
    };
    let t = t.as_flattened().as_flattened().as_flattened();
    rd16(t, id, ((t0 * 3 + t1) * 3 + t2) * 3 + t3) as i32
}
/// `FDKaacEnc_huff_ctab{5,...,10}[t0][t1]` (`id`: `ROM_CTAB5` .. `ROM_CTAB10`)
#[inline(always)]
fn ctab5_10(id: usize, t0: i32, t1: i32) -> i32 {
    let (t, cols): (&[u16], i32) = match id {
        ROM_CTAB5 => (FDK_AAC_ENC_HUFF_CTAB_5.as_flattened(), 9),
        ROM_CTAB6 => (FDK_AAC_ENC_HUFF_CTAB_6.as_flattened(), 9),
        ROM_CTAB7 => (FDK_AAC_ENC_HUFF_CTAB_7.as_flattened(), 8),
        ROM_CTAB8 => (FDK_AAC_ENC_HUFF_CTAB_8.as_flattened(), 8),
        ROM_CTAB9 => (FDK_AAC_ENC_HUFF_CTAB_9.as_flattened(), 13),
        _ => (FDK_AAC_ENC_HUFF_CTAB_10.as_flattened(), 13),
    };
    rd16(t, id, t0 * cols + t1) as i32
}
/// `FDKaacEnc_huff_ctab11[t0][t1]` (`[21][17]`)
#[inline(always)]
fn ctab11(t0: i32, t1: i32) -> i32 {
    rd16(FDK_AAC_ENC_HUFF_CTAB_11.as_flattened(), ROM_CTAB11, t0 * 17 + t1) as i32
}

/// `HI_LTAB(a)`
#[inline(always)]
fn hi_ltab(a: i32) -> i32 {
    a >> 16
}

/// `LO_LTAB(a)`
#[inline(always)]
fn lo_ltab(a: i32) -> i32 {
    a & 0xffff
}

/// `fAbs()` of a `SHORT` value assigned to an `INT` (`t0 = values[i]; t0 = fAbs(t0)`): the
/// `FIXP_DBL` overload, exact.
#[inline(always)]
fn abs_i(x: i16) -> i32 {
    (x as i32).abs()
}

/// `fAbs(values[i])` of a `SHORT` value: the `FIXP_SGL` overload, whose result is a `SHORT`
/// again, so -32768 stays -32768.
#[inline(always)]
fn abs_s(x: i16) -> i32 {
    x.wrapping_abs() as i32
}

/// The four values of a quadruple, as `INT`.
#[inline(always)]
fn quad(values: &[i16], i: usize) -> (i32, i32, i32, i32) {
    (values[i] as i32, values[i + 1] as i32, values[i + 2] as i32, values[i + 3] as i32)
}

/// The absolute values (`abs_i`) of a quadruple and the number of non-zero values among them.
#[inline(always)]
fn quad_abs(values: &[i16], i: usize) -> ((i32, i32, i32, i32), i32) {
    let t = (abs_i(values[i]), abs_i(values[i + 1]), abs_i(values[i + 2]), abs_i(values[i + 3]));
    let sc = (t.0 > 0) as i32 + (t.1 > 0) as i32 + (t.2 > 0) as i32 + (t.3 > 0) as i32;
    (t, sc)
}

/// `(INT)FDKaacEnc_huff_ltab1_2[t0 + 1][t1 + 1][t2 + 1][t3 + 1]`
#[inline(always)]
fn l12(t0: i32, t1: i32, t2: i32, t3: i32) -> i32 {
    ltab1_2(t0 + 1, t1 + 1, t2 + 1, t3 + 1) as i32
}
/// `(INT)FDKaacEnc_huff_ltab3_4[t0][t1][t2][t3]`
#[inline(always)]
fn l34(t0: i32, t1: i32, t2: i32, t3: i32) -> i32 {
    ltab3_4(t0, t1, t2, t3) as i32
}
/// `(INT)FDKaacEnc_huff_ltab5_6[t0 + 4][t1 + 4]`
#[inline(always)]
fn l56(t0: i32, t1: i32) -> i32 {
    ltab5_6(t0 + 4, t1 + 4) as i32
}
/// `(INT)FDKaacEnc_huff_ltab7_8[t0][t1]`
#[inline(always)]
fn l78(t0: i32, t1: i32) -> i32 {
    ltab7_8(t0, t1) as i32
}
/// `(INT)FDKaacEnc_huff_ltab9_10[t0][t1]`
#[inline(always)]
fn l910(t0: i32, t1: i32) -> i32 {
    ltab9_10(t0, t1) as i32
}

// The sums of the counting functions wrap (`wrapping_add`), as signed overflow does on both
// reference builds.

/// The contributions of one quadruple of values in `-1..=1` to the sums of
/// `FDKaacEnc_count1_2_3_4_5_6_7_8_9_10_11()`: `[bc1_2, bc5_6, bc3_4, bc7_8, bc9_10, bc11, sc, 0]`
/// Index `((t0 + 1) * 3 + t1 + 1) * 3 + ...`.
static COUNT1_QUAD: [[i32; 8]; 81] = {
    let mut t = [[0i32; 8]; 81];
    let mut k = 0;
    while k < 81 {
        let q = [
            (k / 27) as i32 - 1,
            (k / 9 % 3) as i32 - 1,
            (k / 3 % 3) as i32 - 1,
            (k % 3) as i32 - 1,
        ];
        let a = [
            q[0].unsigned_abs() as usize,
            q[1].unsigned_abs() as usize,
            q[2].unsigned_abs() as usize,
            q[3].unsigned_abs() as usize,
        ];
        let u =
            [(q[0] + 1) as usize, (q[1] + 1) as usize, (q[2] + 1) as usize, (q[3] + 1) as usize];
        let v =
            [(q[0] + 4) as usize, (q[1] + 4) as usize, (q[2] + 4) as usize, (q[3] + 4) as usize];
        let l12 = FDK_AAC_ENC_HUFF_LTAB_1_2[u[0]][u[1]][u[2]][u[3]];
        let l34 = FDK_AAC_ENC_HUFF_LTAB_3_4[a[0]][a[1]][a[2]][a[3]];
        let l56 = (FDK_AAC_ENC_HUFF_LTAB_5_6[v[0]][v[1]] as i32)
            .wrapping_add(FDK_AAC_ENC_HUFF_LTAB_5_6[v[2]][v[3]] as i32);
        let l78 = (FDK_AAC_ENC_HUFF_LTAB_7_8[a[0]][a[1]] as i32)
            .wrapping_add(FDK_AAC_ENC_HUFF_LTAB_7_8[a[2]][a[3]] as i32);
        let l910 = (FDK_AAC_ENC_HUFF_LTAB_9_10[a[0]][a[1]] as i32)
            .wrapping_add(FDK_AAC_ENC_HUFF_LTAB_9_10[a[2]][a[3]] as i32);
        let l11 = FDK_AAC_ENC_HUFF_LTAB_11[a[0]][a[1]] as i32
            + FDK_AAC_ENC_HUFF_LTAB_11[a[2]][a[3]] as i32;
        t[k] = [
            l12 as i32,
            l56,
            l34 as i32,
            l78,
            l910,
            l11,
            (a[0] > 0) as i32 + (a[1] > 0) as i32 + (a[2] > 0) as i32 + (a[3] > 0) as i32,
            0,
        ];
        k += 1;
    }
    t
};

/// `FDKaacEnc_count1_2_3_4_5_6_7_8_9_10_11`: counts tables 1-11.
///
/// input: quantized spectrum; output: bitCount for tables 1-11
///
/// The quadruples with values in `-1..=1` (all of them when the caller's `maxVal` is right) add
/// their row of [`COUNT1_QUAD`]: the same terms as the straight loop
/// ([`fdk_aac_enc_count1_2_3_4_5_6_7_8_9_10_11_straight`]), added with wrap-around in another
/// grouping, which gives the same sums. Other quadruples take the straight terms.
fn fdk_aac_enc_count1_2_3_4_5_6_7_8_9_10_11(values: &[i16], width: i32, bit_count: &mut [i32]) {
    let mut acc = [0i32; 8];
    let mut i = 0;
    while i < width {
        let (t0, t1, t2, t3) = quad(values, i as usize);
        let in_range = |t: i32| (t + 1) as u32 <= 2;
        if in_range(t0) & in_range(t1) & in_range(t2) & in_range(t3) {
            let row = &COUNT1_QUAD
                [(((t0 + 1) * 3 + t1 + 1) * 3 + t2 + 1) as usize * 3 + (t3 + 1) as usize];
            for (a, &r) in acc.iter_mut().zip(row) {
                *a = a.wrapping_add(r);
            }
        } else {
            acc[0] = acc[0].wrapping_add(l12(t0, t1, t2, t3));
            acc[1] = acc[1].wrapping_add(l56(t0, t1).wrapping_add(l56(t2, t3)));
            let ((t0, t1, t2, t3), n) = quad_abs(values, i as usize);
            acc[6] += n;
            acc[2] = acc[2].wrapping_add(l34(t0, t1, t2, t3));
            acc[3] = acc[3].wrapping_add(l78(t0, t1).wrapping_add(l78(t2, t3)));
            acc[4] = acc[4].wrapping_add(l910(t0, t1).wrapping_add(l910(t2, t3)));
            acc[5] = acc[5].wrapping_add(ltab11(t0, t1) + ltab11(t2, t3));
        }
        i += 4;
    }
    let [bc1_2, bc5_6, bc3_4, bc7_8, bc9_10, bc11, sc, _] = acc;
    bit_count[1] = hi_ltab(bc1_2);
    bit_count[2] = lo_ltab(bc1_2);
    bit_count[3] = hi_ltab(bc3_4) + sc;
    bit_count[4] = lo_ltab(bc3_4) + sc;
    bit_count[5] = hi_ltab(bc5_6);
    bit_count[6] = lo_ltab(bc5_6);
    bit_count[7] = hi_ltab(bc7_8) + sc;
    bit_count[8] = lo_ltab(bc7_8) + sc;
    bit_count[9] = hi_ltab(bc9_10) + sc;
    bit_count[10] = lo_ltab(bc9_10) + sc;
    bit_count[11] = bc11.wrapping_add(sc);
}

/// `FDKaacEnc_count1_2_3_4_5_6_7_8_9_10_11`: straight port, kept as the reference of
/// [`fdk_aac_enc_count1_2_3_4_5_6_7_8_9_10_11`].
#[doc(hidden)]
pub fn fdk_aac_enc_count1_2_3_4_5_6_7_8_9_10_11_straight(
    values: &[i16],
    width: i32,
    bit_count: &mut [i32],
) {
    let (mut bc1_2, mut bc3_4, mut bc5_6, mut bc7_8, mut bc9_10, mut bc11, mut sc) =
        (0i32, 0i32, 0i32, 0i32, 0i32, 0i32, 0i32);

    let mut i = 0;
    while i < width {
        let (t0, t1, t2, t3) = quad(values, i as usize);
        bc1_2 = bc1_2.wrapping_add(l12(t0, t1, t2, t3));
        bc5_6 = bc5_6.wrapping_add(l56(t0, t1).wrapping_add(l56(t2, t3)));

        let ((t0, t1, t2, t3), n) = quad_abs(values, i as usize);
        sc += n;

        bc3_4 = bc3_4.wrapping_add(l34(t0, t1, t2, t3));
        bc7_8 = bc7_8.wrapping_add(l78(t0, t1).wrapping_add(l78(t2, t3)));
        bc9_10 = bc9_10.wrapping_add(l910(t0, t1).wrapping_add(l910(t2, t3)));
        bc11 = bc11.wrapping_add(ltab11(t0, t1) + ltab11(t2, t3));
        i += 4;
    }
    bit_count[1] = hi_ltab(bc1_2);
    bit_count[2] = lo_ltab(bc1_2);
    bit_count[3] = hi_ltab(bc3_4) + sc;
    bit_count[4] = lo_ltab(bc3_4) + sc;
    bit_count[5] = hi_ltab(bc5_6);
    bit_count[6] = lo_ltab(bc5_6);
    bit_count[7] = hi_ltab(bc7_8) + sc;
    bit_count[8] = lo_ltab(bc7_8) + sc;
    bit_count[9] = hi_ltab(bc9_10) + sc;
    bit_count[10] = lo_ltab(bc9_10) + sc;
    bit_count[11] = bc11.wrapping_add(sc);
}

/// `FDKaacEnc_count3_4_5_6_7_8_9_10_11`: counts tables 3-11.
fn fdk_aac_enc_count3_4_5_6_7_8_9_10_11(values: &[i16], width: i32, bit_count: &mut [i32]) {
    let (mut bc3_4, mut bc5_6, mut bc7_8, mut bc9_10, mut bc11, mut sc) =
        (0i32, 0i32, 0i32, 0i32, 0i32, 0i32);

    let mut i = 0;
    while i < width {
        let (t0, t1, t2, t3) = quad(values, i as usize);
        bc5_6 = bc5_6.wrapping_add(l56(t0, t1).wrapping_add(l56(t2, t3)));

        let ((t0, t1, t2, t3), n) = quad_abs(values, i as usize);
        sc += n;

        bc3_4 = bc3_4.wrapping_add(l34(t0, t1, t2, t3));
        bc7_8 = bc7_8.wrapping_add(l78(t0, t1).wrapping_add(l78(t2, t3)));
        bc9_10 = bc9_10.wrapping_add(l910(t0, t1).wrapping_add(l910(t2, t3)));
        bc11 = bc11.wrapping_add(ltab11(t0, t1) + ltab11(t2, t3));
        i += 4;
    }

    bit_count[1] = INVALID_BITCOUNT;
    bit_count[2] = INVALID_BITCOUNT;
    bit_count[3] = hi_ltab(bc3_4) + sc;
    bit_count[4] = lo_ltab(bc3_4) + sc;
    bit_count[5] = hi_ltab(bc5_6);
    bit_count[6] = lo_ltab(bc5_6);
    bit_count[7] = hi_ltab(bc7_8) + sc;
    bit_count[8] = lo_ltab(bc7_8) + sc;
    bit_count[9] = hi_ltab(bc9_10) + sc;
    bit_count[10] = lo_ltab(bc9_10) + sc;
    bit_count[11] = bc11.wrapping_add(sc);
}

/// `FDKaacEnc_count5_6_7_8_9_10_11`: counts tables 5-11.
fn fdk_aac_enc_count5_6_7_8_9_10_11(values: &[i16], width: i32, bit_count: &mut [i32]) {
    let (mut bc5_6, mut bc7_8, mut bc9_10, mut bc11, mut sc) = (0i32, 0i32, 0i32, 0i32, 0i32);

    let mut i = 0;
    while i < width {
        let (t0, t1, t2, t3) = quad(values, i as usize);
        bc5_6 = bc5_6.wrapping_add(l56(t0, t1).wrapping_add(l56(t2, t3)));

        let ((t0, t1, t2, t3), n) = quad_abs(values, i as usize);
        sc += n;

        bc7_8 = bc7_8.wrapping_add(l78(t0, t1).wrapping_add(l78(t2, t3)));
        bc9_10 = bc9_10.wrapping_add(l910(t0, t1).wrapping_add(l910(t2, t3)));
        bc11 = bc11.wrapping_add(ltab11(t0, t1) + ltab11(t2, t3));
        i += 4;
    }
    bit_count[1] = INVALID_BITCOUNT;
    bit_count[2] = INVALID_BITCOUNT;
    bit_count[3] = INVALID_BITCOUNT;
    bit_count[4] = INVALID_BITCOUNT;
    bit_count[5] = hi_ltab(bc5_6);
    bit_count[6] = lo_ltab(bc5_6);
    bit_count[7] = hi_ltab(bc7_8) + sc;
    bit_count[8] = lo_ltab(bc7_8) + sc;
    bit_count[9] = hi_ltab(bc9_10) + sc;
    bit_count[10] = lo_ltab(bc9_10) + sc;
    bit_count[11] = bc11.wrapping_add(sc);
}

/// `FDKaacEnc_count7_8_9_10_11`: counts tables 7-11.
fn fdk_aac_enc_count7_8_9_10_11(values: &[i16], width: i32, bit_count: &mut [i32]) {
    let (mut bc7_8, mut bc9_10, mut bc11, mut sc) = (0i32, 0i32, 0i32, 0i32);

    let mut i = 0;
    while i < width {
        let ((t0, t1, t2, t3), n) = quad_abs(values, i as usize);
        sc += n;

        bc7_8 = bc7_8.wrapping_add(l78(t0, t1).wrapping_add(l78(t2, t3)));
        bc9_10 = bc9_10.wrapping_add(l910(t0, t1).wrapping_add(l910(t2, t3)));
        bc11 = bc11.wrapping_add(ltab11(t0, t1) + ltab11(t2, t3));
        i += 4;
    }

    bit_count[1] = INVALID_BITCOUNT;
    bit_count[2] = INVALID_BITCOUNT;
    bit_count[3] = INVALID_BITCOUNT;
    bit_count[4] = INVALID_BITCOUNT;
    bit_count[5] = INVALID_BITCOUNT;
    bit_count[6] = INVALID_BITCOUNT;
    bit_count[7] = hi_ltab(bc7_8) + sc;
    bit_count[8] = lo_ltab(bc7_8) + sc;
    bit_count[9] = hi_ltab(bc9_10) + sc;
    bit_count[10] = lo_ltab(bc9_10) + sc;
    bit_count[11] = bc11.wrapping_add(sc);
}

/// `FDKaacEnc_count9_10_11`: counts tables 9-11.
fn fdk_aac_enc_count9_10_11(values: &[i16], width: i32, bit_count: &mut [i32]) {
    let (mut bc9_10, mut bc11, mut sc) = (0i32, 0i32, 0i32);

    let mut i = 0;
    while i < width {
        let ((t0, t1, t2, t3), n) = quad_abs(values, i as usize);
        sc += n;

        bc9_10 = bc9_10.wrapping_add(l910(t0, t1).wrapping_add(l910(t2, t3)));
        bc11 = bc11.wrapping_add(ltab11(t0, t1) + ltab11(t2, t3));
        i += 4;
    }

    bit_count[1] = INVALID_BITCOUNT;
    bit_count[2] = INVALID_BITCOUNT;
    bit_count[3] = INVALID_BITCOUNT;
    bit_count[4] = INVALID_BITCOUNT;
    bit_count[5] = INVALID_BITCOUNT;
    bit_count[6] = INVALID_BITCOUNT;
    bit_count[7] = INVALID_BITCOUNT;
    bit_count[8] = INVALID_BITCOUNT;
    bit_count[9] = hi_ltab(bc9_10) + sc;
    bit_count[10] = lo_ltab(bc9_10) + sc;
    bit_count[11] = bc11.wrapping_add(sc);
}

/// `FDKaacEnc_count11`: counts table 11.
fn fdk_aac_enc_count11(values: &[i16], width: i32, bit_count: &mut [i32]) {
    let (mut bc11, mut sc) = (0i32, 0i32);

    let mut i = 0;
    while i < width {
        let ((t0, t1, t2, t3), n) = quad_abs(values, i as usize);
        sc += n;

        bc11 = bc11.wrapping_add(ltab11(t0, t1) + ltab11(t2, t3));
        i += 4;
    }

    for b in bit_count[1..=10].iter_mut() {
        *b = INVALID_BITCOUNT;
    }
    bit_count[11] = bc11.wrapping_add(sc);
}

/// `FDKaacEnc_countEsc`: counts table 11 (with Esc).
fn fdk_aac_enc_count_esc(values: &[i16], width: i32, bit_count: &mut [i32]) {
    let (mut bc11, mut ec, mut sc) = (0i32, 0i32, 0i32);

    let mut i = 0;
    while i < width {
        // fAbs(FIXP_SGL): -32768 stays negative and is not counted as a sign
        let mut t0 = abs_s(values[i as usize]);
        let mut t1 = abs_s(values[i as usize + 1]);

        sc += (t0 > 0) as i32 + (t1 > 0) as i32;

        let t00 = fix_min(t0, 16);
        let t01 = fix_min(t1, 16);
        bc11 = bc11.wrapping_add(ltab11(t00, t01));

        if t0 >= 16 {
            ec += 5;
            loop {
                t0 >>= 1;
                if t0 < 16 {
                    break;
                }
                ec += 2;
            }
        }

        if t1 >= 16 {
            ec += 5;
            loop {
                t1 >>= 1;
                if t1 < 16 {
                    break;
                }
                ec += 2;
            }
        }
        i += 2;
    }

    for b in bit_count[..11].iter_mut() {
        *b = INVALID_BITCOUNT;
    }

    bit_count[11] = bc11.wrapping_add(sc).wrapping_add(ec);
}

/// `COUNT_FUNCTION`
type CountFunction = fn(&[i16], i32, &mut [i32]);

/// `countFuncTable`
static COUNT_FUNC_TABLE: [CountFunction; CODE_BOOK_ESC_LAV as usize + 1] = [
    fdk_aac_enc_count1_2_3_4_5_6_7_8_9_10_11, // 0
    fdk_aac_enc_count1_2_3_4_5_6_7_8_9_10_11, // 1
    fdk_aac_enc_count3_4_5_6_7_8_9_10_11,     // 2
    fdk_aac_enc_count5_6_7_8_9_10_11,         // 3
    fdk_aac_enc_count5_6_7_8_9_10_11,         // 4
    fdk_aac_enc_count7_8_9_10_11,             // 5
    fdk_aac_enc_count7_8_9_10_11,             // 6
    fdk_aac_enc_count7_8_9_10_11,             // 7
    fdk_aac_enc_count9_10_11,                 // 8
    fdk_aac_enc_count9_10_11,                 // 9
    fdk_aac_enc_count9_10_11,                 // 10
    fdk_aac_enc_count9_10_11,                 // 11
    fdk_aac_enc_count9_10_11,                 // 12
    fdk_aac_enc_count11,                      // 13
    fdk_aac_enc_count11,                      // 14
    fdk_aac_enc_count11,                      // 15
    fdk_aac_enc_count_esc,                    // 16
];

/// `FDKaacEnc_bitCount`: the number of bits of `values` (`width` lines) for each code book
/// (`bit_count[0..=11]`, `INVALID_BITCOUNT` for books that cannot code `max_val`).
///
/// `max_val` should be the maximum absolute value of the lines (table reads: see
/// `huff_rom_read`). Where the reference has no defined result, the port sets the counts of
/// books 1 to 11 to `INVALID_BITCOUNT` (deviation).
pub fn fdk_aac_enc_bit_count(
    values: &[i16],
    width: i32,
    max_val: i32,
    bit_count: &mut [i32],
) -> i32 {
    // check if we can use codebook 0
    bit_count[0] = if max_val == 0 { 0 } else { INVALID_BITCOUNT };

    match COUNT_FUNC_TABLE.get(fix_min(max_val, CODE_BOOK_ESC_LAV) as usize) {
        Some(count) => count(values, width, bit_count),
        None => bit_count[1..=11].fill(INVALID_BITCOUNT),
    }

    0
}

/// `FDKaacEnc_countValues`: count difference between actual and zeroed lines.
///
/// The `ULONG` table entries are split with unsigned shifts (`ltab >> 16`, `ltab & 0xffff`) and
/// added to the `INT` count (wrapping). The absolute values are `fAbs(SHORT)` (see `abs_s`).
pub fn fdk_aac_enc_count_values(values: &[i16], width: i32, code_book: i32) -> i32 {
    let mut bit_cnt: i32 = 0;
    let n = width.max(0) as usize;
    let add = |bit_cnt: &mut i32, x: u32| *bit_cnt = bit_cnt.wrapping_add(x as i32);
    let hi = code_book % 2 == 1; // books 1, 3, 5, 7, 9: the high half

    match code_book {
        CODE_BOOK_ZERO_NO => {}

        CODE_BOOK_1_NO | CODE_BOOK_2_NO => {
            for i in (0..n).step_by(4) {
                let (t0, t1, t2, t3) = quad(values, i);
                let l = ltab1_2(t0 + 1, t1 + 1, t2 + 1, t3 + 1);
                add(&mut bit_cnt, if hi { l >> 16 } else { l & 0xffff });
            }
        }

        CODE_BOOK_3_NO | CODE_BOOK_4_NO => {
            for i in (0..n).step_by(4) {
                let t = [values[i], values[i + 1], values[i + 2], values[i + 3]].map(abs_s);
                for &x in &t {
                    bit_cnt = bit_cnt.wrapping_add((x > 0) as i32);
                }
                let l = ltab3_4(t[0], t[1], t[2], t[3]);
                add(&mut bit_cnt, if hi { l >> 16 } else { l & 0xffff });
            }
        }

        CODE_BOOK_5_NO | CODE_BOOK_6_NO => {
            for i in (0..n).step_by(4) {
                let (t0, t1, t2, t3) = quad(values, i);
                let (a, b) = (ltab5_6(t0 + 4, t1 + 4), ltab5_6(t2 + 4, t3 + 4));
                let s = if hi { (a >> 16) + (b >> 16) } else { (a & 0xffff) + (b & 0xffff) };
                add(&mut bit_cnt, s);
            }
        }

        CODE_BOOK_7_NO | CODE_BOOK_8_NO | CODE_BOOK_9_NO | CODE_BOOK_10_NO => {
            let seven = code_book <= CODE_BOOK_8_NO;
            for i in (0..n).step_by(4) {
                let t = [values[i], values[i + 1], values[i + 2], values[i + 3]].map(abs_s);
                for &x in &t {
                    bit_cnt = bit_cnt.wrapping_add((x > 0) as i32);
                }
                let (a, b) = if seven {
                    (ltab7_8(t[0], t[1]), ltab7_8(t[2], t[3]))
                } else {
                    (ltab9_10(t[0], t[1]), ltab9_10(t[2], t[3]))
                };
                let s = if hi { (a >> 16) + (b >> 16) } else { (a & 0xffff) + (b & 0xffff) };
                add(&mut bit_cnt, s);
            }
        }

        CODE_BOOK_ESC_NO => {
            for i in (0..n).step_by(2) {
                let mut t0 = abs_s(values[i]);
                bit_cnt = bit_cnt.wrapping_add((t0 > 0) as i32);
                let mut t1 = abs_s(values[i + 1]);
                bit_cnt = bit_cnt.wrapping_add((t1 > 0) as i32);
                bit_cnt = bit_cnt.wrapping_add(ltab11(fix_min(t0, 16), fix_min(t1, 16)));
                if t0 >= 16 {
                    bit_cnt = bit_cnt.wrapping_add(5);
                    loop {
                        t0 >>= 1;
                        if t0 < 16 {
                            break;
                        }
                        bit_cnt = bit_cnt.wrapping_add(2);
                    }
                }
                if t1 >= 16 {
                    bit_cnt = bit_cnt.wrapping_add(5);
                    loop {
                        t1 >>= 1;
                        if t1 < 16 {
                            break;
                        }
                        bit_cnt = bit_cnt.wrapping_add(2);
                    }
                }
            }
        }

        _ => {}
    }

    bit_cnt
}

/// The sign bits of a pair or quadruple as `FDKaacEnc_codeValues()` builds them:
/// `sign = (sign << zero) + ((UINT)ti >> 31)` for every value; `signLength` counts the non-zero
/// values.
#[inline(always)]
fn signs(v: &[i16]) -> (i32, i32) {
    let (mut sign, mut sign_length) = (0i32, 0i32);
    for &ti in v {
        let zero = (ti != 0) as i32;
        sign_length += zero;
        sign = (sign << zero) + ((ti as i32 as u32) >> 31) as i32;
    }
    (sign, sign_length)
}

/// `FDKaacEnc_codeValues`: writes the Huffman code words of `values` (`width` lines) with
/// `code_book` to the bitstream.
///
/// Table reads: see `huff_rom_read`. Code lengths of 32 bits or more are handled as in
/// `FDKwriteBits()` (shift amounts modulo 32, as both reference builds compute them); the shift
/// of the first code word of books 5 and 6 by such a length wraps the same way.
pub fn fdk_aac_enc_code_values(
    values: &[i16],
    width: i32,
    code_book: i32,
    h_bitstream: &mut FdkBitStream,
) -> i32 {
    let w = width.max(0) as usize;
    let hi = code_book % 2 == 1; // books 1, 3, 5, 7, 9: the high half of the length entry
    let len = |l: u32| if hi { (l >> 16) as i32 } else { (l & 0xffff) as i32 };
    match code_book {
        CODE_BOOK_ZERO_NO => {}

        CODE_BOOK_1_NO | CODE_BOOK_2_NO => {
            let id = if hi { ROM_CTAB1 } else { ROM_CTAB2 };
            for i in (0..w).step_by(4) {
                let t0 = values[i] as i32 + 1;
                let t1 = values[i + 1] as i32 + 1;
                let t2 = values[i + 2] as i32 + 1;
                let t3 = values[i + 3] as i32 + 1;
                let code_word = ctab1_4(id, t0, t1, t2, t3);
                let code_length = len(ltab1_2(t0, t1, t2, t3));
                fdk_write_bits(h_bitstream, code_word as u32, code_length as u32);
            }
        }

        CODE_BOOK_3_NO | CODE_BOOK_4_NO => {
            // C: CODE_BOOK_3_NO loops i < (width >> 2), CODE_BOOK_4_NO i < width; i += 4
            let n = if code_book == CODE_BOOK_3_NO { (w >> 2) * 4 } else { w };
            let id = if hi { ROM_CTAB3 } else { ROM_CTAB4 };
            for i in (0..n).step_by(4) {
                let q = &values[i..i + 4];
                let (sign, sign_length) = signs(q);
                // `index[j] = fAbs(ti)` of an `int`: exact
                let ix = [abs_i(q[0]), abs_i(q[1]), abs_i(q[2]), abs_i(q[3])];
                let code_word = ctab1_4(id, ix[0], ix[1], ix[2], ix[3]);
                let code_length = len(ltab3_4(ix[0], ix[1], ix[2], ix[3]));
                fdk_write_bits(
                    h_bitstream,
                    ((code_word << sign_length) | sign) as u32,
                    code_length.wrapping_add(sign_length) as u32,
                );
            }
        }

        CODE_BOOK_5_NO | CODE_BOOK_6_NO => {
            let id = if hi { ROM_CTAB5 } else { ROM_CTAB6 };
            for i in 0..w >> 2 {
                let t0 = values[4 * i] as i32 + 4;
                let t1 = values[4 * i + 1] as i32 + 4;
                let t2 = values[4 * i + 2] as i32 + 4;
                let t3 = values[4 * i + 3] as i32 + 4;
                let mut code_word = ctab5_10(id, t0, t1);
                let mut code_length = len(ltab5_6(t2, t3)); // length of 2nd cw
                code_word =
                    code_word.wrapping_shl(code_length as u32).wrapping_add(ctab5_10(id, t2, t3));
                code_length = code_length.wrapping_add(len(ltab5_6(t0, t1)));
                fdk_write_bits(h_bitstream, code_word as u32, code_length as u32);
            }
        }

        CODE_BOOK_7_NO | CODE_BOOK_8_NO | CODE_BOOK_9_NO | CODE_BOOK_10_NO | CODE_BOOK_ESC_NO => {
            for i in 0..w >> 1 {
                let q = &values[2 * i..2 * i + 2];
                let (sign, sign_length) = signs(q);
                // `t0 = fAbs(t0)` of an `INT`: exact
                let mut t0 = abs_i(q[0]);
                let t1 = abs_i(q[1]);
                let (code_word, code_length) = match code_book {
                    CODE_BOOK_7_NO => (ctab5_10(ROM_CTAB7, t0, t1), len(ltab7_8(t0, t1))),
                    CODE_BOOK_8_NO => (ctab5_10(ROM_CTAB8, t0, t1), len(ltab7_8(t0, t1))),
                    CODE_BOOK_9_NO => (ctab5_10(ROM_CTAB9, t0, t1), len(ltab9_10(t0, t1))),
                    CODE_BOOK_10_NO => (ctab5_10(ROM_CTAB10, t0, t1), len(ltab9_10(t0, t1))),
                    _ => {
                        let t00 = fix_min(t0, 16);
                        let t01 = fix_min(t1, 16);
                        (ctab11(t00, t01), ltab11(t00, t01))
                    }
                };
                fdk_write_bits(
                    h_bitstream,
                    ((code_word << sign_length) | sign) as u32,
                    code_length.wrapping_add(sign_length) as u32,
                );
                if code_book == CODE_BOOK_ESC_NO {
                    for _ in 0..2 {
                        if t0 >= 16 {
                            let mut n = 4;
                            let mut p = t0;
                            loop {
                                p >>= 1;
                                if p < 16 {
                                    break;
                                }
                                n += 1;
                            }
                            fdk_write_bits(
                                h_bitstream,
                                ((((1 << (n - 3)) - 2) << n) | (t0 - (1 << n))) as u32,
                                (n + n - 3) as u32,
                            );
                        }
                        t0 = t1;
                    }
                }
            }
        }

        _ => {}
    }
    0
}

/// `FDKaacEnc_codeScalefactorDelta`: writes the Huffman code word of a scalefactor delta.
/// Returns 1 (and writes nothing) if `|delta| > CODE_BOOK_SCF_LAV`.
///
/// Where the reference has no defined result, the port returns 1 as well.
pub fn fdk_aac_enc_code_scalefactor_delta(delta: i32, h_bitstream: &mut FdkBitStream) -> i32 {
    if delta.unsigned_abs() > CODE_BOOK_SCF_LAV as u32 {
        return 1;
    }

    let code_word = FDK_AAC_ENC_HUFF_CTABSCF[(delta + CODE_BOOK_SCF_LAV) as usize];
    let code_length = FDK_AAC_ENC_HUFF_LTABSCF[(delta + CODE_BOOK_SCF_LAV) as usize] as i32;
    fdk_write_bits(h_bitstream, code_word, code_length as u32);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table version of `count1_2_3_4_5_6_7_8_9_10_11` against the straight one, also with
    /// values outside `-1..=1`.
    #[test]
    fn count1_equals_straight_port() {
        let mut seed = 0x0c01_4700_9e37_79b9u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        for k in 0..200_000 {
            let width = 4 * (next() % 33) as i32;
            let wild = k % 5 == 0;
            let v: Vec<i16> = (0..width)
                .map(|_| match next() % 16 {
                    0 if wild => [i16::MIN, i16::MAX, 2, -2, 5, -9][(next() % 6) as usize],
                    1..=6 => 0,
                    _ => (next() % 3) as i16 - 1,
                })
                .collect();
            let (mut a, mut b) = ([0x55i32; 12], [0x55i32; 12]);
            fdk_aac_enc_count1_2_3_4_5_6_7_8_9_10_11(&v, width, &mut a);
            fdk_aac_enc_count1_2_3_4_5_6_7_8_9_10_11_straight(&v, width, &mut b);
            assert_eq!(a, b, "count1..11 width {width} {v:?}");
        }
    }
}
