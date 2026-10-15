#!/usr/bin/env python3
"""Fetch, verify and index the ISO conformance bitstreams (PORTING.md, "Conformance bitstreams").

ISO/IEC 14496-26 2nd edition (MPEG-4 Audio conformance):
  Downloads the archives (default: compressedMp4, compressedAss, compressedAdts, compressedAdif)
  and the MD5 list mpeg4audio-conformance2ndEd_20230125.md5 from standards.iso.org into
  work/conformance/archives/ (resumable with `curl -C -`; complete files are skipped), extracts
  them into work/conformance/14496-26/ and checks every extracted file against the MD5 list
  (and the zip CRC-32). referencesWav.zip (9.8 GB) is fetched only with --references-wav.

ISO/IEC 23003-7 (USAC conformance), with --usac:
  Reads the bitstreams (compressedMp4/, 906 files, 140 MB) out of the 3.5 GB archive by HTTP
  range requests, checks their CRC-32 against the archive directory, and stores them in
  work/conformance/23003-7/. --usac-wav downloads the whole archive and extracts the reference
  waveforms as well. ISO publishes no MD5 list for this archive.

Then writes work/conformance/index.json: one record per bitstream with size, MD5, the group (file
name family), role (positive: an AAC family that fdk-aac decodes; negative: a tool fdk-aac does not
support), container, and the AudioSpecificConfig fields (AOT, sampling rates, channels, frame
length, SBR/PS, epConfig, the ER AAC resilience flags). The field `fdk_asc_check` is a static
prediction from the ASC and the 2.0.3 sources (object type accepted, epConfig <= 1, ER with
channelConfiguration > 0), not a measurement.

The files are ISO electronic inserts: they may be used for testing decoders but must not be
redistributed. Everything stays in the git-ignored work/ directory.

Usage:
  scripts/fetch_conformance.py [--with ARCHIVE ...] [--references-wav] [--usac] [--usac-wav]
                             [--jobs N] [--offline] [--rehash] [--keep-going]
  scripts/fetch_conformance.py --summary      print the index summary only

Environment:
  FDK_AAC_RS_WORK_DIR   work directory (default: work/ of the main checkout)
"""
import argparse, collections, concurrent.futures as cf, datetime, hashlib, json, os, re, struct, \
    subprocess, sys, urllib.request, zipfile, zlib

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import aacstream  # noqa: E402

WORK = aacstream.work_dir()
CONF = os.path.join(WORK, 'conformance')
ARCH_DIR = os.path.join(CONF, 'archives')
INDEX = os.path.join(CONF, 'index.json')
CACHE = os.path.join(CONF, 'verified.json')

BASE_26 = 'https://standards.iso.org/iso-iec/14496/-26/ed-2/en/'
MD5_LIST = 'mpeg4audio-conformance2ndEd_20230125.md5'
ROOT_26 = 'mpeg4audio-conformance2ndEd_20230125'  # top directory inside the archives
DIR_26 = os.path.join(CONF, '14496-26')

# Content-Length of the files of the 2nd edition directory, as measured.
ARCHIVES_26 = {
    'compressedMp4.zip': 1207540409,
    'compressedAss.zip': 144111851,
    'compressedAdts.zip': 886858,
    'compressedAdif.zip': 869587,
    'compressedMpeg12.zip': 2106076,
    'referencesWav.zip': 9849662301,
    'referencesMp4.zip': 237718039,
    'referencesTxt.zip': 10176,
    'referencesPar.zip': 13284,
    'informative.zip': 7339010,
    'confTools.zip': 2267879,
    'doc.zip': 15908,
}
DEFAULT_26 = ['compressedMp4.zip', 'compressedAss.zip', 'compressedAdts.zip', 'compressedAdif.zip']

USAC_URL = 'https://standards.iso.org/iso-iec/23003/-7/ed-1/en/ISO_IEC_23003-7_Conformance_Sequences.zip'
USAC_SIZE = 3477455973
USAC_TABLES_URL = 'https://standards.iso.org/iso-iec/23003/-7/ed-1/en/ISO_IEC_23003-7_Conformance_Tables.xlsx'
DIR_USAC = os.path.join(CONF, '23003-7')

