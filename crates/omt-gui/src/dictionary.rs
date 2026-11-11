//! Static field dictionary: friendly labels, one-line descriptions, and value
//! decoding for well-known NBT keys (level.dat, gamerules, entities, item
//! stacks, block entities).
//!
//! Inspired by UMT's approach (UMT ships per-key info sections in its database
//! plus in-code enum decoding — see `Source/RE/UMT_Feature_Audit.md`), but the
//! table below is authored from public Minecraft knowledge and our fixture
//! observations. Lookup is by leaf key name; entries are additive — unknown
//! keys render exactly as before, nothing is guessed.

/// One dictionary row: display label + short explanation.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub label: &'static str,
    pub desc: &'static str,
}

const fn e(label: &'static str, desc: &'static str) -> Entry {
    Entry { label, desc }
}

/// Look up a leaf key name. `None` = not in the dictionary (render raw).
pub fn lookup(key: &str) -> Option<Entry> {
    match key {
        // ---- Java level.dat (legacy `Data.*` and 26.3 flattened) ----
        "LevelName" => Some(e("World Name", "The name shown in the world list and save menus.")),
        "GameType" => Some(e("Game Mode", "0 Survival, 1 Creative, 2 Adventure, 3 Spectator.")),
        "Difficulty" => Some(e("Difficulty", "0 Peaceful, 1 Easy, 2 Normal, 3 Hard.")),
        "difficulty" => Some(e("Difficulty", "peaceful / easy / normal / hard (26.x stores this as text).")),
        "difficulty_settings" => Some(e("Difficulty Settings", "Difficulty, hardcore and locked-difficulty flags (26.x).")),
        "locked" => Some(e("Difficulty Locked", "When set, the difficulty cannot be changed in-game.")),
        "hardcore" => Some(e("Hardcore", "Hardcore mode: one life, game over on death.")),
        "IsHardcore" => Some(e("Hardcore", "Hardcore mode: one life, game over on death.")),
        "allowCommands" => Some(e("Cheats (Commands)", "Whether cheats such as /gamemode are enabled.")),
        "commandsEnabled" => Some(e("Cheats (Commands)", "Whether cheats such as /gamemode are enabled.")),
        "cheatsEnabled" => Some(e("Cheats (Commands)", "Whether cheats are enabled (Bedrock mirror of commandsEnabled).")),
        "ForceGameType" => Some(e("Force Game Mode", "Forces the world's game mode on joining players.")),
        "RandomSeed" | "randomSeed" => Some(e("World Seed", "The seed world generation started from.")),
        "SpawnX" => Some(e("Spawn X", "World spawn point, X coordinate.")),
        "SpawnY" => Some(e("Spawn Y", "World spawn point, Y coordinate.")),
        "SpawnZ" => Some(e("Spawn Z", "World spawn point, Z coordinate.")),
        "spawn" => Some(e("Spawn Point", "Spawn position, angle and dimension (26.x shape).")),
        "pos" => Some(e("Position", "XYZ coordinates, blocks.")),
        "pitch" => Some(e("Pitch", "Vertical look angle in degrees.")),
        "yaw" => Some(e("Yaw", "Horizontal look angle in degrees.")),
        "dimension" => Some(e("Dimension", "Namespaced dimension id, e.g. minecraft:overworld.")),
        "LastPlayed" => Some(e("Last Played", "When the world was last opened (epoch time).")),
        "Time" => Some(e("World Age", "Total elapsed ticks since world creation (24000 ticks = 1 in-game day).")),
        "currentTick" => Some(e("World Age", "Total elapsed ticks since world creation (24000 ticks = 1 in-game day).")),
        "DayTime" => Some(e("Time of Day", "Day-cycle ticks; 0 = dawn, 6000 = noon, 12000 = dusk, 18000 = midnight.")),
        "version" => Some(e("Save Format Version", "19133 = Anvil (all modern saves).")),
        "DataVersion" => Some(e("Data Version", "Content version the world was written by (drives upgrades).")),
        "Player" => Some(e("Saved Player", "Single-player player data snapshot (pre-26.x).")),
        "GameRules" => Some(e("Game Rules", "Toggles and counters that shape world behaviour.")),
        "WorldGenSettings" => Some(e("World Generation", "Seed, structure and dimension generator settings.")),
        "generate_features" => Some(e("Generate Structures", "Whether villages, temples and similar structures generate.")),
        "bonus_chest" => Some(e("Bonus Chest", "Spawns a starter chest near spawn.")),
        "bonusChestEnabled" => Some(e("Bonus Chest", "Spawns a starter chest near spawn.")),
        "bonusChestSpawned" => Some(e("Bonus Chest Spawned", "Whether the bonus chest was already placed.")),
        "seed" => Some(e("World Seed", "The seed world generation started from.")),
        "ServerBrands" => Some(e("Server Brands", "Software that has handled this world (vanilla, Paper, …).")),
        "DataPacks" => Some(e("Data Packs", "Enabled and disabled data packs.")),
        "Initialized" | "initialized" => Some(e("World Initialized", "Whether the world has finished first-time setup.")),
        "WasModded" => Some(e("Was Modded", "A mod touched this world at some point (26.x flag).")),
        "singleplayer_uuid" => Some(e("Owner UUID", "UUID of the single-player owner, as four ints (26.x).")),
        "version_history" => Some(e("Version History", "Data versions this world passed through (26.x).")),
        "Version" => Some(e("Save Version", "The exact game version that wrote this file.")),
        "Name" => Some(e("Name", "Namespaced identifier — an item id in Bedrock stacks, or the game version string that wrote the save.")),
        "Id" => Some(e("Version Id", "Numeric version id (the DataVersion).")),
        "Snapshot" => Some(e("Snapshot Build", "Whether this version was a development snapshot.")),
        "Series" => Some(e("Version Series", "Release line, e.g. main.")),
        "WanderingTraderId" | "WanderingTraderRange" => None,
        "SpawnV1Villagers" => Some(e("Legacy Villagers", "Spawn old-style villagers for compatibility.")),
        "LevelChunkMetaDataDictionary" => Some(e("Chunk Metadata Dictionary", "Internal: per-chunk metadata records (digests, versions).")),
        "FlatWorldLayers" => Some(e("Flat World Preset", "JSON description of the superflat layers.")),
        "Generator" => Some(e("World Generator", "0 old, 1 infinite, 2 flat (Bedrock).")),
        "NetherScale" => Some(e("Nether Scale", "Overworld-to-Nether coordinate ratio (default 8).")),
        "NetworkVersion" => Some(e("Protocol Version", "Bedrock protocol number the world was written with.")),
        "InventoryVersion" => Some(e("Client Version", "Bedrock client version string that last opened the world.")),
        "lastOpenedWithVersion" => Some(e("Last Opened With", "Client version array that last opened the world.")),
        "MinimumCompatibleClientVersion" => Some(e("Minimum Client", "Oldest client version that can still open this world.")),
        "StorageVersion" => Some(e("Storage Version", "LevelDB storage schema version (10 = current).")),
        "WorldVersion" => Some(e("World Version", "Internal Bedrock world-format version counter.")),
        "BiomeOverride" => Some(e("Biome Override", "Internal: forced biome map (usually empty).")),
        "experiments" => Some(e("Experiments", "Experimental gameplay feature flags.")),
        "educationFeaturesEnabled" => Some(e("Education Features", "Bedrock Education content enabled.")),
        "editorWorldType" => Some(e("Editor World Type", "Minecraft Editor world kind (0 = none).")),
        "abilities" => Some(e("Player Abilities", "Movement, building and flight permissions for players.")),
        "attackmobs" => Some(e("Attack Mobs", "Ability: can hit mobs.")),
        "attackplayers" => Some(e("Attack Players", "Ability: can hit players.")),
        "build" => Some(e("Build", "Ability: can place blocks.")),
        "mine" => Some(e("Mine", "Ability: can break blocks.")),
        "doorsandswitches" => Some(e("Use Doors & Switches", "Ability: interact with doors, buttons, levers.")),
        "opencontainers" => Some(e("Open Containers", "Ability: open chests and other containers.")),
        "teleport" => Some(e("Teleport", "Ability: use /tp.")),
        "flying" => Some(e("Flying", "Currently flying (creative flight active).")),
        "mayfly" => Some(e("May Fly", "Allowed to fly.")),
        "instabuild" => Some(e("Instant Build", "Blocks break instantly.")),
        "invulnerable" => Some(e("Invulnerable", "Cannot take damage.")),
        "lightning" => Some(e("Lightning", "Ability: summon lightning.")),
        "op" => Some(e("Operator", "Has operator permissions.")),
        "flySpeed" => Some(e("Fly Speed", "Flight movement speed multiplier.")),
        "walkSpeed" => Some(e("Walk Speed", "Walking speed multiplier.")),
        "verticalFlySpeed" => Some(e("Vertical Fly Speed", "Up/down flight speed multiplier.")),
        "functioncommandlimit" => Some(e("Function Limit", "Max commands one function may run.")),
        "daylightCycle" => Some(e("Daylight Cycle", "0 normal, 1 always day, 2 always night (Bedrock).")),
        "doDaylightCycle" => Some(e("Daylight Cycle", "Advance the day/night clock.")),
        "dodaylightcycle" => Some(e("Daylight Cycle", "Advance the day/night clock.")),
        "doWeatherCycle" | "doweathercycle" => Some(e("Weather Cycle", "Weather changes over time.")),
        "keepInventory" | "keepinventory" => Some(e("Keep Inventory", "Items stay with you on death.")),
        "mobGriefing" | "mobgriefing" => Some(e("Mob Griefing", "Mobs can alter terrain (creepers, endermen…).")),
        "doFireTick" | "dofiretick" => Some(e("Fire Spread", "Fire spreads and burns out.")),
        "doMobSpawning" | "domobspawning" => Some(e("Mob Spawning", "Hostile mobs spawn naturally.")),
        "doMobLoot" | "domobloot" => Some(e("Mob Loot", "Mobs drop items on death.")),
        "doTileDrops" | "dotiledrops" => Some(e("Block Drops", "Broken blocks drop items.")),
        "doEntityDrops" | "doentitydrops" => Some(e("Item Drops", "Minecarts and similar drop their item.")),
        "doInsomnia" | "doinsomnia" => Some(e("Phantoms", "Phantoms can spawn after sleepless nights.")),
        "doImmediateRespawn" | "doimmediaterespawn" => Some(e("Instant Respawn", "Skip the death screen.")),
        "doLimitedCrafting" | "dolimitedcrafting" => Some(e("Limited Crafting", "Only unlocked recipes craftable.")),
        "commandBlockOutput" | "commandblockoutput" => Some(e("Command Block Output", "Announce command block actions in chat.")),
        "commandBlocksEnabled" | "commandblocksenabled" => Some(e("Command Blocks", "Command blocks function in this world.")),
        "randomTickSpeed" => Some(e("Random Tick Speed", "Crop and growth speed (default 3).")),
        "pvp" => Some(e("PvP", "Players can damage each other.")),
        "drowningdamage" => Some(e("Drowning Damage", "Water deals damage.")),
        "falldamage" => Some(e("Fall Damage", "Falling deals damage.")),
        "firedamage" => Some(e("Fire Damage", "Fire deals damage.")),
        "freezedamage" => Some(e("Freeze Damage", "Powder snow deals damage.")),
        "showcoordinates" => Some(e("Show Coordinates", "Display coordinates on screen (Bedrock).")),
        "showDeathMessages" | "showdeathmessages" => Some(e("Death Messages", "Announce player deaths in chat.")),
        "spectatorsGenerateChunks" => Some(e("Spectators Load Chunks", "Spectators cause chunk generation.")),
        "naturalRegeneration" => Some(e("Natural Regeneration", "Health regenerates when hunger is full.")),
        "tntExplodes" | "tntexplodes" => Some(e("TNT Explodes", "TNT is functional.")),
        "maxCommandChainLength" | "maxcommandchainlength" => Some(e("Command Chain Limit", "Max command blocks run per tick.")),
        "spawnRadius" => Some(e("Spawn Radius", "How far from spawn players may appear.")),
        "sendCommandFeedback" | "sendcommandfeedback" => Some(e("Command Feedback", "Show command output in chat.")),

        // ---- entities (Java + Bedrock actors) ----
        "identifier" => Some(e("Entity Type", "Namespaced entity id (Bedrock).")),
        "id" => Some(e("Entity Type", "Entity id string (Java) — or item id inside item stacks.")),
        "Pos" => Some(e("Position", "XYZ coordinates in blocks (floats).")),
        "Motion" => Some(e("Motion", "Velocity vector (blocks/tick).")),
        "Rotation" => Some(e("Rotation", "Pitch and yaw in degrees.")),
        "Health" => Some(e("Health", "Current health in half-hearts.")),
        "HealthF" => Some(e("Health", "Current health (float form, Bedrock).")),
        "MaxHealth" => Some(e("Max Health", "Maximum health in half-hearts.")),
        "Fire" => Some(e("Fire Ticks", "Ticks left burning; -20 means not on fire.")),
        "Air" | "AirSupply" => Some(e("Air Supply", "Ticks of air left before drowning (300 max).")),
        "FallDistance" | "fall_distance" => Some(e("Fall Distance", "Blocks fallen; drives fall damage.")),
        "Invulnerable" => Some(e("Invulnerable", "Immune to all damage except /kill.")),
        "NoAI" => Some(e("No AI", "Entity frozen: no movement or behaviour.")),
        "Silent" => Some(e("Silent", "Plays no sounds.")),
        "Persistent" => Some(e("Won't Despawn", "Never removed when far from players.")),
        "CustomName" => Some(e("Custom Name", "Name shown above the entity (name tag).")),
        "CustomNameVisible" => Some(e("Show Name Always", "Name visible without looking at it.")),
        "UniqueID" => Some(e("Unique ID", "Bedrock's signed-64-bit entity id.")),
        "UUID" => Some(e("UUID", "Java's entity UUID (IntArray[4]).")),
        "Owner" | "owner" | "OwnerNew" => Some(e("Owner", "Tamed-by reference (pets).")),
        "OwnerUUID" => Some(e("Owner UUID", "UUID of the pet's owner.")),
        "Saddle" => Some(e("Saddled", "Carries a saddle.")),
        "ChestedHorse" => Some(e("Carrying Chest", "Horse/donkey has a chest attached.")),
        "Items" => Some(e("Contents", "Container inventory (items or horse chest).")),
        "HandItems" => Some(e("Held Items", "Main-hand and off-hand stacks.")),
        "ArmorItems" => Some(e("Armor", "Equipped armor, feet → head.")),
        "Attributes" => Some(e("Attributes", "Movement speed, max health, and other stats.")),
        "PortalCooldown" => Some(e("Portal Cooldown", "Ticks before the entity can use a portal again.")),
        "DeathTime" => Some(e("Death Animation", "Ticks since death started (visual only).")),
        "HurtTime" => Some(e("Hurt Flash", "Ticks of red damage flash remaining.")),
        "OnGround" => Some(e("On Ground", "Touching the ground.")),
        "equipment" => Some(e("Equipment", "Equipped items (26.x shape).")),
        "Pose" => Some(e("Pose", "Animation pose (standing, sleeping, …).")),
        "Leashed" => Some(e("Leashed", "Attached to a leash.")),
        "Angry" | "angry" => Some(e("Angry", "Hostile state (e.g. angered bees/wolves).")),
        "Sitting" | "sitting" => Some(e("Sitting", "Currently sitting (tamed pets).")),
        "baby" | "IsBaby" => Some(e("Baby", "Juvenile variant.")),
        "variant" => Some(e("Variant", "Appearance/type index (cats, horses…).")),
        "Color" => Some(e("Color", "Color index (collars, wool, …).")),
        "SkeletonType" => Some(e("Skeleton Kind", "0 normal, 1 wither, 2 stray.")),
        " Transform" => None,

        // ---- item stacks ----
        "Count" => Some(e("Count", "Stack size (byte).")),
        "Damage" => Some(e("Durability Used", "Bedrock damage value on tools/armor.")),
        "WasPickedUp" => Some(e("Picked-Up Flag", "Internal Bedrock flag set when a mob picked the item up.")),
        "Slot" => Some(e("Slot", "Container slot index.")),
        "tag" => Some(e("Extra Data", "Legacy compound of item components (pre-1.20.5).")),
        "components" => Some(e("Components", "Modern per-item component map (1.20.5+).")),
        "display" => Some(e("Display", "Custom name and lore.")),
        "Lore" => Some(e("Lore", "Description lines under the item name.")),
        "ench" | "enchantments" => Some(e("Enchantments", "Enchantment list (id + level).")),
        "Unbreakable" => Some(e("Unbreakable", "Never loses durability.")),
        "RepairCost" => Some(e("Repair Cost", "Anvil XP cost added by prior repairs.")),
        "CustomModelData" => Some(e("Custom Model Data", "Resource-pack model selector.")),

        // ---- block entities ----
        "Lock" => Some(e("Lock", "Container locked unless held item name matches.")),
        "CookTime" => Some(e("Cook Progress", "Furnace cooking ticks so far.")),
        "CookTimeTotal" => Some(e("Cook Duration", "Ticks one smelt takes.")),
        "BurnTime" => Some(e("Fuel Burn", "Fuel ticks remaining.")),
        "BurnDuration" => Some(e("Fuel Duration", "Total ticks the current fuel provides.")),
        "RecordItem" => Some(e("Disc", "Music disc inserted in a jukebox.")),
        "Primary" => Some(e("Primary Effect", "Beacon primary potion effect id.")),
        "Secondary" => Some(e("Secondary Effect", "Beacon secondary potion effect id.")),
        "Levels" => Some(e("Beacon Level", "Beacon pyramid tier (1–4).")),
        "Book" => Some(e("Book", "Book/quad sitting in a lectern.")),
        "title" => Some(e("Title", "Written book title.")),
        "author" => Some(e("Author", "Written book author.")),
        "pages" => Some(e("Pages", "Book page texts.")),
        "Text1" => Some(e("Sign Line 1", "Sign text, line 1 (legacy shape).")),
        "Text2" => Some(e("Sign Line 2", "Sign text, line 2 (legacy shape).")),
        "Text3" => Some(e("Sign Line 3", "Sign text, line 3 (legacy shape).")),
        "Text4" => Some(e("Sign Line 4", "Sign text, line 4 (legacy shape).")),
        "front_text" => Some(e("Front Text", "Sign front face (26.x shape).")),
        "back_text" => Some(e("Back Text", "Sign back face (26.x shape).")),
        "messages" => Some(e("Sign Lines", "Four-line text array.")),
        "Command" => Some(e("Command", "Command stored in a command block.")),
        "SuccessCount" => Some(e("Success Count", "Comparator signal from last run.")),
        "LastExecution" => Some(e("Last Run", "Tick the command last executed.")),
        "TrackOutput" => Some(e("Track Output", "Keep last command output for display.")),
        "auto" => Some(e("Auto", "Runs without redstone.")),
        "Delay" => Some(e("Spawn Delay", "Spawner: ticks until next spawn.")),
        "SpawnCount" => Some(e("Spawn Count", "Spawner: mobs per cycle.")),
        "MaxNearbyEntities" => Some(e("Max Nearby", "Spawner: stops when this many mobs are near.")),
        "RequiredPlayerRange" => Some(e("Player Range", "Spawner: needs a player within this range.")),
        "SpawnRange" => Some(e("Spawn Range", "Spawner: horizontal radius for spawns.")),
        "MinSpawnDelay" | "MaxSpawnDelay" => Some(e("Spawn Delay Range", "Spawner: random delay bounds.")),
        "SpawnData" => Some(e("Next Spawn", "Spawner: entity being spawned.")),
        "SpawnPotentials" => Some(e("Spawn Pool", "Spawner: weighted spawn candidates.")),
        "LootTable" => Some(e("Loot Table", "Container fills from this loot table.")),
        "LootTableSeed" => Some(e("Loot Seed", "Deterministic loot rolls.")),
        "KeepPacked" => Some(e("Keep Packed", "Internal Bedrock flag (pending re-save).")),
        "isMovable" => Some(e("Movable", "Pistons may push this block entity.")),
        "block_entities" => Some(e("Block Entities", "Chests, signs, furnaces in this chunk.")),
        _ => None,
    }
}

