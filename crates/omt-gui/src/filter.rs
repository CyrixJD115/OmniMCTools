//! World library filtering: a search syntax plus facet chips and sorting.
//!
//! Search box syntax (all parts AND-ed, case-insensitive):
//! * free words / `"quoted phrases"` match name, folder, path, version and
//!   launcher label; `-word` excludes;
//! * `edition:java|bedrock` (`e:`), `mode:survival|creative|adventure|
//!   spectator` (`m:`), `source:prism|vanilla|modrinth|multimc|mcpelauncher|
//!   bol|windows|uwp|custom` (`src:`), `version:1.21` (`v:`, prefix match);
//! * `cheats:on|off`, `is:fav`, `is:preview`;
//! * `size:>1g`, `size:<500m` (k/m/g suffixes), `played:<7d`, `played:>1y`
//!   (h/d/w/m/y).
//!
//! Unknown `key:value` tokens are reported back so the UI can flag them.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use omt_world::Edition;
use serde::{Deserialize, Serialize};

use crate::sources::Launcher;
use crate::worlds::WorldEntry;

/// Library sort orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortBy {
    #[default]
    LastPlayed,
    Name,
    Size,
    Edition,
    Source,
    Version,
}

impl SortBy {
    pub const ALL: [SortBy; 6] = [Self::LastPlayed, Self::Name, Self::Size, Self::Edition, Self::Source, Self::Version];
}

impl std::fmt::Display for SortBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LastPlayed => "Last played",
            Self::Name => "Name",
            Self::Size => "Size",
            Self::Edition => "Edition",
            Self::Source => "Launcher",
            Self::Version => "Version",
        })
    }
}

/// "Last played" facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Played {
    #[default]
    Any,
    Today,
    Week,
    Month,
    Older,
}

impl Played {
    pub const ALL: [Played; 5] = [Self::Any, Self::Today, Self::Week, Self::Month, Self::Older];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any time",
            Self::Today => "Today",
            Self::Week => "This week",
            Self::Month => "This month",
            Self::Older => "Older",
        }
    }

    fn accepts(self, age: Option<Duration>) -> bool {
        const DAY: u64 = 86_400;
        let Some(a) = age.map(|d| d.as_secs()) else { return self == Self::Any || self == Self::Older };
        match self {
            Self::Any => true,
            Self::Today => a < DAY,
            Self::Week => a < 7 * DAY,
            Self::Month => a < 31 * DAY,
            Self::Older => a >= 31 * DAY,
        }
    }
}

/// Game-mode facet values (`None` = unknown).
pub fn mode_name(m: Option<i32>) -> &'static str {
    match m {
        Some(0) => "Survival",
        Some(1) => "Creative",
        Some(2) => "Adventure",
        Some(3) => "Spectator",
        _ => "Unknown",
    }
}

/// Facet chip state (the clickable filters beside the search box).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facets {
    pub editions: BTreeSet<EditionKey>,
    pub launchers: BTreeSet<Launcher>,
    pub modes: BTreeSet<&'static str>,
    pub played: Played,
    pub favorites: bool,
}

/// `Edition` is not `Ord`; facets key it by this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EditionKey {
    Java,
    Bedrock,
}

impl EditionKey {
    pub fn of(e: Edition) -> Self {
        match e {
            Edition::Java => Self::Java,
            Edition::Bedrock => Self::Bedrock,
        }
    }
}

