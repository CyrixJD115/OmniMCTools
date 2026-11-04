//! Rendering for one NBT editor tab (pure view — no domain logic).

use iced::widget::{button, column, row, scrollable, text, text_input};
use iced::{Color, Element, Length};

use crate::editor::{ChangeKind, EditorMsg, EditorTab};
use crate::{Message, ACCENT, ADDED, EDITED, HIT, MUTED};

pub enum Id {
    Search,
}

impl Id {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Search => "nbt-search",
        }
    }
}

pub fn search_id() -> iced::widget::Id {
    iced::widget::Id::new(Id::Search.as_str())
}

pub fn view(tab: &EditorTab, tab_index: usize) -> Element<'_, Message> {
    let msg = move |m: EditorMsg| Message::EditorTab(tab_index, m);

    let dirty = if tab.is_dirty() { "●" } else { " " };
    let title = row![
        text(format!("{} {}", dirty, tab.title)).size(14),
        text(tab.doc.format.describe()).size(12).color(MUTED),
    ]
    .spacing(10);

    let toolbar = row![
        button("Undo")
            .on_press_maybe(tab.can_undo().then(|| msg(EditorMsg::Undo))),
        button("Redo")
            .on_press_maybe(tab.can_redo().then(|| msg(EditorMsg::Redo))),
        button("Save").on_press(msg(EditorMsg::Save)),
        title,
    ]
    .spacing(8);

    let search_row = row![
        text("Find").size(12),
        text_input("name or value…", &tab.search_input)
            .id(search_id())
            .on_input(move |q| Message::EditorTab(tab_index, EditorMsg::SearchInput(q)))
            .size(13)
            .width(240),
        button("Next ▸").on_press(msg(EditorMsg::SearchNext)),
        text(format!("{}/{}", tab.search_cursor() + 1, tab.search_hit_count().max(1)))
            .size(12)
            .color(MUTED),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    // ---- tree ---------------------------------------------------------------
    let mut tree = iced::widget::Column::new().spacing(1);
    for r in &tab.rows {
        tree = tree.push(row_line(r, tab_index));
    }
    if tab.truncated {
        tree = tree.push(
            text(format!("… showing first {} rows — collapse branches to see more", tab.rows.len()))
                .size(12)
                .color(MUTED),
        );
    }

    // ---- selection panel ----------------------------------------------------
    let panel = selection_panel(tab, tab_index);

    column![
        toolbar,
        search_row,
        iced::widget::rule::horizontal(1.0),
        row![
            scrollable(tree).height(Length::Fill).width(Length::FillPortion(3)),
            scrollable(panel).height(Length::Fill).width(Length::FillPortion(2)),
        ],
        iced::widget::rule::horizontal(1.0),
        text(tab.status.clone()).size(12).color(MUTED),
    ]
    .spacing(6)
    .padding(8)
    .into()
}

fn row_line(r: &crate::editor::Row, tab_index: usize) -> Element<'_, Message> {
    let indent = "  ".repeat(r.depth);
    let marker = if r.is_container {
        if r.expanded { "▾" } else { "▸" }
    } else {
        " "
    };
    let name_color = match r.changed {
        Some(ChangeKind::Added) => ADDED,
        Some(ChangeKind::Edited) => EDITED,
        None if r.search_hit => HIT,
        None => Color::WHITE,
    };
    let (name, action) = if r.is_container {
        (
            format!("{marker} {indent}{} ({})", r.label, r.child_count),
            if r.expanded { EditorMsg::ToggleExpand(r.path.to_string()) } else { EditorMsg::ToggleExpand(r.path.to_string()) },
        )
    } else {
        (format!(" {marker} {indent}{}", r.label), EditorMsg::Select(r.path.to_string()))
    };
    let mut line = row![
        button(text(name).color(name_color).size(13))
            .on_press(Message::EditorTab(tab_index, action))
            .padding(2),
        text(r.type_name).size(11).color(MUTED),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);
    if let Some(v) = &r.value_snbt {
        let shown: String = v.chars().take(80).collect();
        let more = if v.len() > 80 { "…" } else { "" };
        let val_color = if r.search_hit { HIT } else { MUTED };
        line = line.push(text(format!("{shown}{more}")).size(12).color(val_color));
    }
    if r.selected {
        line = line.push(text(" ◂").size(12).color(ACCENT));
    }
    line.into()
}

