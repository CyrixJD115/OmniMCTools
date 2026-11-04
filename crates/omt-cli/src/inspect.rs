//! `omt inspect <world>` — detect + summarize.

use std::path::PathBuf;

use omt_world::{detect, WorldSummary, WorldSource};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// World folder, .mcworld/.mctemplate/.zip archive, or a level.dat file.
    world: PathBuf,

    /// Machine-readable JSON output.
    #[arg(long)]
    json: bool,
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let detected = detect(&args.world)?;
    if args.json {
        let summary = WorldSummary::from_detected(&detected)?;
        println!("{}", serde_json::to_string_pretty(&summary_json(&summary))?);
        return Ok(());
    }

    match &detected {
        omt_world::Detected::BedrockFolder { root, level_dat } => {
            println!("edition          : Bedrock Edition (folder)");
            println!("path             : {}", root.display());
            println!("level name       : {}", level_dat.level_name().unwrap_or("(unnamed)"));
            println!("storage version  : {}", level_dat.storage_version);
            if let Some(v) = level_dat.last_opened_with_version() {
                println!("last opened with : {}", fmt_ver(&v));
            }
            if let Some(v) = level_dat.minimum_compatible_client_version() {
                println!("min client       : {}", fmt_ver(&v));
            }
            print_common(
                level_dat.game_type(),
                level_dat.time_played(),
                level_dat.spawn_xz(),
            );
            let db = root.join("db");
            if db.is_dir() {
                let n_ldb = std::fs::read_dir(&db)?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|x| x == "ldb"))
                    .count();
                println!("db .ldb files    : {n_ldb}");
            }
        }
        omt_world::Detected::JavaFolder { root, level_dat } => {
            println!("edition          : Java Edition (folder)");
            println!("path             : {}", root.display());
            let data = level_dat.tag.get("Data").unwrap_or(&level_dat.tag);
            println!("level name       : {}", data.get("LevelName").and_then(|t| t.as_str()).unwrap_or("(unnamed)"));
            println!("DataVersion      : {}", data.get("DataVersion").and_then(|t| t.as_i64()).unwrap_or(0));
            print_common(
                data.get("GameType").and_then(|t| t.as_i64()).map(|v| v as i32),
                data.get("Time").and_then(|t| t.as_i64()),
                None,
            );
            if root.join("region").is_dir() {
                println!("region/          : present");
            }
        }
        omt_world::Detected::Archive(p) => {
            println!("edition          : world archive (opened in full at M1)");
            println!("path             : {}", p.display());
        }
    }

    let summary = WorldSummary::from_detected(&detected)?;
    if summary.below_floor {
        eprintln!(
            "warning: world is below the read floor ({}); browsing still works, writing is refused.",
            omt_versions::registry().bedrock_release
        );
    }
    Ok(())
}

fn print_common(gamemode: Option<i32>, time: Option<i64>, spawn: Option<(i32, i32)>) {
    if let Some(gm) = gamemode {
        println!("game mode        : {}", gamemode_name(gm));
    }
    if let Some(t) = time {
        println!("time played      : {}s", t);
    }
    if let Some((x, z)) = spawn {
        println!("spawn            : {x}, {z}");
    }
}

fn gamemode_name(gm: i32) -> &'static str {
    match gm {
        0 => "survival",
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "unknown",
    }
}

fn fmt_ver(v: &[i32; 5]) -> String {
    format!("{}.{}.{}.{}.{}", v[0], v[1], v[2], v[3], v[4])
}

fn summary_json(s: &WorldSummary) -> serde_json::Value {
    use serde_json::json;
    json!({
        "edition": match s.edition { omt_world::Edition::Java => "java", omt_world::Edition::Bedrock => "bedrock" },
        "source": match s.source {
            WorldSource::JavaFolder => "java_folder",
            WorldSource::BedrockFolder => "bedrock_folder",
            WorldSource::Archive => "archive",
        },
        "root": s.root.display().to_string(),
        "name": s.name,
        "java_data_version": s.java_data_version,
        "bedrock_storage_version": s.bedrock_storage_version,
        "bedrock_last_opened_with": s.bedrock_last_opened.map(|v| v.to_vec()),
        "gamemode": s.gamemode,
        "time_played_seconds": s.time_played_seconds,
        "spawn_xz": s.spawn.map(|(x, z)| serde_json::json!([x, z])),
        "icon": s.icon.as_ref().map(|p| p.display().to_string()),
        "below_floor": s.below_floor,
    })
}
