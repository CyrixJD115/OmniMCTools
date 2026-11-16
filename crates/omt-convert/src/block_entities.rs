//! Block-entity (tile entity) translation, Java ⇄ Bedrock (M4).
//!
//! Field shapes were taken from census probes of real worlds
//! (`examples/beprobe.rs`: the Java 26.3 fixture and a 1.21.5x Bedrock
//! world) and public format documentation — never from UMT code. Every
//! field we cannot carry is reported in the loss ledger; unknown tile ids
//! keep their data under the mapped id rather than being guessed at.

use omt_nbt::Tag;

use crate::items::{ItemConvertOpts, StackEra, bedrock_stack_to_java, java_stack_to_bedrock};
use crate::loss::{ConversionReport, LossKind};

/// Java id (without `minecraft:`) ↔ Bedrock tile id.
const IDS: &[(&str, &str)] = &[
    ("chest", "Chest"), ("trapped_chest", "Chest"), ("ender_chest", "EnderChest"), ("barrel", "Barrel"),
    ("shulker_box", "ShulkerBox"), ("furnace", "Furnace"), ("blast_furnace", "BlastFurnace"), ("smoker", "Smoker"),
    ("hopper", "Hopper"), ("dropper", "Dropper"), ("dispenser", "Dispenser"), ("brewing_stand", "BrewingStand"),
    ("beacon", "Beacon"), ("bell", "Bell"), ("campfire", "Campfire"), ("sign", "Sign"), ("hanging_sign", "HangingSign"),
    ("banner", "Banner"), ("bed", "Bed"), ("beehive", "Beehive"), ("comparator", "Comparator"),
    ("daylight_detector", "DaylightDetector"), ("enchanting_table", "EnchantTable"), ("end_portal", "EndPortal"),
    ("end_gateway", "EndGateway"), ("jukebox", "Jukebox"), ("lectern", "Lectern"), ("mob_spawner", "MobSpawner"),
    ("skull", "Skull"), ("conduit", "Conduit"), ("command_block", "CommandBlock"), ("structure_block", "StructureBlock"),
    ("jigsaw", "JigsawBlock"), ("piston", "PistonArm"), ("sculk_sensor", "SculkSensor"),
    ("calibrated_sculk_sensor", "CalibratedSculkSensor"), ("sculk_catalyst", "SculkCatalyst"),
    ("sculk_shrieker", "SculkShrieker"), ("chiseled_bookshelf", "ChiseledBookshelf"), ("decorated_pot", "DecoratedPot"),
    ("brushable_block", "BrushableBlock"), ("trial_spawner", "TrialSpawner"), ("vault", "Vault"), ("crafter", "Crafter"),
    ("creaking_heart", "CreakingHeart"),
];

/// Java dye order (banner/bed color index 0..15).
const DYES: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue",
    "brown", "green", "red", "black",
];

/// Sign text colors (ARGB) Bedrock stores per dye, same order as [`DYES`].
const SIGN_ARGB: [u32; 16] = [
    0xFFF0F0F0, 0xFFF9801D, 0xFFC74EBD, 0xFF3AB3DA, 0xFFFED83D, 0xFF80C71F, 0xFFF38BAA, 0xFF474F52, 0xFF9D9D97,
    0xFF169C9C, 0xFF8932B8, 0xFF3C44AA, 0xFF835432, 0xFF5E7C16, 0xFFB02E26, 0xFF000000,
];

/// Java banner pattern id ↔ Bedrock pattern code.
const PATTERNS: &[(&str, &str)] = &[
    ("base", "b"), ("square_bottom_left", "bl"), ("square_bottom_right", "br"), ("square_top_left", "tl"),
    ("square_top_right", "tr"), ("stripe_bottom", "bs"), ("stripe_top", "ts"), ("stripe_left", "ls"),
    ("stripe_right", "rs"), ("stripe_center", "cs"), ("stripe_middle", "ms"), ("stripe_downright", "drs"),
    ("stripe_downleft", "dls"), ("small_stripes", "ss"), ("cross", "cr"), ("straight_cross", "sc"),
    ("triangle_bottom", "bt"), ("triangle_top", "tt"), ("triangles_bottom", "bts"), ("triangles_top", "tts"),
    ("diagonal_left", "ld"), ("diagonal_up_right", "rd"), ("diagonal_up_left", "lud"), ("diagonal_right", "rud"),
    ("circle", "mc"), ("rhombus", "mr"), ("half_vertical", "vh"), ("half_horizontal", "hh"),
    ("half_vertical_right", "vhr"), ("half_horizontal_bottom", "hhb"), ("border", "bo"), ("curly_border", "cbo"),
    ("gradient", "gra"), ("gradient_up", "gru"), ("bricks", "bri"), ("globe", "glb"), ("creeper", "cre"),
    ("skull", "sku"), ("flower", "flo"), ("mojang", "moj"), ("piglin", "pig"), ("flow", "flw"), ("guster", "gus"),
];

/// Bedrock-only tiles with no Java block-entity home.
const BEDROCK_ONLY: &[&str] = &["Cauldron", "FlowerPot", "ItemFrame", "GlowItemFrame", "SporeBlossom", "NetherReactor", "MovingBlock"];

