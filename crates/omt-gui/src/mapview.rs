//! The 2D world viewer canvas (Pruner + Chunk Locator). The parent owns the
//! [`Camera`] and all tool logic; this widget draws tiles + overlays and
//! reports pointer input in **block coordinates** as [`MapEvent`]s.
//!
//! Controls (UMT-like): wheel = zoom at cursor, middle/right drag = pan,
//! left = tool input, Ctrl held = disable chunk snapping.

use iced::mouse::{self, Button, Cursor, ScrollDelta};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use crate::mapdata::{MapData, TILE_PX};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Block coordinates at the viewport center.
    pub cx: f64,
    pub cz: f64,
    /// Screen pixels per block.
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { cx: 0.0, cz: 0.0, zoom: 1.0 }
    }
}

impl Camera {
    pub const MIN_ZOOM: f32 = 1.0 / 16.0;
    pub const MAX_ZOOM: f32 = 32.0;

    pub fn to_screen(self, bounds: Size, bx: f64, bz: f64) -> Point {
        Point::new(
            ((bx - self.cx) * self.zoom as f64) as f32 + bounds.width / 2.0,
            ((bz - self.cz) * self.zoom as f64) as f32 + bounds.height / 2.0,
        )
    }

    pub fn to_block(self, bounds: Size, p: Point) -> (f64, f64) {
        (
            self.cx + ((p.x - bounds.width / 2.0) / self.zoom) as f64,
            self.cz + ((p.y - bounds.height / 2.0) / self.zoom) as f64,
        )
    }

    /// Zoom by `factor` keeping block `at` under the cursor.
    pub fn zoom_at(&mut self, factor: f32, at: (f64, f64)) {
        let new = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let k = (self.zoom / new) as f64;
        self.cx = at.0 + (self.cx - at.0) * k;
        self.cz = at.1 + (self.cz - at.1) * k;
        self.zoom = new;
    }

    /// Fit a chunk-space box into a viewport of roughly `view` pixels.
    pub fn fit(bounds: (i32, i32, i32, i32), view: Size) -> Self {
        let (x0, z0, x1, z1) = bounds;
        let w = ((x1 - x0 + 1) * 16) as f32;
        let h = ((z1 - z0 + 1) * 16) as f32;
        let zoom = (view.width / w).min(view.height / h).clamp(Self::MIN_ZOOM, 4.0);
        // Snap to a power of two for crisp nearest-neighbor tiles.
        let zoom = 2f32.powf(zoom.log2().floor());
        Self { cx: ((x0 + x1 + 1) * 8) as f64, cz: ((z0 + z1 + 1) * 8) as f64, zoom }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MapEvent {
    Pan { dx: f64, dz: f64 },
    Zoom { factor: f32, at: (f64, f64) },
    Press { at: (f64, f64), right: bool, ctrl: bool },
    Drag { at: (f64, f64), ctrl: bool },
    Release { at: (f64, f64), ctrl: bool },
    Hover { at: (f64, f64) },
}

/// A shape outline to draw (block coordinates).
#[derive(Debug, Clone)]
pub struct Outline {
    pub points: Vec<(f64, f64)>,
    pub closed: bool,
    pub active: bool,
    pub handles: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Default)]
pub struct Overlay {
    /// Chunks highlighted red (pruner selection).
    pub red: Vec<(i32, i32)>,
    pub outlines: Vec<Outline>,
    /// A highlighted chunk (locator click / goto).
    pub marker: Option<(i32, i32)>,
    /// Text in the bottom-left corner (cursor coordinates etc.).
    pub caption: String,
}

pub struct MapView<'a, M> {
    pub map: &'a MapData,
    pub camera: Camera,
    pub overlay: Overlay,
    pub on_event: Box<dyn Fn(MapEvent) -> M + 'a>,
}

#[derive(Debug, Default)]
pub struct ViewState {
    pan_from: Option<Point>,
    left_down: bool,
    ctrl: bool,
}

const RED: Color = Color { r: 0.86, g: 0.16, b: 0.16, a: 0.42 };

