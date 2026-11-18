//! Map markers and overlays derived from world data: the world seed,
//! slime chunks (both editions' public formulas), and player positions.

use omt_nbt::{ByteOrder, Tag};
use omt_world::{WorldNode, WorldStore};

/// A player on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerMarker {
    pub label: String,
    pub x: f64,
    pub z: f64,
    /// 0 overworld, 1 nether, 2 end.
    pub dim: u8,
}

/// Java's `java.util.Random` (48-bit LCG).
struct JavaRandom(i64);

impl JavaRandom {
    const MULT: i64 = 0x5_DEEC_E66D;
    const MASK: i64 = (1 << 48) - 1;

    fn new(seed: i64) -> Self {
        Self((seed ^ Self::MULT) & Self::MASK)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(Self::MULT).wrapping_add(0xB)) & Self::MASK;
        (self.0 >> (48 - bits)) as i32
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        if bound & -bound == bound {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }
}

/// Java Edition slime chunk (seed-dependent).
pub fn java_slime(seed: i64, cx: i32, cz: i32) -> bool {
    let a = cx.wrapping_mul(cx).wrapping_mul(0x4c_1906) as i64;
    let b = cx.wrapping_mul(0x5a_c0db) as i64;
    let c = (cz.wrapping_mul(cz) as i64).wrapping_mul(0x43_07a7);
    let d = cz.wrapping_mul(0x5_f24f) as i64;
    let s = seed.wrapping_add(a).wrapping_add(b).wrapping_add(c).wrapping_add(d) ^ 0x3ad_8025f;
    JavaRandom::new(s).next_int(10) == 0
}

/// First output of MT19937 for a seed (only the state words it needs).
fn mt_first(seed: u32) -> u32 {
    let mut mt = [0u32; 398];
    mt[0] = seed;
    for i in 1..398 {
        mt[i] = 1_812_433_253u32.wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_add(i as u32);
    }
    let y = (mt[0] & 0x8000_0000) | (mt[1] & 0x7fff_ffff);
    let mut v = mt[397] ^ (y >> 1) ^ if y & 1 != 0 { 0x9908_b0df } else { 0 };
    v ^= v >> 11;
    v ^= (v << 7) & 0x9d2c_5680;
    v ^= (v << 15) & 0xefc6_0000;
    v ^ (v >> 18)
}

/// Bedrock Edition slime chunk (seed-independent).
pub fn bedrock_slime(cx: i32, cz: i32) -> bool {
    let seed = (cx as u32).wrapping_mul(0x1f1f_1f1f) ^ cz as u32;
    mt_first(seed).is_multiple_of(10)
}

/// Seed of a Java world (`None` when unreadable).
pub fn java_seed(root: &std::path::Path) -> Option<i64> {
    omt_convert::level::read_java_settings(root).seed
}

fn list_f64(t: Option<&Tag>) -> Option<(f64, f64)> {
    let l = t?.as_list()?;
    let f = |t: &Tag| match t {
        Tag::Double(v) => Some(*v),
        Tag::Float(v) => Some(*v as f64),
        other => other.as_i64().map(|v| v as f64),
    };
    Some((f(l.first()?)?, f(l.get(2)?)?))
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

/// Players of a Java world: `level.dat` singleplayer + player data files.
pub fn java_players(world: &omt_java::JavaWorld) -> Vec<PlayerMarker> {
    let mut out = Vec::new();
    let read = |p: &std::path::Path| -> Option<omt_nbt::Nbt> {
        let bytes = std::fs::read(p).ok()?;
        let (payload, _) = omt_nbt::Container::decode(&bytes).ok()?;
        omt_nbt::read(&payload, ByteOrder::Big).ok()
    };
    if let Some(level) = read(&world.root.join("level.dat"))
        && let Some(p) = level.tag.get("Data").and_then(|d| d.get("Player"))
        && let Some((x, z)) = list_f64(p.get("Pos"))
    {
        out.push(PlayerMarker { label: "You (singleplayer)".into(), x, z, dim: java_dim(p.get("Dimension")) });
    }
    if let Ok(ids) = world.list_players() {
        for id in ids {
            let Some(n) = read(&world.players_dir().join(format!("{id}.dat"))) else { continue };
            if let Some((x, z)) = list_f64(n.tag.get("Pos")) {
                let short: String = id.chars().take(8).collect();
                out.push(PlayerMarker { label: format!("Player {short}"), x, z, dim: java_dim(n.tag.get("Dimension")) });
            }
        }
    }
    out
}

/// The local player of a Bedrock world.
pub fn bedrock_players(world: &omt_bedrock::BedrockWorld) -> Vec<PlayerMarker> {
    let Ok(bytes) = world.read_raw(&WorldNode::PlayerFile("local".into())) else { return Vec::new() };
    let Ok(n) = omt_nbt::read(&bytes, ByteOrder::Little) else { return Vec::new() };
    let Some((x, z)) = list_f64(n.tag.get("Pos")) else { return Vec::new() };
    let dim = match n.tag.get("DimensionId").and_then(Tag::as_i64) {
        Some(1) => 1,
        Some(2) => 2,
        _ => 0,
    };
    vec![PlayerMarker { label: "Local player".into(), x, z, dim }]
}

/// An entity on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityMarker {
    /// Namespaced id (`minecraft:zombie`).
    pub id: String,
    pub x: f64,
    pub z: f64,
}

/// Rough category for coloring entity pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Hostile,
    Passive,
    Villager,
    Item,
    Other,
}

