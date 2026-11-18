//! 3D terrain view by heightfield ray-casting ("voxel space": per screen
//! column, march front-to-back across the height + color field and fill
//! vertical spans above a running y-buffer). CPU-only, column-parallel,
//! cost O(width × steps) independent of world size; distant samples step
//! farther apart (LOD) and fade into the sky.
//!
//! The field is a dense [`Grid`] built once from already-rendered map
//! tiles around the camera, so moving the camera inside the grid and
//! turning never touch the world again.

use rayon::prelude::*;

/// The grid sampler: height, map color and surface kind id per cell.
pub type Sample<'a> = dyn Fn(f64, f64) -> Option<(f32, [u8; 3], u16)> + Sync + 'a;

/// A square height + color field (row-major, `n × n` cells of `cell`
/// blocks, top-left at block (`x0`, `z0`)). Heights `f32::NAN` = empty.
/// `kind[i]` indexes [`Grid::kinds`] (the surface block's look for the
/// textured isometric view); [`Grid::NO_KIND`] = no look recorded.
#[derive(Debug, Clone)]
pub struct Grid {
    pub x0: f64,
    pub z0: f64,
    pub cell: f64,
    pub n: usize,
    pub height: Vec<f32>,
    pub color: Vec<[u8; 3]>,
    pub kind: Vec<u16>,
    pub kinds: Vec<crate::blocks3d::Kind>,
}

/// Kind id for cells with no known surface block.
pub const NO_KIND: u16 = u16::MAX;

impl Grid {
    /// Build by sampling `f(x, z) -> (height, rgb, kind)` at every cell
    /// center (`kind` = index into `kinds`, [`NO_KIND`] = none).
    pub fn build(cx: f64, cz: f64, radius: f64, n: usize, kinds: Vec<crate::blocks3d::Kind>, f: &Sample<'_>) -> Self {
        let cell = (2.0 * radius / n as f64).max(1.0);
        let (x0, z0) = (cx - cell * n as f64 / 2.0, cz - cell * n as f64 / 2.0);
        type Row = (Vec<f32>, Vec<[u8; 3]>, Vec<u16>);
        let rows: Vec<Row> = (0..n)
            .into_par_iter()
            .map(|r| {
                let mut h = Vec::with_capacity(n);
                let mut c = Vec::with_capacity(n);
                let mut k = Vec::with_capacity(n);
                for q in 0..n {
                    let (x, z) = (x0 + (q as f64 + 0.5) * cell, z0 + (r as f64 + 0.5) * cell);
                    match f(x, z) {
                        Some((hh, cc, kk)) => {
                            h.push(hh);
                            c.push(cc);
                            k.push(kk);
                        }
                        None => {
                            h.push(f32::NAN);
                            c.push([0; 3]);
                            k.push(NO_KIND);
                        }
                    }
                }
                (h, c, k)
            })
            .collect();
        let (height, color, kind) = rows.into_iter().fold((Vec::with_capacity(n * n), Vec::with_capacity(n * n), Vec::with_capacity(n * n)), |(mut h, mut c, mut k), (rh, rc, rk)| {
            h.extend(rh);
            c.extend(rc);
            k.extend(rk);
            (h, c, k)
        });
        Self { x0, z0, cell, n, height, color, kind, kinds }
    }

    pub fn at(&self, x: f64, z: f64) -> Option<(f32, [u8; 3])> {
        let q = ((x - self.x0) / self.cell).floor();
        let r = ((z - self.z0) / self.cell).floor();
        if q < 0.0 || r < 0.0 || q >= self.n as f64 || r >= self.n as f64 {
            return None;
        }
        let i = r as usize * self.n + q as usize;
        let h = self.height[i];
        (!h.is_nan()).then(|| (h, self.color[i]))
    }

    /// Covered block range half-width.
    pub fn radius(&self) -> f64 {
        self.cell * self.n as f64 / 2.0
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x0 + self.radius(), self.z0 + self.radius())
    }
}

/// The 3D camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cam3 {
    pub x: f64,
    pub z: f64,
    /// Eye height (Y).
    pub y: f32,
    /// Heading in radians; 0 looks north (−Z), increasing clockwise.
    pub yaw: f32,
    /// Look up/down: horizon offset as a fraction of the image height.
    pub pitch: f32,
    /// Horizontal field of view (radians).
    pub fov: f32,
    /// Max view distance in blocks.
    pub far: f32,
    /// Vertical exaggeration.
    pub scale: f32,
}

impl Default for Cam3 {
    fn default() -> Self {
        Self { x: 0.0, z: 0.0, y: 120.0, yaw: 0.0, pitch: 0.12, fov: 1.2, far: 700.0, scale: 1.0 }
    }
}

