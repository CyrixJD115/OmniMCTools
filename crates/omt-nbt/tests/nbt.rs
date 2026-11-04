//! Round-trip and malformed-input tests for `omt-nbt`.

use omt_nbt::{ByteOrder, Container, Error, Nbt, Tag};
use omt_nbt::string;

fn demo_tree() -> Nbt {
    Nbt {
        name: "".into(),
        tag: Tag::Compound(vec![
            ("LevelName".into(), Tag::String("Test's World ✓".into())),
            ("StorageVersion".into(), Tag::Int(10)),
            ("version".into(), Tag::List(vec![
                Tag::Int(1), Tag::Int(21), Tag::Int(90), Tag::Int(0), Tag::Int(0),
            ])),
            ("nested".into(), Tag::Compound(vec![
                ("deep".into(), Tag::List(vec![
                    Tag::Compound(vec![("a".into(), Tag::Byte(1))]),
                    Tag::Compound(vec![("a".into(), Tag::Byte(2))]),
                ])),
                ("f".into(), Tag::Float(1.5)),
                ("d".into(), Tag::Double(-0.25)),
                ("s".into(), Tag::Short(-2)),
                ("l".into(), Tag::Long(i64::MIN)),
            ])),
            ("heights".into(), Tag::LongArray(vec![0, 1, u64::MAX as i64])),
            ("palette".into(), Tag::IntArray(vec![-1, 0, 255])),
            ("raw".into(), Tag::ByteArray(vec![0x0a, 0x00, -0x01])),
            ("empty_list".into(), Tag::List(vec![])),
        ]),
    }
}

#[test]
fn roundtrip_both_orders_and_containers() {
    let demo = demo_tree();
    for order in [ByteOrder::Big, ByteOrder::Little] {
        let bytes = omt_nbt::write(&demo, order).unwrap();
        let back = omt_nbt::read(&bytes, order).unwrap();
        assert_eq!(back, demo, "order {order:?}");

        for container in [Container::Raw, Container::Gzip, Container::Zlib] {
            let packed = container.encode(&bytes);
            let (unpacked, used) = Container::decode(&packed).unwrap();
            assert_eq!(used, container);
            assert_eq!(omt_nbt::read(&unpacked, order).unwrap(), demo);
        }
    }
}

