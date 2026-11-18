//! Map Viewer **Chunk inspector** + region properties (uNmINeD parity):
//! click-select a chunk on the map (or type its coordinates), then a
//! background report shows what the chunk holds — format, status, time,
//! sections, top blocks, biomes, block entities, entities — and, for Java,
//! its region file and the chunk NBT (copyable as SNBT); for Bedrock, its
//! database records.

use std::sync::Arc;

use iced::widget::{column, row, text, text_input};
use iced::{Alignment, Element, Length};
use omt_world::{Chunk, WorldStore};

use crate::panelui::{self, heading, kv, note, switch};
use crate::session::Store;
use crate::style;

#[derive(Debug, Clone)]
pub enum InspectMsg {
    ClickSelect(bool),
    X(String),
    Z(String),
    Locate,
    Select(i32, i32),
    Report(Arc<Report>),
    CopySnbt,
}

/// Inspector state.
#[derive(Debug)]
pub struct Inspect {
    pub click_select: bool,
    pub text: (String, String),
    pub selected: Option<(i32, i32)>,
    pub report: Option<Arc<Report>>,
    pub busy: bool,
}

impl Default for Inspect {
    fn default() -> Self {
        Self { click_select: true, text: (String::new(), String::new()), selected: None, report: None, busy: false }
    }
}

/// What the inspector shows for one chunk.
#[derive(Debug, Default)]
pub struct Report {
    pub cx: i32,
    pub cz: i32,
    pub exists: bool,
    /// `label → value` facts.
    pub facts: Vec<(String, String)>,
    /// Region / record facts (Java region file, Bedrock records).
    pub storage: Vec<(String, String)>,
    pub blocks: Vec<(String, usize)>,
    pub biomes: Vec<String>,
    pub block_entities: Vec<(String, usize)>,
    pub entities: Vec<(String, usize)>,
    /// Java chunk NBT as pretty SNBT (copy).
    pub snbt: Option<String>,
}