MEDIA_EXT = ('.mp4', '.ass', '.adts', '.adif')

# (regex on the file name, group, tool, role). First match wins. Roles follow PORTING.md,
# "Conformance bitstreams": positive = AAC families fdk-aac supports, negative = everything else.
GROUPS = [
    (r'als_', 'als', 'ALS', 'negative'),
    (r'al_sbr_', 'al_sbr', 'HE-AAC / HE-AAC v2 (AAC LC + SBR [+ PS])', 'positive'),
    (r'al960_sbr_', 'al_sbr', 'HE-AAC / HE-AAC v2 (AAC LC + SBR [+ PS])', 'positive'),
    (r'al\d', 'al', 'AAC LC', 'positive'),
    (r'er_al', 'er_al', 'ER AAC LC', 'positive'),
    (r'er_ad_ldmps_', 'er_ad_ldmps', 'ER AAC LD + LD MPEG Surround', 'positive'),
    (r'er_ad', 'er_ad', 'ER AAC LD', 'positive'),
    (r'er_eld_ldmps_', 'er_eld_ldmps', 'ER AAC ELD + LD MPEG Surround (ELDv2)', 'positive'),
    (r'er_eld_sbr_', 'er_eld_sbr', 'ER AAC ELD + LD-SBR', 'positive'),
    (r'er_eld', 'er_eld', 'ER AAC ELD', 'positive'),
    (r'am\d', 'am', 'AAC Main', 'negative'),
    (r'as\d', 'as', 'AAC SSR', 'negative'),
    (r'ap\d', 'ap', 'AAC LTP', 'negative'),
    (r'er_ap', 'er_ap', 'ER AAC LTP', 'negative'),
    (r'er_bs', 'er_bs', 'ER BSAC', 'negative'),
    (r'ac\d', 'ac', 'AAC scalable', 'negative'),
    (r'er_ac', 'er_ac', 'ER AAC scalable', 'negative'),
    (r'ce\d', 'ce', 'CELP', 'negative'),
    (r'er_ce', 'er_ce', 'ER CELP', 'negative'),
    (r'hv\d', 'hv', 'HVXC', 'negative'),
    (r'er_hv', 'er_hv', 'ER HVXC', 'negative'),
    (r'er_hi', 'er_hi', 'ER HILN', 'negative'),
    (r'er_pa', 'er_pa', 'ER Parametric (HILN + HVXC)', 'negative'),
    (r'tv\d', 'tv', 'TwinVQ', 'negative'),
    (r'er_tv', 'er_tv', 'ER TwinVQ', 'negative'),
    (r'sls', 'sls', 'SLS', 'negative'),
    (r'ssc_', 'ssc', 'SSC', 'negative'),
    (r'sy\d', 'sy', 'Structured Audio (algorithmic synthesis and audio FX)', 'negative'),
    (r'tts\d', 'tts', 'TTSI', 'negative'),
    (r'DST_', 'dst', 'DST', 'negative'),
    (r'l[123]_', 'layer', 'MPEG-1/2 Layer 1-3', 'negative'),
    (r'aab', 'aab', 'Advanced AudioBIFS', 'negative'),
    (r'Abv3_', 'abv3', 'AudioBIFS v3', 'negative'),
    (r'ab\d', 'ab', 'AudioBIFS', 'negative'),
]
USAC_GROUPS = {'Fd': 'USAC FD core', 'Lpd': 'USAC LPD core', 'Cct': 'USAC core switching',
               'eSbr': 'USAC eSBR', 'Mps': 'USAC MPS212', 'Ext': 'USAC extensions'}


def log(*a):
    print(*a, flush=True)


def md5_file(path):
    h = hashlib.md5()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def classify(name):
    if re.match(r'(Fd|Lpd|Cct|eSbr|Mps|Ext)_', name):
        g = name.split('_')[0]
        return g, USAC_GROUPS[g], 'positive'
    for rx, g, tool, role in GROUPS:
        if re.match(rx, name):
            return g, tool, role
    return 'unknown', 'unknown', 'unknown'


