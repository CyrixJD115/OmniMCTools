//! The isometric painter shared by the full-frame view and the streamed
//! screen tiles: painter's algorithm in rotated grid space, back to front,
//! each cell a top diamond plus its two visible side faces down to the
//! front neighbours. Vertical strips are painted in parallel.

use crate::iso::{rotate, unrotate};
use crate::voxel::Grid;

/// Original cell (q, r) → rotated index (a, b) in an n×n grid.
fn rot_index(rot: u8, n: usize, q: usize, r: usize) -> (usize, usize) {
    match rot % 4 {
        0 => (q, r),
        1 => (r, n - 1 - q),
        2 => (n - 1 - q, n - 1 - r),
        _ => (n - 1 - r, q),
    }
}

/// Rotated index (a, b) → original cell (q, r).
fn unrot_index(rot: u8, n: usize, a: usize, b: usize) -> (usize, usize) {
    match rot % 4 {
        0 => (a, b),
        1 => (n - 1 - b, a),
        2 => (n - 1 - a, n - 1 - b),
        _ => (b, n - 1 - a),
    }
}

fn shade(c: [u8; 3], k: f32) -> [u8; 4] {
    [(c[0] as f32 * k) as u8, (c[1] as f32 * k) as u8, (c[2] as f32 * k) as u8, 255]
}

/// Where the grid lands in the output image.
#[derive(Debug, Clone, Copy)]
pub(crate) struct View {
    /// Image position of the rotated grid's origin corner (height 0).
    pub ox: f32,
    pub oy: f32,
    /// Pixels per block along a diamond edge; vertical = `s · vscale`.
    pub s: f32,
    pub vs: f32,
}

impl View {
    /// The view whose image pixel (0, 0) is global iso pixel `origin`
    /// (see [`crate::iso::project`]) at `s` pixels per block. Large world
    /// coordinates cancel in f64 before the narrowing to image space.
    pub fn at(grid: &Grid, rot: u8, s: f64, vscale: f64, origin: (f64, f64)) -> Self {
        let span = grid.cell * grid.n as f64;
        let (a, b) = rotate(rot, grid.x0, grid.z0);
        let (c, d) = rotate(rot, grid.x0 + span, grid.z0 + span);
        let (u0, v0) = (a.min(c), b.min(d));
        Self { ox: ((u0 - v0) * s - origin.0) as f32, oy: ((u0 + v0) * s / 2.0 - origin.1) as f32, s: s as f32, vs: (s * vscale) as f32 }
    }

    /// Image position of rotated grid-relative block coords at height `y`.
    fn proj(&self, u: f32, v: f32, y: f32) -> (f32, f32) {
        (self.ox + (u - v) * self.s, self.oy + (u + v) * self.s / 2.0 - y * self.vs)
    }
}

