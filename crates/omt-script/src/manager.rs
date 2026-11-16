//! Extension packages on disk: discovery, install (zip / built-in files),
//! enable/disable, ordering, removal. State persists as `extensions.json`
//! next to the extension folders (`{ "order": […], "disabled": […] }`,
//! written atomically). Unknown folders without a `main.lua` are ignored.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{host, Context, NbtStyleRun};

/// Per-package limits (sanity, not security — see the crate trust model).
const MAX_ENTRY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ManagerError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad manifest.json: {0}")]
    Manifest(String),
    #[error("not an OMT extension zip: {0}")]
    NotExtension(String),
    #[error("unsafe zip entry {0:?}")]
    UnsafeEntry(String),
    #[error("invalid extension id {0:?} (allowed: a-z, 0-9, '-', '_')")]
    BadId(String),
    #[error("{0}")]
    Other(String),
}

/// `manifest.json` (OMT-defined schema; unknown keys are tolerated).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Human-readable name (required).
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
}

/// One installed extension.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionRecord {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    order: Vec<String>,
    #[serde(default)]
    disabled: Vec<String>,
}

/// Extensions directory handle. Cheap to share behind an `Arc`; every
/// mutation bumps [`Manager::revision`] so users (e.g. editor tabs caching a
/// pass) can invalidate.
pub struct Manager {
    root: PathBuf,
    records: RwLock<Vec<ExtensionRecord>>,
    disabled: RwLock<BTreeSet<String>>,
    rev: AtomicU64,
}

/// Recover from a poisoned lock: a prior panic mid-write leaves the data
/// structurally valid (each critical section is a single Vec/BTreeSet op).
fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|e| e.into_inner())
}

