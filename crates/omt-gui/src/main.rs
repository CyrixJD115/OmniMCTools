//! `omt-gui` — OmniMCTools desktop application (Iced 0.14).
//!
//! Frame: top bar + collapsible side rail + page + status bar (`shell`).
//! Pages: Home dashboard, the Worlds library (filters, search syntax),
//! tool landing pages (Map Viewer, NBT Editor, Converter, Pruner), open
//! world sessions and NBT file tabs, Settings, Extensions. Editors work on
//! a workspace copy (Save / Save As / Close); the Map Viewer opens worlds
//! in place, read-only. This module owns state and routes messages.

mod appearance;
mod assets;
mod converter;
mod dictionary;
mod editor;
mod editor_view;
mod extensions;
mod filter;
mod handlers;
mod home;
mod ldb_view;
mod library;
mod locator;
mod mapconfig;
mod mapdata;
mod maplook;
mod maprail;
mod maptile;
mod mapview;
mod mapview_overlay;
mod panelui;
mod chunkread;
mod nbt_session;
mod pruner;
mod rows;
mod session;
mod settings;
mod settings_view;
mod shell;
mod sources;
mod style;
mod textures;
mod viewer;
mod viewer_ext;
mod viewer_highlight;
mod viewer_info;
mod viewer_inspect;
mod viewer_presets;
mod viewer_rules;
mod viewer_3d;
mod viewer_3d_ctl;
mod viewer_3d_input;
mod viewer_blocks;
mod viewer_export;
mod viewer_tex;
mod viewer_view;
mod world_edit;
mod world_edit_3d;
mod world_edit_view;
mod world_settings;
mod worlds;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::widget::{column, container, row, space, text};
use iced::{Alignment, Element, Length, Subscription, Task};
use omt_script::Manager;

use converter::{ConvMsg, ConverterState};
use editor::{EditorMsg, EditorTab};
use extensions::ExtMsg;
use library::{LibMsg, Library};
use nbt_session::{NbtMsg, NbtSession};
use pruner::{PrunerMsg, PrunerState};
use session::{App, WorldSession, blocking};
use settings::Settings;
use settings_view::SettingsMsg;

use viewer::{ViewerMsg, ViewerState};

fn main() -> iced::Result {
    iced::application(OmniMCTools::new, OmniMCTools::update, OmniMCTools::view)
        .title(OmniMCTools::title)
        .theme(|_s: &OmniMCTools| style::omnitheme())
        .scale_factor(|s: &OmniMCTools| s.settings.scale_factor)
        .subscription(OmniMCTools::subscription)
        .window(iced::window::Settings { size: iced::Size::new(1440.0, 900.0), min_size: Some(iced::Size::new(960.0, 600.0)), icon: app_icon(), ..Default::default() })
        .run()
}

fn app_icon() -> Option<iced::window::Icon> {
    let img = omt_art::app_icon(64);
    iced::window::icon::from_rgba(img.rgba, img.w, img.h).ok()
}

/// Session payload carried through messages (sessions are not `Clone`).
type Slot<T> = Arc<Mutex<Option<T>>>;

fn slot<T>(v: T) -> Slot<T> {
    Arc::new(Mutex::new(Some(v)))
}

fn take<T>(s: &Slot<T>) -> Option<T> {
    s.lock().ok().and_then(|mut g| g.take())
}

/// Every navigable page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Library,
    Tool(App),
    Settings,
    Extensions,
    Doc(usize),
    Session(usize),
}

#[derive(Debug, Clone)]
pub enum Message {
    Nav(Page),
    ToggleRail,
    SetTheme(style::ThemeKind),
    GlobalSearch(String),
    Lib(LibMsg),
    LibraryLauncher(sources::Launcher),
    Tex(appearance::TexMsg),
    Rescan,
    Scanned(Vec<worlds::WorldEntry>),
    OpenWorld(App, PathBuf),
    BrowseFolder(App),
    BrowseArchive(App),
    Picked(App, Option<PathBuf>),
    SessionOpened(App, PathBuf, Result<Slot<WorldSession>, String>),
    OpenSettings,
    Ext(ExtMsg),
    Settings(SettingsMsg),
    // Standalone NBT files
    PickFile,
    FilePicked(Option<PathBuf>),
    CloseDoc(usize),
    Doc(usize, EditorMsg),
    // World sessions
    Nbt(usize, NbtMsg),
    Prune(usize, PrunerMsg),
    Conv(usize, ConvMsg),
    View(usize, ViewerMsg),
    Edit(usize, world_edit::EditMsg),
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
    /// Map Viewer keyboard navigation (arrows pan, +/− zoom, * reset, Ctrl+G).
    ViewerKey(crate::viewer::ViewerKey),
}

