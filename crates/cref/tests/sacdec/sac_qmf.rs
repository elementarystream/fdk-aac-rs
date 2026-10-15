// Differential tests of fdk_aac_rs_sacdec::sac_qmf against the C++ reference: the filter banks
// are initialized in the configurations of the MPEG Surround decoder (64 and 32 bands, with and
// without the MPS low delay filter bank), then many slots of random signals are filtered; the
// outputs and the filter states are compared after every slot.

use crate::sac_dec_util::*;
use fdk_aac_rs_cref::fdk_qmf as cq;
use fdk_aac_rs_cref::sacdec_sac_qmf as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_domain::FdkQmfDomainOut;
use fdk_aac_rs_sacdec::sac_qmf::*;

struct CFb(*mut core::ffi::c_void);
impl Drop for CFb {
    fn drop(&mut self) {
        unsafe { cq::cref_qmf_free(self.0) };
    }
}

fn configs() -> Vec<(i32, u32)> {
    vec![(64, 0), (32, 0), (64, QMF_FLAG_MPSLDFB), (32, QMF_FLAG_MPSLDFB), (16, 0), (24, 0)]
}

#[test]
fn sac_qmf_analysis() {
    let mut rng = Rng::new(0x9a1);
    for (nch, flags) in configs() {
        for round in 0..4 {
            let cfb = CFb(unsafe { cq::cref_qmf_new() });
            let mut cstates = vec![0i32; 640];
            let mut states = vec![0i32; 640];
            let mut fb = QmfFilterBank::default();
            let no_cols = 32;
            let cr = unsafe {
                cq::cref_qmf_init_analysis_32(cfb.0, cstates.as_mut_ptr(), no_cols, nch, nch, nch, flags as i32)
            };
            let r = qmf_init_analysis_filter_bank::<i32>(
                &mut fb,
                Some(&mut states),
                no_cols,
                nch,
                nch,
                nch,
                flags,
            );
            assert_eq!(r, cr);
            for slot in 0..40 {
                let input: Vec<i32> = match (round + slot) % 3 {
                    0 => sigs(&mut rng, 128),
                    1 => (0..128).map(|_| rng.i32()).collect(),
                    _ => (0..128).map(|_| (rng.next_u32() as i32) >> 8).collect(),
                };
                let (mut re, mut im) = (vec![0i32; 64], vec![0i32; 64]);
                let (mut cre, mut cim) = (vec![0i32; 64], vec![0i32; 64]);
                let cr = unsafe {
                    c::cref_sacqmf_analysis(cfb.0, input.as_ptr(), cre.as_mut_ptr(), cim.as_mut_ptr())
                };
                let r = calculate_space_analysis_qmf(Some((&fb, &mut states)), &input, &mut re, &mut im);
                assert_eq!(r, cr);
                assert_eq!(re[..nch as usize], cre[..nch as usize], "real ({nch} {flags} {slot})");
                assert_eq!(im[..nch as usize], cim[..nch as usize], "imag ({nch} {flags} {slot})");
                assert_eq!(states, cstates, "states ({nch} {flags} {slot})");
            }
        }
    }
}

#[test]
fn sac_qmf_synthesis() {
    let mut rng = Rng::new(0x9a2);
    for (nch, flags) in configs() {
        for round in 0..4 {
            let cfb = CFb(unsafe { cq::cref_qmf_new() });
            let mut cstates = vec![0i32; 576];
            let mut states = vec![0i32; 576];
            let mut fb = QmfFilterBank::default();
            let no_cols = 32;
            let cr = unsafe {
                cq::cref_qmf_init_synthesis(cfb.0, cstates.as_mut_ptr(), no_cols, nch, nch, nch, flags as i32)
            };
            let r = qmf_init_synthesis_filter_bank(&mut fb, Some(&mut states), no_cols, nch, nch, nch, flags);
            assert_eq!(r, cr);
            // the output scaling of FDK_SpatialDecInit(): -7 + clipProtectGainSF, gain 0x80000000
            let sf = -7 + rng.range(0, 2);
            unsafe { cq::cref_qmf_change_out_scalefactor(cfb.0, sf) };
            qmf_change_out_scalefactor(&mut fb, &mut states, sf);
            let (gm, ge) = if round == 3 { (rng.i32(), rng.range(-2, 2)) } else { (0x80000000u32 as i32, 0) };
            unsafe { cq::cref_qmf_change_out_gain(cfb.0, gm, ge) };
            qmf_change_out_gain(&mut fb, gm, ge);
            let stride = [1, 2][round % 2];
            for slot in 0..40 {
                let re = sigs(&mut rng, 64);
                let im = sigs(&mut rng, 64);
                let mut out = vec![0x5555i32; 128];
                let mut cout = out.clone();
                let mut d = FdkQmfDomainOut { fb, p_syn_qmf_states: Some(states.clone().into_boxed_slice()) };
                let r = calculate_space_synthesis_qmf(Some(&mut d), &re, &im, stride, &mut out);
                fb = d.fb;
                states.copy_from_slice(d.p_syn_qmf_states.as_deref().unwrap());
                let cr = unsafe {
                    c::cref_sacqmf_synthesis(cfb.0, re.as_ptr(), im.as_ptr(), stride, cout.as_mut_ptr())
                };
                assert_eq!(r, cr);
                assert_eq!(out, cout, "output ({nch} {flags} {slot})");
                assert_eq!(states, cstates, "states ({nch} {flags} {slot})");
            }
        }
    }
    let r = calculate_space_synthesis_qmf(None, &[], &[], 1, &mut []) * 10000
        + calculate_space_analysis_qmf(None, &[], &mut [], &mut []);
    assert_eq!(r, unsafe { c::cref_sacqmf_null() });
}
