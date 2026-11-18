//! Map Viewer **3D terrain** mode: the loaded (styled, sun-lit) map tiles
//! around the camera become a height field (`omt_render::voxel::Grid`,
//! built off the UI thread) that is ray-cast into a perspective view
//! (`omt_render::voxel::render`, column-parallel). Turning and moving
//! inside the grid only re-renders; walking far re-centers tile loading
//! and rebuilds the grid. Controls: left-drag look, right/middle-drag
//! move, wheel altitude, W/A/S/D move, Q/E altitude.

use std::collections::HashMap;
use std::sync::Arc;

use iced::mouse::{self, Button, Cursor, ScrollDelta};
use iced::widget::canvas::{self, Frame, Geometry, Text};
use iced::widget::image::Handle;
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};
use omt_render::TileKey;
use omt_render::voxel::{Cam3, Grid};

use crate::mapdata::MapData;
use crate::viewer_3d_input::Key3;

/// Copy of the loaded tiles' heights + styled colors (thread-safe).
pub struct Snapshot {
    tiles: HashMap<TileKey, (u32, Vec<i16>, Vec<u8>)>,
    /// Interned surface block names (per finest tile, one table).
    names: Vec<String>,
    surfaces: HashMap<TileKey, Vec<u16>>,
}

impl Snapshot {
    pub fn of(map: &MapData) -> Self {
        let mut names: Vec<String> = Vec::new();
        let mut intern: HashMap<String, u16> = HashMap::new();
        let mut id = |n: &str| -> u16 {
            *intern.entry(n.to_string()).or_insert_with(|| {
                names.push(n.to_string());
                (names.len() - 1) as u16
            })
        };
        let mut tiles = HashMap::new();
        let mut surfaces = HashMap::new();
        for (k, t, h, c) in map.tiles.iter().filter_map(|(k, t)| t.field().map(|(h, c)| (*k, t, h, c))).filter(|(_, t, _, _)| t.px > 0) {
            tiles.insert(k, (t.px, h, c));
            if !t.surface.is_empty() && !t.names.is_empty() {
                surfaces.insert(k, t.surface.iter().map(|&s| id(t.names.get(s as usize).map(String::as_str).unwrap_or(""))).collect());
            }
        }
        Self { tiles, names, surfaces }
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Height + color at a block from the finest loaded tile.
    pub fn sample(&self, x: f64, z: f64) -> Option<(f32, [u8; 3])> {
        self.sample_full(x, z).map(|(h, c, _)| (h, c))
    }

    /// [`Sample::sample`](Self::sample) plus the surface block's name, when
    /// the finest tile carrying it is loaded (textured iso).
    pub fn sample_full(&self, x: f64, z: f64) -> Option<(f32, [u8; 3], Option<&str>)> {
        let (bx, bz) = (x.floor() as i64, z.floor() as i64);
        let (cx, cz) = (bx.div_euclid(16) as i32, bz.div_euclid(16) as i32);
        for level in 0..=omt_render::MAX_LEVEL {
            let key = TileKey::containing(level, cx, cz);
            let Some((px, h, c)) = self.tiles.get(&key) else { continue };
            let bpp = key.span_blocks() / *px as i64;
            let (x0, z0, _, _) = key.chunk_rect();
            let lx = (bx - x0 as i64 * 16).div_euclid(bpp);
            let lz = (bz - z0 as i64 * 16).div_euclid(bpp);
            if lx < 0 || lz < 0 || lx >= *px as i64 || lz >= *px as i64 {
                continue;
            }
            let i = (lz * *px as i64 + lx) as usize;
            let hh = *h.get(i)?;
            if hh == i16::MIN || c.get(i * 4 + 3).copied().unwrap_or(0) == 0 {
                return None;
            }
            let name = self.surfaces.get(&key).and_then(|s| s.get(i).copied()).and_then(|id| self.names.get(id as usize)).map(String::as_str);
            return Some((hh as f32, [c[i * 4], c[i * 4 + 1], c[i * 4 + 2]], name));
        }
        None
    }
}

/// Grid cells per edge (CPU ray-casting).
pub const GRID_N: usize = 768;
/// Grid cells per edge when the GPU renders the terrain.
pub const GRID_N_GPU: usize = 1024;

/// A GPU terrain mesh (messages need `Debug`).
#[derive(Clone)]
pub struct GpuMesh(pub Arc<omt_gpu::Mesh>);

impl std::fmt::Debug for GpuMesh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuMesh({} columns)", self.0.columns)
    }
}

