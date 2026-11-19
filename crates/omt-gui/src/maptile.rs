//! One loaded map tile: its GPU image plus the identify data (surface
//! block + height per pixel) and, for styled maps (Map Viewer), the
//! unstyled and styled colors so the style can change without re-reading
//! the world.

use iced::widget::image::Handle;
use std::collections::HashMap;

use omt_render::{MAX_LEVEL, TileImage, TileKey};
use omt_render::style::StyleParams;

/// One built tile on the GPU side plus its identify data.
#[derive(Debug, Clone)]
pub struct TileData {
    /// `None` for an empty tile (no chunks: nothing to draw).
    pub handle: Option<Handle>,
    pub px: u32,
    pub stride: i32,
    pub(crate) surface: Vec<u16>,
    pub(crate) heights: Vec<i16>,
    pub(crate) biomes: Vec<u16>,
    pub(crate) names: Vec<String>,
    /// Unstyled colors, kept when a style post-process is active so the
    /// style can change without re-reading the world.
    base: Option<Vec<u8>>,
    /// Styled colors (kept with `base`; the textured view samples them).
    styled: Option<Vec<u8>>,
    span_blocks: i64,
    pub bytes: usize,
    pub errors: usize,
}

impl TileData {
    pub(crate) fn from_image(t: TileImage, post: Option<&StyleParams>) -> Self {
        let mut bytes = t.heap_bytes();
        let span_blocks = t.key.span_blocks();
        let (handle, base) = match post {
            Some(p) if !t.is_empty() => {
                let styled = style_rgba(t.px, span_blocks, &t.rgba, &t.heights, &t.surface, &t.biomes, &t.names, p);
                bytes += styled.len() * 2;
                (Some(Handle::from_rgba(t.px, t.px, styled.clone())), Some((t.rgba, styled)))
            }
            _ => ((!t.is_empty()).then(|| Handle::from_rgba(t.px, t.px, t.rgba)), None),
        };
        let (base, styled) = match base {
            Some((b, s)) => (Some(b), Some(s)),
            None => (None, None),
        };
        Self { handle, px: t.px, stride: t.stride, surface: t.surface, heights: t.heights, biomes: t.biomes, names: t.names, base, styled, span_blocks, bytes, errors: t.stats.errors }
    }

    /// Surface block name + styled color of a pixel (textured view).
    pub fn sample(&self, lx: usize, lz: usize) -> Option<(&str, [u8; 4])> {
        let i = lz * self.px as usize + lx;
        let styled = self.styled.as_ref()?;
        let c = [styled[i * 4], styled[i * 4 + 1], styled[i * 4 + 2], styled[i * 4 + 3]];
        let name = self.names.get(*self.surface.get(i)? as usize)?;
        Some((name.as_str(), c))
    }

    /// Re-light from the kept base colors.
    pub(crate) fn restyle(&mut self, p: &StyleParams) {
        if let Some(base) = &self.base {
            let styled = style_rgba(self.px, self.span_blocks, base, &self.heights, &self.surface, &self.biomes, &self.names, p);
            self.handle = Some(Handle::from_rgba(self.px, self.px, styled.clone()));
            self.styled = Some(styled);
        }
    }

    /// Surface biome name of a pixel.
    pub fn biome(&self, i: usize) -> Option<&str> {
        self.names.get(*self.biomes.get(i)? as usize).map(String::as_str)
    }

    /// Biome histogram of this tile (legend).
    pub fn biome_counts(&self, into: &mut std::collections::HashMap<String, usize>) {
        let mut local: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for &b in &self.biomes {
            if b != u16::MAX {
                *local.entry(b).or_default() += 1;
            }
        }
        for (b, n) in local {
            if let Some(name) = self.names.get(b as usize) {
                *into.entry(name.clone()).or_default() += n;
            }
        }
    }

    /// Heights + styled colors, cloned (3D view snapshot).
    pub fn field(&self) -> Option<(Vec<i16>, Vec<u8>)> {
        Some((self.heights.clone(), self.styled.clone()?))
    }

    /// Styled RGBA of this tile (export).
    pub fn pixels(&self) -> Option<(u32, Vec<u8>)> {
        Some((self.px, self.styled.clone()?))
    }

    /// Whether this tile shows less than full detail (sampled chunks).
    pub fn reduced(&self) -> bool {
        self.stride > 1
    }
}

#[allow(clippy::too_many_arguments)]
fn style_rgba(px: u32, span_blocks: i64, rgba: &[u8], heights: &[i16], surface: &[u16], biomes: &[u16], names: &[String], p: &StyleParams) -> Vec<u8> {
    let view = omt_render::style::TileView { px: px as usize, bpp: (span_blocks as f32 / px.max(1) as f32), rgba, heights, surface, biomes, names };
    omt_render::style::stylize(&view, p)
}


/// Biome queries over a map's loaded tiles (Biomes style legend, cursor
/// readout).
impl crate::mapdata::MapData {
    /// Surface biome at a block from the finest loaded tile.
    pub fn biome_at(&self, bx: i32, bz: i32) -> Option<&str> {
        let (cx, cz) = (bx.div_euclid(16), bz.div_euclid(16));
        for level in 0..=MAX_LEVEL {
            let key = TileKey::containing(level, cx, cz);
            let Some(t) = self.tiles.get(&key) else { continue };
            if t.px == 0 {
                continue;
            }
            let bpp = key.span_blocks() / t.px as i64;
            let (x0, z0, _, _) = key.chunk_rect();
            let lx = (bx as i64 - x0 as i64 * 16).div_euclid(bpp);
            let lz = (bz as i64 - z0 as i64 * 16).div_euclid(bpp);
            if lx < 0 || lz < 0 || lx >= t.px as i64 || lz >= t.px as i64 {
                continue;
            }
            return t.biome((lz * t.px as i64 + lx) as usize);
        }
        None
    }

    /// Surface blocks of the display-level tiles, most common first.
    pub fn block_legend(&self) -> Vec<(String, usize)> {
        let mut m: HashMap<String, usize> = HashMap::new();
        let level = self.legend_level();
        for (k, t) in &self.tiles {
            if k.level != level {
                continue;
            }
            let mut local: HashMap<u16, usize> = HashMap::new();
            for &s in &t.surface {
                if s != u16::MAX {
                    *local.entry(s).or_default() += 1;
                }
            }
            for (s, n) in local {
                if let Some(name) = t.names.get(s as usize) {
                    *m.entry(name.clone()).or_default() += n;
                }
            }
        }
        let mut v: Vec<(String, usize)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// Level the legends count: the display level, or the finest level
    /// with loaded tiles while the display level is still rendering.
    fn legend_level(&self) -> u8 {
        if self.tiles.keys().any(|k| k.level == self.level) {
            return self.level;
        }
        self.tiles.keys().map(|k| k.level).min().unwrap_or(self.level)
    }

    /// Biomes of the display-level tiles, most common first.
    pub fn biome_legend(&self) -> Vec<(String, usize)> {
        let mut m = HashMap::new();
        let level = self.legend_level();
        for (k, t) in &self.tiles {
            if k.level == level {
                t.biome_counts(&mut m);
            }
        }
        let mut v: Vec<(String, usize)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }
}
