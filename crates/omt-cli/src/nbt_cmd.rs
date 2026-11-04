//! `omt nbt <file>` — parse and dump an NBT file.

use std::path::PathBuf;

use omt_nbt::{BedrockLevelDat, ByteOrder, Container, Nbt};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// NBT file: Java .dat (gzip/raw, big-endian), Bedrock level.dat, or a
    /// bare payload.
    file: PathBuf,

    /// Force little-endian (Bedrock disk) interpretation.
    #[arg(long, conflicts_with = "be")]
    le: bool,

    /// Force big-endian (Java) interpretation (default).
    #[arg(long)]
    be: bool,

    /// Treat the file as a Bedrock level.dat (8-byte header, LE).
    #[arg(long)]
    bedrock_header: bool,

    /// Output SNBT instead of the tree view.
    #[arg(long)]
    snbt: bool,

    /// Output a hex dump of the raw file instead of parsing.
    #[arg(long)]
    hex: bool,

    /// Path of the tag to edit (e.g. `Data.GameRules.keepInventory`,
    /// `Inventory[0].Count`). Implies an in-place edit unless --out is given.
    #[arg(long, value_name = "PATH")]
    set: Option<String>,

    /// SNBT value to write (e.g. `1b`, `\"new name\"`, `[I; 1, 2]`).
    #[arg(long, value_name = "SNBT", requires = "set")]
    value: Option<String>,

    /// Save to this file instead of editing in place.
    #[arg(long, requires = "set")]
    out: Option<PathBuf>,

    /// Output JSON instead of the tree view (numbers lose NBT width).
    #[arg(long)]
    json: bool,

    /// Pretty (multi-line) SNBT with `--snbt`.
    #[arg(long, requires = "snbt")]
    pretty: bool,
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(&args.file)
        .map_err(|e| format!("cannot read {}: {e}", args.file.display()))?;

    if let (Some(path), Some(value)) = (&args.set, &args.value) {
        return run_set(args, path, value);
    }

    if args.hex {
        print!("{}", omt_nbt::tree::hexdump(&bytes));
        return Ok(());
    }

    // The document loader handles the Bedrock 8-byte header, gzip and raw
    // formats uniformly; --bedrock-header/--le remain as explicit overrides.
    let nbt: Nbt = if args.bedrock_header {
        BedrockLevelDat::parse(&bytes)?.nbt
    } else if args.le {
        let (payload, _) = Container::decode(&bytes)?;
        omt_nbt::read(&payload, ByteOrder::Little)?
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        // gzip big-endian.
        let (payload, _) = Container::decode(&bytes)?;
        omt_nbt::read(&payload, ByteOrder::Big)?
    } else {
        // Bedrock level.dat (header heuristic), then raw BE, then raw LE.
        match omt_nbt::NbtDocument::from_bytes(&args.file, &bytes) {
            Ok(doc) => doc.nbt,
            Err(_) => {
                let (payload, _) = Container::decode(&bytes)?;
                omt_nbt::read(&payload, ByteOrder::Big)?
            }
        }
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&nbt.tag)?);
    } else if args.snbt {
        if args.pretty {
            println!("{}", omt_nbt::snbt::to_snbt_pretty(&nbt.tag));
        } else {
            println!("{}", omt_nbt::snbt::to_snbt(&nbt.tag));
        }
    } else {
        print!("{}", omt_nbt::tree::dump(&nbt));
    }
    Ok(())
}

fn run_set(args: &Args, path: &str, value: &str) -> Result<(), Box<dyn std::error::Error>> {
    use omt_nbt::{EditOp, TagPath, UndoStack};
    use std::str::FromStr;

    let mut doc = omt_nbt::NbtDocument::load(&args.file).map_err(|e| format!("{e}"))?;
    let tag_path = TagPath::from_str(path).map_err(|e| format!("{e}"))?;
    let new_value = omt_nbt::snbt::from_snbt(value)
        .map_err(|e| format!("invalid SNBT value: {e}"))?;

    let mut undo = UndoStack::default();
    undo.edit(doc.root_mut(), |root| {
        EditOp::prepare_set(root, &tag_path, new_value.clone())
    })
    .map_err(|e| format!("edit failed: {e}"))?;

    let target = if let Some(out) = &args.out {
        out.clone()
    } else {
        args.file.clone()
    };
    // Re-point the document's path so save_to/backup target the right file.
    doc.path = target.clone();
    let backup = doc.save_to(&target).map_err(|e| format!("{e}"))?;
    println!(
        "set {} = {} → {}{}",
        path,
        value,
        target.display(),
        match backup {
            Some(b) => format!(" (backup: {})", b.display()),
            None => String::new(),
        }
    );
    Ok(())
}
