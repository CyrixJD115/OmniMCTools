//! "Chunk save" battle-test (Bedrock side): a no-op NBT decode→encode
//! through the editor's serialization must be **byte-faithful** for every
//! NBT-bearing LevelDB record — otherwise saving one key "mutates" others.
//!
//! `cargo test -p omt-bedrock --release --test chunk_save -- --ignored`

use std::collections::BTreeMap;
use std::path::PathBuf;

use omt_nbt::ByteOrder;

fn fixture(env: &str, fallback: &str) -> Option<PathBuf> {
    let p = std::env::var_os(env).map(PathBuf::from).unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(fallback));
    p.join("level.dat").is_file().then_some(p)
}

fn scratch(name: &str) -> PathBuf {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/chunk-save-fixtures").join(name);
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

#[test]
#[ignore = "needs the Bedrock test world"]
fn bedrock_nbt_resave_is_byte_faithful_on_noop_edits() {
    let Some(src) = fixture("OMT_BEDROCK_NETHER_FIXTURE", "../../../../test_worlds/bedrock-lostsurfur") else {
        eprintln!("skipped: set OMT_BEDROCK_NETHER_FIXTURE");
        return;
    };
    let work = scratch("bedrock");
    omt_world::workspace::copy_dir(&src, &work).unwrap();
    let w = omt_bedrock::BedrockWorld::open_writable(&work).unwrap();
    let before: BTreeMap<Vec<u8>, Vec<u8>> = w.iter_raw().unwrap().into_iter().collect();

    // Round-trip every NBT-bearing record the editor can open, through the
    // editor's own serialization: single LE roots (chunk-adjacent NBT keys)
    // and 0x31 block-entity sequences. A no-op decode→encode must return the
    // exact same bytes, or the editor's save would "mutate" untouched chunks.
    let mut checked_roots = 0usize;
    let mut checked_sequences = 0usize;
    let mut mismatches = 0usize;
    for (k, v) in &before {
        let tag = k.last().copied();
        // Sequences of LE roots (block entities 0x31, ticks 0x33/0x3A).
        if k.len() >= 9 && matches!(tag, Some(0x31) | Some(0x33) | Some(0x3A)) {
            let mut offset = 0usize;
            let mut rebuilt = Vec::new();
            let mut ok = true;
            while offset < v.len() {
                match omt_nbt::read_prefix_with_hints(&v[offset..], ByteOrder::Little) {
                    Ok((nbt, used, hints)) => {
                        rebuilt.extend(omt_nbt::write_with_list_hints(&nbt, ByteOrder::Little, &hints).unwrap());
                        offset += used;
                    }
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && offset == v.len() {
                checked_sequences += 1;
                if rebuilt != *v {
                    mismatches += 1;
                    if mismatches <= 3 {
                        eprintln!("sequence {:02x?} re-encodes differently ({} → {} bytes)", k.iter().map(|b| format!("{b:02x}")).collect::<String>(), v.len(), rebuilt.len());
                    }
                }
            }
            continue;
        }
        // Single LE root (e.g. ~local_player, village data…).
        if let Ok((nbt, used, hints)) = omt_nbt::read_prefix_with_hints(v, ByteOrder::Little)
            && used == v.len()
        {
            checked_roots += 1;
            let rebuilt = omt_nbt::write_with_list_hints(&nbt, ByteOrder::Little, &hints).unwrap();
            if rebuilt != *v {
                mismatches += 1;
                if mismatches <= 3 {
                    eprintln!("root {:02x?} re-encodes differently ({} → {} bytes)", k.iter().map(|b| format!("{b:02x}")).collect::<String>(), v.len(), rebuilt.len());
                }
            }
        }
    }
    eprintln!("checked {checked_roots} single roots, {checked_sequences} sequences, {mismatches} mismatches");
    assert!(checked_roots + checked_sequences > 5, "fixture must exercise real NBT records");
    assert_eq!(mismatches, 0, "no-op editor saves must be byte-faithful");
    let _ = std::fs::remove_dir_all(&work);
}
