//! Rendering for one NBT editor tab (pure view — no domain logic).

use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Color, Element, Length};

use crate::dictionary;
use crate::editor::{ChangeKind, EditorMsg, EditorTab};
use crate::style::{self, Btn};


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

/// Wraps an editor message into the parent's message type.
pub type Wrap<'a, M> = std::rc::Rc<dyn Fn(EditorMsg) -> M + 'a>;

pub fn view<'a, M: Clone + 'a>(tab: &'a EditorTab, wrap: Wrap<'a, M>) -> Element<'a, M> {
    let w = wrap.clone();
    let msg = move |m: EditorMsg| w(m);

    let dirty = if tab.is_dirty() { "● unsaved" } else { "saved" };
    let dirty_color = if tab.is_dirty() { style::warn() } else { style::success() };
    let title = row![
        text(tab.title.clone()).size(14),
        text(dirty).size(11).color(dirty_color),
        text(tab.doc.format.describe()).size(11).color(style::muted()),
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
        text("Find").size(12).color(style::muted()),
        text_input("name or value…", &tab.search_input)
            .id(search_id())
            .on_input({ let w = wrap.clone(); move |q| w(EditorMsg::SearchInput(q)) })
            .size(13)
            .width(260)
            .style(style::input()),
        tool_button("Next ▸", Btn::Ghost, Some(msg(EditorMsg::SearchNext))),
        text(format!("{}/{}", tab.search_cursor() + 1, tab.search_hit_count().max(1)))
            .size(12)
            .color(style::muted()),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);


    // ---- tree ---------------------------------------------------------------
    let mut tree = iced::widget::Column::new().spacing(1);
    for r in &tab.rows {
        tree = tree.push(row_line(r, wrap.clone()));
    }
    if tab.truncated {
        tree = tree.push(
            text(format!("… showing first {} rows — collapse branches to see more", tab.rows.len()))
                .size(12)
                .color(style::muted()),
        );
    }

    // ---- selection panel ----------------------------------------------------
    let panel = selection_panel(tab, wrap.clone());

    column![
        toolbar,
        search_row,
        iced::widget::rule::horizontal(1.0),
        row![
            scrollable(tree).height(Length::Fill).width(Length::FillPortion(3)),
            scrollable(panel).height(Length::Fill).width(Length::FillPortion(2)),
        ],
        iced::widget::rule::horizontal(1.0),
        text(tab.status.clone()).size(12).color(style::muted()),
        text(tab.ext_status.clone()).size(11).color(style::hint()),
    ]
    .spacing(6)
    .padding(8)
    .into()
}

fn tool_button<M: Clone>(label: &str, kind: Btn, on_press: Option<M>) -> button::Button<'_, M> {
    let b = button(text(label.to_string()).size(13)).style(style::button_style(kind));
    match on_press {
        Some(m) => b.on_press(m),
        None => b,
    }
    .padding([5, 10])
}

fn row_line<'a, M: Clone + 'a>(r: &'a crate::editor::Row, wrap: Wrap<'a, M>) -> Element<'a, M> {
    let indent = "  ".repeat(r.depth);
    let marker = if r.is_container {
        if r.expanded { "▾" } else { "▸" }
    } else {
        " "
    };
    let ext_color = r.extra_color();
    let name_color = match r.changed {
        Some(ChangeKind::Added) => style::success(),
        Some(ChangeKind::Edited) => style::warn(),
        None if r.search_hit => style::info(),
        None => ext_color.unwrap_or(Color::WHITE),
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
            .on_press(wrap(action))
            .style(style::button_style(Btn::Ghost))
            .padding(2),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    // Friendly label from the dictionary (raw key stays first — editing
    // identity is preserved; known names gain a readable title).
    if let Some(entry) = dictionary::lookup(&r.label) {
        line = line.push(text(entry.label).size(12).color(style::fg()));
    }
    line = line.push(text(r.type_name).size(11).color(style::muted()));
    if let Some(v) = &r.value_snbt {
        let shown: String = v.chars().take(80).collect();
        let more = if v.len() > 80 { "…" } else { "" };
        let val_color = if r.search_hit { style::info() } else { style::muted() };
        line = line.push(text(format!("{shown}{more}")).size(12).color(val_color));
    }
    if let Some(decoded) = r.value_snbt.as_deref().and_then(|v| dictionary::decode(&r.label, v)) {
        line = line.push(text(format!("= {decoded}")).size(12).color(style::accent()));
    }
    // Extension-provided annotation (e.g. "day 3, 09:30 in-game").
    if let Some(t) = r.extra_text() {
        line = line.push(
            container(text(format!("◆ {t}")).size(12).color(ext_color.unwrap_or(style::accent())))
                .padding([1, 6])
                .style(style::card()),
        );
    }
    if r.selected {
        line = line.push(text(" ◂").size(12).color(style::accent()));
    }
    line.into()
}

