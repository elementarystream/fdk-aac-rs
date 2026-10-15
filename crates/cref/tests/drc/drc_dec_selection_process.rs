// Differential tests of fdk_aac_rs_drc::drc_dec_selection_process against the C++ reference.
//
// A selection process pair gets random request parameters (all SEL_PROC_USER_PARAM values,
// valid and out of range) and runs on generated (parsed) configurations and loudness info sets.
// After every call the return value, the output, the configuration (the process writes the
// virtual DRC sets into it) and the complete state of the selection process are compared.

use core::ffi::c_void;
use fdk_aac_rs_cref::drc_drc_dec_selection_process as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_drc::drc_dec_reader::{drc_dec_read_loudness_info_set, drc_dec_read_uni_drc_config};
use fdk_aac_rs_drc::drc_dec_selection_process::*;
use fdk_aac_rs_drc::drc_dec_types::*;

use super::drc_dec_reader::{
    cmp_bytes, gen_loudness_info_set, gen_uni_drc_config, set_clean, Payload,
};

struct SelPair {
    rs: HandleDrcSelectionProcess,
    c: *mut c_void,
    slot: i32,
}

impl Drop for SelPair {
    fn drop(&mut self) {
        unsafe { c::cref_drc_sel_delete(self.c) };
    }
}

impl SelPair {
    fn new(slot: i32) -> SelPair {
        let mut rs = None;
        let r = drc_dec_selection_process_create(&mut rs);
        let mut cr = 0;
        let c = unsafe { c::cref_drc_sel_create(&mut cr) };
        assert_eq!(r, cr);
        let r = drc_dec_selection_process_init(rs.as_deref_mut());
        let cr = unsafe { c::cref_drc_sel_init(c) };
        assert_eq!(r, cr);
        let p = SelPair { rs, c, slot };
        p.check("init");
        p
    }

    fn check(&self, what: &str) {
        let mut st = vec![0u8; core::mem::size_of::<DrcSelectionProcess>()];
        unsafe { c::cref_drc_sel_state(self.c, self.slot, st.as_mut_ptr()) };
        cmp_bytes(&format!("{what}: state"), pod_bytes(self.rs.as_deref().unwrap()), &st);
    }
}

fn param(rng: &mut Rng) -> (i32, i32) {
    let t = rng.range(-1, 14);
    let v = match t {
        SEL_PROC_LOUDNESS_NORMALIZATION_ON | SEL_PROC_ALBUM_MODE => rng.range(-1, 2),
        SEL_PROC_TARGET_LOUDNESS => match rng.next_u32() % 4 {
            0 => rng.i32(),
            _ => -rng.range(0, 70) * (1 << 24),
        },
        SEL_PROC_EFFECT_TYPE => rng.range(-2, 10),
        SEL_PROC_LOUDNESS_MEASUREMENT_METHOD => rng.range(-1, 3),
        SEL_PROC_DOWNMIX_ID => rng.range(-2, 8),
        SEL_PROC_TARGET_LAYOUT => rng.range(0, 64),
        SEL_PROC_TARGET_CHANNEL_COUNT => rng.range(0, 9),
        SEL_PROC_BASE_CHANNEL_COUNT => rng.range(-1, 9),
        SEL_PROC_SAMPLE_RATE => [48000, -1, 44100, 0][rng.next_u32() as usize % 4],
        SEL_PROC_BOOST | SEL_PROC_COMPRESS => match rng.next_u32() % 4 {
            0 => rng.i32(),
            _ => rng.range(0, 0x4000_0000),
        },
        _ => rng.i32(),
    };
    (t, v)
}

