//! True block-geometry 3D: a dense grid of the real blocks around the
//! camera (decoded chunks) ray-cast per pixel with a 3D DDA (Amanatides &
//! Woo). Faces are textured (top / side textures supplied per block kind,
//! e.g. from the user's resource pack), shaded by face direction, fogged by
//! distance; cut-out texels (flowers, plants, leaves' holes) let the ray
//! pass, water tints and continues. Column-parallel, CPU only.
//!
//! The camera ([`Cam3`]) and projection are shared with the height-field
//! view (`voxel`), so switching detail keeps the framing.

use rayon::prelude::*;

use crate::voxel::Cam3;

/// How one block kind looks.
#[derive(Debug, Clone)]
pub struct Kind {
    /// 16×16 RGBA, row-major.
    pub top: Vec<[u8; 4]>,
    pub side: Vec<[u8; 4]>,
    pub water: bool,
}

impl Kind {
    /// A flat-colored kind (no texture available).
    pub fn flat(c: [u8; 4]) -> Self {
        let tex: Vec<[u8; 4]> = (0..256)
            .map(|i| {
                // Subtle per-texel noise so flat faces still read as blocks.
                let n = ((i as u32).wrapping_mul(2_654_435_761) >> 27) as i32 - 16;
                let f = |v: u8| (v as i32 + n / 2).clamp(0, 255) as u8;
                [f(c[0]), f(c[1]), f(c[2]), c[3]]
            })
            .collect();
        Self { top: tex.clone(), side: tex, water: false }
    }
}

/// Dense block grid; cell 0 = air. Index `(y * sz + z) * sx + x`.
#[derive(Debug, Clone)]
pub struct BlockGrid {
    pub origin: [i32; 3],
    pub size: [usize; 3],
    pub cells: Vec<u16>,
    pub kinds: Vec<Kind>,
}

impl BlockGrid {
    pub fn empty(origin: [i32; 3], size: [usize; 3]) -> Self {
        Self { origin, size, cells: vec![0; size[0] * size[1] * size[2]], kinds: vec![Kind::flat([0, 0, 0, 0])] }
    }

