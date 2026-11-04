//! Error type for all of `omt-convert`.
//!
//! Malformed input NBT is a caller error, never a panic: converters return
//! [`ConvertError`] and leave their report empty (a failed conversion has no
//! meaningful loss ledger).

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    #[error("not an item stack: expected a compound, found {0}")]
    NotAStack(&'static str),

    #[error("item stack has no string `id`/`Name` field")]
    MissingItemId,

    #[error("Java item stack carries both `tag` and `components` (mixed-era) and policy is Reject")]
    MixedEraStack,

    #[error("bad item stack field `{field}`: {detail}")]
    BadField {
        field: &'static str,
        detail: String,
    },
}

pub type Result<T> = std::result::Result<T, ConvertError>;
