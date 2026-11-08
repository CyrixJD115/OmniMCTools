//! `omt chunks <world>` — enumerate chunk columns.

use std::path::PathBuf;

use omt_world::detect;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// World folder or archive.
    world: PathBuf,

    /// Dimension: 0 overworld (default), 1 nether, 2 end.
    #[arg(long, default_value_t = 0)]
    dim: u8,

    /// Machine-readable JSON output.
    #[arg(long)]
    json: bool,
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let detected = detect(&args.world)?;
    match &detected {
        omt_world::Detected::BedrockFolder { root, .. } => {
            let world = omt_bedrock::BedrockWorld::open_read_only(root)?;
            let chunks = world.list_chunks(args.dim)?;
            if args.json {
                let list: Vec<[i32; 2]> = chunks.iter().map(|&(x, z)| [x, z]).collect();
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "world": root.display().to_string(),
                    "dim": args.dim,
                    "count": chunks.len(),
                    "chunks": list,
                }))?);
            } else {
                println!("{} chunks in dimension {}", chunks.len(), args.dim);
                for &(x, z) in &chunks {
                    println!("{x} {z}");
                }
            }
            Ok(())
        }
        omt_world::Detected::JavaFolder { root, .. } => {
            let world = omt_java::JavaWorld::open(root)?;
            let chunks = world.list_chunks(args.dim)?;
            if args.json {
                let list: Vec<[i32; 2]> = chunks.iter().map(|&(x, z)| [x, z]).collect();
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "world": root.display().to_string(),
                    "dim": args.dim,
                    "count": chunks.len(),
                    "chunks": list,
                }))?);
            } else {
                println!("{} chunks in dimension {}", chunks.len(), args.dim);
                if chunks.is_empty() && world.dimension_dir(args.dim).is_none_or(|d| !d.is_dir()) {
                    eprintln!(
                        "note: dimension {} has no region files in this world (unexplored?)",
                        omt_world::Dimension::from_id(args.dim).map(|d| d.name()).unwrap_or("unknown")
                    );
                }
                for &(x, z) in &chunks {
                    println!("{x} {z}");
                }
            }
            Ok(())
        }
        omt_world::Detected::Archive(_) => Err("chunk listing for archives arrives with the archive store".into()),
    }
}
