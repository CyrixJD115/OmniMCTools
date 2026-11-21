//! `asset-gen [OUT_DIR]` — export every generated image as PNG (default
//! `target/omt-art`), plus `sheet.png`, a contact sheet of everything.
//! The GUI does not read these files (it generates the same art at
//! startup); they are for packaging (`app-icon-*.png`) and review.

use std::path::{Path, PathBuf};

use omt_art::{Glyph, Icon, Image, Material, Scene};

fn save(img: &Image, path: &Path) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.w, img.h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    w.write_image_data(&img.rgba).map_err(std::io::Error::other)?;
    Ok(())
}

fn main() -> std::io::Result<()> {
    let out = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("target/omt-art"));
    std::fs::create_dir_all(&out)?;
    let mut sheet_items: Vec<Image> = Vec::new();

    for px in [32, 64, 128, 256] {
        save(&omt_art::app_icon(px), &out.join(format!("app-icon-{px}.png")))?;
    }
    let mark = omt_art::wordmark(4);
    save(&mark, &out.join("wordmark.png"))?;
    sheet_items.push(mark);
    for i in Icon::ALL {
        let img = omt_art::icon(i, 96);
        save(&img, &out.join(format!("icon-{}.png", i.name())))?;
        sheet_items.push(img);
    }
    for g in Glyph::ALL {
        sheet_items.push(omt_art::glyph(g, [230, 232, 236, 255], 96));
    }
    for m in [Material::GrassTop, Material::Stone, Material::Deepslate, Material::DiamondOre, Material::Planks, Material::Netherrack, Material::EndStone, Material::Amethyst, Material::GoldBlock, Material::Copper] {
        sheet_items.push(omt_art::tool_block(m, 96));
    }
    for s in Scene::ALL {
        let img = omt_art::landscape(s, 42, 480, 160, 6);
        save(&img.scaled(2), &out.join(format!("scene-{}.png", s.name())))?;
        sheet_items.push(img);
    }

    // Contact sheet: flow layout on a dark background.
    let width = 1000u32;
    let (mut x, mut y, mut row_h) = (8i32, 8i32, 0u32);
    let mut placed = Vec::new();
    for img in &sheet_items {
        if x as u32 + img.w + 8 > width {
            x = 8;
            y += row_h as i32 + 8;
            row_h = 0;
        }
        placed.push((x, y));
        x += img.w as i32 + 8;
        row_h = row_h.max(img.h);
    }
    let mut sheet = Image::filled(width, y as u32 + row_h + 8, omt_art::hex(0x1b1d22));
    for (img, (x, y)) in sheet_items.iter().zip(placed) {
        sheet.draw(img, x, y);
    }
    save(&sheet, &out.join("sheet.png"))?;
    println!("wrote {}", out.display());
    Ok(())
}
