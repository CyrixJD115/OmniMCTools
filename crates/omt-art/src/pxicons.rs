//! Rasterizer for the vendored **Pixelarticons** (MIT, `assets/pixelarticons/`).
//! The icons are drawn on a 24 × 24 pixel grid with rectilinear paths
//! (`M/m H/h V/v L/l Z/z`, nonzero fill), so each one becomes an exact
//! per-cell mask; rendering at `px` pixels box-filters that mask (exact
//! area coverage), which is pixel-perfect at multiples of 24.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::image::{Image, Rgba};

/// Grid size of the icon set.
pub const GRID: usize = 24;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        /// The SVG source of a vendored icon.
        fn source(name: &str) -> Option<&'static str> {
            match name {
                $($name => Some(include_str!(concat!("../assets/pixelarticons/", $name, ".svg"))),)*
                _ => None,
            }
        }
    };
}

icons!(
    "home", "earth", "map", "braces", "sort-horizontal", "scissors", "plug", "gear", "folder", "archive", "map-pin-home", "user", "bookmark", "sun", "copy", "star", "pencil",
    "search", "close", "plus", "grid-2x2-2", "bulletlist", "reload", "filter", "chevron-left", "chevron-right", "chevron-down", "check", "menu", "target", "upload", "image",
    "zoom-in", "zoom-out", "scale", "square-dashed-cursor", "circle", "lasso", "square-scissors", "clock", "grid-3x3", "frame", "dog", "potion", "lightbulb-off",
    "sliders-vertical", "blocks", "compass", "eye", "invert", "circle-info", "box", "leaf", "colors-swatch", "save", "sparkles",
);

type Seg = ((f32, f32), (f32, f32));

/// Line segments of an SVG path's `d` attribute.
fn segments(d: &str, out: &mut Vec<Seg>) {
    let b = d.as_bytes();
    let (mut x, mut y, mut sx, mut sy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut cmd = b'M';
    let mut j = 0;
    loop {
        while j < b.len() && (b[j] == b' ' || b[j] == b',' || b[j] == b'\n' || b[j] == b'\t') {
            j += 1;
        }
        if j >= b.len() {
            break;
        }
        if b[j].is_ascii_alphabetic() {
            cmd = b[j];
            j += 1;
        }
        let mut take = || -> Option<f32> {
            while j < b.len() && (b[j] == b' ' || b[j] == b',' || b[j] == b'\n' || b[j] == b'\t') {
                j += 1;
            }
            let start = j;
            if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
                j += 1;
            }
            let mut dot = false;
            while j < b.len() && (b[j].is_ascii_digit() || (b[j] == b'.' && !dot)) {
                dot |= b[j] == b'.';
                j += 1;
            }
            d.get(start..j).filter(|s| !s.is_empty() && *s != "-" && *s != "+").and_then(|s| s.parse().ok())
        };
        match cmd {
            b'Z' | b'z' => {
                out.push(((x, y), (sx, sy)));
                (x, y) = (sx, sy);
                cmd = b'\0';
            }
            b'M' | b'm' => {
                let (Some(a), Some(c)) = (take(), take()) else { break };
                let (a, c) = if cmd == b'm' { (a + x, c + y) } else { (a, c) };
                (x, y, sx, sy) = (a, c, a, c);
                // Further pairs are implicit line-tos.
                cmd = if cmd == b'm' { b'l' } else { b'L' };
            }
            b'H' | b'h' => {
                let Some(a) = take() else { break };
                let nx = if cmd == b'h' { x + a } else { a };
                out.push(((x, y), (nx, y)));
                x = nx;
            }
            b'V' | b'v' => {
                let Some(a) = take() else { break };
                let ny = if cmd == b'v' { y + a } else { a };
                out.push(((x, y), (x, ny)));
                y = ny;
            }
            b'L' | b'l' => {
                let (Some(a), Some(c)) = (take(), take()) else { break };
                let (a, c) = if cmd == b'l' { (a + x, c + y) } else { (a, c) };
                out.push(((x, y), (a, c)));
                (x, y) = (a, c);
            }
            // Unsupported command (curves): stop rather than misdraw.
            _ => break,
        }
    }
}

