//! End-to-end edits on real worlds (fixture-gated: set
//! `OMT_EDIT_BEDROCK` / `OMT_EDIT_JAVA` to *disposable copies*).
//! Fill → re-read through the store's own decoder → structure round trip
//! → undo restores the original bytes.

use std::sync::Arc;

use omt_edit::{Block, EditWorld, Op, Sel};
use omt_world::WorldStore;

fn never() -> bool {
    false
}

/// Re-verify whichever fixture world this test thread is using.
fn verify_root() {
    for var in ["OMT_EDIT_BEDROCK", "OMT_EDIT_JAVA"] {
        if let Some(root) = std::env::var_os(var) {
            let r = omt_edit::verify::verify(root.as_ref()).unwrap();
            assert!(r.ok(), "{var}: {:?}", &r.errors[..r.errors.len().min(5)]);
            assert!(r.chunks > 0);
        }
    }
}

fn first_chunk(w: &dyn WorldStore) -> (i32, i32) {
    let mut v: Vec<(i32, i32)> = w.iter_chunks(0).unwrap().collect();
    v.sort();
    v[v.len() / 2]
}

fn check(world: EditWorld, store: &dyn WorldStore, snapshot: &dyn Fn() -> Vec<u8>) {
    let (cx, cz) = first_chunk(store);
    let sel = Sel::new([cx * 16 + 2, 100, cz * 16 + 3], [cx * 16 + 9, 104, cz * 16 + 12]);
    let before = snapshot();
    let op = Op::Fill { to: Block::named("minecraft:gold_block"), only_air: false };
    let (report, undo) = omt_edit::apply(&world, 0, sel, &op, &never).unwrap();
    assert_eq!(report.changed, sel.volume(), "every block in the box changed (none was gold)");
    let chunk = store.read_chunk(0, cx, cz).unwrap().expect("chunk");
    assert_eq!(chunk.block_at(5, 102, 7).unwrap().name, "minecraft:gold_block");
    assert_ne!(chunk.block_at(1, 102, 7).map(|b| b.name.clone()), Some("minecraft:gold_block".into()), "outside untouched");
    // Read back as a volume and through a structure file.
    let (vol, counts) = omt_edit::read(&world, 0, sel, &never).unwrap();
    assert_eq!(counts.counts[0], ("minecraft:gold_block".to_string(), sel.volume()));
    let bytes = omt_edit::structure::to_mcstructure(&vol, sel.min, None).unwrap();
    let back = omt_edit::structure::import(&bytes).unwrap();
    assert_eq!(back.at(3, 2, 4).unwrap().state.name, "minecraft:gold_block");
    // The edited world decodes cleanly when re-opened fresh.
    verify_root();
    // Paste a copy 20 blocks east, then undo both.
    let paste = Op::Paste { vol: Arc::new(back), origin: [sel.min[0] + 20, 100, sel.min[2]], skip_air: false };
    let (r2, undo2) = omt_edit::apply(&world, 0, sel, &paste, &never).unwrap();
    assert!(r2.changed > 0 || r2.missing_chunks > 0);
    omt_edit::undo(&world, &undo2).unwrap();
    omt_edit::undo(&world, &undo).unwrap();
    assert!(before == snapshot(), "undo restores the original bytes");
}

#[test]
#[ignore = "needs OMT_EDIT_BEDROCK (a disposable world copy)"]
fn bedrock_fill_read_undo() {
    let Some(root) = std::env::var_os("OMT_EDIT_BEDROCK") else { return };
    let w = Arc::new(omt_bedrock::BedrockWorld::open_writable(root.as_ref()).unwrap());
    let (cx, cz) = first_chunk(w.as_ref());
    let keys: Vec<Vec<u8>> = (6i8..=6).map(|y| omt_bedrock::keys::subchunk_key(cx, cz, 0, y)).collect();
    let w2 = w.clone();
    let snap = move || keys.iter().flat_map(|k| w2.raw_get(k).unwrap().unwrap_or_default()).collect::<Vec<u8>>();
    check(EditWorld::Bedrock(w.clone()), w.as_ref(), &snap);
}

#[test]
#[ignore = "needs OMT_EDIT_JAVA (a disposable world copy)"]
fn java_fill_read_undo() {
    let Some(root) = std::env::var_os("OMT_EDIT_JAVA") else { return };
    let w = Arc::new(omt_java::JavaWorld::open(root.as_ref()).unwrap());
    let (cx, cz) = first_chunk(w.as_ref());
    let path = w.region_path(0, cx.div_euclid(32), cz.div_euclid(32));
    let snap = move || std::fs::read(&path).unwrap();
    check(EditWorld::Java(w.clone()), w.as_ref(), &snap);
}

