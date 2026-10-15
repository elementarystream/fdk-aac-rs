// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The low power transposer with the band loop of `lp_bands` against its straight port, with the
//! arithmetic of the selected variant (these unit tests run with every `FDK_AAC_RS_ARCH` variant
//! and with `FDK_AAC_RS_NO_SIMD` on the host; `cref` has the same test with the C++ reference's
//! variant and random headers checked against C++).

use super::*;
use crate::env_extr::*;
use crate::sbr_ram::pod_bytes;
use crate::sbrdec_freq_sca::reset_freq_band_tables;

/// xorshift64*
struct Rng(u64);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    /// `lo..=hi`
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next_u32() as u64 % (hi as i64 - lo as i64 + 1) as u64) as i32
    }

    /// Edge values, values of varying magnitude and full range values.
    fn i32(&mut self) -> i32 {
        match self.next_u32() % 16 {
            0 => [0, 1, -1, i32::MAX, i32::MIN, i32::MIN + 1, 0x4000_0000, -0x4000_0000]
                [(self.next_u32() % 8) as usize],
            1..=5 => (self.next_u32() as i32) >> (self.next_u32() % 32),
            _ => self.next_u32() as i32,
        }
    }
}

/// A header of a random AAC or ELD configuration with random frequency band tables.
fn random_header(rng: &mut Rng) -> Option<(SbrHeaderData, i32)> {
    let (flags, overlap) = if rng.next_u32() % 2 == 0 { (0, 6) } else { (SBRDEC_ELD_GRID, 0) };
    let rates = [8000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 64000];
    let sr_in = rates[rng.range(0, rates.len() as i32 - 1) as usize];
    let sr_out = if rng.next_u32() % 4 == 0 { sr_in } else { 2 * sr_in };
    let (frame, downscale) = if flags & SBRDEC_ELD_GRID != 0 {
        ([512, 480, 256, 240][rng.range(0, 3) as usize], [1, 2, 4][rng.range(0, 2) as usize])
    } else {
        ([1024, 960][rng.range(0, 1) as usize], 1)
    };
    let mut h = SbrHeaderData::ZERO;
    if init_header_data(&mut h, sr_in, sr_out, downscale, frame, flags, 1) != 0 {
        return None;
    }
    h.bs_data.start_freq = rng.range(0, 15) as u8;
    h.bs_data.stop_freq = rng.range(0, 15) as u8;
    h.bs_data.freq_scale = rng.range(0, 3) as u8;
    h.bs_data.alter_scale = rng.range(0, 1) as u8;
    h.bs_data.noise_bands = rng.range(0, 3) as u8;
    h.bs_info.xover_band = rng.range(0, 3) as u8;
    if reset_freq_band_tables(&mut h, flags) != 0 {
        return None;
    }
    Some((h, overlap))
}

#[test]
fn transposer_bands_match_straight() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut runs = 0;
    for it in 0..4000 {
        let Some((h, overlap)) = random_header(&mut rng) else {
            continue;
        };
        let f = &h.freq_band_data;
        let time_slots = h.number_time_slots as i32;
        let n_cols = time_slots * h.time_step as i32;
        let mut sp = TransposerSettings::ZERO;
        let mut hp = SbrLppTrans::ZERO;
        if create_lpp_transposer(
            &mut hp,
            &mut sp,
            f.low_subband as i32,
            &f.v_k_master,
            f.num_master as i32,
            f.high_subband as i32,
            time_slots,
            n_cols,
            &f.freq_band_table_noise,
            f.n_nfb as i32,
            h.sbr_proc_smpl_rate,
            0,
            overlap,
        ) != 0
        {
            continue;
        }
        // overlapping and out of order patches
        if it % 8 == 7 && sp.no_of_patches > 1 {
            let p = rng.range(0, sp.no_of_patches as i32 - 1) as usize;
            let q = rng.range(0, sp.no_of_patches as i32 - 1) as usize;
            sp.patch_param[p].target_band_offs =
                sp.patch_param[q].target_band_offs.wrapping_add(rng.range(-3, 3) as u8).min(40);
        }
        for v in hp.lpc.iter_mut() {
            *v = (rng.next_u32() as i32) >> rng.range(0, 31);
        }
        for v in hp.bw_vector_old.iter_mut() {
            *v = (rng.next_u32() >> 1) as i32;
        }
        let mut hs = hp;
        let mut invf_prev = [0i32; MAX_INVF_BANDS];
        for fr in 0..3 {
            let class = rng.next_u32() % 5;
            let re: Vec<i32> = (0..128 * 64)
                .map(|_| match class {
                    0 => 0,
                    1 => rng.i32(),
                    2 => [i32::MIN, i32::MAX, -1, 1][rng.range(0, 3) as usize],
                    _ => (rng.next_u32() as i32) >> rng.range(2, 30),
                })
                .collect();
            let alias: Vec<i32> = (0..64).map(|_| rng.i32()).collect();
            let (mut re_b, mut alias_b) = (re.clone(), alias.clone());
            let (mut re_s, mut alias_s) = (re, alias);
            let sf = QmfScaleFactor {
                lb_scale: rng.range(-10, 20),
                ov_lb_scale: rng.range(-10, 20),
                hb_scale: rng.range(-10, 20),
                ov_hb_scale: rng.range(-10, 20),
            };
            let (mut sf_b, mut sf_s) = (sf, sf);
            let mut invf = [0i32; MAX_INVF_BANDS];
            for v in invf.iter_mut() {
                *v = rng.range(0, 3);
            }
            let pre = rng.range(0, 1);
            let max_off = sp.overlap as i32 / h.time_step as i32;
            let first = rng.range(0, max_off);
            let last = rng.range(0, max_off);
            let ninvf = f.n_invf_bands as i32;
            let k0 = f.v_k_master[0] as i32;
            let step = h.time_step as i32;
            let mut r: Vec<&mut [i32]> = re_b.chunks_mut(64).collect();
            lpp_transposer(
                &mut hp,
                &sp,
                &mut sf_b,
                &mut r,
                &mut alias_b,
                &mut [],
                true,
                pre,
                k0,
                step,
                first,
                last,
                ninvf,
                &invf,
                &invf_prev,
            );
            let mut r: Vec<&mut [i32]> = re_s.chunks_mut(64).collect();
            lpp_transposer_straight(
                &mut hs,
                &sp,
                &mut sf_s,
                &mut r,
                &mut alias_s,
                &mut [],
                true,
                pre,
                k0,
                step,
                first,
                last,
                ninvf,
                &invf,
                &invf_prev,
            );
            let what = format!("lppTransposer bands {it}/{fr}");
            assert!(sf_b == sf_s, "{what}: scale");
            assert_eq!(pod_bytes(&hp), pod_bytes(&hs), "{what}: state");
            if let Some(k) = (0..re_b.len()).find(|&k| re_b[k] != re_s[k]) {
                panic!("{what}: slot {} band {}: {} vs {}", k / 64, k % 64, re_b[k], re_s[k]);
            }
            assert_eq!(alias_b, alias_s, "{what}: degreeAlias");
            invf_prev = invf;
            runs += 1;
        }
    }
    assert!(runs > 1000, "runs {runs}");
}
