/* Minimal MP4/M4A demuxer of fdkdec-ref. See mp4demux.h and PORTING.md, "Reference harness".
 *
 * Supported: ISO BMFF / QuickTime files held in memory; moov/trak/mdia/(hdlr, minf/stbl);
 * stsd with an 'mp4a' sample entry (versions 0, 1 and 2, esds directly or inside 'wave');
 * stsz/stz2, stco/co64, stsc; movie fragments (mvex/trex, moof/traf/tfhd/trun). Not supported:
 * several sample descriptions, edit lists (ignored: every sample is decoded), encrypted entries,
 * external data references, 'sdtp'/'sbgp' semantics. */
#include "mp4demux.h"

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define FOURCC(a, b, c, d) \
  (((uint32_t)(a) << 24) | ((uint32_t)(b) << 16) | ((uint32_t)(c) << 8) | (uint32_t)(d))

typedef struct {
  size_t start;   /* file offset of the box header */
  size_t payload; /* file offset of the payload */
  size_t end;     /* file offset after the box */
  uint32_t type;
} Box;

typedef struct {
  const uint8_t *d;
  size_t size;
  char *err;
  size_t errlen;
} Ctx;

static int fail(Ctx *c, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(c->err, c->errlen, fmt, ap);
  va_end(ap);
  return -1;
}

static uint32_t rd16(const uint8_t *p) { return ((uint32_t)p[0] << 8) | p[1]; }
static uint32_t rd32(const uint8_t *p) {
  return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}
static uint64_t rd64(const uint8_t *p) { return ((uint64_t)rd32(p) << 32) | rd32(p + 4); }

/* Reads the box at *pos within [*pos, end). Returns 1 on success, 0 at the end, -1 on error. */
static int next_box(Ctx *c, size_t *pos, size_t end, Box *b) {
  uint64_t size;
  size_t hdr = 8;
  if (*pos >= end) return 0;
  if (end - *pos < 8) return fail(c, "truncated box header at %zu", *pos);
  size = rd32(c->d + *pos);
  b->type = rd32(c->d + *pos + 4);
  if (size == 1) {
    if (end - *pos < 16) return fail(c, "truncated box header at %zu", *pos);
    size = rd64(c->d + *pos + 8);
    hdr = 16;
  } else if (size == 0) {
    size = end - *pos;
  }
  if (size < hdr || size > end - *pos) return fail(c, "invalid box size at %zu", *pos);
  b->start = *pos;
  b->payload = *pos + hdr;
  b->end = *pos + (size_t)size;
  *pos = b->end;
  return 1;
}

/* Finds the first child of type `type` in [from, end). Returns 1, 0 if absent, -1 on error. */
static int find_child(Ctx *c, size_t from, size_t end, uint32_t type, Box *out) {
  size_t pos = from;
  Box b;
  int r;
  while ((r = next_box(c, &pos, end, &b)) > 0) {
    if (b.type == type) {
      *out = b;
      return 1;
    }
  }
  return r;
}

static int need(Ctx *c, const Box *b, size_t n, const char *what) {
  if (b->end - b->payload < n) return fail(c, "truncated '%s' box", what);
  return 0;
}

static int add_sample(Ctx *c, Mp4Track *t, uint64_t off, uint32_t size) {
  if (off > c->size || size > c->size - off)
    return fail(c, "sample %u (offset %llu, size %u) beyond the end of the file", t->n_samples,
                (unsigned long long)off, size);
  if (t->n_samples == t->cap_samples) {
    t->cap_samples = t->cap_samples ? t->cap_samples * 2 : 1024;
    t->samples = (Mp4Sample *)realloc(t->samples, t->cap_samples * sizeof(Mp4Sample));
    if (!t->samples) return fail(c, "out of memory");
  }
  t->samples[t->n_samples].offset = off;
  t->samples[t->n_samples].size = size;
  t->n_samples++;
  return 0;
}

