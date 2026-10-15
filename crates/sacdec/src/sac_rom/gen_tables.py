#!/usr/bin/env python3
"""Generate the constant tables of fdk-aac-rs-sacdec from the C++ reference.

Writes
  crates/sacdec/src/sac_rom/tables.rs             (libSACdec/src/sac_rom.cpp)
  crates/sacdec/src/sac_calc_m1_and_m2/tables.rs  (file-static tables of sac_calcM1andM2.cpp)
  crates/sacdec/src/sac_tsd/tables.rs             (file-static tables of sac_tsd.cpp)
  crates/sacdec/src/sac_process/tables.rs         (function-local tables of sac_process.cpp)
  crates/sacdec/src/sac_dec_lib/tables.rs         (function-local tables of sac_dec_lib.cpp)

Every table of scalars is dumped with scripts/dump_table.py, which compiles the source for arm64
and x86_64 and fails if the two builds give different values. The dimensions are printed by a
small program compiled against the source (`sizeof`) on both architectures. Struct tables
(treePropertyTable, phiTsd, decConfig) are dumped as their scalar fields in memory order and
written as Rust struct literals. Function-local static tables cannot be named from outside of
their function: their declaration is extracted from the C++ source and compiled at file scope
under the name `<name>_fnlocal` (dump_table.py --prelude, see the command at each table).
Nothing is retyped.

Usage (from the repository root; needs the reference sources, see scripts/setup_reference.sh):
  python3 crates/sacdec/src/sac_rom/gen_tables.py [-j JOBS]
"""
import argparse
import concurrent.futures
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, '..', '..', '..', '..'))
sys.path.insert(0, os.path.join(REPO, 'scripts'))
sys.dont_write_bytecode = True  # no __pycache__ in scripts/
import dump_table  # noqa: E402

LICENSE = '''// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
'''

ROM = 'libSACdec/src/sac_rom.cpp'
CALC = 'libSACdec/src/sac_calcM1andM2.cpp'
TSD = 'libSACdec/src/sac_tsd.cpp'
PROC = 'libSACdec/src/sac_process.cpp'
LIB = 'libSACdec/src/sac_dec_lib.cpp'

