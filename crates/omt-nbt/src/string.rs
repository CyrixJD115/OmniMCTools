//! NBT string bytes ↔ in-memory `String`, **byte-faithful both ways**.
//!
//! Java (big-endian) NBT strings are Java's *modified UTF-8*: U+0000 encodes
//! as `C0 80` and astral chars as CESU-8 surrogate pairs. Bedrock
//! (little-endian) strings are plain UTF-8. Real worlds also contain strings
//! that are neither — raw `00` bytes, overlong sequences, lone surrogates,
//! arbitrary binary (e.g. `StorageKey` payloads). A no-op read→write must
//! reproduce those bytes exactly, or "saving" a record mutates it.
//!
//! Scheme: decoding maps every byte that is not part of a *canonical*
//! sequence for the byte order to a private-use escape char
//! `U+F4xx | U+F400..U+F4FF plane-15 PUA carrying the original byte`;
//! encoding emits that raw byte back. Valid chars that would land in the
//! escape range themselves get their whole original run escaped, so the
//! mapping is injective and round-trips for **any** input:

use crate::ByteOrder;

/// First char of the raw-byte escape range `U+F400..=U+F4FF` (plane-15 PUA).
const ESC_BASE: u32 = 0xF_4000;

fn mask_of(len: usize) -> u8 {
    match len {
        2 => 0x1f,
        3 => 0x0f,
        4 => 0x07,
        _ => 0,
    }
}

/// Number of bytes `s` occupies in the wire encoding for `order`.
pub fn encoded_len(s: &str, order: ByteOrder) -> usize {
    let mut n = 0usize;
    for c in s.chars() {
        let cp = c as u32;
        n += wire_len(cp, order);
    }
    n
}

fn wire_len(cp: u32, order: ByteOrder) -> usize {
    if (ESC_BASE..ESC_BASE + 0x100).contains(&cp) {
        return 1; // escaped raw byte
    }
    if cp == 0 {
        // Java writes NUL canonically as C0 80; Bedrock as a raw byte.
        return match order {
            ByteOrder::Big => 2,
            ByteOrder::Little => 1,
        };
    }
    if cp <= 0x7f {
        1
    } else if cp <= 0x7ff {
        2
    } else if cp <= 0xffff {
        3
    } else {
        match order {
            ByteOrder::Big => 6, // CESU-8 surrogate pair
            ByteOrder::Little => 4, // plain UTF-8
        }
    }
}

/// Encode `s` into wire bytes for `order`, appending to `out`.
pub fn encode(s: &str, order: ByteOrder, out: &mut Vec<u8>) {
    for c in s.chars() {
        encode_char(c as u32, order, out);
    }
}

fn encode_char(cp: u32, order: ByteOrder, out: &mut Vec<u8>) {
    if (ESC_BASE..ESC_BASE + 0x100).contains(&cp) {
        out.push((cp - ESC_BASE) as u8);
        return;
    }
    if cp == 0 {
        match order {
            ByteOrder::Big => out.extend_from_slice(&[0xc0, 0x80]),
            ByteOrder::Little => out.push(0),
        }
        return;
    }
    if cp <= 0x7f {
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
        let pair = [0xd800 + (v >> 10), 0xdc00 + (v & 0x3ff)];
        match order {
            ByteOrder::Big => {
                for unit in pair {
                    out.push(0xe0 | (unit >> 12) as u8);
                    out.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                    out.push(0x80 | (unit & 0x3f) as u8);
                }
            }
            ByteOrder::Little => {
                out.push(0xf0 | (cp >> 18) as u8);
                out.push(0x80 | ((cp >> 12) & 0x3f) as u8);
                out.push(0x80 | ((cp >> 6) & 0x3f) as u8);
                out.push(0x80 | (cp & 0x3f) as u8);
            }
        }
    }
}

/// Decode wire bytes for `order` into a `String`. Any byte run that is not a
/// canonical encoding for `order` (raw NUL, overlongs, lone surrogates,
/// 4-byte sequences in Java data, invalid UTF-8 in Bedrock data, and any run
/// that would decode into the escape range itself) is carried as escaped raw
/// bytes so [`encode`] reproduces the input exactly.
pub fn decode(bytes: &[u8], order: ByteOrder) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let b0 = bytes[i];
        if b0 < 0x80 && b0 != 0 {
            out.push(b0 as char);
            i += 1;
            continue;
        }
        if b0 == 0 {
            // Raw NUL is non-canonical in both wire formats (Java writes
            // C0 80); carry the byte.
            out.push(esc(0));
            i += 1;
            continue;
        }
        // Sequence head: determine length and constraints per byte order.
        let (len, allowed) = match (order, b0) {
            (ByteOrder::Big, 0xc2..=0xdf) | (ByteOrder::Little, 0xc2..=0xdf) => (2usize, true),
            (ByteOrder::Big, 0xc0 | 0xc1) => (2, true), // C0 80 = NUL (Java canonical)
            (ByteOrder::Big, 0xe0..=0xef) | (ByteOrder::Little, 0xe0..=0xef) => (3, true),
            (ByteOrder::Little, 0xf0..=0xf4) => (4, true), // plain UTF-8 astral
            _ => (0, false),
        };
        if !allowed || i + len > bytes.len() {
            out.push(esc(b0));
            i += 1;
            continue;
        }
        let mut cp = (b0 & mask_of(len)) as u32;
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
            out.push(esc(b0));
            i += 1;
            continue;
        }
        // Overlong / out-of-range checks for the plain-UTF-8 orders.
        let overlong = match len {
            2 => cp < 0x80,
            3 => cp < 0x800,
            4 => !(0x10000..=0x10FFFF).contains(&cp),
            _ => false,
        };
        let big_nul = order == ByteOrder::Big && len == 2 && cp == 0; // C0 80 (canonical Java NUL)
        if overlong && !big_nul {
            out.push(esc(b0));
            i += 1;
            continue;
        }
        // Java combines CESU-8 surrogate pairs; a lone surrogate is escaped.
        if (0xd800..=0xdfff).contains(&cp) {
            if order == ByteOrder::Big && len == 3 && cp <= 0xdbff && i + 6 <= bytes.len() {
                let mut lo: u32 = (bytes[i + 3] as u32 & 0x0f) << 12;
                if bytes[i + 3] & 0xf0 == 0xe0
                    && bytes[i + 4] & 0xc0 == 0x80
                    && bytes[i + 5] & 0xc0 == 0x80
                {
                    lo |= ((bytes[i + 4] as u32 & 0x3f) << 6) | (bytes[i + 5] as u32 & 0x3f);
                    if (0xdc00..=0xdfff).contains(&lo) {
                        let combined = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                        if let Some(c) = char::from_u32(combined) {
                            push_valid_or_escape(c as u32, &bytes[i..i + 6], &mut out);
                            i += 6;
                            continue;
                        }
                    }
                }
            }
            out.push(esc(b0));
            i += 1;
            continue;
        }
        match char::from_u32(cp) {
            Some(c) => {
                push_valid_or_escape(c as u32, &bytes[i..i + len], &mut out);
                i += len;
            }
            None => {
                out.push(esc(b0));
                i += 1;
            }
        }
    }
    out
}

