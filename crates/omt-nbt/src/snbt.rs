//! SNBT rendering (the stringified-NBT syntax used by vanilla commands and
//! data packs). Serialization only for now; the parser lands with the NBT
//! editor in M2.

use crate::Tag;

/// Render in one line (clipboard-friendly).
pub fn to_snbt(tag: &Tag) -> String {
    let mut s = String::new();
    render(tag, &mut s, None, 0);
    s
}

/// Render with newlines and two-space indentation.
pub fn to_snbt_pretty(tag: &Tag) -> String {
    let mut s = String::new();
    render(tag, &mut s, Some(2), 0);
    s
}

fn render(tag: &Tag, out: &mut String, indent: Option<usize>, depth: usize) {
    match tag {
        Tag::Byte(v) => out.push_str(&format!("{v}b")),
        Tag::Short(v) => out.push_str(&format!("{v}s")),
        Tag::Int(v) => out.push_str(&format!("{v}")),
        Tag::Long(v) => out.push_str(&format!("{v}L")),
        Tag::Float(v) => {
            out.push_str(&float_snbt(*v as f64, "f", v.fract() == 0.0));
        }
        Tag::Double(v) => out.push_str(&float_snbt(*v, "d", v.fract() == 0.0)),
        Tag::ByteArray(v) => render_array(out, "B", v.iter().map(|b| b.to_string()), v.len(), indent, depth),
        Tag::String(v) => push_quoted(out, v),
        Tag::List(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            // In pretty mode, only break lists whose elements are themselves
            // containers; scalar lists stay inline (vanilla style).
            let break_lines = indent.is_some()
                && items.iter().any(|t| matches!(t, Tag::Compound(_) | Tag::List(_)));
            render_seq(out, items.iter(), items.len(), break_lines.then_some(indent.unwrap_or(2)), depth, |item, o, ind, d| {
                render(item, o, ind, d)
            });
        }
        Tag::Compound(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            let (open, sep, close) = match indent {
                Some(_) => ("{\n", ",\n", "}"),
                None => ("{", ", ", "}"),
            };
            out.push_str(open);
            for (i, (name, child)) in entries.iter().enumerate() {
                if let Some(w) = indent {
                    push_indent(out, w, depth + 1);
                }
                push_key(out, name);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                render(child, out, indent, depth + 1);
                if i + 1 < entries.len() {
                    out.push_str(sep);
                } else if indent.is_some() {
                    out.push('\n');
                }
            }
            if let Some(w) = indent {
                push_indent(out, w, depth);
            }
            out.push_str(close);
        }
        Tag::IntArray(v) => render_array(out, "I", v.iter().map(|i| i.to_string()), v.len(), indent, depth),
        Tag::LongArray(v) => render_array(out, "L", v.iter().map(|i| i.to_string()), v.len(), indent, depth),
    }
}

fn render_array(
    out: &mut String,
    kind: &str,
    items: impl ExactSizeIterator<Item = String>,
    len: usize,
    indent: Option<usize>,
    depth: usize,
) {
    if len == 0 {
        out.push_str(&format!("[{kind};]"));
        return;
    }
    out.push_str(&format!("[{kind}; "));
    for (i, item) in items.enumerate() {
        out.push_str(&item);
        if i + 1 < len {
            out.push_str(", ");
        }
    }
    out.push(']');
    let _ = (indent, depth); // arrays stay inline
}

fn render_seq<'a>(
    out: &mut String,
    items: impl Iterator<Item = &'a Tag>,
    len: usize,
    indent: Option<usize>,
    depth: usize,
    mut f: impl FnMut(&Tag, &mut String, Option<usize>, usize),
) {
    out.push('[');
    for (i, item) in items.enumerate() {
        if let Some(w) = indent
            && i > 0
        {
            push_indent(out, w, depth + 1);
        }
        f(item, out, child_indent(indent, item), depth + 1);
        if i + 1 < len {
            out.push_str(if indent.is_some() { ",\n" } else { ", " });
        }
    }
    if len > 0
        && let Some(w) = indent
    {
        push_indent(out, w, depth);
    }
    out.push(']');
}

