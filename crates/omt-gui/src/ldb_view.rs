//! LevelDB Search (UMT editor side-tab, Bedrock only): every database key
//! with a decoded description, substring / hex search, open as NBT (single
//! roots or sequences), hex view for everything else.

use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Element, Font, Length, Task};

use crate::session::{Store, blocking};
use crate::style::{self, Btn};

const PAGE: usize = 400;

#[derive(Debug, Clone)]
pub struct KeyRow {
    pub key: Vec<u8>,
    pub size: usize,
    pub desc: String,
    pub printable: String,
}

#[derive(Debug, Clone)]
pub enum LdbMsg {
    Query(String),
    Search,
    Loaded(Result<std::sync::Arc<Vec<KeyRow>>, String>),
    Select(usize),
    Open(usize),
    More,
}

#[derive(Default)]
pub struct LdbView {
    all: std::sync::Arc<Vec<KeyRow>>,
    filtered: Vec<usize>,
    query: String,
    shown: usize,
    loading: bool,
    selected: Option<usize>,
    hex: String,
    pub status: String,
}

/// Human description of a key (shared with the CLI's vocabulary).
pub fn describe(key: &[u8]) -> String {
    use omt_bedrock::keys::{DbKey, decode_key};
    let dim = |d: u8| omt_world::Dimension::from_id(d).map(|d| d.name()).unwrap_or("?");
    match decode_key(key) {
        DbKey::Subchunk { x, z, dim: d, y } => format!("Subchunk {x}:{z} · {} · y {y}", dim(d)),
        DbKey::ChunkRecord { x, z, dim: d, tag } => {
            let what = match tag {
                0x2b => "Data3D (heightmap + biomes)",
                0x2c => "Chunk version",
                0x31 => "Block entities",
                0x32 => "Entities (legacy)",
                0x33 => "Pending block ticks",
                0x36 => "Finalized state",
                0x3a => "Pending fluid ticks",
                0x77 => "Structure references",
                _ => "Chunk record",
            };
            format!("{what} {x}:{z} · {} · tag 0x{tag:02x}", dim(d))
        }
        DbKey::Digp { x, z, dim: d } => format!("Actor digest {x}:{z} · {}", dim(d)),
        DbKey::Actor { .. } => "Actor (entity)".into(),
        DbKey::LocalPlayer => "Local player".into(),
        DbKey::Player { .. } => "Player".into(),
        DbKey::Other => "Named record".into(),
    }
}

fn printable(key: &[u8]) -> String {
    if key.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        String::from_utf8_lossy(key).into_owned()
    } else {
        key.iter().map(|b| format!("{b:02x}")).collect()
    }
}

pub fn hexdump(bytes: &[u8], limit: usize) -> String {
    let mut out = String::new();
    for (i, chunk) in bytes[..bytes.len().min(limit)].chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk.iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '.' }).collect();
        out.push_str(&format!("{:08x}  {:<48}  {ascii}\n", i * 16, hex.join(" ")));
    }
    if bytes.len() > limit {
        out.push_str(&format!("… {} more bytes\n", bytes.len() - limit));
    }
    out
}

impl LdbView {
    pub fn is_loaded(&self) -> bool {
        !self.all.is_empty() || self.loading
    }

