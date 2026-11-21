//! Extension **NBT actions** in the NBT editor: the selected tag's panel
//! lists the actions that apply (by tag name / type), running one hands the
//! tag to the action's tool (`ext_tools`), and the tool's edits come back
//! here as one undoable step.

use std::sync::atomic::{AtomicU64, Ordering};

use iced::widget::{button, column, row, text};
use iced::Element;
use omt_nbt::{EditOp, Tag, TagPath};

use crate::editor::{EditorMsg, EditorTab};
use crate::style::{self, Btn};

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A fresh editor tab identity.
pub fn next_uid() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl EditorTab {
    /// Actions that apply to the tag at `path` (named `label`).
    pub fn actions_for(&self, path: &TagPath, label: &str) -> Vec<&omt_extensions::tool::ToolDecl> {
        let Some(tag) = path.resolve(self.doc.root()) else { return Vec::new() };
        self.actions.iter().filter(|a| a.applies(label, tag.id())).collect()
    }

    /// Start an action on the selected tag (the app opens its tool).
    pub(crate) fn run_action(&mut self, key: String) {
        let Some(path) = self.selected.clone() else {
            self.status = "nothing selected".into();
            return;
        };
        let Some(tag) = path.resolve(self.doc.root()).cloned() else { return };
        let name = match path.last() {
            Some(omt_nbt::PathSeg::Key(k)) => k.clone(),
            Some(omt_nbt::PathSeg::Index(i)) => format!("[{i}]"),
            None => self.doc.nbt.name.clone(),
        };
        self.pending_action = Some((key, path, name, tag));
    }

    /// An action edited its tag: replace it (one undo step).
    pub fn apply_action_edit(&mut self, path: &TagPath, new: Tag, who: &str) {
        let r = EditOp::prepare_set(self.doc.root(), path, new).map_err(|e| e.to_string()).and_then(|op| self.undo.push(op, self.doc.root_mut()).map_err(|e| e.to_string()));
        self.status = match r {
            Ok(()) => format!("{who} edited {path} — Ctrl+Z reverts"),
            Err(e) => format!("{who}: edit rejected ({e})"),
        };
        self.rebuild();
    }
}

/// Buttons for the actions that apply to the selected row.
pub fn action_buttons<'a, M: Clone + 'a>(tab: &'a EditorTab, path: &TagPath, label: &str, wrap: impl Fn(EditorMsg) -> M) -> Option<Element<'a, M>> {
    let acts = tab.actions_for(path, label);
    if acts.is_empty() {
        return None;
    }
    let buttons = acts.into_iter().map(|a| button(text(a.name.clone()).size(12)).padding([5, 10]).on_press(wrap(EditorMsg::RunAction(a.key()))).style(style::button_style(Btn::Standard)).into());
    Some(column![text("Extension actions").size(12).color(style::muted()), row(buttons).spacing(6).wrap()].spacing(4).into())
}