#[test]
fn little_endian_bytes_differ_from_big_endian() {
    let nbt = Nbt { name: "x".into(), tag: Tag::Int(0x01020304) };
    let be = omt_nbt::write(&nbt, ByteOrder::Big).unwrap();
    let le = omt_nbt::write(&nbt, ByteOrder::Little).unwrap();
    assert_ne!(be, le);
    assert_eq!(&be[be.len() - 4..], &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(&le[le.len() - 4..], &[0x04, 0x03, 0x02, 0x01]);
}

/// Bedrock fixture header shape: `0a 00 00 00` ver, `len` then compound with
/// empty name — verified by hexdump against `world_ref/*/level.dat`.
#[test]
fn bedrock_level_dat_shape() {
    let payload = omt_nbt::write(
        &Nbt { name: "".into(), tag: Tag::Compound(vec![("V".into(), Tag::Int(7))]) },
        ByteOrder::Little,
    )
    .unwrap();
    let mut file = Vec::new();
    file.extend_from_slice(&10u32.to_le_bytes());
    file.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    file.extend_from_slice(&payload);

    let ld = omt_nbt::BedrockLevelDat::parse(&file).unwrap();
    assert_eq!(ld.storage_version, 10);
    assert_eq!(ld.nbt.tag.get("V").unwrap().as_i64(), Some(7));
    assert_eq!(ld.serialize().unwrap(), file);
}

#[test]
fn level_dat_rejects_short_header_and_bad_length() {
    assert!(matches!(
        omt_nbt::BedrockLevelDat::parse(&[0, 0, 0]),
        Err(Error::BadLevelDatHeader(_))
    ));
    let mut file = Vec::new();
    file.extend_from_slice(&10u32.to_le_bytes());
    file.extend_from_slice(&9999u32.to_le_bytes());
    file.extend_from_slice(&[0x0a]);
    assert!(matches!(
        omt_nbt::BedrockLevelDat::parse(&file),
        Err(Error::BadLevelDatHeader(_))
    ));
}

#[test]
fn truncated_input_is_structured_error() {
    let bytes = omt_nbt::write(&demo_tree(), ByteOrder::Big).unwrap();
    for cut in [0usize, 1, 3, 7, 11, bytes.len() / 2, bytes.len() - 1] {
        let res = omt_nbt::read(&bytes[..cut], ByteOrder::Big);
        assert!(
            matches!(res, Err(Error::UnexpectedEof { .. }) | Err(Error::AbsurdLength { .. })),
            "cut at {cut} gave {res:?}"
        );
    }
}

#[test]
fn hostile_counts_do_not_allocate() {
    // Compound claiming a string longer than the buffer.
    let bad = [0x0a, 0x00, 0x00, 0x08, 0x00, 0xff, 0xff, 0x41];
    assert!(matches!(
        omt_nbt::read(&bad, ByteOrder::Big),
        Err(Error::AbsurdLength { .. })
    ));
    // Int array claiming 0xffff_ffff entries from a tiny buffer.
    let bad2 = [
        0x0a, 0x00, 0x00, 0x0b, 0x00, 0x01, 0x6e, 0xff, 0xff, 0xff, 0xff, 0x01,
    ];
    assert!(matches!(
        omt_nbt::read(&bad2, ByteOrder::Big),
        Err(Error::AbsurdLength { .. })
    ));
}

#[test]
fn trailing_data_detected_and_prefix_read_tolerates() {
    let bytes = omt_nbt::write(&demo_tree(), ByteOrder::Big).unwrap();
    let mut padded = bytes.clone();
    let garbage = b"garbage-from-region-file";
    padded.extend_from_slice(garbage);
    assert!(matches!(omt_nbt::read(&padded, ByteOrder::Big), Err(Error::TrailingData(n)) if n == garbage.len()));
    let (nbt, consumed) = omt_nbt::read_prefix(&padded, ByteOrder::Big).unwrap();
    assert_eq!(consumed, bytes.len());
    assert_eq!(nbt.tag, demo_tree().tag);
}

#[test]
fn writer_rejects_heterogeneous_lists() {
    let bad = Nbt {
        name: "".into(),
        tag: Tag::List(vec![Tag::Int(1), Tag::String("x".into())]),
    };
    assert!(matches!(
        omt_nbt::write(&bad, ByteOrder::Big),
        Err(Error::ListTypeMismatch { .. })
    ));
}

#[test]
fn modified_utf8_roundtrip() {
    for s in [
        "plain",
        "with \"quotes\" and \\backslash",
        "uniscroll ✓ ✗ ⌘ 你好",
        "emoji astral: 🎮🧱",
        "", " ",
    ] {
        let mut enc = Vec::new();
        string::encode_modified_utf8(s, &mut enc);
        assert_eq!(string::decode_modified_utf8(&enc), s, "s={s:?}");
        assert_eq!(string::encoded_len(s), enc.len());
        // NBT round-trip preserves it too.
        let nbt = Nbt { name: s.into(), tag: Tag::String(s.into()) };
        let bytes = omt_nbt::write(&nbt, ByteOrder::Big).unwrap();
        assert_eq!(omt_nbt::read(&bytes, ByteOrder::Big).unwrap(), nbt);
    }
}

#[test]
fn modified_utf8_null_and_lossy() {
    // NUL encodes as C0 80 (2 bytes) — the whole point of modified UTF-8.
    let s = "a\0b";
    let mut enc = Vec::new();
    string::encode_modified_utf8(s, &mut enc);
    assert_eq!(&enc, b"a\xc0\x80b");
    assert_eq!(string::decode_modified_utf8(&enc), s);

    // Invalid lead byte decodes lossily instead of failing.
    assert_eq!(string::decode_modified_utf8(&[0xff, 0xfe]), "\u{fffd}\u{fffd}");
    // Truncated 3-byte sequence at end of buffer.
    assert_eq!(string::decode_modified_utf8(&[0xe2, 0x9c]), "\u{fffd}");
}

// ---- seeded pseudo-random property test ---------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545f4914f6cdd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_tag(rng: &mut Rng, depth: u8) -> Tag {
    use Tag::*;
    let kind = if depth >= 4 { rng.below(8) } else { rng.below(12) };
    match kind {
        0 => Byte(rng.next() as i8),
        1 => Short(rng.next() as i16),
        2 => Int(rng.next() as i32),
        3 => Long(rng.next() as i64),
        4 => Float(f32::from_bits(rng.next() as u32)),
        5 => Double(f64::from_bits(rng.next())),
        6 => ByteArray((0..rng.below(16)).map(|_| rng.next() as i8).collect()),
        7 => {
            let n = rng.below(8) as usize;
            Tag::String(std::string::String::from_iter((0..n).map(|_| {
                char::from_u32(32 + (rng.below(0x10_ffff - 32) as u32)).unwrap_or('x')
            })))
        }
        8 => {
            // NBT lists are homogeneous: pick the element type once.
            let elem_kind = rng.below(11);
            let n = rng.below(5);
            let item = |rng: &mut Rng| match elem_kind {
                0 => Byte(rng.next() as i8),
                1 => Short(rng.next() as i16),
                2 => Int(rng.next() as i32),
                3 => Long(rng.next() as i64),
                4 => Float(f32::from_bits(rng.next() as u32)),
                5 => Double(f64::from_bits(rng.next())),
                6 => ByteArray((0..rng.below(16)).map(|_| rng.next() as i8).collect()),
                7 => IntArray((0..rng.below(12)).map(|_| rng.next() as i32).collect()),
                8 => LongArray((0..rng.below(12)).map(|_| rng.next() as i64).collect()),
                _ => {
                    let n = rng.below(5) as usize;
                    Compound((0..n).map(|i| (format!("k{i}"), random_tag(rng, depth + 1))).collect())
                }
            };
            List((0..n).map(|_| item(&mut *rng)).collect())
        }
        _ => Compound({
            let n = rng.below(5) as usize;
            (0..n)
                .map(|i| (format!("k{i}_{}", rng.below(1000)), random_tag(rng, depth + 1)))
                .collect()
        }),
    }
}

