//! `omt-nbt` — Named Binary Tag support for OmniMCTools.
//!
//! Covers everything the tool needs at disk level:
//! - big-endian (Java) and little-endian (Bedrock disk) NBT,
//! - raw / gzip / zlib containers with sniffing,
//! - the 8-byte Bedrock `level.dat` header,
//! - SNBT rendering and a human-readable tree dump.
//!
//! Clean-room implemented from the on-disk format; RE evidence from UMT
//! (`umt_re_evidence/`, research report §4.4) only fixed the *scope*: tag ids
//! 0–12 exactly, both byte orders, root compound with (possibly empty) name.

pub mod container;
pub mod document;
pub mod edit;
pub mod error;
pub mod level_dat;
pub mod snbt;
pub mod string;
pub mod tag;
pub mod tree;

pub use container::Container;
pub use document::{NbtDocument, NbtFormat};
pub use edit::{ChildId, EditError, EditOp, PathSeg, TagPath, UndoStack};
pub use error::{Error, Result};
pub use level_dat::BedrockLevelDat;
pub use tag::{ByteOrder, Nbt, Tag, TagId};

/// Read one NBT root (id + name + payload) from `buf` using `order`.
///
/// Errors with [`Error::TrailingData`] when bytes remain after the root tag;
/// use [`read_prefix`] to tolerate (and report) trailing data.
pub fn read(buf: &[u8], order: ByteOrder) -> Result<Nbt> {
    let (nbt, consumed) = read_prefix(buf, order)?;
    if buf.len() > consumed {
        return Err(Error::TrailingData(buf.len() - consumed));
    }
    Ok(nbt)
}

/// Read one NBT root, returning how many bytes it consumed. Region-file chunk
/// payloads are followed by other chunks' data, so this is the normal entry
/// point for fragmented streams.
pub fn read_prefix(buf: &[u8], order: ByteOrder) -> Result<(Nbt, usize)> {
    let mut r = Reader::new(buf, order);
    let id = r.u8()?;
    if id == 0 {
        return Err(Error::InvalidTagId(0));
    }
    let tag_id = TagId::from_id(id).ok_or(Error::InvalidTagId(id))?;
    let name = r.string()?;
    let tag = r.payload(tag_id, 1)?;
    Ok((Nbt { name, tag }, r.pos))
}

/// Serialize an NBT root (id byte + name + payload).
///
/// Fails on in-memory trees that cannot be encoded faithfully: heterogeneous
/// lists, and strings whose modified-UTF-8 encoding exceeds 65,535 bytes.
pub fn write(nbt: &Nbt, order: ByteOrder) -> Result<Vec<u8>> {
    validate::payload(&nbt.tag)?;
    let mut out = Vec::with_capacity(256);
    out.push(nbt.tag.id());
    write::push_string(&mut out, &nbt.name, order);
    write::push_payload(&mut out, &nbt.tag, order);
    Ok(out)
}

pub(crate) const MAX_DEPTH: u16 = 512;

/// Rejects in-memory trees the writer cannot encode faithfully. Runs once up
/// front so the write path itself stays infallible.
mod validate {
    use super::{string, Error, Result, Tag};

