//! The Map Viewer's uNmINeD-parity extensions, kept out of `viewer.rs`:
//! Level info, Server players, Biomes / Blocks / Entities lists, Chunk
//! inspector, Block filters + Stylesheet (render rules), Highlighters,
//! area Image export and Saved map settings. One message type
//! ([`ExtMsg`]) and one state struct ([`Ext`]) hang off [`ViewerState`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use iced::{Element, Task};
use omt_render::players::PlayerInfo;

use crate::mapview::Overlay;
use crate::session::{Store, WorldSession, blocking};
use crate::viewer::{PanelTab, ViewerMsg, ViewerState};
use crate::viewer_export::{Area, ExportState, Zoom};
use crate::viewer_highlight::{HlKind, HlMsg, HlState, Highlighter};
use crate::viewer_info::{LevelInfo, ListMsg, ListState, Lists};
use crate::viewer_inspect::{Inspect, InspectMsg};
use crate::viewer_presets::PresetMsg;
use crate::viewer_rules::{RulesDraft, RulesMsg};

/// Highlight scan identity: (settings stamp, dimension, chunk rect).
type ScanKey = (u64, u8, (i32, i32, i32, i32));

#[derive(Debug, Clone)]
pub enum ExportMsg {
    Area(Area),
    Zoom(Zoom),
    MinX(String),
    MinZ(String),
    MaxX(String),
    MaxZ(String),
    Background(String),
    Trim(bool),
    Start,
    Picked(Option<PathBuf>),
    Cancel,
    Done(Result<String, String>),
}

#[derive(Debug, Clone)]
pub enum ExtMsg {
    Loaded(Arc<LevelInfo>, Arc<Vec<PlayerInfo>>),
    Size(u64),
    OpenFolder,
    List(ListMsg),
    Rules(RulesMsg),
    Hl(HlMsg),
    Inspect(InspectMsg),
    Export(ExportMsg),
    Preset(PresetMsg),
}

/// Extension state of one viewer session.
#[derive(Debug, Default)]
pub struct Ext {
    /// Registered render-rules id (`omt_render::rules`).
    pub rules_id: u64,
    pub info: Option<Arc<LevelInfo>>,
    pub size: Option<u64>,
    pub players: Arc<Vec<PlayerInfo>>,
    pub names: HashMap<String, String>,
    pub lists: ListState,
    pub rules_draft: RulesDraft,
    pub hl: HlState,
    hl_key: Option<ScanKey>,
    pub inspect: Inspect,
    pub export: ExportState,
    pub preset_draft: String,
    pub biomes: Vec<(String, usize, [u8; 3])>,
    pub blocks: Vec<(String, usize, [u8; 3])>,
    legend_tiles: (usize, u8),
}

impl Ext {
    /// Fresh state for a session whose render rules are `rules_id`.
    pub fn with_rules(rules_id: u64) -> Self {
        Self { rules_id, ..Default::default() }
    }
}

/// The user's player-name map file (uNmINeD `playernames.txt` format).
pub fn names_path() -> PathBuf {
    crate::settings::config_dir().join("player_names.txt")
}

fn load_names() -> HashMap<String, String> {
    std::fs::read_to_string(names_path()).map(|t| omt_render::players::parse_name_map(&t)).unwrap_or_default()
}

fn save_names(m: &HashMap<String, String>) -> Result<(), String> {
    let mut lines: Vec<String> = m.iter().map(|(k, v)| format!("{k} {v}")).collect();
    lines.sort();
    let text = format!("// Player names (id name), used by the OmniMCTools Map Viewer.\n{}\n", lines.join("\n"));
    let p = names_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, text).map_err(|e| e.to_string())
}

fn ext(m: ExtMsg) -> ViewerMsg {
    ViewerMsg::Ext(m)
}

