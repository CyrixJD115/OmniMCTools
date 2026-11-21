//! Item stacks and containers, one shape for both editions.
//!
//! * Java 1.20.5+: `{Slot, id, count, components: {minecraft:custom_name,
//!   minecraft:lore, minecraft:enchantments, minecraft:damage,
//!   minecraft:container}}`
//! * Java before 1.20.5: `{Slot, id, Count, tag: {display: {Name, Lore},
//!   Enchantments: [{id, lvl}], Damage, BlockEntityTag: {Items}}}`
//! * Bedrock: `{Slot, Name, Count, Damage, tag: {display: {Name, Lore},
//!   ench: [{id, lvl}], Items}}`

use omt_nbt::Tag;

/// A normalized item stack.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub slot: i32,
    /// Namespaced id (`minecraft:diamond_sword`).
    pub id: String,
    pub count: i32,
    pub damage: i32,
    /// Custom name (plain text), when renamed.
    pub custom_name: Option<String>,
    pub lore: Vec<String>,
    /// `(id, level)`; Bedrock numeric ids become `bedrock:<n>`.
    pub enchantments: Vec<(String, i32)>,
    /// Items stored inside (shulker boxes, bundles).
    pub contents: Vec<Item>,
}

fn get<'a>(t: &'a Tag, key: &str) -> Option<&'a Tag> {
    match t {
        Tag::Compound(e) => e.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn int(t: Option<&Tag>) -> Option<i64> {
    match t? {
        Tag::Byte(v) => Some(i64::from(*v)),
        Tag::Short(v) => Some(i64::from(*v)),
        Tag::Int(v) => Some(i64::from(*v)),
        Tag::Long(v) => Some(*v),
        _ => None,
    }
}

fn string(t: Option<&Tag>) -> Option<&str> {
    match t? {
        Tag::String(s) => Some(s),
        _ => None,
    }
}

/// Text component (JSON string, plain string or compound) → plain text.
pub fn plain_text(t: &Tag) -> String {
    match t {
        Tag::String(s) => match serde_json::from_str::<serde_json::Value>(s) {
            Ok(v) => json_text(&v),
            Err(_) => s.clone(),
        },
        Tag::Compound(_) => {
            let mut s = string(get(t, "text")).unwrap_or_default().to_string();
            if let Some(Tag::List(extra)) = get(t, "extra") {
                s.extend(extra.iter().map(plain_text));
            }
            s
        }
        Tag::List(l) => l.iter().map(plain_text).collect(),
        _ => String::new(),
    }
}

fn json_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().map(json_text).collect(),
        serde_json::Value::Object(o) => {
            let mut s = o.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_string();
            if let Some(serde_json::Value::Array(extra)) = o.get("extra") {
                s.extend(extra.iter().map(json_text));
            }
            s
        }
        other => other.to_string(),
    }
}

fn namespaced(id: &str) -> String {
    if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") }
}

