//! Wire protocol between OmniMCTools and **native plugins** (compiled
//! dynamic libraries written in Rust against the `omt-plugin` SDK).
//!
//! Everything crossing the library boundary is JSON built from the types
//! here plus a tiny C ABI ([`ffi`]): the plugin registers its tools, the
//! host dispatches UI events to them, and the plugin calls back for world /
//! NBT / item operations. NBT values travel as **SNBT strings**, so the
//! protocol is language- and layout-neutral and never shares Rust types
//! across the boundary (which has no stable ABI).
//!
//! This crate is `serde` only — no `unsafe`, no heavy deps — so plugin
//! authors compile almost nothing to depend on it. The `unsafe` FFI glue
//! lives in `omt-plugin` (the SDK) and `omt-nativehost` (the loader).

use serde::{Deserialize, Serialize};

/// Protocol version. The host refuses a plugin whose
/// [`PluginManifest::api_version`] differs.
pub const ABI_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// What a plugin declares at load (`omt_plugin_register`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub api_version: u32,
    /// Human name of the whole plugin.
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    pub entries: Vec<EntryDecl>,
}

/// One tool or NBT action the plugin offers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryDecl {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    pub kind: EntryKind,
    /// Tools only: also usable from the Map Viewer panel (gets map clicks).
    #[serde(default)]
    pub map: bool,
    /// NBT actions: tag names they apply to (empty = any).
    #[serde(default)]
    pub names: Vec<String>,
    /// NBT actions: tag type ids they apply to (empty = any).
    #[serde(default)]
    pub types: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Tool,
    NbtAction,
}

// ---------------------------------------------------------------------------
// Dispatch (host → plugin)
// ---------------------------------------------------------------------------

/// A call from the host into one tool instance (`omt_plugin_dispatch`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dispatch {
    /// Which [`EntryDecl::id`].
    pub entry: String,
    /// Stable per open tab; the plugin keeps state keyed by this.
    pub instance: u64,
    pub method: Method,
    pub ctx: Ctx,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Method {
    /// First call for an instance: construct it, then render.
    Init,
    /// A UI event, then render.
    Event { event: Event },
    /// Re-render only (the app changed something).
    Refresh,
    /// Drop the instance (tab closed).
    Shutdown,
}

/// What a tool instance was opened with.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ctx {
    pub world: Option<WorldInfo>,
    /// NBT action target as SNBT.
    pub node: Option<String>,
    /// NBT action target name.
    pub node_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldInfo {
    pub name: String,
    /// `"JAVA"` / `"BEDROCK"`.
    pub edition: String,
    pub path: String,
    pub writable: bool,
}

/// A UI input, mirroring the Lua `ToolEvent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Click { id: String },
    Input { id: String, value: String },
    Submit { id: String, value: String },
    Toggle { id: String, value: bool },
    Slider { id: String, value: f64 },
    Select { id: String, value: String },
    Row { id: String, row: usize },
    Slot { id: String, slot: i32 },
    Picked { id: String, path: String },
    MapClick { x: i32, y: Option<i32>, z: i32, dim: u8 },
}

// ---------------------------------------------------------------------------
// Output (plugin → host)
// ---------------------------------------------------------------------------

/// What a dispatch produces.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Output {
    #[serde(default)]
    pub title: String,
    pub view: Option<Node>,
    pub error: Option<String>,
    #[serde(default)]
    pub effects: Effects,
}

/// Side effects the plugin asks the app for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Effects {
    #[serde(default)]
    pub notes: Vec<String>,
    pub clipboard: Option<String>,
    #[serde(default)]
    pub dirty: bool,
    /// NBT action: replace the selected tag with this SNBT.
    pub replace: Option<String>,
    #[serde(default)]
    pub logs: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// UI tree
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Standard,
    Primary,
    Danger,
    Ghost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    #[default]
    None,
    File,
    Folder,
    SaveFile,
    World,
}

/// One item stack shown in a chest grid.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Item {
    pub slot: i32,
    pub id: String,
    pub count: i32,
    #[serde(default)]
    pub damage: i32,
    pub label: Option<String>,
    #[serde(default)]
    pub lore: Vec<String>,
    /// `(id, level)`.
    #[serde(default)]
    pub enchantments: Vec<(String, i32)>,
    #[serde(default)]
    pub glint: bool,
    #[serde(default)]
    pub contents: Vec<Item>,
}

