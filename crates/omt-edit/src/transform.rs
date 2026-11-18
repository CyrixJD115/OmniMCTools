//! Clipboard transforms: rotate 90° clockwise (seen from above, north up)
//! and mirror along X or Z — positions *and* block states, for both
//! editions' direction properties:
//! * Java: `facing`, `axis`, `rotation` (0–15), connection sides
//!   (`north`/`east`/`south`/`west`), stair `shape`;
//! * Bedrock: `minecraft:cardinal_direction`, `facing_direction` /
//!   `minecraft:facing_direction` (2 N, 3 S, 4 W, 5 E), `direction`
//!   (0 S, 1 W, 2 N, 3 E), `weirdo_direction` (0 E, 1 W, 2 S, 3 N),
//!   `ground_sign_direction` (0–15), `pillar_axis`.
//!
//! Transformed Bedrock entries drop their raw palette NBT; it is rebuilt
//! from the new states when pasted (with the world's block `version`).

use crate::{Block, Volume};

/// A transform of the horizontal plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Xform {
    RotateCw,
    FlipX,
    FlipZ,
}

/// Cardinal index: 0 north, 1 east, 2 south, 3 west.
fn turn(c: u8, x: Xform) -> u8 {
    match x {
        Xform::RotateCw => (c + 1) % 4,
        Xform::FlipX => [0, 3, 2, 1][c as usize],
        Xform::FlipZ => [2, 1, 0, 3][c as usize],
    }
}

const NAMES: [&str; 4] = ["north", "east", "south", "west"];

fn name_dir(v: &str) -> Option<u8> {
    NAMES.iter().position(|n| *n == v).map(|i| i as u8)
}

/// Integer-coded direction tables (value → cardinal), per Bedrock property.
fn int_table(prop: &str) -> Option<&'static [(i64, u8)]> {
    Some(match prop {
        "facing_direction" | "minecraft:facing_direction" => &[(2, 0), (5, 1), (3, 2), (4, 3)],
        "direction" => &[(2, 0), (3, 1), (0, 2), (1, 3)],
        "weirdo_direction" => &[(3, 0), (0, 1), (2, 2), (1, 3)],
        _ => return None,
    })
}

/// 16-step rotations (0 = south, clockwise) under a transform.
fn rot16(r: i64, x: Xform) -> i64 {
    match x {
        Xform::RotateCw => (r + 4).rem_euclid(16),
        Xform::FlipX => (16 - r).rem_euclid(16),
        Xform::FlipZ => (8 - r).rem_euclid(16),
    }
}

fn swap_axis(v: &str, x: Xform) -> String {
    match (v, x) {
        ("x", Xform::RotateCw) => "z".into(),
        ("z", Xform::RotateCw) => "x".into(),
        _ => v.to_string(),
    }
}

/// Transform one block state's properties.
pub fn transform_state(b: &Block, x: Xform) -> Block {
    let mut changed = false;
    let mut props: Vec<(String, String)> = Vec::with_capacity(b.state.properties.len());
    // Connection sides are renamed, so collect them separately first.
    let mut sides: [Option<String>; 4] = Default::default();
    for (k, v) in &b.state.properties {
        let (raw, quoted) = (v.trim_matches('"'), v.starts_with('"'));
        let requote = |s: String| if quoted { format!("\"{s}\"") } else { s };
        if let Some(i) = name_dir(k) {
            sides[i as usize] = Some(v.clone());
            changed = true;
            continue;
        }
        let new = match k.as_str() {
            "facing" | "minecraft:cardinal_direction" => name_dir(raw).map(|c| requote(NAMES[turn(c, x) as usize].to_string())),
            "axis" | "pillar_axis" => Some(requote(swap_axis(raw, x))),
            "rotation" | "ground_sign_direction" => raw.trim_end_matches(['b', 's']).parse::<i64>().ok().map(|r| rot16(r, x).to_string()),
            "shape" if x != Xform::RotateCw => Some(requote(match raw {
                "inner_left" => "inner_right".into(),
                "inner_right" => "inner_left".into(),
                "outer_left" => "outer_right".into(),
                "outer_right" => "outer_left".into(),
                other => other.to_string(),
            })),
            p => int_table(p).and_then(|t| {
                let n: i64 = raw.trim_end_matches(['b', 's']).parse().ok()?;
                let c = t.iter().find(|(v, _)| *v == n)?.1;
                let to = turn(c, x);
                let suffix = if raw.ends_with('b') { "b" } else { "" };
                t.iter().find(|(_, d)| *d == to).map(|(v, _)| format!("{v}{suffix}"))
            }),
        };
        match new {
            Some(n) if n != *v => {
                changed = true;
                props.push((k.clone(), n));
            }
            _ => props.push((k.clone(), v.clone())),
        }
    }
    // A side's value moves to the side it now faces.
    for (i, side) in sides.iter().enumerate() {
        if let Some(v) = side {
            props.push((NAMES[turn(i as u8, x) as usize].to_string(), v.clone()));
        }
    }
    props.sort_by(|a, b| a.0.cmp(&b.0));
    if !changed {
        return b.clone();
    }
    let mut state = b.state.clone();
    state.properties = props;
    Block { state, bedrock_raw: None }
}

