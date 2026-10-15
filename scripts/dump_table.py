#!/usr/bin/env python3
"""Dump constant tables of the C++ reference as Rust array literals.

The C++ sources define their tables with macros (FL2FXCONST_*, STC, WTC, ...). Instead of
re-implementing those macros, this tool compiles a small program that #includes the C++ source
file (so that file-static tables are visible) and prints the raw values with the exact compiler
semantics of the reference build. Tables are never retyped (PORTING.md, "Reference material").

The program is compiled for every architecture variant the host can build (arm64 and x86_64 on
Apple silicon); the tool fails if the variants give different values, because the port keeps one
copy of each table.

Usage:
  scripts/dump_table.py SOURCE SYMBOL ELEMTYPE [--rust-type T] [--name NAME] [--hex] [--per-line N]
                      [--count EXPR] [--prelude CODE] [--static] [--arch ARCH]

  SOURCE    path of a .cpp/.h file relative to the C++ source root
            (e.g. libFDK/src/FDK_tools_rom.cpp)
  SYMBOL    C expression of the table (e.g. SineTable512 or &pow2Coeff[0])
  ELEMTYPE  scalar C type used to reinterpret the table memory (e.g. SHORT, LONG, UCHAR).
            Struct tables (e.g. FIXP_SPK) are dumped as their scalar fields in memory order.
  --count   number of ELEMTYPE elements (default: sizeof(SYMBOL)/sizeof(ELEMTYPE))
  --static  emit `pub static` (the default)
  --const   emit `pub const` instead of `pub static`
  --arch    compile only for this architecture (arm64, x86_64)

Environment:
  FDK_AAC_RS_C_DIR   C++ source checkout (default: <repo>/work/fdk-aac-v2.0.3)
"""
import argparse, glob, os, platform, subprocess, sys, tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def find_src():
    if os.environ.get('FDK_AAC_RS_C_DIR'):
        return os.environ['FDK_AAC_RS_C_DIR']
    # Worktrees use the reference of the main checkout.
    try:
        common = subprocess.run(['git', '-C', REPO, 'rev-parse', '--path-format=absolute',
                                 '--git-common-dir'], capture_output=True, text=True,
                                check=True).stdout.strip()
        cand = os.path.join(os.path.dirname(common), 'work', 'fdk-aac-v2.0.3')
        if os.path.isdir(cand):
            return cand
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
    return os.path.join(REPO, 'work', 'fdk-aac-v2.0.3')


CDIR = find_src()

RUST = {'SHORT': 'i16', 'FIXP_SGL': 'i16', 'USHORT': 'u16', 'LONG': 'i32', 'INT': 'i32',
        'FIXP_DBL': 'i32', 'UINT': 'u32', 'ULONG': 'u32', 'UCHAR': 'u8', 'SCHAR': 'i8',
        'char': 'i8', 'INT64': 'i64', 'UINT64': 'u64', 'float': 'f32', 'double': 'f64'}

# Include order of upstream's CMakeLists.txt, then the private source directories.
INCLUDES = ['libAACdec/include', 'libAACenc/include', 'libSYS/include', 'libArithCoding/include',
            'libDRCdec/include', 'libSACdec/include', 'libSACenc/include', 'libSBRdec/include',
            'libSBRenc/include', 'libMpegTPDec/include', 'libMpegTPEnc/include',
            'libFDK/include', 'libPCMutils/include']


def include_dirs():
    dirs = [os.path.join(CDIR, d) for d in INCLUDES]
    dirs += sorted(glob.glob(os.path.join(CDIR, 'lib*', 'src')))
    return dirs


def archs(requested):
    if requested:
        return [requested]
    if platform.system() == 'Darwin':
        return ['arm64', 'x86_64']
    return [None]


def dump(prog, arch):
    with tempfile.TemporaryDirectory() as td:
        cpp = os.path.join(td, 'dump.cpp')
        exe = os.path.join(td, 'dump')
        open(cpp, 'w').write(prog)
        cmd = ['clang++', '-O0', '-w', '-fno-exceptions', '-fno-rtti']
        if platform.system() == 'Darwin':
            cmd += ['-Wl,-dead_strip']
        else:
            cmd += ['-ffunction-sections', '-fdata-sections', '-Wl,--gc-sections',
                    '-Wl,--unresolved-symbols=ignore-all']
        if arch:
            cmd += ['-arch', arch]
        cmd += ['-I' + d for d in include_dirs()] + [cpp, '-o', exe]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            sys.stderr.write(r.stderr)
            sys.exit(1)
        run = [exe] if not arch or arch == platform.machine() else ['arch', '-' + arch, exe]
        return subprocess.run(run, capture_output=True, text=True, check=True).stdout.split()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('source'); ap.add_argument('symbol'); ap.add_argument('elemtype')
    ap.add_argument('--rust-type'); ap.add_argument('--name'); ap.add_argument('--hex', action='store_true')
    ap.add_argument('--per-line', type=int, default=8); ap.add_argument('--count')
    ap.add_argument('--prelude', default='')
    ap.add_argument('--const', action='store_true'); ap.add_argument('--static', action='store_true')
    ap.add_argument('--arch')
    a = ap.parse_args()
    src = os.path.join(CDIR, a.source) if not os.path.isabs(a.source) else a.source
    count = a.count or f'(sizeof({a.symbol}) / sizeof({a.elemtype}))'
    isfloat = a.elemtype in ('float', 'double')
    sym = a.symbol.lstrip('&')
    prog = f'''#include "{src}"
#include <stdio.h>
{a.prelude}
int main() {{
  const {a.elemtype}* p = (const {a.elemtype}*)(const void*)(&{sym});
  long n = (long)({count});
  printf("%ld\\n", n);
  for (long i = 0; i < n; i++) {{
    {'printf("%.17g\\n", (double)p[i]);' if isfloat else 'printf("%lld\\n", (long long)p[i]);'}
  }}
  return 0;
}}
'''
    results = {arch: dump(prog, arch) for arch in archs(a.arch)}
    first = next(iter(results.values()))
    for arch, out in results.items():
        if out != first:
            sys.exit(f'error: {a.symbol} differs between architectures ({", ".join(map(str, results))})')
    n = int(first[0])
    vals = first[1:1 + n]
    rt = a.rust_type or RUST.get(a.elemtype, a.elemtype)

    def fmt(v):
        if isfloat:
            return v if ('.' in v or 'e' in v or 'n' in v) else v + '.0'
        iv = int(v)
        if a.hex:
            bits = {'i8': 8, 'u8': 8, 'i16': 16, 'u16': 16, 'i32': 32, 'u32': 32, 'i64': 64, 'u64': 64}[rt]
            h = f'0x{iv & ((1 << bits) - 1):0{bits // 4}X}'
            if rt.startswith('i') and iv < 0:
                return f'{h}u{bits} as {rt}'
            return h
        return str(iv)

    items = [fmt(v) for v in vals]
    lines = [', '.join(items[i:i + a.per_line]) + ',' for i in range(0, len(items), a.per_line)]
    name = a.name or sym.split('[')[0]
    kw = 'const' if a.const else 'static'
    print(f'pub {kw} {name}: [{rt}; {n}] = [')
    for l in lines:
        print('    ' + l)
    print('];')


if __name__ == '__main__':
    main()