#[test]
fn selection_process() {
    let mut rng = Rng::new(0xd5c0_3001);
    let mut selected_real = 0;
    for run in 0..1500 {
        set_clean(rng.next_u32() % 4 != 0);
        let mut p = SelPair::new(0);
        let mode = match rng.next_u32() % 8 {
            0 => rng.range(-101, 3),
            1 => SEL_PROC_MPEG_4_AAC,
            2 => SEL_PROC_TEST_TIME_DOMAIN,
            _ => SEL_PROC_MPEG_D_USAC,
        };
        let r = drc_dec_selection_process_set_codec_mode(p.rs.as_deref_mut(), mode);
        let cr = unsafe { c::cref_drc_sel_set_codec_mode(p.c, mode) };
        assert_eq!(r, cr, "setCodecMode({mode})");
        p.check("setCodecMode");

        let mut cfg = Box::new(UniDrcConfig::ZERO);
        let mut lis = Box::new(LoudnessInfoSet::ZERO);
        let mut out = SelProcOutput::ZERO;
        for step in 0..12 {
            if step == 0 || rng.next_u32() % 4 == 0 {
                let w = gen_uni_drc_config(&mut rng);
                let (_b, mut bs) = Payload::new(&w, None).rs();
                if drc_dec_read_uni_drc_config(&mut bs, &mut cfg) != 0 {
                    *cfg = UniDrcConfig::ZERO;
                }
            }
            if step == 0 || rng.next_u32() % 4 == 0 {
                let w = gen_loudness_info_set(&mut rng);
                let (_b, mut bs) = Payload::new(&w, None).rs();
                if drc_dec_read_loudness_info_set(&mut bs, &mut lis) != 0 {
                    *lis = LoudnessInfoSet::ZERO;
                }
            }
            for _ in 0..rng.range(0, 3) {
                let (t, v) = param(&mut rng);
                let null_diff = rng.next_u32() % 8 == 0;
                let (mut d, mut cd) = (0, 0);
                let r = drc_dec_selection_process_set_param(
                    p.rs.as_deref_mut().unwrap(),
                    t,
                    v,
                    if null_diff { None } else { Some(&mut d) },
                );
                let cr = unsafe { c::cref_drc_sel_set_param(p.c, t, v, &mut cd, null_diff as i32) };
                assert_eq!((r, d), (cr, cd), "setParam({t}, {v})");
                p.check(&format!("setParam({t}, {v})"));
            }
            for t in -1..3 {
                let r = drc_dec_selection_process_get_param(p.rs.as_deref().unwrap(), t);
                let cr = unsafe { c::cref_drc_sel_get_param(p.c, t) };
                assert_eq!(r, cr, "getParam({t})");
            }

            let mut c_cfg = pod_bytes(&*cfg).to_vec();
            let mut c_out = pod_bytes(&out).to_vec();
            let mut st = vec![0u8; core::mem::size_of::<DrcSelectionProcess>()];
            let r = drc_dec_selection_process_process(p.rs.as_deref_mut(), &mut cfg, &lis, &mut out);
            let cr = unsafe {
                c::cref_drc_sel_process(
                    p.c,
                    p.slot,
                    c_cfg.as_mut_ptr(),
                    pod_bytes(&*lis).as_ptr(),
                    c_out.as_mut_ptr(),
                    st.as_mut_ptr(),
                )
            };
            let what = format!("run {run} step {step}: process");
            assert_eq!(r, cr, "{what}");
            cmp_bytes(&format!("{what}: output"), pod_bytes(&out), &c_out);
            cmp_bytes(&format!("{what}: config"), pod_bytes(&*cfg), &c_cfg);
            cmp_bytes(&format!("{what}: state"), pod_bytes(p.rs.as_deref().unwrap()), &st);
            if r == 0 && out.selected_drc_set_ids[..out.num_selected_drc_sets.min(3) as usize]
                .iter()
                .any(|&id| id > 0)
            {
                selected_real += 1;
            }
            if rng.next_u32() % 16 == 0 {
                out = SelProcOutput::ZERO;
            }
        }
    }
    assert!(selected_real > 1000, "{selected_real}");
}