impl Cam3 {
    /// Move in the view plane (forward along the heading, right = strafe).
    pub fn walk(&mut self, forward: f64, right: f64) {
        let (s, c) = (self.yaw as f64).sin_cos();
        self.x += s * forward + c * right;
        self.z += -c * forward + s * right;
    }
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t) as u8)
}

/// Render `w × h` RGBA. `sky` = (zenith, horizon).
pub fn render(grid: &Grid, cam: &Cam3, w: usize, h: usize, sky: ([u8; 3], [u8; 3])) -> Vec<u8> {
    let horizon = h as f32 * (0.5 + cam.pitch);
    let focal = (w as f32 / 2.0) / (cam.fov / 2.0).tan();
    // Columns are rendered independently, then interleaved into rows.
    let cols: Vec<Vec<[u8; 3]>> = (0..w)
        .into_par_iter()
        .map(|i| {
            let rel = ((i as f32 + 0.5) / w as f32 - 0.5) * cam.fov;
            let a = cam.yaw + rel;
            let (dx, dz) = (a.sin() as f64, -(a.cos() as f64));
            // Perpendicular distance correction (no fisheye).
            let corr = rel.cos();
            let mut col = vec![[0u8; 3]; h];
            for (y, px) in col.iter_mut().enumerate() {
                let t = (y as f32 / horizon.max(1.0)).clamp(0.0, 1.0);
                *px = mix(sky.0, sky.1, t);
            }
            let mut ybuf = h as f32;
            let mut d = 1.0f32;
            let mut step = 0.25f32;
            while d < cam.far && ybuf > 0.0 {
                let (x, z) = (cam.x + dx * d as f64, cam.z + dz * d as f64);
                if let Some((hh, c)) = grid.at(x, z) {
                    let dist = d * corr;
                    let sy = (cam.y - hh) * cam.scale / dist * focal + horizon;
                    if sy < ybuf {
                        let fog = ((d / cam.far).powf(1.6) * 0.9).clamp(0.0, 0.9);
                        let shade = mix(c, sky.1, fog);
                        let top = sy.max(0.0) as usize;
                        for px in col.iter_mut().take(ybuf as usize).skip(top) {
                            *px = shade;
                        }
                        ybuf = sy.max(0.0);
                    }
                }
                d += step;
                step = (step * 1.012).min(grid.cell as f32 * 4.0).max(0.25);
            }
            col
        })
        .collect();
    let mut out = vec![0u8; w * h * 4];
    for (i, col) in cols.iter().enumerate() {
        for (y, c) in col.iter().enumerate() {
            let o = (y * w + i) * 4;
            out[o..o + 3].copy_from_slice(c);
            out[o + 3] = 255;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hill_ahead_rises_above_flat_ground() {
        // Flat ground at y=60 with a tall red pillar 50 blocks north.
        let f = |x: f64, z: f64| -> Option<(f32, [u8; 3], u16)> {
            if (x.abs() < 6.0) && (z + 50.0).abs() < 6.0 { Some((150.0, [200, 30, 30], NO_KIND)) } else { Some((60.0, [40, 160, 40], NO_KIND)) }
        };
        let g = Grid::build(0.0, 0.0, 128.0, 256, Vec::new(), &f);
        assert_eq!(g.n, 256);
        let cam = Cam3 { y: 90.0, ..Default::default() };
        let (w, h) = (64, 48);
        let img = render(&g, &cam, w, h, ([10, 20, 60], [150, 180, 220]));
        let px = |x: usize, y: usize| [img[(y * w + x) * 4], img[(y * w + x) * 4 + 1], img[(y * w + x) * 4 + 2]];
        // Center column: the pillar is visible above the horizon.
        let horizon = (h as f32 * (0.5 + cam.pitch)) as usize;
        assert!((0..horizon).any(|y| px(w / 2, y)[0] > 120 && px(w / 2, y)[1] < 100), "pillar seen");
        // The edge column misses the pillar: its bottom is ground (green);
        // the top row is sky.
        assert!(px(0, h - 1)[1] > px(0, h - 1)[0]);
        assert_eq!(px(0, 0), [10, 20, 60]);
        // Turning around (yaw π) hides the pillar.
        let back = render(&g, &Cam3 { yaw: std::f32::consts::PI, ..cam }, w, h, ([10, 20, 60], [150, 180, 220]));
        assert!(!(0..horizon).any(|y| back[(y * w + w / 2) * 4] > 120 && back[(y * w + w / 2) * 4 + 1] < 100));
    }

    #[test]
    fn walking_follows_heading() {
        let mut c = Cam3::default();
        c.walk(10.0, 0.0);
        assert!((c.z + 10.0).abs() < 1e-6 && c.x.abs() < 1e-6, "north is -z");
        c.yaw = std::f32::consts::FRAC_PI_2;
        c.walk(10.0, 0.0);
        assert!((c.x - 10.0).abs() < 1e-4, "east is +x");
    }
}
