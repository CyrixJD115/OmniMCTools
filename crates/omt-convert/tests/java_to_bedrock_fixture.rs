//! Fixture-gated end-to-end: convert a synthetic Java world into a **copy**
//! of a real 1.26.52 Bedrock world, reopen it, and verify the converted
//! columns decode exactly — plus that untouched fixture chunks are
//! byte-identical afterwards.
//!
//! Run with: `cargo test -p omt-convert --test java_to_bedrock_fixture -- --ignored`
//! Fixture path: `OMT_BEDROCK_FIXTURE` env or `../../world_ref/e1j7l0I-l68=`
//! (never committed; see .gitignore).

use std::path::{Path, PathBuf};

use omt_bedrock::keys;
use omt_java::RegionWriter;
use omt_nbt::{ByteOrder, Container, Nbt, Tag};
use omt_world::WorldStore;

fn fixture_dir() -> PathBuf {
    std::env::var_os("OMT_BEDROCK_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../world_ref/e1j7l0I-l68=")
        })
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn level_dat() -> Vec<u8> {
    let nbt = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![(
            "Data".into(),
            Tag::Compound(vec![
                ("LevelName".into(), Tag::String("OMT Convert Fixture".into())),
                ("DataVersion".into(), Tag::Int(4440)),
                ("SpawnX".into(), Tag::Int(0)),
                ("SpawnZ".into(), Tag::Int(0)),
            ]),
        )]),
    };
    Container::Gzip.encode(&omt_nbt::write(&nbt, ByteOrder::Big).unwrap())
}

/// One uniform-block section at `y` + one chest block entity + plains.
fn chunk_nbt(x: i32, z: i32, y: i8, block: &str, status: &str) -> Vec<u8> {
    let nbt = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![
            ("DataVersion".into(), Tag::Int(4440)),
            ("xPos".into(), Tag::Int(x)),
            ("zPos".into(), Tag::Int(z)),
            ("Status".into(), Tag::String(status.into())),
            ("InhabitedTime".into(), Tag::Long(7)),
            ("sections".into(), Tag::List(vec![Tag::Compound(vec![
                ("Y".into(), Tag::Byte(y)),
                ("block_states".into(), Tag::Compound(vec![
                    ("palette".into(), Tag::List(vec![
                        Tag::Compound(vec![("Name".into(), Tag::String(block.into()))]),
                    ])),
                ])),
                ("biomes".into(), Tag::Compound(vec![(
                    "palette".into(),
                    Tag::List(vec![Tag::String("minecraft:plains".into())]),
                )])),
            ])])),
            ("block_entities".into(), Tag::List(vec![Tag::Compound(vec![
                ("id".into(), Tag::String("minecraft:chest".into())),
                ("x".into(), Tag::Int(x * 16)),
                ("y".into(), Tag::Int(y as i32 * 16)),
                ("z".into(), Tag::Int(z * 16)),
            ])])),
        ]),
    };
    omt_nbt::write(&nbt, ByteOrder::Big).unwrap()
}

fn build_java_world(dir: &Path) {
    std::fs::create_dir_all(dir.join("region")).unwrap();
    std::fs::write(dir.join("level.dat"), level_dat()).unwrap();

    let mut writer = RegionWriter::new();
    writer.put(0, 0, chunk_nbt(0, 0, 0, "minecraft:stone", "minecraft:full"), 1);
    writer.put(1, 0, chunk_nbt(1, 0, 0, "minecraft:dirt", "minecraft:full"), 2);
    writer.put(-1, -1, chunk_nbt(-1, -1, -4, "minecraft:deepslate", "minecraft:full"), 3);
    // Proto chunk: must be skipped by the default filter (UMT parity).
    writer.put(-2, 0, chunk_nbt(-2, 0, 0, "minecraft:stone", "minecraft:empty"), 4);
    std::fs::write(dir.join("region/r.0.0.mca"), writer.finish()).unwrap();
}

