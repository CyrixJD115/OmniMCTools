//! Map Viewer 3D mode control: camera input, tile re-centering, and the
//! coalesced background grid builds / frame renders (see `viewer_3d`).

use std::sync::Arc;

use iced::Task;
use iced::widget::image::Handle;
use omt_render::voxel::{Cam3, Grid, render};

use crate::mapview::Camera;
use crate::session::{Store, blocking};
use crate::viewer::{ViewerMsg, ViewerState};
use crate::viewer_3d::{GRID_N, GRID_N_GPU, GpuBlocks, GpuMesh, Snapshot, sky};

impl ViewerState {
    /// Enter / leave the 3D terrain view.
    pub(crate) fn set_3d(&mut self, on: bool, store: Option<&Store>) -> Task<ViewerMsg> {
        if on == self.v3.on {
            return Task::none();
        }
        self.v3.on = on;
        self.v3.clear_frame();
        if !on {
            self.leave_iso();
            self.capture(false);
            self.v3.grid = None;
            self.v3.frame = None;
            self.v3.alloc = None;
            // Back to a sensible 2D zoom around where the 3D camera was.
            self.ctl.camera = Camera { cx: self.v3.cam.x, cz: self.v3.cam.z, zoom: self.ctl.camera.zoom.max(0.5) };
            return self.ctl.refill(store).map(ViewerMsg::Tiles);
        }
        let (cx, cz) = (self.ctl.camera.cx, self.ctl.camera.cz);
        let ground = self.ctl.map.as_ref().and_then(|m| Snapshot::of(m).sample(cx, cz)).map_or(64.0, |(h, _)| h);
        self.v3.cam.x = cx;
        self.v3.cam.z = cz + self.v3.cam.far as f64 * 0.25;
        self.v3.cam.y = ground + 110.0;
        self.v3.cam.yaw = 0.0;
        if self.v3.size == (0, 0) {
            self.v3.size = (self.ctl.view.width as usize, self.ctl.view.height as usize);
        }
        // Look a little down at the landscape.
        self.v3.look = -0.2;
        self.sync_pitch();
        Task::batch([self.recenter_3d(store), fetch_dpi()])
    }

    /// Point the 2D tile engine at the area around the 3D camera (so the
    /// tiles the grid needs get loaded) and rebuild the grid.
    pub(crate) fn recenter_3d(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        if self.v3.iso.is_some() {
            return self.iso_pump(store);
        }
        let span = 2.2 * self.v3.cam.far as f64;
        let px = self.ctl.view.width.min(self.ctl.view.height).max(1.0) as f64;
        self.ctl.camera = Camera { cx: self.v3.cam.x, cz: self.v3.cam.z, zoom: (px / span) as f32 };
        let t = self.ctl.refill(store).map(ViewerMsg::Tiles);
        Task::batch([t, self.request_grid()])
    }

    pub(crate) fn request_grid(&mut self) -> Task<ViewerMsg> {
        // The isometric view streams its own tiles.
        if !self.v3.on || self.v3.iso.is_some() {
            return Task::none();
        }
        if self.v3.building {
            self.v3.grid_dirty = true;
            return Task::none();
        }
        let Some(map) = self.ctl.map.as_ref() else { return Task::none() };
        let snap = Snapshot::of(map);
        if snap.is_empty() {
            return Task::none();
        }
        self.v3.building = true;
        let (cx, cz) = (self.v3.cam.x, self.v3.cam.z);
        let r = self.v3.cam.far as f64;
        // Isometric frames are drawn on the CPU: no GPU mesh needed.
        let use_gpu = self.v3.gpu && self.v3.iso.is_none();
        Task::perform(
            blocking(move || {
                let gpu = if use_gpu { omt_gpu::gpu() } else { None };
                let n = if gpu.is_some() { GRID_N_GPU } else { GRID_N };
                // Surface block looks for the textured iso (interned once
                // per name; kinds stay empty when nothing loaded).
                let pack = crate::textures::current();
                let kinds: std::sync::Mutex<Vec<omt_render::blocks3d::Kind>> = std::sync::Mutex::new(Vec::new());
                let seen: std::sync::Mutex<std::collections::HashMap<String, u16>> = std::sync::Mutex::new(std::collections::HashMap::new());
                let mut grid = Grid::build(cx, cz, r, n, Vec::new(), &|x, z| {
                    let (h, c, name) = snap.sample_full(x, z)?;
                    let kid = match name {
                        Some(n) => *seen.lock().unwrap_or_else(|e| e.into_inner()).entry(n.to_string()).or_insert_with(|| {
                            let mut ks = kinds.lock().unwrap_or_else(|e| e.into_inner());
                            ks.push(crate::viewer_blocks::kind_of(n, pack.as_deref()));
                            (ks.len() - 1) as u16
                        }),
                        None => omt_render::voxel::NO_KIND,
                    };
                    Some((h, c, kid))
                });
                grid.kinds = kinds.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let grid = Arc::new(grid);
                let mesh = gpu.map(|g| GpuMesh(Arc::new(g.mesh(&grid))));
                (grid, mesh)
            }),
            |(g, m)| ViewerMsg::Grid3(g, m),
        )
    }

