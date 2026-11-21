//! Map Viewer 3D camera input, game-style:
//! * **Controls** — *Drag* (left-drag look, right-drag move) or *Mouse
//!   look* (click the view to capture the mouse like in game; Esc
//!   releases). Mouse up looks up.
//! * **Movement** — *Normal* (the camera stays above the ground / block
//!   tops) or *Spectator* (fly through blocks).
//! * Keys (Minecraft layout): W A S D move, Space up, Shift down, Ctrl
//!   faster; held keys move smoothly on a per-frame tick. In mouse look
//!   the wheel changes the fly speed, like spectator mode.

use std::time::{Duration, Instant};

use iced::{Subscription, Task};
use serde::{Deserialize, Serialize};

use crate::session::Store;
use crate::viewer::{ViewerMsg, ViewerState};
use crate::viewer_3d::Ev3;

/// How the mouse drives the 3D camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Controls3 {
    /// Left-drag looks, right/middle-drag moves.
    #[default]
    Drag,
    /// Click to capture the mouse; moving it looks around (Esc releases).
    MouseLook,
}

/// Collision behaviour of the 3D camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Move3 {
    /// Stay above the terrain / block tops.
    #[default]
    Normal,
    /// Fly through everything.
    Spectator,
}

/// Movement keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key3 {
    Forward,
    Back,
    Left,
    Right,
    Up,
    Down,
    Sprint,
}

/// Held movement keys (bit set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held(u8);

impl Held {
    pub fn set(&mut self, k: Key3, down: bool) {
        let bit = 1u8 << k as u8;
        if down { self.0 |= bit } else { self.0 &= !bit }
    }

    pub fn has(self, k: Key3) -> bool {
        self.0 & (1 << k as u8) != 0
    }

    /// Any key that moves the camera (Sprint alone does not).
    pub fn moving(self) -> bool {
        self.0 & !(1 << Key3::Sprint as u8) != 0
    }

    /// (forward, right, up) in −1..=1.
    pub fn axes(self) -> (f64, f64, f32) {
        let a = |p: Key3, n: Key3| (self.has(p) as i8 - self.has(n) as i8) as f64;
        (a(Key3::Forward, Key3::Back), a(Key3::Right, Key3::Left), a(Key3::Up, Key3::Down) as f32)
    }
}

/// Radians of look per logical pixel of drag / per raw motion count.
const DRAG_LOOK: f32 = 0.006;
const RAW_LOOK: f32 = 0.0028;
/// Vertical look limit (±85°).
const MAX_LOOK: f32 = 1.48;

impl ViewerState {
    /// Derive the camera's horizon offset (`Cam3::pitch`) from the look
    /// angle. The GPU renders a true perspective (any angle up to ±85°);
    /// the CPU ray-casters only shift the horizon, so their range is
    /// clamped and the angle follows.
    pub(crate) fn sync_pitch(&mut self) {
        let (w, h) = (self.v3.size.0.max(1) as f32, self.v3.size.1.max(1) as f32);
        let focal = (w / 2.0) / (self.v3.cam.fov / 2.0).tan();
        self.v3.look = self.v3.look.clamp(-MAX_LOOK, MAX_LOOK);
        let mut p = self.v3.look.tan() * focal / h;
        if !self.v3.gpu {
            p = p.clamp(-0.45, 0.7);
            self.v3.look = (p * h / focal).atan();
        }
        self.v3.cam.pitch = p;
    }

    fn turn(&mut self, dx: f32, dy: f32, k: f32) {
        let k = k * self.prefs.look_sens.clamp(0.1, 5.0);
        self.v3.yaw_add(dx * k);
        // Mouse up (negative dy) looks up.
        self.v3.look -= dy * k;
        self.sync_pitch();
    }

    /// Ground under the camera (block tops in Blocks detail).
    fn ground(&self) -> f32 {
        match (&self.v3.bgrid, self.v3.blocks) {
            (Some(b), true) => b.top_at(self.v3.cam.x.floor() as i32, self.v3.cam.z.floor() as i32).map_or(f32::MIN, |t| t as f32 + 1.0),
            _ => self.v3.grid.as_ref().and_then(|g| g.at(self.v3.cam.x, self.v3.cam.z)).map_or(f32::MIN, |(h, _)| h),
        }
    }

    /// Keep the camera inside its limits (above the ground in Normal).
    fn clamp_height(&mut self) {
        let floor = match self.prefs.movement {
            Move3::Normal => {
                let g = self.ground();
                if g == f32::MIN { -100.0 } else { g + if self.v3.blocks { 1.8 } else { 3.0 } }
            }
            Move3::Spectator => -100.0,
        };
        self.v3.cam.y = self.v3.cam.y.clamp(floor, 1200.0);
    }

    /// Grab / release the mouse (Mouse look controls).
    pub(crate) fn capture(&mut self, on: bool) {
        let on = on && self.prefs.controls == Controls3::MouseLook && self.v3.on;
        if on != self.v3.locked {
            iced_winit::omt_pointer::request(on);
        }
        self.v3.locked = on;
        if !on {
            self.v3.held = Held::default();
        }
        self.v3.last_tick = None;
    }

