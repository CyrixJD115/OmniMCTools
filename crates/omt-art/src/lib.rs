//! `omt-art` — every image OmniMCTools shows is generated here, in code:
//! noise-based block textures, isometric blocks, pixel icons, a 5×7 pixel
//! font with a title renderer, and seeded pixel-art landscapes. Nothing is
//! copied from Minecraft's assets; output is deterministic.
//!
//! The GUI builds its images from these functions at startup (a few
//! milliseconds); `tools/asset-gen` writes the same art to PNG files for
//! packaging (app icon) and previews.

pub mod blocks;
pub mod font;
pub mod icons;
pub mod image;
pub mod landscape;
mod map_glyphs;

pub use blocks::{BlockFaces, Material, iso_block, texture};
pub use font::{TitleStyle, pixel_text, text_width, title};
pub use icons::{Glyph, Icon, glyph, icon, tool_block};
pub use image::{Image, Rgba, Rng, hex, mix, shade};
pub use landscape::{Scene, fade_bottom, landscape, landscape_with, scene_for};

/// The OmniMCTools wordmark ("OMNI" grass-green, "MCTOOLS" stone-white).
pub fn wordmark(scale: u32) -> Image {
    let green = TitleStyle { top: hex(0x9be86a), bottom: hex(0x3f8f2a), outline: hex(0x0f1a0c), shadow: [0, 0, 0, 110] };
    let stone = TitleStyle { top: hex(0xffffff), bottom: hex(0xa9adb5), outline: hex(0x14161b), shadow: [0, 0, 0, 110] };
    let a = title("OMNI", scale, green);
    let b = title("MCTOOLS", scale, stone);
    let mut out = Image::new(a.w + b.w, a.h.max(b.h));
    out.draw(&a, 0, 0);
    out.draw(&b, a.w as i32 - scale as i32, 0);
    out
}

/// The app icon: an isometric grass block with a diamond-ore inset badge.
pub fn app_icon(px: u32) -> Image {
    let block = iso_block(&BlockFaces::grass(), px / 2);
    let mut out = Image::new(px, px);
    out.draw(&block, 0, 0);
    let badge = iso_block(&BlockFaces::uniform(Material::DiamondOre), px / 6);
    out.draw(&badge, (px - badge.w) as i32, (px - badge.h) as i32);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn wordmark_and_app_icon() {
        let w = super::wordmark(2);
        assert!(w.w > 100 && w.h >= 18);
        let i = super::app_icon(64);
        assert_eq!((i.w, i.h), (64, 64));
        assert!(i.coverage() > 0.5);
    }
}
