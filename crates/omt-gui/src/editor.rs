//! NBT editor tab state (M2): document + undo + navigation + search.
//!
//! View-free by design — `editor_view` renders this, `main` routes messages.
//! Every mutation goes through the shared `omt_nbt` edit engine.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use omt_nbt::{ChildId, EditOp, NbtDocument, Tag, TagPath, UndoStack};
use omt_nbt::snbt::{self, from_snbt};

pub use crate::rows::{ChangeKind, Row};
use crate::rows::collect_hits;
/// Visible-row cap so giant documents (actor NBT, big arrays) stay fluid.
const MAX_ROWS: usize = 1500;

#[derive(Debug, Clone)]
pub enum EditorMsg {
    ToggleExpand(String),
    Select(String),
    ValueInput(String),
    CommitValue,
    NameInput(String),
    CommitRename,
    AddNameInput(String),
    AddValueInput(String),
    CommitAddChild,
    CommitAddIndex,
    DeleteSelected,
    MoveUp,
    MoveDown,
    Undo,
    Redo,
    Save,
    SearchInput(String),
    SearchNext,
    CopySelected,
    SnbtImportInput(String),
    PasteFromClipboard(String),
    /// Ask the host to read the clipboard and send `PasteFromClipboard`.
    RequestPaste,
    ApplyImport,
    /// Run an extension NBT action (`ext/id`) on the selected tag.
    RunAction(String),
}

/// Where a document is saved. `File` documents save themselves (atomic +
/// backup); the others set [`EditorTab::pending_save`] and the owning world
/// session writes them into its workspace store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sink {
    File,
    /// A Java chunk inside a region file (big-endian NBT).
    JavaChunk { region: PathBuf, x: i32, z: i32 },
    /// One Bedrock LevelDB value holding a single little-endian NBT root.
    BedrockKey(Vec<u8>),
    /// A Bedrock value holding a *sequence* of LE roots (block entities
    /// 0x31, pending ticks): edited as a root `entries` list.
    BedrockSequence(Vec<u8>),
    /// Several Bedrock actor records (one chunk's entities), edited as a
    /// root `entries` list written back one key per entry.
    BedrockActors(Vec<Vec<u8>>),
    /// A file inside a world workspace: written atomically by the session
    /// (no `.omt_backup` inside the workspace — the world Save keeps one).
    WorkspaceFile(PathBuf),
}

pub struct EditorTab {
    pub doc: NbtDocument,
    pub sink: Sink,
    /// Set when a non-file document asked to be saved.
    pub pending_save: bool,
    pub undo: UndoStack,
    /// `applied.len()` at last save; dirty when different.
    saved_ops: usize,
    pub title: String,
    expanded: HashSet<String>,
    pub selected: Option<TagPath>,
    pub rows: Vec<Row>,
    pub truncated: bool,

    pub value_input: String,
    pub name_input: String,
    pub add_name_input: String,
    pub add_value_input: String,
    pub search_input: String,
    search_hits: Vec<TagPath>,
    search_cursor: usize,
    pub snbt_import: String,

    /// Lua extensions (shared app-wide); `None` in tests/CLI paths.
    pub ext: Option<Arc<omt_script::Manager>>,
    /// Empty-list element types per entry root, for sequence sinks
    /// (Bedrock block entities / ticks / actors) — see `sink_bytes`.
    pub entry_hints: Vec<omt_nbt::ListHints>,
    /// Last extension pass: `(manager revision, document root, styles)` —
    /// reused while the tree is unchanged so keystrokes never re-run Lua.
    pub(crate) ext_cache: Option<(u64, Tag, HashMap<String, omt_script::RowStyle>)>,
    /// Stable identity (extension actions hand their edits back to it).
    pub uid: u64,
    /// Extension NBT actions offered on the selected tag, and the
    /// discovery generation they come from.
    pub actions: Vec<omt_extensions::tool::ToolDecl>,
    pub actions_gen: u64,
    /// An action the user started: `(ext/id, path, name, tag)` — the app
    /// picks it up and opens the action's tool.
    pub pending_action: Option<(String, TagPath, String, Tag)>,
    /// Live styles for row rendering (kept in sync with `ext_cache`).
    pub(crate) ext_styles: HashMap<String, omt_script::RowStyle>,
    /// The tree state whose extension edit was last applied/undone (see
    /// `rows::refresh_extensions`) — keyed by content, so undoing an
    /// extension edit is respected until the tree changes otherwise.
    pub(crate) ext_edited_input: Option<Tag>,
    /// One-line health note ("2 extensions", "extension error: …").
    pub ext_status: String,