    #[inline]
    fn at(&self, x: i32, y: i32, z: i32) -> u16 {
        let (lx, ly, lz) = (x - self.origin[0], y - self.origin[1], z - self.origin[2]);
        if lx < 0 || ly < 0 || lz < 0 || lx as usize >= self.size[0] || ly as usize >= self.size[1] || lz as usize >= self.size[2] {
            return 0;
        }
        self.cells[(ly as usize * self.size[2] + lz as usize) * self.size[0] + lx as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, id: u16) {
        let (lx, ly, lz) = (x - self.origin[0], y - self.origin[1], z - self.origin[2]);
        if lx < 0 || ly < 0 || lz < 0 || lx as usize >= self.size[0] || ly as usize >= self.size[1] || lz as usize >= self.size[2] {
            return;
        }
        let i = (ly as usize * self.size[2] + lz as usize) * self.size[0] + lx as usize;
        self.cells[i] = id;
    }

    /// Y of the highest block in a column (`None` = empty column).
    pub fn top_at(&self, x: i32, z: i32) -> Option<i32> {
        (0..self.size[1] as i32).rev().map(|ly| ly + self.origin[1]).find(|&y| self.at(x, y, z) != 0)
    }

    /// Center of the grid (blocks).
    pub fn center(&self) -> (f64, f64) {
        (self.origin[0] as f64 + self.size[0] as f64 / 2.0, self.origin[2] as f64 + self.size[2] as f64 / 2.0)
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Ray-cast `w × h` RGBA. `sky` = (zenith, horizon).
pub fn render(g: &BlockGrid, cam: &Cam3, w: usize, h: usize, sky: ([u8; 3], [u8; 3])) -> Vec<u8> {
    render_hl(g, cam, w, h, sky, None)
}

/// [`render`] with blocks inside an inclusive box tinted (selection).
pub fn render_hl(g: &BlockGrid, cam: &Cam3, w: usize, h: usize, sky: ([u8; 3], [u8; 3]), hl: Option<([i32; 3], [i32; 3])>) -> Vec<u8> {
    let horizon = h as f32 * (0.5 + cam.pitch);
    let focal = (w as f32 / 2.0) / (cam.fov / 2.0).tan();
    let s3 = |c: [u8; 3]| [c[0] as f32, c[1] as f32, c[2] as f32];
    let (zen, hor) = (s3(sky.0), s3(sky.1));
    let max_dist = (g.size[0].max(g.size[2]) as f32) * 1.5;
    let cols: Vec<Vec<[u8; 3]>> = (0..w)
        .into_par_iter()
        .map(|i| {
            let rel = ((i as f32 + 0.5) / w as f32 - 0.5) * cam.fov;
            let a = cam.yaw + rel;
            let (ux, uz) = (a.sin(), -a.cos());
            let corr = rel.cos();
            let mut col = Vec::with_capacity(h);
            for y in 0..h {
                let s = (horizon - (y as f32 + 0.5)) / focal;
                let d = [ux, s * corr, uz];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                let d = [d[0] / len, d[1] / len, d[2] / len];
                let t = (y as f32 / horizon.max(1.0)).clamp(0.0, 1.0);
                let sky_c = mix(zen, hor, t);
                let c = cast(g, [cam.x as f32, cam.y, cam.z as f32], d, max_dist, hl).map_or(sky_c, |(c, dist)| mix(c, hor, ((dist / max_dist).powf(1.5) * 0.85).min(0.85)));
                col.push([c[0] as u8, c[1] as u8, c[2] as u8]);
            }
            col
        })
        .collect();
    let mut out = vec![0u8; w * h * 4];
    for (x, col) in cols.iter().enumerate() {
        for (y, c) in col.iter().enumerate() {
            let o = (y * w + x) * 4;
            out[o..o + 3].copy_from_slice(c);
            out[o + 3] = 255;
        }
    }
    out
}

/// The solid (non-water) block under image pixel (`px`, `py`) of a
/// `w × h` render from `cam` — 3D picking for editors.
pub fn pick(g: &BlockGrid, cam: &Cam3, w: usize, h: usize, px: f32, py: f32) -> Option<[i32; 3]> {
    let horizon = h as f32 * (0.5 + cam.pitch);
    let focal = (w as f32 / 2.0) / (cam.fov / 2.0).tan();
    let rel = (px / w as f32 - 0.5) * cam.fov;
    let a = cam.yaw + rel;
    let s = (horizon - py) / focal;
    let d = [a.sin(), s * rel.cos(), -a.cos()];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let d = [d[0] / len, d[1] / len, d[2] / len];
    let o = [cam.x as f32, cam.y, cam.z as f32];
    let mut cell = [o[0].floor() as i32, o[1].floor() as i32, o[2].floor() as i32];
    let step = [d[0].signum() as i32, d[1].signum() as i32, d[2].signum() as i32];
    let inv = [1.0 / d[0].abs().max(1e-9), 1.0 / d[1].abs().max(1e-9), 1.0 / d[2].abs().max(1e-9)];
    let mut t_max = [0f32; 3];
    for k in 0..3 {
        t_max[k] = (if d[k] > 0.0 { cell[k] as f32 + 1.0 - o[k] } else { o[k] - cell[k] as f32 }) * inv[k];
    }
    let far = (g.size[0].max(g.size[2]) as f32) * 1.5;
    let mut t = 0.0;
    while t < far {
        let id = g.at(cell[0], cell[1], cell[2]);
        if id != 0 && !g.kinds[id as usize].water {
            return Some(cell);
        }
        let axis = if t_max[0] < t_max[1] { if t_max[0] < t_max[2] { 0 } else { 2 } } else if t_max[1] < t_max[2] { 1 } else { 2 };
        t = t_max[axis];
        cell[axis] += step[axis];
        t_max[axis] += inv[axis];
    }
    None
}

/// DDA through the grid; returns the lit color and hit distance.
fn cast(g: &BlockGrid, o: [f32; 3], d: [f32; 3], max_dist: f32, hl: Option<([i32; 3], [i32; 3])>) -> Option<([f32; 3], f32)> {
    let in_hl = |c: [i32; 3]| hl.is_some_and(|(a, b)| (0..3).all(|k| c[k] >= a[k] && c[k] <= b[k]));
    let mut cell = [o[0].floor() as i32, o[1].floor() as i32, o[2].floor() as i32];
    let step = [d[0].signum() as i32, d[1].signum() as i32, d[2].signum() as i32];
    let inv = [1.0 / d[0].abs().max(1e-9), 1.0 / d[1].abs().max(1e-9), 1.0 / d[2].abs().max(1e-9)];
    let mut t_max = [0f32; 3];
    for k in 0..3 {
        let next = if d[k] > 0.0 { cell[k] as f32 + 1.0 - o[k] } else { o[k] - cell[k] as f32 };
        t_max[k] = next * inv[k];
    }
    let mut t = 0f32;
    let mut axis = 1usize;
    let mut water: Option<[f32; 3]> = None;
    let mut water_t = 0f32;
    let ymin = g.origin[1] - 1;
    let ymax = g.origin[1] + g.size[1] as i32 + 64;
    while t < max_dist {
        let id = g.at(cell[0], cell[1], cell[2]);
        if id != 0 {
            let kind = &g.kinds[id as usize];
            let p = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
            let (tex, u, v, shade) = match axis {
                1 if d[1] < 0.0 => (&kind.top, p[0].rem_euclid(1.0), p[2].rem_euclid(1.0), 1.0),
                1 => (&kind.side, p[0].rem_euclid(1.0), p[2].rem_euclid(1.0), 0.5),
                0 => (&kind.side, p[2].rem_euclid(1.0), 1.0 - p[1].rem_euclid(1.0), 0.78),
                _ => (&kind.side, p[0].rem_euclid(1.0), 1.0 - p[1].rem_euclid(1.0), 0.9),
            };
            let tx = &tex[((v * 16.0) as usize).min(15) * 16 + ((u * 16.0) as usize).min(15)];
            if kind.water {
                if water.is_none() {
                    water = Some([tx[0] as f32, tx[1] as f32, tx[2] as f32]);
                    water_t = t;
                }
            } else if tx[3] >= 128 {
                let c = [tx[0] as f32 * shade, tx[1] as f32 * shade, tx[2] as f32 * shade];
                let c = match water {
                    // Deeper water hides more of the floor.
                    Some(wc) => mix(c, wc, (0.45 + (t - water_t) * 0.06).min(0.9)),
                    None => c,
                };
                let c = if in_hl(cell) { mix(c, [80.0, 210.0, 255.0], 0.42) } else { c };
                return Some((c, t));
            }
        } else if let Some(wc) = water {
            // Left the water upward / sideways: show the surface color.
            if t - water_t > 0.01 && cell[1] > o[1].floor() as i32 {
                return Some((wc, water_t));
            }
        }
        // Advance to the next cell.
        axis = if t_max[0] < t_max[1] { if t_max[0] < t_max[2] { 0 } else { 2 } } else if t_max[1] < t_max[2] { 1 } else { 2 };
        t = t_max[axis];
        cell[axis] += step[axis];
        t_max[axis] += inv[axis];
        if cell[1] < ymin || (cell[1] > ymax && d[1] > 0.0) {
            break;
        }
    }
    water.map(|wc| (wc, water_t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_textured_ground_and_a_red_tower() {
        let mut g = BlockGrid::empty([-32, 0, -32], [64, 40, 64]);
        g.kinds.push(Kind::flat([60, 170, 60, 255])); // 1 grass
        g.kinds.push(Kind::flat([210, 40, 40, 255])); // 2 tower
        for x in -32..32 {
            for z in -32..32 {
                g.set(x, 10, z, 1);
            }
        }
        for y in 11..35 {
            for x in -2..2 {
                for z in -22..-18 {
                    g.set(x, y, z, 2);
                }
            }
        }
        let cam = Cam3 { x: 0.0, z: 0.0, y: 20.0, pitch: 0.0, ..Default::default() };
        let (w, h) = (48, 36);
        let img = render(&g, &cam, w, h, ([10, 20, 60], [150, 180, 220]));
        let px = |x: usize, y: usize| [img[(y * w + x) * 4], img[(y * w + x) * 4 + 1], img[(y * w + x) * 4 + 2]];
        // Center column above the horizon: red tower; the edge column's
        // bottom (clear of the tower's footprint): green ground.
        assert!((0..h / 2).any(|y| px(w / 2, y)[0] > 120 && px(w / 2, y)[1] < 90), "tower");
        let b = px(0, h - 1);
        assert!(b[1] > b[0] && b[1] > b[2], "ground {b:?}");
        // Looking away: no tower.
        let back = render(&g, &Cam3 { yaw: std::f32::consts::PI, ..cam }, w, h, ([10, 20, 60], [150, 180, 220]));
        assert!(!(0..h / 2).any(|y| back[(y * w + w / 2) * 4] > 120 && back[(y * w + w / 2) * 4 + 1] < 90));
    }

    #[test]
    fn cutout_texels_let_the_ray_through() {
        let mut g = BlockGrid::empty([0, 0, 0], [8, 8, 8]);
        let mut glass = Kind::flat([255, 255, 255, 0]);
        glass.top.iter_mut().for_each(|p| p[3] = 0);
        g.kinds.push(glass);
        g.kinds.push(Kind::flat([0, 0, 255, 255]));
        g.set(4, 5, 4, 1);
        g.set(4, 2, 4, 2);
        assert_eq!(g.top_at(4, 4), Some(5));
        assert_eq!(g.top_at(0, 0), None);
        // Picking straight down from above hits the top solid block.
        let cam = Cam3 { x: 4.5, z: 4.5, y: 7.5, pitch: -5000.0, fov: 0.2, ..Default::default() };
        assert_eq!(pick(&g, &cam, 9, 9, 4.5, 4.5).map(|c| [c[0], c[2]]), Some([4, 4]));
        let hit = cast(&g, [4.5, 7.5, 4.5], [0.0, -1.0, 0.0], 50.0, None).unwrap();
        let tinted = cast(&g, [4.5, 7.5, 4.5], [0.0, -1.0, 0.0], 50.0, Some(([4, 2, 4], [4, 2, 4]))).unwrap();
        assert!(tinted.0[1] > hit.0[1] + 40.0, "selection tint");
        assert!(hit.0[2] > 150.0 && hit.0[0] < 50.0, "saw the blue block below the clear one: {:?}", hit.0);
    }
}
