"""Header parsers for MPEG-4 Audio streams, used by fetch_conformance.py and corpus.py.

Reads the AudioSpecificConfig (ISO/IEC 14496-3, 1.6.2.1) of MP4 files (from the `esds` box of
each track), LOAS/LATM AudioSyncStreams (from the first StreamMuxConfig), ADTS (fixed header) and
ADIF (header with its program_config_elements). The parsers cover what the corpus index needs:
audio object type, sampling rates, channels, frame length and the error-resilience fields
(epConfig, aacSectionDataResilienceFlag, aacScalefactorDataResilienceFlag,
aacSpectralDataResilienceFlag). Specific configurations of object types other than the
GASpecificConfig family, ELD and USAC are not parsed.

Python 3 standard library only.
"""
import struct

SAMPLING_RATES = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025,
                  8000, 7350, None, None, None]

# USAC (ISO/IEC 23003-3, table 72): usacSamplingFrequencyIndex 0x00..0x0c as above, 0x0f..0x1b
USAC_SAMPLING_RATES = SAMPLING_RATES[:13] + [None, None, 57600, 51200, 40000, 38400, 34150,
                                             28800, 25600, 20000, 19200, 17075, 14400, 12800,
                                             9600, None, None, None, None]

# channelConfiguration -> number of channels (ISO/IEC 14496-3:2019, table 1.19)
CHANNELS = {1: 1, 2: 2, 3: 3, 4: 4, 5: 5, 6: 6, 7: 8, 8: 2, 9: 3, 10: 4, 11: 7, 12: 8, 13: 24,
            14: 8}

AOT_NAMES = {
    0: 'NULL', 1: 'AAC Main', 2: 'AAC LC', 3: 'AAC SSR', 4: 'AAC LTP', 5: 'SBR',
    6: 'AAC scalable', 7: 'TwinVQ', 8: 'CELP', 9: 'HVXC', 12: 'TTSI', 13: 'Main synthetic',
    14: 'Wavetable synthesis', 15: 'General MIDI', 16: 'Algorithmic synthesis and audio FX',
    17: 'ER AAC LC', 19: 'ER AAC LTP', 20: 'ER AAC scalable', 21: 'ER TwinVQ', 22: 'ER BSAC',
    23: 'ER AAC LD', 24: 'ER CELP', 25: 'ER HVXC', 26: 'ER HILN', 27: 'ER Parametric',
    28: 'SSC', 29: 'PS', 30: 'MPEG Surround', 32: 'Layer-1', 33: 'Layer-2', 34: 'Layer-3',
    35: 'DST', 36: 'ALS', 37: 'SLS', 38: 'SLS non-core', 39: 'ER AAC ELD', 40: 'SMR simple',
    41: 'SMR main', 42: 'USAC', 43: 'SAOC', 44: 'LD MPEG Surround', 45: 'SAOC-DE',
}

GA_AOTS = {1, 2, 3, 4, 6, 7, 17, 19, 20, 21, 22, 23}
ER_AOTS = {17, 19, 20, 21, 22, 23, 24, 25, 26, 27, 39}

# The object types for which the fdk-aac 2.0.3 decoder sets up a decoder
# (CAacDecoder_Init, libAACdec/src/aacdecoder.cpp; AOT_ER_AAC_SCAL only with layer 0).
FDK_DECODER_AOTS = {2, 5, 29, 17, 20, 23, 39, 42}


class BitReader:
    def __init__(self, data, pos=0):
        self.data = data
        self.pos = pos  # in bits

    def bits_left(self):
        return len(self.data) * 8 - self.pos

    def read(self, n):
        if n == 0:
            return 0
        if self.pos + n > len(self.data) * 8:
            raise EOFError('read past the end of the data')
        v = 0
        for _ in range(n):
            byte = self.data[self.pos >> 3]
            v = (v << 1) | ((byte >> (7 - (self.pos & 7))) & 1)
            self.pos += 1
        return v

    def skip(self, n):
        if self.pos + n > len(self.data) * 8:
            raise EOFError('skip past the end of the data')
        self.pos += n

    def align(self, anchor=0):
        r = (self.pos - anchor) % 8
        if r:
            self.skip(8 - r)


