//! Entity (mob/object) translation, Java ⇄ Bedrock (M4).
//!
//! Shapes from census probes (`examples/entprobe.rs`, `omt-bedrock/examples/
//! actorkeys.rs`) of the Java 26.3 fixture and a Bedrock world:
//! - Java: `id`, `UUID` `[I;4]`, `Pos`/`Motion` Double lists, `Rotation`
//!   Float list, `Health`, `Age`, `PersistenceRequired`, `CustomName`
//!   (text component), equipment, `Tags`.
//! - Bedrock actors: `identifier`, `UniqueID` Long, `definitions`
//!   (`+<identifier>` component groups), `Pos`/`Motion`/`Rotation` **Float**
//!   lists, `Persistent`, `IsBaby`, `CustomName` (plain), `Armor`/`Mainhand`/
//!   `Offhand` item lists. Storage key = `actorprefix` + BE
//!   `[−(UniqueID>>32)][UniqueID low32]` (probe-verified).
//!
//! Fields with no counterpart are reported, never guessed.

use omt_nbt::Tag;

use crate::items::{ItemConvertOpts, StackEra, bedrock_stack_to_java, java_stack_to_bedrock};
use crate::loss::{ConversionReport, LossKind};

/// Java id ↔ Bedrock identifier where they differ.
const RENAMES: &[(&str, &str)] = &[
    ("villager", "villager_v2"),
    ("zombie_villager", "zombie_villager_v2"),
    ("experience_orb", "xp_orb"),
    ("evoker", "evocation_illager"),
    ("end_crystal", "ender_crystal"),
    ("tropical_fish", "tropicalfish"),
    ("eye_of_ender", "eye_of_ender_signal"),
    ("potion", "splash_potion"),
    ("snow_golem", "snow_golem"),
    ("tnt", "tnt"),
    ("mooshroom", "mooshroom"),
];

/// Java entity ids Bedrock stores as blocks (or not at all).
const NOT_ACTORS: &[&str] = &["item_frame", "glow_item_frame", "falling_block", "marker", "interaction", "block_display", "item_display", "text_display"];

fn short(id: &str) -> &str {
    id.split(['<']).next().unwrap_or(id).strip_prefix("minecraft:").unwrap_or(id)
}

pub fn bedrock_identifier(java_id: &str) -> Option<String> {
    let s = short(java_id);
    if NOT_ACTORS.contains(&s) {
        return None;
    }
    if s.ends_with("_chest_boat") || s.ends_with("_chest_raft") {
        return Some("minecraft:chest_boat".into());
    }
    if s.ends_with("_boat") || s.ends_with("_raft") {
        return Some("minecraft:boat".into());
    }
    let b = RENAMES.iter().find(|(j, _)| *j == s).map_or(s, |(_, b)| *b);
    Some(format!("minecraft:{b}"))
}

pub fn java_id(bedrock_identifier: &str) -> String {
    let s = short(bedrock_identifier);
    let j = RENAMES.iter().find(|(_, b)| *b == s).map_or(s, |(j, _)| *j);
    if j == "boat" {
        return "minecraft:oak_boat".into();
    }
    if j == "chest_boat" {
        return "minecraft:oak_chest_boat".into();
    }
    format!("minecraft:{j}")
}

/// Deterministic Bedrock UniqueID from a Java UUID (derived, not random):
/// high word negative in 1..=0x7fff like the game's own ids.
pub fn unique_id_from_uuid(uuid: &[i32]) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for w in uuid {
        for b in w.to_be_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    let hi = ((h >> 32) as u32 % 0x7ff0 + 0x10) as i64;
    let lo = (h & 0xffff_ffff) as i64;
    (-hi << 32) | lo
}

/// The `actorprefix` key suffix for a UniqueID (probe-verified layout).
pub fn actor_key_suffix(unique_id: i64) -> [u8; 8] {
    let hi = (-(unique_id >> 32)) as u32;
    let lo = unique_id as u32;
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&hi.to_be_bytes());
    out[4..].copy_from_slice(&lo.to_be_bytes());
    out
}

