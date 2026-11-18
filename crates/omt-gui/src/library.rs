//! The world library: every detected world as a grid of thumbnail cards
//! or a dense list, with the search syntax (`crate::filter`), facet chips
//! (edition, launcher, game mode, last played, favorites), sorting and
//! per-world actions (open in any app, star, show in file manager).
//! Tool pages embed the same library with a fixed target app.

use std::path::PathBuf;

use iced::widget::{button, column, container, image, pick_list, row, scrollable, space, stack, text, text_input, tooltip};
use iced::{Alignment, Color, Element, Length};
use omt_art::{Glyph, Icon};

use crate::Message;
use crate::assets;
use crate::filter::{self, EditionKey, Facets, Played, Query, SortBy};
use crate::session::App;
use crate::settings::Settings;
use crate::sources::Launcher;
use crate::style::{self, Btn};
use crate::worlds::{self, WorldEntry};

#[derive(Debug, Clone)]
pub enum LibMsg {
    Query(String),
    Edition(EditionKey),
    Launcher(Launcher),
    Mode(&'static str),
    Played(Played),
    Favorites,
    Clear,
    Sort(SortBy),
    Grid(bool),
    ToggleFilters,
    Star(PathBuf),
    Reveal(PathBuf),
    /// Choose a cover image for a world (file dialog in the app).
    PickCover(PathBuf),
    SetCover(PathBuf, Option<PathBuf>),
}

/// Library UI state (shared by the Worlds page and tool pages).
#[derive(Debug, Default)]
pub struct Library {
    pub query: String,
    pub parsed: Query,
    pub facets: Facets,
    pub filtered: Vec<usize>,
    pub show_filters: bool,
}

fn toggle<T: Ord>(set: &mut std::collections::BTreeSet<T>, v: T) {
    if !set.remove(&v) {
        set.insert(v);
    }
}

impl Library {
    pub fn refresh(&mut self, worlds: &[WorldEntry], s: &Settings) {
        let fav = |w: &WorldEntry| s.is_favorite(&w.origin);
        self.filtered = filter::apply(worlds, &self.parsed, &self.facets, s.library_sort, &fav);
    }

