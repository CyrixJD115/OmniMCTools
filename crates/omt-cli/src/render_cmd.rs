//! `omt render <world> --out map.png` — top-down map (Chunk Locator view).
//!
//! Memory-safe for any world size: the world is never loaded whole (Bedrock
//! uses a key-only column index + batched exact reads; Java reads one region
//! at a time), and the PNG is **streamed** band by band — peak memory is a
//! few hundred MB whether the world has 5 thousand or 5 million chunks.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use omt_render::{AreaSpec, Band, MapSource, NameTable, Palette, RenderOptions, render_area};
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
    /// Chunk-space bounds `x0,z0,x1,z1` (inclusive), or `core` = the area
    /// holding 99 % of the chunks (drops far-away outlier chunks).
    #[arg(long, allow_hyphen_values = true)]
    bounds: Option<String>,
    /// Disable biome tinting.
    #[arg(long)]
    no_tint: bool,
    /// Level of detail: blocks per pixel (1, 2, 4 … 256; box-averaged, every
    /// chunk is still read so the result is exact, just smaller).
    #[arg(long, default_value_t = 1)]
    scale: u32,
    /// Fast preview: read one chunk per N×N chunk cell (N = power of two) and
    /// draw it as one pixel — N² fewer reads. Overrides --scale.
    #[arg(long)]
    sample: Option<u32>,
    /// Refuse outputs larger than this many megapixels (use --scale/--sample).
    #[arg(long, default_value_t = 1024)]
    max_megapixels: u64,
    /// Chunks read per band (memory ceiling; lowered automatically when the
    /// system is short on memory).
    #[arg(long, default_value_t = 4096)]
    band_chunks: usize,
}

/// Inclusive chunk-space render bounds `(x0, z0, x1, z1)`.
pub type RenderBounds = (i32, i32, i32, i32);

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

fn log2_pow2(v: u32, what: &str) -> Result<u8, Box<dyn std::error::Error>> {
    if v == 0 || !v.is_power_of_two() || v > 256 {
        return Err(format!("{what} must be a power of two in 1..=256 (got {v})").into());
    }
    Ok(v.trailing_zeros() as u8)
}

/// A world kept open for a [`MapSource`] (which only holds a weak ref).
/// Held only to keep the handle alive while rendering.
#[allow(dead_code)]
pub enum OpenWorld {
    Bedrock(Arc<omt_bedrock::BedrockWorld>),
    Java(Arc<omt_java::JavaWorld>),
}

/// Open a world dimension as a render source (archives are extracted into a
/// workspace that the returned guard deletes on drop). Keep the returned
/// [`OpenWorld`] alive while rendering.
pub fn open_source(path: &std::path::Path, dim: u8) -> Result<(MapSource, OpenWorld, Option<Workspace>), Box<dyn std::error::Error>> {
    let (root, ws) = if workspace::is_archive(path) {
        let ws = Workspace::open(path, &workspace::default_base())?;
        (ws.root().to_path_buf(), Some(ws))
    } else {
        (path.to_path_buf(), None)
    };
    let (src, world) = match omt_world::detect(&root)? {
        omt_world::Detected::BedrockFolder { .. } => {
            let world = Arc::new(omt_bedrock::BedrockWorld::open_read_only(&root)?);
            let index = world.column_index()?;
            (MapSource::bedrock(&world, &index, dim), OpenWorld::Bedrock(world))
        }
        omt_world::Detected::JavaFolder { .. } => {
            let world = Arc::new(omt_java::JavaWorld::open(&root)?);
            (MapSource::java(&world, dim)?, OpenWorld::Java(world))
        }
        omt_world::Detected::Archive(p) => return Err(format!("nested archive {}", p.display()).into()),
    };
    Ok((src, world, ws))
}

