//! Settings → Appearance → **Add-on packs**: the ordered resource-pack
//! stack over the base textures (top wins per texture, like the game's
//! Global Resources screen). Packs come from OMT's add-on folder (imported
//! `.mcpack` / `.mcaddon` / `.zip` / folders), the games installed on this
//! computer, and the open world (`world_resource_packs.json`, applied on
//! top while that world is open). Marketplace packs are encrypted: listed,
//! never loaded.

use std::path::PathBuf;

use iced::widget::{button, column, container, image, row, space, text, toggler};
use iced::{Alignment, Element, Length, Task};
use omt_texpack::{Addon, AddonKind, AddonSource};

use crate::appearance::TexMsg;
use crate::session::blocking;
use crate::style::{self, Btn};
use crate::textures::{self, AddonPref};
use crate::{Message, OmniMCTools, Page};

#[derive(Debug, Clone)]
pub enum AddonMsg {
    Scanned(Vec<Addon>),
    Toggle(String, bool),
    /// Move up (`-1`) or down (`+1`) in the stack.
    Move(String, i32),
    ImportFile,
    ImportFolder,
    Imported(Result<Vec<Addon>, String>),
    Remove(String),
    WorldPacks(bool),
}

/// Debug/testing hook (`OMT_DEBUG_IMPORT_PACK=<file or folder>`): import a
/// pack at startup through the same path as "Add pack file…" (minus the
/// native file dialog) — headless import tests.
pub(crate) fn debug_import() -> Task<Message> {
    let Some(p) = std::env::var_os("OMT_DEBUG_IMPORT_PACK").map(PathBuf::from) else { return Task::none() };
    Task::perform(blocking(move || omt_texpack::addons::import(&p, &textures::library()).map_err(|e| e.to_string())), |r| Message::Tex(TexMsg::Addon(AddonMsg::Imported(r))))
}

impl OmniMCTools {
    /// The application's update: every message, then keep the open
    /// world's packs applied.
    pub(crate) fn step(&mut self, m: Message) -> Task<Message> {
        let t = self.update(m);
        Task::batch([t, self.sync_world_packs(), self.refresh_iso_textures(), self.sync_ext_tools()])
    }

    /// A new texture stack is active: isometric views re-render their
    /// tiles (the 2D texture layer restamps itself on its own).
    fn refresh_iso_textures(&mut self) -> Task<Message> {
        let g = textures::generation();
        if g == self.tex.seen_gen {
            return Task::none();
        }
        self.tex.seen_gen = g;
        let mut tasks = Vec::new();
        for (i, tab) in self.sessions.iter_mut().enumerate() {
            if let crate::SessionTab { sess: Some(sess), ui: crate::Ui::Viewer(ui), .. } = tab {
                tasks.push(ui.iso_pump(sess.store.as_ref()).map(move |m| Message::View(i, m)));
            }
        }
        Task::batch(tasks)
    }

    /// The world whose packs should apply: the world of the session on
    /// screen (the last one stays while browsing other pages).
    fn sync_world_packs(&mut self) -> Task<Message> {
        let Page::Session(i) = self.screen else { return Task::none() };
        let Some(origin) = self.sessions.get(i).and_then(|s| s.sess.as_ref()).map(|s| s.origin()) else { return Task::none() };
        if self.tex.world.as_ref() == Some(&origin) {
            return Task::none();
        }
        // Reload only when a world with packs comes or goes.
        let had = self.tex.world_has_packs;
        let has = !omt_texpack::addons::world_packs(&origin).is_empty();
        self.tex.world = Some(origin);
        self.tex.world_has_packs = has;
        if self.settings.world_packs && (had || has) { self.load_textures() } else { Task::none() }
    }

    /// Spec for [`textures::load`] from the settings + open world.
    pub(crate) fn tex_spec(&self) -> textures::LoadSpec {
        textures::LoadSpec { choice: self.settings.texture_pack.clone(), stack: self.settings.addons.clone(), world: self.tex.world.clone().filter(|_| self.settings.world_packs) }
    }

    pub(crate) fn scan_addons(&self) -> Task<Message> {
        let world = self.tex.world.clone();
        Task::perform(blocking(move || textures::available(world.as_deref())), |a| Message::Tex(TexMsg::Addon(AddonMsg::Scanned(a))))
    }

