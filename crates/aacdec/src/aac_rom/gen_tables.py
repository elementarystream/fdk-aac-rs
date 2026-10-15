#!/usr/bin/env python3
"""Generate crates/aacdec/src/aac_rom/tables.rs from the C++ reference.

Every value comes from the C++ sources compiled on arm64 and x86_64; nothing is retyped:

* The list of tables (name, element type, rank) is taken from the definitions in
  libAACdec/src/aac_rom.cpp. Each table of scalars is dumped with scripts/dump_table.py, which
  compiles the source for both architectures and fails if they give different values. The
  dimensions of the multi-dimensional tables are printed by a small program compiled against the
  source (`sizeof`), on both architectures.
* The pointer tables (sfbOffsetTables, AACcodeBookDescriptionTable, AACcodeBookDescriptionSCL,
  aHuffTable, aQuantTable) are printed by the same program as indices into the lists of the
  tables they can point to, plus their scalar fields, and written with the Rust names of those
  tables.
* aStateConstant2State (function pointers of aacdec_hcrs.cpp) is written by hand in aac_rom.rs
  and checked by the differential test.

This module is also used by crates/aacdec/src/usacdec_rom/gen_tables.py.

Usage (from the repository root; needs the reference sources, see scripts/setup_reference.sh):
  python3 crates/aacdec/src/aac_rom/gen_tables.py [-j JOBS]
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

SRC = 'libAACdec/src/aac_rom.cpp'

LICENSE = '''// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
'''

# C element type -> (dump type, Rust element type, format)
TYPES = {
    'FIXP_DBL': ('LONG', 'FixpDbl', 'dbl'),
    'FIXP_TCC': ('LONG', 'FixpTcc', 'dbl'),
    'FIXP_FILT': ('LONG', 'FixpFilt', 'dbl'),
    'SCHAR': ('SCHAR', 'i8', 'dec'),
    'SHORT': ('SHORT', 'i16', 'dec'),
    'USHORT': ('USHORT', 'u16', 'hex4'),
    'UINT': ('UINT', 'u32', 'hex6'),
    'UCHAR': ('UCHAR', 'u8', 'dec'),
    'FIXP_WTB': ('SHORT', 'FixpWtb', 'dec'),
    'FIXP_SGL': ('SHORT', 'FixpSgl', 'dec'),
    'FIXP_LPC': ('SHORT', 'FixpLpc', 'dec'),
    'MP4_ELEMENT_ID': ('INT', 'Mp4ElementId', 'elid'),
}

# Names whose mechanical conversion is unreadable.
NAME_OVERRIDES = {
    'AACcodeBookDescriptionTable': 'AAC_CODE_BOOK_DESCRIPTION_TABLE',
    'AACcodeBookDescriptionSCL': 'AAC_CODE_BOOK_DESCRIPTION_SCL',
    'FDKaacDec_tnsCoeff3': 'FDK_AAC_DEC_TNS_COEFF3',
    'FDKaacDec_tnsCoeff4': 'FDK_AAC_DEC_TNS_COEFF4',
    'FDKaacDec_tnsCoeff3_gain_ld': 'FDK_AAC_DEC_TNS_COEFF3_GAIN_LD',
    'FDKaacDec_tnsCoeff4_gain_ld': 'FDK_AAC_DEC_TNS_COEFF4_GAIN_LD',
}


def rust_name(c):
    """Mechanical SCREAMING_SNAKE_CASE name of a C table: InverseQuantTable ->
    INVERSE_QUANT_TABLE, HuffmanCodeBook_1 -> HUFFMAN_CODE_BOOK_1, aHuffTree41 -> A_HUFF_TREE_41
    (digits are separated only in names without underscores, as in tools_rom),
    fdk_dec_Da -> FDK_DEC_DA."""
    if c in NAME_OVERRIDES:
        return NAME_OVERRIDES[c]
    s = re.sub(r'([a-z0-9])([A-Z])', r'\1_\2', c)
    s = re.sub(r'([A-Z]+)([A-Z][a-z])', r'\1_\2', s)
    if '_' not in c:
        s = re.sub(r'([A-Za-z])(\d)', r'\1_\2', s)
    return s.upper().replace('__', '_')


def find_tables(src):
    """[(static, ctype, name, rank, pointer)] of the table definitions of src (C++ text)."""
    text = re.sub(r'/\*.*?\*/', '', src, flags=re.S)
    text = re.sub(r'//[^\n]*', '', text)
    out = []
    for m in re.finditer(r'^(static\s+)?const\s+(\w+)\s*(\*?)\s*(\w+)\s*((?:\[[^\]]*\]\s*)+)=',
                         text, re.M):
        rank = m.group(5).count('[')
        out.append((bool(m.group(1)), m.group(2), m.group(4), rank, m.group(3) == '*'))
    return out


def dump(src, symbol, elemtype):
    cmd = [sys.executable, os.path.join(REPO, 'scripts', 'dump_table.py'), src, symbol, elemtype]
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO)
    if r.returncode != 0:
        sys.exit(f'dump_table.py failed for {symbol}:\n{r.stderr}')
    body = r.stdout.split('=', 1)[1]
    return [int(v) for v in re.findall(r'-?\d+', body)]


def run_program(src, body):
    """Compiles `#include src` + body for both architectures (dump_table.dump) and returns the
    whitespace separated output; fails if the architectures differ."""
    prog = f'#include "{os.path.join(dump_table.CDIR, src)}"\n#include <stdio.h>\n{body}\n'
    results = {arch: dump_table.dump(prog, arch) for arch in dump_table.archs(None)}
    first = next(iter(results.values()))
    for arch, out in results.items():
        if out != first:
            sys.exit(f'error: program output differs between architectures ({list(results)})')
    return first


def shapes(src, tables):
    """{name: [dims]} of the scalar tables, from sizeof in C."""
    lines = []
    for _, _, name, rank, _ in tables:
        expr = name
        dims = []
        for _ in range(rank):
            dims.append(f'(long)(sizeof({expr}) / sizeof({expr}[0]))')
            expr += '[0]'
        lines.append('  printf("' + ' '.join(['%ld'] * rank) + '\\n", ' + ', '.join(dims) + ');')
    out = run_program(src, 'int main() {\n' + '\n'.join(lines) + '\n  return 0;\n}')
    vals = [int(v) for v in out]
    res = {}
    pos = 0
    for _, _, name, rank, _ in tables:
        res[name] = vals[pos:pos + rank]
        pos += rank
    return res


def fmt_value(v, kind, elid=None):
    if kind == 'dbl':
        return f'0x{v:08X}' if v >= 0 else str(v)
    if kind == 'hex4':
        return f'0x{v:04X}'
    if kind == 'hex6':
        return f'0x{v:06X}'
    if kind == 'elid':
        return elid[v]
    return str(v)


def nested(items, dims, per_line, indent):
    """Rust literal of a (possibly multi-dimensional) array from the flat items."""
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


def table_item(src, ctype, name, dims, vals, elid):
    dtype, rtype, kind = TYPES[ctype]
    per_line = {'dbl': 6, 'hex4': 8, 'hex6': 7, 'elid': 7}.get(kind, 12)
    if dims[-1] in (2, 3, 4) and len(dims) > 1:
        per_line = dims[-1]
    total = 1
    for d in dims:
        total *= d
    assert total == len(vals), (name, dims, len(vals))
    items = [fmt_value(v, kind, elid) for v in vals]
    lit = nested(items, dims, per_line, 0)
    return (f'/// `{name}`\n///\n/// Generated with `scripts/dump_table.py {src} {name} {dtype}`.\n'
            f'pub static {rust_name(name)}: {rust_type(rtype, dims)} = {lit};\n')


def element_ids():
    hdr = open(os.path.join(dump_table.CDIR, 'libSYS/include/FDK_audio.h')).read()
    m = re.search(r'typedef enum \{(.*?)\} MP4_ELEMENT_ID;', hdr, re.S)
    ids = {}
    # Only the enumerators with a plain number (ID_NONE .. ID_SCAL); the USAC IDs are written
    # as `n + USAC_ID_BIT` and do not occur in elementsTab (checked: every value must be named).
    for name, val in re.findall(r'(ID_\w+)\s*=\s*(-?\d+)\s*,', m.group(1)):
        ids.setdefault(int(val), name)
    return ids


def scalar_tables(src, a_j):
    """(tables, shapes, values) of the scalar tables of src."""
    text = open(os.path.join(dump_table.CDIR, src)).read()
    tables = [t for t in find_tables(text) if not t[4] and t[1] in TYPES]
    shp = shapes(src, tables)
    with concurrent.futures.ThreadPoolExecutor(a_j) as ex:
        vals = list(ex.map(lambda t: dump(src, t[2], TYPES[t[1]][0]), tables))
    return tables, shp, vals


def pointer_tables():
    """The Rust code of the pointer tables of aac_rom.cpp."""
    text = open(os.path.join(dump_table.CDIR, SRC)).read()
    tables = find_tables(text)
    sfb = [n for _, t, n, _, p in tables if t == 'SHORT' and n.startswith('sfb_') and not p]
    cbs = [n for _, t, n, _, p in tables if n.startswith('HuffmanCodeBook_') and not p]
    trees = [n for _, t, n, _, p in tables if n.startswith('aHuffTree') and t == 'UINT'
             and not p and re.fullmatch(r'aHuffTree\d+', n)]
    vals = [n for _, t, n, _, p in tables if n.startswith('aValTab') and not p]

    def lst(name, items):
        return (f'static const void *const {name}[] = {{' + ', '.join(f'(const void *){i}'
                                                                     for i in items) + '};\n')

    body = lst('sfb', sfb) + lst('cbs', cbs) + lst('trees', trees) + lst('vals', vals) + '''
static int idx(const void *p, const void *const *l, int n) {
  if (p == NULL) return -1;
  for (int i = 0; i < n; i++)
    if (l[i] == p) return i;
  return -2;
}
#define IDX(p, l) idx((const void *)(p), l, (int)(sizeof(l) / sizeof(l[0])))
int main() {
  for (int i = 0; i < 5; i++)
    for (int j = 0; j < 16; j++)
      printf("%d %d %d %d\\n", IDX(sfbOffsetTables[i][j].sfbOffsetLong, sfb),
             IDX(sfbOffsetTables[i][j].sfbOffsetShort, sfb),
             (int)sfbOffsetTables[i][j].numberOfSfbLong, (int)sfbOffsetTables[i][j].numberOfSfbShort);
  for (int i = 0; i < 13; i++)
    printf("%d %d %d %d\\n", IDX(AACcodeBookDescriptionTable[i].CodeBook, cbs),
           (int)AACcodeBookDescriptionTable[i].Dimension, (int)AACcodeBookDescriptionTable[i].numBits,
           (int)AACcodeBookDescriptionTable[i].Offset);
  printf("%d %d %d %d\\n", IDX(AACcodeBookDescriptionSCL.CodeBook, cbs),
         (int)AACcodeBookDescriptionSCL.Dimension, (int)AACcodeBookDescriptionSCL.numBits,
         (int)AACcodeBookDescriptionSCL.Offset);
  printf("%d\\n", (int)(sizeof(aHuffTable) / sizeof(aHuffTable[0])));
  for (int i = 0; i < (int)(sizeof(aHuffTable) / sizeof(aHuffTable[0])); i++)
    printf("%d\\n", IDX(aHuffTable[i], trees));
  printf("%d\\n", (int)(sizeof(aQuantTable) / sizeof(aQuantTable[0])));
  for (int i = 0; i < (int)(sizeof(aQuantTable) / sizeof(aQuantTable[0])); i++)
    printf("%d\\n", IDX(aQuantTable[i], vals));
  return 0;
}
'''
    out = [int(v) for v in run_program(SRC, body)]
    assert all(v >= -1 for v in out), 'a pointer table points outside the candidate lists'
    pos = 0

    def take(n):
        nonlocal pos
        r = out[pos:pos + n]
        pos += n
        return r

    def ref(i, names):
        return '&[]' if i < 0 else f'&{rust_name(names[i])}'

    code = '\n/// `sfbOffsetTables[5][16]`: per frame length (1024, 960, 768, 512, 480) and sampling\n'
    code += '/// rate index. Generated by `gen_tables.py` (pointer targets and fields printed by a\n'
    code += f'/// program compiled against `{SRC}`).\n'
    code += 'pub static SFB_OFFSET_TABLES: [[SfbInfo; 16]; 5] = [\n'
    for i in range(5):
        code += '    [\n'
        for j in range(16):
            lo, sh, nl, ns = take(4)
            code += (f'        SfbInfo {{ sfb_offset_long: {ref(lo, sfb)}, sfb_offset_short: '
                     f'{ref(sh, sfb)}, number_of_sfb_long: {nl}, number_of_sfb_short: {ns} }},\n')
        code += '    ],\n'
    code += '];\n'

    def cbd(v):
        cb, dim, nb, off = v
        return (f'CodeBookDescription {{ code_book: {ref(cb, cbs)}, dimension: {dim}, '
                f'num_bits: {nb}, offset: {off} }}')

    code += ('\n/// `AACcodeBookDescriptionTable[13]`. Generated by `gen_tables.py` (pointer targets\n'
             f'/// and fields printed by a program compiled against `{SRC}`).\n'
             'pub static AAC_CODE_BOOK_DESCRIPTION_TABLE: [CodeBookDescription; 13] = [\n')
    for _ in range(13):
        code += f'    {cbd(take(4))},\n'
    code += '];\n'
    code += ('\n/// `AACcodeBookDescriptionSCL`. Generated by `gen_tables.py` (pointer target and\n'
             f'/// fields printed by a program compiled against `{SRC}`).\n'
             f'pub static AAC_CODE_BOOK_DESCRIPTION_SCL: CodeBookDescription = {cbd(take(4))};\n')
    n = take(1)[0]
    code += ('\n/// `aHuffTable[MAX_CB]`: the decode tree of each codebook. Generated by `gen_tables.py`\n'
             f'/// (pointer targets printed by a program compiled against `{SRC}`).\n'
             f'pub static A_HUFF_TABLE: [&[u32]; {n}] = [\n')
    for i in take(n):
        code += f'    {ref(i, trees)},\n'
    code += '];\n'
    n = take(1)[0]
    code += ('\n/// `aQuantTable[]`: the quantized value table of each codebook. Generated by\n'
             f'/// `gen_tables.py` (pointer targets printed by a program compiled against `{SRC}`).\n'
             f'pub static A_QUANT_TABLE: [&[i8]; {n}] = [\n')
    for i in take(n):
        code += f'    {ref(i, vals)},\n'
    code += '];\n'
    assert pos == len(out)
    return code


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('-j', type=int, default=3)
    a = ap.parse_args()

    tables, shp, vals = scalar_tables(SRC, a.j)
    elid = element_ids()
    out = LICENSE + f'''//
// Ported from {SRC}.
//
// GENERATED by `python3 crates/aacdec/src/aac_rom/gen_tables.py`; do not edit. Every table of
// scalars is dumped with scripts/dump_table.py (the command is given at each table), which checks
// that the arm64 and x86_64 builds give the same values.

//! The constant tables of `aac_rom.cpp` (except `aStateConstant2State`, see `aac_rom.rs`).

use super::{{CodeBookDescription, FixpFilt, FixpTcc, SfbInfo}};
use fdk_aac_rs_fdk::archdef::FixpWtb;
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::FixpDbl;
'''
    for (static, ctype, name, rank, _), v in zip(tables, vals):
        out += '\n' + table_item(SRC, ctype, name, shp[name], v, elid)
    out += pointer_tables()
    open(os.path.join(HERE, 'tables.rs'), 'w').write(out)
    print(f'{len(tables)} scalar tables + 5 pointer tables')


if __name__ == '__main__':
    main()
