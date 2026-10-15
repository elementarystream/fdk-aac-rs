// Differential tests of fdk_aac_rs_sacenc::sacenc_nlc_enc against the C++ reference.
//
// fdk_sacenc_ecDataPairEnc/SingleEnc on random CLD/ICC data (fine and coarse, smooth and
// random, every start band, band count, set index and independency flag), written after a
// random number of bits into a bitstream; the bitstream state and the buffer are compared.
// The file-static helpers (PCM coding, 1D/2D Huffman coding, bit counting) are compared on
// random data in the ranges the encoder produces.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::sacenc_sacenc_nlc_enc as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::{fdk_init_bit_stream, fdk_write_bits, FdkBitStream, BS_WRITER};
use fdk_aac_rs_sacenc::sacenc_huff_tab::HuffEntry;
use fdk_aac_rs_sacenc::sacenc_nlc_enc as r;
use fdk_aac_rs_sacenc::sacenc_nlc_enc::{t_CLD, t_ICC, MAXBANDS};

/// A bitstream writer on both sides over equal buffers, `pre` bits written first.
pub struct Bs {
    pub cbuf: Vec<u8>,
    pub rbuf: BitMem,
    pub size: u32,
    pub pre: i32,
    pub preval: u32,
    pub state: [u32; 6],
}

impl Bs {
    pub fn new(rng: &mut Rng, size: u32) -> Self {
        let cbuf: Vec<u8> = (0..size).map(|_| rng.next_u32() as u8).collect();
        Bs {
            rbuf: BitMem::from_slice(&cbuf),
            cbuf,
            size,
            pre: rng.range(0, 70),
            preval: rng.next_u32(),
            state: [0; 6],
        }
    }

    /// The Rust bitstream, initialized as `sacenc_bs_begin()` does.
    pub fn rust(&mut self) -> FdkBitStream {
        let mut bs = FdkBitStream::default();
        fdk_init_bit_stream(&mut bs, &self.rbuf, self.size, 0, BS_WRITER);
        let mut pre = self.pre;
        while pre > 0 {
            let n = pre.min(25);
            fdk_write_bits(&mut bs, self.preval, n as u32);
            pre -= n;
        }
        bs
    }

    pub fn check(&self, bs: &FdkBitStream, what: &str) {
        let s = [
            bs.cache_word,
            bs.bits_in_cache,
            bs.h_bit_buf.valid_bits,
            bs.h_bit_buf.read_offset,
            bs.h_bit_buf.write_offset,
            bs.h_bit_buf.bit_ndx,
        ];
        assert_eq!(self.state, s, "{what}: bitstream state");
        assert!(self.cbuf == self.rbuf.to_vec(), "{what}: buffer");
    }
}

/// Quantized data of a data type in the msb domain (with offset): CLD 0..=30 (coarse 0..=14),
/// ICC 0..=7 (coarse 0..=3). Smooth or random.
fn qdata(rng: &mut Rng, levels: i32, n: usize) -> Vec<i16> {
    let smooth = rng.range(0, 2);
    let mut v = Vec::with_capacity(n);
    let mut x = rng.range(0, levels - 1);
    for _ in 0..n {
        match smooth {
            0 => x = rng.range(0, levels - 1),
            1 => x = (x + rng.range(-1, 1)).clamp(0, levels - 1),
            _ => x = (x + rng.range(-3, 3)).clamp(0, levels - 1),
        }
        v.push(x as i16);
    }
    v
}

fn levels(dt: i32, coarse: i32) -> (i32, i32) {
    match (dt, coarse) {
        (0, 0) => (31, 15),
        (0, _) => (15, 7),
        (_, 0) => (8, 0),
        _ => (4, 0),
    }
}

