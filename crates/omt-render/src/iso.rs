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

/// Screen position of rotated block coords `(u, v)` at height `y` (shared
/// by both painters).
struct View {
    wf: f32,
    hf: f32,
    /// Screen pos of the rotated-grid origin (cell (0,0) corner).
    ox: f32,
    oy: f32,
    /// Pixels per block along a diamond edge; vertical = `s · vscale`.
    s: f32,
    vs: f32,
}

impl View {
    fn new(cam: &IsoCam, w: usize, h: usize, n: usize, hh: &[f32], ac: f32, bc: f32) -> Self {
        let href = {
            let (a, b) = (ac.clamp(0.0, n as f32 - 1.0) as usize, bc.clamp(0.0, n as f32 - 1.0) as usize);
            let v = hh[b * n + a];
            if v.is_nan() { 64.0 } else { v }
        };
        let s = cam.scale.max(0.01);
        Self { wf: w as f32, hf: h as f32, ox: w as f32 / 2.0 + ac * s, oy: h as f32 / 2.0 + bc * s / 2.0 + href * s * cam.vscale, s, vs: s * cam.vscale }
    }
    /// Screen position of rotated block coords `(u, v)` at height `y`.
    fn proj(&self, u: f32, v: f32, y: f32) -> (f32, f32) {
        (self.ox + (u - v) * self.s, self.oy + (u + v) * self.s / 2.0 - y * self.vs)
    }
}

/// Render `w × h` RGBA over `background`.
pub fn render(grid: &Grid, cam: &IsoCam, w: usize, h: usize, background: [u8; 3]) -> Vec<u8> {
    paint(grid, cam, w, h, background, false)
}

/// [`render`] with the grid's per-cell block looks ([`Grid::kinds`]): top
/// diamonds carry the world-aligned block texture, sides the shaded side
/// texture (cut-out texels show what was painted behind). Cells without a
/// kind, and water, fall back to the flat painter.
pub fn render_textured(grid: &Grid, cam: &IsoCam, w: usize, h: usize, background: [u8; 3]) -> Vec<u8> {
    paint(grid, cam, w, h, background, true)
}

