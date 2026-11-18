//! Converter app — UMT's step flow: INPUT → OUTPUT EDITION → OPTIONS →
//! CONVERT → SAVE, each step badged incomplete / in progress / complete /
//! failed. Latest ⇄ latest only (PLAN §1.3), so the version drop-down is
//! fixed to the pinned release.

use std::path::PathBuf;

use iced::widget::{button, column, container, row, scrollable, space, text, text_input, toggler};
use iced::{Element, Length, Task};
use omt_world::Edition;

use crate::session::{WorldSession, blocking};
use crate::style::{self, Btn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Output,
    Options,
    Convert,
    Save,
}

#[derive(Debug, Clone)]
pub enum ConvMsg {
    Step(u8),
    SkipPartial(bool),
    Dim(usize, bool),
    GameMode(Option<i32>),
    Name(String),
    Start,
    Progress(String),
    Done(Result<std::sync::Arc<omt_convert::world::WorldReport>, String>),
    PickSave,
    SavePicked(Option<PathBuf>),
    Saved(Result<String, String>),
}

pub struct ConverterState {
    step: Step,
    target: Edition,
    opts: omt_convert::world::WorldOptions,
    name: String,
    running: bool,
    log: Vec<String>,
    report: Option<std::sync::Arc<omt_convert::world::WorldReport>>,
    out_dir: Option<PathBuf>,
    saved_to: Option<String>,
    failed: Option<String>,
}

impl ConverterState {
    pub fn new(sess: &WorldSession, skip_partial: bool) -> Self {
        let target = match sess.edition() {
            Edition::Java => Edition::Bedrock,
            Edition::Bedrock => Edition::Java,
        };
        Self {
            step: Step::Output,
            target,
            opts: omt_convert::world::WorldOptions { skip_partial, ..Default::default() },
            name: sess.summary.name.clone(),
            running: false,
            log: Vec::new(),
            report: None,
            out_dir: None,
            saved_to: None,
            failed: None,
        }
    }

    pub fn update(&mut self, sess: &mut WorldSession, base: PathBuf, msg: ConvMsg) -> Task<ConvMsg> {
        match msg {
            ConvMsg::Step(s) => {
                self.step = match s {
                    0 => Step::Output,
                    1 => Step::Options,
                    2 => Step::Convert,
                    _ => Step::Save,
                }
            }
            ConvMsg::SkipPartial(v) => self.opts.skip_partial = v,
            ConvMsg::Dim(i, v) => self.opts.dims[i] = v,
            ConvMsg::GameMode(g) => self.opts.game_mode = g,
            ConvMsg::Name(n) => self.name = n,
            ConvMsg::Start => {
                if self.running {
                    return Task::none();
                }
                self.running = true;
                self.failed = None;
                self.report = None;
                self.log = vec!["Starting conversion…".into()];
                let src = sess.root();
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                let out = base.join(format!("converted-{stamp}"));
                self.out_dir = Some(out.clone());
                let mut opts = self.opts.clone();
                opts.level_name = (!self.name.trim().is_empty()).then(|| self.name.trim().to_string());
                let to = self.target;
                let (tx, rx) = iced::futures::channel::mpsc::unbounded::<String>();
                let work = blocking(move || {
                    let tx2 = std::sync::Mutex::new(tx);
                    let progress = move |m: String| {
                        if let Ok(t) = tx2.lock() {
                            let _ = t.unbounded_send(m);
                        }
                    };
                    omt_convert::world::convert_world(&src, &out, to, &opts, &progress).map(std::sync::Arc::new).map_err(|e| e.to_string())
                });
                return Task::batch([Task::run(rx, ConvMsg::Progress), Task::perform(work, ConvMsg::Done)]);
            }
            ConvMsg::Progress(m) => self.log.push(m),
            ConvMsg::Done(res) => {
                self.running = false;
                match res {
                    Ok(r) => {
                        self.log.push(format!("Converted {} chunks.", r.total_converted()));
                        self.report = Some(r);
                        self.step = Step::Save;
                    }
                    Err(e) => {
                        self.log.push(format!("Failed: {e}"));
                        self.failed = Some(e);
                    }
                }
            }
            ConvMsg::PickSave => {
                let java = self.target == Edition::Java;
                let name = sanitize(&self.name);
                return Task::perform(
                    async move {
                        let d = rfd::AsyncFileDialog::new().set_title("Save the converted world").set_file_name(if java { name.clone() } else { format!("{name}.mcworld") });
                        let d = if java { d } else { d.add_filter("Bedrock world", &["mcworld"]) };
                        if java { d.pick_folder().await } else { d.save_file().await }.map(|h| h.path().to_path_buf())
                    },
                    ConvMsg::SavePicked,
                );
            }
            ConvMsg::SavePicked(Some(dest)) => {
                let Some(out) = self.out_dir.clone() else { return Task::none() };
                let java = self.target == Edition::Java;
                let dest = if java { dest.join(sanitize(&self.name)) } else { dest };
                return Task::perform(
                    blocking(move || {
                        if dest.exists() {
                            return Err(format!("{} already exists", dest.display()));
                        }
                        if omt_world::workspace::is_archive(&dest) {
                            omt_world::workspace::write_archive(&out, &dest).map_err(|e| e.to_string())?;
                        } else {
                            omt_world::workspace::copy_dir(&out, &dest).map_err(|e| e.to_string())?;
                        }
                        Ok(dest.display().to_string())
                    }),
                    ConvMsg::Saved,
                );
            }
            ConvMsg::SavePicked(None) => {}
            ConvMsg::Saved(res) => match res {
                Ok(p) => {
                    self.log.push(format!("Saved to {p}"));
                    self.saved_to = Some(p);
                }
                Err(e) => self.log.push(format!("Save failed: {e}")),
            },
        }
        Task::none()
    }

