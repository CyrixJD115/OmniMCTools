//! `omt render <world> --out map.png` — top-down map (Chunk Locator view).

use std::path::PathBuf;

use omt_render::{Palette, RenderOptions, render_chunk, render_world};
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
use std::collections::BTreeMap;

/// Inclusive chunk-space render bounds `(x0, z0, x1, z1)`.
pub type RenderBounds = (i32, i32, i32, i32);
/// Raw LevelDB records of one chunk column.
pub type ColumnRecs = Vec<(Vec<u8>, Vec<u8>)>;
/// Column coordinates → their raw records (one streaming batch).
pub type ColumnBatch = BTreeMap<(i32, i32), ColumnRecs>;
/// A resolved world root plus its archive workspace guard (if any).
pub type RootAndGuard = (std::path::PathBuf, Option<Workspace>);

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
    // Bedrock gets a snapshot-backed fast path (one sequential pass over an
    // in-memory view + parallel decode/render — the per-chunk snapshot API
    // re-collects the whole DB per chunk and dominates large worlds).
    if let Some((root, ws)) = bedrock_root(&args.world)? {
        return run_bedrock(root, ws, args);
    }
    let (store, ws) = open_store(&args.world)?;
    let mut opts = RenderOptions::for_dimension(args.dim);
    if args.max_y.is_some() {
        opts.max_y = args.max_y;
    }
    opts.biome_tint = !args.no_tint;
    let bounds = parse_bounds(&args.bounds)?;
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


/// Resolve a world path to a Bedrock world root (folder, or archive
/// extracted into a workspace whose guard the caller must keep alive).
fn parse_bounds(spec: &Option<String>) -> Result<Option<RenderBounds>, Box<dyn std::error::Error>> {
    match spec {
        Some(b) => {
            let v: Vec<i32> = b.split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>()?;
            if v.len() != 4 {
                return Err("--bounds expects x0,z0,x1,z1".into());
            }
            Ok(Some((v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3]))))
        }
        None => Ok(None),
    }
}

fn bedrock_root(
    path: &std::path::Path,
) -> Result<Option<RootAndGuard>, Box<dyn std::error::Error>> {
    let (root, ws) = if workspace::is_archive(path) {
        let ws = Workspace::open(path, &workspace::default_base())?;
        (ws.root().to_path_buf(), Some(ws))
    } else {
        (path.to_path_buf(), None)
    };
    match omt_world::detect(&root) {
        Ok(omt_world::Detected::BedrockFolder { .. }) => Ok(Some((root, ws))),
        Err(e) => Err(e.into()),
        Ok(_) => Ok(None),
    }
}

/// Fast Bedrock render: ONE snapshot of the database, one sequential pass
/// grouping chunk records per column (the snapshot's key order keeps columns
/// contiguous), then rayon-parallel decode + render in bounded batches.
fn run_bedrock(
    root: std::path::PathBuf,
    ws: Option<Workspace>,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error>> {
    use rayon::prelude::*;
    use std::collections::{BTreeMap, HashMap};

    let world = omt_bedrock::BedrockWorld::open_read_only(&root)?;
    let snap = world.snapshot()?;
    let mut opts = RenderOptions::for_dimension(args.dim);
    if args.max_y.is_some() {
        opts.max_y = args.max_y;
    }
    opts.biome_tint = !args.no_tint;
    let bounds = parse_bounds(&args.bounds)?;
    let in_bounds = |x: i32, z: i32| match bounds {
        Some((x0, z0, x1, z1)) => x >= x0 && x <= x1 && z >= z0 && z <= z1,
        None => true,
    };
    let started = std::time::Instant::now();
    let palette = Palette::embedded();
    const BATCH_COLUMNS: usize = 4096;

    let mut renders: HashMap<(i32, i32), omt_render::ChunkRender> = HashMap::new();
    let mut errors = 0usize;
    let mut rendered = 0usize;
    let mut batch: ColumnBatch = BTreeMap::new();
    let mut current: Option<(i32, i32)> = None;

    for (k, v) in snap.iter() {
        let hit = match omt_bedrock::keys::decode_key(&k) {
            omt_bedrock::DbKey::ChunkRecord { x, z, dim: d, .. }
            | omt_bedrock::DbKey::Subchunk { x, z, dim: d, .. } => {
                d == args.dim && in_bounds(x, z)
            }
            _ => false,
        };
        if !hit {
            continue;
        }
        let col = match omt_bedrock::keys::decode_key(&k) {
            omt_bedrock::DbKey::ChunkRecord { x, z, .. } | omt_bedrock::DbKey::Subchunk { x, z, .. } => (x, z),
            _ => unreachable!("matched above"),
        };
        if current != Some(col) {
            if batch.len() >= BATCH_COLUMNS {
                let decoded: Vec<_> = batch
                    .par_iter()
                    .map(|(&(cx, cz), recs)| {
                        (
                            (cx, cz),
                            omt_bedrock::BedrockWorld::decode_snapshot_column(args.dim, cx, cz, recs),
                        )
                    })
                    .collect();
                for ((cx, cz), chunk) in decoded {
                    match chunk {
                        Ok(Some(chunk)) => {
                            rendered += 1;
                            renders.insert((cx, cz), render_chunk(&chunk, palette, &opts));
                        }
                        Ok(None) => {}
                        Err(_) => errors += 1,
                    }
                }
                batch.clear();
            }
            current = Some(col);
        }
        batch.entry(col).or_default().push((k.to_vec(), v.to_vec()));
    }
    drop(snap);
    // Tail batch.
    let decoded: Vec<_> = batch
        .par_iter()
        .map(|(&(cx, cz), recs)| {
            ((cx, cz), omt_bedrock::BedrockWorld::decode_snapshot_column(args.dim, cx, cz, recs))
        })
        .collect();
    for ((cx, cz), chunk) in decoded {
        match chunk {
            Ok(Some(chunk)) => {
                rendered += 1;
                renders.insert((cx, cz), render_chunk(&chunk, palette, &opts));
            }
            Ok(None) => {}
            Err(_) => errors += 1,
        }
    }
    drop(batch);
    drop(world);

    omt_render::shade_all(&mut renders);
    let image = omt_render::MapImage::stitch(&renders);
    let Some(image) = image else {
        drop(ws);
        return Err("no chunks to render in that dimension/area".into());
    };
    image.write_png(&args.out)?;
    drop(ws);
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
