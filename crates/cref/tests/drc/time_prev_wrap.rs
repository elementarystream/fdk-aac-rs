// A uniDrc gain payload whose 16th node time is exactly 32768, which the SHORT node-time
// storage wraps to -32768. At the following frame, processDrcTime reaches _processNodeSegments
// with timePrev == -32768 (verified through the port's gain-decoder state, which the state
// comparison keeps bit-equal to the reference).
//
// Result (v2.0.3, this test): `FDK_drcDec_ProcessTime` returns DRC_DEC_NOT_OK. In
// `_interpolateDrcGain` (drcGainDec_process.cpp:160), with timePrev == -32768 the segment
// duration `duration = (SHORT)(time - timePrev)` is `(SHORT)(time + 32768)`, which is negative
// for every in-frame segment time (0 <= time <= 32767), so `tGainStep < 0` returns DE_NOT_OK;
// the port behaves identically.
//
// Bitstream syntax follows libDRCdec/src/drcDec_reader.cpp (v0 payloads).

use fdk_aac_rs_drc::drc_dec_gain_decoder::NUM_LNB_FRAMES;
use fdk_aac_rs_drc::drc_dec_lib::{
    DRC_DEC_ALL, DRC_DEC_DRC1, DRC_DEC_EFFECT_TYPE, DRC_DEC_IS_ACTIVE,
    DRC_DEC_LOUDNESS_NORMALIZATION_ON, DRC_DEC_MPEG_D_USAC, DRC_DEC_NOT_OK, DRC_DEC_OK,
};
use fdk_aac_rs_drc::drc_dec_selection_process::DETR_GENERAL_COMPR;

use super::drc_dec_lib::Pair;
use super::drc_dec_reader::{BitWriter, Payload};

/// timeDelta() coding of drcDec_reader.cpp `_decodeTimeDelta` (Z = bits of the prefix-3 mu).
fn put_time_delta(w: &mut BitWriter, d: u32, z: u32) {
    if d <= 1 {
        w.put(0, 2);
    } else if d <= 5 {
        w.put(1, 2);
        w.put(d - 2, 2);
    } else if d <= 13 {
        w.put(2, 2);
        w.put(d - 6, 3);
    } else {
        w.put(3, 2);
        w.put(d - 14, z);
    }
}

/// A minimal uniDrcConfig(): one drcCoefficientsUniDrc (drcLocation 1) with one gain set
/// (GCP_REGULAR, GIT_LINEAR, fullFrame 0, timeAlignment 0, timeDeltaMin 3, one band, gain
/// sequence 0), and one drcInstructionsUniDrc (drcSetId 1, base layout, both channels in
/// gain set 0).
fn directed_uni_drc_config() -> Payload {
    let mut w = BitWriter::new();
    w.put(0, 1); // sampleRatePresent
    w.put(0, 7); // downmixInstructionsCountV0
    w.put(0, 1); // drcDescriptionBasicPresent
    w.put(1, 3); // drcCoefficientsUniDrcCountV0
    w.put(1, 6); // drcInstructionsUniDrcCountV0
    // channelLayout()
    w.put(2, 7); // baseChannelCount
    w.put(0, 1); // layoutSignalingPresent
    // drcCoefficientsUniDrc() v0
    w.put(1, 4); // drcLocation
    w.put(0, 1); // drcFrameSizePresent
    w.put(1, 6); // gainSetCount
    // gainSet() v0
    w.put(0, 2); // gainCodingProfile = GCP_REGULAR
    w.put(1, 1); // gainInterpolationType = GIT_LINEAR (GIT_SPLINE = 0)
    w.put(0, 1); // fullFrame = 0
    w.put(0, 1); // timeAlignment = 0 (timeOffset = -1)
    w.put(1, 1); // timeDeltaMinPresent
    w.put(2, 11); // bsTimeDeltaMin -> timeDeltaMin = 3
    w.put(1, 4); // bandCount = 1
    // band 0: gainSequenceIndex auto-increments to 0 (v0); drcCharacteristic: cicpIndex 0 = absent
    w.put(0, 7);
    // drcInstructionsUniDrc() v0
    w.put(1, 6); // drcSetId = 1
    w.put(1, 4); // drcLocation
    w.put(0, 7); // downmixId[0] = 0 (base layout)
    w.put(0, 1); // additionalDownmixIdPresent
    w.put(0x20, 16); // drcSetEffect = EB_GENERAL_COMPR
    w.put(0, 1); // limiterPeakTargetPresent
    w.put(0, 1); // drcSetTargetLoudnessPresent
    w.put(0, 1); // dependsOnDrcSetPresent
    w.put(0, 1); // noIndependentUse
    // gainSetIndex per channel (channelCount = 2)
    w.put(1, 6); // bsGainSetIndex = 1 -> gainSetIndex[0] = 0
    w.put(1, 1); // repeatSequenceIndex
    w.put(0, 5); // bsRepeatSequenceCount = 0 -> one repeat -> gainSetIndex[1] = 0
    // gainModification() of the single channel group (v0, bandCount 1)
    w.put(0, 1); // gainScalingPresent
    w.put(0, 1); // gainOffsetPresent
    w.put(0, 1); // uniDrcConfigExtPresent
    Payload::new(&w, None)
}