impl ViewerState {
    /// Background loads at open: level info + players, folder size.
    pub fn ext_load(&mut self, sess: &WorldSession) -> Task<ViewerMsg> {
        self.ext.names = load_names();
        let Some(store) = sess.store.clone() else { return Task::none() };
        let summary = sess.summary.clone();
        let names = self.ext.names.clone();
        let root = sess.root();
        let info = Task::perform(
            blocking(move || {
                let info = crate::viewer_info::level_info(&store, &summary);
                let players = match &store {
                    Store::Java(w) => omt_render::players::java_players(w, &names),
                    Store::Bedrock(w) => omt_render::players::bedrock_players(w, &names),
                };
                (Arc::new(info), Arc::new(players))
            }),
            |(i, p)| ext(ExtMsg::Loaded(i, p)),
        );
        let size = Task::perform(blocking(move || crate::viewer_info::folder_size(&root)), |s| ext(ExtMsg::Size(s)));
        // Debug/testing hook (`OMT_DEBUG_INSPECT=x,z`): inspect a chunk at open.
        let inspect = match std::env::var("OMT_DEBUG_INSPECT").ok().and_then(|v| v.split_once(',').map(|(a, b)| (a.trim().parse::<i32>(), b.trim().parse::<i32>()))) {
            Some((Ok(x), Ok(z))) => self.select_chunk(sess.store.as_ref(), x, z),
            _ => Task::none(),
        };
        Task::batch([info, size, inspect])
    }

    /// Rules changed: register them and re-render.
    pub fn apply_rules(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        let id = omt_render::rules::register(self.prefs.rules.compile());
        if id == self.ext.rules_id {
            return Task::none();
        }
        self.ext.rules_id = id;
        self.hl_reset();
        self.rerender(store)
    }

    fn hl_reset(&mut self) {
        self.ext.hl_key = None;
    }

    pub fn update_ext(&mut self, sess: &WorldSession, m: ExtMsg) -> Task<ViewerMsg> {
        let store = sess.store.as_ref();
        match m {
            ExtMsg::Loaded(info, players) => {
                self.ext.info = Some(info);
                self.ext.players = players;
                Task::none()
            }
            ExtMsg::Size(s) => {
                self.ext.size = Some(s);
                Task::none()
            }
            ExtMsg::OpenFolder => {
                let root = sess.root();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new(if cfg!(target_os = "windows") { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(root).spawn();
                });
                Task::none()
            }
            ExtMsg::List(l) => self.update_lists(sess, l),
            ExtMsg::Rules(r) => {
                if crate::viewer_rules::apply(&mut self.prefs.rules, &mut self.ext.rules_draft, r) {
                    return self.apply_rules(store);
                }
                Task::none()
            }
            ExtMsg::Hl(HlMsg::Done(stamp, hits, n)) => {
                if self.ext.hl_key.is_some_and(|k| k.0 == stamp) {
                    self.ext.hl.hits = hits;
                    self.ext.hl.chunks = n;
                    self.ext.hl.busy = false;
                }
                Task::none()
            }
            ExtMsg::Hl(h) => {
                if crate::viewer_highlight::apply(&mut self.prefs.highlights, &mut self.ext.hl, h) {
                    self.hl_reset();
                }
                Task::none()
            }
            ExtMsg::Inspect(i) => self.update_inspect(store, i),
            ExtMsg::Export(e) => self.update_export(e),
            ExtMsg::Preset(p) => {
                let changed = crate::viewer_presets::apply(&mut self.prefs, &mut self.ext.preset_draft, p);
                if changed {
                    self.restyle();
                    self.hl_reset();
                    return self.apply_rules(store);
                }
                Task::none()
            }
        }
    }

    fn update_lists(&mut self, sess: &WorldSession, l: ListMsg) -> Task<ViewerMsg> {
        match l {
            ListMsg::Search(which, s) => self.ext.lists.set(which, s),
            ListMsg::SelectPlayer(i) => {
                self.ext.lists.selected = Some(i);
                self.ext.lists.name_draft = self.ext.players.get(i).and_then(|p| p.name.clone()).unwrap_or_default();
            }
            ListMsg::NameDraft(s) => self.ext.lists.name_draft = s,
            ListMsg::SaveName => {
                let Some(p) = self.ext.lists.selected.and_then(|i| self.ext.players.get(i)).cloned() else { return Task::none() };
                let name = self.ext.lists.name_draft.trim().to_string();
                if name.is_empty() {
                    self.ext.names.remove(&p.id);
                } else {
                    self.ext.names.insert(p.id.clone(), name.clone());
                }
                if let Err(e) = save_names(&self.ext.names) {
                    self.status = format!("could not save player names: {e}");
                }
                let mut players = (*self.ext.players).clone();
                if let Some(q) = players.iter_mut().find(|q| q.id == p.id) {
                    q.name = (!name.is_empty()).then_some(name);
                }
                self.ext.players = Arc::new(players);
            }
            ListMsg::GoTo(dim, x, z) => return self.goto_dim(sess, dim, x, z),
            ListMsg::Copy(s) => {
                self.status = format!("copied {s}");
                return iced::clipboard::write(s);
            }
            ListMsg::HighlightBiome(b) | ListMsg::HighlightBlock(b) if b.is_empty() => {}
            ListMsg::HighlightBiome(b) => self.add_highlight(HlKind::Biomes, b),
            ListMsg::HighlightBlock(b) => self.add_highlight(HlKind::Blocks, b),
        }
        Task::none()
    }