def _aot(br):
    a = br.read(5)
    if a == 31:
        a = 32 + br.read(6)
    return a


def _rate(br):
    idx = br.read(4)
    if idx == 15:
        return idx, br.read(24)
    return idx, SAMPLING_RATES[idx]


def parse_pce(br, anchor):
    """program_config_element(); returns a dict with the channel count."""
    p = {'element_instance_tag': br.read(4), 'object_type': br.read(2)}
    p['sampling_frequency_index'] = br.read(4)
    nf, ns, nb = br.read(4), br.read(4), br.read(4)
    nl, na, nc = br.read(2), br.read(3), br.read(4)
    if br.read(1):
        br.skip(4)  # mono_mixdown_element_number
    if br.read(1):
        br.skip(4)  # stereo_mixdown_element_number
    if br.read(1):
        br.skip(3)  # matrix_mixdown_idx, pseudo_surround_enable
    ch = 0
    for _ in range(nf + ns + nb):
        ch += 2 if br.read(1) else 1
        br.skip(4)
    br.skip(4 * nl + 4 * na + 5 * nc)
    br.align(anchor)
    br.skip(8 * br.read(8))  # comment_field_data
    p.update(front=nf, side=ns, back=nb, lfe=nl, assoc_data=na, cc=nc, channels=ch + nl)
    return p


def _ga_specific(br, asc, aot, anchor):
    asc['frame_length_flag'] = br.read(1)
    if br.read(1):  # dependsOnCoreCoder
        asc['core_coder_delay'] = br.read(14)
    ext = br.read(1)
    if asc['channel_configuration'] == 0:
        asc['pce'] = parse_pce(br, anchor)
    if aot in (6, 20):
        asc['layer'] = br.read(3)
    if ext:
        if aot == 22:
            asc['num_of_sub_frame'] = br.read(5)
            asc['layer_length'] = br.read(11)
        if aot in (17, 19, 20, 23):
            asc['vcb11'] = br.read(1)  # aacSectionDataResilienceFlag
            asc['rvlc'] = br.read(1)  # aacScalefactorDataResilienceFlag
            asc['hcr'] = br.read(1)  # aacSpectralDataResilienceFlag
        asc['extension_flag3'] = br.read(1)
    elif aot in (17, 19, 20, 23):
        asc['vcb11'] = asc['rvlc'] = asc['hcr'] = 0


def _sbr_header(br):
    br.skip(1 + 4 + 4 + 3 + 2)  # bs_amp_res, start, stop, xover_band, reserved
    extra1, extra2 = br.read(1), br.read(1)
    if extra1:
        br.skip(2 + 1 + 2)
    if extra2:
        br.skip(2 + 2 + 1 + 1)


def _eld_specific(br, asc):
    asc['frame_length_flag'] = br.read(1)
    asc['vcb11'] = br.read(1)
    asc['rvlc'] = br.read(1)
    asc['hcr'] = br.read(1)
    asc['ld_sbr'] = br.read(1)
    if asc['ld_sbr']:
        asc['ld_sbr_sampling_rate'] = br.read(1)  # 0: single rate, 1: dual rate
        asc['ld_sbr_crc'] = br.read(1)
        asc['sbr'] = 1
        if asc.get('sampling_rate'):
            asc['extension_sampling_rate'] = asc['sampling_rate'] << asc['ld_sbr_sampling_rate']
        n = {1: 1, 2: 1, 3: 2, 4: 3, 5: 3, 6: 3, 7: 4, 11: 4, 12: 4, 14: 4}.get(
            asc['channel_configuration'], 0)
        for _ in range(n):
            _sbr_header(br)
    exts = []
    while True:
        t = br.read(4)
        if t == 0:  # ELDEXT_TERM
            break
        n = br.read(4)
        if n == 15:
            m = br.read(8)
            n += m
            if m == 255:
                n += br.read(16)
        exts.append({2: 'LDSAC', 3: 'DOWNSCALEINFO'}.get(t, t))
        br.skip(8 * n)
    asc['eld_ext'] = exts


