//! [`ChunkSet`]: a compact set of chunk coordinates, bucketed by 32×32-chunk
//! region with one 1024-bit bitmap per region (128 bytes per region instead
//! of ~24 bytes per chunk in a `HashSet`). Map viewers keep "which chunks
//! exist" for whole worlds in memory — millions of chunks fit in a few MB.

use std::collections::HashMap;

/// Chunks per region edge.
pub const REGION_CHUNKS: i32 = 32;

type Bits = [u64; 16];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChunkSet {
    regions: HashMap<(i32, i32), Box<Bits>>,
    len: usize,
    bounds: Option<(i32, i32, i32, i32)>,
}

#[inline]
fn split(cx: i32, cz: i32) -> ((i32, i32), usize) {
    let r = (cx.div_euclid(REGION_CHUNKS), cz.div_euclid(REGION_CHUNKS));
    let i = (cz.rem_euclid(REGION_CHUNKS) * REGION_CHUNKS + cx.rem_euclid(REGION_CHUNKS)) as usize;
    (r, i)
}

impl ChunkSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a chunk; returns true when it was not present.
    pub fn insert(&mut self, cx: i32, cz: i32) -> bool {
        let (r, i) = split(cx, cz);
        let bits = self.regions.entry(r).or_insert_with(|| Box::new([0; 16]));
        let (w, b) = (i / 64, 1u64 << (i % 64));
        if bits[w] & b != 0 {
            return false;
        }
        bits[w] |= b;
        self.len += 1;
        self.bounds = Some(match self.bounds {
            None => (cx, cz, cx, cz),
            Some((x0, z0, x1, z1)) => (x0.min(cx), z0.min(cz), x1.max(cx), z1.max(cz)),
        });
        true
    }

    pub fn contains(&self, cx: i32, cz: i32) -> bool {
        let (r, i) = split(cx, cz);
        self.regions.get(&r).is_some_and(|bits| bits[i / 64] & (1u64 << (i % 64)) != 0)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Inclusive chunk bounds `(min_x, min_z, max_x, max_z)`.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        self.bounds
    }

    /// Bounds of the world's "core": per axis, the region-granular range
    /// holding all but `trim` (e.g. 0.005 = 0.5 %) of the chunks on each
    /// side. Far-flung outlier chunks (a stray portal trip thousands of
    /// chunks away) otherwise blow a fit-to-world view or a whole-world
    /// image up to gigapixels. Falls back to the full bounds when trimming
    /// would leave nothing.
    pub fn core_bounds(&self, trim: f64) -> Option<(i32, i32, i32, i32)> {
        let full = self.bounds?;
        let mut xs: Vec<(i32, u32)> = Vec::with_capacity(self.regions.len());
        let mut zs: Vec<(i32, u32)> = Vec::with_capacity(self.regions.len());
        for (&(rx, rz), bits) in &self.regions {
            let n: u32 = bits.iter().map(|w| w.count_ones()).sum();
            xs.push((rx, n));
            zs.push((rz, n));
        }
        let cut = (self.len as f64 * trim.clamp(0.0, 0.49)).floor() as u64;
        let range = |v: &mut Vec<(i32, u32)>| -> (i32, i32) {
            v.sort_unstable_by_key(|e| e.0);
            let (mut acc, mut lo) = (0u64, v[0].0);
            for &(r, n) in v.iter() {
                acc += n as u64;
                if acc > cut {
                    lo = r;
                    break;
                }
            }
            let (mut acc, mut hi) = (0u64, v[v.len() - 1].0);
            for &(r, n) in v.iter().rev() {
                acc += n as u64;
                if acc > cut {
                    hi = r;
                    break;
                }
            }
            (lo, hi)
        };
        let (rx0, rx1) = range(&mut xs);
        let (rz0, rz1) = range(&mut zs);
        if rx0 > rx1 || rz0 > rz1 {
            return Some(full);
        }
        let b = (
            (rx0 * REGION_CHUNKS).max(full.0),
            (rz0 * REGION_CHUNKS).max(full.1),
            (rx1 * REGION_CHUNKS + REGION_CHUNKS - 1).min(full.2),
            (rz1 * REGION_CHUNKS + REGION_CHUNKS - 1).min(full.3),
        );
        Some(if b.0 <= b.2 && b.1 <= b.3 { b } else { full })
    }

    /// Number of non-empty regions.
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// Approximate heap bytes held.
    pub fn heap_bytes(&self) -> usize {
        self.regions.len() * (std::mem::size_of::<Bits>() + 32)
    }

    /// Every chunk (unordered).
    pub fn iter(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.regions.iter().flat_map(|(&(rx, rz), bits)| region_iter(rx, rz, bits))
    }

    /// Chunks inside an inclusive chunk rectangle, visiting only the
    /// regions that overlap it.
    pub fn in_rect(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        let (rx0, rz0) = (x0.div_euclid(REGION_CHUNKS), z0.div_euclid(REGION_CHUNKS));
        let (rx1, rz1) = (x1.div_euclid(REGION_CHUNKS), z1.div_euclid(REGION_CHUNKS));
        // Sparse worlds: walking the region map beats probing a huge rect.
        let rect_regions = (rx1 - rx0 + 1) as i64 * (rz1 - rz0 + 1) as i64;
        if rect_regions > self.regions.len() as i64 {
            for (&(rx, rz), bits) in &self.regions {
                if rx >= rx0 && rx <= rx1 && rz >= rz0 && rz <= rz1 {
                    out.extend(region_iter(rx, rz, bits).filter(|&(x, z)| x >= x0 && x <= x1 && z >= z0 && z <= z1));
                }
            }
        } else {
            for rz in rz0..=rz1 {
                for rx in rx0..=rx1 {
                    if let Some(bits) = self.regions.get(&(rx, rz)) {
                        out.extend(region_iter(rx, rz, bits).filter(|&(x, z)| x >= x0 && x <= x1 && z >= z0 && z <= z1));
                    }
                }
            }
        }
        out
    }

    /// Whether any chunk lies inside the inclusive rectangle.
    pub fn any_in_rect(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        let (rx0, rz0) = (x0.div_euclid(REGION_CHUNKS), z0.div_euclid(REGION_CHUNKS));
        let (rx1, rz1) = (x1.div_euclid(REGION_CHUNKS), z1.div_euclid(REGION_CHUNKS));
        let rect_regions = (rx1 - rx0 + 1) as i64 * (rz1 - rz0 + 1) as i64;
        let hit = |rx: i32, rz: i32, bits: &Bits| {
            region_iter(rx, rz, bits).any(|(x, z)| x >= x0 && x <= x1 && z >= z0 && z <= z1)
        };
        if rect_regions > self.regions.len() as i64 {
            self.regions.iter().any(|(&(rx, rz), bits)| rx >= rx0 && rx <= rx1 && rz >= rz0 && rz <= rz1 && hit(rx, rz, bits))
        } else {
            (rz0..=rz1).any(|rz| (rx0..=rx1).any(|rx| self.regions.get(&(rx, rz)).is_some_and(|b| hit(rx, rz, b))))
        }
    }
}

