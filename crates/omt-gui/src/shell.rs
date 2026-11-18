//! The app frame: gradient top bar (wordmark, breadcrumb, global search,
//! open buttons, theme switcher, settings), collapsible side rail
//! (Browse / Tools / Open worlds & files / System) and the status bar.

use iced::widget::{button, column, container, pick_list, row, scrollable, space, text, text_input, tooltip};
use iced::{Alignment, Element, Length};
use omt_art::{Glyph, Icon};

use crate::assets;

use crate::session::App;
use crate::style::{self, Btn, ThemeKind};
use crate::{Message, OmniMCTools, Page, SessionTab, Ui};

fn tip<'a>(content: impl Into<Element<'a, Message>>, label: String, show: bool) -> Element<'a, Message> {
    if show {
        tooltip(content, container(text(label).size(12)).padding([4, 8]).style(style::card()), tooltip::Position::Right).into()
    } else {
        content.into()
    }
}

impl OmniMCTools {
    pub(crate) fn top_bar(&self) -> Element<'_, Message> {
        let crumb = match self.screen {
            Page::Home => "Home".to_string(),
            Page::Library => "Worlds".into(),
            Page::Tool(a) => a.name().into(),
            Page::Settings => "Settings".into(),
            Page::Extensions => "Extensions".into(),
            Page::Doc(i) => self.docs.get(i).map(|d| format!("Files › {}", d.title)).unwrap_or_default(),
            Page::Session(i) => self.sessions.get(i).map(|s| format!("{} › {}", s.app.name(), s.title)).unwrap_or_default(),
        };
        let search = text_input("Search worlds…", &self.library.query)
            .on_input(Message::GlobalSearch)
            .on_submit(Message::Nav(Page::Library))
            .icon(text_input::Icon { font: iced::Font::DEFAULT, code_point: '⌕', size: Some(iced::Pixels(15.0)), spacing: 8.0, side: text_input::Side::Left })
            .padding([7, 12])
            .size(13)
            .width(340)
            .style(style::input());
        let bar = row![
            button(assets::glyph(Glyph::Menu, style::muted(), 16.0)).padding(8).on_press(Message::ToggleRail).style(style::button_style(Btn::Ghost)),
            button(assets::wordmark(22.0)).padding([4, 6]).on_press(Message::Nav(Page::Home)).style(style::button_style(Btn::Ghost)),
            container(text(crumb).size(13).color(style::muted())).padding([0, 8]),
            space::horizontal(),
            search,
            button(row![assets::icon(Icon::Folder, 16.0), text("Open world…").size(12)].spacing(6).align_y(Alignment::Center)).padding([7, 12]).on_press(Message::BrowseFolder(App::MapViewer)).style(style::button_style(Btn::Standard)),
            pick_list(ThemeKind::ALL.to_vec(), Some(self.settings.theme), Message::SetTheme).text_size(12).padding([7, 10]).width(160),
            button(assets::icon(Icon::Settings, 18.0)).padding(7).on_press(Message::OpenSettings).style(style::button_style(Btn::Ghost)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        container(bar).padding([6, 10]).width(Length::Fill).style(style::topbar()).into()
    }

    fn nav_item<'a>(&self, icon: Icon, label: String, page: Page, badge: Option<String>) -> Element<'a, Message> {
        let active = self.screen == page;
        let collapsed = self.settings.rail_collapsed;
        let mut content = row![assets::icon(icon, 18.0)].spacing(10).align_y(Alignment::Center);
        if !collapsed {
            content = content.push(text(label.clone()).size(13).width(Length::Fill));
            if let Some(b) = badge {
                content = content.push(text(b).size(11).color(style::hint()));
            }
        }
        let accent = container(space::horizontal().width(3)).height(20).style(move |_| container::Style {
            background: active.then(|| iced::Background::Color(style::accent())),
            border: iced::border::rounded(2),
            ..Default::default()
        });
        let b = button(content).width(Length::Fill).padding([7, 10]).on_press(Message::Nav(page)).style(style::button_style(if active { Btn::NavActive } else { Btn::Nav }));
        tip(row![accent, b].spacing(4).align_y(Alignment::Center), label, collapsed)
    }

    fn rail_heading<'a>(&self, title: &'a str) -> Element<'a, Message> {
        if self.settings.rail_collapsed {
            iced::widget::rule::horizontal(1).into()
        } else {
            container(text(title).size(10).font(style::BOLD).color(style::hint())).padding([8, 12]).into()
        }
    }

    pub(crate) fn rail(&self) -> Element<'_, Message> {
        let collapsed = self.settings.rail_collapsed;
        let mut col = column![
            self.rail_heading("BROWSE"),
            self.nav_item(Icon::Home, "Home".into(), Page::Home, None),
            self.nav_item(Icon::Worlds, "Worlds".into(), Page::Library, Some(self.worlds.len().to_string())),
            self.rail_heading("TOOLS"),
        ]
        .spacing(2);
        for app in App::ALL {
            let open = self.sessions.iter().filter(|s| s.app == app).count();
            let badge = if open > 0 { Some(format!("{open} open")) } else { app.alpha().then(|| "alpha".to_string()) };
            col = col.push(self.nav_item(app.icon(), app.name().into(), Page::Tool(app), badge));
        }
        if !self.sessions.is_empty() || !self.docs.is_empty() {
            col = col.push(self.rail_heading("OPEN"));
            for (i, s) in self.sessions.iter().enumerate() {
                let dirty = s.sess.as_ref().is_some_and(|x| x.modified);
                col = col.push(self.open_item(s.app.icon(), &s.title, dirty, Page::Session(i), Message::CloseSession(i), collapsed));
            }
            for (i, d) in self.docs.iter().enumerate() {
                col = col.push(self.open_item(Icon::NbtEditor, &d.title, d.is_dirty(), Page::Doc(i), Message::CloseDoc(i), collapsed));
            }
        }
        let bottom = column![
            self.nav_item(Icon::Extensions, "Extensions".into(), Page::Extensions, None),
            self.nav_item(Icon::Settings, "Settings".into(), Page::Settings, None),
        ]
        .spacing(2);
        let body = column![scrollable(col).height(Length::Fill), bottom].spacing(6).padding([8, 6]);
        container(body).width(if collapsed { 68 } else { 232 }).height(Length::Fill).style(style::rail()).into()
    }

    fn open_item<'a>(&self, icon: Icon, title: &str, dirty: bool, page: Page, close: Message, collapsed: bool) -> Element<'a, Message> {
        let active = self.screen == page;
        let dot = if dirty { " ●" } else { "" };
        let mut content = row![assets::icon(icon, 16.0)].spacing(8).align_y(Alignment::Center);
        if !collapsed {
            content = content.push(text(format!("{title}{dot}")).size(12).width(Length::Fill));
        }
        let b = button(content).width(Length::Fill).padding([6, 10]).on_press(Message::Nav(page)).style(style::button_style(if active { Btn::NavActive } else { Btn::Nav }));
        if collapsed {
            return tip(b, title.to_string(), true);
        }
        row![b, button(assets::glyph(Glyph::Close, style::hint(), 10.0)).padding(6).on_press(close).style(style::button_style(Btn::Ghost))].align_y(Alignment::Center).into()
    }

    pub(crate) fn session_view<'a>(&'a self, i: usize, tab: &'a SessionTab) -> Element<'a, Message> {
        let Some(sess) = tab.sess.as_ref() else {
            return container(text(tab.busy.clone().unwrap_or_default()).color(style::warn())).center(Length::Fill).into();
        };
        let state: Element<'a, Message> = if sess.in_place {
            container(text("READ-ONLY").size(10).font(style::BOLD)).padding([2, 8]).style(style::badge(style::info())).into()
        } else if sess.modified {
            container(text("UNSAVED CHANGES").size(10).font(style::BOLD)).padding([2, 8]).style(style::badge(style::warn())).into()
        } else {
            text("no unsaved changes").size(12).color(style::muted()).into()
        };
        let editable = !sess.in_place && tab.app != App::Converter && tab.busy.is_none();
        let mut bar = row![
            assets::icon(tab.app.icon(), 22.0),
            column![
                {
                    let mut r = row![text(format!("{} — {}", tab.title, tab.app.name())).size(15).font(style::BOLD)].spacing(8).align_y(Alignment::Center);
                    if tab.app.alpha() {
                        r = r.push(style::alpha_badge());
                    }
                    r
                },
                text(format!("{} · {} · {}", sess.edition().name(), style::version_label(&sess.summary), sess.origin().display())).size(11).color(style::muted()),
            ]
            .spacing(2),
            space::horizontal(),
            state,
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if !sess.in_place {
            bar = bar.push(button(text("Save").size(13)).on_press_maybe(editable.then_some(Message::SaveSession(i))).style(style::button_style(Btn::Confirm)));
            bar = bar.push(button(text("Save As…").size(13)).on_press_maybe(editable.then_some(Message::SaveAsSession(i))).style(style::button_style(Btn::Standard)));
        }
        // In-place sessions (the Map Viewer) reach the other tools through
        // the viewer's own compact "Tools" menu, not this header.
        bar = bar.push(button(text("Close").size(13)).on_press(Message::CloseSession(i)).style(style::button_style(Btn::Danger)));
        let header = container(bar).padding([8, 12]).style(style::panel());
        let body: Element<'a, Message> = match &tab.ui {
            Ui::Nbt(ui) => ui.view(sess).map(move |m| Message::Nbt(i, m)),
            Ui::Pruner(ui) => ui.view(sess).map(move |m| Message::Prune(i, m)),
            Ui::Converter(ui) => ui.view(sess).map(move |m| Message::Conv(i, m)),
            Ui::Viewer(ui) => ui.view(&self.settings.bookmarks, sess.summary.spawn).map(move |m| Message::View(i, m)),
            Ui::Editor(ui) => ui.view().map(move |m| Message::Edit(i, m)),
        };
        let status = match &tab.ui {
            Ui::Pruner(p) => p.status_line(),
            Ui::Nbt(n) => n.status.clone(),
            Ui::Viewer(v) => v.status_line(),
            Ui::Editor(e) => e.status_line(),
            Ui::Converter(_) => String::new(),
        };
        let mut col = column![header, container(body).height(Length::Fill)];
        if !status.is_empty() {
            col = col.push(container(text(status).size(12).color(style::muted())).padding([4, 12]));
        }
        col.into()
    }

    pub(crate) fn status_bar(&self) -> Element<'_, Message> {
        container(
            row![
                text(self.status.clone()).size(12).color(style::muted()),
                space::horizontal(),
                text("Ctrl+Z undo · Ctrl+Y redo · Ctrl+S save · Ctrl+F find · offline, no account").size(11).color(style::hint()),
            ]
            .spacing(16),
        )
        .padding([5, 12])
        .style(style::topbar())
        .into()
    }
}
