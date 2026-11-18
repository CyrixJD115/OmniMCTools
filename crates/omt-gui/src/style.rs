//! OMT's design system: theme presets (Deepslate, Overworld, Nether, End,
//! Classic) as color tokens, plus the component styles built on them.
//!
//! Look: a modern take on Minecraft's in-game UI — solid block buttons
//! with a hard darker "lip" under them that collapses when pressed, crisp
//! 1-px borders, small radii, soft elevation on cards, gradient top bar.
//! Colors are named after Minecraft materials (grass accent, diamond info,
//! gold warning, redstone danger) but are OMT's own palette.
//!
//! Tokens are read through functions (`fg()`, `muted()`, …) so switching
//! the theme in Settings re-skins everything on the next frame.

use std::sync::RwLock;

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector, border, gradient};
use serde::{Deserialize, Serialize};

const fn rgb(v: u32) -> Color {
    Color::from_rgb((((v >> 16) & 0xff) as f32) / 255.0, (((v >> 8) & 0xff) as f32) / 255.0, ((v & 0xff) as f32) / 255.0)
}

/// Theme presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeKind {
    /// Dark stone — the default.
    #[default]
    Deepslate,
    /// Light, daylight greens.
    Overworld,
    /// Crimson and lava.
    Nether,
    /// Void purple and amethyst.
    End,
    /// UMT-like slate with blue accents.
    Classic,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 5] = [Self::Deepslate, Self::Overworld, Self::Nether, Self::End, Self::Classic];

    /// Hero-banner scene of this theme (`None` = follows the time of day).
    pub fn scene(self) -> Option<omt_art::Scene> {
        match self {
            Self::Deepslate => None,
            Self::Overworld | Self::Classic => Some(omt_art::Scene::Day),
            Self::Nether => Some(omt_art::Scene::Nether),
            Self::End => Some(omt_art::Scene::End),
        }
    }
}

impl std::fmt::Display for ThemeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Deepslate => "Deepslate (dark)",
            Self::Overworld => "Overworld (light)",
            Self::Nether => "Nether",
            Self::End => "The End",
            Self::Classic => "Classic",
        })
    }
}

/// Color tokens of one theme.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    /// Page background.
    pub bg: Color,
    /// Side rail / top bar base.
    pub nav: Color,
    /// Cards and panels.
    pub surface: Color,
    /// Hovered / selected surfaces.
    pub surface_hi: Color,
    /// Inputs and wells.
    pub well: Color,
    pub border: Color,
    pub border_hi: Color,
    pub fg: Color,
    pub muted: Color,
    pub hint: Color,
    /// Primary call-to-action fill + its pressed lip.
    pub accent: Color,
    pub accent_hi: Color,
    pub accent_lip: Color,
    /// Focus rings, selection, links ("diamond").
    pub info: Color,
    pub warn: Color,
    pub danger: Color,
    pub danger_lip: Color,
    /// Standard button fill + lip.
    pub button: Color,
    pub button_hi: Color,
    pub button_lip: Color,
    /// Text on accent fills.
    pub on_accent: Color,
    /// Map canvas backdrop.
    pub map_bg: Color,
    pub dark: bool,
}

const DEEPSLATE: Tokens = Tokens {
    bg: rgb(0x17181c),
    nav: rgb(0x111215),
    surface: rgb(0x1f2126),
    surface_hi: rgb(0x2a2d34),
    well: rgb(0x131418),
    border: rgb(0x2e3138),
    border_hi: rgb(0x4a4e58),
    fg: rgb(0xeceef1),
    muted: rgb(0xa3a8b2),
    hint: rgb(0x6b717c),
    accent: rgb(0x4aa632),
    accent_hi: rgb(0x5fc044),
    accent_lip: rgb(0x23561a),
    info: rgb(0x3fc8d8),
    warn: rgb(0xf2c14e),
    danger: rgb(0xd94444),
    danger_lip: rgb(0x6e1d1d),
    button: rgb(0x34373f),
    button_hi: rgb(0x40444d),
    button_lip: rgb(0x1a1c20),
    on_accent: rgb(0xffffff),
    map_bg: rgb(0x0f1013),
    dark: true,
};

