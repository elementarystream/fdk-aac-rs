// The decoder through the stubbed reference (shim aacdec_aacdecoder_stub.cpp): the reference's
// glue (aacdecoder.cpp, aacdecoder_lib.cpp, aacdec_drc.cpp) compiled with a hook that repeats
// every read of a coupling channel element on dummy channels filled with different byte
// patterns. The shim's SBR and MPEG Surround decoders are stubs; they are not observable for
// AAC-LC streams without SBR, so the port is compared with the stubbed reference on those
// streams only. SBR, PS and MPEG Surround streams are compared with the reference library in
// aacdecoder_lib.rs.

use fdk_aac_rs_cref::aacdec_aacdecoder_stub as s;
use fdk_aac_rs_fdk::audio::*;

use crate::aacdecoder_lib::*;

fn run_mp4_stub(rel: &str, max_dec: usize) -> bool {
    let Some((asc, aus)) = demux(rel) else {
        return false;
    };
    let mut pair = Pair::open_with(STUB, TT_MP4_RAW, 1, rel).unwrap();
    pair.config_raw(&asc);
    let refs: Vec<&[u8]> = aus.iter().map(|a| a.as_slice()).collect();
    run_packets(&mut pair, &refs, max_dec, |_| 0);
    true
}

#[test]
fn stub_lc_streams_and_cce() {
    // The AAC-LC conformance streams through the stubbed reference (where the stubs are not
    // observable it equals the library), and the check of the CCE dummy channel.
    let files = conformance_mp4(|n| {
        (n.starts_with("al") && n.as_bytes().get(2).is_some_and(|c| c.is_ascii_digit()) && !n.contains("sbr"))
            || n.starts_with("er_al")
    });
    let mut ran = 0;
    for f in &files {
        if run_mp4_stub(f, 50) {
            ran += 1;
        }
    }
    let (mut reads, mut dep) = (0, 0);
    unsafe { s::cref_aacdecoder_stub_cce_stats(&mut reads, &mut dep) };
    eprintln!(
        "stub_lc_streams_and_cce: {ran} of {} streams; CCE reads {reads}, pattern dependent {dep}",
        files.len()
    );
    assert_eq!(dep, 0, "a CCE read depends on the contents of the dummy channel");
}