/// The 24 × 24 fill mask of an icon (row-major), cached.
pub fn mask(name: &str) -> Option<[bool; GRID * GRID]> {
    static CACHE: OnceLock<Mutex<HashMap<String, [bool; GRID * GRID]>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(m) = cache.lock().ok().and_then(|c| c.get(name).copied()) {
        return Some(m);
    }
    let svg = source(name)?;
    let mut segs = Vec::new();
    let mut rest = svg;
    while let Some(k) = rest.find(" d=\"") {
        rest = &rest[k + 4..];
        let end = rest.find('"')?;
        segments(&rest[..end], &mut segs);
        rest = &rest[end..];
    }
    let mut m = [false; GRID * GRID];
    for py in 0..GRID {
        let cy = py as f32 + 0.5;
        for px in 0..GRID {
            let cx = px as f32 + 0.5;
            // Nonzero winding of a ray to the right of the cell centre.
            let mut w = 0i32;
            for &((x0, y0), (x1, y1)) in &segs {
                if (y0 <= cy) != (y1 <= cy) {
                    let xi = x0 + (cy - y0) * (x1 - x0) / (y1 - y0);
                    if xi > cx {
                        w += if y1 > y0 { 1 } else { -1 };
                    }
                }
            }
            m[py * GRID + px] = w != 0;
        }
    }
    if let Ok(mut c) = cache.lock() {
        c.insert(name.to_string(), m);
    }
    Some(m)
}

/// Area coverage (0..1) of the mask, shifted by `off` grid cells, over each
/// pixel of a `px × px` raster.
fn coverage(m: &[bool; GRID * GRID], px: usize, off: f32) -> Vec<f32> {
    let k = GRID as f32 / px as f32;
    let mut out = vec![0.0; px * px];
    for y in 0..px {
        let (y0, y1) = (y as f32 * k - off, (y + 1) as f32 * k - off);
        for x in 0..px {
            let (x0, x1) = (x as f32 * k - off, (x + 1) as f32 * k - off);
            let mut a = 0.0;
            for gy in (y0.floor().max(0.0) as usize)..(y1.ceil().min(GRID as f32) as usize) {
                let oy = (y1.min(gy as f32 + 1.0) - y0.max(gy as f32)).max(0.0);
                for gx in (x0.floor().max(0.0) as usize)..(x1.ceil().min(GRID as f32) as usize) {
                    if m[gy * GRID + gx] {
                        a += oy * (x1.min(gx as f32 + 1.0) - x0.max(gx as f32)).max(0.0);
                    }
                }
            }
            out[y * px + x] = (a / (k * k)).min(1.0);
        }
    }
    out
}

/// Render an icon at `px` pixels in `color`, optionally over a one-cell
/// down-right drop shadow. Unknown names render empty.
pub fn render(name: &str, px: u32, color: Rgba, shadow: Option<Rgba>) -> Image {
    let mut img = Image::new(px, px);
    let Some(m) = mask(name) else { return img };
    let n = px as usize;
    let fg = coverage(&m, n, 0.0);
    let sh = shadow.map(|_| coverage(&m, n, 1.0));
    for i in 0..n * n {
        let af = fg[i] * color[3] as f32 / 255.0;
        let (sc, ash) = match (&sh, shadow) {
            (Some(s), Some(c)) => (c, s[i] * c[3] as f32 / 255.0 * (1.0 - af)),
            _ => ([0; 4], 0.0),
        };
        let a = af + ash;
        if a <= 0.0 {
            continue;
        }
        let mixc = |k: usize| ((color[k] as f32 * af + sc[k] as f32 * ash) / a).round() as u8;
        img.rgba[i * 4..i * 4 + 4].copy_from_slice(&[mixc(0), mixc(1), mixc(2), (a * 255.0).round() as u8]);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rectilinear_paths() {
        let mut s = Vec::new();
        segments("M4 20h16v2H4zm16-10h2v10h-2z", &mut s);
        assert_eq!(s.len(), 8);
        assert_eq!(s[0], ((4.0, 20.0), (20.0, 20.0)));
        assert_eq!(s[4].0, (20.0, 10.0), "relative move after close");
        let m = mask("home").unwrap();
        // Floor bar of the house: row 20, x 4..20 filled, outside empty.
        assert!((4..20).all(|x| m[20 * GRID + x]) && !m[20 * GRID + 3] && !m[20 * GRID + 20]);
        assert!(mask("no-such-icon").is_none());
    }

    #[test]
    fn half_size_is_exact_box_filter() {
        // 12 px = 2×2 cells per pixel: alpha is a multiple of 1/4.
        let img = render("plus", 12, [255; 4], None);
        assert!(img.rgba.as_chunks::<4>().0.iter().all(|p| [0, 64, 128, 191, 255].contains(&p[3])), "{:?}", img.rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect::<Vec<_>>());
    }
}
