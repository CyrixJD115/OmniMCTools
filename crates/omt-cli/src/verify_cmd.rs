//! `omt verify <world>` — decode every chunk, block entity and entity of
//! a world (fresh, read-only) and report anything that fails. Run it after
//! editing a world (the World Editor works on copies) before playing it.

use std::path::PathBuf;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// World folder (Java or Bedrock).
    world: PathBuf,
    /// Show at most this many failures.
    #[arg(long, default_value_t = 30)]
    limit: usize,
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let t = std::time::Instant::now();
    let r = omt_edit::verify::verify(&args.world)?;
    println!("chunks          : {}", r.chunks);
    println!("block entities  : {}", r.block_entities);
    println!("entities        : {}", r.entities);
    println!("time            : {:.1}s", t.elapsed().as_secs_f32());
    if r.ok() {
        println!("result          : OK — everything decodes");
        return Ok(());
    }
    println!("result          : {} problem(s)", r.errors.len());
    for e in r.errors.iter().take(args.limit) {
        println!("  {e}");
    }
    Err(format!("{} problem(s) found", r.errors.len()).into())
}
