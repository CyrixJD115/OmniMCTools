//! The [`Host`] handle a plugin uses to reach back into OmniMCTools during
//! a callback: worlds, NBT, items, storage, notifications. Every call
//! serializes a [`HostRequest`], hands it to the host through the C ABI,
//! and decodes the [`HostResponse`].

use core::ffi::c_void;

use omt_plugin_abi::ffi;
use omt_plugin_abi::{BlockEntity, Container, HostRequest, HostResponse, Item, NamedTag, Rect, WorldInfo};

/// Reaches the host. Valid only for the duration of the callback it was
/// handed to (do not store it).
pub struct Host {
    raw: *const ffi::Host,
}

/// Sink that appends the host's reply bytes into a `Vec<u8>`.
unsafe extern "C" fn collect(ctx: *mut c_void, ptr: *const u8, len: usize) {
    if ctx.is_null() || ptr.is_null() {
        return;
    }
    // SAFETY: `ctx` is a `&mut Vec<u8>` we passed as `sink_ctx`; `ptr`..len
    // is the host's lent, initialized reply buffer, valid for this call.
    unsafe {
        let v = &mut *(ctx as *mut Vec<u8>);
        v.extend_from_slice(core::slice::from_raw_parts(ptr, len));
    }
}

impl Host {
    pub(crate) fn new(raw: *const ffi::Host) -> Self {
        Self { raw }
    }

    /// Perform one request and decode the reply.
    pub fn request(&self, req: &HostRequest) -> HostResponse {
        if self.raw.is_null() {
            return HostResponse::Err { message: "no host".into() };
        }
        let bytes = omt_plugin_abi::to_bytes(req);
        let mut out: Vec<u8> = Vec::new();
        // SAFETY: `self.raw` is a host-provided `*const ffi::Host` valid for
        // the callback; `call` invokes our `collect` sink with the reply.
        unsafe {
            let host = &*self.raw;
            (host.call)(host.ctx, bytes.as_ptr(), bytes.len(), (&mut out as *mut Vec<u8>).cast(), collect);
        }
        omt_plugin_abi::from_bytes(&out).unwrap_or(HostResponse::Err { message: "malformed host reply".into() })
    }

    fn text(&self, req: &HostRequest) -> Result<Option<String>, String> {
        match self.request(req) {
            HostResponse::Text { value } => Ok(value),
            HostResponse::Err { message } => Err(message),
            other => Err(format!("unexpected host reply: {other:?}")),
        }
    }

    // --- app integration --------------------------------------------------

    /// A status-line / toast message.
    pub fn notify(&self, message: impl Into<String>) {
        self.request(&HostRequest::Notify { message: message.into() });
    }

    /// A log line shown under the tool.
    pub fn log(&self, message: impl Into<String>) {
        self.request(&HostRequest::Log { level: "info".into(), message: message.into() });
    }

    /// Copy text to the clipboard.
    pub fn clipboard(&self, text: impl Into<String>) {
        self.request(&HostRequest::Clipboard { text: text.into() });
    }

    /// Progress line + bar while a callback runs (`fraction` < 0 = spinner).
    pub fn progress(&self, text: impl Into<String>, fraction: f32) {
        self.request(&HostRequest::Progress { text: Some(text.into()), fraction });
    }

    /// Clear the progress line.
    pub fn clear_progress(&self) {
        self.request(&HostRequest::Progress { text: None, fraction: -1.0 });
    }

    /// Whether the user pressed Stop. Long loops should check this.
    pub fn cancelled(&self) -> bool {
        matches!(self.request(&HostRequest::Cancelled), HostResponse::Bool { value: true })
    }

    /// NBT action: hand the (edited) selected tag back to the editor (SNBT),
    /// applied as one undoable step.
    pub fn replace_node(&self, snbt: impl Into<String>) {
        self.request(&HostRequest::ReplaceNode { snbt: snbt.into() });
    }

    /// Per-plugin saved data.
    pub fn storage_get(&self, key: &str) -> serde_json::Value {
        match self.request(&HostRequest::StorageGet { key: key.into() }) {
            HostResponse::Json { value } => value,
            _ => serde_json::Value::Null,
        }
    }

    pub fn storage_set(&self, key: &str, value: serde_json::Value) {
        let value = if value.is_null() { None } else { Some(value) };
        self.request(&HostRequest::StorageSet { key: key.into(), value });
    }

    pub fn world_list(&self) -> Vec<WorldInfo> {
        match self.request(&HostRequest::WorldList) {
            HostResponse::Worlds { value } => value,
            _ => Vec::new(),
        }
    }

    // --- world ------------------------------------------------------------

