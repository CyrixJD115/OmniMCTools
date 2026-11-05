//! Unit tests for the Bedrock → Java chunk converter (moved out of
//! `chunk_to_java.rs` to respect the <700-line file budget; see AGENTS.md).

use omt_convert::chunk_to_java::{BedrockConversion, JavaChunkWrite, bedrock_chunk_to_java};
use omt_convert::{LossKind, ProtoFilter, RegistryRules};
use omt_nbt::{ByteOrder, Tag};
use omt_world::packing;
use omt_world::{BedrockBiomeSlot, BlockState, Chunk, Slice};


    fn filter() -> ProtoFilter {
        ProtoFilter::default()
    }

    fn air() -> BlockState {
        BlockState::air()
    }

    fn bedrock_column() -> Chunk {
        let mut chunk = Chunk { x: 3, z: -5, dim: 0, ..Default::default() };
        chunk.status = Some("full".into());
        // Slot −4 (Y −64): uniform stone section, uniform plains biome.
        chunk.slices.insert(
            -4,
            Slice {
                palette: vec![BlockState { name: "minecraft:stone".into(), properties: vec![] }],
                indices: vec![0; 4096],
            },
        );
        // Slot 0 (Y 0): air/stone pattern, stateful stairs at one position.
        let mut indices = vec![0u16; 4096]; // palette[0] = air
        indices[(2 << 8) | (3 << 4) | 1] = 1; // stone at (x=1,y=2,z=3)
        indices[(5 << 8) | (6 << 4) | 4] = 2; // stairs at (x=4,y=5,z=6)
        chunk.slices.insert(
            0,
            Slice {
                palette: vec![
                    air(),
                    BlockState { name: "minecraft:stone".into(), properties: vec![] },
                    BlockState {
                        name: "minecraft:oak_stairs".into(),
                        properties: vec![("facing".into(), "east".into())],
                    },
                ],
                indices,
            },
        );
        // Biomes: slot 4 uniform plains (id 1); slot 0 mixed plains/desert
        // with variance inside one cell.
        let mut slot0_indices = vec![0u16; 4096];
        // Cell (cx=0, cy=0, cz=0): 63× plains (0), 1× desert (1) — the
        // desert block loses the vote and is reported as coerced.
        slot0_indices[omt_bedrock::subchunk::block_index_xzy(3, 0, 0)] = 1;
        // Cell (cx=1, cy=0, cz=0): uniform desert — wins that cell.
        for dx in 0..4usize {
            for dy in 0..4usize {
                for dz in 0..4usize {
                    let i = omt_bedrock::subchunk::block_index_xzy(4 + dx, dy, dz);
                    slot0_indices[i] = 1;
                }
            }
        }
        chunk.bedrock_biomes = Some(vec![
            BedrockBiomeSlot::NonExistent,
            BedrockBiomeSlot::NonExistent,
            BedrockBiomeSlot::NonExistent,
            BedrockBiomeSlot::NonExistent,
            BedrockBiomeSlot::uniform(1), // slot 4 = Y 0
        ]);
        // Replace slot 0's marker with a real storage at index 4 below.
        let mut slots = chunk.bedrock_biomes.take().unwrap();
        slots[4] = BedrockBiomeSlot::Storage {
            palette: vec![1, 2], // plains, desert
            indices: Some(slot0_indices),
            bits: 1,
        };
        slots[0] = BedrockBiomeSlot::uniform(1); // bottom slot: plains
        chunk.bedrock_biomes = Some(slots);
        // data3D heightmap: 256 values, z-major.
        chunk.heightmaps =
            vec![("bedrock_2d".into(), (0..256).map(|i| (100 + i) as u64).collect())];
        chunk
    }

    fn convert(chunk: &Chunk) -> JavaChunkWrite {
        match bedrock_chunk_to_java(chunk, &RegistryRules::default(), &filter(), 4440).unwrap() {
            BedrockConversion::Converted(w) => w,
            BedrockConversion::SkippedProto { .. } => panic!("expected conversion"),
        }
    }

    fn parse(w: &JavaChunkWrite) -> Tag {
        let nbt = omt_nbt::read(&w.chunk_nbt, ByteOrder::Big).unwrap();
        nbt.tag
    }

    #[test]
    fn root_fields_match_the_java_write_shape() {
        let w = convert(&bedrock_column());
        let root = parse(&w);
        assert_eq!(root.get("DataVersion").and_then(Tag::as_i64), Some(4440));
        assert_eq!(root.get("xPos").and_then(Tag::as_i64), Some(3));
        assert_eq!(root.get("yPos").and_then(Tag::as_i64), Some(-4));
        assert_eq!(root.get("zPos").and_then(Tag::as_i64), Some(-5));
        assert_eq!(root.get("Status").and_then(Tag::as_str), Some("minecraft:full"));
        assert!(root.get("block_ticks").and_then(Tag::as_list).unwrap().is_empty());
        assert_eq!(
            root.get("PostProcessing").and_then(Tag::as_list).unwrap().len(),
            24
        );
        assert_eq!(root.get("InhabitedTime").and_then(Tag::as_i64), Some(0));
    }

    #[test]
    fn stateful_blocks_drop_properties_with_loss_rows() {
        let w = convert(&bedrock_column());
        let root = parse(&w);
        let sections = root.get("sections").and_then(Tag::as_list).unwrap();
        assert_eq!(sections.len(), 2, "all-air bottom slot exists here; slots −4 and 0 are not air");
        let top = sections.last().unwrap();
        let palette = top
            .get("block_states")
            .unwrap()
            .get("palette")
            .and_then(Tag::as_list)
            .unwrap();
        assert_eq!(palette.len(), 3);
        assert!(palette[2].get("Properties").is_none(), "unmapped properties dropped");
        let stairs_loss = w
            .report
            .losses
            .iter()
            .filter(|l| l.kind == LossKind::UnmappedId && l.path.contains("oak_stairs"))
            .count();
        assert_eq!(stairs_loss, 1, "one deduped row for the stairs properties");
    }

    #[test]
    fn empty_sections_are_omitted() {
        let mut chunk = bedrock_column();
        chunk.slices.remove(&0); // leave only the uniform stone slot −4
        let w = convert(&chunk);
        let root = parse(&w);
        let sections = root.get("sections").and_then(Tag::as_list).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].get("Y").and_then(Tag::as_i64), Some(-4));
    }

    #[test]
    fn single_entry_palette_omits_data() {
        let w = convert(&bedrock_column());
        let root = parse(&w);
        let sections = root.get("sections").and_then(Tag::as_list).unwrap();
        let bottom = sections.first().unwrap();
        let bs = bottom.get("block_states").unwrap();
        assert_eq!(bs.get("palette").and_then(Tag::as_list).unwrap().len(), 1);
        assert!(bs.get("data").is_none(), "palette ≤ 1 ⇒ no data long array");
    }

    #[test]
    fn heightmap_packs_and_round_trips() {
        let w = convert(&bedrock_column());
        let root = parse(&w);
        let Tag::LongArray(packed) = root
            .get("Heightmaps")
            .unwrap()
            .get("MOTION_BLOCKING")
            .unwrap()
        else {
            panic!("heightmap is not a long array");
        };
        assert_eq!(packed.len(), 37);
        let unpacked = packing::unpack_java_heightmap(packed);
        let expected: Vec<u16> = (0..256).map(|i| (100 + i) as u16).collect();
        assert_eq!(unpacked, expected);
    }

    #[test]
    fn biomes_downsample_with_vote_and_loss_row() {
        let w = convert(&bedrock_column());
        let root = parse(&w);
        let sections = root.get("sections").and_then(Tag::as_list).unwrap();
        let top = sections.last().unwrap();
        let biomes = top.get("biomes").unwrap();
        let palette: Vec<&str> = biomes
            .get("palette")
            .and_then(Tag::as_list)
            .unwrap()
            .iter()
            .filter_map(Tag::as_str)
            .collect();
        assert_eq!(palette, vec!["minecraft:plains", "minecraft:desert"]);
        let cells = packing::unpack_java_biome_cells(
            match biomes.get("data").unwrap() {
                Tag::LongArray(v) => v,
                _ => panic!("biome data is not a long array"),
            },
            palette.len(),
        );
        // 63 of the 64 blocks in cell 0 are plains — plains wins the vote;
        // cell 1 is uniform desert and carries desert into the palette.
        assert_eq!(cells[0], 0, "plains wins the vote in mixed cell 0");
        assert_eq!(cells[1], 1, "uniform desert cell elects desert");
        assert_eq!(cells[63], 0, "all-plain cells stay plains");
        // Uniform slot maps to a one-entry palette without data.
        let bottom = sections.first().unwrap();
        let bb = bottom.get("biomes").unwrap();
        assert_eq!(bb.get("palette").and_then(Tag::as_list).unwrap().len(), 1);
        assert!(bb.get("data").is_none());
        // One ValueCoerced row counts the mixed cells for the chunk.
        let coerced = w
            .report
            .losses
            .iter()
            .filter(|l| l.kind == LossKind::ValueCoerced && l.path.contains("biomes"))
            .count();
        assert_eq!(coerced, 1);
        assert!(w.report.losses.iter().any(|l| {
            l.kind == LossKind::ValueCoerced && l.detail.contains("1 of 64 biome cells")
        }));
    }

    #[test]
    fn non_existent_slots_chain_to_the_storage_below() {
        let mut chunk = bedrock_column();
        // Make slot 0 NonExistent so it chains down to the uniform slot 4.
        let slots = chunk.bedrock_biomes.as_mut().unwrap();
        slots[4] = BedrockBiomeSlot::NonExistent;
        slots[3] = BedrockBiomeSlot::uniform(2); // desert above bottom
        let w = convert(&chunk);
        let root = parse(&w);
        let sections = root.get("sections").and_then(Tag::as_list).unwrap();
        let top = sections.last().unwrap();
        let names: Vec<&str> = top
            .get("biomes")
            .unwrap()
            .get("palette")
            .and_then(Tag::as_list)
            .unwrap()
            .iter()
            .filter_map(Tag::as_str)
            .collect();
        // Slot 0 chains: NonExistent at 4 → storage below at 3 → desert (2).
        assert_eq!(names, vec!["minecraft:desert"]);
    }

    #[test]
    fn unknown_biome_id_falls_back_to_ocean_with_loss() {
        let mut chunk = bedrock_column();
        let slots = chunk.bedrock_biomes.as_mut().unwrap();
        slots[0] = BedrockBiomeSlot::uniform(777);
        slots[4] = BedrockBiomeSlot::NonExistent;
        let w = convert(&chunk);
        assert!(w
            .report
            .losses
            .iter()
            .any(|l| l.kind == LossKind::UnmappedId && l.path.contains("biome id 777")));
        let root = parse(&w);
        let sections = root.get("sections").and_then(Tag::as_list).unwrap();
        let bottom = sections.first().unwrap();
        let names: Vec<&str> = bottom
            .get("biomes")
            .unwrap()
            .get("palette")
            .and_then(Tag::as_list)
            .unwrap()
            .iter()
            .filter_map(Tag::as_str)
            .collect();
        assert_eq!(names, vec!["minecraft:ocean"]);
    }

    #[test]
    fn block_entities_pass_through_with_aggregate_row() {
        let mut chunk = bedrock_column();
        chunk.block_entities = vec![Tag::Compound(vec![
            ("id".to_string(), Tag::String("minecraft:chest".into())),
            ("x".to_string(), Tag::Int(48)),
            ("y".to_string(), Tag::Int(-64)),
            ("z".to_string(), Tag::Int(-80)),
        ])];
        let w = convert(&chunk);
        let root = parse(&w);
        let bes = root.get("block_entities").and_then(Tag::as_list).unwrap();
        assert_eq!(bes.len(), 1);
        assert_eq!(bes[0].get("id").and_then(Tag::as_str), Some("minecraft:chest"));
        assert_eq!(w.report.count_with_kind(LossKind::UnsupportedComponent), 1);
    }

    #[test]
    fn actors_get_id_rename_and_uuid_loss_row() {
        let mut chunk = bedrock_column();
        chunk.entities = vec![
            Tag::Compound(vec![
                ("identifier".to_string(), Tag::String("minecraft:pig".into())),
                ("UniqueID".to_string(), Tag::Long(42)),
                ("Pos".to_string(), Tag::List(vec![Tag::Float(0.5), Tag::Float(64.0), Tag::Float(0.5)])),
            ]),
            Tag::Compound(vec![
                ("id".to_string(), Tag::String("minecraft:pig".into())),
                ("UUID".to_string(), Tag::IntArray(vec![1, 2, 3, 4])),
            ]),
        ];
        let w = convert(&chunk);
        assert_eq!(w.entities.len(), 2);
        // identifier → id; everything else opaque.
        let Tag::Compound(fields) = &w.entities[0] else { panic!() };
        assert!(fields.iter().any(|(k, _)| k == "id"));
        assert!(!fields.iter().any(|(k, _)| k == "identifier"));
        // The actor that already had a UUID produced no extra loss.
        let uuid_rows = w
            .report
            .losses
            .iter()
            .filter(|l| l.kind == LossKind::FieldDropped && l.path == "entities")
            .count();
        assert_eq!(uuid_rows, 1, "one UUID loss row for the one UUID-less actor");
    }

    #[test]
    fn non_full_status_is_skipped_by_the_filter() {
        let mut chunk = bedrock_column();
        chunk.status = Some("empty".into());
        let out = bedrock_chunk_to_java(&chunk, &RegistryRules::default(), &filter(), 4440)
            .unwrap();
        assert!(matches!(out, BedrockConversion::SkippedProto { .. }));
    }

    /// Guards the uniform-slot fast path when `bedrock_biomes` is missing
    /// entirely: sections still convert, with a per-slot loss row.
    #[test]
    fn missing_biome_data_records_a_row_and_omits_biomes() {
        let mut chunk = bedrock_column();
        chunk.bedrock_biomes = None;
        let w = convert(&chunk);
        assert_eq!(
            w.report
                .losses
                .iter()
                .filter(|l| l.path.contains("biomes"))
                .count(),
            2,
            "one row per written section"
        );
        let root = parse(&w);
        for section in root.get("sections").and_then(Tag::as_list).unwrap() {
            assert!(section.get("biomes").is_none());
        }
    }

