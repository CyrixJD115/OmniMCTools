//! `omt-gui` — OmniMCTools desktop application (Iced 0.14).
//!
//! Shell (UMT layout): Home with the three app tiles → pick a world →
//! a *session* tab per opened world (NBT Editor / Converter / Pruner) that
//! works on a workspace copy with Save / Save As / Close, plus standalone
//! NBT file tabs and a Settings page. This module only routes messages.

mod converter;
mod dictionary;
mod editor;
mod editor_view;
mod extensions;
mod home;
mod ldb_view;
mod locator;
mod mapdata;
mod mapview;
mod nbt_session;
mod pruner;
mod rows;
mod session;
mod settings;
mod settings_view;
mod style;
mod world_settings;
mod worlds;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Color, Element, Length, Subscription, Task};
use omt_script::Manager;

use converter::{ConvMsg, ConverterState};
use editor::{EditorMsg, EditorTab};
use extensions::ExtMsg;
use nbt_session::{NbtMsg, NbtSession};
use pruner::{PrunerMsg, PrunerState};
use session::{App, WorldSession, blocking};
use settings::Settings;
use settings_view::SettingsMsg;

pub(crate) use style::{ACCENT, MUTED};
/// Added tags.
pub(crate) const ADDED: Color = style::SUCCESS;
/// Edited tags.
pub(crate) const EDITED: Color = style::WARN;
/// Current search hit.
pub(crate) const HIT: Color = style::ACCENT;

fn main() -> iced::Result {
    iced::application(OmniMCTools::new, OmniMCTools::update, OmniMCTools::view)
        .title(OmniMCTools::title)
        .theme(|_s: &OmniMCTools| style::omnitheme())
        .scale_factor(|s: &OmniMCTools| s.settings.scale_factor)
        .subscription(OmniMCTools::subscription)
        .window(iced::window::Settings { size: iced::Size::new(1280.0, 800.0), ..Default::default() })
        .run()
}

/// Session payload carried through messages (sessions are not `Clone`).
type Slot<T> = Arc<Mutex<Option<T>>>;

fn slot<T>(v: T) -> Slot<T> {
    Arc::new(Mutex::new(Some(v)))
}

fn take<T>(s: &Slot<T>) -> Option<T> {
    s.lock().ok().and_then(|mut g| g.take())
}

#[derive(Debug, Clone)]
pub enum Message {
    // Home / navigation
    ChooseApp(App),
    Rescan,
    Scanned(Vec<worlds::WorldEntry>),
    OpenWorld(App, PathBuf),
    BrowseFolder(App),
    BrowseArchive(App),
    Picked(App, Option<PathBuf>),
    SessionOpened(App, PathBuf, Result<Slot<WorldSession>, String>),
    GoHome,
    OpenSettings,
    OpenExtensions,
    Ext(ExtMsg),
    Settings(SettingsMsg),
    // Standalone NBT files
    PickFile,
    FilePicked(Option<PathBuf>),
    SelectDoc(usize),
    CloseDoc(usize),
    Doc(usize, EditorMsg),
    // World sessions
    SelectSession(usize),
    Nbt(usize, NbtMsg),
    Prune(usize, PrunerMsg),
    Conv(usize, ConvMsg),
    SaveSession(usize),
    SaveAsSession(usize),
    SaveAsPicked(usize, Option<PathBuf>),
    SessionSaved(usize, Slot<WorldSession>, Result<String, String>),
    CloseSession(usize),
    // Hotkeys
    FocusSearch,
    HotkeyUndo,
    HotkeyRedo,
    HotkeySave,
}

enum Ui {
    Nbt(Box<NbtSession>),
    Pruner(Box<PrunerState>),
    Converter(Box<ConverterState>),
}

struct SessionTab {
    /// `None` while a background save holds the session.
    sess: Option<WorldSession>,
    ui: Ui,
    title: String,
    app: App,
    busy: Option<String>,
    confirm_close: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Settings,
    Extensions,
    Doc(usize),
    Session(usize),
}

