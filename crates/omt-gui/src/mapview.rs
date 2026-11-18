//! The 2D world viewer canvas (Pruner + Chunk Locator). The parent owns the
//! [`Camera`] and all tool logic; this widget draws tiles + overlays and
//! reports pointer input in **block coordinates** as [`MapEvent`]s.
//!
//! Controls (UMT-like): wheel = zoom at cursor, middle/right drag = pan,
//! left = tool input, Ctrl held = disable chunk snapping.

use iced::mouse::{self, Button, Cursor, ScrollDelta};
use iced::widget::canvas::{self, Frame, Geometry};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use crate::mapdata::MapData;

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
    /// Far enough out for any world (level-of-detail tiles keep it cheap).
    pub const MIN_ZOOM: f32 = 1.0 / 4096.0;
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

    /// Block-space viewport rectangle (min/max world coords on screen).
    pub fn visible_blocks(self, view: Size) -> (f64, f64, f64, f64) {
        let (x0, z0) = self.to_block(view, Point::ORIGIN);
        let (x1, z1) = self.to_block(view, Point::new(view.width, view.height));
        (x0.min(x1), z0.min(z1), x0.max(x1), z0.max(z1))
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
    /// The canvas size changed (tiles are planned for the real viewport).
    Resize(Size),
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
    /// Labeled pins (spawn, players, bookmarks).
    pub markers: Vec<Marker>,
    /// Chunk grid: `None` = automatic (when zoomed in), else forced.
    pub chunk_grid: Option<bool>,
    /// Region (32×32 chunk) grid.
    pub region_grid: bool,
    /// Slime chunks to tint.
    pub slime: Vec<(i32, i32)>,
    /// Scale bar + compass.
    pub hud: bool,
    /// Textured cells: (cell x, cell z, handle), `textured_cell` blocks wide.
    pub textured: Vec<(i32, i32, iced::widget::image::Handle)>,
    pub textured_cell: i32,
    /// Rulers along the top/left edges.
    pub ruler: crate::mapview_overlay::Ruler,
    /// Distance measurement (from, to) in blocks.
    pub measure: Option<[(f64, f64); 2]>,
    /// Cursor position (block coordinates) — marked on the rulers.
    pub cursor: Option<(f64, f64)>,
}

/// A labeled map pin.
#[derive(Debug, Clone)]
pub struct Marker {
    pub x: f64,
    pub z: f64,
    pub label: String,
    pub icon: Option<iced::widget::image::Handle>,
    pub color: Color,
}

/// The map canvas as an element, clipped to its bounds (zoomed-in tiles
/// extend far past the viewport; unclipped they would paint over the UI).
///
/// Overlays (grids, selections, pins, HUD, rulers) are a second canvas
/// stacked on top: renderers paint a layer's images after its vector
/// shapes, so overlays drawn into the tiles' own layer ended up *under*
/// the map.
pub fn element<'a, M: 'a>(mut mv: MapView<'a, M>) -> iced::Element<'a, M> {
    let mut ov = std::mem::take(&mut mv.overlay);
    mv.overlay.textured = std::mem::take(&mut ov.textured);
    mv.overlay.textured_cell = ov.textured_cell;
    let top = crate::mapview_overlay::OverlayView { overlay: ov, camera: mv.camera };
    let fill = iced::Length::Fill;
    iced::widget::container(iced::widget::stack![iced::widget::canvas(mv).width(fill).height(fill), iced::widget::canvas(top).width(fill).height(fill)]).clip(true).into()
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
    size: Option<Size>,
}


impl<M> canvas::Program<M> for MapView<'_, M> {
    type State = ViewState;

    fn update(&self, state: &mut ViewState, event: &iced::Event, bounds: Rectangle, cursor: Cursor) -> Option<canvas::Action<M>> {
        let size = bounds.size();
        if let iced::Event::Window(_) = event
            && state.size != Some(size)
        {
            state.size = Some(size);
            return Some(canvas::Action::publish((self.on_event)(MapEvent::Resize(size))));
        }
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
        // Layering: iced paints a geometry's images after its vector
        // content AND batches images by filter mode, so call order inside one
        // frame is NOT paint order (smooth coarse tiles used to cover crisp
        // full-detail ones). Geometries are painted in order, so: background,
        // then one geometry per detail level (coarsest first), then overlays.
        let mut geoms = Vec::new();
        let mut bg = Frame::new(renderer, size);
        bg.fill_rectangle(Point::ORIGIN, size, crate::style::t().map_bg);
        geoms.push(bg.into_geometry());
        let cam = self.camera;
        let smooth = crate::mapdata::config().smooth;
        let mut layer: Option<(u8, Frame)> = None;
        for (key, tile) in self.map.draw_list() {
            let Some(handle) = &tile.handle else { continue };
            let (x0, z0, _, _) = key.chunk_rect();
            let tile_screen = (key.span_blocks() as f64 * cam.zoom as f64) as f32;
            let p = cam.to_screen(size, x0 as f64 * 16.0, z0 as f64 * 16.0);
            if p.x > size.width || p.y > size.height || p.x + tile_screen < 0.0 || p.y + tile_screen < 0.0 {
                continue;
            }
            if layer.as_ref().is_none_or(|(l, _)| *l != key.level) {
                if let Some((_, f)) = layer.take() {
                    geoms.push(f.into_geometry());
                }
                layer = Some((key.level, Frame::new(renderer, size)));
            }
            // Crisp blocks when magnified at full detail; smooth when the
            // image is minified or shows reduced (sampled) detail.
            let minified = tile_screen < tile.px as f32;
            let mut img = canvas::Image::new(handle.clone());
            img.filter_method = if minified || (smooth && tile.reduced()) {
                iced::widget::image::FilterMethod::Linear
            } else {
                iced::widget::image::FilterMethod::Nearest
            };
            img.snap = true;
            if let Some((_, f)) = layer.as_mut() {
                f.draw_image(Rectangle::new(p, Size::new(tile_screen, tile_screen)), img);
            }
        }
        if let Some((_, f)) = layer.take() {
            geoms.push(f.into_geometry());
        }
        if !self.overlay.textured.is_empty() {
            let mut tf = Frame::new(renderer, size);
            let span = self.overlay.textured_cell as f64;
            let edge = (span * cam.zoom as f64) as f32;
            for (cx, cz, h) in &self.overlay.textured {
                let p = cam.to_screen(size, *cx as f64 * span, *cz as f64 * span);
                if p.x > size.width || p.y > size.height || p.x + edge < 0.0 || p.y + edge < 0.0 {
                    continue;
                }
                let mut img = canvas::Image::new(h.clone());
                img.filter_method = iced::widget::image::FilterMethod::Nearest;
                img.snap = true;
                tf.draw_image(Rectangle::new(p, Size::new(edge, edge)), img);
            }
            geoms.push(tf.into_geometry());
        }

        geoms
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