def _usac_config(br, asc):
    idx = br.read(5)
    rate = br.read(24) if idx == 0x1f else USAC_SAMPLING_RATES[idx]
    asc['usac_sampling_frequency'] = rate
    asc['core_sbr_frame_length_index'] = br.read(3)
    asc['usac_channel_configuration_index'] = br.read(5)


def _extensions(br, asc, ext_aot):
    """The extensions after the specific configuration, in the order of fdk-aac's
    AudioSpecificConfig_ExtensionParse: SBR (0x2b7), PS (0x548), MPEG Surround (0x76a),
    LD MPEG Surround (0x7cc), SAOC (0x7cb). Stops at the first other value. The data are not
    validated: trailing bits that happen to match a sync word are read as an extension, as the
    reference does."""
    last = None
    ext_id = None
    exts = []
    while br.bits_left() >= 11:
        last = ext_id
        save = br.pos
        ext_id = br.read(11)
        if ext_id == 0x2b7:
            if ext_aot != 5 and br.bits_left() >= 5:
                ext_aot = _aot(br)
                exts.append('SBR')
                if ext_aot in (5, 22):
                    asc['sbr'] = br.read(1)
                    asc['implicit_backward_compatible'] = 1
                    if asc['sbr']:
                        _, asc['extension_sampling_rate'] = _rate(br)
                    if ext_aot == 22:
                        asc['extension_channel_configuration'] = br.read(4)
        elif ext_id == 0x548:
            exts.append('PS')
            if last == 0x2b7 and ext_aot == 5 and br.bits_left() > 0:
                asc['ps'] = br.read(1)
        elif ext_id in (0x76a, 0x7cc, 0x7cb):
            name = {0x76a: 'MPS', 0x7cc: 'LDMPS', 0x7cb: 'SAOC'}[ext_id]
            if br.bits_left() >= 1 and br.read(1):
                n = br.read(8)
                if n == 0xff:
                    n += br.read(16)
                br.skip(8 * n)
                exts.append(name)
                asc[name.lower() + '_present'] = 1
            else:
                exts.append(name + '(absent)')
        else:
            br.pos = save
            break
    if exts:
        asc['asc_extensions'] = exts


def parse_asc(data, bit_offset=0):
    """Parses an AudioSpecificConfig. Returns a dict; `asc_bits` is its parsed length in bits
    (exact for the object types whose specific configuration is parsed)."""
    br = BitReader(data, bit_offset)
    anchor = br.pos
    asc = {}
    aot = _aot(br)
    asc['aot'] = aot
    asc['sampling_frequency_index'], asc['sampling_rate'] = _rate(br)
    asc['channel_configuration'] = br.read(4)
    ext_aot = 0
    if aot in (5, 29):
        ext_aot = 5
        asc['sbr'] = 1
        asc['ps'] = 1 if aot == 29 else 0
        asc['explicit_hierarchical'] = aot
        _, asc['extension_sampling_rate'] = _rate(br)
        aot = _aot(br)
        asc['core_aot'] = aot
        if aot == 22:
            asc['extension_channel_configuration'] = br.read(4)
    asc['specific_config_parsed'] = True
    try:
        if aot in GA_AOTS:
            _ga_specific(br, asc, aot, anchor)
        elif aot == 39:
            _eld_specific(br, asc)
        elif aot == 42:
            _usac_config(br, asc)
        else:
            asc['specific_config_parsed'] = False
        if asc['specific_config_parsed'] and aot in ER_AOTS:
            asc['ep_config'] = br.read(2)
            if asc['ep_config'] >= 2:
                asc['asc_bits'] = None  # ErrorProtectionSpecificConfig not parsed
                return _finish(asc)
        if asc['specific_config_parsed'] and aot != 42:
            _extensions(br, asc, ext_aot)
    except EOFError as e:
        asc['parse_error'] = str(e)
    asc['asc_bits'] = br.pos - anchor if asc['specific_config_parsed'] else None
    return _finish(asc)