    /// Switch between terrain and block detail.
    pub(crate) fn set_detail(&mut self, blocks: bool, store: Option<&Store>) -> Task<ViewerMsg> {
        if blocks == self.v3.blocks {
            return Task::none();
        }
        self.v3.blocks = blocks;
        if blocks {
            // Come down closer for block detail.
            let ground = self.v3.grid.as_ref().and_then(|g| g.at(self.v3.cam.x, self.v3.cam.z)).map_or(self.v3.cam.y - 60.0, |(h, _)| h);
            self.v3.cam.y = self.v3.cam.y.min(ground + 40.0);
            self.v3.look = self.v3.look.min(-0.3);
            self.sync_pitch();
            self.v3.settle = true;
            return self.request_blocks(store);
        }
        self.v3.bgrid = None;
        self.request_frame()
    }

    /// Build the block grid around the camera (coalesced).
    pub(crate) fn request_blocks(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        if !self.v3.on || !self.v3.blocks {
            return Task::none();
        }
        if self.v3.bbuilding {
            self.v3.bdirty = true;
            return Task::none();
        }
        let Some(store) = store.cloned() else { return Task::none() };
        self.v3.bbuilding = true;
        let (dim, cx, cz, y) = (self.ctl.dim, self.v3.cam.x, self.v3.cam.z, self.v3.cam.y);
        let use_gpu = self.v3.gpu;
        let dist = self.prefs.block_distance.clamp(32, 512);
        Task::perform(
            blocking(move || {
                let gpu = if use_gpu { omt_gpu::gpu() } else { None };
                let radius = if gpu.is_some() { dist } else { dist.min(crate::viewer_blocks::RADIUS) };
                let grid = Arc::new(crate::viewer_blocks::build_r(&store, dim, cx, cz, y, radius)?);
                let mesh = gpu.map(|g| GpuBlocks(Arc::new(g.block_mesh(&grid))));
                Ok((grid, mesh))
            }),
            ViewerMsg::BGrid,
        )
    }

    pub(crate) fn on_bgrid(&mut self, r: Result<(Arc<omt_render::blocks3d::BlockGrid>, Option<GpuBlocks>), String>, store: Option<&Store>) -> Task<ViewerMsg> {
        self.v3.bbuilding = false;
        match r {
            Ok((g, m)) => {
                self.v3.bmesh = m;
                // First grid after switching to blocks: stand 24 blocks
                // above the real ground under the camera.
                if std::mem::take(&mut self.v3.settle)
                    && let Some(top) = g.top_at(self.v3.cam.x.floor() as i32, self.v3.cam.z.floor() as i32)
                {
                    self.v3.cam.y = top as f32 + 24.0;
                }
                self.v3.bgrid = Some(g);
            }
            Err(e) => self.status = format!("3D blocks failed: {e}"),
        }
        let mut t = vec![self.request_frame()];
        if std::mem::take(&mut self.v3.bdirty) {
            t.push(self.request_blocks(store));
        }
        Task::batch(t)
    }

    /// Render size in pixels for a path that renders at `factor` of the
    /// screen: GPU paths use physical pixels × render quality (sharp on
    /// scaled displays); CPU paths ignore the display scale (cost).
    pub(crate) fn render_px(&self, factor: f32) -> (u32, u32) {
        let dpi = if self.v3.gpu { self.v3.dpi.max(1.0) } else { 1.0 };
        let s = dpi * self.prefs.render_scale.clamp(0.25, 2.0) * factor;
        ((self.v3.size.0 as f32 * s).clamp(32.0, 4096.0) as u32, (self.v3.size.1 as f32 * s).clamp(24.0, 4096.0) as u32)
    }

