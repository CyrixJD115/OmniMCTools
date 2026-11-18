//! Tests of `items` (kept apart: per-file budget < 1000 lines).
use super::*;
use omt_nbt::snbt::{from_snbt, to_snbt};

fn conv_to_bedrock(snbt: &str) -> (Tag, ConversionReport) {
    let tag = from_snbt(snbt).unwrap();
    java_stack_to_bedrock(&tag, &ItemConvertOpts::default()).unwrap()
}

fn conv_to_java(snbt: &str, era: StackEra) -> (Tag, ConversionReport) {
    let tag = from_snbt(snbt).unwrap();
    bedrock_stack_to_java(&tag, era, &ItemConvertOpts::default()).unwrap()
}

// ---- structural core -------------------------------------------------

#[test]
fn plain_components_stack_maps_top_level_fields() {
    let (out, report) = conv_to_bedrock(
        "{id: \"minecraft:apple\", count: 3, components: {\"minecraft:damage\": 2}}",
    );
    assert_eq!(out.get("Name").and_then(Tag::as_str), Some("minecraft:apple"));
    assert_eq!(out.get("Count"), Some(&Tag::Byte(3)));
    assert_eq!(out.get("Damage"), Some(&Tag::Short(2)));
    assert_eq!(out.get("WasPickedUp"), Some(&Tag::Byte(0)));
    assert!(report.is_clean(), "unexpected losses: {report}");
}

#[test]
fn legacy_stack_count_damage_and_slot() {
    let (out, report) =
        conv_to_bedrock("{id: \"minecraft:diamond_sword\", Count: 1b, Damage: 40}");
    assert_eq!(out.get("Count"), Some(&Tag::Byte(1)));
    assert_eq!(out.get("Damage"), Some(&Tag::Short(40)));
    assert!(report.is_clean(), "{report}");
}

#[test]
fn missing_count_defaults_to_one() {
    let (out, _) = conv_to_bedrock("{id: \"minecraft:stick\"}");
    assert_eq!(out.get("Count"), Some(&Tag::Byte(1)));
}

#[test]
fn count_coercion_is_reported() {
    // 300 does not fit a Bedrock Byte.
    let (out, report) = conv_to_bedrock("{id: \"minecraft:apple\", count: 300}");
    assert_eq!(out.get("Count"), Some(&Tag::Byte(127)));
    assert_eq!(report.count_with_kind(LossKind::ValueCoerced), 1);
}

#[test]
fn errors_are_structured_not_panics() {
    assert!(matches!(
        java_stack_to_bedrock(&Tag::Int(3), &ItemConvertOpts::default()),
        Err(ConvertError::NotAStack(_))
    ));
    let no_id = from_snbt("{count: 1}").unwrap();
    assert!(matches!(
        java_stack_to_bedrock(&no_id, &ItemConvertOpts::default()),
        Err(ConvertError::MissingItemId)
    ));
    let bad_count = from_snbt("{id: \"minecraft:x\", count: \"three\"}").unwrap();
    assert!(matches!(
        java_stack_to_bedrock(&bad_count, &ItemConvertOpts::default()),
        Err(ConvertError::BadField { .. })
    ));
}

// ---- findings loss rows ----------------------------------------------

#[test]
fn loss_row_1_unknown_components_dropped_and_reported() {
    let (out, report) = conv_to_bedrock(
        "{id: \"minecraft:apple\", count: 1, components: {\
          \"minecraft:custom_name\": '\"Golden\"', \
          \"minecraft:fire_resistance\": {}, \
          \"minecraft:intangible_projectile\": {}, \
          \"minecraft:damage\": 0}}",
    );
    // Unknown components must not appear on the Bedrock side…
    assert!(out.get("fire_resistance").is_none());
    // …and each is an explicit ledger row.
    assert_eq!(report.count_with_kind(LossKind::UnsupportedComponent), 2, "{report}");
    assert!(report.to_string().contains("no Bedrock mapping"));
    // The mapped sibling survives.
    assert_eq!(
        out.get("display").and_then(|d| d.get("Name")).and_then(Tag::as_str),
        Some("\"Golden\"")
    );
}

#[test]
fn loss_row_2_was_picked_up_dropped_and_reported() {
    // Exact Bedrock shape from the 1.26.52 fixture (vault key_item).
    // The default `WasPickedUp: 0b` carries no information — it drops
    // without a ledger row so every converted stack isn't flagged.
    let (out, report) = conv_to_java(
        "{Name: \"minecraft:trial_key\", Count: 1b, Damage: 2s, WasPickedUp: 0b}",
        StackEra::Components,
    );
    assert_eq!(out.get("id").and_then(Tag::as_str), Some("minecraft:trial_key"));
    assert_eq!(out.get("count"), Some(&Tag::Int(1)));
    assert_eq!(
        out.get("components").and_then(|c| c.get("minecraft:damage")),
        Some(&Tag::Int(2))
    );
    assert!(out.get("WasPickedUp").is_none());
    assert!(report.is_clean(), "{report}");

    // A non-zero picked-up marker *is* state being lost — reported.
    let (_, report) = conv_to_java(
        "{Name: \"minecraft:trial_key\", Count: 1b, WasPickedUp: 1b}",
        StackEra::Components,
    );
    assert_eq!(report.count_with_kind(LossKind::FieldDropped), 1);
    assert!(report.to_string().contains("WasPickedUp"));
}

