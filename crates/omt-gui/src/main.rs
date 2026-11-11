//! `omt-gui` — OmniMCTools desktop application (Iced 0.14).
//!
//! Shell: Home (world discovery) + one tab per open NBT document. All editor
//! logic lives in `editor` (state), `editor_view` (rendering) and `dictionary`
//! (friendly labels); this module only routes messages.

mod dictionary;
mod editor;
mod editor_view;
mod style;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Color, Element, Length, Subscription, Task, Theme};
use omt_world::WorldSummary;

use editor::{EditorMsg, EditorTab};

pub(crate) use style::{ACCENT, MUTED};
/// Added tags.
pub(crate) const ADDED: Color = style::SUCCESS;
/// Edited tags.
pub(crate) const EDITED: Color = style::WARN;
/// Current search hit.
pub(crate) const HIT: Color = style::ACCENT;

fn main() -> iced::Result {
    iced::application(OmniMCTools::new, OmniMCTools::update, OmniMCTools::view)
        .title(|s: &OmniMCTools| {
            let tab = s.active.and_then(|i| s.tabs.get(i)).map(|t| t.title.as_str());
            match tab {
                Some(t) => format!("{t} — OmniMCTools"),
                None => "OmniMCTools".to_string(),
            }
        })
        .theme(|_s: &OmniMCTools| omnitheme())
        .subscription(OmniMCTools::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1200.0, 760.0),
            ..Default::default()
        })
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    Refresh,
    PickFile,
    FilePicked(Option<std::path::PathBuf>),
    CloseTab(usize),
    SelectTab(usize),
    EditorTab(usize, EditorMsg),
    FocusSearch,
    HotkeyUndo,
    HotkeyRedo,
    HotkeySave,
    PasteRequested(usize),
}

struct OmniMCTools {
    worlds: Vec<WorldSummary>,
    scan_dir: Option<std::path::PathBuf>,
    tabs: Vec<EditorTab>,
    active: Option<usize>,
    status: String,
}

impl OmniMCTools {
    fn new() -> Self {
        let scan_dir = std::env::var_os("OMT_WORLDS_DIR").map(std::path::PathBuf::from);
        let mut app = Self { worlds: Vec::new(), scan_dir, tabs: Vec::new(), active: None, status: "ready".into() };
        app.rescan();
        app
    }

