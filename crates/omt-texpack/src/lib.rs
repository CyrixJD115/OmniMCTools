//! `omt-texpack` — real Minecraft block textures for the UI and the map,
//! read from the user's **installed** Bedrock game (BedrockOnLinux,
//! mcpelauncher, Windows) or from an opt-in download of Mojang's
//! `bedrock-samples` into the user's cache. Nothing is bundled: these
//! files are Mojang's (Minecraft EULA). Without a pack, OMT falls back to
//! its procedural art (`omt-art`) and map colors.

pub mod addons;
pub mod brarchive;
pub mod decode;
pub mod download;
pub mod locate;
pub mod pack;

pub use locate::{Found, installed, vanilla_stack};
pub use addons::{Addon, AddonKind, AddonSource};
pub use pack::{Face, Pack};

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny two-layer stack: base JSON + a newer archived override.
    #[test]
    fn stack_resolution() {
        let tmp = std::env::temp_dir().join(format!("omt-texpack-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let base = tmp.join("vanilla");
        let newer = tmp.join("vanilla_1.21.0");
        std::fs::create_dir_all(base.join("textures/blocks")).unwrap();
        std::fs::create_dir_all(newer.join("__brarchive/textures")).unwrap();
        let png = |rgb: [u8; 3]| {
            let mut out = Vec::new();
            let mut e = png::Encoder::new(&mut out, 1, 2);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[rgb[0], rgb[1], rgb[2], 0, 0, 0]).unwrap();
            drop(w);
            out
        };
        std::fs::write(base.join("blocks.json"), "{ // comment\n \"grass\": {\"textures\": {\"up\": \"grass_top\", \"side\": \"grass_side\"},},\n}").unwrap();
        std::fs::write(base.join("textures/terrain_texture.json"), r#"{"texture_data": {"grass_top": {"textures": ["textures/blocks/grass_top"]}}}"#).unwrap();
        std::fs::write(base.join("textures/blocks/grass_top.png"), png([10, 200, 10])).unwrap();
        std::fs::write(base.join("textures/blocks/log_oak.png"), png([100, 70, 40])).unwrap();
        let arch = brarchive::build(&[("deepslate.png", &png([50, 50, 55])), ("log_oak.png", &png([1, 2, 3]))]);
        std::fs::write(newer.join("__brarchive/textures/blocks.brarchive"), arch).unwrap();
        let pack = Pack::load("test", &[base.clone(), newer.clone()]).unwrap();
        let top = pack.face("minecraft:grass_block", Face::Up, false).unwrap();
        assert_eq!((top.w, top.h), (1, 1), "flipbook cropped to square");
        assert_eq!(&top.rgba[..3], &[10, 200, 10]);
        assert_eq!(&pack.face("minecraft:deepslate", Face::Up, false).unwrap().rgba[..3], &[50, 50, 55], "archived file");
        assert_eq!(&pack.face("minecraft:oak_log", Face::Side, false).unwrap().rgba[..3], &[1, 2, 3], "flipped name, newer layer wins");
        assert!(pack.face("minecraft:unknown_thing", Face::Up, false).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
