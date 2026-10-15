// Differential tests of fdk_aac_rs_sbrdec::huff_dec against the C++ reference.
//
// DecodeHuffmanCW with every SBR and PS codebook on random bitstreams (also past the end of the
// valid bits): the decoded values and the position after every code word are compared.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_cref::sbrdec_huff_dec::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_sbrdec::huff_dec::*;
use fdk_aac_rs_sbrdec::sbr_rom::*;

fn book(k: i32) -> Huffman {
    match k {
        0 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL10T,
        1 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL10F,
        2 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE10T,
        3 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE10F,
        4 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL11T,
        5 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL11F,
        6 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE11T,
        7 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE11F,
        8 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_NOISE_LEVEL11T,
        9 => &FDK_SBR_DECODER_SBR_HUFF_BOOK_NOISE_BALANCE11T,
        10 => &A_BOOK_PS_IID_TIME_DECODE,
        11 => &A_BOOK_PS_IID_FREQ_DECODE,
        12 => &A_BOOK_PS_ICC_TIME_DECODE,
        13 => &A_BOOK_PS_ICC_FREQ_DECODE,
        14 => &A_BOOK_PS_IID_FINE_TIME_DECODE,
        _ => &A_BOOK_PS_IID_FINE_FREQ_DECODE,
    }
}

#[test]
fn decode_huffman_cw_random() {
    let mut rng = Rng::new(0x4ff);
    for it in 0..20_000 {
        let k = rng.range(0, 15);
        let mut buf: Vec<u8> = (0..64).map(|_| rng.next_u32() as u8).collect();
        // (sometimes runs of equal bits: the longest code words)
        if rng.next_u32() % 4 == 0 {
            let v = if rng.next_u32() % 2 == 0 { 0 } else { 0xFF };
            let s = rng.range(0, 60) as usize;
            for b in &mut buf[s..(s + 4).min(64)] {
                *b = v;
            }
        }
        let valid = rng.range(0, 64 * 8);
        let n = 40;
        let mut out = vec![0i32; 2 * n];
        let mut cbuf = buf.clone();
        unsafe { cref_sbrdec_huff_dec_decode(k, cbuf.as_mut_ptr(), 64, valid, n as i32, out.as_mut_ptr()) };
        let mut bs = FdkBitStream::default();
        fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&buf), 64, valid as u32, BS_READER);
        for i in 0..n {
            let v = decode_huffman_cw(book(k), &mut bs);
            let left = fdk_get_valid_bits(&mut bs) as i32;
            assert_eq!([v, left], [out[2 * i], out[2 * i + 1]], "{it}: book {k} code word {i}");
        }
    }
}