/* Reads an expandable descriptor length (ISO/IEC 14496-1, 8.3.3). */
static int desc_len(const uint8_t *p, size_t avail, size_t *used, uint32_t *len) {
  uint32_t v = 0;
  size_t i;
  for (i = 0; i < 4 && i < avail; i++) {
    v = (v << 7) | (p[i] & 0x7f);
    if (!(p[i] & 0x80)) {
      *used = i + 1;
      *len = v;
      return 0;
    }
  }
  return -1;
}

static int parse_esds(Ctx *c, const Box *b, Mp4Track *t) {
  const uint8_t *p = c->d + b->payload + 4; /* version and flags */
  const uint8_t *e = c->d + b->end;
  size_t used;
  uint32_t len;
  if (need(c, b, 4, "esds")) return -1;
  /* ES_Descriptor */
  if (p >= e || *p != 0x03) return fail(c, "esds: no ES_Descriptor");
  if (desc_len(p + 1, (size_t)(e - p - 1), &used, &len)) return fail(c, "esds: bad length");
  p += 1 + used;
  if (len > (size_t)(e - p) || len < 3) return fail(c, "esds: truncated ES_Descriptor");
  e = p + len;
  {
    unsigned flags = p[2];
    p += 3;
    if (flags & 0x80) p += 2;                       /* dependsOn_ES_ID */
    if (flags & 0x40) p += (p < e ? 1 + *p : 0);    /* URL */
    if (flags & 0x20) p += 2;                       /* OCR_ES_Id */
  }
  /* DecoderConfigDescriptor */
  while (p < e) {
    unsigned tag = *p;
    const uint8_t *q;
    if (desc_len(p + 1, (size_t)(e - p - 1), &used, &len)) return fail(c, "esds: bad length");
    q = p + 1 + used;
    if (len > (size_t)(e - q)) return fail(c, "esds: truncated descriptor");
    if (tag == 0x04) {
      const uint8_t *qe = q + len;
      if (len < 13) return fail(c, "esds: truncated DecoderConfigDescriptor");
      t->oti = q[0];
      q += 13;
      while (q < qe) {
        unsigned tag2 = *q;
        const uint8_t *r;
        if (desc_len(q + 1, (size_t)(qe - q - 1), &used, &len))
          return fail(c, "esds: bad length");
        r = q + 1 + used;
        if (len > (size_t)(qe - r)) return fail(c, "esds: truncated descriptor");
        if (tag2 == 0x05) {
          t->asc = r; /* any length: e.g. Structured Audio has long ones */
          t->asc_len = (uint32_t)len;
          return 0;
        }
        q = r + len;
      }
      return 0; /* no DecoderSpecificInfo */
    }
    p = q + len;
  }
  /* No DecoderConfigDescriptor (e.g. TTSI): the samples are still returned; the decoder gets no
   * configuration and rejects them (a negative test). */
  return 0;
}

static int parse_stsd(Ctx *c, const Box *stsd, Mp4Track *t) {
  size_t pos = stsd->payload + 8;
  Box e, x;
  int r;
  if (need(c, stsd, 8, "stsd")) return -1;
  if (rd32(c->d + stsd->payload + 4) < 1) return fail(c, "stsd: no sample entry");
  if ((r = next_box(c, &pos, stsd->end, &e)) <= 0) return r < 0 ? -1 : fail(c, "stsd: empty");
  t->codec[0] = (char)(e.type >> 24);
  t->codec[1] = (char)(e.type >> 16);
  t->codec[2] = (char)(e.type >> 8);
  t->codec[3] = (char)e.type;
  t->codec[4] = 0;
  if (e.type != FOURCC('m', 'p', '4', 'a')) return 0;
  {
    /* SampleEntry (8) + AudioSampleEntry (20); QuickTime version 1 adds 16, version 2 adds 36. */
    size_t child = e.payload + 28;
    unsigned version;
    if (e.end - e.payload < 28) return fail(c, "truncated 'mp4a' entry");
    version = rd16(c->d + e.payload + 8);
    if (version == 1) child += 16;
    else if (version == 2) child += 36;
    if (child > e.end) return fail(c, "truncated 'mp4a' entry");
    r = find_child(c, child, e.end, FOURCC('e', 's', 'd', 's'), &x);
    if (r < 0) return -1;
    if (r == 0) {
      Box w;
      r = find_child(c, child, e.end, FOURCC('w', 'a', 'v', 'e'), &w);
      if (r < 0) return -1;
      if (r == 0) return fail(c, "'mp4a' entry without 'esds'");
      r = find_child(c, w.payload, w.end, FOURCC('e', 's', 'd', 's'), &x);
      if (r <= 0) return r < 0 ? -1 : fail(c, "'wave' without 'esds'");
    }
    return parse_esds(c, &x, t);
  }
}

