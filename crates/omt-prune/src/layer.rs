//! Selection layers — the pruner's data model.
//!
//! Mirrors UMT's observable pruner behavior (feature page + guide): every
//! tool adds one *layer*; a layer selects chunks (shown red); layers combine
//! by **union** ("order does not matter, no layer limit"); each layer can be
//! **inverted**; *Prune* deletes every selected chunk. Shapes are stored in
//! block coordinates so the map UI can draw them unsnapped; a chunk belongs
//! to a shape when its **center** (`x*16+8`, `z*16+8`) does — with snapping
//! on (control points on chunk corners) that equals "the chunk is inside".

use serde::{Deserialize, Serialize};

/// Comparison for the Time (InhabitedTime) tool: UMT offers `>`, `=`, `<`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeOp {
    Greater,
    Equal,
    Less,
}

impl TimeOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Greater => ">",
            Self::Equal => "=",
            Self::Less => "<",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            ">" | "gt" => Some(Self::Greater),
            "=" | "==" | "eq" => Some(Self::Equal),
            "<" | "lt" => Some(Self::Less),
            _ => None,
        }
    }

    pub fn holds(self, value: i64, threshold: i64) -> bool {
        match self {
            Self::Greater => value > threshold,
            Self::Equal => value == threshold,
            Self::Less => value < threshold,
        }
    }
}

/// One pruning tool's geometry (block coordinates, `f64`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// Axis-aligned rectangle between two corners (inclusive block extents).
    Box { x0: f64, z0: f64, x1: f64, z1: f64 },
    /// Center + radius in blocks.
    Circle { cx: f64, cz: f64, radius: f64 },
    /// Closed polygon (even-odd rule); fewer than 3 points selects nothing.
    Polygon { points: Vec<(f64, f64)> },
    /// Infinite half-plane: everything to the *left* of the directed line
    /// through `(x, z)` at `angle_deg` (0° = +X/east, 90° = +Z/south).
    Split { x: f64, z: f64, angle_deg: f64 },
    /// Chunks whose `InhabitedTime` (ticks) compares true. Java only —
    /// Bedrock does not store the value.
    Time { op: TimeOp, ticks: i64 },
}

impl Shape {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Box { .. } => "Box",
            Self::Circle { .. } => "Circle",
            Self::Polygon { .. } => "Polygon",
            Self::Split { .. } => "Split",
            Self::Time { .. } => "Time",
        }
    }

    /// Geometric containment of a block-space point. `Time` is not
    /// geometric and returns `false` here (see [`Layer::selects`]).
    pub fn contains_point(&self, px: f64, pz: f64) -> bool {
        match self {
            Self::Box { x0, z0, x1, z1 } => {
                let (lx, hx) = (x0.min(*x1), x0.max(*x1) + 1.0);
                let (lz, hz) = (z0.min(*z1), z0.max(*z1) + 1.0);
                px >= lx && px < hx && pz >= lz && pz < hz
            }
            Self::Circle { cx, cz, radius } => {
                let (dx, dz) = (px - cx, pz - cz);
                dx * dx + dz * dz <= radius * radius
            }
            Self::Polygon { points } => point_in_polygon(points, px, pz),
            Self::Split { x, z, angle_deg } => {
                let a = angle_deg.to_radians();
                let (dx, dz) = (a.cos(), a.sin());
                // Minecraft's map has +Z pointing down the screen; "left of
                // the direction" is the positive cross-product side there.
                dx * (pz - z) - dz * (px - x) >= 0.0
            }
            Self::Time { .. } => false,
        }
    }

    /// Chunk-space bounding box `(min_x, min_z, max_x, max_z)` of a
    /// geometric shape, `None` for unbounded shapes (Split, Time).
    pub fn chunk_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let to = |b: f64| (b / 16.0).floor() as i32;
        match self {
            Self::Box { x0, z0, x1, z1 } => Some((
                to(x0.min(*x1)),
                to(z0.min(*z1)),
                to(x0.max(*x1)),
                to(z0.max(*z1)),
            )),
            Self::Circle { cx, cz, radius } => {
                Some((to(cx - radius), to(cz - radius), to(cx + radius), to(cz + radius)))
            }
            Self::Polygon { points } if !points.is_empty() => {
                let (mut a, mut b, mut c, mut d) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for &(x, z) in points {
                    a = a.min(x);
                    b = b.min(z);
                    c = c.max(x);
                    d = d.max(z);
                }
                Some((to(a), to(b), to(c), to(d)))
            }
            _ => None,
        }
    }
}

fn point_in_polygon(points: &[(f64, f64)], px: f64, pz: f64) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (xi, zi) = points[i];
        let (xj, zj) = points[j];
        if (zi > pz) != (zj > pz) && px < (xj - xi) * (pz - zi) / (zj - zi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// One selection layer in the sidebar list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub shape: Shape,
    /// UMT's "Inverted" toggle: select everything the shape does not.
    pub inverted: bool,
    /// Disabled layers stay in the list but select nothing.
    pub enabled: bool,
}

impl Layer {
    pub fn new(shape: Shape) -> Self {
        Self { shape, inverted: false, enabled: true }
    }

    pub fn inverted(mut self) -> Self {
        self.inverted = !self.inverted;
        self
    }