#[test]
fn null_handles() {
    let mut cfg = Box::new(UniDrcConfig::ZERO);
    let lis = Box::new(LoudnessInfoSet::ZERO);
    let mut out = SelProcOutput::ZERO;
    let r = drc_dec_selection_process_process(None, &mut cfg, &lis, &mut out);
    let mut c_cfg = pod_bytes(&*cfg).to_vec();
    let mut c_out = pod_bytes(&out).to_vec();
    let mut st = vec![0u8; core::mem::size_of::<DrcSelectionProcess>()];
    let cr = unsafe {
        c::cref_drc_sel_process(
            core::ptr::null_mut(),
            1,
            c_cfg.as_mut_ptr(),
            pod_bytes(&*lis).as_ptr(),
            c_out.as_mut_ptr(),
            st.as_mut_ptr(),
        )
    };
    assert_eq!(r, cr);
    assert_eq!(drc_dec_selection_process_init(None), DRCDEC_SELECTION_PROCESS_NOT_OK);
    assert_eq!(drc_dec_selection_process_set_codec_mode(None, 1), DRCDEC_SELECTION_PROCESS_NOT_OK);
    let mut h: HandleDrcSelectionProcess = None;
    assert_eq!(drc_dec_selection_process_delete(&mut h), DRCDEC_SELECTION_PROCESS_INVALID_HANDLE);
}

/// FALLBACK_EFFECT_TYPE_REQUESTS (`fallbackEffectTypeRequests`, file-static; a copy compiled into
/// the shim), element by element.
#[test]
fn fallback_effect_type_requests() {
    use fdk_aac_rs_cref::drc_drc_dec_selection_process_tables as t;
    let mut bytes = 0;
    let p = unsafe { t::cref_drc_sel_fallback_effect_type_requests(&mut bytes) } as *const i32;
    let c = unsafe { std::slice::from_raw_parts(p, bytes as usize / 4) };
    let r: Vec<i32> = FALLBACK_EFFECT_TYPE_REQUESTS.iter().flatten().copied().collect();
    assert_eq!(c, &r[..]);
}

/// P_ORDERING (`pOrdering`, function-local in `_getMethodValue()`), element by element through
/// the function of the copy: with the order -1, it stores the table element.
#[test]
fn p_ordering() {
    use fdk_aac_rs_cref::drc_drc_dec_selection_process_tables as t;
    for (msr, row) in P_ORDERING.iter().enumerate() {
        for (ms, &v) in row.iter().enumerate() {
            let mut ret = -1;
            let o = unsafe { t::cref_drc_sel_get_method_value(msr as i32, ms as i32, &mut ret) };
            assert_eq!(ret, 0);
            assert_eq!(o, v, "pOrdering[{msr}][{ms}]");
        }
    }
    // Indices outside of the table are rejected.
    let mut ret = 0;
    unsafe { t::cref_drc_sel_get_method_value(11, 0, &mut ret) };
    assert_ne!(ret, 0);
    unsafe { t::cref_drc_sel_get_method_value(0, 12, &mut ret) };
    assert_ne!(ret, 0);
}

/// SECOND_TRY (`secondTry`, function-local in `_selectDrcCharacteristic()`), element by element
/// through the function of the copy: a DRC set with the CICP characteristic `k` stays selected
/// for the request `r` (1..=11) if and only if `k` is `r`, `secondTry[r]`, or the third try
/// (`r - 1` for `r` in 2..=5, 9 for `r` == 11). `secondTry[0]` is never read: a request below 1
/// is an error of the first try.
#[test]
fn second_try() {
    use fdk_aac_rs_cref::drc_drc_dec_selection_process_tables as t;
    for r in 1..=11i32 {
        let third = match r {
            2..=5 => r - 1,
            11 => 9,
            _ => -1,
        };
        for k in 0..=15i32 {
            let mut ret = -1;
            let n = unsafe { t::cref_drc_sel_select_drc_characteristic(r, k, &mut ret) };
            assert_eq!(ret, 0);
            let hit = k == r || k == SECOND_TRY[r as usize] || k == third;
            assert_eq!(n, if hit { 1 } else { 2 }, "secondTry[{r}], characteristic {k}");
        }
    }
}