    fn add_highlight(&mut self, kind: HlKind, name: String) {
        let short = name.trim_start_matches("minecraft:").to_string();
        let h = &mut self.prefs.highlights;
        h.enabled = true;
        let colors = [[255, 90, 200], [80, 230, 255], [255, 210, 60], [120, 255, 120]];
        h.items.push(Highlighter { kind, query: short.clone(), color: colors[h.items.len() % colors.len()], opacity: if kind == HlKind::Blocks { 90 } else { 50 }, ..Default::default() });
        self.hl_reset();
        self.status = format!("highlighting {short} (Highlighters tab)");
    }

    // ----------------------------------------------------------- inspector
    fn update_inspect(&mut self, store: Option<&Store>, m: InspectMsg) -> Task<ViewerMsg> {
        let st = &mut self.ext.inspect;
        match m {
            InspectMsg::ClickSelect(v) => st.click_select = v,
            InspectMsg::X(s) => st.text.0 = s,
            InspectMsg::Z(s) => st.text.1 = s,
            InspectMsg::Locate => {
                if let (Ok(x), Ok(z)) = (st.text.0.trim().parse::<i32>(), st.text.1.trim().parse::<i32>()) {
                    self.ctl.camera.cx = x as f64 * 16.0 + 8.0;
                    self.ctl.camera.cz = z as f64 * 16.0 + 8.0;
                    let t = self.select_chunk(store, x, z);
                    return Task::batch([t, self.ctl.refill(store).map(ViewerMsg::Tiles)]);
                }
                self.status = "enter chunk X and Z".into();
            }
            InspectMsg::Select(x, z) => return self.select_chunk(store, x, z),
            InspectMsg::Report(r) => {
                if st.selected == Some((r.cx, r.cz)) {
                    st.report = Some(r);
                    st.busy = false;
                }
            }
            InspectMsg::CopySnbt => {
                if let Some(s) = st.report.as_ref().and_then(|r| r.snbt.clone()) {
                    self.status = "copied chunk NBT (SNBT)".into();
                    return iced::clipboard::write(s);
                }
            }
        }
        Task::none()
    }

    fn select_chunk(&mut self, store: Option<&Store>, x: i32, z: i32) -> Task<ViewerMsg> {
        let st = &mut self.ext.inspect;
        st.selected = Some((x, z));
        st.text = (x.to_string(), z.to_string());
        st.busy = true;
        let Some(store) = store.cloned() else { return Task::none() };
        let dim = self.ctl.dim;
        Task::perform(blocking(move || crate::viewer_inspect::report(&store, dim, x, z)), |r| ext(ExtMsg::Inspect(InspectMsg::Report(Arc::new(r)))))
    }

    /// A left click while the inspector tab is open selects a chunk.
    pub fn inspect_click(&mut self, store: Option<&Store>, at: (f64, f64)) -> Option<Task<ViewerMsg>> {
        if self.prefs.panel() != Some(PanelTab::Inspect) || !self.ext.inspect.click_select || self.v3.on {
            return None;
        }
        let (x, z) = ((at.0 / 16.0).floor() as i32, (at.1 / 16.0).floor() as i32);
        Some(self.select_chunk(store, x, z))
    }