struct OmniMCTools {
    settings: Settings,
    settings_draft: Settings,
    /// App-wide Lua extension host state (see `omt-script`).
    ext: Arc<Manager>,
    ext_status: String,
    worlds: Vec<worlds::WorldEntry>,
    scanning: bool,
    opening: bool,
    picker: Option<App>,
    docs: Vec<EditorTab>,
    sessions: Vec<SessionTab>,
    screen: Screen,
    status: String,
}

impl OmniMCTools {
    fn new() -> (Self, Task<Message>) {
        let settings = Settings::load();
        let ext_root = settings::config_dir().join("extensions");
        let mut app = Self {
            settings_draft: settings.clone(),
            ext: Arc::new(Manager::open(&ext_root)),
            ext_status: String::new(),
            settings,
            worlds: Vec::new(),
            scanning: false,
            opening: false,
            picker: None,
            docs: Vec::new(),
            sessions: Vec::new(),
            screen: Screen::Home,
            status: "ready — everything stays on this computer".into(),
        };
        // `omt-gui [FILE…]` opens each path as a document tab;
        // `omt-gui --app nbt|pruner|converter WORLD` opens a world session;
        // `omt-gui --extensions` opens the Extensions page directly.
        let mut args = std::env::args_os().skip(1);
        let mut launch = None;
        let mut open_extensions = false;
        while let Some(arg) = args.next() {
            if arg == "--app" {
                let app_kind = match args.next().and_then(|a| a.into_string().ok()).as_deref() {
                    Some("nbt") => App::NbtEditor,
                    Some("pruner") => App::Pruner,
                    Some("converter") => App::Converter,
                    other => {
                        app.status = format!("--app expects nbt, pruner or converter (got {other:?})");
                        continue;
                    }
                };
                if let Some(world) = args.next() {
                    launch = Some((app_kind, PathBuf::from(world)));
                }
                continue;
            }
            if arg == "--extensions" {
                open_extensions = true;
                continue;
            }
            let path = PathBuf::from(arg);
            match EditorTab::open(&path) {
                Ok(mut tab) => {
                    tab.set_extensions(app.ext.clone());
                    app.docs.push(tab);
                }
                Err(e) => app.status = format!("open failed: {e}"),
            }
        }
        if !app.docs.is_empty() {
            app.screen = Screen::Doc(0);
        }
        if open_extensions {
            app.screen = Screen::Extensions;
        }
        let scan = app.rescan();
        let open = match launch {
            Some((kind, world)) => app.open_session(kind, world),
            None => Task::none(),
        };
        (app, Task::batch([scan, open]))
    }

    fn title(&self) -> String {
        let t = match self.screen {
            Screen::Doc(i) => self.docs.get(i).map(|d| d.title.clone()),
            Screen::Session(i) => self.sessions.get(i).map(|s| format!("{} — {}", s.title, s.app.name())),
            Screen::Settings => Some("Settings".into()),
            Screen::Extensions => Some("Extensions".into()),
            Screen::Home => None,
        };
        match t {
            Some(t) => format!("{t} — OmniMCTools"),
            None => "OmniMCTools".into(),
        }
    }

    fn rescan(&mut self) -> Task<Message> {
        self.scanning = true;
        let folders = worlds::scan_folders(&self.settings);
        Task::perform(blocking(move || worlds::discover(&folders)), Message::Scanned)
    }