fn child_indent(indent: Option<usize>, _item: &Tag) -> Option<usize> {
    indent
}

fn push_indent(out: &mut String, width: usize, depth: usize) {
    for _ in 0..width * depth {
        out.push(' ');
    }
}

/// Keys may be unquoted when they look like vanilla identifiers.
fn push_key(out: &mut String, name: &str) {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '+') {
        out.push_str(name);
    } else {
        push_quoted(out, name);
    }
}

fn push_quoted(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
}

/// Vanilla prints floats with a decimal point or exponent; `1.0f`, `1e-3f`.
fn float_snbt(v: f64, suffix: &str, _is_whole: bool) -> String {
    if v.is_nan() {
        return format!("NaN{suffix}");
    }
    if v.is_infinite() {
        return format!("{}Inf{suffix}", if v > 0.0 { "" } else { "-" });
    }
    let mut s = format!("{v:?}"); // Debug = shortest round-trip representation
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        s.push_str(".0");
    }
    s.push_str(suffix);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars() {
        assert_eq!(to_snbt(&Tag::Byte(-3)), "-3b");
        assert_eq!(to_snbt(&Tag::Short(7)), "7s");
        assert_eq!(to_snbt(&Tag::Int(42)), "42");
        assert_eq!(to_snbt(&Tag::Long(-9)), "-9L");
        assert_eq!(to_snbt(&Tag::Float(1.5)), "1.5f");
        assert_eq!(to_snbt(&Tag::Double(2.0)), "2.0d");
        assert_eq!(to_snbt(&Tag::String("hi".into())), "\"hi\"");
    }

    #[test]
    fn containers() {
        let c = Tag::Compound(vec![
            ("plain".into(), Tag::Int(1)),
            ("with space".into(), Tag::String("v".into())),
            ("list".into(), Tag::List(vec![Tag::Int(1), Tag::Int(2)])),
        ]);
        assert_eq!(
            to_snbt(&c),
            "{plain:1, \"with space\":\"v\", list:[1, 2]}"
        );
        assert_eq!(to_snbt(&Tag::List(vec![])), "[]");
        assert_eq!(to_snbt(&Tag::IntArray(vec![1, 2, 3])), "[I; 1, 2, 3]");
    }
}

// ===========================================================================
// Parser (M2): from_snbt — the inverse of to_snbt above.
// ====================================================================================

#[derive(Debug, thiserror::Error)]
#[error("SNBT error at {line}:{col}: {message}")]
pub struct SnbtError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

pub type SnbtResult<T> = std::result::Result<T, SnbtError>;

/// Parse an SNBT string into a `Tag`. Accepts a bare value or a compound;
/// most callers pass the output of [`to_snbt`].
pub fn from_snbt(input: &str) -> SnbtResult<Tag> {
    let mut p = Parser::new(input);
    p.skip_ws();
    let tag = p.value()?;
    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(p.err("unexpected trailing characters"));
    }
    Ok(tag)
}

