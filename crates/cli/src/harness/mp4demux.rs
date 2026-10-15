// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from scripts/ref-harness/mp4demux.h, scripts/ref-harness/mp4demux.c (the reference harness
// of this project, not part of fdk-aac).

//! The minimal MP4/M4A demuxer of the decoder CLI: finds an audio track, its
//! AudioSpecificConfig and the byte ranges of its samples. It is a port of the reference
//! harness's demuxer (`scripts/ref-harness/mp4demux.c`) with the same behavior: the same track,
//! configuration and samples, and failure in the same cases (PORTING.md, "Reference harness").
//!
//! Supported: ISO BMFF / QuickTime files held in memory; `moov/trak/mdia/(hdlr, minf/stbl)`;
//! `stsd` with an `mp4a` sample entry (versions 0, 1 and 2, `esds` directly or inside `wave`);
//! `stsz`/`stz2`, `stco`/`co64`, `stsc`; movie fragments (`mvex/trex`, `moof/traf/tfhd/trun`).
//! Not supported: several sample descriptions, edit lists (ignored: every sample is decoded),
//! encrypted entries, external data references, `sdtp`/`sbgp` semantics.
//!
//! Offsets are `usize` positions in the file data; every read is bounds-checked (the C code reads
//! only inside the boxes it has checked, so the checks never change the result).

/// `Mp4Sample`: the byte range of a sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mp4Sample {
    /// Offset in the file.
    pub offset: u64,
    /// Size in bytes.
    pub size: u32,
}

/// `Mp4Track`: the demuxed track.
#[derive(Clone, Debug, Default)]
pub struct Mp4Track {
    /// `tkhd` track_ID.
    pub track_id: u32,
    /// Sample entry type of the first sample description (as the C string: up to the first NUL
    /// byte).
    pub codec: Vec<u8>,
    /// objectTypeIndication of the DecoderConfigDescriptor, -1 if none.
    pub oti: i32,
    /// The DecoderSpecificInfo (the AudioSpecificConfig): offset and length in the file data
    /// (`None`: there is none).
    pub asc: Option<(usize, usize)>,
    /// 1 if samples were taken from movie fragments.
    pub fragmented: bool,
    /// The samples.
    pub samples: Vec<Mp4Sample>,
}

impl Mp4Track {
    /// The AudioSpecificConfig bytes (empty if there is none).
    pub fn asc<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        match self.asc {
            Some((o, l)) => data.get(o..o + l).unwrap_or(&[]),
            None => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Box_ {
    /// file offset of the box header
    start: usize,
    /// file offset of the payload
    payload: usize,
    /// file offset after the box
    end: usize,
    type_: u32,
}

struct Ctx<'a> {
    d: &'a [u8],
    err: String,
}

const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    ((a as u32) << 24) | ((b as u32) << 16) | ((c as u32) << 8) | d as u32
}

