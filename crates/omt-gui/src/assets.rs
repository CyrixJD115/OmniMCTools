//! Image handles for the UI, generated from `omt-art` on first use and
//! cached for the life of the process: icons, tinted glyphs, the wordmark,
//! pixel-font headings, hero banners and per-world thumbnails.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use iced::widget::image::{FilterMethod, Handle};
use iced::widget::image;
use iced::{Color, Element, Length};
use omt_art::{Glyph, Icon, Image, Material, Scene};

fn handle(img: Image) -> Handle {
    Handle::from_rgba(img.w, img.h, img.rgba)
}

fn rgba(c: Color) -> [u8; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [q(c.r), q(c.g), q(c.b), q(c.a)]
}

type Cache<K> = OnceLock<Mutex<HashMap<K, Handle>>>;

fn cached<K: std::hash::Hash + Eq + Clone>(cell: &'static Cache<K>, key: K, make: impl FnOnce() -> Image) -> Handle {
    let map = cell.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(m) = map.lock()
        && let Some(h) = m.get(&key)
    {
        return h.clone();
    }
    let h = handle(make());
    if let Ok(mut m) = map.lock() {
        m.insert(key, h.clone());
    }
    h
}

/// Crisp (nearest-neighbor) image element of a handle.
pub fn pixel<'a, M: 'a>(h: Handle, w: f32, hgt: f32) -> Element<'a, M> {
    image(h).filter_method(FilterMethod::Nearest).width(Length::Fixed(w)).height(Length::Fixed(hgt)).into()
}

/// Raster size for an icon shown at `size` logical px: twice the size
/// (sharp on 1×–2× displays, filtered down smoothly), rounded up to a
/// multiple of the 24-px icon grid when close to one (pixel-exact).
fn icon_px(size: f32) -> u32 {
    let px = (size * 2.0).round().max(8.0) as u32;
    let snapped = px.div_ceil(24) * 24;
    if snapped - px <= 4 { snapped } else { px }
}

/// Smoothly scaled image element (icons rasterized above display size).
fn smooth<'a, M: 'a>(h: Handle, size: f32) -> Element<'a, M> {
    image(h).filter_method(FilterMethod::Linear).width(Length::Fixed(size)).height(Length::Fixed(size)).into()
}

/// A colored app icon (Pixelarticons, tinted + shadowed) at `size` px.
pub fn icon<'a, M: 'a>(i: Icon, size: f32) -> Element<'a, M> {
    static C: Cache<(Icon, u32)> = OnceLock::new();
    let px = icon_px(size);
    smooth(cached(&C, (i, px), || omt_art::icon(i, px)), size)
}

/// A theme-tinted glyph at `size` px.
pub fn glyph<'a, M: 'a>(g: Glyph, color: Color, size: f32) -> Element<'a, M> {
    static C: Cache<(Glyph, [u8; 4], u32)> = OnceLock::new();
    let c = rgba(color);
    let px = icon_px(size);
    smooth(cached(&C, (g, c, px), || omt_art::glyph(g, c, px)), size)
}

/// The OmniMCTools wordmark, `height` px tall.
pub fn wordmark<'a, M: 'a>(height: f32) -> Element<'a, M> {
    static C: Cache<u8> = OnceLock::new();
    let h = cached(&C, 0, || omt_art::wordmark(1));
    // Unscaled wordmark is 9 px tall (7 + outline + shadow).
    let w = omt_art::wordmark(1).w as f32 * height / 10.0;
    pixel(h, w, height)
}

/// Pixel-font heading (scale 1 font = 7 px tall).
pub fn heading<'a, M: 'a>(text: &str, color: Color, scale: u32) -> Element<'a, M> {
    static C: Cache<(String, [u8; 4], u32)> = OnceLock::new();
    let c = rgba(color);
    let shadow = [0, 0, 0, 90];
    let make = || omt_art::pixel_text(text, scale, c).with_shadow(scale as i32 / 2 + 1, scale as i32 / 2 + 1, shadow);
    let h = cached(&C, (text.to_string(), c, scale), make);
    let w = (omt_art::text_width(text) * scale + scale / 2 + 1) as f32;
    pixel(h, w, (7 * scale + scale / 2 + 1) as f32)
}

/// A large isometric block (tool cards) — real textures when a pack is
/// active.
pub fn block<'a, M: 'a>(m: Material, size: f32) -> Element<'a, M> {
    static C: Cache<(Material, u64)> = OnceLock::new();
    let make = || match crate::textures::material_faces(m) {
        Some(f) => omt_art::iso_block(&f, 32),
        None => omt_art::tool_block(m, 64),
    };
    pixel(cached(&C, (m, crate::textures::generation()), make), size, size)
}

/// A block's top texture (active pack) or its map color, as an icon.
pub fn block_icon<'a, M: 'a>(name: &str, size: f32) -> Element<'a, M> {
    static C: Cache<(String, u64)> = OnceLock::new();
    let make = || {
        if let Some(pack) = crate::textures::current()
            && let Some(img) = pack.face(name, omt_texpack::Face::Up, true).or_else(|| pack.face(name, omt_texpack::Face::Side, false))
        {
            return (*img).clone();
        }
        let c = omt_render::Palette::embedded().color_of(name);
        let mut img = Image::filled(16, 16, [c[0], c[1], c[2], 255]);
        for i in 0..16 {
            img.set(i, 0, [255, 255, 255, 60]);
            img.set(0, i, [255, 255, 255, 60]);
        }
        img
    };
    pixel(cached(&C, (name.to_string(), crate::textures::generation()), make), size, size)
}

