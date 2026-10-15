#!/usr/bin/env python3
"""Profiles the bench cases (scripts/bench.py), for the Rust port and the reference: with Linux perf,
or on macOS with `sample` (on inputs repeated --repeat times, because `sample` attaches to the
running process; ADTS, LOAS and PCM can be concatenated).

For every case it runs `perf record` on the Rust CLI and on the reference harness and writes the
flat profile (self time per symbol, top N) to work/perf/<build>/<case>.{rust,ref}.txt. Run it in
the Linux container with the PERFMON capability:

  LINUX_REF_PERF=1 scripts/linux_ref.sh python3 scripts/perf_profile.py [--build ref-x86_64] [--dec EXE --enc EXE]
  scripts/perf_profile.py                    # macOS arm64: ref-arm64

Usage: scripts/perf_profile.py [--build B] [--dec EXE] [--enc EXE] [--filter REGEX] [--top N] [--freq HZ]
"""
import argparse, os, platform, re, subprocess, sys, time

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402


def repeated(path, n):
    """`path` concatenated n times (cached next to the bench inputs)."""
    if n <= 1 or path.endswith('.mp4'):
        return path
    dst = os.path.join(bench.BENCH, 'prof', f'x{n}-' + os.path.basename(path))
    if not os.path.exists(dst):
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        data = open(path, 'rb').read()
        with open(dst, 'wb') as f:
            for _ in range(n):
                f.write(data)
    return dst


def mp4_access_units(data):
    """The ASC and the access units of the first MPEG-4 audio track of an MP4 file."""
    import struct
    import aacstream
    asc = bytes.fromhex(aacstream.mp4_info(data)['asc_hex'])
    tables = {}

    def walk(s, e, depth):
        for t, s2, e2 in aacstream._boxes(data, s, e):
            if t in ('moov', 'trak', 'mdia', 'minf', 'stbl'):
                walk(s2, e2, depth + 1)
                if t == 'trak' and 'stsz' in tables and 'hdlr' in tables and tables['hdlr'] == 'soun':
                    raise StopIteration
                if t == 'trak':
                    tables.clear()
            elif t == 'hdlr':
                tables['hdlr'] = data[s2 + 8:s2 + 12].decode('latin1')
            elif t in ('stsz', 'stsc', 'stco', 'co64'):
                tables[t] = (s2, e2)
    try:
        walk(0, len(data), 0)
    except StopIteration:
        pass
    s2, _ = tables['stsz']
    fixed, n = struct.unpack('>II', data[s2 + 4:s2 + 12])
    sizes = [fixed] * n if fixed else list(struct.unpack('>%dI' % n, data[s2 + 12:s2 + 12 + 4 * n]))
    s2, _ = tables['stsc']
    stsc = [struct.unpack('>III', data[s2 + 8 + 12 * i:s2 + 20 + 12 * i])
            for i in range(struct.unpack('>I', data[s2 + 4:s2 + 8])[0])]
    if 'stco' in tables:
        s2, _ = tables['stco']
        k = struct.unpack('>I', data[s2 + 4:s2 + 8])[0]
        offs = list(struct.unpack('>%dI' % k, data[s2 + 8:s2 + 8 + 4 * k]))
    else:
        s2, _ = tables['co64']
        k = struct.unpack('>I', data[s2 + 4:s2 + 8])[0]
        offs = list(struct.unpack('>%dQ' % k, data[s2 + 8:s2 + 8 + 8 * k]))
    aus, si = [], 0
    for c, off in enumerate(offs):
        per = [e[1] for e in stsc if e[0] <= c + 1][-1]
        for _ in range(per):
            if si >= len(sizes):
                break
            aus.append(data[off:off + sizes[si]])
            off += sizes[si]
            si += 1
    return asc, aus


def repeated_mp4(args, n):
    """`-t mp4 FILE` as raw access units repeated n times: `-t raw --asc HEX --au-sizes F FILE`."""
    path = args[-1]
    base = os.path.join(bench.BENCH, 'prof', f'x{n}-' + os.path.basename(path))
    raw, sizes = base + '.raw', base + '.sizes'
    asc, aus = mp4_access_units(open(path, 'rb').read())
    if not os.path.exists(raw):
        os.makedirs(os.path.dirname(raw), exist_ok=True)
        with open(raw, 'wb') as f, open(sizes, 'w') as g:
            for _ in range(n):
                for au in aus:
                    f.write(au)
                    g.write(f'{len(au)}\n')
    return ['-t', 'raw', '--asc', asc.hex(), '--au-sizes', sizes, raw]


