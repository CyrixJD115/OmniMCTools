//! CLI: build the OMT semantic registry table from pinned sources.
//!
//! All paths default to the repo layout so `cargo run -p registry-gen`
//! works from the workspace root; every default is overridable. Defaults
//! that point OUTSIDE the repo (`../../../RE/…` jar, chunker data) are
//! never committed — clean-room rule (AGENTS rule 4).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Parser;

use registry_gen::bedrock_side::read_chunker_bedrock;
use registry_gen::biomes_enchants::{read_biomes, read_enchantments};
use registry_gen::emit::{write_stats, write_table};
use registry_gen::handmaps::HandMaps;
use registry_gen::java_side::{read_chunker_java, read_jar_blockstates, verify_jar_vs_chunker, Verification};
use registry_gen::semantic::assemble;

/// Defaults are relative to this crate's directory (`tools/registry-gen`).
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[derive(Debug, Parser)]
#[command(
    name = "registry-gen",
    about = "Generate the OMT semantic Java⇄Bedrock block-state + enchantment table",
    disable_help_subcommand = true
)]
struct Args {
    /// Pinned Mojang client.jar (data extraction only).
    #[arg(long, default_value = "../../../RE/client-26.3/client.jar")]
    java_jar: PathBuf,
    /// Chunker java registry data (blocks.json).
    #[arg(long, default_value = "../../../RE/chunker-ref/cli/data/java/26.3/blocks.json")]
    chunker_java: PathBuf,
    /// Chunker bedrock registry data (block_states.json).
    #[arg(long, default_value = "../../../RE/chunker-ref/cli/data/bedrock/1.26.50.0/block_states.json")]
    chunker_bedrock: PathBuf,
    /// Chunker bedrock biome id table.
    #[arg(long, default_value = "../../../RE/chunker-ref/cli/src/test/resources/bedrock/resolver/biome_ids.json")]
    chunker_biomes: PathBuf,
    /// Hand-authored semantic maps (never overwritten by regeneration).
    #[arg(long, default_value = "hand-maps.ron")]
    hand_maps: PathBuf,
    /// Output RON rule table (embedded by omt-convert).
    #[arg(long, default_value = "../../crates/omt-convert/data/registry.ron")]
    out: PathBuf,
    /// Output assembly-stats JSON (committed next to the table).
    #[arg(long, default_value = "../../crates/omt-convert/data/registry-stats.json")]
    stats_out: PathBuf,
}

fn main() {
    let args = Args::parse();
    if let Err(report) = run(&args) {
        eprintln!("error: {report}");
        std::process::exit(1);
    }
}

