//! `omt-nativehost` — loads OmniMCTools **native plugins** (compiled Rust
//! dynamic libraries built with the `omt-plugin` SDK) and runs them through
//! the same [`ToolBackend`] the GUI uses for Lua tools.
//!
//! A native-plugin "extension" is a folder in the extensions directory that
//! contains one dynamic library (`*.so` / `*.dll` / `*.dylib`) exporting the
//! C ABI from [`omt_plugin_abi::ffi`]. [`discover_native`] loads each and
//! lists its tools / actions; [`NativeHost`] runs one open tool on its own
//! thread, servicing the plugin's host calls through
//! [`omt_extensions::ops`].
//!
//! All `unsafe` is confined to loading the library and the two C callbacks
//! ([`host_call`], the collecting sink).

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use libloading::{Library, Symbol};
use omt_extensions::ops::{self, HostCtx};
use omt_extensions::runtime::{Effects, Shared};
use omt_extensions::tool::{ToolBackend, ToolDecl, ToolKind, ToolOutput, ToolSpec};
use omt_extensions::ui::ToolEvent;
use omt_plugin_abi as abi;
use omt_plugin_abi::ffi;

mod convert;

/// Unique tool-instance ids across every native host (one loaded library
/// serves all tabs, so its instance map needs globally distinct keys).
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// The one dynamic library inside a plugin folder.
pub fn native_lib(dir: &Path) -> Option<PathBuf> {
    let mut libs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) && p.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x, "so" | "dll" | "dylib")))
        .collect();
    libs.sort();
    libs.into_iter().next()
}

/// A loaded plugin library and its resolved entry points.
struct Loaded {
    _lib: Library,
    register: ffi::FnRegister,
    dispatch: ffi::FnDispatch,
}

impl Loaded {
    /// Open the library and check its ABI version.
    fn open(path: &Path) -> Result<Loaded, String> {
        // SAFETY: loading any native library runs its initializers; the user
        // installed this plugin knowingly (same trust model as Lua).
        let lib = unsafe { Library::new(path) }.map_err(|e| format!("cannot load {}: {e}", path.display()))?;
        // SAFETY: the symbols are looked up by the agreed names; we only call
        // them with the documented ABI.
        unsafe {
            let version: Symbol<ffi::FnAbiVersion> = lib.get(ffi::SYM_ABI_VERSION).map_err(|_| "not an OMT plugin (no omt_plugin_abi_version)".to_string())?;
            let v = version();
            if v != abi::ABI_VERSION {
                return Err(format!("plugin ABI v{v}, this OMT speaks v{} — rebuild the plugin", abi::ABI_VERSION));
            }
            let register: Symbol<ffi::FnRegister> = lib.get(ffi::SYM_REGISTER).map_err(|_| "plugin missing omt_plugin_register".to_string())?;
            let dispatch: Symbol<ffi::FnDispatch> = lib.get(ffi::SYM_DISPATCH).map_err(|_| "plugin missing omt_plugin_dispatch".to_string())?;
            let (register, dispatch) = (*register, *dispatch);
            Ok(Loaded { _lib: lib, register, dispatch })
        }
    }

    /// Ask the plugin for its manifest.
    fn manifest(&self) -> Result<abi::PluginManifest, String> {
        let mut out: Vec<u8> = Vec::new();
        // SAFETY: `collect` only appends the lent bytes into `out`.
        unsafe { (self.register)((&mut out as *mut Vec<u8>).cast(), collect) };
        abi::from_bytes(&out)
    }
}

/// Sink: append the lent bytes into a `Vec<u8>` behind `ctx`.
unsafe extern "C" fn collect(ctx: *mut c_void, ptr: *const u8, len: usize) {
    if ctx.is_null() || ptr.is_null() {
        return;
    }
    // SAFETY: `ctx` is a `&mut Vec<u8>` we provided; `ptr`..len is the lent,
    // initialized buffer valid for this call.
    unsafe {
        let v = &mut *(ctx as *mut Vec<u8>);
        v.extend_from_slice(core::slice::from_raw_parts(ptr, len));
    }
}

/// The host callback the plugin calls back through: run one
/// [`abi::HostRequest`] against the [`HostCtx`] behind `ctx` and reply.
unsafe extern "C" fn host_call(ctx: *mut c_void, req_ptr: *const u8, req_len: usize, sink_ctx: *mut c_void, sink: ffi::Sink) {
    // SAFETY: `ctx` is the `&mut HostCtx` we passed as the host ctx; it is
    // borrowed uniquely for the duration of this synchronous call. `req_ptr`
    // ..req_len is the plugin's lent request bytes.
    let resp = unsafe {
        let hc = &mut *(ctx as *mut HostCtx);
        let req = core::slice::from_raw_parts(req_ptr, req_len);
        match abi::from_bytes::<abi::HostRequest>(req) {
            Ok(r) => ops::execute(&r, hc),
            Err(e) => abi::HostResponse::Err { message: format!("bad host request: {e}") },
        }
    };
    let bytes = abi::to_bytes(&resp);
    // SAFETY: `sink` is the plugin's sink valid for this call.
    unsafe { sink(sink_ctx, bytes.as_ptr(), bytes.len()) };
}

