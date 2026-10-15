// Differential tests of fdk_aac_rs_fdk::nlc_dec against the C++ reference.
//
// The decoders read random bitstreams (random memory, positions and numbers of valid bits, see
// `bitbuffer::Pair`) and valid data written by the MPEG Surround encoder of the reference. After
// every call the return value, the outputs, the complete bitstream state and its memory are
// compared.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use super::bitbuffer::{state, Pair};
use fdk_aac_rs_cref::fdk_nlc_dec as c;
use fdk_aac_rs_cref::fdk_nlc_dec_enc as enc;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream, BS_READER};
use fdk_aac_rs_fdk::nlc_dec::*;

/// Length of the output and history arrays of the tests (start band + data bands).
const LEN: usize = 64;

/// A pair that reads `bits` bits of `mem` (a power of two bytes) from the start.
fn reader(mem: &[u8], bits: u32) -> Pair {
    let mut p = Pair { rs: FdkBitStream::default(), rbuf: BitMem::from_slice(mem), cst: [0; 9], cbuf: mem.to_vec() };
    fdk_init_bit_stream(&mut p.rs, &p.rbuf, mem.len() as u32, bits, BS_READER);
    p.cst = state(&p.rs);
    p
}

/// A random pair whose bit buffer has many valid bits most of the time.
fn random_pair(rng: &mut Rng) -> Pair {
    let size_log2 = 1 + rng.next_u32() % 8;
    let mut p = Pair::random(rng, size_log2);
    if rng.next_u32() % 4 != 0 {
        let b = &mut p.rs.h_bit_buf;
        b.valid_bits = b.buf_bits() - rng.next_u32() % 64.min(b.buf_bits());
        p.cst = state(&p.rs);
    }
    p
}

/// Runs EcDataPairDec on both sides and compares everything.
#[allow(clippy::too_many_arguments)]
fn ec_data_pair_dec_both(
    p: &mut Pair,
    decoder: i32,
    out: &mut [[i8; LEN]; 2],
    history: &[i8; LEN],
    data_type: i32,
    start: i32,
    bands: i32,
    pair: i32,
    coarse: i32,
    allow: i32,
    what: &str,
) -> i32 {
    let mut out_c = *out;
    let mut hist_c = *history;
    let [o1, o2] = out;
    let r = ec_data_pair_dec(
        decoder, &mut p.rs, o1, o2, history, data_type, start, bands, pair, coarse, allow,
    );
    let (st, buf) = p.c();
    let [c1, c2] = &mut out_c;
    let e = unsafe {
        c::cref_nlc_dec_ec_data_pair_dec(
            st,
            buf,
            decoder,
            c1.as_mut_ptr(),
            c2.as_mut_ptr(),
            hist_c.as_mut_ptr(),
            data_type,
            start,
            bands,
            pair,
            coarse,
            allow,
        )
    };
    let what = format!(
        "{what}: decoder {decoder} type {data_type} start {start} bands {bands} pair {pair} \
         coarse {coarse} allow {allow}"
    );
    assert_eq!(r, e, "return value, {what}");
    assert_eq!(*out, out_c, "outputs, {what}");
    assert_eq!(*history, hist_c, "history, {what}");
    p.check(&what);
    r
}

#[test]
fn ec_data_pair_dec_random() {
    let mut rng = Rng::new(30);
    let mut types = vec![t_CLD, t_ICC, t_IPD, t_OLD, t_IOC, t_PDG];
    if !cfg!(debug_assertions) {
        // The Huffman decoding of t_NRG hits FDK_ASSERT(0) (a debug_assert! in the port).
        types.push(t_NRG);
    }
    let mut ok = 0;
    for iter in 0..30_000 {
        let mut p = random_pair(&mut rng);
        let mut out = [[0i8; LEN]; 2];
        for o in out.iter_mut() {
            o.iter_mut().for_each(|v| *v = rng.next_u32() as i8);
        }
        let mut history = [0i8; LEN];
        history.iter_mut().for_each(|v| *v = rng.range(-16, 16) as i8);
        if iter % 8 == 0 {
            history.iter_mut().for_each(|v| *v = rng.next_u32() as i8);
        }
        // several parameter sets from the same bitstream
        for set in 0..4 {
            let data_type = types[(rng.next_u32() as usize) % types.len()];
            let decoder = rng.range(0, 2);
            let bands = match rng.next_u32() % 4 {
                0 => rng.range(0, 3),
                _ => rng.range(0, 28),
            };
            let start = rng.range(0, (LEN as i32 - bands).min(35));
            let (pair, coarse, allow) = (rng.range(0, 1), rng.range(0, 1), rng.range(0, 1));
            let what = format!("iter {iter} set {set}");
            let r = ec_data_pair_dec_both(
                &mut p, decoder, &mut out, &history, data_type, start, bands, pair, coarse, allow,
                &what,
            );
            if r == HUFFDEC_OK {
                ok += 1;
            }
        }
    }
    // The random streams must exercise the successful paths as well.
    assert!(ok > 10_000, "only {ok} successful decodes");
}

