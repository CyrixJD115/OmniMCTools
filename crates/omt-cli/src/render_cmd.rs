//! `omt render <world> --out map.png` — top-down map (Chunk Locator view).

use std::path::PathBuf;

use omt_render::{RenderOptions, render_world};
use omt_world::WorldStore;
use omt_world::workspace::{self, Workspace};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// World folder or archive (.zip/.mcworld/.mctemplate).
    world: PathBuf,
    /// Output PNG path.
    #[arg(long)]
    out: PathBuf,
    /// Dimension: 0 overworld, 1 nether, 2 end.
    #[arg(long, default_value_t = 0)]
    dim: u8,
    /// Slicer: hide every block above this Y.
    #[arg(long, allow_hyphen_values = true)]
    max_y: Option<i32>,
    /// Chunk-space bounds `x0,z0,x1,z1` (inclusive).
    #[arg(long, allow_hyphen_values = true)]
    bounds: Option<String>,
    /// Disable biome tinting.
    #[arg(long)]
    no_tint: bool,
}

/// A read-only store plus the workspace guard of an extracted archive.
pub type OpenedStore = (Box<dyn WorldStore + Sync>, Option<Workspace>);

/// Open any world read-only as a store. Archives are extracted into a
/// temporary workspace that the returned guard deletes on drop.
pub fn open_store(path: &std::path::Path) -> Result<OpenedStore, Box<dyn std::error::Error>> {
    let (root, ws) = if workspace::is_archive(path) {
        let ws = Workspace::open(path, &workspace::default_base())?;
        (ws.root().to_path_buf(), Some(ws))
    } else {
        (path.to_path_buf(), None)
    };
    let store: Box<dyn WorldStore + Sync> = match omt_world::detect(&root)? {
        omt_world::Detected::JavaFolder { .. } => Box::new(omt_java::JavaWorld::open(&root)?),
        omt_world::Detected::BedrockFolder { .. } => Box::new(omt_bedrock::BedrockWorld::open_read_only(&root)?),
        omt_world::Detected::Archive(p) => return Err(format!("nested archive {}", p.display()).into()),
    };
    Ok((store, ws))
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let (store, ws) = open_store(&args.world)?;
    let mut opts = RenderOptions::for_dimension(args.dim);
    if args.max_y.is_some() {
        opts.max_y = args.max_y;
    }
    opts.biome_tint = !args.no_tint;
    let bounds = match &args.bounds {
        Some(b) => {
            let v: Vec<i32> = b.split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>()?;
            if v.len() != 4 {
                return Err("--bounds expects x0,z0,x1,z1".into());
            }
            Some((v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])))
        }
        None => None,
    };
    let started = std::time::Instant::now();
    let (image, rendered, errors) = render_world(store.as_ref(), args.dim, bounds, &opts)?;
    drop(store);
    if let Some(ws) = ws {
        ws.discard()?;
    }
    let Some(image) = image else {
        return Err("no chunks to render in that dimension/area".into());
    };
    image.write_png(&args.out)?;
    println!(
        "rendered {rendered} chunk(s) ({errors} unreadable) → {}×{} px, origin chunk ({}, {}), {:.1}s → {}",
        image.width,
        image.height,
        image.min_cx,
        image.min_cz,
        started.elapsed().as_secs_f32(),
        args.out.display()
    );
    Ok(())
}