#[test]
fn loss_row_3_mixed_era_resolves_by_policy() {
    let mixed = "{id: \"minecraft:book\", count: 1, tag: {display: {Name: \"old\"}}, \
                 components: {\"minecraft:custom_name\": \"new\"}}";

    let (out, report) = {
        let tag = from_snbt(mixed).unwrap();
        java_stack_to_bedrock(&tag, &ItemConvertOpts::default()).unwrap()
    };
    assert_eq!(
        out.get("display").and_then(|d| d.get("Name")).and_then(Tag::as_str),
        Some("new"),
        "PreferComponents keeps the components-era name"
    );
    assert_eq!(report.count_with_kind(LossKind::MixedEra), 1);

    let prefer_tag = ItemConvertOpts {
        mixed_era: MixedEraPolicy::PreferTag,
        ..Default::default()
    };
    let (out, report) = {
        let tag = from_snbt(mixed).unwrap();
        java_stack_to_bedrock(&tag, &prefer_tag).unwrap()
    };
    assert_eq!(
        out.get("display").and_then(|d| d.get("Name")).and_then(Tag::as_str),
        Some("old")
    );
    assert_eq!(report.count_with_kind(LossKind::MixedEra), 1);

    let reject = ItemConvertOpts {
        mixed_era: MixedEraPolicy::Reject,
        ..Default::default()
    };
    let tag = from_snbt(mixed).unwrap();
    assert!(matches!(
        java_stack_to_bedrock(&tag, &reject),
        Err(ConvertError::MixedEraStack)
    ));
}

// ---- enchantments ----------------------------------------------------

#[test]
fn enchantments_map_through_the_id_table() {
    let opts = ItemConvertOpts {
        enchant_ids: EnchantIdMap::from_pairs([
            ("minecraft:sharpness".to_string(), 9),
            ("minecraft:protection".to_string(), 0),
        ]),
        ..Default::default()
    };
    let java = from_snbt(
        "{id: \"minecraft:diamond_sword\", count: 1, components: {\
          \"minecraft:enchantments\": {levels: {\"minecraft:sharpness\": 5, \"minecraft:protection\": 2}}}}",
    )
    .unwrap();
    let (bedrock, report) = java_stack_to_bedrock(&java, &opts).unwrap();
    let ench = bedrock.get("ench").and_then(Tag::as_list).unwrap();
    assert_eq!(ench.len(), 2);
    assert_eq!(ench[0].get("id"), Some(&Tag::Short(9)));
    assert_eq!(ench[0].get("lvl"), Some(&Tag::Short(5)));
    assert!(report.is_clean(), "{report}");

    // Round-trip back through the same table.
    let (java2, report2) = bedrock_stack_to_java(&bedrock, StackEra::Components, &opts).unwrap();
    assert!(report2.is_clean(), "{report2}");
    let levels = java2
        .get("components")
        .and_then(|c| c.get("minecraft:enchantments"))
        .and_then(|e| e.get("levels"))
        .unwrap();
    assert_eq!(levels.get("minecraft:sharpness"), Some(&Tag::Int(5)));
}

#[test]
fn unmapped_enchant_is_a_loss_not_a_guess() {
    let (out, report) = conv_to_bedrock(
        "{id: \"minecraft:pickaxe\", count: 1, components: {\
          \"minecraft:enchantments\": {levels: {\"minecraft:efficiency\": 3}}}}",
    );
    assert!(
        out.get("ench").is_none(),
        "unmapped enchant must not be fabricated"
    );
    assert_eq!(report.count_with_kind(LossKind::UnmappedId), 1);
    assert!(report.to_string().contains("minecraft:efficiency"));
}

#[test]
fn legacy_enchantments_list_shape() {
    let (out, report) = conv_to_bedrock(
        "{id: \"minecraft:bow\", Count: 1b, tag: {Enchantments: [{id: \"minecraft:power\", lvl: 4s}]}}",
    );
    // No id table by default -> unmapped loss, structure still verified.
    assert_eq!(report.count_with_kind(LossKind::UnmappedId), 1);
    assert!(out.get("ench").is_none());
}

// ---- custom name / lore ------------------------------------------------

