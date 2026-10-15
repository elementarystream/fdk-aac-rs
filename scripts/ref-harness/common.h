/* Shared helpers of the reference harness programs (fdkdec-ref, fdkenc-ref): SHA-256, the JSON
 * Lines writer, argument parsing and file input. See PORTING.md, "Reference harness". */
#ifndef REF_HARNESS_COMMON_H
#define REF_HARNESS_COMMON_H

#include <stdint.h>
#include <stdio.h>
#include <stddef.h>

/* ---- SHA-256 (FIPS 180-4) ------------------------------------------------------------------ */

typedef struct {
  uint32_t h[8];
  uint64_t len;
  uint8_t buf[64];
  size_t fill;
} Sha256;

void sha256_init(Sha256 *s);
void sha256_update(Sha256 *s, const void *data, size_t n);
void sha256_final(Sha256 *s, uint8_t out[32]);
/* Lowercase hex digest of a buffer (65 bytes including the terminator). */
void sha256_hex(const void *data, size_t n, char out[65]);

/* ---- JSON Lines writer --------------------------------------------------------------------- */

/* One record per line, keys in the order written, no whitespace. A NULL file disables output. */
typedef struct {
  FILE *f;
  int need_comma;
} Json;

void j_begin(Json *j, const char *call);
void j_end(Json *j);
void j_int(Json *j, const char *key, long long v);
void j_uint(Json *j, const char *key, unsigned long long v);
void j_bool(Json *j, const char *key, int v);
void j_str(Json *j, const char *key, const char *v); /* v must not need escaping */
void j_hex(Json *j, const char *key, const uint8_t *data, size_t n);
void j_null(Json *j, const char *key);
void j_obj_begin(Json *j, const char *key);
void j_obj_end(Json *j);
void j_arr_begin(Json *j, const char *key);
void j_arr_int(Json *j, long long v);
void j_arr_end(Json *j);

/* ---- Utilities ----------------------------------------------------------------------------- */

void die(const char *fmt, ...);
/* Parses a decimal, 0x hex or negative integer; dies with a message naming `what` on error. */
long long parse_int(const char *s, const char *what);
/* Parses a hex string (whitespace ignored) into a malloc'ed buffer. Returns the length. */
size_t parse_hex(const char *s, uint8_t **out, const char *what);
/* Reads a whole file into a malloc'ed buffer. */
uint8_t *read_file(const char *path, size_t *size);
/* Opens a log or output file; "-" is stdout. NULL path gives NULL. */
FILE *open_out(const char *path);

/* Splits "PREFIX:REST" at the first ':'; returns the integer prefix and sets *rest. */
long long split_at(const char *s, const char **rest, const char *what);

/* Growable list of "events" applied at a call index (param-at, flags-at, ...). */
typedef struct {
  long long at;
  int kind;
  long long id;
  long long value;
  char name[64];
} Event;

typedef struct {
  Event *v;
  size_t n, cap;
} EventList;

void events_push(EventList *l, Event e);

#endif