#[test]
fn ec_data_pair_dec_encoded() {
    let maxbands = unsafe { enc::cref_nlc_dec_enc_maxbands() } as usize;
    let mut rng = Rng::new(31);
    for iter in 0..20_000 {
        let data_type = rng.range(0, 1); // t_CLD, t_ICC
        let coarse = rng.range(0, 1);
        let (lo, hi) = match (data_type, coarse) {
            (0, 0) => (-15, 15),
            (0, _) => (-7, 7),
            (_, 0) => (0, 7),
            _ => (0, 3),
        };
        let bands = rng.range(1, maxbands as i32);
        let start = rng.range(0, maxbands as i32 - bands);
        let pair = rng.range(0, 1);
        let indep = rng.range(0, 1);
        // smooth data (small differences) or random data
        let smooth = iter % 2 == 0;
        let mut data = [[0i16; 64]; 2];
        let mut hist = [0i16; 64];
        let mut v = rng.range(lo, hi);
        for b in 0..maxbands {
            if smooth {
                v = (v + rng.range(-1, 1)).clamp(lo, hi);
                hist[b] = (v + rng.range(-1, 1)).clamp(lo, hi) as i16;
                data[0][b] = v as i16;
                data[1][b] = (v + rng.range(-1, 1)).clamp(lo, hi) as i16;
            } else {
                hist[b] = rng.range(lo, hi) as i16;
                data[0][b] = rng.range(lo, hi) as i16;
                data[1][b] = rng.range(lo, hi) as i16;
            }
        }
        // the encoder takes data[2][MAXBANDS] and history[MAXBANDS]
        let mut flat = vec![0i16; 2 * maxbands];
        flat[..maxbands].copy_from_slice(&data[0][..maxbands]);
        flat[maxbands..].copy_from_slice(&data[1][..maxbands]);
        let mut mem = vec![0u8; 256];
        let bits = unsafe {
            enc::cref_nlc_dec_enc_ec_data(
                mem.as_mut_ptr(),
                mem.len() as u32,
                flat.as_mut_ptr(),
                hist.as_mut_ptr(),
                data_type,
                start,
                bands,
                coarse,
                indep,
                pair,
            )
        };
        assert!(bits > 0, "nothing encoded");
        let mut p = reader(&mem, bits as u32);
        let mut out = [[0x55i8; LEN]; 2];
        let mut history = [0i8; LEN];
        for b in 0..maxbands {
            history[b] = hist[b] as i8;
        }
        let what = format!("iter {iter} encoded: type {data_type} coarse {coarse} start {start} bands {bands} pair {pair} indep {indep} bits {bits} data {:?} hist {:?}", &data[0][..maxbands], &hist[..maxbands]);
        let r = ec_data_pair_dec_both(
            &mut p,
            SAOC_DECODER, // libSACenc writes LD-MPS (ELDv2) data
            &mut out,
            &history,
            data_type,
            start,
            bands,
            pair,
            coarse,
            (indep == 0) as i32,
            &what,
        );
        assert_eq!(r, HUFFDEC_OK, "{what}");
        assert_eq!(fdk_get_valid_bits(&mut p.rs), 0, "{what}: bits left");
        let (s, n) = (start as usize, bands as usize);
        for k in 0..1 + pair as usize {
            let expect: Vec<i8> = data[k][s..s + n].iter().map(|&v| v as i8).collect();
            assert_eq!(&out[k][s..s + n], &expect[..], "{what}: decoded set {k}");
        }
    }
}

#[test]
fn huff_dec_reshape_random() {
    let mut rng = Rng::new(32);
    for iter in 0..30_000 {
        let mut p = random_pair(&mut rng);
        for call in 0..3 {
            let num_val = match rng.next_u32() % 4 {
                0 => rng.range(-2, 4),
                _ => rng.range(0, 72),
            };
            let mut out: Vec<i32> = (0..80).map(|_| rng.next_u32() as i32).collect();
            let mut out_c = out.clone();
            let r = huff_dec_reshape(&mut p.rs, &mut out, num_val);
            let (st, buf) = p.c();
            let e = unsafe { c::cref_nlc_dec_huff_dec_reshape(st, buf, out_c.as_mut_ptr(), num_val) };
            let what = format!("iter {iter} call {call} num_val {num_val}");
            assert_eq!(r, e, "{what}");
            assert_eq!(out, out_c, "{what}");
            p.check(&what);
        }
    }
}

#[test]
fn sym_restore_ipd_random() {
    let mut rng = Rng::new(33);
    for iter in 0..20_000 {
        let mut p = random_pair(&mut rng);
        let lav = rng.range(0, 8);
        let mut data = match iter % 2 {
            0 => [rng.range(0, 15) as i8, rng.range(0, 15) as i8],
            _ => [rng.next_u32() as i8, rng.next_u32() as i8],
        };
        let mut data_c = data;
        let r = sym_restore_ipd(&mut p.rs, lav, &mut data);
        let (st, buf) = p.c();
        let e = unsafe { c::cref_nlc_dec_sym_restore_ipd(st, buf, lav, data_c.as_mut_ptr()) };
        assert_eq!(r, e);
        assert_eq!(data, data_c, "iter {iter} lav {lav}");
        p.check("sym_restoreIPD");
    }
}