#[test]
fn seeded_random_roundtrips() {
    for seed in 1..200u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e3779b97f4a7c15));
        let nbt = Nbt { name: format!("root{seed}"), tag: random_tag(&mut rng, 0) };
        for order in [ByteOrder::Big, ByteOrder::Little] {
            let bytes = omt_nbt::write(&nbt, order).unwrap();
            assert_eq!(omt_nbt::read(&bytes, order).unwrap(), nbt, "seed={seed} order={order:?}");
        }
    }
}

#[test]
fn json_serialization_smoke() {
    let demo = demo_tree();
    let json = serde_json::to_value(&demo.tag).unwrap();
    assert_eq!(json["LevelName"], "Test's World ✓");
    assert_eq!(json["StorageVersion"], 10);
    assert_eq!(json["version"][2], 90);
}

/// Fuzz-smoke: mutated valid NBT must always produce a structured error or a
/// successful parse — never a panic. Run the long version with `cargo test
/// --release -- --ignored`; this is the seed for a real cargo-fuzz target.
#[test]
#[ignore = "long-running fuzz smoke; run explicitly"]
fn fuzz_smoke_mutations() {
    let base = omt_nbt::write(&demo_tree(), ByteOrder::Big).unwrap();
    let mut rng = Rng(0xdecafbad1234);
    for _ in 0..200_000 {
        let mut buf = base.clone();
        let mutations = 1 + rng.below(4) as usize;
        for _ in 0..mutations {
            let idx = rng.below(buf.len() as u64) as usize;
            buf[idx] = rng.next() as u8;
        }
        let _ = omt_nbt::read(&buf, ByteOrder::Big);
        let _ = omt_nbt::read(&buf, ByteOrder::Little);
    }
}

