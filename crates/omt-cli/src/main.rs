//! `omt` — the OmniMCTools command line.

mod chunk;
mod chunks;
mod convert_cmd;
mod inspect;
mod ldb;
mod nbt_cmd;
mod registry_cmd;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "omt",
    version,
    about = "OmniMCTools — Minecraft world tool suite (Java + Bedrock, latest stable)",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Detect a world and print a summary (edition, version, metadata).
    Inspect(inspect::Args),
    /// Parse an NBT file and dump it (tree, SNBT, JSON, or hex).
    Nbt(nbt_cmd::Args),
    /// List chunk columns in a world dimension.
    Chunks(chunks::Args),
    /// Dump one decoded chunk column.
    Chunk(chunk::Args),
    /// Raw LevelDB browser for Bedrock worlds.
    Ldb(ldb::Args),
    /// Conversion tools (item stacks; grows with M3/M4).
    Convert(convert_cmd::Args),
    /// Inspect the embedded semantic registry table (`stats`).
    Registry(registry_cmd::Args),
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Inspect(args) => inspect::run(&args),
        Command::Nbt(args) => nbt_cmd::run(&args),
        Command::Chunks(args) => chunks::run(&args),
        Command::Chunk(args) => chunk::run(&args),
        Command::Ldb(args) => ldb::run(&args),
        Command::Convert(args) => convert_cmd::run(&args),
        Command::Registry(args) => registry_cmd::run(args),
    };
    if let Err(err) = result {
        eprintln!("error: {err}");
        let mut source: Option<&dyn std::error::Error> = std::error::Error::source(err.as_ref());
        while let Some(s) = source {
            eprintln!("  caused by: {s}");
            source = s.source();
        }
        std::process::exit(1);
    }
}