    fn badge(&self, s: Step) -> (&'static str, iced::Color) {
        let order = |x: Step| x as u8;
        let complete = match s {
            Step::Output | Step::Options => order(self.step) > order(s) || self.report.is_some(),
            Step::Convert => self.report.is_some(),
            Step::Save => self.saved_to.is_some(),
        };
        if s == Step::Convert && self.failed.is_some() {
            ("failed", style::danger())
        } else if s == Step::Convert && self.running {
            ("in progress", style::warn())
        } else if complete {
            ("complete", style::success())
        } else {
            ("incomplete", style::hint())
        }
    }

    pub fn view<'a>(&'a self, sess: &'a WorldSession) -> Element<'a, ConvMsg> {
        let steps = [(Step::Output, "OUTPUT"), (Step::Options, "OPTIONS"), (Step::Convert, "CONVERT"), (Step::Save, "SAVE")];
        let mut bar = row![container(column![text("INPUT").size(12), text("complete").size(10).color(style::success())]).padding(8).style(style::card())].spacing(6);
        for (i, (s, label)) in steps.iter().enumerate() {
            let (b, c) = self.badge(*s);
            bar = bar.push(
                button(column![text(*label).size(12), text(b).size(10).color(c)])
                    .on_press(ConvMsg::Step(i as u8))
                    .style(style::button_style(if self.step == *s { Btn::Primary } else { Btn::Standard })),
            );
        }
        let reg = omt_versions::registry();
        let src_desc = format!("{} · {} · {}", sess.summary.name, sess.edition().name(), crate::style::version_label(&sess.summary));
        let body: Element<'a, ConvMsg> = match self.step {
            Step::Output => column![
                text("Output Edition").size(18),
                text(format!("Input: {src_desc}")).size(12).color(style::muted()),
                row![edition_card(Edition::Java, self.target), edition_card(Edition::Bedrock, self.target)].spacing(10),
                text(format!(
                    "Target version: latest ({}). OMT converts latest ⇄ latest only; older worlds should be opened and saved once in the current game first.",
                    if self.target == Edition::Java { format!("Java {}", reg.java_release()) } else { format!("Bedrock {}", reg.bedrock_release) }
                ))
                .size(12)
                .color(style::muted()),
                row![space::horizontal(), button(text("Next").size(13)).on_press(ConvMsg::Step(1)).style(style::button_style(Btn::Primary))],
            ]
            .spacing(12)
            .into(),
            Step::Options => {
                let dims = omt_world::Dimension::ALL.iter().enumerate().fold(column![].spacing(6), |c, (i, d)| {
                    c.push(toggler(self.opts.dims[i]).label(d.name()).on_toggle(move |v| ConvMsg::Dim(i, v)))
                });
                let modes = [(None, "Keep"), (Some(0), "Survival"), (Some(1), "Creative"), (Some(2), "Adventure"), (Some(3), "Spectator")];
                let gm = modes.iter().fold(row![].spacing(4), |r, (v, l)| {
                    r.push(button(text(*l).size(12)).on_press(ConvMsg::GameMode(*v)).style(style::button_style(if self.opts.game_mode == *v { Btn::Primary } else { Btn::Standard })))
                });
                column![
                    text("Conversion Options").size(18),
                    opt("Skip Partially-Generated Chunks", "Removes chunks that are not fully generated for a more seamless world.", toggler(self.opts.skip_partial).on_toggle(ConvMsg::SkipPartial).into()),
                    opt("Dimensions", "Which dimensions to convert.", dims.into()),
                    opt("Game Mode", "Override the game mode of the output world.", gm.into()),
                    opt("World Name", "Name of the output world.", text_input("name", &self.name).on_input(ConvMsg::Name).width(260).style(style::input()).into()),
                    opt("Depth Fixer / Allow Older Chunk Versions", "Not needed: both latest editions share the Y −64…319 range, so nothing is raised or clipped.", text("n/a").size(12).color(style::muted()).into()),
                    row![space::horizontal(), button(text("Next").size(13)).on_press(ConvMsg::Step(2)).style(style::button_style(Btn::Primary))],
                ]
                .spacing(10)
                .into()
            }
            Step::Convert | Step::Save => {
                let log = self.log.iter().fold(column![].spacing(2), |c, l| c.push(text(l.clone()).size(12)));
                let summary: Element<'a, ConvMsg> = match &self.report {
                    Some(r) => {
                        let rows = r.dims.iter().fold(column![].spacing(2), |c, d| {
                            let n = omt_world::Dimension::from_id(d.dim).map(|x| x.name()).unwrap_or("?");
                            c.push(text(format!("{n}: {} converted · {} partial skipped · {} failed", d.converted, d.skipped_proto, d.failed)).size(12))
                        });
                        let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
                        for l in &r.losses.losses {
                            *kinds.entry(l.kind.label()).or_default() += 1;
                        }
                        let losses = if kinds.is_empty() { "no data loss recorded".to_string() } else { kinds.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(" · ") };
                        column![rows, text(format!("Loss ledger: {losses}")).size(11).color(style::muted())].spacing(4).into()
                    }
                    None => space::vertical().height(0).into(),
                };
                let save_btn = button(text(if self.target == Edition::Java { "Save World (folder)…" } else { "Save World (.mcworld)…" }).size(13))
                    .on_press_maybe((self.report.is_some() && !self.running).then_some(ConvMsg::PickSave))
                    .style(style::button_style(Btn::Confirm))
                    .padding([8, 14]);
                column![
                    row![
                        text(if self.step == Step::Save { "Save the Converted World" } else { "Convert" }).size(18),
                        space::horizontal(),
                        button(text(if self.running { "Converting…" } else { "Start Conversion" }).size(13))
                            .on_press_maybe((!self.running).then_some(ConvMsg::Start))
                            .style(style::button_style(Btn::Primary))
                            .padding([8, 14]),
                        save_btn,
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                    text(format!("{} → {}", sess.edition().name(), self.target.name())).size(12).color(style::muted()),
                    summary,
                    container(scrollable(log).height(Length::Fill)).padding(8).style(style::card()).height(Length::Fill).width(Length::Fill),
                    text(self.saved_to.as_ref().map(|p| format!("Saved: {p}")).unwrap_or_default()).size(12).color(style::success()),
                ]
                .spacing(10)
                .into()
            }
        };
        column![bar, container(body).padding(12).height(Length::Fill)].spacing(10).padding(12).into()
    }
}

fn edition_card(e: Edition, current: Edition) -> Element<'static, ConvMsg> {
    let selected = e == current;
    container(
        column![
            text(e.name()).size(16),
            text(match e {
                Edition::Java => "Windows, Mac, Linux — saves a world folder",
                Edition::Bedrock => "Windows, Android, iOS, consoles via Realms — saves a .mcworld",
            })
            .size(11)
            .color(style::muted()),
            text(if selected { "✔ output" } else { "this is the input edition" }).size(11).color(if selected { style::success() } else { style::hint() }),
        ]
        .spacing(4),
    )
    .padding(14)
    .width(Length::FillPortion(1))
    .style(move |t| if selected { style::card_selected()(t) } else { style::card()(t) })
    .into()
}

fn opt<'a>(title: &'a str, desc: &'a str, control: Element<'a, ConvMsg>) -> Element<'a, ConvMsg> {
    container(row![column![text(title).size(14), text(desc).size(11).color(style::muted())].spacing(2).width(Length::Fill), control].spacing(12).align_y(iced::Alignment::Center))
        .padding(10)
        .style(style::card())
        .into()
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() { "My Converted World".into() } else { s }
}