/// Transform a whole volume (positions + states).
pub fn transform(v: &Volume, x: Xform) -> Volume {
    let [sx, sy, sz] = v.size;
    let size = match x {
        Xform::RotateCw => [sz, sy, sx],
        _ => v.size,
    };
    let palette: Vec<Block> = v.palette.iter().map(|b| transform_state(b, x)).collect();
    let mut idx = vec![0u32; v.idx.len()];
    for y in 0..sy {
        for z in 0..sz {
            for xx in 0..sx {
                let (nx, nz) = match x {
                    Xform::RotateCw => (sz - 1 - z, xx),
                    Xform::FlipX => (sx - 1 - xx, z),
                    Xform::FlipZ => (xx, sz - 1 - z),
                };
                let src = ((y * sz + z) * sx + xx) as usize;
                let dst = ((y * size[2] + nz) * size[0] + nx) as usize;
                idx[dst] = v.idx[src];
            }
        }
    }
    Volume { size, palette, idx }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VolumeBuilder;
    use omt_world::BlockState;

    fn st(name: &str, props: &[(&str, &str)]) -> Block {
        Block { state: BlockState { name: name.into(), properties: props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }, bedrock_raw: None }
    }

    #[test]
    fn java_and_bedrock_states_rotate() {
        let p = |b: &Block, k: &str| b.state.properties.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
        let stairs = st("minecraft:oak_stairs", &[("facing", "north"), ("shape", "inner_left")]);
        let r = transform_state(&stairs, Xform::RotateCw);
        assert_eq!(p(&r, "facing").as_deref(), Some("east"));
        assert_eq!(p(&r, "shape").as_deref(), Some("inner_left"), "rotation keeps handedness");
        let m = transform_state(&stairs, Xform::FlipX);
        assert_eq!((p(&m, "facing").as_deref(), p(&m, "shape").as_deref()), (Some("north"), Some("inner_right")));
        let log = st("minecraft:oak_log", &[("pillar_axis", "x")]);
        assert_eq!(p(&transform_state(&log, Xform::RotateCw), "pillar_axis").as_deref(), Some("z"));
        let sign = st("minecraft:standing_sign", &[("ground_sign_direction", "3")]);
        assert_eq!(p(&transform_state(&sign, Xform::RotateCw), "ground_sign_direction").as_deref(), Some("7"));
        let furnace = st("minecraft:furnace", &[("minecraft:cardinal_direction", "\"south\"")]);
        assert_eq!(p(&transform_state(&furnace, Xform::RotateCw), "minecraft:cardinal_direction").as_deref(), Some("\"west\""));
        let hopper = st("minecraft:hopper", &[("facing_direction", "2")]);
        assert_eq!(p(&transform_state(&hopper, Xform::RotateCw), "facing_direction").as_deref(), Some("5"), "north → east");
        let bstairs = st("minecraft:stone_stairs", &[("weirdo_direction", "3")]);
        assert_eq!(p(&transform_state(&bstairs, Xform::FlipZ), "weirdo_direction").as_deref(), Some("2"), "north → south");
        let fence = st("minecraft:oak_fence", &[("east", "true"), ("north", "false"), ("south", "false"), ("west", "false")]);
        let f = transform_state(&fence, Xform::RotateCw);
        assert_eq!(p(&f, "south").as_deref(), Some("true"));
        assert_eq!(p(&f, "east").as_deref(), Some("false"));
        assert!(transform_state(&st("minecraft:stone", &[]), Xform::RotateCw).bedrock_raw.is_none());
    }

    #[test]
    fn positions_rotate_and_flip() {
        let mut vb = VolumeBuilder::new([3, 1, 2]);
        let s = vb.id(&Block::named("stone"));
        vb.set(0, 0, 0, s); // north-west corner
        let v = vb.finish();
        let r = transform(&v, Xform::RotateCw);
        assert_eq!(r.size, [2, 1, 3]);
        assert_eq!(r.at(1, 0, 0).unwrap().state.name, "minecraft:stone", "NW → NE after a clockwise turn");
        let fx = transform(&v, Xform::FlipX);
        assert_eq!(fx.at(2, 0, 0).unwrap().state.name, "minecraft:stone");
        let four = (0..4).fold(v.clone(), |acc, _| transform(&acc, Xform::RotateCw));
        assert_eq!(four.idx, v.idx, "four turns = identity");
    }
}
