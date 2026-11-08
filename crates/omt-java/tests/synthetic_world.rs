//! Synthetic Java world: build a full world folder from scratch, then read
//! it back through the store — the Java-side M1 acceptance stand-in until a
//! real Java fixture world is provided (PLAN.md §8).

use std::path::Path;

use omt_java::{JavaWorld, RegionWriter};
use omt_nbt::{ByteOrder, Container, Nbt, Tag};
use omt_world::WorldStore;

fn level_dat() -> Vec<u8> {
    let nbt = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![(
            "Data".into(),
            Tag::Compound(vec![
                ("LevelName".into(), Tag::String("OMT Synthetic".into())),
                ("DataVersion".into(), Tag::Int(4440)),
                ("GameType".into(), Tag::Int(1)),
                ("SpawnX".into(), Tag::Int(0)),
                ("SpawnZ".into(), Tag::Int(0)),
            ]),
        )]),
    };
    Container::Gzip.encode(&omt_nbt::write(&nbt, ByteOrder::Big).unwrap())
}

fn chunk_nbt(x: i32, z: i32, y: i8, block: &str) -> Vec<u8> {
    let nbt = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![
            ("DataVersion".into(), Tag::Int(4440)),
            ("xPos".into(), Tag::Int(x)),
            ("zPos".into(), Tag::Int(z)),
            ("Status".into(), Tag::String("minecraft:full".into())),
            ("InhabitedTime".into(), Tag::Long(1234)),
            ("sections".into(), Tag::List(vec![Tag::Compound(vec![
                ("Y".into(), Tag::Byte(y as i8)),
                ("block_states".into(), Tag::Compound(vec![
                    ("palette".into(), Tag::List(vec![
                        Tag::Compound(vec![("Name".into(), Tag::String(block.into()))]),
                        Tag::Compound(vec![
                            ("Name".into(), Tag::String("minecraft:air".into())),
                        ]),
                    ])),
                    ("data".into(), Tag::LongArray(vec![0, 0, 0, 0])), // all palette[0]
                ])),
                ("biomes".into(), Tag::Compound(vec![(
                    "palette".into(),
                    Tag::List(vec![Tag::String("minecraft:plains".into())]),
                )])),
            ])])),
            ("block_entities".into(), Tag::List(vec![Tag::Compound(vec![
                ("id".into(), Tag::String("minecraft:chest".into())),
                ("x".into(), Tag::Int(x * 16)),
                ("y".into(), Tag::Int(64)),
                ("z".into(), Tag::Int(z * 16)),
            ])])),
        ]),
    };
    omt_nbt::write(&nbt, ByteOrder::Big).unwrap()
}

fn build_world(dir: &Path) {
    std::fs::create_dir_all(dir.join("region")).unwrap();
    std::fs::create_dir_all(dir.join("playerdata")).unwrap();
    std::fs::write(dir.join("level.dat"), level_dat()).unwrap();
    std::fs::write(
        dir.join("levelname.txt"),
        "ignored by java, present for parity",
    )
    .unwrap();

    let mut writer = RegionWriter::new();
    writer.put(0, 0, chunk_nbt(0, 0, 0, "minecraft:stone"), 1);
    writer.put(1, 0, chunk_nbt(1, 0, 0, "minecraft:dirt"), 2);
    std::fs::write(dir.join("region/r.0.0.mca"), writer.finish()).unwrap();

    let mut writer = RegionWriter::new();
    writer.put(-1, -1, chunk_nbt(-1, -1, -4, "minecraft:deepslate"), 3);
    std::fs::write(dir.join("region/r.-1.-1.mca"), writer.finish()).unwrap();

    let player = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![(
            "Inventory".into(),
            Tag::List(vec![Tag::Compound(vec![
                ("id".into(), Tag::String("minecraft:diamond".into())),
                ("Count".into(), Tag::Byte(64)),
                ("Slot".into(), Tag::Byte(0)),
            ])]),
        )]),
    };
    let gz = Container::Gzip.encode(&omt_nbt::write(&player, ByteOrder::Big).unwrap());
    std::fs::write(dir.join("playerdata/00000000-0000-0000-0000-000000000001.dat"), gz).unwrap();
}

