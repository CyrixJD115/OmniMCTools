//! Fixture-gated e2e: open the REAL Java 26.3 world (PLAN §8 `java-latest`),
//! exercising the namespaced layout, the modern level.dat shape, the
//! flattened palette, and a full chunk decode through the shared model.
//!
//! Run: `cargo test -p omt-java --test java26_fixture -- --ignored`
//! Path: `OMT_JAVA_FIXTURE` env or `../../world_ref/java-26.3` (never committed).

use std::path::{Path, PathBuf};

use omt_java::{JavaWorld, JavaLayout};
use omt_world::WorldStore;

fn fixture_dir() -> PathBuf {
    std::env::var_os("OMT_JAVA_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../world_ref/java-26.3")
        })
}

#[test]
#[ignore = "fixture-gated: needs the real Java 26.3 world (OMT_JAVA_FIXTURE / world_ref)"]
fn java26_fixture_roundtrip() {
    let dir = fixture_dir();
    if !dir.is_dir() {
        panic!("Java 26.3 fixture not found at {} (set OMT_JAVA_FIXTURE)", dir.display());
    }

    let detected = omt_world::detect(&dir).unwrap();
    assert!(matches!(detected, omt_world::Detected::JavaFolder { .. }));

    let world = JavaWorld::open(&dir).unwrap();
    assert_eq!(world.layout, JavaLayout::Namespaced, "26.3 worlds use the namespaced layout");
    assert_eq!(world.data_version(), Some(5023), "26.3 final DataVersion");

    // Modern level.dat shape.
    let data = world.data();
    assert_eq!(
        data.get("LevelName").and_then(|t| t.as_str()),
        Some("Omni MC Tools")
    );
    assert_eq!(omt_world::java_spawn_xz(data), Some((0, 0)), "spawn compound pos [0,107,0]");
    assert!(
        data.get("GameRules").is_none(),
        "gamerules moved to data/minecraft/game_rules.dat in 26.3"
    );
    assert!(data.get("difficulty_settings").is_some(), "difficulty moved into a compound");

    // Overworld chunks enumerate through dimensions/minecraft/overworld/region.
    let chunks = world.list_chunks(0).unwrap();
    assert!(!chunks.is_empty(), "fixture has an explored overworld");
    assert!(world.dimension_dir(0).unwrap().is_dir());

    // Unexplored dimensions: empty list, no error (friendly handling).
    if !world.dimension_dir(1).map(|d| d.is_dir()).unwrap_or(false) {
        assert_eq!(world.list_chunks(1).unwrap(), Vec::<(i32, i32)>::new());
    }

    // One real FULL chunk decodes through the shared model with the flattened
    // palette (string entries + { "" | id, properties } compounds). Real
    // worlds mix generation states — the sorted-first chunk can be a proto
    // chunk (e.g. minecraft:structure_starts), so scan for a full one.
    let chunk = chunks
        .iter()
        .map(|&(x, z)| (x, z, world.read_chunk(0, x, z).unwrap().expect("chunk present")))
        .find(|(_, _, c)| c.status.as_deref() == Some("minecraft:full"))
        .map(|(x, z, c)| (x, z, c))
        .expect("fixture contains full chunks");
    let (x, z) = (chunk.0, chunk.1);
    let chunk = chunk.2;
    assert_eq!(chunk.data_version, 5023);
    assert!(chunk.biomes_3d, "sections carry 3D biomes");
    assert!(!chunk.slices.is_empty());
    // Palette names must be real (the 26.3 flattening once defaulted every
    // entry to air in the reader): at least one non-air name somewhere.
    let non_air = chunk
        .slices
        .values()
        .flat_map(|s| s.palette.iter())
        .any(|b| b.name != "minecraft:air");
    assert!(non_air, "decoded palette must contain non-air block names");
    // Property-bearing entries decode (e.g. leaves/vines carry properties).
    let with_props = chunk
        .slices
        .values()
        .flat_map(|s| s.palette.iter())
        .find(|b| !b.properties.is_empty());
    if let Some(b) = with_props {
        assert!(b.get("_nonexistent_key_").is_none());
    }

    // Heightmaps: modern chunk roots carry the 4 standard packed maps.
    let names: Vec<&str> = chunk.heightmaps.iter().map(|(n, _)| n.as_str()).collect();
    for expected in ["MOTION_BLOCKING", "WORLD_SURFACE"] {
        assert!(names.contains(&expected), "missing heightmap {expected}: {names:?}");
    }

    // Player file via players/data.
    let players = world.list_players().unwrap();
    assert!(!players.is_empty(), "fixture world has a player");
    let raw = world
        .read_raw(&omt_world::WorldNode::PlayerFile(players[0].clone()))
        .unwrap();
    assert!(raw.len() > 8, "player .dat non-empty");

    // WorldStore objectivity across the layout boundary.
    let store = &world as &dyn WorldStore;
    assert_eq!(store.iter_chunks(0).unwrap().count(), chunks.len());
}