    pub(super) fn payload(tag: &Tag) -> Result<()> {
        match tag {
            Tag::String(v) => {
                if string::encoded_len(v) > u16::MAX as usize {
                    return Err(Error::StringTooLong(string::encoded_len(v)));
                }
                Ok(())
            }
            Tag::List(items) => {
                if let Some(first) = items.first() {
                    let id = first.id();
                    for (i, item) in items.iter().enumerate() {
                        if item.id() != id {
                            return Err(Error::ListTypeMismatch { declared: id, actual: item.id(), index: i });
                        }
                    }
                }
                for item in items {
                    payload(item)?;
                }
                Ok(())
            }
            Tag::Compound(entries) => {
                for (name, child) in entries {
                    if string::encoded_len(name) > u16::MAX as usize {
                        return Err(Error::StringTooLong(string::encoded_len(name)));
                    }
                    payload(child)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Streaming reader over a byte slice. Public so sibling crates can parse
/// embedded NBT-ish structures (Bedrock subchunk palettes, actor payloads)
/// that do not start with a full NBT root header.
pub struct Reader<'a> {
    buf: &'a [u8],
    pub(crate) pos: usize,
    le: bool,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8], order: ByteOrder) -> Self {
        Self { buf, pos: 0, le: order == ByteOrder::Little }
    }

    /// Construct a reader positioned at `offset` (for payload-follows-prefix
    /// formats such as Anvil chunk bodies).
    pub fn with_offset(buf: &'a [u8], order: ByteOrder, offset: usize) -> Self {
        Self { buf, pos: offset.min(buf.len()), le: order == ByteOrder::Little }
    }

    /// Bytes consumed so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Read one named payload: tag id byte, name, payload. Returns `None`
    /// when the next byte is `TAG_End` (0) — the terminator for loose
    /// compound-style sequences like Bedrock palette entries.
    pub fn read_named(&mut self) -> Result<Option<(String, Tag)>> {
        let id = self.u8()?;
        if id == 0 {
            return Ok(None);
        }
        let tag_id = TagId::from_id(id).ok_or(Error::InvalidTagId(id))?;
        let name = self.string()?;
        Ok(Some((name, self.payload(tag_id, 1)?)))
    }

    #[inline]
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let rest = self.buf.len() - self.pos;
        if rest < n {
            return Err(Error::UnexpectedEof { offset: self.pos, needed: n, available: rest });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(if self.le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(if self.le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    pub fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        let arr = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
        Ok(if self.le { u64::from_le_bytes(arr) } else { u64::from_be_bytes(arr) })
    }

    pub fn string(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        if len > self.remaining() {
            return Err(Error::AbsurdLength { what: "string", len: len as u64, available: self.remaining() });
        }
        let bytes = self.take(len)?;
        Ok(string::decode_modified_utf8(bytes))
    }

    fn check_count(&self, what: &'static str, count: u64, min_elem_bytes: u64) -> Result<usize> {
        let avail = self.remaining() as u64;
        if count.saturating_mul(min_elem_bytes) > avail {
            return Err(Error::AbsurdLength { what, len: count, available: avail as usize });
        }
        Ok(count as usize)
    }

    pub(crate) fn payload(&mut self, id: TagId, depth: u16) -> Result<Tag> {
        if depth > MAX_DEPTH {
            return Err(Error::DepthLimitExceeded(MAX_DEPTH));
        }
        Ok(match id {
            TagId::Byte => Tag::Byte(self.u8()? as i8),
            TagId::Short => Tag::Short(self.u16()? as i16),
            TagId::Int => Tag::Int(self.u32()? as i32),
            TagId::Long => Tag::Long(self.u64()? as i64),
            TagId::Float => Tag::Float(f32::from_bits(self.u32()?)),
            TagId::Double => Tag::Double(f64::from_bits(self.u64()?)),
            TagId::ByteArray => {
                let count = self.u32()? as u64;
                let n = self.check_count("byte array", count, 1)?;
                Tag::ByteArray(self.take(n)?.iter().map(|&b| b as i8).collect())
            }
            TagId::String => Tag::String(self.string()?),
            TagId::List => {
                let elem_raw = self.u8()?;
                let count = self.u32()? as u64;
                if elem_raw == 0 {
                    // End-typed lists must be empty; anything else is malformed.
                    if count != 0 {
                        return Err(Error::InvalidTagId(0));
                    }
                    Tag::List(Vec::new())
                } else {
                    let elem = TagId::from_id(elem_raw).ok_or(Error::InvalidTagId(elem_raw))?;
                    // Smallest possible payload is 1 byte; this guards against
                    // hostile counts trying to allocate gigabytes.
                    let n = self.check_count("list", count, 1)?;
                    let mut v = Vec::with_capacity(n.min(4096));
                    for _ in 0..n {
                        v.push(self.payload(elem, depth + 1)?);
                    }
                    Tag::List(v)
                }
            }
            TagId::Compound => {
                let mut entries = Vec::new();
                loop {
                    let id = self.u8()?;
                    if id == 0 {
                        break;
                    }
                    let tid = TagId::from_id(id).ok_or(Error::InvalidTagId(id))?;
                    let name = self.string()?;
                    let tag = self.payload(tid, depth + 1)?;
                    entries.push((name, tag));
                }
                Tag::Compound(entries)
            }
            TagId::IntArray => {
                let count = self.u32()? as u64;
                let n = self.check_count("int array", count, 4)?;
                let mut v = Vec::with_capacity(n.min(1 << 20));
                for _ in 0..n {
                    v.push(self.u32()? as i32);
                }
                Tag::IntArray(v)
            }
            TagId::LongArray => {
                let count = self.u32()? as u64;
                let n = self.check_count("long array", count, 8)?;
                let mut v = Vec::with_capacity(n.min(1 << 20));
                for _ in 0..n {
                    v.push(self.u64()? as i64);
                }
                Tag::LongArray(v)
            }
        })
    }
}

pub mod write {
    use super::{string, ByteOrder, Tag};

    /// Public wrapper: append a payload (no tag id / name).
    pub fn push_payload_pub(out: &mut Vec<u8>, tag: &Tag, ) {
        push_payload(out, tag, ByteOrder::Little)
    }

    /// Public wrapper: append an empty name in the given order.
    pub fn push_string_pub(out: &mut Vec<u8>, s: &str) {
        push_string(out, s, ByteOrder::Little)
    }

    /// Append a length-prefixed, modified-UTF-8 string. Callers must have run
    /// `validate::payload` (or otherwise ensured the encoding fits in u16).
    pub(super) fn push_string(out: &mut Vec<u8>, s: &str, order: ByteOrder) {
        let start = out.len();
        out.extend_from_slice(&[0, 0]); // length placeholder
        let len_start = out.len();
        string::encode_modified_utf8(s, out);
        let len = out.len() - len_start;
        debug_assert!(len <= u16::MAX as usize, "validate::payload must run first");
        let bytes = (len as u16).to_bytes(order);
        out[start] = bytes[0];
        out[start + 1] = bytes[1];
    }

    pub(super) fn push_payload(out: &mut Vec<u8>, tag: &Tag, order: ByteOrder) {
        match tag {
            Tag::Byte(v) => out.push(*v as u8),
            Tag::Short(v) => out.extend_from_slice(&order.arr2(*v as u16)),
            Tag::Int(v) => out.extend_from_slice(&order.arr4(*v as u32)),
            Tag::Long(v) => out.extend_from_slice(&order.arr8(*v as u64)),
            Tag::Float(v) => out.extend_from_slice(&order.arr4(v.to_bits())),
            Tag::Double(v) => out.extend_from_slice(&order.arr8(v.to_bits())),
            Tag::ByteArray(v) => {
                out.extend_from_slice(&order.arr4(v.len() as u32));
                for b in v {
                    out.push(*b as u8);
                }
            }
            Tag::String(v) => push_string(out, v, order),
            Tag::List(items) => {
                match items.first() {
                    Some(first) => out.push(first.id()),
                    None => out.push(0),
                }
                out.extend_from_slice(&order.arr4(items.len() as u32));
                for item in items {
                    push_payload(out, item, order);
                }
            }
            Tag::Compound(entries) => {
                for (name, child) in entries {
                    out.push(child.id());
                    push_string(out, name, order);
                    push_payload(out, child, order);
                }
                out.push(0);
            }
            Tag::IntArray(v) => {
                out.extend_from_slice(&order.arr4(v.len() as u32));
                for i in v {
                    out.extend_from_slice(&order.arr4(*i as u32));
                }
            }
            Tag::LongArray(v) => {
                out.extend_from_slice(&order.arr4(v.len() as u32));
                for i in v {
                    out.extend_from_slice(&order.arr8(*i as u64));
                }
            }
        }
    }

    trait ToBytes {
        fn to_bytes(self, order: ByteOrder) -> [u8; 2];
    }
    impl ToBytes for u16 {
        fn to_bytes(self, order: ByteOrder) -> [u8; 2] {
            match order {
                ByteOrder::Big => self.to_be_bytes(),
                ByteOrder::Little => self.to_le_bytes(),
            }
        }
    }

    impl ByteOrder {
        #[inline]
        pub(super) fn arr2(self, v: u16) -> [u8; 2] {
            match self {
                ByteOrder::Big => v.to_be_bytes(),
                ByteOrder::Little => v.to_le_bytes(),
            }
        }
        #[inline]
        pub(super) fn arr4(self, v: u32) -> [u8; 4] {
            match self {
                ByteOrder::Big => v.to_be_bytes(),
                ByteOrder::Little => v.to_le_bytes(),
            }
        }
        #[inline]
        pub(super) fn arr8(self, v: u64) -> [u8; 8] {
            match self {
                ByteOrder::Big => v.to_be_bytes(),
                ByteOrder::Little => v.to_le_bytes(),
            }
        }
    }
}
