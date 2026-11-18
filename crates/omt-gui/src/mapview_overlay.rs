//! Map overlays, drawn on their own canvas layer above the tiles: slime
//! chunks, chunk/region grids, selections, highlighted chunk, pins and
//! labels, scale bar + compass, cursor caption and block/chunk/region
//! rulers along the top and left edges.

use iced::mouse::Cursor;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};
use serde::{Deserialize, Serialize};

use crate::mapview::{Camera, Overlay};

const RED: Color = Color { r: 0.86, g: 0.16, b: 0.16, a: 0.42 };

/// Ruler units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Ruler {
    Off,
    #[default]
    Blocks,
    Chunks,
    Regions,
}

impl Ruler {
    pub const ALL: [Ruler; 4] = [Ruler::Off, Ruler::Blocks, Ruler::Chunks, Ruler::Regions];

    pub fn name(self) -> &'static str {
        match self {
            Ruler::Off => "Off",
            Ruler::Blocks => "Blocks",
            Ruler::Chunks => "Chunks",
            Ruler::Regions => "Regions",
        }
    }

    /// Blocks per unit.
    fn unit(self) -> f64 {
        match self {
            Ruler::Chunks => 16.0,
            Ruler::Regions => 512.0,
            _ => 1.0,
        }
    }
}

/// Draw-only canvas program (never captures input).
pub struct OverlayView {
    pub overlay: Overlay,
    pub camera: Camera,
}

impl<M> canvas::Program<M> for OverlayView {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: Cursor) -> Vec<Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        draw_overlay(&mut frame, &self.overlay, size, self.camera);
        vec![frame.into_geometry()]
    }
}

/// A "nice" step in units so that ticks are at least `min_px` apart.
pub fn ruler_step(px_per_unit: f64, min_px: f64) -> f64 {
    let mut step = 1.0;
    for _ in 0..40 {
        for m in [1.0, 2.0, 5.0] {
            if step * m * px_per_unit >= min_px {
                return step * m;
            }
        }
        step *= 10.0;
    }
    step
}

/// Rulers along the top and left edges.
fn draw_rulers(frame: &mut Frame, size: Size, cam: Camera, ruler: Ruler) {
    if ruler == Ruler::Off {
        return;
    }
    const W: f32 = 20.0;
    let unit = ruler.unit();
    let px_per_unit = unit * cam.zoom as f64;
    let step = ruler_step(px_per_unit, 70.0);
    let minor = step / if step >= 5.0 && (step as i64) % 5 == 0 { 5.0 } else { 2.0 };
    let bg = Color::from_rgba(0.04, 0.05, 0.07, 0.78);
    let fg = Color::from_rgb8(0xd8, 0xdc, 0xe2);
    let tick = Color::from_rgba(1.0, 1.0, 1.0, 0.55);
    frame.fill_rectangle(Point::ORIGIN, Size::new(size.width, W), bg);
    frame.fill_rectangle(Point::new(0.0, W), Size::new(W, size.height - W), bg);
    let (x0, z0) = cam.to_block(size, Point::ORIGIN);
    let (x1, z1) = cam.to_block(size, Point::new(size.width, size.height));
    let suffix = match ruler {
        Ruler::Chunks => "c",
        Ruler::Regions => "r",
        _ => "",
    };
    // Horizontal (X) ruler.
    let ticks = Path::new(|b| {
        let mut u = (x0 / unit / minor).floor() * minor;
        while u * unit <= x1 {
            let s = cam.to_screen(size, u * unit, 0.0);
            let major = (u / step).round() * step == u || ((u / step) - (u / step).round()).abs() < 1e-9;
            if s.x > W {
                b.move_to(Point::new(s.x, if major { 4.0 } else { 13.0 }));
                b.line_to(Point::new(s.x, W));
            }
            u += minor;
        }
        let mut v = (z0 / unit / minor).floor() * minor;
        while v * unit <= z1 {
            let s = cam.to_screen(size, 0.0, v * unit);
            let major = ((v / step) - (v / step).round()).abs() < 1e-9;
            if s.y > W {
                b.move_to(Point::new(if major { 4.0 } else { 13.0 }, s.y));
                b.line_to(Point::new(W, s.y));
            }
            v += minor;
        }
    });
    frame.stroke(&ticks, Stroke::default().with_color(tick).with_width(1.0));
    let mut u = (x0 / unit / step).floor() * step;
    while u * unit <= x1 {
        let s = cam.to_screen(size, u * unit, 0.0);
        if s.x > W + 2.0 {
            frame.fill_text(Text { content: format!("{}{suffix}", u as i64), position: Point::new(s.x + 3.0, 2.0), color: fg, size: iced::Pixels(10.0), ..Text::default() });
        }
        u += step;
    }
    let mut v = (z0 / unit / step).floor() * step;
    while v * unit <= z1 {
        let s = cam.to_screen(size, 0.0, v * unit);
        if s.y > W + 2.0 {
            // Vertical text is not available: short labels beside the tick.
            frame.fill_text(Text { content: format!("{}{suffix}", v as i64), position: Point::new(1.0, s.y + 2.0), color: fg, size: iced::Pixels(9.0), ..Text::default() });
        }
        v += step;
    }
    frame.fill_rectangle(Point::ORIGIN, Size::new(W, W), Color::from_rgba(0.04, 0.05, 0.07, 0.95));
    frame.fill_text(Text { content: match ruler { Ruler::Chunks => "ch", Ruler::Regions => "rg", _ => "bl" }.into(), position: Point::new(3.0, 4.0), color: tick, size: iced::Pixels(10.0), ..Text::default() });
}

