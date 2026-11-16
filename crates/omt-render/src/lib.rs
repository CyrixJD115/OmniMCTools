//! `omt-render` — 2D world rendering for the Chunk Locator and the
//! Pruner's viewer (UMT feature list: biome tinting, block textures,
//! heightmap & water shading, Slicer, dimension switcher).
//!
//! Colors come from the pinned `client.jar` (`tools/color-gen` →
//! `data/colors.json`), so the map matches vanilla textures; Bedrock worlds
//! resolve through the converter's inverse block table.

pub mod map;
pub mod palette;
pub mod render;

pub use map::{MapImage, render_chunks, render_world, shade_all};
pub use palette::{BiomeColors, BlockColor, Palette, Tint};
pub use render::{ChunkRender, RenderOptions, biome_at, render_chunk, shade};