fn floats(t: Option<&Tag>, n: usize) -> Option<Vec<f64>> {
    let l = t?.as_list()?;
    let v: Vec<f64> = l
        .iter()
        .filter_map(|x| match x {
            Tag::Float(f) => Some(*f as f64),
            Tag::Double(d) => Some(*d),
            _ => None,
        })
        .collect();
    (v.len() == n).then_some(v)
}

fn int(t: Option<&Tag>) -> Option<i64> {
    t.and_then(|t| t.as_i64())
}

fn empty_bedrock_item() -> Tag {
    Tag::Compound(vec![
        ("Count".into(), Tag::Byte(0)),
        ("Damage".into(), Tag::Short(0)),
        ("Name".into(), Tag::String(String::new())),
        ("WasPickedUp".into(), Tag::Byte(0)),
    ])
}

fn to_bedrock_item(it: Option<&Tag>, opts: &ItemConvertOpts, report: &mut ConversionReport) -> Tag {
    match it {
        Some(t) if t.get("id").is_some() => match java_stack_to_bedrock(t, opts) {
            Ok((b, rep)) => {
                report.extend(&rep);
                b
            }
            Err(_) => empty_bedrock_item(),
        },
        _ => empty_bedrock_item(),
    }
}

/// Java entity → Bedrock actor NBT + its UniqueID. `None` = no Bedrock
/// actor counterpart (reported).
pub fn java_to_bedrock(e: &Tag, opts: &ItemConvertOpts, report: &mut ConversionReport) -> Option<(Tag, i64)> {
    let jid = e.get("id").and_then(|t| t.as_str()).unwrap_or("");
    let Some(ident) = bedrock_identifier(jid) else {
        report.record(LossKind::UnmappedId, format!("entities {jid}"), "Bedrock stores this as a block or has no equivalent; dropped");
        return None;
    };
    let pos = floats(e.get("Pos"), 3)?;
    let uuid: Vec<i32> = match e.get("UUID") {
        Some(Tag::IntArray(u)) => u.clone(),
        _ => vec![pos[0].to_bits() as i32, pos[1].to_bits() as i32, pos[2].to_bits() as i32, jid.len() as i32],
    };
    let uid = unique_id_from_uuid(&uuid);
    let flist = |v: Vec<f64>| Tag::List(v.into_iter().map(|x| Tag::Float(x as f32)).collect());
    let mut c: Vec<(String, Tag)> = vec![
        ("identifier".into(), Tag::String(ident.clone())),
        ("definitions".into(), Tag::List(vec![Tag::String(format!("+{ident}"))])),
        ("UniqueID".into(), Tag::Long(uid)),
        ("Pos".into(), flist(pos)),
        ("Rotation".into(), flist(floats(e.get("Rotation"), 2).unwrap_or(vec![0.0, 0.0]))),
        ("Motion".into(), flist(floats(e.get("Motion"), 3).unwrap_or(vec![0.0, 0.0, 0.0]))),
        ("Persistent".into(), Tag::Byte(int(e.get("PersistenceRequired")).unwrap_or(0) as i8)),
        ("OnGround".into(), Tag::Byte(int(e.get("OnGround")).unwrap_or(0) as i8)),
        ("Invulnerable".into(), Tag::Byte(int(e.get("Invulnerable")).unwrap_or(0) as i8)),
    ];
    if let Some(age) = int(e.get("Age"))
        && age < 0
    {
        c.push(("IsBaby".into(), Tag::Byte(1)));
    }
    if let Some(name) = e.get("CustomName") {
        c.push(("CustomName".into(), Tag::String(crate::block_entities::plain_text(name))));
        c.push(("CustomNameVisible".into(), Tag::Byte(int(e.get("CustomNameVisible")).unwrap_or(0) as i8)));
    }
    if let Some(tags) = e.get("Tags") {
        c.push(("Tags".into(), tags.clone()));
    }
    if short(jid) == "item" {
        c.push(("Item".into(), to_bedrock_item(e.get("Item"), opts, report)));
    }
    // Equipment: 26.x `equipment{head,chest,legs,feet,mainhand,offhand}`
    // or legacy `ArmorItems` (feet→head) / `HandItems`.
    let eq = e.get("equipment");
    let legacy_armor = e.get("ArmorItems").and_then(|t| t.as_list());
    if eq.is_some() || legacy_armor.is_some() {
        let slot = |k: &str, legacy_i: usize| eq.and_then(|q| q.get(k)).or_else(|| legacy_armor.and_then(|l| l.get(legacy_i)));
        let mut armor = Vec::new();
        for (k, li) in [("head", 3), ("chest", 2), ("legs", 1), ("feet", 0)] {
            armor.push(to_bedrock_item(slot(k, li), opts, report));
        }
        armor.push(empty_bedrock_item()); // body slot
        c.push(("Armor".into(), Tag::List(armor)));
        let hands = e.get("HandItems").and_then(|t| t.as_list());
        let main = eq.and_then(|q| q.get("mainhand")).or_else(|| hands.and_then(|h| h.first()));
        let off = eq.and_then(|q| q.get("offhand")).or_else(|| hands.and_then(|h| h.get(1)));
        c.push(("Mainhand".into(), Tag::List(vec![to_bedrock_item(main, opts, report)])));
        c.push(("Offhand".into(), Tag::List(vec![to_bedrock_item(off, opts, report)])));
    }
    if e.get("Health").is_some() || e.get("attributes").is_some() {
        report.record(LossKind::FieldDropped, "entities Health/attributes", "Bedrock rebuilds attributes from the entity definition; values reset to defaults");
    }
    Some((Tag::Compound(c), uid))
}