# --- download ----------------------------------------------------------------------------------

def remote_size(url):
    req = urllib.request.Request(url, method='HEAD')
    with urllib.request.urlopen(req, timeout=60) as r:
        return int(r.headers['Content-Length'])


def download(url, dst, size=None, offline=False):
    """Resumable download with curl (-C -). Skips a complete file. Returns the path."""
    if os.path.exists(dst) and (size is None or os.path.getsize(dst) == size):
        return dst
    if offline:
        raise SystemExit(f'{dst} is missing or incomplete and --offline was given')
    if size is None:
        size = remote_size(url)
        if os.path.exists(dst) and os.path.getsize(dst) == size:
            return dst
    part = dst + '.part'
    if not (os.path.exists(part) and os.path.getsize(part) == size):
        log(f'downloading {url} ({size / 1e6:.1f} MB)')
        for attempt in range(5):
            r = subprocess.run(['curl', '-fsS', '-C', '-', '--retry', '5', '--retry-delay', '5',
                                '-o', part, url])
            if os.path.exists(part) and os.path.getsize(part) == size:
                break
            log(f'curl exit {r.returncode}, {os.path.getsize(part) if os.path.exists(part) else 0}'
                f' of {size} bytes; retrying ({attempt + 1}/5)')
        else:
            raise SystemExit(f'download of {url} failed')
    os.replace(part, dst)
    return dst


# --- MD5 list ----------------------------------------------------------------------------------

def read_md5_list(path):
    out = {}
    with open(path, encoding='latin1') as f:
        for line in f:
            parts = line.rstrip('\r\n').split(None, 1)
            if len(parts) == 2:
                rel = parts[1].strip()
                if rel.startswith('./'):
                    rel = rel[2:]
                out[rel] = parts[0].lower()
    return out


def load_cache():
    try:
        with open(CACHE) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_cache(cache):
    tmp = CACHE + '.tmp'
    with open(tmp, 'w') as f:
        json.dump(cache, f, indent=0, sort_keys=True)
    os.replace(tmp, CACHE)


def cached_md5(path, rel, cache, rehash=False):
    st = os.stat(path)
    c = cache.get(rel)
    if not rehash and c and c[0] == st.st_size and c[1] == st.st_mtime_ns:
        return c[2]
    h = md5_file(path)
    cache[rel] = [st.st_size, st.st_mtime_ns, h]
    return h


def member_rel(name):
    """Relative path of a zip member below the archive root. The 14496-26 archives store
    '../mpeg4audio-conformance2ndEd_20230125/<dir>/...'; this never returns a path outside."""
    parts = [p for p in name.replace('\\', '/').split('/') if p not in ('', '.', '..')]
    if parts and parts[0] == ROOT_26:
        parts = parts[1:]
    return '/'.join(parts)


def extract_26(archive, md5s, cache, stats, rehash):
    """Extracts one archive into DIR_26 and checks every file against the MD5 list."""
    name = os.path.basename(archive)
    st = stats[name]
    with zipfile.ZipFile(archive) as z:
        members = [i for i in z.infolist() if not i.is_dir()]
        for i in members:
            rel = member_rel(i.filename)
            if not rel:
                continue
            dst = os.path.join(DIR_26, rel)
            if os.path.exists(dst) and os.path.getsize(dst) == i.file_size:
                h = cached_md5(dst, rel, cache, rehash)
            else:
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                hh = hashlib.md5()
                tmp = dst + '.part'
                with z.open(i) as src, open(tmp, 'wb') as out:  # zipfile checks the CRC-32
                    for chunk in iter(lambda: src.read(1 << 20), b''):
                        hh.update(chunk)
                        out.write(chunk)
                os.replace(tmp, dst)
                h = hh.hexdigest()
                s = os.stat(dst)
                cache[rel] = [s.st_size, s.st_mtime_ns, h]
                st['extracted'] += 1
            st['files'] += 1
            exp = md5s.get(rel)
            if exp is None:
                st['not_in_md5_list'].append(rel)
            elif exp != h:
                st['md5_mismatch'].append(rel)
            else:
                st['md5_ok'] += 1
        prefix = members and member_rel(members[0].filename).split('/')[0]
    in_list = {r for r in md5s if r.split('/')[0] == prefix}
    present = {member_rel(i.filename) for i in members}
    st['in_md5_list_not_in_archive'] = sorted(in_list - present)
    # The _checksum/<file>.md5sum files repeat the MD5 of each file; cross-check them.
    for rel in sorted(present):
        if '/_checksum/' in '/' + rel and rel.endswith('.md5sum'):
            target = rel.replace('_checksum/', '').rsplit('.md5sum', 1)[0]
            try:
                with open(os.path.join(DIR_26, rel), encoding='latin1') as f:
                    tok = f.read().split()
            except OSError:
                continue
            if tok and target in md5s and tok[0].lower() != md5s[target]:
                st['md5sum_file_disagrees'].append(rel)


