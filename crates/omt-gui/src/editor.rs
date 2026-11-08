//! NBT editor tab state (M2): document + undo + navigation + search.
//!
//! View-free by design — `editor_view` renders this, `main` routes messages.
//! Every mutation goes through the shared `omt_nbt` edit engine.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use omt_nbt::{ChildId, EditOp, NbtDocument, Tag, TagPath, UndoStack};
use omt_nbt::snbt::{self, from_snbt};

/// Visible-row cap so giant documents (actor NBT, big arrays) stay fluid.
const MAX_ROWS: usize = 1500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Edited,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub path: TagPath,
    pub depth: usize,
    /// Display name (compound key, `[i]` for list items, `""` root label).
    pub label: String,
    pub type_name: &'static str,
    /// SNBT for scalar/array rows (editable); `None` for containers.
    pub value_snbt: Option<String>,
    pub is_container: bool,
    pub child_count: usize,
    pub expanded: bool,
    pub changed: Option<ChangeKind>,
    pub search_hit: bool,
    pub selected: bool,
}

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
    ApplyImport,
}

pub struct EditorTab {
    pub doc: NbtDocument,
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
        let mut tab = Self {
            doc,
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
            status: String::new(),
            last_backup: None,
        };
        tab.rebuild();
        Ok(tab)
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

    pub fn update(&mut self, msg: EditorMsg) -> Vec<iced::Task<Message>> {
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

    fn rebuild(&mut self) {
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
            rows: &mut rows,
            truncated: &mut truncated,
        };
        build(self.doc.root(), &mut path, "(root)".into(), 0, &mut ctx);
        self.rows = rows;
        self.truncated = truncated;
    }
}

fn collect_hits(tag: &Tag, q: &str, path: &mut TagPath, out: &mut Vec<TagPath>) {
    let name_hit = matches!(path.last(), Some(omt_nbt::PathSeg::Key(k)) if k.contains(q));
    let value_hit = match tag {
        Tag::Compound(_) | Tag::List(_) => false,
        scalar => {
            let s = snbt::to_snbt(scalar);
            s.len() <= 4096 && s.contains(q)
        }
    };
    if name_hit || value_hit {
        out.push(path.clone());
    }
    match tag {
        Tag::Compound(entries) => {
            for (k, v) in entries {
                path.push_key_mut(k.clone());
                collect_hits(v, q, path, out);
                path.pop();
            }
        }
        Tag::List(items) => {
            for (i, v) in items.iter().enumerate() {
                path.push_index_mut(i);
                collect_hits(v, q, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn path_of(s: &str) -> TagPath {
    use std::str::FromStr;
    TagPath::from_str(s).unwrap_or_default()
}

use crate::Message;