def _finish(asc):
    aot = asc['aot']
    core = asc.get('core_aot', aot)
    cc = asc['channel_configuration']
    if cc == 0 and 'pce' in asc:
        asc['channels'] = asc['pce']['channels']
    elif core == 42 and 'usac_channel_configuration_index' in asc:
        asc['channels'] = CHANNELS.get(asc['usac_channel_configuration_index'])
    else:
        asc['channels'] = CHANNELS.get(cc)
    flf = asc.get('frame_length_flag')
    if core in (23, 39):
        asc['frame_length'] = None if flf is None else (480 if flf else 512)
    elif core in GA_AOTS:
        asc['frame_length'] = None if flf is None else (960 if flf else 1024)
    elif core == 42 and 'core_sbr_frame_length_index' in asc:
        asc['frame_length'] = 768 if asc['core_sbr_frame_length_index'] == 0 else 1024
    asc['aot_name'] = AOT_NAMES.get(aot, str(aot))
    return asc


# --- MP4 ---------------------------------------------------------------------------------------

def _boxes(data, start, end):
    p = start
    while p + 8 <= end:
        size, typ = struct.unpack('>I4s', data[p:p + 8])
        hdr = 8
        if size == 1:
            size = struct.unpack('>Q', data[p + 8:p + 16])[0]
            hdr = 16
        elif size == 0:
            size = end - p
        if size < hdr or p + size > end:
            return
        yield typ.decode('latin1'), p + hdr, p + size
        p += size


def _desc_len(data, p):
    n = 0
    for _ in range(4):
        b = data[p]
        p += 1
        n = (n << 7) | (b & 0x7f)
        if not b & 0x80:
            break
    return n, p


def parse_esds(payload):
    """payload: the esds box body (after version/flags). Returns OTI, stream type and the
    DecoderSpecificInfo bytes."""
    out = {}
    p = 0
    if payload[p] != 0x03:
        return out
    n, p = _desc_len(payload, p + 1)
    p += 2
    flags = payload[p]
    p += 1
    if flags & 0x80:
        p += 2
    if flags & 0x40:
        p += 1 + payload[p]
    if flags & 0x20:
        p += 2
    if payload[p] != 0x04:
        return out
    n, p = _desc_len(payload, p + 1)
    end = p + n
    out['object_type_indication'] = payload[p]
    out['stream_type'] = payload[p + 1] >> 2
    q = p + 13
    if q < end and payload[q] == 0x05:
        n, q = _desc_len(payload, q + 1)
        out['decoder_specific_info'] = bytes(payload[q:q + n])
    return out


SAMPLE_ENTRY_SKIP = {'mp4a': 28, 'mp4s': 8, 'enca': 28}


def parse_mp4(data):
    """Returns {'tracks': [...]} with handler, sample entry, OTI, stream type, ASC and the
    number of samples of each track."""
    tracks = []
    for t, s, e in _boxes(data, 0, len(data)):
        if t != 'moov':
            continue
        for t2, s2, e2 in _boxes(data, s, e):
            if t2 != 'trak':
                continue
            tr = {}
            _walk_trak(data, s2, e2, tr)
            tracks.append(tr)
    return {'tracks': tracks}