const OVERWORLD: Tokens = Tokens {
    bg: rgb(0xe8ebe3),
    nav: rgb(0xd5dacb),
    surface: rgb(0xf7f8f3),
    surface_hi: rgb(0xffffff),
    well: rgb(0xffffff),
    border: rgb(0xc2c8b7),
    border_hi: rgb(0x8e9783),
    fg: rgb(0x1b1f19),
    muted: rgb(0x4f564b),
    hint: rgb(0x858c80),
    accent: rgb(0x3c8527),
    accent_hi: rgb(0x4a9c32),
    accent_lip: rgb(0x1d4d13),
    info: rgb(0x16829a),
    warn: rgb(0xa86a00),
    danger: rgb(0xc23434),
    danger_lip: rgb(0x6e1414),
    button: rgb(0xd0d1d4),
    button_hi: rgb(0xe0e1e4),
    button_lip: rgb(0x58585a),
    on_accent: rgb(0xffffff),
    map_bg: rgb(0x2a2d27),
    dark: false,
};

const NETHER: Tokens = Tokens {
    bg: rgb(0x1a1113),
    nav: rgb(0x120b0c),
    surface: rgb(0x241619),
    surface_hi: rgb(0x321d22),
    well: rgb(0x140b0d),
    border: rgb(0x3e252b),
    border_hi: rgb(0x63383f),
    fg: rgb(0xf3e9e6),
    muted: rgb(0xc0a5a0),
    hint: rgb(0x80655f),
    accent: rgb(0xd9622b),
    accent_hi: rgb(0xef7a3c),
    accent_lip: rgb(0x6b2a10),
    info: rgb(0xf2c14e),
    warn: rgb(0xf2c14e),
    danger: rgb(0xe23b4e),
    danger_lip: rgb(0x6e1420),
    button: rgb(0x3c252a),
    button_hi: rgb(0x4a2e34),
    button_lip: rgb(0x1a0d10),
    on_accent: rgb(0xffffff),
    map_bg: rgb(0x0f0809),
    dark: true,
};

const END: Tokens = Tokens {
    bg: rgb(0x120f19),
    nav: rgb(0x0c0a11),
    surface: rgb(0x1b1725),
    surface_hi: rgb(0x262033),
    well: rgb(0x0e0b14),
    border: rgb(0x312a42),
    border_hi: rgb(0x524670),
    fg: rgb(0xf0ecf7),
    muted: rgb(0xb0a6c4),
    hint: rgb(0x6f6585),
    accent: rgb(0x8a62d6),
    accent_hi: rgb(0xa07be0),
    accent_lip: rgb(0x3d2470),
    info: rgb(0x6fe0c8),
    warn: rgb(0xe8dd8a),
    danger: rgb(0xe0566a),
    danger_lip: rgb(0x6a1a2a),
    button: rgb(0x2e2740),
    button_hi: rgb(0x3a3150),
    button_lip: rgb(0x120e1a),
    on_accent: rgb(0xffffff),
    map_bg: rgb(0x0a0810),
    dark: true,
};

const CLASSIC: Tokens = Tokens {
    bg: rgb(0x1d2128),
    nav: rgb(0x16181e),
    surface: rgb(0x232830),
    surface_hi: rgb(0x2d333e),
    well: rgb(0x16181e),
    border: rgb(0x2c323a),
    border_hi: rgb(0x474e5a),
    fg: rgb(0xe8eaed),
    muted: rgb(0x9aa1ad),
    hint: rgb(0x66707e),
    accent: rgb(0x228bf6),
    accent_hi: rgb(0x3f9cfa),
    accent_lip: rgb(0x0f4d8f),
    info: rgb(0x228bf6),
    warn: rgb(0xe0a030),
    danger: rgb(0xc04040),
    danger_lip: rgb(0x5a1d1d),
    button: rgb(0x2d333e),
    button_hi: rgb(0x384050),
    button_lip: rgb(0x14171c),
    on_accent: rgb(0xffffff),
    map_bg: rgb(0x121418),
    dark: true,
};

