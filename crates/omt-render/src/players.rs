//! Every player stored in a world (uNmINeD's "Server players" list):
//!
//! - **Java**: the single-player `level.dat` player, `playerdata/<uuid>.dat`
//!   (26.3+: `players/data`), and pre-1.7.6 `players/<name>.dat`. Names come
//!   from `usercache.json` (world folder or the server folder above it),
//!   Bukkit's `bukkit.lastKnownName`, or the user's name map.
//! - **Bedrock**: `~local_player` and every `player_server_<uuid>` record,
//!   linked to `player_<id>` records for the MSA / self-signed ids. Bedrock
//!   stores no names: they come from the user's name map.

use std::collections::HashMap;

use omt_nbt::{ByteOrder, Tag};

/// One player record.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInfo {
    /// Stable id (Java UUID, Bedrock `server_<uuid>`, or `local`).
    pub id: String,
    pub name: Option<String>,
    /// Bedrock Microsoft account id / self-signed id when known.
    pub msa_id: Option<String>,
    pub self_signed_id: Option<String>,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub dim: u8,
    pub game_mode: Option<i64>,
    pub level: Option<i64>,
    pub health: Option<f64>,
    /// The world's own (single-player / host) player.
    pub local: bool,
}

impl PlayerInfo {
    /// Display label: name, else a short id.
    pub fn label(&self) -> String {
        match (&self.name, self.local) {
            (Some(n), _) => n.clone(),
            (None, true) => "Local player".into(),
            (None, false) => format!("Player {}", self.id.trim_start_matches("server_").chars().take(8).collect::<String>()),
        }
    }

    /// Lookup keys for the user's name map (any id the player is known by).
    pub fn ids(&self) -> Vec<&str> {
        [Some(self.id.as_str()), self.msa_id.as_deref(), self.self_signed_id.as_deref()].into_iter().flatten().collect()
    }

    pub fn game_mode_name(&self) -> Option<&'static str> {
        Some(match self.game_mode? {
            0 => "Survival",
            1 => "Creative",
            2 => "Adventure",
            3 => "Spectator",
            _ => return None,
        })
    }
}

fn num(t: Option<&Tag>) -> Option<f64> {
    match t? {
        Tag::Double(v) => Some(*v),
        Tag::Float(v) => Some(*v as f64),
        other => other.as_i64().map(|v| v as f64),
    }
}

fn pos(t: &Tag) -> Option<(f64, f64, f64)> {
    let l = t.get("Pos")?.as_list()?;
    Some((num(l.first())?, num(l.get(1))?, num(l.get(2))?))
}

fn java_dim(t: Option<&Tag>) -> u8 {
    match t {
        Some(Tag::String(s)) if s.ends_with("the_nether") => 1,
        Some(Tag::String(s)) if s.ends_with("the_end") => 2,
        Some(other) => match other.as_i64() {
            Some(-1) => 1,
            Some(1) => 2,
            _ => 0,
        },
        None => 0,
    }
}

fn base(id: String, t: &Tag, dim: u8, local: bool) -> Option<PlayerInfo> {
    let (x, y, z) = pos(t)?;
    Some(PlayerInfo {
        id,
        name: None,
        msa_id: None,
        self_signed_id: None,
        x,
        y,
        z,
        dim,
        game_mode: t.get("playerGameType").or_else(|| t.get("PlayerGameMode")).and_then(Tag::as_i64),
        level: t.get("XpLevel").or_else(|| t.get("PlayerLevel")).and_then(Tag::as_i64),
        health: num(t.get("Health")),
        local,
    })
}

/// `usercache.json` entries (uuid → name) from the world folder or the
/// server folder above it.
pub fn java_usercache(root: &std::path::Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for dir in [Some(root), root.parent()].into_iter().flatten() {
        let Ok(text) = std::fs::read_to_string(dir.join("usercache.json")) else { continue };
        let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for it in items {
            if let (Some(u), Some(n)) = (it.get("uuid").and_then(|v| v.as_str()), it.get("name").and_then(|v| v.as_str())) {
                out.entry(u.to_lowercase()).or_insert_with(|| n.to_string());
            }
        }
    }
    out
}