# (source, C symbol, dump type, Rust name, Rust element type, format)
TABLES = {
    ROM: [
        ('dequantCPC__FDK', 'LONG', 'DEQUANT_CPC__FDK', 'FixpDbl', 'dbl'),
        ('dequantICC__FDK', 'LONG', 'DEQUANT_ICC__FDK', 'FixpDbl', 'dbl'),
        ('dequantCLD__FDK', 'LONG', 'DEQUANT_CLD__FDK', 'FixpDbl', 'dbl'),
        ('dequantIPD__FDK', 'LONG', 'DEQUANT_IPD__FDK', 'FixpDbl', 'dbl'),
        ('dequantIPD_CLD_ICC_splitAngle__FDK', 'LONG', 'DEQUANT_IPD_CLD_ICC_SPLIT_ANGLE__FDK',
         'FixpDbl', 'dbl'),
        ('dequantCLD_c_l', 'LONG', 'DEQUANT_CLD_C_L', 'FixpDbl', 'dbl'),
        ('H11_nc', 'LONG', 'H11_NC', 'FixpDbl', 'dbl'),
        ('H12_nc', 'LONG', 'H12_NC', 'FixpDbl', 'dbl'),
        ('dequantCLD_c1', 'LONG', 'DEQUANT_CLD_C1', 'FixpDbl', 'dbl'),
        ('BP__FDK', 'LONG', 'BP__FDK', 'FixpDbl', 'dbl'),
        ('BP_GF__FDK', 'LONG', 'BP_GF__FDK', 'FixpDbl', 'dbl'),
        ('samplingFreqTable', 'INT', 'SAMPLING_FREQ_TABLE', 'i32', 'dec'),
        ('freqResTable', 'UCHAR', 'FREQ_RES_TABLE', 'u8', 'dec'),
        ('freqResTable_LD', 'UCHAR', 'FREQ_RES_TABLE_LD', 'u8', 'dec'),
        ('tempShapeChanTable', 'UCHAR', 'TEMP_SHAPE_CHAN_TABLE', 'u8', 'dec'),
        ('kernels_4_to_71', 'SCHAR', 'KERNELS_4_TO_71', 'i8', 'dec'),
        ('kernels_5_to_71', 'SCHAR', 'KERNELS_5_TO_71', 'i8', 'dec'),
        ('kernels_7_to_71', 'SCHAR', 'KERNELS_7_TO_71', 'i8', 'dec'),
        ('kernels_10_to_71', 'SCHAR', 'KERNELS_10_TO_71', 'i8', 'dec'),
        ('kernels_14_to_71', 'SCHAR', 'KERNELS_14_TO_71', 'i8', 'dec'),
        ('kernels_20_to_71', 'SCHAR', 'KERNELS_20_TO_71', 'i8', 'dec'),
        ('kernels_28_to_71', 'SCHAR', 'KERNELS_28_TO_71', 'i8', 'dec'),
        ('kernels_4_to_64', 'SCHAR', 'KERNELS_4_TO_64', 'i8', 'dec'),
        ('kernels_5_to_64', 'SCHAR', 'KERNELS_5_TO_64', 'i8', 'dec'),
        ('kernels_7_to_64', 'SCHAR', 'KERNELS_7_TO_64', 'i8', 'dec'),
        ('kernels_9_to_64', 'SCHAR', 'KERNELS_9_TO_64', 'i8', 'dec'),
        ('kernels_12_to_64', 'SCHAR', 'KERNELS_12_TO_64', 'i8', 'dec'),
        ('kernels_15_to_64', 'SCHAR', 'KERNELS_15_TO_64', 'i8', 'dec'),
        ('kernels_23_to_64', 'SCHAR', 'KERNELS_23_TO_64', 'i8', 'dec'),
        ('mapping_4_to_28', None, None, None, None),
        ('mapping_15_to_23', 'UCHAR', 'MAPPING_15_TO_23', 'u8', 'dec'),
        ('mapping_12_to_23', 'UCHAR', 'MAPPING_12_TO_23', 'u8', 'dec'),
        ('mapping_9_to_23', 'UCHAR', 'MAPPING_9_TO_23', 'u8', 'dec'),
        ('mapping_7_to_23', 'UCHAR', 'MAPPING_7_TO_23', 'u8', 'dec'),
        ('mapping_5_to_23', 'UCHAR', 'MAPPING_5_TO_23', 'u8', 'dec'),
        ('mapping_4_to_23', 'UCHAR', 'MAPPING_4_TO_23', 'u8', 'dec'),
        ('clipGainTable__FDK', 'LONG', 'CLIP_GAIN_TABLE__FDK', 'FixpDbl', 'dbl'),
        ('clipGainSFTable__FDK', 'UCHAR', 'CLIP_GAIN_SF_TABLE__FDK', 'u8', 'dec'),
        ('pbStrideTable', 'UCHAR', 'PB_STRIDE_TABLE', 'u8', 'dec'),
        ('smgTimeTable', 'INT', 'SMG_TIME_TABLE', 'i32', 'dec'),
        ('envShapeDataTable__FDK', 'LONG', 'ENV_SHAPE_DATA_TABLE__FDK', 'FixpDbl', 'dbl'),
        ('row2channelSTP', 'SCHAR', 'ROW2CHANNEL_STP', 'i8', 'dec'),
        ('row2channelGES', 'SCHAR', 'ROW2CHANNEL_GES', 'i8', 'dec'),
        ('row2residual', 'SCHAR', 'ROW2RESIDUAL', 'i8', 'dec'),
    ],
    CALC: [
        ('sinIpd_tab', 'LONG', 'SIN_IPD_TAB', 'FixpDbl', 'dbl'),
        ('sqrt_one_minus_ICC2', 'SHORT', 'SQRT_ONE_MINUS_ICC2', 'FixpSgl', 'dec'),
        ('sqrt_CLD_e', 'SCHAR', 'SQRT_CLD_E', 'i8', 'dec'),
        ('sqrt_CLD_m', 'LONG', 'SQRT_CLD_M', 'FixpDbl', 'dbl'),
        ('CLD_m', 'LONG', 'CLD_M', 'FixpDbl', 'dbl'),
    ],
    TSD: [
        ('nBitsTsdCW_32slots', 'UCHAR', 'N_BITS_TSD_CW_32SLOTS', 'u8', 'dec'),
        ('nBitsTsdCW_64slots', 'UCHAR', 'N_BITS_TSD_CW_64SLOTS', 'u8', 'dec'),
    ],
}

