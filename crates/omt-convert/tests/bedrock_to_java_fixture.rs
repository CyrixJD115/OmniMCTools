//! Fixture-gated end-to-end, reverse direction (M3b): convert a **copy** of
//! the real 1.26.52 Bedrock fixture world toward a synthetic Java world,
//! reopen it with `JavaWorld`, and verify blocks at absolute positions
//! through the two independent decode paths (Bedrock XZY→YZX store decode
//! vs Java pack→reader decode). Also proves the Bedrock source columns stay
//! byte-identical (the source is only ever opened read-only; conversion
//! writes land in the Java target).
//!
//! Run with: `cargo test -p omt-convert --test bedrock_to_java_fixture -- --ignored`
//! Fixture path: `OMT_BEDROCK_FIXTURE` env or `../../world_ref/e1j7l0I-l68=`
//! (never committed; see .gitignore).

use std::path::{Path, PathBuf};

use omt_convert::{ConvertOptions, apply_java_conversions, convert_bedrock_chunks};
use omt_bedrock::keys;
use omt_java::RegionFile;
use omt_nbt::{ByteOrder, Nbt, Tag};
use omt_world::{BedrockBiomeSlot, WorldStore};

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

/// Snapshot the raw subchunk records of a few columns (source-untouched proof).
fn snapshot_columns(world: &omt_bedrock::BedrockWorld, watched: &[(i32, i32)]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    for &(x, z) in watched {
        for y in -4i8..=19 {
            if let Some(v) = world.raw_get(&keys::subchunk_key(x, z, 0, y)).unwrap() {
                out.push((keys::subchunk_key(x, z, 0, y), v));
            }
        }
    }
    out
}