#[test]
fn ec_data_enc() {
    let mut rng = Rng::new(1);
    for it in 0..20000 {
        let dt = rng.range(0, 1);
        let coarse = rng.range(0, 1);
        let (lv, off) = levels(dt, coarse);
        let pair = rng.range(0, 1);
        let set_idx = rng.range(0, 2);
        let rows = (set_idx + 1 + pair) as usize;
        let start = if rng.range(0, 3) == 0 { rng.range(0, 22) } else { 0 };
        let bands = rng.range(1, MAXBANDS as i32 - start);
        let indep = rng.range(0, 1);
        let mut data = vec![[0i16; MAXBANDS]; 4];
        let base = qdata(&mut rng, lv, MAXBANDS);
        for row in data.iter_mut().take(rows) {
            let d = if rng.range(0, 1) == 0 { base.clone() } else { qdata(&mut rng, lv, MAXBANDS) };
            for (k, v) in d.iter().enumerate() {
                row[k] = v - off as i16;
            }
        }
        let mut hist = [0i16; MAXBANDS];
        let h = qdata(&mut rng, lv, MAXBANDS);
        for k in 0..MAXBANDS {
            hist[k] = h[k] - off as i16;
        }
        let mut b = Bs::new(&mut rng, 512);
        let flat: Vec<i16> = data.iter().flatten().copied().collect();
        let rc = unsafe {
            c::cref_sacenc_nlc_enc_ecData(
                b.cbuf.as_mut_ptr(),
                b.size,
                b.pre,
                b.preval,
                flat.as_ptr(),
                rows as i32,
                hist.as_ptr(),
                dt,
                set_idx,
                start,
                bands,
                coarse,
                indep,
                pair,
                b.state.as_mut_ptr(),
            )
        };
        let mut bs = b.rust();
        let rr = if pair != 0 {
            r::fdk_sacenc_ec_data_pair_enc(&mut bs, &data, &hist, dt, set_idx, start, bands, coarse, indep)
        } else {
            r::fdk_sacenc_ec_data_single_enc(&mut bs, &data, &hist, dt, set_idx, start, bands, coarse, indep)
        };
        assert_eq!(rc, rr);
        b.check(&bs, &format!("ecData it {it}"));
    }
}

#[test]
fn small_helpers() {
    let mut rng = Rng::new(2);
    for _ in 0..5000 {
        let n = rng.range(1, MAXBANDS as i32) as usize;
        let inp: Vec<i16> = (0..MAXBANDS).map(|_| rng.range(-40, 40) as i16).collect();
        let prev: Vec<i16> = (0..MAXBANDS).map(|_| rng.range(-40, 40) as i16).collect();
        let off = rng.range(-20, 20) as i16;

        let (mut lc, mut mc) = ([0i16; MAXBANDS], [0i16; MAXBANDS]);
        let (mut lr, mut mr) = ([0i16; MAXBANDS], [0i16; MAXBANDS]);
        unsafe { c::cref_sacenc_nlc_enc_split_lsb(inp.as_ptr(), off, n as i32, lc.as_mut_ptr(), mc.as_mut_ptr()) };
        r::split_lsb(&inp, off, n as i32, Some(&mut lr), Some(&mut mr));
        assert_eq!((lc, mc), (lr, mr));

        let mut oc = [7i16; MAXBANDS + 2];
        let mut or = oc;
        unsafe { c::cref_sacenc_nlc_enc_calc_diff_freq(inp.as_ptr(), oc.as_mut_ptr(), n as i32) };
        r::calc_diff_freq(&inp, &mut or, n as i32);
        assert_eq!(oc, or);
        unsafe {
            c::cref_sacenc_nlc_enc_calc_diff_time(inp.as_ptr(), prev.as_ptr(), oc.as_mut_ptr(), n as i32)
        };
        r::calc_diff_time(&inp, &prev, &mut or, n as i32);
        assert_eq!(oc, or);

        let lav = 2 * rng.range(0, 4) + 1;
        let mut dc = [rng.range(-lav, lav) as i16, rng.range(-lav, lav) as i16];
        let mut dr = dc;
        let (mut sc, mut sr) = (0i16, 0i16);
        let nc = unsafe { c::cref_sacenc_nlc_enc_sym_check(dc.as_mut_ptr(), lav, &mut sc) };
        let nr = r::sym_check(&mut dr, lav, Some(&mut sr));
        assert_eq!((nc, dc, sc), (nr, dr, sr));

        let v = rng.next_u32() >> rng.range(0, 31);
        assert_eq!(unsafe { c::cref_sacenc_nlc_enc_ilog2(v) }, r::ilog2(v));

        let lvls = [1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13, 15, 19, 25, 31, 51][rng.range(0, 15) as usize];
        let nv = rng.range(0, 50) as i16;
        assert_eq!(unsafe { c::cref_sacenc_nlc_enc_calc_pcm_bits(nv, lvls) }, r::calc_pcm_bits(nv, lvls));

        for lav in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 40, 32767] {
            for dt in [t_CLD, t_ICC] {
                assert_eq!(
                    unsafe { c::cref_sacenc_nlc_enc_get_next_lav_step(lav, dt) } as i8,
                    r::get_next_lav_step(lav, dt)
                );
            }
        }
    }
}

