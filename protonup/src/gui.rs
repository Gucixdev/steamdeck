//! The window: a row of sources, two tabs (installed / available), big touch targets, a progress bar
//! for the running install and a queue for "install all".
use deck_common::{accent_button, danger_button, human_bytes, quiet_button, theme, AMBER, DIM, GREEN, PAD, PURPLE, RED, SMALL_SIZE, SPACING, TEXT_SIZE, TITLE_SIZE};
use iced::widget::{button, column, container, progress_bar, row, scrollable, space, text, Column};
use iced::{Element, Fill, Length, Task};
use protonup::install::{self, Installed, Progress};
use protonup::sources::{self, Source, SOURCES};
use protonup::{github, steam, Release};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

pub fn run(dir: PathBuf, pinned: bool) -> iced::Result {
    iced::application(move || App::new(dir.clone(), pinned), App::update, App::view)
        .title("ProtonUp")
        .theme(|_: &App| theme())
        .window_size((1100.0, 700.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Installed,
    Available,
}

#[derive(Debug, Clone)]
enum Msg {
    Tab(Tab),
    Source(&'static str),
    Refresh,
    LoadMore,
    Loaded(&'static str, usize, Result<(Vec<Release>, bool), String>),
    Install(String),
    AskAll,
    CancelAll,
    InstallAll,
    Event(Event),
    AskRemove(String),
    CancelRemove,
    Remove(String),
    UseDir(PathBuf),
}

#[derive(Debug, Clone)]
enum Event {
    Progress(Progress),
    Finished(Result<install::Outcome, String>),
}

struct Busy {
    tag: String,
    phase: String,
    done: u64,
    total: u64,
}

#[derive(Default)]
struct Listing {
    releases: Vec<Release>,
    page: usize,
    more: bool,
    loading: bool,
}

struct App {
    dir: PathBuf,
    dirs: Vec<PathBuf>,
    tab: Tab,
    source: &'static Source,
    lists: HashMap<&'static str, Listing>,
    installed: Vec<Installed>,
    error: Option<String>,
    notice: Option<String>,
    busy: Option<Busy>,
    queue: VecDeque<(&'static Source, Release)>,
    queue_total: usize,
    confirm_remove: Option<String>,
    confirm_all: bool,
}

impl App {
    fn new(dir: PathBuf, pinned: bool) -> (Self, Task<Msg>) {
        let home = dirs::home_dir().unwrap_or_default();
        let dirs = if pinned { vec![dir.clone()] } else { steam::tool_dirs(&home) };
        let installed = install::list_installed(&dir);
        let mut app = App {
            dir,
            dirs,
            tab: if installed.is_empty() { Tab::Available } else { Tab::Installed },
            source: sources::default_source(),
            lists: HashMap::new(),
            installed,
            error: None,
            notice: None,
            busy: None,
            queue: VecDeque::new(),
            queue_total: 0,
            confirm_remove: None,
            confirm_all: false,
        };
        let t = app.fetch_page(1);
        (app, t)
    }

    fn reload(&mut self) {
        self.installed = install::list_installed(&self.dir);
    }

    fn fetch_page(&mut self, page: usize) -> Task<Msg> {
        let src = self.source;
        let l = self.lists.entry(src.id).or_default();
        l.loading = true;
        Task::perform(async move { github::releases_page(src, page, 100) }, move |r| Msg::Loaded(src.id, page, r))
    }

    fn start_next(&mut self) -> Task<Msg> {
        if self.busy.is_some() {
            return Task::none();
        }
        let Some((src, rel)) = self.queue.pop_front() else { return Task::none() };
        self.busy = Some(Busy { tag: rel.tag_name.clone(), phase: "starting".into(), done: 0, total: 0 });
        Task::run(install_stream(src, rel, self.dir.clone()), Msg::Event)
    }

    fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Tab(t) => self.tab = t,
            Msg::Source(id) => {
                if let Some(s) = sources::find(id) {
                    self.source = s;
                    self.confirm_all = false;
                    self.tab = Tab::Available;
                    if !self.lists.contains_key(id) {
                        return self.fetch_page(1);
                    }
                }
            }
            Msg::Refresh => {
                self.error = None;
                self.lists.remove(self.source.id);
                self.reload();
                return self.fetch_page(1);
            }
            Msg::LoadMore => {
                let next = self.lists.get(self.source.id).map(|l| l.page + 1).unwrap_or(1);
                return self.fetch_page(next);
            }
            Msg::Loaded(id, page, r) => {
                let l = self.lists.entry(id).or_default();
                l.loading = false;
                match r {
                    Ok((mut v, more)) => {
                        if page == 1 {
                            l.releases.clear();
                        }
                        l.releases.append(&mut v);
                        l.page = page;
                        l.more = more;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Msg::UseDir(d) => {
                self.dir = d;
                self.reload();
            }
            Msg::AskRemove(n) => self.confirm_remove = Some(n),
            Msg::CancelRemove => self.confirm_remove = None,
            Msg::Remove(n) => {
                self.confirm_remove = None;
                match install::remove(&self.dir, &n) {
                    Ok(()) => self.notice = Some(format!("Removed {n}. Restart Steam.")),
                    Err(e) => self.error = Some(e),
                }
                self.reload();
            }
            Msg::Install(tag) => {
                let src = self.source;
                let Some(rel) = self.lists.get(src.id).and_then(|l| l.releases.iter().find(|r| r.tag_name == tag)).cloned() else {
                    return Task::none();
                };
                self.error = None;
                self.notice = None;
                self.queue.push_back((src, rel));
                self.queue_total += 1;
                return self.start_next();
            }
            Msg::AskAll => self.confirm_all = true,
            Msg::CancelAll => self.confirm_all = false,
            Msg::InstallAll => {
                self.confirm_all = false;
                let src = self.source;
                let have: Vec<String> = self.installed.iter().map(|i| i.name.clone()).collect();
                if let Some(l) = self.lists.get(src.id) {
                    for r in &l.releases {
                        if !have.contains(&r.dir_name(src)) && !self.queue.iter().any(|(_, q)| q.tag_name == r.tag_name) {
                            self.queue.push_back((src, r.clone()));
                            self.queue_total += 1;
                        }
                    }
                }
                self.error = None;
                self.notice = None;
                return self.start_next();
            }
            Msg::Event(Event::Progress(p)) => {
                if let Some(b) = &mut self.busy {
                    match p {
                        Progress::Phase(s) => b.phase = s,
                        Progress::Download { done, total } => {
                            b.phase = "downloading".into();
                            b.done = done;
                            b.total = total;
                        }
                    }
                }
            }
            Msg::Event(Event::Finished(r)) => {
                let tag = self.busy.take().map(|b| b.tag).unwrap_or_default();
                self.reload();
                match r {
                    Ok(o) => {
                        let note = if o.verified { "" } else { " (the project publishes no checksum, not verified)" };
                        self.notice = Some(format!("Installed {tag}{note}. Restart Steam, then pick it in the game's Compatibility settings."));
                        return self.start_next();
                    }
                    Err(e) => {
                        self.error = Some(format!("{tag}: {e}"));
                        self.queue.clear();
                        self.queue_total = 0;
                    }
                }
            }
        }
        if self.busy.is_none() && self.queue.is_empty() {
            self.queue_total = 0;
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Msg> {
        let tab_btn = |label: &str, t: Tab| {
            let b = button(text(label.to_string()).size(TEXT_SIZE)).padding(PAD).on_press(Msg::Tab(t));
            if self.tab == t { b.style(accent_button()) } else { b.style(quiet_button()) }
        };
        let header = row![
            text("ProtonUp").size(TITLE_SIZE).color(GREEN),
            space::horizontal(),
            tab_btn("Installed", Tab::Installed),
            tab_btn("Available", Tab::Available),
            button(text("Refresh").size(TEXT_SIZE)).padding(PAD).on_press(Msg::Refresh).style(quiet_button()),
        ]
        .spacing(SPACING)
        .align_y(iced::Alignment::Center);

        let mut chips = row![].spacing(8);
        for s in SOURCES {
            let b = button(text(s.name).size(SMALL_SIZE)).padding(10).on_press(Msg::Source(s.id));
            chips = chips.push(if s.id == self.source.id && self.tab == Tab::Available { b.style(accent_button()) } else { b.style(quiet_button()) });
        }

        let dir_line: Element<Msg> = if self.dirs.len() > 1 {
            let mut r = row![text("Install into").size(SMALL_SIZE).color(DIM)].spacing(SPACING).align_y(iced::Alignment::Center);
            for d in &self.dirs {
                let label = d.parent().map(|p| p.display().to_string()).unwrap_or_default();
                let b = button(text(label).size(SMALL_SIZE)).padding(8).on_press(Msg::UseDir(d.clone()));
                r = r.push(if *d == self.dir { b.style(accent_button()) } else { b.style(quiet_button()) });
            }
            r.into()
        } else {
            text(self.dir.display().to_string()).size(SMALL_SIZE).color(DIM).into()
        };

        let mut status = Column::new().spacing(6);
        if let Some(b) = &self.busy {
            let pct = if b.total > 0 { b.done as f32 / b.total as f32 } else { 0.0 };
            let queued = if self.queue_total > 1 { format!("  ({} left in the queue)", self.queue.len()) } else { String::new() };
            status = status.push(
                row![
                    text(format!("{}: {}{}", b.tag, b.phase, queued)).size(TEXT_SIZE),
                    progress_bar(0.0..=1.0, pct).length(Length::Fill),
                    text(format!("{} / {}", human_bytes(b.done), human_bytes(b.total))).size(SMALL_SIZE).color(DIM),
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center),
            );
        }
        if let Some(n) = &self.notice {
            status = status.push(text(n.clone()).size(SMALL_SIZE).color(GREEN));
        }
        if let Some(e) = &self.error {
            status = status.push(text(e.clone()).size(SMALL_SIZE).color(RED));
        }

        let body: Element<Msg> = match self.tab {
            Tab::Installed => self.installed_view(),
            Tab::Available => self.available_view(),
        };
        let top = if self.tab == Tab::Available {
            column![header, chips, dir_line, status]
        } else {
            column![header, dir_line, status]
        };
        container(column![top.spacing(SPACING), body].spacing(SPACING)).padding(PAD).width(Fill).height(Fill).into()
    }

    fn installed_view(&self) -> Element<'_, Msg> {
        if self.installed.is_empty() {
            return text("Nothing installed yet. Open Available and install a tool.").size(TEXT_SIZE).color(DIM).into();
        }
        let mut col = Column::new().spacing(8);
        for i in &self.installed {
            let confirm = self.confirm_remove.as_deref() == Some(i.name.as_str());
            let actions: Element<Msg> = if confirm {
                row![
                    text("Remove?").size(TEXT_SIZE).color(AMBER),
                    button(text("Yes, remove").size(TEXT_SIZE)).padding(PAD).on_press(Msg::Remove(i.name.clone())).style(danger_button()),
                    button(text("Cancel").size(TEXT_SIZE)).padding(PAD).on_press(Msg::CancelRemove).style(quiet_button()),
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center)
                .into()
            } else {
                button(text("Remove").size(TEXT_SIZE)).padding(PAD).on_press(Msg::AskRemove(i.name.clone())).style(quiet_button()).into()
            };
            col = col.push(
                row![
                    text(i.name.clone()).size(TEXT_SIZE),
                    space::horizontal(),
                    text(human_bytes(i.size)).size(SMALL_SIZE).color(DIM),
                    actions,
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center),
            );
        }
        scrollable(col).height(Fill).into()
    }

    fn available_view(&self) -> Element<'_, Msg> {
        let src = self.source;
        let about = text(src.about).size(SMALL_SIZE).color(DIM);
        let Some(l) = self.lists.get(src.id) else {
            return column![about, text("Loading releases...").size(TEXT_SIZE).color(DIM)].spacing(SPACING).into();
        };
        if l.releases.is_empty() {
            let msg = if l.loading { "Loading releases..." } else { "No releases loaded. Check the connection and press Refresh." };
            return column![about, text(msg).size(TEXT_SIZE).color(DIM)].spacing(SPACING).into();
        }
        let have: Vec<&str> = self.installed.iter().map(|i| i.name.as_str()).collect();
        let busy = self.busy.is_some();
        let missing: Vec<&Release> = l.releases.iter().filter(|r| !have.contains(&r.dir_name(src).as_str())).collect();
        let missing_size: u64 = missing.iter().filter_map(|r| r.archive(src)).map(|a| a.size).sum();

        let mut bar = row![about, space::horizontal()].spacing(SPACING).align_y(iced::Alignment::Center);
        if self.confirm_all {
            bar = bar.push(text(format!("Download {} releases, {}?", missing.len(), human_bytes(missing_size))).size(TEXT_SIZE).color(AMBER));
            bar = bar.push(button(text("Yes, install all").size(TEXT_SIZE)).padding(PAD).on_press(Msg::InstallAll).style(danger_button()));
            bar = bar.push(button(text("Cancel").size(TEXT_SIZE)).padding(PAD).on_press(Msg::CancelAll).style(quiet_button()));
        } else if !missing.is_empty() && !busy {
            bar = bar.push(
                button(text(format!("Install all ({}, {})", missing.len(), human_bytes(missing_size))).size(SMALL_SIZE))
                    .padding(12)
                    .on_press(Msg::AskAll)
                    .style(quiet_button()),
            );
        }

        let mut col = Column::new().spacing(8);
        for r in &l.releases {
            let size = r.archive(src).map(|a| human_bytes(a.size)).unwrap_or_default();
            let has = have.contains(&r.dir_name(src).as_str()) && !matches!(src.dir, sources::DirName::Fixed(_));
            let queued = self.queue.iter().any(|(_, q)| q.tag_name == r.tag_name) || self.busy.as_ref().map(|b| b.tag == r.tag_name).unwrap_or(false);
            let action: Element<Msg> = if has {
                text("installed").size(SMALL_SIZE).color(PURPLE).into()
            } else if queued {
                text("queued").size(SMALL_SIZE).color(AMBER).into()
            } else {
                button(text("Install").size(TEXT_SIZE)).padding(PAD).on_press(Msg::Install(r.tag_name.clone())).style(accent_button()).into()
            };
            col = col.push(
                row![
                    text(r.tag_name.clone()).size(TEXT_SIZE),
                    text(r.date().to_string()).size(SMALL_SIZE).color(DIM),
                    space::horizontal(),
                    text(size).size(SMALL_SIZE).color(DIM),
                    action,
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center),
            );
        }
        if l.more {
            let label = if l.loading { "Loading..." } else { "Load older releases" };
            let b = button(text(label).size(TEXT_SIZE)).padding(PAD).style(quiet_button());
            col = col.push(if l.loading { b } else { b.on_press(Msg::LoadMore) });
        }
        column![bar, scrollable(col).height(Fill)].spacing(SPACING).into()
    }
}

/// Runs the blocking install on its own thread and turns its progress callbacks into a stream.
fn install_stream(src: &'static Source, rel: Release, dir: PathBuf) -> impl iced::futures::Stream<Item = Event> {
    iced::stream::channel(64, async move |tx: iced::futures::channel::mpsc::Sender<Event>| {
        let mut tx_progress = tx.clone();
        let mut tx_done = tx;
        let handle = std::thread::spawn(move || {
            install::install(src, &rel, &dir, &mut |p| {
                let _ = iced::futures::executor::block_on(iced::futures::SinkExt::send(&mut tx_progress, Event::Progress(p)));
            })
        });
        let res = loop {
            if handle.is_finished() {
                break handle.join().unwrap_or_else(|_| Err("install thread crashed".into()));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let _ = iced::futures::SinkExt::send(&mut tx_done, Event::Finished(res)).await;
    })
}