fn run(args: &Args) -> Result<(), String> {
    let resolve = |p: &PathBuf| -> PathBuf {
        if p.is_absolute() {
            p.clone()
        } else {
            crate_dir().join(p)
        }
    };
    let jar = resolve(&args.java_jar);
    let chunker_java_path = resolve(&args.chunker_java);
    let chunker_bedrock_path = resolve(&args.chunker_bedrock);
    let biomes_path = resolve(&args.chunker_biomes);
    let hand_path = resolve(&args.hand_maps);
    let out_path = resolve(&args.out);
    let stats_path = resolve(&args.stats_out);

    println!("reading jar blockstates: {}", jar.display());
    let jar = read_jar_blockstates(&jar)?;
    println!("  {} blockstate files", jar.len());

    println!("reading Chunker java registry: {}", chunker_java_path.display());
    let java = read_chunker_java(&chunker_java_path)?;
    println!("  {} blocks", java.len());

    println!("reading Chunker bedrock registry: {}", chunker_bedrock_path.display());
    let bedrock = read_chunker_bedrock(&chunker_bedrock_path)?;
    println!("  {} blocks", bedrock.len());

    // Cross-verify: every disagreement is a bug or a documented difference.
    let mut hard_errors = Vec::new();
    let mut soft_notes = Vec::new();
    for finding in verify_jar_vs_chunker(&jar, &java) {
        match finding {
            Verification::PropMissingInChunker { block, prop } => hard_errors
                .push(format!("{block}: jar property {prop:?} missing from the registry data")),
            Verification::ValueMissingInChunker { block, prop, value } => hard_errors
                .push(format!("{block}: jar variant value {prop}={value} missing from the registry data")),
            Verification::JarOnly(name) => soft_notes.push(format!(
                "{name}: jar blockstate without a registry entry (known item-only name)"
            )),
            Verification::ChunkerOnly(name) => soft_notes
                .push(format!("{name}: registry entry without a jar blockstate")),
        }
    }
    if !hard_errors.is_empty() {
        return Err(format!(
            "jar ↔ registry version mismatch ({} findings); re-pin both sources:\n  {}",
            hard_errors.len(),
            hard_errors.iter().take(10).cloned().collect::<Vec<_>>().join("\n  "),
        ));
    }
    println!(
        "cross-verify: {} soft notes (item-only names etc.), 0 hard mismatches",
        soft_notes.len()
    );

    println!("loading hand-maps: {}", hand_path.display());
    let hand = HandMaps::load(&hand_path)?;
    println!(
        "  {} patterns, {} block entries, {} biome aliases",
        hand.patterns.len(),
        hand.blocks.len(),
        hand.biome_aliases.len()
    );

    let (biome_ids, biome_warnings) = read_biomes(&biomes_path, &hand.biome_aliases)?;
    println!("biomes: {} entries", biome_ids.len());

    let jar_ench_names = jar_tooltip_order(&resolve(&args.java_jar))?;
    let (enchant_ids, ench_warnings) = read_enchantments(jar_ench_names.as_deref())?;
    println!("enchantments: {} entries", enchant_ids.len());

    let seed = omt_convert::RegistryRules::seed();
    let (rules, stats) = assemble(&java, &bedrock, &hand, biome_ids, enchant_ids, &seed);
    write_table(&out_path, &rules)?;
    write_stats(&stats_path, &stats)?;

    // Report — the honest ledger of what is and is not in the table.
    println!();
    println!("== assembly ==");
    println!(
        "java blocks {}/{} states {} — mapped: {} blocks, {} states (+{} static rules)",
        stats.java_blocks,
        java.len(),
        stats.java_states,
        stats.blocks_mapped,
        stats.states_mapped,
        stats.static_rules,
    );
    println!("table: {} state rules, {} block rules", rules.state_rules.len(), rules.block_rules.len());
    println!("coverage: {:.1}% of java states", coverage_pct(&stats));
    for warning in biome_warnings.iter().chain(ench_warnings.iter()) {
        println!("note: {warning}");
    }
    for warning in &stats.warnings {
        println!("warning: {warning}");
    }
    if !stats.reverse_collisions.is_empty() {
        println!(
            "reverse collisions (alphabetically-first java state wins): {}",
            stats.reverse_collisions.len()
        );
        for (kept, dropped) in stats.reverse_collisions.iter().take(10) {
            println!("  kept {kept} · dropped {dropped}");
        }
    }
    println!("gaps ({} blocks stay on the loss-ledger passthrough):", stats.gaps.len());
    let mut gap_blocks: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (block, reason) in &stats.gaps {
        gap_blocks.entry(block.as_str()).or_default().push(reason);
    }
    for (block, reasons) in gap_blocks.iter().take(40) {
        println!("  {block}: {}", reasons.join("; "));
    }
    if gap_blocks.len() > 40 {
        println!("  … and {} more (see {})", gap_blocks.len() - 40, stats_path.display());
    }
    println!();
    println!("wrote {}", out_path.display());
    println!("wrote {}", stats_path.display());
    Ok(())
}

/// Extract `data/minecraft/tags/enchantment/tooltip_order.json` from the
/// jar for the enchantment cross-check.
fn jar_tooltip_order(jar_path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = std::fs::File::open(jar_path).map_err(|e| format!("opening {jar_path:?}: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("reading jar: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("jar entry {i}: {e}"))?;
        if entry.name() == "data/minecraft/tags/enchantment/tooltip_order.json" {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)
                .map_err(|e| format!("reading tooltip_order: {e}"))?;
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

fn coverage_pct(stats: &registry_gen::semantic::AssemblyStats) -> f64 {
    if stats.java_states == 0 {
        return 100.0;
    }
    100.0 * stats.states_mapped as f64 / stats.java_states as f64
}