static THEME: RwLock<ThemeKind> = RwLock::new(ThemeKind::Deepslate);

/// Switch the active theme (startup + Settings).
pub fn set_theme(k: ThemeKind) {
    if let Ok(mut t) = THEME.write() {
        *t = k;
    }
}

pub fn theme_kind() -> ThemeKind {
    THEME.read().map(|t| *t).unwrap_or_default()
}

/// Tokens of the active theme.
pub fn t() -> Tokens {
    tokens_of(theme_kind())
}

/// Tokens of any theme (previews).
pub fn tokens_of(k: ThemeKind) -> Tokens {
    match k {
        ThemeKind::Deepslate => DEEPSLATE,
        ThemeKind::Overworld => OVERWORLD,
        ThemeKind::Nether => NETHER,
        ThemeKind::End => END,
        ThemeKind::Classic => CLASSIC,
    }
}

pub fn fg() -> Color {
    t().fg
}
pub fn muted() -> Color {
    t().muted
}
pub fn hint() -> Color {
    t().hint
}
pub fn accent() -> Color {
    t().accent
}
pub fn info() -> Color {
    t().info
}
pub fn success() -> Color {
    t().accent
}
pub fn warn() -> Color {
    t().warn
}
pub fn danger() -> Color {
    t().danger
}

/// Button variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    /// Stone-grey block button.
    Standard,
    /// Accent call-to-action (grass green by default).
    Primary,
    /// Same as Primary (save / convert / open).
    Confirm,
    /// Redstone destructive action.
    Danger,
    /// Borderless — toolbar chips, tab closers.
    Ghost,
    /// Side-rail item (inactive).
    Nav,
    /// Side-rail item (active page).
    NavActive,
    /// Filter chip, off.
    Chip,
    /// Filter chip, on.
    ChipOn,
    /// A clickable card (world tiles, tool cards).
    Card,
    /// Map tool-rail button (square, quiet until hovered).
    Rail,
    /// Map tool-rail button, active tool / layer on.
    RailOn,
}

fn lip(fill: Color, hover: Color, lip: Color, status: button::Status, radius: f32) -> button::Style {
    let (bg, off) = match status {
        button::Status::Hovered => (hover, 3.0),
        button::Status::Pressed => (fill, 1.0),
        _ => (fill, 3.0),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: t().fg,
        border: border::rounded(radius).color(lip).width(1),
        shadow: Shadow { color: lip, offset: Vector::new(0.0, off), blur_radius: 0.0 },
        snap: true,
    }
}

