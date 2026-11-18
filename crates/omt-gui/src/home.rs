//! Home dashboard (hero banner with stats and quick actions, tool cards,
//! "jump back in" recent worlds, detected launchers) and the tool landing
//! pages (tool hero + the library targeted at that tool).

use iced::widget::{button, column, container, row, scrollable, space, stack, text};
use iced::{Alignment, Color, Element, Length};
use omt_art::Icon;

use crate::Message;
use crate::Page;
use crate::assets;
use crate::library::{self, Library};
use crate::session::App;
use crate::settings::Settings;
use crate::style::{self, Btn};
use crate::worlds::{self, WorldEntry};

fn stat<'a>(value: String, label: &'a str) -> Element<'a, Message> {
    container(column![text(value).size(20).font(style::BOLD).color(Color::WHITE), text(label).size(11).color(Color::from_rgb(0.85, 0.87, 0.9))].spacing(0))
        .padding([8, 14])
        .style(style::scrim(0.45))
        .into()
}

fn hero<'a>(worlds: &[WorldEntry]) -> Element<'a, Message> {
    let java = worlds.iter().filter(|w| w.edition() == omt_world::Edition::Java).count();
    let launchers = worlds.iter().map(|w| w.launcher).collect::<std::collections::BTreeSet<_>>().len();
    let total: u64 = worlds.iter().map(|w| w.size).sum();
    let content = column![
        assets::wordmark(40.0),
        text("World tools for Minecraft Java & Bedrock — view, edit, convert and prune. Offline. No account.").size(14).color(Color::WHITE),
        row![
            button(row![assets::icon(Icon::Worlds, 16.0), text("Browse worlds").size(13)].spacing(8).align_y(Alignment::Center)).padding([9, 16]).on_press(Message::Nav(Page::Library)).style(style::button_style(Btn::Primary)),
            button(row![assets::icon(Icon::Map, 16.0), text("Open a map…").size(13)].spacing(8).align_y(Alignment::Center)).padding([9, 16]).on_press(Message::BrowseFolder(App::MapViewer)).style(style::button_style(Btn::Standard)),
            button(row![assets::icon(Icon::NbtEditor, 16.0), text("Open NBT file…").size(13)].spacing(8).align_y(Alignment::Center)).padding([9, 16]).on_press(Message::PickFile).style(style::button_style(Btn::Standard)),
        ]
        .spacing(10),
        space::vertical().height(Length::Fill),
        row![
            stat(worlds.len().to_string(), "worlds"),
            stat(java.to_string(), "Java"),
            stat((worlds.len() - java).to_string(), "Bedrock"),
            stat(launchers.to_string(), "launchers"),
            stat(worlds::human_size(total), "on disk"),
        ]
        .spacing(8),
    ]
    .spacing(12);
    let shade = container(space::vertical()).width(Length::Fill).height(Length::Fill).style(|_| {
        let g = iced::gradient::Linear::new(std::f32::consts::FRAC_PI_2).add_stop(0.0, Color::from_rgba(0.0, 0.0, 0.0, 0.62)).add_stop(0.65, Color::from_rgba(0.0, 0.0, 0.0, 0.1)).add_stop(1.0, Color::TRANSPARENT);
        container::Style { background: Some(iced::Background::Gradient(g.into())), ..Default::default() }
    });
    container(stack![assets::banner(assets::hero_scene(), 300.0), shade, container(content).padding([24, 28]).height(300)])
        .height(300)
        .width(Length::Fill)
        .style(|_| container::Style { border: iced::border::rounded(12), ..Default::default() })
        .clip(true)
        .into()
}

fn tool_card<'a>(app: App) -> Element<'a, Message> {
    let short = match app {
        App::MapViewer => "Explore worlds with lighting, night & cave views",
        App::WorldEditor => "Fill, replace, copy & paste blocks · structures",
        App::NbtEditor => "Edit level data, players, chunks & LevelDB",
        App::Converter => "Java ⇄ Bedrock, settings carried over",
        App::Pruner => "Trim unwanted chunks with layered selections",
    };
    button(
        row![
            assets::block(app.block(), 56.0),
            column![
                {
                    let mut r = row![assets::icon(app.icon(), 16.0), text(app.name()).size(15).font(style::BOLD)].spacing(6).align_y(Alignment::Center);
                    if app.alpha() {
                        r = r.push(style::alpha_badge());
                    }
                    r
                },
                text(short).size(12).color(style::muted())
            ]
            .spacing(4),
        ]
        .spacing(14)
        .align_y(Alignment::Center),
    )
    .width(Length::FillPortion(1))
    .padding(14)
    .on_press(Message::Nav(Page::Tool(app)))
    .style(style::button_style(Btn::Card))
    .into()
}

