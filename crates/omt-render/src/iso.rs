//! **Isometric map** (uNmINeD's "Map isometric"): the 3D view's height /
//! colour grid drawn in a 2:1 isometric projection from one of four view
//! angles. CPU only, so it works without a GPU.
//!
//! Every view angle + zoom defines one infinite, world-anchored **global
//! iso pixel plane** ([`project`]): the GUI cuts it into fixed screen tiles
//! ([`render_tile`]) that are streamed in as they approach the viewport and
//! dropped when they leave it, so panning is a pure translation.

use crate::iso_paint::{View, paint};
use crate::voxel::{Grid, Sample};

/// Height the camera centre is anchored at (sea level-ish): the global
/// plane's vertical origin never depends on the terrain, so tiles rendered
/// at different times always line up.
pub const REF_Y: f64 = 64.0;

/// Isometric camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsoCam {
    /// Block coordinates at the image centre (at height [`REF_Y`]).
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

    /// Global iso pixel of the view centre at `s` pixels per block.
    pub fn center_px(&self, s: f64) -> (f64, f64) {
        project(self.rot, s, self.vscale as f64, self.x, REF_Y, self.z)
    }

    /// Put the view centre on global iso pixel `p` (at `s` px per block).
    pub fn set_center_px(&mut self, s: f64, p: (f64, f64)) {
        let (x, z) = unproject(self.rot, s, self.vscale as f64, p.0, p.1, REF_Y);
        self.x = x;
        self.z = z;
    }
}

/// World (x, z) → rotated (u, v) for view angle `rot`.
pub fn rotate(rot: u8, x: f64, z: f64) -> (f64, f64) {
    match rot % 4 {
        0 => (x, z),
        1 => (z, -x),
        2 => (-x, -z),
        _ => (-z, x),
    }
}

/// Rotated (u, v) → world (x, z) (inverse of [`rotate`]).
pub fn unrotate(rot: u8, u: f64, v: f64) -> (f64, f64) {
    match rot % 4 {
        0 => (u, v),
        1 => (-v, u),
        2 => (-u, -v),
        _ => (v, -u),
    }
}

/// Global iso pixel of world point (x, y, z) at `s` px per block.
pub fn project(rot: u8, s: f64, vscale: f64, x: f64, y: f64, z: f64) -> (f64, f64) {
    let (u, v) = rotate(rot, x, z);
    ((u - v) * s, (u + v) * s / 2.0 - y * s * vscale)
}

/// World (x, z) seen at global iso pixel `(px, py)` on height `y`.
pub fn unproject(rot: u8, s: f64, vscale: f64, px: f64, py: f64, y: f64) -> (f64, f64) {
    let e = px / s;
    let d = 2.0 * (py + y * s * vscale) / s;
    unrotate(rot, (d + e) / 2.0, (d - e) / 2.0)
}

/// World box `(x0, z0, x1, z1)` holding every column (heights in
/// `low..=high`) that can draw into the global pixel rectangle
/// `[px0, px1) × [py0, py1)`, padded by `pad` blocks.
#[allow(clippy::too_many_arguments)]
pub fn world_box(rot: u8, s: f64, vscale: f64, px0: f64, py0: f64, px1: f64, py1: f64, low: f64, high: f64, pad: f64) -> (f64, f64, f64, f64) {
    let (e0, e1) = (px0 / s - pad, px1 / s + pad);
    let (d0, d1) = (2.0 * (py0 + low * s * vscale) / s - 2.0 * pad, 2.0 * (py1 + high * s * vscale) / s + 2.0 * pad);
    let mut b = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (e, d) in [(e0, d0), (e0, d1), (e1, d0), (e1, d1)] {
        let (x, z) = unrotate(rot, (d + e) / 2.0, (d - e) / 2.0);
        b = (b.0.min(x), b.1.min(z), b.2.max(x), b.3.max(z));
    }
    b
}

/// Render `w × h` RGBA centred on `cam` over `background`.
pub fn render(grid: &Grid, cam: &IsoCam, w: usize, h: usize, background: [u8; 3]) -> Vec<u8> {
    frame(grid, cam, w, h, background, false)
}