# --- USAC by range requests --------------------------------------------------------------------

def _range(url, a, b):
    req = urllib.request.Request(url, headers={'Range': f'bytes={a}-{b - 1}'})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                data = r.read()
            if len(data) == b - a:
                return data
        except OSError as e:
            if attempt == 4:
                raise
            log(f'retrying range {a}-{b}: {e}')
    raise IOError(f'short read of {url} [{a}, {b})')


def zip_directory(url, size):
    tail = _range(url, size - min(size, 65536 + 22), size)
    i = tail.rfind(b'PK\x05\x06')
    if i < 0:
        raise IOError('end of central directory not found')
    count, cd_size, cd_off = struct.unpack('<HII', tail[i + 10:i + 20])
    j = tail.rfind(b'PK\x06\x06')
    if j >= 0:
        count, cd_size, cd_off = struct.unpack('<QQQ', tail[j + 32:j + 56])
    cd = _range(url, cd_off, cd_off + cd_size)
    out, p = {}, 0
    for _ in range(count):
        (sig, _, _, _, method, _, _, crc, csize, usize, nlen, elen, clen, _, _, _,
         off) = struct.unpack('<IHHHHHHIIIHHHHHII', cd[p:p + 46])
        if sig != 0x02014b50:
            raise IOError('bad central directory entry')
        name = cd[p + 46:p + 46 + nlen].decode('utf-8', 'replace')
        if 0xffffffff in (csize, usize, off):
            raise IOError(f'{name}: zip64 entry, use --usac-wav (full download)')
        out[name] = (method, crc, csize, usize, off)
        p += 46 + nlen + elen + clen
    return out


def fetch_usac_streams(jobs, cache, stats, rehash):
    st = stats['usac']
    entries = zip_directory(USAC_URL, USAC_SIZE)
    names = sorted(n for n in entries if n.startswith('compressedMp4/') and n.endswith('.mp4'))
    st['archive_mp4'] = len(names)
    st['archive_wav'] = sum(1 for n in entries if n.endswith('.wav'))

    def get(name):
        method, crc, csize, usize, off = entries[name]
        rel = name
        dst = os.path.join(DIR_USAC, rel)
        if os.path.exists(dst) and os.path.getsize(dst) == usize and 'usac/' + rel in cache:
            return rel, cached_md5(dst, 'usac/' + rel, cache, rehash), False
        h = _range(USAC_URL, off, off + 30)
        nlen, elen = struct.unpack('<HH', h[26:30])
        start = off + 30 + nlen + elen
        data = _range(USAC_URL, start, start + csize)
        if method == 8:
            data = zlib.decompress(data, -15)
        elif method != 0:
            raise IOError(f'{name}: compression method {method}')
        if len(data) != usize or zlib.crc32(data) != crc:
            raise IOError(f'{name}: CRC-32 or size mismatch')
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        with open(dst + '.part', 'wb') as f:
            f.write(data)
        os.replace(dst + '.part', dst)
        s = os.stat(dst)
        md5 = hashlib.md5(data).hexdigest()
        cache['usac/' + rel] = [s.st_size, s.st_mtime_ns, md5]
        return rel, md5, True

    with cf.ThreadPoolExecutor(jobs) as ex:
        for rel, _, new in ex.map(get, names):
            st['files'] += 1
            st['extracted'] += new
            st['crc_ok'] += 1


