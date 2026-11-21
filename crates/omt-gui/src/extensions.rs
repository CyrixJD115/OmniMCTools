//! Extensions page: built-in (embedded) extensions + install (zip),
//! enable/disable, reorder, remove, and the master switch. The manager
//! itself lives in `omt-script`; this module is the page's view + message
//! handling, routed from `main`.

use iced::widget::{button, column, container, row, scrollable, text, toggler};
use iced::{Element, Length};
use omt_script::Manager;

use crate::style::{self, Btn};
use crate::Message;

#[derive(Debug, Clone)]
pub enum ExtMsg {
    Back,
    PickZip,
    ZipPicked(Option<std::path::PathBuf>),
    InstallExample,
    Toggle(String),
    Up(String),
    Down(String),
    Remove(String),
    /// After any mutation: existing editor tabs re-run via manager revision.
    Reloaded,
    /// Create a new extension from the template (and open its folder).
    NewExtension,
    /// Re-read every extension from disk (after editing files).
    ReloadAll,
    /// Open an extension's folder (`None` = the extensions folder).
    OpenFolder(Option<String>),
    /// Install a native plugin (a compiled dynamic library).
    PickPlugin,
    PluginPicked(Option<std::path::PathBuf>),
}

/// Copy a dynamic library into its own folder under the extensions
/// directory, so it is discovered as a native plugin.
fn install_plugin(mgr: &Manager, path: &std::path::Path) -> Result<String, String> {
    let ext = path.extension().and_then(|x| x.to_str()).unwrap_or_default();
    if !matches!(ext, "so" | "dll" | "dylib") {
        return Err("not a dynamic library (.so / .dll / .dylib)".into());
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("plugin").trim_start_matches("lib");
    let id: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
    let id = id.trim_matches('-').to_string();
    let id = if id.is_empty() { "plugin".to_string() } else { id };
    let dir = mgr.root().join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::copy(path, dir.join(path.file_name().unwrap_or_default())).map_err(|e| e.to_string())?;
    mgr.reload();
    mgr.set_enabled(&id, true).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Open a folder in the system file manager.
pub fn open_folder(p: std::path::PathBuf) {
    // Never launch a file manager from unit tests.
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        let _ = std::process::Command::new(if cfg!(target_os = "windows") { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(p).spawn();
    });
}

/// Applies a message to the shared manager; returns a status line (and
/// whether the list changed, so the caller can refresh).
pub fn handle(mgr: &Manager, msg: &ExtMsg) -> (String, bool) {
    match msg {
        ExtMsg::Back => (String::new(), false),
        ExtMsg::PickZip | ExtMsg::ZipPicked(None) => (String::new(), false),
        ExtMsg::ZipPicked(Some(path)) => match mgr.install_zip(path) {
            Ok(id) => (format!("installed extension {id:?} from {}", path.display()), true),
            Err(e) => (format!("install failed: {e}"), false),
        },
        ExtMsg::InstallExample => {
            let r = mgr
                .install_files("ticks-time", &omt_script::example_files(), None)
                .and_then(|()| mgr.set_enabled("ticks-time", true))
                .and_then(|()| mgr.install_files("chest-explorer", &omt_extensions::example_files(), None))
                .and_then(|()| mgr.set_enabled("chest-explorer", true));
            match r {
                Ok(()) => ("installed the built-in examples: Chest Explorer (tools in the sidebar, “Open as chest” in the NBT editor) and Tick Clock".into(), true),
                Err(e) => (format!("could not install the examples: {e}"), false),
            }
        }
        ExtMsg::Toggle(id) => {
            let installed = mgr.records().iter().any(|r| r.id == *id);
            if installed {
                let next = !mgr.is_enabled(id);
                match mgr.set_enabled(id, next) {
                    Ok(()) => {
                        let verb = if next { "enabled" } else { "disabled" };
                        (format!("{verb} {id}"), true)
                    }
                    Err(e) => (format!("{e}"), false),
                }
            } else if let Some(b) = omt_extensions::builtins().iter().find(|b| b.id == id) {
                // A shipped built-in that isn't on disk yet: materialize its
                // embedded files (so they stay inspectable/editable), then
                // enable. Nothing was written before this.
                let r = mgr.install_files(b.id, &(b.files)(), None).and_then(|()| mgr.set_enabled(b.id, true));
                match r {
                    Ok(()) => (format!("enabled built-in {id}"), true),
                    Err(e) => (format!("could not enable built-in {id}: {e}"), false),
                }
            } else {
                (format!("no extension {id:?}"), false)
            }
        }
        ExtMsg::Up(id) => match mgr.reorder(id, -1) {
            Ok(()) => (String::new(), true),
            Err(e) => (format!("{e}"), false),
        },
        ExtMsg::Down(id) => match mgr.reorder(id, 1) {
            Ok(()) => (String::new(), true),
            Err(e) => (format!("{e}"), false),
        },
        ExtMsg::Remove(id) => match mgr.remove(id) {
            Ok(()) => (format!("removed {id}"), true),
            Err(e) => (format!("{e}"), false),
        },
        ExtMsg::Reloaded => (String::new(), false),
        ExtMsg::NewExtension => {
            let taken = |id: &str| mgr.root().join(id).exists();
            let n = (1..).find(|n| !taken(&if *n == 1 { "my-extension".to_string() } else { format!("my-extension-{n}") })).unwrap_or(1);
            let (id, name) = if n == 1 { ("my-extension".to_string(), "My Extension".to_string()) } else { (format!("my-extension-{n}"), format!("My Extension {n}")) };
            match mgr.install_files(&id, &omt_extensions::template_files(&name), None).and_then(|()| mgr.set_enabled(&id, true)) {
                Ok(()) => {
                    open_folder(mgr.root().join(&id));
                    (format!("created “{name}” in {} — its tool is in the sidebar; edit main.lua, then press Reload", mgr.root().join(&id).display()), true)
                }
                Err(e) => (format!("could not create the extension: {e}"), false),
            }
        }
        ExtMsg::ReloadAll => {
            mgr.reload();
            (format!("reloaded {} extension(s) from disk — open tools: press Reload on their tab", mgr.records().len()), true)
        }
        ExtMsg::PickPlugin | ExtMsg::PluginPicked(None) => (String::new(), false),
        ExtMsg::PluginPicked(Some(path)) => match install_plugin(mgr, path) {
            Ok(id) => (format!("installed native plugin {id:?} — its tool is in the sidebar"), true),
            Err(e) => (format!("could not install the plugin: {e}"), false),
        },
        ExtMsg::OpenFolder(id) => {
            let p = id.as_ref().map_or_else(|| mgr.root().to_path_buf(), |id| mgr.root().join(id));
            let _ = std::fs::create_dir_all(&p);
            open_folder(p);
            (String::new(), false)
        }
    }
}

pub fn view<'a>(mgr: &'a Manager, extensions_on: bool, status: &'a str, hub: &'a crate::ext_tools::ExtHub) -> Element<'a, Message> {
    // The description column takes the remaining width (so it wraps) and
    // the buttons keep their natural size at the right edge.
    let header = row![
        column![
            text("🧩 Extensions").size(22),
            text("Lua extensions add tool tabs (with their own UI, world / NBT / item access), NBT editor actions, Map Viewer panels and NBT editor row annotations. Extensions run with full trust — install only from sources you trust.")
                .size(12)
                .color(style::muted()),
        ]
        .spacing(4)
        .width(Length::FillPortion(1)),
        button(text("Install from .zip…").size(13))
            .on_press(Message::Ext(ExtMsg::PickZip))
            .style(style::button_style(Btn::Primary))
            .padding([8, 12]),
        button(text("Install examples").size(13))
            .on_press(Message::Ext(ExtMsg::InstallExample))
            .style(style::button_style(Btn::Standard))
            .padding([8, 12]),
        button(text("New extension…").size(13)).on_press(Message::Ext(ExtMsg::NewExtension)).style(style::button_style(Btn::Standard)).padding([8, 12]),
        button(text("Install plugin…").size(13)).on_press(Message::Ext(ExtMsg::PickPlugin)).style(style::button_style(Btn::Standard)).padding([8, 12]),
        button(text("Reload all").size(13)).on_press(Message::Ext(ExtMsg::ReloadAll)).style(style::button_style(Btn::Ghost)).padding([8, 12]),
        button(text("Folder").size(13)).on_press(Message::Ext(ExtMsg::OpenFolder(None))).style(style::button_style(Btn::Ghost)).padding([8, 12]),
        button(text("← Home").size(13)).on_press(Message::Ext(ExtMsg::Back)).style(style::button_style(Btn::Ghost)).padding([8, 12]),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    // Master switch: nothing loads or runs when off (same switch as
    // Settings → General → Extensions).
    let master = container(
        row![
            column![
                text(if extensions_on { "Extensions are on" } else { "Extensions are off" }).size(14),
                text("Off = no extension loads or runs at all — no tool tabs, NBT actions or row notes. Per-extension toggles below only apply while this is on.")
                    .size(11)
                    .color(style::muted()),
            ]
            .spacing(2)
            .width(Length::FillPortion(1)),
            toggler(extensions_on).on_toggle(move |v| Message::Settings(crate::SettingsMsg::Extensions(v))).width(48),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
    )
    .padding(10)
    .style(style::card())
    .width(Length::Fill);

    let records = mgr.records();
    let installed: std::collections::HashSet<&str> = records.iter().map(|r| r.id.as_str()).collect();
    let mut list = column![].spacing(8);
    if records.is_empty() {
        list = list.push(container(
            column![
                text("No extensions enabled yet.").size(14),
                text("Turn on a built-in extension below, install a .zip with manifest.json + main.lua, or install the examples: Chest Explorer (find and open containers, player inventories, “Open as chest”) and Tick Clock (tick counters as in-game time).")
                    .size(12)
                    .color(style::muted()),
            ]
            .spacing(6),
        )
        .padding(16)
        .style(style::card())
        .width(Length::Fill));
    } else {
        for (i, rec) in records.iter().enumerate() {
            list = list.push(ext_card(mgr, rec, i, records.len(), hub));
        }
    }
    let uninstalled: Vec<&omt_extensions::Builtin> = omt_extensions::builtins().iter().filter(|b| !installed.contains(b.id)).collect();
    if !uninstalled.is_empty() {
        list = list.push(text("Built-in — shipped with OmniMCTools, off until you enable one").size(12).color(style::muted()));
        for b in uninstalled {
            list = list.push(builtin_card(b));
        }
    }

    let footer = if status.is_empty() {
        text(format!("extensions live in {}", mgr.root().display())).size(11).color(style::hint())
    } else {
        text(status).size(12).color(style::accent())
    };

    container(column![header, iced::widget::rule::horizontal(1.0), master, scrollable(list).height(Length::Fill), footer].spacing(14).padding(20))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// A shipped-but-not-enabled extension: listed so players can find and turn
/// it on (which writes its embedded files and enables it).
fn builtin_card<'a>(b: &'a omt_extensions::Builtin) -> Element<'a, Message> {
    container(
        row![
            column![
                row![text(b.name).size(15), text("Built-in").size(11).color(style::accent())].spacing(8).align_y(iced::Alignment::Center),
                text(b.description).size(12).color(style::muted()),
            ]
            .spacing(3)
            .width(Length::FillPortion(1)),
            button(text("Enable").size(12))
                .on_press(Message::Ext(ExtMsg::Toggle(b.id.to_string())))
                .style(style::button_style(Btn::Confirm))
                .padding([5, 10]),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    )
    .padding(10)
    .style(style::card())
    .width(Length::Fill)
    .into()
}

fn ext_card<'a>(mgr: &'a Manager, rec: &omt_script::ExtensionRecord, i: usize, total: usize, hub: &'a crate::ext_tools::ExtHub) -> Element<'a, Message> {
    let enabled = mgr.is_enabled(&rec.id);
    let toggle_label = if enabled { "✓ Enabled" } else { "✕ Disabled" };
    let toggle_btn = if enabled { Btn::Confirm } else { Btn::Ghost };
    let meta = [
        (!rec.manifest.version.is_empty()).then(|| format!("v{}", rec.manifest.version)),
        (!rec.manifest.author.is_empty()).then(|| format!("by {}", rec.manifest.author)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    // What it adds: tools (open buttons), NBT actions, load errors.
    let mut provides = row![].spacing(6).align_y(iced::Alignment::Center);
    for d in hub.decls.iter().filter(|d| d.ext == rec.id) {
        provides = provides.push(match d.kind {
            omt_extensions::tool::ToolKind::Tool => button(text(format!("Open {}", d.name)).size(11))
                .padding([3, 8])
                .on_press(Message::ExtTool(crate::ext_tools::ExtToolMsg::Open(d.key())))
                .style(style::button_style(Btn::Chip)),
            omt_extensions::tool::ToolKind::NbtAction => button(text(format!("NBT action: {}", d.name)).size(11)).padding([3, 8]).style(style::button_style(Btn::Chip)),
        });
    }
    let builtin_chip = omt_extensions::builtins().iter().any(|b| b.id == rec.id);
    let errors: Vec<&String> = hub.errors.iter().filter(|e| e.starts_with(&format!("{}:", rec.id))).collect();
    let mut title = row![text(rec.manifest.name.clone()).size(15)].spacing(8).align_y(iced::Alignment::Center);
    if builtin_chip {
        title = title.push(text("Built-in").size(11).color(style::accent()));
    }
    if rec.native {
        title = title.push(text("Native (Rust)").size(11).color(style::info()));
    }
    if !meta.is_empty() {
        title = title.push(text(meta).size(11).color(style::muted()));
    }
    let body = column![
        provides.width(Length::Fill),
        title,
        if rec.manifest.description.is_empty() {
            text(rec.id.clone()).size(12).color(style::muted())
        } else {
            text(rec.manifest.description.clone()).size(12).color(style::muted())
        },
    ]
    .spacing(3);
    let body = errors.into_iter().fold(body, |b, e| b.push(text(e.clone()).size(11).color(style::danger())));
    container(
        row![
            body.width(Length::FillPortion(1)),
            button(text(toggle_label).size(12)).on_press(Message::Ext(ExtMsg::Toggle(rec.id.clone()))).style(style::button_style(toggle_btn)).padding([5, 10]),
            button(text("▲").size(12))
                .on_press_maybe((i > 0).then(|| Message::Ext(ExtMsg::Up(rec.id.clone()))))
                .style(style::button_style(Btn::Standard))
                .padding([5, 8]),
            button(text("▼").size(12))
                .on_press_maybe((i + 1 < total).then(|| Message::Ext(ExtMsg::Down(rec.id.clone()))))
                .style(style::button_style(Btn::Standard))
                .padding([5, 8]),
            button(text("Folder").size(12)).on_press(Message::Ext(ExtMsg::OpenFolder(Some(rec.id.clone())))).style(style::button_style(Btn::Ghost)).padding([5, 10]),
            button(text("Remove").size(12)).on_press(Message::Ext(ExtMsg::Remove(rec.id.clone()))).style(style::button_style(Btn::Danger)).padding([5, 10]),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    )
    .padding(10)
    .style(style::card())
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("omt-gui-ext-page-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn toggle_installs_then_disables_then_reinstalls_builtins() {
        let root = tmp_root("toggle");
        let mgr = Manager::open(&root);
        // Enabling a shipped built-in materializes its embedded files.
        let (status, changed) = handle(&mgr, &ExtMsg::Toggle("chest-explorer".into()));
        assert!(changed, "{status}");
        assert!(mgr.records().iter().any(|r| r.id == "chest-explorer"), "{status}");
        assert!(mgr.is_enabled("chest-explorer"));
        assert!(root.join("chest-explorer/main.lua").is_file());
        // Again: disabled (files stay).
        let (_, changed) = handle(&mgr, &ExtMsg::Toggle("chest-explorer".into()));
        assert!(changed);
        assert!(!mgr.is_enabled("chest-explorer"));
        // Removed: the built-in is still listed by the page (it ships), and
        // toggling brings it back.
        mgr.remove("chest-explorer").unwrap();
        assert!(mgr.records().iter().all(|r| r.id != "chest-explorer"));
        let (_, changed) = handle(&mgr, &ExtMsg::Toggle("chest-explorer".into()));
        assert!(changed);
        assert!(mgr.is_enabled("chest-explorer"));
        // Unknown ids error instead of silently doing nothing.
        assert!(!handle(&mgr, &ExtMsg::Toggle("no-such-ext".into())).1);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod authoring_tests {
    use super::*;

    /// "New extension…" creates a working, enabled extension with a unique id.
    #[test]
    fn new_extension_from_template() {
        let root = std::env::temp_dir().join(format!("omt-newext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mgr = Manager::open(&root);
        let (msg, changed) = handle(&mgr, &ExtMsg::NewExtension);
        assert!(changed, "{msg}");
        let (msg2, _) = handle(&mgr, &ExtMsg::NewExtension);
        assert!(msg2.contains("My Extension 2"), "{msg2}");
        assert_eq!(mgr.enabled_ids(), ["my-extension", "my-extension-2"]);
        let main = std::fs::read_to_string(root.join("my-extension/main.lua")).unwrap();
        assert!(main.contains("name = \"My Extension\"") && !main.contains("__NAME__"));
        let (decls, errs) = omt_extensions::tool::discover(&root, &mgr.enabled_ids());
        assert!(errs.is_empty() && decls.len() == 2, "{errs:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