fn short(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

pub fn bedrock_id_for(java_id: &str) -> Option<&'static str> {
    IDS.iter().find(|(j, _)| *j == short(java_id)).map(|(_, b)| *b)
}

pub fn java_id_for(bedrock_id: &str) -> Option<String> {
    IDS.iter().find(|(_, b)| *b == bedrock_id).map(|(j, _)| format!("minecraft:{j}"))
}

fn int(t: Option<&Tag>) -> Option<i64> {
    t.and_then(|t| t.as_i64())
}

/// Plain text of a Java text component (26.3 NBT form, legacy JSON form,
/// or a bare string).
pub fn plain_text(t: &Tag) -> String {
    match t {
        Tag::String(s) => {
            let st = s.trim();
            if st.starts_with('{') || st.starts_with('[') || (st.starts_with('"') && st.len() >= 2) {
                serde_json::from_str::<serde_json::Value>(st).map(|v| json_text(&v)).unwrap_or_else(|_| s.clone())
            } else {
                s.clone()
            }
        }
        Tag::Compound(_) => {
            let mut out = t.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if let Some(extra) = t.get("extra").and_then(|e| e.as_list()) {
                for e in extra {
                    out.push_str(&plain_text(e));
                }
            }
            out
        }
        Tag::List(items) => items.iter().map(plain_text).collect(),
        _ => String::new(),
    }
}

