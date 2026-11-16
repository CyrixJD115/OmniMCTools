//! `level.dat` mapping between editions (M4): world name, game mode,
//! difficulty, hardcore, cheats, seed, spawn, time and game rules.
//!
//! Bedrock output starts from a neutral **current-version default
//! compound** (key set + types observed in 1.26.52 fixture worlds, values
//! reset to new-world defaults — no template/marketplace markers) and then
//! overlays the mapped Java values. Java output writes the 26.3 `Data`
//! shape (`difficulty_settings`, `spawn{pos}`) **plus** the legacy flat
//! fields, so the game finds what it needs either way; game rules go to
//! `data/minecraft/game_rules.dat` (26.3) as typed values.

use omt_nbt::{BedrockLevelDat, ByteOrder, Container, Nbt, Tag};

use crate::error::{ConvertError, Result};

fn b(v: bool) -> Tag {
    Tag::Byte(v as i8)
}

/// Neutral Bedrock level.dat defaults for the pinned release.
pub fn bedrock_defaults() -> Vec<(String, Tag)> {
    let reg = omt_versions::registry();
    let ver: Vec<i32> = reg.bedrock_level_version().iter().map(|&v| v as i32).collect();
    let mut c: Vec<(&str, Tag)> = vec![
        ("CenterMapsToOrigin", b(false)),
        ("ConfirmedPlatformLockedContent", b(false)),
        ("Difficulty", Tag::Int(2)),
        ("ForceGameType", b(false)),
        ("GameType", Tag::Int(0)),
        ("Generator", Tag::Int(1)),
        ("HasUncompleteWorldFileOnDisk", b(false)),
        ("IsHardcore", b(false)),
        ("LANBroadcast", b(true)),
        ("LANBroadcastIntent", b(true)),
        ("LastPlayed", Tag::Long(0)),
        ("LevelName", Tag::String("Converted World".into())),
        ("LimitedWorldOriginX", Tag::Int(0)),
        ("LimitedWorldOriginY", Tag::Int(32767)),
        ("LimitedWorldOriginZ", Tag::Int(0)),
        ("MinimumCompatibleClientVersion", Tag::List(ver.iter().map(|&v| Tag::Int(v)).collect())),
        ("MultiplayerGame", b(true)),
        ("MultiplayerGameIntent", b(true)),
        ("NetherScale", Tag::Int(8)),
        ("Platform", Tag::Int(2)),
        ("PlatformBroadcastIntent", Tag::Int(3)),
        ("RandomSeed", Tag::Long(0)),
        ("SpawnV1Villagers", b(false)),
        ("SpawnX", Tag::Int(0)),
        ("SpawnY", Tag::Int(32767)),
        ("SpawnZ", Tag::Int(0)),
        ("StorageVersion", Tag::Int(reg.bedrock_storage_version() as i32)),
        ("Time", Tag::Long(0)),
        ("WorldVersion", Tag::Int(1)),
        ("XBLBroadcastIntent", Tag::Int(3)),
        ("baseGameVersion", Tag::String("*".into())),
        ("bonusChestEnabled", b(false)),
        ("bonusChestSpawned", b(false)),
        ("cheatsEnabled", b(false)),
        ("commandsEnabled", b(false)),
        ("currentTick", Tag::Long(0)),
        ("experiments", Tag::Compound(vec![("experiments_ever_used".into(), b(false)), ("saved_with_toggled_experiments".into(), b(false))])),
        ("hasBeenLoadedInCreative", b(false)),
        ("immutableWorld", b(false)),
        ("isFromWorldTemplate", b(false)),
        ("isWorldTemplateOptionLocked", b(false)),
        ("lastOpenedWithVersion", Tag::List(ver.iter().map(|&v| Tag::Int(v)).collect())),
        ("lightningLevel", Tag::Float(0.0)),
        ("lightningTime", Tag::Int(100_000)),
        ("rainLevel", Tag::Float(0.0)),
        ("rainTime", Tag::Int(100_000)),
        ("spawnMobs", b(true)),
        ("texturePacksRequired", b(false)),
        ("useMsaGamertagsOnly", b(false)),
        ("worldStartCount", Tag::Long(0)),
        ("world_policies", Tag::Compound(vec![])),
    ];
    // Bedrock game rules (lowercase keys) with vanilla defaults.
    for (k, v) in [
        ("commandblockoutput", true), ("commandblocksenabled", true), ("dodaylightcycle", true), ("doentitydrops", true),
        ("dofiretick", true), ("doimmediaterespawn", false), ("doinsomnia", true), ("dolimitedcrafting", false),
        ("domobloot", true), ("domobspawning", true), ("dotiledrops", true), ("doweathercycle", true),
        ("drowningdamage", true), ("falldamage", true), ("firedamage", true), ("freezedamage", true),
        ("keepinventory", false), ("mobgriefing", true), ("naturalregeneration", true), ("projectilescanbreakblocks", true),
        ("pvp", true), ("recipesunlock", true), ("respawnblocksexplode", true), ("sendcommandfeedback", true),
        ("showbordereffect", true), ("showcoordinates", false), ("showdaysplayed", false), ("showdeathmessages", true),
        ("showrecipemessages", true), ("showtags", true), ("tntexplodes", true), ("tntexplosiondropdecay", false),
    ] {
        c.push((k, b(v)));
    }
    for (k, v) in [("functioncommandlimit", 10000), ("maxcommandchainlength", 65535), ("playerssleepingpercentage", 100), ("randomtickspeed", 1), ("spawnradius", 10)] {
        c.push((k, Tag::Int(v)));
    }
    c.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn set(c: &mut [(String, Tag)], key: &str, v: Tag) {
    if let Some(slot) = c.iter_mut().find(|(k, _)| k == key) {
        slot.1 = v;
    }
}

/// Bedrock rule key for a Java rule name: `minecraft:keep_inventory`
/// (26.3) or `keepInventory` (legacy) → `keepinventory`.
pub fn bedrock_rule_key(java: &str) -> String {
    java.trim_start_matches("minecraft:").replace('_', "").to_lowercase()
}

/// Java 26.3 rule key for a Bedrock rule, when the names line up.
fn java_rule_aliases(bedrock: &str) -> Option<&'static str> {
    Some(match bedrock {
        "keepinventory" => "minecraft:keep_inventory",
        "dodaylightcycle" => "minecraft:advance_time",
        "doweathercycle" => "minecraft:advance_weather",
        "mobgriefing" => "minecraft:mob_griefing",
        "domobspawning" => "minecraft:spawn_mobs",
        "dofiretick" => "minecraft:fire_spread_radius_around_player",
        "falldamage" => "minecraft:fall_damage",
        "firedamage" => "minecraft:fire_damage",
        "drowningdamage" => "minecraft:drowning_damage",
        "freezedamage" => "minecraft:freeze_damage",
        "naturalregeneration" => "minecraft:natural_health_regeneration",
        "showdeathmessages" => "minecraft:show_death_messages",
        "doimmediaterespawn" => "minecraft:immediate_respawn",
        "tntexplodes" => "minecraft:tnt_explodes",
        "randomtickspeed" => "minecraft:random_tick_speed",
        "spawnradius" => "minecraft:respawn_radius",
        "sendcommandfeedback" => "minecraft:send_command_feedback",
        "commandblockoutput" => "minecraft:command_block_output",
        "dotiledrops" => "minecraft:block_drops",
        "doentitydrops" => "minecraft:entity_drops",
        "domobloot" => "minecraft:mob_drops",
        "dolimitedcrafting" => "minecraft:limited_crafting",
        "playerssleepingpercentage" => "minecraft:players_sleeping_percentage",
        "doinsomnia" => "minecraft:spawn_phantoms",
        "pvp" => "minecraft:pvp",
        _ => return None,
    })
}

