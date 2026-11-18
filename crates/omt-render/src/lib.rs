//! `omt-render` — 2D world rendering for the Chunk Locator and the
//! Pruner's viewer (UMT feature list: biome tinting, block textures,
//! heightmap & water shading, Slicer, dimension switcher).
//!
//! Colors come from the pinned `client.jar` (`tools/color-gen` →
//! `data/colors.json`), so the map matches vanilla textures; Bedrock worlds
//! resolve through the converter's inverse block table.

pub mod area;
pub mod blocks3d;
pub mod map;
pub mod markers;
pub mod palette;
pub mod render;
pub mod source;
pub mod style;
pub mod tilecache;
pub mod voxel;

pub use area::{AreaSpec, AreaStats, Band, MAX_LEVEL, NameTable, TILE_CHUNKS, TILE_PX, TileImage, TileKey, build_tile, render_area, level_stride, stride_for};
pub use map::{MapImage, render_chunks, render_world, shade_all};
pub use palette::{BiomeColors, BlockColor, Palette, Tint};
pub use render::{ChunkRender, NO_SURFACE, RenderOptions, biome_at, render_chunk, shade};
pub use source::MapSource;
pub use tilecache::TileCache;
