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
