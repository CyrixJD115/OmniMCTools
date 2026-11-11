//! Rendering for one NBT editor tab (pure view — no domain logic).

use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Color, Element, Length};

use crate::dictionary;
use crate::editor::{ChangeKind, EditorMsg, EditorTab};
use crate::style::{self, Btn};
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

    let dirty = if tab.is_dirty() { "● unsaved" } else { "saved" };
    let dirty_color = if tab.is_dirty() { EDITED } else { style::SUCCESS };
    let title = row![
        text(tab.title.clone()).size(14),
        text(dirty).size(11).color(dirty_color),
        text(tab.doc.format.describe()).size(11).color(MUTED),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    let toolbar = container(
        row![
            tool_button("↶ Undo", Btn::Standard, tab.can_undo().then(|| msg(EditorMsg::Undo))),
            tool_button("↷ Redo", Btn::Standard, tab.can_redo().then(|| msg(EditorMsg::Redo))),
            tool_button("Save", Btn::Confirm, Some(msg(EditorMsg::Save))),
            space::horizontal(),
            title,
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    )
    .style(style::card())
    .padding([6, 10])
    .width(Length::Fill);

    let search_row = row![
        text("Find").size(12).color(MUTED),
        text_input("name or value…", &tab.search_input)
            .id(search_id())
            .on_input(move |q| Message::EditorTab(tab_index, EditorMsg::SearchInput(q)))
            .size(13)
            .width(260)
            .style(style::input()),
        tool_button("Next ▸", Btn::Ghost, Some(msg(EditorMsg::SearchNext))),
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

fn tool_button(label: &str, kind: Btn, on_press: Option<Message>) -> button::Button<'_, Message> {
    let b = button(text(label.to_string()).size(13)).style(style::button_style(kind));
    match on_press {
        Some(m) => b.on_press(m),
        None => b,
    }
    .padding([5, 10])
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
            EditorMsg::ToggleExpand(r.path.to_string()),
        )
    } else {
        (format!(" {marker} {indent}{}", r.label), EditorMsg::Select(r.path.to_string()))
    };
    let mut line = row![
        button(text(name).color(name_color).size(13))
            .on_press(Message::EditorTab(tab_index, action))
            .style(style::button_style(Btn::Ghost))
            .padding(2),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    // Friendly label from the dictionary (raw key stays first — editing
    // identity is preserved; known names gain a readable title).
    if let Some(entry) = dictionary::lookup(&r.label) {
        line = line.push(text(entry.label).size(12).color(style::FG));
    }
    line = line.push(text(r.type_name).size(11).color(MUTED));
    if let Some(v) = &r.value_snbt {
        let shown: String = v.chars().take(80).collect();
        let more = if v.len() > 80 { "…" } else { "" };
        let val_color = if r.search_hit { HIT } else { MUTED };
        line = line.push(text(format!("{shown}{more}")).size(12).color(val_color));
    }
    if let Some(decoded) = r.value_snbt.as_deref().and_then(|v| dictionary::decode(&r.label, v)) {
        line = line.push(text(format!("= {decoded}")).size(12).color(ACCENT));
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
            // Heading: friendly label when known, raw path underneath.
            if let Some(entry) = dictionary::lookup(&r.label) {
                panel = panel.push(text(entry.label).size(14));
            }
            panel = panel.push(
                row![
                    text(r.path.to_string()).size(11).color(MUTED),
                ]
                .spacing(8),
            );
            if let Some(entry) = dictionary::lookup(&r.label) {
                panel = panel.push(
                    container(text(entry.desc).size(12).color(MUTED))
                        .style(style::card())
                        .padding(8)
                        .width(Length::Fill),
                );
            }
            if let Some(decoded) = r.value_snbt.as_deref().and_then(|v| dictionary::decode(&r.label, v)) {
                panel = panel.push(
                    row![text("Meaning: ").size(12).color(MUTED), text(decoded).size(12).color(ACCENT)]
                        .spacing(4),
                );
            }

            // Inline value editing for scalars/arrays.
            if let Some(current) = &r.value_snbt {
                let _ = current;
                panel = panel.push(text("Value (SNBT — edit and press Enter)").size(12));
                panel = panel.push(
                    text_input("e.g. 42, \"text\", 1.5f, [I; 1, 2]", &tab.value_input)
                        .on_input(move |t| Message::EditorTab(tab_index, EditorMsg::ValueInput(t)))
                        .on_submit(msg(EditorMsg::CommitValue))
                        .size(13)
                        .style(style::input()),
                );
                panel = panel.push(
                    row![
                        action_button("Apply value", Btn::Primary, msg(EditorMsg::CommitValue)),
                        action_button("Copy SNBT", Btn::Standard, msg(EditorMsg::CopySelected)),
                        action_button("Delete", Btn::Danger, msg(EditorMsg::DeleteSelected)),
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
                            .size(13)
                            .style(style::input()),
                    );
                    panel =
                        panel.push(action_button("Rename", Btn::Primary, msg(EditorMsg::CommitRename)));
                }
                panel = panel.push(
                    row![
                        action_button("Copy SNBT", Btn::Standard, msg(EditorMsg::CopySelected)),
                        move_button("▲", EditorMsg::MoveUp, !r.path.is_root(), msg),
                        move_button("▼", EditorMsg::MoveDown, !r.path.is_root(), msg),
                        action_button("Delete", Btn::Danger, msg(EditorMsg::DeleteSelected)),
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
                            .size(13)
                            .style(style::input()),
                    );
                }
                panel = panel.push(
                    text_input("value SNBT (empty = 0b)", &tab.add_value_input)
                        .on_input(move |t| Message::EditorTab(tab_index, EditorMsg::AddValueInput(t)))
                        .size(13)
                        .style(style::input()),
                );
                if is_compound {
                    panel = panel.push(action_button("Add child", Btn::Primary, msg(EditorMsg::CommitAddChild)));
                } else {
                    panel =
                        panel.push(action_button("Append element", Btn::Primary, msg(EditorMsg::CommitAddIndex)));
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
            .size(13)
            .style(style::input()),
    );
    panel = panel.push(
        row![
            action_button("Import into selected compound", Btn::Standard, msg(EditorMsg::ApplyImport)),
            action_button("Paste from clipboard", Btn::Standard, Message::PasteRequested(tab_index)),
        ]
        .spacing(6),
    );
    panel.into()
}

fn action_button(label: &str, kind: Btn, on_press: Message) -> button::Button<'_, Message> {
    button(text(label.to_string()).size(12))
        .on_press(on_press)
        .style(style::button_style(kind))
        .padding([5, 10])
}

fn move_button(
    label: &str,
    m: EditorMsg,
    enabled: bool,
    msg: impl Fn(EditorMsg) -> Message,
) -> button::Button<'_, Message> {
    let b = button(text(label.to_string()).size(12)).style(style::button_style(Btn::Standard));
    match enabled {
        true => b.on_press(msg(m)),
        false => b,
    }
    .padding([5, 8])
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