#[test]
#[ignore = "requires the local 1.26.52 fixture world (world_ref/)"]
fn converts_fixture_bedrock_world_to_java() {
    let fixture = fixture_dir();
    assert!(
        fixture.join("db").is_dir(),
        "fixture world not found at {} (set OMT_BEDROCK_FIXTURE)",
        fixture.display()
    );

    let tmp = std::env::temp_dir().join(format!("omt-convert-b2j-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let source_dir = tmp.join("bedrock-copy");
    let java_dir = tmp.join("java-target");
    eprintln!("[b2j-fixture] copying fixture world…");
    copy_dir(&fixture, &source_dir).expect("copy fixture world");

    // Inject one synthetic actor into the COPY (disposable) so the entity
    // path is exercised deterministically: append an 8-byte actor id to the
    // first chunk column's digp and write the actorprefix record.
    eprintln!("[b2j-fixture] probing copy (read-only open)…");
    let probe = omt_bedrock::BedrockWorld::open_read_only(&source_dir).unwrap();
    let chunks = probe.list_chunks(0).unwrap();
    assert!(!chunks.is_empty(), "fixture has no overworld chunks");
    let (px, pz) = chunks[0];
    drop(probe);

    let actor_id: [u8; 8] = [0xde, 0xad, 0xbe, 0xef, 0x00, 0x01, 0x02, 0x03];
    let actor = Nbt {
        name: String::new(),
        tag: Tag::Compound(vec![
            ("identifier".to_string(), Tag::String("minecraft:pig".into())),
            ("UniqueID".to_string(), Tag::Long(987654321)),
            ("Pos".to_string(), Tag::List(vec![Tag::Float(0.5), Tag::Float(64.0), Tag::Float(0.5)])),
        ]),
    };
    eprintln!("[b2j-fixture] injecting actor…");
    {
        let mut writer = omt_bedrock::BedrockWorld::open_writable(&source_dir).unwrap();
        let mut digp = writer.raw_get(&keys::digp_key(px, pz, 0)).unwrap().unwrap_or_default();
        digp.extend_from_slice(&actor_id);
        writer
            .raw_put_many(&[
                (keys::digp_key(px, pz, 0), digp),
                (keys::actor_key(actor_id), omt_nbt::write(&actor, ByteOrder::Little).unwrap()),
            ])
            .unwrap();
        writer.flush().unwrap();
    }

    // Watch two untouched columns for the byte-identical proof.
    let watched: Vec<(i32, i32)> = chunks.iter().rev().take(2).copied().collect();
    eprintln!("[b2j-fixture] snapshotting watched columns…");
    let before_world = omt_bedrock::BedrockWorld::open_read_only(&source_dir).unwrap();
    let before = snapshot_columns(&before_world, &watched);
    assert!(!before.is_empty());
    // Keep a full read of the probed column for the block comparisons below.
    let bedrock_probe = before_world.read_chunk_full(0, px, pz).unwrap().expect("probed column");
    drop(before_world);

    // Convert the whole overworld (read-only source) and apply to the Java
    // target.
    eprintln!("[b2j-fixture] converting (this is the slow phase, ~2057 columns)…");
    let source = omt_bedrock::BedrockWorld::open_read_only(&source_dir).unwrap();
    let mut outcome = convert_bedrock_chunks(&source, 0, &ConvertOptions::default()).unwrap();
    eprintln!("[b2j-fixture] conversion done, applying…");
    assert!(outcome.failed.is_empty(), "no failures: {:?}", &outcome.failed[..outcome.failed.len().min(5)]);
    assert_eq!(outcome.skipped_proto, 0, "Bedrock columns are all full");
    let applied =
        apply_java_conversions(&java_dir, 0, &mut outcome, source.levelname.as_deref()).unwrap();
    assert_eq!(applied, outcome.converted.len());
    println!(
        "converted {} columns; merged ledger: {}",
        applied,
        outcome.merged_report()
    );
    drop(source);

    // Reopen the Java target through the Java store.
    let java = omt_java::JavaWorld::open(&java_dir).unwrap();
    let java_chunks = java.list_chunks(0).unwrap();
    assert_eq!(java_chunks.len(), outcome.converted.len(), "every converted column is readable");

    // Absolute-position block equality across several sections: compare the
    // Bedrock-side universal decode against the Java-side decode. Stateless
    // blocks must match exactly; stateful ones match by name (properties
    // drop is the expected, reported loss).
    let slots: Vec<i32> = bedrock_probe
        .slices
        .keys()
        .copied()
        .map(|y| y as i32)
        .filter(|y| *y >= 0 && java_chunks.contains(&(px, pz)))
        .collect();
    assert!(!slots.is_empty());
    let java_probe = java.read_chunk(0, px, pz).unwrap().expect("converted probed column");
    let mut checked = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    for slot in &slots {
        let base = slot * 16;
        for &(x, iny, z) in &[
            (0usize, 0usize, 0usize),
            (15, 0, 15),
            (0, 15, 15),
            (15, 15, 0),
            (7, 7, 7),
            (3, 11, 5),
        ] {
            let wy = base + iny as i32;
            let bedrock_block = bedrock_probe.block_at(x, wy, z);
            let java_block = java_probe.block_at(x, wy, z);
            match (bedrock_block, java_block) {
                (Some(b), Some(j)) => {
                    // Stateless-only assertion: air and stateless blocks must
                    // be exact; stateful drops surface in the loss ledger.
                    if b.properties.is_empty() {
                        assert_eq!(
                            b.name, j.name,
                            "block mismatch at absolute ({},{},{}): bedrock={:?} java={:?}",
                            x, wy, z, b.name, j.name
                        );
                    } else {
                        assert_eq!(
                            b.name, j.name,
                            "stateful block name mismatch at ({},{},{}), properties dropped",
                            x, wy, z
                        );
                    }
                    checked += 1;
                }
                (None, None) => {}
                (a, b) => mismatches.push(format!(
                    "presence mismatch at ({x},{wy},{z}): bedrock={:?} java={:?}",
                    a.map(|s| &s.name),
                    b.map(|s| &s.name)
                )),
            }
        }
    }
    assert!(checked > 20, "expected a real sample, got {checked}");
    assert!(
        mismatches.is_empty(),
        "presence mismatches ({}): {:?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(5)]
    );

    // Heightmap: Java MOTION_BLOCKING unpacks to the Bedrock data3D values
    // (1:1 numeric copy per the probe fact).
    let (_, bedrock_hm) = bedrock_probe.heightmaps.first().expect("bedrock heightmap");
    let java_nbt = java.read_chunk_nbt(0, px, pz).unwrap().unwrap();
    let Tag::LongArray(packed) = java_nbt
        .tag
        .get("Heightmaps")
        .expect("java heightmaps")
        .get("MOTION_BLOCKING")
        .expect("MOTION_BLOCKING")
    else {
        panic!("java heightmap not a long array");
    };
    let java_hm = omt_world::packing::unpack_java_heightmap(packed);
    let expected: Vec<u16> = bedrock_hm.iter().take(256).map(|&v| v as u16).collect();
    assert_eq!(java_hm, expected, "heightmap must copy 1:1 from data3D");

    // Biomes: the first uniform/uniform-resolvable biome of the probed
    // column must appear under its inverse-mapped name in some Java section
    // (or be an explicit loss row when the id is unmapped).
    if let Some(biomes) = &bedrock_probe.bedrock_biomes {
        let first_storage = biomes.iter().find_map(|s| match s {
            BedrockBiomeSlot::Storage { palette, indices: None, .. } => Some(palette[0]),
            BedrockBiomeSlot::Storage { palette, .. } => Some(palette[0]),
            BedrockBiomeSlot::NonExistent => None,
        });
        if let Some(id) = first_storage {
            let rules = omt_convert::RegistryRules::default();
            let names = rules.biome_names();
            if let Some(name) = names.get(&id) {
                // Find the Java section whose biomes carry that name.
                let has_name = java_nbt
                    .tag
                    .get("sections")
                    .and_then(Tag::as_list)
                    .unwrap()
                    .iter()
                    .filter_map(|s| s.get("biomes"))
                    .any(|b| {
                        b.get("palette")
                            .and_then(Tag::as_list)
                            .is_some_and(|ps| ps.iter().any(|p| p.as_str() == Some(name.as_str())))
                    });
                assert!(
                    has_name || outcome.merged_report().losses.iter().any(|l| l.path.contains("biome")),
                    "uniform biome id {id} ({name}) should appear in some Java section"
                );
            }
        }
    }

    // Block entities passed through with equal counts.
    assert_eq!(
        java_probe.block_entities.len(),
        bedrock_probe.block_entities.len(),
        "block entity passthrough count"
    );

    // Entity region: the injected actor is present with the Java 1.17 root
    // shape and the identifier→id rename; its UUID-less state is a loss row.
    let (erx, erz) = (px.div_euclid(32), pz.div_euclid(32));
    let epath = java_dir.join("entities").join(format!("r.{erx}.{erz}.mca"));
    assert!(epath.is_file(), "entity region written at {}", epath.display());
    let eregion = RegionFile::open(&epath).unwrap();
    let ebytes = std::fs::read(&epath).unwrap();
    let eraw = eregion.read_raw(&ebytes, px, pz).unwrap();
    let eroot = omt_nbt::read(&eraw, ByteOrder::Big).unwrap().tag;
    assert_eq!(eroot.get("Position"), Some(&Tag::IntArray(vec![px, pz])));
    let entities = eroot.get("Entities").and_then(Tag::as_list).unwrap();
    let pig = entities
        .iter()
        .find(|e| e.get("id").and_then(Tag::as_str) == Some("minecraft:pig"))
        .expect("injected actor present under Java `id`");
    assert!(pig.get("identifier").is_none());
    assert!(pig.get("UUID").is_none(), "no UUID invented");
    assert!(
        outcome
            .merged_report()
            .losses
            .iter()
            .any(|l| l.detail.contains("no Java UUID")),
        "UUID-less actors are reported, not silently rewritten"
    );

    // The Bedrock source columns are byte-identical afterwards.
    let after_world = omt_bedrock::BedrockWorld::open_read_only(&source_dir).unwrap();
    let after = snapshot_columns(&after_world, &watched);
    assert_eq!(before, after, "untouched fixture columns must be byte-identical");

    std::fs::remove_dir_all(&tmp).unwrap();
}