fn selection_panel<'a, M: Clone + 'a>(tab: &'a EditorTab, wrap: Wrap<'a, M>) -> Element<'a, M> {
    let w = wrap.clone();
    let msg = move |m: EditorMsg| w(m);
    let mut panel = column![text("Selected tag").size(14)].spacing(8);

    match tab.selected_row() {
        None => {
            panel = panel.push(text("Click a row to select it.").size(12).color(style::muted()));
        }
        Some(r) => {
            // Heading: friendly label when known, raw path underneath.
            if let Some(entry) = dictionary::lookup(&r.label) {
                panel = panel.push(text(entry.label).size(14));
            }
            panel = panel.push(
                row![
                    text(r.path.to_string()).size(11).color(style::muted()),
                ]
                .spacing(8),
            );
            if let Some(entry) = dictionary::lookup(&r.label) {
                panel = panel.push(
                    container(text(entry.desc).size(12).color(style::muted()))
                        .style(style::card())
                        .padding(8)
                        .width(Length::Fill),
                );
            }
            // Extension-provided description (below the dictionary entry).
            if let Some(desc) = r.extra_description() {
                panel = panel.push(
                    container(
                        column![
                            text("Extension").size(11).color(style::muted()),
                            text(desc).size(12).color(r.extra_color().unwrap_or(style::accent())),
                        ]
                        .spacing(2),
                    )
                    .style(style::card())
                    .padding(8)
                    .width(Length::Fill),
                );
            }
            if let Some(decoded) = r.value_snbt.as_deref().and_then(|v| dictionary::decode(&r.label, v)) {
                panel = panel.push(
                    row![text("Meaning: ").size(12).color(style::muted()), text(decoded).size(12).color(style::accent())]
                        .spacing(4),
                );
            }

            // Inline value editing for scalars/arrays.
            if let Some(current) = &r.value_snbt {
                let _ = current;
                panel = panel.push(text("Value (SNBT — edit and press Enter)").size(12));
                panel = panel.push(
                    text_input("e.g. 42, \"text\", 1.5f, [I; 1, 2]", &tab.value_input)
                        .on_input({ let w = wrap.clone(); move |t| w(EditorMsg::ValueInput(t)) })
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
                            .on_input({ let w = wrap.clone(); move |t| w(EditorMsg::NameInput(t)) })
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
                        move_button("▲", EditorMsg::MoveUp, !r.path.is_root(), &msg),
                        move_button("▼", EditorMsg::MoveDown, !r.path.is_root(), &msg),
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
                            .on_input({ let w = wrap.clone(); move |t| w(EditorMsg::AddNameInput(t)) })
                            .size(13)
                            .style(style::input()),
                    );
                }
                panel = panel.push(
                    text_input("value SNBT (empty = 0b)", &tab.add_value_input)
                        .on_input({ let w = wrap.clone(); move |t| w(EditorMsg::AddValueInput(t)) })
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
            .on_input({ let w = wrap.clone(); move |t| w(EditorMsg::SnbtImportInput(t)) })
            .size(13)
            .style(style::input()),
    );
    panel = panel.push(
        row![
            action_button("Import into selected compound", Btn::Standard, msg(EditorMsg::ApplyImport)),
            action_button("Paste from clipboard", Btn::Standard, msg(EditorMsg::RequestPaste)),
        ]
        .spacing(6),
    );
    panel.into()
}

fn action_button<M: Clone>(label: &str, kind: Btn, on_press: M) -> button::Button<'_, M> {
    button(text(label.to_string()).size(12))
        .on_press(on_press)
        .style(style::button_style(kind))
        .padding([5, 10])
}

fn move_button<M: Clone>(
    label: &str,
    m: EditorMsg,
    enabled: bool,
    msg: impl Fn(EditorMsg) -> M,
) -> button::Button<'_, M> {
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