    pub fn load(&mut self, store: Option<Store>) -> Task<LdbMsg> {
        let Some(Store::Bedrock(w)) = store else { return Task::none() };
        self.loading = true;
        self.status = "Reading database keys…".into();
        Task::perform(
            blocking(move || {
                let keys = w.key_index().map_err(|e| e.to_string())?;
                Ok(std::sync::Arc::new(
                    keys.into_iter()
                        .map(|(k, size)| KeyRow { desc: describe(&k), printable: printable(&k), size, key: k })
                        .collect(),
                ))
            }),
            LdbMsg::Loaded,
        )
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_lowercase();
        let hex: Option<Vec<u8>> = (q.len() >= 2 && q.len().is_multiple_of(2) && q.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| (0..q.len()).step_by(2).filter_map(|i| u8::from_str_radix(&q[i..i + 2], 16).ok()).collect());
        self.filtered = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                q.is_empty()
                    || r.printable.to_lowercase().contains(&q)
                    || r.desc.to_lowercase().contains(&q)
                    || hex.as_ref().is_some_and(|h| r.key.windows(h.len()).any(|w| w == h.as_slice()))
            })
            .map(|(i, _)| i)
            .collect();
        self.shown = PAGE;
        self.status = format!("{} of {} keys", self.filtered.len(), self.all.len());
    }

    /// Returns the key to open in the NBT editor, if any.
    pub fn update(&mut self, msg: LdbMsg, store: Option<Store>) -> (Task<LdbMsg>, Option<Vec<u8>>) {
        match msg {
            LdbMsg::Query(q) => self.query = q,
            LdbMsg::Search => self.refilter(),
            LdbMsg::Loaded(res) => {
                self.loading = false;
                match res {
                    Ok(rows) => {
                        self.all = rows;
                        self.refilter();
                    }
                    Err(e) => self.status = format!("failed: {e}"),
                }
            }
            LdbMsg::Select(i) => {
                self.selected = Some(i);
                if let (Some(Store::Bedrock(w)), Some(row)) = (store, self.all.get(i)) {
                    self.hex = match w.raw_get(&row.key) {
                        Ok(Some(v)) => hexdump(&v, 4096),
                        Ok(None) => "(deleted)".into(),
                        Err(e) => format!("error: {e}"),
                    };
                }
            }
            LdbMsg::Open(i) => return (Task::none(), self.all.get(i).map(|r| r.key.clone())),
            LdbMsg::More => self.shown += PAGE,
        }
        (Task::none(), None)
    }

    pub fn view(&self) -> Element<'_, LdbMsg> {
        let search = row![
            text_input("search keys (text, description or hex bytes)…", &self.query)
                .on_input(LdbMsg::Query)
                .on_submit(LdbMsg::Search)
                .style(style::input()),
            button(text("Search").size(12)).on_press(LdbMsg::Search).style(style::button_style(Btn::Primary)),
        ]
        .spacing(6);
        let list = self.filtered.iter().take(self.shown).fold(column![].spacing(2), |c, &i| {
            let r = &self.all[i];
            let sel = self.selected == Some(i);
            c.push(
                button(
                    row![
                        text(r.printable.chars().take(48).collect::<String>()).size(12).font(Font::MONOSPACE).width(Length::FillPortion(3)),
                        text(r.desc.clone()).size(12).color(style::muted()).width(Length::FillPortion(4)),
                        text(format!("{} B", r.size)).size(11).color(style::hint()).width(70),
                    ]
                    .spacing(8),
                )
                .on_press(LdbMsg::Select(i))
                .width(Length::Fill)
                .padding([3, 6])
                .style(style::button_style(if sel { Btn::Primary } else { Btn::Ghost })),
            )
        });
        let more: Element<'_, LdbMsg> = if self.filtered.len() > self.shown {
            button(text(format!("Show more ({} hidden)", self.filtered.len() - self.shown)).size(12)).on_press(LdbMsg::More).style(style::button_style(Btn::Ghost)).into()
        } else {
            space::vertical().height(0).into()
        };
        let detail: Element<'_, LdbMsg> = match self.selected.and_then(|i| self.all.get(i).map(|r| (i, r))) {
            Some((i, r)) => column![
                row![
                    text(r.desc.clone()).size(13),
                    space::horizontal(),
                    button(text("Open in NBT editor").size(12)).on_press(LdbMsg::Open(i)).style(style::button_style(Btn::Confirm)),
                ]
                .align_y(iced::Alignment::Center),
                scrollable(text(self.hex.clone()).size(11).font(Font::MONOSPACE)).height(Length::Fill),
            ]
            .spacing(6)
            .into(),
            None => text("Select a key to see its bytes.").size(12).color(style::muted()).into(),
        };
        column![
            row![text("LevelDB Search").size(18), space::horizontal(), text(self.status.clone()).size(12).color(style::muted())],
            search,
            row![
                scrollable(column![list, more].spacing(4)).height(Length::Fill).width(Length::FillPortion(3)),
                container(detail).padding(8).style(style::card()).width(Length::FillPortion(2)).height(Length::Fill),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .padding(12)
        .into()
    }
}