static int parse_stbl(Ctx *c, const Box *stbl, Mp4Track *t) {
  Box stsd, sz, co, sc;
  int r, is_stz2 = 0, is_co64 = 0;
  uint32_t n_sizes, fixed = 0, field = 32, n_chunks, n_sc, i, s = 0;
  const uint8_t *sizes, *offs, *scs;
  if ((r = find_child(c, stbl->payload, stbl->end, FOURCC('s', 't', 's', 'd'), &stsd)) <= 0)
    return r < 0 ? -1 : fail(c, "no 'stsd'");
  if (parse_stsd(c, &stsd, t)) return -1;
  r = find_child(c, stbl->payload, stbl->end, FOURCC('s', 't', 's', 'z'), &sz);
  if (r == 0) {
    r = find_child(c, stbl->payload, stbl->end, FOURCC('s', 't', 'z', '2'), &sz);
    is_stz2 = 1;
  }
  if (r <= 0) return r < 0 ? -1 : fail(c, "no 'stsz'");
  r = find_child(c, stbl->payload, stbl->end, FOURCC('s', 't', 'c', 'o'), &co);
  if (r == 0) {
    r = find_child(c, stbl->payload, stbl->end, FOURCC('c', 'o', '6', '4'), &co);
    is_co64 = 1;
  }
  if (r <= 0) return r < 0 ? -1 : fail(c, "no 'stco'");
  if ((r = find_child(c, stbl->payload, stbl->end, FOURCC('s', 't', 's', 'c'), &sc)) <= 0)
    return r < 0 ? -1 : fail(c, "no 'stsc'");

  if (need(c, &sz, 12, "stsz") || need(c, &co, 8, "stco") || need(c, &sc, 8, "stsc")) return -1;
  if (is_stz2) {
    field = c->d[sz.payload + 7];
    if (field != 4 && field != 8 && field != 16) return fail(c, "stz2: bad field size");
  } else {
    fixed = rd32(c->d + sz.payload + 4);
  }
  n_sizes = rd32(c->d + sz.payload + 8);
  sizes = c->d + sz.payload + 12;
  if (!fixed && (uint64_t)n_sizes * field / 8 > sz.end - sz.payload - 12)
    return fail(c, "truncated 'stsz'");
  n_chunks = rd32(c->d + co.payload + 4);
  offs = c->d + co.payload + 8;
  if ((uint64_t)n_chunks * (is_co64 ? 8 : 4) > co.end - co.payload - 8)
    return fail(c, "truncated 'stco'");
  n_sc = rd32(c->d + sc.payload + 4);
  scs = c->d + sc.payload + 8;
  if ((uint64_t)n_sc * 12 > sc.end - sc.payload - 8) return fail(c, "truncated 'stsc'");

  for (i = 0; i < n_sc && s < n_sizes; i++) {
    uint32_t first = rd32(scs + 12 * i), per = rd32(scs + 12 * i + 4);
    uint32_t last = i + 1 < n_sc ? rd32(scs + 12 * (i + 1)) - 1 : n_chunks;
    uint32_t ch;
    if (first < 1 || last > n_chunks) return fail(c, "stsc: chunk index out of range");
    for (ch = first; ch <= last && s < n_sizes; ch++) {
      uint64_t off = is_co64 ? rd64(offs + 8 * (ch - 1)) : rd32(offs + 4 * (ch - 1));
      uint32_t k;
      for (k = 0; k < per && s < n_sizes; k++, s++) {
        uint32_t size;
        if (fixed) size = fixed;
        else if (field == 32) size = rd32(sizes + 4 * s);
        else if (field == 16) size = rd16(sizes + 2 * s);
        else if (field == 8) size = sizes[s];
        else size = (s & 1) ? (sizes[s / 2] & 15) : (sizes[s / 2] >> 4);
        if (add_sample(c, t, off, size)) return -1;
        off += size;
      }
    }
  }
  if (s < n_sizes) return fail(c, "stsc/stco describe %u of %u samples", s, n_sizes);
  return 0;
}