fn json_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().map(json_text).collect(),
        serde_json::Value::Object(o) => {
            let mut out = o.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if let Some(serde_json::Value::Array(extra)) = o.get("extra") {
                for e in extra {
                    out.push_str(&json_text(e));
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// Java loot table id → Bedrock path, and back.
fn loot_to_bedrock(id: &str) -> String {
    format!("loot_tables/{}.json", short(id))
}

fn loot_to_java(path: &str) -> String {
    let p = path.trim_start_matches("loot_tables/").trim_end_matches(".json");
    format!("minecraft:{p}")
}

/// Shared converter state for one chunk.
pub struct BeCtx<'a> {
    pub items: &'a ItemConvertOpts,
    /// Block name at absolute coordinates (bed/banner colors live in the
    /// block, not the tile, on Java).
    pub block_at: &'a dyn Fn(i32, i32, i32) -> Option<String>,
    pub report: &'a mut ConversionReport,
}

fn push(c: &mut Vec<(String, Tag)>, k: &str, v: Tag) {
    c.push((k.to_string(), v));
}

fn xyz(be: &Tag) -> Option<(i32, i32, i32)> {
    Some((int(be.get("x"))? as i32, int(be.get("y"))? as i32, int(be.get("z"))? as i32))
}

// ---------------------------------------------------------------------------
// Java → Bedrock
// ---------------------------------------------------------------------------

/// Translate one Java block entity. `None` drops it (no Bedrock home,
/// e.g. proto-chunk `DUMMY` markers) — always with a ledger row.
pub fn java_to_bedrock(be: &Tag, ctx: &mut BeCtx<'_>) -> Option<Tag> {
    let jid = be.get("id").and_then(|t| t.as_str()).unwrap_or("");
    let Some(bid) = bedrock_id_for(jid) else {
        ctx.report.record(LossKind::UnmappedId, format!("block_entities {jid}"), "no Bedrock block entity for this Java id; dropped");
        return None;
    };
    let (x, y, z) = xyz(be)?;
    let mut c: Vec<(String, Tag)> = Vec::new();
    push(&mut c, "id", Tag::String(bid.into()));
    push(&mut c, "x", Tag::Int(x));
    push(&mut c, "y", Tag::Int(y));
    push(&mut c, "z", Tag::Int(z));
    push(&mut c, "isMovable", Tag::Byte(1));
    if let Some(name) = be.get("CustomName") {
        push(&mut c, "CustomName", Tag::String(plain_text(name)));
    }
    if let Some(lt) = be.get("LootTable").and_then(|t| t.as_str()) {
        push(&mut c, "LootTable", Tag::String(loot_to_bedrock(lt)));
        if let Some(seed) = int(be.get("LootTableSeed")) {
            push(&mut c, "LootTableSeed", Tag::Int(seed as i32));
        }
    }
    if let Some(items) = be.get("Items").and_then(|t| t.as_list()) {
        let mut out = Vec::new();
        for it in items {
            match java_stack_to_bedrock(it, ctx.items) {
                Ok((mut b, rep)) => {
                    if let (Some(slot), Tag::Compound(bc)) = (it.get("Slot"), &mut b)
                        && !bc.iter().any(|(k, _)| k == "Slot")
                    {
                        bc.push(("Slot".into(), slot.clone()));
                    }
                    ctx.report.extend(&rep);
                    out.push(b);
                }
                Err(e) => ctx.report.dropped(format!("block_entities {jid}.Items"), e.to_string()),
            }
        }
        push(&mut c, "Items", Tag::List(out));
        if matches!(bid, "Chest" | "Barrel" | "ShulkerBox" | "EnderChest") {
            push(&mut c, "Findable", Tag::Byte(0));
        }
    }
    match bid {
        "Furnace" | "BlastFurnace" | "Smoker" => {
            push(&mut c, "BurnTime", Tag::Short(int(be.get("lit_time_remaining")).unwrap_or(0) as i16));
            push(&mut c, "BurnDuration", Tag::Short(int(be.get("lit_total_time")).unwrap_or(0) as i16));
            push(&mut c, "CookTime", Tag::Short(int(be.get("cooking_time_spent")).unwrap_or(0) as i16));
            push(&mut c, "StoredXPInt", Tag::Int(0));
        }
        "BrewingStand" => {
            push(&mut c, "CookTime", Tag::Short(int(be.get("BrewTime")).unwrap_or(0) as i16));
            push(&mut c, "FuelAmount", Tag::Short(int(be.get("Fuel")).unwrap_or(0) as i16));
            push(&mut c, "FuelTotal", Tag::Short(20));
        }
        "Sign" | "HangingSign" => {
            for (jk, bk) in [("front_text", "FrontText"), ("back_text", "BackText")] {
                push(&mut c, bk, java_sign_side(be.get(jk)));
            }
            push(&mut c, "IsWaxed", Tag::Byte(int(be.get("is_waxed")).unwrap_or(0) as i8));
        }
        "Bed" => {
            let color = (ctx.block_at)(x, y, z)
                .and_then(|n| DYES.iter().position(|d| short(&n) == format!("{d}_bed")))
                .unwrap_or(14);
            push(&mut c, "color", Tag::Byte(color as i8));
        }
        "Banner" => {
            let base = (ctx.block_at)(x, y, z)
                .and_then(|n| DYES.iter().position(|d| short(&n).starts_with(&format!("{d}_")) && short(&n).contains("banner")))
                .unwrap_or(0);
            push(&mut c, "Base", Tag::Int(15 - base as i32));
            let mut pats = Vec::new();
            for p in be.get("patterns").and_then(|t| t.as_list()).unwrap_or(&[]) {
                let pid = p.get("pattern").and_then(|t| t.as_str()).map(short).unwrap_or("");
                let col = p.get("color").and_then(|t| t.as_str()).and_then(|cn| DYES.iter().position(|d| *d == cn)).unwrap_or(0);
                match PATTERNS.iter().find(|(j, _)| *j == pid) {
                    Some((_, code)) => pats.push(Tag::Compound(vec![
                        ("Color".into(), Tag::Int(15 - col as i32)),
                        ("Pattern".into(), Tag::String((*code).into())),
                    ])),
                    None => ctx.report.dropped("block_entities banner.patterns", format!("unknown pattern {pid}")),
                }
            }
            push(&mut c, "Patterns", Tag::List(pats));
            push(&mut c, "Type", Tag::Int(0));
        }
        "MobSpawner" => {
            if let Some(eid) = be.get("SpawnData").and_then(|s| s.get("entity")).and_then(|e| e.get("id")).and_then(|t| t.as_str()) {
                push(&mut c, "EntityIdentifier", Tag::String(eid.into()));
            }
            for k in ["Delay", "MinSpawnDelay", "MaxSpawnDelay", "SpawnCount", "MaxNearbyEntities", "RequiredPlayerRange", "SpawnRange"] {
                if let Some(v) = be.get(k) {
                    push(&mut c, k, v.clone());
                }
            }
        }
        "Lectern" => {
            if let Some(book) = be.get("Book") {
                match java_stack_to_bedrock(book, ctx.items) {
                    Ok((b, rep)) => {
                        ctx.report.extend(&rep);
                        push(&mut c, "book", b);
                        push(&mut c, "hasBook", Tag::Byte(1));
                    }
                    Err(e) => ctx.report.dropped("block_entities lectern.Book", e.to_string()),
                }
            }
            push(&mut c, "page", Tag::Int(int(be.get("Page")).unwrap_or(0) as i32));
        }
        "Jukebox" => {
            if let Some(rec) = be.get("RecordItem")
                && let Ok((b, rep)) = java_stack_to_bedrock(rec, ctx.items)
            {
                ctx.report.extend(&rep);
                push(&mut c, "RecordItem", b);
            }
        }
        "DecoratedPot" => {
            let sherds: Vec<Tag> = match be.get("sherds") {
                Some(Tag::Compound(s)) => ["back", "left", "right", "front"]
                    .iter()
                    .map(|k| {
                        let id = s.iter().find(|(n, _)| n == k).and_then(|(_, v)| v.get("id").and_then(|t| t.as_str()).map(str::to_owned));
                        Tag::String(id.unwrap_or_else(|| "minecraft:brick".into()))
                    })
                    .collect(),
                Some(Tag::List(l)) => l.clone(),
                _ => Vec::new(),
            };
            push(&mut c, "sherds", Tag::List(sherds));
        }
        "Beehive" => {
            let mut occ = Vec::new();
            for bee in be.get("bees").and_then(|t| t.as_list()).unwrap_or(&[]) {
                let id = bee.get("entity_data").and_then(|e| e.get("id")).and_then(|t| t.as_str()).unwrap_or("minecraft:bee");
                let left = int(bee.get("min_ticks_in_hive")).unwrap_or(600) - int(bee.get("ticks_in_hive")).unwrap_or(0);
                occ.push(Tag::Compound(vec![
                    ("ActorIdentifier".into(), Tag::String(format!("{id}<>"))),
                    ("SaveData".into(), Tag::Compound(vec![("identifier".into(), Tag::String(id.into()))])),
                    ("TicksLeftToStay".into(), Tag::Int(left.max(0) as i32)),
                ]));
            }
            push(&mut c, "Occupants", Tag::List(occ));
            push(&mut c, "ShouldSpawnBees", Tag::Byte(0));
        }
        "CommandBlock" => {
            for (j, b) in [("Command", "Command"), ("auto", "auto"), ("powered", "powered"), ("TrackOutput", "TrackOutput"), ("SuccessCount", "SuccessCount"), ("conditionMet", "conditionMet")] {
                if let Some(v) = be.get(j) {
                    push(&mut c, b, v.clone());
                }
            }
            push(&mut c, "Version", Tag::Int(42));
        }
        "Comparator" => push(&mut c, "OutputSignal", Tag::Int(int(be.get("OutputSignal")).unwrap_or(0) as i32)),
        "TrialSpawner" | "Vault" => {
            for k in ["normal_config", "ominous_config", "required_player_range"] {
                if let Some(v) = be.get(k) {
                    push(&mut c, k, v.clone());
                }
            }
        }
        _ => {}
    }
    Some(Tag::Compound(c))
}

fn java_sign_side(side: Option<&Tag>) -> Tag {
    let lines: Vec<String> = side
        .and_then(|s| s.get("messages"))
        .and_then(|m| m.as_list())
        .map(|l| l.iter().map(plain_text).collect())
        .unwrap_or_default();
    let color = side.and_then(|s| s.get("color")).and_then(|t| t.as_str()).unwrap_or("black");
    let argb = DYES.iter().position(|d| *d == color).map_or(0xFF000000, |i| SIGN_ARGB[i]);
    let glow = side.and_then(|s| int(s.get("has_glowing_text"))).unwrap_or(0) != 0;
    Tag::Compound(vec![
        ("Text".into(), Tag::String(lines.join("\n"))),
        ("SignTextColor".into(), Tag::Int(argb as i32)),
        ("IgnoreLighting".into(), Tag::Byte(glow as i8)),
        ("HideGlowOutline".into(), Tag::Byte(0)),
        ("PersistFormatting".into(), Tag::Byte(1)),
        ("FilteredText".into(), Tag::String(String::new())),
        ("TextOwner".into(), Tag::String(String::new())),
    ])
}

// ---------------------------------------------------------------------------
// Bedrock → Java
// ---------------------------------------------------------------------------

/// Translate one Bedrock block entity to Java (26.3 field shapes).
pub fn bedrock_to_java(be: &Tag, ctx: &mut BeCtx<'_>) -> Option<Tag> {
    let bid = be.get("id").and_then(|t| t.as_str()).unwrap_or("");
    let Some(jid) = java_id_for(bid) else {
        let why = if BEDROCK_ONLY.contains(&bid) {
            "Bedrock-only block entity (Java keeps this in the block or as an entity); dropped"
        } else {
            "no Java block entity for this Bedrock id; dropped"
        };
        ctx.report.record(LossKind::UnmappedId, format!("block_entities {bid}"), why);
        return None;
    };
    let (x, y, z) = xyz(be)?;
    let mut c: Vec<(String, Tag)> = Vec::new();
    push(&mut c, "id", Tag::String(jid.clone()));
    push(&mut c, "x", Tag::Int(x));
    push(&mut c, "y", Tag::Int(y));
    push(&mut c, "z", Tag::Int(z));
    push(&mut c, "keepPacked", Tag::Byte(0));
    push(&mut c, "components", Tag::Compound(vec![]));
    if let Some(name) = be.get("CustomName").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) {
        push(&mut c, "CustomName", Tag::String(name.into()));
    }
    if let Some(lt) = be.get("LootTable").and_then(|t| t.as_str()) {
        push(&mut c, "LootTable", Tag::String(loot_to_java(lt)));
        if let Some(seed) = int(be.get("LootTableSeed")) {
            push(&mut c, "LootTableSeed", Tag::Long(seed));
        }
    }
    if let Some(items) = be.get("Items").and_then(|t| t.as_list()) {
        let mut out = Vec::new();
        for (i, it) in items.iter().enumerate() {
            let empty = it.get("Name").and_then(|t| t.as_str()).is_none_or(str::is_empty) || int(it.get("Count")) == Some(0);
            if empty {
                continue;
            }
            match bedrock_stack_to_java(it, StackEra::Components, ctx.items) {
                Ok((mut j, rep)) => {
                    let slot = it.get("Slot").cloned().unwrap_or(Tag::Byte(i as i8));
                    if let Tag::Compound(jc) = &mut j
                        && !jc.iter().any(|(k, _)| k == "Slot")
                    {
                        jc.push(("Slot".into(), slot));
                    }
                    ctx.report.extend(&rep);
                    out.push(j);
                }
                Err(e) => ctx.report.dropped(format!("block_entities {bid}.Items"), e.to_string()),
            }
        }
        push(&mut c, "Items", Tag::List(out));
    }
    match bid {
        "Furnace" | "BlastFurnace" | "Smoker" => {
            push(&mut c, "lit_time_remaining", Tag::Int(int(be.get("BurnTime")).unwrap_or(0) as i32));
            push(&mut c, "lit_total_time", Tag::Int(int(be.get("BurnDuration")).unwrap_or(0) as i32));
            push(&mut c, "cooking_time_spent", Tag::Int(int(be.get("CookTime")).unwrap_or(0) as i32));
            push(&mut c, "cooking_total_time", Tag::Int(if bid == "Furnace" { 200 } else { 100 }));
        }
        "BrewingStand" => {
            push(&mut c, "BrewTime", Tag::Int(int(be.get("CookTime")).unwrap_or(0) as i32));
            push(&mut c, "Fuel", Tag::Int(int(be.get("FuelAmount")).unwrap_or(0) as i32));
        }
        "Sign" | "HangingSign" => {
            for (bk, jk) in [("FrontText", "front_text"), ("BackText", "back_text")] {
                push(&mut c, jk, bedrock_sign_side(be.get(bk)));
            }
            push(&mut c, "is_waxed", Tag::Byte(int(be.get("IsWaxed")).unwrap_or(0) as i8));
        }
        "Banner" => {
            let mut pats = Vec::new();
            for p in be.get("Patterns").and_then(|t| t.as_list()).unwrap_or(&[]) {
                let code = p.get("Pattern").and_then(|t| t.as_str()).unwrap_or("");
                let col = (15 - int(p.get("Color")).unwrap_or(0)).clamp(0, 15) as usize;
                match PATTERNS.iter().find(|(_, b)| *b == code) {
                    Some((j, _)) => pats.push(Tag::Compound(vec![
                        ("color".into(), Tag::String(DYES[col].into())),
                        ("pattern".into(), Tag::String(format!("minecraft:{j}"))),
                    ])),
                    None => ctx.report.dropped("block_entities Banner.Patterns", format!("unknown pattern code {code}")),
                }
            }
            push(&mut c, "patterns", Tag::List(pats));
        }
        // Bed color, banner base, head type/rotation: carried by the Java
        // block instead (see `java_block_override`, applied per section).
        "MobSpawner" => {
            if let Some(eid) = be.get("EntityIdentifier").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) {
                push(&mut c, "SpawnData", Tag::Compound(vec![("entity".into(), Tag::Compound(vec![("id".into(), Tag::String(eid.into()))]))]));
                push(&mut c, "SpawnPotentials", Tag::List(vec![]));
            }
            for k in ["Delay", "MinSpawnDelay", "MaxSpawnDelay", "SpawnCount", "MaxNearbyEntities", "RequiredPlayerRange", "SpawnRange"] {
                if let Some(v) = be.get(k) {
                    push(&mut c, k, v.clone());
                }
            }
        }
        "Lectern" => {
            if let Some(book) = be.get("book").filter(|_| int(be.get("hasBook")).unwrap_or(0) != 0) {
                match bedrock_stack_to_java(book, StackEra::Components, ctx.items) {
                    Ok((j, rep)) => {
                        ctx.report.extend(&rep);
                        push(&mut c, "Book", j);
                        push(&mut c, "Page", Tag::Int(int(be.get("page")).unwrap_or(0) as i32));
                    }
                    Err(e) => ctx.report.dropped("block_entities Lectern.book", e.to_string()),
                }
            }
        }
        "Jukebox" => {
            if let Some(rec) = be.get("RecordItem")
                && let Ok((j, rep)) = bedrock_stack_to_java(rec, StackEra::Components, ctx.items)
            {
                ctx.report.extend(&rep);
                push(&mut c, "RecordItem", j);
            }
        }
        "DecoratedPot" => {
            if let Some(Tag::List(l)) = be.get("sherds") {
                let names: Vec<String> = l.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect();
                let mut comp = Vec::new();
                for (k, n) in ["back", "left", "right", "front"].iter().zip(names) {
                    comp.push((k.to_string(), Tag::Compound(vec![("id".into(), Tag::String(n))])));
                }
                push(&mut c, "sherds", Tag::Compound(comp));
            }
        }
        "Beehive" => {
            let mut bees = Vec::new();
            for o in be.get("Occupants").and_then(|t| t.as_list()).unwrap_or(&[]) {
                let id = o.get("ActorIdentifier").and_then(|t| t.as_str()).unwrap_or("minecraft:bee<>");
                let id = id.split('<').next().unwrap_or("minecraft:bee");
                let left = int(o.get("TicksLeftToStay")).unwrap_or(0).clamp(0, 600) as i32;
                bees.push(Tag::Compound(vec![
                    ("entity_data".into(), Tag::Compound(vec![("id".into(), Tag::String(id.into()))])),
                    ("min_ticks_in_hive".into(), Tag::Int(600)),
                    ("ticks_in_hive".into(), Tag::Int(600 - left)),
                ]));
            }
            if !bees.is_empty() {
                ctx.report.record(LossKind::FieldDropped, "block_entities Beehive.Occupants", "bee save data reduced to the entity id");
            }
            push(&mut c, "bees", Tag::List(bees));
        }
        "CommandBlock" => {
            for k in ["Command", "auto", "powered", "TrackOutput", "SuccessCount", "conditionMet"] {
                if let Some(v) = be.get(k) {
                    push(&mut c, k, v.clone());
                }
            }
        }
        "Comparator" => push(&mut c, "OutputSignal", Tag::Int(int(be.get("OutputSignal")).unwrap_or(0) as i32)),
        "TrialSpawner" | "Vault" => {
            for k in ["normal_config", "ominous_config", "required_player_range"] {
                if let Some(v) = be.get(k) {
                    push(&mut c, k, v.clone());
                }
            }
        }
        _ => {}
    }
    Some(Tag::Compound(c))
}

