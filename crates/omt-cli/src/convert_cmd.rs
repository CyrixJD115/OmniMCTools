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
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let Command::ItemStack {
        to_bedrock,
        to_java,
        legacy_java,
        snbt,
    } = &args.command;

    if !*to_bedrock && !*to_java {
        return Err("choose a direction: --to-bedrock or --to-java".into());
    }

    let input = match snbt_or_stdin(snbt)? {
        Some(s) => s,
        None => return Err("no SNBT given (pass it as an argument or pipe it in)".into()),
    };
    let tag = from_snbt(&input).map_err(|e| format!("invalid SNBT: {e}"))?;

    let (out, report): (Tag, ConversionReport) = if *to_bedrock {
        java_stack_to_bedrock(&tag, &ItemConvertOpts::default())?
    } else {
        let era = if *legacy_java {
            StackEra::Legacy
        } else {
            StackEra::Components
        };
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
