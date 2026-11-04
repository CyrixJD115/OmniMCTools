//! Entity-list handling for chunk conversion (findings Task C,
//! `ChunkTaskConvert::run` `0x459C10`).
//!
//! UMT's universal chunk stores entities under an era-dependent key:
//! `Level.Entities` (legacy era) or root lowercase `entities` (modern
//! era). When writing a Java destination the converter assembles an
//! entity-region root `{DataVersion, Position: [I; x, z], Entities}`.
//!
//! **Border-chunk special case — deliberately absent.** The findings
//! conclude: "OMT converts whole worlds with global chunk addressing in
//! one rayon pipeline → no border special-case needed; entity regions are
//! keyed by absolute chunk coords." UMT's region-edge deferral
//! (`0x459C10` head + the `0x45CC30` QHash state machine) exists only to
//! sequence writes across its independently-converted 32×32 region
//! tasks; nothing about it changes the output format, so it is not
//! mirrored here.

use omt_nbt::Tag;

use crate::proto::ChunkEra;

/// The key under which a universal chunk of this era keeps its entity
/// list: `Entities` (legacy, under `Level`) / `entities` (modern, at
/// root). Casing is a UMT-verified format fact (findings Task C).
pub fn entities_key(era: ChunkEra) -> &'static str {
    match era {
        ChunkEra::Legacy => "Entities",
        ChunkEra::Modern => "entities",
    }
}

/// Find the entity list in a universal chunk compound, reading it from
/// the era-correct location. Returns the list tag (possibly empty);
/// `None` when the chunk has no entity list at all (callers treat that
/// as "nothing to move").
pub fn find_entities(chunk: &Tag, era: ChunkEra) -> Option<&Tag> {
    let entries = chunk.as_compound()?;
    match era {
        ChunkEra::Legacy => chunk.get("Level")?.get("Entities"),
        ChunkEra::Modern => entries
            .iter()
            .find(|(k, _)| k == "entities")
            .map(|(_, v)| v),
    }
}

/// Build a Java entity-region root compound (1.17+ format, UMT-verified):
///
/// ```text
/// {DataVersion: <dv>, Position: [I; chunk_x, chunk_z], Entities: [...]}
/// ```
///
/// `Position` is a **two-int** int-array — the call site pushes exactly
/// `2` (`FUN_00459910("Position", 2)`, findings Task C) — holding the
/// chunk's X and Z. Entity NBT is carried opaquely: the list elements
/// are moved as-is (id/Pos/Rotation remapping is M4 registry work).
pub fn entity_region_root(
    chunk_x: i32,
    chunk_z: i32,
    data_version: i32,
    entities: Vec<Tag>,
) -> Tag {
    Tag::Compound(vec![
        ("DataVersion".to_string(), Tag::Int(data_version)),
        ("Entities".to_string(), Tag::List(entities)),
        (
            "Position".to_string(),
            Tag::IntArray(vec![chunk_x, chunk_z]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use omt_nbt::snbt::{from_snbt, to_snbt};

    #[test]
    fn key_casing_is_era_dependent() {
        assert_eq!(entities_key(ChunkEra::Legacy), "Entities");
        assert_eq!(entities_key(ChunkEra::Modern), "entities");
    }

    #[test]
    fn find_entities_reads_era_correct_location() {
        let legacy = from_snbt(
            "{Level: {Entities: [{id: \"minecraft:cow\"}]}, Status: \"full\"}",
        )
        .unwrap();
        let modern = from_snbt("{entities: [{id: \"minecraft:pig\"}], Status: \"full\"}").unwrap();

        let found = find_entities(&legacy, ChunkEra::Legacy).unwrap();
        assert_eq!(found.as_list().unwrap().len(), 1);
        assert_eq!(
            found.as_list().unwrap()[0]
                .get("id")
                .and_then(Tag::as_str),
            Some("minecraft:cow")
        );

        let found = find_entities(&modern, ChunkEra::Modern).unwrap();
        assert_eq!(
            found.as_list().unwrap()[0]
                .get("id")
                .and_then(Tag::as_str),
            Some("minecraft:pig")
        );

        // Wrong-era lookups miss (list lives elsewhere) and absent lists
        // are None — both are "nothing to move", never an error.
        assert!(find_entities(&legacy, ChunkEra::Modern).is_none());
        assert!(find_entities(&modern, ChunkEra::Legacy).is_none());
        assert!(
            find_entities(&from_snbt("{Status: \"full\"}").unwrap(), ChunkEra::Modern)
                .is_none()
        );
    }

    #[test]
    fn entity_region_root_is_java_117_shape() {
        let pig = from_snbt("{id: \"minecraft:pig\", Pos: [0.5, 64.0, 0.5]}").unwrap();
        let root = entity_region_root(-3, 7, 3953, vec![pig]);

        assert_eq!(root.get("DataVersion"), Some(&Tag::Int(3953)));
        // Position: exactly 2 ints, [x, z] — the UMT-verified shape.
        assert_eq!(
            root.get("Position"),
            Some(&Tag::IntArray(vec![-3, 7])),
            "Position must be [I; chunk_x, chunk_z] with exactly two ints"
        );
        let entities = root.get("Entities").unwrap().as_list().unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(
            entities[0].get("id").and_then(Tag::as_str),
            Some("minecraft:pig")
        );

        // SNBT round-trips (CLI-facing shape check).
        let snbt = to_snbt(&root);
        assert!(snbt.contains("Position:[I; -3, 7]"), "in {snbt}");
    }

    #[test]
    fn empty_entity_region_root_is_valid() {
        let root = entity_region_root(0, 0, 4189, Vec::new());
        assert_eq!(root.get("Entities"), Some(&Tag::List(Vec::new())));
        assert_eq!(root.get("Position"), Some(&Tag::IntArray(vec![0, 0])));
    }
}
