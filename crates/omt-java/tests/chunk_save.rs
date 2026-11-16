//! "Chunk save" battle-tests: editing one chunk through the exact paths the
//! GUI's NBT editor uses must leave every *other* record byte-identical, and
//! a no-op round-trip must not change the edited record's bytes either.
//!
//! Java: the editor rewrites the region file with [`RegionFile::with_chunk`].
//! Bedrock: the editor rewrites one LevelDB value via `raw_put_many` with the
//! re-serialized NBT (`NbtDocument`/`sink_bytes` semantics).
//!
//! `cargo test -p omt-java --release --test chunk_save -- --ignored`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use omt_java::RegionFile;
use omt_nbt::{ByteOrder, Nbt};

fn fixture(env: &str, fallback: &str) -> Option<PathBuf> {
    let p = std::env::var_os(env).map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(fallback));
    p.join("level.dat").is_file().then_some(p)
}

fn scratch(name: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/chunk-save-fixtures").join(name);
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

/// Raw payloads of every region in a dir (path → (x,z) → raw chunk bytes).
fn all_regions(dir: &Path) -> BTreeMap<PathBuf, BTreeMap<(i32, i32), Vec<u8>>> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        let Some((rx, rz)) = RegionFile::coords_from_name(&p) else { continue };
        let bytes = std::fs::read(&p).unwrap();
        let region = RegionFile::from_bytes(&p, &bytes).unwrap();
        let mut chunks = BTreeMap::new();
        for (x, z) in region.chunk_coords(rx, rz) {
            if let Ok(raw) = region.read_raw(&bytes, x, z) {
                chunks.insert((x, z), raw);
            }
        }
        out.insert(p, chunks);
    }
    out
}

#[test]
#[ignore = "needs a Java fixture world"]
fn java_chunk_resave_only_touches_the_edited_chunk() {
    let Some(src) = fixture("OMT_JAVA_FIXTURE", "../../../../world_ref/java-26.3") else {
        eprintln!("skipped: no Java fixture");
        return;
    };
    let work = scratch("java");
    omt_world::workspace::copy_dir(&src, &work).unwrap();
    let region_dir = work.join("dimensions/minecraft/overworld/region");
    let before = all_regions(&region_dir);

    // Pick a real chunk, run it through the editor's exact save path:
    // decode → NbtDocument (format-preserving) → serialize → with_chunk.
    let (target_path, target_xz, target_raw) = {
        let (p, chunks) = before.iter().next().unwrap();
        let (&xz, raw) = chunks.iter().next().unwrap();
        (p.clone(), xz, raw.clone())
    };
    let bytes = std::fs::read(&target_path).unwrap();
    let region = RegionFile::from_bytes(&target_path, &bytes).unwrap();
    let nbt: Nbt = omt_nbt::read_prefix(&target_raw, ByteOrder::Big).unwrap().0;
    // One real, undoable edit: bump a top-level int if present, else add one.
    let edited = match &nbt.tag {
        omt_nbt::Tag::Compound(entries) => {
            let mut entries = entries.clone();
            match entries.iter_mut().find(|(k, _)| k == "LastUpdate") {
                Some((_, omt_nbt::Tag::Int(v))) => *v = v.wrapping_add(1),
                _ => entries.push(("LastUpdate".into(), omt_nbt::Tag::Int(42))),
            }
            Nbt { name: nbt.name.clone(), tag: omt_nbt::Tag::Compound(entries) }
        }
        other => Nbt { name: nbt.name.clone(), tag: other.clone() },
    };
    let payload = {
        let mut buf = Vec::new();
        // The editor writes the chunk NBT exactly as `omt_nbt::write` does
        // (see `nbt_session::write_pending`).
        buf.extend(omt_nbt::write(&edited, ByteOrder::Big).unwrap());
        buf
    };
    let now = 1;
    let image = region.with_chunk(&bytes, target_xz.0, target_xz.1, &payload, now);
    let tmp = target_path.with_extension("mca.omt_tmp");
    std::fs::write(&tmp, &image).unwrap();
    std::fs::rename(&tmp, &target_path).unwrap();

    let after = all_regions(&region_dir);
    assert_eq!(after.len(), before.len(), "no region files added/removed");
    for (path, chunks) in &before {
        let now_chunks = &after[path];
        assert_eq!(now_chunks.len(), chunks.len(), "chunk count changed in {}", path.display());
        for (xz, raw) in chunks {
            let now_raw = &now_chunks[xz];
            if path == &target_path && xz == &target_xz {
                // The edited chunk must decode to the edited NBT.
                let decoded: Nbt = omt_nbt::read_prefix(now_raw, ByteOrder::Big).unwrap().0;
                assert_eq!(decoded, edited, "edited chunk round-trips");
            } else {
                assert_eq!(now_raw, raw, "chunk {xz:?} in {} mutated by a neighbor's save", path.display());
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
}
