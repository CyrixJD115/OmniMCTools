//! Settings page (UMT Settings, minus account/updates/telemetry).

use iced::widget::{button, column, container, row, scrollable, slider, space, text, text_input, toggler};
use iced::{Element, Length};

use crate::settings::Settings;
use crate::style::{self, Btn};

#[derive(Debug, Clone)]
pub enum SettingsMsg {
    Scale(f32),
    WorkingDir(String),
    PickWorkingDir,
    WorkingDirPicked(Option<std::path::PathBuf>),
    AddFolder,
    FolderPicked(Option<std::path::PathBuf>),
    RemoveFolder(usize),
    AutoDetect(bool),
    Friendly(bool),
    NetherSlicer(i32),
    ConfirmPrune(bool),
    SkipPartial(bool),
    DebugLog(bool),
    ClearRecent,
    Save,
    Reset,
    Back,
}

fn item<'a>(title: &'a str, desc: &'a str, control: Element<'a, SettingsMsg>) -> Element<'a, SettingsMsg> {
    container(
        row![
            column![text(title).size(14), text(desc).size(11).color(style::MUTED)].spacing(2).width(Length::Fill),
            control,
        ]
        .spacing(16)
        .align_y(iced::Alignment::Center),
    )
    .padding(10)
    .style(style::card())
    .into()
}

pub fn view<'a>(s: &'a Settings, dirty: bool) -> Element<'a, SettingsMsg> {
    let wd = s.working_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    let folders = s.world_folders.iter().enumerate().fold(column![].spacing(4), |c, (i, f)| {
        c.push(
            row![
                text(f.display().to_string()).size(12).width(Length::Fill),
                button(text("Remove").size(11)).on_press(SettingsMsg::RemoveFolder(i)).style(style::button_style(Btn::Ghost)),
            ]
            .align_y(iced::Alignment::Center),
        )
    });
    let general = column![
        text("General").size(18),
        item("Scale Factor", "Change the scale of the app, up to 200%.", row![
            slider(0.75..=2.0, s.scale_factor, SettingsMsg::Scale).step(0.05_f32).width(200),
            text(format!("{:.0}%", s.scale_factor * 100.0)).size(12).width(48),
        ].spacing(8).align_y(iced::Alignment::Center).into()),
        item(
            "Working Directory",
            "Where world copies (workspaces) are kept while you edit. Empty = default. Needs free space ≥ the world size.",
            row![
                text_input("default", &wd).on_input(SettingsMsg::WorkingDir).width(260),
                button(text("Browse…").size(12)).on_press(SettingsMsg::PickWorkingDir).style(style::button_style(Btn::Standard)),
            ].spacing(6).into(),
        ),
        item("Detect Game Folders", "Scan the standard Java and Bedrock world locations of this computer.", toggler(s.auto_detect_folders).on_toggle(SettingsMsg::AutoDetect).into()),
        container(column![
            row![
                column![text("World Folders").size(14), text("Extra folders to list worlds from (a saves folder, minecraftWorlds, or any folder of worlds).").size(11).color(style::MUTED)].width(Length::Fill),
                button(text("Add folder…").size(12)).on_press(SettingsMsg::AddFolder).style(style::button_style(Btn::Standard)),
            ].align_y(iced::Alignment::Center),
            folders,
        ].spacing(6)).padding(10).style(style::card()),
        item("Friendly Names", "Show readable labels and descriptions next to raw NBT keys in the editor.", toggler(s.friendly_names).on_toggle(SettingsMsg::Friendly).into()),
        item("Copy Worlds", "Worlds are always opened as a copy; nothing touches the original until you Save (Bedrock LevelDB compaction makes direct editing unsafe).", text("Always").size(12).color(style::MUTED).into()),
        item("Debug Log", "Write a diagnostic log next to the settings file.", toggler(s.debug_log).on_toggle(SettingsMsg::DebugLog).into()),
    ]
    .spacing(8);
    let viewer = column![
        text("World Viewer & Pruner").size(18),
        item("Nether Slicer", "Default Y level the Nether map starts below (its bedrock roof hides everything).", row![
            slider(32..=127, s.nether_slicer, SettingsMsg::NetherSlicer).width(200),
            text(format!("Y {}", s.nether_slicer)).size(12).width(48),
        ].spacing(8).align_y(iced::Alignment::Center).into()),
        item("Confirm Before Pruning", "Ask for confirmation before chunks are deleted.", toggler(s.confirm_prune).on_toggle(SettingsMsg::ConfirmPrune).into()),
    ]
    .spacing(8);
    let converter = column![
        text("Converter Defaults").size(18),
        item("Skip Partially-Generated Chunks", "Removes chunks that are not fully generated for a more seamless world.", toggler(s.skip_partial_chunks).on_toggle(SettingsMsg::SkipPartial).into()),
    ]
    .spacing(8);
    let about = column![
        text("About").size(18),
        text(format!("OmniMCTools {} — independent, clean-room, open source (GPL-3.0-or-later). Inspired by Universal Minecraft Tool; not affiliated.", env!("CARGO_PKG_VERSION"))).size(12).color(style::MUTED),
        text(format!("Settings file: {}", Settings::path().display())).size(11).color(style::HINT),
        row![button(text("Clear recent worlds").size(12)).on_press(SettingsMsg::ClearRecent).style(style::button_style(Btn::Ghost))],
    ]
    .spacing(6);
    let bar = row![
        button(text("← Back").size(13)).on_press(SettingsMsg::Back).style(style::button_style(Btn::Ghost)),
        text("Settings").size(22),
        space::horizontal(),
        button(text("Reset to defaults").size(12)).on_press(SettingsMsg::Reset).style(style::button_style(Btn::Standard)),
        button(text(if dirty { "Save settings" } else { "Saved" }).size(13))
            .on_press_maybe(dirty.then_some(SettingsMsg::Save))
            .style(style::button_style(Btn::Confirm))
            .padding([8, 14]),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);
    column![
        container(bar).padding([12, 20]),
        scrollable(column![general, viewer, converter, about].spacing(22).padding([0, 20]).max_width(900)).height(Length::Fill),
    ]
    .into()
}

/// Apply a settings message; returns true when the draft changed.
pub fn apply(s: &mut Settings, msg: &SettingsMsg) -> bool {
    match msg {
        SettingsMsg::Scale(v) => s.scale_factor = *v,
        SettingsMsg::WorkingDir(v) => s.working_dir = (!v.trim().is_empty()).then(|| v.into()),
        SettingsMsg::WorkingDirPicked(Some(p)) => s.working_dir = Some(p.clone()),
        SettingsMsg::FolderPicked(Some(p)) => {
            if !s.world_folders.contains(p) {
                s.world_folders.push(p.clone());
            }
        }
        SettingsMsg::RemoveFolder(i) => {
            if *i < s.world_folders.len() {
                s.world_folders.remove(*i);
            }
        }
        SettingsMsg::AutoDetect(v) => s.auto_detect_folders = *v,
        SettingsMsg::Friendly(v) => s.friendly_names = *v,
        SettingsMsg::NetherSlicer(v) => s.nether_slicer = *v,
        SettingsMsg::ConfirmPrune(v) => s.confirm_prune = *v,
        SettingsMsg::SkipPartial(v) => s.skip_partial_chunks = *v,
        SettingsMsg::DebugLog(v) => s.debug_log = *v,
        SettingsMsg::ClearRecent => s.recent.clear(),
        SettingsMsg::Reset => *s = Settings::default(),
        _ => return false,
    }
    true
}
