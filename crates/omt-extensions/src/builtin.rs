//! Extensions shipped inside the binary (`include_bytes!`): they are listed
//! on the Extensions page even before they exist on disk, and enabling one
//! writes its files into the extensions folder (so it stays inspectable and
//! editable) and turns it on. Nothing is written for players who never opt
//! in, and a removed built-in simply shows up here again.

use omt_script::Manifest;

/// A shipped, embedded extension.
pub struct Builtin {
    /// Folder id once materialized (`a-z0-9-`).
    pub id: &'static str,
    /// Shown on the card before it is installed.
    pub name: &'static str,
    /// Shown on the card before it is installed.
    pub description: &'static str,
    /// The embedded files (`manifest.json`, `main.lua`, …).
    pub files: fn() -> Vec<(String, Vec<u8>)>,
}

impl Builtin {
    /// The embedded manifest (falls back to a minimal one when unparsable).
    pub fn manifest(&self) -> Manifest {
        let files: Vec<(String, Vec<u8>)> = (self.files)();
        files
            .iter()
            .find(|(name, _)| name == "manifest.json")
            .and_then(|(_, bytes)| serde_json::from_slice(bytes).ok())
            .unwrap_or_else(|| Manifest { name: self.name.into(), version: String::new(), author: "OmniMCTools".into(), description: self.description.into() })
    }
}

/// Every extension that ships with the app (source in `assets/`, embedded
/// at compile time). Order = list order on the Extensions page.
pub fn builtins() -> &'static [Builtin] {
    &[
        Builtin {
            id: "chest-explorer",
            name: "Chest Explorer",
            description: "Find and open containers and player inventories as chest grids, and an 'Open as chest' action in the NBT editor.",
            files: crate::example_files,
        },
        Builtin {
            id: "ticks-time",
            name: "Tick Clock",
            description: "Tick counters shown as in-game time (row annotations in the NBT editor).",
            files: omt_script::example_files,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_have_valid_ids_and_parsable_manifests() {
        let ids: Vec<&str> = builtins().iter().map(|b| b.id).collect();
        assert!(ids.contains(&"chest-explorer"));
        assert!(ids.contains(&"ticks-time"));
        for b in builtins() {
            assert!(b.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "id {}", b.id);
            let m = b.manifest();
            assert_eq!(m.name, b.name, "{} manifest name", b.id);
            let files = (b.files)();
            assert!(files.iter().any(|(n, _)| n == "main.lua"), "{} ships main.lua", b.id);
            assert!(files.iter().all(|(_, bytes)| !bytes.is_empty()));
        }    }
}
