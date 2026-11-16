//! Extensions page: install (zip / built-in example), enable/disable,
//! reorder, remove. The manager itself lives in `omt-script`; this module is
//! the page's view + message handling, routed from `main`.

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Element, Length};
use omt_script::Manager;

use crate::style::{self, Btn};
use crate::Message;

#[derive(Debug, Clone)]
pub enum ExtMsg {
    Back,
    PickZip,
    ZipPicked(Option<std::path::PathBuf>),
    InstallExample,
    Toggle(String),
    Up(String),
    Down(String),
    Remove(String),
    /// After any mutation: existing editor tabs re-run via manager revision.
    Reloaded,
}

/// Applies a message to the shared manager; returns a status line (and
/// whether the list changed, so the caller can refresh).
pub fn handle(mgr: &Manager, msg: &ExtMsg) -> (String, bool) {
    match msg {
        ExtMsg::Back => (String::new(), false),
        ExtMsg::PickZip | ExtMsg::ZipPicked(None) => (String::new(), false),
        ExtMsg::ZipPicked(Some(path)) => match mgr.install_zip(path) {
            Ok(id) => (format!("installed extension {id:?} from {}", path.display()), true),
            Err(e) => (format!("install failed: {e}"), false),
        },
        ExtMsg::InstallExample => {
            let files = omt_script::example_files();
            match mgr.install_files("ticks-time", &files, None).and_then(|()| mgr.set_enabled("ticks-time", true)) {
                Ok(()) => ("installed the built-in example: Tick Clock".into(), true),
                Err(e) => (format!("could not install the example: {e}"), false),
            }
        }
        ExtMsg::Toggle(id) => {
            let next = !mgr.is_enabled(id);
            match mgr.set_enabled(id, next) {
                Ok(()) => {
                    let verb = if next { "enabled" } else { "disabled" };
                    (format!("{verb} {id}"), true)
                }
                Err(e) => (format!("{e}"), false),
            }
        }
        ExtMsg::Up(id) => match mgr.reorder(id, -1) {
            Ok(()) => (String::new(), true),
            Err(e) => (format!("{e}"), false),
        },
        ExtMsg::Down(id) => match mgr.reorder(id, 1) {
            Ok(()) => (String::new(), true),
            Err(e) => (format!("{e}"), false),
        },
        ExtMsg::Remove(id) => match mgr.remove(id) {
            Ok(()) => (format!("removed {id}"), true),
            Err(e) => (format!("{e}"), false),
        },
        ExtMsg::Reloaded => (String::new(), false),
    }
}

pub fn view<'a>(mgr: &'a Manager, status: &'a str) -> Element<'a, Message> {
    let header = row![
        column![
            text("🧩 Extensions").size(22),
            text("Lua extensions decorate the NBT editor: extra annotations, descriptions and colors on rows. Extensions run with full trust — install only from sources you trust.")
                .size(12)
                .color(style::MUTED),
        ]
        .spacing(4),
        space::horizontal(),
        button(text("Install from .zip…").size(13))
            .on_press(Message::Ext(ExtMsg::PickZip))
            .style(style::button_style(Btn::Primary))
            .padding([8, 12]),
        button(text("Install example (Tick Clock)").size(13))
            .on_press(Message::Ext(ExtMsg::InstallExample))
            .style(style::button_style(Btn::Standard))
            .padding([8, 12]),
        button(text("← Home").size(13)).on_press(Message::Ext(ExtMsg::Back)).style(style::button_style(Btn::Ghost)).padding([8, 12]),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let records = mgr.records();
    let list = if records.is_empty() {
        column![container(
            column![
                text("No extensions installed.").size(14),
                text("Install a .zip with manifest.json + main.lua, or try the built-in Tick Clock example (tick counters shown as in-game time).")
                    .size(12)
                    .color(style::MUTED),
            ]
            .spacing(6),
        )
        .padding(16)
        .style(style::card())
        .width(Length::Fill)]
    } else {
        records.iter().enumerate().fold(column![].spacing(8), |c, (i, rec)| {
            c.push(ext_card(mgr, rec, i, records.len()))
        })
    };

    let footer = if status.is_empty() {
        text(format!("extensions live in {}", mgr.root().display())).size(11).color(style::HINT)
    } else {
        text(status).size(12).color(style::ACCENT)
    };

    container(column![header, iced::widget::rule::horizontal(1.0), scrollable(list).height(Length::Fill), footer].spacing(14).padding(20))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn ext_card<'a>(mgr: &'a Manager, rec: &omt_script::ExtensionRecord, i: usize, total: usize) -> Element<'a, Message> {
    let enabled = mgr.is_enabled(&rec.id);
    let toggle_label = if enabled { "✓ Enabled" } else { "✕ Disabled" };
    let toggle_btn = if enabled { Btn::Confirm } else { Btn::Ghost };
    let meta = [
        (!rec.manifest.version.is_empty()).then(|| format!("v{}", rec.manifest.version)),
        (!rec.manifest.author.is_empty()).then(|| format!("by {}", rec.manifest.author)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let body = column![
        row![
            text(rec.manifest.name.clone()).size(15),
            if meta.is_empty() { text("").size(11) } else { text(meta).size(11).color(style::MUTED) },
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        if rec.manifest.description.is_empty() {
            text(rec.id.clone()).size(12).color(style::MUTED)
        } else {
            text(rec.manifest.description.clone()).size(12).color(style::MUTED)
        },
    ]
    .spacing(3);
    container(
        row![
            body,
            space::horizontal(),
            button(text(toggle_label).size(12)).on_press(Message::Ext(ExtMsg::Toggle(rec.id.clone()))).style(style::button_style(toggle_btn)).padding([5, 10]),
            button(text("▲").size(12))
                .on_press_maybe((i > 0).then(|| Message::Ext(ExtMsg::Up(rec.id.clone()))))
                .style(style::button_style(Btn::Standard))
                .padding([5, 8]),
            button(text("▼").size(12))
                .on_press_maybe((i + 1 < total).then(|| Message::Ext(ExtMsg::Down(rec.id.clone()))))
                .style(style::button_style(Btn::Standard))
                .padding([5, 8]),
            button(text("Remove").size(12)).on_press(Message::Ext(ExtMsg::Remove(rec.id.clone()))).style(style::button_style(Btn::Danger)).padding([5, 10]),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    )
    .padding(10)
    .style(style::card())
    .into()
}