impl Ctx<'_> {
    fn fail(&mut self, msg: String) -> i32 {
        self.err = msg;
        -1
    }

    fn b(&self, p: usize) -> u32 {
        self.d.get(p).copied().unwrap_or(0) as u32
    }

    fn rd16(&self, p: usize) -> u32 {
        (self.b(p) << 8) | self.b(p + 1)
    }

    fn rd32(&self, p: usize) -> u32 {
        (self.b(p) << 24) | (self.b(p + 1) << 16) | (self.b(p + 2) << 8) | self.b(p + 3)
    }

    fn rd64(&self, p: usize) -> u64 {
        ((self.rd32(p) as u64) << 32) | self.rd32(p + 4) as u64
    }

    /// Reads the box at `*pos` within `[*pos, end)`. Returns 1 on success, 0 at the end, -1 on
    /// error.
    fn next_box(&mut self, pos: &mut usize, end: usize, b: &mut Box_) -> i32 {
        let mut hdr = 8usize;
        if *pos >= end {
            return 0;
        }
        if end - *pos < 8 {
            return self.fail(format!("truncated box header at {}", *pos));
        }
        let mut size = self.rd32(*pos) as u64;
        b.type_ = self.rd32(*pos + 4);
        if size == 1 {
            if end - *pos < 16 {
                return self.fail(format!("truncated box header at {}", *pos));
            }
            size = self.rd64(*pos + 8);
            hdr = 16;
        } else if size == 0 {
            size = (end - *pos) as u64;
        }
        if size < hdr as u64 || size > (end - *pos) as u64 {
            return self.fail(format!("invalid box size at {}", *pos));
        }
        b.start = *pos;
        b.payload = *pos + hdr;
        b.end = *pos + size as usize;
        *pos = b.end;
        1
    }

    /// Finds the first child of type `type_` in `[from, end)`. Returns 1, 0 if absent, -1 on
    /// error.
    fn find_child(&mut self, from: usize, end: usize, type_: u32, out: &mut Box_) -> i32 {
        let mut pos = from;
        let mut b = Box_::default();
        loop {
            let r = self.next_box(&mut pos, end, &mut b);
            if r <= 0 {
                return r;
            }
            if b.type_ == type_ {
                *out = b;
                return 1;
            }
        }
    }

    fn need(&mut self, b: &Box_, n: usize, what: &str) -> i32 {
        if b.end - b.payload < n {
            return self.fail(format!("truncated '{what}' box"));
        }
        0
    }

    fn add_sample(&mut self, t: &mut Mp4Track, off: u64, size: u32) -> i32 {
        let fsize = self.d.len() as u64;
        if off > fsize || size as u64 > fsize - off {
            return self.fail(format!(
                "sample {} (offset {}, size {}) beyond the end of the file",
                t.samples.len(),
                off,
                size
            ));
        }
        t.samples.push(Mp4Sample { offset: off, size });
        0
    }
}

/// Reads an expandable descriptor length (ISO/IEC 14496-1, 8.3.3) from `d[p..p + avail]`.
fn desc_len(d: &[u8], p: usize, avail: usize) -> Option<(usize, u32)> {
    let mut v: u32 = 0;
    let mut i = 0;
    while i < 4 && i < avail {
        let x = d.get(p + i).copied().unwrap_or(0);
        v = (v << 7) | (x & 0x7f) as u32;
        if x & 0x80 == 0 {
            return Some((i + 1, v));
        }
        i += 1;
    }
    None
}

fn parse_esds(c: &mut Ctx, b: &Box_, t: &mut Mp4Track) -> i32 {
    let d = c.d;
    let mut p = b.payload + 4; /* version and flags */
    let mut e = b.end;
    if c.need(b, 4, "esds") != 0 {
        return -1;
    }
    let at = |i: usize| d.get(i).copied().unwrap_or(0);
    /* ES_Descriptor */
    if p >= e || at(p) != 0x03 {
        return c.fail("esds: no ES_Descriptor".into());
    }
    let Some((used, len)) = desc_len(d, p + 1, e - p - 1) else {
        return c.fail("esds: bad length".into());
    };
    p += 1 + used;
    if len as usize > e - p || len < 3 {
        return c.fail("esds: truncated ES_Descriptor".into());
    }
    e = p + len as usize;
    {
        let flags = at(p + 2);
        p += 3;
        if flags & 0x80 != 0 {
            p += 2; /* dependsOn_ES_ID */
        }
        if flags & 0x40 != 0 {
            p += if p < e { 1 + at(p) as usize } else { 0 }; /* URL */
        }
        if flags & 0x20 != 0 {
            p += 2; /* OCR_ES_Id */
        }
    }
    /* DecoderConfigDescriptor */
    while p < e {
        let tag = at(p);
        let Some((used, len)) = desc_len(d, p + 1, e - p - 1) else {
            return c.fail("esds: bad length".into());
        };
        let mut q = p + 1 + used;
        if len as usize > e - q {
            return c.fail("esds: truncated descriptor".into());
        }
        if tag == 0x04 {
            let qe = q + len as usize;
            if len < 13 {
                return c.fail("esds: truncated DecoderConfigDescriptor".into());
            }
            t.oti = at(q) as i32;
            q += 13;
            while q < qe {
                let tag2 = at(q);
                let Some((used, len)) = desc_len(d, q + 1, qe - q - 1) else {
                    return c.fail("esds: bad length".into());
                };
                let r = q + 1 + used;
                if len as usize > qe - r {
                    return c.fail("esds: truncated descriptor".into());
                }
                if tag2 == 0x05 {
                    t.asc = Some((r, len as usize)); /* any length: e.g. Structured Audio has long ones */
                    return 0;
                }
                q = r + len as usize;
            }
            return 0; /* no DecoderSpecificInfo */
        }
        p = q + len as usize;
    }
    /* No DecoderConfigDescriptor (e.g. TTSI): the samples are still returned; the decoder gets
     * no configuration and rejects them (a negative test). */
    0
}