#[test]
fn pcm_and_lsb_coding() {
    let mut rng = Rng::new(3);
    for it in 0..10000 {
        let lvls = [3i16, 4, 5, 6, 7, 8, 9, 11, 13, 15, 19, 25, 31, 51][rng.range(0, 13) as usize];
        let off = rng.range(0, 3) as i16;
        let in1: Vec<i16> = (0..2 * MAXBANDS).map(|_| (rng.range(0, lvls as i32 - 1) - off as i32) as i16).collect();
        let in2: Vec<i16> = (0..2 * MAXBANDS).map(|_| (rng.range(0, lvls as i32 - 1) - off as i32) as i16).collect();
        let which = rng.range(1, 3);
        let n = rng.range(0, if which == 3 { 2 * MAXBANDS as i32 } else { MAXBANDS as i32 }) as i16;
        let mut b = Bs::new(&mut rng, 256);
        unsafe {
            c::cref_sacenc_nlc_enc_apply_pcm_coding(
                b.cbuf.as_mut_ptr(),
                b.size,
                b.pre,
                b.preval,
                in1.as_ptr(),
                in2.as_ptr(),
                which,
                off,
                n,
                lvls,
                b.state.as_mut_ptr(),
            )
        };
        let mut bs = b.rust();
        r::apply_pcm_coding(
            &mut bs,
            (which & 1 != 0).then_some(&in1[..]),
            (which & 2 != 0).then_some(&in2[..]),
            off,
            n,
            lvls,
        );
        b.check(&bs, &format!("pcm it {it}"));

        let nl = rng.range(0, 3) as u32;
        let n = rng.range(0, MAXBANDS as i32);
        let mut b = Bs::new(&mut rng, 256);
        unsafe {
            c::cref_sacenc_nlc_enc_apply_lsb_coding(
                b.cbuf.as_mut_ptr(),
                b.size,
                b.pre,
                b.preval,
                in1.as_ptr(),
                nl,
                n,
                b.state.as_mut_ptr(),
            )
        };
        let mut bs = b.rust();
        r::apply_lsb_coding(&mut bs, &in1, nl, n);
        b.check(&bs, &format!("lsb it {it}"));
    }
}