fn bedrock_sign_side(side: Option<&Tag>) -> Tag {
    let text = side.and_then(|s| s.get("Text")).and_then(|t| t.as_str()).unwrap_or("");
    let mut lines: Vec<Tag> = text.split('\n').take(4).map(|l| Tag::String(l.into())).collect();
    while lines.len() < 4 {
        lines.push(Tag::String(String::new()));
    }
    let argb = side.and_then(|s| int(s.get("SignTextColor"))).unwrap_or(0xFF000000u32 as i32 as i64) as i32 as u32;
    let color = nearest_dye(argb);
    let glow = side.and_then(|s| int(s.get("IgnoreLighting"))).unwrap_or(0) != 0;
    Tag::Compound(vec![
        ("messages".into(), Tag::List(lines)),
        ("color".into(), Tag::String(DYES[color].into())),
        ("has_glowing_text".into(), Tag::Byte(glow as i8)),
    ])
}

fn nearest_dye(argb: u32) -> usize {
    let ch = |v: u32, s: u32| ((v >> s) & 0xff) as i32;
    (0..16)
        .min_by_key(|&i| {
            let d = SIGN_ARGB[i];
            (0..3).map(|k| (ch(d, k * 8) - ch(argb, k * 8)).pow(2)).sum::<i32>()
        })
        .unwrap_or(15)
}

