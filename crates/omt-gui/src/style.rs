//! Shared component styling. Palette comes from UMT's recovered QSS tokens
//! (window #1d2128, panels #232830, inputs/checked #16181e, borders #2c323a,
//! hover #2d333e/#474e5a, accents #228bf6/#43a975/#c04040) — see
//! `Source/RE/UMT_Research_Report.md` §theme.

use iced::widget::{button, container, text_input};
use iced::{border, Background, Color, Theme};

pub const BG: Color = rgb(0x1d, 0x21, 0x28);
pub const PANEL: Color = rgb(0x23, 0x28, 0x30);
pub const PANEL_DEEP: Color = rgb(0x16, 0x18, 0x1e);
pub const BORDER: Color = rgb(0x2c, 0x32, 0x3a);
pub const HOVER: Color = rgb(0x2d, 0x33, 0x3e);
pub const HOVER_BORDER: Color = rgb(0x47, 0x4e, 0x5a);
pub const FG: Color = rgb(0xe8, 0xea, 0xed);
pub const MUTED: Color = rgb(0x9a, 0xa1, 0xad);
pub const ACCENT: Color = rgb(0x22, 0x8b, 0xf6);
pub const ACCENT_HOVER: Color = rgb(0x3f, 0x9c, 0xfa);
pub const SUCCESS: Color = rgb(0x43, 0xa9, 0x75);
pub const SUCCESS_HOVER: Color = rgb(0x5b, 0xc7, 0x90);
pub const WARN: Color = rgb(0xe0, 0xa0, 0x30);
pub const DANGER: Color = rgb(0xc0, 0x40, 0x40);
pub const DANGER_HOVER: Color = rgb(0xd0, 0x54, 0x54);
/// UMT's disabled/hint grey (#66707e).
pub const HINT: Color = rgb(0x66, 0x70, 0x7e);

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

/// Button variants matching UMT's `CustomButton` classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    /// Default panel-grey action button.
    Standard,
    /// Accent-blue call-to-action.
    Primary,
    /// Green confirm button (UMT's save/convert color).
    Confirm,
    /// Red destructive action.
    Danger,
    /// Borderless, muted — tab-bar and toolbar-chip buttons.
    Ghost,
}

pub fn button_style(kind: Btn) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let enabled = !matches!(status, button::Status::Disabled);
        let (bg, fg, line) = match (kind, status) {
            (Btn::Standard, button::Status::Hovered) => (HOVER, FG, HOVER_BORDER),
            (Btn::Standard, button::Status::Pressed) => (PANEL_DEEP, FG, HOVER_BORDER),
            (Btn::Standard, _) => (PANEL, FG, BORDER),
            (Btn::Primary, button::Status::Hovered) => (ACCENT_HOVER, Color::WHITE, ACCENT_HOVER),
            (Btn::Primary, button::Status::Pressed) => (ACCENT, Color::WHITE, ACCENT),
            (Btn::Primary, _) => (ACCENT, Color::WHITE, ACCENT),
            (Btn::Confirm, button::Status::Hovered) => (SUCCESS_HOVER, Color::WHITE, SUCCESS_HOVER),
            (Btn::Confirm, button::Status::Pressed) => (SUCCESS, Color::WHITE, SUCCESS),
            (Btn::Confirm, _) => (SUCCESS, Color::WHITE, SUCCESS),
            (Btn::Danger, button::Status::Hovered) => (DANGER_HOVER, Color::WHITE, DANGER_HOVER),
            (Btn::Danger, button::Status::Pressed) => (DANGER, Color::WHITE, DANGER),
            (Btn::Danger, _) => (DANGER, Color::WHITE, DANGER),
            (Btn::Ghost, button::Status::Hovered) => (PANEL, FG, BORDER),
            (Btn::Ghost, _) => (iced::Color::TRANSPARENT, MUTED, iced::Color::TRANSPARENT),
        };
        let (bg, fg) = if enabled { (bg, fg) } else { (PANEL, rgb(0x66, 0x70, 0x7e)) };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: border::rounded(6).color(line).width(1),
            ..Default::default()
        }
    }
}

/// Rounded panel card (world entries, editor toolbar, selection panel boxes).
pub fn card() -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(PANEL)),
        text_color: Some(FG),
        border: border::rounded(8).color(BORDER).width(1),
        ..Default::default()
    }
}

/// Active-tab / selected-chip style (UMT: `CustomButton[checked]` — deep bg +
/// accent border).
pub fn card_selected() -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(PANEL_DEEP)),
        text_color: Some(FG),
        border: border::rounded(6).color(ACCENT).width(1),
        ..Default::default()
    }
}

/// Dark text input (UMT `QLineEdit`: #16181e bg, border, accent on focus).
pub fn input() -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_theme, status| {
        let line = match status {
            text_input::Status::Focused { .. } => ACCENT,
            text_input::Status::Hovered => HOVER_BORDER,
            _ => BORDER,
        };
        text_input::Style {
            background: Background::Color(PANEL_DEEP),
            border: border::rounded(6).color(line).width(1),
            icon: MUTED,
            placeholder: rgb(0x70, 0x78, 0x85),
            value: FG,
            selection: ACCENT.scale_alpha(0.4),
        }
    }
}