enum Ui {
    Nbt(Box<NbtSession>),
    Pruner(Box<PrunerState>),
    Converter(Box<ConverterState>),
    Viewer(Box<ViewerState>),
    Editor(Box<world_edit::EditorState>),
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

struct OmniMCTools {
    settings: Settings,
    settings_draft: Settings,
    /// App-wide Lua extension host state (see `omt-script`).
    ext: Arc<Manager>,
    ext_status: String,
    worlds: Vec<worlds::WorldEntry>,
    library: Library,
    scanning: bool,
    opening: bool,
    docs: Vec<EditorTab>,
    sessions: Vec<SessionTab>,
    screen: Page,
    status: String,
    tex: appearance::TexState,
}

impl OmniMCTools {
    fn new() -> (Self, Task<Message>) {
        let settings = Settings::load();
        style::set_theme(settings.theme);
        mapdata::set_config(settings.map_config());
        let ext_root = settings::config_dir().join("extensions");
        let mut app = Self {
            settings_draft: settings.clone(),
            ext: Arc::new(Manager::open(&ext_root)),
            ext_status: String::new(),
            settings,
            worlds: Vec::new(),
            library: Library::default(),
            scanning: false,
            opening: false,
            docs: Vec::new(),
            sessions: Vec::new(),
            screen: Page::Home,
            status: "ready — everything stays on this computer".into(),
            tex: appearance::TexState::default(),
        };
        // `omt-gui [FILE…]` opens each path as a document tab;
        // `omt-gui --app map|nbt|pruner|converter WORLD` opens a world;
        // `omt-gui --page worlds|settings|extensions|map|…` opens a page.
        let mut args = std::env::args_os().skip(1);
        let mut launch = None;
        while let Some(arg) = args.next() {
            if arg == "--app" {
                let kind = match args.next().and_then(|a| a.into_string().ok()).as_deref() {
                    Some("map" | "viewer") => App::MapViewer,
                    Some("edit" | "editor") => App::WorldEditor,
                    Some("nbt") => App::NbtEditor,
                    Some("pruner") => App::Pruner,
                    Some("converter") => App::Converter,
                    other => {
                        app.status = format!("--app expects map, nbt, pruner or converter (got {other:?})");
                        continue;
                    }
                };
                if let Some(world) = args.next() {
                    launch = Some((kind, PathBuf::from(world)));
                }
                continue;
            }
            if arg == "--extensions" {
                app.screen = Page::Extensions;
                continue;
            }
            if arg == "--page" {
                app.screen = match args.next().and_then(|a| a.into_string().ok()).as_deref() {
                    Some("worlds" | "library") => Page::Library,
                    Some("settings") => Page::Settings,
                    Some("extensions") => Page::Extensions,
                    Some("map") => Page::Tool(App::MapViewer),
                    Some("editor") => Page::Tool(App::WorldEditor),
                    Some("nbt") => Page::Tool(App::NbtEditor),
                    Some("converter") => Page::Tool(App::Converter),
                    Some("pruner") => Page::Tool(App::Pruner),
                    _ => Page::Home,
                };
                continue;
            }
            let path = PathBuf::from(arg);
            match EditorTab::open(&path) {
                Ok(mut tab) => {
                    tab.set_extensions(app.ext.clone());
                    app.docs.push(tab);
                    app.screen = Page::Doc(app.docs.len() - 1);
                }
                Err(e) => app.status = format!("open failed: {e}"),
            }
        }
        let scan = app.rescan();
        let tex = app.load_textures();
        let open = match launch {
            Some((kind, world)) => app.open_session(kind, world),
            None => Task::none(),
        };
        (app, Task::batch([scan, tex, open]))
    }

    fn title(&self) -> String {
        let t = match self.screen {
            Page::Doc(i) => self.docs.get(i).map(|d| d.title.clone()),
            Page::Session(i) => self.sessions.get(i).map(|s| format!("{} — {}", s.title, s.app.name())),
            Page::Settings => Some("Settings".into()),
            Page::Extensions => Some("Extensions".into()),
            Page::Library => Some("Worlds".into()),
            Page::Tool(a) => Some(a.name().into()),
            Page::Home => None,
        };
        match t {
            Some(t) => format!("{t} — OmniMCTools"),
            None => "OmniMCTools".into(),
        }
    }

    fn rescan(&mut self) -> Task<Message> {
        self.scanning = true;
        let roots = worlds::scan_roots(&self.settings);
        Task::perform(blocking(move || worlds::discover(&roots)), Message::Scanned)
    }

