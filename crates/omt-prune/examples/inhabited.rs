//! Probe: InhabitedTime histogram of a (copied!) Java world.
//! `cargo run -p omt-prune --example inhabited --release -- <world> [dim]`
fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("world path"));
    let dim: u8 = args.next().map(|d| d.parse().expect("dim")).unwrap_or(0);
    let world = omt_prune::PruneWorld::open(&root).expect("open");
    let chunks = world.scan(dim, true).expect("scan");
    let mut v: Vec<i64> = chunks.iter().filter_map(|c| c.inhabited).collect();
    v.sort_unstable();
    let n = v.len();
    println!("{n} chunks; zero: {}", v.iter().filter(|&&t| t == 0).count());
    for q in [0.0, 0.25, 0.5, 0.75, 0.9, 0.99, 1.0] {
        if n > 0 {
            let i = ((n - 1) as f64 * q) as usize;
            println!("p{:>3}: {} ticks", (q * 100.0) as u32, v[i]);
        }
    }
}