/// Everything read out of a Java world's settings files.
#[derive(Debug, Clone, Default)]
pub struct JavaSettings {
    pub name: Option<String>,
    pub game_type: Option<i32>,
    pub difficulty: Option<i32>,
    pub hardcore: Option<bool>,
    pub cheats: Option<bool>,
    pub seed: Option<i64>,
    pub spawn: Option<(i32, i32, i32)>,
    pub time: Option<i64>,
    pub day_time: Option<i64>,
    /// (rule name as stored, value) — typed (26.3) or textual (legacy).
    pub rules: Vec<(String, Tag)>,
}

fn read_gz(path: &std::path::Path) -> Option<Nbt> {
    let bytes = std::fs::read(path).ok()?;
    let (payload, _) = Container::decode(&bytes).ok()?;
    omt_nbt::read(&payload, ByteOrder::Big).ok()
}

/// Read a Java world's settings across both layout eras.
pub fn read_java_settings(root: &std::path::Path) -> JavaSettings {
    let mut s = JavaSettings::default();
    let Some(level) = read_gz(&root.join("level.dat")) else { return s };
    let data = level.tag.get("Data").unwrap_or(&level.tag);
    let int = |t: Option<&Tag>| t.and_then(|t| t.as_i64());
    s.name = data.get("LevelName").and_then(|t| t.as_str()).map(str::to_owned);
    s.game_type = int(data.get("GameType")).map(|v| v as i32);
    s.difficulty = match data.get("difficulty_settings").and_then(|d| d.get("difficulty")) {
        Some(Tag::String(d)) => ["peaceful", "easy", "normal", "hard"].iter().position(|x| x == d).map(|i| i as i32),
        _ => int(data.get("Difficulty")).map(|v| v as i32),
    };
    s.hardcore = int(data.get("difficulty_settings").and_then(|d| d.get("hardcore")).or(data.get("hardcore"))).map(|v| v != 0);
    s.cheats = int(data.get("allowCommands")).map(|v| v != 0);
    s.seed = read_gz(&root.join("data/minecraft/world_gen_settings.dat"))
        .and_then(|n| int(n.tag.get("data").and_then(|d| d.get("seed"))))
        .or_else(|| int(data.get("WorldGenSettings").and_then(|w| w.get("seed"))))
        .or_else(|| int(data.get("RandomSeed")));
    s.spawn = match data.get("spawn").and_then(|sp| sp.get("pos")) {
        Some(Tag::IntArray(p)) if p.len() == 3 => Some((p[0], p[1], p[2])),
        _ => match (int(data.get("SpawnX")), int(data.get("SpawnY")), int(data.get("SpawnZ"))) {
            (Some(x), Some(y), Some(z)) => Some((x as i32, y as i32, z as i32)),
            _ => None,
        },
    };
    s.time = int(data.get("Time"));
    s.day_time = int(data.get("DayTime"));
    if let Some(rules) = read_gz(&root.join("data/minecraft/game_rules.dat")).and_then(|n| n.tag.get("data").and_then(|d| d.as_compound()).map(<[_]>::to_vec)) {
        s.rules = rules;
    } else if let Some(rules) = data.get("GameRules").and_then(|g| g.as_compound()) {
        s.rules = rules.to_vec();
    }
    s
}

