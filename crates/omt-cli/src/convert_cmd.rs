//! `omt convert item-stack` — hand-testable Java ⇄ Bedrock stack mapping.
//!
//! Reads one item stack as SNBT (argument or stdin) and writes the
//! converted stack as pretty SNBT on stdout. The loss ledger always goes
//! to stderr so piping stays clean.

use std::io::{IsTerminal, Read};

use omt_convert::{
    ConversionReport, ItemConvertOpts, StackEra, bedrock_stack_to_java, java_stack_to_bedrock,
};
use omt_nbt::Tag;
use omt_nbt::snbt::{from_snbt, to_snbt_pretty};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Convert one item stack given as SNBT (stdin when omitted).
    ItemStack {
        /// Convert a Java stack (either era) to Bedrock item NBT.
        #[arg(long, conflicts_with = "to_java")]
        to_bedrock: bool,

        /// Convert a Bedrock stack to Java item NBT.
        #[arg(long)]
        to_java: bool,

        /// With --to-java: emit the pre-1.20.5 (`tag`) layout instead of
        /// 1.20.5+ `components`.
        #[arg(long, requires = "to_java")]
        legacy_java: bool,

        /// The stack as SNBT; read from stdin when omitted.
        snbt: Option<String>,
    },

    /// Convert Java chunk columns into an (existing) Bedrock world's
    /// database. The Bedrock world must be a disposable copy: records of
    /// converted columns are replaced.
    Chunks {
        /// Java world folder (source).
        #[arg(long)]
        java: std::path::PathBuf,

        /// Bedrock world folder (target; opened writable).
        #[arg(long)]
        bedrock: std::path::PathBuf,

        /// Dimension to convert.
        #[arg(long, value_enum, default_value = "overworld")]
        dim: DimArg,

        /// Rule table RON (block state mappings, extra biome ids); seeded
        /// defaults when omitted.
        #[arg(long)]
        rules: Option<std::path::PathBuf>,

        /// Keep proto chunks (default: skip, UMT `skipProtoChunks` ON).
        #[arg(long)]
        keep_proto: bool,

        /// Convert and print the report without touching the target.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum DimArg {
    Overworld,
    Nether,
    End,
}

impl From<DimArg> for u8 {
    fn from(d: DimArg) -> u8 {
        match d {
            DimArg::Overworld => 0,
            DimArg::Nether => 1,
            DimArg::End => 2,
        }
    }
}

/// The loss kind tallies shown in the summary.
fn loss_tally(report: &omt_convert::ConversionReport) -> String {
    use omt_convert::LossKind;
    let kinds = [
        LossKind::FieldDropped,
        LossKind::ValueCoerced,
        LossKind::MixedEra,
        LossKind::UnmappedId,
        LossKind::UnsupportedComponent,
    ];
    let parts: Vec<String> = kinds
        .iter()
        .map(|k| (k.label(), report.count_with_kind(*k)))
        .filter(|(_, n)| *n > 0)
        .map(|(label, n)| format!("{label}={n}"))
        .collect();
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(", ")
    }
}

fn run_chunks(
    java: std::path::PathBuf,
    bedrock: std::path::PathBuf,
    dim: DimArg,
    rules: Option<std::path::PathBuf>,
    keep_proto: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use omt_convert::{ConvertOptions, apply_conversions, convert_java_chunks, load_rules};

    let java = omt_java::JavaWorld::open(&java)?;
    let rules = match &rules {
        Some(path) => load_rules(path).map_err(|e| format!("rule table: {e}"))?,
        None => omt_convert::RegistryRules::default(),
    };
    let opts = ConvertOptions {
        filter: omt_convert::ProtoFilter::new(!keep_proto),
        rules,
    };

    let mut outcome = convert_java_chunks(&java, dim.into(), &opts)?;

    println!(
        "chunks: {} converted, {} skipped (proto), {} failed",
        outcome.converted.len(),
        outcome.skipped_proto,
        outcome.failed.len(),
    );
    for (x, z, err) in outcome.failed.iter().take(10) {
        eprintln!("  failed ({x},{z}): {err}");
    }
    if dry_run {
        println!("dry run: target untouched");
    } else {
        use omt_world::WorldStore;
        let mut target = omt_bedrock::BedrockWorld::open_writable(&bedrock)?;
        let applied = apply_conversions(&target, dim.into(), &mut outcome)?;
        target.flush().map_err(|e| format!("flush: {e}"))?;
        println!("applied {applied} chunk columns to {}", bedrock.display());
    }
    println!("losses: {}", loss_tally(&outcome.merged_report()));
    Ok(())
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    match &args.command {
        Command::ItemStack { to_bedrock, to_java, legacy_java, snbt } => {
            run_item_stack(*to_bedrock, *to_java, *legacy_java, snbt)
        }
        Command::Chunks { java, bedrock, dim, rules, keep_proto, dry_run } => {
            run_chunks(
                java.clone(),
                bedrock.clone(),
                *dim,
                rules.clone(),
                *keep_proto,
                *dry_run,
            )
        }
    }
}

fn run_item_stack(
    to_bedrock: bool,
    to_java: bool,
    legacy_java: bool,
    snbt: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !to_bedrock && !to_java {
        return Err("choose a direction: --to-bedrock or --to-java".into());
    }

    let input = match snbt_or_stdin(snbt)? {
        Some(s) => s,
        None => return Err("no SNBT given (pass it as an argument or pipe it in)".into()),
    };
    let tag = from_snbt(&input).map_err(|e| format!("invalid SNBT: {e}"))?;

    let (out, report): (Tag, ConversionReport) = if to_bedrock {
        java_stack_to_bedrock(&tag, &ItemConvertOpts::default())?
    } else {
        let era = if legacy_java { StackEra::Legacy } else { StackEra::Components };
        bedrock_stack_to_java(&tag, era, &ItemConvertOpts::default())?
    };

    if !report.is_clean() {
        eprintln!("{report}");
    }
    println!("{}", to_snbt_pretty(&out));
    Ok(())
}

/// Positional argument, or all of stdin when absent.
fn snbt_or_stdin(snbt: &Option<String>) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if let Some(s) = snbt {
        return Ok(Some(s.clone()));
    }
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(Some(buf))
}
