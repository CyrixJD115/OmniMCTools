//! The in-memory NBT value tree.

use serde::{Serialize, Serializer};

/// Disk byte order. Java disk NBT is big-endian; Bedrock disk NBT is
/// little-endian. (Bedrock *network* NBT is a different varint-flavored
/// encoding and is out of scope for the disk tooling.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ByteOrder {
    Big,
    Little,
}

/// NBT type ids 0–12. `End` (0) exists only as a compound terminator / empty
/// list marker and is never a value, so it is absent from [`Tag`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagId {
    Byte,
    Short,
    Int,
    Long,
    Float,
    Double,
    ByteArray,
    String,
    List,
    Compound,
    IntArray,
    LongArray,
}

impl TagId {
    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            1 => Some(Self::Byte),
            2 => Some(Self::Short),
            3 => Some(Self::Int),
            4 => Some(Self::Long),
            5 => Some(Self::Float),
            6 => Some(Self::Double),
            7 => Some(Self::ByteArray),
            8 => Some(Self::String),
            9 => Some(Self::List),
            10 => Some(Self::Compound),
            11 => Some(Self::IntArray),
            12 => Some(Self::LongArray),
            _ => None,
        }
    }

    pub const fn id(self) -> u8 {
        match self {
            Self::Byte => 1,
            Self::Short => 2,
            Self::Int => 3,
            Self::Long => 4,
            Self::Float => 5,
            Self::Double => 6,
            Self::ByteArray => 7,
            Self::String => 8,
            Self::List => 9,
            Self::Compound => 10,
            Self::IntArray => 11,
            Self::LongArray => 12,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Byte => "TAG_Byte",
            Self::Short => "TAG_Short",
            Self::Int => "TAG_Int",
            Self::Long => "TAG_Long",
            Self::Float => "TAG_Float",
            Self::Double => "TAG_Double",
            Self::ByteArray => "TAG_Byte_Array",
            Self::String => "TAG_String",
            Self::List => "TAG_List",
            Self::Compound => "TAG_Compound",
            Self::IntArray => "TAG_Int_Array",
            Self::LongArray => "TAG_Long_Array",
        }
    }
}

/// A parsed NBT root: the root's name (often empty) and its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Nbt {
    pub name: String,
    pub tag: Tag,
}

/// An NBT value. Compounds keep entry order (stable diffs, faithful
/// write-back); lookups are linear which is fine for world-sized compounds.
#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(Vec<(String, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub const fn id(&self) -> u8 {
        match self {
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }

    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Byte(_) => "TAG_Byte",
            Self::Short(_) => "TAG_Short",
            Self::Int(_) => "TAG_Int",
            Self::Long(_) => "TAG_Long",
            Self::Float(_) => "TAG_Float",
            Self::Double(_) => "TAG_Double",
            Self::ByteArray(_) => "TAG_Byte_Array",
            Self::String(_) => "TAG_String",
            Self::List(_) => "TAG_List",
            Self::Compound(_) => "TAG_Compound",
            Self::IntArray(_) => "TAG_Int_Array",
            Self::LongArray(_) => "TAG_Long_Array",
        }
    }

    // ---- Compound helpers -------------------------------------------------

    pub fn compound() -> Tag {
        Tag::Compound(Vec::new())
    }

    pub fn as_compound(&self) -> Option<&[(String, Tag)]> {
        match self {
            Self::Compound(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn as_compound_mut(&mut self) -> Option<&mut Vec<(String, Tag)>> {
        match self {
            Self::Compound(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn get(&self, name: &str) -> Option<&Tag> {
        self.as_compound()?.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Tag> {
        self.as_compound_mut()?.iter_mut().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    /// Insert or replace, preserving position on replace.
    pub fn insert(&mut self, name: impl Into<String>, value: Tag) {
        let name = name.into();
        if let Some(slot) = self.get_mut(&name) {
            *slot = value;
        } else if let Some(entries) = self.as_compound_mut() {
            entries.push((name, value));
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<Tag> {
        let entries = self.as_compound_mut()?;
        let idx = entries.iter().position(|(k, _)| k == name)?;
        Some(entries.remove(idx).1)
    }

    // ---- Typed readers ----------------------------------------------------

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Byte(v) => Some(*v as i64),
            Self::Short(v) => Some(*v as i64),
            Self::Int(v) => Some(*v as i64),
            Self::Long(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Tag]> {
        match self {
            Self::List(v) => Some(v),
            _ => None,
        }
    }
}

impl Tag {
    pub fn child_count(&self) -> usize {
        match self {
            Tag::ByteArray(v) => v.len(),
            Tag::List(v) => v.len(),
            Tag::Compound(v) => v.len(),
            Tag::IntArray(v) => v.len(),
            Tag::LongArray(v) => v.len(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.child_count() == 0
    }
}

/// JSON-flavored serialization for CLI `--json` output. Numbers lose their
/// NBT width (documented in `omt nbt --help`); use the tree/SNBT views when
/// the distinction matters.
impl Serialize for Tag {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Tag::Byte(v) => serializer.serialize_i8(*v),
            Tag::Short(v) => serializer.serialize_i16(*v),
            Tag::Int(v) => serializer.serialize_i32(*v),
            Tag::Long(v) => serializer.serialize_i64(*v),
            Tag::Float(v) => serializer.serialize_f64(*v as f64),
            Tag::Double(v) => serializer.serialize_f64(*v),
            Tag::ByteArray(v) => v.serialize(serializer),
            Tag::String(v) => serializer.serialize_str(v),
            Tag::List(v) => {
                use serde::ser::SerializeSeq;
                let mut seq = serializer.serialize_seq(Some(v.len()))?;
                for item in v {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Tag::Compound(entries) => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            Tag::IntArray(v) => {
                use serde::ser::SerializeSeq;
                let mut seq = serializer.serialize_seq(Some(v.len()))?;
                for item in v {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Tag::LongArray(v) => {
                use serde::ser::SerializeSeq;
                let mut seq = serializer.serialize_seq(Some(v.len()))?;
                for item in v {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
        }
    }
}
