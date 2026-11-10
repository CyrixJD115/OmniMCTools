//! Registry-table tests (M3 registry-gen wire-in): the embedded generated
//! table's sanity gates and synthetic stateful round-trips in BOTH
//! directions — no fixture, no jar. The fixture-gated companion lives in
//! `registry_fixture_stats.rs`.

use std::collections::{BTreeMap, BTreeSet};

use omt_convert::chunk_bedrock::java_chunk_to_bedrock;
use omt_convert::chunk_to_java::bedrock_chunk_to_java;
use omt_convert::{ChunkConversion, LossKind, ProtoFilter, RegistryRules};
use omt_nbt::{ByteOrder, Tag};
use omt_world::{BedrockBiomeSlot, BlockState, Chunk, Slice};

fn embedded() -> &'static RegistryRules {
    RegistryRules::embedded()
}

// ---------------------------------------------------------------------------
// Table sanity gates
// ---------------------------------------------------------------------------

#[test]
fn embedded_table_carries_the_generated_registry() {
    let rules = embedded();
    assert!(rules.version >= 1);
    assert!(rules.state_rules.len() > 30_000, "generated table carries the state rules");
    assert!(!rules.block_rules.is_empty(), "static renames present");
    // Biomes: the full modern set (seed aliases included).
    assert_eq!(rules.biome_ids.get("minecraft:basalt_deltas"), Some(&181));
    assert_eq!(rules.biome_ids.get("minecraft:ocean"), Some(&0));
    // Enchantments: complete through lunge; sweeping_edge never guessed.
    assert_eq!(rules.enchant_ids.get("minecraft:sharpness"), Some(&9));
    assert_eq!(rules.enchant_ids.get("minecraft:lunge"), Some(&41));
    assert!(!rules.enchant_ids.contains_key("minecraft:sweeping_edge"));
}

#[test]
fn generated_coverage_meets_the_95_percent_gate() {
    // The committed stats JSON is the generator's own accounting for this
    // exact table build (see tools/registry-gen).
    const STATS: &str = include_str!("../data/registry-stats.json");
    let stats: serde_json::Value = serde_json::from_str(STATS).unwrap();
    let mapped = stats["states_mapped"].as_u64().unwrap();
    let total = stats["java_states"].as_u64().unwrap();
    let pct = 100.0 * mapped as f64 / total as f64;
    assert!(pct >= 95.0, "registry covers only {pct:.1}% of java states");
}

#[test]
fn air_and_default_states_resolve_without_loss() {
    let rules = embedded();
    let mut report = omt_convert::ConversionReport::new();
    let mut seen = BTreeSet::new();
    // Propertyless blocks pass through clean.
    for name in ["minecraft:air", "minecraft:stone", "minecraft:bricks"] {
        let out = rules.map_block(&BlockState { name: name.into(), properties: vec![] }, &mut seen, &mut report);
        assert!(!out.states_dropped, "{name}");
    }
    // The default state of a stateful block (Chunker lists it as an
    // explicit combo) resolves through the exact state rule.
    let stairs_default = BlockState {
        name: "minecraft:oak_stairs".into(),
        properties: vec![
            ("facing".into(), "north".into()),
            ("half".into(), "bottom".into()),
            ("shape".into(), "straight".into()),
            ("waterlogged".into(), "false".into()),
        ],
    };
    let out = rules.map_block(&stairs_default, &mut seen, &mut report);
    assert!(!out.states_dropped);
    assert_eq!(out.state.name, "minecraft:oak_stairs");
    assert!(report.is_clean(), "{report}");
}

// ---------------------------------------------------------------------------
// Forward (Java → Bedrock): stairs / fence / leaves carry their states
// ---------------------------------------------------------------------------

