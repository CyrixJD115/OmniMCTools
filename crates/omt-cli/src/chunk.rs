//! `omt chunk <world> <x> <z>` — dump one decoded chunk column.

use std::path::PathBuf;

use omt_world::detect;

#[derive(Debug, clap::Args)]
pub struct Args {
    world: PathBuf,
    x: i32,
    z: i32,

    #[arg(long, default_value_t = 0)]
    dim: u8,

    /// Show full palettes (default: first 8 entries).
    #[arg(long)]
    full: bool,

    /// Dump block entities / actors as SNBT.
    #[arg(long)]
    snbt: bool,
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let detected = detect(&args.world)?;
    let chunk = match &detected {
        omt_world::Detected::BedrockFolder { root, .. } => {
            let world = omt_bedrock::BedrockWorld::open_read_only(root)?;
            world.read_chunk_full(args.dim, args.x, args.z)?
        }
        omt_world::Detected::JavaFolder { root, .. } => {
            let world = omt_java::JavaWorld::open(root)?;
            use omt_world::WorldStore as _;
            world.read_chunk(args.dim, args.x, args.z)?
        }
        _ => return Err("chunk dump supports world folders".into()),
    };
    let Some(chunk) = chunk else {
        eprintln!("chunk ({},{}) dim {} not present", args.x, args.z, args.dim);
        std::process::exit(1);
    };

    println!("chunk ({},{}) dim {}", chunk.x, chunk.z, chunk.dim);
    println!("version   : {}", chunk.data_version);
    println!("status    : {}", chunk.status.as_deref().unwrap_or("?"));
    println!("slices    : {} (Y {}..{})", chunk.slices.len(),
        chunk.slices.keys().next().copied().unwrap_or(0),
        chunk.slices.keys().next_back().copied().unwrap_or(0));
    for (y, slice) in &chunk.slices {
        let shown = if args.full { slice.palette.len() } else { 8 };
        let names: Vec<String> = slice
            .palette
            .iter()
            .take(shown)
            .map(|b| {
                if b.properties.is_empty() {
                    b.name.clone()
                } else {
                    format!("{}[{}]", b.name,
                        b.properties.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","))
                }
            })
            .collect();
        let more = slice.palette.len().saturating_sub(shown);
        println!("  y={y:>3}: {} palette: {}{}",
            slice.palette.len(),
            names.join(", "),
            if more > 0 { format!(" (+{more} more)") } else { String::new() });
    }
    if let Some(slots) = &chunk.bedrock_biomes {
        let uniform: Vec<String> = slots
            .iter()
            .map(|s| match s {
                omt_world::BedrockBiomeSlot::NonExistent => "·".into(),
                omt_world::BedrockBiomeSlot::Storage { palette, .. } => {
                    format!("{}", palette.first().copied().unwrap_or(0))
                }
            })
            .collect();
        println!("biomes    : {} slots [{}]", slots.len(), uniform.join(" "));
    }
    println!("block ent : {}", chunk.block_entities.len());
    println!("actors    : {}", chunk.entities.len());
    if args.snbt {
        for be in &chunk.block_entities {
            println!("--- block entity ---");
            println!("{}", omt_nbt::snbt::to_snbt_pretty(be));
        }
        for e in &chunk.entities {
            println!("--- actor ---");
            println!("{}", omt_nbt::snbt::to_snbt_pretty(e));
        }
    }
    Ok(())
}