/// Shrink the band size when the system is low on memory (each band chunk
/// costs roughly 100 KB transiently while decoding).
fn band_budget(requested: usize) -> usize {
    let requested = requested.max(64);
    match omt_world::sysmem::mem_info() {
        Some(m) => {
            let affordable = (m.available / 4 / (100 * 1024)) as usize;
            requested.min(affordable.max(256))
        }
        None => requested,
    }
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let started = std::time::Instant::now();
    let (src, world, ws) = open_source(&args.world, args.dim)?;
    let index_time = started.elapsed();
    let mut opts = RenderOptions::for_dimension(args.dim);
    if args.max_y.is_some() {
        opts.max_y = args.max_y;
    }
    opts.biome_tint = !args.no_tint;

    let Some((wx0, wz0, wx1, wz1)) = src.chunks().bounds() else {
        return Err("no chunks to render in that dimension".into());
    };
    let requested = if args.bounds.as_deref().map(str::trim) == Some("core") {
        src.chunks().core_bounds(0.005)
    } else {
        parse_bounds(&args.bounds)?
    };
    let (x0, z0, x1, z1) = match requested {
        Some((a, b, c, d)) => (a.max(wx0), b.max(wz0), c.min(wx1), d.min(wz1)),
        None => (wx0, wz0, wx1, wz1),
    };
    if x0 > x1 || z0 > z1 || !src.chunks().any_in_rect(x0, z0, x1, z1) {
        return Err("no chunks to render in that dimension/area".into());
    }
    let (level, stride) = match args.sample {
        Some(n) => (0, 1i32 << log2_pow2(n, "--sample")?),
        None => (log2_pow2(args.scale, "--scale")?, 1),
    };
    let spec = AreaSpec { x0, z0, w: x1 - x0 + 1, h: z1 - z0 + 1, level, stride, north_row: true };
    let (pw, ph) = spec.px_size();
    let mpx = (pw as u64 * ph as u64).div_ceil(1_000_000);
    if mpx > args.max_megapixels {
        // Suggest the smallest scale that fits.
        let mut s = args.scale.max(1);
        while s < 256 && (pw as u64 / (s / args.scale.max(1)) as u64) * (ph as u64 / (s / args.scale.max(1)) as u64) > args.max_megapixels * 1_000_000 {
            s *= 2;
        }
        let core = match src.chunks().core_bounds(0.005) {
            Some(c) if args.bounds.is_none() && c != (wx0, wz0, wx1, wz1) => format!(
                ", or --bounds core (99 % of chunks lie in {}×{} chunks — the full box is stretched by outliers)",
                c.2 - c.0 + 1,
                c.3 - c.1 + 1
            ),
            _ => String::new(),
        };
        return Err(format!(
            "output would be {pw}×{ph} px ({mpx} MP > --max-megapixels {}); try --scale {s}, --sample N{core}",
            args.max_megapixels
        )
        .into());
    }

    let file = std::fs::File::create(&args.out)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), pw as u32, ph as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut writer = enc.write_header()?;
    let mut stream = writer.stream_writer()?;

    let total_rows = ph;
    let mut written = 0usize;
    let mut io_err: Option<std::io::Error> = None;
    let mut last_report = std::time::Instant::now();
    let band = band_budget(args.band_chunks);
    let mut names = NameTable::default();
    let interrupted = AtomicBool::new(false);
    let stats = render_area(&src, spec, Palette::embedded(), &opts, band, &mut names, &|| interrupted.load(Ordering::Relaxed), &mut |b: Band<'_>| {
        if io_err.is_some() {
            return;
        }
        if let Err(e) = stream.write_all(&b.rgba[..b.rows * b.width * 4]) {
            io_err = Some(e);
            interrupted.store(true, Ordering::Relaxed);
            return;
        }
        written += b.rows;
        if last_report.elapsed().as_secs_f32() > 2.0 {
            last_report = std::time::Instant::now();
            eprintln!("  {:>5.1}% ({written}/{total_rows} rows, {:.0}s)", written as f64 * 100.0 / total_rows.max(1) as f64, started.elapsed().as_secs_f32());
        }
    });
    if let Some(e) = io_err {
        return Err(e.into());
    }
    let stats = stats.ok_or("render cancelled")?;
    stream.finish()?;
    drop(writer);
    drop(src);
    drop(world);
    if let Some(ws) = ws {
        ws.discard()?;
    }
    let detail = if stride > 1 { format!("sampled 1 chunk per {stride}×{stride}") } else { format!("{} block(s)/px", 1u32 << level) };
    println!(
        "rendered {} chunk(s) ({} unreadable) → {pw}×{ph} px ({detail}), origin chunk ({x0}, {z0}), index {:.1}s, total {:.1}s → {}",
        stats.chunks,
        stats.errors,
        index_time.as_secs_f32(),
        started.elapsed().as_secs_f32(),
        args.out.display()
    );
    Ok(())
}
