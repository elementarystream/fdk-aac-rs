#!/usr/bin/env python3
"""End-to-end corpus runs (PORTING.md, "End-to-end corpus"; interface in PORTING.md, "Reference harness").

Every corpus entry is run through a decoder or encoder program with the command-line interface of
The reference harness (fdkdec-ref, fdkenc-ref of work/build-ref/<build>/harness)
or the Rust CLI (fdk-aac-rs-dec, fdk-aac-rs-enc). The SHA-256 of each output (PCM or bitstream)
and of each JSON Lines log is compared with, or written to, a hash file in sha256sum format:

    <sha256>  <name>.pcm      decoder output       <sha256>  <name>.bs   encoder output
    <sha256>  <name>.jsonl    log of the run

Manifest (JSON): a list of entries, or an object with an "entries" list. Entry fields:
    name    unique name (used for the output file names)
    kind    "decode" or "encode"
    input   input file, relative to work/
    args    arguments before the input; "{work}" is replaced by the work directory
The output options (-o, -l) are added by this script and must not appear in args.

The hash files are not part of the repository; each machine makes its own with the reference
harness. Usage:
    scripts/e2e.py --manifest M --build ref-arm64                 # reference run: write or check the hashes
    scripts/e2e.py --manifest M --build ref-arm64 --update        # reference run, rewrite the hashes
    scripts/e2e.py --manifest M --build ref-arm64 --impl rust     # Rust CLI, check the hashes

--build selects the reference build: its harness programs (for --impl ref) and the default hash
file work/sums/SHA256SUMS.<build> (below the work directory, FDK_AAC_RS_WORK_DIR if set); --sums
FILE selects another one. A reference run writes the default hash file if it does not exist yet,
and otherwise checks that the build reproduces it; --update (or --update-sums) rewrites it. When
the hash file is written, the hashes of the entries that did not run (--filter, missing inputs,
failures) are kept. A missing file given with --sums is only written with --update. A Rust run
needs the hash file and stops if it is missing: run the reference harness first (the command
without --impl rust).

For a mismatch, the first differing call of the log and the log of the reference build
--ref-build (default: --build) is reported. That log is taken from work/e2e/ref-<ref-build>/ or
produced there with the reference harness when it exists. Comparing two reference builds:
    scripts/e2e.py --manifest M --build ref-x86_64-mac --sums work/sums/SHA256SUMS.ref-arm64 --ref-build ref-arm64
The Rust CLIs of the x86 variant on Apple silicon (--build ref-x86_64 --ref-build ref-x86_64-mac)
need work/sums/SHA256SUMS.ref-x86_64: a copy of the file of a Linux machine, or one written with
the Rosetta 2 build:
    scripts/e2e.py --manifest M --build ref-x86_64-mac --sums work/sums/SHA256SUMS.ref-x86_64 --update

--enc-args passes extra arguments to the Rust encoder for the encode entries, e.g. the thread
count of the opt-in multi-threading, whose output must be the same for every count:
    scripts/e2e.py --manifest M --build ref-arm64 --impl rust --filter '^enc-' --enc-args '-j 4'
--dec-args does the same for the Rust decoder and the decode entries (the opt-in frame pipelining
of -j N returns every frame one call later; the CLI writes the log of one thread):
    scripts/e2e.py --manifest M --build ref-arm64 --impl rust --filter '^(dec-|local-dec)' --dec-args '-j 2'

On macOS, ref-x86_64-mac binaries run under Rosetta 2 (arch -x86_64). ref-x86_64 runs on Linux,
e.g. in the container: scripts/linux_ref.sh scripts/e2e.py --manifest M --build ref-x86_64. ref-i686
(the x86_32 variant) runs on Linux x86-64 as well; the Rust CLIs of that variant are built for
the target i686-unknown-linux-gnu:
    scripts/linux_ref.sh scripts/e2e.py --manifest M --build ref-i686 --impl rust \
      --dec-exe target/linux/i686-unknown-linux-gnu/release/fdk-aac-rs-dec \
      --enc-exe target/linux/i686-unknown-linux-gnu/release/fdk-aac-rs-enc
"""
import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import platform
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORK = os.environ.get('FDK_AAC_RS_WORK_DIR', os.path.join(REPO, 'work'))
EXT = {'decode': 'pcm', 'encode': 'bs'}


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def load_manifest(path):
    with open(path) as f:
        m = json.load(f)
    entries = m['entries'] if isinstance(m, dict) else m
    names = set()
    for e in entries:
        if e['kind'] not in EXT:
            sys.exit(f'{e["name"]}: unknown kind {e["kind"]!r}')
        if e['name'] in names:
            sys.exit(f'duplicate entry name {e["name"]!r}')
        names.add(e['name'])
    return entries


