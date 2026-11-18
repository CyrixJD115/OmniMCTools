//! Map Viewer **saved map settings** (uNmINeD "Load/save map settings"):
//! named snapshots of the map look — style + lighting, block filters,
//! stylesheet, highlighters and layer toggles — that can be re-applied to
//! any world; one may be the default for newly opened worlds.

use iced::widget::{column, row, text, text_input};
use iced::{Alignment, Element, Length};
use omt_render::style::StyleParams;
use serde::{Deserialize, Serialize};

use crate::panelui::{self, heading, note};
use crate::style;
use crate::viewer::ViewerPrefs;
use crate::viewer_highlight::HlPrefs;
use crate::viewer_rules::MapRules;

/// One saved look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MapPreset {
    pub name: String,
    pub style: StyleParams,
    pub rules: MapRules,
    pub highlights: HlPrefs,
    pub chunk_grid: Option<bool>,
    pub region_grid: bool,
    pub slime: bool,
    pub spawn: bool,
    pub players: bool,
    pub bookmarks: bool,
    pub hud: bool,
    pub textured: bool,
    pub entities: bool,
    pub border: bool,
}

impl MapPreset {
    pub fn capture(name: String, p: &ViewerPrefs) -> Self {
        Self {
            name,
            style: p.style,
            rules: p.rules.clone(),
            highlights: p.highlights.clone(),
            chunk_grid: p.chunk_grid,
            region_grid: p.region_grid,
            slime: p.slime,
            spawn: p.spawn,
            players: p.players,
            bookmarks: p.bookmarks,
            hud: p.hud,
            textured: p.textured,
            entities: p.entities,
            border: p.border,
        }
    }

    pub fn apply_to(&self, p: &mut ViewerPrefs) {
        p.style = self.style;
        p.rules = self.rules.clone();
        p.highlights = self.highlights.clone();
        p.chunk_grid = self.chunk_grid;
        p.region_grid = self.region_grid;
        p.slime = self.slime;
        p.spawn = self.spawn;
        p.players = self.players;
        p.bookmarks = self.bookmarks;
        p.hud = self.hud;
        p.textured = self.textured;
        p.entities = self.entities;
        p.border = self.border;
    }
}

#[derive(Debug, Clone)]
pub enum PresetMsg {
    Name(String),
    Save,
    Load(usize),
    Delete(usize),
    SetDefault(Option<usize>),
}

/// The **Saved settings** tab.
pub fn tab<'a, M: Clone + 'a>(p: &'a ViewerPrefs, draft: &'a str, on: impl Fn(PresetMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let (o1, o2) = (on.clone(), on.clone());
    let mut col = column![
        heading("SAVE CURRENT MAP SETTINGS"),
        row![
            text_input("Settings name", draft).on_input(move |s| o1(PresetMsg::Name(s))).on_submit(o2(PresetMsg::Save)).size(12).style(style::input()).width(Length::Fill),
            panelui::action("Save", Some(on(PresetMsg::Save))),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
        note("Saves style, lighting, block filters, stylesheet, highlighters and layers. Saving under an existing name overwrites it."),
        heading("SAVED SETTINGS"),
    ]
    .spacing(8);
    if p.presets.is_empty() {
        col = col.push(note("Nothing saved yet."));
    }
    for (i, s) in p.presets.iter().enumerate() {
        let is_default = p.default_preset.as_deref() == Some(s.name.as_str());
        col = col.push(
            row![
                text(if is_default { format!("{} (default)", s.name) } else { s.name.clone() }).size(12).width(Length::Fill),
                panelui::action("Load", Some(on(PresetMsg::Load(i)))),
                panelui::action(if is_default { "Undefault" } else { "Default" }, Some(on(PresetMsg::SetDefault(if is_default { None } else { Some(i) })))),
                panelui::action("✕", Some(on(PresetMsg::Delete(i)))),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }
    col.into()
}

/// Apply a preset message to the prefs; returns `true` when the look changed.
pub fn apply(p: &mut ViewerPrefs, draft: &mut String, m: PresetMsg) -> bool {
    match m {
        PresetMsg::Name(s) => *draft = s,
        PresetMsg::Save => {
            let name = match draft.trim() {
                "" => format!("Settings {}", p.presets.len() + 1),
                n => n.to_string(),
            };
            let snap = MapPreset::capture(name.clone(), p);
            match p.presets.iter_mut().find(|s| s.name == name) {
                Some(s) => *s = snap,
                None => p.presets.push(snap),
            }
            draft.clear();
        }
        PresetMsg::Load(i) => {
            if let Some(s) = p.presets.get(i).cloned() {
                s.apply_to(p);
                return true;
            }
        }
        PresetMsg::Delete(i) => {
            if i < p.presets.len() {
                let gone = p.presets.remove(i);
                if p.default_preset.as_deref() == Some(gone.name.as_str()) {
                    p.default_preset = None;
                }
            }
        }
        PresetMsg::SetDefault(i) => p.default_preset = i.and_then(|i| p.presets.get(i)).map(|s| s.name.clone()),
    }
    false
}