static int parse_fragments(Ctx *c, Mp4Track *t, uint32_t trex_size) {
  size_t pos = 0;
  Box moof;
  int r;
  while ((r = next_box(c, &pos, c->size, &moof)) > 0) {
    size_t p2 = moof.payload;
    Box traf;
    int first_traf = 1;
    uint64_t prev_end = moof.start;
    if (moof.type != FOURCC('m', 'o', 'o', 'f')) continue;
    while ((r = next_box(c, &p2, moof.end, &traf)) > 0) {
      Box tfhd, trun;
      size_t p3;
      uint32_t flags, def_size = trex_size;
      uint64_t base;
      const uint8_t *q;
      int first_trun = 1;
      if (traf.type != FOURCC('t', 'r', 'a', 'f')) continue;
      if ((r = find_child(c, traf.payload, traf.end, FOURCC('t', 'f', 'h', 'd'), &tfhd)) <= 0)
        return r < 0 ? -1 : fail(c, "traf without tfhd");
      if (need(c, &tfhd, 8, "tfhd")) return -1;
      q = c->d + tfhd.payload;
      flags = rd32(q) & 0xffffff;
      if (rd32(q + 4) != t->track_id) {
        first_traf = 0;
        continue;
      }
      q += 8;
      {
        size_t n = 8 + (flags & 1 ? 8 : 0) + (flags & 2 ? 4 : 0) + (flags & 8 ? 4 : 0) +
                   (flags & 0x10 ? 4 : 0) + (flags & 0x20 ? 4 : 0);
        if (need(c, &tfhd, n, "tfhd")) return -1;
      }
      if (flags & 1) {
        base = rd64(q);
        q += 8;
      } else if ((flags & 0x20000) || first_traf) {
        base = moof.start;
      } else {
        base = prev_end;
      }
      if (flags & 2) q += 4;
      if (flags & 8) q += 4;
      if (flags & 0x10) def_size = rd32(q);
      first_traf = 0;
      p3 = traf.payload;
      while ((r = next_box(c, &p3, traf.end, &trun)) > 0) {
        uint32_t tf, n, i;
        uint64_t off;
        size_t need_n;
        if (trun.type != FOURCC('t', 'r', 'u', 'n')) continue;
        if (need(c, &trun, 8, "trun")) return -1;
        q = c->d + trun.payload;
        tf = rd32(q) & 0xffffff;
        n = rd32(q + 4);
        q += 8;
        need_n = 8 + (tf & 1 ? 4 : 0) + (tf & 4 ? 4 : 0) +
                 (size_t)n * (((tf & 0x100) ? 4 : 0) + ((tf & 0x200) ? 4 : 0) +
                              ((tf & 0x400) ? 4 : 0) + ((tf & 0x800) ? 4 : 0));
        if (need(c, &trun, need_n, "trun")) return -1;
        if (tf & 1) {
          off = base + (int64_t)(int32_t)rd32(q);
          q += 4;
        } else {
          off = first_trun ? base : prev_end;
        }
        if (tf & 4) q += 4;
        for (i = 0; i < n; i++) {
          uint32_t size = def_size;
          if (tf & 0x100) q += 4;
          if (tf & 0x200) {
            size = rd32(q);
            q += 4;
          }
          if (tf & 0x400) q += 4;
          if (tf & 0x800) q += 4;
          if (add_sample(c, t, off, size)) return -1;
          off += size;
        }
        prev_end = off;
        first_trun = 0;
        t->fragmented = 1;
      }
      if (r < 0) return -1;
    }
    if (r < 0) return -1;
  }
  return r < 0 ? -1 : 0;
}

