//! An original 5×7 pixel font (caps, digits, a little punctuation) and a
//! wordmark renderer: gradient fill, dark outline, hard drop shadow — the
//! blocky title look, in OMT's own colors.

use crate::image::{Image, Rgba, mix};

/// 5×7 glyph rows (bit 4 = leftmost column).
fn glyph(c: char) -> Option<[u8; 7]> {
    let g = match c.to_ascii_uppercase() {
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e],
        'D' => [0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0c],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1e, 0x01, 0x01, 0x0e, 0x01, 0x01, 0x1e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
        '.' => [0, 0, 0, 0, 0, 0x0c, 0x0c],
        ',' => [0, 0, 0, 0, 0x0c, 0x04, 0x08],
        ':' => [0, 0x0c, 0x0c, 0, 0x0c, 0x0c, 0],
        '-' => [0, 0, 0, 0x1f, 0, 0, 0],
        '+' => [0, 0x04, 0x04, 0x1f, 0x04, 0x04, 0],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0, 0x04],
        '?' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0, 0x04],
        '&' => [0x0c, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0d],
        '\'' => [0x04, 0x04, 0x08, 0, 0, 0, 0],
        ' ' => [0; 7],
        _ => return None,
    };
    Some(g)
}

/// Glyph advance in font pixels (5 wide + 1 gap; space is narrower).
fn advance(c: char) -> u32 {
    match c {
        ' ' => 4,
        '.' | ',' | ':' | '!' | '\'' => 4,
        _ => 6,
    }
}

/// Width of `text` in font pixels (unscaled).
pub fn text_width(text: &str) -> u32 {
    text.chars().map(advance).sum::<u32>().saturating_sub(1)
}

/// Plain one-color text at `scale` (no effects).
pub fn pixel_text(text: &str, scale: u32, color: Rgba) -> Image {
    let mut img = Image::new(text_width(text).max(1) * scale, 7 * scale);
    let mut x = 0i32;
    for c in text.chars() {
        let g = glyph(c).unwrap_or(glyph('?').unwrap_or([0; 7]));
        // Narrow glyphs are drawn shifted left so they sit centred.
        let shift = if advance(c) == 4 && c != ' ' { -1 } else { 0 };
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits & (0x10 >> col) != 0 {
                    img.fill_rect((x + shift + col) * scale as i32, row as i32 * scale as i32, scale as i32, scale as i32, color);
                }
            }
        }
        x += advance(c) as i32;
    }
    img
}

/// Title styling.
#[derive(Debug, Clone, Copy)]
pub struct TitleStyle {
    pub top: Rgba,
    pub bottom: Rgba,
    pub outline: Rgba,
    pub shadow: Rgba,
}

/// Gradient-filled, outlined, drop-shadowed title text. The result is
/// `(w + 2) * scale + shadow` wide.
pub fn title(text: &str, scale: u32, st: TitleStyle) -> Image {
    let base = pixel_text(text, 1, [255, 255, 255, 255]);
    // 1-px outline ring around the unscaled glyphs.
    let mut lay = Image::new(base.w + 2, base.h + 2);
    for y in 0..base.h as i32 {
        for x in 0..base.w as i32 {
            if base.get(x, y)[3] > 0 {
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                    if base.get(x + dx, y + dy)[3] == 0 {
                        lay.set(x + 1 + dx, y + 1 + dy, st.outline);
                    }
                }
            }
        }
    }
    for y in 0..base.h as i32 {
        let t = y as f32 / (base.h - 1) as f32;
        let fill = mix(st.top, st.bottom, t);
        for x in 0..base.w as i32 {
            if base.get(x, y)[3] > 0 {
                lay.set(x + 1, y + 1, fill);
            }
        }
    }
    lay.scaled(scale).with_shadow(scale as i32, scale as i32, st.shadow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_render() {
        assert_eq!(text_width("A"), 5);
        assert_eq!(text_width("AB"), 11);
        let t = pixel_text("OMT", 2, [255, 255, 255, 255]);
        assert_eq!((t.w, t.h), (17 * 2, 14));
        assert!(t.coverage() > 0.3);
        let s = TitleStyle { top: [255; 4], bottom: [100, 100, 100, 255], outline: [0, 0, 0, 255], shadow: [0, 0, 0, 120] };
        let ti = title("OMNI", 3, s);
        assert_eq!(ti.w, (text_width("OMNI") + 2) * 3 + 3);
    }

    #[test]
    fn every_letter_has_a_glyph() {
        for c in ('A'..='Z').chain('0'..='9') {
            assert!(glyph(c).is_some(), "{c}");
        }
    }
}
