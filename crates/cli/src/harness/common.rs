// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from scripts/ref-harness/common.h, scripts/ref-harness/common.c (the reference harness of
// this project, not part of fdk-aac).

//! The shared helpers of the command-line tools: SHA-256, the JSON Lines writer and the argument
//! parsing helpers of the reference harness (`scripts/ref-harness/common.c`). The logs must be
//! byte-identical to the reference harness's (PORTING.md, "Reference harness"), so the writer
//! reproduces the C writer exactly: keys in the order written, no whitespace, strings written as
//! they are.

use std::io::Write;

// ---- SHA-256 (FIPS 180-4) --------------------------------------------------------------------

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// A SHA-256 state.
pub struct Sha256 {
    h: [u32; 8],
    len: u64,
    buf: [u8; 64],
    fill: usize,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// A new state.
    pub fn new() -> Self {
        Sha256 {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            len: 0,
            buf: [0; 64],
            fill: 0,
        }
    }

    fn block(&mut self, p: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([p[4 * i], p[4 * i + 1], p[4 * i + 2], p[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.h;
        for i in 0..64 {
            let t1 = h
                .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
                .wrapping_add((e & f) ^ (!e & g))
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
                .wrapping_add((a & b) ^ (a & c) ^ (b & c));
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in self.h.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *x = x.wrapping_add(y);
        }
    }

    /// Adds data.
    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let k = (64 - self.fill).min(data.len());
            self.buf[self.fill..self.fill + k].copy_from_slice(&data[..k]);
            self.fill += k;
            data = &data[k..];
            if self.fill == 64 {
                let b = self.buf;
                self.block(&b);
                self.fill = 0;
            }
        }
    }

    /// The digest.
    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        let len = self.len;
        self.update(&[0x80]);
        while self.fill != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        self.len = len;
        let mut out = [0u8; 32];
        for i in 0..8 {
            out[4 * i..4 * i + 4].copy_from_slice(&self.h[i].to_be_bytes());
        }
        out
    }
}

/// The lowercase hex digest of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut s = Sha256::new();
    s.update(data);
    hex(&s.finish())
}

/// Lowercase hex.
pub fn hex(data: &[u8]) -> String {
    const HX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push(HX[(b >> 4) as usize] as char);
        s.push(HX[(b & 15) as usize] as char);
    }
    s
}

// ---- JSON Lines writer -----------------------------------------------------------------------

/// The JSON Lines writer (`Json` of common.c): one record per line, keys in the order written,
/// no whitespace. Without an output nothing is written.
pub struct Json {
    out: Option<Box<dyn Write>>,
    need_comma: bool,
    error: bool,
}

impl Json {
    /// Whether a log is written.
    pub fn enabled(&self) -> bool {
        self.out.is_some()
    }

    /// A writer to `out` (`None`: no log).
    pub fn new(out: Option<Box<dyn Write>>) -> Self {
        Json { out, need_comma: false, error: false }
    }

    fn put(&mut self, b: &[u8]) {
        if let Some(o) = self.out.as_mut() {
            if o.write_all(b).is_err() {
                self.error = true;
            }
        }
    }

    fn key(&mut self, key: Option<&str>) {
        if self.need_comma {
            self.put(b",");
        }
        if let Some(k) = key {
            self.put(b"\"");
            self.put(k.as_bytes());
            self.put(b"\":");
        }
        self.need_comma = true;
    }

    /// Starts a record (`{"call":"..."`).
    pub fn begin(&mut self, call: &str) {
        if self.out.is_none() {
            return;
        }
        self.put(b"{\"call\":\"");
        self.put(call.as_bytes());
        self.put(b"\"");
        self.need_comma = true;
    }

    /// Ends a record.
    pub fn end(&mut self) {
        if self.out.is_none() {
            return;
        }
        self.put(b"}\n");
        self.need_comma = false;
    }

