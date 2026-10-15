// Differential tests of fdk_aac_rs_aacdec::pulsedata against the C++ reference.
//
// CPulseData_Read on random bitstreams (long and short blocks, every start band and frame
// length) and CPulseData_Apply on the data it accepts.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use crate::block_ctx::{random_bytes, CONFIGS};
use fdk_aac_rs_aacdec::channelinfo::*;
use fdk_aac_rs_aacdec::pulsedata::*;
use fdk_aac_rs_cref::aacdec_pulsedata as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_init_bit_stream, FdkBitStream, BS_READER};

fn pd_to(p: &CPulseData) -> [i32; 11] {
    let mut v = [0; 11];
    v[0] = p.pulse_data_present as i32;
    v[1] = p.number_pulse as i32;
    v[2] = p.pulse_start_band as i32;
    for i in 0..4 {
        v[3 + i] = p.pulse_offset[i] as i32;
        v[7 + i] = p.pulse_amp[i] as i32;
    }
    v
}

fn pd_from(v: &[i32; 11]) -> CPulseData {
    let mut p = CPulseData::default();
    p.pulse_data_present = v[0] as u8;
    p.number_pulse = v[1] as u8;
    p.pulse_start_band = v[2] as u8;
    for i in 0..4 {
        p.pulse_offset[i] = v[3 + i] as u8;
        p.pulse_amp[i] = v[7 + i] as u8;
    }
    p
}

#[test]
fn read_and_apply() {
    let mut rng = Rng::new(0x9015_0001);
    let mut accepted = 0;
    for it in 0..40000 {
        let (spf, sri) = CONFIGS[(rng.next_u32() % CONFIGS.len() as u32) as usize];
        let mut t = SamplingRateInfo::default();
        get_sampling_rate_info(&mut t, spf, sri, 48000);
        let sfb = t.scale_factor_bands_long;
        let max_sfb = rng.range(0, t.number_of_scale_factor_bands_long as i32);
        let ws = (rng.next_u32() % 4) as i32;
        let buf = random_bytes(&mut rng, 16);
        let mut cbuf = buf.clone();
        let init: [i32; 11] = std::array::from_fn(|_| (rng.next_u32() % 256) as i32);
        let frame_length = [spf as i32, spf as i32 / 2, 1024, 0][(rng.next_u32() % 4) as usize];

        let mut cpd = init;
        let mut cleft = 0;
        let ce = unsafe {
            c::cref_pulsedata_read(cbuf.as_mut_ptr(), 16, cpd.as_mut_ptr(), sfb.as_ptr(), max_sfb, ws, frame_length, &mut cleft)
        };
        let mut bs = FdkBitStream::default();
        // SAFETY: buf outlives bs.
        fdk_init_bit_stream(&mut bs, &BitMem::from_slice(&buf), 16, 128, BS_READER);
        let mut rpd = pd_from(&init);
        let mut ics = CIcsInfo::default();
        ics.max_sf_bands = max_sfb as u8;
        ics.window_sequence = ws;
        let re = cpulse_data_read(&mut bs, &mut rpd, sfb, &ics, frame_length as i16);
        assert_eq!(re, ce, "CPulseData_Read {it}");
        assert_eq!(pd_to(&rpd), cpd, "CPulseData_Read {it}: data");
        assert_eq!(fdk_get_valid_bits(&mut bs) as i32, cleft, "CPulseData_Read {it}: bits");

        if re == 0 && frame_length > 0 {
            accepted += 1;
            let spec: Vec<i32> = (0..1024).map(|_| rng.range(-20, 20)).collect();
            let mut cs = spec.clone();
            unsafe { c::cref_pulsedata_apply(cpd.as_ptr(), sfb.as_ptr(), cs.as_mut_ptr()) };
            let mut rs = spec.clone();
            cpulse_data_apply(&rpd, sfb, &mut rs);
            assert_eq!(rs, cs, "CPulseData_Apply {it}");
        }
    }
    assert!(accepted > 1000);
}