/// 3D mode state.
#[derive(Default)]
pub struct View3 {
    pub on: bool,
    pub cam: Cam3,
    pub grid: Option<Arc<Grid>>,
    pub frame: Option<Handle>,
    pub building: bool,
    pub rendering: bool,
    /// A newer grid / frame was requested while one was being built.
    pub grid_dirty: bool,
    pub frame_dirty: bool,
    /// Canvas size in logical pixels.
    pub size: (usize, usize),
    /// Window scale factor (0 = not known yet → 1).
    pub dpi: f32,
    /// Keeps the shown frame's GPU upload alive (dropping the previous
    /// frame's allocation frees it: one frame resident at a time).
    pub alloc: Option<iced::widget::image::Allocation>,
    /// "Blocks" detail: ray-cast real blocks instead of the height field.
    pub blocks: bool,
    pub bgrid: Option<Arc<omt_render::blocks3d::BlockGrid>>,
    pub bbuilding: bool,
    pub bdirty: bool,
    /// Place the camera on the ground when the next block grid arrives.
    pub settle: bool,
    /// Render terrain on the GPU when one is available.
    pub gpu: bool,
    pub mesh: Option<GpuMesh>,
    /// GPU textured block mesh (Blocks detail).
    pub bmesh: Option<GpuBlocks>,
    /// Vertical look angle (radians, + up); `cam.pitch` derives from it.
    pub look: f32,
    /// Held movement keys.
    pub held: crate::viewer_3d_input::Held,
    /// Mouse captured (Mouse look controls).
    pub locked: bool,
    /// Last movement tick.
    pub last_tick: Option<std::time::Instant>,
    /// Isometric mode: view angle (0..3); `None` = perspective 3D.
    pub iso: Option<u8>,
    pub iso_cam: omt_render::iso::IsoCam,
    /// Iso "Textured" detail (block looks instead of flat map colors).
    pub iso_textured: bool,
    /// Screen-space drag offset accumulated since the shown iso frame was
    /// rendered: during a pan the frame is drawn shifted by this, and the
    /// next full render (on release / big offset) resets it.
    pub iso_pan: (f32, f32),
}

/// A GPU block mesh (messages need `Debug`).
#[derive(Clone)]
pub struct GpuBlocks(pub Arc<omt_gpu::blocks::BlockMesh>);

impl std::fmt::Debug for GpuBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuBlocks({} faces)", self.0.faces)
    }
}

impl View3 {
    /// Turn the heading, kept in 0..2π.
    pub fn yaw_add(&mut self, d: f32) {
        self.cam.yaw = (self.cam.yaw + d).rem_euclid(std::f32::consts::TAU);
    }

    /// Whether the camera wandered far enough from the grid center that
    /// tiles should be re-centered and the grid rebuilt.
    pub fn needs_recenter(&self) -> bool {
        if self.blocks {
            return match &self.bgrid {
                Some(g) => {
                    let (gx, gz) = g.center();
                    ((self.cam.x - gx).powi(2) + (self.cam.z - gz).powi(2)).sqrt() > g.size[0] as f64 / 2.0 * 0.45
                }
                None => false,
            };
        }
        match &self.grid {
            Some(g) => {
                let (gx, gz) = g.center();
                ((self.cam.x - gx).powi(2) + (self.cam.z - gz).powi(2)).sqrt() > g.radius() * 0.45
            }
            None => false,
        }
    }
}

/// Sky colors for a style (night is dark).
pub fn sky(night: bool) -> ([u8; 3], [u8; 3]) {
    if night { ([4, 6, 18], [24, 34, 66]) } else { ([62, 120, 210], [176, 208, 236]) }
}

/// Input from the 3D canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ev3 {
    Look { dx: f32, dy: f32 },
    Move { dx: f32, dy: f32 },
    Altitude(f32),
    KeyDown(crate::viewer_3d_input::Key3),
    KeyUp(crate::viewer_3d_input::Key3),
    /// Capture / release the mouse (Mouse look).
    Capture(bool),
    /// The window lost focus (release everything).
    Unfocused,
    /// Per-frame movement tick.
    Tick,
    Resize(Size),
    /// A click without dragging, as fractions of the canvas (0..1).
    Click { fx: f32, fy: f32 },
    /// A drag ended (button released).
    End,
}