    /// An integer member.
    pub fn int(&mut self, key: &str, v: i64) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.put(v.to_string().as_bytes());
    }

    /// An unsigned integer member.
    pub fn uint(&mut self, key: &str, v: u64) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.put(v.to_string().as_bytes());
    }

    /// A boolean member.
    pub fn boolean(&mut self, key: &str, v: bool) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.put(if v { b"true" } else { b"false" });
    }

    /// A string member, written as it is (the bytes must not need escaping).
    pub fn str_bytes(&mut self, key: &str, v: &[u8]) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.put(b"\"");
        self.put(v);
        self.put(b"\"");
    }

    /// A string member.
    pub fn str(&mut self, key: &str, v: &str) {
        self.str_bytes(key, v.as_bytes());
    }

    /// A hex string member.
    pub fn hex(&mut self, key: &str, data: &[u8]) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.put(b"\"");
        self.put(hex(data).as_bytes());
        self.put(b"\"");
    }

    /// A `null` member.
    pub fn null(&mut self, key: &str) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.put(b"null");
    }

    /// Starts an object member (`key` `None`: an array element).
    pub fn obj_begin(&mut self, key: Option<&str>) {
        if self.out.is_none() {
            return;
        }
        self.key(key);
        self.put(b"{");
        self.need_comma = false;
    }

    /// Ends an object.
    pub fn obj_end(&mut self) {
        if self.out.is_none() {
            return;
        }
        self.put(b"}");
        self.need_comma = true;
    }

    /// Starts an array member.
    pub fn arr_begin(&mut self, key: &str) {
        if self.out.is_none() {
            return;
        }
        self.key(Some(key));
        self.put(b"[");
        self.need_comma = false;
    }

    /// An integer array element.
    pub fn arr_int(&mut self, v: i64) {
        if self.out.is_none() {
            return;
        }
        self.key(None);
        self.put(v.to_string().as_bytes());
    }

    /// Ends an array.
    pub fn arr_end(&mut self) {
        if self.out.is_none() {
            return;
        }
        self.put(b"]");
        self.need_comma = true;
    }

    /// Flushes the output; whether all writes succeeded.
    pub fn flush(&mut self) -> bool {
        if let Some(o) = self.out.as_mut() {
            if o.flush().is_err() {
                self.error = true;
            }
        }
        !self.error
    }
}

// ---- Utilities -------------------------------------------------------------------------------

/// The error of `die()`: the message; the program exits with code 2.
#[derive(Debug)]
pub struct Die(pub String);

/// `die(fmt, ...)` as an error value.
pub fn die<T>(msg: impl Into<String>) -> Result<T, Die> {
    Err(Die(msg.into()))
}

/// `strtoll(s, &end, 0)`: the value and the number of bytes consumed (0 if no number was
/// parsed), or `None` on overflow (`ERANGE`).
fn strtoll0(s: &[u8]) -> (Option<i64>, usize) {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut base = 10u32;
    if i < s.len() && s[i] == b'0' {
        if i + 1 < s.len()
            && (s[i + 1] == b'x' || s[i + 1] == b'X')
            && i + 2 < s.len()
            && (s[i + 2] as char).is_ascii_hexdigit()
        {
            base = 16;
            i += 2;
        } else {
            base = 8;
        }
    }
    let start = i;
    let mut acc: u128 = 0;
    let mut overflow = false;
    while i < s.len() {
        let d = match (s[i] as char).to_digit(base) {
            Some(d) => d,
            None => break,
        };
        acc = acc.saturating_mul(base as u128).saturating_add(d as u128);
        if acc > (i64::MAX as u128) + 1 {
            overflow = true;
        }
        i += 1;
    }
    if i == start {
        // A lone "0" before an invalid hex prefix or digit: strtoll parsed "0" (base 8 case),
        // otherwise nothing.
        if base == 8 {
            return (Some(0), start);
        }
        return (None, 0);
    }
    if overflow || (!neg && acc > i64::MAX as u128) {
        return (None, i);
    }
    let v = if neg { (acc as i128).wrapping_neg() as i64 } else { acc as i64 };
    (Some(v), i)
}

