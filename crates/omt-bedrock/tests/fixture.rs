//! Fixture-driven integration tests. Set `OMT_BEDROCK_FIXTURE` to a Bedrock
//! world folder (e.g. one of the `world_ref` worlds); tests are skipped
//! otherwise so CI without fixtures stays green.

use omt_bedrock::{BedrockWorld, keys};

fn fixture() -> Option<std::path::PathBuf> {
    std::env::var_os("OMT_BEDROCK_FIXTURE").map(std::path::PathBuf::from)
}

#[test]
fn opens_and_lists_overworld_chunks() {
    let Some(world) = fixture() else { eprintln!("skipped: OMT_BEDROCK_FIXTURE not set"); return };
    let w = BedrockWorld::open_read_only(&world).expect("open");
    let chunks = w.list_chunks(0).expect("list");
    assert!(chunks.len() > 100, "expected a real world, got {} columns", chunks.len());
    eprintln!("{} columns in overworld", chunks.len());
}

#[test]
fn decodes_chunk_zero_completely() {
    let Some(world) = fixture() else { eprintln!("skipped: OMT_BEDROCK_FIXTURE not set"); return };
    let w = BedrockWorld::open_read_only(&world).expect("open");
    let chunks = w.list_chunks(0).expect("list");
    let &(x, z) = chunks.first().expect("non-empty");
    let chunk = w.read_chunk_full(0, x, z).expect("read").expect("present");
    assert!(!chunk.slices.is_empty(), "chunk ({x},{z}) has no slices");
    for (y, slice) in &chunk.slices {
        assert_eq!(slice.indices.len(), 4096, "slice {y}");
        assert!(!slice.palette.is_empty(), "slice {y} empty palette");
        for state in &slice.palette {
            assert!(
                state.name.contains(':'),
                "suspicious block name {:?} in slice {y}",
                state.name
            );
        }
    }
    assert!(chunk.bedrock_biomes.is_some(), "data3D biomes missing");
    assert!(!chunk.bedrock_biomes.as_ref().unwrap().is_empty());
    eprintln!(
        "chunk ({x},{z}): {} slices, {} biome slots, {} block entities, {} actors, palette sample: {:?}",
        chunk.slices.len(),
        chunk.bedrock_biomes.as_ref().unwrap().len(),
        chunk.block_entities.len(),
        chunk.entities.len(),
        chunk.slices.values().next().unwrap().palette.first().map(|b| b.name.clone())
    );
}

/// The stress test: decode EVERY subchunk in the overworld. Any malformed
/// record fails the run — this is the M1 acceptance bar.
#[test]
fn decodes_every_overworld_subchunk() {
    let Some(world) = fixture() else { eprintln!("skipped: OMT_BEDROCK_FIXTURE not set"); return };
    let w = BedrockWorld::open_read_only(&world).expect("open");
    let chunks = w.list_chunks(0).expect("list");
    let mut subchunks = 0usize;
    let mut blocks_total = 0u64;
    for &(x, z) in &chunks {
        let chunk = w
            .read_chunk_full(0, x, z)
            .expect("chunk decodes")
            .unwrap_or_else(|| panic!("listed chunk ({x},{z}) vanished"));
        for (y, slice) in &chunk.slices {
            let max = slice.indices.iter().copied().max().unwrap_or(0) as usize;
            assert!(
                max < slice.palette.len(),
                "index {max} out of palette ({}) at ({x},{z}) slice {y}",
                slice.palette.len()
            );
            subchunks += 1;
            blocks_total += 4096;
        }
    }
    eprintln!("{subchunks} subchunks, {blocks_total} blocks decoded with zero errors");
    assert!(subchunks > chunks.len(), "expected multiple slices per column");
}

#[test]
fn digp_and_actors_decode_as_nbt() {
    let Some(world) = fixture() else { eprintln!("skipped: OMT_BEDROCK_FIXTURE not set"); return };
    let w = BedrockWorld::open_read_only(&world).expect("open");
    let mut actors = 0usize;
    for dim in 0u8..3 {
        for &(x, z) in &w.list_chunks(dim).expect("list") {
            if let Some(digp) = w.raw_get(&keys::digp_key(x, z, dim)).expect("get") {
                assert_eq!(digp.len() % 8, 0, "digp ({x},{z}) dim {dim} not a list of 8-byte ids");
                for id in digp.chunks_exact(8) {
                    let mut key = b"actorprefix".to_vec();
                    key.extend_from_slice(id);
                    if let Some(raw) = w.raw_get(&key).expect("get") {
                        let nbt = omt_bedrock::store::parse_actor_nbt(&raw).expect("actor NBT");
                        let has_id = nbt
                            .tag
                            .get("identifier")
                            .and_then(|t| t.as_str())
                            .is_some();
                        assert!(has_id, "actor without identifier: {}", omt_nbt::snbt::to_snbt(&nbt.tag).chars().take(80).collect::<String>());
                        actors += 1;
                    }
                }
            }
        }
    }
    eprintln!("{actors} actors decoded with identifiers");
}

#[test]
fn subchunk_reencode_is_byte_faithful() {
    let Some(world) = fixture() else { eprintln!("skipped: OMT_BEDROCK_FIXTURE not set"); return };
    let w = BedrockWorld::open_read_only(&world).expect("open");
    let snapshot_ok = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let checked = std::sync::Arc::clone(&snapshot_ok);
    for &(x, z) in &w.list_chunks(0).expect("list") {
        for y in -4i8..=19 {
            let key = keys::subchunk_key(x, z, 0, y);
            let Some(raw) = w.raw_get(&key).expect("get") else { continue };
            let sub = omt_bedrock::Subchunk::parse(&raw, Some(y)).expect("parse");
            // Round-trip must reproduce the exact bytes for untouched data.
            let re = sub.serialize();
            if re == raw {
                checked.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            } else {
                // Only acceptable when our bits-width normalization changed
                // (vanilla sometimes pads); flag everything for now.
                panic!(
                    "subchunk ({x},{z}) y={y} re-encode differs: {} vs {} bytes",
                    re.len(),
                    raw.len()
                );
            }
        }
    }
    eprintln!("{} subchunks re-encode byte-identically", checked.load(std::sync::atomic::Ordering::Relaxed));
}
