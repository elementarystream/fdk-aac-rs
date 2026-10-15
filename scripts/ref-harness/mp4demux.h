/* Minimal MP4/M4A demuxer of fdkdec-ref: finds an audio track, its AudioSpecificConfig and the
 * byte ranges of its samples. See PORTING.md, "Reference harness" for what is and is not
 * supported. */
#ifndef REF_HARNESS_MP4DEMUX_H
#define REF_HARNESS_MP4DEMUX_H

#include <stddef.h>
#include <stdint.h>

typedef struct {
  uint64_t offset;
  uint32_t size;
} Mp4Sample;

typedef struct {
  uint32_t track_id;
  char codec[5];         /* sample entry type of the first sample description, e.g. "mp4a" */
  int oti;               /* objectTypeIndication of the DecoderConfigDescriptor, -1 if none */
  const uint8_t *asc;    /* DecoderSpecificInfo (the AudioSpecificConfig), points into the file
                            data; NULL if there is none */
  uint32_t asc_len;
  int fragmented;        /* 1 if samples were taken from movie fragments */
  Mp4Sample *samples;
  uint32_t n_samples;
  uint32_t cap_samples;
} Mp4Track;

/* Demuxes the file in `data`. track_id = 0 selects the first track with a 'soun' handler.
 * Returns 0 on success, or -1 with a message in err. */
int mp4_demux(const uint8_t *data, size_t size, uint32_t track_id, Mp4Track *out, char *err,
              size_t errlen);

#endif
