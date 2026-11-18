//! Map Viewer list tabs (uNmINeD parity): **Level info** (level.dat facts,
//! world border, folder), **Server players**, **Biomes**, **Blocks** and
//! **Entities** in view. Each list is searchable; picking an item goes to
//! it on the map or turns it into a highlighter.

use std::sync::Arc;

use iced::widget::{column, row, text, text_input};
use iced::{Alignment, Element, Length};
use omt_nbt::Tag;
use omt_render::players::PlayerInfo;

use crate::panelui::{self, heading, kv, note, search};
use crate::session::Store;
use crate::style;

/// Level facts for the Level info tab.
#[derive(Debug, Default, Clone)]
pub struct LevelInfo {
    pub facts: Vec<(String, String)>,
    /// Other scalar `level.dat` fields (name → value).
    pub extra: Vec<(String, String)>,
    /// World border (min x, min z, max x, max z) in blocks (Java).
    pub border: Option<(f64, f64, f64, f64)>,
}

fn scalar(t: &Tag) -> Option<String> {
    Some(match t {
        Tag::Byte(v) => v.to_string(),
        Tag::Short(v) => v.to_string(),
        Tag::Int(v) => v.to_string(),
        Tag::Long(v) => v.to_string(),
        Tag::Float(v) => format!("{v}"),
        Tag::Double(v) => format!("{v:.2}"),
        Tag::String(s) => s.clone(),
        _ => return None,
    })
}

fn game_mode(v: i64) -> String {
    match v {
        0 => "Survival".into(),
        1 => "Creative".into(),
        2 => "Adventure".into(),
        3 => "Spectator".into(),
        n => n.to_string(),
    }
}

fn difficulty(v: i64) -> String {
    ["Peaceful", "Easy", "Normal", "Hard"].get(v as usize).map_or(v.to_string(), |s| s.to_string())
}

fn day_time(t: i64) -> String {
    let day = t / 24_000;
    let tod = t.rem_euclid(24_000);
    let h = (tod / 1000 + 6) % 24;
    let m = (tod % 1000) * 60 / 1000;
    format!("day {day}, {h:02}:{m:02}")
}