/// Everything above the tiles.
pub fn draw_overlay(frame: &mut Frame, ov: &Overlay, size: Size, cam: Camera) {
        let chunk_px = 16.0 * cam.zoom;
        // Slime chunks.
        if !ov.slime.is_empty() {
            let slime = Path::new(|b| {
                for &(cx, cz) in &ov.slime {
                    let p = cam.to_screen(size, (cx * 16) as f64, (cz * 16) as f64);
                    b.rectangle(p, Size::new(chunk_px.max(1.0), chunk_px.max(1.0)));
                }
            });
            frame.fill(&slime, Color::from_rgba(0.35, 0.95, 0.35, 0.28));
        }
        // Chunk grid when zoomed in enough to be useful (or forced).
        let show_chunks = ov.chunk_grid.unwrap_or(chunk_px >= 24.0) && chunk_px >= 4.0;
        if ov.region_grid && chunk_px * 32.0 >= 6.0 {
            let (x0, z0) = cam.to_block(size, Point::ORIGIN);
            let (x1, z1) = cam.to_block(size, Point::new(size.width, size.height));
            let grid = Path::new(|b| {
                let mut x = (x0 / 512.0).floor() * 512.0;
                while x <= x1 {
                    let s = cam.to_screen(size, x, 0.0);
                    b.move_to(Point::new(s.x, 0.0));
                    b.line_to(Point::new(s.x, size.height));
                    x += 512.0;
                }
                let mut z = (z0 / 512.0).floor() * 512.0;
                while z <= z1 {
                    let s = cam.to_screen(size, 0.0, z);
                    b.move_to(Point::new(0.0, s.y));
                    b.line_to(Point::new(size.width, s.y));
                    z += 512.0;
                }
            });
            frame.stroke(&grid, Stroke::default().with_color(Color::from_rgba(0.25, 0.85, 0.95, 0.35)).with_width(1.0));
        }
        if show_chunks {
            let (x0, z0) = cam.to_block(size, Point::ORIGIN);
            let (x1, z1) = cam.to_block(size, Point::new(size.width, size.height));
            let grid = Path::new(|b| {
                let mut x = (x0 / 16.0).floor() * 16.0;
                while x <= x1 {
                    let s = cam.to_screen(size, x, 0.0);
                    b.move_to(Point::new(s.x, 0.0));
                    b.line_to(Point::new(s.x, size.height));
                    x += 16.0;
                }
                let mut z = (z0 / 16.0).floor() * 16.0;
                while z <= z1 {
                    let s = cam.to_screen(size, 0.0, z);
                    b.move_to(Point::new(0.0, s.y));
                    b.line_to(Point::new(size.width, s.y));
                    z += 16.0;
                }
            });
            frame.stroke(&grid, Stroke::default().with_color(Color::from_rgba(1.0, 1.0, 1.0, 0.08)).with_width(1.0));
        }
        if !ov.red.is_empty() {
            let red = Path::new(|b| {
                for &(cx, cz) in &ov.red {
                    let p = cam.to_screen(size, (cx * 16) as f64, (cz * 16) as f64);
                    if p.x > size.width || p.y > size.height || p.x + chunk_px < 0.0 || p.y + chunk_px < 0.0 {
                        continue;
                    }
                    b.rectangle(p, Size::new(chunk_px.max(1.0), chunk_px.max(1.0)));
                }
            });
            frame.fill(&red, RED);
        }
        for o in &ov.outlines {
            if o.points.len() >= 2 {
                let path = Path::new(|b| {
                    let first = cam.to_screen(size, o.points[0].0, o.points[0].1);
                    b.move_to(first);
                    for p in &o.points[1..] {
                        b.line_to(cam.to_screen(size, p.0, p.1));
                    }
                    if o.closed {
                        b.close();
                    }
                });
                let color = if o.active { Color::from_rgb8(0xff, 0xd0, 0x40) } else { Color::from_rgba(1.0, 1.0, 1.0, 0.7) };
                frame.stroke(&path, Stroke::default().with_color(color).with_width(if o.active { 2.0 } else { 1.0 }));
            }
            for h in &o.handles {
                let p = cam.to_screen(size, h.0, h.1);
                frame.fill(&Path::circle(p, 5.0), Color::from_rgb8(0xff, 0xff, 0xff));
                frame.stroke(&Path::circle(p, 5.0), Stroke::default().with_color(Color::from_rgb8(0x22, 0x8b, 0xf6)).with_width(2.0));
            }
        }
        if let Some((cx, cz)) = ov.marker {
            let p = cam.to_screen(size, (cx * 16) as f64, (cz * 16) as f64);
            let s = Size::new(chunk_px.max(3.0), chunk_px.max(3.0));
            // Fill first so the highlight survives even when the tile is
            // bright; the stroke pins the exact chunk outline.
            frame.fill(&Path::rectangle(p, s), Color::from_rgba(0.13, 0.55, 0.96, 0.30));
            frame.stroke(&Path::rectangle(p, s), Stroke::default().with_color(Color::from_rgb8(0x22, 0x8b, 0xf6)).with_width(2.0));
        }
        // Labels: kept inside the canvas and never overlapping (dense areas
        // show pins only).
        let mut placed: Vec<Rectangle> = Vec::new();
        for m in &ov.markers {
            let p = cam.to_screen(size, m.x, m.z);
            if p.x < -40.0 || p.y < -40.0 || p.x > size.width + 40.0 || p.y > size.height + 40.0 {
                continue;
            }
            match &m.icon {
                Some(h) => {
                    let mut img = canvas::Image::new(h.clone());
                    img.filter_method = iced::widget::image::FilterMethod::Nearest;
                    frame.fill(&Path::circle(Point::new(p.x, p.y - 12.0), 14.0), Color::from_rgba(0.0, 0.0, 0.0, 0.45));
                    frame.draw_image(Rectangle::new(Point::new(p.x - 10.0, p.y - 22.0), Size::new(20.0, 20.0)), img);
                }
                None => {
                    frame.fill(&Path::circle(p, 5.0), m.color);
                    frame.stroke(&Path::circle(p, 5.0), Stroke::default().with_color(Color::BLACK).with_width(1.5));
                }
            }
            let w = m.label.chars().count() as f32 * 6.6 + 10.0;
            let rect = Rectangle::new(Point::new(p.x - w / 2.0, p.y + 4.0), Size::new(w, 17.0));
            let inside = rect.x >= 0.0 && rect.y >= 0.0 && rect.x + rect.width <= size.width && rect.y + rect.height <= size.height - 24.0;
            if !m.label.is_empty() && inside && !placed.iter().any(|r| r.intersects(&rect)) {
                placed.push(rect);
                frame.fill_rectangle(Point::new(p.x - w / 2.0, p.y + 4.0), Size::new(w, 17.0), Color::from_rgba(0.0, 0.0, 0.0, 0.62));
                frame.fill_text(Text {
                    content: m.label.clone(),
                    position: Point::new(p.x, p.y + 6.0),
                    color: m.color,
                    size: iced::Pixels(12.0),
                    align_x: iced::alignment::Horizontal::Center.into(),
                    ..Text::default()
                });
            }
        }
        if ov.hud {
            draw_hud(frame, size, cam);
        }
        if !ov.caption.is_empty() {
            frame.fill_rectangle(Point::new(0.0, size.height - 22.0), Size::new(size.width, 22.0), Color::from_rgba(0.0, 0.0, 0.0, 0.55));
            frame.fill_text(Text {
                content: ov.caption.clone(),
                position: Point::new(8.0, size.height - 18.0),
                color: Color::from_rgb8(0xe8, 0xea, 0xed),
                size: iced::Pixels(13.0),
                ..Text::default()
            });
        }
    if let Some([a, b]) = ov.measure {
        draw_measure(frame, size, cam, a, b);
    }
    draw_rulers(frame, size, cam, ov.ruler);
    if let (Some((x, z)), true) = (ov.cursor, ov.ruler != Ruler::Off) {
        // Cursor markers on both rulers.
        let p = cam.to_screen(size, x, z);
        let c = Color::from_rgb8(0xff, 0xd2, 0x4a);
        if p.x > 20.0 {
            frame.fill(&Path::new(|b| {
                b.move_to(Point::new(p.x - 5.0, 0.0));
                b.line_to(Point::new(p.x + 5.0, 0.0));
                b.line_to(Point::new(p.x, 9.0));
                b.close();
            }), c);
        }
        if p.y > 20.0 {
            frame.fill(&Path::new(|b| {
                b.move_to(Point::new(0.0, p.y - 5.0));
                b.line_to(Point::new(0.0, p.y + 5.0));
                b.line_to(Point::new(9.0, p.y));
                b.close();
            }), c);
        }
    }
}

