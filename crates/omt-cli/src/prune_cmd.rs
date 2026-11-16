//! `omt prune <world>` — the World Pruner from the command line.
//!
//! Each tool flag adds one layer (block coordinates); prefix a spec with
//! `!` to invert that layer. A chunk is deleted when ANY layer selects it.
//! Without `--out`/`--in-place` the command only prints the plan.
//!
//! ```text
//! omt prune World --box '!-256,-256,255,255' --out World-pruned   # keep 32×32 chunks around 0,0
//! omt prune World --time '<1200' --in-place                        # drop chunks visited < 1 min
//! omt prune BedrockWorld --circle 0,0,500 --optimize --in-place
//! ```

use std::path::PathBuf;

use omt_prune::{Layer, PruneWorld, Selection, Shape, TimeOp};
use omt_world::workspace::{self, Workspace};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// World folder or archive (.zip/.mcworld/.mctemplate).
    world: PathBuf,
    /// Dimension: 0 overworld, 1 nether, 2 end.
    #[arg(long, default_value_t = 0)]
    dim: u8,
    /// Box layer: `x0,z0,x1,z1` (block corners, inclusive).
    #[arg(long = "box", value_name = "SPEC", allow_hyphen_values = true)]
    boxes: Vec<String>,
    /// Circle layer: `cx,cz,radius`.
    #[arg(long = "circle", value_name = "SPEC", allow_hyphen_values = true)]
    circles: Vec<String>,
    /// Polygon layer: `x,z;x,z;x,z[;…]` (closed automatically).
    #[arg(long = "polygon", value_name = "SPEC", allow_hyphen_values = true)]
    polygons: Vec<String>,
    /// Split layer: `x,z,angle_deg` — selects the left side of the line.
    #[arg(long = "split", value_name = "SPEC", allow_hyphen_values = true)]
    splits: Vec<String>,
    /// Time layer (Java): `>N`, `=N` or `<N` InhabitedTime ticks (20 = 1 s).
    #[arg(long = "time", value_name = "SPEC", allow_hyphen_values = true)]
    times: Vec<String>,
    /// Load layers from a RON file (`Selection` as saved by the GUI).
    #[arg(long)]
    layers: Option<PathBuf>,
    /// Run a LevelDB compaction afterwards (Bedrock "Optimize").
    #[arg(long)]
    optimize: bool,
    /// Write the pruned world here (folder, or archive by extension).
    #[arg(long, conflicts_with = "in_place")]
    out: Option<PathBuf>,
    /// Overwrite the original (a `.omt_backup` of it is kept).
    #[arg(long)]
    in_place: bool,
    /// Print every selected chunk coordinate.
    #[arg(long)]
    list: bool,
}

fn split_inv(spec: &str) -> (bool, &str) {
    match spec.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, spec),
    }
}

fn nums(spec: &str, n: usize, what: &str) -> Result<Vec<f64>, String> {
    let v: Result<Vec<f64>, _> = spec.split(',').map(|s| s.trim().parse::<f64>()).collect();
    match v {
        Ok(v) if v.len() == n => Ok(v),
        _ => Err(format!("{what} expects {n} comma-separated numbers, got {spec:?}")),
    }
}

fn build_selection(args: &Args) -> Result<Selection, Box<dyn std::error::Error>> {
    let mut layers = Vec::new();
    if let Some(path) = &args.layers {
        let sel: Selection = ron::from_str(&std::fs::read_to_string(path)?)?;
        layers.extend(sel.layers);
    }
    let mut push = |inv: bool, shape: Shape| {
        let mut l = Layer::new(shape);
        l.inverted = inv;
        layers.push(l);
    };
    for s in &args.boxes {
        let (inv, s) = split_inv(s);
        let v = nums(s, 4, "--box")?;
        push(inv, Shape::Box { x0: v[0], z0: v[1], x1: v[2], z1: v[3] });
    }
    for s in &args.circles {
        let (inv, s) = split_inv(s);
        let v = nums(s, 3, "--circle")?;
        push(inv, Shape::Circle { cx: v[0], cz: v[1], radius: v[2] });
    }
    for s in &args.polygons {
        let (inv, s) = split_inv(s);
        let points = s
            .split(';')
            .map(|p| nums(p, 2, "--polygon point").map(|v| (v[0], v[1])))
            .collect::<Result<Vec<_>, _>>()?;
        if points.len() < 3 {
            return Err("--polygon needs at least 3 points".into());
        }
        push(inv, Shape::Polygon { points });
    }
    for s in &args.splits {
        let (inv, s) = split_inv(s);
        let v = nums(s, 3, "--split")?;
        push(inv, Shape::Split { x: v[0], z: v[1], angle_deg: v[2] });
    }
    for s in &args.times {
        let (inv, s) = split_inv(s);
        let (op, rest) = s.split_at(s.find(|c: char| c.is_ascii_digit() || c == '-').unwrap_or(s.len()));
        let op = TimeOp::parse(op).ok_or_else(|| format!("--time expects >N, =N or <N, got {s:?}"))?;
        let ticks: i64 = rest.trim().parse().map_err(|_| format!("--time ticks not a number: {rest:?}"))?;
        push(inv, Shape::Time { op, ticks });
    }
    Ok(Selection::new(layers))
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let selection = build_selection(args)?;
    if selection.is_empty() && !args.optimize {
        return Err("no layers given (use --box/--circle/--polygon/--split/--time or --layers)".into());
    }
    let ws = Workspace::open(&args.world, &workspace::default_base())?;
    let result = prune_in(&ws, args, &selection);
    let outcome = match result {
        Ok(true) => match (&args.out, args.in_place) {
            (Some(out), _) => ws.save_as(out).map(|()| println!("saved pruned world to {}", out.display())),
            (None, true) => ws.save().map(|b| println!("saved in place; original kept at {}", b.display())),
            (None, false) => Ok(()),
        },
        Ok(false) => Ok(()),
        Err(e) => {
            let _ = ws.discard();
            return Err(e);
        }
    };
    ws.discard()?;
    Ok(outcome?)
}

/// Returns `true` when the world was modified and should be written out.
fn prune_in(ws: &Workspace, args: &Args, selection: &Selection) -> Result<bool, Box<dyn std::error::Error>> {
    let world = PruneWorld::open(ws.root())?;
    let dim_name = omt_world::Dimension::from_id(args.dim).map(|d| d.name()).unwrap_or("?");
    if selection.needs_inhabited_time() && world.edition() == omt_world::Edition::Bedrock {
        eprintln!("note: Bedrock worlds store no InhabitedTime — Time layers select nothing");
    }
    let plan = world.plan(args.dim, selection)?;
    println!(
        "{} · {dim_name}: {} chunk(s) present, {} selected for deletion, {} kept",
        world.edition().name(),
        plan.total,
        plan.selected.len(),
        plan.kept()
    );
    if args.list {
        for (x, z) in &plan.selected {
            println!("  {x} {z}");
        }
    }
    let write = args.out.is_some() || args.in_place;
    if !write {
        println!("dry run — pass --out <path> or --in-place to prune");
        return Ok(false);
    }
    let report = world.execute(&plan)?;
    println!(
        "deleted {} chunk(s): {} file(s) rewritten, {} deleted, {} record(s), {} actor(s)",
        report.chunks_deleted,
        report.files_rewritten,
        report.files_deleted,
        report.records_deleted,
        report.actors_deleted
    );
    if args.optimize {
        world.optimize()?;
        println!("optimized (database compacted)");
    }
    Ok(true)
}