    // -------------------------------------------------------------- export
    fn update_export(&mut self, m: ExportMsg) -> Task<ViewerMsg> {
        let st = &mut self.ext.export;
        match m {
            ExportMsg::Area(a) => st.area = a,
            ExportMsg::Zoom(z) => st.zoom = z,
            ExportMsg::MinX(s) => st.min.0 = s,
            ExportMsg::MinZ(s) => st.min.1 = s,
            ExportMsg::MaxX(s) => st.max.0 = s,
            ExportMsg::MaxZ(s) => st.max.1 = s,
            ExportMsg::Background(s) => st.background = s,
            ExportMsg::Trim(v) => st.trim = v,
            ExportMsg::Start => {
                if let Err(e) = crate::viewer_export::resolve(st, &self.ctl) {
                    st.status = e;
                    return Task::none();
                }
                return Task::perform(
                    async { rfd::AsyncFileDialog::new().set_title("Export map image").add_filter("PNG image", &["png"]).set_file_name("map.png").save_file().await.map(|h| h.path().to_path_buf()) },
                    |p| ext(ExtMsg::Export(ExportMsg::Picked(p))),
                );
            }
            ExportMsg::Picked(None) => {}
            ExportMsg::Picked(Some(path)) => {
                let job = match crate::viewer_export::resolve(st, &self.ctl) {
                    Ok(j) => j,
                    Err(e) => {
                        st.status = e;
                        return Task::none();
                    }
                };
                let Some(src) = self.ctl.map.as_ref().map(|m| m.source.clone()) else { return Task::none() };
                let (cancel, done) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
                let span = (omt_render::TILE_CHUNKS as i64 * 16) << job.level;
                let tiles = ((job.x1 - 1).div_euclid(span) - job.x0.div_euclid(span) + 1) as u64 * ((job.z1 - 1).div_euclid(span) - job.z0.div_euclid(span) + 1) as u64;
                st.busy = Some((Arc::clone(&cancel), Arc::clone(&done), tiles));
                let (w, h) = job.px_size();
                st.status = format!("exporting {w}×{h} px…");
                let look = crate::viewer_export::Look { opts: self.ctl.options(), style: self.prefs.style, threads: crate::mapconfig::config().threads as usize };
                return Task::perform(blocking(move || crate::viewer_export::export_area(&src, look, job, &path, &cancel, &done)), |r| ext(ExtMsg::Export(ExportMsg::Done(r))));
            }
            ExportMsg::Cancel => {
                if let Some((c, _, _)) = &st.busy {
                    c.store(true, Ordering::Relaxed);
                }
            }
            ExportMsg::Done(r) => {
                st.busy = None;
                st.status = r.unwrap_or_else(|e| format!("export failed: {e}"));
                self.status = st.status.clone();
            }
        }
        Task::none()
    }

    // ---------------------------------------------------------- highlights
    /// Rescan the visible chunks when highlighters or the view changed.
    pub fn plan_highlights(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        let active = self.prefs.highlights.active();
        if active.is_empty() || self.v3.on {
            if !self.ext.hl.hits.is_empty() {
                self.ext.hl.hits = Arc::default();
            }
            self.ext.hl_key = None;
            return Task::none();
        }
        let (x0, z0, x1, z1) = self.ctl.camera.visible_blocks(self.ctl.view);
        let r = ((x0 / 16.0).floor() as i32, (z0 / 16.0).floor() as i32, (x1 / 16.0).ceil() as i32, (z1 / 16.0).ceil() as i32);
        let n = (r.2 - r.0 + 1) as i64 * (r.3 - r.1 + 1) as i64;
        let stamp = {
            let mut h = omt_render::source::Fnv::new();
            h.bytes(format!("{:?}{}", active.iter().map(|(_, q)| q).collect::<Vec<_>>(), self.ext.rules_id).as_bytes());
            h.finish()
        };
        let key = (stamp, self.ctl.dim, r);
        if self.ext.hl_key == Some(key) || self.ext.hl.busy {
            return Task::none();
        }
        self.ext.hl.too_wide = n > crate::viewer_highlight::MAX_SCAN_CHUNKS;
        if self.ext.hl.too_wide {
            self.ext.hl_key = Some(key);
            self.ext.hl.hits = Arc::default();
            self.ext.hl.chunks = 0;
            return Task::none();
        }
        let (Some(store), Some(map)) = (store.cloned(), self.ctl.map.as_ref()) else { return Task::none() };
        let chunks: Vec<(i32, i32)> = (r.1..=r.3).flat_map(|z| (r.0..=r.2).map(move |x| (x, z))).filter(|&(x, z)| map.chunk_present(x, z)).collect();
        self.ext.hl_key = Some(key);
        self.ext.hl.busy = true;
        self.ext.hl.chunks = chunks.len();
        let queries: Vec<_> = active.into_iter().map(|(_, q)| q).collect();
        let (dim, threads) = (self.ctl.dim, crate::mapconfig::config().threads as usize);
        Task::perform(
            blocking(move || {
                let read = crate::chunkread::read_chunks(&store, dim, &chunks, threads);
                let pal = omt_render::Palette::embedded();
                let mut hits = Vec::new();
                for c in &read {
                    omt_render::finder::scan_chunk(c, pal, &queries, &mut hits);
                }
                (Arc::new(hits), read.len())
            }),
            move |(h, n)| ext(ExtMsg::Hl(HlMsg::Done(stamp, h, n))),
        )
    }