/// Paint `grid` into a `w × h` RGBA image over `background`. With
/// `textured` (and [`Grid::kinds`] loaded) top diamonds carry the
/// world-aligned block texture and sides the shaded side texture.
pub(crate) fn paint(grid: &Grid, rot: u8, view: View, w: usize, h: usize, background: [u8; 3], textured: bool) -> Vec<u8> {
    let textured = textured && !grid.kinds.is_empty();
    let bg_px = [background[0], background[1], background[2], 255];
    let n = grid.n;
    if n == 0 || w == 0 || h == 0 {
        let mut out = vec![0u8; w * h * 4];
        for px in out.as_chunks_mut::<4>().0 {
            *px = bg_px;
        }
        return out;
    }
    // Heights/colours in rotated index space.
    let mut hh = vec![f32::NAN; n * n];
    let mut cc = vec![[0u8; 3]; n * n];
    let mut kk = vec![crate::voxel::NO_KIND; if textured { n * n } else { 0 }];
    for r in 0..n {
        for q in 0..n {
            let (a, b) = rot_index(rot, n, q, r);
            hh[b * n + a] = grid.height[r * n + q];
            cc[b * n + a] = grid.color[r * n + q];
            if textured {
                kk[b * n + a] = grid.kind[r * n + q];
            }
        }
    }
    let c = grid.cell as f32;
    let s = view.s;
    let hf = h as f32;
    let (low, high) = hh.iter().copied().filter(|v| !v.is_nan()).fold((f32::INFINITY, f32::NEG_INFINITY), |(l, hi), v| (l.min(v), hi.max(v)));
    if !low.is_finite() {
        let mut out = vec![0u8; w * h * 4];
        for px in out.as_chunks_mut::<4>().0 {
            *px = bg_px;
        }
        return out;
    }
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
    let (fax, faz) = unrotate(rot, 1.0, 0.0);
    // Vertical strips painted in parallel: each strip runs the painter's
    // algorithm over the cells that touch its x range, clipped to it.
    let strips = rayon::current_num_threads().clamp(1, 32).min(w.div_ceil(32));
    let strip_w = w.div_ceil(strips);
    let paint_strip = |xs: usize| -> Vec<u8> {
        let xe = (xs + strip_w).min(w);
        let sw = xe - xs;
        let mut out = vec![0u8; sw * h * 4];
        for px in out.as_chunks_mut::<4>().0 {
            *px = bg_px;
        }
        let (xsf, xef) = (xs as f32, xe as f32);
        let put = |x: i64, y: i64, col: [u8; 4], out: &mut Vec<u8>| {
            if x >= xs as i64 && y >= 0 && (x as usize) < xe && (y as usize) < h {
                let o = (y as usize * sw + (x as usize - xs)) * 4;
                out[o..o + 4].copy_from_slice(&col);
            }
        };
        // Only the cells that can reach this strip: screen x bounds the
        // anti-diagonal (a - b), screen y and the height range bound the
        // diagonal (a + b).
        let e_lo = ((xsf - view.ox) / s / c - 1.0).floor() as i64;
        let e_hi = ((xef - view.ox) / s / c + 1.0).ceil() as i64;
        let d_lo = ((2.0 * (-view.oy + low * view.vs) / s) / c - 2.0).floor().max(0.0) as usize;
        let d_hi = (((2.0 * (hf - view.oy + high * view.vs) / s) / c + 2.0).ceil().max(0.0) as usize).min(2 * n - 2);
        for diag in d_lo..=d_hi {
            let a0 = (diag.saturating_sub(n - 1) as i64).max((diag as i64 + e_lo).div_euclid(2));
            let a1 = ((diag.min(n - 1)) as i64).min((diag as i64 + e_hi).div_euclid(2) + 1);
            if a1 < a0 {
                continue;
            }
            for a in a0.max(0) as usize..=a1.min(diag.min(n - 1) as i64) as usize {
                if a > diag {
                    continue;
                }
                let b = diag - a;
                let top = hh[b * n + a];
                if top.is_nan() {
                    continue;
                }
                let (u, v) = (a as f32 * c, b as f32 * c);
                // Top diamond centre and extents.
                let (cx, cy) = view.proj(u + c / 2.0, v + c / 2.0, top);
                let (ha, hb) = (c * s, c * s / 2.0);
                // Cull (sides can reach far below).
                let depth = (top - low) * view.vs;
                if cx + ha < xsf || cx - ha > xef || cy + hb + depth < 0.0 || cy - hb > hf {
                    continue;
                }
                let col = cc[b * n + a];
                let kind = if textured {
                    let k = kk[b * n + a];
                    (k != crate::voxel::NO_KIND).then(|| grid.kinds.get(k as usize)).flatten()
                } else {
                    None
                };
                let use_tex = kind.is_some_and(|k| !k.water);
                // Side faces: down to the front neighbours (or the lowest cell).
                // Unknown neighbours (grid edge, unloaded) end in a thin slab.
                let front = |aa: usize, bb: usize| if aa < n && bb < n && !hh[bb * n + aa].is_nan() { hh[bb * n + aa] } else { top - 6.0 * c.max(1.0) };
                let left_drop = ((top - front(a, b + 1)).max(0.0)) * view.vs;
                let right_drop = ((top - front(a + 1, b)).max(0.0)) * view.vs;
                let (x0, x1) = (((cx - ha).floor() as i64).max(xs as i64), ((cx + ha).ceil() as i64).min(xe as i64));
                if let (true, Some(k)) = (use_tex, kind) {
                    // World origin of this cell (the painter walks rotated
                    // indices; textures need the original compass position).
                    let (q, r) = unrot_index(rot, n, a, b);
                    let (wx0, wz0) = (grid.x0 + q as f64 * c as f64, grid.z0 + r as f64 * c as f64);
                    let flat = [col[0], col[1], col[2], 255];
                    // Cut-out texels (plants, leaves) take the cell's map color
                    // so they never punch holes down to the sky.
                    let opaque = |t: [u8; 4], k: f32| if t[3] < 128 { flat } else { shade([t[0], t[1], t[2]], k) };
                    // Left face (+v side), textured and sheared.
                    if left_drop > 0.0 {
                        for x in ((cx - ha).floor() as i64).max(xs as i64)..(cx.ceil() as i64).min(xe as i64) {
                            let t = ((x as f32 + 0.5) - (cx - ha)) / ha;
                            let ytop = cy + t * hb;
                            for y in (ytop.floor() as i64).max(0)..((ytop + left_drop).ceil() as i64).min(h as i64) {
                                let wy = top - ((y as f32 + 0.5) - ytop) / view.vs;
                                let (du, dv) = unrotate(rot, (t * c) as f64, c as f64);
                                let tc = texel_row(&k.side, texel_col(wx0 + du, wz0 + dv, fax != 0.0), wy);
                                put(x, y, opaque(tc, 0.72), &mut out);
                            }
                        }
                    }
                    // Right face (+a side).
                    if right_drop > 0.0 {
                        for x in (cx.floor() as i64).max(xs as i64)..((cx + ha).ceil() as i64).min(xe as i64) {
                            let t = ((x as f32 + 0.5) - cx) / ha;
                            let ytop = cy + hb - t * hb;
                            for y in (ytop.floor() as i64).max(0)..((ytop + right_drop).ceil() as i64).min(h as i64) {
                                let wy = top - ((y as f32 + 0.5) - ytop) / view.vs;
                                let (du, dv) = unrotate(rot, c as f64, (t * c) as f64);
                                let tc = texel_row(&k.side, texel_col(wx0 + du, wz0 + dv, faz != 0.0), wy);
                                put(x, y, opaque(tc, 0.56), &mut out);
                            }
                        }
                    }
                    // Top diamond: inverse-map each pixel into the cell and
                    // sample the world-aligned top texture.
                    let (y0, y1) = (((cy - hb).floor() as i64).max(0), ((cy + hb).ceil() as i64).min(h as i64));
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
                            let diff = ((x as f32 + 0.5) - view.ox) / s;
                            let sum = 2.0 * ((y as f32 + 0.5) - view.oy + top * view.vs) / s;
                            let (du, dv) = unrotate(rot, ((diff + sum) / 2.0 - u) as f64, ((sum - diff) / 2.0 - v) as f64);
                            let tc = texel_at(&k.top, wx0 + du, wz0 + dv);
                            put(x, y, opaque(tc, 1.0), &mut out);
                        }
                    }
                    continue;
                }
                // Left face (+v side): x from cx-ha to cx, top edge from (cx-ha, cy) to (cx, cy+hb).
                if left_drop > 0.0 {
                    let lc = shade(col, 0.72);
                    for x in ((cx - ha).floor() as i64).max(xs as i64)..(cx.ceil() as i64).min(xe as i64) {
                        let t = ((x as f32 + 0.5) - (cx - ha)) / ha;
                        let ytop = cy + t * hb;
                        for y in (ytop.floor() as i64).max(0)..((ytop + left_drop).ceil() as i64).min(h as i64) {
                            put(x, y, lc, &mut out);
                        }
                    }
                }
                if right_drop > 0.0 {
                    let rc2 = shade(col, 0.56);
                    for x in (cx.floor() as i64).max(xs as i64)..((cx + ha).ceil() as i64).min(xe as i64) {
                        let t = ((x as f32 + 0.5) - cx) / ha;
                        let ytop = cy + hb - t * hb;
                        for y in (ytop.floor() as i64).max(0)..((ytop + right_drop).ceil() as i64).min(h as i64) {
                            put(x, y, rc2, &mut out);
                        }
                    }
                }
                // Top diamond.
                let tc = [col[0], col[1], col[2], 255];
                let (y0, y1) = (((cy - hb).floor() as i64).max(0), ((cy + hb).ceil() as i64).min(h as i64));
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
    };
    use rayon::prelude::*;
    let parts: Vec<Vec<u8>> = (0..strips).into_par_iter().map(|k| paint_strip(k * strip_w)).collect();
    let mut out = vec![0u8; w * h * 4];
    for (k, part) in parts.iter().enumerate() {
        let xs = k * strip_w;
        if xs >= w {
            continue;
        }
        let sw = (xs + strip_w).min(w) - xs;
        for y in 0..h {
            out[(y * w + xs) * 4..(y * w + xs + sw) * 4].copy_from_slice(&part[y * sw * 4..(y + 1) * sw * 4]);
        }
    }
    out
}
