//! Generator-side multi-property computations for the semantic assembler:
//! state families that are a function of *several* Java properties at once
//! (bitfields and combined tables) — not expressible as per-property maps.
//! Value semantics transcribed from the MIT-licensed HiveGamesOSS/Chunker
//! reference tables (attributed in the emitted table header) and verified
//! against the fixture-world palette probes.

use std::collections::BTreeMap;

use crate::handmaps::Computed;

/// Generator-side multi-property computations. Each consumes a fixed set
/// of Java properties and writes the computed Bedrock state(s); returns
/// the consumed property names.
pub fn run_computed(
    kind: Computed,
    state: &BTreeMap<String, String>,
    out: &mut BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let flag = |name: &str| -> Result<bool, String> {
        match state.get(name) {
            Some(v) => match v.as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                other => Err(format!("computed face property {name}={other:?} not a bool")),
            },
            None => Err(format!("computed input property {name} missing")),
        }
    };
    let consumed: Vec<String> = match kind {
        Computed::MultiFaceBits => {
            // `glow_lichen` spells the bottom face `bottom`, `resin_clump`
            // spells it `down` — consume whichever the state carries.
            let bottom_prop = if state.contains_key("bottom") { "bottom" } else { "down" };
            let (down, up, south, west, north, east) = (
                flag(bottom_prop)?,
                flag("up")?,
                flag("south")?,
                flag("west")?,
                flag("north")?,
                flag("east")?,
            );
            let bits = down as i32 | (up as i32) << 1 | (south as i32) << 2
                | (west as i32) << 3 | (north as i32) << 4 | (east as i32) << 5;
            out.insert("multi_face_direction_bits".into(), bits.to_string());
            vec![
                bottom_prop.into(),
                "up".into(),
                "south".into(),
                "west".into(),
                "north".into(),
                "east".into(),
            ]
        }
        Computed::VineBits => {
            let (south, west, north, east) =
                (flag("south")?, flag("west")?, flag("north")?, flag("east")?);
            let bits = south as i32 | (west as i32) << 1 | (north as i32) << 2 | (east as i32) << 3;
            out.insert("vine_direction_bits".into(), bits.to_string());
            vec!["south".into(), "west".into(), "north".into(), "east".into()]
        }
        Computed::BooksStored => {
            let mut bits = 0i32;
            let mut consumed = Vec::new();
            for slot in 0..6usize {
                if flag(&format!("slot_{slot}_occupied"))? {
                    bits |= 1 << slot;
                }
                consumed.push(format!("slot_{slot}_occupied"));
            }
            out.insert("books_stored".into(), bits.to_string());
            consumed
        }
        Computed::ButtonFacing => {
            let face = state
                .get("face")
                .ok_or_else(|| "computed input property face missing".to_string())?;
            let facing = state.get("facing").cloned().unwrap_or_default();
            let value = match (face.as_str(), facing.as_str()) {
                ("floor", _) => "1",
                ("ceiling", _) => "0",
                ("wall", "north") => "2",
                ("wall", "south") => "3",
                ("wall", "west") => "4",
                ("wall", "east") => "5",
                (other, _) => return Err(format!("button face {other:?} unmapped")),
            };
            out.insert("facing_direction".into(), value.into());
            vec!["face".into(), "facing".into()]
        }
        Computed::LeverDirection => {
            let face = state
                .get("face")
                .ok_or_else(|| "computed input property face missing".to_string())?;
            let facing = state.get("facing").cloned().unwrap_or_default();
            let value = match (face.as_str(), facing.as_str()) {
                ("floor", "north" | "south") => "up_north_south",
                ("floor", "east" | "west") => "up_east_west",
                ("wall", "north") => "north",
                ("wall", "south") => "south",
                ("wall", "west") => "west",
                ("wall", "east") => "east",
                ("ceiling", "north" | "south") => "down_north_south",
                ("ceiling", "east" | "west") => "down_east_west",
                (other, _) => return Err(format!("lever face/facing {other:?} unmapped")),
            };
            out.insert("lever_direction".into(), value.into());
            vec!["face".into(), "facing".into()]
        }
        Computed::MushroomFaceBits => {
            // Partial table from the Chunker reference (MIT):
            // Java face booleans (north, east, south, west, up, down) →
            // Bedrock `huge_mushroom_bits`; first row wins per combo.
            const TABLE: &[([bool; 6], i32)] = &[
                ([true, true, true, true, true, true], 14),
                ([true, true, true, true, false, true], 14),
                ([true, true, false, false, true, false], 3),
                ([true, false, false, true, true, false], 1),
                ([true, false, false, false, true, false], 2),
                ([false, true, true, false, true, false], 9),
                ([false, true, false, false, true, false], 6),
                ([false, false, true, true, true, false], 7),
                ([false, false, true, false, true, false], 8),
                ([false, false, false, true, true, false], 4),
                ([false, false, false, false, true, false], 5),
                ([false, false, false, false, false, false], 0),
                ([true, true, true, true, true, false], 14),
                ([true, true, true, true, false, false], 14),
                ([true, true, true, false, true, true], 14),
                ([true, true, true, false, true, false], 14),
                ([true, true, true, false, false, true], 14),
                ([true, true, true, false, false, false], 14),
                ([true, true, false, true, true, true], 14),
                ([true, true, false, true, true, false], 14),
                ([true, true, false, true, false, true], 14),
                ([true, true, false, true, false, false], 14),
                ([true, true, false, false, true, true], 3),
                ([true, true, false, false, false, true], 3),
                ([true, true, false, false, false, false], 3),
                ([true, false, true, true, true, true], 14),
                ([true, false, true, true, true, false], 14),
                ([true, false, true, true, false, true], 14),
                ([true, false, true, true, false, false], 14),
                ([true, false, true, false, true, true], 14),
                ([true, false, true, false, true, false], 14),
                ([true, false, true, false, false, true], 14),
                ([true, false, true, false, false, false], 0),
                ([true, false, false, true, true, true], 1),
                ([true, false, false, true, false, true], 1),
                ([true, false, false, true, false, false], 1),
                ([true, false, false, false, true, true], 2),
                ([true, false, false, false, false, true], 2),
                ([true, false, false, false, false, false], 2),
                ([false, true, true, true, true, true], 14),
                ([false, true, true, true, true, false], 14),
                ([false, true, true, true, false, true], 14),
                ([false, true, true, true, false, false], 14),
                ([false, true, true, false, true, true], 9),
                ([false, true, true, false, false, true], 9),
                ([false, true, true, false, false, false], 9),
                ([false, true, false, true, true, true], 14),
                ([false, true, false, true, true, false], 14),
                ([false, true, false, true, false, true], 14),
                ([false, true, false, true, false, false], 0),
                ([false, true, false, false, true, true], 6),
                ([false, true, false, false, false, true], 6),
                ([false, true, false, false, false, false], 6),
                ([false, false, true, true, true, true], 7),
                ([false, false, true, true, false, true], 7),
                ([false, false, true, true, false, false], 7),
                ([false, false, true, false, true, true], 8),
                ([false, false, true, false, false, true], 8),
                ([false, false, true, false, false, false], 8),
                ([false, false, false, true, true, true], 4),
                ([false, false, false, true, false, true], 4),
                ([false, false, false, true, false, false], 4),
                ([false, false, false, false, true, true], 0),
                ([false, false, false, false, false, true], 0),
                ([false, false, false, false, false, false], 13),
                ([false, false, false, false, false, false], 12),
                ([false, false, false, false, false, false], 11),
            ];
            let (n, e, s, w, u, d) = (
                flag("north")?,
                flag("east")?,
                flag("south")?,
                flag("west")?,
                flag("up")?,
                flag("down")?,
            );
            let faces = [n, e, s, w, u, d];
            for (pattern_faces, bits) in TABLE {
                if *pattern_faces == faces {
                    out.insert("huge_mushroom_bits".into(), bits.to_string());
                    return Ok(vec![
                        "north".into(), "east".into(), "south".into(),
                        "west".into(), "up".into(), "down".into(),
                    ]);
                }
            }
            return Err(format!(
                "mushroom face combo {faces:?} absent from the reference table"
            ));
        }
    };
    Ok(consumed)
}
