//! Java ⇄ Bedrock item stack conversion (findings Task A).
//!
//! UMT's own item mappings are *data* in its downloaded `database.ndb`
//! (the binary contains zero item-key literals), so nothing here is
//! UMT-verified line-by-line. What the findings *do* verify, and what
//! this module therefore guarantees:
//!
//! 1. The pipeline shape — parse → opaque passthrough → name-keyed
//!    transforms → **explicit loss ledger** (nothing silently dropped).
//! 2. The permanent loss rows (findings "Data-loss ledger"):
//!    Java→Bedrock drops unmapped components (reported); Bedrock→Java
//!    drops `WasPickedUp` and other Bedrock-only fields (reported);
//!    mixed `tag`+`components` stacks resolve per policy (reported).
//!
//! The Bedrock wire shape is grounded in the 1.26.52 fixtures (block
//! entities carry `{Name: String, Count: Byte, Damage: Short,
//! WasPickedUp: Byte}` even for plain items). The sub-compound shapes we
//! could not observe in fixtures (`display`, `ench`, book pages) are
//! marked **OMT design decision** below and must be revisited when a
//! fixture with named/enchanted/book items exists (PLAN §8 fixture
//! checklist).
//!
//! Id *values* (item ids, enchantment numeric ids) pass through
//! unmapped: real mapping tables arrive with the M3 registry generator
//! (`tools/registry-gen`), which replaces UMT's `database.ndb`. Until
//! then unmappable ids become [`LossKind::UnmappedId`] rows.

use std::collections::HashMap;

use omt_nbt::Tag;

use crate::error::{ConvertError, Result};
use crate::loss::{ConversionReport, LossKind};

/// Java item-stack format generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StackEra {
    /// ≤ 1.20.4: `Count` (Byte), `Damage` (Int), `tag` compound
    /// (`display.Name`, `display.Lore`, `Enchantments`/`ench`, book
    /// fields, …).
    Legacy,
    /// ≥ 1.20.5: `count` (Int) + `components` compound keyed by
    /// namespaced component ids (`minecraft:custom_name`, …).
    Components,
}

/// What to do when a stack carries **both** `tag` and `components`
/// (mixed-era worlds are the norm after in-game upgrades; UMT ships a
/// `mixedComp` option for exactly this — findings Task A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MixedEraPolicy {
    /// Use `components`, drop `tag` (reported as [`LossKind::MixedEra`]).
    /// Default: components are the current-format source of truth.
    #[default]
    PreferComponents,
    /// Use `tag`, drop `components` (reported).
    PreferTag,
    /// Fail with [`ConvertError::MixedEraStack`].
    Reject,
}

/// Java⇄Bedrock enchantment-id table. Deliberately **empty** by default:
/// the real table is registry data (M3 `tools/registry-gen`), and
/// guessing numeric ids would violate the "never silently wrong" rule —
/// unmapped enchantments are dropped with [`LossKind::UnmappedId`].
#[derive(Debug, Clone, Default)]
pub struct EnchantIdMap {
    java_to_bedrock: HashMap<String, i16>,
    bedrock_to_java: HashMap<i16, String>,
}

impl EnchantIdMap {
    /// An empty map (everything unmapped). The M3 registry will provide
    /// the populated one.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from `(java_id, bedrock_numeric_id)` pairs (both directions).
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, i16)>) -> Self {
        let mut map = Self::default();
        for (java, bedrock) in pairs {
            map.bedrock_to_java.insert(bedrock, java.clone());
            map.java_to_bedrock.insert(java, bedrock);
        }
        map
    }

    pub fn java_to_bedrock(&self, java: &str) -> Option<i16> {
        self.java_to_bedrock.get(java).copied()
    }

    pub fn bedrock_to_java(&self, bedrock: i16) -> Option<String> {
        self.bedrock_to_java.get(&bedrock).cloned()
    }
}

/// Options for one conversion.
#[derive(Debug, Clone)]
pub struct ItemConvertOpts {
    pub mixed_era: MixedEraPolicy,
    pub enchant_ids: EnchantIdMap,
}

