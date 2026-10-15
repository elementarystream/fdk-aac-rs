// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_core.h, libFDK/src/FDK_core.cpp.

//! Port of `FDK_core.h`: FDK tools versioning support.
//!
//! The two reference builds differ only in `__TIME__`.

use crate::audio::{fdk_lib_info_lookup, lib_version, lib_version_string, LibInfo, FDK_TOOLS};

/// `FDK_TOOLS_LIB_VL0`
pub const FDK_TOOLS_LIB_VL0: i32 = 3;
/// `FDK_TOOLS_LIB_VL1`
pub const FDK_TOOLS_LIB_VL1: i32 = 1;
/// `FDK_TOOLS_LIB_VL2`
pub const FDK_TOOLS_LIB_VL2: i32 = 0;
/// `FDK_TOOLS_LIB_TITLE`
pub const FDK_TOOLS_LIB_TITLE: &str = "FDK Tools";
/// `FDK_TOOLS_LIB_BUILD_DATE`: `__DATE__` in C, which the port cannot reproduce; the value of
/// `SUPPRESS_BUILD_DATE_INFO` is used (PORTING.md, "Deviations from the reference").
pub const FDK_TOOLS_LIB_BUILD_DATE: &str = "";
/// `FDK_TOOLS_LIB_BUILD_TIME`: `__TIME__` in C, see [`FDK_TOOLS_LIB_BUILD_DATE`].
pub const FDK_TOOLS_LIB_BUILD_TIME: &str = "";

/// `FDK_toolsGetLibInfo(info)`: enters the FDK tools library information into the next free
/// entry of `info` (an array of `FDK_MODULE_LAST` entries). Returns 0 on success and -1 if
/// `info` is `None` (`NULL`), the module is already listed or there is no free entry.
pub fn fdk_tools_get_lib_info(info: Option<&mut [LibInfo]>) -> i32 {
    let Some(info) = info else {
        return -1;
    };

    // search for next free tab
    let i = fdk_lib_info_lookup(info, FDK_TOOLS);
    if i < 0 {
        return -1;
    }

    let info = &mut info[i as usize];

    let v = lib_version(FDK_TOOLS_LIB_VL0, FDK_TOOLS_LIB_VL1, FDK_TOOLS_LIB_VL2);

    info.version = v;
    // LIB_VERSION_STRING: writes the string and its terminating NUL; the rest of the buffer is
    // left as it is.
    lib_version_string(info);

    info.module_id = FDK_TOOLS;
    info.build_date = FDK_TOOLS_LIB_BUILD_DATE;
    info.build_time = FDK_TOOLS_LIB_BUILD_TIME;
    info.title = FDK_TOOLS_LIB_TITLE;
    info.flags = 1;

    0
}