fn to_java_item(it: &Tag, opts: &ItemConvertOpts, report: &mut ConversionReport) -> Option<Tag> {
    if it.get("Name").and_then(|t| t.as_str()).is_none_or(str::is_empty) {
        return None;
    }
    let (j, rep) = bedrock_stack_to_java(it, StackEra::Components, opts).ok()?;
    report.extend(&rep);
    Some(j)
}

/// Bedrock actor → Java entity (26.3 shapes). UUIDs are never invented:
/// Java assigns one on load when absent.
pub fn bedrock_to_java(a: &Tag, opts: &ItemConvertOpts, report: &mut ConversionReport) -> Option<Tag> {
    let ident = a.get("identifier").and_then(|t| t.as_str())?;
    let pos = floats(a.get("Pos"), 3)?;
    let dlist = |v: Vec<f64>| Tag::List(v.into_iter().map(Tag::Double).collect());
    let mut c: Vec<(String, Tag)> = vec![
        ("id".into(), Tag::String(java_id(ident))),
        ("Pos".into(), dlist(pos)),
        ("Motion".into(), dlist(floats(a.get("Motion"), 3).unwrap_or(vec![0.0, 0.0, 0.0]))),
        (
            "Rotation".into(),
            Tag::List(floats(a.get("Rotation"), 2).unwrap_or(vec![0.0, 0.0]).into_iter().map(|x| Tag::Float(x as f32)).collect()),
        ),
        ("PersistenceRequired".into(), Tag::Byte(int(a.get("Persistent")).unwrap_or(0) as i8)),
        ("OnGround".into(), Tag::Byte(int(a.get("OnGround")).unwrap_or(0) as i8)),
        ("Invulnerable".into(), Tag::Byte(int(a.get("Invulnerable")).unwrap_or(0) as i8)),
    ];
    if int(a.get("IsBaby")).unwrap_or(0) != 0 {
        c.push(("Age".into(), Tag::Int(-24000)));
    }
    if let Some(name) = a.get("CustomName").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) {
        c.push(("CustomName".into(), Tag::String(name.into())));
        c.push(("CustomNameVisible".into(), Tag::Byte(int(a.get("CustomNameVisible")).unwrap_or(0) as i8)));
    }
    if let Some(tags) = a.get("Tags") {
        c.push(("Tags".into(), tags.clone()));
    }
    if short(ident) == "item"
        && let Some(it) = a.get("Item").and_then(|i| to_java_item(i, opts, report))
    {
        c.push(("Item".into(), it));
    }
    let armor = a.get("Armor").and_then(|t| t.as_list());
    let main = a.get("Mainhand").and_then(|t| t.as_list()).and_then(|l| l.first());
    let off = a.get("Offhand").and_then(|t| t.as_list()).and_then(|l| l.first());
    let mut eq = Vec::new();
    if let Some(armor) = armor {
        for (i, k) in ["head", "chest", "legs", "feet"].iter().enumerate() {
            if let Some(it) = armor.get(i).and_then(|t| to_java_item(t, opts, report)) {
                eq.push((k.to_string(), it));
            }
        }
    }
    for (k, t) in [("mainhand", main), ("offhand", off)] {
        if let Some(it) = t.and_then(|t| to_java_item(t, opts, report)) {
            eq.push((k.to_string(), it));
        }
    }
    if !eq.is_empty() {
        c.push(("equipment".into(), Tag::Compound(eq)));
    }
    if !ident.starts_with("minecraft:") {
        report.record(LossKind::UnmappedId, format!("entities {}", short(ident)), "add-on (non-vanilla) actor: no Java entity; written under its Bedrock id");
    }
    Some(Tag::Compound(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omt_nbt::snbt::from_snbt;

    #[test]
    fn key_layout_matches_probe() {
        // Probe: UniqueID 0xfffffffe00000005 ↔ key 00000002 00000005.
        assert_eq!(actor_key_suffix(0xfffffffe00000005u64 as i64), [0, 0, 0, 2, 0, 0, 0, 5]);
        let uid = unique_id_from_uuid(&[1, 2, 3, 4]);
        assert!(uid < 0);
        assert_eq!(uid, unique_id_from_uuid(&[1, 2, 3, 4]), "deterministic");
        let k = actor_key_suffix(uid);
        assert_ne!(&k[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn cow_roundtrip() {
        let java = from_snbt(r#"{id:"minecraft:villager", UUID:[I;1,2,3,4], Pos:[1.5d, 64.0d, -2.5d], Rotation:[90.0f, 0.0f], Motion:[0.0d,0.0d,0.0d], Age:-100, PersistenceRequired:1b, CustomName:{text:"Bob"}, equipment:{mainhand:{id:"minecraft:emerald", count:2}}}"#).unwrap();
        let mut rep = ConversionReport::new();
        let opts = ItemConvertOpts::default();
        let (b, _) = java_to_bedrock(&java, &opts, &mut rep).unwrap();
        assert_eq!(b.get("identifier").unwrap().as_str(), Some("minecraft:villager_v2"));
        assert!(matches!(b.get("Pos").unwrap().as_list().unwrap()[0], Tag::Float(f) if f == 1.5));
        assert_eq!(b.get("IsBaby").unwrap().as_i64(), Some(1));
        assert_eq!(b.get("CustomName").unwrap().as_str(), Some("Bob"));
        let main = &b.get("Mainhand").unwrap().as_list().unwrap()[0];
        assert_eq!(main.get("Name").unwrap().as_str(), Some("minecraft:emerald"));

        let back = bedrock_to_java(&b, &opts, &mut rep).unwrap();
        assert_eq!(back.get("id").unwrap().as_str(), Some("minecraft:villager"));
        assert!(matches!(back.get("Pos").unwrap().as_list().unwrap()[2], Tag::Double(d) if d == -2.5));
        assert_eq!(back.get("PersistenceRequired").unwrap().as_i64(), Some(1));
        assert_eq!(back.get("equipment").unwrap().get("mainhand").unwrap().get("id").unwrap().as_str(), Some("minecraft:emerald"));
        assert!(back.get("UUID").is_none(), "never invented");
        // Frames are blocks in Bedrock.
        let frame = from_snbt(r#"{id:"minecraft:item_frame", Pos:[0.0d,0.0d,0.0d]}"#).unwrap();
        assert!(java_to_bedrock(&frame, &opts, &mut rep).is_none());
    }
}
