//! Property tests (§4 quality gates): arbitrary valid `Tag` trees must
//! round-trip exactly through the binary codec (BE **and** LE) and through
//! SNBT render→parse.
//!
//! Known gotchas pinned as dedicated cases: signed `ByteArray` (i8), i64
//! MIN/MAX, float shortest-round-trip (subnormals, infinities, ±0.0 — NaN is
//! excluded because NaN != NaN makes equality meaningless).
//!
//! SNBT expressibility constraints honored by the strategies: compound keys
//! are unique per compound (the parser upserts duplicates), and floats are
//! non-NaN (the renderer emits `NaNf`, which parses back but never compares
//! equal).

use std::collections::HashSet;

use proptest::prelude::*;
use omt_nbt::{ByteOrder, Nbt, Tag};

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn finite_f32() -> impl Strategy<Value = f32> {
    any::<f32>().prop_filter("non-NaN", |f| !f.is_nan())
}

fn finite_f64() -> impl Strategy<Value = f64> {
    any::<f64>().prop_filter("non-NaN", |f| !f.is_nan())
}

/// Compound keys: non-empty, mostly unquoted-friendly, occasionally needing
/// quotes (spaces/punctuation) to exercise the key quoting path.
fn key() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[a-zA-Z_][a-zA-Z0-9_]{0,10}",
        1 => "[a-zA-Z0-9_ .:,/+\\-]{0,12}",
    ]
}

/// Arbitrary-ish string values: exercises quoting/escaping (quotes,
/// backslashes, newlines) without lone-surrogate hazards.
fn string_val() -> impl Strategy<Value = Tag> {
    prop_oneof![
        2 => "[a-zA-Z0-9_ .:,/+\\-]{0,16}".prop_map(Tag::String),
        1 => ("\\\\|\\\"|\\n|\\t|x", 0u8..8).prop_map(|(_, n)| Tag::String(format!("q{n}\"\\\n\tend"))),
        1 => Just(Tag::String(String::new())),
    ]
}

fn scalar() -> impl Strategy<Value = Tag> {
    prop_oneof![
        3 => any::<i8>().prop_map(Tag::Byte),
        2 => any::<i16>().prop_map(Tag::Short),
        3 => any::<i32>().prop_map(Tag::Int),
        3 => any::<i64>().prop_map(Tag::Long),
        1 => Just(Tag::Byte(i8::MIN)),
        1 => Just(Tag::Long(i64::MAX)),
        1 => Just(Tag::Long(i64::MIN)),
        3 => finite_f32().prop_map(Tag::Float),
        3 => finite_f64().prop_map(Tag::Double),
        1 => Just(Tag::Float(f32::MIN_POSITIVE)),
        1 => Just(Tag::Double(f64::MIN_POSITIVE)),
        1 => Just(Tag::Float(f32::NEG_INFINITY)),
        1 => Just(Tag::Float(f32::INFINITY)),
        1 => Just(Tag::Double(-0.0)),
        2 => string_val(),
        2 => proptest::collection::vec(any::<i8>(), 0..16).prop_map(Tag::ByteArray),
        1 => proptest::collection::vec(any::<i32>(), 0..12).prop_map(Tag::IntArray),
        1 => proptest::collection::vec(any::<i64>(), 0..12).prop_map(Tag::LongArray),
    ]
}

/// One *concrete* scalar variant as a boxed strategy — the building block
/// for homogeneous lists (the binary format declares a single element type
/// id per list, so mixed-type lists are unrepresentable).
fn scalar_variant() -> impl Strategy<Value = BoxedStrategy<Tag>> {
    prop_oneof![
        Just(any::<i8>().prop_map(Tag::Byte).boxed()),
        Just(any::<i16>().prop_map(Tag::Short).boxed()),
        Just(any::<i32>().prop_map(Tag::Int).boxed()),
        Just(any::<i64>().prop_map(Tag::Long).boxed()),
        Just(finite_f32().prop_map(Tag::Float).boxed()),
        Just(finite_f64().prop_map(Tag::Double).boxed()),
        Just("[a-zA-Z0-9_ .:,/+\\-]{0,16}".prop_map(Tag::String).boxed()),
        Just(proptest::collection::vec(any::<i8>(), 0..8).prop_map(Tag::ByteArray).boxed()),
    ]
}

/// Remove later entries whose key duplicates an earlier one (the SNBT parser
/// upserts duplicate keys, so duplicates are not round-trippable trees).
fn dedupe(entries: Vec<(String, Tag)>) -> Vec<(String, Tag)> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter(|(k, _)| seen.insert(k.clone()))
        .collect()
}

/// A `Tag::List` whose elements all share one type id: pick the element
/// *kind* once (a concrete scalar variant, or the recursive List/Compound
/// shape), then vec over that single strategy. `depth` bounds nesting.
fn list_strategy(depth: u32) -> BoxedStrategy<Tag> {
    if depth == 0 {
        // Base case: scalar-typed lists only (no recursion while building).
        return scalar_variant()
            .prop_flat_map(|elem| proptest::collection::vec(elem, 0..6))
            .prop_map(Tag::List)
            .boxed();
    }
    let elem_kind = prop_oneof![
        5 => scalar_variant(),
        1 => Just(list_strategy(depth - 1).boxed()),
        1 => Just(compound_strategy(depth - 1).boxed()),
    ];
    elem_kind
        .prop_flat_map(|elem| proptest::collection::vec(elem, 0..6))
        .prop_map(Tag::List)
        .boxed()
}