impl<M> canvas::Program<M> for MapView<'_, M> {
    type State = ViewState;

    fn update(&self, state: &mut ViewState, event: &iced::Event, bounds: Rectangle, cursor: Cursor) -> Option<canvas::Action<M>> {
        let size = bounds.size();
        if let iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) = event {
            state.ctrl = m.control();
            return None;
        }
        let iced::Event::Mouse(mouse_event) = event else { return None };
        let pos = cursor.position_in(bounds);
        let block = |p: Point| self.camera.to_block(size, p);
        let publish = |e: MapEvent| Some(canvas::Action::publish((self.on_event)(e)).and_capture());
        match mouse_event {
            mouse::Event::WheelScrolled { delta } => {
                let p = pos?;
                let y = match delta {
                    ScrollDelta::Lines { y, .. } => *y,
                    ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };
                let factor = if y > 0.0 { 2.0 } else { 0.5 };
                publish(MapEvent::Zoom { factor, at: block(p) })
            }
            mouse::Event::ButtonPressed(b) => {
                let p = pos?;
                match b {
                    Button::Middle => {
                        state.pan_from = Some(p);
                        Some(canvas::Action::capture())
                    }
                    Button::Right => {
                        state.pan_from = Some(p);
                        publish(MapEvent::Press { at: block(p), right: true, ctrl: state.ctrl })
                    }
                    Button::Left => {
                        state.left_down = true;
                        publish(MapEvent::Press { at: block(p), right: false, ctrl: state.ctrl })
                    }
                    _ => None,
                }
            }
            mouse::Event::CursorMoved { .. } => {
                let p = cursor.position_in(bounds).or_else(|| cursor.position().map(|q| Point::new(q.x - bounds.x, q.y - bounds.y)))?;
                if let Some(from) = state.pan_from {
                    state.pan_from = Some(p);
                    let z = self.camera.zoom as f64;
                    return publish(MapEvent::Pan { dx: -((p.x - from.x) as f64) / z, dz: -((p.y - from.y) as f64) / z });
                }
                if state.left_down {
                    return publish(MapEvent::Drag { at: block(p), ctrl: state.ctrl });
                }
                pos.and_then(|p| publish(MapEvent::Hover { at: block(p) }))
            }
            mouse::Event::ButtonReleased(Button::Left) if state.left_down => {
                state.left_down = false;
                let p = cursor.position_in(bounds).unwrap_or(Point::ORIGIN);
                publish(MapEvent::Release { at: block(p), ctrl: state.ctrl })
            }
            mouse::Event::ButtonReleased(Button::Middle | Button::Right) => {
                state.pan_from = None;
                None
            }
            _ => None,
        }
    }

    fn draw(&self, _state: &ViewState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: Cursor) -> Vec<Geometry> {
        let size = bounds.size();
        // Two geometries — iced paints a canvas geometry's images *after* its
        // vector content regardless of draw order, so tiles and overlays must
        // not share one: overlays in their own (later) geometry always land
        // above the rendered map (grid, selection, outlines, caption).
        let mut tiles = Frame::new(renderer, size);
        tiles.fill_rectangle(Point::ORIGIN, size, Color::from_rgb8(0x12, 0x14, 0x18));
        let cam = self.camera;
        let tile_screen = TILE_PX as f32 * cam.zoom;
        for (rx, rz, handle) in &self.map.tiles {
            let p = cam.to_screen(size, (*rx * TILE_PX as i32) as f64, (*rz * TILE_PX as i32) as f64);
            if p.x > size.width || p.y > size.height || p.x + tile_screen < 0.0 || p.y + tile_screen < 0.0 {
                continue;
            }
            let mut img = canvas::Image::new(handle.clone());
            img.filter_method = iced::widget::image::FilterMethod::Nearest;
            img.snap = true;
            tiles.draw_image(Rectangle::new(p, Size::new(tile_screen, tile_screen)), img);
        }

        let mut frame = Frame::new(renderer, size);
        let chunk_px = 16.0 * cam.zoom;
        // Chunk grid when zoomed in enough to be useful.
        if chunk_px >= 24.0 {
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
        if !self.overlay.red.is_empty() {
            let red = Path::new(|b| {
                for &(cx, cz) in &self.overlay.red {
                    let p = cam.to_screen(size, (cx * 16) as f64, (cz * 16) as f64);
                    if p.x > size.width || p.y > size.height || p.x + chunk_px < 0.0 || p.y + chunk_px < 0.0 {
                        continue;
                    }
                    b.rectangle(p, Size::new(chunk_px.max(1.0), chunk_px.max(1.0)));
                }
            });
            frame.fill(&red, RED);
        }
        for o in &self.overlay.outlines {
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
        if let Some((cx, cz)) = self.overlay.marker {
            let p = cam.to_screen(size, (cx * 16) as f64, (cz * 16) as f64);
            let s = Size::new(chunk_px.max(3.0), chunk_px.max(3.0));
            // Fill first so the highlight survives even when the tile is
            // bright; the stroke pins the exact chunk outline.
            frame.fill(&Path::rectangle(p, s), Color::from_rgba(0.13, 0.55, 0.96, 0.30));
            frame.stroke(&Path::rectangle(p, s), Stroke::default().with_color(Color::from_rgb8(0x22, 0x8b, 0xf6)).with_width(2.0));
        }
        if !self.overlay.caption.is_empty() {
            frame.fill_rectangle(Point::new(0.0, size.height - 22.0), Size::new(size.width, 22.0), Color::from_rgba(0.0, 0.0, 0.0, 0.55));
            frame.fill_text(Text {
                content: self.overlay.caption.clone(),
                position: Point::new(8.0, size.height - 18.0),
                color: Color::from_rgb8(0xe8, 0xea, 0xed),
                size: iced::Pixels(13.0),
                ..Text::default()
            });
        }
        vec![tiles.into_geometry(), frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &ViewState, bounds: Rectangle, cursor: Cursor) -> mouse::Interaction {
        if state.pan_from.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_roundtrip_and_zoom_anchor() {
        let mut c = Camera { cx: 100.0, cz: -50.0, zoom: 2.0 };
        let size = Size::new(800.0, 600.0);
        let p = c.to_screen(size, 120.0, -40.0);
        let (bx, bz) = c.to_block(size, p);
        assert!((bx - 120.0).abs() < 1e-3 && (bz + 40.0).abs() < 1e-3);
        c.zoom_at(2.0, (120.0, -40.0));
        let p2 = c.to_screen(size, 120.0, -40.0);
        assert!((p2.x - p.x).abs() < 1e-3 && (p2.y - p.y).abs() < 1e-3, "anchor stays put");
        let f = Camera::fit((-10, -10, 9, 9), Size::new(640.0, 640.0));
        assert_eq!((f.cx, f.cz), (0.0, 0.0));
        assert!(f.zoom <= 2.0);
    }
}