struct Parser<'a> {
    src: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self { src: text.as_bytes(), text, pos: 0 }
    }

    fn err(&self, message: impl Into<String>) -> SnbtError {
        let consumed = &self.text[..self.pos.min(self.text.len())];
        let line = consumed.matches('\n').count() + 1;
        let col = consumed.rfind('\n').map(|i| self.pos - i).unwrap_or(self.pos + 1);
        SnbtError { line, col, message: message.into() }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> SnbtResult<()> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(format!("expected {:?}, found {:?}", byte as char, self.peek().map(|b| b as char).unwrap_or('\0'))))
        }
    }

    /// A full value.
    fn value(&mut self) -> SnbtResult<Tag> {
        match self.peek() {
            Some(b'{') => self.compound(),
            Some(b'[') => self.list_or_array(),
            Some(b'"') => Ok(Tag::String(self.quoted_string()?)),
            Some(b'\'') => Ok(Tag::String(self.quoted_string()?)),
            _ => self.scalar(),
        }
    }

    fn compound(&mut self) -> SnbtResult<Tag> {
        self.expect(b'{')?;
        let mut entries: Vec<(String, Tag)> = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Tag::Compound(entries));
                }
                Some(b'"') | Some(b'\'') => {
                    let key = self.quoted_string()?;
                    self.skip_ws();
                    self.expect(b':')?;
                    self.skip_ws();
                    let value = self.value()?;
                    self.upsert(&mut entries, key, value);
                }
                Some(_) => {
                    let key = self.bareword("compound key");
                    self.skip_ws();
                    self.expect(b':')?;
                    self.skip_ws();
                    let value = self.value()?;
                    self.upsert(&mut entries, key, value);
                }
                None => return Err(self.err("unterminated compound")),
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {}
                _ => return Err(self.err("expected ',' or '}' in compound")),
            }
        }
    }

    /// Later duplicate keys win (matches vanilla command behavior).
    fn upsert(&self, entries: &mut Vec<(String, Tag)>, key: String, value: Tag) {
        if let Some(slot) = entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            entries.push((key, value));
        }
    }

    fn list_or_array(&mut self) -> SnbtResult<Tag> {
        self.expect(b'[')?;
        self.skip_ws();
        // Typed array prefix: [B; …] [I; …] [L; …] (also tolerate lowercase).
        if let Some(b) = self.peek()
            && matches!(b, b'B' | b'I' | b'L' | b'b' | b'i' | b'l')
        {
            let save = self.pos;
            let kind = b.to_ascii_uppercase();
            self.pos += 1;
            self.skip_ws();
            if self.peek() == Some(b';') {
                self.pos += 1;
                self.skip_ws();
                return self.typed_array(kind);
            }
            // Not an array marker — rewind (it was a bareword element).
            self.pos = save;
        }
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Tag::List(items));
                }
                None => return Err(self.err("unterminated list")),
                _ => {
                    items.push(self.value()?);
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => {
                            self.pos += 1;
                        }
                        Some(b']') => {}
                        _ => return Err(self.err("expected ',' or ']' in list")),
                    }
                }
            }
        }
    }

    fn typed_array(&mut self, kind: u8) -> SnbtResult<Tag> {
        let mut raw_numbers: Vec<Tag> = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                None => return Err(self.err("unterminated array")),
                _ => {
                    raw_numbers.push(self.scalar()?);
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => {
                            self.pos += 1;
                        }
                        Some(b']') => {}
                        _ => return Err(self.err("expected ',' or ']' in array")),
                    }
                }
            }
        }
        let numbers: Vec<&Tag> = raw_numbers.iter().collect();
        match kind {
            b'B' => Ok(Tag::ByteArray(
                numbers.iter().map(|t| i8::from_snbt_number(t)).collect::<SnbtResult<_>>()?,
            )),
            b'I' => Ok(Tag::IntArray(numbers.iter().map(|t| i32::from_snbt_number(t)).collect::<SnbtResult<_>>()?)),
            _ => Ok(Tag::LongArray(numbers.iter().map(|t| i64::from_snbt_number(t)).collect::<SnbtResult<_>>()?)),
        }
    }

    fn quoted_string(&mut self) -> SnbtResult<String> {
        let quote = self.bump().unwrap();
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string")),
                Some(b) if b == quote => return Ok(out),
                Some(b'\\') => match self.bump() {
                    Some(b'"') => out.push('"'),
                    Some(b'\'') => out.push('\''),
                    Some(b'\\') => out.push('\\'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'b') => out.push('\u{8}'),
                    Some(b'f') => out.push('\u{c}'),
                    Some(other) => {
                        return Err(self.err(format!("unknown escape \\{}", other as char)));
                    }
                    None => return Err(self.err("unterminated escape")),
                },
                Some(b) => {
                    // Collect the (possibly multi-byte) character.
                    let start = self.pos - 1;
                    let mut end = self.pos;
                    while end < self.src.len() && (self.src[end] & 0xc0) == 0x80 {
                        end += 1;
                    }
                    self.pos = end;
                    out.push_str(&self.text[start..end]);
                    let _ = b;
                }
            }
        }
    }

    /// A bareword: everything up to a SNBT delimiter.
    fn bareword(&mut self, what: &str) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b'{' | b'}' | b'[' | b']' | b':' | b',' | b'"' | b'\'' | b' ' | b'\t' | b'\n' | b'\r') {
                break;
            }
            self.pos += 1;
        }
        let _ = what;
        self.text[start..self.pos].to_string()
    }

    /// A scalar: numbers (with optional suffixes), booleans, bareword strings.
    fn scalar(&mut self) -> SnbtResult<Tag> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b'{' | b'}' | b'[' | b']' | b':' | b',' | b'"' | b'\'' | b' ' | b'\t' | b'\n' | b'\r') {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.err("expected a value"));
        }
        let word = &self.text[start..self.pos];
        parse_scalar(word).ok_or_else(|| {
            let p = self.clone_position(start);
            p.err(format!("invalid value {word:?}"))
        })
    }

    fn clone_position(&self, pos: usize) -> Parser<'a> {
        Parser { src: self.src, text: self.text, pos }
    }
}

