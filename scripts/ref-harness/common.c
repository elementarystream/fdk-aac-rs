/* Shared helpers of the reference harness programs. See common.h and PORTING.md, "Reference
 * harness". */
#include "common.h"

#include <ctype.h>
#include <errno.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>

/* ---- SHA-256 ------------------------------------------------------------------------------- */

static const uint32_t K256[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
    0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
    0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
    0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
    0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
    0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2};

#define ROR(x, n) (((x) >> (n)) | ((x) << (32 - (n))))

static void sha256_block(Sha256 *s, const uint8_t *p) {
  uint32_t w[64], a, b, c, d, e, f, g, h;
  int i;
  for (i = 0; i < 16; i++)
    w[i] = ((uint32_t)p[4 * i] << 24) | ((uint32_t)p[4 * i + 1] << 16) |
           ((uint32_t)p[4 * i + 2] << 8) | p[4 * i + 3];
  for (i = 16; i < 64; i++) {
    uint32_t s0 = ROR(w[i - 15], 7) ^ ROR(w[i - 15], 18) ^ (w[i - 15] >> 3);
    uint32_t s1 = ROR(w[i - 2], 17) ^ ROR(w[i - 2], 19) ^ (w[i - 2] >> 10);
    w[i] = w[i - 16] + s0 + w[i - 7] + s1;
  }
  a = s->h[0]; b = s->h[1]; c = s->h[2]; d = s->h[3];
  e = s->h[4]; f = s->h[5]; g = s->h[6]; h = s->h[7];
  for (i = 0; i < 64; i++) {
    uint32_t t1 = h + (ROR(e, 6) ^ ROR(e, 11) ^ ROR(e, 25)) + ((e & f) ^ (~e & g)) + K256[i] + w[i];
    uint32_t t2 = (ROR(a, 2) ^ ROR(a, 13) ^ ROR(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
    h = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
  }
  s->h[0] += a; s->h[1] += b; s->h[2] += c; s->h[3] += d;
  s->h[4] += e; s->h[5] += f; s->h[6] += g; s->h[7] += h;
}

void sha256_init(Sha256 *s) {
  static const uint32_t iv[8] = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                                 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
  memcpy(s->h, iv, sizeof(iv));
  s->len = 0;
  s->fill = 0;
}

void sha256_update(Sha256 *s, const void *data, size_t n) {
  const uint8_t *p = (const uint8_t *)data;
  s->len += n;
  while (n > 0) {
    size_t k = 64 - s->fill;
    if (k > n) k = n;
    memcpy(s->buf + s->fill, p, k);
    s->fill += k;
    p += k;
    n -= k;
    if (s->fill == 64) {
      sha256_block(s, s->buf);
      s->fill = 0;
    }
  }
}

void sha256_final(Sha256 *s, uint8_t out[32]) {
  uint64_t bits = s->len * 8;
  uint8_t pad = 0x80, zero = 0, lenb[8];
  int i;
  sha256_update(s, &pad, 1);
  while (s->fill != 56) sha256_update(s, &zero, 1);
  for (i = 0; i < 8; i++) lenb[i] = (uint8_t)(bits >> (56 - 8 * i));
  sha256_update(s, lenb, 8);
  for (i = 0; i < 8; i++) {
    out[4 * i] = (uint8_t)(s->h[i] >> 24);
    out[4 * i + 1] = (uint8_t)(s->h[i] >> 16);
    out[4 * i + 2] = (uint8_t)(s->h[i] >> 8);
    out[4 * i + 3] = (uint8_t)s->h[i];
  }
}

void sha256_hex(const void *data, size_t n, char out[65]) {
  static const char hx[] = "0123456789abcdef";
  Sha256 s;
  uint8_t d[32];
  int i;
  sha256_init(&s);
  sha256_update(&s, data, n);
  sha256_final(&s, d);
  for (i = 0; i < 32; i++) {
    out[2 * i] = hx[d[i] >> 4];
    out[2 * i + 1] = hx[d[i] & 15];
  }
  out[64] = 0;
}

/* ---- JSON Lines writer --------------------------------------------------------------------- */

static void j_key(Json *j, const char *key) {
  if (j->need_comma) fputc(',', j->f);
  if (key) fprintf(j->f, "\"%s\":", key);
  j->need_comma = 1;
}

void j_begin(Json *j, const char *call) {
  if (!j->f) return;
  fprintf(j->f, "{\"call\":\"%s\"", call);
  j->need_comma = 1;
}

void j_end(Json *j) {
  if (!j->f) return;
  fputs("}\n", j->f);
  j->need_comma = 0;
}

void j_int(Json *j, const char *key, long long v) {
  if (!j->f) return;
  j_key(j, key);
  fprintf(j->f, "%lld", v);
}

void j_uint(Json *j, const char *key, unsigned long long v) {
  if (!j->f) return;
  j_key(j, key);
  fprintf(j->f, "%llu", v);
}

void j_bool(Json *j, const char *key, int v) {
  if (!j->f) return;
  j_key(j, key);
  fputs(v ? "true" : "false", j->f);
}

void j_str(Json *j, const char *key, const char *v) {
  if (!j->f) return;
  j_key(j, key);
  fprintf(j->f, "\"%s\"", v);
}

void j_hex(Json *j, const char *key, const uint8_t *data, size_t n) {
  size_t i;
  if (!j->f) return;
  j_key(j, key);
  fputc('"', j->f);
  for (i = 0; i < n; i++) fprintf(j->f, "%02x", data[i]);
  fputc('"', j->f);
}

void j_null(Json *j, const char *key) {
  if (!j->f) return;
  j_key(j, key);
  fputs("null", j->f);
}

void j_obj_begin(Json *j, const char *key) {
  if (!j->f) return;
  j_key(j, key);
  fputc('{', j->f);
  j->need_comma = 0;
}

void j_obj_end(Json *j) {
  if (!j->f) return;
  fputc('}', j->f);
  j->need_comma = 1;
}

void j_arr_begin(Json *j, const char *key) {
  if (!j->f) return;
  j_key(j, key);
  fputc('[', j->f);
  j->need_comma = 0;
}

void j_arr_int(Json *j, long long v) {
  if (!j->f) return;
  j_key(j, NULL);
  fprintf(j->f, "%lld", v);
}

void j_arr_end(Json *j) {
  if (!j->f) return;
  fputc(']', j->f);
  j->need_comma = 1;
}

/* ---- Utilities ----------------------------------------------------------------------------- */

void die(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  vfprintf(stderr, fmt, ap);
  va_end(ap);
  fputc('\n', stderr);
  exit(2);
}

long long parse_int(const char *s, const char *what) {
  char *end;
  long long v;
  errno = 0;
  v = strtoll(s, &end, 0);
  if (errno || end == s || *end) die("invalid integer for %s: '%s'", what, s);
  return v;
}

static int hexval(int c) {
  if (c >= '0' && c <= '9') return c - '0';
  if (c >= 'a' && c <= 'f') return c - 'a' + 10;
  if (c >= 'A' && c <= 'F') return c - 'A' + 10;
  return -1;
}

size_t parse_hex(const char *s, uint8_t **out, const char *what) {
  size_t n = 0, cap = strlen(s) / 2 + 1;
  uint8_t *b = (uint8_t *)malloc(cap);
  int hi = -1;
  if (!b) die("out of memory");
  for (; *s; s++) {
    int v;
    if (isspace((unsigned char)*s)) continue;
    v = hexval((unsigned char)*s);
    if (v < 0) die("invalid hex digit in %s", what);
    if (hi < 0) {
      hi = v;
    } else {
      b[n++] = (uint8_t)(hi << 4 | v);
      hi = -1;
    }
  }
  if (hi >= 0) die("odd number of hex digits in %s", what);
  *out = b;
  return n;
}

uint8_t *read_file(const char *path, size_t *size) {
  FILE *f = fopen(path, "rb");
  uint8_t *buf = NULL;
  size_t n = 0, cap = 0;
  if (!f) die("cannot open %s", path);
  for (;;) {
    size_t r;
    if (cap - n < 65536) {
      cap = cap ? cap * 2 : 1 << 20;
      buf = (uint8_t *)realloc(buf, cap);
      if (!buf) die("out of memory");
    }
    r = fread(buf + n, 1, cap - n, f);
    n += r;
    if (r == 0) break;
  }
  if (ferror(f)) die("cannot read %s", path);
  fclose(f);
  *size = n;
  return buf;
}

FILE *open_out(const char *path) {
  FILE *f;
  if (!path) return NULL;
  if (strcmp(path, "-") == 0) return stdout;
  f = fopen(path, "wb");
  if (!f) die("cannot create %s", path);
  return f;
}

long long split_at(const char *s, const char **rest, const char *what) {
  const char *c = strchr(s, ':');
  char tmp[64];
  if (!c || (size_t)(c - s) >= sizeof(tmp)) die("expected N:... for %s: '%s'", what, s);
  memcpy(tmp, s, (size_t)(c - s));
  tmp[c - s] = 0;
  *rest = c + 1;
  return parse_int(tmp, what);
}

void events_push(EventList *l, Event e) {
  if (l->n == l->cap) {
    l->cap = l->cap ? l->cap * 2 : 16;
    l->v = (Event *)realloc(l->v, l->cap * sizeof(Event));
    if (!l->v) die("out of memory");
  }
  l->v[l->n++] = e;
}