    /// Containers (chests, barrels, shulkers, hoppers…) in a block box.
    pub fn containers(&self, dim: u8, x0: i32, z0: i32, x1: i32, z1: i32) -> Result<Vec<Container>, String> {
        match self.request(&HostRequest::Containers { rect: Rect { dim, x0, z0, x1, z1 } }) {
            HostResponse::Containers { value } => Ok(value),
            HostResponse::Err { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }

    /// Block entities in a block box; `filter` = kind substring.
    pub fn block_entities(&self, dim: u8, x0: i32, z0: i32, x1: i32, z1: i32, filter: Option<&str>) -> Result<Vec<BlockEntity>, String> {
        match self.request(&HostRequest::BlockEntities { rect: Rect { dim, x0, z0, x1, z1 }, filter: filter.map(str::to_string) }) {
            HostResponse::BlockEntities { value } => Ok(value),
            HostResponse::Err { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }

    /// Block at (x, y, z): `(name, properties)`; `None` when unloaded/air.
    pub fn block(&self, dim: u8, x: i32, y: i32, z: i32) -> Option<(String, Vec<(String, String)>)> {
        match self.request(&HostRequest::Block { dim, x, y, z }) {
            HostResponse::Block { name: Some(name), properties } => Some((name, properties)),
            _ => None,
        }
    }

    /// Players: `(id, tag SNBT)`.
    pub fn players(&self) -> Vec<NamedTag> {
        match self.request(&HostRequest::Players) {
            HostResponse::Tags { value } => value,
            _ => Vec::new(),
        }
    }

    /// level.dat as SNBT.
    pub fn level(&self) -> Option<String> {
        self.text(&HostRequest::Level).ok().flatten()
    }

    /// Present chunk coordinates in a dimension.
    pub fn chunks(&self, dim: u8) -> Vec<(i32, i32)> {
        match self.request(&HostRequest::Chunks { dim }) {
            HostResponse::Chunks { value } => value,
            _ => Vec::new(),
        }
    }

    /// Replace a block entity (whole tag, SNBT). Editable sessions only.
    pub fn set_block_entity(&self, dim: u8, x: i32, y: i32, z: i32, snbt: impl Into<String>) -> Result<(), String> {
        match self.request(&HostRequest::SetBlockEntity { dim, x, y, z, snbt: snbt.into() }) {
            HostResponse::Bool { .. } => Ok(()),
            HostResponse::Err { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }

    /// Set one block (`"stone"` or `"oak_log[axis=y]"`).
    pub fn set_block(&self, dim: u8, x: i32, y: i32, z: i32, block: &str) -> Result<bool, String> {
        match self.request(&HostRequest::SetBlock { dim, x, y, z, block: block.into() }) {
            HostResponse::Bool { value } => Ok(value),
            HostResponse::Err { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }

    /// Fill a box; `replace` restricts to matching blocks. Returns changes.
    pub fn fill(&self, dim: u8, a: [i32; 3], b: [i32; 3], block: &str, replace: Option<&str>) -> Result<u64, String> {
        match self.request(&HostRequest::Fill { dim, a, b, block: block.into(), replace: replace.map(str::to_string) }) {
            HostResponse::Count { value } => Ok(value),
            HostResponse::Err { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }

    // --- items / nbt ------------------------------------------------------

    /// Normalize items from a list / container / inventory tag (SNBT).
    pub fn items_read(&self, snbt: &str) -> Vec<Item> {
        match self.request(&HostRequest::ItemsRead { snbt: snbt.into() }) {
            HostResponse::Items { value } => value,
            _ => Vec::new(),
        }
    }

    /// A new item-stack tag (SNBT). `java` picks the layout.
    pub fn item_make(&self, id: &str, count: i32, slot: i32, java: bool) -> Option<String> {
        self.text(&HostRequest::ItemMake { id: id.into(), count, slot, java }).ok().flatten()
    }

    /// Read an NBT file → SNBT.
    pub fn nbt_read(&self, path: &str) -> Result<String, String> {
        self.text(&HostRequest::NbtRead { path: path.into() }).and_then(|v| v.ok_or_else(|| "empty".into()))
    }

    /// Write SNBT to an NBT file (`format`: java / java_raw / bedrock / bedrock_level).
    pub fn nbt_write(&self, path: &str, snbt: &str, format: &str) -> Result<(), String> {
        match self.request(&HostRequest::NbtWrite { path: path.into(), snbt: snbt.into(), format: format.into() }) {
            HostResponse::Bool { .. } => Ok(()),
            HostResponse::Err { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }

    /// Validate SNBT (returns the error, or Ok).
    pub fn nbt_check(&self, snbt: &str) -> Result<(), String> {
        match self.request(&HostRequest::NbtParse { snbt: snbt.into() }) {
            HostResponse::Bool { .. } => Ok(()),
            HostResponse::Err { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }

    /// Pretty-print (or re-normalize) an SNBT string.
    pub fn snbt(&self, snbt: &str, pretty: bool) -> String {
        self.text(&HostRequest::NbtSnbt { snbt: snbt.into(), pretty }).ok().flatten().unwrap_or_default()
    }
}
