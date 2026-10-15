#!/usr/bin/env python3
"""Speed benchmark of the Rust port against the reference library.

Every case runs the reference harness (work/build-ref/<build>/harness/fdkdec-ref, fdkenc-ref)
and the Rust CLIs (fdk-aac-rs-dec, fdk-aac-rs-enc), which take the same options and make the same
library calls, on the same input without a log. Reported: the wall-clock time and the CPU time
(user + system) of the best of N runs, the speedup, and whether the outputs are identical.

Inputs (work/bench, created on first use; nothing is committed): the real-world stream
work/rw/lc320.aac (243 s, stereo, 44.1 kHz) decoded by the reference to PCM; a 5.1 PCM
made from it (L, R, (L+R)/2, (L-R)/2, L, R); encodings of these by the reference encoder, which
are the decoder inputs; and the largest USAC conformance streams (work/conformance/23003-7).

Usage: scripts/bench.py [--build B] [--dec EXE] [--enc EXE] [--v3] [--runs N] [--filter REGEX] [--threads N]
                      [--dec-args ARGS] [--mem]
  --build    reference build (default: ref-arm64 on arm64 macOS, ref-x86_64 on Linux x86-64;
             ref-i686 for the x86_32 variant)
  --dec/--enc  the Rust CLIs (default: target/release/, target/linux/release/ in the container)
  --v3       default the Rust CLIs to the optional x86-64-v3 build of scripts/linux_check.sh --v3
             (target/linux/x86-64-v3/release/ in the container)
  --threads  thread count of the Rust encoder and decoder (-j N, the opt-in multi-threading of the
             encoder and frame pipelining of the decoder, from 4 threads the chunked decoding of
             long inputs; default 1); the reference is single-threaded. The CPU time then exceeds
             the wall-clock time.
  --dec-args extra arguments of the Rust decoder (e.g. "--chunks 0": no chunked decoding)
  --mem      also print the peak resident set size of both programs (the largest of the runs)
Environment: FDK_AAC_RS_WORK_DIR (default <repo>/work).
"""
import argparse, atexit, filecmp, glob, os, platform, re, shutil, subprocess, sys, tempfile, time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORK = os.environ.get('FDK_AAC_RS_WORK_DIR', os.path.join(REPO, 'work'))
BENCH = os.path.join(WORK, 'bench')
SRC = os.path.join(WORK, 'rw', 'lc320.aac')


def p(*a):
    return ['--param'] + [f'AACENC_{a[0]}={a[1]}'] + sum((['--param', f'AACENC_{k}={v}'] for k, v in zip(a[2::2], a[3::2])), [])


# (name, input PCM, channels, encoder parameters, transport of the output)
ENCODES = [
    ('lc128', 'stereo', 2, p('AOT', 2, 'BITRATE', 128000, 'TRANSMUX', 2, 'AFTERBURNER', 1), 'adts'),
    ('lc128-noab', 'stereo', 2, p('AOT', 2, 'BITRATE', 128000, 'TRANSMUX', 2, 'AFTERBURNER', 0), 'adts'),
    ('he64', 'stereo', 2, p('AOT', 5, 'BITRATE', 64000, 'TRANSMUX', 2, 'AFTERBURNER', 1), 'adts'),
    ('hev2-32', 'stereo', 2, p('AOT', 29, 'BITRATE', 32000, 'TRANSMUX', 2, 'AFTERBURNER', 1), 'adts'),
    ('ld128', 'stereo', 2, p('AOT', 23, 'BITRATE', 128000, 'TRANSMUX', 10, 'AFTERBURNER', 1), 'loas'),
    ('eld-sbr64', 'stereo', 2, p('AOT', 39, 'BITRATE', 64000, 'SBR_MODE', 1, 'TRANSMUX', 10, 'AFTERBURNER', 1), 'loas'),
    ('lc384-5.1', '5.1', 6, p('AOT', 2, 'BITRATE', 384000, 'CHANNELMODE', 6, 'TRANSMUX', 2, 'AFTERBURNER', 1), 'adts'),
]


def default_build():
    if platform.system() == 'Darwin':
        return 'ref-arm64' if platform.machine() == 'arm64' else 'ref-x86_64-mac'
    return 'ref-x86_64'


# ru_maxrss is in bytes on macOS and in KiB on Linux.
RSS_UNIT = 1 if platform.system() == 'Darwin' else 1024


def run(cmd, runs):
    """Best wall-clock time of `runs` runs, the CPU time of that run and the largest peak resident
    set size (bytes) of all runs."""
    best = None
    rss = 0
    for _ in range(runs):
        t = time.perf_counter()
        pr = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        _, status, ru = os.wait4(pr.pid, 0)
        d = time.perf_counter() - t
        pr.returncode = os.waitstatus_to_exitcode(status)
        if pr.returncode != 0:
            raise subprocess.CalledProcessError(pr.returncode, cmd)
        cpu = ru.ru_utime + ru.ru_stime
        rss = max(rss, ru.ru_maxrss * RSS_UNIT)
        if best is None or d < best[0]:
            best = (d, cpu)
    return best + (rss,)