def load_sums(path):
    sums = {}
    if os.path.exists(path):
        with open(path) as f:
            for line in f:
                if line.strip():
                    h, name = line.rstrip('\n').split('  ', 1)
                    sums[name] = h
    return sums


def write_sums(path, sums):
    with open(path, 'w') as f:
        for name in sorted(sums):
            f.write(f'{sums[name]}  {name}\n')


def ref_programs(build):
    d = os.path.join(WORK, 'build-ref', build, 'harness')
    return {'decode': os.path.join(d, 'fdkdec-ref'), 'encode': os.path.join(d, 'fdkenc-ref')}


def prefix(build, impl):
    """Rosetta 2 prefix for the x86-64 macOS reference build on an arm64 host."""
    if impl == 'ref' and build == 'ref-x86_64-mac' and platform.machine() == 'arm64':
        return ['arch', '-x86_64']
    return []


def run_entry(e, programs, pre, out_dir, enc_args=(), dec_args=()):
    """Runs one entry; returns {file name: sha256} and an error string (None if it ran)."""
    out = os.path.join(out_dir, f'{e["name"]}.{EXT[e["kind"]]}')
    log = os.path.join(out_dir, f'{e["name"]}.jsonl')
    args = [a.replace('{work}', WORK) for a in e['args']]
    if e['kind'] == 'encode':
        args += list(enc_args)
    else:
        args += list(dec_args)
    inp = os.path.join(WORK, e['input'])
    if not os.path.exists(inp):
        return None, 'missing input'
    cmd = pre + [programs[e['kind']]] + args + ['-o', out, '-l', log, inp]
    r = subprocess.run(cmd, capture_output=True, text=True, errors='replace')
    if r.returncode != 0:
        return {}, f'exit code {r.returncode}: {(r.stdout + r.stderr).strip()[-300:]}'
    res = {}
    for p in (out, log):
        res[os.path.basename(p)] = sha256(p) if os.path.exists(p) else None
    return res, None


