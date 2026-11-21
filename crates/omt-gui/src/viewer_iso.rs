//! **Isometric** map mode (uNmINeD "Map isometric"): the loaded map's
//! height / colour field drawn by [`omt_render::iso`] from one of four
//! view angles, as streamed screen tiles (`viewer_isotiles`). Drag pans
//! (the tiles move with the cursor, new ones render just past the window
//! edge), the wheel zooms about the cursor.

use iced::Task;
use omt_render::iso::REF_Y;

use crate::session::Store;
use crate::viewer::{ViewerMsg, ViewerState};
use crate::viewer_3d::Ev3;

/// Isometric zoom bounds (pixels per block).
const MIN_SCALE: f32 = 0.25;
const MAX_SCALE: f32 = 16.0;

impl ViewerState {
    /// Enter isometric mode at `rot` (`None` = back to the 2D map).
    pub(crate) fn set_iso(&mut self, rot: Option<u8>, store: Option<&Store>) -> Task<ViewerMsg> {
        let Some(rot) = rot else {
            self.leave_iso();
            return self.set_3d(false, store);
        };
        let entering = self.v3.iso.is_none();
        if entering {
            let (cx, cz) = if self.v3.on { (self.v3.cam.x, self.v3.cam.z) } else { (self.ctl.camera.cx, self.ctl.camera.cz) };
            self.v3.iso_cam.x = cx;
            self.v3.iso_cam.z = cz;
            self.v3.iso_cam.scale = (self.ctl.camera.zoom * 2.0).clamp(1.0, 4.0);
        }
        // Whatever was on screen (perspective 3D, another angle) goes.
        self.v3.clear_frame();
        self.v3.iso = Some(rot % 4);
        self.v3.iso_cam.rot = rot % 4;
        let detail = if self.v3.blocks && !self.v3.iso_blocks { self.set_detail(false, store) } else { Task::none() };
        let on = if self.v3.on { Task::none() } else { self.set_3d(true, store) };
        // The iso view has no use for the perspective grid / mesh.
        self.v3.grid = None;
        self.v3.mesh = None;
        self.sync_iso();
        let blocks = if self.v3.iso_blocks { Task::batch([self.request_iso_blocks(store), self.request_frame()]) } else { Task::none() };
        Task::batch([on, detail, blocks, self.iso_pump(store)])
    }

    /// Drop the iso tiles and give the map loader its camera back.
    pub(crate) fn leave_iso(&mut self) {
        self.v3.clear_frame();
        self.v3.iso = None;
        self.isot.clear();
        self.ctl.area = None;
    }

    /// Keep the 3D camera on the iso centre.
    pub(crate) fn sync_iso(&mut self) {
        self.v3.cam.x = self.v3.iso_cam.x;
        self.v3.cam.z = self.v3.iso_cam.z;
    }

    /// Mouse / wheel input in iso mode (`None` = not in iso mode).
    pub(crate) fn iso_input(&mut self, e: &Ev3, store: Option<&Store>) -> Option<Task<ViewerMsg>> {
        self.v3.iso?;
        match *e {
            Ev3::Look { dx, dy } | Ev3::Move { dx, dy } => {
                // A pure translation of the tile layer: the tiles follow the
                // cursor exactly, the pump streams in the newly exposed edge.
                let t0 = std::time::Instant::now();
                // The camera moves against the drag.
                let v = &mut self.isot.vel;
                *v = (v.0 * 0.6 - dx * 0.4, v.1 * 0.6 - dy * 0.4);
                self.v3.iso_cam.pan(dx, dy);
                self.v3.iso_pan.0 += dx;
                self.v3.iso_pan.1 += dy;
                self.sync_iso();
                let blocks = if self.v3.iso_blocks { self.request_frame() } else { Task::none() };
                let pump = self.iso_pump(store);
                let missing = self.iso_missing();
                self.isot.drag_record(t0.elapsed(), missing);
                Some(Task::batch([blocks, pump]))
            }
            Ev3::End => {
                self.isot.vel = (0.0, 0.0);
                Some(self.iso_settle(store))
            }
            Ev3::Zoom { y, fx, fy } => {
                let f = if y > 0.0 { 1.25 } else { 0.8 };
                let old = self.v3.iso_cam.scale;
                let new = (old * f).clamp(MIN_SCALE, MAX_SCALE);
                if new == old {
                    return Some(Task::none());
                }
                // Zoom about the cursor: the global point under it stays put.
                let (w, h) = (self.v3.size.0 as f64, self.v3.size.1 as f64);
                let (ax, ay) = ((fx as f64 - 0.5) * w, (fy as f64 - 0.5) * h);
                let c = self.v3.iso_cam.center_px(old as f64);
                let k = (new / old) as f64;
                self.v3.iso_cam.scale = new;
                self.v3.iso_cam.set_center_px(new as f64, ((c.0 + ax) * k - ax, (c.1 + ay) * k - ay));
                self.sync_iso();
                Some(self.iso_settle(store))
            }
            Ev3::Altitude(y) => self.iso_input(&Ev3::Zoom { y, fx: 0.5, fy: 0.5 }, store),
            Ev3::Tick => match self.isot.drag_step() {
                Some((dx, dy)) => self.iso_input(&Ev3::Move { dx, dy }, store),
                None => Some(Task::none()),
            },
            Ev3::Resize(_) => None,
            _ => Some(Task::none()),
        }
    }

