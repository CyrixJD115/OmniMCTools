//! Tests of `icons` (kept apart for the file-size budget).
use super::*;

#[test]
fn every_icon_and_glyph_has_art() {
    for i in Icon::ALL {
        for px in [16, 24, 48] {
            let img = icon(i, px);
            assert_eq!((img.w, img.h), (px, px), "{i:?}");
            assert!(img.coverage() > 0.08, "{i:?} at {px} px is nearly empty");
        }
    }
    for g in Glyph::ALL {
        let img = glyph(g, [255; 4], 24);
        assert!(img.coverage() > 0.02, "{g:?}");
        // One colour: every visible pixel is the tint (alpha varies only).
        assert!(img.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).all(|p| p[..3] == [255, 255, 255]), "{g:?}");
    }
}

#[test]
fn grid_multiples_are_crisp() {
    // At 24 / 48 px every pixel is fully on or off.
    for g in Glyph::ALL {
        for px in [24, 48] {
            let img = glyph(g, [255; 4], px);
            assert!(img.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0 || p[3] == 255), "{g:?} @ {px}");
        }
    }
}