/// Human name for a Java DataVersion when we know it (fixture- or
/// memory-verified values only; unknown numbers stay unnamed rather than
/// guessed).
pub fn data_version_name(dv: i64) -> Option<&'static str> {
    match dv {
        1451 => Some("1.12.2"),
        2730 => Some("1.17.1"),
        3700 => Some("1.20.4"),
        3955 => Some("1.21"),
        4189 => Some("1.21.4"),
        5023 => Some("26.3"),
        _ => None,
    }
}

// ---- SNBT value decoding ---------------------------------------------------

/// Decode a scalar SNBT value into friendly text for known keys.
/// Best-effort: unparseable or unknown keys return `None`.
pub fn decode(key: &str, snbt: &str) -> Option<String> {
    let v = snbt.trim();
    match key {
        "GameType" => {
            let n = parse_int(v)?;
            Some(match n {
                0 => "Survival".into(),
                1 => "Creative".into(),
                2 => "Adventure".into(),
                3 => "Spectator".into(),
                _ => format!("{n} (unknown)"),
            })
        }
        "Difficulty" => {
            // Legacy: int 0..3. 26.3: text ("normal").
            if let Some(n) = parse_int(v) {
                Some(
                    match n {
                        0 => "Peaceful",
                        1 => "Easy",
                        2 => "Normal",
                        3 => "Hard",
                        _ => return Some(format!("{n} (unknown)")),
                    }
                    .into(),
                )
            } else {
                Some(v.trim_matches('"').to_string())
            }
        }
        "DifficultyValue" => {
            let f = parse_float(v)?;
            Some(
                match f {
                    x if x < 0.5 => "Peaceful",
                    x if x < 1.5 => "Easy",
                    x if x < 2.5 => "Normal",
                    _ => "Hard",
                }
                .into(),
            )
        }
        "LastPlayed" => {
            let n = parse_int(v)?;
            Some(epoch_display(n))
        }
        "Time" | "currentTick" => {
            let n = parse_int(v)?;
            Some(format!("{n} ticks ≈ {} in-game day(s)", n / 24_000))
        }
        "DayTime" => {
            let n = parse_int(v)?;
            Some(daytime_display(n.rem_euclid(24_000)))
        }
        "version" => {
            let n = parse_int(v)?;
            Some(match n {
                19133 => "Anvil (modern)".into(),
                19132 => "McRegion (legacy)".into(),
                _ => format!("{n}"),
            })
        }
        "DataVersion" => {
            let n = parse_int(v)?;
            Some(match data_version_name(n) {
                Some(name) => format!("{n} → {name}"),
                None => format!("{n} (see minecraft.wiki Data_version)"),
            })
        }
        "MinimumCompatibleClientVersion" | "lastOpenedWithVersion" => {
            // IntArray like "[I; 1, 26, 50, 0, 0]" → "1.26.50"
            let nums: Vec<i64> = v
                .trim_start_matches("[I;")
                .trim_end_matches(']')
                .split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect();
            if nums.len() >= 3 {
                Some(format!("{}.{}.{}", nums[0], nums[1], nums[2]))
            } else {
                None
            }
        }
        "allowCommands" | "commandsEnabled" | "cheatsEnabled" => {
            parse_bool(v).map(|b| if b { "Cheats enabled".into() } else { "Cheats disabled".into() })
        }
        "hardcore" | "IsHardcore" => {
            parse_bool(v).map(|b| if b { "Hardcore ON".into() } else { "Off".into() })
        }
        "keepInventory" | "keepinventory" | "doDaylightCycle" | "dodaylightcycle"
        | "mobGriefing" | "mobgriefing" | "pvp" => {
            parse_bool(v).map(|b| if b { "On".into() } else { "Off".into() })
        }
        "LevelName" => None, // already human text
        _ => None,
    }
}

