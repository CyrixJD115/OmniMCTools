//! Empirical probe: what does the data3D heightmap measure, and in which
//! column order? Compares each chunk's stored heightmap against surfaces
//! computed from its own subchunks, under every plausible hypothesis.
//!
//! Usage: `cargo run -p omt-bedrock --example heightprobe -- <world-dir>`
//!
//! Answers printed per hypothesis (absolute Y semantics; "solid" excludes
//! air AND water/lava, "non-air" excludes air only):
//!   non-air+1 x-major / z-major, solid+1 x-major / z-major, …
//! The hypothesis with all-columns agreement wins (fixture-derived fact).

use bedrock_leveldb::{Db, OpenOptions};
use omt_bedrock::keys;
use omt_bedrock::subchunk::{Data3d, Subchunk};
use std::path::PathBuf;

fn main() {
    let world = PathBuf::from(std::env::args().nth(1).expect("usage: heightprobe <world-dir>"));
    let db = Db::open(world.join("db"), OpenOptions { read_only: true, ..Default::default() })
        .expect("open db");
    let snapshot = db.snapshot().expect("snapshot");

    // Collect (x, z) columns that have both data3D and at least one subchunk.
    let mut columns: Vec<(i32, i32)> = Vec::new();
    let mut has_data3d = std::collections::BTreeSet::new();
    let mut has_sub = std::collections::BTreeSet::new();
    for (k, _) in snapshot.iter() {
        let k = k.to_vec();
        match keys::decode_key(&k) {
            keys::DbKey::ChunkRecord { x, z, dim: 0, tag: keys::tag::DATA_3D, .. } => {
                has_data3d.insert((x, z));
            }
            keys::DbKey::Subchunk { x, z, dim: 0, .. } => {
                has_sub.insert((x, z));
            }
            _ => {}
        }
    }
    for c in &has_data3d {
        if has_sub.contains(c) {
            columns.push(*c);
        }
    }
    columns.sort();
    println!("columns with data3D + subchunks: {}", columns.len());

    let mut scores: Vec<(&str, u64, u64)> = Vec::new(); // (hypothesis, agree, total)
    let mut mismatch_names: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut version_seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut palette_entries_checked = 0usize;

    for &(cx, cz) in columns.iter().take(40) {
        let Some(raw) = db.get(&keys::data3d_key(cx, cz, 0)).expect("get") else { continue };
        let Ok(data) = Data3d::parse(&raw) else { continue };

        // Decode all subchunks of this column.
        let mut subs: Vec<(i8, Subchunk)> = Vec::new();
        for y in -4i8..=19 {
            let Some(sraw) = db.get(&keys::subchunk_key(cx, cz, 0, y)).expect("get") else {
                continue;
            };
            if let Ok(sc) = Subchunk::parse(&sraw, Some(y)) {
                subs.push((y, sc));
            }
        }
        if subs.is_empty() {
            continue;
        }

        // Palette version ints.
        for (_, sc) in &subs {
            for st in &sc.storages {
                if let Some(entries) = &st.raw_entries {
                    for e in entries {
                        palette_entries_checked += 1;
                        if let omt_nbt::Tag::Compound(fields) = e {
                            if let Some(omt_nbt::Tag::Int(v)) = fields.iter().find(|(k, _)| k == "version").map(|(_, t)| t) {
                                *version_seen.entry(v.to_string()).or_default() += 1;
                            }
                        }
                    }
                }
            }
        }

        const SNOW: &str = "minecraft:snow_layer";
        const DECOR: &[&str] = &[
            SNOW,
            "minecraft:short_grass", "minecraft:grass", "minecraft:tall_grass",
            "minecraft:fern", "minecraft:large_fern", "minecraft:dandelion",
            "minecraft:poppy", "minecraft:blue_orchid", "minecraft:allium",
            "minecraft:oxeye_daisy", "minecraft:cornflower",
            "minecraft:lily_of_the_valley", "minecraft:wither_rose",
            "minecraft:sweet_berry_bush", "minecraft:dead_bush",
            "minecraft:red_mushroom", "minecraft:brown_mushroom",
            "minecraft:torch", "minecraft:soul_torch", "minecraft:redstone_torch",
            "minecraft:leaf_litter", "minecraft:wildflowers", "minecraft:azure_bluet",
            "minecraft:bush", "minecraft:peony", "minecraft:sunflower", "minecraft:lilac",
            "minecraft:rose_bush", "minecraft:peony", "minecraft:snow", "minecraft:carpet",
        ];
        // Compute surfaces per column, walking slots TOP-DOWN.
        let by_y: std::collections::BTreeMap<i8, &Subchunk> =
            subs.iter().map(|(y, sc)| (*y, sc)).collect();
        let mut nonair_top = vec![i32::MIN; 256];
        let mut solid_top = vec![i32::MIN; 256];
        let mut snow_top = vec![i32::MIN; 256];
        let mut decor_top = vec![i32::MIN; 256];
        let mut top_block: Vec<Option<String>> = vec![None; 256];
        for y in (-4i8..=19).rev() {
            let Some(sc) = by_y.get(&y) else { continue };
            for x in 0..16usize {
                for z in 0..16usize {
                    let i = x * 16 + z;
                    if nonair_top[i] != i32::MIN
                        && solid_top[i] != i32::MIN
                        && snow_top[i] != i32::MIN
                        && decor_top[i] != i32::MIN
                    {
                        continue;
                    }
                    for iny in (0..16usize).rev() {
                        let wy = y as i32 * 16 + iny as i32;
                        if let Some(b) = sc.block(x, iny, z) {
                            if b.name == "minecraft:air" {
                                continue;
                            }
                            if nonair_top[i] == i32::MIN {
                                nonair_top[i] = wy;
                                top_block[i] = Some(b.name.clone());
                            }
                            let fluid = b.name == "minecraft:water"
                                || b.name == "minecraft:flowing_water"
                                || b.name == "minecraft:lava"
                                || b.name == "minecraft:flowing_lava";
                            if !fluid && solid_top[i] == i32::MIN {
                                solid_top[i] = wy;
                            }
                            if b.name != SNOW && snow_top[i] == i32::MIN {
                                snow_top[i] = wy;
                            }
                            if !DECOR.contains(&b.name.as_str()) && decor_top[i] == i32::MIN {
                                decor_top[i] = wy;
                            }
                            if nonair_top[i] != i32::MIN
                                && solid_top[i] != i32::MIN
                                && snow_top[i] != i32::MIN
                                && decor_top[i] != i32::MIN
                            {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Score every hypothesis. Java order differs from Bedrock's block
        // order, so test both column orders for the *stored* map.
        let mut mismatch_shown = 0u32;
        for (surface_name, surface) in [
            ("non-air", &nonair_top),
            ("excl-snow", &snow_top),
            ("excl-decor", &decor_top),
            ("solid", &solid_top),
        ] {
            for delta_name in ["+0", "+1", "+64", "+65"] {
                for (order_name, index_of) in [
                    ("x-major", (|x: usize, z: usize| x * 16 + z) as fn(usize, usize) -> usize),
                    ("z-major", (|x: usize, z: usize| z * 16 + x) as fn(usize, usize) -> usize),
                ] {
                    let name = format!("{surface_name}{delta_name} {order_name}");
                    let slot = scores.iter_mut().find(|(n, _, _)| *n == name);
                    let slot = match slot {
                        Some(s) => s,
                        None => {
                            scores.push((Box::leak(name.clone().into_boxed_str()), 0, 0));
                            scores.last_mut().unwrap()
                        }
                    };
                    for x in 0..16usize {
                        for z in 0..16usize {
                            let delta: i32 = match delta_name {
                                "+1" => 1,
                                "+64" => 64,
                                "+65" => 65,
                                _ => 0,
                            };
                            let expect = surface[x * 16 + z] + delta;
                            let stored = data.heightmap[index_of(x, z)] as i32;
                            slot.2 += 1;
                            if stored == expect {
                                slot.1 += 1;
                            }
                            let winner = order_name == "z-major"
                                && surface_name == "non-air"
                                && delta_name == "+65";
                            if stored != expect && winner {
                                *mismatch_names
                                    .entry(top_block[x * 16 + z].clone().unwrap_or_default())
                                    .or_default() += 1;
                                if mismatch_shown < 8 {
                                    mismatch_shown += 1;
                                    println!(
                                        "MISMATCH chunk ({cx},{cz}) x={x} z={z}: stored={} expect={} (delta {}) top={:?}",
                                        stored, expect, stored - expect, top_block[x * 16 + z]
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    println!("\npalette entries checked: {palette_entries_checked}; version ints seen: {version_seen:?}");
    println!("\nwinner-mismatch top blocks: {mismatch_names:?}");
    println!("\nhypothesis agreement (agree/total):");
    scores.sort_by_key(|(_, a, t)| std::cmp::Reverse((*t, *a)));
    for (name, agree, total) in &scores {
        println!("  {name:22} {agree}/{total}");
    }
}