pub fn button_style(kind: Btn) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let k = t();
        if matches!(status, button::Status::Disabled) && matches!(kind, Btn::Rail | Btn::RailOn) {
            return button::Style { text_color: k.hint, border: border::rounded(5), ..Default::default() };
        }
        if matches!(status, button::Status::Disabled) && !matches!(kind, Btn::Ghost | Btn::Nav | Btn::NavActive) {
            return button::Style {
                background: Some(Background::Color(k.surface)),
                text_color: k.hint,
                border: border::rounded(4).color(k.border).width(1),
                shadow: Shadow::default(),
                snap: true,
            };
        }
        let hovered = matches!(status, button::Status::Hovered);
        match kind {
            Btn::Standard => lip(k.button, k.button_hi, k.button_lip, status, 4.0),
            Btn::Primary | Btn::Confirm => {
                let mut s = lip(k.accent, k.accent_hi, k.accent_lip, status, 4.0);
                s.text_color = k.on_accent;
                s
            }
            Btn::Danger => {
                let mut s = lip(k.danger, mix(k.danger, Color::WHITE, 0.12), k.danger_lip, status, 4.0);
                s.text_color = Color::WHITE;
                s
            }
            Btn::Ghost => button::Style {
                background: hovered.then_some(Background::Color(k.surface_hi)),
                text_color: if hovered { k.fg } else { k.muted },
                border: border::rounded(4),
                ..Default::default()
            },
            Btn::Nav => button::Style {
                background: hovered.then_some(Background::Color(k.surface)),
                text_color: if hovered { k.fg } else { k.muted },
                border: border::rounded(6),
                ..Default::default()
            },
            Btn::NavActive => button::Style {
                background: Some(Background::Color(k.surface_hi)),
                text_color: k.fg,
                border: border::rounded(6).color(k.border).width(1),
                ..Default::default()
            },
            Btn::Chip => button::Style {
                background: Some(Background::Color(if hovered { k.surface_hi } else { k.surface })),
                text_color: if hovered { k.fg } else { k.muted },
                border: border::rounded(14).color(k.border).width(1),
                ..Default::default()
            },
            Btn::ChipOn => button::Style {
                background: Some(Background::Color(k.info.scale_alpha(0.18))),
                text_color: k.fg,
                border: border::rounded(14).color(k.info).width(1),
                ..Default::default()
            },
            Btn::Card => button::Style {
                background: Some(Background::Color(if hovered { k.surface_hi } else { k.surface })),
                text_color: k.fg,
                border: border::rounded(8).color(if hovered { k.border_hi } else { k.border }).width(1),
                shadow: Shadow { color: Color::BLACK.scale_alpha(if hovered { 0.45 } else { 0.25 }), offset: Vector::new(0.0, if hovered { 6.0 } else { 2.0 }), blur_radius: if hovered { 16.0 } else { 6.0 } },
                snap: true,
            },
            Btn::Rail => button::Style {
                background: hovered.then_some(Background::Color(k.surface_hi)),
                text_color: if hovered { k.fg } else { k.muted },
                border: border::rounded(5).color(if hovered { k.border } else { Color::TRANSPARENT }).width(1),
                ..Default::default()
            },
            Btn::RailOn => button::Style {
                background: Some(Background::Color(k.info.scale_alpha(if hovered { 0.28 } else { 0.20 }))),
                text_color: k.fg,
                border: border::rounded(5).color(k.info).width(1),
                ..Default::default()
            },
        }
    }
}

fn mix(a: Color, b: Color, f: f32) -> Color {
    Color::from_rgba(a.r + (b.r - a.r) * f, a.g + (b.g - a.g) * f, a.b + (b.b - a.b) * f, a.a)
}

/// Elevated card.
pub fn card() -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let k = t();
        container::Style {
            background: Some(Background::Color(k.surface)),
            text_color: Some(k.fg),
            border: border::rounded(8).color(k.border).width(1),
            shadow: Shadow { color: Color::BLACK.scale_alpha(0.25), offset: Vector::new(0.0, 2.0), blur_radius: 6.0 },
            snap: true,
        }
    }
}

/// Flat card (no blur shadow) for dense side panels.
pub fn card_flat() -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let k = t();
        container::Style { background: Some(Background::Color(k.surface)), text_color: Some(k.fg), border: border::rounded(8).color(k.border).width(1), ..Default::default() }
    }
}

/// Selected card / active chip (info-colored border).
pub fn card_selected() -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let k = t();
        container::Style {
            background: Some(Background::Color(k.surface_hi)),
            text_color: Some(k.fg),
            border: border::rounded(6).color(k.info).width(1),
            ..Default::default()
        }
    }
}