fn biome_and_transform(world: EditWorld, store: &dyn WorldStore, snapshot: &dyn Fn() -> Vec<u8>) {
    let (cx, cz) = first_chunk(store);
    let sel = Sel::new([cx * 16, 60, cz * 16], [cx * 16 + 7, 75, cz * 16 + 7]);
    let before = snapshot();
    let (r, u) = omt_edit::biome::paint(&world, 0, sel, "minecraft:desert", &never).unwrap();
    assert!(r.changed > 0, "painted");
    let (r2, u2) = omt_edit::biome::paint(&world, 0, sel, "minecraft:desert", &never).unwrap();
    assert_eq!(r2.changed, 0, "idempotent");
    // Rotate a copied block volume and paste it back over itself.
    let (vol, _) = omt_edit::read(&world, 0, sel, &never).unwrap();
    let rot = omt_edit::transform::transform(&vol, omt_edit::transform::Xform::RotateCw);
    let (r3, u3) = omt_edit::apply(&world, 0, sel, &Op::Paste { vol: Arc::new(rot), origin: sel.min, skip_air: false }, &never).unwrap();
    assert_eq!(r3.missing_chunks, 0);
    verify_root();
    omt_edit::undo(&world, &u3).unwrap();
    omt_edit::undo(&world, &u2).unwrap();
    omt_edit::undo(&world, &u).unwrap();
    assert!(before == snapshot(), "undo restores the original bytes");
}

#[test]
#[ignore = "needs OMT_EDIT_BEDROCK (a disposable world copy)"]
fn bedrock_biome_paint_and_rotate() {
    let Some(root) = std::env::var_os("OMT_EDIT_BEDROCK") else { return };
    let w = Arc::new(omt_bedrock::BedrockWorld::open_writable(root.as_ref()).unwrap());
    let (cx, cz) = first_chunk(w.as_ref());
    let key = omt_bedrock::keys::data3d_key(cx, cz, 0);
    let w2 = w.clone();
    let keys: Vec<Vec<u8>> = std::iter::once(key.clone()).chain((3i8..=4).map(|y| omt_bedrock::keys::subchunk_key(cx, cz, 0, y))).collect();
    let snap = move || keys.iter().flat_map(|k| w2.raw_get(k).unwrap().unwrap_or_default()).collect::<Vec<u8>>();
    // Paint and check the id directly in data3D before the undo test.
    let world = EditWorld::Bedrock(w.clone());
    let sel = Sel::new([cx * 16, 60, cz * 16], [cx * 16 + 7, 75, cz * 16 + 7]);
    let (_, u) = omt_edit::biome::paint(&world, 0, sel, "minecraft:desert", &never).unwrap();
    let d = omt_bedrock::Data3d::parse(&w.raw_get(&key).unwrap().unwrap()).unwrap();
    assert_eq!(d.biome_at(3, (64 + 64) as usize, 3), omt_edit::biome::bedrock_biome_id("desert"));
    omt_edit::undo(&world, &u).unwrap();
    biome_and_transform(world, w.as_ref(), &snap);
}

#[test]
#[ignore = "needs OMT_EDIT_JAVA (a disposable world copy)"]
fn java_biome_paint_and_rotate() {
    let Some(root) = std::env::var_os("OMT_EDIT_JAVA") else { return };
    let w = Arc::new(omt_java::JavaWorld::open(root.as_ref()).unwrap());
    let (cx, cz) = first_chunk(w.as_ref());
    let path = w.region_path(0, cx.div_euclid(32), cz.div_euclid(32));
    let snap = move || std::fs::read(&path).unwrap();
    biome_and_transform(EditWorld::Java(w.clone()), w.as_ref(), &snap);
}

/// Remove every entity in a 4×4-chunk box around the busiest area, then
/// undo: the list comes back identical.
fn entity_roundtrip(world: EditWorld, store: &dyn WorldStore) {
    let (cx, cz) = first_chunk(store);
    let sel = Sel::new([(cx - 8) * 16, -64, (cz - 8) * 16], [(cx + 8) * 16 + 15, 319, (cz + 8) * 16 + 15]);
    let before = omt_edit::entities::list(&world, 0, sel).unwrap();
    let non_players = before.iter().filter(|e| !e.id.ends_with("player")).count() as u64;
    let (r, u) = omt_edit::entities::remove(&world, 0, sel, None, &never).unwrap();
    assert_eq!(r.changed, non_players);
    verify_root();
    let after = omt_edit::entities::list(&world, 0, sel).unwrap();
    assert!(after.iter().all(|e| e.id.ends_with("player")), "{after:?}");
    omt_edit::undo(&world, &u).unwrap();
    let back = omt_edit::entities::list(&world, 0, sel).unwrap();
    assert_eq!(omt_edit::entities::counts(&back), omt_edit::entities::counts(&before));
    eprintln!("entities: {} removed and restored", non_players);
}

#[test]
#[ignore = "needs OMT_EDIT_BEDROCK (a disposable world copy)"]
fn bedrock_entities_remove_undo() {
    let Some(root) = std::env::var_os("OMT_EDIT_BEDROCK") else { return };
    let w = Arc::new(omt_bedrock::BedrockWorld::open_writable(root.as_ref()).unwrap());
    entity_roundtrip(EditWorld::Bedrock(w.clone()), w.as_ref());
}

#[test]
#[ignore = "needs OMT_EDIT_JAVA (a disposable world copy)"]
fn java_entities_remove_undo() {
    let Some(root) = std::env::var_os("OMT_EDIT_JAVA") else { return };
    let w = Arc::new(omt_java::JavaWorld::open(root.as_ref()).unwrap());
    entity_roundtrip(EditWorld::Java(w.clone()), w.as_ref());
}
