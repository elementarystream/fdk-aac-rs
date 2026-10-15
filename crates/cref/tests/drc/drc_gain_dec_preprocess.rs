// Differential tests of fdk_aac_rs_drc::drc_gain_dec_preprocess against the C++ reference.
//
// The characteristic functions (_getSlopeSign, _compressorIO_sigmoid(_inverse),
// _compressorIO_nodes(_inverse), _mapGain) run on the CICP characteristics and on random custom
// characteristics as the reader produces them, with random levels and gains.
// prepareDrcGain and _toLinear are covered by the API test (drc_dec_lib.rs).

use fdk_aac_rs_cref::drc_drc_gain_dec_preprocess as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_drc::drc_dec_rom::*;
use fdk_aac_rs_drc::drc_dec_types::*;
use fdk_aac_rs_drc::drc_gain_dec_preprocess::*;

/// A characteristic as `_readCustomDrcCharacteristic` or the CICP tables give it, with its
/// format.
fn random_char(rng: &mut Rng) -> (i32, CustomDrcChar) {
    let mut c = CustomDrcChar::ZERO;
    match rng.next_u32() % 6 {
        0 => {
            let i = (rng.next_u32() % 6) as usize;
            *c.sigmoid_mut() = if rng.next_u32() % 2 == 0 {
                CICP_DRC_CHAR_SIGMOID_LEFT[i]
            } else {
                CICP_DRC_CHAR_SIGMOID_RIGHT[i]
            };
            (CF_SIGMOID, c)
        }
        1 => {
            let i = (rng.next_u32() % 5) as usize;
            *c.nodes_mut() = if rng.next_u32() % 2 == 0 {
                CICP_DRC_CHAR_NODES_LEFT[i]
            } else {
                CICP_DRC_CHAR_NODES_RIGHT[i]
            };
            (CF_NODES, c)
        }
        2 | 3 => {
            let s = c.sigmoid_mut();
            let bs_gain = rng.range(0, 63);
            s.gain = if rng.next_u32() % 2 == 0 { bs_gain << 9 } else { (-bs_gain) << 9 } as i16;
            let r = rng.range(0, 15);
            s.io_ratio = (410 + ((((3 * r) << 15) / 5) >> 4)) as i16;
            let e = rng.range(0, 15);
            s.exp = if e < 15 { ((1 + 2 * e) << 10) as i16 } else { 32767 };
            s.flip_sign = (rng.next_u32() % 2) as u8;
            (CF_SIGMOID, c)
        }
        _ => {
            let n = c.nodes_mut();
            n.characteristic_node_count = rng.range(1, 4) as u8;
            n.node_level[0] = -7936;
            n.node_gain[0] = 0;
            let left = rng.next_u32() % 2 == 0;
            for i in 0..n.characteristic_node_count as usize {
                let d = ((1 + rng.range(0, 31)) << 8) as i16;
                n.node_level[i + 1] =
                    if left { n.node_level[i].wrapping_sub(d) } else { n.node_level[i].wrapping_add(d) };
                n.node_gain[i + 1] = ((rng.range(0, 255) - 128) << 7) as i16;
            }
            (CF_NODES, c)
        }
    }
}

fn level(rng: &mut Rng) -> i32 {
    match rng.next_u32() % 4 {
        0 => rng.i32(),
        _ => rng.range(-100, 40) << 24 | (rng.next_u32() as i32 & 0xff_ffff),
    }
}

fn gain(rng: &mut Rng) -> i16 {
    match rng.next_u32() % 4 {
        0 => rng.i16(),
        1 => 0,
        _ => (rng.range(-64 * 8, 64 * 8) << 5) as i16,
    }
}

#[test]
fn characteristics() {
    let mut rng = Rng::new(0xd5c0_4001);
    let mut ok = [0u32; 5];
    for _ in 0..200_000 {
        let (fs, s) = random_char(&mut rng);
        let (ft, t) = random_char(&mut rng);
        let sb = pod_bytes(&s).as_ptr();
        let tb = pod_bytes(&t).as_ptr();

        let (mut r_sign, mut c_sign) = (7, 7);
        let r = get_slope_sign(fs, &s, &mut r_sign);
        let cr = unsafe { c::cref_drc_pre_get_slope_sign(fs, sb, &mut c_sign) };
        assert_eq!((r, r_sign), (cr, c_sign), "_getSlopeSign({fs}, {s:?})");

        let lv = level(&mut rng);
        let g = gain(&mut rng);
        if fs == CF_SIGMOID {
            let (mut o, mut co) = (0x1234, 0x1234);
            let r = compressor_io_sigmoid(s.sigmoid(), lv, &mut o);
            let cr = unsafe { c::cref_drc_pre_sigmoid(sb, lv, &mut co) };
            assert_eq!((r, o), (cr, co), "_compressorIO_sigmoid({:?}, {lv})", s.sigmoid());
            ok[0] += (r == 0) as u32;
            let r = compressor_io_sigmoid_inverse(s.sigmoid(), g, &mut o);
            let cr = unsafe { c::cref_drc_pre_sigmoid_inverse(sb, g, &mut co) };
            assert_eq!((r, o), (cr, co), "_compressorIO_sigmoid_inverse({:?}, {g})", s.sigmoid());
            ok[1] += (r == 0) as u32;
        } else {
            let (mut o, mut co) = (0x1234, 0x1234);
            let r = compressor_io_nodes(s.nodes(), lv, &mut o);
            let cr = unsafe { c::cref_drc_pre_nodes(sb, lv, &mut co) };
            assert_eq!((r, o), (cr, co), "_compressorIO_nodes({:?}, {lv})", s.nodes());
            ok[2] += 1;
            let r = compressor_io_nodes_inverse(s.nodes(), g, &mut o);
            let cr = unsafe { c::cref_drc_pre_nodes_inverse(sb, g, &mut co) };
            assert_eq!((r, o), (cr, co), "_compressorIO_nodes_inverse({:?}, {g})", s.nodes());
            ok[3] += 1;
        }

        let (mut o, mut co) = (0x1234, 0x1234);
        let r = map_gain(fs, &s, ft, &t, g, &mut o);
        let cr = unsafe { c::cref_drc_pre_map_gain(fs, sb, ft, tb, g, &mut co) };
        assert_eq!((r, o), (cr, co), "_mapGain({fs}, {s:?}, {ft}, {t:?}, {g})");
        ok[4] += (r == 0) as u32;
    }
    assert!(ok.iter().all(|&n| n > 10_000), "{ok:?}");
}