def first_difference(a, b):
    """Describes the first differing record of two JSON Lines logs."""
    with open(a) as fa, open(b) as fb:
        for i, (la, lb) in enumerate(zip(fa, fb), 1):
            if la == lb:
                continue
            ra, rb = json.loads(la), json.loads(lb)
            where = f'line {i}: {ra.get("call")}' + (f' n={ra["n"]}' if 'n' in ra else '')
            if ra.get('call') != rb.get('call'):
                return f'{where}: call {ra.get("call")!r} vs {rb.get("call")!r}'
            keys = []

            def walk(x, y, path):
                if isinstance(x, dict) and isinstance(y, dict):
                    for k in list(x) + [k for k in y if k not in x]:
                        walk(x.get(k), y.get(k), f'{path}.{k}' if path else k)
                elif isinstance(x, list) and isinstance(y, list) and len(x) == len(y):
                    for k, (u, v) in enumerate(zip(x, y)):
                        walk(u, v, f'{path}[{k}]')
                elif x != y:
                    keys.append(f'{path}: {x!r} vs {y!r}')
            walk(ra, rb, '')
            return f'{where}: ' + '; '.join(keys[:5])
        na, nb = sum(1 for _ in open(a)), sum(1 for _ in open(b))
        return f'logs differ in length ({na} vs {nb} lines)' if na != nb else 'logs are equal'


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--manifest', required=True)
    ap.add_argument('--build', required=True, help='reference build, e.g. ref-arm64')
    ap.add_argument('--impl', choices=['ref', 'rust'], default='ref')
    ap.add_argument('--dec-exe', help='decoder program (default: target/release/fdk-aac-rs-dec)')
    ap.add_argument('--enc-exe', help='encoder program (default: target/release/fdk-aac-rs-enc)')
    ap.add_argument('--sums', help='hash file (default: work/sums/SHA256SUMS.<build>)')
    ap.add_argument('--ref-build', help='reference build of the logs for diffs (default: --build)')
    ap.add_argument('--update', '--update-sums', action='store_true',
                    help='rewrite the hashes even if the hash file exists (reference runs only)')
    ap.add_argument('--filter', help='regular expression on entry names')
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--keep', action='store_true', help='keep the outputs in work/e2e/<impl>-<build>/')
    ap.add_argument('--enc-args', default='',
                    help='extra arguments of the Rust encoder for encode entries (e.g. "-j 4")')
    ap.add_argument('--dec-args', default='',
                    help='extra arguments of the Rust decoder for decode entries (e.g. "-j 2")')
    a = ap.parse_args()
    if a.update and a.impl != 'ref':
        sys.exit('--update writes reference hashes; use it with --impl ref')
    enc_args = a.enc_args.split()
    if enc_args and a.impl != 'rust':
        sys.exit('--enc-args is for --impl rust')
    dec_args = a.dec_args.split()
    if dec_args and a.impl != 'rust':
        sys.exit('--dec-args is for --impl rust')

    entries = load_manifest(a.manifest)
    if a.filter:
        entries = [e for e in entries if re.search(a.filter, e['name'])]
    sums_path = a.sums or os.path.join(WORK, 'sums', f'SHA256SUMS.{a.build}')
    if not os.path.exists(sums_path) and not a.update:
        if a.impl == 'rust':
            sys.exit(f'missing hash file {sums_path}; write it with the reference harness first:\n'
                     f'    scripts/e2e.py --manifest {a.manifest} --build {a.build}\n'
                     '(the header of scripts/e2e.py shows the x86 variant on Apple silicon)')
        if a.sums:
            sys.exit(f'missing hash file {sums_path}; add --update to write it')
    # A reference run writes the hash file if it does not exist yet, and otherwise checks it.
    write = a.update or not os.path.exists(sums_path)
    sums = load_sums(sums_path)
    if a.impl == 'ref':
        programs = ref_programs(a.build)
    else:
        programs = {'decode': a.dec_exe or os.path.join(REPO, 'target', 'release', 'fdk-aac-rs-dec'),
                    'encode': a.enc_exe or os.path.join(REPO, 'target', 'release', 'fdk-aac-rs-enc')}
    for p in programs.values():
        if not os.path.exists(p):
            sys.exit(f'missing program {p}')
    pre = prefix(a.build, a.impl)
    ref_build = a.ref_build or a.build
    can_diff = not (a.impl == 'ref' and ref_build == a.build)
    out_dir = os.path.join(WORK, 'e2e', f'{a.impl}-{a.build}')
    if enc_args:
        out_dir += '-' + re.sub(r'[^A-Za-z0-9]+', '', a.enc_args)
    if dec_args:
        out_dir += '-dec' + re.sub(r'[^A-Za-z0-9]+', '', a.dec_args)
    os.makedirs(out_dir, exist_ok=True)

    failures = skipped = 0
    with cf.ThreadPoolExecutor(a.jobs) as ex:
        futs = {ex.submit(run_entry, e, programs, pre, out_dir, enc_args, dec_args): e for e in entries}
        for fut in cf.as_completed(futs):
            e = futs[fut]
            res, err = fut.result()
            if res is None:
                # Inputs that are not present here (local-only material of another developer,
                # Fraunhofer files not fetched by hand) are skipped, not failed.
                skipped += 1
                print(f'skip {e["name"]}: {err}')
                continue
            if err:
                failures += 1
                print(f'FAIL {e["name"]}: {err}')
                continue
            if write:
                sums.update({k: v for k, v in res.items() if v})
                print(f'ok   {e["name"]}')
                ok = True
            else:
                bad = [k for k, v in res.items() if sums.get(k) != v]
                ok = not bad
                if ok:
                    print(f'ok   {e["name"]}')
                else:
                    failures += 1
                    missing = [k for k in bad if k not in sums]
                    msg = f'no reference hash for {", ".join(missing)}' if missing else f'{", ".join(bad)} differ'
                    if missing and a.impl == 'ref':
                        msg += ' (--update adds it)'
                    log = os.path.join(out_dir, f'{e["name"]}.jsonl')
                    ref_log = os.path.join(WORK, 'e2e', f'ref-{ref_build}', f'{e["name"]}.jsonl')
                    if not missing and can_diff and not os.path.exists(ref_log):
                        rp = ref_programs(ref_build)
                        if os.path.exists(rp[e['kind']]):
                            os.makedirs(os.path.dirname(ref_log), exist_ok=True)
                            run_entry(e, rp, prefix(ref_build, 'ref'), os.path.dirname(ref_log))
                    if not missing and can_diff and os.path.exists(ref_log) and os.path.exists(log):
                        msg += '; first difference: ' + first_difference(ref_log, log)
                    print(f'FAIL {e["name"]}: {msg}')
            if ok and not a.keep:
                for k in res:
                    p = os.path.join(out_dir, k)
                    if os.path.exists(p):
                        os.remove(p)
    if write:
        os.makedirs(os.path.dirname(os.path.abspath(sums_path)), exist_ok=True)
        write_sums(sums_path, sums)
        print(f'wrote the hashes to {sums_path}')
    else:
        print(f'compared with the hashes of {sums_path}')
    print(f'{len(entries) - failures - skipped} of {len(entries) - skipped} entries passed'
          f' ({skipped} skipped: input missing)')
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