    /// Returns `true` when settings changed (sort / layout / favorites).
    pub fn update(&mut self, m: LibMsg, s: &mut Settings) -> bool {
        match m {
            LibMsg::Query(q) => {
                self.parsed = Query::parse(&q);
                self.query = q;
            }
            LibMsg::Edition(e) => toggle(&mut self.facets.editions, e),
            LibMsg::Launcher(l) => toggle(&mut self.facets.launchers, l),
            LibMsg::Mode(m) => toggle(&mut self.facets.modes, m),
            LibMsg::Played(p) => self.facets.played = p,
            LibMsg::Favorites => self.facets.favorites = !self.facets.favorites,
            LibMsg::Clear => {
                self.facets = Facets::default();
                self.query.clear();
                self.parsed = Query::default();
            }
            LibMsg::ToggleFilters => self.show_filters = !self.show_filters,
            LibMsg::Sort(o) => {
                s.library_sort = o;
                return true;
            }
            LibMsg::Grid(g) => {
                s.library_grid = g;
                return true;
            }
            LibMsg::Star(p) => {
                s.toggle_favorite(p);
                return true;
            }
            LibMsg::Reveal(p) => reveal(&p),
            LibMsg::PickCover(_) => {}
            LibMsg::SetCover(w, img) => {
                s.set_cover(w, img);
                return true;
            }
        }
        false
    }
}

/// Open a folder in the system file manager.
fn reveal(p: &std::path::Path) {
    let cmd = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(p).spawn();
}

fn lib(m: LibMsg) -> Message {
    Message::Lib(m)
}

fn chip<'a>(label: String, on: bool, msg: Message) -> Element<'a, Message> {
    button(text(label).size(12)).padding([4, 12]).on_press(msg).style(style::button_style(if on { Btn::ChipOn } else { Btn::Chip })).into()
}

pub fn edition_badge<'a>(w: &WorldEntry) -> Element<'a, Message> {
    let (label, color) = match w.edition() {
        omt_world::Edition::Java => ("JAVA", style::java_color()),
        omt_world::Edition::Bedrock => ("BEDROCK", style::bedrock_color()),
    };
    container(text(label).size(10).font(style::BOLD)).padding([2, 8]).style(style::badge_solid(color)).into()
}

/// A world's cover: the user's assigned image, the world's own icon, or
/// a generated scene.
pub fn cover_handle(w: &WorldEntry, s: &Settings) -> image::Handle {
    if let Some(c) = s.cover(&w.origin) {
        return image::Handle::from_path(c);
    }
    match &w.summary.icon {
        Some(p) => image::Handle::from_path(p),
        None => assets::world_thumb(&w.summary.name, w.edition() == omt_world::Edition::Bedrock),
    }
}

fn thumb<'a>(w: &WorldEntry, s: &Settings, width: f32, height: f32) -> Element<'a, Message> {
    let h = cover_handle(w, s);
    container(image(h).content_fit(iced::ContentFit::Cover).filter_method(image::FilterMethod::Nearest).width(width).height(height))
        .style(|_| container::Style { border: iced::border::rounded(6), ..Default::default() })
        .clip(true)
        .into()
}

fn app_button<'a>(app: App, origin: PathBuf, opening: bool) -> Element<'a, Message> {
    let b = button(assets::icon(app.icon(), 18.0)).padding(5).on_press_maybe((!opening).then(|| Message::OpenWorld(app, origin))).style(style::button_style(Btn::Standard));
    tooltip(b, container(text(format!("Open in {}", app.name())).size(12)).padding([4, 8]).style(style::card()), tooltip::Position::Top).into()
}

fn actions<'a>(w: &WorldEntry, target: Option<App>, opening: bool) -> Element<'a, Message> {
    match target {
        Some(app) => button(row![assets::icon(app.icon(), 16.0), text(format!("Open in {}", app.name())).size(12)].spacing(6).align_y(Alignment::Center))
            .padding([6, 12])
            .on_press_maybe((!opening).then(|| Message::OpenWorld(app, w.origin.clone())))
            .style(style::button_style(Btn::Primary))
            .into(),
        None => App::ALL.iter().fold(row![].spacing(4), |r, &a| r.push(app_button(a, w.origin.clone(), opening))).into(),
    }
}

fn cover_button<'a>(w: &WorldEntry, custom: bool) -> Element<'a, Message> {
    let b = button(container(assets::glyph(Glyph::Picture, Color::WHITE, 12.0)).center(20)).padding(2).on_press(if custom { lib(LibMsg::SetCover(w.origin.clone(), None)) } else { lib(LibMsg::PickCover(w.origin.clone())) }).style(move |_t, st| {
        let a = if matches!(st, button::Status::Hovered) { 0.75 } else { 0.5 };
        button::Style { background: Some(iced::Background::Color(Color::BLACK.scale_alpha(a))), border: iced::border::rounded(12), ..Default::default() }
    });
    tooltip(b, container(text(if custom { "Reset cover" } else { "Set a cover image…" }).size(12)).padding([4, 8]).style(style::card()), tooltip::Position::Left).into()
}

fn star<'a>(w: &WorldEntry, fav: bool) -> Element<'a, Message> {
    let inner: Element<'a, Message> = if fav { assets::icon(Icon::Star, 16.0) } else { assets::glyph(Glyph::Plus, Color::from_rgba(1.0, 1.0, 1.0, 0.8), 12.0) };
    let b = button(container(inner).center(20)).padding(2).on_press(lib(LibMsg::Star(w.origin.clone()))).style(move |_t, st| {
        let a = if matches!(st, button::Status::Hovered) { 0.75 } else { 0.5 };
        button::Style { background: Some(iced::Background::Color(Color::BLACK.scale_alpha(a))), border: iced::border::rounded(12), ..Default::default() }
    });
    tooltip(b, container(text(if fav { "Unstar" } else { "Star this world" }).size(12)).padding([4, 8]).style(style::card()), tooltip::Position::Left).into()
}

fn meta_line(w: &WorldEntry) -> String {
    let played = w.last_modified.map(worlds::relative_time).unwrap_or_else(|| "never".into());
    format!("{} · {} · {}", worlds::human_size(w.size), played, filter::mode_name(w.summary.gamemode))
}

fn card<'a>(w: &'a WorldEntry, s: &'a Settings, target: Option<App>, opening: bool) -> Element<'a, Message> {
    let fav = s.is_favorite(&w.origin);
    let cover = stack![
        thumb(w, s, 264.0, 132.0),
        container(row![edition_badge(w), space::horizontal(), cover_button(w, s.cover(&w.origin).is_some()), star(w, fav)].spacing(4).align_y(Alignment::Center)).padding(8).width(Length::Fill),
    ];
    let preview = if w.preview { " · Preview" } else { "" };
    let body = column![
        text(w.summary.name.clone()).size(15).font(style::BOLD),
        text(format!("{}{preview}", w.version)).size(11).color(style::muted()),
        text(meta_line(w)).size(11).color(style::muted()),
        row![assets::icon(launcher_icon(w.launcher), 12.0), text(w.folder_label.clone()).size(11).color(style::hint())].spacing(6).align_y(Alignment::Center),
        actions(w, target, opening),
    ]
    .spacing(4);
    container(column![cover, container(body).padding([10, 12])]).width(266).style(style::card()).into()
}

fn list_row<'a>(w: &'a WorldEntry, s: &'a Settings, target: Option<App>, opening: bool) -> Element<'a, Message> {
    let fav = s.is_favorite(&w.origin);
    container(
        row![
            thumb(w, s, 96.0, 54.0),
            column![
                row![text(w.summary.name.clone()).size(14).font(style::BOLD), edition_badge(w)].spacing(8).align_y(Alignment::Center),
                text(format!("{} · {} · {}", w.version, meta_line(w), w.folder_label)).size(11).color(style::muted()),
            ]
            .spacing(4)
            .width(Length::Fill),
            cover_button(w, s.cover(&w.origin).is_some()),
            star(w, fav),
            button(assets::icon(Icon::Folder, 16.0)).padding(5).on_press(lib(LibMsg::Reveal(w.origin.clone()))).style(style::button_style(Btn::Ghost)),
            actions(w, target, opening),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding(8)
    .style(style::card())
    .into()
}

pub fn launcher_icon(l: Launcher) -> Icon {
    match l {
        Launcher::JavaVanilla | Launcher::Prism | Launcher::MultiMc | Launcher::Modrinth => Icon::Worlds,
        Launcher::Mcpelauncher | Launcher::BedrockOnLinux | Launcher::BedrockWindows | Launcher::BedrockUwp => Icon::Archive,
        Launcher::Custom => Icon::Folder,
    }
}

fn filters<'a>(l: &'a Library, worlds: &'a [WorldEntry]) -> Element<'a, Message> {
    let c = filter::counts(worlds);
    let f = &l.facets;
    let mut ed = row![text("Edition").size(12).color(style::hint()).width(90)].spacing(6).align_y(Alignment::Center);
    ed = ed.push(chip(format!("Java  {}", c.java), f.editions.contains(&EditionKey::Java), lib(LibMsg::Edition(EditionKey::Java))));
    ed = ed.push(chip(format!("Bedrock  {}", c.bedrock), f.editions.contains(&EditionKey::Bedrock), lib(LibMsg::Edition(EditionKey::Bedrock))));
    let la = c.launchers.iter().fold(row![text("Launcher").size(12).color(style::hint()).width(90)].spacing(6).align_y(Alignment::Center), |r, (lch, n)| {
        r.push(chip(format!("{}  {n}", lch.name()), f.launchers.contains(lch), lib(LibMsg::Launcher(*lch))))
    });
    let mo = c.modes.iter().fold(row![text("Game mode").size(12).color(style::hint()).width(90)].spacing(6).align_y(Alignment::Center), |r, (m, n)| {
        r.push(chip(format!("{m}  {n}"), f.modes.contains(m), lib(LibMsg::Mode(m))))
    });
    let pl = Played::ALL.iter().fold(row![text("Last played").size(12).color(style::hint()).width(90)].spacing(6).align_y(Alignment::Center), |r, p| {
        r.push(chip(p.label().into(), f.played == *p, lib(LibMsg::Played(*p))))
    });
    let help = text("Search syntax: e:java · src:prism · mode:creative · v:1.21 · size:>1g · played:<7d · is:fav · cheats:on · \"exact name\" · -exclude").size(11).color(style::hint());
    container(column![ed.wrap(), la.wrap(), mo.wrap(), pl.wrap(), help].spacing(8)).padding(12).style(style::card()).width(Length::Fill).into()
}

/// The library (world grid/list) with its toolbar.
pub fn view<'a>(l: &'a Library, worlds: &'a [WorldEntry], s: &'a Settings, target: Option<App>, scanning: bool, opening: bool) -> Element<'a, Message> {
    let active = l.facets.active();
    let filter_label = if active > 0 { format!("Filters · {active}") } else { "Filters".into() };
    let search = text_input("Search worlds…  (try e:bedrock played:<7d)", &l.query).on_input(|q| lib(LibMsg::Query(q))).padding([8, 12]).size(13).style(style::input()).width(Length::Fill);
    let toolbar = row![
        search,
        button(row![assets::glyph(Glyph::Filter, style::fg(), 14.0), text(filter_label).size(12)].spacing(6).align_y(Alignment::Center))
            .padding([7, 12])
            .on_press(lib(LibMsg::ToggleFilters))
            .style(style::button_style(if l.show_filters || active > 0 { Btn::ChipOn } else { Btn::Standard })),
        chip(if l.facets.favorites { "★ Starred".into() } else { "☆ Starred".into() }, l.facets.favorites, lib(LibMsg::Favorites)),
        pick_list(SortBy::ALL.to_vec(), Some(s.library_sort), |o| lib(LibMsg::Sort(o))).text_size(12).padding([7, 10]),
        button(assets::glyph(Glyph::Grid, if s.library_grid { style::info() } else { style::muted() }, 14.0)).padding(7).on_press(lib(LibMsg::Grid(true))).style(style::button_style(Btn::Ghost)),
        button(assets::glyph(Glyph::List, if s.library_grid { style::muted() } else { style::info() }, 14.0)).padding(7).on_press(lib(LibMsg::Grid(false))).style(style::button_style(Btn::Ghost)),
        button(assets::glyph(Glyph::Refresh, style::muted(), 14.0)).padding(7).on_press(Message::Rescan).style(style::button_style(Btn::Ghost)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let mut col = column![toolbar].spacing(12);
    if l.show_filters {
        col = col.push(filters(l, worlds));
    }
    let note = if opening {
        text("Opening… (editors work on a copy; the viewer opens in place, read-only)").size(12).color(style::warn())
    } else if scanning {
        text("Scanning launcher folders…").size(12).color(style::muted())
    } else if worlds.is_empty() {
        text("No worlds found. Add folders in Settings, or use Open world… in the top bar.").size(12).color(style::muted())
    } else {
        text(format!("{} of {} worlds", l.filtered.len(), worlds.len())).size(12).color(style::muted())
    };
    let mut head = row![note, space::horizontal()].align_y(Alignment::Center);
    if active > 0 || !l.query.is_empty() {
        head = head.push(button(text("Clear filters").size(12)).on_press(lib(LibMsg::Clear)).style(style::button_style(Btn::Ghost)));
    }
    if !l.parsed.unknown.is_empty() {
        head = head.push(text(format!("not understood: {}", l.parsed.unknown.join(" "))).size(11).color(style::warn()));
    }
    col = col.push(head);
    let items: Element<'a, Message> = if s.library_grid {
        l.filtered.iter().fold(row![].spacing(14), |r, &i| r.push(card(&worlds[i], s, target, opening))).wrap().vertical_spacing(14).into()
    } else {
        l.filtered.iter().fold(column![].spacing(6), |c, &i| c.push(list_row(&worlds[i], s, target, opening))).into()
    };
    col.push(scrollable(container(items).padding([0, 4])).height(Length::Fill)).into()
}
