//! Fixture-gated end-to-end prunes (`#[ignore]`; worlds are never in git).
//!
//! Java: `OMT_JAVA_FIXTURE` or `../../world_ref/java-26.3`.
//! Bedrock: `OMT_BEDROCK_NETHER_FIXTURE` — a world with explored Nether/End
//! (the overworld-only `world_ref` worlds cannot exercise dimension keys).
//! Each test copies the world into `target/` first; originals are only read.
//!
//! `cargo test -p omt-prune --release --test fixture_prune -- --ignored`

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use omt_java::RegionFile;
use omt_prune::{Layer, PruneWorld, Selection, Shape, TimeOp};

fn fixture(env: &str, fallback: &str) -> Option<PathBuf> {
    let p = std::env::var_os(env)
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(fallback));
    p.join("level.dat").is_file().then_some(p)
}

fn scratch(name: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/omt-prune-fixtures").join(name);
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

/// Raw (decompressed) payload of every chunk in every region of `dir`.
fn payloads(dir: &Path) -> std::collections::BTreeMap<(i32, i32), Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        let Some((rx, rz)) = RegionFile::coords_from_name(&p) else { continue };
        let bytes = std::fs::read(&p).unwrap();
        let region = RegionFile::from_bytes(&p, &bytes).unwrap();
        for (x, z) in region.chunk_coords(rx, rz) {
            if let Ok(raw) = region.read_raw(&bytes, x, z) {
                out.insert((x, z), raw);
            }
        }
    }
    out
}

#[test]
#[ignore = "needs a Java fixture world"]
fn java_prune_keeps_survivors_byte_identical() {
    let Some(src) = fixture("OMT_JAVA_FIXTURE", "../../../../world_ref/java-26.3") else {
        eprintln!("skipped: no Java fixture");
        return;
    };
    let work = scratch("java");
    omt_world::workspace::copy_dir(&src, &work).unwrap();
    let world = PruneWorld::open(&work).unwrap();
    let selection = Selection::new(vec![
        Layer::new(Shape::Polygon { points: vec![(-300.0, -300.0), (300.0, -300.0), (0.0, 300.0)] }),
        Layer::new(Shape::Time { op: TimeOp::Greater, ticks: 100 }),
    ]);
    let plan = world.plan(0, &selection).unwrap();
    assert!(!plan.selected.is_empty() && plan.kept() > 0, "fixture must exercise both sides");
    let doomed: HashSet<_> = plan.selected.iter().copied().collect();

    let PruneWorld::Java(jw) = &world else { panic!("java fixture") };
    let kinds = ["region", "entities", "poi"];
    let before: Vec<_> = kinds.iter().map(|k| payloads(&jw.dimension_subdir(0, k).unwrap())).collect();

    let report = world.execute(&plan).unwrap();
    assert_eq!(report.chunks_deleted, plan.selected.len());

    for (i, kind) in kinds.iter().enumerate() {
        let after = payloads(&jw.dimension_subdir(0, kind).unwrap());
        for (xz, raw) in &before[i] {
            match after.get(xz) {
                Some(now) => {
                    assert!(!doomed.contains(xz), "{kind} {xz:?} should be gone");
                    assert_eq!(now, raw, "{kind} {xz:?} changed");
                }
                None => assert!(doomed.contains(xz), "{kind} {xz:?} lost but not selected"),
            }
        }
    }
    // Re-planning finds nothing left to delete.
    let again = PruneWorld::open(&work).unwrap().plan(0, &selection).unwrap();
    assert!(again.selected.is_empty());
    assert_eq!(again.total, plan.kept());
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "needs a Bedrock fixture with an explored Nether"]
fn bedrock_prune_is_dimension_exact() {
    let Some(src) = fixture("OMT_BEDROCK_NETHER_FIXTURE", "../../../../test_worlds/bedrock-lostsurfur") else {
        eprintln!("skipped: set OMT_BEDROCK_NETHER_FIXTURE");
        return;
    };
    let work = scratch("bedrock");
    omt_world::workspace::copy_dir(&src, &work).unwrap();
    let world = PruneWorld::open(&work).unwrap();
    let counts = |w: &PruneWorld| -> Vec<usize> { (0..3).map(|d| w.scan(d, false).unwrap().len()).collect() };
    let before = counts(&world);
    assert!(before[1] > 0, "fixture needs nether chunks");

    // Delete EVERY overworld chunk whose x,z also exists in the nether: the
    // old raw-prefix delete wiped those nether columns too.
    let nether: HashSet<_> = world.scan(1, false).unwrap().iter().map(|c| (c.x, c.z)).collect();
    let mut plan = world.plan(0, &Selection::new(vec![Layer::new(Shape::Split { x: 0.0, z: 0.0, angle_deg: 0.0 })])).unwrap();
    plan.selected.retain(|xz| nether.contains(xz));
    assert!(!plan.selected.is_empty(), "need overlapping columns");
    let report = world.execute(&plan).unwrap();
    assert_eq!(report.chunks_deleted, plan.selected.len());

    let after = counts(&world);
    assert_eq!(after[0], before[0] - plan.selected.len());
    assert_eq!(after[1], before[1], "nether untouched by an overworld prune");
    assert_eq!(after[2], before[2], "end untouched");
    world.optimize().unwrap();
    drop(world);
    let reopened = PruneWorld::open(&work).unwrap();
    assert_eq!(counts(&reopened), after, "survives compaction + reopen");
    let _ = std::fs::remove_dir_all(&work);
}