#[test]
fn huff_enc_1d_2d() {
    let mut rng = Rng::new(4);
    for it in 0..20000 {
        let dt = rng.range(0, 1);
        let with = rng.range(0, 1);
        let vmax = if dt == t_CLD { 30 } else { 7 };
        let mut inp: Vec<i16> = (0..MAXBANDS + 2).map(|_| rng.range(-vmax, vmax) as i16).collect();
        inp[0] = rng.range(0, vmax) as i16;
        let dim1 = rng.range(0, 1);
        let n = rng.range(0, MAXBANDS as i32) as i16;
        let p0 = rng.range(0, 1) as i16;
        let mut b = Bs::new(&mut rng, 256);
        let mut ic = inp.clone();
        let rc = unsafe {
            c::cref_sacenc_nlc_enc_huff_enc_1D(
                b.cbuf.as_mut_ptr(),
                b.size,
                b.pre,
                b.preval,
                with,
                dt,
                dim1,
                ic.as_mut_ptr(),
                n,
                p0,
                b.state.as_mut_ptr(),
            )
        };
        let mut bs = b.rust();
        let rr = r::huff_enc_1d((with != 0).then_some(&mut bs), dt, dim1, &inp, n, p0);
        assert_eq!(rc, rr, "1D it {it}");
        b.check(&bs, &format!("1D it {it}"));

        // 2D: pairs bounded by the LAV of lav_idx.
        let lav_idx = rng.range(0, 3) as i16;
        let lav = if dt == t_CLD { 2 * lav_idx as i32 + 3 } else { 2 * lav_idx as i32 + 1 };
        let mut pairs = [[0i16; 2]; MAXBANDS];
        for p in pairs.iter_mut() {
            p[0] = rng.range(-lav, lav) as i16;
            p[1] = rng.range(-lav, lav) as i16;
        }
        let tab = [rng.range(0, 1) as i16, rng.range(0, 1) as i16];
        let stride = rng.range(1, 2) as i16;
        let n = rng.range(0, MAXBANDS as i32) as i16;
        let p0has = rng.range(0, 3);
        let p00 = rng.range(0, vmax) as i16;
        let p01 = rng.range(0, vmax) as i16;
        let mut b = Bs::new(&mut rng, 256);
        let mut pc: Vec<i16> = pairs.iter().flatten().copied().collect();
        let rc = unsafe {
            c::cref_sacenc_nlc_enc_huff_enc_2D(
                b.cbuf.as_mut_ptr(),
                b.size,
                b.pre,
                b.preval,
                with,
                dt,
                tab[0],
                tab[1],
                lav_idx,
                pc.as_mut_ptr(),
                n,
                stride,
                p0has,
                p00,
                p01,
                b.state.as_mut_ptr(),
            )
        };
        let mut bs = b.rust();
        let mut pr = pairs;
        let rr = r::huff_enc_2d(
            (with != 0).then_some(&mut bs),
            dt,
            tab,
            lav_idx,
            &mut pr,
            n,
            stride,
            [(p0has & 1 != 0).then_some(p00), (p0has & 2 != 0).then_some(p01)],
        );
        assert_eq!(rc, rr, "2D it {it}");
        assert_eq!(pc, pr.iter().flatten().copied().collect::<Vec<_>>(), "2D data it {it}");
        b.check(&bs, &format!("2D it {it}"));

        // getHuffEntry for every LAV (also the ones without a table).
        let lav = [1, 3, 5, 7, 9, 2][rng.range(0, 5) as usize];
        let mut d = [[0i16; 2]; 4];
        for p in d.iter_mut() {
            p[0] = rng.range(0, lav.min(9)) as i16;
            p[1] = rng.range(0, lav.min(9)) as i16;
        }
        if (dt == t_ICC && lav == 9) || (dt == t_ICC && lav == 7) {
            for p in d.iter_mut() {
                p[0] = p[0].min(7);
                p[1] = p[1].min(7);
            }
        }
        let i = rng.range(0, 3) as usize;
        let (mut ec, mut xc) = (rng.next_u32(), rng.next_u32());
        let (mut er, mut xr) = (HuffEntry { packed: ec }, HuffEntry { packed: xc });
        let df: Vec<i16> = d.iter().flatten().copied().collect();
        unsafe {
            c::cref_sacenc_nlc_enc_getHuffEntry(lav, dt, i as i32, tab[0], tab[1], df.as_ptr(), &mut ec, &mut xc)
        };
        r::get_huff_entry(lav, dt, i, tab, &d, &mut er, &mut xr);
        assert_eq!((ec, xc), (er.packed, xr.packed), "getHuffEntry it {it}");
    }
}

/// The difference arrays that ecData*Enc builds: frequency differences or time differences
/// (first value, history value, differences).
fn diff_data(rng: &mut Rng, dt: i32, coarse: i32, n: usize, time: bool) -> Vec<i16> {
    let (lv, _) = levels(dt, coarse);
    let a = qdata(rng, lv, n);
    let mut out = vec![0i16; MAXBANDS + 2];
    if time {
        let h = qdata(rng, lv, n);
        out[0] = a[0];
        out[1] = h[0];
        for i in 0..n {
            out[i + 2] = a[i] - h[i];
        }
    } else {
        out[0] = a[0];
        for i in 1..n {
            out[i] = a[i] - a[i - 1];
        }
    }
    out
}

