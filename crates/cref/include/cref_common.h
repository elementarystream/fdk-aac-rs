/* Common include of the shims in crates/cref/shim/. Shims define extern "C" functions named
   cref_*, one definition per line start (`RET cref_name(ARGS) {`), with scalar parameter types
   (INT, FIXP_DBL, SHORT, UCHAR, ...) or pointers; build.rs generates the Rust declarations.
   Internal headers of a library are reachable as "libX/src/foo.h". */
#ifndef FDK_AAC_RS_CREF_COMMON_H
#define FDK_AAC_RS_CREF_COMMON_H
#include <stdint.h>
#include <string.h>
/* The C library headers that the reference includes, included here at global scope first: shims
   that include a reference .cpp file inside a namespace would otherwise pull them into the
   namespace (with glibc and libstdc++, <math.h> declares the std:: overloads in the global
   namespace, which then fails inside a namespace). */
#include <math.h>
#include <cmath>
#include <stdlib.h>
#include <stdio.h>
#include <assert.h>
#include "machine_type.h"
#include "genericStds.h"
#endif