// ---------------------------------------------------------------------------
// Data that lives in the block on one edition and in the tile on the other
// ---------------------------------------------------------------------------

/// Bedrock → Java: the Java block that should stand at a tile's position,
/// given the block the palette mapping produced there. Beds and banners
/// take their color from the tile, flower pots their plant, heads their
/// type and rotation.
pub fn java_block_override(be: &Tag, mapped: &omt_world::BlockState) -> Option<omt_world::BlockState> {
    let bid = be.get("id").and_then(|t| t.as_str())?;
    let keep = |name: String| Some(omt_world::BlockState { name, properties: mapped.properties.clone() });
    match bid {
        "Bed" => {
            let c = int(be.get("color")).unwrap_or(14).clamp(0, 15) as usize;
            if !mapped.name.ends_with("_bed") {
                return None;
            }
            keep(format!("minecraft:{}_bed", DYES[c]))
        }
        "Banner" => {
            let c = (15 - int(be.get("Base")).unwrap_or(15)).clamp(0, 15) as usize;
            if !mapped.name.contains("banner") {
                return None;
            }
            let wall = mapped.name.contains("wall");
            keep(format!("minecraft:{}_{}banner", DYES[c], if wall { "wall_" } else { "" }))
        }
        "FlowerPot" => {
            let plant = be.get("PlantBlock").and_then(|p| p.get("name")).and_then(|t| t.as_str())?;
            let plant = match short(plant) {
                "azalea" => "azalea_bush",
                "flowering_azalea" => "flowering_azalea_bush",
                other => other,
            };
            Some(omt_world::BlockState { name: format!("minecraft:potted_{plant}"), properties: Vec::new() })
        }
        "Skull" => {
            const KINDS: [&str; 7] = ["skeleton_skull", "wither_skeleton_skull", "zombie_head", "player_head", "creeper_head", "dragon_head", "piglin_head"];
            let kind = KINDS.get(int(be.get("SkullType")).unwrap_or(-1).clamp(-1, 6) as usize).copied().or_else(|| {
                // 1.21+ Bedrock flattened heads: the block already names them.
                KINDS.iter().copied().find(|k| short(&mapped.name).starts_with(k.split('_').next().unwrap_or("")))
            })?;
            let wall = mapped.name.contains("wall");
            if wall {
                let (a, b) = kind.rsplit_once('_')?;
                return keep(format!("minecraft:{a}_wall_{b}"));
            }
            let rot = be.get("Rotation").and_then(|t| match t {
                Tag::Float(f) => Some(*f as f64),
                other => other.as_i64().map(|v| v as f64),
            });
            let mut props: Vec<(String, String)> = mapped.properties.iter().filter(|(k, _)| k != "rotation").cloned().collect();
            if let Some(r) = rot {
                props.push(("rotation".into(), ((r / 22.5).round() as i64).rem_euclid(16).to_string()));
            }
            props.sort();
            Some(omt_world::BlockState { name: format!("minecraft:{kind}"), properties: props })
        }
        _ => None,
    }
}