    /// After a drag / zoom: stream tiles and, in 3D-blocks detail, rebuild
    /// the block mesh once the view left it and render a frame.
    fn iso_settle(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        let blocks = if self.v3.iso_blocks {
            let (bx, bz) = self.v3.iso_bcenter;
            let off = ((self.v3.iso_cam.x - bx).powi(2) + (self.v3.iso_cam.z - bz).powi(2)).sqrt();
            let mesh = if off > self.prefs.block_distance as f64 * 0.4 { self.request_iso_blocks(store) } else { Task::none() };
            Task::batch([mesh, self.request_frame()])
        } else {
            Task::none()
        };
        Task::batch([blocks, self.iso_pump(store)])
    }

    /// Iso detail: 0 colors, 1 textured (CPU painter), 2 GPU 3D blocks.
    pub(crate) fn set_iso_detail(&mut self, d: u8, store: Option<&Store>) -> Task<ViewerMsg> {
        let textured = d >= 1;
        let blocks = d == 2 && omt_gpu::gpu().is_some();
        if d == 2 && !blocks {
            self.status = "3D blocks need a GPU — showing the textured painter".into();
        }
        let mut tasks = Vec::new();
        self.v3.iso_textured = textured;
        if blocks != self.v3.iso_blocks {
            self.v3.iso_blocks = blocks;
            self.v3.blocks = blocks;
            if blocks {
                tasks.push(self.request_iso_blocks(store));
            } else {
                self.v3.bgrid = None;
                self.v3.bmesh = None;
            }
        }
        self.v3.clear_frame();
        tasks.push(self.iso_pump(store));
        Task::batch(tasks)
    }

    /// Build the GPU block mesh around the iso centre.
    fn request_iso_blocks(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        let ground = self.ctl.map.as_ref().and_then(|m| crate::viewer_3d::Snapshot::of(m).sample(self.v3.iso_cam.x, self.v3.iso_cam.z)).map_or(64.0, |(h, _)| h);
        // The block window spans 140 below to 60 above this height.
        self.v3.cam.y = ground + 50.0;
        self.v3.iso_bcenter = (self.v3.iso_cam.x, self.v3.iso_cam.z);
        self.request_blocks(store)
    }

    /// A GPU 3D-blocks frame job for the iso view (`None` = the tiles are
    /// the whole picture).
    pub(crate) fn iso_frame(&self, sky: [u8; 3]) -> Option<Box<dyn FnOnce() -> iced::widget::image::Handle + Send>> {
        self.v3.iso?;
        if !self.v3.iso_blocks {
            return None;
        }
        let mesh = self.v3.bmesh.clone()?;
        // Orthographic camera matching the tile painter's framing: a
        // block's top diagonal spans 2·scale logical pixels, the view
        // centre sits at the reference height.
        let (w, h) = self.render_px(1.0);
        let logical_h = self.v3.size.1.max(1) as f32;
        let ortho = logical_h / (2.0 * std::f32::consts::SQRT_2 * self.v3.iso_cam.scale);
        let yaw = (self.v3.iso_cam.rot as f32 * 90.0 - 45.0).to_radians();
        let cam = omt_render::voxel::Cam3 { x: self.v3.iso_cam.x, z: self.v3.iso_cam.z, y: REF_Y as f32, yaw, pitch: 0.0, far: 1.0e6, ortho, ..Default::default() };
        let skyc = (sky, sky);
        Some(Box::new(move || match omt_gpu::gpu() {
            Some(g) => {
                // The block window is only a patch of the view: key its sky
                // out so the isometric tiles show all around it.
                let mut px = g.render_blocks(&mesh.0, &cam, w, h, skyc);
                for p in px.as_chunks_mut::<4>().0 {
                    if p[0].abs_diff(sky[0]) <= 3 && p[1].abs_diff(sky[1]) <= 3 && p[2].abs_diff(sky[2]) <= 3 {
                        *p = [0; 4];
                    }
                }
                iced::widget::image::Handle::from_rgba(w, h, px)
            }
            None => iced::widget::image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
        }))
    }
}