/// Text input (well background, diamond focus ring).
pub fn input() -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_theme, status| {
        let k = t();
        let line = match status {
            text_input::Status::Focused { .. } => k.info,
            text_input::Status::Hovered => k.border_hi,
            _ => k.border,
        };
        text_input::Style { background: Background::Color(k.well), border: border::rounded(6).color(line).width(1), icon: k.muted, placeholder: k.hint, value: k.fg, selection: k.info.scale_alpha(0.35) }
    }
}

/// Flat panel (session headers, side panels).
pub fn panel() -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let k = t();
        container::Style { background: Some(Background::Color(k.surface)), text_color: Some(k.fg), border: Border { color: k.border, width: 1.0, radius: 0.0.into() }, ..Default::default() }
    }
}

/// The side navigation rail.
pub fn rail() -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let k = t();
        container::Style { background: Some(Background::Color(k.nav)), text_color: Some(k.fg), border: Border { color: k.border, width: 1.0, radius: 0.0.into() }, ..Default::default() }
    }
}

/// The top navigation bar: a subtle vertical gradient over the nav color.
pub fn topbar() -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let k = t();
        let g = gradient::Linear::new(std::f32::consts::PI).add_stop(0.0, mix(k.nav, k.surface, 0.55)).add_stop(1.0, k.nav);
        container::Style { background: Some(Background::Gradient(g.into())), text_color: Some(k.fg), border: Border { color: k.border, width: 1.0, radius: 0.0.into() }, ..Default::default() }
    }
}

/// Page background.
pub fn page() -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style { background: Some(Background::Color(t().bg)), text_color: Some(t().fg), ..Default::default() }
}

/// Small colored badge (edition / launcher / status pills).
pub fn badge(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(color.scale_alpha(0.18))),
        text_color: Some(mix(color, t().fg, if t().dark { 0.25 } else { 0.0 })),
        border: border::rounded(10).color(color.scale_alpha(0.55)).width(1),
        ..Default::default()
    }
}

/// Solid badge for use on top of images.
pub fn badge_solid(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(mix(color, Color::BLACK, 0.25))),
        text_color: Some(Color::WHITE),
        border: border::rounded(10).color(mix(color, Color::WHITE, 0.3)).width(1),
        shadow: Shadow { color: Color::BLACK.scale_alpha(0.4), offset: Vector::new(0.0, 1.0), blur_radius: 3.0 },
        snap: true,
    }
}

/// Semi-transparent scrim over art (hero text, map HUD).
pub fn scrim(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(Color::BLACK.scale_alpha(alpha))),
        text_color: Some(Color::WHITE),
        border: border::rounded(8),
        ..Default::default()
    }
}

/// The "ALPHA" pill for experimental features.
pub fn alpha_badge<'a, M: 'a>() -> iced::Element<'a, M> {
    container(iced::widget::text("ALPHA").size(10).font(BOLD)).padding([1, 7]).style(badge(rgb(0xf2913d))).into()
}

/// Edition badge colors.
pub fn java_color() -> Color {
    rgb(0xe07b39)
}
pub fn bedrock_color() -> Color {
    rgb(0x5aa9e6)
}

pub(crate) fn version_label(w: &omt_world::WorldSummary) -> String {
    if let Some(dv) = w.java_data_version {
        match crate::dictionary::data_version_name(i64::from(dv)) {
            Some(name) => format!("{name} · DataVersion {dv}"),
            None => format!("DataVersion {dv}"),
        }
    } else if let Some(v) = w.bedrock_last_opened {
        format!("{}.{}.{}", v[0], v[1], v[2])
    } else {
        "unknown version".into()
    }
}

/// The iced theme for built-in widgets (pick lists, togglers, sliders,
/// scrollbars) derived from the active tokens.
pub(crate) fn omnitheme() -> Theme {
    let k = t();
    Theme::custom("OmniMCTools", iced::theme::Palette { background: k.bg, text: k.fg, primary: k.info, success: k.accent, warning: k.warn, danger: k.danger })
}

/// Bold variant of the default font (headings).
pub const BOLD: iced::Font = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
