//! A world opened in one of the apps: its copy-on-open workspace plus one
//! shared store handle (one LevelDB handle per world — the pruner, the map
//! and the NBT editor all go through it).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use omt_bedrock::BedrockWorld;
use omt_java::JavaWorld;
use omt_world::{Edition, Workspace, WorldStore, WorldSummary};

/// Run blocking work on a dedicated thread and await it from an iced task.
pub fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl Future<Output = T> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.expect("worker thread panicked") }
}

/// The UMT apps a world can be opened in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    NbtEditor,
    Converter,
    Pruner,
}

impl App {
    pub const fn name(self) -> &'static str {
        match self {
            Self::NbtEditor => "NBT Editor",
            Self::Converter => "Converter",
            Self::Pruner => "Pruner",
        }
    }
}

#[derive(Clone)]
pub enum Store {
    Java(Arc<JavaWorld>),
    Bedrock(Arc<BedrockWorld>),
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        match omt_world::detect(root).map_err(|e| e.to_string())? {
            omt_world::Detected::JavaFolder { .. } => {
                Ok(Self::Java(Arc::new(JavaWorld::open(root).map_err(|e| e.to_string())?)))
            }
            omt_world::Detected::BedrockFolder { .. } => {
                Ok(Self::Bedrock(Arc::new(BedrockWorld::open_writable(root).map_err(|e| e.to_string())?)))
            }
            omt_world::Detected::Archive(p) => Err(format!("{} is an archive", p.display())),
        }
    }

    pub fn world(&self) -> &(dyn WorldStore + Sync) {
        match self {
            Self::Java(w) => w.as_ref(),
            Self::Bedrock(w) => w.as_ref(),
        }
    }

    pub fn edition(&self) -> Edition {
        match self {
            Self::Java(_) => Edition::Java,
            Self::Bedrock(_) => Edition::Bedrock,
        }
    }

    pub fn prune_world(&self) -> omt_prune::PruneWorld {
        match self {
            Self::Java(w) => omt_prune::PruneWorld::Java(Arc::clone(w)),
            Self::Bedrock(w) => omt_prune::PruneWorld::Bedrock(Arc::clone(w)),
        }
    }
}

pub struct WorldSession {
    pub app: App,
    pub workspace: Option<Workspace>,
    pub store: Option<Store>,
    pub summary: WorldSummary,
    pub title: String,
    /// Unsaved modifications exist in the workspace.
    pub modified: bool,
}

impl std::fmt::Debug for WorldSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldSession").field("app", &self.app).field("title", &self.title).finish()
    }
}

impl WorldSession {
    /// Copy/extract the world and open its store (blocking).
    pub fn open(app: App, origin: &Path, base: &Path) -> Result<Self, String> {
        let ws = Workspace::open(origin, base).map_err(|e| e.to_string())?;
        let detected = omt_world::detect(ws.root()).map_err(|e| e.to_string())?;
        let summary = WorldSummary::from_detected(&detected).map_err(|e| e.to_string())?;
        let store = match Store::open(ws.root()) {
            Ok(s) => s,
            Err(e) => {
                let _ = ws.discard();
                return Err(e);
            }
        };
        let title = summary.name.clone();
        Ok(Self { app, workspace: Some(ws), store: Some(store), summary, title, modified: false })
    }

    pub fn root(&self) -> PathBuf {
        self.workspace.as_ref().map(|w| w.root().to_path_buf()).unwrap_or_default()
    }

    pub fn origin(&self) -> PathBuf {
        self.workspace.as_ref().map(|w| w.origin().to_path_buf()).unwrap_or_default()
    }

    pub fn edition(&self) -> Edition {
        self.summary.edition
    }

    /// Release the store, write the workspace over the origin (backup
    /// kept), reopen. Blocking. Fails if a background job still holds the
    /// store.
    pub fn save(&mut self) -> Result<PathBuf, String> {
        self.release_store()?;
        let result = self.workspace.as_ref().ok_or("no workspace")?.save().map_err(|e| e.to_string());
        self.store = Some(Store::open(&self.root())?);
        let backup = result?;
        self.modified = false;
        Ok(backup)
    }

    /// Export a copy (folder or `.mcworld`/`.zip`) without touching the origin.
    pub fn save_as(&mut self, dest: &Path) -> Result<(), String> {
        self.release_store()?;
        let result = self.workspace.as_ref().ok_or("no workspace")?.save_as(dest).map_err(|e| e.to_string());
        self.store = Some(Store::open(&self.root())?);
        result
    }

    fn release_store(&mut self) -> Result<(), String> {
        if let Some(store) = self.store.take() {
            let unique = match &store {
                Store::Java(w) => Arc::strong_count(w) == 1,
                Store::Bedrock(w) => Arc::strong_count(w) == 1,
            };
            if !unique {
                self.store = Some(store);
                return Err("a background job is still using the world — wait for it to finish".into());
            }
            if let Store::Bedrock(w) = store
                && let Ok(mut w) = Arc::try_unwrap(w)
            {
                let _ = omt_world::WorldStore::flush(&mut w);
            }
        }
        Ok(())
    }

    /// Discard the workspace copy (UMT "Close" without saving).
    pub fn close(mut self) {
        self.store = None;
        if let Some(ws) = self.workspace.take() {
            let _ = ws.discard();
        }
    }
}

impl Drop for WorldSession {
    fn drop(&mut self) {
        self.store = None;
        if let Some(ws) = self.workspace.take() {
            let _ = ws.discard();
        }
    }
}