#[test]
fn synthetic_world_roundtrip() {
    let dir = std::env::temp_dir().join(format!("omt-java-synth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    build_world(&dir);

    // Detection (omt-world) recognizes it as Java.
    let detected = omt_world::detect(&dir).unwrap();
    assert!(matches!(detected, omt_world::Detected::JavaFolder { .. }));

    let world = JavaWorld::open(&dir).unwrap();
    assert_eq!(world.layout, omt_java::JavaLayout::Classic);
    assert_eq!(world.data_version(), Some(4440));
    assert_eq!(
        world.level_dat.tag.get("Data").and_then(|d| d.get("LevelName")).and_then(|t| t.as_str()),
        Some("OMT Synthetic")
    );

    let chunks = world.list_chunks(0).unwrap();
    assert_eq!(chunks, vec![(-1, -1), (0, 0), (1, 0)]);

    // Decoded chunk through the shared model.
    let chunk = world.read_chunk(0, -1, -1).unwrap().expect("present");
    assert_eq!(chunk.status.as_deref(), Some("minecraft:full"));
    assert_eq!(chunk.inhabited_time, 1234);
    let slice = chunk.slices.get(&-4).expect("section -4");
    assert_eq!(slice.palette[0].name, "minecraft:deepslate");
    assert_eq!(chunk.block_at(0, -64, 0).unwrap().name, "minecraft:deepslate");
    assert!(chunk.biomes_3d);
    assert_eq!(chunk.block_entities.len(), 1);

    // Player data.
    let players = world.list_players().unwrap();
    assert_eq!(players, vec!["00000000-0000-0000-0000-000000000001"]);
    let raw = world
        .read_raw(&omt_world::WorldNode::PlayerFile(players[0].clone()))
        .unwrap();
    let (payload, _) = Container::decode(&raw).unwrap();
    let nbt = omt_nbt::read(&payload, ByteOrder::Big).unwrap();
    assert_eq!(
        nbt.tag.get("Inventory").and_then(|t| t.as_list()).unwrap()[0]
            .get("id")
            .and_then(|t| t.as_str()),
        Some("minecraft:diamond")
    );

    // WorldStore trait objectivity: same API as Bedrock.
    let store = &world as &dyn WorldStore;
    assert_eq!(store.iter_chunks(0).unwrap().count(), 3);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn java_index_unpacking() {
    use omt_java::store::unpack_java_indices;
    // 2 entries → 4-bit packing… wait, vanilla uses bits=4 minimum; with a
    // palette of 2, data packs at 4 bits: 16 values per long.
    let packed = vec![0x1111111111111111i64]; // 16 × value 1
    let unpacked = unpack_java_indices(&packed, 2);
    assert_eq!(unpacked.len(), 4096);
    assert_eq!(unpacked[..16].to_vec(), vec![1u16; 16]);
    assert_eq!(unpacked[16], 0);
}

// ---------------------------------------------------------------------------
// 26.3-era namespaced layout (PLAN §0, 2026-11-08): dimensions/minecraft/
// <dim>/{region,entities,poi,data}, players/data/, and the new level.dat
// Data shape — plus the flattened block palette (List<String> and
// { "" | id, properties } entries; see omt_java::java_palette_entry).
// ---------------------------------------------------------------------------

fn level_dat_modern() -> Vec<u8> {
    let nbt = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![(
            "Data".into(),
            Tag::Compound(vec![
                ("LevelName".into(), Tag::String("OMT Synthetic 26".into())),
                ("DataVersion".into(), Tag::Int(5023)),
                ("GameType".into(), Tag::Int(0)),
                ("Time".into(), Tag::Long(42)),
                ("difficulty_settings".into(), Tag::Compound(vec![
                    ("difficulty".into(), Tag::String("normal".into())),
                    ("hardcore".into(), Tag::Byte(0)),
                    ("locked".into(), Tag::Byte(0)),
                ])),
                ("spawn".into(), Tag::Compound(vec![
                    ("pos".into(), Tag::IntArray(vec![16, 100, -32])),
                    ("pitch".into(), Tag::Float(0.0)),
                    ("yaw".into(), Tag::Float(0.0)),
                    ("dimension".into(), Tag::String("minecraft:overworld".into())),
                ])),
                ("version_history".into(), Tag::List(vec![Tag::Int(5023)])),
                ("singleplayer_uuid".into(), Tag::IntArray(vec![1, 2, 3, 4])),
                ("WasModded".into(), Tag::Byte(0)),
                ("initialized".into(), Tag::Byte(1)),
            ]),
        )]),
    };
    Container::Gzip.encode(&omt_nbt::write(&nbt, ByteOrder::Big).unwrap())
}

/// Chunk using the two NEW 26.3 palette entry shapes: one section with a
/// plain-string palette (default-state blocks), one with compound entries
/// carrying `id` + `properties` and an empty-named property-less entry.
fn chunk_nbt_modern(x: i32, z: i32) -> Vec<u8> {
    let nbt = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![
            ("DataVersion".into(), Tag::Int(5023)),
            ("xPos".into(), Tag::Int(x)),
            ("zPos".into(), Tag::Int(z)),
            ("Status".into(), Tag::String("minecraft:full".into())),
            ("InhabitedTime".into(), Tag::Long(7)),
            ("isLightOn".into(), Tag::Byte(1)),
            ("yPos".into(), Tag::Int(-4)),
            ("sections".into(), Tag::List(vec![
                // String palette (26.3): bare block names, no data (≤1 entry).
                Tag::Compound(vec![
                    ("Y".into(), Tag::Byte(-4)),
                    ("block_states".into(), Tag::Compound(vec![
                        ("palette".into(), Tag::List(vec![
                            Tag::String("minecraft:bedrock".into()),
                        ])),
                    ])),
                ]),
                // Compound palette (26.3): { "" } and { id, properties }.
                Tag::Compound(vec![
                    ("Y".into(), Tag::Byte(0)),
                    ("block_states".into(), Tag::Compound(vec![
                        ("palette".into(), Tag::List(vec![
                            Tag::Compound(vec![
                                ("".into(), Tag::String("minecraft:stone".into())),
                            ]),
                            Tag::Compound(vec![
                                ("id".into(), Tag::String("minecraft:oak_stairs".into())),
                                ("properties".into(), Tag::Compound(vec![
                                    ("facing".into(), Tag::String("north".into())),
                                    ("waterlogged".into(), Tag::String("false".into())),
                                ])),
                            ]),
                        ])),
                        ("data".into(), Tag::LongArray(vec![0, 0, 0, 0])),
                    ])),
                ]),
            ])),
        ]),
    };
    omt_nbt::write(&nbt, ByteOrder::Big).unwrap()
}

