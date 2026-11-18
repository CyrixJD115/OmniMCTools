//! **Isometric map** (uNmINeD's "Map isometric"): the 3D view's height /
//! colour grid drawn in a 2:1 isometric projection from one of four view
//! angles. Painter's algorithm in rotated grid space: back to front, each
//! cell is a top diamond plus its two visible side faces down to the front
//! neighbours. CPU only, so it works without a GPU.

use crate::voxel::Grid;

/// Isometric camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsoCam {
    /// Block coordinates at the image centre.
    pub x: f64,
    pub z: f64,
    /// View angle: 0 = looking north-west (north up-left), then clockwise
    /// in 90° steps.
    pub rot: u8,
    /// Screen pixels per block along a diamond edge's horizontal extent.
    pub scale: f32,
    /// Vertical exaggeration.
    pub vscale: f32,
}

impl Default for IsoCam {
    fn default() -> Self {
        Self { x: 0.0, z: 0.0, rot: 0, scale: 2.0, vscale: 1.0 }
    }
}

impl IsoCam {
    /// Move the centre by a screen-space drag (pixels).
    pub fn pan(&mut self, dx: f32, dy: f32) {
        // Screen → rotated (u, v): sx = (u - v)·s, sy = (u + v)·s/2.
        let s = self.scale.max(0.01) as f64;
        let (dx, dy) = (dx as f64, dy as f64);
        let du = -(dx / s + 2.0 * dy / s) / 2.0;
        let dv = -(2.0 * dy / s - dx / s) / 2.0;
        let (ddx, ddz) = unrotate(self.rot, du, dv);
        self.x += ddx;
        self.z += ddz;
    }
}

/// Rotated (u, v) → world (dx, dz) offsets.
fn unrotate(rot: u8, u: f64, v: f64) -> (f64, f64) {
    match rot % 4 {
        0 => (u, v),
        1 => (-v, u),
        2 => (-u, -v),
        _ => (v, -u),
    }
}

/// Original cell (q, r) → rotated index (a, b) in an n×n grid.
fn rot_index(rot: u8, n: usize, q: usize, r: usize) -> (usize, usize) {
    match rot % 4 {
        0 => (q, r),
        1 => (r, n - 1 - q),
        2 => (n - 1 - q, n - 1 - r),
        _ => (n - 1 - r, q),
    }
}

fn shade(c: [u8; 3], k: f32) -> [u8; 4] {
    [(c[0] as f32 * k) as u8, (c[1] as f32 * k) as u8, (c[2] as f32 * k) as u8, 255]
}