fn section_title<'a>(title: &'a str, action: Option<(&'a str, Message)>) -> Element<'a, Message> {
    let mut r = row![assets::heading(title, style::fg(), 2), space::horizontal()].align_y(Alignment::Center);
    if let Some((label, msg)) = action {
        r = r.push(button(text(label).size(12)).on_press(msg).style(style::button_style(Btn::Ghost)));
    }
    r.into()
}

fn recent_card<'a>(w: &'a WorldEntry, s: &Settings) -> Element<'a, Message> {
    let h = library::cover_handle(w, s);
    let img = iced::widget::image(h).content_fit(iced::ContentFit::Cover).filter_method(iced::widget::image::FilterMethod::Nearest).width(200).height(100);
    button(
        column![
            stack![container(img).clip(true).style(|_| container::Style { border: iced::border::rounded(6), ..Default::default() }), container(library::edition_badge(w)).padding(6)],
            text(w.summary.name.clone()).size(13).font(style::BOLD),
            text(w.last_modified.map(worlds::relative_time).unwrap_or_default()).size(11).color(style::muted()),
        ]
        .spacing(6),
    )
    .padding(8)
    .width(216)
    .on_press(Message::OpenWorld(App::MapViewer, w.origin.clone()))
    .style(style::button_style(Btn::Card))
    .into()
}

pub fn view<'a>(worlds: &'a [WorldEntry], s: &'a Settings, scanning: bool) -> Element<'a, Message> {
    let tools = App::ALL.iter().fold(row![].spacing(12), |r, &a| r.push(tool_card(a)));
    let recent: Element<'a, Message> = if worlds.is_empty() {
        text(if scanning { "Scanning launcher folders…" } else { "No worlds found yet — add a folder in Settings or open one from the top bar." }).size(12).color(style::muted()).into()
    } else {
        scrollable(worlds.iter().take(8).fold(row![].spacing(12), |r, w| r.push(recent_card(w, s))).padding([0, 0]))
            .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(4).scroller_width(4)))
            .into()
    };
    let mut launchers: Vec<(crate::sources::Launcher, String, usize)> = Vec::new();
    for w in worlds {
        match launchers.iter_mut().find(|(l, _, _)| *l == w.launcher) {
            Some(e) => e.2 += 1,
            None => launchers.push((w.launcher, w.launcher.name().to_string(), 1)),
        }
    }
    let launcher_row = launchers.into_iter().fold(row![].spacing(8), |r, (l, name, n)| {
        r.push(
            button(row![assets::icon(library::launcher_icon(l), 16.0), text(name).size(12), text(n.to_string()).size(12).color(style::muted())].spacing(8).align_y(Alignment::Center))
                .padding([8, 12])
                .on_press(Message::LibraryLauncher(l))
                .style(style::button_style(Btn::Card)),
        )
    });
    let fav_count = worlds.iter().filter(|w| s.is_favorite(&w.origin)).count();
    let body = column![
        hero(worlds),
        section_title("TOOLS", None),
        tools,
        section_title("JUMP BACK IN", Some(("See all worlds →", Message::Nav(Page::Library)))),
        recent,
        section_title("DETECTED LAUNCHERS", None),
        launcher_row.wrap().vertical_spacing(8),
        text(if fav_count > 0 { format!("{fav_count} starred world(s) — star more from the library to pin them first.") } else { "Tip: star worlds in the library to pin them to the top.".into() }).size(11).color(style::hint()),
    ]
    .spacing(16)
    .padding(24);
    scrollable(body).height(Length::Fill).into()
}

/// A tool's landing page: hero strip + library targeted at the tool.
pub fn tool_page<'a>(app: App, lib: &'a Library, worlds: &'a [WorldEntry], s: &'a Settings, scanning: bool, opening: bool) -> Element<'a, Message> {
    let read_only = if app.read_only() { "Opens worlds in place, read-only." } else { "Works on a copy — your world changes only when you Save." };
    let head = container(
        row![
            assets::block(app.block(), 84.0),
            column![
                {
                    let mut r = row![assets::heading(&app.name().to_uppercase(), style::fg(), 3)].spacing(10).align_y(Alignment::Center);
                    if app.alpha() {
                        r = r.push(style::alpha_badge());
                    }
                    r
                },
                text(app.tagline()).size(13).color(style::muted()),
                text(read_only).size(11).color(style::hint()),
            ]
            .spacing(8)
            .width(Length::Fill),
            column![
                button(row![assets::icon(Icon::Folder, 16.0), text("Open folder…").size(12)].spacing(6).align_y(Alignment::Center)).width(170).padding([8, 12]).on_press(Message::BrowseFolder(app)).style(style::button_style(Btn::Standard)),
                button(row![assets::icon(Icon::Archive, 16.0), text(".mcworld / .zip…").size(12)].spacing(6).align_y(Alignment::Center)).width(170).padding([8, 12]).on_press(Message::BrowseArchive(app)).style(style::button_style(Btn::Standard)),
            ]
            .spacing(6),
        ]
        .spacing(20)
        .align_y(Alignment::Center),
    )
    .padding(18)
    .style(style::card());
    column![head, library::view(lib, worlds, s, Some(app), scanning, opening)].spacing(16).padding(24).into()
}
