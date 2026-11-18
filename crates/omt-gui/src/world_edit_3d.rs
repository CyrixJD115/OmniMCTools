//! World Editor 3D preview: the real blocks around the selection
//! (`viewer_blocks::build`) ray-cast with the selection tinted, from an
//! orbit camera (drag: orbit, wheel: zoom, right-drag: pan, W/A/S/D/Q/E).
//! Rebuilt after every edit so results show in 3D immediately.

use std::sync::Arc;

use iced::Task;
use iced::widget::image::Handle;
use omt_render::blocks3d::BlockGrid;
use omt_render::voxel::Cam3;

use crate::session::{Store, blocking};
use crate::viewer_3d::Ev3;
use crate::world_edit::{EditMsg, EditorState};

/// Preview state.
#[derive(Default)]
pub struct Preview {
    pub on: bool,
    pub grid: Option<Arc<BlockGrid>>,
    pub frame: Option<Handle>,
    /// Keeps the shown frame uploaded (see `viewer_3d_ctl::request_frame`).
    pub alloc: Option<iced::widget::image::Allocation>,
    pub building: bool,
    pub rendering: bool,
    /// Camera moved while a frame was rendering.
    pub pending: bool,
    pub dirty: bool,
    /// Orbit: target, yaw, elevation (radians), distance.
    pub target: [f64; 3],
    pub yaw: f32,
    pub elev: f32,
    pub dist: f32,
    pub size: (usize, usize),
    /// The next 3D pick sets the second corner.
    pub pick_second: bool,
    /// Debug/testing hook (`OMT_DEBUG_PICK=fx,fy;fx,fy`): clicks applied
    /// once the first frame is shown.
    pub debug_picks: Option<String>,
    pub debug_used: Option<()>,
}

impl Preview {
    fn cam(&self, w: usize, h: usize) -> Cam3 {
        let fov = 1.1f32;
        let (s, c) = self.yaw.sin_cos();
        let horiz = (self.dist * self.elev.cos()) as f64;
        let mut cam = Cam3 { x: self.target[0] - s as f64 * horiz, z: self.target[2] + c as f64 * horiz, y: self.target[1] as f32 + self.dist * self.elev.sin(), yaw: self.yaw, fov, ..Default::default() };
        // Put the target at the image center: horizon offset for the
        // downward look angle.
        let focal = (w as f32 / 2.0) / (fov / 2.0).tan();
        cam.pitch = -self.elev.tan() * focal / h.max(1) as f32;
        cam
    }
}

impl EditorState {
    pub(crate) fn set_preview(&mut self, on: bool, store: Option<&Store>) -> Task<EditMsg> {
        self.preview.on = on;
        if !on {
            return Task::none();
        }
        let Some(sel) = self.sel() else {
            self.status = "select an area to preview it in 3D".into();
            self.preview.on = false;
            return Task::none();
        };
        let s = sel.size();
        self.preview.target = [(sel.min[0] + sel.max[0]) as f64 / 2.0 + 0.5, (sel.min[1] + sel.max[1]) as f64 / 2.0, (sel.min[2] + sel.max[2]) as f64 / 2.0 + 0.5];
        self.preview.dist = (s[0].max(s[2]).max(s[1]) as f32 * 2.6).clamp(24.0, 180.0);
        self.preview.elev = 0.6;
        self.preview.yaw = 0.6;
        if self.preview.size == (0, 0) {
            self.preview.size = (((self.ctl.view.width * 2.0 / 3.0) as usize).max(64), ((self.ctl.view.height * 2.0 / 3.0) as usize).max(48));
        }
        self.preview.grid = None;
        self.rebuild_preview(store)
    }

    /// Re-read the blocks around the selection (after edits, too).
    pub(crate) fn rebuild_preview(&mut self, store: Option<&Store>) -> Task<EditMsg> {
        if !self.preview.on {
            return Task::none();
        }
        if self.preview.building {
            self.preview.dirty = true;
            return Task::none();
        }
        let Some(store) = store.cloned() else { return Task::none() };
        self.preview.building = true;
        let (dim, t) = (self.ctl.dim, self.preview.target);
        Task::perform(blocking(move || crate::viewer_blocks::build(&store, dim, t[0], t[2], t[1] as f32 + 100.0).map(Arc::new)), EditMsg::PGrid)
    }

    pub(crate) fn on_pgrid(&mut self, r: Result<Arc<BlockGrid>, String>, store: Option<&Store>) -> Task<EditMsg> {
        self.preview.building = false;
        match r {
            Ok(g) => self.preview.grid = Some(g),
            Err(e) => self.status = format!("3D preview failed: {e}"),
        }
        let again = if std::mem::take(&mut self.preview.dirty) { self.rebuild_preview(store) } else { Task::none() };
        Task::batch([self.render_preview(), again])
    }

