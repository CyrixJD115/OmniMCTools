//! Tests of `mapdata` (kept apart to respect the file-size budget).
use super::*;

#[test]
fn level_follows_zoom() {
    assert_eq!(level_for(4.0, 0), 0);
    assert_eq!(level_for(1.0, 0), 0);
    assert_eq!(level_for(0.5, 0), 1);
    assert_eq!(level_for(1.0 / 16.0, 0), 4);
    assert_eq!(level_for(1.0 / 16.0, 1), 3, "bias sharpens");
    assert_eq!(level_for(1.0 / 16.0, -1), 5, "bias coarsens");
    assert_eq!(level_for(1e-9, 0), MAX_LEVEL);
}

#[test]
fn tile_range_covers_viewport_and_handles_negatives() {
    // Level 0 tile = 32 chunks = 512 blocks.
    assert_eq!(tile_range(0, 0.0, 0.0, 511.9, 511.9), (0, 0, 0, 0));
    assert_eq!(tile_range(0, -1.0, -1.0, 10.0, 10.0), (-1, -1, 0, 0));
    assert_eq!(tile_range(0, -600.0, 300.0, -100.0, 700.0), (-2, 0, -1, 1));
    // Level 1 tile = 1024 blocks.
    assert_eq!(tile_range(1, -600.0, 300.0, 1100.0, 700.0), (-1, 0, 1, 0));
}

#[test]
fn quality_budgets_and_pressure() {
    assert_eq!(MapQuality::Auto.budget(Pressure::Normal), Some(4096));
    assert_eq!(MapQuality::Auto.budget(Pressure::Low), Some(1024), "auto degrades under pressure");
    assert_eq!(MapQuality::High.budget(Pressure::Critical), Some(16_384), "explicit presets are honored");
    assert_eq!(MapQuality::Full.budget(Pressure::Normal), None);
    let c = MapConfig { memory_mb: 512, ..Default::default() };
    assert_eq!(memory_budget(&c, Pressure::Normal, None), 512 << 20);
    assert_eq!(memory_budget(&c, Pressure::Low, None), 256 << 20);
    assert_eq!(memory_budget(&c, Pressure::Critical, None), 128 << 20);
}

fn image(key: TileKey, px: u32) -> TileImage {
    TileImage {
        key,
        stride: 1,
        px,
        rgba: vec![0; (px * px * 4) as usize],
        surface: vec![0; (px * px) as usize],
        heights: vec![64; (px * px) as usize],
        biomes: vec![u16::MAX; (px * px) as usize],
        names: vec!["minecraft:stone".into()],
        stats: Default::default(),
        millis: 0,
    }
}

fn src_with(chunks: &[(i32, i32)]) -> MapSource {
    struct Fake(omt_world::ChunkSet);
    impl omt_world::WorldStore for Fake {
        fn read_chunk(&self, _: u8, _: i32, _: i32) -> omt_world::WorldResult<Option<omt_world::Chunk>> {
            Ok(None)
        }
        fn write_chunk(&self, _: u8, _: i32, _: i32, _: &omt_world::Chunk) -> omt_world::WorldResult<()> {
            Ok(())
        }
        fn delete_chunk(&self, _: u8, _: i32, _: i32) -> omt_world::WorldResult<()> {
            Ok(())
        }
        fn iter_chunks(&self, _: u8) -> omt_world::WorldResult<Box<dyn Iterator<Item = (i32, i32)> + '_>> {
            Ok(Box::new(self.0.iter()))
        }
        fn read_raw(&self, n: &omt_world::WorldNode) -> omt_world::WorldResult<Vec<u8>> {
            Err(omt_world::WorldError::NoSuchNode(n.clone()))
        }
        fn flush(&mut self) -> omt_world::WorldResult<()> {
            Ok(())
        }
    }
    MapSource::store(Arc::new(Fake(chunks.iter().copied().collect())), 0).unwrap()
}

#[test]
fn eviction_is_byte_bounded_and_keeps_on_screen() {
    let mut map = MapData::new(src_with(&[(0, 0)]));
    let keys = [TileKey::new(0, 0, 0), TileKey::new(0, 5, 5), TileKey::new(0, -8, -8), TileKey::new(2, 0, 0)];
    for k in keys {
        map.insert(k, image(k, 64));
    }
    let one = map.tiles[&keys[0]].bytes;
    assert_eq!(map.bytes(), one * 4);
    let keep: HashSet<TileKey> = [keys[0]].into();
    // Room for two tiles: the other-level tile and the farthest
    // same-level tile go first.
    let n = map.evict(one * 2, 0, &keep, (0.0, 0.0));
    assert_eq!(n, 2);
    assert!(map.tiles.contains_key(&keys[0]) && map.tiles.contains_key(&keys[1]));
    assert!(!map.tiles.contains_key(&keys[3]) && !map.tiles.contains_key(&keys[2]));
    // On-screen tiles survive even a zero budget.
    map.evict(0, 0, &keep, (0.0, 0.0));
    assert!(map.tiles.contains_key(&keys[0]));
    assert_eq!(map.bytes(), one);
}

#[test]
fn coarse_fallback_hides_once_covered() {
    // Chunks in level-0 tiles (0,0) and (1,0) only.
    let mut map = MapData::new(src_with(&[(0, 0), (40, 5)]));
    let coarse = TileKey::new(1, 0, 0); // covers level-0 tiles (0..=1, 0..=1)
    map.insert(coarse, image(coarse, 512));
    let view = (0, 0, 1, 1);
    map.update_hidden(0, view);
    assert!(map.draw_list().iter().any(|(k, _)| *k == coarse), "fallback drawn while fine tiles load");
    map.insert(TileKey::new(0, 0, 0), image(TileKey::new(0, 0, 0), 512));
    map.update_hidden(0, view);
    assert!(map.draw_list().iter().any(|(k, _)| *k == coarse), "(1,0) still missing");
    map.insert(TileKey::new(0, 1, 0), image(TileKey::new(0, 1, 0), 512));
    map.update_hidden(0, view);
    // (0,1)/(1,1) have no chunks → fully covered → hidden.
    assert!(!map.draw_list().iter().any(|(k, _)| *k == coarse), "covered fallback hidden");
    // Draw order: coarse levels before fine ones.
    map.update_hidden(1, (0, 0, 0, 0));
    let levels: Vec<u8> = map.draw_list().iter().map(|(k, _)| k.level).collect();
    assert!(levels.windows(2).all(|w| w[0] >= w[1]), "{levels:?}");
}

#[test]
fn identify_prefers_finest_tile() {
    let mut map = MapData::new(src_with(&[(0, 0), (1, 1)]));
    let coarse = TileKey::new(1, 0, 0);
    let mut c = image(coarse, 512);
    c.heights = vec![10; 512 * 512];
    map.insert(coarse, c);
    assert_eq!(map.identify(3, 3), Some(("minecraft:stone", 10, true)), "coarse tile is approximate");
    let fine = TileKey::new(0, 0, 0);
    map.insert(fine, image(fine, 512));
    assert_eq!(map.identify(3, 3), Some(("minecraft:stone", 64, false)));
    assert_eq!(map.identify(100, 100), None, "absent chunk");
}
