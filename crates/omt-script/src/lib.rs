//! `omt-script` — Lua extension host for OmniMCTools (clean-room M7 ext half).
//!
//! Extensions are zip packages (`manifest.json` + `main.lua`, optional extra
//! modules/assets) installed into an extensions directory. `main.lua` returns
//! a table of *sub-extensions*; those with `type = EXTENSION_TYPE.NBT_EDITOR_STYLE`
//! decorate the NBT editor tree: the host calls each `main(self, root, context)`
//! once, then walks the tree depth-first (compounds and lists, children via
//! `current:child(i)` 0-based and `current.childCount`) calling
//! `recursion(self, root, current, context)` at every node.
//!
//! The runtime protocol (registration order, `main`-then-walk, tree walk,
//! globals `BASE_PATH`/`EXTENSION_TYPE`/`TYPE`/`FILE_TYPE`/`EDITION`,
//! `context.type`) mirrors the behavioral facts recovered from UMT's embedded
//! bootstrap (see `../RE/` evidence); everything styling-related that UMT never
//! exposed (`setExtraText`/`setDescription`/`setColor`) is an OMT-defined
//! addition, as are the exact semantics of ambiguous methods (documented on
//! each method). See `EXTENSIONS.md` for the authoring guide.
//!
//! Extensions run with full Lua privileges and full trust — install only from
//! sources you trust (same model as UT-era script extensions). The only
//! hardening applied: Lua's C-module loader (`package.cpath`) is disabled, and
//! `print` is routed into the run log instead of stdout.

mod host;
pub mod lua;
mod manager;
mod tag;

pub use manager::{example_files, ExtensionRecord, Manifest, Manager};

use std::collections::HashMap;

use omt_nbt::Tag;

/// Bit flags for [`Context::file_type`] (OMT-defined values; UMT's exact
/// bit assignment is not recovered — extensions should compare against the
/// `FILE_TYPE` table, never hard-code numbers).
pub mod file_type {
    pub const CHUNK: u32 = 1;
    pub const MAP: u32 = 2;
    pub const PLAYER: u32 = 4;
    pub const LEVEL: u32 = 8;
    pub const GENERIC: u32 = 16;
}

/// Edition of the document an extension run is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Edition {
    Java,
    Bedrock,
    #[default]
    Unknown,
}

impl Edition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Java => "JAVA",
            Self::Bedrock => "BEDROCK",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Per-run document context (exposed to Lua as the `context` table:
/// `context.type` = file-type bits, `context.edition` = `"JAVA"|"BEDROCK"|"UNKNOWN"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub file_type: u32,
    pub edition: Edition,
}

/// One extension-decorated row. All fields optional; extensions set them via
/// the OMT-defined `setExtraText` / `setDescription` / `setColor` methods.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowStyle {
    /// Short annotation shown next to the value (e.g. `day 3, 09:30`).
    pub extra_text: Option<String>,
    /// Longer explanation shown in the selection panel.
    pub description: Option<String>,
    /// Row tint as RGB (0–255 per channel).
    pub color: Option<(u8, u8, u8)>,
}

impl RowStyle {
    fn is_empty(&self) -> bool {
        self.extra_text.is_none() && self.description.is_none() && self.color.is_none()
    }
}

/// Result of one extension pass over a document.
#[derive(Debug, Default)]
pub struct NbtStyleRun {
    /// Styles keyed by [`omt_nbt::TagPath`] display string (same form the
    /// editor's rows use). Nodes created *during* the run (`addChild`…) have
    /// no original path and are not keyed.
    pub styles: HashMap<String, RowStyle>,
    /// `Some` when extensions edited the tree: the full replacement root.
    /// The caller applies it through its own (undoable) edit engine.
    pub edited_root: Option<Tag>,
    /// Non-fatal per-extension errors (a broken extension never aborts the
    /// run or the editor; it is reported and skipped).
    pub errors: Vec<String>,
    /// `logger.*` calls and overridden `print` output, `(level, message)`.
    pub logs: Vec<(String, String)>,
    /// Nodes visited by the walk (bounded by the host budget).
    pub visited: usize,
}
