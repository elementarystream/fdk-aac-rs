// Tests of the optimized functions of fdk_aac_rs_aacdec::stereo against their straight ports,
// which keep their differential tests against C (stereo.rs; those run the optimized versions).

use fdk_aac_rs_aacdec::stereo::*;
use fdk_aac_rs_cref::testutil::Rng;

/// CJointStereo_GenerateMSOutput(): any band width (also not a multiple of 4), slices shorter
/// than the band, any scale (shift amounts modulo 32).
#[test]
fn generate_ms_output_fast_vs_straight() {
    let mut rng = Rng::new(0x57e0_f001);
    for _ in 0..100000 {
        let n = rng.range(0, 100) as u32;
        let nl = (n as i32 + rng.range(-3, 3)).max(0) as usize;
        let nr = (n as i32 + rng.range(-3, 3)).max(0) as usize;
        let l: Vec<i32> = (0..nl).map(|_| rng.i32()).collect();
        let r: Vec<i32> = (0..nr).map(|_| rng.i32()).collect();
        let (ls, rs) = if rng.next_u32() % 8 == 0 { (rng.next_u32(), rng.next_u32()) } else { (rng.range(0, 31) as u32, rng.range(0, 31) as u32) };
        let (mut al, mut ar) = (l.clone(), r.clone());
        cjoint_stereo_generate_ms_output_straight(&mut al, &mut ar, ls, rs, n);
        let (mut bl, mut br) = (l.clone(), r.clone());
        cjoint_stereo_generate_ms_output(&mut bl, &mut br, ls, rs, n);
        assert_eq!((al, ar), (bl, br), "GenerateMSOutput({n}, {ls}, {rs}) {nl} {nr}");
    }
}
