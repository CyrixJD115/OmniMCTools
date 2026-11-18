//! **Isometric** map mode (uNmINeD "Map isometric"): the 3D view's height /
//! colour grid drawn by [`omt_render::iso`] from one of four view angles.
//! Drag pans, the wheel zooms, the grid follows the view.

use iced::Task;
use omt_render::iso::IsoCam;

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
            self.v3.iso = None;
            return self.set_3d(false, store);
        };
        let entering = self.v3.iso.is_none();
        self.v3.iso = Some(rot % 4);
        self.v3.iso_cam.rot = rot % 4;
        if entering {
            let (cx, cz) = if self.v3.on { (self.v3.cam.x, self.v3.cam.z) } else { (self.ctl.camera.cx, self.ctl.camera.cz) };
            self.v3.iso_cam.x = cx;
            self.v3.iso_cam.z = cz;
            self.v3.iso_cam.scale = (self.ctl.camera.zoom * 2.0).clamp(1.0, 4.0);
        }
        let detail = if self.v3.blocks { self.set_detail(false, store) } else { Task::none() };
        let on = if self.v3.on { Task::none() } else { self.set_3d(true, store) };
        self.sync_iso();
        Task::batch([on, detail, self.recenter_3d(store), self.request_frame()])
    }

    /// Keep the 3D camera (grid centre, tile loading) on the iso centre.
    pub(crate) fn sync_iso(&mut self) {
        self.v3.cam.x = self.v3.iso_cam.x;
        self.v3.cam.z = self.v3.iso_cam.z;
    }

    /// Grid half-width for the iso view: what the canvas shows, bounded.
    pub(crate) fn iso_radius(&self) -> f64 {
        let (w, h) = self.v3.size;
        ((w.max(h) as f64) / self.v3.iso_cam.scale as f64).clamp(64.0, 3000.0)
    }

    /// Mouse / wheel input in iso mode (`None` = not in iso mode).
    pub(crate) fn iso_input(&mut self, e: &Ev3, store: Option<&Store>) -> Option<Task<ViewerMsg>> {
        self.v3.iso?;
        match *e {
            Ev3::Look { dx, dy } | Ev3::Move { dx, dy } => {
                self.v3.iso_cam.pan(dx, dy);
            }
            Ev3::Altitude(y) => {
                let f = if y > 0.0 { 1.25 } else { 0.8 };
                self.v3.iso_cam.scale = (self.v3.iso_cam.scale * f).clamp(MIN_SCALE, MAX_SCALE);
            }
            Ev3::Resize(_) => return None,
            _ => return Some(Task::none()),
        }
        self.sync_iso();
        // Rebuild the grid when the view leaves the built area or the zoom
        // changed the needed extent a lot.
        let need = self.iso_radius();
        let regrid = match &self.v3.grid {
            Some(g) => {
                let (gx, gz) = (g.x0 + g.radius(), g.z0 + g.radius());
                let off = ((self.v3.iso_cam.x - gx).powi(2) + (self.v3.iso_cam.z - gz).powi(2)).sqrt();
                off > g.radius() * 0.35 || need > g.radius() * 1.2 || need < g.radius() * 0.3
            }
            None => true,
        };
        let grid = if regrid { self.recenter_3d(store) } else { Task::none() };
        Some(Task::batch([grid, self.request_frame()]))
    }

    /// An iso frame job for the current grid.
    pub(crate) fn iso_frame(&self, sky: [u8; 3]) -> Option<Box<dyn FnOnce() -> iced::widget::image::Handle + Send>> {
        self.v3.iso?;
        let grid = self.v3.grid.clone()?;
        let (w, h) = self.render_px(1.0);
        let cam: IsoCam = self.v3.iso_cam;
        Some(Box::new(move || iced::widget::image::Handle::from_rgba(w, h, omt_render::iso::render(&grid, &cam, w as usize, h as usize, sky))))
    }
}