int mp4_demux(const uint8_t *data, size_t size, uint32_t track_id, Mp4Track *t, char *err,
              size_t errlen) {
  Ctx c = {data, size, err, errlen};
  Box moov, trak, mvex, b;
  size_t pos;
  int r, found = 0;
  uint32_t trex_size = 0;

  memset(t, 0, sizeof(*t));
  t->oti = -1;
  if ((r = find_child(&c, 0, size, FOURCC('m', 'o', 'o', 'v'), &moov)) <= 0)
    return r < 0 ? -1 : fail(&c, "no 'moov' box");

  pos = moov.payload;
  while ((r = next_box(&c, &pos, moov.end, &trak)) > 0) {
    Box tkhd, mdia, hdlr;
    uint32_t id;
    if (trak.type != FOURCC('t', 'r', 'a', 'k')) continue;
    if (find_child(&c, trak.payload, trak.end, FOURCC('t', 'k', 'h', 'd'), &tkhd) <= 0 ||
        find_child(&c, trak.payload, trak.end, FOURCC('m', 'd', 'i', 'a'), &mdia) <= 0)
      continue;
    if (need(&c, &tkhd, 24, "tkhd")) return -1;
    id = rd32(data + tkhd.payload + (data[tkhd.payload] == 1 ? 20 : 12));
    if (track_id ? id != track_id
                 : (find_child(&c, mdia.payload, mdia.end, FOURCC('h', 'd', 'l', 'r'), &hdlr) <= 0 ||
                    need(&c, &hdlr, 12, "hdlr") ||
                    rd32(data + hdlr.payload + 8) != FOURCC('s', 'o', 'u', 'n')))
      continue;
    t->track_id = id;
    found = 1;
    if (find_child(&c, mdia.payload, mdia.end, FOURCC('m', 'i', 'n', 'f'), &b) <= 0 ||
        find_child(&c, b.payload, b.end, FOURCC('s', 't', 'b', 'l'), &b) <= 0)
      return fail(&c, "track %u: no 'minf/stbl'", id);
    if (parse_stbl(&c, &b, t)) return -1;
    break;
  }
  if (r < 0) return -1;
  if (!found) {
    if (track_id) return fail(&c, "no track with ID %u", track_id);
    return fail(&c, "no audio track");
  }
  if (strcmp(t->codec, "mp4a") != 0) return fail(&c, "unsupported sample entry '%s'", t->codec);

  /* Movie fragments */
  r = find_child(&c, moov.payload, moov.end, FOURCC('m', 'v', 'e', 'x'), &mvex);
  if (r < 0) return -1;
  if (r > 0) {
    pos = mvex.payload;
    while ((r = next_box(&c, &pos, mvex.end, &b)) > 0) {
      if (b.type == FOURCC('t', 'r', 'e', 'x') && b.end - b.payload >= 24 &&
          rd32(data + b.payload + 4) == t->track_id)
        trex_size = rd32(data + b.payload + 16);
    }
    if (r < 0) return -1;
    if (parse_fragments(&c, t, trex_size)) return -1;
  }
  return 0;
}