/// An item's inventory icon: its item texture from the active pack, else
/// the block's face (block items), else a colored tile with its initial.
pub fn item_icon<'a, M: 'a>(name: &str, size: f32) -> Element<'a, M> {
    static C: Cache<(String, u64)> = OnceLock::new();
    let make = || {
        if let Some(pack) = crate::textures::current() {
            if let Some(img) = pack.item(name) {
                return (*img).clone();
            }
            if let Some(img) = pack.face(name, omt_texpack::Face::Up, true).or_else(|| pack.face(name, omt_texpack::Face::Side, false)) {
                return (*img).clone();
            }
        }
        let c = omt_render::Palette::embedded().color_of(name);
        let known = c[3] > 0 && c[..3] != [0, 0, 0];
        let base = if known { [c[0], c[1], c[2], 255] } else { [0x6b, 0x5b, 0x95, 255] };
        let mut img = Image::filled(16, 16, base);
        for i in 0..16 {
            img.set(i, 0, [255, 255, 255, 70]);
            img.set(0, i, [255, 255, 255, 70]);
            img.set(i, 15, [0, 0, 0, 90]);
            img.set(15, i, [0, 0, 0, 90]);
        }
        let initial: String = name.rsplit(':').next().unwrap_or(name).chars().next().map(|ch| ch.to_ascii_uppercase().to_string()).unwrap_or_default();
        let glyph = omt_art::pixel_text(&initial, 1, [255, 255, 255, 235]);
        img.draw(&glyph, (16 - glyph.w as i32) / 2, (16 - glyph.h as i32) / 2);
        img
    };
    pixel(cached(&C, (name.to_string(), crate::textures::generation()), make), size, size)
}

fn scene_art(scene: Scene, seed: u64, w: u32, h: u32, cell: u32) -> Image {
    omt_art::landscape_with(scene, seed, w, h, cell, &crate::textures::material_texture)
}

/// Hero banner of a scene (fills the width, cropped to `height`).
pub fn banner<'a, M: 'a>(scene: Scene, height: f32) -> Element<'a, M> {
    static C: Cache<(Scene, u64)> = OnceLock::new();
    let h = cached(&C, (scene, crate::textures::generation()), || scene_art(scene, 7, 640, 160, 8));
    image(h).filter_method(FilterMethod::Nearest).content_fit(iced::ContentFit::Cover).width(Length::Fill).height(Length::Fixed(height)).into()
}

/// A stable thumbnail for a world without an icon.
pub fn world_thumb(name: &str, bedrock: bool) -> Handle {
    static C: Cache<(String, bool, u64)> = OnceLock::new();
    let (scene, seed) = omt_art::scene_for(name, bedrock);
    cached(&C, (name.to_string(), bedrock, crate::textures::generation()), || scene_art(scene, seed, 160, 90, 5))
}

/// Scene for the Home hero: the theme's own, or the time of day.
pub fn hero_scene() -> Scene {
    if let Some(s) = crate::style::theme_kind().scene() {
        return s;
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let hour = (secs + utc_offset_secs()).div_euclid(3600).rem_euclid(24);
    match hour {
        6..=16 => Scene::Day,
        17..=19 => Scene::Sunset,
        _ => Scene::Night,
    }
}

/// Standard-time UTC offset from the system zone (Unix: the POSIX TZ
/// footer of `/etc/localtime`, e.g. `CET-1CEST,…` → +3600; DST ignored).
/// 0 when unknown.
fn utc_offset_secs() -> i64 {
    let tz = std::env::var("TZ").ok().filter(|s| !s.starts_with(':') && !s.contains('/')).or_else(|| {
        let bytes = std::fs::read("/etc/localtime").ok()?;
        let text = String::from_utf8_lossy(&bytes);
        text.trim_end_matches('\n').rsplit('\n').next().map(str::to_string)
    });
    tz.as_deref().and_then(posix_std_offset).unwrap_or(0)
}

/// Offset east of UTC of a POSIX TZ string's standard time.
fn posix_std_offset(tz: &str) -> Option<i64> {
    let s = tz.trim();
    // Skip the zone name: letters, or a <quoted> name.
    let rest = if let Some(r) = s.strip_prefix('<') { r.split_once('>')?.1 } else { s.trim_start_matches(|c: char| c.is_ascii_alphabetic()) };
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '+' || c == '-')).unwrap_or(rest.len());
    let off = &rest[..end];
    if off.is_empty() {
        return None;
    }
    let (sign, num) = match off.as_bytes()[0] {
        b'-' => (1, &off[1..]),
        b'+' => (-1, &off[1..]),
        _ => (-1, off),
    };
    let mut parts = num.split(':').map(|p| p.parse::<i64>().unwrap_or(0));
    let h = parts.next()?;
    let m = parts.next().unwrap_or(0);
    // POSIX offsets are west-positive.
    Some(sign * (h * 3600 + m * 60))
}

#[cfg(test)]
mod tests {
    #[test]
    fn posix_offsets() {
        assert_eq!(super::posix_std_offset("EST5EDT,M3.2.0,M11.1.0"), Some(-5 * 3600));
        assert_eq!(super::posix_std_offset("CET-1CEST,M3.5.0,M10.5.0/3"), Some(3600));
        assert_eq!(super::posix_std_offset("<+0530>-5:30"), Some(5 * 3600 + 1800));
        assert_eq!(super::posix_std_offset("UTC0"), Some(0));
        assert_eq!(super::posix_std_offset("UTC"), None);
    }
}