    /// Whether this layer selects chunk `(cx, cz)`. `inhabited` is the
    /// chunk's InhabitedTime when the edition stores one; a Time layer on a
    /// world without it selects nothing (inverted or not) instead of
    /// accidentally selecting the whole world.
    pub fn selects(&self, cx: i32, cz: i32, inhabited: Option<i64>) -> bool {
        if !self.enabled {
            return false;
        }
        let raw = match &self.shape {
            Shape::Time { op, ticks } => match inhabited {
                Some(v) => op.holds(v, *ticks),
                None => return false,
            },
            shape => shape.contains_point(cx as f64 * 16.0 + 8.0, cz as f64 * 16.0 + 8.0),
        };
        raw != self.inverted
    }
}

/// The full layer stack. A chunk is selected when **any** enabled layer
/// selects it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub layers: Vec<Layer>,
}

impl Selection {
    pub fn new(layers: Vec<Layer>) -> Self {
        Self { layers }
    }

    pub fn is_empty(&self) -> bool {
        !self.layers.iter().any(|l| l.enabled)
    }

    /// True when some enabled layer needs per-chunk InhabitedTime.
    pub fn needs_inhabited_time(&self) -> bool {
        self.layers.iter().any(|l| l.enabled && matches!(l.shape, Shape::Time { .. }))
    }

    pub fn selects(&self, cx: i32, cz: i32, inhabited: Option<i64>) -> bool {
        self.layers.iter().any(|l| l.selects(cx, cz, inhabited))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(shape: Shape) -> Layer {
        Layer::new(shape)
    }

    #[test]
    fn box_selects_chunks_inside() {
        // Blocks 0..=31 x 0..=15 → chunks (0,0) and (1,0).
        let l = sel(Shape::Box { x0: 0.0, z0: 0.0, x1: 31.0, z1: 15.0 });
        assert!(l.selects(0, 0, None));
        assert!(l.selects(1, 0, None));
        assert!(!l.selects(2, 0, None));
        assert!(!l.selects(0, 1, None));
        assert!(!l.selects(-1, 0, None));
        // Corner order doesn't matter.
        let r = sel(Shape::Box { x0: 31.0, z0: 15.0, x1: 0.0, z1: 0.0 });
        assert!(r.selects(1, 0, None));
    }

    #[test]
    fn circle_and_inversion() {
        let l = sel(Shape::Circle { cx: 8.0, cz: 8.0, radius: 20.0 });
        assert!(l.selects(0, 0, None));
        assert!(l.selects(1, 0, None)); // center (24,8) at distance 16
        assert!(!l.selects(2, 0, None)); // (40,8) at 32
        let inv = l.clone().inverted();
        assert!(!inv.selects(0, 0, None));
        assert!(inv.selects(5, 5, None));
    }

    #[test]
    fn polygon_triangle() {
        let l = sel(Shape::Polygon { points: vec![(0.0, 0.0), (64.0, 0.0), (0.0, 64.0)] });
        assert!(l.selects(0, 0, None));
        assert!(l.selects(1, 1, None)); // (24,24): 24+24 < 64
        assert!(!l.selects(3, 3, None));
        assert!(!sel(Shape::Polygon { points: vec![(0.0, 0.0), (1.0, 1.0)] }).selects(0, 0, None));
    }

    #[test]
    fn split_half_plane() {
        // Line along +X through the origin: "left" is +Z (south, down-screen).
        let l = sel(Shape::Split { x: 0.0, z: 0.0, angle_deg: 0.0 });
        assert!(l.selects(0, 0, None));
        assert!(l.selects(10, 3, None));
        assert!(!l.selects(0, -1, None));
        // Rotated 180°: the other side.
        let r = sel(Shape::Split { x: 0.0, z: 0.0, angle_deg: 180.0 });
        assert!(r.selects(0, -1, None));
        assert!(!r.selects(0, 1, None));
    }

    #[test]
    fn time_layer_semantics() {
        let l = sel(Shape::Time { op: TimeOp::Less, ticks: 1200 });
        assert!(l.selects(0, 0, Some(0)));
        assert!(!l.selects(0, 0, Some(1200)));
        // No InhabitedTime (Bedrock) → never selects, even inverted.
        assert!(!l.selects(0, 0, None));
        assert!(!l.clone().inverted().selects(0, 0, None));
        assert!(TimeOp::Greater.holds(5, 4) && TimeOp::Equal.holds(4, 4));
    }

    #[test]
    fn union_and_disabled() {
        let mut s = Selection::new(vec![
            sel(Shape::Box { x0: 0.0, z0: 0.0, x1: 15.0, z1: 15.0 }),
            sel(Shape::Box { x0: 160.0, z0: 0.0, x1: 175.0, z1: 15.0 }),
        ]);
        assert!(s.selects(0, 0, None) && s.selects(10, 0, None));
        assert!(!s.selects(5, 0, None));
        s.layers[1].enabled = false;
        assert!(!s.selects(10, 0, None));
        assert!(!s.needs_inhabited_time());
    }

    #[test]
    fn bounds() {
        let b = Shape::Circle { cx: 0.0, cz: 0.0, radius: 17.0 }.chunk_bounds();
        assert_eq!(b, Some((-2, -2, 1, 1)));
        assert_eq!(Shape::Split { x: 0.0, z: 0.0, angle_deg: 0.0 }.chunk_bounds(), None);
    }
}
