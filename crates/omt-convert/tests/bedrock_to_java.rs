//! Fixture-independent end-to-end for the reverse direction (M3b): build
//! universal `Chunk` values directly (the shape `BedrockWorld::read_chunk_full`
//! returns — YZX-normalized slices, data3D biomes, actors), convert them to
//! Java, write a world folder via the pipeline apply step, then reopen it
//! with `JavaWorld` and verify blocks at **absolute positions** (the
//! permutation-bug class needs absolute checks — symmetric round-trips
//! hide it).

use std::collections::BTreeMap;

use omt_convert::{JavaChunkWrite, JavaDimensionOutcome, apply_java_conversions, bedrock_chunk_to_java};
use omt_java::RegionFile;
use omt_java::{JavaWorld, RegionWriter};
use omt_nbt::{ByteOrder, Tag};
use omt_world::{BedrockBiomeSlot, BlockState, Chunk, Slice, WorldStore, packing};

fn air() -> BlockState {
    BlockState::air()
}

fn state(name: &str, props: &[(&str, &str)]) -> BlockState {
    BlockState {
        name: name.into(),
        properties: props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    }
}

/// Expected slot-0 palette index for a local position — derived from the
/// position so any permutation of axes breaks the assertions.
fn pattern_index(x: usize, y: usize, z: usize) -> u16 {
    ((x * 7 + z * 3 + y) % 3 + 1) as u16
}

/// A chunk with a non-symmetric pattern: every block in slot 0 carries a
/// position-derived palette index (1..=3, never air).
fn patterned_chunk(x: i32, z: i32) -> Chunk {
    let mut chunk = Chunk { x, z, dim: 0, ..Default::default() };
    chunk.status = Some("full".into());

    let palette = vec![
        air(),
        state("minecraft:oak_planks", &[]),
        state("minecraft:cobblestone", &[]),
        state("minecraft:oak_stairs", &[("facing", "east")]),
    ];
    let mut indices = vec![0u16; 4096];
    for y in 0..16usize {
        for z in 0..16usize {
            for x in 0..16usize {
                // YZX order (the universal model's order).
                indices[(y << 8) | (z << 4) | x] = pattern_index(x, y, z);
            }
        }
    }
    chunk.slices.insert(0, Slice { palette, indices });

    // Slot −4: uniform deepslate floor (single-entry palette, no data).
    chunk.slices.insert(
        -4,
        Slice {
            palette: vec![state("minecraft:deepslate", &[])],
            indices: vec![0; 4096],
        },
    );

    // Biomes: slot 4 (world Y 0) per-block plains/desert split on x < 8;
    // slots 1..3 NonExistent (chain), slot 0 uniform river.
    let mut slot4 = vec![0u16; 4096]; // 0 = plains (id 1)
    for y in 0..16usize {
        for z in 0..16usize {
            for x in 8..16usize {
                slot4[omt_bedrock::subchunk::block_index_xzy(x, y, z)] = 1; // desert (id 2)
            }
        }
    }
    chunk.bedrock_biomes = Some(vec![
        BedrockBiomeSlot::uniform(7),                                                    // slot 0 (Y −64)
        BedrockBiomeSlot::NonExistent,                                                   // slot 1: chains to river
        BedrockBiomeSlot::NonExistent,                                                   // slot 2
        BedrockBiomeSlot::NonExistent,                                                   // slot 3
        BedrockBiomeSlot::Storage { palette: vec![1, 2], indices: Some(slot4), bits: 1 }, // slot 4 (Y 0)
    ]);

    // data3D-equivalent heightmap: z-major, x fastest (probe fact).
    chunk.heightmaps =
        vec![("bedrock_2d".into(), (0..256).map(|i| (70 + i % 16) as u64).collect())];

    // A block entity + an actor (Bedrock shape: identifier + UniqueID,
    // no UUID — probe-verified actor shape).
    chunk.block_entities = vec![Tag::Compound(vec![
        ("id".to_string(), Tag::String("Chest".into())),
        ("x".to_string(), Tag::Int(x * 16 + 1)),
        ("y".to_string(), Tag::Int(0)),
        ("z".to_string(), Tag::Int(z * 16 + 2)),
    ])];
    chunk.entities = vec![Tag::Compound(vec![
        ("identifier".to_string(), Tag::String("minecraft:pig".into())),
        ("UniqueID".to_string(), Tag::Long(12345)),
        ("Pos".to_string(), Tag::List(vec![Tag::Float(0.5), Tag::Float(64.0), Tag::Float(0.5)])),
    ])];
    chunk
}