/// `parse_int(s, what)`: a decimal, `0x` hexadecimal, octal or negative integer
/// (`strtoll(s, &end, 0)` without trailing characters).
pub fn parse_int(s: &str, what: &str) -> Result<i64, Die> {
    let b = s.as_bytes();
    let (v, used) = strtoll0(b);
    match v {
        Some(v) if used > 0 && used == b.len() => Ok(v),
        _ => die(format!("invalid integer for {what}: '{s}'")),
    }
}

/// `parse_hex(s, out, what)`: a hex string (whitespace ignored).
pub fn parse_hex(s: &str, what: &str) -> Result<Vec<u8>, Die> {
    let mut out = Vec::new();
    let mut hi: Option<u8> = None;
    for &c in s.as_bytes() {
        if c.is_ascii_whitespace() || c == 0x0b {
            continue;
        }
        let v = match (c as char).to_digit(16) {
            Some(v) => v as u8,
            None => return die(format!("invalid hex digit in {what}")),
        };
        match hi {
            None => hi = Some(v),
            Some(h) => {
                out.push(h << 4 | v);
                hi = None;
            }
        }
    }
    if hi.is_some() {
        return die(format!("odd number of hex digits in {what}"));
    }
    Ok(out)
}

/// `read_file(path, size)`: the whole file.
pub fn read_file(path: &str) -> Result<Vec<u8>, Die> {
    std::fs::read(path).or_else(|_| die(format!("cannot open {path}")))
}

/// `open_out(path)`: an output file; `-` is stdout, no path is no output.
pub fn open_out(path: Option<&str>) -> Result<Option<Box<dyn Write>>, Die> {
    match path {
        None => Ok(None),
        Some("-") => Ok(Some(Box::new(std::io::BufWriter::new(std::io::stdout())))),
        Some(p) => match std::fs::File::create(p) {
            Ok(f) => Ok(Some(Box::new(std::io::BufWriter::with_capacity(1 << 16, f)))),
            Err(_) => die(format!("cannot create {p}")),
        },
    }
}

/// `split_at(s, rest, what)`: splits `PREFIX:REST` at the first `:` and parses the prefix.
pub fn split_at<'a>(s: &'a str, what: &str) -> Result<(i64, &'a str), Die> {
    match s.find(':') {
        Some(c) if c < 64 => {
            let v = parse_int(&s[..c], what)?;
            Ok((v, &s[c + 1..]))
        }
        _ => die(format!("expected N:... for {what}: '{s}'")),
    }
}

/// `snprintf(buf, size, "%s", s)`: `s` truncated to `size - 1` bytes (at a byte boundary; the
/// result is used for ASCII tokens only).
pub fn truncate_c(s: &str, size: usize) -> &str {
    let b = s.as_bytes();
    if b.len() < size {
        return s;
    }
    let mut n = size - 1;
    while n > 0 && !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

/// `strtok_r(s, delims)`: the non-empty tokens.
pub fn strtok<'a>(s: &'a str, delims: &'a [char]) -> impl Iterator<Item = &'a str> + 'a {
    s.split(move |c| delims.contains(&c)).filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn parse_int_like_strtoll() {
        assert_eq!(parse_int("10", "x").unwrap(), 10);
        assert_eq!(parse_int("-3", "x").unwrap(), -3);
        assert_eq!(parse_int("0x1F", "x").unwrap(), 31);
        assert_eq!(parse_int("010", "x").unwrap(), 8);
        assert_eq!(parse_int("0", "x").unwrap(), 0);
        assert_eq!(parse_int(" 7", "x").unwrap(), 7);
        assert!(parse_int("08", "x").is_err());
        assert!(parse_int("0x", "x").is_err());
        assert!(parse_int("", "x").is_err());
        assert!(parse_int("1a", "x").is_err());
        assert!(parse_int("99999999999999999999", "x").is_err());
    }
}