def extract_usac_full(archive, cache, stats, rehash):
    st = stats['usac']
    with zipfile.ZipFile(archive) as z:
        for i in z.infolist():
            if i.is_dir():
                continue
            rel = member_rel(i.filename)
            dst = os.path.join(DIR_USAC, rel)
            if os.path.exists(dst) and os.path.getsize(dst) == i.file_size:
                cached_md5(dst, 'usac/' + rel, cache, rehash)
            else:
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                hh = hashlib.md5()
                with z.open(i) as src, open(dst + '.part', 'wb') as out:
                    for chunk in iter(lambda: src.read(1 << 20), b''):
                        hh.update(chunk)
                        out.write(chunk)
                os.replace(dst + '.part', dst)
                s = os.stat(dst)
                cache['usac/' + rel] = [s.st_size, s.st_mtime_ns, hh.hexdigest()]
                st['extracted'] += 1
            st['files'] += 1
            st['crc_ok'] += 1


# --- index -------------------------------------------------------------------------------------

def fdk_asc_check(asc):
    """Static prediction of the 2.0.3 decoder's set-up from the ASC alone (see the module doc)."""
    if not asc:
        return 'no MPEG-4 audio config'
    aot = asc['aot']
    core = asc.get('core_aot', aot)
    if aot not in aacstream.FDK_DECODER_AOTS or (aot in (5, 29) and core != 2):
        return 'reject: object type'
    if core == 20 and asc.get('layer', 0) > 0:
        return 'reject: scalable layer > 0'
    if 'ep_config' in asc:
        if asc['ep_config'] > 1:
            return 'reject: epConfig > 1'
        if asc['channel_configuration'] == 0:
            return 'reject: ER with channelConfiguration 0'
    return 'accept'


def index_record(rel_from_conf, path, md5):
    name = os.path.basename(path)
    group, tool, role = classify(name)
    with open(path, 'rb') as f:
        data = f.read()
    info = aacstream.stream_info(path, data)
    asc = info.get('asc') or {}
    parts = rel_from_conf.split('/')
    rec = {
        'file': rel_from_conf,
        'size': len(data),
        'md5': md5,
        'set': parts[0],
        'dir': '/'.join(parts[1:-1]),
        'normative': 'add-opt' not in parts,
        'group': group,
        'tool': tool,
        'role': role,
        'container': info.get('container'),
        'aot': asc.get('aot'),
        'aot_name': asc.get('aot_name'),
        'core_aot': asc.get('core_aot', asc.get('aot')),
        'sample_rate': asc.get('sampling_rate'),
        'ext_sample_rate': asc.get('extension_sampling_rate'),
        'channel_config': asc.get('channel_configuration'),
        'channels': asc.get('channels'),
        'frame_length': asc.get('frame_length'),
        'frame_length_flag': asc.get('frame_length_flag'),
        'sbr': asc.get('sbr', 1 if asc.get('ld_sbr') else None),
        'ps': asc.get('ps'),
        'ep_config': asc.get('ep_config'),
        'vcb11': asc.get('vcb11'),
        'rvlc': asc.get('rvlc'),
        'hcr': asc.get('hcr'),
        'fdk_asc_check': fdk_asc_check(asc) if asc else 'no MPEG-4 audio config',
    }
    for k in ('layer', 'eld_ext', 'ld_sbr', 'asc_extensions', 'usac_sampling_frequency',
              'usac_channel_configuration_index', 'core_sbr_frame_length_index'):
        if k in asc:
            rec[k] = asc[k]
    if asc.get('parse_error'):
        rec['parse_error'] = asc['parse_error']
    for k in ('frames', 'crc', 'audio_mux_version', 'num_program', 'num_layer', 'num_sub_frames',
              'bitstream_type', 'parse_error', 'asc_hex'):
        if info.get(k) is not None:
            rec[k] = info[k]
    if 'tracks' in info:
        rec['tracks'] = len(info['tracks'])
        rec['audio_tracks'] = sum(1 for t in info['tracks'] if t.get('object_type_indication') == 0x40)
        other = sorted({(t.get('object_type_indication'), t.get('stream_type'))
                        for t in info['tracks'] if t.get('object_type_indication') != 0x40},
                       key=str)
        if other:
            rec['other_tracks'] = [{'oti': o, 'stream_type': s} for o, s in other]
    return rec


