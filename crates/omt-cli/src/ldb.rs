//! `omt ldb <world> list|get` — raw LevelDB browser (M7 grows this into the
//! full GUI browser; the CLI backend lands now because it costs nothing).

use std::path::PathBuf;

use clap::Subcommand;
use omt_bedrock::keys::DbKey;
use omt_world::detect;

#[derive(Debug, clap::Args)]
pub struct Args {
    world: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List keys (optionally only those with a text/hex prefix).
    List {
        /// Hex or ASCII prefix to filter by (auto-detected).
        #[arg(long)]
        prefix: Option<String>,
    },
    /// Dump a key's value as hex (or NBT when it parses).
    Get {
        /// Key as hex bytes.
        key: String,
        /// Try NBT interpretation first.
        #[arg(long)]
        nbt: bool,
    },
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let detected = detect(&args.world)?;
    let omt_world::Detected::BedrockFolder { root, .. } = &detected else {
        return Err("ldb browsing supports Bedrock folders".into());
    };
    let world = omt_bedrock::BedrockWorld::open_read_only(root)?;
    match &args.command {
        Command::List { prefix } => {
            let filter: Option<Vec<u8>> = prefix.as_deref().and_then(parse_prefix);
            let mut n = 0usize;
            for (k, v) in world.iter_raw()? {
                if let Some(f) = &filter {
                    if !k.starts_with(f) {
                        continue;
                    }
                }
                n += 1;
                let desc = describe(&k);
                println!("{} {:>6}B  {}", hex(&k), v.len(), desc);
            }
            eprintln!("{n} keys");
            Ok(())
        }
        Command::Get { key, nbt } => {
            let bytes = hex_decode(key).ok_or("key must be hex")?;
            let Some(v) = world.raw_get(&bytes)? else {
                eprintln!("key not found");
                std::process::exit(1);
            };
            if *nbt {
                match omt_bedrock::store::parse_actor_nbt(&v) {
                    Ok(nbt) => {
                        println!("{}", omt_nbt::tree::dump(&nbt));
                        return Ok(());
                    }
                    Err(e) => eprintln!("(not NBT: {e}; falling back to hex)"),
                }
            }
            print!("{}", omt_nbt::tree::hexdump(&v));
            Ok(())
        }
    }
}

fn describe(k: &[u8]) -> String {
    match keys_decode(k) {
        DbKey::Subchunk { x, z, dim, y } => format!("subchunk ({x},{z}) dim {dim} y={y}"),
        DbKey::ChunkRecord { x, z, dim, tag } => {
            format!("chunk-record ({x},{z}) dim {dim} tag={tag:#04x} {}", tag_name(tag))
        }
        DbKey::Digp { x, z, dim } => format!("digp ({x},{z}) dim {dim}"),
        DbKey::Actor { .. } => "actor".into(),
        DbKey::LocalPlayer => "~local_player".into(),
        DbKey::Player { .. } => "player".into(),
        DbKey::Other => {
            if k.iter().all(|b| b.is_ascii_graphic() || *b == b'~' || *b == b'_') {
                String::from_utf8_lossy(k).into_owned()
            } else {
                String::new()
            }
        }
    }
}

// Thin indirection so the CLI only depends on the crate's public API.
fn keys_decode(k: &[u8]) -> DbKey<'_> {
    omt_bedrock::keys::decode_key(k)
}

fn tag_name(tag: u8) -> &'static str {
    match tag {
        0x2b => "data3D",
        0x2c => "version",
        0x2d => "data2D-legacy",
        0x31 => "block-entities",
        0x33 => "block-ticks",
        0x35 => "edges?",
        0x36 => "finalized",
        0x3a => "fluid-ticks",
        0x77 => "structure-refs",
        _ => "?",
    }
}

fn hex(k: &[u8]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn parse_prefix(s: &str) -> Option<Vec<u8>> {
    if s.chars().all(|c| c.is_ascii_hexdigit()) && s.len() % 2 == 0 {
        hex_decode(s)
    } else {
        Some(s.as_bytes().to_vec())
    }
}