    fn rescan(&mut self) {
        self.worlds.clear();
        let Some(dir) = self.scan_dir.clone() else {
            self.status = "set OMT_WORLDS_DIR to list worlds here".into();
            return;
        };
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                let mut found = 0;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    if let Ok(detected) = omt_world::detect(&path)
                        && let Ok(summary) = WorldSummary::from_detected(&detected)
                    {
                        self.worlds.push(summary);
                        found += 1;
                    }
                }
                self.status = format!("{found} world(s) in {}", dir.display());
            }
            Err(e) => self.status = format!("cannot read {}: {e}", dir.display()),
        }
    }

    fn open_tab(&mut self, path: std::path::PathBuf) -> Task<Message> {
        match EditorTab::open(&path) {
            Ok(tab) => {
                self.tabs.push(tab);
                self.active = Some(self.tabs.len() - 1);
                self.status = format!("opened {}", path.display());
                Task::none()
            }
            Err(e) => {
                self.status = format!("open failed: {e}");
                Task::none()
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => self.rescan(),
            Message::PickFile => {
                return Task::perform(pick_nbt_file(), Message::FilePicked);
            }
            Message::FilePicked(Some(path)) => return self.open_tab(path),
            Message::FilePicked(None) => self.status = "no file picked".into(),
            Message::CloseTab(i) => {
                if i < self.tabs.len() {
                    let dirty = self.tabs[i].is_dirty();
                    if dirty && !self.tabs[i].status.starts_with("force-closed") {
                        self.tabs[i].status = "unsaved changes — press Close again to force".into();
                        self.status = format!("tab {} has unsaved changes", self.tabs[i].title);
                        return Task::none();
                    }
                    self.tabs.remove(i);
                    self.active = if self.tabs.is_empty() {
                        None
                    } else {
                        Some(i.min(self.tabs.len() - 1))
                    };
                }
            }
            Message::SelectTab(i) => self.active = Some(i),
            Message::FocusSearch => {
                return iced::widget::operation::focus(editor_view::search_id());
            }
            Message::PasteRequested(i) => {
                return iced::clipboard::read().map(move |clip| {
                    Message::EditorTab(i, EditorMsg::PasteFromClipboard(clip.unwrap_or_default()))
                });
            }
            Message::HotkeyUndo | Message::HotkeyRedo | Message::HotkeySave => {
                let msg = match message {
                    Message::HotkeyUndo => EditorMsg::Undo,
                    Message::HotkeyRedo => EditorMsg::Redo,
                    _ => EditorMsg::Save,
                };
                if let Some(i) = self.active.filter(|&i| i < self.tabs.len()) {
                    return self.update(Message::EditorTab(i, msg));
                }
            }
            Message::EditorTab(i, m) => {
                if let Some(tab) = self.tabs.get_mut(i) {
                    let tasks = tab.update(m);
                    self.status = tab.status.clone();
                    if !tasks.is_empty() {
                        return tasks.into_iter().next().unwrap_or(Task::none());
                    }
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(handle_hotkey)
    }

    fn view(&self) -> Element<'_, Message> {
        column![
            self.tab_bar(),
            container(self.body()).height(Length::Fill),
            self.status_bar(),
        ]
        .width(Length::Fill)
        .into()
    }

    // ---- top bar: brand + Home chip + document tab chips --------------------
    fn tab_bar(&self) -> Element<'_, Message> {
        let mut bar = row![self.brand()].spacing(8).align_y(iced::Alignment::Center);
        bar = bar.push(tab_chip("Home", false, false, Message::SelectTab(usize::MAX), None));
        for (i, tab) in self.tabs.iter().enumerate() {
            let dirty = tab.is_dirty();
            bar = bar.push(tab_chip(
                &tab.title,
                self.active == Some(i),
                dirty,
                Message::SelectTab(i),
                Some(Message::CloseTab(i)),
            ));
        }
        container(scrollable(container(bar).padding([8, 12])))
            .style(|_| container::Style {
                background: Some(iced::Background::Color(style::BG)),
                ..Default::default()
            })
            .into()
    }

    fn brand(&self) -> Element<'_, Message> {
        row![
            text("Omni").size(17).color(style::ACCENT),
            text("MCTools").size(17),
        ]
        .padding([0, 6])
        .into()
    }

    fn body(&self) -> Element<'_, Message> {
        match self.active {
            Some(i) if i < self.tabs.len() => editor_view::view(&self.tabs[i], i),
            _ => self.home(),
        }
    }

    fn status_bar(&self) -> Element<'_, Message> {
        row![
            text(self.status.clone()).size(12).color(MUTED),
            space::horizontal(),
            text("Ctrl+Z undo · Ctrl+Y redo · Ctrl+S save · Ctrl+F find")
                .size(12)
                .color(style::HINT),
        ]
        .padding([6, 12])
        .spacing(16)
        .into()
    }

    fn home(&self) -> Element<'_, Message> {
        let header = column![
            text("OmniMCTools").size(30),
            text("Minecraft world tools — Java + Bedrock, latest stable")
                .size(13)
                .color(MUTED),
        ]
        .spacing(4);

        let actions = row![
            button(text("Open NBT / level.dat file…").color(Color::WHITE))
                .on_press(Message::PickFile)
                .style(style::button_style(style::Btn::Primary))
                .padding([8, 14]),
            button(text("Rescan worlds"))
                .on_press(Message::Refresh)
                .style(style::button_style(style::Btn::Standard))
                .padding([8, 14]),
        ]
        .spacing(10);

        let count = self.worlds.len();
        let worlds: Element<'_, Message> = if self.worlds.is_empty() {
            column![
                text("No worlds found.").color(MUTED),
                text(match &self.scan_dir {
                    Some(d) => format!("Set OMT_WORLDS_DIR to a folder of worlds (currently {}), or open files directly.", d.display()),
                    None => "Set OMT_WORLDS_DIR to a folder of Minecraft worlds, or open files directly.".to_string(),
                })
                .size(12)
                .color(MUTED),
            ]
            .spacing(4)
            .into()
        } else {
            container(scrollable(
                self.worlds
                    .iter()
                    .filter_map(|w| {
                        let level = w.root.join("level.dat");
                        level.is_file().then(|| world_card(w, level))
                    })
                    .fold(iced::widget::Column::new().spacing(8), |c, card| c.push(card)),
            ))
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
        };

        container(
            column![
                header,
                actions,
                iced::widget::rule::horizontal(1.0),
                text(format!("Worlds ({count})")).size(16),
                worlds,
            ]
            .spacing(12)
            .padding(20),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

/// One world entry as a rounded card: name, edition, version, open action.
/// Takes ownership of the level.dat path so the message owns it too.
fn world_card(w: &WorldSummary, level: std::path::PathBuf) -> Element<'_, Message> {
    let version = version_label(w);
    row![
        text(w.name.clone()).size(15),
        text(w.edition.name()).size(12).color(MUTED),
        text(version).size(12).color(MUTED),
        space::horizontal(),
        button(text("Open level.dat").color(Color::WHITE))
            .on_press(Message::FilePicked(Some(level)))
            .style(style::button_style(style::Btn::Confirm))
            .padding([6, 12]),
    ]
    .spacing(16)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Tab-bar chip (Home or a document), with an optional ✕ close button.
fn tab_chip(
    title: &str,
    active: bool,
    dirty: bool,
    select: Message,
    close: Option<Message>,
) -> Element<'_, Message> {
    let dot = if dirty { " ●" } else { "" };
    let label = text(format!("{title}{dot}")).size(13);
    let chip = button(label)
        .on_press(select)
        .style(style::button_style(style::Btn::Ghost))
        .padding([6, 10]);
    let chip = if active {
        container(chip).style(style::card_selected())
    } else {
        container(chip)
    };
    let mut cell = row![chip].spacing(2).align_y(iced::Alignment::Center);
    if let Some(close_msg) = close {
        cell = cell.push(
            button(text("✕").size(11))
                .on_press(close_msg)
                .style(style::button_style(style::Btn::Ghost))
                .padding([6, 4]),
        );
    }
    cell.into()
}