    /// Per-frame tick while keys are held or the mouse is captured.
    pub fn subscription(&self) -> Subscription<ViewerMsg> {
        if self.v3.iso.is_some() && self.isot.drag_step().is_some() {
            // Synthetic 60 Hz drag (`OMT_DEBUG_ISO_DRAG`): ticks become
            // drag steps in `iso_input`.
            return iced::time::every(Duration::from_millis(16)).map(|_| ViewerMsg::Ev3(Ev3::Tick));
        }
        if self.v3.on && (self.v3.locked || self.v3.held.moving()) {
            iced::time::every(Duration::from_millis(16)).map(|_| ViewerMsg::Ev3(Ev3::Tick))
        } else {
            Subscription::none()
        }
    }

    pub(crate) fn on_ev3(&mut self, e: Ev3, store: Option<&Store>) -> Task<ViewerMsg> {
        // Old feel for drag-moving and the wheel: faster when high up.
        let above = (self.v3.cam.y - self.ground().max(-100.0)).max(16.0);
        let speed = (above / 160.0) as f64;
        if let Some(t) = self.iso_input(&e, store) {
            return t;
        }
        match e {
            Ev3::Look { dx, dy } => self.turn(dx, dy, DRAG_LOOK),
            Ev3::Move { dx, dy } => self.v3.cam.walk(dy as f64 * speed * 2.0, -dx as f64 * speed * 2.0),
            Ev3::Altitude(y) | Ev3::Zoom { y, .. } => {
                if self.v3.locked {
                    // Mouse look: the wheel sets the fly speed.
                    self.prefs.fly_speed = (self.prefs.fly_speed * if y > 0.0 { 1.2 } else { 1.0 / 1.2 }).clamp(2.0, 400.0);
                    return Task::none();
                }
                self.v3.cam.y += y * 10.0;
            }
            Ev3::KeyDown(k) => {
                self.v3.held.set(k, true);
                return Task::none();
            }
            Ev3::KeyUp(k) => {
                self.v3.held.set(k, false);
                return Task::none();
            }
            Ev3::End => return Task::none(),
            Ev3::Capture(on) => {
                self.capture(on);
                return Task::none();
            }
            Ev3::Unfocused => {
                self.capture(false);
                self.v3.held = Held::default();
                return Task::none();
            }
            Ev3::Tick => {
                let now = Instant::now();
                let dt = self.v3.last_tick.map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f32()).min(0.1);
                self.v3.last_tick = Some(now);
                let mut changed = false;
                if self.v3.locked {
                    let (dx, dy) = iced_winit::omt_pointer::take_motion();
                    if dx != 0.0 || dy != 0.0 {
                        self.turn(dx as f32, dy as f32, RAW_LOOK);
                        changed = true;
                    }
                }
                let h = self.v3.held;
                if h.moving() {
                    let (f, r, u) = h.axes();
                    let mut v = self.prefs.fly_speed * if h.has(Key3::Sprint) { 2.5 } else { 1.0 };
                    if !self.v3.blocks {
                        // Terrain view spans kilometres: scale with height.
                        v *= (above / 16.0).max(1.0);
                    }
                    let step = (v * dt) as f64;
                    let norm = if f != 0.0 && r != 0.0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                    self.v3.cam.walk(f * step * norm, r * step * norm);
                    self.v3.cam.y += u * step as f32;
                    changed = true;
                }
                if !changed {
                    return Task::none();
                }
            }
            Ev3::Click { .. } => return Task::none(),
            Ev3::Resize(s) => {
                self.v3.size = (s.width.max(32.0) as usize, s.height.max(24.0) as usize);
                self.sync_pitch();
                // The window may have moved to a screen with another scale.
                return Task::batch([crate::viewer_3d_ctl::fetch_dpi(), self.request_frame(), self.iso_pump(store)]);
            }
        }
        self.clamp_height();
        if self.v3.needs_recenter() {
            if self.v3.blocks {
                return Task::batch([self.request_blocks(store), self.request_frame()]);
            }
            return Task::batch([self.recenter_3d(store), self.request_frame()]);
        }
        self.request_frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_keys_and_axes() {
        let mut h = Held::default();
        assert!(!h.moving());
        h.set(Key3::Sprint, true);
        assert!(!h.moving(), "sprint alone does not move");
        h.set(Key3::Forward, true);
        h.set(Key3::Left, true);
        h.set(Key3::Up, true);
        assert_eq!(h.axes(), (1.0, -1.0, 1.0));
        h.set(Key3::Back, true);
        assert_eq!(h.axes().0, 0.0, "opposite keys cancel");
        h.set(Key3::Forward, false);
        h.set(Key3::Back, false);
        h.set(Key3::Left, false);
        h.set(Key3::Up, false);
        assert!(!h.moving());
    }
}