    pub status: String,
    pub last_backup: Option<PathBuf>,
}

impl EditorTab {
    pub fn open(path: &Path) -> Result<Self, String> {
        let doc = NbtDocument::load(path).map_err(|e| e.to_string())?;
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        Ok(Self::with_doc(doc, title, Sink::File))
    }

    /// An in-memory document saved through `sink`.
    pub fn from_nbt(title: String, nbt: omt_nbt::Nbt, order: omt_nbt::ByteOrder, sink: Sink) -> Self {
        let doc = NbtDocument {
            path: PathBuf::from(&title),
            format: match order {
                omt_nbt::ByteOrder::Big => omt_nbt::NbtFormat::RawBigEndian,
                omt_nbt::ByteOrder::Little => omt_nbt::NbtFormat::RawLittleEndian,
            },
            nbt,
            bedrock_storage_version: 0,
            list_hints: std::collections::BTreeMap::new(),
        };
        Self::with_doc(doc, title, sink)
    }

    pub fn with_doc(doc: NbtDocument, title: String, sink: Sink) -> Self {
        let mut tab = Self {
            doc,
            sink,
            pending_save: false,
            undo: UndoStack::default(),
            saved_ops: 0,
            title,
            expanded: HashSet::from([String::new()]), // root expanded
            selected: None,
            rows: Vec::new(),
            truncated: false,
            value_input: String::new(),
            name_input: String::new(),
            add_name_input: String::new(),
            add_value_input: String::new(),
            search_input: String::new(),
            search_hits: Vec::new(),
            search_cursor: 0,
            snbt_import: String::new(),
            ext: None,
            entry_hints: Vec::new(),
            ext_cache: None,
            uid: crate::editor_actions::next_uid(),
            actions: Vec::new(),
            actions_gen: 0,
            pending_action: None,
            ext_styles: HashMap::new(),
            ext_edited_input: None,
            ext_status: String::new(),
            status: String::new(),
            last_backup: None,
        };
        tab.rebuild();
        tab
    }

    /// The owning session finished writing a pending save.
    pub fn mark_saved(&mut self, status: String) {
        self.pending_save = false;
        self.saved_ops = self.undo.applied_ops().len();
        self.status = status;
    }

    /// Attach the app-wide extension manager and run the first pass over
    /// the just-opened document (so rows are decorated from the start).
    pub fn set_extensions(&mut self, mgr: Arc<omt_script::Manager>) {
        self.ext = Some(mgr);
        self.ext_cache = None;
        self.rebuild();
    }

