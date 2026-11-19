//! Highlighter data sources (uNmINeD's Block finder / Biome finder /
//! InhabitedTime): scan decoded chunks and report where queries match.
//! The Map Viewer runs them over the visible chunks and draws the hits.

use omt_world::Chunk;

use crate::palette::Palette;
use crate::rules::matches;

/// What a highlighter looks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    /// Blocks matching any pattern, between `min_y` and `max_y`.
    Blocks { patterns: Vec<String>, min_y: i32, max_y: i32 },
    /// Biomes matching any pattern (4×4 cells), between `min_y` and `max_y`.
    Biomes { patterns: Vec<String>, min_y: i32, max_y: i32 },
    /// Chunks with InhabitedTime of at least `min_ticks` (Java).
    Inhabited { min_ticks: i64 },
}

/// One match: `size`×`size` blocks at block (`x`, `z`); `value` is the Y
/// of a block hit or the ticks of an inhabited chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub query: u8,
    pub x: i32,
    pub z: i32,
    pub size: u8,
    pub value: i64,
}

/// Per-chunk cap on block hits (keeps a "find stone" query bounded).
pub const MAX_BLOCK_HITS: usize = 256;

/// Scan one chunk for every query (`query` index = position in `queries`).
pub fn scan_chunk(chunk: &Chunk, palette: &Palette, queries: &[Query], out: &mut Vec<Hit>) {
    for (qi, q) in queries.iter().enumerate() {
        let qi = qi as u8;
        match q {
            Query::Blocks { patterns, min_y, max_y } => blocks(chunk, qi, patterns, *min_y, *max_y, out),
            Query::Biomes { patterns, min_y, max_y } => biomes(chunk, palette, qi, patterns, *min_y, *max_y, out),
            Query::Inhabited { min_ticks } => {
                if chunk.inhabited_time >= *min_ticks && chunk.inhabited_time > 0 {
                    out.push(Hit { query: qi, x: chunk.x * 16, z: chunk.z * 16, size: 16, value: chunk.inhabited_time });
                }
            }
        }
    }
}

fn blocks(chunk: &Chunk, qi: u8, patterns: &[String], min_y: i32, max_y: i32, out: &mut Vec<Hit>) {
    // Top-most matching Y per column.
    let mut top = [i32::MIN; 256];
    for (&sy, slice) in chunk.slices.iter().rev() {
        let (y0, y1) = (sy as i32 * 16, sy as i32 * 16 + 15);
        if y1 < min_y || y0 > max_y {
            continue;
        }
        let hit: Vec<bool> = slice.palette.iter().map(|b| patterns.iter().any(|p| matches(p, &b.name))).collect();
        if !hit.iter().any(|&h| h) {
            continue;
        }
        for (i, &pi) in slice.indices.iter().enumerate() {
            if !hit.get(pi as usize).copied().unwrap_or(false) {
                continue;
            }
            let y = y0 + (i / 256) as i32;
            if y < min_y || y > max_y {
                continue;
            }
            let col = i % 256;
            top[col] = top[col].max(y);
        }
    }
    let mut n = 0;
    for (col, &y) in top.iter().enumerate() {
        if y != i32::MIN && n < MAX_BLOCK_HITS {
            out.push(Hit { query: qi, x: chunk.x * 16 + (col % 16) as i32, z: chunk.z * 16 + (col / 16) as i32, size: 1, value: y as i64 });
            n += 1;
        }
    }
}

fn biomes(chunk: &Chunk, palette: &Palette, qi: u8, patterns: &[String], min_y: i32, max_y: i32, out: &mut Vec<Hit>) {
    let (lo, hi) = match (chunk.slices.keys().next(), chunk.slices.keys().next_back()) {
        (Some(&a), Some(&b)) => (a as i32 * 16, b as i32 * 16 + 15),
        _ => (min_y.max(-64), max_y.min(319)),
    };
    let (y0, y1) = (min_y.max(lo), max_y.min(hi));
    for cz in 0..4 {
        for cx in 0..4 {
            let (lx, lz) = (cx * 4 + 2, cz * 4 + 2);
            let mut y = y0;
            let found = loop {
                if y > y1 {
                    break false;
                }
                if crate::render::biome_at(chunk, palette, lx, y, lz).is_some_and(|b| patterns.iter().any(|p| matches(p, b))) {
                    break true;
                }
                y += 4;
            };
            if found {
                out.push(Hit { query: qi, x: chunk.x * 16 + cx as i32 * 4, z: chunk.z * 16 + cz as i32 * 4, size: 4, value: 0 });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omt_world::{BiomeCells, BlockState, Slice};

    #[test]
    fn finds_blocks_biomes_and_time() {
        let mut chunk = Chunk { x: 2, z: -1, inhabited_time: 500, ..Default::default() };
        let mut indices = vec![0u16; 4096];
        indices[5 * 256 + 3 * 16 + 7] = 1; // diamond ore at x=7, y=5, z=3
        chunk.slices.insert(0, Slice { palette: vec![BlockState::air(), BlockState { name: "minecraft:diamond_ore".into(), properties: vec![] }], indices });
        chunk.java_biomes.insert(0, BiomeCells { palette: vec!["minecraft:plains".into(), "minecraft:swamp".into()], cells: (0..64).map(|c| (c % 4 == 0) as u16).collect() });
        let qs = [
            Query::Blocks { patterns: vec!["*diamond_ore".into()], min_y: -64, max_y: 320 },
            Query::Blocks { patterns: vec!["diamond_ore".into()], min_y: 10, max_y: 20 },
            Query::Biomes { patterns: vec!["swamp".into()], min_y: 0, max_y: 15 },
            Query::Inhabited { min_ticks: 100 },
        ];
        let mut hits = Vec::new();
        scan_chunk(&chunk, Palette::embedded(), &qs, &mut hits);
        assert!(hits.contains(&Hit { query: 0, x: 39, z: -13, size: 1, value: 5 }));
        assert!(!hits.iter().any(|h| h.query == 1), "outside the Y range");
        assert_eq!(hits.iter().filter(|h| h.query == 2).count(), 4, "x4 == 0 column of cells");
        assert!(hits.contains(&Hit { query: 3, x: 32, z: -16, size: 16, value: 500 }));
    }
}