/// A minimal loudnessInfoSet(): no entries.
fn directed_loudness_info_set() -> Payload {
    let mut w = BitWriter::new();
    w.put(0, 6); // loudnessInfoAlbumCountV0
    w.put(0, 6); // loudnessInfoCountV0
    w.put(0, 1); // loudnessInfoSetExtPresent
    Payload::new(&w, None)
}

/// uniDrcGain() for gain sequence 0 with a single node at the end of the frame.
fn directed_gain_simple() -> Payload {
    let mut w = BitWriter::new();
    w.put(0, 1); // drcGainCodingMode = 0 (one node at the frame end)
    w.put(0, 1); // gainInitial: sign
    w.put(0, 8); // gainInitial: magnitude
    w.put(0, 1); // uniDrcGainExtPresent
    Payload::new(&w, None)
}

/// uniDrcGain() for gain sequence 0 with 16 nodes whose times are -1 + 3*sum(deltas) with
/// deltas 3, 728, ..., 728. The last node time is 32768, which the SHORT storage of
/// GAIN_NODE.time wraps to exactly -32768. The intermediate times 2192..30584 become node
/// reservoir entries (time - 2*frameSize); the wrapped node stays in the current frame, so it
/// is the last node of the linear node buffer.
fn directed_gain_wrap() -> Payload {
    let mut w = BitWriter::new();
    let z = 10u32; // _getZ(1024 / 3)
    w.put(1, 1); // drcGainCodingMode = 1
    for _ in 0..15 {
        w.put(0, 1); // nNodes end markers -> nNodes = 16
    }
    w.put(1, 1);
    // GIT_LINEAR: no slope steepness codes
    w.put(0, 1); // frameEndFlag = 0 (gainSet.fullFrame == 0): unclamped accumulated times
    put_time_delta(&mut w, 3, z);
    for _ in 0..15 {
        put_time_delta(&mut w, 728, z);
    }
    // gains (GCP_REGULAR): initial gain 0 dB, then 15 codewords '10' (deltaGain +1)
    w.put(0, 1);
    w.put(0, 8);
    for _ in 0..15 {
        w.put(1, 1);
        w.put(0, 1);
    }
    w.put(0, 1); // uniDrcGainExtPresent
    Payload::new(&w, None)
}

/// `timePrev` of the first segment of the current frame as `_processNodeSegments` computes it
/// from the gain-decoder state (DM_REGULAR_DELAY, delaySamples 0, lnbPointerDiff 0), with the
/// last node time of the previous frame and its node count.
fn time_prev_of_previous_frame(p: &Pair) -> (i16, i16, i32) {
    let d = p.rs.as_deref().unwrap();
    let g = d.h_gain_dec.as_deref().unwrap();
    let lnb_pointer = g.drc_gain_buffers.lnb_pointer;
    let prev = ((lnb_pointer + NUM_LNB_FRAMES as i32 - 1) % NUM_LNB_FRAMES as i32) as usize;
    let lnb = g.drc_gain_buffers.lnb(0); // activeDrcOffset + gainElementForGroup = 0
    let n = lnb.n_nodes[prev];
    let last = lnb.node(prev, n - 1);
    // processDrcTime: nodePrevious.time -= frameSize; _processNodeSegments: timePrev =
    // nodePrevious.time + offset (offset = frameSize); both in SHORT arithmetic.
    let a = (last.time as i32 - g.frame_size) as i16;
    let time_prev = (a as i32 + g.frame_size) as i16;
    (time_prev, last.time, n)
}