def prepare(ref_dec, ref_enc):
    os.makedirs(BENCH, exist_ok=True)
    stereo = os.path.join(BENCH, 'stereo.s16le')
    if not os.path.exists(stereo):
        subprocess.run([ref_dec, '-t', 'adts', '-o', stereo, SRC], check=True,
                       stdout=subprocess.DEVNULL)
    five = os.path.join(BENCH, '5.1.s16le')
    if not os.path.exists(five):
        import array
        a = array.array('h')
        with open(stereo, 'rb') as f:
            a.frombytes(f.read())
        out = array.array('h', bytes(len(a) * 6))
        for i in range(len(a) // 2):
            l, r = a[2 * i], a[2 * i + 1]
            out[6 * i:6 * i + 6] = array.array('h', [l, r, (l + r) // 2, (l - r) // 2, l, r])
        with open(five, 'wb') as f:
            f.write(out.tobytes())
    for name, pcm, ch, params, tt in ENCODES:
        dst = os.path.join(BENCH, f'{name}.{tt}')
        if not os.path.exists(dst):
            subprocess.run([ref_enc, '-f', 's16le', '--rate', '44100', '--channels', str(ch)] + params +
                           ['-o', dst, os.path.join(BENCH, f'{pcm}.s16le')], check=True,
                           stdout=subprocess.DEVNULL)


def cases():
    c = [('dec lc320 (243 s)', 'dec', ['-t', 'adts', SRC])]
    for name, pcm, ch, params, tt in ENCODES:
        c.append((f'dec {name}', 'dec', ['-t', tt, os.path.join(BENCH, f'{name}.{tt}')]))
    usac = sorted(glob.glob(os.path.join(WORK, 'conformance', '23003-7', 'compressedMp4', '*.mp4')),
                  key=os.path.getsize, reverse=True)[:12]
    if usac:
        c.append(('dec usac (12 streams)', 'dec-multi', [['-t', 'mp4', f] for f in usac]))
    for name, pcm, ch, params, tt in ENCODES:
        c.append((f'enc {name}', 'enc', ['-f', 's16le', '--rate', '44100', '--channels', str(ch)] + params +
                  [os.path.join(BENCH, f'{pcm}.s16le')]))
    return c


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--build', default=default_build())
    ap.add_argument('--dec')
    ap.add_argument('--enc')
    ap.add_argument('--v3', action='store_true')
    ap.add_argument('--runs', type=int, default=3)
    ap.add_argument('--filter')
    ap.add_argument('--threads', type=int, default=1)
    ap.add_argument('--dec-args', default='',
                    help='extra arguments of the Rust decoder (e.g. "--chunks 0")')
    ap.add_argument('--mem', action='store_true',
                    help='also print the peak resident set size of both programs (MiB)')
    a = ap.parse_args()
    dec_extra = a.dec_args.split()
    tdir = os.path.join(REPO, 'target', 'linux' if os.path.exists('/.dockerenv') else '',
                        'x86-64-v3' if a.v3 else '', 'release')
    rdec = a.dec or os.path.join(tdir, 'fdk-aac-rs-dec')
    renc = a.enc or os.path.join(tdir, 'fdk-aac-rs-enc')
    h = os.path.join(WORK, 'build-ref', a.build, 'harness')
    cdec, cenc = os.path.join(h, 'fdkdec-ref'), os.path.join(h, 'fdkenc-ref')
    prepare(cdec, cenc)
    tmp = tempfile.mkdtemp(prefix='fdk-aac-rs-bench-')
    atexit.register(shutil.rmtree, tmp, True)
    print(f'reference {a.build}; Rust {rdec}; best of {a.runs}'
          + (f'; Rust threads {a.threads}' if a.threads != 1 else ''))
    print(f'{"case":24} {"ref wall":>9} {"rust wall":>10} {"speedup":>8} {"ref cpu":>8} {"rust cpu":>9}'
          + (f' {"ref MiB":>8} {"rust MiB":>9}' if a.mem else '') + '  output')
    tot = [0.0, 0.0]
    jr = ['-j', str(a.threads)] if a.threads != 1 else []
    for name, kind, args in cases():
        if a.filter and not re.search(a.filter, name):
            continue
        out_c, out_r = os.path.join(tmp, 'c.out'), os.path.join(tmp, 'r.out')
        if kind == 'dec-multi':
            res = []
            same = True
            for exe, out in ((cdec, out_c), (rdec, out_r)):
                w = c = 0.0
                m = 0
                for i, x in enumerate(args):
                    o = f'{out}.{i}'
                    j = jr + dec_extra if exe == rdec else []
                    tw, tc, tm = run([exe] + x[:-1] + j + ['-o', o, x[-1]], a.runs)
                    w, c, m = w + tw, c + tc, max(m, tm)
                res.append((w, c, m))
            same = all(filecmp.cmp(f'{out_c}.{i}', f'{out_r}.{i}', shallow=False) for i in range(len(args)))
            (cw, cc, cm), (rw, rc, rm) = res
        else:
            exe_c, exe_r = (cdec, rdec) if kind == 'dec' else (cenc, renc)
            cw, cc, cm = run([exe_c] + args[:-1] + ['-o', out_c, args[-1]], a.runs)
            extra = dec_extra if kind == 'dec' else []
            rw, rc, rm = run([exe_r] + args[:-1] + jr + extra + ['-o', out_r, args[-1]], a.runs)
            same = filecmp.cmp(out_c, out_r, shallow=False)
        tot[0] += cw
        tot[1] += rw
        print(f'{name:24} {cw:9.3f} {rw:10.3f} {cw / rw:7.2f}x {cc:8.3f} {rc:9.3f}'
              + (f' {cm / 2**20:8.1f} {rm / 2**20:9.1f}' if a.mem else '')
              + f'  {"identical" if same else "DIFFERENT"}')
    print(f'{"total":24} {tot[0]:9.3f} {tot[1]:10.3f} {tot[0] / tot[1]:7.2f}x')


if __name__ == '__main__':
    main()
