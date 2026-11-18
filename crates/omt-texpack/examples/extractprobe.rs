//! `cargo run -p omt-texpack --example extractprobe ZIP OUT_DIR` — extract
//! a downloaded bedrock-samples zip the way the in-app download does and
//! load it as a pack (verifies the samples layout end to end).

fn main() {
    let mut a = std::env::args().skip(1);
    let (zip, out) = (a.next().expect("ZIP"), a.next().expect("OUT_DIR"));
    let t = std::time::Instant::now();
    let n = omt_texpack::download::extract(zip.as_ref(), out.as_ref()).expect("extract");
    println!("extracted {n} files in {:?}", t.elapsed());
    let rp = std::path::Path::new(&out).join("resource_pack");
    let pack = omt_texpack::Pack::load("samples", &[rp]).expect("pack");
    println!("{} block textures", pack.texture_count());
    for b in ["minecraft:grass_block", "minecraft:deepslate", "minecraft:oak_log", "minecraft:cherry_leaves", "minecraft:white_wool", "minecraft:copper_block"] {
        let img = pack.face(b, omt_texpack::Face::Up, false);
        println!("{b}: {:?}", img.map(|i| (i.w, i.h)));
    }
}
