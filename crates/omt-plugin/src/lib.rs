//! `omt-plugin` — SDK for **native OmniMCTools plugins**: compiled Rust
//! dynamic libraries (`cdylib`) that add tool tabs and NBT actions, driving
//! the same extension callers as Lua extensions but with the full power of
//! Rust (any crate, real types, heavy computation).
//!
//! A plugin:
//! 1. builds a [`Plugin`] (its name + entries, each with a factory);
//! 2. implements [`Instance`] for each entry (`view`, optional `on_event`);
//! 3. exports the C ABI with [`export_plugin!`].
//!
//! ```ignore
//! use omt_plugin::prelude::*;
//!
//! struct Hello { clicks: u32 }
//! impl Instance for Hello {
//!     fn view(&mut self, _host: &Host, _ctx: &Ctx) -> Node {
//!         ui::column([ui::heading("Hello"), ui::button("go", format!("{}", self.clicks)).into()])
//!     }
//!     fn on_event(&mut self, ev: &Event, _host: &Host, _ctx: &Ctx) {
//!         if let Event::Click { id } = ev { if id == "go" { self.clicks += 1 } }
//!     }
//! }
//!
//! fn plugin() -> Plugin {
//!     Plugin::new("Hello").tool(ToolDef::new("hello", "Hello").icon("sparkles"), |_ctx| Box::new(Hello { clicks: 0 }))
//! }
//! export_plugin!(plugin);
//! ```
//!
//! The library boundary is C (JSON + the sink pattern in
//! [`omt_plugin_abi::ffi`]); this crate scopes the `unsafe` to
//! [`ffi_register`] / [`ffi_dispatch`], which [`export_plugin!`] calls.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;

use omt_plugin_abi::{Ctx as WireCtx, Dispatch, EntryDecl, EntryKind, Event, Method, Node, Output, PluginManifest, WorldInfo};

pub mod host;
pub mod ui;

pub use host::Host;
pub use omt_plugin_abi::{ABI_VERSION, ButtonStyle, Event as RawEvent, Item, Pick, ffi};

/// NBT type ids, for `ActionDef::types`.
pub mod types {
    pub const BYTE: u8 = 1;
    pub const SHORT: u8 = 2;
    pub const INT: u8 = 3;
    pub const LONG: u8 = 4;
    pub const FLOAT: u8 = 5;
    pub const DOUBLE: u8 = 6;
    pub const BYTE_ARRAY: u8 = 7;
    pub const STRING: u8 = 8;
    pub const LIST: u8 = 9;
    pub const COMPOUND: u8 = 10;
    pub const INT_ARRAY: u8 = 11;
    pub const LONG_ARRAY: u8 = 12;
}

/// The handy imports for writing a plugin.
pub mod prelude {
    pub use crate::ui;
    pub use crate::{Ctx, Host, Instance, Plugin, ToolDef, ActionDef, export_plugin, types};
    pub use omt_plugin_abi::{ButtonStyle, Event, Item, Node, Pick};
}

/// Context a tool instance is created / called with.
pub struct Ctx {
    wire: WireCtx,
}

impl Ctx {
    /// The world the tool was opened on (name / edition / writable).
    pub fn world(&self) -> Option<&WorldInfo> {
        self.wire.world.as_ref()
    }
    /// NBT action: the selected tag as SNBT.
    pub fn node(&self) -> Option<&str> {
        self.wire.node.as_deref()
    }
    /// NBT action: the selected tag's name.
    pub fn node_name(&self) -> Option<&str> {
        self.wire.node_name.as_deref()
    }
    /// `true` when the open world can be written to.
    pub fn writable(&self) -> bool {
        self.wire.world.as_ref().is_some_and(|w| w.writable)
    }
}

/// One live tool / action. The instance persists across events for its tab.
pub trait Instance: 'static + Send {
    /// Build the UI (called after `init` and after every event).
    fn view(&mut self, host: &Host, ctx: &Ctx) -> Node;
    /// Handle a UI event (default: ignore).
    fn on_event(&mut self, _ev: &Event, _host: &Host, _ctx: &Ctx) {}
    /// Override the tab title (default: the entry name).
    fn title(&self) -> Option<String> {
        None
    }
}

type Factory = Box<dyn Fn(&Ctx) -> Box<dyn Instance> + Send>;