/// A `Tag::Compound` with unique keys; values may be any tag of bounded depth
/// (compound values are keyed, so mixed value types are representable).
fn compound_strategy(depth: u32) -> BoxedStrategy<Tag> {
    let value = if depth == 0 {
        scalar().boxed()
    } else {
        prop_oneof![
            4 => scalar(),
            1 => list_strategy(depth - 1),
            1 => compound_strategy(depth - 1),
        ]
        .boxed()
    };
    proptest::collection::vec((key(), value), 0..6)
        .prop_map(|entries| Tag::Compound(dedupe(entries)))
        .boxed()
}

fn tag() -> impl Strategy<Value = Tag> {
    prop_oneof![
        4 => scalar(),
        1 => list_strategy(3),
        2 => compound_strategy(3),
    ]
}

fn nbt() -> impl Strategy<Value = Nbt> {
    (key(), tag()).prop_map(|(name, tag)| Nbt { name, tag })
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    /// write → read must reproduce the tree exactly, both byte orders.
    #[test]
    fn binary_roundtrip_be_le(nbt in nbt()) {
        for order in [ByteOrder::Big, ByteOrder::Little] {
            let bytes = omt_nbt::write(&nbt, order).unwrap();
            let back = omt_nbt::read(&bytes, order).unwrap();
            prop_assert_eq!(&back, &nbt);
        }
    }

    /// SNBT render → parse must reproduce every tree SNBT can express.
    #[test]
    fn snbt_render_parse_roundtrip(nbt in nbt()) {
        let text = omt_nbt::snbt::to_snbt(&nbt.tag);
        let back = omt_nbt::snbt::from_snbt(&text)
            .unwrap_or_else(|e| panic!("re-parse of {text:?} failed: {e}"));
        prop_assert_eq!(back, nbt.tag);
    }
}

// ---------------------------------------------------------------------------
// Pinned gotcha cases (deterministic, run on every `cargo test`)
// ---------------------------------------------------------------------------

fn gotcha_tags() -> Vec<Tag> {
    vec![
        Tag::Byte(-1),
        Tag::Byte(i8::MIN),
        Tag::Short(i16::MIN),
        Tag::Int(i32::MIN),
        Tag::Long(i64::MAX),
        Tag::Long(i64::MIN),
        // signed ByteArray: negative byte values survive both directions
        Tag::ByteArray(vec![i8::MIN, -1, 0, 1, i8::MAX]),
        Tag::ByteArray(Vec::new()),
        Tag::IntArray(vec![i32::MIN, i32::MAX]),
        Tag::LongArray(vec![i64::MIN, i64::MAX]),
        // float shortest-round-trip: values whose decimal form must recover
        // the exact same bits (subnormals, huge, tiny, ±0.0)
        Tag::Float(0.1),
        Tag::Float(f32::MIN_POSITIVE),
        Tag::Float(f32::MAX),
        Tag::Float(f32::INFINITY),
        Tag::Float(f32::NEG_INFINITY),
        Tag::Float(-0.0),
        Tag::Double(0.1),
        Tag::Double(f64::MIN_POSITIVE),
        Tag::Double(f64::MAX),
        Tag::Double(-0.0),
    ]
}

#[test]
fn gotchas_binary_roundtrip() {
    for tag in gotcha_tags() {
        for order in [ByteOrder::Big, ByteOrder::Little] {
            let nbt = Nbt { name: "gotcha".into(), tag: tag.clone() };
            let bytes = omt_nbt::write(&nbt, order).unwrap();
            assert_eq!(omt_nbt::read(&bytes, order).unwrap(), nbt, "{tag:?} via {order:?}");
        }
    }
}

#[test]
fn gotchas_snbt_roundtrip() {
    for tag in gotcha_tags() {
        let text = omt_nbt::snbt::to_snbt(&tag);
        let back = omt_nbt::snbt::from_snbt(&text)
            .unwrap_or_else(|e| panic!("re-parse of {text:?} failed: {e}"));
        assert_eq!(back, tag, "via {text:?}");
    }
}

/// Empty and single-entry containers, plus the depth-bound nesting that
/// mirrors what the fuzz target exercises (MAX_DEPTH is 512; we stay shallow).
#[test]
fn container_edges_roundtrip() {
    let nbt = Nbt {
        name: String::new(),
        tag: Tag::Compound(vec![
            ("empty_list".into(), Tag::List(Vec::new())),
            ("one_list".into(), Tag::List(vec![Tag::Int(7)])),
            ("homogeneous_list_of_lists".into(), Tag::List(vec![
                Tag::List(vec![Tag::Int(1)]),
                Tag::List(vec![]),
                Tag::List(vec![Tag::Int(2), Tag::Int(3)]),
            ])),
            ("list_of_compounds".into(), Tag::List(vec![
                Tag::Compound(vec![("a".into(), Tag::Int(1))]),
                Tag::Compound(vec![("b".into(), Tag::String("x".into()))]),
            ])),
            ("empty_compound".into(), Tag::Compound(Vec::new())),
            ("empty_barr".into(), Tag::ByteArray(Vec::new())),
            ("deep".into(), (0..16).fold(Tag::Int(1), |t, _| Tag::List(vec![t]))),
        ]),
    };
    for order in [ByteOrder::Big, ByteOrder::Little] {
        let bytes = omt_nbt::write(&nbt, order).unwrap();
        assert_eq!(omt_nbt::read(&bytes, order).unwrap(), nbt, "via {order:?}");
    }
    let text = omt_nbt::snbt::to_snbt(&nbt.tag);
    let back = omt_nbt::snbt::from_snbt(&text).unwrap();
    assert_eq!(back, nbt.tag);
}