    fn save_settings(&mut self) {
        // Keep the Settings page draft in step with changes made elsewhere.
        self.settings_draft.favorites = self.settings.favorites.clone();
        self.settings_draft.bookmarks = self.settings.bookmarks.clone();
        self.settings_draft.viewer = self.settings.viewer.clone();
        self.settings_draft.theme = self.settings.theme;
        self.settings_draft.rail_collapsed = self.settings.rail_collapsed;
        self.settings_draft.library_grid = self.settings.library_grid;
        self.settings_draft.library_sort = self.settings.library_sort;
        self.settings_draft.recent = self.settings.recent.clone();
        self.settings_draft.texture_pack = self.settings.texture_pack.clone();
        self.settings_draft.covers = self.settings.covers.clone();
        self.settings_draft.pruner_look = self.settings.pruner_look;
        if let Err(e) = self.settings.save() {
            self.status = format!("could not save settings: {e}");
        }
    }

    fn open_session(&mut self, app: App, path: PathBuf) -> Task<Message> {
        if self.opening {
            return Task::none();
        }
        // Already open in this app → just switch to it.
        if let Some(i) = self.sessions.iter().position(|s| s.app == app && s.sess.as_ref().is_some_and(|x| x.origin() == path)) {
            self.screen = Page::Session(i);
            return Task::none();
        }
        self.opening = true;
        self.status = if app.read_only() { format!("opening {} in the {}…", path.display(), app.name()) } else { format!("copying {} into a workspace for the {}…", path.display(), app.name()) };
        let base = self.settings.workspace_base();
        let p2 = path.clone();
        Task::perform(blocking(move || WorldSession::open(app, &p2, &base).map(slot)), move |r| Message::SessionOpened(app, path.clone(), r))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Nav(p) => {
                if p == Page::Settings {
                    self.settings_draft = self.settings.clone();
                }
                if p == Page::Extensions {
                    self.ext.reload();
                    self.ext_status.clear();
                }
                self.screen = p;
            }
            Message::ToggleRail => {
                self.settings.rail_collapsed = !self.settings.rail_collapsed;
                self.save_settings();
            }
            Message::SetTheme(t) => {
                self.settings.theme = t;
                style::set_theme(t);
                self.save_settings();
            }
            Message::GlobalSearch(q) => {
                self.library.update(LibMsg::Query(q), &mut self.settings);
                self.library.refresh(&self.worlds, &self.settings);
                if !matches!(self.screen, Page::Library | Page::Tool(_)) {
                    self.screen = Page::Library;
                }
            }
            Message::Lib(LibMsg::PickCover(world)) => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new().set_title("Choose a cover image").add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "gif"]).pick_file().await.map(|h| h.path().to_path_buf())
                    },
                    move |p| match p {
                        Some(img) => Message::Lib(LibMsg::SetCover(world.clone(), Some(img))),
                        None => Message::Picked(App::MapViewer, None),
                    },
                );
            }
            Message::Lib(m) => {
                if self.library.update(m, &mut self.settings) {
                    self.save_settings();
                }
                self.library.refresh(&self.worlds, &self.settings);
            }
            Message::LibraryLauncher(l) => {
                self.library.facets = filter::Facets::default();
                self.library.facets.launchers.insert(l);
                self.library.show_filters = true;
                self.library.refresh(&self.worlds, &self.settings);
                self.screen = Page::Library;
            }
            Message::Tex(m) => return self.on_tex(m),
            Message::Rescan => return self.rescan(),
            Message::Scanned(w) => {
                self.scanning = false;
                let launchers = w.iter().map(|e| e.launcher).collect::<std::collections::BTreeSet<_>>().len();
                self.status = format!("{} world(s) found across {launchers} launcher(s)", w.len());
                self.worlds = w;
                self.library.refresh(&self.worlds, &self.settings);
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
                        self.save_settings();
                        return self.add_session(app, sess);
                    }
                    Ok(None) => {}
                    Err(e) => self.status = format!("could not open the world: {e}"),
                }
            }
            Message::OpenSettings => return self.update(Message::Nav(Page::Settings)),
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
                    self.screen = Page::Doc(self.docs.len() - 1);
                    self.status = format!("opened {}", path.display());
                }
                Err(e) => self.status = format!("open failed: {e}"),
            },
            Message::FilePicked(None) => {}
            Message::CloseDoc(i) => {
                if i < self.docs.len() {
                    if self.docs[i].is_dirty() && !self.docs[i].status.starts_with("unsaved") {
                        self.docs[i].status = "unsaved changes — press ✕ again to discard them".into();
                        self.screen = Page::Doc(i);
                        return Task::none();
                    }
                    self.docs.remove(i);
                    self.screen = Page::Home;
                }
            }
            Message::Doc(i, m) => {
                if let Some(tab) = self.docs.get_mut(i) {
                    let tasks = tab.update(m);
                    self.status = tab.status.clone();
                    return Task::batch(tasks.into_iter().map(move |t| t.map(move |m| Message::Doc(i, m))));
                }
            }
            Message::Nbt(i, m) => {
                if let nbt_session::NbtMsg::Loc(locator::LocMsg::MapUi(u)) = &m {
                    self.apply_map_ui(*u);
                }
                if let Some(SessionTab { sess: Some(sess), ui: Ui::Nbt(ui), .. }) = self.sessions.get_mut(i) {
                    return ui.update(sess, m).map(move |m| Message::Nbt(i, m));
                }
            }
            Message::Prune(i, m) => {
                if let PrunerMsg::MapUi(u) = &m {
                    self.apply_map_ui(*u);
                }
                let look_change = matches!(m, PrunerMsg::Look(_));
                if let Some(SessionTab { sess: Some(sess), ui: Ui::Pruner(ui), .. }) = self.sessions.get_mut(i) {
                    let t = ui.update(sess, m).map(move |m| Message::Prune(i, m));
                    if look_change {
                        self.settings.pruner_look = ui.look;
                        self.save_settings();
                    }
                    return t;
                }
            }
            Message::Conv(i, m) => {
                let base = self.settings.workspace_base();
                if let Some(SessionTab { sess: Some(sess), ui: Ui::Converter(ui), .. }) = self.sessions.get_mut(i) {
                    return ui.update(sess, base, m).map(move |m| Message::Conv(i, m));
                }
            }
            Message::View(i, m) => {
                // "Open in <tool>" from the viewer's Tools menu becomes a
                // regular world open (before the viewer borrows anything).
                if let ViewerMsg::OpenIn(app) = &m {
                    if let Some(origin) = self.sessions.get(i).and_then(|t| t.sess.as_ref()).map(|s| s.origin()) {
                        return self.update(Message::OpenWorld(*app, origin));
                    }
                    return Task::none();
                }
                if let ViewerMsg::MapUi(u) = &m {
                    self.apply_map_ui(*u);
                }
                let marks_before = self.settings.bookmarks.len();
                let mut task = Task::none();
                let mut prefs = None;
                if let Some(SessionTab { sess: Some(sess), ui: Ui::Viewer(ui), .. }) = self.sessions.get_mut(i) {
                    task = ui.update(sess, &mut self.settings.bookmarks, m).map(move |m| Message::View(i, m));
                    prefs = Some(ui.prefs.clone());
                }
                if let Some(p) = prefs
                    && (p != self.settings.viewer || marks_before != self.settings.bookmarks.len())
                {
                    self.settings.viewer = p;
                    self.save_settings();
                }
                return task;
            }
            Message::Edit(i, m) => {
                if let Some(SessionTab { sess: Some(sess), ui: Ui::Editor(ui), .. }) = self.sessions.get_mut(i) {
                    return ui.update(sess, m).map(move |m| Message::Edit(i, m));
                }
            }
            Message::SaveSession(i) => return self.save_session(i, None),
            Message::SaveAsSession(i) => {
                let bedrock = self.sessions.get(i).and_then(|s| s.sess.as_ref()).is_some_and(|s| s.edition() == omt_world::Edition::Bedrock);
                return Task::perform(
                    async move {
                        let d = rfd::AsyncFileDialog::new().set_title("Save a copy of the world");
                        if bedrock { d.add_filter("Bedrock world", &["mcworld"]).save_file().await } else { d.save_file().await }.map(|h| h.path().to_path_buf())
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
                }
            }
            Message::CloseSession(i) => return self.close_session(i),
            Message::FocusSearch => return iced::widget::operation::focus(editor_view::search_id()),
            Message::ViewerKey(k) => {
                // Only the viewer session on screen reacts (uNmINeD-style
                // map navigation: arrows pan, +/− zoom, * resets, Ctrl+G).
                if let Page::Session(i) = self.screen
                    && matches!(self.sessions.get(i).map(|t| &t.ui), Some(Ui::Viewer(_)))
                {
                    return self.update(Message::View(i, crate::viewer::ViewerMsg::Key(k)));
                }
            }
            Message::HotkeyUndo | Message::HotkeyRedo | Message::HotkeySave => {
                let em = match message {
                    Message::HotkeyUndo => EditorMsg::Undo,
                    Message::HotkeyRedo => EditorMsg::Redo,
                    _ => EditorMsg::Save,
                };
                match self.screen {
                    Page::Doc(i) => return self.update(Message::Doc(i, em)),
                    Page::Session(i) => {
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

    fn close_session(&mut self, i: usize) -> Task<Message> {
        let Some(tab) = self.sessions.get_mut(i) else { return Task::none() };
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
            self.screen = Page::Session(i);
            return Task::none();
        }
        let tab = self.sessions.remove(i);
        let in_place = tab.sess.as_ref().is_some_and(|s| s.in_place);
        if let Some(sess) = tab.sess {
            // Deleting a large workspace can take a while.
            std::thread::spawn(move || sess.close());
        }
        // Indices after `i` shifted down.
        self.screen = match self.screen {
            Page::Session(j) if j > i => Page::Session(j - 1),
            Page::Session(_) => Page::Tool(tab.app),
            other => other,
        };
        self.status = if in_place { "world closed".into() } else { "world closed (workspace copy discarded)".into() };
        Task::none()
    }

    fn add_session(&mut self, app: App, sess: WorldSession) -> Task<Message> {
        let i = self.sessions.len();
        let title = sess.title.clone();
        let (ui, task) = match app {
            App::NbtEditor => (Ui::Nbt(Box::new(NbtSession::new(&sess, self.settings.nether_slicer, self.ext.clone()))), Task::none()),
            App::Pruner => {
                let mut p = PrunerState::new(0);
                p.set_look(self.settings.pruner_look);
                let t = p.load(&sess).map(move |m| Message::Prune(i, m));
                (Ui::Pruner(Box::new(p)), t)
            }
            App::Converter => (Ui::Converter(Box::new(ConverterState::new(&sess, self.settings.skip_partial_chunks))), Task::none()),
            App::WorldEditor => {
                let mut e = world_edit::EditorState::new();
                let t = e.load(&sess).map(move |m| Message::Edit(i, m));
                (Ui::Editor(Box::new(e)), t)
            }
            App::MapViewer => {
                let mut v = ViewerState::new(&sess, self.settings.viewer.clone());
                let t = v.load(&sess).map(move |m| Message::View(i, m));
                (Ui::Viewer(Box::new(v)), t)
            }
        };
        self.sessions.push(SessionTab { sess: Some(sess), ui, title, app, busy: None, confirm_close: false });
        self.screen = Page::Session(i);
        self.status = if app.read_only() { format!("opened in the {} (read-only, in place)", app.name()) } else { format!("opened in the {} — you are editing a copy; Save writes it back", app.name()) };
        task
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![iced::event::listen_with(handlers::handle_hotkey)];
        if self.tex.download.is_some() {
            subs.push(iced::time::every(std::time::Duration::from_millis(250)).map(|_| Message::Tex(appearance::TexMsg::Tick)));
        }
        // 3D camera movement ticks (keys held / mouse captured).
        for (i, tab) in self.sessions.iter().enumerate() {
            if let Ui::Viewer(ui) = &tab.ui {
                subs.push(ui.subscription().with(i).map(|(i, m)| Message::View(i, m)));
            }
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Message> {
        let page = container(self.body()).width(Length::Fill).height(Length::Fill).style(style::page());
        column![self.top_bar(), row![self.rail(), page].height(Length::Fill), self.status_bar()].width(Length::Fill).into()
    }

    fn body(&self) -> Element<'_, Message> {
        match self.screen {
            Page::Home => home::view(&self.worlds, &self.settings, self.scanning),
            Page::Library => column![
                row![assets::heading("WORLDS", style::fg(), 3), space::horizontal(), text(format!("{} worlds", self.worlds.len())).size(12).color(style::muted())].align_y(Alignment::Center),
                library::view(&self.library, &self.worlds, &self.settings, None, self.scanning, self.opening),
            ]
            .spacing(16)
            .padding(24)
            .into(),
            Page::Tool(app) => home::tool_page(app, &self.library, &self.worlds, &self.settings, self.scanning, self.opening),
            Page::Settings => settings_view::view(&self.settings_draft, self.settings_draft != self.settings, self.appearance()),
            Page::Extensions => extensions::view(&self.ext, &self.ext_status),
            Page::Doc(i) => match self.docs.get(i) {
                Some(tab) => container(editor_view::view(tab, std::rc::Rc::new(move |m| Message::Doc(i, m)))).padding(8).into(),
                None => space::vertical().into(),
            },
            Page::Session(i) => match self.sessions.get(i) {
                Some(tab) => self.session_view(i, tab),
                None => space::vertical().into(),
            },
        }
    }
}