/// Measured distance (blocks).
pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// A measuring line with end caps and a distance label.
fn draw_measure(frame: &mut Frame, size: Size, cam: Camera, a: (f64, f64), b: (f64, f64)) {
    let (pa, pb) = (cam.to_screen(size, a.0, a.1), cam.to_screen(size, b.0, b.1));
    let c = Color::from_rgb8(0xff, 0xd2, 0x4a);
    let line = Path::new(|p| {
        p.move_to(pa);
        p.line_to(pb);
    });
    frame.stroke(&line, Stroke::default().with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.6)).with_width(4.0));
    frame.stroke(&line, Stroke::default().with_color(c).with_width(2.0));
    for p in [pa, pb] {
        frame.fill(&Path::circle(p, 4.0), c);
    }
    let d = distance(a, b);
    let label = format!("{:.0} blocks · {:.1} chunks · ΔX {:.0} ΔZ {:.0}", d, d / 16.0, b.0 - a.0, b.1 - a.1);
    let mid = Point::new((pa.x + pb.x) / 2.0 + 8.0, (pa.y + pb.y) / 2.0 - 22.0);
    let w = label.chars().count() as f32 * 6.6 + 12.0;
    frame.fill_rectangle(mid, Size::new(w, 18.0), Color::from_rgba(0.04, 0.05, 0.07, 0.85));
    frame.fill_text(Text { content: label, position: Point::new(mid.x + 6.0, mid.y + 3.0), color: c, size: iced::Pixels(12.0), ..Text::default() });
}