/// Read Level info from the opened store.
pub fn level_info(store: &Store, summary: &omt_world::detect::WorldSummary) -> LevelInfo {
    let mut info = LevelInfo::default();
    let f = &mut info.facts;
    f.push(("Level name".into(), summary.name.clone()));
    f.push(("Level path".into(), summary.root.display().to_string()));
    let (data, java) = match store {
        Store::Java(w) => (w.level_dat.tag.get("Data").cloned().unwrap_or_else(|| w.level_dat.tag.clone()), true),
        Store::Bedrock(w) => (w.level_dat.nbt.tag.clone(), false),
    };
    let get = |k: &str| data.get(k);
    let num = |k: &str| get(k).and_then(Tag::as_i64);
    if java {
        f.push(("Edition".into(), "Java Edition".into()));
        if let Some(v) = get("Version").and_then(|v| v.get("Name")).and_then(Tag::as_str) {
            f.push(("Minecraft version".into(), v.to_string()));
        }
        if let Some(dv) = num("DataVersion") {
            f.push(("DataVersion".into(), dv.to_string()));
        }
        let seed = get("WorldGenSettings").and_then(|w| w.get("seed")).and_then(Tag::as_i64).or_else(|| num("RandomSeed"));
        if let Some(s) = seed {
            f.push(("World seed".into(), s.to_string()));
        }
        if let Some(g) = num("GameType") {
            f.push(("Game type".into(), game_mode(g)));
        }
    } else {
        f.push(("Edition".into(), "Bedrock Edition".into()));
        if let Some(Tag::List(v)) = get("lastOpenedWithVersion") {
            let parts: Vec<String> = v.iter().filter_map(Tag::as_i64).map(|n| n.to_string()).collect();
            f.push(("Minecraft version".into(), parts.join(".")));
        }
        if let Some(s) = summary.bedrock_storage_version {
            f.push(("Storage version".into(), s.to_string()));
        }
        if let Some(s) = num("RandomSeed") {
            f.push(("World seed".into(), s.to_string()));
        }
        if let Some(g) = num("GameType") {
            f.push(("Game type".into(), game_mode(g)));
        }
    }
    if let Some(d) = num("Difficulty") {
        f.push(("Difficulty".into(), difficulty(d)));
    }
    if let (Some(x), Some(y), Some(z)) = (num("SpawnX"), num("SpawnY"), num("SpawnZ")) {
        f.push(("World spawn".into(), format!("{x}, {y}, {z}")));
    }
    if let Some(t) = num("DayTime").or_else(|| num("Time")) {
        f.push(("Time".into(), day_time(t)));
    }
    if let Some(t) = num("Time").or_else(|| num("currentTick")) {
        f.push(("World age".into(), format!("{} ticks ({:.1} h)", t, t as f64 / 72_000.0)));
    }
    let lp = num("LastPlayed");
    if let Some(lp) = lp {
        // Java stores milliseconds, Bedrock seconds.
        let secs = if java { lp / 1000 } else { lp };
        f.push(("Last played".into(), format!("{} (unix)", secs)));
    }
    let rain = num("raining").map(|v| v != 0).or_else(|| get("rainLevel").and_then(|t| if let Tag::Float(v) = t { Some(*v > 0.0) } else { None }));
    if let Some(r) = rain {
        let thunder = num("thundering").is_some_and(|v| v != 0) || get("lightningLevel").is_some_and(|t| matches!(t, Tag::Float(v) if *v > 0.0));
        f.push(("Weather".into(), if thunder { "Thunderstorm" } else if r { "Rain" } else { "Clear" }.into()));
    }
    for (k, label) in [("hardcore", "Hardcore"), ("allowCommands", "Cheats"), ("commandsEnabled", "Cheats")] {
        if let Some(v) = num(k) {
            f.push((label.into(), if v != 0 { "Yes" } else { "No" }.into()));
        }
    }
    if java {
        let (cx, cz, size) = (get("BorderCenterX").and_then(|t| scalar(t)?.parse::<f64>().ok()), get("BorderCenterZ").and_then(|t| scalar(t)?.parse::<f64>().ok()), get("BorderSize").and_then(|t| scalar(t)?.parse::<f64>().ok()));
        if let (Some(cx), Some(cz), Some(s)) = (cx, cz, size)
            && s < 59_000_000.0
        {
            info.border = Some((cx - s / 2.0, cz - s / 2.0, cx + s / 2.0, cz + s / 2.0));
            f.push(("World border".into(), format!("{s:.0} blocks around {cx:.0}, {cz:.0}")));
        }
    }
    if let Some(fields) = data.as_compound() {
        let shown = ["LevelName", "DataVersion", "RandomSeed", "GameType", "Difficulty", "SpawnX", "SpawnY", "SpawnZ", "DayTime", "Time", "LastPlayed", "raining", "thundering", "hardcore", "allowCommands", "commandsEnabled"];
        let clip = |v: String| if v.chars().count() > 40 { format!("{}…", v.chars().take(40).collect::<String>()) } else { v };
        let mut extra: Vec<(String, String)> = fields.iter().filter(|(k, _)| !shown.contains(&k.as_str())).filter_map(|(k, v)| Some((k.clone(), clip(scalar(v)?)))).collect();
        extra.sort();
        info.extra = extra;
    }
    info
}