/// Normalize one item compound (`None` = not an item: no id/Name).
pub fn item(t: &Tag) -> Option<Item> {
    // Java before 1.8 stored numeric ids (`id:276s`).
    let legacy = int(get(t, "id")).and_then(|n| crate::legacy_items::name(n, int(get(t, "Damage")).unwrap_or(0)));
    let id = legacy.as_deref().or_else(|| string(get(t, "id"))).or_else(|| string(get(t, "Name"))).filter(|s| !s.is_empty())?;
    if id == "minecraft:air" || id == "air" {
        return None;
    }
    let mut it = Item {
        slot: int(get(t, "Slot")).or_else(|| int(get(t, "slot"))).unwrap_or(-1) as i32,
        id: namespaced(id),
        count: int(get(t, "count")).or_else(|| int(get(t, "Count"))).unwrap_or(1) as i32,
        damage: int(get(t, "Damage")).unwrap_or(0) as i32,
        ..Default::default()
    };
    // Java 1.20.5+ components.
    if let Some(c) = get(t, "components") {
        it.custom_name = get(c, "minecraft:custom_name").or_else(|| get(c, "minecraft:item_name")).map(plain_text).filter(|s| !s.is_empty());
        if let Some(Tag::List(l)) = get(c, "minecraft:lore") {
            it.lore = l.iter().map(plain_text).collect();
        }
        if let Some(e) = get(c, "minecraft:enchantments") {
            // 1.21.5+: map id → level; 1.20.5–1.21.4: {levels: {…}}.
            let levels = get(e, "levels").unwrap_or(e);
            if let Tag::Compound(m) = levels {
                it.enchantments = m.iter().filter_map(|(k, v)| Some((namespaced(k), int(Some(v))? as i32))).collect();
            }
        }
        it.damage = int(get(c, "minecraft:damage")).unwrap_or(it.damage as i64) as i32;
        if let Some(Tag::List(l)) = get(c, "minecraft:container") {
            it.contents = l
                .iter()
                .filter_map(|e| {
                    let mut i = item(get(e, "item")?)?;
                    i.slot = int(get(e, "slot")).unwrap_or(-1) as i32;
                    Some(i)
                })
                .collect();
        }
        if let Some(Tag::List(l)) = get(c, "minecraft:bundle_contents") {
            it.contents = l.iter().filter_map(item).collect();
        }
    }
    // Java legacy / Bedrock `tag`.
    if let Some(tag) = get(t, "tag") {
        if let Some(d) = get(tag, "display") {
            it.custom_name = it.custom_name.or_else(|| get(d, "Name").map(plain_text).filter(|s| !s.is_empty()));
            if let Some(Tag::List(l)) = get(d, "Lore") {
                it.lore = l.iter().map(plain_text).collect();
            }
        }
        for key in ["Enchantments", "ench", "StoredEnchantments"] {
            if let Some(Tag::List(l)) = get(tag, key) {
                for e in l {
                    let id = match get(e, "id") {
                        Some(Tag::String(s)) => namespaced(s),
                        other => match int(other) {
                            Some(n) => bedrock_enchantment(n),
                            None => continue,
                        },
                    };
                    it.enchantments.push((id, int(get(e, "lvl")).unwrap_or(1) as i32));
                }
            }
        }
        it.damage = int(get(tag, "Damage")).unwrap_or(it.damage as i64) as i32;
        let inner = get(tag, "BlockEntityTag").and_then(|b| get(b, "Items")).or_else(|| get(tag, "Items"));
        if let Some(Tag::List(l)) = inner {
            it.contents = l.iter().filter_map(item).collect();
        }
    }
    Some(it)
}

/// Bedrock stores enchantments by numeric id (`ench: [{id: 15s, lvl: 3s}]`).
fn bedrock_enchantment(n: i64) -> String {
    const NAMES: [&str; 41] = [
        "protection", "fire_protection", "feather_falling", "blast_protection", "projectile_protection", "thorns", "respiration", "depth_strider",
        "aqua_affinity", "sharpness", "smite", "bane_of_arthropods", "knockback", "fire_aspect", "looting", "efficiency", "silk_touch", "unbreaking",
        "fortune", "power", "punch", "flame", "infinity", "luck_of_the_sea", "lure", "frost_walker", "mending", "binding_curse", "vanishing_curse",
        "impaling", "riptide", "loyalty", "channeling", "multishot", "piercing", "quick_charge", "soul_speed", "swift_sneak", "wind_burst", "density", "breach",
    ];
    usize::try_from(n).ok().and_then(|i| NAMES.get(i)).map_or_else(|| format!("bedrock:{n}"), |s| format!("minecraft:{s}"))
}

/// Normalize a list of item compounds (anything else is skipped).
pub fn items(list: &Tag) -> Vec<Item> {
    match list {
        Tag::List(l) => l.iter().filter_map(item).collect(),
        _ => Vec::new(),
    }
}