fn build_world_modern(dir: &Path) {
    let dim = dir.join("dimensions").join("minecraft").join("overworld");
    std::fs::create_dir_all(dim.join("region")).unwrap();
    std::fs::create_dir_all(dir.join("players").join("data")).unwrap();
    std::fs::write(dir.join("level.dat"), level_dat_modern()).unwrap();

    let mut writer = RegionWriter::new();
    writer.put(0, 0, chunk_nbt_modern(0, 0), 1);
    std::fs::write(dim.join("region/r.0.0.mca"), writer.finish()).unwrap();

    let player = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![(
            "Inventory".into(),
            Tag::List(vec![Tag::Compound(vec![
                ("id".into(), Tag::String("minecraft:emerald".into())),
                ("Count".into(), Tag::Byte(5)),
                ("Slot".into(), Tag::Byte(1)),
            ])]),
        )]),
    };
    let gz = Container::Gzip.encode(&omt_nbt::write(&player, ByteOrder::Big).unwrap());
    std::fs::write(
        dir.join("players/data/00000000-0000-0000-0000-000000000002.dat"),
        gz,
    )
    .unwrap();
}

#[test]
fn synthetic_world_26_namespaced_roundtrip() {
    let dir = std::env::temp_dir().join(format!("omt-java-synth26-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    build_world_modern(&dir);

    let detected = omt_world::detect(&dir).unwrap();
    assert!(matches!(detected, omt_world::Detected::JavaFolder { .. }));

    let world = JavaWorld::open(&dir).unwrap();
    assert_eq!(world.layout, omt_java::JavaLayout::Namespaced);
    assert_eq!(world.data_version(), Some(5023));

    // New level.dat Data shape through the era-aware spawn helper.
    let spawn = omt_world::java_spawn_xz(world.data()).expect("modern spawn");
    assert_eq!(spawn, (16, -32));

    // Dimension dirs resolve into the namespaced tree.
    let dim_dir = world.dimension_dir(0).expect("overworld dir");
    assert!(dim_dir.ends_with("dimensions/minecraft/overworld/region"));
    // Unexplored dimensions resolve to the (missing) namespaced path.
    let nether = world.dimension_dir(1).expect("nether dir");
    assert!(nether.ends_with("dimensions/minecraft/the_nether/region"));
    assert!(!nether.is_dir());
    assert_eq!(world.list_chunks(1).unwrap(), Vec::<(i32, i32)>::new());

    let chunks = world.list_chunks(0).unwrap();
    assert_eq!(chunks, vec![(0, 0)]);

    // Both new palette entry shapes decode with properties intact.
    let chunk = world.read_chunk(0, 0, 0).unwrap().expect("present");
    let bedrock = chunk.slices.get(&-4).expect("section -4");
    assert_eq!(bedrock.palette[0].name, "minecraft:bedrock");
    assert!(bedrock.palette[0].properties.is_empty());
    let surface = chunk.slices.get(&0).expect("section 0");
    assert_eq!(surface.palette[0].name, "minecraft:stone"); // { "" } shape
    let stairs = &surface.palette[1]; // { id, properties } shape
    assert_eq!(stairs.name, "minecraft:oak_stairs");
    assert_eq!(stairs.get("facing"), Some("north"));
    assert_eq!(stairs.get("waterlogged"), Some("false"));

    // Players live under players/data on this layout.
    let players = world.list_players().unwrap();
    assert_eq!(players, vec!["00000000-0000-0000-0000-000000000002"]);
    let raw = world
        .read_raw(&omt_world::WorldNode::PlayerFile(players[0].clone()))
        .unwrap();
    let (payload, _) = Container::decode(&raw).unwrap();
    let nbt = omt_nbt::read(&payload, ByteOrder::Big).unwrap();
    assert_eq!(
        nbt.tag.get("Inventory").and_then(|t| t.as_list()).unwrap()[0]
            .get("id")
            .and_then(|t| t.as_str()),
        Some("minecraft:emerald")
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn palette_entry_shapes() {
    use omt_java::java_palette_entry;
    // Legacy compound.
    let legacy = Tag::Compound(vec![
        ("Name".into(), Tag::String("minecraft:oak_fence".into())),
        ("Properties".into(), Tag::Compound(vec![
            ("north".into(), Tag::String("true".into())),
        ])),
    ]);
    let b = java_palette_entry(&legacy).unwrap();
    assert_eq!(b.name, "minecraft:oak_fence");
    assert_eq!(b.get("north"), Some("true"));
    // 26.3 string entry.
    let b = java_palette_entry(&Tag::String("minecraft:air".into())).unwrap();
    assert_eq!(b.name, "minecraft:air");
    // 26.3 { "" } and { id, properties }.
    let bare = Tag::Compound(vec![("".into(), Tag::String("minecraft:dirt".into()))]);
    assert_eq!(java_palette_entry(&bare).unwrap().name, "minecraft:dirt");
    let proped = Tag::Compound(vec![
        ("id".into(), Tag::String("minecraft:vine".into())),
        ("properties".into(), Tag::Compound(vec![
            ("south".into(), Tag::String("true".into())),
        ])),
    ]);
    let b = java_palette_entry(&proped).unwrap();
    assert_eq!(b.name, "minecraft:vine");
    assert_eq!(b.get("south"), Some("true"));
    // Malformed → None (caller decides default vs typed error).
    assert!(java_palette_entry(&Tag::Int(1)).is_none());
    assert!(java_palette_entry(&Tag::Compound(vec![])).is_none());
}
