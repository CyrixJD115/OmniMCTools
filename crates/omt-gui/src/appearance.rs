//! Settings → Appearance: the visual theme picker and the Block Textures
//! panel (installed packs, automatic choice, opt-in download of Mojang's
//! vanilla textures with progress), plus their message handling.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use iced::widget::{button, column, container, progress_bar, row, space, stack, text};
use iced::{Alignment, Color, Element, Length, Task};
use omt_texpack::Found;

use crate::assets;
use crate::session::blocking;
use crate::style::{self, Btn, ThemeKind};
use crate::textures::{self, Progress, TexChoice};
use crate::{Message, OmniMCTools};

#[derive(Debug, Clone)]
pub enum TexMsg {
    Choose(TexChoice),
    Rescan,
    Scanned(Vec<Found>),
    Loaded(String),
    Download,
    Cancel,
    Tick,
    Downloaded(Result<String, String>),
}

/// Texture-panel state owned by the app.
#[derive(Debug, Default)]
pub struct TexState {
    pub found: Vec<Found>,
    pub status: String,
    pub download: Option<Arc<Progress>>,
}

impl OmniMCTools {
    /// Load the configured pack at startup (and after changes).
    pub(crate) fn load_textures(&mut self) -> Task<Message> {
        let choice = self.settings.texture_pack.clone();
        Task::batch([
            Task::perform(blocking(textures::found), |f| Message::Tex(TexMsg::Scanned(f))),
            Task::perform(blocking(move || textures::load(&choice)), |s| Message::Tex(TexMsg::Loaded(s))),
        ])
    }

    pub(crate) fn on_tex(&mut self, m: TexMsg) -> Task<Message> {
        match m {
            TexMsg::Choose(c) => {
                self.settings.texture_pack = c;
                self.save_settings();
                self.tex.status = "loading textures…".into();
                return self.load_textures();
            }
            TexMsg::Rescan => return self.load_textures(),
            TexMsg::Scanned(f) => self.tex.found = f,
            TexMsg::Loaded(s) => {
                self.tex.status = s.clone();
                self.status = s;
            }
            TexMsg::Download => {
                if self.tex.download.is_some() {
                    return Task::none();
                }
                let p = Arc::new(Progress::default());
                self.tex.download = Some(p.clone());
                self.tex.status = "downloading Mojang's vanilla textures…".into();
                return Task::perform(blocking(move || textures::download(p)), |r| Message::Tex(TexMsg::Downloaded(r)));
            }
            TexMsg::Cancel => {
                if let Some(p) = &self.tex.download {
                    p.cancel.store(true, Ordering::Relaxed);
                }
            }
            TexMsg::Tick => {}
            TexMsg::Downloaded(r) => {
                self.tex.download = None;
                match r {
                    Ok(s) => {
                        self.tex.status = s;
                        return self.load_textures();
                    }
                    Err(e) => self.tex.status = format!("download failed: {e}"),
                }
            }
        }
        Task::none()
    }