impl Facets {
    pub fn active(&self) -> usize {
        self.editions.len() + self.launchers.len() + self.modes.len() + usize::from(self.played != Played::Any) + usize::from(self.favorites)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Term {
    Text(String, bool),
    Edition(EditionKey),
    Mode(String),
    Source(Launcher),
    Version(String),
    Cheats(bool),
    Fav,
    Preview,
    SizeGt(u64),
    SizeLt(u64),
    /// Played within (`true`) / longer ago than (`false`) a duration.
    Played(Duration, bool),
}

/// A parsed search query.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    terms: Vec<Term>,
    /// Tokens that looked like `key:value` but were not understood.
    pub unknown: Vec<String>,
}

fn tokens(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in q.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_size(v: &str) -> Option<u64> {
    let v = v.trim().to_ascii_lowercase();
    let (num, mul) = match v.trim_end_matches('b').chars().last()? {
        'k' => (&v[..v.trim_end_matches('b').len() - 1], 1u64 << 10),
        'm' => (&v[..v.trim_end_matches('b').len() - 1], 1 << 20),
        'g' => (&v[..v.trim_end_matches('b').len() - 1], 1 << 30),
        't' => (&v[..v.trim_end_matches('b').len() - 1], 1 << 40),
        _ => (v.trim_end_matches('b'), 1),
    };
    num.parse::<f64>().ok().map(|n| (n * mul as f64) as u64)
}

fn parse_age(v: &str) -> Option<Duration> {
    let v = v.trim().to_ascii_lowercase();
    let unit = v.chars().last()?;
    let n: f64 = v[..v.len() - unit.len_utf8()].parse().ok()?;
    let secs = match unit {
        'h' => 3600.0,
        'd' => 86_400.0,
        'w' => 604_800.0,
        'm' => 2_592_000.0,
        'y' => 31_536_000.0,
        _ => return None,
    };
    Some(Duration::from_secs_f64(n * secs))
}

impl Query {
    pub fn parse(q: &str) -> Self {
        let mut out = Self::default();
        for tok in tokens(q) {
            let (neg, body) = match tok.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_string()),
                _ => (false, tok.clone()),
            };
            let Some((k, v)) = body.split_once(':').filter(|(k, v)| !k.is_empty() && !v.is_empty() && !k.contains('/')) else {
                out.terms.push(Term::Text(body.to_lowercase(), neg));
                continue;
            };
            let v_l = v.to_ascii_lowercase();
            let term = match k.to_ascii_lowercase().as_str() {
                "edition" | "e" | "ed" => match v_l.as_str() {
                    "java" | "j" => Some(Term::Edition(EditionKey::Java)),
                    "bedrock" | "b" | "be" => Some(Term::Edition(EditionKey::Bedrock)),
                    _ => None,
                },
                "mode" | "m" | "gamemode" => Some(Term::Mode(v_l)),
                "source" | "src" | "launcher" | "l" => Launcher::from_token(&v_l).map(Term::Source),
                "version" | "v" => Some(Term::Version(v_l)),
                "cheats" | "commands" => match v_l.as_str() {
                    "on" | "yes" | "true" => Some(Term::Cheats(true)),
                    "off" | "no" | "false" => Some(Term::Cheats(false)),
                    _ => None,
                },
                "is" => match v_l.as_str() {
                    "fav" | "favorite" | "favourite" | "star" | "starred" => Some(Term::Fav),
                    "preview" | "beta" => Some(Term::Preview),
                    _ => None,
                },
                "size" => {
                    if let Some(n) = v_l.strip_prefix('>') {
                        parse_size(n).map(Term::SizeGt)
                    } else if let Some(n) = v_l.strip_prefix('<') {
                        parse_size(n).map(Term::SizeLt)
                    } else {
                        None
                    }
                }
                "played" | "p" => {
                    if let Some(n) = v_l.strip_prefix('<') {
                        parse_age(n).map(|d| Term::Played(d, true))
                    } else if let Some(n) = v_l.strip_prefix('>') {
                        parse_age(n).map(|d| Term::Played(d, false))
                    } else {
                        None
                    }
                }
                _ => {
                    // Not a known key: treat `C:/…`-like text as plain text.
                    out.terms.push(Term::Text(body.to_lowercase(), neg));
                    continue;
                }
            };
            match term {
                // Negation only makes sense for text; structured terms with
                // `-` are reported as unknown.
                Some(t) if !neg => out.terms.push(t),
                _ => out.unknown.push(tok),
            }
        }
        out
    }