/// Java chunk NBT with one section holding stairs + fence + leaves.
fn java_chunk_nbt() -> omt_nbt::Nbt {
    let palette = Tag::List(vec![
        Tag::Compound(vec![("Name".into(), Tag::String("minecraft:air".into()))]),
        Tag::Compound(vec![
            ("Name".into(), Tag::String("minecraft:oak_stairs".into())),
            (
                "Properties".into(),
                Tag::Compound(vec![
                    ("facing".into(), Tag::String("north".into())),
                    ("half".into(), Tag::String("bottom".into())),
                    ("shape".into(), Tag::String("straight".into())),
                    ("waterlogged".into(), Tag::String("false".into())),
                ]),
            ),
        ]),
        Tag::Compound(vec![
            ("Name".into(), Tag::String("minecraft:oak_fence".into())),
            (
                "Properties".into(),
                Tag::Compound(vec![
                    ("east".into(), Tag::String("false".into())),
                    ("north".into(), Tag::String("true".into())),
                    ("south".into(), Tag::String("false".into())),
                    ("waterlogged".into(), Tag::String("false".into())),
                    ("west".into(), Tag::String("false".into())),
                ]),
            ),
        ]),
        Tag::Compound(vec![
            ("Name".into(), Tag::String("minecraft:oak_leaves".into())),
            (
                "Properties".into(),
                Tag::Compound(vec![
                    ("distance".into(), Tag::String("7".into())),
                    ("persistent".into(), Tag::String("true".into())),
                    ("waterlogged".into(), Tag::String("false".into())),
                ]),
            ),
        ]),
    ]);
    let mut data = vec![0i64; 256];
    let mut set = |x: usize, y: usize, z: usize, v: i64| {
        let i = (y << 8) | (z << 4) | x;
        data[i / 16] |= v << ((i % 16) * 4);
    };
    set(1, 2, 3, 1); // stairs
    set(4, 5, 6, 2); // fence
    set(7, 8, 9, 3); // leaves
    omt_nbt::Nbt {
        name: String::new(),
        tag: Tag::Compound(vec![
            ("DataVersion".into(), Tag::Int(omt_versions::registry().java_data_version as i32)),
            ("xPos".into(), Tag::Int(0)),
            ("zPos".into(), Tag::Int(0)),
            ("Status".into(), Tag::String("minecraft:full".into())),
            (
                "sections".into(),
                Tag::List(vec![Tag::Compound(vec![
                    ("Y".into(), Tag::Byte(0)),
                    (
                        "block_states".into(),
                        Tag::Compound(vec![
                            ("palette".into(), palette),
                            ("data".into(), Tag::LongArray(data)),
                        ]),
                    ),
                    (
                        "biomes".into(),
                        Tag::Compound(vec![(
                            "palette".into(),
                            Tag::List(vec![Tag::String("minecraft:plains".into())]),
                        )]),
                    ),
                ])]),
            ),
        ]),
    }
}

#[test]
fn forward_stateful_blocks_convert_with_states_and_no_loss() {
    let java = omt_convert::parse_java_chunk(&java_chunk_nbt()).unwrap();
    let out =
        java_chunk_to_bedrock(&java, 0, embedded(), &ProtoFilter::default()).unwrap();
    let ChunkConversion::Converted(writes) = out else { panic!("expected conversion") };
    // No unmapped-id loss: every palette entry is table-covered.
    assert!(
        !writes.report.losses.iter().any(|l| l.kind == LossKind::UnmappedId),
        "{}",
        writes.report
    );

    let sub_raw = &writes
        .records
        .iter()
        .find(|(k, _)| k.len() == 10 && k[8] == 0x2f)
        .map(|(_, v)| v.clone())
        .expect("subchunk record");
    let sub = omt_bedrock::subchunk::Subchunk::parse(sub_raw, Some(0)).unwrap();

    // Stairs: facing→weirdo_direction, half→upside_down_bit, shape→corner.
    let stairs = sub.block(1, 2, 3).unwrap();
    assert_eq!(stairs.name, "minecraft:oak_stairs");
    let props: BTreeMap<&str, &str> =
        stairs.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(props.get("weirdo_direction"), Some(&"3"));
    assert_eq!(props.get("upside_down_bit"), Some(&"0b"));
    assert_eq!(props.get("minecraft:corner"), Some(&"none"));

    // Fence: boolean connectors, waterlogged dropped (documented loss).
    let fence = sub.block(4, 5, 6).unwrap();
    assert_eq!(fence.name, "minecraft:oak_fence");
    let fence_props: BTreeMap<&str, &str> =
        fence.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(fence_props.get("minecraft:connection_north"), Some(&"1b"));
    assert_eq!(fence_props.get("minecraft:connection_east"), Some(&"0b"));

    // Leaves: persistent carried, update fixed, distance dropped.
    let leaves = sub.block(7, 8, 9).unwrap();
    assert_eq!(leaves.name, "minecraft:oak_leaves");
    let leaf_props: BTreeMap<&str, &str> =
        leaves.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(leaf_props.get("persistent_bit"), Some(&"1b"));
    assert_eq!(leaf_props.get("update_bit"), Some(&"0b"));
    assert!(!leaf_props.contains_key("distance"), "distance is a documented drop");
}

// ---------------------------------------------------------------------------
// Reverse (Bedrock → Java): the derived inverse restores Java states
// ---------------------------------------------------------------------------

fn bedrock_chunk() -> Chunk {
    let mut chunk = Chunk { x: 0, z: 0, dim: 0, ..Default::default() };
    chunk.status = Some("full".into());
    let mut indices = vec![0u16; 4096];
    indices[(1 << 8) | (2 << 4) | 3] = 1; // stairs at (x=3,y=1,z=2)
    indices[(4 << 8) | (5 << 4) | 6] = 2; // fence at (x=6,y=4,z=5)
    chunk.slices.insert(
        0,
        Slice {
            palette: vec![
                BlockState::air(),
                BlockState {
                    name: "minecraft:oak_stairs".into(),
                    properties: vec![
                        ("minecraft:corner".into(), "none".into()),
                        ("upside_down_bit".into(), "0b".into()),
                        ("weirdo_direction".into(), "3".into()),
                    ],
                },
                BlockState {
                    name: "minecraft:oak_fence".into(),
                    properties: vec![
                        ("minecraft:connection_east".into(), "0b".into()),
                        ("minecraft:connection_north".into(), "1b".into()),
                        ("minecraft:connection_south".into(), "0b".into()),
                        ("minecraft:connection_west".into(), "0b".into()),
                    ],
                },
            ],
            indices,
        },
    );
    chunk.bedrock_biomes = Some(vec![BedrockBiomeSlot::uniform(1)]);
    chunk
}