fn parse_stsd(c: &mut Ctx, stsd: &Box_, t: &mut Mp4Track) -> i32 {
    let mut pos = stsd.payload + 8;
    let mut e = Box_::default();
    let mut x = Box_::default();
    if c.need(stsd, 8, "stsd") != 0 {
        return -1;
    }
    if c.rd32(stsd.payload + 4) < 1 {
        return c.fail("stsd: no sample entry".into());
    }
    let r = c.next_box(&mut pos, stsd.end, &mut e);
    if r <= 0 {
        return if r < 0 { -1 } else { c.fail("stsd: empty".into()) };
    }
    let ty = e.type_.to_be_bytes();
    // The C string of the four bytes ends at the first NUL byte.
    t.codec = ty.iter().copied().take_while(|&b| b != 0).collect();
    if e.type_ != fourcc(b'm', b'p', b'4', b'a') {
        return 0;
    }
    {
        /* SampleEntry (8) + AudioSampleEntry (20); QuickTime version 1 adds 16, version 2 adds
         * 36. */
        let mut child = e.payload + 28;
        if e.end - e.payload < 28 {
            return c.fail("truncated 'mp4a' entry".into());
        }
        let version = c.rd16(e.payload + 8);
        if version == 1 {
            child += 16;
        } else if version == 2 {
            child += 36;
        }
        if child > e.end {
            return c.fail("truncated 'mp4a' entry".into());
        }
        let mut r = c.find_child(child, e.end, fourcc(b'e', b's', b'd', b's'), &mut x);
        if r < 0 {
            return -1;
        }
        if r == 0 {
            let mut w = Box_::default();
            r = c.find_child(child, e.end, fourcc(b'w', b'a', b'v', b'e'), &mut w);
            if r < 0 {
                return -1;
            }
            if r == 0 {
                return c.fail("'mp4a' entry without 'esds'".into());
            }
            r = c.find_child(w.payload, w.end, fourcc(b'e', b's', b'd', b's'), &mut x);
            if r <= 0 {
                return if r < 0 { -1 } else { c.fail("'wave' without 'esds'".into()) };
            }
        }
        parse_esds(c, &x, t)
    }
}

