//! Home screen: UMT-style app tiles (NBT Editor · Converter · Pruner) and,
//! once an app is chosen, the world picker (discovered worlds as cards with
//! icon, edition, version, "Mode, Cheats" blurb and last-played time, plus
//! Browse folder / Open archive).

use iced::widget::{button, column, container, image, row, scrollable, space, text};
use iced::{Element, Length};

use crate::session::App;
use crate::style::{self, Btn};
use crate::worlds::{self, WorldEntry};
use crate::Message;

const APPS: [(App, &str, &str); 3] = [
    (App::NbtEditor, "🗂  NBT Editor", "Edit the raw saved data of your worlds: world settings, players, entities, chunks, LevelDB keys — with undo/redo, search, copy/paste and a chunk locator."),
    (App::Converter, "⇄  Converter", "Convert worlds between Java and Bedrock (latest versions) with skip-partial-chunks, game mode and per-dimension options; settings and game rules carried over."),
    (App::Pruner, "✂  Pruner", "Select and remove unwanted parts of your worlds with layered Box, Circle, Polygon, Split and Time selections, then Optimize."),
];

pub fn view<'a>(
    worlds: &'a [WorldEntry],
    picker: Option<App>,
    scanning: bool,
    opening: bool,
    recent: &'a [std::path::PathBuf],
) -> Element<'a, Message> {
    let header = row![
        column![
            row![text("Omni").size(30).color(style::ACCENT), text("MCTools").size(30)],
            text("Free, offline Minecraft world tools — Java + Bedrock, latest stable. No account, no internet.").size(13).color(style::MUTED),
        ]
        .spacing(4),
        space::horizontal(),
        button(text("Open NBT file…").size(13)).on_press(Message::PickFile).style(style::button_style(Btn::Standard)).padding([8, 12]),
        button(text("🧩 Extensions").size(13)).on_press(Message::OpenExtensions).style(style::button_style(Btn::Standard)).padding([8, 12]),
        button(text("⚙ Settings").size(13)).on_press(Message::OpenSettings).style(style::button_style(Btn::Standard)).padding([8, 12]),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let tiles = APPS.iter().fold(row![].spacing(14), |r, (app, title, desc)| {
        let selected = picker == Some(*app);
        let body = column![text(*title).size(20), text(*desc).size(12).color(style::MUTED)].spacing(8);
        let tile = button(container(body).padding(16).width(Length::Fill))
            .on_press(Message::ChooseApp(*app))
            .width(Length::FillPortion(1))
            .style(style::button_style(if selected { Btn::Primary } else { Btn::Standard }));
        r.push(tile)
    });

    let picker_view: Element<'a, Message> = match picker {
        None => column![
            text("Choose an app above, then pick a world.").size(13).color(style::MUTED),
            recent_list(recent),
        ]
        .spacing(10)
        .into(),
        Some(app) => world_picker(app, worlds, scanning, opening),
    };

    container(column![header, tiles, iced::widget::rule::horizontal(1.0), picker_view].spacing(16).padding(20))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn recent_list(recent: &[std::path::PathBuf]) -> Element<'_, Message> {
    if recent.is_empty() {
        return space::vertical().height(0).into();
    }
    let items = recent.iter().fold(column![text("Recent").size(14)].spacing(4), |c, p| {
        c.push(text(p.display().to_string()).size(12).color(style::MUTED))
    });
    items.into()
}

fn world_picker<'a>(app: App, worlds: &'a [WorldEntry], scanning: bool, opening: bool) -> Element<'a, Message> {
    let actions = row![
        text(format!("Select a world for the {}", app.name())).size(16),
        space::horizontal(),
        button(text("Browse folder…").size(12)).on_press(Message::BrowseFolder(app)).style(style::button_style(Btn::Standard)),
        button(text("Open .mcworld / .zip…").size(12)).on_press(Message::BrowseArchive(app)).style(style::button_style(Btn::Standard)),
        button(text("Rescan").size(12)).on_press(Message::Rescan).style(style::button_style(Btn::Ghost)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);
    let note: Element<'a, Message> = if opening {
        text("Copying the world into a workspace… (the original is never modified until you Save)").size(12).color(style::WARN).into()
    } else if scanning {
        text("Scanning world folders…").size(12).color(style::MUTED).into()
    } else if worlds.is_empty() {
        text("No worlds found in the standard folders. Add folders in Settings, or Browse.").size(12).color(style::MUTED).into()
    } else {
        text(format!("{} world(s)", worlds.len())).size(12).color(style::MUTED).into()
    };
    let list = worlds.iter().fold(column![].spacing(8), |c, w| c.push(world_card(app, w, opening)));
    column![actions, note, scrollable(list).height(Length::Fill)].spacing(10).into()
}

fn world_card(app: App, w: &WorldEntry, opening: bool) -> Element<'_, Message> {
    let icon: Element<'_, Message> = match &w.summary.icon {
        Some(p) => image(image::Handle::from_path(p)).width(64).height(64).into(),
        None => container(text(if w.summary.edition == omt_world::Edition::Java { "J" } else { "B" }).size(26))
            .center(64)
            .style(style::card_selected())
            .into(),
    };
    let version = crate::style::version_label(&w.summary);
    let played = w.last_modified.map(worlds::format_time).unwrap_or_default();
    let info = column![
        text(w.summary.name.clone()).size(16),
        text(format!("{} · {} · {}", w.summary.edition.name(), version, worlds::blurb(w))).size(12).color(style::MUTED),
        text(format!("Last played {played} · {} · {}", w.folder_label, w.origin.display())).size(11).color(style::HINT),
    ]
    .spacing(3);
    container(
        row![
            icon,
            info,
            space::horizontal(),
            button(text(format!("Open in {}", app.name())).size(13))
                .on_press_maybe((!opening).then(|| Message::OpenWorld(app, w.origin.clone())))
                .style(style::button_style(Btn::Confirm))
                .padding([8, 14]),
        ]
        .spacing(14)
        .align_y(iced::Alignment::Center),
    )
    .padding(10)
    .style(style::card())
    .into()
}