    pub(crate) fn on_addon(&mut self, m: AddonMsg) -> Task<Message> {
        match m {
            AddonMsg::Scanned(a) => {
                self.tex.addons = a;
                return Task::none();
            }
            AddonMsg::Toggle(id, on) => match self.settings.addons.iter_mut().find(|p| p.id == id) {
                Some(p) => p.enabled = on,
                // Newly enabled packs go on top.
                None => self.settings.addons.insert(0, AddonPref { id, enabled: on }),
            },
            AddonMsg::Move(id, d) => {
                // Unlisted packs join the stack (disabled) before moving.
                if !self.settings.addons.iter().any(|p| p.id == id) {
                    self.settings.addons.push(AddonPref { id: id.clone(), enabled: false });
                }
                let Some(i) = self.settings.addons.iter().position(|p| p.id == id) else { return Task::none() };
                let j = (i as i32 + d).clamp(0, self.settings.addons.len() as i32 - 1) as usize;
                self.settings.addons.swap(i, j);
                if !self.settings.addons[j].enabled {
                    self.save_settings();
                    return Task::none();
                }
            }
            AddonMsg::ImportFile | AddonMsg::ImportFolder => {
                let folder = matches!(m, AddonMsg::ImportFolder);
                return Task::perform(
                    async move {
                        let d = rfd::AsyncFileDialog::new().set_title("Add a resource pack");
                        let picked = if folder { d.pick_folder().await } else { d.add_filter("Resource packs", &["mcpack", "mcaddon", "zip"]).pick_file().await };
                        match picked {
                            Some(h) => {
                                let p: PathBuf = h.path().to_path_buf();
                                blocking(move || omt_texpack::addons::import(&p, &textures::library()).map_err(|e| e.to_string())).await
                            }
                            None => Ok(Vec::new()),
                        }
                    },
                    |r| Message::Tex(TexMsg::Addon(AddonMsg::Imported(r))),
                );
            }
            AddonMsg::Imported(r) => match r {
                Ok(v) if v.is_empty() => return Task::none(),
                Ok(v) => {
                    let names: Vec<String> = v.iter().map(|a| a.name.clone()).collect();
                    for a in v.iter().rev() {
                        self.settings.addons.retain(|p| p.id != a.id);
                        self.settings.addons.insert(0, AddonPref { id: a.id.clone(), enabled: !a.encrypted });
                    }
                    self.tex.status = format!("added {} — loading textures…", names.join(", "));
                }
                Err(e) => {
                    self.tex.status = format!("could not add the pack: {e}");
                    return Task::none();
                }
            },
            AddonMsg::Remove(id) => {
                if let Some(a) = self.tex.addons.iter().find(|a| a.id == id && a.source == AddonSource::Imported)
                    && a.root.starts_with(textures::library())
                {
                    if let Err(e) = std::fs::remove_dir_all(&a.root) {
                        self.tex.status = format!("could not remove {}: {e}", a.name);
                        return Task::none();
                    }
                    self.settings.addons.retain(|p| p.id != id);
                }
            }
            AddonMsg::WorldPacks(on) => self.settings.world_packs = on,
        }
        self.save_settings();
        self.load_textures()
    }