    /// Recompute the Biomes / Blocks legends when their tab is open.
    pub fn refresh_legends(&mut self) {
        let tab = self.prefs.panel();
        if !matches!(tab, Some(PanelTab::Biomes | PanelTab::Blocks)) {
            return;
        }
        let Some(map) = &self.ctl.map else { return };
        let key = (map.tiles.len(), map.level);
        if self.ext.legend_tiles == key && !(self.ext.biomes.is_empty() && self.ext.blocks.is_empty()) {
            return;
        }
        self.ext.legend_tiles = key;
        self.ext.biomes = map.biome_legend().into_iter().map(|(n, c)| {
            let rgb = omt_render::style::biome_color(&n);
            (n, c, rgb)
        }).collect();
        let pal = omt_render::Palette::embedded();
        self.ext.blocks = map
            .block_legend()
            .into_iter()
            .map(|(n, c)| {
                let col = pal.block(&omt_world::BlockState { name: n.clone(), properties: Vec::new() }).rgba;
                (n, c, [col[0], col[1], col[2]])
            })
            .collect();
    }

    /// Extension overlay layers (highlights, border, selection, labels).
    pub fn ext_overlay(&self, ov: &mut Overlay) {
        if !self.ext.hl.hits.is_empty() {
            ov.highlights = crate::viewer_highlight::marks(&self.prefs.highlights, &self.ext.hl.hits);
            ov.dim_map = self.prefs.highlights.darken as f32 / 100.0;
        }
        if self.prefs.border && self.ctl.dim == 0 {
            ov.border = self.ext.info.as_ref().and_then(|i| i.border);
        }
        ov.region_labels = self.prefs.region_labels;
        if self.prefs.panel() == Some(PanelTab::Inspect)
            && let Some((x, z)) = self.ext.inspect.selected
        {
            ov.selection = Some((x as f64 * 16.0, z as f64 * 16.0, x as f64 * 16.0 + 16.0, z as f64 * 16.0 + 16.0));
        }
    }

    /// Body of an extension tab (`None` for the classic tabs).
    pub fn ext_tab(&self) -> Option<Element<'_, ViewerMsg>> {
        let tab = self.prefs.panel()?;
        let list = |m: ListMsg| ext(ExtMsg::List(m));
        Some(match tab {
            PanelTab::Info => crate::viewer_info::info_tab(self.ext.info.as_deref(), self.ext.size, self.ctl.map.as_ref().map(|m| m.chunk_total()), ext(ExtMsg::OpenFolder)),
            PanelTab::Inspect => crate::viewer_inspect::tab(&self.ext.inspect, |m| ext(ExtMsg::Inspect(m))),
            PanelTab::Players => crate::viewer_info::players_tab(&self.ext.players, &self.ext.lists, list),
            PanelTab::Biomes => crate::viewer_info::legend_tab(Lists::Biomes, &self.ext.biomes, &self.ext.lists, list),
            PanelTab::Blocks => crate::viewer_info::legend_tab(Lists::Blocks, &self.ext.blocks, &self.ext.lists, list),
            PanelTab::Entities => crate::viewer_info::entities_tab(&self.entities, self.ctl.dim, self.prefs.entities, &self.ext.lists, list),
            PanelTab::Filter => crate::viewer_rules::filter_tab(&self.prefs.rules, &self.ext.rules_draft, |m| ext(ExtMsg::Rules(m))),
            PanelTab::Stylesheet => crate::viewer_rules::stylesheet_tab(&self.prefs.rules, &self.ext.rules_draft, |m| ext(ExtMsg::Rules(m))),
            PanelTab::Highlight => crate::viewer_highlight::tab(&self.prefs.highlights, &self.ext.hl, self.bedrock, |m| ext(ExtMsg::Hl(m))),
            PanelTab::Presets => crate::viewer_presets::tab(&self.prefs, &self.ext.preset_draft, |m| ext(ExtMsg::Preset(m))),
            _ => return None,
        })
    }
}

