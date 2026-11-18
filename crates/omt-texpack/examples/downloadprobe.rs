//! `cargo run -p omt-texpack --example downloadprobe CACHE_DIR` — runs the
//! in-app download (`fetch_samples`) end to end into a cache folder.

fn main() {
    let cache = std::path::PathBuf::from(std::env::args().nth(1).expect("CACHE_DIR"));
    let dir = omt_texpack::locate::download_dir(&cache);
    let mut last = 0u64;
    let t = std::time::Instant::now();
    let r = omt_texpack::download::fetch_samples(&dir, omt_texpack::download::SAMPLES_TAG, &mut |done, total| {
        if done - last > 20 << 20 {
            last = done;
            println!("{} / {:?} MB", done >> 20, total.map(|t| t >> 20));
        }
        true
    });
    println!("result: {r:?} in {:?}", t.elapsed());
    let none = |_: &str| None;
    for f in omt_texpack::installed(None, Some(&cache), &none) {
        let p = omt_texpack::Pack::load(f.label.clone(), &f.layers).expect("pack");
        println!("{}: {} block textures", f.label, p.texture_count());
    }
}
