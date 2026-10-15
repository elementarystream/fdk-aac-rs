// Differential tests of fdk_aac_rs_sacenc::sacenc_staticgain against the C++ reference.
//
// The configuration functions, the initialization for every downmix gain and pre gain factor
// (and invalid ones), the getters, and the post gain on random and realistic PCM with every
// downmix gain and scale factors of -31..=31.

use fdk_aac_rs_cref::sacenc_sacenc_staticgain as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_sacenc::sacenc_staticgain as r;
use fdk_aac_rs_sacenc::sacenc_staticgain::{StaticGain, StaticGainConfig};

use super::sacenc_filter::pcm;

pub fn gain_ints(g: &StaticGain) -> [i32; 6] {
    [g.enc_mode, g.fixed_gain_dmx, g.pre_gain_factor_db, g.post_gain_fdk, g.p_pre_gain_fdk[0], g.p_pre_gain_fdk[1]]
}

fn gain_from(v: &[i32; 6]) -> StaticGain {
    StaticGain {
        enc_mode: v[0],
        fixed_gain_dmx: v[1],
        pre_gain_factor_db: v[2],
        post_gain_fdk: v[3],
        p_pre_gain_fdk: [v[4], v[5]],
    }
}

fn cfg_ints(c: &StaticGainConfig) -> [i32; 3] {
    [c.enc_mode, c.fixed_gain_dmx, c.pre_gain_factor_db]
}

#[test]
fn config_and_init() {
    let mut s = [0i32; 2];
    unsafe { c::cref_sacenc_staticgain_sizes(s.as_mut_ptr()) };
    assert_eq!(s, [3, 6]);

    let mut v = [0i32; 9];
    unsafe { c::cref_sacenc_staticgain_null(v.as_mut_ptr()) };
    let r = [
        r::fdk_sacenc_static_gain_open_config(None),
        r::fdk_sacenc_static_gain_open(None),
        r::fdk_sacenc_static_gain_close_config(Some(&mut None)),
        r::fdk_sacenc_static_gain_close(Some(&mut None)),
        r::fdk_sacenc_static_gain_init_default_config(None),
        r::fdk_sacenc_static_gain_set_dmx_gain(None, 0),
        r::fdk_sacenc_static_gain_set_enc_mode(None, 8),
        r::fdk_sacenc_static_gain_close_config(None),
        r::fdk_sacenc_static_gain_close(None),
    ];
    assert_eq!(v, r);

    let mut rng = Rng::new(1);
    for it in 0..5000 {
        let hc = unsafe { c::cref_sacenc_staticgain_open_config() };
        let hg = unsafe { c::cref_sacenc_staticgain_open() };
        let mut rc: Option<Box<StaticGainConfig>> = None;
        let mut rg: Option<Box<StaticGain>> = None;
        assert_eq!(r::fdk_sacenc_static_gain_open_config(Some(&mut rc)), 0);
        assert_eq!(r::fdk_sacenc_static_gain_open(Some(&mut rg)), 0);
        let (rc, rg) = (rc.as_mut().unwrap(), rg.as_mut().unwrap());

        let mut cv = [0i32; 3];
        unsafe { c::cref_sacenc_staticgain_get_config(hc, cv.as_mut_ptr()) };
        assert_eq!(cv, cfg_ints(rc), "fresh config");

        assert_eq!(unsafe { c::cref_sacenc_staticgain_init_default(hc) }, r::fdk_sacenc_static_gain_init_default_config(Some(rc)));
        let dmx = if rng.range(0, 3) == 0 { rng.range(-2, 9) } else { rng.range(0, 7) };
        if rng.range(0, 4) != 0 {
            assert_eq!(unsafe { c::cref_sacenc_staticgain_set_dmx_gain(hc, dmx) }, r::fdk_sacenc_static_gain_set_dmx_gain(Some(rc), dmx));
        }
        let mode = [8, 8, 0, 15][rng.range(0, 3) as usize];
        assert_eq!(unsafe { c::cref_sacenc_staticgain_set_enc_mode(hc, mode) }, r::fdk_sacenc_static_gain_set_enc_mode(Some(rc), mode));
        if rng.range(0, 2) == 0 {
            // preGainFactorDb is not settable through the API.
            unsafe { c::cref_sacenc_staticgain_get_config(hc, cv.as_mut_ptr()) };
            cv[2] = rng.range(-22, 22);
            unsafe { c::cref_sacenc_staticgain_set_config(hc, cv.as_ptr()) };
            rc.pre_gain_factor_db = cv[2];
        }
        if !(1..=7).contains(&rc.fixed_gain_dmx) && rc.fixed_gain_dmx != 0 {
            // No defined reference result (see the module documentation): skipped.
            unsafe {
                c::cref_sacenc_staticgain_close(hg);
                c::cref_sacenc_staticgain_close_config(hc);
            }
            continue;
        }
        unsafe { c::cref_sacenc_staticgain_get_config(hc, cv.as_mut_ptr()) };
        assert_eq!(cv, cfg_ints(rc));

        let (mut sc, mut sr) = (-99, -99);
        let ec = unsafe { c::cref_sacenc_staticgain_init(hg, hc, &mut sc) };
        let er = r::fdk_sacenc_static_gain_init(rg, rc, &mut sr);
        let mut gv = [0i32; 6];
        unsafe { c::cref_sacenc_staticgain_get(hg, gv.as_mut_ptr()) };
        assert_eq!((ec, sc, gv), (er, sr, gain_ints(rg)), "init it {it}");

        let mut g = [0i32; 6];
        unsafe { c::cref_sacenc_staticgain_getters(hg, g.as_mut_ptr()) };
        let p = r::fdk_sacenc_get_pre_gain_ptr_fdk(rg);
        assert_eq!(
            g,
            [p[0], p[1], 1, r::fdk_sacenc_get_post_gain_fdk(rg), r::fdk_sacenc_static_gain_get_dmx_gain(rg), 1]
        );

        unsafe {
            assert_eq!(c::cref_sacenc_staticgain_close(hg), 0);
            assert_eq!(c::cref_sacenc_staticgain_close_config(hc), 0);
        }
    }
}

