//! Error type for all of `omt-nbt`.

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("unexpected end of input at offset {offset}: needed {needed} bytes, {available} available")]
    UnexpectedEof { offset: usize, needed: usize, available: usize },

    #[error("invalid tag id 0x{0:02x}")]
    InvalidTagId(u8),

    #[error("list element type mismatch: declared 0x{declared:02x}, element {index} is 0x{actual:02x}")]
    ListTypeMismatch { declared: u8, actual: u8, index: usize },

    #[error("absurd length {len} for {what}: only {available} bytes remain")]
    AbsurdLength { what: &'static str, len: u64, available: usize },

    #[error("nesting deeper than {0} levels")]
    DepthLimitExceeded(u16),

    #[error("string too long for NBT ({0} encoded bytes, max 65535)")]
    StringTooLong(usize),

    #[error("container/compression error: {0}")]
    Container(String),

    #[error("{0} bytes of trailing data after root tag")]
    TrailingData(usize),

    #[error("invalid Bedrock level.dat: {0}")]
    BadLevelDatHeader(String),
}

pub type Result<T> = std::result::Result<T, Error>;
