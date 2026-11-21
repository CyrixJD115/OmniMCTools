//! What a running tool shares with the app: progress, cancellation,
//! notifications, the world it was opened on and how to open others.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use omt_world::{Edition, WorldStore};

/// Writes into a world, supplied by the app (it knows each edition's
/// on-disk format; stores' generic `write_chunk` is not used for edits).
pub trait WorldWriter: Send + Sync {
    /// Replace chunk (`cx`, `cz`)'s block entities with `list` (complete set).
    fn set_block_entities(&self, dim: u8, cx: i32, cz: i32, list: &[omt_nbt::Tag]) -> Result<(), String>;
    /// Set every block in the inclusive box `a`..`b` to `block`
    /// (`name` or `name[key=value,…]`); with `replace`, only blocks matching
    /// it. Returns the number of blocks changed.
    fn fill(&self, dim: u8, a: [i32; 3], b: [i32; 3], block: &str, replace: Option<&str>) -> Result<u64, String>;
}

/// An opened world an extension can read (and write, when `writable`).
#[derive(Clone)]
pub struct WorldHandle {
    pub store: Arc<dyn WorldStore>,
    pub edition: Edition,
    /// Folder on disk (the workspace copy for editable sessions).
    pub path: PathBuf,
    pub name: String,
    /// Writes go to this store (an editable session: the user saves them
    /// back with the session's Save). In-place viewers are read-only.
    pub writable: bool,
    /// How writes reach disk (`None` = read-only).
    pub writer: Option<Arc<dyn WorldWriter>>,
}

impl std::fmt::Debug for WorldHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldHandle").field("name", &self.name).field("path", &self.path).field("edition", &self.edition).field("writable", &self.writable).finish()
    }
}

/// Opens a world folder for an extension (provided by the app: it knows
/// the editions' stores). Opened read-only.
pub type WorldOpener = Arc<dyn Fn(&Path) -> Result<WorldHandle, String> + Send + Sync>;

/// A world in the app's library (`omt.world.list()`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldEntry {
    pub name: String,
    pub path: PathBuf,
    pub edition: Edition,
}

/// Live state shared between a tool's worker and the app.
#[derive(Default)]
pub struct Shared {
    /// Set by the app's Stop button; checked by the Lua hook and long loops.
    pub cancel: AtomicBool,
    /// `omt.progress(text, fraction)` while a callback runs.
    pub progress: Mutex<Option<(String, f32)>>,
    /// A callback is running.
    pub busy: AtomicBool,
}

impl Shared {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn set_progress(&self, p: Option<(String, f32)>) {
        if let Ok(mut g) = self.progress.lock() {
            *g = p;
        }
    }

    pub fn progress(&self) -> Option<(String, f32)> {
        self.progress.lock().ok().and_then(|g| g.clone())
    }
}

/// Side effects a callback asks the app for (collected per call).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Effects {
    /// `omt.notify` messages (status line / toast).
    pub notes: Vec<String>,
    /// `omt.clipboard(text)`.
    pub clipboard: Option<String>,
    /// The tool wrote to the session's world (mark it modified).
    pub dirty: bool,
    /// NBT action result: replace the selected tag with this.
    pub replace: Option<omt_nbt::Tag>,
    /// `print` / `omt.log` output, `(level, message)`.
    pub logs: Vec<(String, String)>,
}

/// Per-Lua-state context (`Lua::app_data`).
pub struct Rt {
    pub shared: Arc<Shared>,
    /// The world the tool was opened on.
    pub world: Option<WorldHandle>,
    pub opener: Option<WorldOpener>,
    pub library: Vec<WorldEntry>,
    /// The extension's own folder (`omt.storage`, assets).
    pub ext_dir: PathBuf,
    pub effects: Effects,
}

impl Rt {
    /// Fail fast when the user pressed Stop.
    pub fn check(&self) -> mlua::Result<()> {
        if self.shared.cancelled() { Err(mlua::Error::runtime("cancelled")) } else { Ok(()) }
    }
}
