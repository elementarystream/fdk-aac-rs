// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/FDK_delay.h, libAACdec/src/FDK_delay.cpp.

//! Port of `FDK_delay.h`: a signal delay line for several channels.
//!
//! The `.ii` files of the `aarch64` and `x86` builds are identical for `FDK_delay.cpp` apart from
//! the expansion of `FDK_ASSERT`.

use fdk_aac_rs_fdk::fdk_assert;

use crate::aac_rom::PcmDec;

/// `MAX_FRAME_LENGTH`
pub const MAX_FRAME_LENGTH: u32 = 1024;

/// `FDK_SignalDelay`: the delay line state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FdkSignalDelay {
    /// `delay_line`: the allocated delay line (`num_channels * delay` samples; empty for
    /// `NULL`).
    pub delay_line: Vec<PcmDec>,
    /// `delay`: delay required in samples (per channel).
    pub delay: u16,
    /// `num_channels`: number of channels to delay.
    pub num_channels: u8,
}

/// `FDK_Delay_Create(data, delay, num_channels)`: create the delay line (zero-filled). Returns 0
/// (C returns -1 if the allocation fails; the port aborts like any Rust allocation).
pub fn fdk_delay_create(data: &mut FdkSignalDelay, delay: u16, num_channels: u8) -> i32 {
    fdk_assert!(num_channels > 0);

    if delay > 0 {
        data.delay_line = vec![0; num_channels as usize * delay as usize];
    } else {
        data.delay_line = Vec::new();
    }
    data.num_channels = num_channels;
    data.delay = delay;

    0
}

/// `FDK_Delay_Apply(data, time_buffer, frame_length, channel)`: apply the delay to the first
/// `frame_length` samples of `time_buffer` of channel `channel`.
///
/// C overlaps source and destination in one `FDKmemcpy` when `frame_length < delay`
/// (destination before the source); `memcpy` copies forward on both reference platforms (on
/// Apple platforms it is `memmove`), which gives the result of a `memmove`, as here.
///
/// With `channel >= num_channels` or a `time_buffer` shorter than `frame_length` the reference
/// has no defined result; the port changes nothing.
pub fn fdk_delay_apply(
    data: &mut FdkSignalDelay,
    time_buffer: &mut [PcmDec],
    frame_length: u32,
    channel: u8,
) {
    if data.delay > 0 {
        fdk_assert!(frame_length <= MAX_FRAME_LENGTH);
        fdk_assert!(channel < data.num_channels);
        let delay = data.delay as usize;
        let fl = frame_length as usize;
        let line = channel as usize * delay;
        if line + delay > data.delay_line.len() || fl > time_buffer.len() {
            return;
        }
        if fl >= delay {
            let tmp: Vec<PcmDec> = time_buffer[fl - delay..fl].to_vec();
            time_buffer.copy_within(0..fl - delay, delay);
            time_buffer[..delay].copy_from_slice(&data.delay_line[line..line + delay]);
            data.delay_line[line..line + delay].copy_from_slice(&tmp);
        } else {
            let tmp: Vec<PcmDec> = time_buffer[..fl].to_vec();
            time_buffer[..fl].copy_from_slice(&data.delay_line[line..line + fl]);
            data.delay_line.copy_within(line + fl..line + delay, line);
            data.delay_line[line + (delay - fl)..line + delay].copy_from_slice(&tmp);
        }
    }
}

/// `FDK_Delay_Destroy(data)`: free the delay line.
pub fn fdk_delay_destroy(data: &mut FdkSignalDelay) {
    data.delay_line = Vec::new();
    data.delay = 0;
    data.num_channels = 0;
}