/// [`render`] with the grid's per-cell block looks ([`Grid::kinds`]): top
/// diamonds carry the world-aligned block texture, sides the shaded side
/// texture (cut-out texels show the map colour). Cells without a kind, and
/// water, fall back to the flat painter.
pub fn render_textured(grid: &Grid, cam: &IsoCam, w: usize, h: usize, background: [u8; 3]) -> Vec<u8> {
    frame(grid, cam, w, h, background, true)
}

fn frame(grid: &Grid, cam: &IsoCam, w: usize, h: usize, background: [u8; 3], textured: bool) -> Vec<u8> {
    let s = cam.scale.max(0.01) as f64;
    let (cx, cy) = cam.center_px(s);
    let view = View::at(grid, cam.rot, s, cam.vscale as f64, (cx - w as f64 / 2.0, cy - h as f64 / 2.0));
    paint(grid, cam.rot, view, w, h, background, textured)
}

/// One screen tile of the global iso plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileSpec {
    pub rot: u8,
    /// Rendered pixels per block.
    pub scale: f64,
    pub vscale: f64,
    /// Global pixel of the tile's top-left corner (at `scale`).
    pub px: f64,
    pub py: f64,
    /// Tile size in pixels.
    pub size: usize,
    /// Height range of the loaded terrain (bounds which columns can reach
    /// the tile).
    pub low: f32,
    pub high: f32,
}

impl TileSpec {
    /// Grid cell size: whole blocks when zoomed in, a power of two blocks
    /// per cell when a block is smaller than a pixel.
    pub fn cell(&self) -> f64 {
        if self.scale >= 1.0 { 1.0 } else { (1.0 / self.scale).log2().ceil().exp2().min(64.0) }
    }

    /// Cell-aligned world box the tile's grid covers.
    pub fn grid_box(&self) -> (f64, f64, f64, f64) {
        let c = self.cell();
        let (x0, z0, x1, z1) = world_box(self.rot, self.scale, self.vscale, self.px, self.py, self.px + self.size as f64, self.py + self.size as f64, self.low as f64, self.high as f64, 2.0 * c);
        ((x0 / c).floor() * c, (z0 / c).floor() * c, (x1 / c).ceil() * c, (z1 / c).ceil() * c)
    }
}

impl TileSpec {
    /// Whether the column at world (x, z) can draw into this tile (inside
    /// the rotated band the tile sees, padded by two cells).
    pub fn reaches(&self, x: f64, z: f64) -> bool {
        let (u, v) = rotate(self.rot, x, z);
        let (s, vs) = (self.scale, self.scale * self.vscale);
        let pad = 2.0 * self.cell();
        let (e, d) = (u - v, u + v);
        let size = self.size as f64;
        e >= self.px / s - pad && e <= (self.px + size) / s + pad && d >= 2.0 * (self.py + self.low as f64 * vs) / s - 2.0 * pad && d <= 2.0 * (self.py + size + self.high as f64 * vs) / s + 2.0 * pad
    }
}

/// Sample the columns that can reach a tile from `f` into a cell-aligned
/// grid (so neighbouring tiles agree pixel for pixel). Set
/// [`Grid::kinds`] afterwards for the textured painter.
pub fn tile_grid(spec: &TileSpec, f: &Sample<'_>) -> Grid {
    let c = spec.cell();
    let (x0, z0, x1, z1) = spec.grid_box();
    let n = (((x1 - x0).max(z1 - z0) / c).ceil() as usize).max(1);
    Grid::build_at(x0, z0, c, n, Vec::new(), f)
}

