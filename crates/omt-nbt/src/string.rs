//! NBT strings use Java's *modified UTF-8* (JVM CESU-8 variant):
//! - U+0000 encodes as `C0 80`,
//! - astral plane chars (U+10000..) encode as a 6-byte UTF-16 surrogate pair.
//!
//! Decoding is lossy: byte sequences that are invalid modified UTF-8 decode
//! to U+FFFD rather than failing the whole parse (real worlds contain junk),
//! but correct input round-trips exactly.

/// Number of bytes `s` occupies in modified UTF-8.
pub fn encoded_len(s: &str) -> usize {
    let mut n = 0usize;
    for c in s.chars() {
        let cp = c as u32;
        n += if cp == 0 {
            2
        } else if cp <= 0x7f {
            1
        } else if cp <= 0x7ff {
            2
        } else if cp <= 0xffff {
            3
        } else {
            6 // surrogate pair
        };
    }
    n
}

/// Encode `s` into modified UTF-8, appending to `out`.
pub fn encode_modified_utf8(s: &str, out: &mut Vec<u8>) {
    for c in s.chars() {
        let cp = c as u32;
        if cp == 0 {
            out.extend_from_slice(&[0xc0, 0x80]);
        } else if cp <= 0x7f {
            out.push(cp as u8);
        } else if cp <= 0x7ff {
            out.push(0xc0 | (cp >> 6) as u8);
            out.push(0x80 | (cp & 0x3f) as u8);
        } else if cp <= 0xffff {
            out.push(0xe0 | (cp >> 12) as u8);
            out.push(0x80 | ((cp >> 6) & 0x3f) as u8);
            out.push(0x80 | (cp & 0x3f) as u8);
        } else {
            let v = cp - 0x10000;
            let hi = 0xd800 + (v >> 10);
            let lo = 0xdc00 + (v & 0x3ff);
            for unit in [hi, lo] {
                out.push(0xe0 | (unit >> 12) as u8);
                out.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                out.push(0x80 | (unit & 0x3f) as u8);
            }
        }
    }
}

const REPLACEMENT: char = '\u{fffd}';

/// Lossy decode of modified UTF-8 (also accepts plain UTF-8; astral chars
/// arrive either as one 4-byte sequence or a CESU-8 surrogate pair).
pub fn decode_modified_utf8(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        if b0 < 0x80 {
            out.push(b0 as char);
            i += 1;
            continue;
        }
        let (len, mut cp) = match b0 {
            0xc0..=0xdf => (2usize, (b0 & 0x1f) as u32),
            0xe0..=0xef => (3usize, (b0 & 0x0f) as u32),
            0xf0..=0xf4 => (4usize, (b0 & 0x07) as u32), // plain UTF-8 astral
            _ => {
                out.push(REPLACEMENT);
                i += 1;
                continue;
            }
        };
        if i + len > bytes.len() {
            out.push(REPLACEMENT);
            break;
        }
        let mut ok = true;
        for k in 1..len {
            let b = bytes[i + k];
            if b & 0xc0 != 0x80 {
                ok = false;
                break;
            }
            cp = (cp << 6) | (b & 0x3f) as u32;
        }
        if !ok {
            out.push(REPLACEMENT);
            i += 1;
            continue;
        }
        // Combine CESU-8 surrogate pairs (0xd800..=0xdfff singles → replacement).
        if (0xd800..=0xdfff).contains(&cp) {
            if len == 3 && cp <= 0xdbff && i + 3 < bytes.len() {
                // try low surrogate
                let e1 = bytes[i + 3];
                if e1 & 0xf0 == 0xe0 {
                    let e2 = if i + 4 < bytes.len() { bytes[i + 4] } else { 0 };
                    let e3 = if i + 5 < bytes.len() { bytes[i + 5] } else { 0 };
                    let lo = ((e1 as u32 & 0x0f) << 12)
                        | ((e2 as u32 & 0x3f) << 6)
                        | (e3 as u32 & 0x3f);
                    if (0xdc00..=0xdfff).contains(&lo)
                        && e2 & 0xc0 == 0x80
                        && e3 & 0xc0 == 0x80
                    {
                        let combined = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                        if let Some(c) = char::from_u32(combined) {
                            out.push(c);
                            i += 6;
                            continue;
                        }
                    }
                }
            }
            out.push(REPLACEMENT);
            i += len;
            continue;
        }
        match char::from_u32(cp) {
            Some(c) => out.push(c),
            None => out.push(REPLACEMENT),
        }
        i += len;
    }
    out
}