#[test]
fn post_gain() {
    let mut rng = Rng::new(2);
    let hg = unsafe { c::cref_sacenc_staticgain_open() };
    for it in 0..20000 {
        let post = match rng.range(0, 3) {
            0 => 0x7FFF_FFFF,
            1 => [0x6BA27F00, 0x5A827C00, 0x4C1BFA80, 0x40000000, 0x35D13F80, 0x2D413D00, 0x20000000][rng.range(0, 6) as usize],
            _ => rng.i32(),
        };
        let g = gain_from(&[8, 2, 0, post, rng.i32(), rng.i32()]);
        unsafe { c::cref_sacenc_staticgain_set(hg, gain_ints(&g).as_ptr()) };
        let n = rng.range(0, 1100) as usize;
        let x = pcm(&mut rng, n.max(1));
        let scale = if rng.range(0, 1) == 0 { 0 } else { rng.range(-31, 31) };
        let mut xc = x.clone();
        let mut xr = x.clone();
        let ec = unsafe { c::cref_sacenc_staticgain_apply(hg, xc.as_mut_ptr(), n as i32, scale) };
        let er = r::fdk_sacenc_static_post_gain_apply_fdk(&g, &mut xr, n as i32, scale);
        assert_eq!((ec, xc), (er, xr), "it {it} post {post:#x} scale {scale}");
    }
    unsafe { c::cref_sacenc_staticgain_close(hg) };
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
    use fdk_aac_rs_sacenc::sacenc_staticgain::{DMX_GAIN_TABLE_FDK, PRE_GAIN_FACTOR_TABLE_FDK};
    let f = c::cref_sacenc_staticgain_table;
    assert_eq!(c_table(f, 0), PRE_GAIN_FACTOR_TABLE_FDK.to_vec(), "preGainFactorTable__FDK");
    assert_eq!(c_table(f, 1), DMX_GAIN_TABLE_FDK.to_vec(), "dmxGainTable__FDK");
}