fn parse_int(v: &str) -> Option<i64> {
    v.trim_end_matches(['b', 'B', 's', 'S', 'l', 'L']).parse().ok()
}

fn parse_float(v: &str) -> Option<f64> {
    v.trim_end_matches(['f', 'F', 'd', 'D']).parse().ok()
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim() {
        "1b" | "1" | "true" => Some(true),
        "0b" | "0" | "false" => Some(false),
        _ => None,
    }
}

/// Epoch → "YYYY-MM-DD HH:MM (local)". Heuristic scale: ≥1e12 treated as ms
/// (Java shape), otherwise seconds (this Bedrock fixture stores seconds).
fn epoch_display(n: i64) -> String {
    let secs = if n.abs() >= 1_000_000_000_000 { n / 1000 } else { n };
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant's algorithm), y/m/d from Unix day number.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let (h, min) = (rem / 3600, (rem % 3600) / 60);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}")
}

/// Day-cycle tick → "HH:MM (in-game)". 0 tick = 06:00 dawn.
fn daytime_display(t: i64) -> String {
    let mins = ((t * 24 * 60) / 24_000 + 6 * 60) % (24 * 60);
    format!("{:02}:{:02} in-game time", mins / 60, mins % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_keys() {
        let g = lookup("GameType").unwrap();
        assert_eq!(g.label, "Game Mode");
        assert!(lookup("totally_unknown").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn enum_decoding() {
        assert_eq!(decode("GameType", "0").unwrap(), "Survival");
        assert_eq!(decode("GameType", "1b").unwrap(), "Creative");
        assert_eq!(decode("GameType", "9").unwrap(), "9 (unknown)");
        assert_eq!(decode("Difficulty", "3").unwrap(), "Hard");
        assert_eq!(decode("Difficulty", "\"normal\"").unwrap(), "normal");
        assert_eq!(decode("DifficultyValue", "0.5f").unwrap(), "Easy");
        assert_eq!(decode("DifficultyValue", "3.0f").unwrap(), "Hard");
    }

    #[test]
    fn time_decoding() {
        // Java ms shape (UTC; matches `TZ=UTC date -d @1794153184`).
        assert_eq!(decode("LastPlayed", "1794153184608L").unwrap(), "2026-11-08 15:53");
        // Bedrock seconds shape (fixture; matches `TZ=UTC date -d @1792768119`).
        assert_eq!(decode("LastPlayed", "1792768119L").unwrap(), "2026-10-23 15:08");
        assert_eq!(decode("Time", "48000L").unwrap(), "48000 ticks ≈ 2 in-game day(s)");
        assert_eq!(decode("DayTime", "0L").unwrap(), "06:00 in-game time");
        assert_eq!(decode("DayTime", "6000L").unwrap(), "12:00 in-game time");
    }

    #[test]
    fn version_decoding() {
        assert_eq!(decode("version", "19133").unwrap(), "Anvil (modern)");
        assert_eq!(decode("DataVersion", "5023").unwrap(), "5023 → 26.3");
        assert!(decode("DataVersion", "1").unwrap().contains("wiki"));
        assert_eq!(
            decode("MinimumCompatibleClientVersion", "[I; 1, 26, 50, 0, 0]").unwrap(),
            "1.26.50"
        );
        assert_eq!(decode("commandsEnabled", "1b").unwrap(), "Cheats enabled");
        assert_eq!(decode("allowCommands", "0b").unwrap(), "Cheats disabled");
        assert_eq!(decode("pvp", "true").unwrap(), "On");
    }

    #[test]
    fn data_version_names() {
        assert_eq!(data_version_name(1451), Some("1.12.2"));
        assert_eq!(data_version_name(2730), Some("1.17.1"));
        assert_eq!(data_version_name(1), None);
    }
}