# The mapping_*_to_28 tables are declared in sac_rom.h but not defined: they are skipped.
TABLES[ROM] = [t for t in TABLES[ROM] if t[1] is not None]


def dump(src, symbol, elemtype):
    cmd = [sys.executable, os.path.join(REPO, 'scripts', 'dump_table.py'), src, symbol, elemtype]
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO)
    if r.returncode != 0:
        sys.exit(f'dump_table.py failed for {symbol}:\n{r.stderr}')
    body = r.stdout.split('=', 1)[1]
    return [int(v) for v in re.findall(r'-?\d+', body)]


def fnlocal_decl(src, name):
    """The declaration of the function-local `static const` table `name` of `src`, renamed to
    `<name>_fnlocal` (for dump_table.py --prelude)."""
    text = open(os.path.join(dump_table.CDIR, src)).read()
    pat = (r'static\s+const\s+([A-Za-z_][A-Za-z_0-9]*)\s+' + re.escape(name) +
           r'\s*((?:\[[^\]]*\])*)\s*=\s*(\{.*?\});')
    m = re.search(pat, text, re.S)
    if not m:
        sys.exit(f'{name} not found in {src}')
    init = re.sub(r'\s+', ' ', re.sub(r'/\*.*?\*/', '', m.group(3), flags=re.S))
    return f'static const {m.group(1)} {name}_fnlocal{m.group(2)} = {init};'


def dump_fnlocal(src, name, elemtype):
    """Dumps a function-local table with dump_table.py; returns (values, command)."""
    decl = fnlocal_decl(src, name)
    cmd = [sys.executable, os.path.join(REPO, 'scripts', 'dump_table.py'), src, name + '_fnlocal',
           elemtype, '--prelude', decl]
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO)
    if r.returncode != 0:
        sys.exit(f'dump_table.py failed for {name}:\n{r.stderr}')
    body = r.stdout.split('=', 1)[1]
    vals = [int(v) for v in re.findall(r'-?\d+', body)]
    return vals, f"scripts/dump_table.py {src} {name}_fnlocal {elemtype} --prelude '{decl}'"


def run_program(src, body):
    """Compiles `#include src` + body for both architectures and returns the whitespace
    separated output; fails if the architectures differ."""
    prog = f'#include "{os.path.join(dump_table.CDIR, src)}"\n#include <stdio.h>\n{body}\n'
    results = {arch: dump_table.dump(prog, arch) for arch in dump_table.archs(None)}
    first = next(iter(results.values()))
    for arch, out in results.items():
        if out != first:
            sys.exit(f'error: program output differs between architectures ({list(results)})')
    return first


def shapes(src, names):
    """{name: [dims]} from sizeof in C (the rank is found by the program itself)."""
    lines = []
    for name in names:
        lines.append(f'  printf("{name} ");')
        lines.append(f'  DIMS({name});')
    body = '''
#include <type_traits>
template <typename T> void dims_of() {
  if (std::rank<T>::value == 0) { printf("\\n"); return; }
  printf("%ld ", (long)std::extent<T>::value);
  dims_of<typename std::remove_extent<T>::type>();
}
#define DIMS(x) dims_of<std::remove_cv<decltype(x)>::type>()
int main() {
''' + '\n'.join(lines) + '\n  return 0;\n}'
    out = run_program(src, body)
    res = {}
    cur = None
    for tok in out:
        if tok in names:
            cur = tok
            res[cur] = []
        else:
            res[cur].append(int(tok))
    return res