struct Entry {
    decl: EntryDecl,
    factory: Factory,
}

/// A tool-entry declaration (builder).
pub struct ToolDef(EntryDecl);

impl ToolDef {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self(EntryDecl { id: id.into(), name: name.into(), description: String::new(), icon: String::new(), kind: EntryKind::Tool, map: false, names: Vec::new(), types: Vec::new() })
    }
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.0.icon = icon.into();
        self
    }
    pub fn describe(mut self, d: impl Into<String>) -> Self {
        self.0.description = d.into();
        self
    }
    /// Also offer the tool in the Map Viewer panel (it receives map clicks).
    pub fn map(mut self, on: bool) -> Self {
        self.0.map = on;
        self
    }
}

/// An NBT-action declaration (builder).
pub struct ActionDef(EntryDecl);

impl ActionDef {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self(EntryDecl { id: id.into(), name: name.into(), description: String::new(), icon: String::new(), kind: EntryKind::NbtAction, map: false, names: Vec::new(), types: Vec::new() })
    }
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.0.icon = icon.into();
        self
    }
    pub fn describe(mut self, d: impl Into<String>) -> Self {
        self.0.description = d.into();
        self
    }
    /// Tag names it applies to (empty = any).
    pub fn names(mut self, names: &[&str]) -> Self {
        self.0.names = names.iter().map(|s| s.to_string()).collect();
        self
    }
    /// Tag type ids it applies to (see [`types`]; empty = any).
    pub fn types(mut self, types: &[u8]) -> Self {
        self.0.types = types.to_vec();
        self
    }
}

/// A plugin: its identity plus its tool / action entries.
pub struct Plugin {
    name: String,
    version: String,
    author: String,
    description: String,
    entries: Vec<Entry>,
}

impl Plugin {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), version: String::new(), author: String::new(), description: String::new(), entries: Vec::new() }
    }
    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.version = v.into();
        self
    }
    pub fn author(mut self, a: impl Into<String>) -> Self {
        self.author = a.into();
        self
    }
    pub fn describe(mut self, d: impl Into<String>) -> Self {
        self.description = d.into();
        self
    }
    /// Add a tool, with a factory that builds a fresh instance per tab.
    pub fn tool(mut self, def: ToolDef, factory: impl Fn(&Ctx) -> Box<dyn Instance> + Send + 'static) -> Self {
        self.entries.push(Entry { decl: def.0, factory: Box::new(factory) });
        self
    }
    /// Add an NBT action (opened on the selected tag).
    pub fn action(mut self, def: ActionDef, factory: impl Fn(&Ctx) -> Box<dyn Instance> + Send + 'static) -> Self {
        self.entries.push(Entry { decl: def.0, factory: Box::new(factory) });
        self
    }

    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            api_version: ABI_VERSION,
            name: self.name.clone(),
            version: self.version.clone(),
            author: self.author.clone(),
            description: self.description.clone(),
            entries: self.entries.iter().map(|e| e.decl.clone()).collect(),
        }
    }
}

/// Owns the plugin and its live instances (one per open tab). The
/// [`export_plugin!`] macro keeps one of these in a static.
pub struct Runtime {
    plugin: Plugin,
    instances: HashMap<u64, (String, Box<dyn Instance>)>,
}

impl Runtime {
    pub fn new(plugin: Plugin) -> Self {
        Self { plugin, instances: HashMap::new() }
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.plugin.entries.iter().find(|e| e.decl.id == id)
    }

    fn dispatch(&mut self, d: Dispatch, host: &Host) -> Output {
        let ctx = Ctx { wire: d.ctx };
        match d.method {
            Method::Shutdown => {
                self.instances.remove(&d.instance);
                return Output::default();
            }
            Method::Init => {
                let Some(entry) = self.entry(&d.entry) else {
                    return Output { error: Some(format!("no entry {:?}", d.entry)), ..Default::default() };
                };
                let inst = (entry.factory)(&ctx);
                self.instances.insert(d.instance, (d.entry.clone(), inst));
            }
            Method::Event { ref event } => {
                if let Some((_, inst)) = self.instances.get_mut(&d.instance) {
                    inst.on_event(event, host, &ctx);
                }
            }
            Method::Refresh => {}
        }
        // Default title from the entry name (looked up before the mutable borrow).
        let default_title = self.entry(&d.entry).map(|e| e.decl.name.clone()).unwrap_or_default();
        let Some((_, inst)) = self.instances.get_mut(&d.instance) else {
            return Output { error: Some("tool instance is gone".into()), ..Default::default() };
        };
        let view = inst.view(host, &ctx);
        let title = inst.title().unwrap_or(default_title);
        Output { title, view: Some(view), error: None, effects: Default::default() }
    }
}

