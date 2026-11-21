//! `omt-extensions` — the OmniMCTools extension platform, built on the Lua
//! core in `omt-script` (packages, tag API, NBT-editor style runs).
//!
//! Extensions can:
//! * add **tool tabs** (`EXTENSION_TYPE.TOOL`): a declarative UI ([`ui`])
//!   rebuilt from `view(self, ctx)` and driven by `on_event(self, ev, ctx)`,
//!   running on its own worker thread ([`tool`]) so long jobs never block
//!   the app (progress + cancel built in);
//! * add **NBT actions** (`EXTENSION_TYPE.NBT_ACTION`): tools opened on the
//!   NBT editor's selected tag that can show it (e.g. as a chest) and edit it;
//! * use the `omt` library ([`api`]): worlds (chunks, blocks, block
//!   entities, containers, players, level.dat; writes on editable sessions),
//!   NBT files / SNBT, normalized items for both editions, JSON, persistent
//!   storage, logging, notifications, clipboard.

pub mod api;
pub mod items;
mod legacy_items;
pub mod runtime;
pub mod builtin;
pub mod ops;
pub mod tool;
pub mod world_api;
pub mod ui;

pub use builtin::{builtins, Builtin};
pub use ui::{ButtonStyle, ItemSlot, Pick, ToolEvent, UiNode};

/// The built-in example extension (`assets/chest-explorer/`): Chest
/// Explorer + Player Inventories tools and the "Open as chest" NBT action.
pub fn example_files() -> Vec<(String, Vec<u8>)> {
    vec![
        ("main.lua".into(), include_bytes!("../assets/chest-explorer/main.lua").to_vec()),
        ("manifest.json".into(), include_bytes!("../assets/chest-explorer/manifest.json").to_vec()),
    ]
}

/// Files of a new extension named `name` (the "New extension" template: a
/// working tool showing the world, storage, inputs and map clicks).
pub fn template_files(name: &str) -> Vec<(String, Vec<u8>)> {
    let main = include_str!("../assets/template/main.lua").replace("__NAME__", name);
    let manifest = serde_json::json!({ "name": name, "version": "0.1.0", "author": "", "description": "A new extension — edit main.lua, then Reload." });
    vec![("main.lua".into(), main.into_bytes()), ("manifest.json".into(), serde_json::to_vec_pretty(&manifest).unwrap_or_default())]
}