def fmt_value(v, kind):
    if kind == 'dbl':
        return f'0x{v:08X}' if v >= 0 else str(v)
    return str(v)


def nested(items, dims, per_line, indent):
    pad = ' ' * indent
    if len(dims) == 1:
        if len(items) <= per_line:
            return '[' + ', '.join(items) + ']'
        rows = [', '.join(items[i:i + per_line]) + ','
                for i in range(0, len(items), per_line)]
        return '[\n' + ''.join(f'{pad}    {r}\n' for r in rows) + f'{pad}]'
    step = len(items) // dims[0]
    inner = [nested(items[i * step:(i + 1) * step], dims[1:], per_line, indent + 4)
             for i in range(dims[0])]
    if all('\n' not in s for s in inner) and sum(len(s) + 2 for s in inner) < 90 - indent:
        return '[' + ', '.join(inner) + ']'
    return '[\n' + ''.join(f'{pad}    {s},\n' for s in inner) + f'{pad}]'


def rust_type(rtype, dims):
    t = rtype
    for d in reversed(dims):
        t = f'[{t}; {d}]'
    return t


def table_item(src, cname, dtype, rname, rtype, kind, dims, vals):
    per_line = 6 if kind == 'dbl' else 16
    if dims[-1] in (2, 3, 4) and len(dims) > 1:
        per_line = dims[-1]
    total = 1
    for d in dims:
        total *= d
    assert total == len(vals), (cname, dims, len(vals))
    lit = nested([fmt_value(v, kind) for v in vals], dims, per_line, 0)
    return (f'/// `{cname}`\n///\n/// Generated with `scripts/dump_table.py {src} {cname} {dtype}`.\n'
            f'pub static {rname}: {rust_type(rtype, dims)} = {lit};\n')


def scalar_tables(src, specs, jobs):
    shp = shapes(src, [s[0] for s in specs])
    with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
        vals = list(ex.map(lambda s: dump(src, s[0], s[1]), specs))
    return ''.join('\n' + table_item(src, s[0], s[1], s[2], s[3], s[4], shp[s[0]], v)
                   for s, v in zip(specs, vals))