/// A CPU-count choice for the nav picker (`0` = all cores).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuChoice(pub usize);

impl std::fmt::Display for CpuChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0 => write!(f, "Max CPU ({})", omt_render::workers::cpus()),
            n => write!(f, "{n} CPU"),
        }
    }
}

impl ViewerState {
    /// Render-thread picker (uNmINeD's chunk processor count).
    pub fn cpu_picker(&self) -> Element<'_, ViewerMsg> {
        use iced::widget::{pick_list, tooltip};
        let cur = CpuChoice(crate::mapconfig::config().threads as usize);
        let choices: Vec<CpuChoice> = omt_render::workers::choices().into_iter().map(CpuChoice).collect();
        let pl = pick_list(choices, Some(cur), |c: CpuChoice| ViewerMsg::MapUi(crate::mapconfig::MapUiMsg::Threads(c.0 as u16))).text_size(12).width(118);
        tooltip(pl, iced::widget::container(iced::widget::text("CPU cores used to render the map").size(12)).padding([4, 8]).style(crate::style::card()), tooltip::Position::Bottom).into()
    }

    /// The area export form (Export tab).
    pub fn export_area_ui(&self) -> Element<'_, ViewerMsg> {
        use crate::panelui::{action, heading, kv, note, switch};
        use iced::widget::{column, pick_list, row, text, text_input};
        let st = &self.ext.export;
        let on = |m: ExportMsg| ext(ExtMsg::Export(m));
        let mut col = column![
            heading("IMAGE EXPORT"),
            row![text("Area").size(12).width(80), pick_list(Area::ALL.to_vec(), Some(st.area), move |a| on(ExportMsg::Area(a))).text_size(12).width(iced::Length::Fill)].align_y(iced::Alignment::Center),
            row![text("Zoom").size(12).width(80), pick_list(Zoom::ALL.to_vec(), Some(st.zoom), move |z| on(ExportMsg::Zoom(z))).text_size(12).width(iced::Length::Fill)].align_y(iced::Alignment::Center),
        ]
        .spacing(8);
        if st.area == Area::Custom {
            let field = |ph: &'static str, v: &str, f: fn(String) -> ExportMsg| text_input(ph, v).on_input(move |s| on(f(s))).size(12).width(72).style(crate::style::input());
            col = col
                .push(row![text("From X/Z").size(12).width(80), field("min X", &st.min.0, ExportMsg::MinX), field("min Z", &st.min.1, ExportMsg::MinZ)].spacing(4).align_y(iced::Alignment::Center))
                .push(row![text("To X/Z").size(12).width(80), field("max X", &st.max.0, ExportMsg::MaxX), field("max Z", &st.max.1, ExportMsg::MaxZ)].spacing(4).align_y(iced::Alignment::Center));
        }
        col = col
            .push(row![text("Background").size(12).width(80), text_input("transparent (or #rrggbb)", &st.background).on_input(move |s| on(ExportMsg::Background(s))).size(12).style(crate::style::input())].align_y(iced::Alignment::Center))
            .push(switch("Trim to the world's chunks", st.trim, move |v| on(ExportMsg::Trim(v))));
        match crate::viewer_export::resolve(st, &self.ctl) {
            Ok(job) => {
                let (w, h) = job.px_size();
                col = col
                    .push(kv("Area (blocks)", format!("{}, {} → {}, {}", job.x0, job.z0, job.x1 - 1, job.z1 - 1)))
                    .push(kv("Image size", format!("{w} × {h} px")))
                    .push(kv("Working memory", crate::viewer_info::human_bytes(job.memory_bytes())));
            }
            Err(e) => col = col.push(note(e)),
        }
        col = match &st.busy {
            Some((_, done, total)) => col.push(row![text(format!("Rendering… {} / {} tiles", done.load(Ordering::Relaxed), total)).size(12).width(iced::Length::Fill), action("Cancel", Some(on(ExportMsg::Cancel)))].align_y(iced::Alignment::Center)),
            None => col.push(action("Export image…", Some(on(ExportMsg::Start)))),
        };
        if !st.status.is_empty() {
            col = col.push(note(st.status.clone()));
        }
        col.push(note("Renders fresh from the world with the current style, filters and stylesheet; large areas stream to disk.")).into()
    }
}