    fn matches(&self, w: &WorldEntry, fav: bool, now: SystemTime) -> bool {
        let age = w.last_modified.and_then(|t| now.duration_since(t).ok());
        self.terms.iter().all(|t| match t {
            Term::Text(s, neg) => {
                let hay = [w.summary.name.as_str(), w.folder_label.as_str(), w.version.as_str(), w.launcher.name()];
                let hit = hay.iter().any(|h| h.to_lowercase().contains(s.as_str())) || w.origin.to_string_lossy().to_lowercase().contains(s.as_str());
                hit != *neg
            }
            Term::Edition(e) => EditionKey::of(w.edition()) == *e,
            Term::Mode(m) => mode_name(w.summary.gamemode).to_lowercase().starts_with(m.as_str()),
            Term::Source(l) => w.launcher == *l,
            Term::Version(v) => w.version.to_lowercase().starts_with(v.as_str()) || w.version.to_lowercase().contains(&format!(" {v}")),
            Term::Cheats(c) => w.cheats == Some(*c),
            Term::Fav => fav,
            Term::Preview => w.preview,
            Term::SizeGt(n) => w.size > *n,
            Term::SizeLt(n) => w.size < *n,
            Term::Played(d, within) => age.is_some_and(|a| (a < *d) == *within),
        })
    }
}

/// Apply query + facets + sort; returns indices into `worlds`.
/// Favorites float to the top within the chosen order.
pub fn apply(worlds: &[WorldEntry], q: &Query, f: &Facets, sort: SortBy, is_fav: &dyn Fn(&WorldEntry) -> bool) -> Vec<usize> {
    let now = SystemTime::now();
    let mut idx: Vec<usize> = (0..worlds.len())
        .filter(|&i| {
            let w = &worlds[i];
            let fav = is_fav(w);
            (f.editions.is_empty() || f.editions.contains(&EditionKey::of(w.edition())))
                && (f.launchers.is_empty() || f.launchers.contains(&w.launcher))
                && (f.modes.is_empty() || f.modes.contains(mode_name(w.summary.gamemode)))
                && f.played.accepts(w.last_modified.and_then(|t| now.duration_since(t).ok()))
                && (!f.favorites || fav)
                && q.matches(w, fav, now)
        })
        .collect();
    idx.sort_by(|&a, &b| {
        let (wa, wb) = (&worlds[a], &worlds[b]);
        let fav = is_fav(wb).cmp(&is_fav(wa));
        let ord = match sort {
            SortBy::LastPlayed => wb.last_modified.cmp(&wa.last_modified),
            SortBy::Name => wa.summary.name.to_lowercase().cmp(&wb.summary.name.to_lowercase()),
            SortBy::Size => wb.size.cmp(&wa.size),
            SortBy::Edition => EditionKey::of(wa.edition()).cmp(&EditionKey::of(wb.edition())).then(wb.last_modified.cmp(&wa.last_modified)),
            SortBy::Source => wa.launcher.cmp(&wb.launcher).then(wa.folder_label.cmp(&wb.folder_label)),
            SortBy::Version => wb.version.cmp(&wa.version),
        };
        fav.then(ord)
    });
    idx
}