impl Default for ItemConvertOpts {
    fn default() -> Self {
        Self {
            mixed_era: MixedEraPolicy::default(),
            enchant_ids: EnchantIdMap::empty(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal model
// ---------------------------------------------------------------------------

/// Parsed, edition-neutral stack. Everything we have no rule for stays
/// in `passthrough` and is re-emitted on the target side only if the
/// target edition has a slot for it — otherwise it is *reported*.
#[derive(Debug, Clone, PartialEq)]
struct StackModel {
    id: String,
    count: i64,
    damage: Option<i64>,
    /// Container slot (Java `Slot`), passed through when present.
    slot: Option<i64>,
    custom_name: Option<String>,
    lore: Option<Vec<String>>,
    /// Java enchantments as `(java_id, level)`.
    enchants: Vec<(String, i32)>,
    book: Option<BookModel>,
}

#[derive(Debug, Clone, PartialEq)]
struct BookModel {
    title: Option<String>,
    author: Option<String>,
    /// Visible page text. Java components-era books separate `raw` and
    /// `filtered` text; only one can be carried per page.
    pages: Vec<PageText>,
}

#[derive(Debug, Clone, PartialEq)]
struct PageText {
    raw: String,
    /// The filtered (chat-filtered) variant, when present — always a
    /// loss going to Bedrock (no per-page pair there).
    filtered: Option<String>,
}

fn bad_field(field: &'static str, detail: impl Into<String>) -> ConvertError {
    ConvertError::BadField {
        field,
        detail: detail.into(),
    }
}

fn read_count(stack: &Tag) -> Result<i64> {
    if let Some(v) = stack.get("count").or_else(|| stack.get("Count")) {
        let name = if stack.get("count").is_some() { "count" } else { "Count" };
        let n = v
            .as_i64()
            .ok_or_else(|| bad_field(name, format!("expected an integer, found {}", v.type_name())))?;
        if n < 0 {
            return Err(bad_field(name, format!("negative count {n}")));
        }
        return Ok(n);
    }
    Ok(1)
}

/// Parse a Java stack of either era into the shared model, recording
/// everything that will not survive the trip.
fn parse_java(stack: &Tag, report: &mut ConversionReport, opts: &ItemConvertOpts) -> Result<StackModel> {
    let id = stack
        .get("id")
        .and_then(Tag::as_str)
        .ok_or(ConvertError::MissingItemId)?
        .to_string();

    let mut model = StackModel {
        id,
        count: read_count(stack)?,
        damage: None,
        slot: stack.get("Slot").and_then(Tag::as_i64),
        custom_name: None,
        lore: None,
        enchants: Vec::new(),
        book: None,
    };

    let tag = stack.get("tag");
    let components = stack.get("components");
    match (tag.is_some(), components.is_some()) {
        (true, true) => match opts.mixed_era {
            MixedEraPolicy::Reject => return Err(ConvertError::MixedEraStack),
            MixedEraPolicy::PreferComponents => {
                report.record(
                    LossKind::MixedEra,
                    "tag",
                    "stack has both `tag` and `components`; `tag` discarded per policy",
                );
                parse_java_tag(components.unwrap(), &mut model, report, "components");
            }
            MixedEraPolicy::PreferTag => {
                report.record(
                    LossKind::MixedEra,
                    "components",
                    "stack has both `tag` and `components`; `components` discarded per policy",
                );
                parse_java_tag(tag.unwrap(), &mut model, report, "tag");
            }
        },
        (true, false) => parse_java_tag(tag.unwrap(), &mut model, report, "tag"),
        (false, true) => parse_java_tag(components.unwrap(), &mut model, report, "components"),
        (false, false) => {}
    }

    if let Some(damage) = stack.get("Damage").and_then(Tag::as_i64) {
        model.damage = Some(damage);
    }
    Ok(model)
}

/// Apply the known fields of a `tag` (legacy) or `components` (modern)
/// compound to the model; unknown keys are recorded, never guessed.
fn parse_java_tag(
    source: &Tag,
    model: &mut StackModel,
    report: &mut ConversionReport,
    source_name: &str,
) {
    let Some(entries) = source.as_compound() else {
        report.record(
            LossKind::UnsupportedComponent,
            source_name,
            format!("expected a compound, found {}", source.type_name()),
        );
        return;
    };

    for (key, value) in entries {
        // Normalized component id without the `minecraft:` prefix; bare
        // keys from hand-written NBT are accepted too.
        let bare = key.strip_prefix("minecraft:").unwrap_or(key);
        match bare {
            "custom_name" => {
                if let Some(name) = value.as_str() {
                    model.custom_name = Some(name.to_string());
                } else {
                    report.dropped(
                        format!("{source_name}.{key}"),
                        format!("custom_name is not a string ({})", value.type_name()),
                    );
                }
            }
            "lore" => match read_string_list(value) {
                Some(lines) => model.lore = Some(lines),
                None => report.dropped(
                    format!("{source_name}.{key}"),
                    format!("lore is not a list of strings ({})", value.type_name()),
                ),
            },
            "enchantments" | "Enchantments" | "ench" => {
                read_java_enchants(value, &mut model.enchants, report, &format!("{source_name}.{key}"));
            }
            "damage" | "Damage" => {
                if let Some(d) = value.as_i64() {
                    model.damage = Some(d);
                } else {
                    report.dropped(
                        format!("{source_name}.{key}"),
                        format!("damage is not an integer ({})", value.type_name()),
                    );
                }
            }
            "written_book_content" => {
                read_java_written_book(value, model, report, &format!("{source_name}.{key}"));
            }
            "writable_book_content" => {
                read_java_writable_book(value, model, report, &format!("{source_name}.{key}"));
            }
            "display" => {
                // Legacy-era location for name/lore.
                if let Some(name) = value.get("Name").and_then(Tag::as_str) {
                    model.custom_name = Some(name.to_string());
                }
                if let Some(lore) = value.get("Lore").and_then(read_string_list) {
                    model.lore = Some(lore);
                }
                let known = value
                    .get("Name")
                    .is_some()
                    | value.get("Lore").is_some();
                if !known {
                    report.dropped(
                        format!("{source_name}.{key}"),
                        "display compound has no Name/Lore we can map",
                    );
                }
            }
            "pages" => {
                // Legacy-era book pages: list of plain strings.
                if let Some(pages) = read_string_list(value) {
                    let book = model.book.get_or_insert_with(|| BookModel {
                        title: None,
                        author: None,
                        pages: Vec::new(),
                    });
                    book.pages = pages
                        .into_iter()
                        .map(|raw| PageText { raw, filtered: None })
                        .collect();
                } else {
                    report.dropped(
                        format!("{source_name}.{key}"),
                        format!("pages is not a list of strings ({})", value.type_name()),
                    );
                }
            }
            "title" => {
                if let Some(t) = value.as_str() {
                    model
                        .book
                        .get_or_insert_with(|| BookModel {
                            title: None,
                            author: None,
                            pages: Vec::new(),
                        })
                        .title = Some(t.to_string());
                }
            }
            "author" => {
                if let Some(a) = value.as_str() {
                    model
                        .book
                        .get_or_insert_with(|| BookModel {
                            title: None,
                            author: None,
                            pages: Vec::new(),
                        })
                        .author = Some(a.to_string());
                }
            }
            _ => {
                // Findings loss row 1: any component without a known
                // Bedrock home is dropped and reported.
                report.record(
                    LossKind::UnsupportedComponent,
                    format!("{source_name}.\"{key}\""),
                    "no Bedrock mapping for this field yet (extend the M4 rule set)",
                );
            }
        }
    }
}

fn read_string_list(value: &Tag) -> Option<Vec<String>> {
    match value {
        Tag::List(items) => items
            .iter()
            .map(|i| i.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>(),
        _ => None,
    }
}

/// Java enchantments, both spellings: the components-era compound
/// (`{levels: {id: lvl}}`, or the flattened `{id: lvl}` form) and the
/// legacy list (`[{id: String, lvl}]`, or numeric-id `ench`).
fn read_java_enchants(
    value: &Tag,
    out: &mut Vec<(String, i32)>,
    report: &mut ConversionReport,
    path: &str,
) {
    let levels = match value {
        Tag::Compound(_) => match value.get("levels") {
            Some(levels) => levels,
            None => value, // flattened post-1.21.5 form
        },
        Tag::List(_) => {
            // Legacy list form.
            let Some(items) = value.as_list() else {
                report.dropped(path, "enchantments list is malformed");
                return;
            };
            for entry in items {
                let Some(id) = entry
                    .get("id")
                    .and_then(|id| id.as_str().map(str::to_string).or_else(|| {
                        id.as_i64().map(|n| format!("java-numeric:{n}"))
                    }))
                else {
                    report.dropped(path, "enchantment entry without id");
                    continue;
                };
                let lvl = entry.get("lvl").and_then(Tag::as_i64).unwrap_or(0) as i32;
                out.push((id, lvl));
            }
            return;
        }
        _ => {
            report.dropped(
                path,
                format!("enchantments has unexpected type {}", value.type_name()),
            );
            return;
        }
    };

    let Some(entries) = levels.as_compound() else {
        report.dropped(path, "enchantments.levels is not a compound");
        return;
    };
    for (id, lvl) in entries {
        match lvl.as_i64() {
            Some(lvl) => out.push((id.clone(), lvl as i32)),
            None => report.dropped(format!("{path}.{id}"), "enchantment level is not an integer"),
        }
    }
}

fn read_java_written_book(
    value: &Tag,
    model: &mut StackModel,
    report: &mut ConversionReport,
    path: &str,
) {
    let mut book = BookModel {
        title: value
            .get("title")
            .and_then(Tag::as_str)
            .map(str::to_string),
        author: value
            .get("author")
            .and_then(Tag::as_str)
            .map(str::to_string),
        pages: Vec::new(),
    };
    match value.get("pages").and_then(Tag::as_list) {
        Some(pages) => {
            for (i, page) in pages.iter().enumerate() {
                let raw = page.get("raw").and_then(Tag::as_str);
                let plain = page.as_str();
                match raw.or(plain) {
                    Some(text) => book.pages.push(PageText {
                        raw: text.to_string(),
                        filtered: page.get("filtered").and_then(Tag::as_str).map(str::to_string),
                    }),
                    None => report.dropped(
                        format!("{path}.pages[{i}]"),
                        "page has no raw/plain text we can read",
                    ),
                }
            }
        }
        None => report.dropped(format!("{path}.pages"), "book has no pages list"),
    }
    model.book = Some(book);
}

fn read_java_writable_book(
    value: &Tag,
    model: &mut StackModel,
    report: &mut ConversionReport,
    path: &str,
) {
    let mut book = BookModel {
        title: None,
        author: None,
        pages: Vec::new(),
    };
    match value.get("pages").and_then(Tag::as_list) {
        Some(pages) => {
            for (i, page) in pages.iter().enumerate() {
                let raw = page.get("raw").and_then(Tag::as_str);
                let plain = page.as_str();
                match raw.or(plain) {
                    Some(text) => book.pages.push(PageText {
                        raw: text.to_string(),
                        filtered: page.get("filtered").and_then(Tag::as_str).map(str::to_string),
                    }),
                    None => report.dropped(
                        format!("{path}.pages[{i}]"),
                        "page has no raw/plain text we can read",
                    ),
                }
            }
        }
        None => report.dropped(format!("{path}.pages"), "book has no pages list"),
    }
    model.book = Some(book);
}

/// Bedrock item NBT as observed in the 1.26.52 fixtures (block-entity
/// item stacks): `{Name: String, Count: Byte, Damage: Short,
/// WasPickedUp: Byte}` plus, on items that have them, the sub-compounds
/// below.
fn parse_bedrock(
    stack: &Tag,
    report: &mut ConversionReport,
    opts: &ItemConvertOpts,
) -> Result<StackModel> {
    let id = stack
        .get("Name")
        .and_then(Tag::as_str)
        .ok_or(ConvertError::MissingItemId)?
        .to_string();

    let mut model = StackModel {
        id,
        count: read_count(stack)?,
        // `Damage: 0s` is Bedrock's always-written default (fixture) —
        // Java omits zero damage, so normalize it away to keep
        // round-trips free of phantom `minecraft:damage: 0` components.
        damage: stack.get("Damage").and_then(Tag::as_i64).filter(|d| *d != 0),
        slot: None,
        custom_name: None,
        lore: None,
        enchants: Vec::new(),
        book: None,
    };

    if let Some(display) = stack.get("display") {
        model.custom_name = display.get("Name").and_then(Tag::as_str).map(str::to_string);
        model.lore = display.get("Lore").and_then(read_string_list);
    }

    if let Some(ench) = stack.get("ench").and_then(Tag::as_list) {
        for entry in ench {
            let numeric = entry.get("id").and_then(Tag::as_i64);
            let lvl = entry.get("lvl").and_then(Tag::as_i64).unwrap_or(0) as i32;
            match numeric {
                Some(n) => match opts.enchant_ids.bedrock_to_java(n as i16) {
                    Some(java) => model.enchants.push((java, lvl)),
                    None => report.record(
                        LossKind::UnmappedId,
                        "ench",
                        format!(
                            "Bedrock enchant id {n} has no Java mapping in the id table \
                             (registry data lands with M3 registry-gen)"
                        ),
                    ),
                },
                None => report.dropped("ench[]", "enchantment entry without numeric id"),
            }
        }
    }

    if stack.get("pages").is_some() || stack.get("title").is_some() || stack.get("author").is_some()
    {
        model.book = Some(BookModel {
            title: stack.get("title").and_then(Tag::as_str).map(str::to_string),
            author: stack.get("author").and_then(Tag::as_str).map(str::to_string),
            pages: Vec::new(),
        });
        if let Some(book) = &mut model.book
            && let Some(pages) = stack.get("pages").and_then(Tag::as_list)
        {
            for (i, page) in pages.iter().enumerate() {
                // OMT design decision (no fixture evidence): Bedrock
                // book pages as a list of `{text: String}` compounds.
                if let Some(text) = page.get("text").and_then(Tag::as_str) {
                    book.pages.push(PageText {
                        raw: text.to_string(),
                        filtered: None,
                    });
                } else if let Some(text) = page.as_str() {
                    book.pages.push(PageText {
                        raw: text.to_string(),
                        filtered: None,
                    });
                } else {
                    report.dropped(
                        format!("pages[{i}]"),
                        "page has no text we can read",
                    );
                }
            }
        }
    }

    // Findings loss row 2: Bedrock-only fields have no Java home.
    // `WasPickedUp` is reported only when it carries information (≠ 0):
    // the game writes `0b` on every item (fixture-verified), so reporting
    // the default would flag literally every stack; a non-zero marker is
    // real state being dropped.
    if let Some(v) = stack.get("WasPickedUp")
        && v.as_i64().map(|n| n != 0).unwrap_or(true)
    {
        report.dropped(
            "WasPickedUp",
            "Bedrock-only field has no Java home (picked-up marker lost)",
        );
    }
    // Any other unrecognized Bedrock field is likewise reported (row 2).
    if let Some(entries) = stack.as_compound() {
        const KNOWN: [&str; 10] = [
            "Name", "Count", "Damage", "WasPickedUp", "display", "ench", "title", "author",
            "pages", "Slot",
        ];
        for (key, _) in entries {
            if !KNOWN.contains(&key.as_str()) {
                report.dropped(
                    key.clone(),
                    "unknown Bedrock item field has no Java home",
                );
            }
        }
    }

    Ok(model)
}

// ---------------------------------------------------------------------------
// Emit: model -> edition NBT
// ---------------------------------------------------------------------------

/// Clamp an integer into `i8`/`i16` range, recording a coercion when
/// the value changed.
fn clamp_int(report: &mut ConversionReport, path: &str, value: i64, lo: i64, hi: i64) -> i64 {
    if value < lo {
        report.record(
            LossKind::ValueCoerced,
            path,
            format!("value {value} clamped to {lo} to fit the target width"),
        );
        lo
    } else if value > hi {
        report.record(
            LossKind::ValueCoerced,
            path,
            format!("value {value} clamped to {hi} to fit the target width"),
        );
        hi
    } else {
        value
    }
}

/// Build the Bedrock stack NBT from a parsed model.
fn emit_bedrock(model: &StackModel, report: &mut ConversionReport, opts: &ItemConvertOpts) -> Tag {
    let mut out = Tag::compound();
    out.insert("Name", Tag::String(model.id.clone()));
    out.insert(
        "Count",
        Tag::Byte(clamp_int(report, "Count", model.count, i8::MIN as i64, i8::MAX as i64) as i8),
    );
    out.insert(
        "Damage",
        Tag::Short(
            clamp_int(
                report,
                "Damage",
                model.damage.unwrap_or(0),
                i16::MIN as i64,
                i16::MAX as i64,
            ) as i16,
        ),
    );
    // Fixtures show WasPickedUp present even on plain items; whether UMT
    // writes it is unverified (findings Task A residual) — we do, as the
    // game itself does.
    out.insert("WasPickedUp", Tag::Byte(0));

    if model.slot.is_some() {
        report.dropped("Slot", "Java container slot is positional on the Bedrock side");
    }

    if model.custom_name.is_some() || model.lore.is_some() {
        let mut display = Tag::compound();
        if let Some(name) = &model.custom_name {
            display.insert("Name", Tag::String(name.clone()));
        }
        if let Some(lore) = &model.lore {
            display.insert(
                "Lore",
                Tag::List(lore.iter().cloned().map(Tag::String).collect()),
            );
        }
        out.insert("display", display);
    }

    if !model.enchants.is_empty() {
        let mut ench = Vec::new();
        for (java_id, lvl) in &model.enchants {
            match opts.enchant_ids.java_to_bedrock(java_id) {
                Some(numeric) => {
                    let mut entry = Tag::compound();
                    entry.insert("id", Tag::Short(numeric));
                    entry.insert(
                        "lvl",
                        Tag::Short(clamp_int(report, "ench.lvl", *lvl as i64, 0, i16::MAX as i64) as i16),
                    );
                    ench.push(entry);
                }
                None => report.record(
                    LossKind::UnmappedId,
                    "ench",
                    format!(
                        "Java enchant `{java_id}` has no Bedrock numeric id in the id table \
                         (registry data lands with M3 registry-gen)"
                    ),
                ),
            }
        }
        if !ench.is_empty() {
            out.insert("ench", Tag::List(ench));
        }
    }

    if let Some(book) = &model.book {
        if let Some(title) = &book.title {
            out.insert("title", Tag::String(title.clone()));
        }
        if let Some(author) = &book.author {
            out.insert("author", Tag::String(author.clone()));
        }
        if !book.pages.is_empty() {
            let mut pages = Vec::new();
            for (i, page) in book.pages.iter().enumerate() {
                let mut p = Tag::compound();
                p.insert("text", Tag::String(page.raw.clone()));
                pages.push(p);
                if page.filtered.is_some() {
                    report.dropped(
                        format!("pages[{i}].filtered"),
                        "Bedrock books carry one text per page; the filtered variant is dropped",
                    );
                }
            }
            out.insert("pages", Tag::List(pages));
        }
    }

    out
}

/// Build a Java stack of the requested era from a parsed model.
fn emit_java(model: &StackModel, era: StackEra, report: &mut ConversionReport) -> Tag {
    let mut out = Tag::compound();
    out.insert("id", Tag::String(model.id.clone()));
    match era {
        StackEra::Legacy => {
            out.insert("Count", Tag::Byte(clamp_int(report, "Count", model.count, 1, i8::MAX as i64) as i8));
            if let Some(slot) = model.slot {
                out.insert("Slot", Tag::Byte(clamp_int(report, "Slot", slot, i8::MIN as i64, i8::MAX as i64) as i8));
            }
            if let Some(damage) = model.damage {
                out.insert("Damage", Tag::Int(damage as i32));
            }
            let mut tag = Tag::compound();
            if model.custom_name.is_some() || model.lore.is_some() {
                let mut display = Tag::compound();
                if let Some(name) = &model.custom_name {
                    display.insert("Name", Tag::String(name.clone()));
                }
                if let Some(lore) = &model.lore {
                    display.insert(
                        "Lore",
                        Tag::List(lore.iter().cloned().map(Tag::String).collect()),
                    );
                }
                tag.insert("display", display);
            }
            if !model.enchants.is_empty() {
                let ench = model
                    .enchants
                    .iter()
                    .map(|(id, lvl)| {
                        let mut e = Tag::compound();
                        e.insert("id", Tag::String(id.clone()));
                        e.insert("lvl", Tag::Short(*lvl as i16));
                        e
                    })
                    .collect();
                tag.insert("Enchantments", Tag::List(ench));
            }
            if let Some(book) = &model.book {
                if let Some(title) = &book.title {
                    tag.insert("title", Tag::String(title.clone()));
                }
                if let Some(author) = &book.author {
                    tag.insert("author", Tag::String(author.clone()));
                }
                if !book.pages.is_empty() {
                    tag.insert(
                        "pages",
                        Tag::List(
                            book.pages
                                .iter()
                                .map(|p| Tag::String(p.raw.clone()))
                                .collect(),
                        ),
                    );
                }
            }
            if !tag.is_empty() {
                out.insert("tag", tag);
            }
        }
        StackEra::Components => {
            out.insert("count", Tag::Int(clamp_int(report, "count", model.count, 1, i32::MAX as i64) as i32));
            if let Some(slot) = model.slot {
                out.insert("Slot", Tag::Byte(clamp_int(report, "Slot", slot, i8::MIN as i64, i8::MAX as i64) as i8));
            }
            let mut components = Tag::compound();
            if let Some(name) = &model.custom_name {
                components.insert("minecraft:custom_name", Tag::String(name.clone()));
            }
            if let Some(lore) = &model.lore {
                components.insert(
                    "minecraft:lore",
                    Tag::List(lore.iter().cloned().map(Tag::String).collect()),
                );
            }
            if !model.enchants.is_empty() {
                let mut levels = Tag::compound();
                for (id, lvl) in &model.enchants {
                    levels.insert(id.clone(), Tag::Int(*lvl));
                }
                let mut ench = Tag::compound();
                ench.insert("levels", levels);
                components.insert("minecraft:enchantments", ench);
            }
            if let Some(damage) = model.damage {
                components.insert("minecraft:damage", Tag::Int(damage as i32));
            }
            if let Some(book) = &model.book {
                let mut content = Tag::compound();
                if let Some(title) = &book.title {
                    content.insert("title", Tag::String(title.clone()));
                }
                if let Some(author) = &book.author {
                    content.insert("author", Tag::String(author.clone()));
                }
                if !book.pages.is_empty() {
                    content.insert(
                        "pages",
                        Tag::List(
                            book.pages
                                .iter()
                                .map(|p| {
                                    let mut page = Tag::compound();
                                    page.insert("raw", Tag::String(p.raw.clone()));
                                    page
                                })
                                .collect(),
                        ),
                    );
                }
                components.insert("minecraft:written_book_content", content);
            }
            if !components.is_empty() {
                out.insert("components", components);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Convert a Java item stack (either era; see [`StackEra`]) to Bedrock
/// item NBT. The input compound is read, never mutated.
///
/// Returns the Bedrock stack plus the loss ledger for everything that
/// did not survive.
pub fn java_stack_to_bedrock(
    stack: &Tag,
    opts: &ItemConvertOpts,
) -> Result<(Tag, ConversionReport)> {
    stack
        .as_compound()
        .ok_or(ConvertError::NotAStack(stack.type_name()))?;
    let mut report = ConversionReport::new();
    let model = parse_java(stack, &mut report, opts)?;
    let bedrock = emit_bedrock(&model, &mut report, opts);
    Ok((bedrock, report))
}

/// Convert a Bedrock item stack to Java item NBT of the requested era.
pub fn bedrock_stack_to_java(
    stack: &Tag,
    target: StackEra,
    opts: &ItemConvertOpts,
) -> Result<(Tag, ConversionReport)> {
    stack
        .as_compound()
        .ok_or(ConvertError::NotAStack(stack.type_name()))?;
    let mut report = ConversionReport::new();
    let model = parse_bedrock(stack, &mut report, opts)?;
    let java = emit_java(&model, target, &mut report);
    Ok((java, report))
}

#[cfg(test)]
mod tests {
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
}
