//! Row model for the NBT editor tree + search hit collection.
//!
//! Split out of `editor.rs` (file budget): the row struct, its change-kind
//! marker, and the recursive search used by the editor tab.

use omt_nbt::{Tag, TagPath};
use omt_nbt::EditOp;
use omt_nbt::snbt;

/// Marks rows touched by undoable edits (added vs. edited).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Edited,
}

/// One visible tree row (see `editor.rs::rebuild`).
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
    /// Extension-provided decoration (see `omt-script`), keyed by row path.
    pub extra: Option<omt_script::RowStyle>,
}

impl Row {
    pub fn extra_text(&self) -> Option<&str> {
        self.extra.as_ref()?.extra_text.as_deref()
    }
    pub fn extra_description(&self) -> Option<&str> {
        self.extra.as_ref()?.description.as_deref()
    }
    pub fn extra_color(&self) -> Option<iced::Color> {
        Some(iced::Color::from_rgb8(
            self.extra.as_ref()?.color?.0,
            self.extra.as_ref()?.color?.1,
            self.extra.as_ref()?.color?.2,
        ))
    }
}

/// Collect paths whose key or (bounded) scalar value contains `q`.
pub fn collect_hits(tag: &Tag, q: &str, path: &mut TagPath, out: &mut Vec<TagPath>) {
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

/// Files / keys a document was opened from map to `omt-script` file-type
/// bits (extensions filter on `context.type`).
pub fn file_type_bits(label: &str) -> u32 {
    let l = label.to_ascii_lowercase();
    if l.contains("level.dat") || l.contains("level_old") {
        omt_script::file_type::LEVEL
    } else if l.contains("map_") || l.starts_with("map #") {
        omt_script::file_type::MAP
    } else if l.contains("player") || l.contains("local_player") {
        omt_script::file_type::PLAYER
    } else {
        omt_script::file_type::GENERIC
    }
}


/// Editor-tab extension pass (impl lives here to keep `editor.rs` in the
/// file budget): derive the run context, reuse the cached pass while the
/// tree and extension set are unchanged, apply extension edits through the
/// undo engine once per tree state.
impl crate::editor::EditorTab {
    /// Document context for extension runs: edition from the sink/format,
    /// file-type bits from the title.
    pub(crate) fn script_context(&self) -> omt_script::Context {
        let edition = match &self.sink {
            crate::editor::Sink::JavaChunk { .. } => omt_script::Edition::Java,
            crate::editor::Sink::BedrockKey(_) | crate::editor::Sink::BedrockSequence(_) | crate::editor::Sink::BedrockActors(_) => omt_script::Edition::Bedrock,
            crate::editor::Sink::File | crate::editor::Sink::WorkspaceFile(_) => match self.doc.format {
                omt_nbt::NbtFormat::RawLittleEndian | omt_nbt::NbtFormat::BedrockLevelDat => omt_script::Edition::Bedrock,
                _ => omt_script::Edition::Java,
            },
        };
        omt_script::Context { file_type: file_type_bits(&self.title), edition }
    }


    /// Run enabled extensions over the document (cached per manager
    /// revision + tree content). An extension tree edit is applied through
    /// the undo engine as one `SetValue` at the root — at most once per
    /// tree state: undoing an extension's edit is respected (the same tree
    /// is never re-edited), and a fresh user edit re-arms the pass. A
    /// non-convergent extension that keeps editing an already-edited tree
    /// is skipped with a status note (no loops).
    pub(crate) fn refresh_extensions(&mut self) {
        let Some(mgr) = self.ext.clone() else { return };
        let rev = mgr.revision();
        let root = self.doc.root().clone();
        if let Some((cached_rev, cached_root, styles)) = &self.ext_cache
            && *cached_rev == rev
            && *cached_root == root
        {
            self.ext_styles = styles.clone();
            return;
        }
        let ctx = self.script_context();
        let run = mgr.run(&self.doc.nbt.name, &root, &ctx);
        let mut notes: Vec<String> = Vec::new();
        let cache_root = if let Some(new_root) = run.edited_root {
            if self.ext_edited_input.as_ref() == Some(&root) {
                // The user undid this exact extension edit (or the extension
                // is non-convergent): the same tree state is never re-edited.
                notes.push("extension edit not reapplied to this tree state (undo respected)".into());
                root.clone()
            } else {
                let op = EditOp::prepare_set(&root, &TagPath::root(), new_root.clone());
                match op.map_err(|e| e.to_string()).and_then(|op| self.undo.push(op, self.doc.root_mut()).map_err(|e| e.to_string())) {
                    Ok(()) => {
                        self.ext_edited_input = Some(root.clone());
                        self.status = "an extension modified the document — Ctrl+Z reverts".into();
                    }
                    Err(e) => notes.push(format!("extension edit rejected: {e}")),
                }
                new_root
            }
        } else {
            self.ext_edited_input = None;
            root
        };
        self.ext_cache = Some((rev, cache_root, run.styles.clone()));
        let enabled = mgr.enabled_ids().len();
        let styled = run.styles.len();
        if !run.errors.is_empty() {
            notes.push(format!("{} extension error(s): {}", run.errors.len(), run.errors[0]));
        }
        self.ext_status = match notes.is_empty() {
            true => format!("{enabled} extension(s) · {styled} styled row(s)"),
            false => notes.join("; "),
        };
        self.ext_styles = run.styles;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_type_bits_from_common_names() {
        assert_eq!(file_type_bits("level.dat"), omt_script::file_type::LEVEL);
        assert_eq!(file_type_bits("LevelDat (level.dat)"), omt_script::file_type::LEVEL);
        assert_eq!(file_type_bits("map_0.dat"), omt_script::file_type::MAP);
        assert_eq!(file_type_bits("~local_player"), omt_script::file_type::PLAYER);
        assert_eq!(file_type_bits("actors 3:-5"), omt_script::file_type::GENERIC);
    }
}

#[cfg(test)]
mod extension_tests {
    use crate::editor::{EditorTab, Sink};
    use omt_nbt::{ByteOrder, Nbt, Tag};

    fn level_dat_doc() -> Nbt {
        Nbt {
            name: String::new(),
            tag: Tag::Compound(vec![(
                "Data".into(),
                Tag::Compound(vec![
                    ("Time".into(), Tag::Long(6000)),
                    ("DayTime".into(), Tag::Long(6000)),
                    ("InhabitedTime".into(), Tag::Long(72_000)),
                ]),
            )]),
        }
    }

    fn manager_with_example(tag: &str) -> std::sync::Arc<omt_script::Manager> {
        let root = std::env::temp_dir().join(format!("omt-gui-ext-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mgr = std::sync::Arc::new(omt_script::Manager::open(&root));
        mgr.install_files("ticks-time", &omt_script::example_files(), None).unwrap();
        mgr
    }

    /// The full production path the editor rows render from: Manager →
    /// Lua pass → style cache → Row.extra. Same code `editor_view` draws.
    #[test]
    fn example_extension_decorates_editor_rows() {
        let mgr = manager_with_example("rows");
        let mut tab = EditorTab::from_nbt("level.dat".into(), level_dat_doc(), ByteOrder::Big, Sink::File);
        tab.set_extensions(mgr.clone());
        tab.update(crate::editor::EditorMsg::ToggleExpand(".Data".into()));

        let row = |label: &str| tab.rows.iter().find(|r| r.label == label).cloned().unwrap();
        let time = row("Time");
        assert_eq!(time.extra_text(), Some("day 0, 12:00 in-game"));
        let color = time.extra_color().expect("tinted");
        assert_eq!((color.r * 255.0) as u8, 0x7f);
        assert_eq!((color.g * 255.0) as u8, 0xb0);
        assert_eq!((color.b * 255.0) as u8, 0x69);
        assert!(time.extra_description().unwrap().starts_with("Tick Clock: time"));
        assert_eq!(row("DayTime").extra_text(), Some("day 0, 12:00 in-game"));
        assert_eq!(row("InhabitedTime").extra_text(), Some("1h 00m 00s played"));
        assert!(row("Data").extra_text().is_none());

        // Health note surfaced under the editor status line.
        assert!(tab.ext_status.contains("1 extension(s)"), "{}", tab.ext_status);
        // Document untouched: styling-only pass → no undoable edit recorded.
        assert!(!tab.is_dirty());

        // Disabling the extension clears the decoration on the next pass.
        mgr.set_enabled("ticks-time", false).unwrap();
        tab.rebuild();
        assert!(tab.rows.iter().find(|r| r.label == "Time").unwrap().extra_text().is_none());
        let _ = std::fs::remove_dir_all(mgr.root());
    }

    /// Extension tree edits land as one undoable root SetValue (Ctrl+Z reverts).
    #[test]
    fn extension_edit_is_undoable_and_not_reapplied() {
        let root = std::env::temp_dir().join(format!("omt-gui-ext-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mgr = std::sync::Arc::new(omt_script::Manager::open(&root));
        let main_lua = r#"
local F = {}
F.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
F.recursive = true
function F:recursion(root, current)
  if current.name == "Time" and current.type == TYPE.LONG then
    current:setLong(100)
  end
end
return { F }
"#;
        mgr.install_files(
            "fixer",
            &[
                ("manifest.json".into(), br#"{ "name": "fixer" }"#.to_vec()),
                ("main.lua".into(), main_lua.as_bytes().to_vec()),
            ],
            None,
        )
        .unwrap();

        let mut tab = EditorTab::from_nbt("level.dat".into(), level_dat_doc(), ByteOrder::Big, Sink::File);
        tab.set_extensions(mgr.clone());
        tab.rebuild();
        // The edit was applied to the document.
        assert_eq!(tab.doc.root().get("Data").and_then(|d| d.get("Time")), Some(&Tag::Long(100)));
        assert!(tab.is_dirty(), "extension edit is a real undoable change");
        // Re-running over the edited tree is a cache hit → no second edit, no loop.
        tab.rebuild();
        assert_eq!(tab.doc.root().get("Data").and_then(|d| d.get("Time")), Some(&Tag::Long(100)));
        // Undo reverts the extension's edit…
        tab.update(crate::editor::EditorMsg::Undo);
        assert_eq!(tab.doc.root().get("Data").and_then(|d| d.get("Time")), Some(&Tag::Long(6000)));
        // …and the undo is respected: the same tree state is not re-edited.
        tab.rebuild();
        assert_eq!(tab.doc.root().get("Data").and_then(|d| d.get("Time")), Some(&Tag::Long(6000)));
        assert!(tab.ext_status.contains("undo respected"), "{}", tab.ext_status);
        // A fresh user edit re-arms the pass: the changed tree gets one more
        // (convergent) extension edit.
        tab.selected = Some(omt_nbt::TagPath::root().push_key("Data").push_key("DayTime"));
        tab.value_input = "7200L".into();
        tab.update(crate::editor::EditorMsg::CommitValue);
        assert_eq!(tab.doc.root().get("Data").and_then(|d| d.get("DayTime")), Some(&Tag::Long(7200)));
        tab.rebuild();
        assert_eq!(tab.doc.root().get("Data").and_then(|d| d.get("Time")), Some(&Tag::Long(100)));
        let _ = std::fs::remove_dir_all(mgr.root());
    }

    /// Context derivation: sink + title → edition + FILE_TYPE bits.
    #[test]
    fn script_context_matches_sink_and_title() {
        let mk = |title: &str, order: ByteOrder| {
            let mut t = EditorTab::from_nbt(title.into(), level_dat_doc(), order, Sink::File);
            t.rebuild();
            t.script_context()
        };
        assert_eq!(mk("level.dat", ByteOrder::Big), omt_script::Context { file_type: omt_script::file_type::LEVEL, edition: omt_script::Edition::Java });
        assert_eq!(mk("level.dat", ByteOrder::Little), omt_script::Context { file_type: omt_script::file_type::LEVEL, edition: omt_script::Edition::Bedrock });
        assert_eq!(mk("map_42.dat", ByteOrder::Big), omt_script::Context { file_type: omt_script::file_type::MAP, edition: omt_script::Edition::Java });
        let mut t = EditorTab::from_nbt("Chunk 3:5".into(), level_dat_doc(), ByteOrder::Big, Sink::JavaChunk { region: "/nonexistent".into(), x: 3, z: 5 });
        t.rebuild();
        assert_eq!(t.script_context(), omt_script::Context { file_type: omt_script::file_type::GENERIC, edition: omt_script::Edition::Java });
    }
}