#[test]
fn fuzz_smoke_short() {
    let base = omt_nbt::write(&demo_tree(), ByteOrder::Big).unwrap();
    let mut rng = Rng(0xfeedface);
    for _ in 0..2_000 {
        let mut buf = base.clone();
        for _ in 0..1 + rng.below(4) {
            let idx = rng.below(buf.len() as u64) as usize;
            buf[idx] = rng.next() as u8;
        }
        let _ = omt_nbt::read(&buf, ByteOrder::Big);
        let _ = omt_nbt::read(&buf, ByteOrder::Little);
    }
}

// ---- M2: SNBT parser ------------------------------------------------------

#[test]
fn snbt_scalars_roundtrip() {
    for tag in [
        Tag::Byte(-3), Tag::Short(7), Tag::Int(42), Tag::Long(-9),
        Tag::Float(1.5), Tag::Double(2.25), Tag::String("hi".into()),
        Tag::String("with \"quotes\" and \\n newline".into()),
        Tag::ByteArray(vec![1, -2, 3]), Tag::IntArray(vec![-1, 0, 255]),
        Tag::LongArray(vec![0, i64::MAX]), Tag::List(vec![]),
        Tag::List(vec![Tag::Int(1), Tag::Int(2)]),
    ] {
        let snbt = omt_nbt::snbt::to_snbt(&tag);
        let back = omt_nbt::snbt::from_snbt(&snbt).unwrap_or_else(|e| panic!("{snbt}: {e}"));
        assert_eq!(back, tag, "via {snbt:?}");
    }
}

#[test]
fn snbt_nested_compound_roundtrip() {
    let tag = Tag::Compound(vec![
        ("plain".into(), Tag::Int(1)),
        ("with space".into(), Tag::String("v".into())),
        ("list".into(), Tag::List(vec![Tag::Int(1), Tag::Int(2)])),
        ("nested".into(), Tag::Compound(vec![
            ("deep".into(), Tag::List(vec![
                Tag::Compound(vec![("a".into(), Tag::Byte(1))]),
                Tag::Compound(vec![("a".into(), Tag::Byte(2))]),
            ])),
        ])),
    ]);
    let snbt = omt_nbt::snbt::to_snbt(&tag);
    assert_eq!(omt_nbt::snbt::from_snbt(&snbt).unwrap(), tag);
}

#[test]
fn snbt_accepts_vanilla_syntax() {
    use omt_nbt::snbt::from_snbt;
    assert_eq!(from_snbt("1b").unwrap(), Tag::Byte(1));
    assert_eq!(from_snbt("true").unwrap(), Tag::Byte(1));
    assert_eq!(from_snbt("false").unwrap(), Tag::Byte(0));
    assert_eq!(from_snbt("-3s").unwrap(), Tag::Short(-3));
    assert_eq!(from_snbt("12345678901L").unwrap(), Tag::Long(12345678901));
    assert_eq!(from_snbt("1.5f").unwrap(), Tag::Float(1.5));
    assert_eq!(from_snbt("2.0d").unwrap(), Tag::Double(2.0));
    assert_eq!(from_snbt("bare_word").unwrap(), Tag::String("bare_word".into()));
    assert_eq!(from_snbt("'single quoted'").unwrap(), Tag::String("single quoted".into()));
    assert_eq!(from_snbt("[I; 1, 2, 3]").unwrap(), Tag::IntArray(vec![1, 2, 3]));
    assert_eq!(from_snbt("[B;]").unwrap(), Tag::ByteArray(vec![]));
    assert_eq!(from_snbt("[L; -1]").unwrap(), Tag::LongArray(vec![-1]));
    // Bare unsuffixed int → Int.
    assert_eq!(from_snbt("42").unwrap(), Tag::Int(42));
    // Duplicate keys: last wins.
    assert_eq!(
        from_snbt("{a:1,a:2}").unwrap(),
        Tag::Compound(vec![("a".into(), Tag::Int(2))])
    );
}

#[test]
fn snbt_rejects_malformed() {
    use omt_nbt::snbt::from_snbt;
    for bad in ["{a:1", "[1, 2", "\"unterminated", "{a 1}", "[I; 1, x]", "1 2"] {
        assert!(from_snbt(bad).is_err(), "should reject {bad:?}");
    }
}