/// Paint a tile's grid (from [`tile_grid`]) as `size × size` RGBA.
pub fn paint_tile(spec: &TileSpec, grid: &Grid, background: [u8; 3], textured: bool) -> Vec<u8> {
    let view = View::at(grid, spec.rot, spec.scale, spec.vscale, (spec.px, spec.py));
    paint(grid, spec.rot, view, spec.size, spec.size, background, textured)
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

    fn hills(x: f64, z: f64) -> Option<(f32, [u8; 3], u16)> {
        let h = 64.0 + ((x * 0.07).sin() * (z * 0.05).cos() * 18.0) as f32;
        Some((h.floor(), [(x.rem_euclid(7.0) * 30.0) as u8, 120, (z.rem_euclid(5.0) * 40.0) as u8], NO_KIND))
    }

    #[test]
    fn projection_round_trips() {
        for rot in 0..4 {
            let (px, py) = project(rot, 3.0, 1.0, 123.5, REF_Y, -77.25);
            let (x, z) = unproject(rot, 3.0, 1.0, px, py, REF_Y);
            assert!((x - 123.5).abs() < 1e-9 && (z + 77.25).abs() < 1e-9, "rot {rot}: {x},{z}");
            let mut c = IsoCam { rot, scale: 3.0, ..Default::default() };
            c.set_center_px(3.0, (px + 30.0, py - 12.0));
            let mut d = IsoCam { x: 123.5, z: -77.25, rot, scale: 3.0, ..Default::default() };
            d.pan(-30.0, 12.0);
            assert!((c.x - d.x).abs() < 1e-9 && (c.z - d.z).abs() < 1e-9, "pan == move centre");
        }
    }

    /// Four tiles side by side paint exactly what one tile twice the size
    /// paints (no seams, whatever the view angle or zoom).
    #[test]
    fn tiles_are_seamless() {
        for (rot, scale) in [(0u8, 3.0), (1, 2.0), (2, 0.5), (3, 1.0)] {
            let base = TileSpec { rot, scale, vscale: 1.0, px: 1000.0, py: -2000.0, size: 128, low: 40.0, high: 90.0 };
            let big = paint_tile(&base, &tile_grid(&base, &hills), [0, 0, 0], false);
            let mut diff = 0;
            for (qx, qy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let s = TileSpec { px: base.px + qx as f64 * 64.0, py: base.py + qy as f64 * 64.0, size: 64, ..base };
                let img = paint_tile(&s, &tile_grid(&s, &hills), [0, 0, 0], false);
                for y in 0..64 {
                    for x in 0..64 {
                        let a = &img[(y * 64 + x) * 4..][..4];
                        let b = &big[((y + qy * 64) * 128 + x + qx * 64) * 4..][..4];
                        diff += usize::from(a != b);
                    }
                }
            }
            assert_eq!(diff, 0, "rot {rot} scale {scale}: differing pixels at tile seams");
            // Terrain covers the tile (no sky holes inside the loaded area).
            let sky = big.as_chunks::<4>().0.iter().filter(|p| **p == [0, 0, 0, 255]).count();
            assert!(sky < 128 * 128 / 50, "rot {rot}: {sky} sky pixels");
        }
    }

    /// Timing probe: `cargo test --release -p omt-render iso_bench -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing probe"]
    fn iso_bench() {
        let g = Grid::build(0.0, 0.0, 700.0, 768, Vec::new(), &|x: f64, z: f64| Some((64.0 + ((x * 0.05).sin() * (z * 0.04).cos() * 20.0) as f32, [90, 140, 60], NO_KIND)));
        let cam = IsoCam { scale: 2.0, ..Default::default() };
        let t = std::time::Instant::now();
        for _ in 0..5 {
            std::hint::black_box(render(&g, &cam, 1600, 900, [0, 0, 0]));
        }
        println!("iso render 1600x900, 768² grid: {:.1} ms/frame", t.elapsed().as_secs_f64() * 1000.0 / 5.0);
        for scale in [4.0, 2.0, 1.0, 0.5] {
            let spec = TileSpec { rot: 1, scale, vscale: 1.0, px: 5000.0, py: 3000.0, size: 256, low: 30.0, high: 110.0 };
            let t = std::time::Instant::now();
            let g = tile_grid(&spec, &hills);
            let built = t.elapsed();
            std::hint::black_box(paint_tile(&spec, &g, [0, 0, 0], false));
            println!("256² tile @ {scale} px/block: grid {}² in {:.1} ms, total {:.1} ms", g.n, built.as_secs_f64() * 1000.0, t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
