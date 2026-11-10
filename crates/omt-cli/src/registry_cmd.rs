//! `omt registry` — inspect the embedded semantic registry table.

use clap::Parser;

/// The generator's own accounting for this exact table build (committed
/// beside the table; see `tools/registry-gen`).
const GEN_STATS: &str = include_str!("../../omt-convert/data/registry-stats.json");

/// Prints the embedded table's shape and the generator's coverage
/// accounting. (The stats themselves come from the committed generator
/// report — the binary cannot re-run the generator.)
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Table shape, coverage, and reverse-collision counts.
    Stats {
        /// Print the java-only blocks that stay on the loss-ledger passthrough.
        #[arg(long)]
        gaps: bool,
    },
}

pub fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::Stats { gaps } => run_stats(gaps),
    }
}

fn run_stats(gaps: bool) -> Result<(), Box<dyn std::error::Error>> {
    let rules = omt_convert::RegistryRules::embedded();
    let stats: serde_json::Value = serde_json::from_str(GEN_STATS)?;

    println!("table version: {}", rules.version);
    println!(
        "state rules: {} (java state → bedrock name + states)",
        rules.state_rules.len()
    );
    println!("static renames: {}", rules.block_rules.len());
    println!("biome ids: {}", rules.biome_ids.len());
    println!("enchantment ids: {}", rules.enchant_ids.len());
    println!("palette version: {}", rules.block_palette_version);

    let java_states = stats["java_states"].as_u64().unwrap_or(0);
    let mapped = stats["states_mapped"].as_u64().unwrap_or(0);
    let java_blocks = stats["java_blocks"].as_u64().unwrap_or(0);
    let blocks_mapped = stats["blocks_mapped"].as_u64().unwrap_or(0);
    let collisions = stats["reverse_collisions"].as_array().map(Vec::len).unwrap_or(0);
    let gap_blocks = stats["gaps"].as_array().map(Vec::len).unwrap_or(0);
    println!();
    println!(
        "generation coverage: {mapped}/{java_states} java states ({:.1}%), \
         {blocks_mapped}/{java_blocks} blocks",
        if java_states == 0 {
            100.0
        } else {
            100.0 * mapped as f64 / java_states as f64
        }
    );
    println!("reverse collisions (alphabetically-first java state wins): {collisions}");
    println!("gap blocks (loss-ledger passthrough): {gap_blocks}");
    if gaps {
        for g in stats["gaps"].as_array().unwrap_or(&Vec::new()) {
            println!(
                "  {} — {}",
                g["block"].as_str().unwrap_or("?"),
                g["reason"].as_str().unwrap_or("?")
            );
        }
    }
    Ok(())
}