/// Implementation of `omt_plugin_register`: emit the plugin manifest.
///
/// # Safety
/// `sink`/`sink_ctx` must be the host's valid sink for this call.
pub unsafe fn ffi_register(runtime: &std::sync::Mutex<Runtime>, sink_ctx: *mut core::ffi::c_void, sink: ffi::Sink) {
    let bytes = runtime.lock().map(|rt| omt_plugin_abi::to_bytes(&rt.plugin.manifest())).unwrap_or_default();
    // SAFETY: caller contract — `sink` is the host's sink valid for this call.
    unsafe { sink(sink_ctx, bytes.as_ptr(), bytes.len()) };
}

/// Implementation of `omt_plugin_dispatch`: run one call and emit its
/// [`Output`]. User code is wrapped so a panic becomes an error Output
/// instead of unwinding across the C boundary.
///
/// # Safety
/// `req_ptr`..`req_len` must be a valid byte range; `host` a valid
/// `*const ffi::Host`; `sink`/`sink_ctx` the host's sink for this call.
pub unsafe fn ffi_dispatch(runtime: &std::sync::Mutex<Runtime>, req_ptr: *const u8, req_len: usize, host: *const ffi::Host, sink_ctx: *mut core::ffi::c_void, sink: ffi::Sink) {
    // SAFETY: caller contract — the request bytes are valid for this call.
    let req = unsafe { core::slice::from_raw_parts(req_ptr, req_len) };
    let out = match omt_plugin_abi::from_bytes::<Dispatch>(req) {
        Ok(d) => {
            let h = Host::new(host);
            std::panic::catch_unwind(AssertUnwindSafe(|| match runtime.lock() {
                Ok(mut rt) => rt.dispatch(d, &h),
                Err(_) => Output { error: Some("plugin runtime poisoned".into()), ..Default::default() },
            }))
            .unwrap_or_else(|_| Output { error: Some("plugin panicked".into()), ..Default::default() })
        }
        Err(e) => Output { error: Some(format!("bad dispatch: {e}")), ..Default::default() },
    };
    let bytes = omt_plugin_abi::to_bytes(&out);
    // SAFETY: caller contract — `sink` is the host's sink valid for this call.
    unsafe { sink(sink_ctx, bytes.as_ptr(), bytes.len()) };
}

/// Export the C ABI for a plugin. `$build` is a `fn() -> Plugin`.
#[macro_export]
macro_rules! export_plugin {
    ($build:path) => {
        static __OMT_RUNTIME: ::std::sync::OnceLock<::std::sync::Mutex<$crate::Runtime>> = ::std::sync::OnceLock::new();
        fn __omt_runtime() -> &'static ::std::sync::Mutex<$crate::Runtime> {
            __OMT_RUNTIME.get_or_init(|| ::std::sync::Mutex::new($crate::Runtime::new($build())))
        }
        /// C ABI: protocol version.
        #[unsafe(no_mangle)]
        pub extern "C" fn omt_plugin_abi_version() -> u32 {
            $crate::ABI_VERSION
        }
        /// C ABI: emit the plugin manifest.
        ///
        /// # Safety
        /// `sink`/`sink_ctx` must be the host's valid sink.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn omt_plugin_register(sink_ctx: *mut ::core::ffi::c_void, sink: $crate::ffi::Sink) {
            unsafe { $crate::ffi_register(__omt_runtime(), sink_ctx, sink) }
        }
        /// C ABI: run one dispatch.
        ///
        /// # Safety
        /// Pointers must be valid for the call (host contract).
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn omt_plugin_dispatch(req_ptr: *const u8, req_len: usize, host: *const $crate::ffi::Host, sink_ctx: *mut ::core::ffi::c_void, sink: $crate::ffi::Sink) {
            unsafe { $crate::ffi_dispatch(__omt_runtime(), req_ptr, req_len, host, sink_ctx, sink) }
        }
    };
}