/// Render `w × h` RGBA over `background`.
pub fn render(grid: &Grid, cam: &IsoCam, w: usize, h: usize, background: [u8; 3]) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    for px in out.as_chunks_mut::<4>().0 {
        *px = [background[0], background[1], background[2], 255];
    }
    let n = grid.n;
    if n == 0 || w == 0 || h == 0 {
        return out;
    }
    // Heights/colours in rotated index space.
    let mut hh = vec![f32::NAN; n * n];
    let mut cc = vec![[0u8; 3]; n * n];
    for r in 0..n {
        for q in 0..n {
            let (a, b) = rot_index(cam.rot, n, q, r);
            hh[b * n + a] = grid.height[r * n + q];
            cc[b * n + a] = grid.color[r * n + q];
        }
    }
    let c = grid.cell as f32;
    let s = cam.scale.max(0.01);
    // Camera centre in rotated index space (continuous).
    let qc = ((cam.x - grid.x0) / grid.cell) as f32;
    let rc = ((cam.z - grid.z0) / grid.cell) as f32;
    let nf = n as f32;
    let (ac, bc) = match cam.rot % 4 {
        0 => (qc, rc),
        1 => (rc, nf - qc),
        2 => (nf - qc, nf - rc),
        _ => (nf - rc, qc),
    };
    // Height reference: the cell under the centre (keeps it mid-screen).
    let href = {
        let (a, b) = (ac.clamp(0.0, nf - 1.0) as usize, bc.clamp(0.0, nf - 1.0) as usize);
        let v = hh[b * n + a];
        if v.is_nan() { 64.0 } else { v }
    };
    let (wf, hf) = (w as f32, h as f32);
    let vs = s * cam.vscale;
    // Screen position of rotated block coords (u, v) at height y.
    let proj = |u: f32, v: f32, y: f32| -> (f32, f32) { (wf / 2.0 + (u - v) * s, hf / 2.0 + (u + v) * s / 2.0 - (y - href) * vs) };
    let put = |x: i64, y: i64, col: [u8; 4], out: &mut Vec<u8>| {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            let o = (y as usize * w + x as usize) * 4;
            out[o..o + 4].copy_from_slice(&col);
        }
    };
    let low = hh.iter().copied().filter(|v| !v.is_nan()).fold(f32::INFINITY, f32::min);
    for diag in 0..(2 * n - 1) {
        let a0 = diag.saturating_sub(n - 1);
        let a1 = diag.min(n - 1);
        for a in a0..=a1 {
            let b = diag - a;
            let top = hh[b * n + a];
            if top.is_nan() {
                continue;
            }
            let (u, v) = ((a as f32 - ac) * c, (b as f32 - bc) * c);
            // Top diamond centre and extents.
            let (cx, cy) = proj(u + c / 2.0, v + c / 2.0, top);
            let (ha, hb) = (c * s, c * s / 2.0);
            // Cull (sides can reach far below).
            let depth = (top - low) * vs;
            if cx + ha < 0.0 || cx - ha > wf || cy + hb + depth < 0.0 || cy - hb > hf {
                continue;
            }
            let col = cc[b * n + a];
            // Side faces: down to the front neighbours (or the lowest cell).
            // Unknown neighbours (grid edge, unloaded) end in a thin slab.
            let front = |aa: usize, bb: usize| if aa < n && bb < n && !hh[bb * n + aa].is_nan() { hh[bb * n + aa] } else { top - 6.0 * c.max(1.0) };
            let left_drop = ((top - front(a, b + 1)).max(0.0)) * vs;
            let right_drop = ((top - front(a + 1, b)).max(0.0)) * vs;
            // Left face (+v side): x from cx-ha to cx, top edge from (cx-ha, cy) to (cx, cy+hb).
            if left_drop > 0.0 {
                let lc = shade(col, 0.72);
                let (x0, x1) = ((cx - ha).floor() as i64, cx.ceil() as i64);
                for x in x0..x1 {
                    let t = ((x as f32 + 0.5) - (cx - ha)) / ha;
                    let ytop = cy + t * hb;
                    for y in ytop.floor() as i64..(ytop + left_drop).ceil() as i64 {
                        put(x, y, lc, &mut out);
                    }
                }
            }
            if right_drop > 0.0 {
                let rc2 = shade(col, 0.56);
                let (x0, x1) = (cx.floor() as i64, (cx + ha).ceil() as i64);
                for x in x0..x1 {
                    let t = ((x as f32 + 0.5) - cx) / ha;
                    let ytop = cy + hb - t * hb;
                    for y in ytop.floor() as i64..(ytop + right_drop).ceil() as i64 {
                        put(x, y, rc2, &mut out);
                    }
                }
            }
            // Top diamond.
            let tc = [col[0], col[1], col[2], 255];
            let (x0, x1) = ((cx - ha).floor() as i64, (cx + ha).ceil() as i64);
            let (y0, y1) = ((cy - hb).floor() as i64, (cy + hb).ceil() as i64);
            for y in y0..y1 {
                let dy = ((y as f32 + 0.5) - cy).abs() / hb.max(0.01);
                if dy > 1.0 {
                    continue;
                }
                let half = ha * (1.0 - dy);
                for x in x0..x1 {
                    if ((x as f32 + 0.5) - cx).abs() <= half + 0.5 {
                        put(x, y, tc, &mut out);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tower_grid() -> Grid {
        // 32×32 flat green ground at 60 with a red tower (100) at the centre.
        Grid::build(0.0, 0.0, 16.0, 32, &|x: f64, z: f64| {
            if x.abs() < 2.0 && z.abs() < 2.0 { Some((100.0, [220, 30, 30])) } else { Some((60.0, [40, 170, 40])) }
        })
    }

    #[test]
    fn tower_rises_above_the_ground_and_rotates() {
        let g = tower_grid();
        let (w, h) = (200, 200);
        let red = |img: &[u8]| img.as_chunks::<4>().0.iter().filter(|p| p[0] > 150 && p[1] < 80).count();
        let cam = IsoCam { x: 0.0, z: 0.0, rot: 0, scale: 3.0, vscale: 1.0 };
        let img = render(&g, &cam, w, h, [0, 0, 0]);
        // The tower's top + sides are drawn, bigger than its footprint alone.
        let r0 = red(&img);
        assert!(r0 > 4 * 4 * 9, "tower drawn with sides: {r0}");
        // Shaded side pixels exist (darker red).
        assert!(img.as_chunks::<4>().0.iter().any(|p| p[0] > 100 && p[0] < 200 && p[1] < 40));
        // Every view angle shows the same symmetric tower.
        for rot in 1..4 {
            let r = red(&render(&g, &IsoCam { rot, ..cam }, w, h, [0, 0, 0]));
            assert!((r as i64 - r0 as i64).abs() < r0 as i64 / 5, "rot {rot}: {r} vs {r0}");
        }
        // Background stays where nothing projects.
        assert_eq!(&img[..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn pan_moves_the_centre() {
        let mut c = IsoCam { scale: 2.0, ..Default::default() };
        c.pan(0.0, 20.0);
        // Dragging down moves the view north-west (towards smaller u + v).
        assert!(c.x < 0.0 && c.z < 0.0, "{c:?}");
    }
}