    /// Render the current view (coalesced: one frame in flight). The
    /// finished frame is uploaded to the UI renderer *before* it replaces
    /// the shown one (`image::allocate`), so moving never flashes blank.
    pub(crate) fn request_frame(&mut self) -> Task<ViewerMsg> {
        if !self.v3.on {
            return Task::none();
        }
        if self.v3.rendering {
            self.v3.frame_dirty = true;
            return Task::none();
        }
        let sky = sky(self.prefs.style.style == omt_render::style::MapStyle::Night);
        let cam = self.v3.cam;
        type Job = Box<dyn FnOnce() -> Handle + Send>;
        type GpuJob = Box<dyn FnOnce(&omt_gpu::Gpu) -> Vec<u8> + Send>;
        if self.v3.iso.is_some() {
            let Some(job) = self.iso_frame(sky.1) else { return Task::none() };
            self.v3.rendering = true;
            self.v3.iso_pan_sent = self.v3.iso_pan;
            let g = self.v3.frame_gen;
            return Task::perform(blocking(job), |h| h).then(move |h: Handle| {
                let shown = h.clone();
                iced::widget::image::allocate(h).map(move |a| ViewerMsg::Frame3(g, shown.clone(), a.ok()))
            });
        }
        let gpu_frame = |f: GpuJob, w: u32, h: u32| -> Job {
            Box::new(move || match omt_gpu::gpu() {
                Some(g) => Handle::from_rgba(w, h, f(&g)),
                None => Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
            })
        };
        let job: Job = if self.v3.blocks {
            if let (true, Some(m)) = (self.v3.gpu, self.v3.bmesh.clone()) {
                // Fog reaches the sky at the block render distance.
                let cam = Cam3 { far: self.prefs.block_distance as f32, ..cam };
                let (w, h) = self.render_px(1.0);
                gpu_frame(Box::new(move |g| g.render_blocks(&m.0, &cam, w, h, sky)), w, h)
            } else {
                let Some(bg) = self.v3.bgrid.clone() else { return Task::none() };
                // Block ray-casting is heavy: a third of the canvas.
                let (w, h) = self.render_px(1.0 / 3.0);
                Box::new(move || Handle::from_rgba(w, h, omt_render::blocks3d::render(&bg, &cam, w as usize, h as usize, sky)))
            }
        } else if let (true, Some(mesh)) = (self.v3.gpu, self.v3.mesh.clone()) {
            let (w, h) = self.render_px(1.0);
            gpu_frame(Box::new(move |g| g.render(&mesh.0, &cam, w, h, sky)), w, h)
        } else {
            let Some(grid) = self.v3.grid.clone() else { return Task::none() };
            let (w, h) = self.render_px(0.5);
            Box::new(move || Handle::from_rgba(w, h, render(&grid, &cam, w as usize, h as usize, sky)))
        };
        self.v3.rendering = true;
        let g = self.v3.frame_gen;
        Task::perform(blocking(job), |h| h).then(move |h: Handle| {
            let shown = h.clone();
            iced::widget::image::allocate(h).map(move |a| ViewerMsg::Frame3(g, shown.clone(), a.ok()))
        })
    }

    pub(crate) fn on_grid(&mut self, g: Arc<Grid>, m: Option<GpuMesh>) -> Task<ViewerMsg> {
        self.v3.building = false;
        self.v3.grid = Some(g);
        self.v3.mesh = m;
        let mut tasks = vec![self.request_frame()];
        if std::mem::take(&mut self.v3.grid_dirty) {
            tasks.push(self.request_grid());
        }
        Task::batch(tasks)
    }

    pub(crate) fn on_frame(&mut self, g: u64, h: Handle, alloc: Option<iced::widget::image::Allocation>) -> Task<ViewerMsg> {
        self.v3.rendering = false;
        // Rendered for a view that has changed since (3D → Iso, another
        // angle or detail): never show it — render the current view instead.
        if g != self.v3.frame_gen || (self.v3.iso.is_some() && !self.v3.iso_blocks) {
            self.v3.frame_dirty = false;
            return self.request_frame();
        }
        self.v3.frame = Some(h);
        // The new frame matches the camera as it was when requested: only
        // the drag since then still offsets it.
        self.v3.iso_pan.0 -= self.v3.iso_pan_sent.0;
        self.v3.iso_pan.1 -= self.v3.iso_pan_sent.1;
        self.v3.iso_pan_sent = (0.0, 0.0);
        // Replacing the allocation releases the previous frame's upload.
        self.v3.alloc = alloc;
        if std::mem::take(&mut self.v3.frame_dirty) { self.request_frame() } else { Task::none() }
    }
}

/// Ask the window for its scale factor (physical / logical pixels).
pub(crate) fn fetch_dpi() -> Task<ViewerMsg> {
    iced::window::latest().and_then(iced::window::scale_factor).map(ViewerMsg::Dpi3)
}
