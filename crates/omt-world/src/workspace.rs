//! Copy-on-open workspaces — UMT's safety model, PLAN §3.3.
//!
//! Opening a world never touches the original: folders are copied and
//! archives (`.zip`, `.mcworld`, `.mctemplate`) are extracted into a private
//! workspace directory, every tool edits that copy, **Save** writes the copy
//! back over the origin (keeping one `<name>.omt_backup` of what it
//! replaced), **Save As** exports a folder or archive anywhere, and
//! **Close/discard** deletes the copy.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("I/O on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("archive {path}: {reason}")]
    Archive { path: PathBuf, reason: String },
    #[error("no level.dat found inside {0}")]
    NoWorld(PathBuf),
}

pub type WorkspaceResult<T> = Result<T, WorkspaceError>;

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> WorkspaceError + '_ {
    move |source| WorkspaceError::Io { path: path.to_path_buf(), source }
}

/// Where a workspace came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginKind {
    Folder,
    /// `.zip`, `.mcworld` or `.mctemplate`.
    Archive,
}

/// Archive extensions the workspace opens and writes.
pub const ARCHIVE_EXTENSIONS: [&str; 3] = ["zip", "mcworld", "mctemplate"];

pub fn is_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ARCHIVE_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

/// Default parent for workspaces: `$XDG_DATA_HOME/omt/workspaces` (or the
/// platform equivalent), overridable by the Settings "Working Directory".
pub fn default_base() -> PathBuf {
    if let Some(dir) = std::env::var_os("OMT_WORKSPACE_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(dir).join("OmniMCTools").join("workspaces");
    }
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("omt").join("workspaces");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/omt/workspaces");
    }
    std::env::temp_dir().join("omt-workspaces")
}

/// An open world copy.
#[derive(Debug)]
pub struct Workspace {
    origin: PathBuf,
    kind: OriginKind,
    /// The workspace directory (owned; removed by [`Workspace::discard`]).
    dir: PathBuf,
    /// The world root inside `dir` (the folder holding `level.dat`).
    root: PathBuf,
}

impl Workspace {
    /// Copy/extract `origin` into a fresh directory under `base`.
    pub fn open(origin: &Path, base: &Path) -> WorkspaceResult<Self> {
        let stem = origin
            .file_stem()
            .map(|s| sanitize(&s.to_string_lossy()))
            .unwrap_or_else(|| "world".into());
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let dir = base.join(format!("{stem}-{stamp}"));
        std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        let result = if origin.is_dir() {
            copy_dir(origin, &dir).map(|()| (OriginKind::Folder, dir.clone()))
        } else if is_archive(origin) {
            extract(origin, &dir).and_then(|()| {
                find_world_root(&dir)
                    .map(|r| (OriginKind::Archive, r))
                    .ok_or_else(|| WorkspaceError::NoWorld(origin.to_path_buf()))
            })
        } else {
            Err(WorkspaceError::NoWorld(origin.to_path_buf()))
        };
        match result {
            Ok((kind, root)) => Ok(Self { origin: origin.to_path_buf(), kind, dir, root }),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                Err(e)
            }
        }
    }

    pub fn origin(&self) -> &Path {
        &self.origin
    }

    pub fn kind(&self) -> OriginKind {
        self.kind
    }

    /// The editable world root (contains `level.dat`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write the workspace back over the origin. The replaced original is
    /// kept as `<origin>.omt_backup` (one generation; an older backup is
    /// replaced). Returns the backup path.
    pub fn save(&self) -> WorkspaceResult<PathBuf> {
        let backup = sibling(&self.origin, ".omt_backup");
        let staging = sibling(&self.origin, ".omt_saving");
        remove_any(&staging)?;
        match self.kind {
            OriginKind::Folder => copy_dir(&self.root, &staging)?,
            OriginKind::Archive => write_archive(&self.root, &staging)?,
        }
        remove_any(&backup)?;
        if self.origin.exists() {
            std::fs::rename(&self.origin, &backup).map_err(io(&self.origin))?;
        }
        std::fs::rename(&staging, &self.origin).map_err(io(&self.origin))?;
        Ok(backup)
    }

    /// Export to `dest`: an archive when `dest` has an archive extension
    /// (Bedrock → `.mcworld`), otherwise a folder. Refuses to overwrite.
    pub fn save_as(&self, dest: &Path) -> WorkspaceResult<()> {
        if dest.exists() {
            return Err(WorkspaceError::Io {
                path: dest.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, "destination exists"),
            });
        }
        if is_archive(dest) { write_archive(&self.root, dest) } else { copy_dir(&self.root, dest) }
    }

    /// Delete the workspace copy (UMT "Close" without saving).
    pub fn discard(self) -> WorkspaceResult<()> {
        remove_any(&self.dir)
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(48)
        .collect();
    if s.is_empty() { "world".into() } else { s }
}

fn remove_any(path: &Path) -> WorkspaceResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path).map_err(io(path)),
        Ok(_) => std::fs::remove_file(path).map_err(io(path)),
        Err(_) => Ok(()),
    }
}

/// Recursive copy (files + dirs; symlinks are copied as their targets'
/// contents would be risky, so they are skipped — worlds never use them).
pub fn copy_dir(src: &Path, dst: &Path) -> WorkspaceResult<()> {
    std::fs::create_dir_all(dst).map_err(io(dst))?;
    for entry in std::fs::read_dir(src).map_err(io(src))? {
        let entry = entry.map_err(io(src))?;
        let ty = entry.file_type().map_err(io(&entry.path()))?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &to).map_err(io(&entry.path()))?;
        }
    }
    Ok(())
}