fn paint(grid: &Grid, cam: &IsoCam, w: usize, h: usize, background: [u8; 3], textured: bool) -> Vec<u8> {
    let textured = textured && !grid.kinds.is_empty();
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
    let view = View::new(cam, w, h, n, &hh, ac, bc);
    let put = |x: i64, y: i64, col: [u8; 4], out: &mut Vec<u8>| {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            let o = (y as usize * w + x as usize) * 4;
            out[o..o + 4].copy_from_slice(&col);
        }
    };
    let low = hh.iter().copied().filter(|v| !v.is_nan()).fold(f32::INFINITY, f32::min);
    // Texture lookup helpers (world-aligned: textures keep their compass
    // orientation at every view angle and repeat per block inside a cell).
    let texel_at = |tex: &[[u8; 4]], wx: f64, wz: f64| -> [u8; 4] {
        let tx = (((wx - wx.floor()) * 16.0) as usize).min(15);
        let tz = (((wz - wz.floor()) * 16.0) as usize).min(15);
        tex[tz * 16 + tx]
    };
    let texel_col = |wx: f64, wz: f64, along_x: bool| -> usize {
        let w = if along_x { wx } else { wz };
        (((w - w.floor()) * 16.0) as usize).min(15)
    };
    let texel_row = |tex: &[[u8; 4]], col: usize, y: f32| -> [u8; 4] {
        let f = (-y) - (-y).floor();
        tex[((f * 16.0) as usize).min(15) * 16 + col]
    };
    let (fax, faz) = unrotate(cam.rot, 1.0, 0.0);
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
            let (cx, cy) = view.proj(u + c / 2.0, v + c / 2.0, top);
            let (ha, hb) = (c * s, c * s / 2.0);
            // Cull (sides can reach far below).
            let depth = (top - low) * view.vs;
            if cx + ha < 0.0 || cx - ha > view.wf || cy + hb + depth < 0.0 || cy - hb > view.hf {
                continue;
            }
            let col = cc[b * n + a];
            let kind = if textured {
                let k = grid.kind[b * n + a];
                (k != crate::voxel::NO_KIND).then(|| grid.kinds.get(k as usize)).flatten()
            } else {
                None
            };
            let use_tex = textured && kind.is_some_and(|k| !k.water);
            // Side faces: down to the front neighbours (or the lowest cell).
            // Unknown neighbours (grid edge, unloaded) end in a thin slab.
            let front = |aa: usize, bb: usize| if aa < n && bb < n && !hh[bb * n + aa].is_nan() { hh[bb * n + aa] } else { top - 6.0 * c.max(1.0) };
            let left_drop = ((top - front(a, b + 1)).max(0.0)) * view.vs;
            let right_drop = ((top - front(a + 1, b)).max(0.0)) * view.vs;
            // World origin of this cell (the painter walks rotated indices;
            // textures need the original compass position).
            let (q, r) = match cam.rot % 4 {
                0 => (a, b),
                1 => (n - 1 - b, a),
                2 => (n - 1 - a, n - 1 - b),
                _ => (b, n - 1 - a),
            };
            let (wx0, wz0) = (grid.x0 + q as f64 * c as f64, grid.z0 + r as f64 * c as f64);
            let flat = [col[0], col[1], col[2], 255];
            // Cut-out texels (plants, leaves) take the cell's map color so
            // they never punch holes down to the sky.
            let opaque = |t: [u8; 4], k: f32| if t[3] < 128 { flat } else { shade([t[0], t[1], t[2]], k) };
            let (x0, x1) = ((cx - ha).floor() as i64, (cx + ha).ceil() as i64);
            if use_tex {
                let k = kind.unwrap();
                // Left face (+v side), textured and sheared.
                if left_drop > 0.0 {
                    let (x0l, x1l) = ((cx - ha).floor() as i64, cx.ceil() as i64);
                    for x in x0l..x1l {
                        let t = ((x as f32 + 0.5) - (cx - ha)) / ha;
                        let ytop = cy + t * hb;
                        for y in ytop.floor() as i64..(ytop + left_drop).ceil() as i64 {
                            let wy = top - ((y as f32 + 0.5) - ytop) / view.vs;
                            let (du, dv) = unrotate(cam.rot, (t * c) as f64, c as f64);
                            let tc = texel_row(&k.side, texel_col(wx0 + du, wz0 + dv, fax != 0.0), wy);
                            put(x, y, opaque(tc, 0.72), &mut out);
                        }
                    }
                }
                // Right face (+a side).
                if right_drop > 0.0 {
                    let (x0r, x1r) = (cx.floor() as i64, (cx + ha).ceil() as i64);
                    for x in x0r..x1r {
                        let t = ((x as f32 + 0.5) - cx) / ha;
                        let ytop = cy + hb - t * hb;
                        for y in ytop.floor() as i64..(ytop + right_drop).ceil() as i64 {
                            let wy = top - ((y as f32 + 0.5) - ytop) / view.vs;
                            let (du, dv) = unrotate(cam.rot, c as f64, (t * c) as f64);
                            let tc = texel_row(&k.side, texel_col(wx0 + du, wz0 + dv, faz != 0.0), wy);
                            put(x, y, opaque(tc, 0.56), &mut out);
                        }
                    }
                }
                // Top diamond: inverse-map each pixel into the cell and
                // sample the world-aligned top texture.
                let (y0, y1) = ((cy - hb).floor() as i64, (cy + hb).ceil() as i64);
                let u0 = (a as f32 - ac) * c;
                let v0 = (b as f32 - bc) * c;
                for y in y0..y1 {
                    let dy = ((y as f32 + 0.5) - cy).abs() / hb.max(0.01);
                    if dy > 1.0 {
                        continue;
                    }
                    let half = ha * (1.0 - dy);
                    for x in x0..x1 {
                        if ((x as f32 + 0.5) - cx).abs() > half + 0.5 {
                            continue;
                        }
                        let diff = ((x as f32 + 0.5) - view.ox) / view.s;
                        let sum = 2.0 * ((y as f32 + 0.5) - view.oy + top * view.vs) / view.s;
                        let (du, dv) = unrotate(cam.rot, ((diff + sum) / 2.0 - u0) as f64, ((sum - diff) / 2.0 - v0) as f64);
                        let tc = texel_at(&k.top, wx0 + du, wz0 + dv);
                        put(x, y, opaque(tc, 1.0), &mut out);
                    }
                }
                continue;
            }
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
    use crate::voxel::NO_KIND;

    fn tower_grid() -> Grid {
        // 32×32 flat green ground at 60 with a red tower (100) at the centre.
        Grid::build(
            0.0,
            0.0,
            16.0,
            32,
            Vec::new(),
            &|x: f64, z: f64| {
                if x.abs() < 2.0 && z.abs() < 2.0 { Some((100.0, [220, 30, 30], NO_KIND)) } else { Some((60.0, [40, 170, 40], NO_KIND)) }
            },
        )
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