    pub(crate) fn render_preview(&mut self) -> Task<EditMsg> {
        let Some(g) = self.preview.grid.clone() else { return Task::none() };
        if self.preview.rendering {
            self.preview.pending = true;
            return Task::none();
        }
        self.preview.rendering = true;
        let (w, h) = self.preview.size;
        let cam = self.preview.cam(w, h);
        let hl = self.sel().map(|s| (s.min, s.max));
        Task::perform(blocking(move || Handle::from_rgba(w as u32, h as u32, omt_render::blocks3d::render_hl(&g, &cam, w, h, crate::viewer_3d::sky(false), hl))), |h| h).then(|h: Handle| {
            let shown = h.clone();
            iced::widget::image::allocate(h).map(move |a| EditMsg::PFrame(shown.clone(), a.ok()))
        })
    }

    pub(crate) fn on_pframe(&mut self, h: Handle, alloc: Option<iced::widget::image::Allocation>) -> Task<EditMsg> {
        self.preview.rendering = false;
        self.preview.frame = Some(h);
        self.preview.alloc = alloc;
        if self.preview.grid.is_some()
            && let Some(spec) = self.preview.debug_picks.take().or_else(|| std::env::var("OMT_DEBUG_PICK").ok().filter(|_| !self.preview.pick_second && self.a.is_some() && self.preview.debug_used.is_none()))
        {
            self.preview.debug_used = Some(());
            let mut last = Task::none();
            for pair in spec.split(';') {
                let v: Vec<f32> = pair.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() == 2 {
                    last = self.pick_3d(v[0], v[1]);
                }
            }
            return last;
        }
        if std::mem::take(&mut self.preview.pending) { self.render_preview() } else { Task::none() }
    }

    /// Click in the preview: pick the block and set a selection corner.
    fn pick_3d(&mut self, fx: f32, fy: f32) -> Task<EditMsg> {
        let Some(g) = self.preview.grid.clone() else { return Task::none() };
        let (w, h) = self.preview.size;
        let cam = self.preview.cam(w, h);
        let Some(c) = omt_render::blocks3d::pick(&g, &cam, w, h, fx * w as f32, fy * h as f32) else {
            self.status = "no block under the cursor".into();
            return Task::none();
        };
        if self.preview.pick_second {
            self.b = Some((c[0], c[2]));
            let a_y = self.y.0.min(self.y.1);
            self.y = (a_y.min(c[1]), a_y.max(c[1]));
            self.status = format!("corner 2 at {} {} {} — selection set", c[0], c[1], c[2]);
        } else {
            self.a = Some((c[0], c[2]));
            self.b = Some((c[0], c[2]));
            self.y = (c[1], c[1]);
            self.status = format!("corner 1 at {} {} {} — click the opposite corner", c[0], c[1], c[2]);
        }
        self.preview.pick_second = !self.preview.pick_second;
        self.sync_corners();
        self.render_preview()
    }

    pub(crate) fn on_pev(&mut self, e: Ev3) -> Task<EditMsg> {
        let p = &mut self.preview;
        let pan = |p: &mut Preview, fwd: f64, right: f64| {
            let (s, c) = (p.yaw as f64).sin_cos();
            p.target[0] += s * fwd + c * right;
            p.target[2] += -c * fwd + s * right;
        };
        match e {
            Ev3::Look { dx, dy } => {
                p.yaw += dx * 0.01;
                // Orbit: dragging up looks up (lowers the orbit), like the viewer.
                p.elev = (p.elev + dy * 0.006).clamp(0.05, 1.5);
            }
            Ev3::Move { dx, dy } => {
                let k = p.dist as f64 / 300.0;
                pan(p, dy as f64 * k, -dx as f64 * k);
            }
            Ev3::Altitude(y) => p.dist = (p.dist * if y > 0.0 { 0.85 } else { 1.18 }).clamp(6.0, 220.0),
            Ev3::KeyDown(k) => {
                use crate::viewer_3d_input::Key3;
                match k {
                    Key3::Forward => pan(p, 4.0, 0.0),
                    Key3::Back => pan(p, -4.0, 0.0),
                    Key3::Left => pan(p, 0.0, -4.0),
                    Key3::Right => pan(p, 0.0, 4.0),
                    Key3::Up => p.target[1] += 2.0,
                    Key3::Down => p.target[1] -= 2.0,
                    Key3::Sprint => return Task::none(),
                }
            }
            Ev3::KeyUp(_) | Ev3::Capture(_) | Ev3::Unfocused | Ev3::Tick => return Task::none(),
            Ev3::Resize(s) => p.size = (((s.width * 2.0 / 3.0) as usize).max(64), ((s.height * 2.0 / 3.0) as usize).max(48)),
            Ev3::Click { fx, fy } => return self.pick_3d(fx, fy),
        }
        self.render_preview()
    }
}