/// Load every native plugin in `ids` (folder names under `root`) and list
/// its tools and actions. Broken plugins are reported, never fatal.
pub fn discover_native(root: &Path, ids: &[String]) -> (Vec<ToolDecl>, Vec<String>) {
    let (mut decls, mut errors) = (Vec::new(), Vec::new());
    for ext in ids {
        let dir = root.join(ext);
        let Some(path) = native_lib(&dir) else { continue };
        match Loaded::open(&path).and_then(|l| l.manifest()) {
            Ok(m) if m.api_version == abi::ABI_VERSION => {
                for e in m.entries {
                    decls.push(ToolDecl {
                        ext: ext.clone(),
                        id: e.id,
                        name: e.name,
                        description: e.description,
                        icon: e.icon,
                        kind: if matches!(e.kind, abi::EntryKind::Tool) { ToolKind::Tool } else { ToolKind::NbtAction },
                        map: e.map,
                        names: e.names,
                        types: e.types,
                    });
                }
            }
            Ok(m) => errors.push(format!("{ext}: plugin ABI v{} ≠ v{}", m.api_version, abi::ABI_VERSION)),
            Err(e) => errors.push(format!("{ext}: {e}")),
        }
    }
    (decls, errors)
}

enum Req {
    Event(ToolEvent, Sender<ToolOutput>),
    Refresh(Sender<ToolOutput>),
}

/// A running native tool (its worker thread owns the loaded library).
pub struct NativeHost {
    tx: Sender<Req>,
    shared: Arc<Shared>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl NativeHost {
    /// Start the tool; the receiver yields the first view (after init).
    pub fn start(spec: ToolSpec) -> (Self, Receiver<ToolOutput>) {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = channel::<Req>();
        let (first_tx, first_rx) = channel();
        let sh = Arc::clone(&shared);
        let join = std::thread::Builder::new().name(format!("plugin-{}-{}", spec.ext, spec.tool)).spawn(move || worker(spec, sh, rx, first_tx)).ok();
        (Self { tx, shared, join }, first_rx)
    }
}

impl ToolBackend for NativeHost {
    fn send(&self, ev: ToolEvent) -> Receiver<ToolOutput> {
        let (tx, rx) = channel();
        self.shared.cancel.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Req::Event(ev, tx));
        rx
    }
    fn refresh(&self) -> Receiver<ToolOutput> {
        let (tx, rx) = channel();
        let _ = self.tx.send(Req::Refresh(tx));
        rx
    }
    fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
    fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
    fn shutdown(self: Box<Self>) {
        self.shared.cancel.store(true, Ordering::Relaxed);
        let Self { tx, join, .. } = *self;
        drop(tx);
        if let Some(j) = join {
            let _ = j.join();
        }
    }
}

/// The tool's own thread: load the library, then dispatch Init / events /
/// Refresh, servicing host calls, until the channel closes (Shutdown).
fn worker(spec: ToolSpec, shared: Arc<Shared>, rx: Receiver<Req>, first: Sender<ToolOutput>) {
    let dir = spec.root.join(&spec.ext);
    let loaded = match native_lib(&dir).ok_or_else(|| "no plugin library in the folder".to_string()).and_then(|p| Loaded::open(&p)) {
        Ok(l) => l,
        Err(e) => {
            let _ = first.send(ToolOutput { title: spec.tool.clone(), error: Some(e), ..Default::default() });
            return;
        }
    };
    let instance = NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed);
    let node_snbt = spec.node.as_ref().map(|(_, t)| omt_nbt::snbt::to_snbt(t));
    let node_name = spec.node.as_ref().map(|(n, _)| n.clone());
    let wire_ctx = abi::Ctx { world: spec.world.as_ref().map(ops::world_info), node: node_snbt, node_name };

    let run = |method: abi::Method| -> ToolOutput {
        shared.busy.store(true, Ordering::Relaxed);
        let mut hc = HostCtx {
            world: spec.world.clone(),
            opener: spec.opener.clone(),
            library: spec.library.clone(),
            ext_dir: dir.clone(),
            shared: Arc::clone(&shared),
            effects: Effects::default(),
        };
        let d = abi::Dispatch { entry: spec.tool.clone(), instance, method, ctx: wire_ctx.clone() };
        let req = abi::to_bytes(&d);
        let host = ffi::Host { ctx: (&mut hc as *mut HostCtx).cast(), call: host_call };
        let mut out: Vec<u8> = Vec::new();
        // SAFETY: `dispatch` is the plugin's C entry; pointers are valid for
        // the call, `host`/`collect` service callbacks synchronously.
        unsafe { (loaded.dispatch)(req.as_ptr(), req.len(), &host as *const ffi::Host, (&mut out as *mut Vec<u8>).cast(), collect) };
        shared.set_progress(None);
        shared.busy.store(false, Ordering::Relaxed);
        let output: abi::Output = abi::from_bytes(&out).unwrap_or(abi::Output { error: Some("plugin returned no output".into()), ..Default::default() });
        let mut eff = hc.effects;
        // Anything the plugin also set in Output.effects (rare) merges in.
        eff.notes.extend(output.effects.notes);
        eff.logs.extend(output.effects.logs);
        eff.dirty |= output.effects.dirty;
        if output.effects.clipboard.is_some() {
            eff.clipboard = output.effects.clipboard;
        }
        if let Some(s) = output.effects.replace
            && let Ok(t) = omt_nbt::snbt::from_snbt(&s)
        {
            eff.replace = Some(t);
        }
        ToolOutput { title: output.title, view: output.view.map(convert::node), error: output.error, effects: eff }
    };

    let _ = first.send(run(abi::Method::Init));
    while let Ok(req) = rx.recv() {
        match req {
            Req::Event(ev, reply) => {
                let _ = reply.send(run(abi::Method::Event { event: convert::event(ev) }));
            }
            Req::Refresh(reply) => {
                let _ = reply.send(run(abi::Method::Refresh));
            }
        }
    }
    // Channel closed = tab closing: let the plugin drop its instance.
    let _ = run(abi::Method::Shutdown);
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