fn histogram(items: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut m: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for i in items {
        *m.entry(i).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

fn id_of(t: &omt_nbt::Tag) -> String {
    t.get("id").or_else(|| t.get("identifier")).and_then(omt_nbt::Tag::as_str).unwrap_or("?").to_string()
}

fn ticks(t: i64) -> String {
    let s = t / 20;
    format!("{t} ticks ({}h {:02}m {:02}s)", s / 3600, (s / 60) % 60, s % 60)
}

/// Build the report (blocking; run on a worker).
pub fn report(store: &Store, dim: u8, cx: i32, cz: i32) -> Report {
    let mut r = Report { cx, cz, ..Default::default() };
    r.facts.push(("Chunk".into(), format!("{cx}, {cz}")));
    r.facts.push(("Blocks".into(), format!("{}, {} → {}, {}", cx * 16, cz * 16, cx * 16 + 15, cz * 16 + 15)));
    r.facts.push(("Region".into(), format!("r.{}.{}  (local {}, {})", cx.div_euclid(32), cz.div_euclid(32), cx.rem_euclid(32), cz.rem_euclid(32))));
    let chunk: Option<Chunk> = match store {
        Store::Java(w) => {
            let path = w.region_path(dim, cx.div_euclid(32), cz.div_euclid(32));
            r.storage.push(("Region file".into(), path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()));
            if let Ok(meta) = std::fs::metadata(&path) {
                r.storage.push(("Region size".into(), format!("{} KB", meta.len() / 1024)));
            }
            if let Ok(region) = omt_java::RegionFile::open(&path) {
                r.storage.push(("Chunks in region".into(), region.chunk_count().to_string()));
                if let Some(ts) = region.timestamp(cx, cz) {
                    r.storage.push(("Saved (timestamp)".into(), ts.to_string()));
                }
                if let Ok(raw) = region.read_raw_from_file(cx, cz) {
                    r.storage.push(("Compressed size".into(), format!("{} bytes", raw.len())));
                }
            }
            if let Ok(Some(nbt)) = w.read_chunk_nbt(dim, cx, cz) {
                r.snbt = Some(omt_nbt::snbt::to_snbt_pretty(&nbt.tag));
            }
            w.read_chunk(dim, cx, cz).ok().flatten()
        }
        Store::Bedrock(w) => {
            if let Ok(mut recs) = w.chunk_records(dim, cx, cz) {
                // Records by tag, sub-chunks bottom to top.
                recs.sort_by_key(|(k, _)| match omt_bedrock::keys::decode_key(k) {
                    omt_bedrock::keys::DbKey::Subchunk { y, .. } => (0x2f, y as i32),
                    omt_bedrock::keys::DbKey::ChunkRecord { tag, .. } => (tag, i32::MIN),
                    _ => (u8::MAX, 0),
                });
                for (k, v) in recs {
                    let label = match omt_bedrock::keys::decode_key(&k) {
                        omt_bedrock::keys::DbKey::Subchunk { y, .. } => format!("SubChunk y={y} (v{})", v.first().copied().unwrap_or(0)),
                        omt_bedrock::keys::DbKey::ChunkRecord { tag, .. } => bedrock_tag_name(tag).to_string(),
                        _ => "record".into(),
                    };
                    r.storage.push((label, format!("{} bytes", v.len())));
                }
            }
            w.read_chunk(dim, cx, cz).ok().flatten()
        }
    };
    let Some(c) = chunk else {
        r.facts.push(("State".into(), "not generated / not saved".into()));
        return r;
    };
    r.exists = true;
    if c.data_version > 0 {
        let label = if matches!(store, Store::Java(_)) { "DataVersion" } else { "Chunk version" };
        r.facts.push((label.into(), c.data_version.to_string()));
    }
    if let Some(s) = &c.status {
        r.facts.push(("Status".into(), s.trim_start_matches("minecraft:").to_string()));
    }
    if matches!(store, Store::Java(_)) {
        r.facts.push(("Inhabited time".into(), ticks(c.inhabited_time)));
    }
    if let (Some(lo), Some(hi)) = (c.slices.keys().next(), c.slices.keys().next_back()) {
        r.facts.push(("Sections".into(), format!("{} (Y {} → {})", c.slices.len(), *lo as i32 * 16, *hi as i32 * 16 + 15)));
    }
    r.blocks = histogram(c.slices.values().flat_map(|s| s.indices.iter().filter_map(|&i| s.palette.get(i as usize)).filter(|b| b.name != omt_world::BlockState::AIR && b.name != "minecraft:cave_air" && b.name != "minecraft:void_air").map(|b| b.name.clone())));
    r.facts.push(("Non-air blocks".into(), r.blocks.iter().map(|(_, n)| n).sum::<usize>().to_string()));
    let mut biomes: Vec<String> = c.java_biomes.values().flat_map(|b| b.palette.iter().cloned()).collect();
    if let Some(slots) = &c.bedrock_biomes {
        let pal = omt_render::Palette::embedded();
        for s in slots {
            if let omt_world::BedrockBiomeSlot::Storage { palette, .. } = s {
                biomes.extend(palette.iter().map(|&id| pal.bedrock_biome_name(id).map_or(format!("#{id}"), str::to_owned)));
            }
        }
    }
    biomes.sort();
    biomes.dedup();
    r.biomes = biomes;
    r.block_entities = histogram(c.block_entities.iter().map(id_of));
    let mut ents: Vec<String> = c.entities.iter().map(id_of).collect();
    if let Store::Java(w) = store
        && ents.is_empty()
    {
        ents = omt_render::markers::java_entities(w, dim, &[(cx, cz)]).into_iter().map(|e| e.id).collect();
    }
    r.entities = histogram(ents.into_iter());
    r
}

fn bedrock_tag_name(tag: u8) -> &'static str {
    match tag {
        0x2b => "Data3D (heights + biomes)",
        0x2c => "Version",
        0x2d => "Data2D (legacy biomes)",
        0x2e => "Data2DLegacy",
        0x2f => "SubChunkPrefix",
        0x30 => "LegacyTerrain",
        0x31 => "BlockEntity",
        0x32 => "Entity (legacy)",
        0x33 => "PendingTicks",
        0x35 => "BiomeState",
        0x36 => "FinalizedState",
        0x39 => "HardcodedSpawners",
        0x3a => "RandomTicks",
        0x3b => "Checksums",
        0x3d => "MetaDataHash",
        0x3f => "BlendingData",
        0x40 => "ActorDigestVersion",
        0x76 => "VersionOld",
        0x77 => "StructureRefs",
        _ => "Other record",
    }
}

/// The **Chunk inspector** tab.
pub fn tab<'a, M: Clone + 'a>(st: &'a Inspect, on: impl Fn(InspectMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let (o1, o2, o3, o4) = (on.clone(), on.clone(), on.clone(), on.clone());
    let mut col = column![
        switch("Select chunks by clicking the map", st.click_select, move |v| o1(InspectMsg::ClickSelect(v))),
        row![
            text("Chunk").size(12),
            text_input("X", &st.text.0).on_input(move |s| o2(InspectMsg::X(s))).on_submit(o4(InspectMsg::Locate)).size(12).width(70).style(style::input()),
            text_input("Z", &st.text.1).on_input(move |s| o3(InspectMsg::Z(s))).on_submit(on(InspectMsg::Locate)).size(12).width(70).style(style::input()),
            panelui::action("Locate", Some(on(InspectMsg::Locate))),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    ]
    .spacing(8);
    let Some(r) = &st.report else {
        return col.push(note(if st.busy { "Reading the chunk…" } else { "Select a chunk on the map to view its contents." })).into();
    };
    if st.busy {
        col = col.push(note("Reading the chunk…"));
    }
    col = col.push(heading("CHUNK"));
    for (k, v) in &r.facts {
        col = col.push(kv(k.clone(), v.clone()));
    }
    if !r.storage.is_empty() {
        col = col.push(heading(if r.snbt.is_some() { "REGION FILE" } else { "DATABASE RECORDS" }));
        for (k, v) in &r.storage {
            col = col.push(kv(k.clone(), v.clone()));
        }
    }
    if !r.exists {
        return col.into();
    }
    let list = |title: &'a str, items: &'a [(String, usize)], mut col: iced::widget::Column<'a, M>| {
        if items.is_empty() {
            return col;
        }
        col = col.push(heading(title));
        for (name, n) in items.iter().take(24) {
            col = col.push(row![text(name.trim_start_matches("minecraft:").to_string()).size(12).width(Length::Fill), text(n.to_string()).size(11).color(style::muted())].spacing(6));
        }
        if items.len() > 24 {
            col = col.push(note(format!("… and {} more", items.len() - 24)));
        }
        col
    };
    col = list("BLOCKS", &r.blocks, col);
    if !r.biomes.is_empty() {
        col = col.push(heading("BIOMES")).push(text(r.biomes.iter().map(|b| b.trim_start_matches("minecraft:")).collect::<Vec<_>>().join(", ")).size(12));
    }
    col = list("BLOCK ENTITIES", &r.block_entities, col);
    col = list("ENTITIES", &r.entities, col);
    if r.snbt.is_some() {
        col = col.push(panelui::action("Copy chunk NBT as SNBT", Some(on(InspectMsg::CopySnbt))));
    }
    col.into()
}