/// Counts per facet value over the whole library (chip badges).
#[derive(Debug, Default)]
pub struct Counts {
    pub java: usize,
    pub bedrock: usize,
    pub launchers: Vec<(Launcher, usize)>,
    pub modes: Vec<(&'static str, usize)>,
}

pub fn counts(worlds: &[WorldEntry]) -> Counts {
    let mut c = Counts::default();
    let mut l: std::collections::BTreeMap<Launcher, usize> = Default::default();
    let mut m: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for w in worlds {
        match w.edition() {
            Edition::Java => c.java += 1,
            Edition::Bedrock => c.bedrock += 1,
        }
        *l.entry(w.launcher).or_default() += 1;
        *m.entry(mode_name(w.summary.gamemode)).or_default() += 1;
    }
    c.launchers = l.into_iter().collect();
    c.modes = m.into_iter().collect();
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn world(name: &str, ed: Edition, l: Launcher, mode: i32, size: u64, days_ago: u64, version: &str) -> WorldEntry {
        WorldEntry {
            summary: omt_world::WorldSummary {
                edition: ed,
                source: omt_world::WorldSource::BedrockFolder,
                root: PathBuf::from(format!("/w/{name}")),
                name: name.into(),
                java_data_version: None,
                bedrock_storage_version: None,
                bedrock_last_opened: None,
                gamemode: Some(mode),
                time_played_seconds: None,
                spawn: None,
                icon: None,
                below_floor: false,
            },
            origin: PathBuf::from(format!("/w/{name}")),
            folder_label: l.name().into(),
            launcher: l,
            preview: false,
            last_modified: Some(SystemTime::now() - Duration::from_secs(days_ago * 86_400 + 60)),
            cheats: Some(mode == 1),
            size,
            version: version.into(),
        }
    }

    fn lib() -> Vec<WorldEntry> {
        vec![
            world("Survival Island", Edition::Bedrock, Launcher::Mcpelauncher, 0, 300 << 20, 1, "1.21.120"),
            world("Creative Build", Edition::Java, Launcher::Prism, 1, 2 << 30, 40, "26.3 · DataVersion 5023"),
            world("Hardcore Run", Edition::Java, Launcher::JavaVanilla, 0, 50 << 20, 3, "26.3 · DataVersion 5023"),
            world("Old Realm", Edition::Bedrock, Launcher::BedrockOnLinux, 2, 1 << 30, 400, "1.20.80"),
        ]
    }

    fn names(w: &[WorldEntry], idx: &[usize]) -> Vec<String> {
        idx.iter().map(|&i| w[i].summary.name.clone()).collect()
    }

    #[test]
    fn query_syntax() {
        let w = lib();
        let no = |_: &WorldEntry| false;
        let run = |q: &str| names(&w, &apply(&w, &Query::parse(q), &Facets::default(), SortBy::Name, &no));
        assert_eq!(run("e:java"), ["Creative Build", "Hardcore Run"]);
        assert_eq!(run("edition:bedrock src:bol"), ["Old Realm"]);
        assert_eq!(run("mode:creative"), ["Creative Build"]);
        assert_eq!(run("size:>500m"), ["Creative Build", "Old Realm"]);
        assert_eq!(run("played:<7d"), ["Hardcore Run", "Survival Island"]);
        assert_eq!(run("played:>1y"), ["Old Realm"]);
        assert_eq!(run("v:1.21"), ["Survival Island"]);
        assert_eq!(run("\"survival island\""), ["Survival Island"]);
        assert_eq!(run("-run e:java"), ["Creative Build"]);
        assert_eq!(run("cheats:on"), ["Creative Build"]);
        let q = Query::parse("foo:bar e:wii is:fav");
        assert_eq!(q.unknown, vec!["e:wii".to_string()]);
        assert!(Query::parse("C:/Games").unknown.is_empty(), "paths stay text");
    }

    #[test]
    fn facets_sort_and_favorites() {
        let w = lib();
        let mut f = Facets::default();
        f.launchers.insert(Launcher::Prism);
        f.launchers.insert(Launcher::Mcpelauncher);
        let no = |_: &WorldEntry| false;
        assert_eq!(names(&w, &apply(&w, &Query::default(), &f, SortBy::Size, &no)), ["Creative Build", "Survival Island"]);
        let f = Facets { played: Played::Older, ..Default::default() };
        assert_eq!(names(&w, &apply(&w, &Query::default(), &f, SortBy::Name, &no)), ["Creative Build", "Old Realm"]);
        let fav = |e: &WorldEntry| e.summary.name == "Old Realm";
        let all = apply(&w, &Query::default(), &Facets::default(), SortBy::LastPlayed, &fav);
        assert_eq!(names(&w, &all)[0], "Old Realm", "favorites first");
        assert_eq!(names(&w, &all)[1], "Survival Island");
        let c = counts(&w);
        assert_eq!((c.java, c.bedrock), (2, 2));
        assert_eq!(parse_size("1.5g"), Some((1.5 * (1u64 << 30) as f64) as u64));
        assert_eq!(parse_age("2w"), Some(Duration::from_secs(1_209_600)));
    }
}
