//! Tests of `icons` (kept apart for the file-size budget).
use super::*;

#[test]
fn art_grids_are_16x16() {
    for i in Icon::ALL {
        if matches!(i, Icon::Worlds | Icon::Settings) {
            continue;
        }
        for row in art(i) {
            assert_eq!(row.chars().count(), 16, "{i:?}: {row}");
            assert!(row.chars().all(|c| c == '.' || palette(c).is_some()), "{i:?}: {row}");
        }
    }
}

#[test]
fn icons_render() {
    for i in Icon::ALL {
        let img = icon(i);
        assert_eq!((img.w, img.h), (16, 16), "{i:?}");
        assert!(img.coverage() > 0.15, "{i:?} is nearly empty");
    }
    for g in Glyph::ALL {
        let img = glyph(g, [255; 4]);
        assert!(img.coverage() > 0.05, "{g:?}");
    }
}

#[test]
fn map_glyph_grids_are_16x16() {
    for g in Glyph::ALL {
        if let Some(rows) = crate::map_glyphs::art(g) {
            for row in rows {
                assert_eq!(row.chars().count(), 16, "{g:?}: {row}");
                assert!(row.chars().all(|c| c == '.' || c == '#'), "{g:?}: {row}");
            }
        }
    }
}