fn selection_panel(tab: &EditorTab, tab_index: usize) -> Element<'_, Message> {
    let msg = move |m: EditorMsg| Message::EditorTab(tab_index, m);
    let mut panel = column![text("Selected tag").size(14)].spacing(8);

    match tab.selected_row() {
        None => {
            panel = panel.push(text("Click a row to select it.").size(12).color(MUTED));
        }
        Some(r) => {
            panel = panel.push(
                row![
                    text(r.path.to_string()).size(12).color(MUTED),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );

            // Inline value editing for scalars/arrays.
            if let Some(current) = &r.value_snbt {
                let _ = current;
                panel = panel.push(text("Value (SNBT — edit and press Enter)").size(12));
                panel = panel.push(
                    text_input("e.g. 42, \"text\", 1.5f, [I; 1, 2]", &tab.value_input)
                        .on_input(move |t| Message::EditorTab(tab_index, EditorMsg::ValueInput(t)))
                        .on_submit(msg(EditorMsg::CommitValue))
                        .size(13),
                );
                panel = panel.push(
                    row![
                        button("Apply value").on_press(msg(EditorMsg::CommitValue)),
                        button("Copy SNBT").on_press(msg(EditorMsg::CopySelected)),
                        button("Delete").on_press(msg(EditorMsg::DeleteSelected)),
                    ]
                    .spacing(6),
                );
            } else {
                // Container: rename (keys only), children controls.
                let is_key = matches!(r.path.last(), Some(omt_nbt::PathSeg::Key(_)));
                if is_key && !r.path.is_root() {
                    panel = panel.push(text("Rename key").size(12));
                    panel = panel.push(
                        text_input("new name", &tab.name_input)
                            .on_input(move |t| Message::EditorTab(tab_index, EditorMsg::NameInput(t)))
                            .on_submit(msg(EditorMsg::CommitRename))
                            .size(13),
                    );
                    panel = panel.push(button("Rename").on_press(msg(EditorMsg::CommitRename)));
                }
                panel = panel.push(
                    row![
                        button("Copy SNBT").on_press(msg(EditorMsg::CopySelected)),
                        button("Move ▲").on_press_maybe((!r.path.is_root()).then(|| msg(EditorMsg::MoveUp))),
                        button("Move ▼").on_press_maybe((!r.path.is_root()).then(|| msg(EditorMsg::MoveDown))),
                        button("Delete").on_press_maybe(
                            (!r.path.is_root()).then(|| msg(EditorMsg::DeleteSelected))
                        ),
                    ]
                    .spacing(6),
                );
            }

            // Add-child controls when a container is selected.
            if r.is_container {
                let is_compound = matches!(tab.doc.root().resolve_path(&r.path), Some(omt_nbt::Tag::Compound(_)));
                panel = panel.push(iced::widget::rule::horizontal(1.0));
                panel = panel.push(text("Add tag").size(12));
                if is_compound {
                    panel = panel.push(
                        text_input("new tag name", &tab.add_name_input)
                            .on_input(move |t| Message::EditorTab(tab_index, EditorMsg::AddNameInput(t)))
                            .size(13),
                    );
                }
                panel = panel.push(
                    text_input("value SNBT (empty = 0b)", &tab.add_value_input)
                        .on_input(move |t| Message::EditorTab(tab_index, EditorMsg::AddValueInput(t)))
                        .size(13),
                );
                if is_compound {
                    panel = panel.push(button("Add child").on_press(msg(EditorMsg::CommitAddChild)));
                } else {
                    panel = panel.push(button("Append element").on_press(msg(EditorMsg::CommitAddIndex)));
                }
            }
        }
    }

    // SNBT import (paste target) — always available.
    panel = panel.push(iced::widget::rule::horizontal(1.0));
    panel = panel.push(text("Paste SNBT").size(12));
    panel = panel.push(
        text_input("paste SNBT here…", &tab.snbt_import)
            .on_input(move |t| Message::EditorTab(tab_index, EditorMsg::SnbtImportInput(t)))
            .size(13),
    );
    panel = panel.push(
        row![
            button("Import into selected compound").on_press(msg(EditorMsg::ApplyImport)),
            button("Paste from clipboard").on_press(Message::PasteRequested(tab_index)),
        ]
        .spacing(6),
    );
    panel.into()
}

// Local trait-let so the panel can inspect the selected container type.
trait ResolvePath {
    fn resolve_path(&self, path: &omt_nbt::TagPath) -> Option<&omt_nbt::Tag>;
}

impl ResolvePath for omt_nbt::Tag {
    fn resolve_path(&self, path: &omt_nbt::TagPath) -> Option<&omt_nbt::Tag> {
        path.resolve(self)
    }
}