    /// Serialized payload for the session-side sink write.
    pub fn sink_bytes(&self) -> Result<Vec<u8>, String> {
        match &self.sink {
            Sink::BedrockSequence(_) | Sink::BedrockActors(_) => {
                let entries = self.doc.root().get("entries").and_then(|t| t.as_list()).map(<[Tag]>::to_vec).unwrap_or_default();
                let mut out = Vec::new();
                for (i, e) in entries.iter().enumerate() {
                    let nbt = omt_nbt::Nbt { name: String::new(), tag: e.clone() };
                    let hints = self.entry_hints.get(i).cloned().unwrap_or_default();
                    // Hints keep no-op saves byte-identical to the game's bytes.
                    out.extend(omt_nbt::write_with_list_hints(&nbt, omt_nbt::ByteOrder::Little, &hints).map_err(|e| e.to_string())?);
                }
                Ok(out)
            }
            _ => self.doc.to_bytes().map_err(|e| e.to_string()),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.undo.applied_ops().len() != self.saved_ops
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    pub fn search_cursor(&self) -> usize {
        self.search_cursor
    }

    pub fn search_hit_count(&self) -> usize {
        self.search_hits.len()
    }

    /// Currently selected row (for the view's context controls).
    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.iter().find(|r| r.selected)
    }

    /// Expand a path's row (user clicks toggle; this is for the
    /// `OMT_DEBUG_NBT_ACTION` hook so a scripted edit is visible).
    pub(crate) fn expand(&mut self, path: &TagPath) {
        self.expanded.insert(path.to_string());
    }

    pub fn update(&mut self, msg: EditorMsg) -> Vec<iced::Task<EditorMsg>> {
        // Returned tasks (clipboard ops); kept small.
        match msg {
            EditorMsg::ToggleExpand(path) => {
                if self.expanded.contains(&path) {
                    self.expanded.remove(&path);
                } else {
                    self.expanded.insert(path);
                }
                self.rebuild();
            }
            EditorMsg::Select(path) => {
                self.selected = Some(path_of(&path));
                self.sync_inputs_to_selection();
                self.rebuild();
            }
            EditorMsg::ValueInput(text) => self.value_input = text,
            EditorMsg::CommitValue => self.commit_value(),
            EditorMsg::NameInput(text) => self.name_input = text,
            EditorMsg::CommitRename => self.commit_rename(),
            EditorMsg::AddNameInput(text) => self.add_name_input = text,
            EditorMsg::AddValueInput(text) => self.add_value_input = text,
            EditorMsg::CommitAddChild => self.commit_add_child(),
            EditorMsg::CommitAddIndex => self.commit_add_index(),
            EditorMsg::RunAction(key) => self.run_action(key),
            EditorMsg::DeleteSelected => self.delete_selected(),
            EditorMsg::MoveUp => self.move_selected(-1),
            EditorMsg::MoveDown => self.move_selected(1),
            EditorMsg::Undo => {
                match self.undo.undo(self.doc.root_mut()) {
                    Ok(true) => {
                        self.status = "undone".into();
                        self.rebuild();
                    }
                    Ok(false) => self.status = "nothing to undo".into(),
                    Err(e) => self.status = format!("undo failed: {e}"),
                }
            }
            EditorMsg::Redo => {
                match self.undo.redo(self.doc.root_mut()) {
                    Ok(true) => {
                        self.status = "redone".into();
                        self.rebuild();
                    }
                    Ok(false) => self.status = "nothing to redo".into(),
                    Err(e) => self.status = format!("redo failed: {e}"),
                }
            }
            EditorMsg::Save => self.save(),
            EditorMsg::SearchInput(text) => {
                self.search_input = text;
                self.run_search();
                self.rebuild();
            }
            EditorMsg::SearchNext => {
                if !self.search_hits.is_empty() {
                    self.search_cursor = (self.search_cursor + 1) % self.search_hits.len();
                    let hit = self.search_hits[self.search_cursor].clone();
                    self.reveal(&hit);
                    self.rebuild();
                }
            }
            EditorMsg::CopySelected => {
                if let Some(path) = &self.selected
                    && let Some(tag) = path.resolve(self.doc.root())
                {
                    let snbt_text = snbt::to_snbt(tag);
                    self.status = format!("copied {} chars of SNBT", snbt_text.len());
                    return vec![iced::clipboard::write(snbt_text)];
                }
                self.status = "nothing selected to copy".into();
            }
            EditorMsg::SnbtImportInput(text) => self.snbt_import = text,
            EditorMsg::PasteFromClipboard(text) => {
                self.snbt_import = text;
                self.status = "pasted from clipboard — press Import to apply".into();
            }
            EditorMsg::ApplyImport => self.apply_import(),
            EditorMsg::RequestPaste => {
                return vec![iced::clipboard::read().map(|c| EditorMsg::PasteFromClipboard(c.unwrap_or_default()))];
            }
        }
        Vec::new()
    }

    fn sync_inputs_to_selection(&mut self) {
        if let Some(path) = self.selected.clone()
            && let Some(tag) = path.resolve(self.doc.root())
        {
            match tag {
                Tag::Compound(_) | Tag::List(_) => {
                    self.value_input.clear();
                    self.name_input = match path.last() {
                        Some(omt_nbt::PathSeg::Key(k)) => k.clone(),
                        Some(omt_nbt::PathSeg::Index(i)) => format!("[{i}]"),
                        None => String::new(),
                    };
                }
                scalar => {
                    self.value_input = snbt::to_snbt(scalar);
                }
            }
        }
    }

    fn commit_value(&mut self) {
        let Some(path) = self.selected.clone() else {
            self.status = "nothing selected".into();
            return;
        };
        match from_snbt(&self.value_input) {
            Ok(new) => {
                let current_type = path.resolve(self.doc.root()).map(|t| t.id());
                // Guard accidental type changes (SNBT can't always express the
                // original width); keep the tag type when the parse widened.
                let new = match current_type {
                    Some(id) if id == new.id() => new,
                    Some(_) => {
                        // Type change is allowed but loudly reported.
                        self.status = format!(
                            "type changed {} → {}",
                            path.resolve(self.doc.root()).unwrap().type_name(),
                            new.type_name()
                        );
                        new
                    }
                    None => new,
                };
                match self
                    .undo
                    .push_set_coalescing(&path, new, self.doc.root_mut())
                {
                    Ok(()) => {
                        if self.status.is_empty() {
                            self.status = "value set".into();
                        }
                        self.rebuild();
                    }
                    Err(e) => self.status = format!("{e}"),
                }
            }
            Err(e) => self.status = format!("invalid SNBT: {e}"),
        }
    }

    fn commit_rename(&mut self) {
        let Some(path) = self.selected.clone() else { return };
        let Some(omt_nbt::PathSeg::Key(old_key)) = path.last().cloned() else {
            self.status = "only compound children can be renamed".into();
            return;
        };
        let new_key = self.name_input.trim().to_string();
        if new_key.is_empty() {
            self.status = "name cannot be empty".into();
            return;
        }
        let parent = path.parent();
        let result = self.undo.edit(self.doc.root_mut(), |root| {
            EditOp::prepare_rename(root, &parent, &old_key, &new_key)
        });
        match result {
            Ok(()) => {
                self.status = format!("renamed to {new_key:?}");
                self.selected = Some(parent.push_key(new_key));
                self.sync_inputs_to_selection();
                self.rebuild();
            }
            Err(e) => self.status = format!("{e}"),
        }
    }

    fn commit_add_child(&mut self) {
        let Some(path) = self.selected.clone().filter(|p| !p.is_root()) else {
            self.status = "select a compound first".into();
            return;
        };
        if !matches!(path.resolve(self.doc.root()), Some(Tag::Compound(_))) {
            self.status = "selected tag is not a compound".into();
            return;
        }
        let key = self.add_name_input.trim().to_string();
        if key.is_empty() {
            self.status = "new tag name required".into();
            return;
        }
        let value = match from_snbt(&self.add_value_input) {
            Ok(v) if !self.add_value_input.trim().is_empty() => v,
            Ok(_) => Tag::Byte(0),
            Err(e) => {
                self.status = format!("invalid SNBT value: {e}");
                return;
            }
        };
        let result = self.undo.edit(self.doc.root_mut(), |root| {
            EditOp::prepare_insert_child(root, &path, &key, value)
        });
        match result {
            Ok(()) => {
                self.status = format!("added {key:?}");
                self.expanded.insert(path.to_string());
                self.add_name_input.clear();
                self.add_value_input.clear();
                self.rebuild();
            }
            Err(e) => self.status = format!("{e}"),
        }
    }

    fn commit_add_index(&mut self) {
        let Some(path) = self.selected.clone() else { return };
        if !matches!(path.resolve(self.doc.root()), Some(Tag::List(_))) {
            self.status = "selected tag is not a list".into();
            return;
        }
        let value = match from_snbt(&self.add_value_input) {
            Ok(v) if !self.add_value_input.trim().is_empty() => v,
            Ok(_) => Tag::Byte(0),
            Err(e) => {
                self.status = format!("invalid SNBT value: {e}");
                return;
            }
        };
        let result = self.undo.edit(self.doc.root_mut(), |root| {
            EditOp::prepare_insert_index(root, &path, None, value)
        });
        match result {
            Ok(()) => {
                self.status = "appended list element".into();
                self.expanded.insert(path.to_string());
                self.add_value_input.clear();
                self.rebuild();
            }
            Err(e) => self.status = format!("{e}"),
        }
    }

    fn delete_selected(&mut self) {
        let Some(path) = self.selected.clone() else { return };
        if path.is_root() {
            self.status = "the root tag cannot be deleted".into();
            return;
        }
        let parent = path.parent();
        let child = match path.last().cloned() {
            Some(omt_nbt::PathSeg::Key(k)) => ChildId::Key(k),
            Some(omt_nbt::PathSeg::Index(i)) => ChildId::Index(i),
            None => return,
        };
        let label = path.to_string();
        let result = self.undo.edit(self.doc.root_mut(), |root| {
            EditOp::prepare_remove(root, &parent, &child)
        });
        match result {
            Ok(()) => {
                self.status = format!("deleted {label}");
                self.selected = Some(parent);
                self.sync_inputs_to_selection();
                self.rebuild();
            }
            Err(e) => self.status = format!("{e}"),
        }
    }

    fn move_selected(&mut self, delta: i32) {
        let Some(path) = self.selected.clone() else { return };
        let parent = path.parent();
        let Some(omt_nbt::PathSeg::Index(i)) = path.last().cloned() else {
            self.status = "move up/down applies to list elements".into();
            return;
        };
        let target = i as i32 + delta;
        if target < 0 {
            self.status = "already first".into();
            return;
        }
        let to = target as usize;
        // Remove then insert at adjusted index.
        let result = self.undo.edit(self.doc.root_mut(), |root| {
            EditOp::prepare_move(
                root,
                &parent,
                &ChildId::Index(i),
                &parent,
                "",
                Some(if delta < 0 { to } else { to + 1 }), // after removal the element shifts
            )
        });
        match result {
            Ok(()) => {
                self.status = if delta < 0 { "moved up".into() } else { "moved down".into() };
                self.selected = Some(parent.push_index(to));
                self.rebuild();
            }
            Err(e) => self.status = format!("{e}"),
        }
    }

    fn apply_import(&mut self) {
        match from_snbt(&self.snbt_import) {
            Ok(tag) => {
                let target_parent = self
                    .selected
                    .clone()
                    .filter(|p| matches!(p.resolve(self.doc.root()), Some(Tag::Compound(_))))
                    .unwrap_or_default();
                let key = format!("imported_{}", self.undo.applied_ops().len() + 1);
                let result = self.undo.edit(self.doc.root_mut(), |root| {
                    EditOp::prepare_insert_child(root, &target_parent, &key, tag.clone())
                });
                match result {
                    Ok(()) => {
                        self.status = format!("imported SNBT as {key:?}");
                        self.expanded.insert(target_parent.to_string());
                        self.rebuild();
                    }
                    Err(e) => self.status = format!("{e}"),
                }
            }
            Err(e) => self.status = format!("invalid SNBT: {e}"),
        }
    }

    fn save(&mut self) {
        if self.sink != Sink::File {
            match self.sink_bytes() {
                Ok(_) => {
                    self.pending_save = true;
                    self.status = "saving into the world workspace…".into();
                }
                Err(e) => self.status = format!("save aborted — document does not serialize: {e}"),
            }
            return;
        }
        // Re-serialize first: a validation failure must not touch the disk.
        match self.doc.to_bytes() {
            Ok(_) => match self.doc.save() {
                Ok(backup) => {
                    self.saved_ops = self.undo.applied_ops().len();
                    self.last_backup = backup.clone();
                    self.status = match backup {
                        Some(p) => format!("saved (backup at {})", p.display()),
                        None => "saved".to_string(),
                    };
                }
                Err(e) => self.status = format!("save FAILED — original untouched: {e}"),
            },
            Err(e) => self.status = format!("save aborted — document does not serialize: {e}"),
        }
    }

    fn run_search(&mut self) {
        self.search_hits.clear();
        self.search_cursor = 0;
        let q = self.search_input.trim();
        if q.is_empty() {
            return;
        }
        let root = self.doc.root();
        collect_hits(root, q, &mut TagPath::root(), &mut self.search_hits);
        collect_hits(root, q, &mut TagPath::root(), &mut self.search_hits);
        self.status = if self.search_hits.is_empty() {
            format!("no matches for {q:?}")
        } else {
            format!("{} matches for {q:?}", self.search_hits.len())
        };
    }

    fn reveal(&mut self, path: &TagPath) {
        // Expand every ancestor.
        for i in 1..=path.0.len() {
            let prefix = TagPath(path.0[..i].to_vec());
            self.expanded.insert(prefix.to_string());
        }
        self.selected = Some(path.clone());
        self.sync_inputs_to_selection();
    }

    pub(crate) fn rebuild(&mut self) {
        self.refresh_extensions();
        let changes: HashMap<String, ChangeKind> = self
            .undo
            .applied_ops()
            .iter()
            .map(|op| {
                let kind = if op.is_addition() { ChangeKind::Added } else { ChangeKind::Edited };
                (op.touched_path().to_string(), kind)
            })
            .collect();
        let current_hit = self.search_hits.get(self.search_cursor).map(|p| p.to_string());

        let mut rows = Vec::new();
        let mut truncated = false;
        let selected_str = self.selected.as_ref().map(|p| p.to_string());

        /// Shared state threaded through the recursive row builder.
        struct Ctx<'a> {
            expanded: &'a HashSet<String>,
            changes: &'a HashMap<String, ChangeKind>,
            current_hit: &'a Option<String>,
            selected_str: &'a Option<String>,
            ext_styles: &'a HashMap<String, omt_script::RowStyle>,
            rows: &'a mut Vec<Row>,
            truncated: &'a mut bool,
        }

        fn build(tag: &Tag, path: &mut TagPath, label: String, depth: usize, ctx: &mut Ctx<'_>) {
            if ctx.rows.len() >= MAX_ROWS {
                *ctx.truncated = true;
                return;
            }
            let path_str = path.to_string();
            let (is_container, child_count) = match tag {
                Tag::Compound(entries) => (true, entries.len()),
                Tag::List(items) => (true, items.len()),
                _ => (false, 0),
            };
            let value_snbt = if is_container { None } else { Some(snbt::to_snbt(tag)) };
            ctx.rows.push(Row {
                path: path.clone(),
                depth,
                label,
                type_name: tag.type_name(),
                value_snbt,
                is_container,
                child_count,
                expanded: is_container && ctx.expanded.contains(&path_str),
                changed: ctx.changes.get(&path_str).copied(),
                search_hit: ctx.current_hit.as_deref() == Some(path_str.as_str()),
                selected: ctx.selected_str.as_deref() == Some(path_str.as_str()),
                extra: ctx.ext_styles.get(&path_str).cloned(),
            });
            if !is_container || !ctx.expanded.contains(&path_str) {
                return;
            }
            match tag {
                Tag::Compound(entries) => {
                    for (k, v) in entries {
                        path.push_key_mut(k.clone());
                        build(v, path, k.clone(), depth + 1, ctx);
                        path.pop();
                    }
                }
                Tag::List(items) => {
                    for (i, v) in items.iter().enumerate() {
                        path.push_index_mut(i);
                        build(v, path, format!("[{i}]"), depth + 1, ctx);
                        path.pop();
                    }
                }
                _ => {}
            }
        }

        let mut path = TagPath::root();
        let mut ctx = Ctx {
            expanded: &self.expanded,
            changes: &changes,
            current_hit: &current_hit,
            selected_str: &selected_str,
            ext_styles: &self.ext_styles,
            rows: &mut rows,
            truncated: &mut truncated,
        };
        build(self.doc.root(), &mut path, "(root)".into(), 0, &mut ctx);
        self.rows = rows;
        self.truncated = truncated;
    }
}

fn path_of(s: &str) -> TagPath {
    use std::str::FromStr;
    TagPath::from_str(s).unwrap_or_default()
}