def build_index(cache, md5s, sources):
    recs = []
    for base in (DIR_26, DIR_USAC):
        if not os.path.isdir(base):
            continue
        for root, _, files in os.walk(base):
            if '_checksum' in root.split(os.sep):
                continue
            for fn in files:
                if not fn.endswith(MEDIA_EXT):
                    continue
                path = os.path.join(root, fn)
                rel = os.path.relpath(path, CONF).replace(os.sep, '/')
                if base == DIR_26:
                    key = os.path.relpath(path, DIR_26).replace(os.sep, '/')
                    h = cached_md5(path, key, cache)
                else:
                    key = 'usac/' + os.path.relpath(path, DIR_USAC).replace(os.sep, '/')
                    h = cached_md5(path, key, cache)
                recs.append(index_record(rel, path, h))
    recs.sort(key=lambda r: r['file'])
    out = {
        'generated': datetime.date.today().isoformat(),
        'md5_list': MD5_LIST,
        'sources': sources,
        'note': 'fdk_asc_check is a static prediction from the AudioSpecificConfig, not a '
                'measurement of the reference decoder.',
        'files': recs,
    }
    with open(INDEX + '.tmp', 'w') as f:
        json.dump(out, f, indent=1)
    os.replace(INDEX + '.tmp', INDEX)
    return out


def summary(idx):
    recs = idx['files']
    log(f'index: {len(recs)} bitstreams')
    c = collections.Counter((r['set'], r['dir'], r['role'], r['group']) for r in recs)
    for (s, d, role, g), n in sorted(c.items()):
        log(f'  {s:9} {d:22} {role:8} {g:13} {n:5}')
    roles = collections.Counter(r['role'] for r in recs)
    log('  by role: ' + ', '.join(f'{k} {v}' for k, v in sorted(roles.items())))
    chk = collections.Counter((r['role'], r['fdk_asc_check']) for r in recs)
    log('  role x fdk_asc_check: ' + '; '.join(f'{k[0]}/{k[1]}: {v}' for k, v in sorted(chk.items())))
    fl = collections.Counter((r['set'], r['group'], r['container'], r['frame_length'])
                             for r in recs if r.get('frame_length_flag') == 1)
    log(f'  frameLengthFlag = 1 (960 or 480 samples): {sum(fl.values())} bitstreams')
    for (s, g, cont, n), k in sorted(fl.items(), key=str):
        log(f'    {s:9} {g:13} {cont:5} {n:5} {k:5}')
    errs = [r['file'] for r in recs if r.get('parse_error')]
    if errs:
        log(f'  header parse errors: {len(errs)} (e.g. {errs[:3]})')


