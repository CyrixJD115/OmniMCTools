//! World Settings page (UMT's first editor side-tab): the common level.dat
//! fields with friendly labels and decoded enums (GameType / Difficulty —
//! UMT decodes these in code), spawn, time, seed, and every game rule —
//! across both editions and both Java layout eras (26.3 moved gamerules to
//! `data/minecraft/game_rules.dat` and the seed to `world_gen_settings.dat`).

use std::path::{Path, PathBuf};

use iced::widget::{button, column, container, pick_list, row, scrollable, space, text, text_input, toggler};
use iced::{Element, Length};
use omt_nbt::{NbtDocument, Tag, TagPath};

use crate::style::{self, Btn};

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Text,
    Number,
    Bool,
    /// (stored value, label) — stored as the tag's own type (int or string).
    Enum(Vec<(String, &'static str)>),
    /// Space-separated ints edited into an IntArray (26.3 `spawn.pos`).
    IntTriple,
}

#[derive(Debug, Clone)]
struct Field {
    file: usize,
    path: String,
    label: String,
    desc: &'static str,
    kind: Kind,
    draft: String,
    original: String,
    group: &'static str,
}

#[derive(Debug, Clone)]
pub enum WsMsg {
    Edit(usize, String),
    Apply,
    Revert,
}

pub struct WorldSettings {
    root: PathBuf,
    files: Vec<(PathBuf, NbtDocument)>,
    fields: Vec<Field>,
    bedrock: bool,
    pub status: String,
}

fn tag_text(t: &Tag) -> String {
    match t {
        Tag::Byte(v) => v.to_string(),
        Tag::Short(v) => v.to_string(),
        Tag::Int(v) => v.to_string(),
        Tag::Long(v) => v.to_string(),
        Tag::Float(v) => v.to_string(),
        Tag::Double(v) => v.to_string(),
        Tag::String(s) => s.clone(),
        Tag::IntArray(v) => v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(" "),
        other => omt_nbt::snbt::to_snbt(other),
    }
}

/// Parse `draft` into a tag of the same type as `like`.
fn parse_like(like: &Tag, draft: &str) -> Result<Tag, String> {
    let d = draft.trim();
    let bad = |what: &str| format!("\"{d}\" is not a valid {what}");
    Ok(match like {
        Tag::Byte(_) => Tag::Byte(d.parse().map_err(|_| bad("byte"))?),
        Tag::Short(_) => Tag::Short(d.parse().map_err(|_| bad("short"))?),
        Tag::Int(_) => Tag::Int(d.parse().map_err(|_| bad("integer"))?),
        Tag::Long(_) => Tag::Long(d.parse().map_err(|_| bad("long"))?),
        Tag::Float(_) => Tag::Float(d.parse().map_err(|_| bad("number"))?),
        Tag::Double(_) => Tag::Double(d.parse().map_err(|_| bad("number"))?),
        Tag::String(_) => Tag::String(draft.to_string()),
        Tag::IntArray(_) => Tag::IntArray(d.split_whitespace().map(|p| p.parse::<i32>().map_err(|_| bad("list of integers"))).collect::<Result<_, _>>()?),
        _ => return Err("this field type cannot be edited here".into()),
    })
}

const GAME_TYPES: [(&str, &str); 4] = [("0", "Survival"), ("1", "Creative"), ("2", "Adventure"), ("3", "Spectator")];
const DIFFICULTY_INT: [(&str, &str); 4] = [("0", "Peaceful"), ("1", "Easy"), ("2", "Normal"), ("3", "Hard")];
const DIFFICULTY_STR: [(&str, &str); 4] = [("peaceful", "Peaceful"), ("easy", "Easy"), ("normal", "Normal"), ("hard", "Hard")];

fn enum_kind(opts: &[(&str, &'static str)]) -> Kind {
    Kind::Enum(opts.iter().map(|(v, l)| (v.to_string(), *l)).collect())
}

impl WorldSettings {
    pub fn load(root: &Path, bedrock: bool) -> Self {
        let mut ws = Self { root: root.to_path_buf(), files: Vec::new(), fields: Vec::new(), bedrock, status: String::new() };
        let candidates: &[&str] = if bedrock {
            &["level.dat"]
        } else {
            &["level.dat", "data/minecraft/game_rules.dat", "data/minecraft/world_gen_settings.dat"]
        };
        for rel in candidates {
            let p = root.join(rel);
            if let Ok(doc) = NbtDocument::load(&p) {
                ws.files.push((p, doc));
            }
        }
        ws.build_fields();
        ws
    }

    fn get(&self, file: usize, path: &str) -> Option<&Tag> {
        let tp: TagPath = path.parse().ok()?;
        tp.resolve(self.files.get(file)?.1.root())
    }

    fn add(&mut self, file: usize, path: &str, label: &str, desc: &'static str, kind: Kind, group: &'static str) {
        let Some(tag) = self.get(file, path) else { return };
        let mut kind = kind;
        if kind == Kind::Number && matches!(tag, Tag::Byte(_)) && path.rsplit('.').next().is_some_and(|k| k.starts_with("minecraft:") || k.chars().all(|c| c.is_ascii_lowercase())) {
            kind = Kind::Bool;
        }
        let v = tag_text(tag);
        self.fields.push(Field { file, path: path.into(), label: label.into(), desc, kind, draft: v.clone(), original: v, group });
    }

    fn build_fields(&mut self) {
        self.fields.clear();
        if self.files.is_empty() {
            return;
        }
        if self.bedrock {
            self.add(0, "LevelName", "World Name", "Shown in the world list (also written to levelname.txt).", Kind::Text, "World");
            self.add(0, "GameType", "Game Mode", "Default game mode for players.", enum_kind(&GAME_TYPES[..3]), "World");
            self.add(0, "Difficulty", "Difficulty", "Peaceful, Easy, Normal or Hard.", enum_kind(&DIFFICULTY_INT), "World");
            self.add(0, "commandsEnabled", "Cheats", "Allow commands (disables achievements).", Kind::Bool, "World");
            self.add(0, "RandomSeed", "Seed", "World generation seed.", Kind::Number, "World");
            self.add(0, "SpawnX", "Spawn X", "World spawn X.", Kind::Number, "Spawn");
            self.add(0, "SpawnY", "Spawn Y", "World spawn Y (32767 = find surface).", Kind::Number, "Spawn");
            self.add(0, "SpawnZ", "Spawn Z", "World spawn Z.", Kind::Number, "Spawn");
            self.add(0, "Time", "Time of Day", "World time in ticks (24000 per day).", Kind::Number, "Time");
            self.add(0, "currentTick", "Current Tick", "Total ticks the world has run.", Kind::Number, "Time");
            let rules: Vec<String> = self.files[0]
                .1
                .root()
                .as_compound()
                .map(|c| {
                    c.iter()
                        .filter(|(k, v)| matches!(v, Tag::Byte(_) | Tag::Int(_)) && k.chars().all(|ch| ch.is_ascii_lowercase()))
                        .map(|(k, _)| k.clone())
                        .collect()
                })
                .unwrap_or_default();
            for k in rules {
                self.add(0, &k, &k, "Game rule.", Kind::Number, "Game Rules");
            }
        } else {
            let gametype = enum_kind(&GAME_TYPES);
            self.add(0, "Data.LevelName", "World Name", "Shown in the world list.", Kind::Text, "World");
            self.add(0, "Data.GameType", "Game Mode", "Default game mode for players.", gametype, "World");
            if self.get(0, "Data.difficulty_settings.difficulty").is_some() {
                self.add(0, "Data.difficulty_settings.difficulty", "Difficulty", "Peaceful, Easy, Normal or Hard.", enum_kind(&DIFFICULTY_STR), "World");
                self.add(0, "Data.difficulty_settings.hardcore", "Hardcore", "One life; the world is deleted or spectated on death.", Kind::Bool, "World");
                self.add(0, "Data.difficulty_settings.locked", "Difficulty Locked", "Prevents changing the difficulty in game.", Kind::Bool, "World");
            } else {
                self.add(0, "Data.Difficulty", "Difficulty", "Peaceful, Easy, Normal or Hard.", enum_kind(&DIFFICULTY_INT), "World");
                self.add(0, "Data.hardcore", "Hardcore", "One life.", Kind::Bool, "World");
            }
            self.add(0, "Data.allowCommands", "Cheats", "Allow commands.", Kind::Bool, "World");
            if let Some(f) = self.files.iter().position(|(p, _)| p.ends_with("world_gen_settings.dat")) {
                self.add(f, "data.seed", "Seed", "World generation seed.", Kind::Number, "World");
            } else {
                self.add(0, "Data.WorldGenSettings.seed", "Seed", "World generation seed.", Kind::Number, "World");
            }
            if self.get(0, "Data.spawn.pos").is_some() {
                self.add(0, "Data.spawn.pos", "Spawn Position", "World spawn as X Y Z.", Kind::IntTriple, "Spawn");
                self.add(0, "Data.spawn.dimension", "Spawn Dimension", "Dimension the world spawn is in.", Kind::Text, "Spawn");
            } else {
                for (k, l) in [("SpawnX", "Spawn X"), ("SpawnY", "Spawn Y"), ("SpawnZ", "Spawn Z")] {
                    self.add(0, &format!("Data.{k}"), l, "World spawn.", Kind::Number, "Spawn");
                }
            }
            self.add(0, "Data.Time", "Game Time", "Total ticks the world has run.", Kind::Number, "Time");
            self.add(0, "Data.DayTime", "Time of Day", "Ticks into the day cycle (24000 per day).", Kind::Number, "Time");
            if let Some(f) = self.files.iter().position(|(p, _)| p.ends_with("game_rules.dat")) {
                let keys: Vec<String> = self.files[f].1.root().get("data").and_then(|t| t.as_compound()).map(|c| c.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default();
                for k in keys {
                    let label = k.trim_start_matches("minecraft:").replace('_', " ");
                    self.add(f, &format!("data.\"{k}\""), &label, "Game rule.", Kind::Number, "Game Rules");
                }
            } else {
                let keys: Vec<String> = self.get(0, "Data.GameRules").and_then(|t| t.as_compound()).map(|c| c.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default();
                for k in keys {
                    self.add(0, &format!("Data.GameRules.{k}"), &k, "Game rule (stored as text).", Kind::Text, "Game Rules");
                }
            }
        }
    }

    pub fn dirty(&self) -> bool {
        self.fields.iter().any(|f| f.draft != f.original)
    }

    /// Returns true when files were written (the session becomes modified).
    pub fn update(&mut self, msg: WsMsg) -> bool {
        match msg {
            WsMsg::Edit(i, v) => {
                if let Some(f) = self.fields.get_mut(i) {
                    f.draft = v;
                }
            }
            WsMsg::Revert => {
                for f in &mut self.fields {
                    f.draft = f.original.clone();
                }
                self.status = "reverted".into();
            }
            WsMsg::Apply => return self.apply(),
        }
        false
    }

    fn apply(&mut self) -> bool {
        let mut touched = std::collections::BTreeSet::new();
        for f in &self.fields {
            if f.draft == f.original {
                continue;
            }
            let Ok(tp) = f.path.parse::<TagPath>() else { continue };
            let Some(doc) = self.files.get_mut(f.file).map(|(_, d)| d) else { continue };
            let Some(slot) = tp.resolve_mut(doc.root_mut()) else { continue };
            match parse_like(slot, &f.draft) {
                Ok(v) => {
                    *slot = v;
                    touched.insert(f.file);
                }
                Err(e) => {
                    self.status = format!("{}: {e}", f.label);
                    return false;
                }
            }
        }
        for i in &touched {
            let (path, doc) = &self.files[*i];
            let res = doc.to_bytes().map_err(|e| e.to_string()).and_then(|bytes| {
                let tmp = path.with_extension("dat.omt_tmp");
                std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, path).map_err(|e| e.to_string())
            });
            if let Err(e) = res {
                self.status = format!("write failed: {e}");
                return false;
            }
        }
        if self.bedrock
            && let Some(name) = self.fields.iter().find(|f| f.path == "LevelName" && f.draft != f.original)
        {
            let _ = std::fs::write(self.root.join("levelname.txt"), &name.draft);
        }
        for f in &mut self.fields {
            f.original = f.draft.clone();
        }
        self.status = format!("applied to {} file(s) in the workspace — Save the world to keep it", touched.len());
        !touched.is_empty()
    }

    pub fn view(&self) -> Element<'_, WsMsg> {
        let mut col = column![].spacing(6);
        let mut group = "";
        for (i, f) in self.fields.iter().enumerate() {
            if f.group != group {
                group = f.group;
                col = col.push(text(group).size(16).color(style::accent()));
            }
            let control: Element<'_, WsMsg> = match &f.kind {
                Kind::Bool => toggler(f.draft == "1").on_toggle(move |b| WsMsg::Edit(i, if b { "1" } else { "0" }.into())).into(),
                Kind::Enum(opts) => {
                    let labels: Vec<EnumOpt> = opts.iter().map(|(v, l)| EnumOpt(v.clone(), l)).collect();
                    let current = labels.iter().find(|o| o.0 == f.draft).cloned();
                    pick_list(labels, current, move |o: EnumOpt| WsMsg::Edit(i, o.0)).width(180).into()
                }
                _ => text_input("", &f.draft).on_input(move |v| WsMsg::Edit(i, v)).width(240).style(style::input()).into(),
            };
            let changed = f.draft != f.original;
            col = col.push(
                container(
                    row![
                        column![
                            text(f.label.clone()).size(14).color(if changed { style::warn() } else { style::fg() }),
                            text(format!("{} · {}", f.desc, f.path)).size(11).color(style::muted()),
                        ]
                        .width(Length::Fill),
                        control,
                    ]
                    .spacing(12)
                    .align_y(iced::Alignment::Center),
                )
                .padding(8)
                .style(style::card()),
            );
        }
        let bar = row![
            text("World Settings").size(18),
            space::horizontal(),
            text(self.status.clone()).size(12).color(style::muted()),
            button(text("Revert").size(12)).on_press_maybe(self.dirty().then_some(WsMsg::Revert)).style(style::button_style(Btn::Standard)),
            button(text("Apply").size(12)).on_press_maybe(self.dirty().then_some(WsMsg::Apply)).style(style::button_style(Btn::Confirm)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        let body: Element<'_, WsMsg> = if self.fields.is_empty() {
            text("level.dat could not be read.").color(style::muted()).into()
        } else {
            scrollable(col.padding([0, 4])).height(Length::Fill).into()
        };
        column![bar, body].spacing(10).padding(12).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnumOpt(String, &'static str);

impl std::fmt::Display for EnumOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_like_types() {
        assert_eq!(parse_like(&Tag::Byte(0), "1").unwrap(), Tag::Byte(1));
        assert_eq!(parse_like(&Tag::Long(0), "-5").unwrap(), Tag::Long(-5));
        assert_eq!(parse_like(&Tag::IntArray(vec![]), "1 2 3").unwrap(), Tag::IntArray(vec![1, 2, 3]));
        assert!(parse_like(&Tag::Int(0), "x").is_err());
        assert_eq!(parse_like(&Tag::String("a".into()), "hard").unwrap(), Tag::String("hard".into()));
    }

    #[test]
    fn java_26_fixture_fields() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../world_ref/java-26.3");
        if !root.join("level.dat").is_file() {
            return; // fixture-gated
        }
        let ws = WorldSettings::load(&root, false);
        let labels: Vec<&str> = ws.fields.iter().map(|f| f.label.as_str()).collect();
        assert!(labels.contains(&"Difficulty") && labels.contains(&"Seed") && labels.contains(&"Spawn Position"));
        assert!(ws.fields.iter().filter(|f| f.group == "Game Rules").count() > 40, "26.3 game_rules.dat read");
        let diff = ws.fields.iter().find(|f| f.label == "Difficulty").unwrap();
        assert_eq!(diff.draft, "normal");
    }
}
