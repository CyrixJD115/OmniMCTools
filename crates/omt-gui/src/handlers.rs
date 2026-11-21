//! Message handlers split out of `main.rs` (file-size budget): saving
//! sessions, the Extensions and Settings pages, hotkeys.

use std::path::PathBuf;

use iced::Task;

use crate::extensions::{self, ExtMsg};
use crate::session::blocking;
use crate::settings_view::{self, SettingsMsg};
use crate::{Message, OmniMCTools, Page, mapdata, slot, style};

impl OmniMCTools {
    pub(crate) fn save_session(&mut self, i: usize, dest: Option<PathBuf>) -> Task<Message> {
        // Extension tools on this world let go of its store first.
        if let Some(origin) = self.sessions.get(i).and_then(|t| t.sess.as_ref()).map(|s| s.origin()) {
            self.suspend_tools(&origin);
        }
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
    pub(crate) fn on_ext(&mut self, m: ExtMsg) -> Task<Message> {
        match &m {
            ExtMsg::Back => {
                self.screen = Page::Home;
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
            ExtMsg::PickPlugin => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new().set_title("Install a native plugin").add_filter("Dynamic library", &["so", "dll", "dylib"]).pick_file().await.map(|h| h.path().to_path_buf())
                    },
                    |p| Message::Ext(ExtMsg::PluginPicked(p)),
                );
            }
            ExtMsg::PluginPicked(None) => return Task::none(),
            _ => {}
        }
        let (status, _changed) = extensions::handle(&self.ext, &m);
        if !status.is_empty() {
            self.ext_status = status;
        }
        Task::none()
    }

    /// A map toolbar change: update + save the settings (and the Settings
    /// page draft) and publish the map config to every open map.
    pub(crate) fn apply_map_ui(&mut self, u: mapdata::MapUiMsg) {
        self.settings.apply_map_ui(u);
        self.settings_draft.apply_map_ui(u);
        mapdata::set_config(self.settings.map_config());
        if let Err(e) = self.settings.save() {
            self.status = format!("could not save settings: {e}");
        }
    }

    pub(crate) fn on_settings(&mut self, m: SettingsMsg) -> Task<Message> {
        match m {
            SettingsMsg::Back => self.screen = Page::Home,
            SettingsMsg::Save => {
                // Keep things changed outside the Settings page.
                self.settings_draft.favorites = self.settings.favorites.clone();
                self.settings_draft.bookmarks = self.settings.bookmarks.clone();
                self.settings_draft.viewer = self.settings.viewer.clone();
                self.settings_draft.theme = self.settings.theme;
                self.settings_draft.rail_collapsed = self.settings.rail_collapsed;
                self.settings_draft.texture_pack = self.settings.texture_pack.clone();
                self.settings_draft.addons = self.settings.addons.clone();
                self.settings_draft.world_packs = self.settings.world_packs;
                self.settings_draft.covers = self.settings.covers.clone();
                self.settings_draft.pruner_look = self.settings.pruner_look;
                self.settings_draft.library_grid = self.settings.library_grid;
                self.settings_draft.library_sort = self.settings.library_sort;
                self.settings = self.settings_draft.clone();
                style::set_theme(self.settings.theme);
                mapdata::set_config(self.settings.map_config());
                self.ext.set_master(self.settings.extensions);
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
            SettingsMsg::ClearMapCache => {
                let cache = omt_render::TileCache::new(mapdata::cache_dir(), 0);
                let (bytes, files) = cache.usage();
                self.status = match cache.clear() {
                    Ok(()) => format!("map cache cleared ({files} tiles, {:.1} MB)", bytes as f64 / (1024.0 * 1024.0)),
                    Err(e) => format!("could not clear the map cache: {e}"),
                };
            }
            other => {
                settings_view::apply(&mut self.settings_draft, &other);
                // Scale previews live.
                if let SettingsMsg::Scale(v) = other {
                    self.settings.scale_factor = v;
                }
                // The extensions master switch applies immediately (the
                // extensions page carries the same switch).
                if let SettingsMsg::Extensions(v) = other {
                    self.settings.extensions = v;
                    self.ext.set_master(v);
                }
            }
        }
        Task::none()
    }
}

pub(crate) fn handle_hotkey(event: iced::Event, status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
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
    // Map Viewer navigation keys (act only when a viewer is on screen):
    // arrows pan 16 blocks, Shift+arrows 1, Ctrl+arrows 256; +/− zoom;
    // * resets to 1:1; Ctrl+G cycles the chunk grid.
    use crate::viewer::ViewerKey;
    let zoom_key = |c: &str| key == Key::Character(c.into()) && !modifiers.contains(Modifiers::CTRL);
    if zoom_key("+") || zoom_key("=") {
        return Some(Message::ViewerKey(ViewerKey::ZoomIn));
    }
    if zoom_key("-") {
        return Some(Message::ViewerKey(ViewerKey::ZoomOut));
    }
    if zoom_key("*") {
        return Some(Message::ViewerKey(ViewerKey::ZoomReset));
    }
    if let Key::Named(named) = &key {
        let d = if modifiers.contains(Modifiers::SHIFT) {
            1.0
        } else if ctrl {
            256.0
        } else {
            16.0
        };
        let nav = match named {
            iced::keyboard::key::Named::ArrowLeft => Some(ViewerKey::Pan(-d, 0.0)),
            iced::keyboard::key::Named::ArrowRight => Some(ViewerKey::Pan(d, 0.0)),
            iced::keyboard::key::Named::ArrowUp => Some(ViewerKey::Pan(0.0, -d)),
            iced::keyboard::key::Named::ArrowDown => Some(ViewerKey::Pan(0.0, d)),
            _ => None,
        };
        if let Some(k) = nav {
            return Some(Message::ViewerKey(k));
        }
    }
    if ctrl && (char_key("g") || char_key("G")) {
        return Some(Message::ViewerKey(ViewerKey::CycleChunkGrid));
    }
    None
}