/// A decoded char that is itself in the escape range must not enter the
/// string as-is (it would collide with raw-byte escapes): escape the whole
/// original run instead.
fn push_valid_or_escape(cp: u32, run: &[u8], out: &mut String) {
    if (ESC_BASE..ESC_BASE + 0x100).contains(&cp) {
        for &b in run {
            out.push(esc(b));
        }
    } else {
        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
    }
}

fn esc(b: u8) -> char {
    char::from_u32(ESC_BASE + b as u32).expect("plane-15 PUA")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(bytes: &[u8], order: ByteOrder) {
        let s = decode(bytes, order);
        let mut out = Vec::new();
        encode(&s, order, &mut out);
        assert_eq!(out, bytes, "round trip {bytes:02x?} via {s:?}");
        assert_eq!(encoded_len(&s, order), bytes.len());
    }

    #[test]
    fn canonical_text_round_trips_both_orders() {
        rt(b"hello", ByteOrder::Big);
        rt(b"hello", ByteOrder::Little);
        rt("café \u{e9}moji \u{1f600}".as_bytes(), ByteOrder::Little);
        rt(&[0xed, 0xa0, 0x80, 0xed, 0xb0, 0x80], ByteOrder::Big); // CESU pair = U+10000
    }

    #[test]
    fn java_nul_is_canonical_c080() {
        let s = decode(&[0xc0, 0x80], ByteOrder::Big);
        assert_eq!(s, "\0");
        let mut out = Vec::new();
        encode(&s, ByteOrder::Big, &mut out);
        assert_eq!(out, vec![0xc0, 0x80]);
    }

    #[test]
    fn raw_bytes_round_trip() {
        // Raw NUL, overlong C0 80 in Bedrock data, invalid tails, binary.
        rt(&[0x00], ByteOrder::Big);
        rt(&[0x00], ByteOrder::Little);
        rt(&[0xc0, 0x80], ByteOrder::Little); // overlong → raw bytes
        rt(&[0xff, 0xfe, 0x41], ByteOrder::Little);
        rt(&[0xf0, 0x9f, 0x98], ByteOrder::Little); // truncated UTF-8
        rt(&[0xf4, 0x00, 0x80], ByteOrder::Big); // invalid in mUTF8
        rt(b"\x00\x01storage\xff key", ByteOrder::Little);
        // 4-byte astral is canonical in Bedrock, escaped in Java.
        rt(&[0xf0, 0x9f, 0x98, 0x80], ByteOrder::Little);
        rt(&[0xf0, 0x9f, 0x98, 0x80], ByteOrder::Big);
        // A real plane-15 PUA char inside the escape range (would collide
        // with raw-byte escapes) survives via escaping.
        let pua = "\u{f4080}".to_string();
        assert!((0xF4000..0xF4100).contains(&(pua.chars().next().unwrap() as u32)));
        let mut enc = Vec::new();
        encode(&pua, ByteOrder::Big, &mut enc);
        let back = decode(&enc, ByteOrder::Big);
        let mut out2 = Vec::new();
        encode(&back, ByteOrder::Big, &mut out2);
        assert_eq!(out2, enc);
    }

    #[test]
    fn fuzz_random_bytes_round_trip() {
        // xorshift over interesting byte mixes; both orders must round-trip.
        let mut state = 0x12345678u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for case in 0..2000 {
            let len = (next() % 24) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| match case % 4 {
                0 => (next() & 0xff) as u8,
                1 => [0x00u8, 0x41, 0x80, 0xc0, 0xe0, 0xf0, 0xf4, 0xff][(next() % 8) as usize],
                2 => [0x00u8, 0xc0, 0x80, 0xed, 0xa0, 0x80, 0xf0, 0x9f][(next() % 8) as usize],
                _ => b"na\xffme"[..][(next() % 5) as usize],
            }).collect();
            rt(&bytes, ByteOrder::Big);
            rt(&bytes, ByteOrder::Little);
        }
    }
}