impl Manager {
    /// Opens (and if needed creates) the extensions directory. A missing or
    /// corrupt `extensions.json` degrades to "everything enabled, folder
    /// order" rather than an error.
    pub fn open(root: impl Into<PathBuf>) -> Self {
        let mgr = Self {
            root: root.into(),
            records: RwLock::new(Vec::new()),
            disabled: RwLock::new(BTreeSet::new()),
            rev: AtomicU64::new(1),
        };
        mgr.reload();
        mgr
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Monotonic revision; changes on any mutation or reload.
    pub fn revision(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }

    fn bump(&self) {
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    /// Rescans the directory and re-applies saved ordering.
    pub fn reload(&self) {
        let cfg = std::fs::read_to_string(self.root.join("extensions.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Config>(&t).ok())
            .unwrap_or_default();
        let mut records = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for e in entries.flatten() {
                let dir = e.path();
                if !dir.join("main.lua").is_file() {
                    continue;
                }
                let Some(id) = e.file_name().to_str().map(str::to_string) else { continue };
                let manifest = std::fs::read_to_string(dir.join("manifest.json"))
                    .ok()
                    .and_then(|t| serde_json::from_str::<Manifest>(&t).ok())
                    .unwrap_or(Manifest { name: id.clone(), version: String::new(), author: String::new(), description: String::new() });
                records.push(ExtensionRecord { id, dir, manifest });
            }
        }
        // Saved order first (existing ids only), then any new folders.
        records.sort_by_key(|r| {
            let pos = cfg.order.iter().position(|o| o == &r.id);
            (pos.unwrap_or(usize::MAX), r.id.clone())
        });
        *write(&self.disabled) = cfg.disabled.into_iter().collect();
        *write(&self.records) = records;
        self.bump();
    }

    /// Installed extensions in run order.
    pub fn records(&self) -> Vec<ExtensionRecord> {
        read(&self.records).clone()
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        !read(&self.disabled).contains(id)
    }

    /// Enabled ids in run order.
    pub fn enabled_ids(&self) -> Vec<String> {
        read(&self.records).iter().filter(|r| self.is_enabled(&r.id)).map(|r| r.id.clone()).collect()
    }

    pub fn set_enabled(&self, id: &str, on: bool) -> Result<(), ManagerError> {
        {
            let mut d = write(&self.disabled);
            if on {
                d.remove(id);
            } else if read(&self.records).iter().any(|r| r.id == id) {
                d.insert(id.to_string());
            } else {
                return Err(ManagerError::Other(format!("no extension {id:?}")));
            }
        }
        self.save()
    }

    /// Move `id` one slot up (−1) or down (+1) in the run order.
    pub fn reorder(&self, id: &str, delta: i32) -> Result<(), ManagerError> {
        let mut order: Vec<String> = read(&self.records).iter().map(|r| r.id.clone()).collect();
        let Some(pos) = order.iter().position(|o| o == id) else {
            return Err(ManagerError::Other(format!("no extension {id:?}")));
        };
        let target = pos as i64 + i64::from(delta);
        if !(0..order.len() as i64).contains(&target) {
            return Ok(());
        }
        let target = target as usize;
        order.swap(pos, target);
        self.persist_order(&order)
    }

    /// Deletes an extension's folder and config entries.
    pub fn remove(&self, id: &str) -> Result<(), ManagerError> {
        let dir = read(&self.records).iter().find(|r| r.id == id).map(|r| r.dir.clone());
        let Some(dir) = dir else {
            return Err(ManagerError::Other(format!("no extension {id:?}")));
        };
        std::fs::remove_dir_all(dir)?;
        self.reload();
        self.save()
    }

    fn persist_order(&self, order: &[String]) -> Result<(), ManagerError> {
        // `reload` sorts by saved order, so persist then reload keeps state honest.
        let disabled: Vec<String> = read(&self.disabled).iter().cloned().collect();
        let cfg = Config { order: order.to_vec(), disabled };
        let text = serde_json::to_string_pretty(&cfg).map_err(|e| ManagerError::Other(e.to_string()))?;
        std::fs::create_dir_all(&self.root)?;
        let tmp = self.root.join("extensions.json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, self.root.join("extensions.json"))?;
        self.reload();
        Ok(())
    }

    fn save(&self) -> Result<(), ManagerError> {
        let order: Vec<String> = read(&self.records).iter().map(|r| r.id.clone()).collect();
        self.persist_order(&order)
    }

    /// Installs an extension from a `.zip` archive. The zip must carry
    /// `manifest.json` + `main.lua` either at its root or under a single
    /// top-level folder. Installing an existing id replaces it.
    pub fn install_zip(&self, zip_path: &Path) -> Result<String, ManagerError> {
        let file = std::fs::File::open(zip_path)?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| ManagerError::NotExtension(e.to_string()))?;
        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        let prefix = install_prefix(&names)?;        let mut files = Vec::new();
        let mut total: u64 = 0;
        for name in &names {
            if name.ends_with('/') {
                continue;
            }
            let rel = strip_prefix_dir(name, &prefix);
            if rel.is_empty() {
                continue;
            }
            ensure_safe_rel(rel)?;
            let mut entry = archive.by_name(name).map_err(|e| ManagerError::NotExtension(e.to_string()))?;
            if entry.is_dir() || entry.is_symlink() {
                return Err(ManagerError::UnsafeEntry(name.clone()));
            }
            if entry.size() > MAX_ENTRY_BYTES {
                return Err(ManagerError::UnsafeEntry(format!("{name} (too large)")));
            }
            total += entry.size();
            if total > MAX_TOTAL_BYTES {
                return Err(ManagerError::UnsafeEntry("archive exceeds size budget".into()));
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            std::io::Read::read_to_end(&mut entry, &mut bytes)?;
            files.push((rel.to_string(), bytes));
        }
        let manifest_src = files
            .iter()
            .find(|(p, _)| p == "manifest.json")
            .map(|(_, b)| b.clone())
            .ok_or_else(|| ManagerError::NotExtension("manifest.json missing".into()))?;
        let manifest: Manifest =
            serde_json::from_slice(&manifest_src).map_err(|e| ManagerError::Manifest(e.to_string()))?;
        let id = id_from_name(&manifest.name)?;
        self.install_files(&id, &files, Some(manifest))?;
        Ok(id)
    }

    /// Installs `(relative path, bytes)` file sets (used by zip install and
    /// the built-in example). Replaces an existing installation of `id`.
    pub fn install_files(&self, id: &str, files: &[(String, Vec<u8>)], manifest: Option<Manifest>) -> Result<(), ManagerError> {
        if !valid_id(id) {
            return Err(ManagerError::BadId(id.to_string()));
        }
        if !files.iter().any(|(p, _)| p == "main.lua") {
            return Err(ManagerError::NotExtension("main.lua missing".into()));
        }
        let manifest = match manifest {
            Some(m) => m,
            None => {
                let src = files
                    .iter()
                    .find(|(p, _)| p == "manifest.json")
                    .map(|(_, b)| b.clone())
                    .ok_or_else(|| ManagerError::NotExtension("manifest.json missing".into()))?;
                serde_json::from_slice(&src).map_err(|e| ManagerError::Manifest(e.to_string()))?
            }
        };
        let dir = self.root.join(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        for (rel, bytes) in files {
            ensure_safe_rel(rel)?;
            let dest = dir.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, bytes)?;
        }
        let _ = std::fs::write(dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap_or_default());
        self.reload();
        self.save()
    }

    /// Runs all enabled extensions over one document (see [`crate::host`]).
    pub fn run(&self, root_name: &str, root: &omt_nbt::Tag, ctx: &Context) -> NbtStyleRun {
        let ids = self.enabled_ids();
        if ids.is_empty() {
            return NbtStyleRun::default();
        }
        let run = host::run(&self.root, &ids, root_name, root, ctx);
        if std::env::var_os("OMT_SCRIPT_DEBUG").is_some() {
            eprintln!(
                "[omt-script] run ids={ids:?} errors={:?} logs={:?} styles={} edited={}",
                run.errors,
                run.logs,
                run.styles.len(),
                run.edited_root.is_some()
            );
        }
        run
    }
}

/// `""` when `manifest.json` sits at the zip root, else the single top-level
/// folder that contains it.
fn install_prefix(names: &[String]) -> Result<String, ManagerError> {
    if names.iter().any(|n| n == "manifest.json" || n == "main.lua") {
        return Ok(String::new());
    }
    let tops: BTreeSet<String> = names.iter().filter_map(|n| n.split_once('/').map(|(top, _)| top.to_string())).collect();
    if tops.len() == 1 {
        let Some(top) = tops.into_iter().next() else { return Err(ManagerError::NotExtension("empty archive".into())) };
        if names.iter().any(|n| n == &format!("{top}/manifest.json") || n == &format!("{top}/main.lua")) {
            return Ok(top);
        }
    }
    Err(ManagerError::NotExtension("manifest.json/main.lua not at the archive root".into()))
}

fn strip_prefix_dir<'a>(name: &'a str, prefix: &str) -> &'a str {
    if prefix.is_empty() {
        name
    } else {
        name.strip_prefix(&format!("{prefix}/")).unwrap_or(name)
    }
}

fn ensure_safe_rel(rel: &str) -> Result<(), ManagerError> {
    let path = Path::new(rel);
    let clean = path.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    if clean && !rel.starts_with('/') && !rel.contains('\\') && !rel.contains(": ") {
        Ok(())
    } else {
        Err(ManagerError::UnsafeEntry(rel.to_string()))
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') && id.len() <= 64
}

fn id_from_name(name: &str) -> Result<String, ManagerError> {
    let mut id = String::new();
    for ch in name.trim().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_' {
            id.push(ch);
        } else if ch.is_ascii_uppercase() {
            id.push(ch.to_ascii_lowercase());
        } else {
            id.push('-');
        }
    }
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-').to_string();
    if valid_id(&id) {
        Ok(id)
    } else {
        Err(ManagerError::BadId(name.to_string()))
    }
}

/// The OMT-authored example extension (also shipped as source files under
/// `assets/ticks-time/`): tick counters shown as in-game time.
pub fn example_files() -> Vec<(String, Vec<u8>)> {
    vec![
        ("manifest.json".into(), include_bytes!("../assets/ticks-time/manifest.json").to_vec()),
        ("main.lua".into(), include_bytes!("../assets/ticks-time/main.lua").to_vec()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("omt-script-mgr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            z.start_file(*name, opts).unwrap();
            std::io::Write::write_all(&mut z, data.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn id_slugs() {
        assert_eq!(id_from_name("Tick Clock").unwrap(), "tick-clock");
        assert_eq!(id_from_name("A_B!").unwrap(), "a_b");
        assert!(id_from_name("  ").is_err());
    }

    #[test]
    fn install_toggle_order_remove() {
        let root = tmp_root("full");
        let mgr = Manager::open(&root);
        mgr.install_files("a", &[("main.lua".into(), b"return {}".to_vec())], Some(Manifest { name: "A".into(), version: "1".into(), author: String::new(), description: String::new() })).unwrap();
        mgr.install_files("b", &[("main.lua".into(), b"return {}".to_vec())], Some(Manifest { name: "B".into(), version: "1".into(), author: String::new(), description: String::new() })).unwrap();
        assert_eq!(mgr.enabled_ids(), vec!["a", "b"]);

        mgr.set_enabled("a", false).unwrap();
        assert_eq!(mgr.enabled_ids(), vec!["b"]);
        // Persisted: a fresh Manager sees the same state.
        assert_eq!(Manager::open(&root).enabled_ids(), vec!["b"]);

        mgr.set_enabled("a", true).unwrap();
        mgr.reorder("b", -1).unwrap();
        assert_eq!(mgr.enabled_ids(), vec!["b", "a"]);
        assert_eq!(Manager::open(&root).enabled_ids(), vec!["b", "a"]);

        // Reinstall replaces.
        mgr.install_files("b", &[("main.lua".into(), b"return {1}".to_vec())], None).unwrap_err(); // manifest.json required
        mgr.remove("b").unwrap();
        assert_eq!(mgr.enabled_ids(), vec!["a"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn zip_install_variants_and_safety() {
        let root = tmp_root("zip");
        std::fs::create_dir_all(&root).unwrap();
        let mgr = Manager::open(&root);
        let manifest = r#"{ "name": "Tick Clock", "version": "1.0", "author": "x", "description": "y" }"#;

        let z1 = root.join("flat.zip");
        write_zip(&z1, &[("manifest.json", manifest), ("main.lua", "return {}")]);
        assert_eq!(mgr.install_zip(&z1).unwrap(), "tick-clock");

        let z2 = root.join("nested.zip");
        write_zip(&z2, &[("pkg/manifest.json", manifest), ("pkg/main.lua", "return {}"), ("pkg/lib/util.lua", "return 1")]);
        assert_eq!(mgr.install_zip(&z2).unwrap(), "tick-clock");
        assert!(mgr.root().join("tick-clock/lib/util.lua").is_file());

        let z3 = root.join("evil.zip");
        write_zip(&z3, &[("manifest.json", manifest), ("main.lua", "return {}"), ("../evil.lua", "x")]);
        assert!(mgr.install_zip(&z3).is_err());
        assert!(!root.parent().unwrap().join("evil.lua").exists());

        let z4 = root.join("notext.zip");
        write_zip(&z4, &[("a/b/manifest.json", manifest), ("a/b/main.lua", "return {}")]);
        assert!(mgr.install_zip(&z4).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn example_installs() {
        let root = tmp_root("example");
        let mgr = Manager::open(&root);
        mgr.install_files("ticks-time", &example_files(), None).unwrap();
        assert_eq!(mgr.enabled_ids(), vec!["ticks-time"]);
        let rec = &mgr.records()[0];
        assert_eq!(rec.manifest.name, "Tick Clock");
        let _ = std::fs::remove_dir_all(&root);
    }
}