fn parse_stbl(c: &mut Ctx, stbl: &Box_, t: &mut Mp4Track) -> i32 {
    let mut stsd = Box_::default();
    let mut sz = Box_::default();
    let mut co = Box_::default();
    let mut sc = Box_::default();
    let mut is_stz2 = false;
    let mut is_co64 = false;
    let mut fixed: u32 = 0;
    let mut field: u32 = 32;

    let r = c.find_child(stbl.payload, stbl.end, fourcc(b's', b't', b's', b'd'), &mut stsd);
    if r <= 0 {
        return if r < 0 { -1 } else { c.fail("no 'stsd'".into()) };
    }
    if parse_stsd(c, &stsd, t) != 0 {
        return -1;
    }
    let mut r = c.find_child(stbl.payload, stbl.end, fourcc(b's', b't', b's', b'z'), &mut sz);
    if r == 0 {
        r = c.find_child(stbl.payload, stbl.end, fourcc(b's', b't', b'z', b'2'), &mut sz);
        is_stz2 = true;
    }
    if r <= 0 {
        return if r < 0 { -1 } else { c.fail("no 'stsz'".into()) };
    }
    r = c.find_child(stbl.payload, stbl.end, fourcc(b's', b't', b'c', b'o'), &mut co);
    if r == 0 {
        r = c.find_child(stbl.payload, stbl.end, fourcc(b'c', b'o', b'6', b'4'), &mut co);
        is_co64 = true;
    }
    if r <= 0 {
        return if r < 0 { -1 } else { c.fail("no 'stco'".into()) };
    }
    r = c.find_child(stbl.payload, stbl.end, fourcc(b's', b't', b's', b'c'), &mut sc);
    if r <= 0 {
        return if r < 0 { -1 } else { c.fail("no 'stsc'".into()) };
    }

    if c.need(&sz, 12, "stsz") != 0 || c.need(&co, 8, "stco") != 0 || c.need(&sc, 8, "stsc") != 0 {
        return -1;
    }
    if is_stz2 {
        field = c.b(sz.payload + 7);
        if field != 4 && field != 8 && field != 16 {
            return c.fail("stz2: bad field size".into());
        }
    } else {
        fixed = c.rd32(sz.payload + 4);
    }
    let n_sizes = c.rd32(sz.payload + 8);
    let sizes = sz.payload + 12;
    if fixed == 0 && (n_sizes as u64) * (field as u64) / 8 > (sz.end - sz.payload - 12) as u64 {
        return c.fail("truncated 'stsz'".into());
    }
    let n_chunks = c.rd32(co.payload + 4);
    let offs = co.payload + 8;
    if (n_chunks as u64) * (if is_co64 { 8 } else { 4 }) > (co.end - co.payload - 8) as u64 {
        return c.fail("truncated 'stco'".into());
    }
    let n_sc = c.rd32(sc.payload + 4);
    let scs = sc.payload + 8;
    if (n_sc as u64) * 12 > (sc.end - sc.payload - 8) as u64 {
        return c.fail("truncated 'stsc'".into());
    }

    let mut s: u32 = 0;
    let mut i: u32 = 0;
    while i < n_sc && s < n_sizes {
        let first = c.rd32(scs + 12 * i as usize);
        let per = c.rd32(scs + 12 * i as usize + 4);
        let last = if i + 1 < n_sc {
            c.rd32(scs + 12 * (i as usize + 1)).wrapping_sub(1)
        } else {
            n_chunks
        };
        if first < 1 || last > n_chunks {
            return c.fail("stsc: chunk index out of range".into());
        }
        let mut ch = first;
        while ch <= last && s < n_sizes {
            let mut off: u64 = if is_co64 {
                c.rd64(offs + 8 * (ch as usize - 1))
            } else {
                c.rd32(offs + 4 * (ch as usize - 1)) as u64
            };
            let mut k: u32 = 0;
            while k < per && s < n_sizes {
                let size: u32 = if fixed != 0 {
                    fixed
                } else if field == 32 {
                    c.rd32(sizes + 4 * s as usize)
                } else if field == 16 {
                    c.rd16(sizes + 2 * s as usize)
                } else if field == 8 {
                    c.b(sizes + s as usize)
                } else if s & 1 != 0 {
                    c.b(sizes + (s / 2) as usize) & 15
                } else {
                    c.b(sizes + (s / 2) as usize) >> 4
                };
                if c.add_sample(t, off, size) != 0 {
                    return -1;
                }
                off = off.wrapping_add(size as u64);
                k += 1;
                s += 1;
            }
            if ch == u32::MAX {
                break;
            }
            ch += 1;
        }
        i += 1;
    }
    if s < n_sizes {
        return c.fail(format!("stsc/stco describe {s} of {n_sizes} samples"));
    }
    0
}