def header(src, doc, uses):
    return LICENSE + f'''//
// Ported from {src}.
//
// GENERATED by `python3 crates/sacdec/src/sac_rom/gen_tables.py`; do not edit. Every table is
// dumped with scripts/dump_table.py (the command is given at each table), which checks that the
// arm64 and x86_64 builds give the same values.

//! {doc}
{uses}'''


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('-j', type=int, default=3)
    a = ap.parse_args()

    # sac_rom.cpp
    out = header(ROM, 'The constant tables of `sac_rom.cpp`.',
                 '\nuse super::TreeProperties;\nuse fdk_aac_rs_fdk::common_fix::FixpDbl;\n')
    out += scalar_tables(ROM, TABLES[ROM], a.j)
    # treePropertyTable: {SCHAR numInputChannels, numOutputChannels, numOttBoxes, numTttBoxes,
    # ottModeLfe[MAX_NUM_OTT]}
    n = int(run_program(ROM, 'int main() { printf("%d %d\\n", (int)(sizeof(treePropertyTable) / '
                             'sizeof(treePropertyTable[0])), (int)sizeof(TREEPROPERTIES)); }')[0])
    vals = dump(ROM, 'treePropertyTable', 'SCHAR')
    assert len(vals) == n * 9, len(vals)
    out += ('\n/// `treePropertyTable`\n///\n/// Generated with `scripts/dump_table.py '
            f'{ROM} treePropertyTable SCHAR` (9 bytes per entry).\n'
            f'pub static TREE_PROPERTY_TABLE: [TreeProperties; {n}] = [\n')
    for i in range(n):
        v = vals[i * 9:(i + 1) * 9]
        out += (f'    TreeProperties {{ num_input_channels: {v[0]}, num_output_channels: {v[1]}, '
                f'num_ott_boxes: {v[2]}, num_ttt_boxes: {v[3]}, ott_mode_lfe: '
                f'[{", ".join(map(str, v[4:9]))}] }},\n')
    out += '];\n'
    os.makedirs(HERE, exist_ok=True)
    open(os.path.join(HERE, 'tables.rs'), 'w').write(out)

    # sac_calcM1andM2.cpp
    out = header(CALC, 'The file-static tables of `sac_calcM1andM2.cpp`.',
                 '\nuse fdk_aac_rs_fdk::common_fix::{FixpDbl, FixpSgl};\n')
    out += scalar_tables(CALC, TABLES[CALC], a.j)
    d = os.path.join(HERE, '..', 'sac_calc_m1_and_m2')
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, 'tables.rs'), 'w').write(out)

    # sac_tsd.cpp
    out = header(TSD, 'The file-static tables of `sac_tsd.cpp`.',
                 '\nuse fdk_aac_rs_fdk::common_fix::FixpDbl;\n')
    out += scalar_tables(TSD, TABLES[TSD], a.j)
    vals = dump(TSD, 'phiTsd', 'LONG')
    assert len(vals) == 16, len(vals)
    out += ('\n/// `phiTsd` (`FIXP_DPK`: real and imaginary part of each entry)\n///\n'
            f'/// Generated with `scripts/dump_table.py {TSD} phiTsd LONG`.\n'
            'pub static PHI_TSD: [[FixpDbl; 2]; 8] = '
            + nested([fmt_value(v, 'dbl') for v in vals], [8, 2], 2, 0) + ';\n')
    d = os.path.join(HERE, '..', 'sac_tsd')
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, 'tables.rs'), 'w').write(out)

    # sac_process.cpp: chanelIdx of mapChannel()
    out = header(PROC, 'The function-local tables of `sac_process.cpp`.', '')
    vals, cmd = dump_fnlocal(PROC, 'chanelIdx', 'UCHAR')
    assert len(vals) % 8 == 0, len(vals)
    rows = len(vals) // 8
    out += ('\n/// `chanelIdx` of `mapChannel()`\n///\n'
            f'/// Generated with `{cmd}`.\n'
            f'pub static CHANEL_IDX: [[u8; 8]; {rows}] = '
            + nested([str(v) for v in vals], [rows, 8], 8, 0) + ';\n')
    d = os.path.join(HERE, '..', 'sac_process')
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, 'tables.rs'), 'w').write(out)

    # sac_dec_lib.cpp: mapIdx2QmfBands of mpegSurroundDecoder_GetNrOfQmfBands(), decConfig of
    # mpegSurroundDecoder_Create()
    out = header(LIB, 'The function-local tables of `sac_dec_lib.cpp`.',
                 '\nuse crate::sac_dec_interface::SpatialDecConfig;\n')
    vals, cmd = dump_fnlocal(LIB, 'mapIdx2QmfBands', 'UCHAR')
    out += ('\n/// `mapIdx2QmfBands` of `mpegSurroundDecoder_GetNrOfQmfBands()`\n///\n'
            f'/// Generated with `{cmd}`.\n'
            f'pub static MAP_IDX2_QMF_BANDS: [u8; {len(vals)}] = '
            + nested([str(v) for v in vals], [len(vals)], 16, 0) + ';\n')
    vals, cmd = dump_fnlocal(LIB, 'decConfig', 'INT')
    assert len(vals) == 4, len(vals)
    out += ('\n/// `decConfig` of `mpegSurroundDecoder_Create()` (decoderLevel, decoderMode,\n'
            '/// maxNumOutputChannels, binauralMode)\n///\n'
            f'/// Generated with `{cmd}`.\n'
            'pub static DEC_CONFIG: SpatialDecConfig = SpatialDecConfig {\n'
            f'    decoder_level: {vals[0]},\n    decoder_mode: {vals[1]},\n'
            f'    max_num_output_channels: {vals[2]},\n    binaural_mode: {vals[3]},\n}};\n')
    d = os.path.join(HERE, '..', 'sac_dec_lib')
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, 'tables.rs'), 'w').write(out)
    print('done')


if __name__ == '__main__':
    main()