pub struct Canvas3<'a, M> {
    pub frame: Option<&'a Handle>,
    /// Frame is at native resolution: draw it without filtering.
    pub crisp: bool,
    /// Mouse-look controls (click captures) instead of drag controls.
    pub mouse_look: bool,
    /// The mouse is captured.
    pub locked: bool,
    /// Isometric mode (drag pans, wheel zooms).
    pub iso: bool,
    /// Iso drag offset: the shown frame is drawn shifted by this while a
    /// fresh render is pending (keeps panning glued to the cursor).
    pub iso_pan: (f32, f32),
    pub status: String,
    pub on_event: Box<dyn Fn(Ev3) -> M + 'a>,
}

#[derive(Debug, Default)]
pub struct State3 {
    drag: Option<(Point, bool)>,
    size: Option<Size>,
    /// Press position + whether the pointer moved since (click detection).
    press: Option<(Point, bool)>,
    /// Last cursor position while captured (fallback look).
    last: Option<Point>,
}

fn wheel(delta: &ScrollDelta) -> f32 {
    match delta {
        ScrollDelta::Lines { y, .. } => *y,
        ScrollDelta::Pixels { y, .. } => *y / 40.0,
    }
}

/// Minecraft-layout movement keys (arrows too).
fn key3(key: &iced::keyboard::Key) -> Option<Key3> {
    use iced::keyboard::{Key, key::Named};
    Some(match key {
        Key::Named(Named::Space) => Key3::Up,
        Key::Named(Named::Shift) => Key3::Down,
        Key::Named(Named::Control) => Key3::Sprint,
        Key::Named(Named::ArrowUp) => Key3::Forward,
        Key::Named(Named::ArrowDown) => Key3::Back,
        Key::Named(Named::ArrowLeft) => Key3::Left,
        Key::Named(Named::ArrowRight) => Key3::Right,
        Key::Character(c) => match c.as_str().to_ascii_lowercase().as_str() {
            "w" => Key3::Forward,
            "s" => Key3::Back,
            "a" => Key3::Left,
            "d" => Key3::Right,
            _ => return None,
        },
        _ => return None,
    })
}

impl<M> Canvas3<'_, M> {
    /// Keys act while the pointer is over the view (or captured); key
    /// releases always pass so nothing sticks.
    fn key(&self, e: &iced::keyboard::Event, over: bool) -> Option<canvas::Action<M>> {
        use iced::keyboard::{Event, Key, key::Named};
        match e {
            Event::KeyPressed { key: Key::Named(Named::Escape), .. } if self.locked => Some(canvas::Action::publish((self.on_event)(Ev3::Capture(false))).and_capture()),
            Event::KeyPressed { key, .. } if over || self.locked => Some(canvas::Action::publish((self.on_event)(Ev3::KeyDown(key3(key)?))).and_capture()),
            Event::KeyReleased { key, .. } => Some(canvas::Action::publish((self.on_event)(Ev3::KeyUp(key3(key)?)))),
            _ => None,
        }
    }
}