/// Total size of a folder (bytes), best effort.
pub fn folder_size(root: &std::path::Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

/// Human size.
pub fn human_bytes(b: u64) -> String {
    match b {
        b if b >= 1 << 30 => format!("{:.2} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} KB", b / 1024),
    }
}

/// The **Level info** tab.
pub fn info_tab<'a, M: Clone + 'a>(info: Option<&'a LevelInfo>, size: Option<u64>, chunks: Option<usize>, open_folder: M) -> Element<'a, M> {
    let Some(info) = info else { return note("Reading level.dat…") };
    let mut col = column![heading("LEVEL PROPERTIES")].spacing(6);
    for (k, v) in &info.facts {
        col = col.push(kv(k.clone(), v.clone()));
    }
    if let Some(n) = chunks {
        col = col.push(kv("Chunks (this dimension)", n.to_string()));
    }
    col = col.push(kv("Size on disk", size.map_or_else(|| "measuring…".into(), human_bytes)));
    col = col.push(panelui::action("Open world folder", Some(open_folder)));
    if !info.extra.is_empty() {
        col = col.push(heading("OTHER LEVEL.DAT FIELDS"));
        for (k, v) in info.extra.iter().take(60) {
            col = col.push(kv(k.clone(), v.clone()));
        }
    }
    col.into()
}

/// Messages of the list tabs (wrapped by the caller).
#[derive(Debug, Clone)]
pub enum ListMsg {
    Search(Lists, String),
    SelectPlayer(usize),
    NameDraft(String),
    SaveName,
    GoTo(u8, f64, f64),
    Copy(String),
    HighlightBiome(String),
    HighlightBlock(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lists {
    Players,
    Biomes,
    Blocks,
    Entities,
}

/// Search boxes + player selection of the list tabs.
#[derive(Debug, Default)]
pub struct ListState {
    pub players: String,
    pub biomes: String,
    pub blocks: String,
    pub entities: String,
    pub selected: Option<usize>,
    pub name_draft: String,
}

impl ListState {
    pub fn query(&self, l: Lists) -> &str {
        match l {
            Lists::Players => &self.players,
            Lists::Biomes => &self.biomes,
            Lists::Blocks => &self.blocks,
            Lists::Entities => &self.entities,
        }
    }
    pub fn set(&mut self, l: Lists, s: String) {
        match l {
            Lists::Players => self.players = s,
            Lists::Biomes => self.biomes = s,
            Lists::Blocks => self.blocks = s,
            Lists::Entities => self.entities = s,
        }
    }
}

fn hit(q: &str, s: &str) -> bool {
    q.trim().is_empty() || s.to_lowercase().contains(&q.trim().to_lowercase())
}

const DIMS: [&str; 3] = ["Overworld", "Nether", "End"];

/// The **Server players** tab.
pub fn players_tab<'a, M: Clone + 'a>(players: &'a [PlayerInfo], st: &'a ListState, on: impl Fn(ListMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let o = on.clone();
    let mut col = column![search("Search players", &st.players, move |s| o(ListMsg::Search(Lists::Players, s))), note(format!("{} players", players.len()))].spacing(6);
    for (i, p) in players.iter().enumerate().filter(|(_, p)| hit(&st.players, &p.label()) || p.ids().iter().any(|id| hit(&st.players, id))) {
        let sub = format!("{} · {:.0}, {:.0}, {:.0}", DIMS.get(p.dim as usize).unwrap_or(&"?"), p.x, p.y, p.z);
        col = col.push(panelui::item(panelui::two_line(p.label(), sub), st.selected == Some(i), on(ListMsg::SelectPlayer(i))));
    }
    if let Some(p) = st.selected.and_then(|i| players.get(i)) {
        col = col.push(heading("PLAYER"));
        col = col.push(kv("Name", p.name.clone().unwrap_or_else(|| "(unknown)".into())));
        col = col.push(kv("ID", p.id.clone()));
        if let Some(m) = &p.msa_id {
            col = col.push(kv("MSA ID", m.clone()));
        }
        if let Some(s) = &p.self_signed_id {
            col = col.push(kv("Self-signed ID", s.clone()));
        }
        col = col.push(kv("Dimension", DIMS.get(p.dim as usize).unwrap_or(&"?").to_string()));
        col = col.push(kv("Position", format!("{:.1}, {:.1}, {:.1}", p.x, p.y, p.z)));
        if let Some(g) = p.game_mode_name() {
            col = col.push(kv("Game mode", g));
        }
        if let Some(l) = p.level {
            col = col.push(kv("XP level", l.to_string()));
        }
        if let Some(h) = p.health {
            col = col.push(kv("Health", format!("{h:.0}")));
        }
        col = col.push(
            row![
                panelui::action("Go to", Some(on(ListMsg::GoTo(p.dim, p.x, p.z)))),
                panelui::action("Copy ID", Some(on(ListMsg::Copy(p.id.clone())))),
                panelui::action("Copy name", p.name.clone().map(|n| on(ListMsg::Copy(n)))),
            ]
            .spacing(4),
        );
        let (o1, o2) = (on.clone(), on.clone());
        col = col.push(
            row![
                text_input("Assign a name", &st.name_draft).on_input(move |s| o1(ListMsg::NameDraft(s))).on_submit(o2(ListMsg::SaveName)).size(12).style(style::input()).width(Length::Fill),
                panelui::action("Save", Some(on(ListMsg::SaveName))),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
        col = col.push(note("Names are kept in player_names.txt (uNmINeD playernames.txt format)."));
    }
    col.into()
}

/// A legend list (biomes / blocks): name, swatch, share of the loaded map.
pub fn legend_tab<'a, M: Clone + 'a>(which: Lists, items: &'a [(String, usize, [u8; 3])], st: &'a ListState, on: impl Fn(ListMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let total: usize = items.iter().map(|i| i.1).sum::<usize>().max(1);
    let (ph, what) = if which == Lists::Biomes { ("Search biomes", "biomes") } else { ("Search blocks", "surface blocks") };
    let o = on.clone();
    let mut col = column![search(ph, st.query(which), move |s| o(ListMsg::Search(which, s))), note(format!("{} {what} in the loaded map — click to highlight", items.len()))].spacing(6);
    for (name, n, rgb) in items.iter().filter(|(n, _, _)| hit(st.query(which), n)).take(300) {
        let msg = if which == Lists::Biomes { ListMsg::HighlightBiome(name.clone()) } else { ListMsg::HighlightBlock(name.clone()) };
        let line = row![
            panelui::swatch(*rgb),
            text(name.trim_start_matches("minecraft:").to_string()).size(12).width(Length::Fill),
            text(format!("{:.1} %", *n as f64 * 100.0 / total as f64)).size(11).color(style::muted()),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        col = col.push(row![panelui::item(line, false, on(msg)), panelui::action("⧉", Some(on(ListMsg::Copy(name.clone()))))].spacing(2).align_y(Alignment::Center));
    }
    col.into()
}

/// The **Entities** tab (entities of the visible chunks).
pub fn entities_tab<'a, M: Clone + 'a>(ents: &'a Arc<Vec<omt_render::markers::EntityMarker>>, dim: u8, enabled: bool, st: &'a ListState, on: impl Fn(ListMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let o = on.clone();
    let mut col = column![search("Search entities", &st.entities, move |s| o(ListMsg::Search(Lists::Entities, s)))].spacing(6);
    if !enabled {
        return col.push(note("Turn on the Entities layer (rail) to list entities in view.")).into();
    }
    let matching: Vec<_> = ents.iter().filter(|e| hit(&st.entities, &e.id)).collect();
    col = col.push(note(format!("{} entities in the visible chunks (zoom in to load them)", matching.len())));
    for e in matching.into_iter().take(400) {
        let line = panelui::two_line(e.id.trim_start_matches("minecraft:").to_string(), format!("{:.0}, {:.0}", e.x, e.z));
        col = col.push(panelui::item(line, false, on(ListMsg::GoTo(dim, e.x, e.z))));
    }
    col.into()
}
