//! Generate a small synthetic Java world at a path (demo/test scaffold).
//! Uses the current 26.3-era layout (`dimensions/minecraft/overworld/…`,
//! namespaced; PLAN §0 2026-11-08).
use omt_java::RegionWriter;
use omt_nbt::{ByteOrder, Container, Nbt, Tag};
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: make_demo_world <dir>"));
    let region = dir.join("dimensions").join("minecraft").join("overworld").join("region");
    std::fs::create_dir_all(&region).unwrap();
    let level = Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![("Data".into(), Tag::Compound(vec![
            ("LevelName".into(), Tag::String("OMT Java Demo".into())),
            ("DataVersion".into(), Tag::Int(5023)),
            ("GameType".into(), Tag::Int(1)),
            ("spawn".into(), Tag::Compound(vec![
                ("pos".into(), Tag::IntArray(vec![0, 64, 0])),
                ("dimension".into(), Tag::String("minecraft:overworld".into())),
            ])),
        ]))]),
    };
    std::fs::write(
        dir.join("level.dat"),
        Container::Gzip.encode(&omt_nbt::write(&level, ByteOrder::Big).unwrap()),
    )
    .unwrap();
    let mut regions: std::collections::BTreeMap<(i32, i32), RegionWriter> =
        std::collections::BTreeMap::new();
    for x in -1..=1 {
        for z in -1..=1 {
            let top = if (x + z) % 2 == 0 { "minecraft:stone" } else { "minecraft:grass_block" };
            let chunk = Nbt {
                name: "".into(),
                tag: Tag::Compound(vec![
                    ("DataVersion".into(), Tag::Int(5023)),
                    ("xPos".into(), Tag::Int(x)),
                    ("zPos".into(), Tag::Int(z)),
                    ("Status".into(), Tag::String("minecraft:full".into())),
                    ("sections".into(), Tag::List(vec![Tag::Compound(vec![
                        ("Y".into(), Tag::Byte(0)),
                        ("block_states".into(), Tag::Compound(vec![
                            // 26.3 palette shape: plain string entries.
                            ("palette".into(), Tag::List(vec![
                                Tag::String(top.into()),
                                Tag::String("minecraft:air".into()),
                            ])),
                        ])),
                    ])])),
                ]),
            };
            let raw = omt_nbt::write(&chunk, ByteOrder::Big).unwrap();
            let key = (x.div_euclid(32), z.div_euclid(32));
            regions.entry(key).or_default().put(x, z, raw, 42);
        }
    }
    for ((rx, rz), w) in regions {
        std::fs::write(region.join(format!("r.{rx}.{rz}.mca")), w.finish()).unwrap();
    }
    println!("demo world at {}", dir.display());
}