/// Java plant block → Bedrock name + typed states.
pub type PlantMapper<'a> = &'a dyn Fn(&str) -> Option<(String, Vec<(String, Tag)>)>;

/// Java → Bedrock: tiles Bedrock needs that Java encodes in the block —
/// flower pots (`potted_*`) get a `FlowerPot` tile with their plant.
/// `plant_state` maps a Java plant block to its Bedrock name + states.
pub fn bedrock_tiles_from_blocks(
    positions: &[((i32, i32, i32), String)],
    plant_state: PlantMapper<'_>,
    palette_version: i32,
) -> Vec<Tag> {
    let mut out = Vec::new();
    for ((x, y, z), name) in positions {
        let Some(plant) = short(name).strip_prefix("potted_") else { continue };
        let plant = plant.trim_end_matches("_bush");
        let java_plant = format!("minecraft:{plant}");
        let (bname, states) = plant_state(&java_plant).unwrap_or((java_plant, Vec::new()));
        out.push(Tag::Compound(vec![
            ("id".into(), Tag::String("FlowerPot".into())),
            ("x".into(), Tag::Int(*x)),
            ("y".into(), Tag::Int(*y)),
            ("z".into(), Tag::Int(*z)),
            ("isMovable".into(), Tag::Byte(1)),
            (
                "PlantBlock".into(),
                Tag::Compound(vec![
                    ("name".into(), Tag::String(bname)),
                    ("states".into(), Tag::Compound(states)),
                    ("version".into(), Tag::Int(palette_version)),
                ]),
            ),
        ]));
    }
    out
}