#[test]
fn reverse_stateful_blocks_restore_java_states_without_loss() {
    let out = bedrock_chunk_to_java(&bedrock_chunk(), embedded(), &ProtoFilter::default(), 5023)
        .unwrap();
    let omt_convert::BedrockConversion::Converted(w) = out else { panic!("expected conversion") };
    assert!(!w.report.losses.iter().any(|l| l.kind == LossKind::UnmappedId), "{}", w.report);

    let root = omt_nbt::read(&w.chunk_nbt, ByteOrder::Big).unwrap().tag;
    let sections = root.get("sections").and_then(Tag::as_list).unwrap();
    let states = sections[0].get("block_states").expect("block_states");
    let palette = states.get("palette").and_then(Tag::as_list).unwrap();

    let find = |name: &str| -> Vec<&Tag> {
        palette
            .iter()
            .filter(|t| t.get("Name").and_then(Tag::as_str) == Some(name))
            .collect()
    };
    let props_of = |t: &Tag| -> BTreeMap<String, String> {
        t.get("Properties")
            .and_then(Tag::as_compound)
            .map(|c| c.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
            .unwrap_or_default()
    };

    // Stairs: weirdo 3 + bottom + none → north/bottom/straight/false.
    let stairs = find("minecraft:oak_stairs");
    assert_eq!(stairs.len(), 1);
    let p = props_of(stairs[0]);
    assert_eq!(p.get("facing").map(String::as_str), Some("north"));
    assert_eq!(p.get("half").map(String::as_str), Some("bottom"));
    assert_eq!(p.get("shape").map(String::as_str), Some("straight"));
    assert_eq!(p.get("waterlogged").map(String::as_str), Some("false"));

    // Fence: connection booleans restore, nothing lost.
    let fence = find("minecraft:oak_fence");
    assert_eq!(fence.len(), 1);
    let p = props_of(fence[0]);
    assert_eq!(p.get("north").map(String::as_str), Some("true"));
    assert_eq!(p.get("east").map(String::as_str), Some("false"));
    assert_eq!(p.get("waterlogged").map(String::as_str), Some("false"));
}

// ---------------------------------------------------------------------------
// Both directions chained: Java → Bedrock → Java keeps the state meaning
// ---------------------------------------------------------------------------

#[test]
fn chained_directions_preserve_the_original_java_state() {
    // Forward.
    let java = omt_convert::parse_java_chunk(&java_chunk_nbt()).unwrap();
    let fwd = java_chunk_to_bedrock(&java, 0, embedded(), &ProtoFilter::default()).unwrap();
    let ChunkConversion::Converted(writes) = fwd else { panic!("expected conversion") };
    let sub_raw = &writes
        .records
        .iter()
        .find(|(k, _)| k.len() == 10 && k[8] == 0x2f)
        .map(|(_, v)| v.clone())
        .expect("subchunk record");
    let sub = omt_bedrock::subchunk::Subchunk::parse(sub_raw, Some(0)).unwrap();

    // Decode the Bedrock sidecar into the universal model the reverse
    // converter consumes.
    let storage = &sub.storages[0];
    let chunk = Chunk {
        x: 0,
        z: 0,
        dim: 0,
        status: Some("full".into()),
        slices: [(0i8, Slice {
            palette: storage.palette.clone(),
            indices: storage.indices.clone().expect("indices"),
        })]
        .into_iter()
        .collect(),
        bedrock_biomes: Some(vec![BedrockBiomeSlot::uniform(1)]),
        ..Default::default()
    };
    let rev = bedrock_chunk_to_java(&chunk, embedded(), &ProtoFilter::default(), 5023).unwrap();
    let omt_convert::BedrockConversion::Converted(w) = rev else { panic!("expected conversion") };

    // The round-tripped palette carries the same semantic combos.
    let root = omt_nbt::read(&w.chunk_nbt, ByteOrder::Big).unwrap().tag;
    let palette = root
        .get("sections").and_then(Tag::as_list).unwrap()[0]
        .get("block_states").expect("block_states")
        .get("palette").and_then(Tag::as_list).unwrap()
        .iter()
        .map(|t| {
            let name = t.get("Name").and_then(Tag::as_str).unwrap().to_string();
            let props: Vec<(String, String)> = t
                .get("Properties")
                .and_then(Tag::as_compound)
                .map(|c| {
                    let mut v: Vec<(String, String)> = c
                        .iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                        .collect();
                    v.sort();
                    v
                })
                .unwrap_or_default();
            (name, props)
        })
        .collect::<Vec<_>>();

    let stairs = palette
        .iter()
        .find(|(n, _)| n == "minecraft:oak_stairs")
        .expect("stairs back");
    assert_eq!(
        stairs.1,
        vec![
            ("facing".to_string(), "north".to_string()),
            ("half".to_string(), "bottom".to_string()),
            ("shape".to_string(), "straight".to_string()),
            ("waterlogged".to_string(), "false".to_string()),
        ]
    );
}