#[test]
fn snbt_random_tree_roundtrip() {
    // Reuse the seeded generator at the top of this file.
    for seed in 1..50u64 {
        let mut rng = Rng(seed.wrapping_mul(0x1234_5678_9abc_def1));
        let tag = random_tag(&mut rng, 0);
        // Skip trees with empty non-first lists (SNBT cannot carry their
        // declared element type; empty lists re-parse as empty lists, which
        // still compares equal because the model has no element-type field).
        let snbt = omt_nbt::snbt::to_snbt(&tag);
        let back = omt_nbt::snbt::from_snbt(&snbt)
            .unwrap_or_else(|e| panic!("seed {seed} via {snbt}: {e}"));
        assert_eq!(back, tag, "seed {seed}");
    }
}

#[test]
fn snbt_fixture_level_dat_roundtrip() {
    // Full-document fidelity: parse the real fixture level.dat, render SNBT,
    // re-parse, re-serialize — bytes must match a pure re-serialize.
    let path = std::path::PathBuf::from(
        std::env::var("OMT_BEDROCK_FIXTURE")
            .unwrap_or_else(|_| "/mnt/dev/Dev/Reverse_Engineering/UMT_Crack/world_ref/e1j7l0I-l68=".into()),
    )
    .join("level.dat");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipped: fixture level.dat unavailable");
        return;
    };
    let ld = omt_nbt::BedrockLevelDat::parse(&bytes).unwrap();
    let snbt = omt_nbt::snbt::to_snbt(&ld.nbt.tag);
    let back = omt_nbt::snbt::from_snbt(&snbt).unwrap();
    let rebuilt = omt_nbt::BedrockLevelDat {
        storage_version: ld.storage_version,
        nbt: omt_nbt::Nbt { name: ld.nbt.name.clone(), tag: back },
    };
    assert_eq!(rebuilt.serialize().unwrap(), ld.serialize().unwrap());
}

#[test]
fn fixture_level_dat_edit_save_reload() {
    let src = std::path::PathBuf::from(
        std::env::var("OMT_BEDROCK_FIXTURE")
            .unwrap_or_else(|_| "/mnt/dev/Dev/Reverse_Engineering/UMT_Crack/world_ref/e1j7l0I-l68=".into()),
    )
    .join("level.dat");
    let Ok(original) = std::fs::read(&src) else {
        eprintln!("skipped: fixture level.dat unavailable");
        return;
    };
    let dir = std::env::temp_dir().join(format!("omt-m2-edit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("level.dat");
    std::fs::write(&target, &original).unwrap();

    // Edit keepinventory gamerule through the engine, save, reload.
    use omt_nbt::{EditOp, NbtDocument, TagPath, UndoStack};
    use std::str::FromStr;
    let mut doc = NbtDocument::load(&target).unwrap();
    assert_eq!(doc.format, omt_nbt::NbtFormat::BedrockLevelDat);
    let mut undo = UndoStack::default();
    let path = TagPath::from_str("keepinventory").unwrap();
    undo.edit(doc.root_mut(), |r| EditOp::prepare_set(r, &path, Tag::Byte(1))).unwrap();
    let backup = doc.save().unwrap().expect("first save backs up");
    assert_eq!(std::fs::read(&backup).unwrap(), original, "backup is pristine");

    let re = NbtDocument::load(&target).unwrap();
    assert_eq!(re.nbt.tag.get("keepinventory"), Some(&Tag::Byte(1)));
    assert_eq!(re.bedrock_storage_version, doc.bedrock_storage_version);
    assert_eq!(re.nbt.tag.get("LevelName"), doc.nbt.tag.get("LevelName"));

    // Undo restores the tree in memory; a save-after-undo matches the
    // original byte-for-byte (same writer, same input).
    undo.undo(doc.root_mut()).unwrap();
    assert_eq!(doc.nbt.tag.get("keepinventory"), Some(&Tag::Byte(0)));
    std::fs::remove_dir_all(&dir).unwrap();
}