fn parse_fragments(c: &mut Ctx, t: &mut Mp4Track, trex_size: u32) -> i32 {
    let mut pos = 0usize;
    let mut moof = Box_::default();
    let size = c.d.len();
    let mut r;
    loop {
        r = c.next_box(&mut pos, size, &mut moof);
        if r <= 0 {
            break;
        }
        let mut p2 = moof.payload;
        let mut traf = Box_::default();
        let mut first_traf = true;
        let mut prev_end: u64 = moof.start as u64;
        if moof.type_ != fourcc(b'm', b'o', b'o', b'f') {
            continue;
        }
        loop {
            r = c.next_box(&mut p2, moof.end, &mut traf);
            if r <= 0 {
                break;
            }
            let mut tfhd = Box_::default();
            let mut trun = Box_::default();
            let mut def_size = trex_size;
            let mut first_trun = true;
            if traf.type_ != fourcc(b't', b'r', b'a', b'f') {
                continue;
            }
            let rr =
                c.find_child(traf.payload, traf.end, fourcc(b't', b'f', b'h', b'd'), &mut tfhd);
            if rr <= 0 {
                return if rr < 0 { -1 } else { c.fail("traf without tfhd".into()) };
            }
            if c.need(&tfhd, 8, "tfhd") != 0 {
                return -1;
            }
            let mut q = tfhd.payload;
            let flags = c.rd32(q) & 0xffffff;
            if c.rd32(q + 4) != t.track_id {
                first_traf = false;
                continue;
            }
            q += 8;
            {
                let n = 8
                    + if flags & 1 != 0 { 8 } else { 0 }
                    + if flags & 2 != 0 { 4 } else { 0 }
                    + if flags & 8 != 0 { 4 } else { 0 }
                    + if flags & 0x10 != 0 { 4 } else { 0 }
                    + if flags & 0x20 != 0 { 4 } else { 0 };
                if c.need(&tfhd, n, "tfhd") != 0 {
                    return -1;
                }
            }
            let base: u64 = if flags & 1 != 0 {
                let b = c.rd64(q);
                q += 8;
                b
            } else if (flags & 0x20000) != 0 || first_traf {
                moof.start as u64
            } else {
                prev_end
            };
            if flags & 2 != 0 {
                q += 4;
            }
            if flags & 8 != 0 {
                q += 4;
            }
            if flags & 0x10 != 0 {
                def_size = c.rd32(q);
            }
            first_traf = false;
            let mut p3 = traf.payload;
            loop {
                r = c.next_box(&mut p3, traf.end, &mut trun);
                if r <= 0 {
                    break;
                }
                if trun.type_ != fourcc(b't', b'r', b'u', b'n') {
                    continue;
                }
                if c.need(&trun, 8, "trun") != 0 {
                    return -1;
                }
                q = trun.payload;
                let tf = c.rd32(q) & 0xffffff;
                let n = c.rd32(q + 4);
                q += 8;
                let per_sample: u64 = (if tf & 0x100 != 0 { 4 } else { 0 })
                    + (if tf & 0x200 != 0 { 4 } else { 0 })
                    + (if tf & 0x400 != 0 { 4 } else { 0 })
                    + (if tf & 0x800 != 0 { 4 } else { 0 });
                let need_n: u64 = 8
                    + (if tf & 1 != 0 { 4 } else { 0 })
                    + (if tf & 4 != 0 { 4 } else { 0 })
                    + (n as u64) * per_sample;
                if need_n > (trun.end - trun.payload) as u64 {
                    return c.fail("truncated 'trun' box".into());
                }
                let mut off: u64 = if tf & 1 != 0 {
                    let o = base.wrapping_add(c.rd32(q) as i32 as i64 as u64);
                    q += 4;
                    o
                } else if first_trun {
                    base
                } else {
                    prev_end
                };
                if tf & 4 != 0 {
                    q += 4;
                }
                for _ in 0..n {
                    let mut size = def_size;
                    if tf & 0x100 != 0 {
                        q += 4;
                    }
                    if tf & 0x200 != 0 {
                        size = c.rd32(q);
                        q += 4;
                    }
                    if tf & 0x400 != 0 {
                        q += 4;
                    }
                    if tf & 0x800 != 0 {
                        q += 4;
                    }
                    if c.add_sample(t, off, size) != 0 {
                        return -1;
                    }
                    off = off.wrapping_add(size as u64);
                }
                prev_end = off;
                first_trun = false;
                t.fragmented = true;
            }
            if r < 0 {
                return -1;
            }
        }
        if r < 0 {
            return -1;
        }
    }
    if r < 0 {
        -1
    } else {
        0
    }
}