fn convert_one(chunk: &Chunk) -> JavaChunkWrite {
    match bedrock_chunk_to_java(
        chunk,
        &omt_convert::RegistryRules::default(),
        &omt_convert::ProtoFilter::default(),
        4440,
    )
    .unwrap()
    {
        omt_convert::BedrockConversion::Converted(w) => w,
        omt_convert::BedrockConversion::SkippedProto { .. } => panic!("expected conversion"),
    }
}

fn expected_block_name(idx: u16) -> &'static str {
    match idx {
        1 => "minecraft:oak_planks",
        2 => "minecraft:cobblestone",
        _ => "minecraft:oak_stairs",
    }
}

#[test]
fn synthetic_bedrock_to_java_world_roundtrip() {
    let dir = std::env::temp_dir().join(format!("omt-convert-b2j-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Build the Java target via the pipeline apply step (scaffolds level.dat,
    // writes region + entities files).
    let mut outcome =
        JavaDimensionOutcome { data_version: 4440, ..Default::default() };
    for (x, z) in [(0, 0), (1, 0), (-1, -1)] {
        let w = convert_one(&patterned_chunk(x, z));
        outcome.reports.push(w.report.clone());
        outcome.writes.push(w);
        outcome.converted.push((x, z));
    }
    let applied = apply_java_conversions(&dir, 0, &mut outcome, Some("OMT B2J")).unwrap();
    assert_eq!(applied, 3);

    // Scaffold: level.dat exists and parses through the Java store.
    let world = JavaWorld::open(&dir).unwrap();
    assert_eq!(world.data().get("LevelName").and_then(Tag::as_str), Some("OMT B2J"));

    // Blocks at absolute positions through the full Java read path
    // (JavaWorld region decode → nbt_to_chunk → block_at).
    let chunks: BTreeMap<(i32, i32), _> = [(0, 0), (1, 0), (-1, -1)]
        .into_iter()
        .map(|(x, z)| {
            let chunk = world.read_chunk(0, x, z).unwrap().expect("chunk present");
            ((x, z), chunk)
        })
        .collect();
    for (&(cx, cz), chunk) in &chunks {
        assert_eq!(chunk.status.as_deref(), Some("minecraft:full"));
        // Corners + center of slot 0 (absolute Y 0..15).
        for &(x, y, z) in &[
            (0usize, 0usize, 0usize),
            (15, 0, 15),
            (0, 15, 15),
            (15, 15, 0),
            (7, 7, 7),
            (3, 5, 11),
        ] {
            let got = chunk.block_at(x, y as i32, z).expect("block present").name.clone();
            let expected = expected_block_name(pattern_index(x, y, z));
            assert_eq!(got, expected, "at chunk ({cx},{cz}) local ({x},{y},{z})");
        }
        // Uniform floor slot: absolute Y −64 is deepslate everywhere.
        for &(x, z) in &[(0usize, 0usize), (15, 15)] {
            assert_eq!(chunk.block_at(x, -64, z).unwrap().name, "minecraft:deepslate");
        }
    }

    // Heightmap copied 1:1 (37-long 9-bit round trip through the Java reader).
    let nbt = world.read_chunk_nbt(0, 0, 0).unwrap().unwrap();
    let Tag::LongArray(packed) = nbt.tag.get("Heightmaps").unwrap().get("MOTION_BLOCKING").unwrap()
    else {
        panic!("heightmap not a long array");
    };
    let hm = packing::unpack_java_heightmap(packed);
    let expected_hm: Vec<u16> = (0..256).map(|i| (70 + i % 16) as u16).collect();
    assert_eq!(hm, expected_hm);

    // Biomes: x < 8 → plains, x ≥ 8 → desert (Java palette, 4³ cells).
    let top = nbt
        .tag
        .get("sections")
        .and_then(Tag::as_list)
        .unwrap()
        .iter()
        .find(|s| s.get("Y").and_then(Tag::as_i64) == Some(0))
        .unwrap()
        .clone();
    let biomes = top.get("biomes").unwrap();
    let palette: Vec<String> = biomes
        .get("palette")
        .and_then(Tag::as_list)
        .unwrap()
        .iter()
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    assert_eq!(palette, vec!["minecraft:plains", "minecraft:desert"]);
    let cells = match biomes.get("data").unwrap() {
        Tag::LongArray(v) => packing::unpack_java_biome_cells(v, 2),
        _ => panic!("biome data not packed"),
    };
    let cell_at = |cx: usize, cy: usize, cz: usize| cells[(cy << 4) | (cz << 2) | cx] as usize;
    assert_eq!(palette[cell_at(0, 0, 0)], "minecraft:plains");
    assert_eq!(palette[cell_at(2, 1, 3)], "minecraft:desert");
    // The uniform bottom slot (−4) carries river (id 7) — uniform palette,
    // no data array.
    let bottom = nbt
        .tag
        .get("sections")
        .and_then(Tag::as_list)
        .unwrap()
        .iter()
        .find(|s| s.get("Y").and_then(Tag::as_i64) == Some(-4))
        .unwrap()
        .clone();
    let bottom_names: Vec<&str> = bottom
        .get("biomes")
        .unwrap()
        .get("palette")
        .and_then(Tag::as_list)
        .unwrap()
        .iter()
        .filter_map(Tag::as_str)
        .collect();
    assert_eq!(bottom_names, vec!["minecraft:river"]);
    assert!(bottom.get("biomes").unwrap().get("data").is_none());

    // Block entity passed through (count equality).
    let chunk = chunks.get(&(0, 0)).unwrap();
    assert_eq!(chunk.block_entities.len(), 1);

    // Loss ledger: stateful stairs properties + BE fields + entity rows.
    let merged = outcome.merged_report();
    assert!(merged.count_with_kind(omt_convert::LossKind::UnmappedId) >= 1);
    assert!(!merged.is_clean());

    // Entity region: entities/r.0.0.mca holds the Java 1.17 root shape.
    let epath = dir.join("entities/r.0.0.mca");
    assert!(epath.is_file(), "entity region written");
    let eregion = RegionFile::open(&epath).unwrap();
    let ebytes = std::fs::read(&epath).unwrap();
    let eraw = eregion.read_raw(&ebytes, 0, 0).unwrap();
    let eroot = omt_nbt::read(&eraw, ByteOrder::Big).unwrap().tag;
    assert_eq!(eroot.get("DataVersion").and_then(Tag::as_i64), Some(4440));
    assert_eq!(eroot.get("Position"), Some(&Tag::IntArray(vec![0, 0])));
    let entities = eroot.get("Entities").and_then(Tag::as_list).unwrap();
    assert_eq!(entities.len(), 1);
    // identifier → id rename; UniqueID carried; UUID never invented.
    assert_eq!(entities[0].get("id").and_then(Tag::as_str), Some("minecraft:pig"));
    assert!(entities[0].get("identifier").is_none());
    assert!(entities[0].get("UUID").is_none(), "no UUID invented");
    let _ = RegionWriter::new; // keep the writer import exercised

    // Nether entity regions land in DIM-1/entities (layout mirror check).
    let mut nether_outcome =
        JavaDimensionOutcome { data_version: 4440, ..Default::default() };
    nether_outcome.writes.push(convert_one(&patterned_chunk(0, 0)));
    apply_java_conversions(&dir, 1, &mut nether_outcome, None).unwrap();
    assert!(dir.join("DIM-1/entities/r.0.0.mca").is_file());
    assert!(dir.join("DIM-1/region/r.0.0.mca").is_file());

    std::fs::remove_dir_all(&dir).unwrap();
}
