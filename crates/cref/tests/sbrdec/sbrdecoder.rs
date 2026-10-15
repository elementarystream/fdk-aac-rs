// Differential tests of fdk_aac_rs_sbrdec::sbrdecoder against the C++ reference.
//
// The API (sbrDecoder_Open, InitElement, Header, SetParam, drcFeedChannel, drcDisable, Parse,
// Apply, FreeMem, Close, GetDelay, GetLibInfo) is compared call by call with the complete
// decoder state: with real streams through a capture of the AAC decoder in e2e.rs, and with
// random call sequences in api_fuzz.rs.