async fn pick_nbt_file() -> Option<std::path::PathBuf> {
    rfd::AsyncFileDialog::new()
        .add_filter("NBT documents (dat, nbt, mca…)", &["dat", "nbt", "mcr", "mca", "level"])
        .set_title("Open an NBT file")
        .pick_file()
        .await
        .map(|h| h.path().to_path_buf())
}

fn handle_hotkey(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    use iced::keyboard::{Key, Modifiers};
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event
    else {
        return None;
    };
    if status == iced::event::Status::Captured {
        return None;
    }
    let ctrl = modifiers.contains(Modifiers::CTRL) && !modifiers.contains(Modifiers::SHIFT);
    let ctrl_shift = modifiers.contains(Modifiers::CTRL) && modifiers.contains(Modifiers::SHIFT);
    let char_key = |c: &str| key == Key::Character(c.into());
    if ctrl && (char_key("z") || char_key("Z")) {
        return Some(Message::HotkeyUndo);
    }
    if (ctrl_shift && (char_key("z") || char_key("Z"))) || (ctrl && (char_key("y") || char_key("Y"))) {
        return Some(Message::HotkeyRedo);
    }
    if ctrl && (char_key("s") || char_key("S")) {
        return Some(Message::HotkeySave);
    }
    if ctrl && (char_key("f") || char_key("F")) {
        return Some(Message::FocusSearch);
    }
    None
}

fn version_label(w: &WorldSummary) -> String {
    if let Some(dv) = w.java_data_version {
        match dictionary::data_version_name(i64::from(dv)) {
            Some(name) => format!("{name} · DataVersion {dv}"),
            None => format!("DataVersion {dv}"),
        }
    } else if let Some(v) = w.bedrock_last_opened {
        format!("{}.{}.{}", v[0], v[1], v[2])
    } else {
        "unknown version".into()
    }
}

fn omnitheme() -> Theme {
    Theme::custom(
        "OmniMCTools",
        iced::theme::Palette {
            background: style::BG,
            text: style::FG,
            primary: style::ACCENT,
            success: style::SUCCESS,
            warning: style::WARN,
            danger: style::DANGER,
        },
    )
}