#[test]
#[ignore = "requires the local 1.26.52 fixture world (world_ref/)"]
fn converts_synthetic_java_world_into_fixture_copy() {
    let fixture = fixture_dir();
    assert!(
        fixture.join("db").is_dir(),
        "fixture world not found at {} (set OMT_BEDROCK_FIXTURE)",
        fixture.display()
    );

    let tmp = std::env::temp_dir().join(format!("omt-convert-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let java_dir = tmp.join("java");
    let bedrock_dir = tmp.join("bedrock");
    build_java_world(&java_dir);
    copy_dir(&fixture, &bedrock_dir).expect("copy fixture world");

    // Snapshot two untouched fixture columns' full key/value sets so we can
    // prove the pipeline wrote nothing outside its own coordinates.
    let probe_world = omt_bedrock::BedrockWorld::open_read_only(&bedrock_dir).unwrap();
    let fixture_chunks = probe_world.list_chunks(0).unwrap();
    assert!(!fixture_chunks.is_empty(), "fixture has no overworld chunks");
    let watched: Vec<(i32, i32)> = fixture_chunks.iter().take(2).copied().collect();
    let mut before: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for &(x, z) in &watched {
        for y in -4i8..=19 {
            if let Some(v) = probe_world.raw_get(&keys::subchunk_key(x, z, 0, y)).unwrap() {
                before.push((keys::subchunk_key(x, z, 0, y), v));
            }
        }
    }
    drop(probe_world);

    // Convert + apply.
    let java = omt_java::JavaWorld::open(&java_dir).unwrap();
    let opts = omt_convert::ConvertOptions::default();
    let mut outcome = omt_convert::convert_java_chunks(&java, 0, &opts).unwrap();
    assert_eq!(outcome.converted.len(), 3, "three full chunks convert: {outcome:?}");
    assert_eq!(outcome.skipped_proto, 1, "the empty-status chunk is skipped");
    assert!(outcome.failed.is_empty(), "no failures: {:?}", outcome.failed);

    let mut target = omt_bedrock::BedrockWorld::open_writable(&bedrock_dir).unwrap();
    let applied = omt_convert::apply_conversions(&target, 0, &mut outcome).unwrap();
    assert_eq!(applied, 3);
    target.flush().unwrap();
    drop(target);

    // Reopen read-only and verify the converted columns.
    let check = omt_bedrock::BedrockWorld::open_read_only(&bedrock_dir).unwrap();
    let chunk = check.read_chunk_full(0, 0, 0).unwrap().expect("converted column present");
    // Uniform stone section at Y 0..15 — corners and center all stone. This
    // exercises the write-side YZX→XZY permutation AND the read-side
    // XZY→YZX normalization through the universal model.
    for &(x, y, z) in &[(0usize, 0i32, 0usize), (15, 15, 15), (7, 3, 9), (0, 15, 15)] {
        let block = chunk.block_at(x, y, z).expect("block in converted section");
        assert_eq!(block.name, "minecraft:stone", "at ({x},{y},{z})");
    }
    // Heightmap was recomputed (no Java heightmaps): top stone at y=15 →
    // value 15 + 65 = 80.
    let (_, hm) = chunk.heightmaps.first().expect("heightmap");
    assert_eq!(hm[0], 80);
    // Uniform plains biome in the section's slot (slot-space y=64).
    let biomes = chunk.bedrock_biomes.as_ref().expect("bedrock biomes");
    assert_eq!(biomes[4], omt_world::BedrockBiomeSlot::uniform(1));
    // Block entity survived as little-endian NBT.
    assert_eq!(chunk.block_entities.len(), 1);
    assert_eq!(
        chunk.block_entities[0].get("id").and_then(Tag::as_str),
        Some("minecraft:chest")
    );

    // Deepslate column at slot −4 (world Y −64..−49).
    let deep = check.read_chunk_full(0, -1, -1).unwrap().expect("deepslate column");
    assert_eq!(deep.block_at(3, -64, 5).unwrap().name, "minecraft:deepslate");

    // The skipped proto chunk was not converted or written: it is absent
    // from the converted set, and a column that exists nowhere (neither in
    // the fixture nor converted) stays absent.
    assert!(!outcome.converted.contains(&(-2, 0)));
    assert!(check.read_chunk_full(0, 100_000, 100_000).unwrap().is_none());

    // Fixture columns are untouched, byte for byte.
    let mut after: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for &(x, z) in &watched {
        for y in -4i8..=19 {
            if let Some(v) = check.raw_get(&keys::subchunk_key(x, z, 0, y)).unwrap() {
                after.push((keys::subchunk_key(x, z, 0, y), v));
            }
        }
    }
    assert_eq!(before, after, "untouched fixture columns must be byte-identical");

    std::fs::remove_dir_all(&tmp).unwrap();
}