def _walk_trak(data, s, e, tr):
    for t, s2, e2 in _boxes(data, s, e):
        if t in ('mdia', 'minf', 'stbl', 'edts', 'dinf'):
            _walk_trak(data, s2, e2, tr)
        elif t == 'tkhd':
            v = data[s2]
            tr['track_id'] = struct.unpack('>I', data[s2 + (20 if v == 1 else 12):][:4])[0]
        elif t == 'hdlr':
            tr['handler'] = data[s2 + 8:s2 + 12].decode('latin1')
        elif t == 'mdhd':
            v = data[s2]
            if v == 1:
                ts, dur = struct.unpack('>IQ', data[s2 + 20:s2 + 32])
            else:
                ts, dur = struct.unpack('>II', data[s2 + 12:s2 + 20])
            tr['timescale'], tr['duration'] = ts, dur
        elif t == 'stsz':
            tr['samples'] = struct.unpack('>I', data[s2 + 8:s2 + 12])[0]
        elif t == 'tref':
            for t3, s3, e3 in _boxes(data, s2, e2):
                tr.setdefault('tref', {})[t3] = list(struct.unpack('>%dI' % ((e3 - s3) // 4),
                                                                   data[s3:e3]))
        elif t == 'stsd':
            for t3, s3, e3 in _boxes(data, s2 + 8, e2):
                tr['sample_entry'] = t3
                skip = SAMPLE_ENTRY_SKIP.get(t3, 28)
                if t3 in ('mp4a', 'enca'):
                    ver = struct.unpack('>H', data[s3 + 8:s3 + 10])[0]
                    skip += {1: 16, 2: 36}.get(ver, 0)
                for t4, s4, e4 in _boxes(data, s3 + skip, e3):
                    if t4 == 'esds':
                        tr.update(parse_esds(data[s4 + 4:e4]))
                break


def mp4_info(data):
    """Index fields of an MP4 file: the first MPEG-4 audio track (OTI 0x40) and a summary of all
    tracks."""
    info = {'container': 'mp4'}
    m = parse_mp4(data)
    tracks = []
    first = None
    for tr in m['tracks']:
        d = {k: tr.get(k) for k in ('track_id', 'handler', 'sample_entry', 'samples')}
        d['object_type_indication'] = tr.get('object_type_indication')
        d['stream_type'] = tr.get('stream_type')
        dsi = tr.get('decoder_specific_info')
        if tr.get('object_type_indication') == 0x40 and dsi:
            d['asc_hex'] = dsi.hex()
            d['asc'] = parse_asc(dsi)
            if first is None:
                first = d
        if 'tref' in tr:
            d['tref'] = tr['tref']
        tracks.append(d)
    info['tracks'] = tracks
    if first is not None:
        info['asc'] = first['asc']
        info['asc_hex'] = first['asc_hex']
        info['frames'] = first['samples']
    return info


# --- ADTS, ADIF, LOAS --------------------------------------------------------------------------

def adts_info(data):
    info = {'container': 'adts'}
    p = frames = blocks = 0
    first = None
    while p + 7 <= len(data):
        br = BitReader(data, p * 8)
        if br.read(12) != 0xfff:
            info['resync_error_at'] = p
            break
        h = {'id': br.read(1), 'layer': br.read(2), 'protection_absent': br.read(1),
             'profile': br.read(2), 'sampling_frequency_index': br.read(4)}
        br.skip(1)
        h['channel_configuration'] = br.read(3)
        br.skip(4)
        length = br.read(13)
        br.skip(11)
        nblocks = br.read(2) + 1
        if first is None:
            first = h
        if length < 7:
            info['bad_frame_length_at'] = p
            break
        frames += 1
        blocks += nblocks
        p += length
    info['frames'] = frames
    info['raw_data_blocks'] = blocks
    if first:
        info['crc'] = 0 if first['protection_absent'] else 1
        info['mpeg_id'] = first['id']
        aot = first['profile'] + 1
        info['asc'] = _finish({'aot': aot, 'sampling_frequency_index':
                               first['sampling_frequency_index'],
                               'sampling_rate': SAMPLING_RATES[first['sampling_frequency_index']],
                               'channel_configuration': first['channel_configuration'],
                               'frame_length_flag': 0})
    return info


def adif_info(data):
    info = {'container': 'adif'}
    br = BitReader(data)
    if br.read(32) != 0x41444946:
        info['parse_error'] = 'no ADIF header'
        return info
    if br.read(1):
        br.skip(72)
    br.skip(2)  # original_copy, home
    bstype = br.read(1)
    info['bitstream_type'] = 'VBR' if bstype else 'CBR'
    info['bitrate'] = br.read(23)
    npce = br.read(4) + 1
    pces = []
    for _ in range(npce):
        if bstype == 0:
            br.skip(20)
        pces.append(parse_pce(br, 0))
    info['pces'] = len(pces)
    p0 = pces[0]
    info['asc'] = _finish({'aot': p0['object_type'] + 1,
                           'sampling_frequency_index': p0['sampling_frequency_index'],
                           'sampling_rate': SAMPLING_RATES[p0['sampling_frequency_index']],
                           'channel_configuration': 0, 'pce': p0, 'frame_length_flag': 0})
    return info


def _latm_value(br):
    n = br.read(2)
    return br.read(8 * (n + 1))


def loas_info(data):
    """AudioSyncStream (LOAS): counts the frames and parses the first StreamMuxConfig."""
    info = {'container': 'loas'}
    p = frames = 0
    # Skip leading bytes before the first sync word whose frame is followed by another sync
    # word (the 14496-26 2nd edition .ass files start with the 9 bytes ".mp4\nASS\0").
    for q in range(min(len(data) - 3, 4096)):
        if data[q] == 0x56 and data[q + 1] & 0xe0 == 0xe0:
            nxt = q + 3 + (((data[q + 1] & 0x1f) << 8) | data[q + 2])
            if nxt == len(data) or (nxt + 1 < len(data) and data[nxt] == 0x56 and
                                    data[nxt + 1] & 0xe0 == 0xe0):
                p = q
                break
    if p:
        info['leading_bytes'] = data[:p].hex()
    while p + 3 <= len(data):
        br = BitReader(data, p * 8)
        if br.read(11) != 0x2b7:
            info['resync_error_at'] = p
            break
        length = br.read(13)
        if 'asc' not in info and 'parse_error' not in info:
            try:
                _stream_mux_config(br, info)
            except EOFError as e:
                info['parse_error'] = str(e)
        frames += 1
        p += 3 + length
    info['frames'] = frames
    return info


def _stream_mux_config(br, info):
    if br.read(1):  # useSameStreamMux
        return
    ver = br.read(1)
    info['audio_mux_version'] = ver
    if ver == 1:
        info['audio_mux_version_a'] = br.read(1)
        if info['audio_mux_version_a']:
            info['parse_error'] = 'audioMuxVersionA = 1'
            return
        _latm_value(br)  # taraBufferFullness
    info['all_streams_same_time_framing'] = br.read(1)
    info['num_sub_frames'] = br.read(6) + 1
    info['num_program'] = br.read(4) + 1
    info['num_layer'] = br.read(3) + 1
    if ver == 0:
        start = br.pos
        asc = parse_asc(br.data, start)
        info['asc'] = asc
        info['asc_hex'] = None
    else:
        n = _latm_value(br)
        start = br.pos
        asc = parse_asc(br.data, start)
        asc['asc_len_bits'] = n
        info['asc'] = asc
        if n % 8 == 0 and start % 8 == 0:
            info['asc_hex'] = bytes(br.data[start // 8:start // 8 + n // 8]).hex()


def stream_info(path, data=None):
    """Dispatch by file extension."""
    if data is None:
        with open(path, 'rb') as f:
            data = f.read()
    ext = path.rsplit('.', 1)[-1].lower()
    try:
        if ext in ('mp4', 'm4a'):
            return mp4_info(data)
        if ext == 'adts' or ext == 'aac':
            return adts_info(data)
        if ext == 'adif':
            return adif_info(data)
        if ext in ('ass', 'loas', 'latm'):
            return loas_info(data)
    except (EOFError, IndexError, struct.error, KeyError, TypeError) as e:
        return {'container': ext, 'parse_error': f'{type(e).__name__}: {e}'}
    return {'container': ext}


# --- paths -------------------------------------------------------------------------------------

def work_dir():
    """The git-ignored work/ directory: $FDK_AAC_RS_WORK_DIR, else work/ of the main checkout (git
    worktrees share it), else <repo>/work."""
    import os
    import subprocess
    if os.environ.get('FDK_AAC_RS_WORK_DIR'):
        return os.environ['FDK_AAC_RS_WORK_DIR']
    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    try:
        common = subprocess.run(['git', '-C', repo, 'rev-parse', '--path-format=absolute',
                                 '--git-common-dir'], capture_output=True, text=True,
                                check=True).stdout.strip()
        cand = os.path.join(os.path.dirname(common), 'work')
        if os.path.isdir(cand):
            return cand
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
    return os.path.join(repo, 'work')
