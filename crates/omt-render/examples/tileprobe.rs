//! Print stride / size / chunk counts of LOD tiles around a block position.
//! `cargo run --release -p omt-render --example tileprobe -- <bedrock world> <level> <bx> <bz> [budget]`
use std::sync::Arc;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let world = Arc::new(omt_bedrock::BedrockWorld::open_read_only(std::path::Path::new(&a[0])).unwrap());
    let level: u8 = a[1].parse().unwrap();
    let (bx, bz): (i32, i32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let budget: Option<u64> = a.get(4).map(|b| b.parse().unwrap());
    let index = world.column_index().unwrap();
    let src = omt_render::MapSource::bedrock(&world, &index, 0);
    println!("level {level} stride {}", src.level_stride(level, budget));
    let c = omt_render::TileKey::containing(level, bx.div_euclid(16), bz.div_euclid(16));
    let out = a.get(5).cloned();
    let mut comp: Vec<u8> = Vec::new();
    let mut side = 0usize;
    for tz in c.tz - 1..=c.tz + 1 {
        for tx in c.tx - 1..=c.tx + 1 {
            let k = omt_render::TileKey::new(level, tx, tz);
            let t = omt_render::build_tile(&src, k, budget, omt_render::Palette::embedded(), &omt_render::RenderOptions::default(), &|| false).unwrap();
            let opaque = t.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] != 0).count();
            println!("{k:?}: stride {} px {} chunks {} opaque {opaque} {}ms", t.stride, t.px, t.stats.chunks, t.millis);
            let px = t.px as usize;
            if side == 0 && px > 0 {
                side = px;
                comp = vec![0; 9 * px * px * 4];
            }
            if px == side && px > 0 {
                let (ox, oz) = ((tx - c.tx + 1) as usize * px, (tz - c.tz + 1) as usize * px);
                for z in 0..px {
                    let d = ((oz + z) * 3 * px + ox) * 4;
                    comp[d..d + px * 4].copy_from_slice(&t.rgba[z * px * 4..(z + 1) * px * 4]);
                }
            }
        }
    }
    if let Some(out) = out {
        let f = std::fs::File::create(&out).unwrap();
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), (side * 3) as u32, (side * 3) as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&comp).unwrap();
        println!("composite → {out}");
    }
}
