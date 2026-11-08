//! `omt-gui` — OmniMCTools desktop application (Iced 0.14).
//!
//! Shell: Home (world discovery) + one tab per open NBT document. All editor
//! logic lives in `editor` (state) and `editor_view` (rendering); this module
//! only routes messages.

mod editor;
mod editor_view;

use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Color, Element, Length, Subscription, Task, Theme};
use omt_world::WorldSummary;

use editor::{EditorMsg, EditorTab};

fn main() -> iced::Result {
    iced::application(OmniMCTools::new, OmniMCTools::update, OmniMCTools::view)
        .title(|_s: &OmniMCTools| "OmniMCTools".to_string())
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
        // Tab bar (Home + documents).
        let mut tab_bar = row![button("Home").on_press(Message::SelectTab(usize::MAX)).padding(6)].spacing(4);
        for (i, tab) in self.tabs.iter().enumerate() {
            let dirty = if tab.is_dirty() { " ●" } else { "" };
            let label = format!("{}{dirty}  ✕", tab.title);
            let active = self.active == Some(i);
            let tab_button = button(
                row![
                    text(label.clone()).size(13),
                ]
                .spacing(4),
            )
            .on_press(Message::SelectTab(i))
            .padding(6);
            let tab_button = if active {
                container(tab_button).style(|_t: &Theme| container::Style {
                    background: Some(Color::from_rgb(0.14, 0.17, 0.22).into()),
                    border: iced::border::rounded(4),
                    ..Default::default()
                })
            } else {
                container(tab_button)
            };
            tab_bar = tab_bar.push(
                row![
                    tab_button,
                    button("✕").on_press(Message::CloseTab(i)).padding(4),
                ]
                .spacing(2),
            );
        }

        let body: Element<'_, Message> = match self.active {
            Some(i) if i < self.tabs.len() => editor_view::view(&self.tabs[i], i),
            _ => self.home(),
        };

        column![
            tab_bar,
            iced::widget::rule::horizontal(1.0),
            container(body).height(Length::Fill),
            iced::widget::rule::horizontal(1.0),
            row![
                text(self.status.clone()).size(12).color(MUTED),
                text("Ctrl+Z undo · Ctrl+Y redo · Ctrl+S save · Ctrl+F find").size(12).color(MUTED),
            ]
            .spacing(16),
        ]
        .padding(8)
        .spacing(6)
        .into()
    }

    fn home(&self) -> Element<'_, Message> {
        let header = row![
            text("OmniMCTools").size(26),
            text("Minecraft world tools — Java + Bedrock, latest stable").size(13).color(MUTED),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);

        let actions = row![
            button("Open NBT / level.dat file…").on_press(Message::PickFile),
            button("Rescan worlds").on_press(Message::Refresh),
        ]
        .spacing(10);

        let worlds = if self.worlds.is_empty() {
            column![text("No worlds found — set OMT_WORLDS_DIR, or open files directly.").color(MUTED)]
        } else {
            self.worlds
                .iter()
                .filter_map(|w| {
                    let level = w.root.join("level.dat");
                    level.is_file().then_some(
                        row![
                            text(w.name.clone()).size(15),
                            text(w.edition.name()).size(13).color(MUTED),
                            text(version_label(w)).size(13).color(MUTED),
                            button("Edit level.dat").on_press({
                                // Route through FilePicked to reuse open_tab.
                                Message::FilePicked(Some(level.clone()))
                            }),
                        ]
                        .spacing(16)
                        .align_y(iced::Alignment::Center),
                    )
                })
                .fold(iced::widget::Column::new().spacing(6), |c, r| c.push(r))
        };

        column![
            header,
            iced::widget::rule::horizontal(1.0),
            actions,
            text("Worlds").size(18),
            container(scrollable(worlds)).height(Length::Fill),
        ]
        .spacing(12)
        .into()
    }
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
        format!("DataVersion {dv}")
    } else if let Some(v) = w.bedrock_last_opened {
        format!("{}.{}.{}", v[0], v[1], v[2])
    } else {
        "unknown version".into()
    }
}

// Port-guide §3.1 tokens: window #1d2128, panels #232830, accents
// #228bf6 / #43a975 / #c04040.
pub(crate) const BG: Color = Color::from_rgb(0x1d as f32 / 255.0, 0x21 as f32 / 255.0, 0x28 as f32 / 255.0);
const FG: Color = Color::from_rgb(0xe8 as f32 / 255.0, 0xea as f32 / 255.0, 0xed as f32 / 255.0);
pub(crate) const ACCENT: Color = Color::from_rgb(0x22 as f32 / 255.0, 0x8b as f32 / 255.0, 0xf6 as f32 / 255.0);
const SUCCESS: Color = Color::from_rgb(0x43 as f32 / 255.0, 0xa9 as f32 / 255.0, 0x75 as f32 / 255.0);
const WARN: Color = Color::from_rgb(0xe0 as f32 / 255.0, 0xa0 as f32 / 255.0, 0x30 as f32 / 255.0);
const DANGER: Color = Color::from_rgb(0xc0 as f32 / 255.0, 0x40 as f32 / 255.0, 0x40 as f32 / 255.0);
pub(crate) const MUTED: Color = Color::from_rgb(0x9a as f32 / 255.0, 0xa1 as f32 / 255.0, 0xad as f32 / 255.0);

/// Added tags.
pub(crate) const ADDED: Color = SUCCESS;
/// Edited tags.
pub(crate) const EDITED: Color = WARN;
/// Current search hit.
pub(crate) const HIT: Color = ACCENT;

fn omnitheme() -> Theme {
    Theme::custom(
        "OmniMCTools",
        iced::theme::Palette {
            background: BG,
            text: FG,
            primary: ACCENT,
            success: SUCCESS,
            warning: WARN,
            danger: DANGER,
        },
    )
}