def er_coverage(idx):
    """Markdown tables of the error-resilience configuration of the ER AAC bitstreams (object
    types 17, 19, 20, 23, 39): the (VCB11, RVLC, HCR) combinations, epConfig, frame length,
    sampling rate and channel configuration, per group and container."""
    recs = [r for r in idx['files'] if r.get('core_aot') in (17, 19, 20, 23, 39)]
    out = []

    def table(head, rows):
        out.append('| ' + ' | '.join(head) + ' |')
        out.append('|' + '|'.join('---' for _ in head) + '|')
        for row in rows:
            out.append('| ' + ' | '.join(str(x) for x in row) + ' |')
        out.append('')

    def where(r):
        return f'{r["dir"]} ({r["container"]})'

    out.append('### Bitstreams per group, object type and container\n')
    c = collections.Counter((r['group'], r['core_aot'], where(r)) for r in recs)
    table(['Group', 'AOT', 'Directory (container)', 'Files'],
          [(g, a, w, n) for (g, a, w), n in sorted(c.items())])

    combos = [(v, rv, h) for v in (0, 1) for rv in (0, 1) for h in (0, 1)]
    groups = sorted({(r['group'], r['core_aot']) for r in recs})
    out.append('### (VCB11, RVLC, HCR) combinations\n')
    out.append('VCB11 = aacSectionDataResilienceFlag, RVLC = aacScalefactorDataResilienceFlag, '
               'HCR = aacSpectralDataResilienceFlag. Counts of files; MP4 and LOAS/LATM '
               'separately.\n')
    head = ['VCB11', 'RVLC', 'HCR'] + [f'{g} (AOT {a}) {cont}' for g, a in groups
                                       for cont in ('mp4', 'loas')]
    rows = []
    for combo in combos:
        row = list(combo)
        for g, a in groups:
            for cont in ('mp4', 'loas'):
                row.append(sum(1 for r in recs if r['group'] == g and r['core_aot'] == a and
                               r['container'] == cont and
                               (r.get('vcb11'), r.get('rvlc'), r.get('hcr')) == combo))
        rows.append(row)
    table(head, rows)
    unk = [r['file'] for r in recs if r.get('vcb11') is None]
    if unk:
        out.append(f'Files without parsed flags: {len(unk)} ({", ".join(unk[:5])}).\n')

    out.append('### Files per combination (ER AAC LC, all containers)\n')
    rows = []
    for combo in combos:
        names = sorted({os.path.basename(r['file']).rsplit('.', 1)[0] for r in recs
                        if r['core_aot'] == 17 and
                        (r.get('vcb11'), r.get('rvlc'), r.get('hcr')) == combo})
        stems = sorted({re.sub(r'_(amv\d)$', '', n) for n in names})
        rows.append(list(combo) + [len(names), ', '.join(stems) if stems else '-'])
    table(['VCB11', 'RVLC', 'HCR', 'Files', 'Streams (without the _amv0/_amv1 suffix)'], rows)

    for field, title in (('ep_config', 'epConfig'), ('frame_length', 'Frame length'),
                         ('sample_rate', 'Sampling rate'), ('channel_config', 'channelConfiguration'),
                         ('fdk_asc_check', 'Static ASC check (fdk_asc_check)')):
        out.append(f'### {title}\n')
        c = collections.Counter((r['group'], r['core_aot'], r['container'], r.get(field))
                                for r in recs)
        table(['Group', 'AOT', 'Container', title, 'Files'],
              [(g, a, cont, v, n) for (g, a, cont, v), n in
               sorted(c.items(), key=lambda kv: tuple(str(x) for x in kv[0]))])
    out.append('### MPEG-4 audio tracks per MP4 file, by epConfig\n')
    c = collections.Counter((r['group'], r['core_aot'], r.get('ep_config'), r.get('audio_tracks'))
                            for r in recs if r['container'] == 'mp4')
    table(['Group', 'AOT', 'epConfig', 'Audio tracks', 'Files'],
          [(g, a, e, t, n) for (g, a, e, t), n in
           sorted(c.items(), key=lambda kv: tuple(str(x) for x in kv[0]))])
    return '\n'.join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--with', dest='extra', action='append', default=[],
                    choices=sorted(ARCHIVES_26), help='also fetch this 14496-26 archive')
    ap.add_argument('--only', action='append', default=[], choices=sorted(ARCHIVES_26),
                    help='fetch only these 14496-26 archives (instead of the default four)')
    ap.add_argument('--references-wav', action='store_true',
                    help='also fetch referencesWav.zip (9.8 GB)')
    ap.add_argument('--usac', action='store_true',
                    help='fetch the ISO/IEC 23003-7 bitstreams by range requests (140 MB)')
    ap.add_argument('--usac-wav', action='store_true',
                    help='download the whole ISO/IEC 23003-7 archive (3.5 GB) incl. waveforms')
    ap.add_argument('--jobs', type=int, default=2, help='parallel downloads (default 2)')
    ap.add_argument('--offline', action='store_true', help='do not download, use what is there')
    ap.add_argument('--rehash', action='store_true', help='recompute all MD5s')
    ap.add_argument('--keep-going', action='store_true', help='exit 0 on MD5 mismatches')
    ap.add_argument('--summary', action='store_true', help='print the summary of index.json')
    ap.add_argument('--er-coverage', action='store_true',
                    help='print the ER AAC resilience-tool coverage of index.json (Markdown)')
    args = ap.parse_args()

    if args.summary or args.er_coverage:
        with open(INDEX) as f:
            idx = json.load(f)
        if args.summary:
            summary(idx)
        if args.er_coverage:
            print(er_coverage(idx))
        return 0

    os.makedirs(ARCH_DIR, exist_ok=True)
    md5_path = download(BASE_26 + MD5_LIST, os.path.join(CONF, MD5_LIST), offline=args.offline)
    md5s = read_md5_list(md5_path)
    log(f'{MD5_LIST}: {len(md5s)} entries')

    wanted = list(args.only or DEFAULT_26)
    wanted += [a for a in args.extra if a not in wanted]
    if args.references_wav and 'referencesWav.zip' not in wanted:
        wanted.append('referencesWav.zip')
    jobs = max(1, args.jobs)
    with cf.ThreadPoolExecutor(jobs) as ex:
        paths = list(ex.map(lambda a: download(BASE_26 + a, os.path.join(ARCH_DIR, a),
                                               ARCHIVES_26[a], args.offline), wanted))

    cache = load_cache()
    stats = collections.defaultdict(lambda: {'files': 0, 'extracted': 0, 'md5_ok': 0, 'crc_ok': 0,
                                             'md5_mismatch': [], 'not_in_md5_list': [],
                                             'md5sum_file_disagrees': []})
    for p in paths:
        log(f'extracting and verifying {os.path.basename(p)}')
        extract_26(p, md5s, cache, stats, args.rehash)
        save_cache(cache)

    sources = {'14496-26': {'url': BASE_26, 'md5_list': MD5_LIST, 'archives': wanted}}
    if args.usac_wav:
        p = download(USAC_URL, os.path.join(ARCH_DIR, os.path.basename(USAC_URL)), USAC_SIZE,
                     args.offline)
        extract_usac_full(p, cache, stats, args.rehash)
        sources['23003-7'] = {'url': USAC_URL, 'mode': 'full archive'}
    elif args.usac:
        if not args.offline:
            fetch_usac_streams(jobs, cache, stats, args.rehash)
        sources['23003-7'] = {'url': USAC_URL, 'mode': 'bitstreams by range requests'}
    if args.usac or args.usac_wav:
        if not args.offline:
            download(USAC_TABLES_URL, os.path.join(CONF, os.path.basename(USAC_TABLES_URL)))
    save_cache(cache)

    bad = 0
    for name, st in sorted(stats.items()):
        if name == 'usac':
            log(f'usac (23003-7): {st["files"]} files ({st["extracted"]} newly extracted), '
                f'CRC-32 ok {st["crc_ok"]} (no MD5 list published)')
            continue
        log(f'{name}: {st["files"]} files ({st["extracted"]} newly extracted), '
            f'MD5 ok {st["md5_ok"]}, MD5 mismatches {len(st["md5_mismatch"])}, '
            f'not in the MD5 list {len(st["not_in_md5_list"])}, '
            f'in the list but not in the archive {len(st.get("in_md5_list_not_in_archive", []))}, '
            f'_checksum files disagreeing {len(st["md5sum_file_disagrees"])}')
        for k in ('md5_mismatch', 'not_in_md5_list', 'in_md5_list_not_in_archive',
                  'md5sum_file_disagrees'):
            for r in st.get(k, [])[:20]:
                log(f'    {k}: {r}')
        bad += len(st['md5_mismatch'])
    with open(os.path.join(CONF, 'verify.json'), 'w') as f:
        json.dump(stats, f, indent=1, sort_keys=True)

    idx = build_index(cache, md5s, sources)
    save_cache(cache)
    summary(idx)
    log(f'wrote {INDEX}')
    return 1 if bad and not args.keep_going else 0


if __name__ == '__main__':
    sys.exit(main())