/// Parse a bare scalar token into a Tag.
pub(crate) fn parse_scalar(word: &str) -> Option<Tag> {
    // Boolean sugar (vanilla command syntax).
    match word {
        "true" => return Some(Tag::Byte(1)),
        "false" => return Some(Tag::Byte(0)),
        _ => {}
    }
    // Suffix dispatch.
    let (body, suffix) = match word.as_bytes()[word.len() - 1].to_ascii_lowercase() {
        b'b' | b's' | b'l' | b'f' | b'd' => {
            (&word[..word.len() - 1], word.as_bytes()[word.len() - 1].to_ascii_lowercase())
        }
        _ => (word, 0u8),
    };
    match suffix {
        b'b' => body.parse::<i8>().ok().map(Tag::Byte).or_else(|| Some(Tag::String(word.to_string()))),
        b's' => body.parse::<i16>().ok().map(Tag::Short).or_else(|| Some(Tag::String(word.to_string()))),
        b'l' => body.parse::<i64>().ok().map(Tag::Long).or_else(|| Some(Tag::String(word.to_string()))),
        b'f' => body.parse::<f32>().ok().map(Tag::Float).or_else(|| Some(Tag::String(word.to_string()))),
        b'd' => body.parse::<f64>().ok().map(Tag::Double).or_else(|| Some(Tag::String(word.to_string()))),
        _ => {
            // Unsuffixed: i32 if it fits, else i64 (long), else double.
            if let Ok(i) = body.parse::<i32>() {
                Some(Tag::Int(i))
            } else if let Ok(l) = body.parse::<i64>() {
                Some(Tag::Long(l))
            } else if let Ok(f) = body.parse::<f64>() {
                Some(Tag::Double(f))
            } else {
                Some(Tag::String(word.to_string()))
            }
        }
    }
}

trait FromSnbtNumber {
    fn from_snbt_number(tag: &Tag) -> SnbtResult<Self>
    where
        Self: Sized;
}

impl FromSnbtNumber for i8 {
    fn from_snbt_number(tag: &Tag) -> SnbtResult<Self> {
        tag.as_i64()
            .and_then(|v| i8::try_from(v).ok())
            .ok_or_else(|| SnbtError { line: 0, col: 0, message: format!("array element {:?} is not a byte", crate::snbt::to_snbt(tag)) })
    }
}

impl FromSnbtNumber for i32 {
    fn from_snbt_number(tag: &Tag) -> SnbtResult<Self> {
        tag.as_i64()
            .and_then(|v| i32::try_from(v).ok())
            .ok_or_else(|| SnbtError { line: 0, col: 0, message: format!("array element {:?} is not an int", crate::snbt::to_snbt(tag)) })
    }
}

impl FromSnbtNumber for i64 {
    fn from_snbt_number(tag: &Tag) -> SnbtResult<Self> {
        tag.as_i64()
            .ok_or_else(|| SnbtError { line: 0, col: 0, message: format!("array element {:?} is not a long", crate::snbt::to_snbt(tag)) })
    }
}
