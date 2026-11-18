//! Accuracy of the on-demand LOD tiles against the legacy whole-world render
//! (needs the test worlds under `UMT_Crack/test_worlds`; run with
//! `cargo test -p omt-render -- --ignored`).

use std::path::PathBuf;
use std::sync::Arc;

use omt_render::{MapSource, Palette, RenderOptions, TileKey, build_tile, render_world};

fn fixture(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../test_worlds").join(name);
    p.join("level.dat").is_file().then_some(p)
}

fn never() -> bool {
    false
}

/// Level-0 tile pixels == the matching crop of a whole-world render.
fn assert_l0_matches(src: &MapSource, store: &(dyn omt_world::WorldStore + Sync), key: TileKey) {
    let opts = RenderOptions::default();
    let tile = build_tile(src, key, None, Palette::embedded(), &opts, &never).unwrap();
    assert!(!tile.is_empty(), "tile {key:?} must contain chunks");
    assert_eq!(tile.px, 512);
    let (x0, z0, x1, z1) = key.chunk_rect();
    // Reference: legacy per-chunk store path, one chunk row of margin north
    // so the tile's first row is shaded against the same neighbors.
    let (image, rendered, errors) = render_world(store, 0, Some((x0, z0 - 1, x1, z1)), &opts).unwrap();
    assert!(rendered > 0 && errors == 0);
    let img = image.unwrap();
    let mut mismatch = 0usize;
    let mut compared = 0usize;
    for lz in 0..512usize {
        for lx in 0..512usize {
            let gx = x0 * 16 + lx as i32 - img.min_cx * 16;
            let gz = z0 * 16 + lz as i32 - img.min_cz * 16;
            let a = &tile.rgba[(lz * 512 + lx) * 4..(lz * 512 + lx) * 4 + 4];
            let b = if gx >= 0 && gz >= 0 && (gx as usize) < img.width && (gz as usize) < img.height {
                let i = (gz as usize * img.width + gx as usize) * 4;
                &img.rgba[i..i + 4]
            } else {
                &[0u8; 4][..]
            };
            compared += 1;
            if a != b {
                mismatch += 1;
            }
        }
    }
    assert_eq!(compared, 512 * 512);
    assert_eq!(mismatch, 0, "tile {key:?} diverges from the whole-world render");
}

/// A level-1 tile is the exact box average of its four level-0 children.
fn assert_l1_is_box_average(src: &MapSource, key1: TileKey) {
    let opts = RenderOptions::default();
    let p = Palette::embedded();
    let t1 = build_tile(src, key1, None, p, &opts, &never).unwrap();
    assert_eq!(t1.px, 512);
    let mut full = vec![0u8; 1024 * 1024 * 4];
    for dz in 0..2 {
        for dx in 0..2 {
            let k = TileKey::new(0, key1.tx * 2 + dx, key1.tz * 2 + dz);
            let t = build_tile(src, k, None, p, &opts, &never).unwrap();
            if t.is_empty() {
                continue;
            }
            for z in 0..512usize {
                let dst = ((dz as usize * 512 + z) * 1024 + dx as usize * 512) * 4;
                full[dst..dst + 2048].copy_from_slice(&t.rgba[z * 2048..z * 2048 + 2048]);
            }
        }
    }
    let mut mismatch = 0;
    for z in 0..512usize {
        for x in 0..512usize {
            let (mut acc, mut n) = ([0u32; 3], 0u32);
            for (sx, sz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let i = ((z * 2 + sz) * 1024 + x * 2 + sx) * 4;
                if full[i + 3] != 0 {
                    for c in 0..3 {
                        acc[c] += full[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let mean = |c: usize| (acc[c] + n / 2).checked_div(n).unwrap_or(0) as u8;
            let want = [mean(0), mean(1), mean(2), if n > 0 { 255 } else { 0 }];
            if t1.rgba[(z * 512 + x) * 4..(z * 512 + x) * 4 + 4] != want {
                mismatch += 1;
            }
        }
    }
    assert_eq!(mismatch, 0, "level-1 tile is not the box average of level 0");
}

#[test]
#[ignore = "needs the Bedrock test world"]
fn bedrock_tiles_are_exact() {
    let Some(path) = fixture("bedrock-lostsurfur") else { return };
    let world = Arc::new(omt_bedrock::BedrockWorld::open_read_only(&path).unwrap());
    let index = world.column_index().unwrap();
    let src = MapSource::bedrock(&world, &index, 0);
    assert_eq!(src.chunks().len(), world.list_chunks(0).unwrap().len(), "index agrees with list_chunks");
    // Tiles around the spawn, including negative coordinates.
    assert_l0_matches(&src, world.as_ref(), TileKey::new(0, 0, 0));
    assert_l0_matches(&src, world.as_ref(), TileKey::new(0, 0, -1));
    assert_l1_is_box_average(&src, TileKey::new(1, 0, -1));

    // Sampled tiles stay within the read budget and still draw something.
    let key = TileKey::new(3, 0, -1);
    let t = build_tile(&src, key, Some(1024), Palette::embedded(), &RenderOptions::default(), &never).unwrap();
    assert!(t.stride > 1 && t.px == (key.span() / t.stride) as u32);
    assert!(t.stats.chunks <= 1024 + 32, "read {} chunks", t.stats.chunks);
    assert!(t.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
    // Identify works on the sampled tile (approximate surface).
    assert!(t.heights.iter().any(|&h| h != i16::MIN));
}

#[test]
#[ignore = "needs the Java test world"]
fn java_tiles_are_exact() {
    let Some(path) = fixture("java-fantastic-islands") else { return };
    let world = Arc::new(omt_java::JavaWorld::open(&path).unwrap());
    let src = MapSource::java(&world, 0).unwrap();
    assert_l0_matches(&src, world.as_ref(), TileKey::new(0, -1, -1));
    assert_l0_matches(&src, world.as_ref(), TileKey::new(0, 0, 0));
    assert_l1_is_box_average(&src, TileKey::new(1, -1, -1));
}

#[test]
#[ignore = "needs the Bedrock test world"]
fn cancelled_and_unbound_sources_stop() {
    let Some(path) = fixture("bedrock-lostsurfur") else { return };
    let world = Arc::new(omt_bedrock::BedrockWorld::open_read_only(&path).unwrap());
    let index = world.column_index().unwrap();
    let src = MapSource::bedrock(&world, &index, 0);
    let opts = RenderOptions::default();
    assert!(build_tile(&src, TileKey::new(0, 0, 0), None, Palette::embedded(), &opts, &|| true).is_none(), "cancel stops the build");
    // The source never keeps the world alive (GUI Save needs the only ref).
    assert_eq!(Arc::strong_count(&world), 1);
    drop(world);
    assert!(!src.is_bound());
    assert!(build_tile(&src, TileKey::new(0, 0, 0), None, Palette::embedded(), &opts, &never).is_none());
}