    fn open_session(&mut self, app: App, path: PathBuf) -> Task<Message> {
        if self.opening {
            return Task::none();
        }
        self.opening = true;
        self.status = format!("opening {} in the {}…", path.display(), app.name());
        let base = self.settings.workspace_base();
        let p2 = path.clone();
        Task::perform(blocking(move || WorldSession::open(app, &p2, &base).map(slot)), move |r| Message::SessionOpened(app, path.clone(), r))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ChooseApp(app) => {
                self.picker = Some(app);
                self.screen = Screen::Home;
            }
            Message::Rescan => return self.rescan(),
            Message::Scanned(w) => {
                self.scanning = false;
                self.status = format!("{} world(s) found", w.len());
                self.worlds = w;
            }
            Message::OpenWorld(app, p) => return self.open_session(app, p),
            Message::BrowseFolder(app) => {
                return Task::perform(
                    async { rfd::AsyncFileDialog::new().set_title("Open a world folder").pick_folder().await.map(|h| h.path().to_path_buf()) },
                    move |p| Message::Picked(app, p),
                );
            }
            Message::BrowseArchive(app) => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Open a world archive")
                            .add_filter("World archives", &["mcworld", "mctemplate", "zip"])
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    move |p| Message::Picked(app, p),
                );
            }
            Message::Picked(app, Some(p)) => return self.open_session(app, p),
            Message::Picked(_, None) => {}
            Message::SessionOpened(app, path, res) => {
                self.opening = false;
                match res.map(|s| take(&s)) {
                    Ok(Some(sess)) => {
                        self.settings.push_recent(path);
                        let _ = self.settings.save();
                        self.settings_draft.recent = self.settings.recent.clone();
                        return self.add_session(app, sess);
                    }
                    Ok(None) => {}
                    Err(e) => self.status = format!("could not open the world: {e}"),
                }
            }
            Message::GoHome => self.screen = Screen::Home,
            Message::OpenSettings => {
                self.settings_draft = self.settings.clone();
                self.screen = Screen::Settings;
            }
            Message::OpenExtensions => {
                self.ext.reload();
                self.ext_status.clear();
                self.screen = Screen::Extensions;
            }
            Message::Ext(m) => return self.on_ext(m),
            Message::Settings(m) => return self.on_settings(m),
            Message::PickFile => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("NBT documents (dat, nbt, mca…)", &["dat", "nbt", "dat_old", "mcstructure", "schematic", "litematic"])
                            .set_title("Open an NBT file")
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::FilePicked,
                );
            }
            Message::FilePicked(Some(path)) => match EditorTab::open(&path) {
                Ok(mut tab) => {
                    tab.set_extensions(self.ext.clone());
                    self.docs.push(tab);
                    self.screen = Screen::Doc(self.docs.len() - 1);
                    self.status = format!("opened {}", path.display());
                }
                Err(e) => self.status = format!("open failed: {e}"),
            },
            Message::FilePicked(None) => {}
            Message::SelectDoc(i) => self.screen = Screen::Doc(i),
            Message::CloseDoc(i) => {
                if i < self.docs.len() {
                    if self.docs[i].is_dirty() && !self.docs[i].status.starts_with("unsaved") {
                        self.docs[i].status = "unsaved changes — press ✕ again to discard them".into();
                        self.screen = Screen::Doc(i);
                        return Task::none();
                    }
                    self.docs.remove(i);
                    self.screen = Screen::Home;
                }
            }
            Message::Doc(i, m) => {
                if let Some(tab) = self.docs.get_mut(i) {
                    let tasks = tab.update(m);
                    self.status = tab.status.clone();
                    return Task::batch(tasks.into_iter().map(move |t| t.map(move |m| Message::Doc(i, m))));
                }
            }
            Message::SelectSession(i) => self.screen = Screen::Session(i),
            Message::Nbt(i, m) => {
                if let Some(SessionTab { sess: Some(sess), ui: Ui::Nbt(ui), .. }) = self.sessions.get_mut(i) {
                    return ui.update(sess, m).map(move |m| Message::Nbt(i, m));
                }
            }
            Message::Prune(i, m) => {
                if let Some(SessionTab { sess: Some(sess), ui: Ui::Pruner(ui), .. }) = self.sessions.get_mut(i) {
                    return ui.update(sess, m).map(move |m| Message::Prune(i, m));
                }
            }
            Message::Conv(i, m) => {
                let base = self.settings.workspace_base();
                if let Some(SessionTab { sess: Some(sess), ui: Ui::Converter(ui), .. }) = self.sessions.get_mut(i) {
                    return ui.update(sess, base, m).map(move |m| Message::Conv(i, m));
                }
            }
            Message::SaveSession(i) => return self.save_session(i, None),
            Message::SaveAsSession(i) => {
                let bedrock = self.sessions.get(i).and_then(|s| s.sess.as_ref()).is_some_and(|s| s.edition() == omt_world::Edition::Bedrock);
                return Task::perform(
                    async move {
                        let d = rfd::AsyncFileDialog::new().set_title("Save a copy of the world");
                        if bedrock {
                            d.add_filter("Bedrock world", &["mcworld"]).save_file().await
                        } else {
                            d.save_file().await
                        }
                        .map(|h| h.path().to_path_buf())
                    },
                    move |p| Message::SaveAsPicked(i, p),
                );
            }
            Message::SaveAsPicked(i, Some(dest)) => return self.save_session(i, Some(dest)),
            Message::SaveAsPicked(_, None) => {}
            Message::SessionSaved(i, s, res) => {
                if let Some(tab) = self.sessions.get_mut(i) {
                    tab.sess = take(&s);
                    tab.busy = None;
                    self.status = match res {
                        Ok(m) => m,
                        Err(e) => format!("save failed: {e}"),
                    };
                    // The store was reopened: the Pruner/Locator map tiles
                    // still reflect the saved state, nothing to reload.
                }
            }
            Message::CloseSession(i) => {
                if let Some(tab) = self.sessions.get_mut(i) {
                    let unsaved_docs = match &tab.ui {
                        Ui::Nbt(n) => n.dirty_docs(),
                        _ => 0,
                    };
                    let modified = tab.sess.as_ref().is_some_and(|s| s.modified) || unsaved_docs > 0;
                    if tab.busy.is_some() {
                        self.status = "wait for the current job to finish".into();
                        return Task::none();
                    }
                    if modified && !tab.confirm_close {
                        tab.confirm_close = true;
                        self.status = "this world has unsaved changes — press Close again to discard them".into();
                        self.screen = Screen::Session(i);
                        return Task::none();
                    }
                    let tab = self.sessions.remove(i);
                    if let Some(sess) = tab.sess {
                        // Deleting a large workspace can take a while.
                        std::thread::spawn(move || sess.close());
                    }
                    self.screen = Screen::Home;
                    self.status = "world closed (workspace copy discarded)".into();
                }
            }
            Message::FocusSearch => return iced::widget::operation::focus(editor_view::search_id()),
            Message::HotkeyUndo | Message::HotkeyRedo | Message::HotkeySave => {
                let em = match message {
                    Message::HotkeyUndo => EditorMsg::Undo,
                    Message::HotkeyRedo => EditorMsg::Redo,
                    _ => EditorMsg::Save,
                };
                match self.screen {
                    Screen::Doc(i) => return self.update(Message::Doc(i, em)),
                    Screen::Session(i) => {
                        if let Some(SessionTab { ui: Ui::Nbt(ui), .. }) = self.sessions.get(i)
                            && let Some(d) = ui.active_doc()
                        {
                            return self.update(Message::Nbt(i, NbtMsg::Doc(d, em)));
                        }
                    }
                    _ => {}
                }
            }
        }
        Task::none()
    }

    fn add_session(&mut self, app: App, sess: WorldSession) -> Task<Message> {
        let i = self.sessions.len();
        let title = sess.title.clone();
        let (ui, task) = match app {
            App::NbtEditor => (Ui::Nbt(Box::new(NbtSession::new(&sess, self.settings.nether_slicer, self.ext.clone()))), Task::none()),
            App::Pruner => {
                let mut p = PrunerState::new(0);
                let t = p.load(&sess).map(move |m| Message::Prune(i, m));
                (Ui::Pruner(Box::new(p)), t)
            }
            App::Converter => (Ui::Converter(Box::new(ConverterState::new(&sess, self.settings.skip_partial_chunks))), Task::none()),
        };
        self.sessions.push(SessionTab { sess: Some(sess), ui, title, app, busy: None, confirm_close: false });
        self.screen = Screen::Session(i);
        self.status = format!("opened in the {} — you are editing a copy; Save writes it back", app.name());
        task
    }

    fn save_session(&mut self, i: usize, dest: Option<PathBuf>) -> Task<Message> {
        let Some(tab) = self.sessions.get_mut(i) else { return Task::none() };
        let Some(mut sess) = tab.sess.take() else { return Task::none() };
        tab.busy = Some(if dest.is_some() { "Exporting…".into() } else { "Saving…".into() });
        tab.confirm_close = false;
        Task::perform(
            blocking(move || {
                let res = match &dest {
                    Some(d) => sess.save_as(d).map(|()| format!("exported a copy to {}", d.display())),
                    None => sess.save().map(|b| format!("saved — previous version kept at {}", b.display())),
                };
                (slot(sess), res)
            }),
            move |(s, r)| Message::SessionSaved(i, s, r),
        )
    }

    /// Extensions-page message handling. Mutations bump the manager
    /// revision, so every open editor tab re-runs its extension pass on the
    /// next rebuild.
    fn on_ext(&mut self, m: ExtMsg) -> Task<Message> {
        match &m {
            ExtMsg::Back => {
                self.screen = Screen::Home;
                return Task::none();
            }
            ExtMsg::PickZip => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Install an extension")
                            .add_filter("OMT extension", &["zip"])
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    |p| Message::Ext(ExtMsg::ZipPicked(p)),
                );
            }
            ExtMsg::ZipPicked(None) => return Task::none(),
            _ => {}
        }
        let (status, _changed) = extensions::handle(&self.ext, &m);
        if !status.is_empty() {
            self.ext_status = status;
        }
        Task::none()
    }

    fn on_settings(&mut self, m: SettingsMsg) -> Task<Message> {
        match m {
            SettingsMsg::Back => self.screen = Screen::Home,
            SettingsMsg::Save => {
                self.settings = self.settings_draft.clone();
                self.status = match self.settings.save() {
                    Ok(()) => "settings saved".into(),
                    Err(e) => format!("could not save settings: {e}"),
                };
                return self.rescan();
            }
            SettingsMsg::PickWorkingDir => {
                return Task::perform(
                    async { rfd::AsyncFileDialog::new().set_title("Working directory").pick_folder().await.map(|h| h.path().to_path_buf()) },
                    |p| Message::Settings(SettingsMsg::WorkingDirPicked(p)),
                );
            }
            SettingsMsg::AddFolder => {
                return Task::perform(
                    async { rfd::AsyncFileDialog::new().set_title("Add a worlds folder").pick_folder().await.map(|h| h.path().to_path_buf()) },
                    |p| Message::Settings(SettingsMsg::FolderPicked(p)),
                );
            }
            other => {
                settings_view::apply(&mut self.settings_draft, &other);
                // Scale previews live.
                if let SettingsMsg::Scale(v) = other {
                    self.settings.scale_factor = v;
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(handle_hotkey)
    }

    fn view(&self) -> Element<'_, Message> {
        column![self.tab_bar(), container(self.body()).height(Length::Fill), self.status_bar()].width(Length::Fill).into()
    }

    fn tab_bar(&self) -> Element<'_, Message> {
        let mut bar = row![row![text("Omni").size(17).color(style::ACCENT), text("MCTools").size(17)].padding([0, 6])]
            .spacing(8)
            .align_y(iced::Alignment::Center);
        bar = bar.push(tab_chip("🏠 Home", self.screen == Screen::Home, false, Message::GoHome, None));
        for (i, s) in self.sessions.iter().enumerate() {
            let dirty = s.sess.as_ref().is_some_and(|x| x.modified);
            let icon = match s.app {
                App::NbtEditor => "🗂",
                App::Converter => "⇄",
                App::Pruner => "✂",
            };
            bar = bar.push(tab_chip(&format!("{icon} {}", s.title), self.screen == Screen::Session(i), dirty, Message::SelectSession(i), Some(Message::CloseSession(i))));
        }
        for (i, d) in self.docs.iter().enumerate() {
            bar = bar.push(tab_chip(&format!("📄 {}", d.title), self.screen == Screen::Doc(i), d.is_dirty(), Message::SelectDoc(i), Some(Message::CloseDoc(i))));
        }
        container(scrollable(container(bar).padding([8, 12])).direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(4).scroller_width(4))))
            .style(|_| container::Style { background: Some(iced::Background::Color(style::BG)), ..Default::default() })
            .into()
    }

    fn body(&self) -> Element<'_, Message> {
        match self.screen {
            Screen::Home => home::view(&self.worlds, self.picker, self.scanning, self.opening, &self.settings.recent),
            Screen::Settings => settings_view::view(&self.settings_draft, self.settings_draft != self.settings).map(Message::Settings),
            Screen::Extensions => extensions::view(&self.ext, &self.ext_status),
            Screen::Doc(i) => match self.docs.get(i) {
                Some(tab) => container(editor_view::view(tab, std::rc::Rc::new(move |m| Message::Doc(i, m)))).padding(8).into(),
                None => space::vertical().into(),
            },
            Screen::Session(i) => match self.sessions.get(i) {
                Some(tab) => self.session_view(i, tab),
                None => space::vertical().into(),
            },
        }
    }

    fn session_view<'a>(&'a self, i: usize, tab: &'a SessionTab) -> Element<'a, Message> {
        let Some(sess) = tab.sess.as_ref() else {
            return container(text(tab.busy.clone().unwrap_or_default()).color(style::WARN)).center(Length::Fill).into();
        };
        let modified = if sess.modified { text("● unsaved changes").size(12).color(style::WARN) } else { text("no unsaved changes").size(12).color(style::MUTED) };
        let header = container(
            row![
                column![
                    text(format!("{} — {}", tab.title, tab.app.name())).size(16),
                    text(format!("{} · {} · editing a copy of {}", sess.edition().name(), style::version_label(&sess.summary), sess.origin().display())).size(11).color(style::MUTED),
                ]
                .spacing(2),
                space::horizontal(),
                modified,
                button(text("Save").size(13)).on_press_maybe((tab.app != App::Converter && tab.busy.is_none()).then_some(Message::SaveSession(i))).style(style::button_style(style::Btn::Confirm)),
                button(text("Save As…").size(13)).on_press_maybe((tab.app != App::Converter && tab.busy.is_none()).then_some(Message::SaveAsSession(i))).style(style::button_style(style::Btn::Standard)),
                button(text("Close").size(13)).on_press(Message::CloseSession(i)).style(style::button_style(style::Btn::Danger)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        )
        .padding([8, 12])
        .style(style::panel());
        let body: Element<'a, Message> = match &tab.ui {
            Ui::Nbt(ui) => ui.view(sess).map(move |m| Message::Nbt(i, m)),
            Ui::Pruner(ui) => ui.view(sess).map(move |m| Message::Prune(i, m)),
            Ui::Converter(ui) => ui.view(sess).map(move |m| Message::Conv(i, m)),
        };
        let status = match &tab.ui {
            Ui::Pruner(p) => p.status.clone(),
            Ui::Nbt(n) => n.status.clone(),
            Ui::Converter(_) => String::new(),
        };
        let mut col = column![header, container(body).height(Length::Fill)];
        if !status.is_empty() {
            col = col.push(container(text(status).size(12).color(style::MUTED)).padding([4, 12]));
        }
        col.into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        row![
            text(self.status.clone()).size(12).color(MUTED),
            space::horizontal(),
            text("Ctrl+Z undo · Ctrl+Y redo · Ctrl+S save · Ctrl+F find · offline, no account").size(12).color(style::HINT),
        ]
        .padding([6, 12])
        .spacing(16)
        .into()
    }
}

/// Tab-bar chip with an optional ✕ close button.
fn tab_chip(title: &str, active: bool, dirty: bool, select: Message, close: Option<Message>) -> Element<'static, Message> {
    let dot = if dirty { " ●" } else { "" };
    let chip = button(text(format!("{title}{dot}")).size(13)).on_press(select).style(style::button_style(style::Btn::Ghost)).padding([6, 10]);
    let chip: Element<'static, Message> = if active { container(chip).style(style::card_selected()).into() } else { container(chip).into() };
    let mut cell = row![chip].spacing(2).align_y(iced::Alignment::Center);
    if let Some(close_msg) = close {
        cell = cell.push(button(text("✕").size(11)).on_press(close_msg).style(style::button_style(style::Btn::Ghost)).padding([6, 4]));
    }
    cell.into()
}

fn handle_hotkey(event: iced::Event, status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    use iced::keyboard::{Key, Modifiers};
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
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