/// `mp4_demux(data, size, track_id, out, err, errlen)`: demuxes the file in `data`.
/// `track_id = 0` selects the first track with a `soun` handler. Returns the track, or the error
/// message.
pub fn mp4_demux(data: &[u8], track_id: u32) -> Result<Mp4Track, String> {
    let mut c = Ctx { d: data, err: String::new() };
    let mut t = Mp4Track { oti: -1, ..Default::default() };
    match demux(&mut c, track_id, &mut t) {
        0 => Ok(t),
        _ => Err(c.err),
    }
}

fn demux(c: &mut Ctx, track_id: u32, t: &mut Mp4Track) -> i32 {
    let size = c.d.len();
    let mut moov = Box_::default();
    let mut trak = Box_::default();
    let mut mvex = Box_::default();
    let mut b = Box_::default();
    let mut found = false;
    let mut trex_size: u32 = 0;

    let r = c.find_child(0, size, fourcc(b'm', b'o', b'o', b'v'), &mut moov);
    if r <= 0 {
        return if r < 0 { -1 } else { c.fail("no 'moov' box".into()) };
    }

    let mut pos = moov.payload;
    let mut r;
    loop {
        r = c.next_box(&mut pos, moov.end, &mut trak);
        if r <= 0 {
            break;
        }
        let mut tkhd = Box_::default();
        let mut mdia = Box_::default();
        let mut hdlr = Box_::default();
        if trak.type_ != fourcc(b't', b'r', b'a', b'k') {
            continue;
        }
        if c.find_child(trak.payload, trak.end, fourcc(b't', b'k', b'h', b'd'), &mut tkhd) <= 0
            || c.find_child(trak.payload, trak.end, fourcc(b'm', b'd', b'i', b'a'), &mut mdia) <= 0
        {
            continue;
        }
        if c.need(&tkhd, 24, "tkhd") != 0 {
            return -1;
        }
        let id = c.rd32(tkhd.payload + if c.b(tkhd.payload) == 1 { 20 } else { 12 });
        let skip = if track_id != 0 {
            id != track_id
        } else {
            c.find_child(mdia.payload, mdia.end, fourcc(b'h', b'd', b'l', b'r'), &mut hdlr) <= 0
                || c.need(&hdlr, 12, "hdlr") != 0
                || c.rd32(hdlr.payload + 8) != fourcc(b's', b'o', b'u', b'n')
        };
        if skip {
            continue;
        }
        t.track_id = id;
        found = true;
        let mut minf = Box_::default();
        if c.find_child(mdia.payload, mdia.end, fourcc(b'm', b'i', b'n', b'f'), &mut minf) <= 0
            || c.find_child(minf.payload, minf.end, fourcc(b's', b't', b'b', b'l'), &mut b) <= 0
        {
            return c.fail(format!("track {id}: no 'minf/stbl'"));
        }
        if parse_stbl(c, &b, t) != 0 {
            return -1;
        }
        break;
    }
    if r < 0 {
        return -1;
    }
    if !found {
        if track_id != 0 {
            return c.fail(format!("no track with ID {track_id}"));
        }
        return c.fail("no audio track".into());
    }
    if t.codec != b"mp4a" {
        let codec = String::from_utf8_lossy(&t.codec).into_owned();
        return c.fail(format!("unsupported sample entry '{codec}'"));
    }

    /* Movie fragments */
    let r = c.find_child(moov.payload, moov.end, fourcc(b'm', b'v', b'e', b'x'), &mut mvex);
    if r < 0 {
        return -1;
    }
    if r > 0 {
        pos = mvex.payload;
        let mut r;
        loop {
            r = c.next_box(&mut pos, mvex.end, &mut b);
            if r <= 0 {
                break;
            }
            if b.type_ == fourcc(b't', b'r', b'e', b'x')
                && b.end - b.payload >= 24
                && c.rd32(b.payload + 4) == t.track_id
            {
                trex_size = c.rd32(b.payload + 16);
            }
        }
        if r < 0 {
            return -1;
        }
        if parse_fragments(c, t, trex_size) != 0 {
            return -1;
        }
    }
    0
}