impl<M> canvas::Program<M> for Canvas3<'_, M> {
    type State = State3;

    fn update(&self, st: &mut State3, event: &iced::Event, bounds: Rectangle, cursor: Cursor) -> Option<canvas::Action<M>> {
        let publish = |e: Ev3| Some(canvas::Action::publish((self.on_event)(e)).and_capture());
        if st.size != Some(bounds.size()) {
            st.size = Some(bounds.size());
            return Some(canvas::Action::publish((self.on_event)(Ev3::Resize(bounds.size()))));
        }
        if let iced::Event::Window(iced::window::Event::Unfocused) = event {
            return Some(canvas::Action::publish((self.on_event)(Ev3::Unfocused)));
        }
        if let iced::Event::Keyboard(k) = event {
            return self.key(k, cursor.is_over(bounds));
        }
        if self.locked {
            return match event {
                // Fallback when the platform could not lock the pointer:
                // look with the (hidden) cursor's movement.
                iced::Event::Mouse(mouse::Event::CursorMoved { position }) if !iced_winit::omt_pointer::locked() => {
                    let last = st.last.replace(*position)?;
                    publish(Ev3::Look { dx: position.x - last.x, dy: position.y - last.y })
                }
                iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => publish(Ev3::Altitude(wheel(delta))),
                iced::Event::Mouse(_) => Some(canvas::Action::capture()),
                _ => None,
            };
        }
        st.last = None;
        if self.mouse_look {
            return match event {
                iced::Event::Mouse(mouse::Event::ButtonPressed(Button::Left)) if cursor.is_over(bounds) => publish(Ev3::Capture(true)),
                iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => publish(Ev3::Altitude(wheel(delta))),
                _ => None,
            };
        }
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(b)) => {
                let p = cursor.position_in(bounds)?;
                st.drag = Some((p, *b != Button::Left));
                st.press = (*b == Button::Left).then_some((p, false));
                Some(canvas::Action::capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                st.drag = None;
                let click = st.press.take().and_then(|(p, moved)| {
                    (!moved).then_some(canvas::Action::publish((self.on_event)(Ev3::Click { fx: p.x / bounds.width.max(1.0), fy: p.y / bounds.height.max(1.0) })))
                });
                Some(click.unwrap_or_else(|| canvas::Action::publish((self.on_event)(Ev3::End))))
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (from, moving) = st.drag?;
                let p = cursor.position_in(bounds).or_else(|| cursor.position().map(|q| Point::new(q.x - bounds.x, q.y - bounds.y)))?;
                st.drag = Some((p, moving));
                let (dx, dy) = (p.x - from.x, p.y - from.y);
                if let Some((origin, moved)) = &mut st.press
                    && (p.x - origin.x).abs() + (p.y - origin.y).abs() > 4.0
                {
                    *moved = true;
                }
                publish(if moving { Ev3::Move { dx, dy } } else { Ev3::Look { dx, dy } })
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds)?;
                publish(Ev3::Altitude(wheel(delta)))
            }
            _ => None,
        }
    }

    fn draw(&self, _st: &State3, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: Cursor) -> Vec<Geometry> {
        let size = bounds.size();
        let mut f = Frame::new(renderer, size);
        f.fill_rectangle(Point::ORIGIN, size, crate::style::t().map_bg);
        if let Some(h) = self.frame {
            let mut img = canvas::Image::new(h.clone());
            img.filter_method = if self.crisp { iced::widget::image::FilterMethod::Nearest } else { iced::widget::image::FilterMethod::Linear };
            // While an iso drag is in flight the shown frame is shifted by
            // the accumulated drag (the pending render will catch up).
            let at = if self.iso { Point::new(self.iso_pan.0, self.iso_pan.1) } else { Point::ORIGIN };
            f.draw_image(Rectangle::new(at, size), img);
        }
        let keys = "W A S D move · Space / Shift up / down · Ctrl faster";
        let hint = if self.iso {
            "Drag: pan · Wheel: zoom · view angle in Render settings".to_string()
        } else if self.locked {
            format!("Mouse captured — move to look · {keys} · wheel: speed · Esc: release")
        } else if self.mouse_look {
            format!("Click to capture the mouse · {keys}")
        } else {
            format!("Left-drag: look · Right-drag: move · Wheel: altitude · {keys}")
        };
        f.fill_rectangle(Point::new(0.0, size.height - 22.0), Size::new(size.width, 22.0), Color::from_rgba(0.0, 0.0, 0.0, 0.55));
        f.fill_text(Text {
            content: if self.status.is_empty() { hint } else { format!("{} · {hint}", self.status) },
            position: Point::new(8.0, size.height - 18.0),
            color: Color::from_rgb8(0xe8, 0xea, 0xed),
            size: iced::Pixels(12.0),
            ..Text::default()
        });
        if self.locked {
            // Crosshair.
            let c = Point::new(size.width / 2.0, size.height / 2.0);
            let w = Color::from_rgba(1.0, 1.0, 1.0, 0.85);
            f.fill_rectangle(Point::new(c.x - 7.0, c.y - 1.0), Size::new(14.0, 2.0), w);
            f.fill_rectangle(Point::new(c.x - 1.0, c.y - 7.0), Size::new(2.0, 14.0), w);
        }
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, st: &State3, bounds: Rectangle, cursor: Cursor) -> mouse::Interaction {
        if self.locked {
            mouse::Interaction::Hidden
        } else if st.drag.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            if self.mouse_look { mouse::Interaction::Crosshair } else { mouse::Interaction::Grab }
        } else {
            mouse::Interaction::default()
        }
    }
}