pub fn entity_kind(id: &str) -> EntityKind {
    let n = id.trim_start_matches("minecraft:");
    const HOSTILE: [&str; 30] = [
        "zombie", "skeleton", "creeper", "spider", "enderman", "witch", "pillager", "vindicator", "evoker", "ravager", "slime", "magma_cube", "phantom", "drowned", "husk", "stray", "blaze", "ghast",
        "piglin_brute", "hoglin", "zoglin", "warden", "guardian", "elder_guardian", "shulker", "silverfish", "endermite", "vex", "wither", "breeze",
    ];
    if HOSTILE.iter().any(|h| n == *h || n.ends_with(&format!("_{h}")) || n.starts_with(&format!("{h}_"))) {
        return EntityKind::Hostile;
    }
    if n.contains("villager") || n == "wandering_trader" || n == "iron_golem" {
        return EntityKind::Villager;
    }
    if matches!(n, "item" | "xp_orb" | "experience_orb" | "arrow" | "trident" | "falling_block" | "tnt" | "item_frame" | "glow_item_frame" | "painting" | "armor_stand" | "minecart" | "boat" | "chest_boat") || n.ends_with("_boat") || n.ends_with("minecart") {
        return EntityKind::Item;
    }
    const PASSIVE: [&str; 24] = ["cow", "pig", "sheep", "chicken", "horse", "donkey", "mule", "llama", "cat", "wolf", "fox", "rabbit", "parrot", "bee", "axolotl", "frog", "goat", "camel", "sniffer", "turtle", "panda", "allay", "armadillo", "ocelot"];
    if PASSIVE.iter().any(|p| n == *p || n.contains(p)) {
        return EntityKind::Passive;
    }
    EntityKind::Other
}

fn marker_from(tag: &Tag, id_key: &str) -> Option<EntityMarker> {
    let id = tag.get(id_key).and_then(Tag::as_str)?.to_string();
    let (x, z) = list_f64(tag.get("Pos"))?;
    Some(EntityMarker { id, x, z })
}

/// Entities of the given chunks (Bedrock: `digp` → actor records).
pub fn bedrock_entities(world: &omt_bedrock::BedrockWorld, dim: u8, chunks: &[(i32, i32)]) -> Vec<EntityMarker> {
    let digp_keys: Vec<Vec<u8>> = chunks.iter().map(|&(x, z)| omt_bedrock::keys::digp_key(x, z, dim)).collect();
    let Ok(digps) = world.get_exact(&digp_keys) else { return Vec::new() };
    let actor_keys: Vec<Vec<u8>> = digps.iter().flatten().flat_map(|d| d.as_chunks::<8>().0.iter().map(|id| omt_bedrock::keys::actor_key(*id)).collect::<Vec<_>>()).collect();
    let Ok(actors) = world.get_exact(&actor_keys) else { return Vec::new() };
    actors.iter().flatten().filter_map(|raw| omt_bedrock::store::parse_actor_nbt(raw).ok()).filter_map(|n| marker_from(&n.tag, "identifier")).collect()
}

/// Entities of the given chunks (Java: `entities/r.x.z.mca`).
pub fn java_entities(world: &omt_java::JavaWorld, dim: u8, chunks: &[(i32, i32)]) -> Vec<EntityMarker> {
    let Some(dir) = world.entity_dimension_dir(dim) else { return Vec::new() };
    let mut by_region: std::collections::BTreeMap<(i32, i32), Vec<(i32, i32)>> = Default::default();
    for &(x, z) in chunks {
        by_region.entry((x.div_euclid(32), z.div_euclid(32))).or_default().push((x, z));
    }
    let mut out = Vec::new();
    for ((rx, rz), cs) in by_region {
        let Ok(region) = omt_java::RegionFile::open(&dir.join(format!("r.{rx}.{rz}.mca"))) else { continue };
        for (x, z) in cs {
            if !region.contains(x, z) {
                continue;
            }
            let Ok(n) = region.read_chunk_from_file(x, z) else { continue };
            for e in n.tag.get("Entities").and_then(Tag::as_list).into_iter().flatten() {
                out.extend(marker_from(e, "id"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_random_matches_reference() {
        // `new java.util.Random(0).nextInt()` == -1155484576 (JDK docs/tests).
        assert_eq!(JavaRandom::new(0).next(32), -1_155_484_576);
        // Bounded draws stay in range.
        let mut r = JavaRandom::new(12345);
        assert!((0..1000).all(|_| (0..10).contains(&r.next_int(10))));
    }

    #[test]
    fn entity_categories() {
        assert_eq!(entity_kind("minecraft:zombie_villager"), EntityKind::Hostile);
        assert_eq!(entity_kind("minecraft:cow"), EntityKind::Passive);
        assert_eq!(entity_kind("minecraft:villager_v2"), EntityKind::Villager);
        assert_eq!(entity_kind("minecraft:item"), EntityKind::Item);
        assert_eq!(entity_kind("minecraft:oak_chest_boat"), EntityKind::Item);
        assert_eq!(entity_kind("minecraft:npc"), EntityKind::Other);
    }

    #[test]
    fn mt19937_reference_output() {
        // Reference: MT19937 seeded with 5489 → first output 3499211612.
        assert_eq!(mt_first(5489), 3_499_211_612);
    }

    #[test]
    fn slime_density_is_about_ten_percent() {
        let n = (-50..50).flat_map(|x| (-50..50).map(move |z| (x, z))).filter(|&(x, z)| bedrock_slime(x, z)).count();
        assert!((800..1200).contains(&n), "bedrock {n}");
        let n = (-50..50).flat_map(|x| (-50..50).map(move |z| (x, z))).filter(|&(x, z)| java_slime(123456789, x, z)).count();
        assert!((800..1200).contains(&n), "java {n}");
    }
}