#[test]
fn calc_and_apply_huff_coding() {
    let mut rng = Rng::new(5);
    for it in 0..20000 {
        let dt = rng.range(0, 1);
        let coarse = rng.range(0, 1);
        let n = rng.range(1, MAXBANDS as i32) as usize;
        let dt1 = rng.range(0, 1);
        let dt2 = rng.range(0, 1);
        let which = rng.range(1, 3);
        let in1 = diff_data(&mut rng, dt, coarse, n, dt1 == 1);
        let in2 = diff_data(&mut rng, dt, coarse, n, dt2 == 1);
        let (mut i1c, mut i2c) = (in1.clone(), in2.clone());
        let (mut lc, mut sc) = ([0i16; 2], 0i16);
        let (mut lr, mut sr) = ([0i16; 2], 0i16);
        let bc = unsafe {
            c::cref_sacenc_nlc_enc_calc_huff_bits(
                i1c.as_mut_ptr(),
                i2c.as_mut_ptr(),
                which,
                dt,
                dt1,
                dt2,
                n as i16,
                lc.as_mut_ptr(),
                &mut sc,
            )
        };
        let br = r::calc_huff_bits(
            (which & 1 != 0).then_some(&in1[..]),
            (which & 2 != 0).then_some(&in2[..]),
            dt,
            dt1,
            dt2,
            n as i16,
            &mut lr,
            &mut sr,
        );
        assert_eq!((bc, lc, sc), (br, lr, sr), "calc_huff_bits it {it}");

        let mut b = Bs::new(&mut rng, 512);
        unsafe {
            c::cref_sacenc_nlc_enc_apply_huff_coding(
                b.cbuf.as_mut_ptr(),
                b.size,
                b.pre,
                b.preval,
                i1c.as_mut_ptr(),
                i2c.as_mut_ptr(),
                which,
                dt,
                dt1,
                dt2,
                n as i16,
                lc.as_ptr(),
                sc,
                b.state.as_mut_ptr(),
            )
        };
        let mut bs = b.rust();
        r::apply_huff_coding(
            &mut bs,
            (which & 1 != 0).then_some(&in1[..]),
            (which & 2 != 0).then_some(&in2[..]),
            dt,
            dt1,
            dt2,
            n as i16,
            &lr,
            sr,
        );
        b.check(&bs, &format!("apply_huff_coding it {it}"));

        // TIME_PAIR (not chosen by calc_huff_bits) with both inputs.
        if which == 3 {
            let lav = [rng.range(0, 3) as i16, 0];
            let mut b = Bs::new(&mut rng, 512);
            // Values bounded by the LAV of the chosen table.
            let bound = if dt == t_CLD { 2 * lav[0] + 3 } else { 2 * lav[0] + 1 };
            let clip = |v: &Vec<i16>, keep: usize| -> Vec<i16> {
                v.iter().enumerate().map(|(k, &x)| if k < keep { x } else { x.clamp(-bound, bound) }).collect()
            };
            let (t1, t2) = (clip(&in1, 1 + dt1 as usize), clip(&in2, 1 + dt2 as usize));
            let (mut t1c, mut t2c) = (t1.clone(), t2.clone());
            unsafe {
                c::cref_sacenc_nlc_enc_apply_huff_coding(
                    b.cbuf.as_mut_ptr(),
                    b.size,
                    b.pre,
                    b.preval,
                    t1c.as_mut_ptr(),
                    t2c.as_mut_ptr(),
                    3,
                    dt,
                    dt1,
                    dt2,
                    n as i16,
                    lav.as_ptr(),
                    0x11,
                    b.state.as_mut_ptr(),
                )
            };
            let mut bs = b.rust();
            r::apply_huff_coding(&mut bs, Some(&t1), Some(&t2), dt, dt1, dt2, n as i16, &lav, 0x11);
            b.check(&bs, &format!("time pair it {it}"));
        }
    }
}

/// The table `id` of the reference as INTs.
fn c_table(f: unsafe extern "C" fn(i32, *mut i32) -> i32, id: i32) -> Vec<i32> {
    let mut v = vec![0i32; 1024];
    let n = unsafe { f(id, v.as_mut_ptr()) } as usize;
    v.truncate(n);
    v
}

#[test]
fn tables() {
    use fdk_aac_rs_sacenc::sacenc_nlc_enc::{LAV_HUFF_LEN, LAV_HUFF_VAL, LAV_STEP_CLD, LAV_STEP_ICC};
    let f = c::cref_sacenc_nlc_enc_table;
    let i = |t: &[u8]| t.iter().map(|&x| x as i32).collect::<Vec<_>>();
    assert_eq!(c_table(f, 0), i(&LAV_HUFF_VAL), "lavHuffVal");
    assert_eq!(c_table(f, 1), i(&LAV_HUFF_LEN), "lavHuffLen");
    assert_eq!(c_table(f, 2), i(&LAV_STEP_CLD), "lav_step_CLD");
    assert_eq!(c_table(f, 3), i(&LAV_STEP_ICC), "lav_step_ICC");
}