/// The shallowest directory under `dir` holding a `level.dat` (Java zips
/// usually wrap the world in a folder; `.mcworld` puts it at the root).
pub fn find_world_root(dir: &Path) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::from([(dir.to_path_buf(), 0u8)]);
    while let Some((d, depth)) = queue.pop_front() {
        if d.join("level.dat").is_file() {
            return Some(d);
        }
        if depth >= 3 {
            continue;
        }
        let mut subs: Vec<PathBuf> = std::fs::read_dir(&d)
            .ok()?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n != "__MACOSX"))
            .collect();
        subs.sort();
        queue.extend(subs.into_iter().map(|p| (p, depth + 1)));
    }
    None
}

fn extract(archive: &Path, dir: &Path) -> WorkspaceResult<()> {
    let bad = |reason: String| WorkspaceError::Archive { path: archive.to_path_buf(), reason };
    let file = std::fs::File::open(archive).map_err(io(archive))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| bad(e.to_string()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| bad(e.to_string()))?;
        // `enclosed_name` rejects absolute paths and `..` (zip-slip).
        let Some(rel) = entry.enclosed_name() else {
            return Err(bad(format!("unsafe entry path {:?}", entry.name())));
        };
        let out = dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(io(&out))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf).map_err(io(&out))?;
        std::fs::write(&out, &buf).map_err(io(&out))?;
    }
    Ok(())
}

/// Zip `root`'s contents (world files at the archive root, the layout both
/// `.mcworld` and Java world zips use when produced by OMT).
pub fn write_archive(root: &Path, dest: &Path) -> WorkspaceResult<()> {
    let bad = |reason: String| WorkspaceError::Archive { path: dest.to_path_buf(), reason };
    let file = std::fs::File::create(dest).map_err(io(dest))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(io(&d))?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.is_file() {
                files.push(p);
            }
        }
    }
    files.sort();
    for p in files {
        let rel = p.strip_prefix(root).unwrap_or(&p);
        let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        zip.start_file(name, opts).map_err(|e| bad(e.to_string()))?;
        let data = std::fs::read(&p).map_err(io(&p))?;
        zip.write_all(&data).map_err(io(dest))?;
    }
    zip.finish().map_err(|e| bad(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("omt-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn folder_roundtrip_with_backup() {
        let t = tmp("folder");
        let origin = t.join("World");
        std::fs::create_dir_all(origin.join("db")).unwrap();
        std::fs::write(origin.join("level.dat"), b"v1").unwrap();
        std::fs::write(origin.join("db/a"), b"x").unwrap();
        let ws = Workspace::open(&origin, &t.join("ws")).unwrap();
        assert_eq!(ws.kind(), OriginKind::Folder);
        std::fs::write(ws.root().join("level.dat"), b"v2").unwrap();
        // Original untouched until save.
        assert_eq!(std::fs::read(origin.join("level.dat")).unwrap(), b"v1");
        let backup = ws.save().unwrap();
        assert_eq!(std::fs::read(origin.join("level.dat")).unwrap(), b"v2");
        assert_eq!(std::fs::read(backup.join("level.dat")).unwrap(), b"v1");
        assert_eq!(std::fs::read(origin.join("db/a")).unwrap(), b"x");
        let dir = ws.dir.clone();
        ws.discard().unwrap();
        assert!(!dir.exists());
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn archive_roundtrip_nested_root() {
        let t = tmp("archive");
        let src = t.join("src/MyWorld");
        std::fs::create_dir_all(src.join("region")).unwrap();
        std::fs::write(src.join("level.dat"), b"lvl").unwrap();
        std::fs::write(src.join("region/r.0.0.mca"), b"rg").unwrap();
        // A Java-style zip wrapping the world in a folder.
        let zip_path = t.join("w.zip");
        write_archive(&t.join("src"), &zip_path).unwrap();
        let ws = Workspace::open(&zip_path, &t.join("ws")).unwrap();
        assert_eq!(ws.kind(), OriginKind::Archive);
        assert!(ws.root().ends_with("MyWorld"));
        assert_eq!(std::fs::read(ws.root().join("region/r.0.0.mca")).unwrap(), b"rg");
        // Export as .mcworld (world at archive root) and reopen.
        let out = t.join("out.mcworld");
        ws.save_as(&out).unwrap();
        let again = Workspace::open(&out, &t.join("ws2")).unwrap();
        assert_eq!(std::fs::read(again.root().join("level.dat")).unwrap(), b"lvl");
        assert!(ws.save_as(&out).is_err(), "never overwrites");
        // Save back into the zip origin.
        std::fs::write(ws.root().join("level.dat"), b"new").unwrap();
        ws.save().unwrap();
        let re = Workspace::open(&zip_path, &t.join("ws3")).unwrap();
        assert_eq!(std::fs::read(re.root().join("level.dat")).unwrap(), b"new");
        std::fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn non_world_is_rejected_and_cleaned() {
        let t = tmp("none");
        let zip_path = t.join("empty.zip");
        std::fs::create_dir_all(t.join("e")).unwrap();
        std::fs::write(t.join("e/readme.txt"), b"hi").unwrap();
        write_archive(&t.join("e"), &zip_path).unwrap();
        let base = t.join("ws");
        assert!(matches!(Workspace::open(&zip_path, &base), Err(WorkspaceError::NoWorld(_))));
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0, "failed open leaves nothing");
        std::fs::remove_dir_all(&t).unwrap();
    }
}