#[test]
fn custom_name_and_lore_round_trip_losslessly() {
    let name = "{\"text\":\"Excalibur\",\"italic\":false}";
    let lore_line = "{\"text\":\"of legend\"}";
    let java = from_snbt(&format!(
        "{{id: \"minecraft:diamond_sword\", count: 1, components: {{\
          \"minecraft:custom_name\": '{name}', \
          \"minecraft:lore\": ['{lore_line}']}}}}"
    ))
    .unwrap();
    let (bedrock, r1) = java_stack_to_bedrock(&java, &ItemConvertOpts::default()).unwrap();
    assert!(r1.is_clean(), "{r1}");
    let display = bedrock.get("display").unwrap();
    assert_eq!(display.get("Name").and_then(Tag::as_str), Some(name));
    assert_eq!(display.get("Lore").unwrap().as_list().unwrap().len(), 1);

    let (java2, r2) =
        bedrock_stack_to_java(&bedrock, StackEra::Components, &ItemConvertOpts::default())
            .unwrap();
    assert!(r2.is_clean(), "{r2}");
    let components = java2.get("components").unwrap();
    assert_eq!(
        components.get("minecraft:custom_name").and_then(Tag::as_str),
        Some(name)
    );
    assert_eq!(
        components
            .get("minecraft:lore")
            .and_then(Tag::as_list)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn legacy_display_name_and_lore_to_bedrock() {
    // The name carries a JSON text payload (as on real Java worlds);
    // SNBT quotes make the JSON's own quotes `\\\"` inside the literal.
    let (out, report) = conv_to_bedrock(
        "{id: \"minecraft:chest\", Count: 1b, tag: {display: {Name: \"Quest Marker\", Lore: [\"line one\"]}}}",
    );
    assert_eq!(
        out.get("display").and_then(|d| d.get("Name")).and_then(Tag::as_str),
        Some("Quest Marker")
    );
    assert_eq!(
        out.get("display")
            .and_then(|d| d.get("Lore"))
            .and_then(Tag::as_list)
            .unwrap()
            .len(),
        1
    );
    assert!(report.is_clean(), "{report}");
}

// ---- books -------------------------------------------------------------

#[test]
fn written_book_content_maps_pages_title_author() {
    let (bedrock, report) = conv_to_bedrock(
        "{id: \"minecraft:written_book\", count: 1, components: {\
          \"minecraft:written_book_content\": {\
            title: \"Diary\", author: \"Steve\", \
            pages: [{raw: \"page one\", filtered: \"page one\"}, {raw: \"page two\"}]}}}",
    );
    assert_eq!(bedrock.get("title").and_then(Tag::as_str), Some("Diary"));
    assert_eq!(bedrock.get("author").and_then(Tag::as_str), Some("Steve"));
    let pages = bedrock.get("pages").and_then(Tag::as_list).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].get("text").and_then(Tag::as_str), Some("page one"));
    // The filtered variant is an explicit loss (Bedrock has one text
    // per page).
    assert_eq!(report.count_with_kind(LossKind::FieldDropped), 1);
    assert!(report.to_string().contains("filtered"));

    // …and the surviving text round-trips back into a components-era
    // Java book.
    let (java, r2) =
        bedrock_stack_to_java(&bedrock, StackEra::Components, &ItemConvertOpts::default())
            .unwrap();
    assert!(r2.is_clean(), "{r2}");
    let content = java
        .get("components")
        .and_then(|c| c.get("minecraft:written_book_content"))
        .unwrap();
    assert_eq!(content.get("title").and_then(Tag::as_str), Some("Diary"));
    let pages = content.get("pages").and_then(Tag::as_list).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].get("raw").and_then(Tag::as_str), Some("page two"));
}

#[test]
fn legacy_book_pages_to_bedrock() {
    let (out, report) = conv_to_bedrock(
        "{id: \"minecraft:writable_book\", Count: 1b, \
          tag: {pages: [\"a\", \"b\"], title: \"Notes\", author: \"Alex\"}}",
    );
    let pages = out.get("pages").and_then(Tag::as_list).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].get("text").and_then(Tag::as_str), Some("a"));
    assert!(report.is_clean(), "{report}");
}

// ---- slot --------------------------------------------------------------

#[test]
fn java_slot_is_reported_when_converting_to_bedrock() {
    let (_, report) =
        conv_to_bedrock("{id: \"minecraft:apple\", count: 2, Slot: 5b}");
    assert_eq!(report.count_with_kind(LossKind::FieldDropped), 1);
    assert!(report.to_string().contains("Slot"));
}

// ---- round-trip smoke ----------------------------------------------------

#[test]
fn plain_stack_round_trips_both_directions() {
    let java = from_snbt("{id: \"minecraft:stone\", count: 64}").unwrap();
    let (bedrock, r1) = java_stack_to_bedrock(&java, &ItemConvertOpts::default()).unwrap();
    assert!(r1.is_clean());
    let (java2, r2) =
        bedrock_stack_to_java(&bedrock, StackEra::Components, &ItemConvertOpts::default())
            .unwrap();
    assert!(r2.is_clean(), "{r2}");
    assert_eq!(java2.get("id").and_then(Tag::as_str), Some("minecraft:stone"));
    assert_eq!(java2.get("count"), Some(&Tag::Int(64)));
    assert!(java2.get("components").is_none());
    let snbt = to_snbt(&java2);
    assert!(snbt.contains("id:\"minecraft:stone\""), "{snbt}");
}
