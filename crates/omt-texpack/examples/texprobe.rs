//! `cargo run -p omt-texpack --example texprobe [COLORS_JSON] [OUT.png]`
//! Loads the newest installed/downloaded pack, reports top-face coverage
//! for every visible block of the renderer palette, writes a sample sheet.

fn main() {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let env = |k: &str| std::env::var_os(k);
    let found = omt_texpack::installed(home.as_deref(), None, &env);
    for f in &found {
        println!("found: {} ({} layers)", f.label, f.layers.len());
    }
    let Some(f) = found.first() else { return println!("no pack") };
    let t = std::time::Instant::now();
    let pack = omt_texpack::Pack::load(f.label.clone(), &f.layers).expect("pack");
    println!("loaded {} — {} block textures in {:?}", pack.label, pack.texture_count(), t.elapsed());
    let colors = std::env::args().nth(1).unwrap_or_else(|| "crates/omt-render/data/colors.json".into());
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(colors).expect("colors")).expect("json");
    let names: Vec<String> = v["blocks"].as_object().unwrap().iter().filter(|(_, a)| a[3].as_u64().unwrap_or(0) > 0).map(|(k, _)| k.clone()).collect();
    let t = std::time::Instant::now();
    let mut missing = Vec::new();
    let mut hit = Vec::new();
    for n in &names {
        match pack.face(n, omt_texpack::Face::Up, false) {
            Some(img) => hit.push((n.clone(), img)),
            None => missing.push(n.clone()),
        }
    }
    println!("top faces: {}/{} resolved in {:?}", hit.len(), names.len(), t.elapsed());
    println!("missing (first 60): {:?}", &missing[..missing.len().min(60)]);
    if let Some(out) = std::env::args().nth(2) {
        let cols = 32u32;
        let rows = (hit.len() as u32).div_ceil(cols);
        let mut sheet = omt_art::Image::new(cols * 34, rows * 34);
        for (i, (_, img)) in hit.iter().enumerate() {
            let scaled = if img.w == 16 { img.scaled(2) } else { (**img).clone() };
            sheet.draw(&scaled, (i as u32 % cols * 34) as i32, (i as u32 / cols * 34) as i32);
        }
        let file = std::fs::File::create(&out).unwrap();
        let mut e = png::Encoder::new(std::io::BufWriter::new(file), sheet.w, sheet.h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&sheet.rgba).unwrap();
        println!("wrote {out}");
    }
}