fn region_iter(rx: i32, rz: i32, bits: &Bits) -> impl Iterator<Item = (i32, i32)> + '_ {
    bits.iter().enumerate().flat_map(move |(w, &word)| {
        let mut word = word;
        std::iter::from_fn(move || {
            if word == 0 {
                return None;
            }
            let b = word.trailing_zeros() as usize;
            word &= word - 1;
            let i = (w * 64 + b) as i32;
            Some((rx * REGION_CHUNKS + i % REGION_CHUNKS, rz * REGION_CHUNKS + i / REGION_CHUNKS))
        })
    })
}

impl FromIterator<(i32, i32)> for ChunkSet {
    fn from_iter<T: IntoIterator<Item = (i32, i32)>>(iter: T) -> Self {
        let mut s = Self::new();
        for (x, z) in iter {
            s.insert(x, z);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_contains_bounds_and_negative_coords() {
        let mut s = ChunkSet::new();
        assert!(s.insert(0, 0));
        assert!(!s.insert(0, 0));
        assert!(s.insert(-1, -33));
        assert!(s.insert(31, 31));
        assert!(s.insert(32, 0));
        assert_eq!(s.len(), 4);
        assert!(s.contains(-1, -33) && s.contains(32, 0) && !s.contains(1, 0));
        assert_eq!(s.bounds(), Some((-1, -33, 32, 31)));
        let mut all: Vec<_> = s.iter().collect();
        all.sort_unstable();
        assert_eq!(all, vec![(-1, -33), (0, 0), (31, 31), (32, 0)]);
    }

    #[test]
    fn core_bounds_trims_outliers() {
        let mut s: ChunkSet = (0..64).flat_map(|x| (0..64).map(move |z| (x, z))).collect();
        s.insert(40_000, -30_000); // a stray chunk far away
        assert_eq!(s.bounds(), Some((0, -30_000, 40_000, 63)));
        assert_eq!(s.core_bounds(0.005), Some((0, 0, 63, 63)));
        assert_eq!(s.core_bounds(0.0), s.bounds(), "no trim keeps everything");
        let one: ChunkSet = [(5, 5)].into_iter().collect();
        assert_eq!(one.core_bounds(0.4), Some((5, 5, 5, 5)));
    }

    #[test]
    fn rect_queries_match_brute_force() {
        let pts: Vec<(i32, i32)> = (-70..70).step_by(7).flat_map(|x| (-50..90).step_by(11).map(move |z| (x, z))).collect();
        let s: ChunkSet = pts.iter().copied().collect();
        for &(x0, z0, x1, z1) in &[(-10, -10, 10, 10), (-100, -100, 100, 100), (5, 5, 6, 6), (-64, 0, -33, 31)] {
            let mut got = s.in_rect(x0, z0, x1, z1);
            got.sort_unstable();
            let mut want: Vec<_> = pts.iter().copied().filter(|&(x, z)| x >= x0 && x <= x1 && z >= z0 && z <= z1).collect();
            want.sort_unstable();
            assert_eq!(got, want, "rect {x0},{z0}..{x1},{z1}");
            assert_eq!(s.any_in_rect(x0, z0, x1, z1), !want.is_empty());
        }
    }
}