def profile_mac(cmd, out, top):
    """Self time per symbol from macOS `sample` (the "top of stack" counts)."""
    p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.05)
    raw = out + '.sample'
    subprocess.run(['sample', str(p.pid), '120', '1', '-mayDie', '-file', raw],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    p.wait()
    if not os.path.exists(raw):
        with open(out, 'w') as f:
            f.write(' '.join(cmd) + '\n(the process ended before sample attached; use a longer input)\n')
        return
    counts = []
    take = False
    for line in open(raw, errors='replace'):
        if 'Sort by top of stack' in line:
            take = True
            continue
        if take:
            m = re.match(r'\s+(.*?)\s+(\d+)\s*$', line)
            if not m:
                if counts:
                    break
                continue
            name = re.sub(r'\s*\(in [^)]*\)', '', m.group(1))
            if name in ('_dyld_start', 'start', 'thread_start') or name.startswith('0x'):
                continue  # attaching and loader frames, not work of the process
            counts.append((name, int(m.group(2))))
    total = sum(c for _, c in counts) or 1
    with open(out, 'w') as f:
        f.write(' '.join(cmd) + '\n')
        for name, c in counts[:top]:
            f.write(f'{100.0 * c / total:7.2f}%  {name}\n')
    os.remove(raw)


def profile(cmd, data, out, top, freq):
    subprocess.run(['perf', 'record', '-q', '-F', str(freq), '-o', data, '--'] + cmd,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    rep = subprocess.run(['perf', 'report', '-f', '-i', data, '--no-children', '--sort', 'symbol',
                          '--stdio', '--percent-limit', '0.3'],
                         capture_output=True, text=True, check=True).stdout
    lines = [l for l in rep.splitlines() if l.strip() and not l.startswith('#')][:top]
    with open(out, 'w') as f:
        f.write(' '.join(cmd) + '\n' + '\n'.join(lines) + '\n')
    os.remove(data)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--build', default=bench.default_build())
    ap.add_argument('--dec')
    ap.add_argument('--enc')
    ap.add_argument('--filter')
    ap.add_argument('--top', type=int, default=40)
    ap.add_argument('--freq', type=int, default=4999)
    ap.add_argument('--repeat', type=int, default=4, help='macOS: input repetitions')
    a = ap.parse_args()
    tdir = os.path.join(bench.REPO, 'target', 'linux' if os.path.exists('/.dockerenv') else '', 'release')
    rdec = a.dec or os.path.join(tdir, 'fdk-aac-rs-dec')
    renc = a.enc or os.path.join(tdir, 'fdk-aac-rs-enc')
    h = os.path.join(bench.WORK, 'build-ref', a.build, 'harness')
    cdec, cenc = os.path.join(h, 'fdkdec-ref'), os.path.join(h, 'fdkenc-ref')
    bench.prepare(cdec, cenc)
    outdir = os.path.join(bench.WORK, 'perf', a.build)
    os.makedirs(outdir, exist_ok=True)
    for name, kind, args in bench.cases():
        if a.filter and not re.search(a.filter, name):
            continue
        if kind == 'dec-multi':
            # the largest stream only
            kind, args, name = 'dec', args[0], name.split(' (')[0] + ' (largest stream)'
        slug = re.sub(r'[^a-z0-9.-]+', '_', name.lower()).strip('_')
        exes = (rdec, cdec) if kind == 'dec' else (renc, cenc)
        for tag, exe in zip(('rust', 'ref'), exes):
            out = os.path.join(outdir, f'{slug}.{tag}.txt')
            if platform.system() == 'Darwin':
                margs = repeated_mp4(args, a.repeat * 8) if args[-1].endswith('.mp4') \
                    else args[:-1] + [repeated(args[-1], a.repeat)]
                profile_mac([exe] + margs[:-1] + ['-o', '/dev/null', margs[-1]], out, a.top)
            else:
                profile([exe] + args[:-1] + ['-o', '/dev/null', args[-1]],
                        os.path.join(outdir, f'{slug}.{tag}.data'), out, a.top, a.freq)
        print(f'{name}: {outdir}/{slug}.{{rust,ref}}.txt', flush=True)
    owner = os.environ.get('FDK_AAC_RS_OWNER')
    if owner and os.geteuid() == 0:
        # run as root for perf (scripts/linux_ref.sh): give the files back to the calling user
        subprocess.run(['chown', '-R', owner, os.path.join(bench.WORK, 'perf')], check=True)


if __name__ == '__main__':
    main()
