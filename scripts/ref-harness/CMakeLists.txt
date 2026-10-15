# Reference harness programs (PORTING.md, "Reference harness"), linked against a reference build
# of fdk-aac.
# Built by scripts/setup_reference.sh into work/build-ref/<build>/harness/:
#
#   cmake -S scripts/ref-harness -B <dir> -DCMAKE_BUILD_TYPE=Release \
#     -DFDK_AAC_SRC=work/fdk-aac-v2.0.3 -DFDK_AAC_LIB=work/build-ref/<build>/libfdk-aac.a
#
# The harness code does not compute codec values, so its own optimization level does not matter.
cmake_minimum_required(VERSION 3.13)
project(fdk_aac_rs_ref_harness C CXX)

set(FDK_AAC_SRC "" CACHE PATH "fdk-aac source tree (v2.0.3)")
set(FDK_AAC_LIB "" CACHE FILEPATH "static libfdk-aac.a of the reference build")
if(NOT FDK_AAC_SRC OR NOT FDK_AAC_LIB)
  message(FATAL_ERROR "set FDK_AAC_SRC and FDK_AAC_LIB")
endif()

set(CMAKE_C_STANDARD 99)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()
set(CMAKE_C_FLAGS_RELEASE "-O2")

add_library(harness_common STATIC common.c mp4demux.c)
target_compile_options(harness_common PRIVATE -Wall -Wextra)

foreach(prog fdkdec-ref fdkenc-ref)
  add_executable(${prog} ${prog}.c)
  target_include_directories(${prog} PRIVATE
    ${FDK_AAC_SRC}/libAACdec/include
    ${FDK_AAC_SRC}/libAACenc/include
    ${FDK_AAC_SRC}/libSYS/include)
  target_compile_options(${prog} PRIVATE -Wall -Wextra -Wno-unused-function)
  target_link_libraries(${prog} PRIVATE harness_common ${FDK_AAC_LIB} m)
  # The library is C++ (built with -fno-exceptions -fno-rtti); link with the C++ driver so that
  # its runtime is pulled in.
  set_target_properties(${prog} PROPERTIES LINKER_LANGUAGE CXX)
endforeach()