    /// The add-on stack card.
    pub(crate) fn addons_panel(&self) -> Element<'_, Message> {
        let user: Vec<&Addon> = self.tex.addons.iter().filter(|a| a.source != AddonSource::World).collect();
        // Stack order: listed prefs (top first), then everything else.
        let mut ordered: Vec<(&Addon, bool)> = self.settings.addons.iter().filter_map(|p| user.iter().find(|a| a.id == p.id).map(|a| (*a, p.enabled))).collect();
        for a in &user {
            if !ordered.iter().any(|(b, _)| b.id == a.id) {
                ordered.push((a, false));
            }
        }
        // Packs that cannot change any block texture (GUI / recipe / compat
        // packs — launcher folders hold many) stay out of the way unless on.
        let hidden = ordered.iter().filter(|(a, on)| !a.has_block_textures() && !on).count();
        ordered.retain(|(a, on)| a.has_block_textures() || *on);
        let mut list = column![].spacing(4);
        if ordered.is_empty() {
            list = list.push(text("No add-on packs yet — add a .mcpack, .mcaddon, Java .zip or a pack folder. Packs installed in the game show up here automatically.").size(11).color(style::muted()));
        }
        let n = ordered.len();
        for (i, (a, on)) in ordered.into_iter().enumerate() {
            list = list.push(self.addon_row(a, on, Some((i > 0, i + 1 < n))));
        }
        if hidden > 0 {
            list = list.push(text(format!("{hidden} more pack(s) without block textures hidden (GUI, recipe or compatibility packs — they don't change the map)")).size(11).color(style::hint()));
        }
        let world: Vec<&Addon> = self.tex.addons.iter().filter(|a| a.source == AddonSource::World).collect();
        let mut world_col = column![
            row![
                column![text("World packs").size(13), text("Apply the resource packs the open world carries, on top of your stack (in the world's own order).").size(11).color(style::muted())].spacing(1).width(Length::Fill),
                toggler(self.settings.world_packs).on_toggle(|v| Message::Tex(TexMsg::Addon(AddonMsg::WorldPacks(v)))),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
        ]
        .spacing(4);
        match (&self.tex.world, world.is_empty()) {
            (None, _) => world_col = world_col.push(text("No world open.").size(11).color(style::hint())),
            (Some(_), true) => world_col = world_col.push(text("The open world has no resource packs.").size(11).color(style::hint())),
            (Some(_), false) => {
                for a in world {
                    world_col = world_col.push(self.addon_row(a, self.settings.world_packs, None));
                }
            }
        }
        let actions = row![
            button(text("Add pack file…").size(12)).on_press(Message::Tex(TexMsg::Addon(AddonMsg::ImportFile))).style(style::button_style(Btn::Standard)),
            button(text("Add pack folder…").size(12)).on_press(Message::Tex(TexMsg::Addon(AddonMsg::ImportFolder))).style(style::button_style(Btn::Ghost)),
            space::horizontal(),
            text("Top of the list wins").size(11).color(style::hint()),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        container(
            column![
                column![
                    text("Add-on Packs").size(14),
                    text("Resource packs layered over the block textures above: Bedrock .mcpack / .mcaddon / folders and Java .zip packs. Marketplace packs are encrypted and not supported.").size(11).color(style::muted())
                ]
                .spacing(4),
                actions,
                list,
                world_col,
            ]
            .spacing(10),
        )
        .padding(12)
        .style(style::card())
        .into()
    }

    /// One pack row; `moves` = (can move up, can move down), `None` for
    /// world packs (fixed order, follow the World packs switch).
    fn addon_row<'a>(&self, a: &'a Addon, on: bool, moves: Option<(bool, bool)>) -> Element<'a, Message> {
        let icon: Element<'a, Message> = match &a.icon {
            Some(p) => image(image::Handle::from_path(p)).width(32).height(32).filter_method(image::FilterMethod::Nearest).into(),
            None => crate::assets::block(omt_art::Material::Planks, 32.0),
        };
        let source = match &a.source {
            AddonSource::Imported => "Added to OMT".to_string(),
            AddonSource::Game(g) => format!("Installed in {g}"),
            AddonSource::World => "From this world".to_string(),
        };
        let kind = match a.kind {
            AddonKind::Bedrock => "Bedrock",
            AddonKind::Java => "Java",
        };
        let mut meta = format!("{kind} · {source}");
        if !a.version.is_empty() {
            meta.push_str(&format!(" · v{}", a.version));
        }
        if let Some(s) = &a.subpack {
            meta.push_str(&format!(" · subpack {s}"));
        }
        if !a.encrypted && !a.has_block_textures() {
            meta.push_str(" · no block textures");
        }
        let mut info = column![text(a.name.clone()).size(13), text(meta).size(11).color(style::muted())].spacing(1).width(Length::Fill);
        if a.encrypted {
            info = info.push(text("Marketplace pack (encrypted) — not supported").size(11).color(style::hint()));
        } else if !a.description.is_empty() {
            info = info.push(text(a.description.lines().next().unwrap_or_default().to_string()).size(11).color(style::hint()));
        }
        let mut r = row![icon, info].spacing(10).align_y(Alignment::Center);
        if let Some((up, down)) = moves {
            let id = a.id.clone();
            let mv = |label: &'a str, d: i32, ok: bool| {
                let b = button(text(label).size(12)).padding([2, 8]).style(style::button_style(Btn::Ghost));
                if ok { b.on_press(Message::Tex(TexMsg::Addon(AddonMsg::Move(id.clone(), d)))) } else { b }
            };
            r = r.push(mv("▲", -1, up)).push(mv("▼", 1, down));
            if a.source == AddonSource::Imported {
                r = r.push(button(text("Remove").size(12)).padding([2, 8]).on_press(Message::Tex(TexMsg::Addon(AddonMsg::Remove(a.id.clone())))).style(style::button_style(Btn::Danger)));
            }
            let id = a.id.clone();
            let t = toggler(on && !a.encrypted);
            r = r.push(if a.encrypted { t } else { t.on_toggle(move |v| Message::Tex(TexMsg::Addon(AddonMsg::Toggle(id.clone(), v)))) });
        }
        container(r).padding([6, 10]).style(style::card()).into()
    }
}