/// Normalized container kind of a block-entity id (both editions):
/// `ShulkerBox` / `minecraft:shulker_box` → `shulker_box`.
pub fn container_kind(id: &str) -> String {
    let id = id.rsplit(':').next().unwrap_or(id);
    let mut out = String::new();
    for (i, c) in id.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Grid of a container kind: `(rows, cols)`; `None` = not a container.
pub fn layout(kind: &str) -> Option<(usize, usize)> {
    Some(match kind {
        "chest" | "trapped_chest" | "barrel" | "ender_chest" => (3, 9),
        k if k.ends_with("shulker_box") => (3, 9),
        "hopper" => (1, 5),
        "dispenser" | "dropper" | "crafter" => (3, 3),
        "furnace" | "smoker" | "blast_furnace" | "lit_furnace" => (1, 3),
        "brewing_stand" => (1, 5),
        "chiseled_bookshelf" => (2, 3),
        "decorated_pot" | "jukebox" | "lectern" => (1, 1),
        "campfire" | "soul_campfire" => (1, 4),
        "chest_boat" | "chest_minecart" | "minecart_chest" => (3, 9),
        "hopper_minecart" | "minecart_hopper" => (1, 5),
        _ => return None,
    })
}

/// A new item compound in an edition's layout (`java` = 1.20.5+ shape).
pub fn make(id: &str, count: i32, slot: i32, java: bool) -> Tag {
    let id = namespaced(id);
    if java {
        Tag::Compound(vec![("Slot".into(), Tag::Byte(slot as i8)), ("id".into(), Tag::String(id)), ("count".into(), Tag::Int(count))])
    } else {
        Tag::Compound(vec![
            ("Count".into(), Tag::Byte(count.clamp(0, 127) as i8)),
            ("Damage".into(), Tag::Short(0)),
            ("Name".into(), Tag::String(id)),
            ("Slot".into(), Tag::Byte(slot as i8)),
            ("WasPickedUp".into(), Tag::Byte(0)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(e: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(e.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn java_modern_components() {
        let shulker_item = c(vec![("id", Tag::String("minecraft:apple".into())), ("count", Tag::Int(5))]);
        let t = c(vec![
            ("Slot", Tag::Byte(4)),
            ("id", Tag::String("minecraft:shulker_box".into())),
            ("count", Tag::Int(1)),
            (
                "components",
                c(vec![
                    ("minecraft:custom_name", Tag::String(r#"{"text":"Loot","extra":[{"text":" box"}]}"#.into())),
                    ("minecraft:enchantments", c(vec![("minecraft:unbreaking", Tag::Int(3))])),
                    ("minecraft:container", Tag::List(vec![c(vec![("slot", Tag::Int(2)), ("item", shulker_item)])])),
                ]),
            ),
        ]);
        let it = item(&t).unwrap();
        assert_eq!((it.slot, it.id.as_str(), it.count), (4, "minecraft:shulker_box", 1));
        assert_eq!(it.custom_name.as_deref(), Some("Loot box"));
        assert_eq!(it.enchantments, vec![("minecraft:unbreaking".into(), 3)]);
        assert_eq!((it.contents[0].slot, it.contents[0].id.as_str(), it.contents[0].count), (2, "minecraft:apple", 5));
    }

    #[test]
    fn java_legacy_and_bedrock() {
        let legacy = c(vec![
            ("Slot", Tag::Byte(0)),
            ("id", Tag::String("minecraft:diamond_sword".into())),
            ("Count", Tag::Byte(1)),
            ("tag", c(vec![("Damage", Tag::Int(12)), ("Enchantments", Tag::List(vec![c(vec![("id", Tag::String("minecraft:sharpness".into())), ("lvl", Tag::Short(5))])]))])),
        ]);
        let it = item(&legacy).unwrap();
        assert_eq!((it.damage, it.enchantments[0].1), (12, 5));
        let bedrock = c(vec![
            ("Count", Tag::Byte(64)),
            ("Damage", Tag::Short(0)),
            ("Name", Tag::String("minecraft:cobblestone".into())),
            ("Slot", Tag::Byte(26)),
            ("tag", c(vec![("display", c(vec![("Name", Tag::String("Rocks".into()))])), ("ench", Tag::List(vec![c(vec![("id", Tag::Short(9)), ("lvl", Tag::Short(2))])]))])),
        ]);
        let it = item(&bedrock).unwrap();
        assert_eq!((it.slot, it.count, it.custom_name.as_deref()), (26, 64, Some("Rocks")));
        assert_eq!(it.enchantments, vec![("minecraft:sharpness".into(), 2)]);
        assert_eq!(bedrock_enchantment(15), "minecraft:efficiency");
        assert_eq!(bedrock_enchantment(99), "bedrock:99");
        assert!(item(&c(vec![("Name", Tag::String("minecraft:air".into()))])).is_none(), "air is an empty slot");
        assert_eq!(item(&make("stone", 3, 7, false)).unwrap().slot, 7);
        assert_eq!(item(&make("stone", 3, 7, true)).unwrap().count, 3);
    }

    #[test]
    fn container_kinds_and_layouts() {
        assert_eq!(container_kind("ShulkerBox"), "shulker_box");
        assert_eq!(container_kind("minecraft:trapped_chest"), "trapped_chest");
        assert_eq!(container_kind("BrewingStand"), "brewing_stand");
        assert_eq!(layout("red_shulker_box"), Some((3, 9)));
        assert_eq!(layout("hopper"), Some((1, 5)));
        assert_eq!(layout("sign"), None);
    }
}
