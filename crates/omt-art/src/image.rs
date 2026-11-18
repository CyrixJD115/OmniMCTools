//! A tiny straight-alpha RGBA8 canvas with the handful of operations the
//! generators need (no external image crate: the GUI hands the bytes to
//! iced directly, the asset tool writes PNGs).

/// An RGBA color (straight alpha).
pub type Rgba = [u8; 4];

/// Straight-alpha RGBA8 image, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// `#rrggbb` → opaque color (compile-time friendly).
pub const fn hex(v: u32) -> Rgba {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]
}

/// Scale a color's RGB by `f` (alpha kept).
pub fn shade(c: Rgba, f: f32) -> Rgba {
    let s = |v: u8| (v as f32 * f).round().clamp(0.0, 255.0) as u8;
    [s(c[0]), s(c[1]), s(c[2]), c[3]]
}

/// Linear blend of two colors (`t` = 0 → `a`, 1 → `b`).
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [l(a[0], b[0]), l(a[1], b[1]), l(a[2], b[2]), l(a[3], b[3])]
}

impl Image {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, rgba: vec![0; (w * h * 4) as usize] }
    }

    pub fn filled(w: u32, h: u32, c: Rgba) -> Self {
        let mut img = Self::new(w, h);
        img.fill_rect(0, 0, w as i32, h as i32, c);
        img
    }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h).then(|| ((y as u32 * self.w + x as u32) * 4) as usize)
    }

    pub fn get(&self, x: i32, y: i32) -> Rgba {
        match self.idx(x, y) {
            Some(i) => [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]],
            None => [0; 4],
        }
    }

    /// Overwrite a pixel (out of bounds is ignored).
    pub fn set(&mut self, x: i32, y: i32, c: Rgba) {
        if let Some(i) = self.idx(x, y) {
            self.rgba[i..i + 4].copy_from_slice(&c);
        }
    }

    /// Source-over composite of one pixel.
    pub fn blend(&mut self, x: i32, y: i32, c: Rgba) {
        let Some(i) = self.idx(x, y) else { return };
        let sa = c[3] as f32 / 255.0;
        if sa <= 0.0 {
            return;
        }
        let da = self.rgba[i + 3] as f32 / 255.0;
        let oa = sa + da * (1.0 - sa);
        for (k, &ck) in c.iter().take(3).enumerate() {
            let v = (ck as f32 * sa + self.rgba[i + k] as f32 * da * (1.0 - sa)) / oa;
            self.rgba[i + k] = v.round().clamp(0.0, 255.0) as u8;
        }
        self.rgba[i + 3] = (oa * 255.0).round() as u8;
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        for yy in y.max(0)..(y + h).min(self.h as i32) {
            for xx in x.max(0)..(x + w).min(self.w as i32) {
                self.blend(xx, yy, c);
            }
        }
    }

    /// Composite `src` at (`x`, `y`).
    pub fn draw(&mut self, src: &Image, x: i32, y: i32) {
        for sy in 0..src.h as i32 {
            for sx in 0..src.w as i32 {
                self.blend(x + sx, y + sy, src.get(sx, sy));
            }
        }
    }

    /// Nearest-neighbor upscale by an integer factor (crisp pixels).
    pub fn scaled(&self, k: u32) -> Image {
        let mut out = Image::new(self.w * k, self.h * k);
        for y in 0..out.h {
            for x in 0..out.w {
                let c = self.get((x / k) as i32, (y / k) as i32);
                out.set(x as i32, y as i32, c);
            }
        }
        out
    }

    /// A 1-pixel hard drop shadow (the classic pixel-UI text/icon look).
    pub fn with_shadow(&self, dx: i32, dy: i32, color: Rgba) -> Image {
        let mut out = Image::new(self.w + dx.unsigned_abs(), self.h + dy.unsigned_abs());
        let (ox, oy) = (dx.min(0).abs(), dy.min(0).abs());
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                if self.get(x, y)[3] > 0 {
                    let mut c = color;
                    c[3] = ((color[3] as u32 * self.get(x, y)[3] as u32) / 255) as u8;
                    out.blend(x + ox + dx, y + oy + dy, c);
                }
            }
        }
        out.draw(self, ox, oy);
        out
    }

    /// Fraction of non-transparent pixels (tests / sanity checks).
    pub fn coverage(&self) -> f32 {
        let n = self.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count();
        n as f32 / (self.w * self.h).max(1) as f32
    }
}

/// Deterministic SplitMix64 PRNG (art must be identical on every run).
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform integer in `[lo, hi)`.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo) as u64) as i32
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
}

/// Hash of an integer lattice point → `[0, 1)` (stateless noise basis).
pub fn hash2(seed: u64, x: i32, y: i32) -> f32 {
    let mut r = Rng::new(seed ^ (x as u32 as u64) << 32 ^ (y as u32 as u64).wrapping_mul(0x632b_e59b_d9b4_e019));
    r.f32()
}

/// Smooth 1-D value noise in `[0, 1)`.
pub fn noise1(seed: u64, x: f32) -> f32 {
    let i = x.floor();
    let t = x - i;
    let t = t * t * (3.0 - 2.0 * t);
    let a = hash2(seed, i as i32, 0);
    let b = hash2(seed, i as i32 + 1, 0);
    a + (b - a) * t
}

/// Fractal 1-D noise (octaves of [`noise1`]), `[0, 1)`.
pub fn fbm1(seed: u64, x: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += noise1(seed.wrapping_add(o as u64 * 7919), x * freq) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_and_draw() {
        let mut a = Image::filled(2, 2, hex(0x000000));
        a.blend(0, 0, [255, 255, 255, 128]);
        let p = a.get(0, 0);
        assert!(p[0] > 120 && p[0] < 135 && p[3] == 255);
        let mut b = Image::new(4, 4);
        b.draw(&a, 1, 1);
        assert_eq!(b.get(0, 0)[3], 0);
        assert_eq!(b.get(2, 2), hex(0));
        assert_eq!(a.scaled(3).w, 6);
    }

    #[test]
    fn rng_is_deterministic() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..10 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let n = fbm1(1, 3.3, 4);
        assert!((0.0..1.0).contains(&n));
        assert_eq!(hash2(3, 4, 5), hash2(3, 4, 5));
    }

    #[test]
    fn shadow_grows_canvas() {
        let mut a = Image::new(2, 2);
        a.set(0, 0, hex(0xffffff));
        let s = a.with_shadow(1, 1, [0, 0, 0, 255]);
        assert_eq!((s.w, s.h), (3, 3));
        assert_eq!(s.get(1, 1), [0, 0, 0, 255]);
    }
}
