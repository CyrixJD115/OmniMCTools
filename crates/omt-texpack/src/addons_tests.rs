use std::io::Write;

use super::*;
use crate::{Face, Pack};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("omt-addons-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn png(rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, 1, 1);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    let mut w = e.write_header().unwrap();
    w.write_image_data(&rgb).unwrap();
    drop(w);
    out
}

fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn manifest(uuid: &str, name: &str, kind: &str) -> String {
    format!(r#"{{"format_version": 2, "header": {{"name": "{name}", "description": "§aGreen §rdesc", "uuid": "{uuid}", "version": [1, 2, 6]}}, "modules": [{{"type": "{kind}", "uuid": "00000000-0000-0000-0000-000000000000", "version": [1, 0, 0]}}]}}"#)
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn reads_bedrock_and_java_packs() {
    let d = tmp("read");
    let b = d.join("bed");
    put(&b, "manifest.json", manifest("ABC-1", "pack.name", "resources").as_bytes());
    put(&b, "texts/en_US.lang", b"pack.name=Faithful Bedrock\t##\npack.description=x\n");
    put(&b, "textures/blocks/stone.png", &png([1, 2, 3]));
    let a = read_pack(&b, AddonSource::Imported).unwrap();
    assert_eq!((a.id.as_str(), a.name.as_str(), a.version.as_str(), a.kind), ("abc-1", "Faithful Bedrock", "1.2.6", AddonKind::Bedrock));
    assert_eq!(a.description, "Green desc", "formatting codes stripped");
    assert!(a.has_block_textures() && !a.encrypted);

    let beh = d.join("beh");
    put(&beh, "manifest.json", manifest("B-2", "Logic", "data").as_bytes());
    assert!(read_pack(&beh, AddonSource::Imported).is_none(), "behaviour packs are not texture packs");

    let j = d.join("Faithful 32x");
    put(&j, "pack.mcmeta", br#"{"pack": {"pack_format": 34, "description": [{"text": "Hi "}, {"text": "there"}]}}"#);
    put(&j, "assets/minecraft/textures/block/stone.png", &png([9, 9, 9]));
    let a = read_pack(&j, AddonSource::Imported).unwrap();
    assert_eq!((a.id.as_str(), a.name.as_str(), a.description.as_str(), a.kind), ("java:Faithful 32x", "Faithful 32x", "Hi there", AddonKind::Java));

    let m = d.join("market");
    put(&m, "manifest.json", manifest("M-3", "Paid", "resources").as_bytes());
    put(&m, "contents.json", &[0, 0, 0, 0, 0xfc, 0xb9, 0xcf, 0x9b, 1, 2, 3]);
    assert!(read_pack(&m, AddonSource::Game("x".into())).unwrap().encrypted, "marketplace detected");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn imports_mcaddon_mcpack_zip_and_folders() {
    let d = tmp("import");
    let lib = d.join("library");
    // .mcaddon: a nested .mcpack (resources) + a behaviour folder.
    let inner = zip_of(&[("manifest.json", manifest("R-1", "Res", "resources").as_bytes()), ("textures/blocks/dirt.png", &png([5, 5, 5]))]);
    let addon = zip_of(&[("Res.mcpack", &inner), ("BP/manifest.json", manifest("B-1", "Beh", "data").as_bytes())]);
    put(&d, "thing.mcaddon", &addon);
    let got = import(&d.join("thing.mcaddon"), &lib).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "Res");
    assert!(got[0].root.starts_with(&lib) && got[0].root.join("textures/blocks/dirt.png").is_file());

    // Zipped Java pack (files at the zip root) and a zip-slip attempt.
    let java = zip_of(&[("pack.mcmeta", br#"{"pack": {"pack_format": 15, "description": "J"}}"#), ("assets/minecraft/textures/block/dirt.png", &png([6, 6, 6])), ("../../escape.txt", b"no")]);
    put(&d, "Cool Pack.zip", &java);
    let got = import(&d.join("Cool Pack.zip"), &lib).unwrap();
    assert_eq!((got[0].name.as_str(), got[0].kind), ("Cool Pack", AddonKind::Java));
    assert!(!d.join("escape.txt").exists() && !lib.parent().unwrap().join("escape.txt").exists(), "zip slip blocked");

    // A folder, imported again: replaced, not duplicated.
    let f = d.join("srcpack");
    put(&f, "manifest.json", manifest("R-1", "Res v2", "resources").as_bytes());
    let got = import(&f, &lib).unwrap();
    assert_eq!(got[0].name, "Res v2");
    let all = scan_dir(&lib, AddonSource::Imported);
    assert_eq!(all.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Cool Pack", "Res v2"]);

    put(&d, "empty.zip", &zip_of(&[("readme.txt", b"hi")]));
    assert!(matches!(import(&d.join("empty.zip"), &lib), Err(ImportError::NoPack)));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn world_packs_follow_the_world_order_with_subpacks() {
    let d = tmp("world");
    for (dir, id) in [("p1", "AAA"), ("p2", "BBB"), ("p3", "CCC")] {
        put(&d.join("resource_packs").join(dir), "manifest.json", manifest(id, dir, "resources").as_bytes());
    }
    std::fs::create_dir_all(d.join("resource_packs/p2/subpacks/fancy")).unwrap();
    put(&d, "world_resource_packs.json", br#"[ {"pack_id": "bbb", "subpack": "fancy", "version": [1,0,0]}, {"pack_id": "aaa", "version": [1,0,0]}, ]"#);
    let w = world_packs(&d);
    assert_eq!(w.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["p2", "p1"], "only applied packs, top first");
    assert_eq!(w[0].layers().len(), 2, "subpack layered on its pack");
    assert!(world_packs(&d.join("nope")).is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

/// Tier priority: an add-on with any texture for a face wins over vanilla
/// even under a different name (Java `grass_block_top` vs Bedrock
/// `grass_top` from blocks.json); vanilla fills the gaps.
#[test]
fn addon_tiers_override_vanilla() {
    let d = tmp("tiers");
    let v = d.join("vanilla");
    put(&v, "blocks.json", br#"{"grass": {"textures": {"up": "grass_top", "side": "grass_side"}}}"#);
    put(&v, "textures/terrain_texture.json", br#"{"texture_data": {"grass_top": {"textures": "textures/blocks/grass_top"}}}"#);
    put(&v, "textures/blocks/grass_top.png", &png([0, 200, 0]));
    put(&v, "textures/blocks/stone.png", &png([100, 100, 100]));
    put(&v, "textures/blocks/dirt.png", &png([90, 60, 30]));
    let j = d.join("java");
    put(&j, "pack.mcmeta", br#"{"pack": {"pack_format": 34, "description": ""}}"#);
    put(&j, "assets/minecraft/textures/block/grass_block_top.png", &png([1, 2, 3]));
    put(&j, "assets/minecraft/textures/block/stone.png", &png([4, 5, 6]));
    let b = d.join("bedrock_addon");
    put(&b, "textures/blocks/stone.png", &png([7, 8, 9]));
    let rgb = |p: &Pack, block: &str| p.face(block, Face::Up, false).map(|i| [i.rgba[0], i.rgba[1], i.rgba[2]]);

    let p = Pack::load_tiers("t", &[vec![v.clone()], vec![j.clone()]]).unwrap();
    assert_eq!(rgb(&p, "minecraft:grass_block"), Some([1, 2, 3]), "Java name wins over the vanilla terrain path");
    assert_eq!(rgb(&p, "minecraft:stone"), Some([4, 5, 6]));
    assert_eq!(rgb(&p, "minecraft:dirt"), Some([90, 60, 30]), "vanilla fills gaps");

    let p = Pack::load_tiers("t", &[vec![v.clone()], vec![j.clone()], vec![b.clone()]]).unwrap();
    assert_eq!(rgb(&p, "minecraft:stone"), Some([7, 8, 9]), "top tier wins");
    assert_eq!(rgb(&p, "minecraft:grass_block"), Some([1, 2, 3]), "lower add-on still beats vanilla");

    let p = Pack::load("v", std::slice::from_ref(&v)).unwrap();
    assert_eq!(rgb(&p, "minecraft:grass_block"), Some([0, 200, 0]), "vanilla alone unchanged");
    let _ = std::fs::remove_dir_all(&d);
}

/// Real-file probe: import a Java resource pack (`OMT_JAVA_PACK`, a .zip)
/// into a temp library, stack it over an installed game's vanilla
/// `resource_packs` (`OMT_VANILLA_RP`) and check common blocks resolve to
/// the Java pack's own files. `cargo test -p omt-texpack real_java_pack -- --ignored --nocapture`.
#[test]
#[ignore = "needs OMT_JAVA_PACK + OMT_VANILLA_RP"]
fn real_java_pack() {
    let (Some(zip), Some(rp)) = (std::env::var_os("OMT_JAVA_PACK"), std::env::var_os("OMT_VANILLA_RP")) else { return };
    let d = tmp("realjava");
    let got = import(Path::new(&zip), &d.join("lib")).unwrap();
    assert_eq!(got.len(), 1);
    let a = &got[0];
    assert_eq!(a.kind, AddonKind::Java);
    println!("imported {:?} ({}): {}", a.name, a.id, a.description);
    let vanilla = crate::vanilla_stack(Path::new(&rp));
    let base = Pack::load("vanilla", &vanilla).unwrap();
    let pack = Pack::load_tiers("t", &[vanilla.clone(), a.layers()]).unwrap();
    let blocks = ["stone", "grass_block", "dirt", "oak_log", "oak_planks", "oak_leaves", "sand", "gravel", "cobblestone", "deepslate", "diamond_ore", "white_wool", "glass", "water", "bricks", "andesite", "snow_block", "spruce_planks"];
    let (mut from_java, mut total) = (0, 0);
    for b in blocks {
        let Some(img) = pack.face(&format!("minecraft:{b}"), Face::Up, false) else { continue };
        total += 1;
        // The Java pack's own file for this block (top or plain name).
        let own = [format!("{b}_top"), b.to_string(), format!("{b}_still")].iter().find_map(|n| {
            std::fs::read(a.root.join(format!("assets/minecraft/textures/block/{n}.png"))).ok().and_then(|bytes| crate::decode::decode(&bytes).ok())
        });
        let java = own.as_ref().is_some_and(|o| {
            let mut o = o.clone();
            if o.h > o.w {
                o.rgba.truncate((o.w * o.w * 4) as usize);
                o.h = o.w;
            }
            o.rgba == img.rgba
        });
        let differs = base.face(&format!("minecraft:{b}"), Face::Up, false).is_none_or(|v| v.rgba != img.rgba);
        println!("{b:>14}: {} ({}×{}){}", if java { "JAVA PACK" } else { "vanilla" }, img.w, img.h, if java && !differs { "  (same pixels as vanilla)" } else { "" });
        if own.is_some() {
            assert!(java, "{b}: the pack ships a texture but vanilla won");
            from_java += 1;
        }
    }
    println!("{from_java}/{total} faces from the Java pack, {} textures total", pack.texture_count());
    assert!(from_java >= 8);
    let _ = std::fs::remove_dir_all(&d);
}

/// Zipped Java packs in a launcher folder are listed from the zip index
/// and extracted (block textures only) on first use, then reused.
#[test]
fn launcher_zip_packs_list_and_materialize() {
    let d = tmp("javazip");
    let rp = d.join("resourcepacks");
    std::fs::create_dir_all(&rp).unwrap();
    put(&rp, "Faithful.zip", &zip_of(&[("pack.mcmeta", br#"{"pack": {"pack_format": 34, "description": "F"}}"#), ("assets/minecraft/textures/block/stone.png", &png([3, 3, 3])), ("assets/minecraft/sounds/x.ogg", b"big")]));
    put(&rp, "Recipes.zip", &zip_of(&[("pack.mcmeta", br#"{"pack": {"pack_format": 34, "description": "R"}}"#), ("data/x.json", b"{}")]));
    put(&rp, "notes.zip", &zip_of(&[("readme.txt", b"no pack")]));
    let packs = java_packs(&rp, "Prism · Test");
    assert_eq!(packs.iter().map(|a| (a.name.as_str(), a.block_textures)).collect::<Vec<_>>(), [("Faithful", true), ("Recipes", false)]);
    let a = &packs[0];
    assert!(a.archive && a.source == AddonSource::Game("Prism · Test".into()));
    let cache = d.join("cache");
    let root = materialize(a, &cache).unwrap();
    assert!(root.join("assets/minecraft/textures/block/stone.png").is_file());
    assert!(!root.join("assets/minecraft/sounds/x.ogg").exists(), "only block textures are extracted");
    assert_eq!(materialize(a, &cache).unwrap(), root, "reused");
    let p = Pack::load_tiers("t", &[vec![root]]).unwrap();
    assert_eq!(&p.face("minecraft:stone", Face::Up, false).unwrap().rgba[..3], &[3, 3, 3]);
    let _ = std::fs::remove_dir_all(&d);
}

/// Real-file probe: item icons from an installed game's vanilla stack
/// (`OMT_VANILLA_RP`), optionally under a Java pack (`OMT_JAVA_PACK`).
#[test]
#[ignore = "needs OMT_VANILLA_RP"]
fn real_item_icons() {
    let Some(rp) = std::env::var_os("OMT_VANILLA_RP") else { return };
    let mut tiers = vec![crate::vanilla_stack(Path::new(&rp))];
    let d = tmp("realitems");
    if let Some(zip) = std::env::var_os("OMT_JAVA_PACK") {
        tiers.push(import(Path::new(&zip), &d.join("lib")).unwrap()[0].layers());
    }
    let p = Pack::load_tiers("t", &tiers).unwrap();
    let mut found = 0;
    for n in ["diamond", "diamond_sword", "apple", "golden_apple", "oak_boat", "bow", "ender_pearl", "iron_ingot", "wooden_axe", "chicken", "ink_sac", "golden_sword", "oak_log"] {
        let img = p.item(&format!("minecraft:{n}"));
        println!("{n:>14}: {}", img.as_ref().map_or("—".to_string(), |i| format!("{}×{}", i.w, i.h)));
        found += usize::from(img.is_some());
    }
    assert!(found >= 11, "{found}");
    assert!(p.item("minecraft:oak_log").is_none(), "block items have no item texture");
    let _ = std::fs::remove_dir_all(&d);
}