/// The directed sequence: four plain frames, the wrapping frame, then the target frame,
/// whose `timePrev` is exactly -32768.
#[test]
fn api_time_prev_minus_32768() {
    let cfg = directed_uni_drc_config();
    let lis = directed_loudness_info_set();
    let gain_simple = directed_gain_simple();
    let gain_wrap = directed_gain_wrap();

    let mut p = Pair::open(DRC_DEC_ALL);
    assert_eq!(p.set_codec_mode(DRC_DEC_MPEG_D_USAC), DRC_DEC_OK);
    // Without loudness information the normalization gain is unbounded, and the pre-selection
    // peak-level check (requirement 9) then eliminates every real DRC set in favor of the
    // virtual "no DRC" set. Run with loudness normalization off, as embedders may, and request
    // general-compression DRC explicitly (with no request the selection prefers the virtual
    // effect-none set).
    assert_eq!(p.set_param(DRC_DEC_LOUDNESS_NORMALIZATION_ON, 0), DRC_DEC_OK);
    assert_eq!(p.set_param(DRC_DEC_EFFECT_TYPE, DETR_GENERAL_COMPR), DRC_DEC_OK);
    assert_eq!(p.init(1024, 48000, 2), DRC_DEC_OK);
    assert_eq!(p.read(0, &cfg), DRC_DEC_OK, "uniDrcConfig");
    assert_eq!(p.read(1, &lis), DRC_DEC_OK, "loudnessInfoSet");
    assert_eq!(p.get_param(DRC_DEC_IS_ACTIVE), 1, "drc set not selected");
    assert_eq!(
        p.rs.as_deref().unwrap().sel_proc_output.selected_drc_set_ids[0],
        1,
        "drcSetId 1 not selected"
    );

    let mut audio = vec![0x1a2b_3c4d; 2 * 1024];
    let mut rets = vec![];
    for frame in 0..6 {
        let payload = if frame == 4 { &gain_wrap } else { &gain_simple };
        assert_eq!(p.read(2, payload), DRC_DEC_OK, "frame {frame}");
        assert_eq!(p.preprocess(), DRC_DEC_OK, "frame {frame}");
        let (time_prev, last_time, n) = time_prev_of_previous_frame(&p);
        if frame == 5 {
            assert_eq!(n, 16, "wrapped frame node count");
            assert_eq!(last_time, -32768, "wrapped node time");
            assert_eq!(time_prev, -32768, "timePrev at the target frame");
        }
        let guard_before = p.guard_writes;
        let r = p.process_time(0, DRC_DEC_DRC1, 0, 0, 2, &mut audio, 1024);
        rets.push(r);
        if frame == 5 {
            eprintln!(
                "frame 5: timePrev={time_prev} lastNodeTime={last_time} \
                 processTime ret={r} guardWrites={}",
                p.guard_writes - guard_before
            );
        }
    }
    eprintln!("processTime returns: {rets:?}, total guard writes: {}", p.guard_writes);
    // Measured (v2.0.3, this test): timePrev == -32768 is reached, and the reference's
    // _interpolateDrcGain returns DE_NOT_OK: duration = (SHORT)(time - timePrev) =
    // (SHORT)(time + 32768) is negative for every in-frame segment time. The port behaves
    // identically (checked bit-exactly after every call above).
    assert_eq!(rets[5], DRC_DEC_NOT_OK, "expected the documented DE_NOT_OK path");
    assert_eq!(p.guard_writes, 0, "no writes outside the audio buffer");
}