/// Build a Bedrock `level.dat` (with header) from Java settings.
pub fn bedrock_level_dat(java: &JavaSettings, game_mode_override: Option<i32>) -> Result<Vec<u8>> {
    let mut c = bedrock_defaults();
    if let Some(n) = &java.name {
        set(&mut c, "LevelName", Tag::String(n.clone()));
    }
    // Java spectator (3) → Bedrock spectator (6).
    let gm = game_mode_override.or(java.game_type).map(|g| if g == 3 { 6 } else { g });
    if let Some(g) = gm {
        set(&mut c, "GameType", Tag::Int(g));
    }
    if let Some(d) = java.difficulty {
        set(&mut c, "Difficulty", Tag::Int(d));
    }
    if let Some(h) = java.hardcore {
        set(&mut c, "IsHardcore", b(h));
    }
    if let Some(ch) = java.cheats {
        set(&mut c, "commandsEnabled", b(ch));
        set(&mut c, "cheatsEnabled", b(ch));
    }
    if let Some(seed) = java.seed {
        set(&mut c, "RandomSeed", Tag::Long(seed));
    }
    if let Some((x, y, z)) = java.spawn {
        set(&mut c, "SpawnX", Tag::Int(x));
        set(&mut c, "SpawnY", Tag::Int(y));
        set(&mut c, "SpawnZ", Tag::Int(z));
    }
    if let Some(t) = java.time {
        set(&mut c, "currentTick", Tag::Long(t));
    }
    if let Some(t) = java.day_time.or(java.time) {
        set(&mut c, "Time", Tag::Long(t));
    }
    for (name, value) in &java.rules {
        let key = bedrock_rule_key(name);
        let Some(slot) = c.iter_mut().find(|(k, _)| *k == key) else { continue };
        let v = match (&slot.1, value) {
            (Tag::Byte(_), Tag::Byte(v)) => Tag::Byte(*v),
            (Tag::Byte(_), Tag::String(s)) => b(s == "true"),
            (Tag::Int(_), Tag::Int(v)) => Tag::Int(*v),
            (Tag::Int(_), Tag::String(s)) => match s.parse() {
                Ok(v) => Tag::Int(v),
                Err(_) => continue,
            },
            _ => continue,
        };
        slot.1 = v;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    set(&mut c, "LastPlayed", Tag::Long(now));
    let ld = BedrockLevelDat {
        storage_version: omt_versions::registry().bedrock_storage_version(),
        nbt: Nbt { name: String::new(), tag: Tag::Compound(c) },
    };
    ld.serialize().map_err(|e| ConvertError::Store(format!("level.dat: {e}")))
}

/// Write a Java `level.dat` (+ 26.3 `game_rules.dat`) from a Bedrock one.
pub fn write_java_level(root: &std::path::Path, bedrock: &Tag, data_version: u32, game_mode_override: Option<i32>) -> Result<()> {
    let int = |k: &str| bedrock.get(k).and_then(|t| t.as_i64());
    let name = bedrock.get("LevelName").and_then(|t| t.as_str()).unwrap_or("Converted World").to_string();
    let gm = game_mode_override.map(i64::from).or(int("GameType")).unwrap_or(0);
    let gm = if gm == 6 { 3 } else { gm.clamp(0, 3) } as i32;
    let diff = int("Difficulty").unwrap_or(2).clamp(0, 3) as usize;
    let hard = int("IsHardcore").unwrap_or(0) != 0;
    let cheats = int("commandsEnabled").unwrap_or(0) != 0;
    let (sx, sy, sz) = (int("SpawnX").unwrap_or(0) as i32, int("SpawnY").unwrap_or(64) as i32, int("SpawnZ").unwrap_or(0) as i32);
    let sy = if sy == 32767 { 64 } else { sy };
    let time = int("currentTick").unwrap_or(0);
    let day = int("Time").unwrap_or(0);
    let seed = int("RandomSeed").unwrap_or(0);
    let diff_name = ["peaceful", "easy", "normal", "hard"][diff];
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let data = Tag::Compound(vec![
        ("DataVersion".into(), Tag::Int(data_version as i32)),
        ("version".into(), Tag::Int(19133)),
        ("LevelName".into(), Tag::String(name)),
        ("GameType".into(), Tag::Int(gm)),
        ("allowCommands".into(), b(cheats)),
        ("initialized".into(), b(true)),
        ("LastPlayed".into(), Tag::Long(now_ms)),
        ("Time".into(), Tag::Long(time)),
        ("DayTime".into(), Tag::Long(day)),
        ("difficulty_settings".into(), Tag::Compound(vec![("difficulty".into(), Tag::String(diff_name.into())), ("hardcore".into(), b(hard)), ("locked".into(), b(false))])),
        ("Difficulty".into(), Tag::Byte(diff as i8)),
        ("hardcore".into(), b(hard)),
        ("spawn".into(), Tag::Compound(vec![
            ("pos".into(), Tag::IntArray(vec![sx, sy, sz])),
            ("dimension".into(), Tag::String("minecraft:overworld".into())),
            ("yaw".into(), Tag::Float(0.0)),
            ("pitch".into(), Tag::Float(0.0)),
        ])),
        ("SpawnX".into(), Tag::Int(sx)),
        ("SpawnY".into(), Tag::Int(sy)),
        ("SpawnZ".into(), Tag::Int(sz)),
        ("WorldGenSettings".into(), Tag::Compound(vec![("seed".into(), Tag::Long(seed))])),
        ("Version".into(), Tag::Compound(vec![
            ("Id".into(), Tag::Int(data_version as i32)),
            ("Name".into(), Tag::String(omt_versions::registry().java_release().to_string())),
            ("Series".into(), Tag::String("main".into())),
            ("Snapshot".into(), b(false)),
        ])),
    ]);
    let gz = |n: &Nbt| -> Result<Vec<u8>> {
        Ok(Container::Gzip.encode(&omt_nbt::write(n, ByteOrder::Big).map_err(|e| ConvertError::Store(e.to_string()))?))
    };
    let io = |p: &std::path::Path, e: std::io::Error| ConvertError::Store(format!("{}: {e}", p.display()));
    std::fs::create_dir_all(root).map_err(|e| io(root, e))?;
    let level = Nbt { name: String::new(), tag: Tag::Compound(vec![("Data".into(), data)]) };
    std::fs::write(root.join("level.dat"), gz(&level)?).map_err(|e| io(root, e))?;
    // 26.3 game rules + seed files.
    let mut rules = Vec::new();
    if let Some(c) = bedrock.as_compound() {
        for (k, v) in c {
            if let (Some(jk), Tag::Byte(_) | Tag::Int(_)) = (java_rule_aliases(k), v) {
                rules.push((jk.to_string(), v.clone()));
            }
        }
    }
    let dir = root.join("data/minecraft");
    std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
    let rules_nbt = Nbt { name: String::new(), tag: Tag::Compound(vec![("data".into(), Tag::Compound(rules)), ("DataVersion".into(), Tag::Int(data_version as i32))]) };
    std::fs::write(dir.join("game_rules.dat"), gz(&rules_nbt)?).map_err(|e| io(&dir, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_keys() {
        assert_eq!(bedrock_rule_key("minecraft:keep_inventory"), "keepinventory");
        assert_eq!(bedrock_rule_key("doDaylightCycle"), "dodaylightcycle");
    }

    #[test]
    fn java_to_bedrock_level_roundtrip() {
        let js = JavaSettings {
            name: Some("My World".into()),
            game_type: Some(3),
            difficulty: Some(3),
            hardcore: Some(true),
            cheats: Some(true),
            seed: Some(-42),
            spawn: Some((10, 70, -5)),
            time: Some(5000),
            day_time: Some(1000),
            rules: vec![("minecraft:keep_inventory".into(), Tag::Byte(1)), ("randomTickSpeed".into(), Tag::String("5".into()))],
        };
        let bytes = bedrock_level_dat(&js, None).unwrap();
        let ld = BedrockLevelDat::parse(&bytes).unwrap();
        let t = &ld.nbt.tag;
        assert_eq!(ld.level_name(), Some("My World"));
        assert_eq!(t.get("GameType").unwrap().as_i64(), Some(6), "spectator 3 → 6");
        assert_eq!(t.get("Difficulty").unwrap().as_i64(), Some(3));
        assert_eq!(t.get("RandomSeed").unwrap().as_i64(), Some(-42));
        assert_eq!(t.get("SpawnY").unwrap().as_i64(), Some(70));
        assert_eq!(t.get("Time").unwrap().as_i64(), Some(1000));
        assert_eq!(t.get("keepinventory").unwrap().as_i64(), Some(1));
        assert_eq!(t.get("randomtickspeed").unwrap().as_i64(), Some(5));
        assert_eq!(t.get("IsHardcore").unwrap().as_i64(), Some(1));

        // And back to Java.
        let dir = std::env::temp_dir().join(format!("omt-level-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_java_level(&dir, t, 5023, None).unwrap();
        let back = read_java_settings(&dir);
        assert_eq!(back.name.as_deref(), Some("My World"));
        assert_eq!(back.game_type, Some(3));
        assert_eq!(back.difficulty, Some(3));
        assert_eq!(back.seed, Some(-42));
        assert_eq!(back.spawn, Some((10, 70, -5)));
        assert!(back.rules.iter().any(|(k, v)| k == "minecraft:keep_inventory" && v.as_i64() == Some(1)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
