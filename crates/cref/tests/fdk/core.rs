// Differential tests of fdk_aac_rs_fdk::core against the C++ reference.

use fdk_aac_rs_cref::fdk_core as c;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::audio::{LibInfo, FDK_MODULE_LAST, FDK_TOOLS};
use fdk_aac_rs_fdk::core::*;

const N: usize = FDK_MODULE_LAST as usize;

/// FDK_toolsGetLibInfo() on both sides, starting from the given module ids.
fn check(ids_in: &[i32; N], null_info: bool) {
    let mut ids = [0i32; N];
    let mut version = [0i32; N];
    let mut flags = [0u32; N];
    let mut vstr = [0i8; 32 * N];
    let mut title = [0i8; 32 * N];
    let e = unsafe {
        c::cref_core_get_lib_info(
            null_info as i32,
            ids_in.as_ptr(),
            ids.as_mut_ptr(),
            version.as_mut_ptr(),
            flags.as_mut_ptr(),
            vstr.as_mut_ptr(),
            title.as_mut_ptr(),
        )
    };

    let mut info = [LibInfo::default(); N];
    for (i, e) in info.iter_mut().enumerate() {
        e.module_id = ids_in[i];
        e.version_str = [b'x'; 32];
    }
    let r = fdk_tools_get_lib_info(if null_info { None } else { Some(&mut info) });
    assert_eq!(r, e, "FDK_toolsGetLibInfo result for {ids_in:?}");
    for i in 0..N {
        let what = format!("entry {i} for {ids_in:?}");
        assert_eq!(info[i].module_id, ids[i], "{what}: module_id");
        assert_eq!(info[i].version, version[i], "{what}: version");
        assert_eq!(info[i].flags, flags[i], "{what}: flags");
        let cv: Vec<u8> = vstr[32 * i..32 * i + 32].iter().map(|&b| b as u8).collect();
        assert_eq!(info[i].version_str.as_slice(), cv.as_slice(), "{what}: versionStr");
        let ct: Vec<u8> =
            title[32 * i..32 * i + 32].iter().map(|&b| b as u8).take_while(|&b| b != 0).collect();
        assert_eq!(info[i].title.as_bytes(), ct.as_slice(), "{what}: title");
    }
}

#[test]
fn module_ids() {
    assert_eq!(FDK_MODULE_LAST, unsafe { c::cref_core_fdk_module_last() });
    assert_eq!(FDK_TOOLS, unsafe { c::cref_core_fdk_tools() });
}

#[test]
fn fdk_tools_get_lib_info_() {
    // NULL
    check(&[0; N], true);
    // empty list
    check(&[0; N], false);
    // full list without FDK_TOOLS
    check(&[3; N], false);
    // FDK_TOOLS already listed
    let mut ids = [0; N];
    ids[0] = 3;
    ids[1] = FDK_TOOLS;
    check(&ids, false);
    // random prefixes of other modules
    let mut rng = Rng::new(401);
    for _ in 0..2000 {
        let mut ids = [0i32; N];
        let n = (rng.next_u32() as usize) % (N + 1);
        for id in ids.iter_mut().take(n) {
            *id = 1 + (rng.next_u32() % 40) as i32;
        }
        if rng.next_u32() % 2 == 0 {
            // ids after a free entry
            for id in ids.iter_mut().skip(n + 1) {
                *id = (rng.next_u32() % 40) as i32;
            }
        }
        check(&ids, false);
    }
}