/// A UI node (mirror of the app's `UiNode`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum Node {
    Column { children: Vec<Node>, spacing: f32 },
    Row { children: Vec<Node>, spacing: f32 },
    Card { title: Option<String>, children: Vec<Node> },
    Scroll { child: Box<Node>, height: Option<f32> },
    Text { text: String, size: f32, color: Option<(u8, u8, u8)>, bold: bool, mono: bool },
    Heading { text: String },
    Separator,
    Space { size: f32 },
    Button { id: String, label: String, style: ButtonStyle, pick: Pick, enabled: bool, icon: Option<String> },
    Input { id: String, value: String, placeholder: String, width: Option<f32> },
    Toggle { id: String, label: String, value: bool },
    Slider { id: String, min: f64, max: f64, step: f64, value: f64, label: String },
    Select { id: String, options: Vec<String>, value: Option<String>, placeholder: String },
    Table { id: String, columns: Vec<String>, rows: Vec<Vec<String>>, selected: Option<usize>, height: Option<f32> },
    Chest { id: String, title: String, rows: usize, cols: usize, items: Vec<Item>, selected: Option<i32> },
    Block { name: String, size: f32 },
    Progress { value: f32, label: String },
    Code { text: String },
}

// ---------------------------------------------------------------------------
// Host calls (plugin → host, during a dispatch)
// ---------------------------------------------------------------------------

/// A box of blocks (inclusive), block coordinates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    pub dim: u8,
    pub x0: i32,
    pub z0: i32,
    pub x1: i32,
    pub z1: i32,
}

/// An operation the plugin asks the host to perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostRequest {
    /// Whether the user pressed Stop.
    Cancelled,
    /// NBT action: hand the (edited) selected tag back (SNBT).
    ReplaceNode { snbt: String },
    Notify { message: String },
    Clipboard { text: String },
    Progress { text: Option<String>, fraction: f32 },
    Log { level: String, message: String },
    StorageGet { key: String },
    StorageSet { key: String, value: Option<serde_json::Value> },
    WorldList,
    /// Containers (chests, barrels, shulkers…) with their items + layout.
    Containers { rect: Rect },
    /// Block entities in a box; `filter` = kind substring.
    BlockEntities { rect: Rect, filter: Option<String> },
    Block { dim: u8, x: i32, y: i32, z: i32 },
    Players,
    Level,
    Chunks { dim: u8 },
    /// Replace a block entity (complete tag, SNBT); editable sessions only.
    SetBlockEntity { dim: u8, x: i32, y: i32, z: i32, snbt: String },
    SetBlock { dim: u8, x: i32, y: i32, z: i32, block: String },
    Fill { dim: u8, a: [i32; 3], b: [i32; 3], block: String, replace: Option<String> },
    /// Normalize items from a list/container tag (SNBT) → [`Item`]s.
    ItemsRead { snbt: String },
    /// A new item stack tag (SNBT). `java` picks the layout (default Java).
    ItemMake { id: String, count: i32, slot: i32, java: bool },
    NbtRead { path: String },
    NbtWrite { path: String, snbt: String, format: String },
    NbtParse { snbt: String },
    NbtSnbt { snbt: String, pretty: bool },
}

/// The host's reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ok", rename_all = "snake_case")]
pub enum HostResponse {
    Err { message: String },
    Bool { value: bool },
    Count { value: u64 },
    Text { value: Option<String> },
    Json { value: serde_json::Value },
    Containers { value: Vec<Container> },
    BlockEntities { value: Vec<BlockEntity> },
    Block { name: Option<String>, properties: Vec<(String, String)> },
    Tags { value: Vec<NamedTag> },
    Items { value: Vec<Item> },
    Worlds { value: Vec<WorldInfo> },
    Chunks { value: Vec<(i32, i32)> },
    Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `chest`, `barrel`, `shulker_box`…
    pub kind: String,
    pub rows: usize,
    pub cols: usize,
    pub items: Vec<Item>,
    /// The whole block-entity tag as SNBT (for writes).
    pub snbt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockEntity {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: String,
    pub kind: String,
    pub snbt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedTag {
    pub id: String,
    pub snbt: String,
}

pub mod ffi;

/// Encode a value to a JSON byte vector (never fails for these types).
pub fn to_bytes<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

/// Decode a JSON byte slice.
pub fn from_bytes<'a, T: Deserialize<'a>>(b: &'a [u8]) -> Result<T, String> {
    serde_json::from_slice(b).map_err(|e| e.to_string())
}