/// Scale bar (bottom-right) and a north arrow (top-right).
fn draw_hud(frame: &mut Frame, size: Size, cam: Camera) {
    // Pick a round length in blocks that is 80–200 px long.
    let mut blocks = 1.0f64;
    for step in [1.0, 2.0, 5.0].iter().cycle().take(36).scan(1.0f64, |mag, &s| {
        let v = s * *mag;
        if s == 5.0 {
            *mag *= 10.0;
        }
        Some(v)
    }) {
        blocks = step;
        if step * cam.zoom as f64 >= 80.0 {
            break;
        }
    }
    let px = (blocks * cam.zoom as f64) as f32;
    let (x1, y) = (size.width - 16.0, size.height - 40.0);
    let x0 = x1 - px;
    frame.fill_rectangle(Point::new(x0 - 8.0, y - 20.0), Size::new(px + 16.0, 30.0), Color::from_rgba(0.0, 0.0, 0.0, 0.5));
    let bar = Path::new(|b| {
        b.move_to(Point::new(x0, y - 4.0));
        b.line_to(Point::new(x0, y + 2.0));
        b.line_to(Point::new(x1, y + 2.0));
        b.line_to(Point::new(x1, y - 4.0));
    });
    frame.stroke(&bar, Stroke::default().with_color(Color::WHITE).with_width(2.0));
    let label = if blocks >= 1000.0 { format!("{} k blocks", blocks / 1000.0) } else { format!("{blocks} blocks") };
    frame.fill_text(Text { content: label, position: Point::new(x0, y - 18.0), color: Color::WHITE, size: iced::Pixels(12.0), ..Text::default() });
    // North arrow.
    let c = Point::new(size.width - 30.0, 30.0);
    frame.fill(&Path::circle(c, 18.0), Color::from_rgba(0.0, 0.0, 0.0, 0.5));
    let arrow = Path::new(|b| {
        b.move_to(Point::new(c.x, c.y - 13.0));
        b.line_to(Point::new(c.x + 6.0, c.y + 2.0));
        b.line_to(Point::new(c.x - 6.0, c.y + 2.0));
        b.close();
    });
    frame.fill(&arrow, Color::from_rgb8(0xe5, 0x48, 0x4d));
    frame.fill_text(Text { content: "N".into(), position: Point::new(c.x - 4.0, c.y + 1.0), color: Color::WHITE, size: iced::Pixels(12.0), ..Text::default() });
}


#[cfg(test)]
mod tests {
    #[test]
    fn ruler_steps_are_round() {
        assert_eq!(super::ruler_step(1.0, 70.0), 100.0);
        assert_eq!(super::ruler_step(16.0, 70.0), 5.0);
        assert_eq!(super::ruler_step(0.01, 70.0), 10_000.0);
        assert_eq!(super::ruler_step(40.0, 70.0), 2.0);
    }
}