    pub(crate) fn appearance(&self) -> Element<'_, Message> {
        column![text("Appearance").size(18), self.theme_picker(), self.textures_panel()].spacing(10).into()
    }

    fn theme_picker(&self) -> Element<'_, Message> {
        let current = self.settings.theme;
        let cards = ThemeKind::ALL.iter().fold(row![].spacing(10), |r, &k| {
            let scene = k.scene().unwrap_or(omt_art::Scene::Night);
            let preview = stack![
                container(assets::banner(scene, 70.0)).width(150).height(70).clip(true),
                container(row![swatch(k, 0), swatch(k, 1), swatch(k, 2)].spacing(4)).padding(6),
            ];
            let label = text(k.to_string()).size(12);
            r.push(
                button(column![preview, label].spacing(6))
                    .padding(6)
                    .on_press(Message::SetTheme(k))
                    .style(style::button_style(if k == current { Btn::ChipOn } else { Btn::Card })),
            )
        });
        container(column![text("Theme").size(14), text("Applies instantly. Deepslate's Home banner follows the time of day.").size(11).color(style::muted()), cards.wrap()].spacing(8))
            .padding(12)
            .style(style::card())
            .into()
    }

    fn textures_panel(&self) -> Element<'_, Message> {
        let choice = &self.settings.texture_pack;
        let opt = |label: String, sub: String, c: TexChoice, on: bool| -> Element<'_, Message> {
            button(row![text(if on { "◉" } else { "○" }).size(14), column![text(label).size(13), text(sub).size(11).color(style::muted())].spacing(1)].spacing(10).align_y(Alignment::Center))
                .width(Length::Fill)
                .padding([6, 10])
                .on_press(Message::Tex(TexMsg::Choose(c)))
                .style(style::button_style(if on { Btn::ChipOn } else { Btn::Nav }))
                .into()
        };
        let newest = self.tex.found.first().map(|f| f.label.clone()).unwrap_or_else(|| "none found yet".into());
        let mut list = column![opt("Automatic".into(), format!("Newest found: {newest}"), TexChoice::Auto, *choice == TexChoice::Auto)].spacing(4);
        for f in &self.tex.found {
            let on = *choice == TexChoice::Pack(f.label.clone());
            list = list.push(opt(f.label.clone(), format!("{} pack layer(s) · {}", f.layers.len(), f.layers.first().map(|p| p.display().to_string()).unwrap_or_default()), TexChoice::Pack(f.label.clone()), on));
        }
        list = list.push(opt("Off".into(), "Use OMT's own procedural art only".into(), TexChoice::Off, *choice == TexChoice::Off));
        let download: Element<'_, Message> = match &self.tex.download {
            Some(p) => {
                let done = p.done.load(Ordering::Relaxed) as f32;
                let total = p.total.load(Ordering::Relaxed).max(1) as f32;
                row![
                    progress_bar(0.0..=total, done).length(Length::Fill).girth(8),
                    text(format!("{:.0} / {:.0} MB", done / 1_048_576.0, total / 1_048_576.0)).size(12),
                    button(text("Cancel").size(12)).on_press(Message::Tex(TexMsg::Cancel)).style(style::button_style(Btn::Danger)),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
            }
            None => row![
                button(text("Download vanilla textures…").size(12)).on_press(Message::Tex(TexMsg::Download)).style(style::button_style(Btn::Standard)),
                button(text("Rescan").size(12)).on_press(Message::Tex(TexMsg::Rescan)).style(style::button_style(Btn::Ghost)),
                space::horizontal(),
            ]
            .spacing(8)
            .into(),
        };
        let preview = row![
            assets::block(omt_art::Material::GrassTop, 48.0),
            assets::block(omt_art::Material::Planks, 48.0),
            assets::block(omt_art::Material::Copper, 48.0),
            assets::block(omt_art::Material::DiamondOre, 48.0),
            assets::block(omt_art::Material::Deepslate, 48.0),
        ]
        .spacing(8);
        container(
            column![
                row![column![text("Block Textures").size(14), text("Real Minecraft textures for icons, banners and the textured map view, read from the game installed on this computer (BedrockOnLinux, mcpelauncher, Windows). OMT never ships Mojang's files: the optional download fetches Mojang's official bedrock-samples pack (~170 MB, Minecraft EULA) into your cache and keeps only block textures.").size(11).color(style::muted())].spacing(4).width(Length::Fill), preview].spacing(16).align_y(Alignment::Center),
                list,
                download,
                text(self.tex.status.clone()).size(11).color(style::hint()),
            ]
            .spacing(10),
        )
        .padding(12)
        .style(style::card())
        .into()
    }
}

fn swatch<'a>(k: ThemeKind, i: u8) -> Element<'a, Message> {
    let t = style::tokens_of(k);
    let c: Color = match i {
        0 => t.bg,
        1 => t.surface_hi,
        _ => t.accent,
    };
    container(space::horizontal().width(14)).height(14).style(move |_| container::Style { background: Some(iced::Background::Color(c)), border: iced::border::rounded(3).color(Color::WHITE.scale_alpha(0.6)).width(1), ..Default::default() }).into()
}
