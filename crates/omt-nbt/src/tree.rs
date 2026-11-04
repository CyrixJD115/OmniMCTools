//! Human-readable NBT tree dump — the default `omt nbt` view, modeled on the
//! classic NBT explorer output (and close to what UMT's tree view shows).

use crate::{Nbt, Tag};

const LARGE_ARRAY_INLINE: usize = 8;

/// Classic `hexdump -C`-style dump: offset, 16 bytes hex, ASCII gutter.
pub fn hexdump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let base = i * 16;
        out.push_str(&format!("{base:08x}  "));
        for j in 0..16 {
            match chunk.get(j) {
                Some(b) => out.push_str(&format!("{b:02x} ")),
                None => out.push_str("   "),
            }
            if j == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        for b in chunk {
            out.push(if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '.' });
        }
        out.push_str("|\n");
    }
    if !bytes.is_empty() {
        out.push_str(&format!("{:08x}\n", bytes.len()));
    }
    out
}

/// One-line-per-tag dump:
/// ```text
/// TAG_Compound(""): 3 entries
///   TAG_String("LevelName"): "My World"
///   TAG_Int_Array("heightmap"): [8 ints] 0, 1, 2, ...
/// ```
pub fn dump(nbt: &Nbt) -> String {
    let mut s = String::new();
    dump_named(&nbt.tag, &nbt.name, 0, &mut s);
    s
}

fn dump_named(tag: &Tag, name: &str, depth: usize, out: &mut String) {
    indent(out, depth);
    match tag {
        Tag::Compound(entries) => {
            out.push_str(&format!(
                "{}(\"{name}\"): {} entr{}\n",
                tag.type_name(),
                entries.len(),
                if entries.len() == 1 { "y" } else { "ies" }
            ));
            for (k, v) in entries {
                dump_named(v, k, depth + 1, out);
            }
        }
        Tag::List(items) => {
            if items.is_empty() {
                out.push_str(&format!("{}(\"{name}\"): []\n", tag.type_name()));
                return;
            }
            out.push_str(&format!(
                "{}(\"{name}\"): {} of {}\n",
                tag.type_name(),
                items.len(),
                items[0].type_name()
            ));
            for v in items {
                dump_anon(v, depth + 1, out);
            }
        }
        Tag::ByteArray(v) => {
            out.push_str(&format!(
                "{}(\"{name}\"): {} bytes\n    ",
                tag.type_name(),
                v.len()
            ));
            push_array_preview(out, v.iter().map(|b| b.to_string()), v.len());
            out.push('\n');
        }
        Tag::IntArray(v) => {
            out.push_str(&format!("{}(\"{name}\"): {} ints\n    ", tag.type_name(), v.len()));
            push_array_preview(out, v.iter().map(|i| i.to_string()), v.len());
            out.push('\n');
        }
        Tag::LongArray(v) => {
            out.push_str(&format!("{}(\"{name}\"): {} longs\n    ", tag.type_name(), v.len()));
            push_array_preview(out, v.iter().map(|i| i.to_string()), v.len());
            out.push('\n');
        }
        Tag::String(v) => {
            out.push_str(&format!(
                "{}(\"{name}\"): \"{}\"\n",
                tag.type_name(),
                escape(v)
            ));
        }
        scalar => {
            out.push_str(&format!(
                "{}(\"{name}\"): {}\n",
                tag.type_name(),
                crate::snbt::to_snbt(scalar)
            ));
        }
    }
}

fn dump_anon(tag: &Tag, depth: usize, out: &mut String) {
    match tag {
        Tag::Compound(entries) => {
            indent(out, depth);
            out.push_str(&format!(
                "{}(None): {} entries\n",
                tag.type_name(),
                entries.len()
            ));
            for (k, v) in entries {
                dump_named(v, k, depth + 1, out);
            }
        }
        Tag::List(items) if !items.is_empty() => {
            indent(out, depth);
            out.push_str(&format!(
                "{}(None): {} of {}\n",
                tag.type_name(),
                items.len(),
                items[0].type_name()
            ));
            for v in items {
                dump_anon(v, depth + 1, out);
            }
        }
        other => {
            // Give scalars a synthetic empty name so the formatter is uniform.
            dump_named(other, "None", depth, out);
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn push_array_preview(out: &mut String, items: impl Iterator<Item = String>, len: usize) {
    if len == 0 {
        out.push_str("[]");
        return;
    }
    out.push('[');
    for (i, item) in items.take(LARGE_ARRAY_INLINE).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&item);
    }
    if len > LARGE_ARRAY_INLINE {
        out.push_str(&format!(", … +{}", len - LARGE_ARRAY_INLINE));
    }
    out.push(']');
}

fn escape(s: &str) -> String {
    s.chars().flat_map(|c| match c {
        '"' => vec!['\\', '"'],
        '\\' => vec!['\\', '\\'],
        '\n' => vec!['\\', 'n'],
        '\r' => vec!['\\', 'r'],
        '\t' => vec!['\\', 't'],
        c if (c as u32) < 0x20 => vec!['\\', 'u'],
        c => vec![c],
    })
    .collect()
}
