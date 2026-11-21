//! Isometric input → screen tests: real canvas mouse events become pan
//! steps 1:1 with the cursor, and a pan moves every drawn tile by exactly
//! the drag (the view is glued to the pointer — no lag, no snapping).

use iced::mouse::{self, Button, Cursor};
use iced::widget::canvas::Program;
use iced::{Point, Rectangle, Size};

use crate::viewer_3d::{Canvas3, Ev3, State3};
use crate::viewer_isotiles::{IsoTiles, Layer, TILE};

fn canvas<'a>() -> Canvas3<'a, Ev3> {
    Canvas3 { frame: None, crisp: false, mouse_look: false, locked: false, iso: true, iso_pan: (0.0, 0.0), tiles: Vec::new(), status: String::new(), on_event: Box::new(|e| e) }
}

fn send(c: &Canvas3<'_, Ev3>, st: &mut State3, e: iced::Event, at: (f32, f32)) -> Option<Ev3> {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    c.update(st, &e, bounds, Cursor::Available(Point::new(at.0, at.1))).and_then(|a| a.into_inner().0)
}

#[test]
fn mouse_drag_becomes_exact_pan_steps() {
    let c = canvas();
    let mut st = State3::default();
    let moved = |x, y| iced::Event::Mouse(mouse::Event::CursorMoved { position: Point::new(x, y) });
    // First event reports the canvas size.
    assert!(matches!(send(&c, &mut st, moved(0.0, 0.0), (0.0, 0.0)), Some(Ev3::Resize(_))));
    assert_eq!(send(&c, &mut st, iced::Event::Mouse(mouse::Event::ButtonPressed(Button::Left)), (100.0, 100.0)), None);
    // Every move publishes exactly the cursor delta since the last one.
    let mut total = (0.0, 0.0);
    for (x, y) in [(130.0, 90.0), (131.0, 95.0), (180.0, 140.0), (60.0, 20.0)] {
        match send(&c, &mut st, moved(x, y), (x, y)) {
            Some(Ev3::Look { dx, dy } | Ev3::Move { dx, dy }) => {
                total.0 += dx;
                total.1 += dy;
                assert_eq!((100.0 + total.0, 100.0 + total.1), (x, y), "the pan follows the cursor exactly");
            }
            other => panic!("expected a drag step, got {other:?}"),
        }
    }
    assert_eq!(send(&c, &mut st, iced::Event::Mouse(mouse::Event::ButtonReleased(Button::Left)), (60.0, 20.0)), Some(Ev3::End), "a drag ends, it is not a click");
    // The wheel zooms at the cursor (iso).
    match send(&c, &mut st, iced::Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 } }), (200.0, 450.0)) {
        Some(Ev3::Zoom { y, fx, fy }) => assert_eq!((y, fx, fy), (1.0, 0.25, 0.75)),
        other => panic!("expected zoom, got {other:?}"),
    }
}

#[test]
fn panning_moves_every_tile_by_the_drag() {
    let layer = Layer { rot: 1, scale: 2.0, vscale: 1.0, textured: false, px: TILE as u32, sky: [0; 3], pack: 0 };
    let tiles = IsoTiles::for_test(layer, &[(0, 0), (1, 0), (0, 1), (-1, -1)]);
    let mut cam = omt_render::iso::IsoCam { x: 3.5, z: -2.25, rot: 1, scale: 2.0, vscale: 1.0 };
    let view = (2400.0, 2400.0);
    let sorted = |c: &omt_render::iso::IsoCam| {
        let mut v: Vec<(f32, f32, f32)> = tiles.draw_list(c.center_px(2.0), view).into_iter().map(|t| (t.0, t.1, t.2)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        v
    };
    let before = sorted(&cam);
    assert_eq!(before.len(), 4);
    let mut drag = (0.0f32, 0.0f32);
    for (dx, dy) in [(37.0, -12.0), (-5.0, 3.0), (250.0, 180.0), (0.4, 0.4), (0.4, 0.4)] {
        cam.pan(dx, dy);
        drag = (drag.0 + dx, drag.1 + dy);
        let after = sorted(&cam);
        assert_eq!(after.len(), 4);
        for (b, a) in before.iter().zip(&after) {
            // Every tile moved by the whole drag (±1 px: whole-pixel snapping
            // keeps tiles crisp), all by the same amount (no seams opening).
            assert!((a.0 - b.0 - drag.0).abs() <= 1.0 && (a.1 - b.1 - drag.1).abs() <= 1.0, "{a:?} vs {b:?} after drag {drag:?}");
            assert_eq!((a.0 - b.0, a.1 - b.1), (after[0].0 - before[0].0, after[0].1 - before[0].1));
            assert_eq!(a.2, TILE, "drawn at native size");
        }
    }
}

/// Regression (user report 2026-11-20): after 3D → Iso or an angle change
/// the old full-view frame stayed on screen and slid with the drag. A mode
/// change must drop the frame, its drag offset, and invalidate renders in
/// flight.
#[test]
fn mode_changes_drop_the_old_frame() {
    let mut v = crate::viewer_3d::View3 { frame: Some(iced::widget::image::Handle::from_rgba(1, 1, vec![0; 4])), iso_pan: (40.0, -12.0), iso_pan_sent: (5.0, 1.0), ..Default::default() };
    let g = v.frame_gen;
    v.clear_frame();
    assert!(v.frame.is_none() && v.alloc.is_none());
    assert_eq!((v.iso_pan, v.iso_pan_sent), ((0.0, 0.0), (0.0, 0.0)));
    assert_ne!(v.frame_gen, g, "frames requested before the change are stale");
}
