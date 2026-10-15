// Differential tests of fdk_aac_rs_aacdec::delay against the C++ reference.
//
// FDK_Delay_Create, FDK_Delay_Apply (sequences of frames on random channels, delays longer and
// shorter than the frame) and FDK_Delay_Destroy, comparing the time data and the delay line.

use fdk_aac_rs_aacdec::delay::*;
use fdk_aac_rs_cref::aacdec_delay as c;
use fdk_aac_rs_cref::testutil::Rng;

fn check_state(cp: *mut core::ffi::c_void, r: &FdkSignalDelay, what: &str) {
    let mut line = vec![0i32; 1 << 17];
    let (mut d, mut n) = (0, 0);
    let present = unsafe { c::cref_delay_state(cp, line.as_mut_ptr(), &mut d, &mut n) };
    assert_eq!((r.delay as i32, r.num_channels as i32), (d, n), "{what}: fields");
    assert_eq!(!r.delay_line.is_empty(), present != 0, "{what}: line allocated");
    let len = r.delay_line.len();
    assert_eq!(&r.delay_line[..], &line[..len], "{what}: delay line");
}

#[test]
fn create_apply_destroy() {
    let mut rng = Rng::new(0xde1a_0001);
    for it in 0..500 {
        let delay = [0, 1, 7, 64, 255, 512, 1023, 1024, 1500, 3000][(rng.next_u32() % 10) as usize];
        let nch = 1 + rng.next_u32() % 8;
        let mut cret = 0;
        let cp = unsafe { c::cref_delay_create(delay, nch as i32, &mut cret) };
        let mut r = FdkSignalDelay::default();
        let rret = fdk_delay_create(&mut r, delay as u16, nch as u8);
        assert_eq!(rret, cret, "FDK_Delay_Create {it}");
        check_state(cp, &r, &format!("create {it}"));
        for frame in 0..20 {
            let fl = [1024u32, 960, 512, 480, 256, 100, 1][(rng.next_u32() % 7) as usize];
            let ch = rng.next_u32() % nch;
            // A delay longer than the frame copies overlapping memory with FDKmemcpy (memcpy):
            // glibc's x86-64 memcpy is memmove, its i386 memcpy depends on the CPU (ifunc).
            // The decoder's delay (USAC residual delay compensation) is shorter than the frame.
            if cfg!(target_arch = "x86") && (fl as i32) < delay {
                continue;
            }
            let time: Vec<i32> = (0..fl).map(|_| rng.i32()).collect();
            let mut ct = time.clone();
            unsafe { c::cref_delay_apply(cp, ct.as_mut_ptr(), fl, ch as i32) };
            let mut rt = time.clone();
            fdk_delay_apply(&mut r, &mut rt, fl, ch as u8);
            assert_eq!(rt, ct, "FDK_Delay_Apply {it}/{frame}");
            check_state(cp, &r, &format!("apply {it}/{frame}"));
        }
        fdk_delay_destroy(&mut r);
        assert_eq!((r.delay, r.num_channels, r.delay_line.len()), (0, 0, 0));
        unsafe { c::cref_delay_destroy(cp) };
    }
}