/// All players of a Java world; `names` is the user's id → name map.
pub fn java_players(world: &omt_java::JavaWorld, names: &HashMap<String, String>) -> Vec<PlayerInfo> {
    let read = |p: &std::path::Path| -> Option<omt_nbt::Nbt> {
        let bytes = std::fs::read(p).ok()?;
        let (payload, _) = omt_nbt::Container::decode(&bytes).ok()?;
        omt_nbt::read(&payload, ByteOrder::Big).ok()
    };
    let cache = java_usercache(&world.root);
    let mut out = Vec::new();
    if let Some(level) = read(&world.root.join("level.dat"))
        && let Some(p) = level.tag.get("Data").and_then(|d| d.get("Player"))
        && let Some(mut info) = base("local".into(), p, java_dim(p.get("Dimension")), true)
    {
        info.name = names.get("local").cloned();
        out.push(info);
    }
    let mut files: Vec<(String, std::path::PathBuf, bool)> = world.list_players().unwrap_or_default().into_iter().map(|id| (id.clone(), world.players_dir().join(format!("{id}.dat")), false)).collect();
    // Pre-1.7.6 servers named the files after the player.
    if let Ok(rd) = std::fs::read_dir(world.root.join("players")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "dat")
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
            {
                files.push((stem.to_string(), p.clone(), true));
            }
        }
    }
    for (id, path, named_file) in files {
        let Some(n) = read(&path) else { continue };
        let Some(mut info) = base(id.clone(), &n.tag, java_dim(n.tag.get("Dimension")), false) else { continue };
        info.name = names
            .get(&id)
            .cloned()
            .or_else(|| cache.get(&id.to_lowercase()).cloned())
            .or_else(|| n.tag.get("bukkit").and_then(|b| b.get("lastKnownName")).and_then(Tag::as_str).map(str::to_owned))
            .or_else(|| named_file.then(|| id.clone()));
        out.push(info);
    }
    out
}

/// All players of a Bedrock world; `names` is the user's id → name map.
pub fn bedrock_players(world: &omt_bedrock::BedrockWorld, names: &HashMap<String, String>) -> Vec<PlayerInfo> {
    let read = |key: &[u8]| -> Option<Tag> { omt_nbt::read(&world.raw_get(key).ok()??, ByteOrder::Little).ok().map(|n| n.tag) };
    let dim = |t: &Tag| match t.get("DimensionId").and_then(Tag::as_i64) {
        Some(1) => 1,
        Some(2) => 2,
        _ => 0,
    };
    // `player_<id>` records map account ids to `ServerId`.
    let mut accounts: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for key in world.prefix_keys(b"player_").unwrap_or_default() {
        if key.starts_with(b"player_server_") {
            continue;
        }
        if let Some(t) = read(&key)
            && let Some(server) = t.get("ServerId").and_then(Tag::as_str)
        {
            let msa = t.get("MsaId").and_then(Tag::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
            let ss = t.get("SelfSignedId").and_then(Tag::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
            accounts.insert(server.to_string(), (msa, ss));
        }
    }
    let mut out = Vec::new();
    if let Some(t) = read(b"~local_player")
        && let Some(mut info) = base("local".into(), &t, dim(&t), true)
    {
        info.name = names.get("local").cloned();
        out.push(info);
    }
    for key in world.prefix_keys(b"player_server_").unwrap_or_default() {
        let id = String::from_utf8_lossy(&key["player_".len()..]).into_owned();
        let Some(t) = read(&key) else { continue };
        let Some(mut info) = base(id.clone(), &t, dim(&t), false) else { continue };
        if let Some((msa, ss)) = accounts.get(&String::from_utf8_lossy(&key).into_owned()) {
            info.msa_id = msa.clone();
            info.self_signed_id = ss.clone();
        }
        info.name = info.ids().iter().find_map(|i| names.get(*i).cloned());
        out.push(info);
    }
    out
}

/// Parse a uNmINeD-compatible player name map (`id name` per line, `//`
/// comments) — the format of `playernames.txt`.
pub fn parse_name_map(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#'))
        .filter_map(|l| {
            let (id, name) = l.split_once(char::is_whitespace)?;
            Some((id.trim().to_string(), name.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn name_map_format() {
        let m = super::parse_name_map("// comment\n00000000-0000-0000-0000-000000000000 nobody\nserver_abc  Steve Two\n\n");
        assert_eq!(m.get("server_abc").map(String::as_str), Some("Steve Two"));
        assert_eq!(m.len(), 2);
    }
}