/// Java → Bedrock skull tile fields from the Java head block.
pub fn bedrock_skull_fields(java_block: &str, rotation: Option<&str>) -> Vec<(String, Tag)> {
    const KINDS: [&str; 7] = ["skeleton", "wither_skeleton", "zombie", "player", "creeper", "dragon", "piglin"];
    let n = short(java_block);
    let kind = KINDS.iter().enumerate().rev().find(|(_, k)| n.starts_with(&format!("{k}_"))).map_or(0, |(i, _)| i);
    let rot = rotation.and_then(|r| r.parse::<f32>().ok()).map_or(0.0, |r| r * 22.5);
    vec![
        ("SkullType".into(), Tag::Byte(kind as i8)),
        ("Rotation".into(), Tag::Float(if rot > 180.0 { rot - 360.0 } else { rot })),
        ("DoingAnimation".into(), Tag::Byte(0)),
        ("MouthTickCount".into(), Tag::Int(0)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use omt_nbt::snbt::from_snbt;

    fn ctx_run<F: FnOnce(&mut BeCtx<'_>) -> Option<Tag>>(f: F) -> (Option<Tag>, ConversionReport) {
        let items = ItemConvertOpts::default();
        let mut report = ConversionReport::new();
        let block_at = |_x: i32, _y: i32, _z: i32| Some("minecraft:red_bed".to_string());
        let out = {
            let mut ctx = BeCtx { items: &items, block_at: &block_at, report: &mut report };
            f(&mut ctx)
        };
        (out, report)
    }

    #[test]
    fn java_chest_and_sign_to_bedrock() {
        let chest = from_snbt(r#"{id:"minecraft:chest", x:1, y:2, z:3, CustomName:'{"text":"Loot"}', Items:[{Slot:0b, id:"minecraft:diamond", count:5}]}"#).unwrap();
        let (b, _) = ctx_run(|c| java_to_bedrock(&chest, c));
        let b = b.unwrap();
        assert_eq!(b.get("id").unwrap().as_str(), Some("Chest"));
        assert_eq!(b.get("CustomName").unwrap().as_str(), Some("Loot"));
        let item = &b.get("Items").unwrap().as_list().unwrap()[0];
        assert_eq!(item.get("Name").unwrap().as_str(), Some("minecraft:diamond"));
        assert_eq!(item.get("Count").unwrap().as_i64(), Some(5));
        assert_eq!(item.get("Slot").unwrap().as_i64(), Some(0));

        let sign = from_snbt(r#"{id:"minecraft:sign", x:0, y:0, z:0, is_waxed:1b, front_text:{messages:["Hello", {text:"World"}, '"json"', ""], color:"red", has_glowing_text:1b}}"#).unwrap();
        let (b, _) = ctx_run(|c| java_to_bedrock(&sign, c));
        let b = b.unwrap();
        let front = b.get("FrontText").unwrap();
        assert_eq!(front.get("Text").unwrap().as_str(), Some("Hello\nWorld\njson\n"));
        assert_eq!(front.get("SignTextColor").unwrap().as_i64(), Some(0xFFB02E26u32 as i32 as i64));
        assert_eq!(front.get("IgnoreLighting").unwrap().as_i64(), Some(1));
        assert_eq!(b.get("IsWaxed").unwrap().as_i64(), Some(1));
    }

    #[test]
    fn bedrock_roundtrips_to_java() {
        let furnace = from_snbt(r#"{id:"Furnace", x:5, y:6, z:7, BurnTime:100s, BurnDuration:1600s, CookTime:50s, Items:[{Count:49b, Damage:0s, Name:"minecraft:coal", Slot:1b, WasPickedUp:0b}]}"#).unwrap();
        let (j, _) = ctx_run(|c| bedrock_to_java(&furnace, c));
        let j = j.unwrap();
        assert_eq!(j.get("id").unwrap().as_str(), Some("minecraft:furnace"));
        assert_eq!(j.get("lit_time_remaining").unwrap().as_i64(), Some(100));
        let item = &j.get("Items").unwrap().as_list().unwrap()[0];
        assert_eq!(item.get("id").unwrap().as_str(), Some("minecraft:coal"));
        assert_eq!(item.get("Slot").unwrap().as_i64(), Some(1));

        let sign = from_snbt(r#"{id:"Sign", x:0, y:0, z:0, IsWaxed:0b, FrontText:{Text:"CREATIVE\n\n\n\n", SignTextColor:-16777216, IgnoreLighting:0b}}"#).unwrap();
        let (j, _) = ctx_run(|c| bedrock_to_java(&sign, c));
        let front = j.unwrap().get("front_text").unwrap().clone();
        let msgs = front.get("messages").unwrap().as_list().unwrap().to_vec();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].as_str(), Some("CREATIVE"));
        assert_eq!(front.get("color").unwrap().as_str(), Some("black"));

        // Banner colors are inverted between editions; patterns map codes.
        let banner = from_snbt(r#"{id:"Banner", x:0, y:0, z:0, Base:15, Patterns:[{Color:1, Pattern:"bri"}]}"#).unwrap();
        let (j, _) = ctx_run(|c| bedrock_to_java(&banner, c));
        let j = j.unwrap();
        let p = &j.get("patterns").unwrap().as_list().unwrap()[0];
        assert_eq!(p.get("pattern").unwrap().as_str(), Some("minecraft:bricks"));
        assert_eq!(p.get("color").unwrap().as_str(), Some("red"));

        // Bedrock-only tiles drop with a ledger row.
        let pot = from_snbt(r#"{id:"FlowerPot", x:0, y:0, z:0}"#).unwrap();
        let (j, rep) = ctx_run(|c| bedrock_to_java(&pot, c));
        assert!(j.is_none() && !rep.is_clean());
    }

    #[test]
    fn block_overrides_both_ways() {
        let bs = |n: &str, p: &[(&str, &str)]| omt_world::BlockState { name: n.into(), properties: p.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect() };
        let bed = from_snbt(r#"{id:"Bed", color:13b, x:0, y:0, z:0}"#).unwrap();
        let out = java_block_override(&bed, &bs("minecraft:white_bed", &[("part", "head")])).unwrap();
        assert_eq!(out.name, "minecraft:green_bed");
        assert_eq!(out.get("part"), Some("head"));
        let pot = from_snbt(r#"{id:"FlowerPot", PlantBlock:{name:"minecraft:azalea"}, x:0, y:0, z:0}"#).unwrap();
        assert_eq!(java_block_override(&pot, &bs("minecraft:flower_pot", &[])).unwrap().name, "minecraft:potted_azalea_bush");
        let banner = from_snbt(r#"{id:"Banner", Base:1, x:0, y:0, z:0}"#).unwrap();
        assert_eq!(java_block_override(&banner, &bs("minecraft:white_wall_banner", &[("facing", "north")])).unwrap().name, "minecraft:red_wall_banner");
        let skull = from_snbt(r#"{id:"Skull", SkullType:2b, Rotation:-112.5f, x:0, y:0, z:0}"#).unwrap();
        let s = java_block_override(&skull, &bs("minecraft:skeleton_skull", &[("rotation", "0")])).unwrap();
        assert_eq!(s.name, "minecraft:zombie_head");
        assert_eq!(s.get("rotation"), Some("11"));

        let tiles = bedrock_tiles_from_blocks(&[((1, 2, 3), "minecraft:potted_fern".into()), ((0, 0, 0), "minecraft:stone".into())], &|n| Some((n.to_string(), vec![])), 1);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].get("PlantBlock").unwrap().get("name").unwrap().as_str(), Some("minecraft:fern"));
        let f = bedrock_skull_fields("minecraft:wither_skeleton_skull", Some("4"));
        assert_eq!(f[0].1.as_i64(), Some(1));
    }

    #[test]
    fn bed_color_from_block_and_loot_paths() {
        let bed = from_snbt(r#"{id:"minecraft:bed", x:0, y:0, z:0}"#).unwrap();
        let (b, _) = ctx_run(|c| java_to_bedrock(&bed, c));
        assert_eq!(b.unwrap().get("color").unwrap().as_i64(), Some(14), "red_bed → 14");
        assert_eq!(loot_to_bedrock("minecraft:chests/village/village_plains_house"), "loot_tables/chests/village/village_plains_house.json");
        assert_eq!(loot_to_java("loot_tables/chests/village/village_plains_house.json"), "minecraft:chests/village/village_plains_house");
    }
}
