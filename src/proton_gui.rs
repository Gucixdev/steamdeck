//! The window: a row of sources, two tabs (installed / available), big touch targets, a progress bar
//! for the running install and a queue for "install all".
use crate::games::{self, Game};
use crate::github::Release;
use crate::install::{self, Installed, Progress};
use crate::sources::{self, Source, SOURCES};
use crate::ui::{
    accent_button, danger_button, human_bytes, quiet_button, theme, AMBER, DIM, GREEN, PAD, PURPLE,
    RED, SMALL_SIZE, SPACING, TEXT_SIZE,
};
use crate::ui::{item, nav_item, shell};
use crate::{github, steam};
use iced::widget::{
    button, column, pick_list, progress_bar, row, scrollable, space, text, text_input, Column,
};
use iced::{Element, Fill, Length, Task};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

pub fn run(dir: PathBuf, pinned: bool, tab: Option<String>) -> iced::Result {
    iced::application(
        move || App::new(dir.clone(), pinned, tab.as_deref()),
        App::update,
        App::view,
    )
    .title("ProtonUp")
    .theme(|_: &App| theme())
    .window_size((1100.0, 700.0))
    .run()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Installed,
    Available,
    Games,
}

#[derive(Debug, Clone)]
enum Msg {
    Tab(Tab),
    Source(&'static str),
    SourceName(&'static str),
    GameFilter(String),
    /// the games of a Steam root and what is mapped for them
    Games(PathBuf, Vec<Game>, HashMap<u64, String>),
    /// appid, tool name ("" = Steam default)
    SetTool(u64, String),
    ToolSet(Result<HashMap<u64, String>, String>),
    Refresh,
    Installed(PathBuf, Vec<Installed>),
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
    Progress(String, Progress),
    Finished(String, Result<install::Outcome, String>),
}

/// Downloads running at once, like `dspq -j 3`.
const JOBS: usize = 3;

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
    busy: Vec<Busy>,
    queue: VecDeque<(&'static Source, Release)>,
    queue_total: usize,
    confirm_remove: Option<String>,
    confirm_all: bool,
    games: Vec<Game>,
    mapping: HashMap<u64, String>,
    game_filter: String,
}

impl App {
    fn new(dir: PathBuf, pinned: bool, start: Option<&str>) -> (Self, Task<Msg>) {
        let home = dirs::home_dir().unwrap_or_default();
        let dirs = if pinned {
            vec![dir.clone()]
        } else {
            steam::tool_dirs(&home)
        };
        let installed = install::list_installed_with(&dir, false);
        let mut app = App {
            dir,
            dirs,
            tab: match start {
                Some("installed") => Tab::Installed,
                Some("available") => Tab::Available,
                Some("games") => Tab::Games,
                _ if installed.is_empty() => Tab::Available,
                _ => Tab::Installed,
            },
            source: sources::default_source(),
            lists: HashMap::new(),
            installed,
            error: None,
            notice: None,
            busy: Vec::new(),
            queue: VecDeque::new(),
            queue_total: 0,
            confirm_remove: None,
            confirm_all: false,
            games: Vec::new(),
            mapping: HashMap::new(),
            game_filter: String::new(),
        };
        let t = Task::batch([app.fetch_page(1), app.load_games(), {
            let d = app.dir.clone();
            Task::perform(
                async move { (d.clone(), install::list_installed(&d)) },
                |(d, l)| Msg::Installed(d, l),
            )
        }]);
        (app, t)
    }

    /// Names right away, sizes from a background thread: the size walk touches thousands of files.
    fn reload(&mut self) -> Task<Msg> {
        self.installed = install::list_installed_with(&self.dir, false);
        let dir = self.dir.clone();
        Task::perform(
            async move { (dir.clone(), install::list_installed(&dir)) },
            |(d, l)| Msg::Installed(d, l),
        )
    }

    /// The Steam root is the directory above compatibilitytools.d.
    fn root(&self) -> Option<PathBuf> {
        self.dir.parent().map(PathBuf::from)
    }

    fn load_games(&self) -> Task<Msg> {
        let Some(root) = self.root() else {
            return Task::none();
        };
        Task::perform(
            async move {
                let g = games::games(&root);
                let m = games::mapping(&root);
                (root, g, m)
            },
            |(r, g, m)| Msg::Games(r, g, m),
        )
    }

    fn fetch_page(&mut self, page: usize) -> Task<Msg> {
        let src = self.source;
        let l = self.lists.entry(src.id).or_default();
        l.loading = true;
        Task::perform(
            async move { github::releases_page(src, page, 100) },
            move |r| Msg::Loaded(src.id, page, r),
        )
    }

    /// Fill the free slots from the front of the queue (FIFO, up to JOBS at once).
    fn start_next(&mut self) -> Task<Msg> {
        let mut tasks = Vec::new();
        while self.busy.len() < JOBS {
            let Some((src, rel)) = self.queue.pop_front() else {
                break;
            };
            self.busy.push(Busy {
                tag: rel.tag_name.clone(),
                phase: "starting".into(),
                done: 0,
                total: 0,
            });
            tasks.push(Task::run(
                install_stream(src, rel, self.dir.clone()),
                Msg::Event,
            ));
        }
        Task::batch(tasks)
    }

    fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Tab(t) => self.tab = t,
            Msg::GameFilter(q) => self.game_filter = q,
            Msg::SourceName(n) => {
                if let Some(s) = SOURCES.iter().find(|s| s.name == n) {
                    return self.update(Msg::Source(s.id));
                }
            }
            Msg::Games(root, g, m) => {
                if Some(&root) == self.root().as_ref() {
                    self.games = g;
                    self.mapping = m;
                }
            }
            Msg::SetTool(appid, tool) => {
                let Some(root) = self.root() else {
                    return Task::none();
                };
                self.error = None;
                self.notice = None;
                return Task::perform(
                    async move {
                        let t = if tool.is_empty() {
                            None
                        } else {
                            Some(tool.as_str())
                        };
                        games::set_tool(&root, appid, t).map(|_| games::mapping(&root))
                    },
                    Msg::ToolSet,
                );
            }
            Msg::ToolSet(r) => {
                match r {
                    Ok(m) => {
                        self.mapping = m;
                        self.notice = Some("Saved. Start Steam again; the game uses this tool from its next launch.".into());
                    }
                    Err(e) => self.error = Some(e),
                }
            }
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
                return Task::batch([self.reload(), self.fetch_page(1)]);
            }
            Msg::LoadMore => {
                let next = self
                    .lists
                    .get(self.source.id)
                    .map(|l| l.page + 1)
                    .unwrap_or(1);
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
                return Task::batch([self.reload(), self.load_games()]);
            }
            Msg::Installed(d, l) => {
                if d == self.dir {
                    self.installed = l;
                }
            }
            Msg::AskRemove(n) => self.confirm_remove = Some(n),
            Msg::CancelRemove => self.confirm_remove = None,
            Msg::Remove(n) => {
                self.confirm_remove = None;
                match install::remove(&self.dir, &n) {
                    Ok(()) => self.notice = Some(format!("Removed {n}. Restart Steam.")),
                    Err(e) => self.error = Some(e),
                }
                return self.reload();
            }
            Msg::Install(tag) => {
                let src = self.source;
                let Some(rel) = self
                    .lists
                    .get(src.id)
                    .and_then(|l| l.releases.iter().find(|r| r.tag_name == tag))
                    .cloned()
                else {
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
                        if !have.contains(&r.dir_name(src))
                            && !self.queue.iter().any(|(_, q)| q.tag_name == r.tag_name)
                        {
                            self.queue.push_back((src, r.clone()));
                            self.queue_total += 1;
                        }
                    }
                }
                self.error = None;
                self.notice = None;
                return self.start_next();
            }
            Msg::Event(Event::Progress(tag, p)) => {
                if let Some(b) = self.busy.iter_mut().find(|b| b.tag == tag) {
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
            Msg::Event(Event::Finished(tag, r)) => {
                self.busy.retain(|b| b.tag != tag);
                let reload = self.reload();
                match r {
                    Ok(o) => {
                        let note = if o.verified {
                            ""
                        } else {
                            " (the project publishes no checksum, not verified)"
                        };
                        self.notice = Some(format!("Installed {tag}{note}. Restart Steam, then pick it in the game's Compatibility settings."));
                    }
                    // one failure does not stop the rest; the partial download stays and resumes next time
                    Err(e) => self.error = Some(format!("{tag}: {e}")),
                }
                return Task::batch([reload, self.start_next()]);
            }
        }
        if self.busy.is_empty() && self.queue.is_empty() {
            self.queue_total = 0;
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Msg> {
        let side = column![
            nav_item(
                "Installed",
                self.tab == Tab::Installed,
                Msg::Tab(Tab::Installed)
            ),
            nav_item(
                "Available",
                self.tab == Tab::Available,
                Msg::Tab(Tab::Available)
            ),
            nav_item("Games", self.tab == Tab::Games, Msg::Tab(Tab::Games)),
            space::vertical(),
            nav_item("Refresh", false, Msg::Refresh),
        ]
        .spacing(8)
        .height(Fill);

        let dir_line: Element<Msg> = if self.dirs.len() > 1 {
            let mut r = row![text("Install into").size(SMALL_SIZE).color(DIM)]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center);
            for d in &self.dirs {
                let label = d
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let b = button(text(label).size(SMALL_SIZE))
                    .padding(8)
                    .on_press(Msg::UseDir(d.clone()));
                r = r.push(if *d == self.dir {
                    b.style(accent_button())
                } else {
                    b.style(quiet_button())
                });
            }
            r.into()
        } else {
            text(self.dir.display().to_string())
                .size(SMALL_SIZE)
                .color(DIM)
                .into()
        };

        let mut status = Column::new().spacing(6);
        for b in &self.busy {
            let pct = if b.total > 0 {
                b.done as f32 / b.total as f32
            } else {
                0.0
            };
            status = status.push(
                row![
                    text(format!("{}: {}", b.tag, b.phase)).size(TEXT_SIZE),
                    progress_bar(0.0..=1.0, pct).length(Length::Fill),
                    text(format!(
                        "{} / {}",
                        human_bytes(b.done),
                        human_bytes(b.total)
                    ))
                    .size(SMALL_SIZE)
                    .color(DIM),
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center),
            );
        }
        if self.queue_total > 1 {
            status = status.push(
                text(format!("{} waiting in the queue", self.queue.len()))
                    .size(SMALL_SIZE)
                    .color(DIM),
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
            Tab::Games => self.games_view(),
        };
        let mut top = column![dir_line, status].spacing(SPACING);
        if self.tab == Tab::Available {
            let names: Vec<&'static str> = SOURCES.iter().map(|s| s.name).collect();
            top = top.push(
                row![
                    text("Category").size(SMALL_SIZE).color(DIM),
                    pick_list(names, Some(self.source.name), Msg::SourceName)
                        .text_size(SMALL_SIZE)
                        .padding(10),
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center),
            );
        }
        shell(side.into(), column![top, body].spacing(SPACING).into())
    }

    fn games_view(&self) -> Element<'_, Msg> {
        let mut col = Column::new().spacing(10);
        if games::steam_running() {
            col = col.push(
                text("Steam is running: close it completely (Steam > Exit) before changing a game, or Steam undoes the change.")
                    .size(SMALL_SIZE)
                    .color(AMBER),
            );
        }
        if self.games.is_empty() {
            col = col.push(
                text("No installed games found in this Steam.")
                    .size(TEXT_SIZE)
                    .color(DIM),
            );
        }
        col = col.push(
            text_input("Filter games", &self.game_filter)
                .on_input(Msg::GameFilter)
                .padding(10)
                .size(SMALL_SIZE),
        );
        const DEFAULT: &str = "Steam default";
        let row_for = |appid: u64, name: &str| -> Element<'_, Msg> {
            let cur = self.mapping.get(&appid).cloned();
            let mut options: Vec<String> = vec![DEFAULT.to_string()];
            options.extend(self.installed.iter().map(|i| i.tool.clone()));
            if let Some(c) = &cur {
                if !options.contains(c) {
                    options.push(c.clone());
                }
            }
            let selected = cur.unwrap_or_else(|| DEFAULT.to_string());
            item(
                name.to_string(),
                if appid == 0 {
                    "used by every game that has no choice of its own".to_string()
                } else {
                    format!("app {appid}")
                },
                pick_list(options, Some(selected), move |t| {
                    Msg::SetTool(appid, if t == DEFAULT { String::new() } else { t })
                })
                .text_size(SMALL_SIZE)
                .padding(10)
                .into(),
            )
        };
        let q = self.game_filter.to_lowercase();
        if q.is_empty() {
            col = col.push(row_for(0, "All games"));
        }
        for g in self
            .games
            .iter()
            .filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q))
        {
            col = col.push(row_for(g.appid, &g.name));
        }
        scrollable(col.padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }

    fn installed_view(&self) -> Element<'_, Msg> {
        if self.installed.is_empty() {
            return text("Nothing installed yet. Open Available and install a tool.")
                .size(TEXT_SIZE)
                .color(DIM)
                .into();
        }
        let mut col = Column::new().spacing(8);
        for i in &self.installed {
            let confirm = self.confirm_remove.as_deref() == Some(i.name.as_str());
            let actions: Element<Msg> = if confirm {
                row![
                    text("Remove?").size(TEXT_SIZE).color(AMBER),
                    button(text("Yes, remove").size(TEXT_SIZE))
                        .padding(PAD)
                        .on_press(Msg::Remove(i.name.clone()))
                        .style(danger_button()),
                    button(text("Cancel").size(TEXT_SIZE))
                        .padding(PAD)
                        .on_press(Msg::CancelRemove)
                        .style(quiet_button()),
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center)
                .into()
            } else {
                button(text("Remove").size(TEXT_SIZE))
                    .padding(PAD)
                    .on_press(Msg::AskRemove(i.name.clone()))
                    .style(quiet_button())
                    .into()
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
        scrollable(col.padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }

    fn available_view(&self) -> Element<'_, Msg> {
        let src = self.source;
        let about = text(src.about).size(SMALL_SIZE).color(DIM).width(Fill);
        let Some(l) = self.lists.get(src.id) else {
            return column![
                about,
                text("Loading releases...").size(TEXT_SIZE).color(DIM)
            ]
            .spacing(SPACING)
            .into();
        };
        if l.releases.is_empty() {
            let msg = if l.loading {
                "Loading releases..."
            } else {
                "No releases loaded. Check the connection and press Refresh."
            };
            return column![about, text(msg).size(TEXT_SIZE).color(DIM)]
                .spacing(SPACING)
                .into();
        }
        let have: Vec<&str> = self.installed.iter().map(|i| i.name.as_str()).collect();
        let busy = !self.busy.is_empty();
        let missing: Vec<&Release> = l
            .releases
            .iter()
            .filter(|r| !have.contains(&r.dir_name(src).as_str()))
            .collect();
        let missing_size: u64 = missing
            .iter()
            .filter_map(|r| r.archive(src))
            .map(|a| a.size)
            .sum();

        let mut bar = row![about]
            .spacing(SPACING)
            .align_y(iced::Alignment::Center);
        if self.confirm_all {
            bar = bar.push(
                text(format!(
                    "Download {} releases, {}?",
                    missing.len(),
                    human_bytes(missing_size)
                ))
                .size(TEXT_SIZE)
                .color(AMBER),
            );
            bar = bar.push(
                button(text("Yes, install all").size(TEXT_SIZE))
                    .padding(PAD)
                    .on_press(Msg::InstallAll)
                    .style(danger_button()),
            );
            bar = bar.push(
                button(text("Cancel").size(TEXT_SIZE))
                    .padding(PAD)
                    .on_press(Msg::CancelAll)
                    .style(quiet_button()),
            );
        } else if !missing.is_empty() && !busy {
            bar = bar.push(
                button(
                    text(format!(
                        "Install all ({}, {})",
                        missing.len(),
                        human_bytes(missing_size)
                    ))
                    .size(SMALL_SIZE),
                )
                .padding(12)
                .on_press(Msg::AskAll)
                .style(quiet_button()),
            );
        }

        let mut col = Column::new().spacing(8);
        for r in &l.releases {
            let size = r
                .archive(src)
                .map(|a| human_bytes(a.size))
                .unwrap_or_default();
            let has = have.contains(&r.dir_name(src).as_str())
                && !matches!(src.dir, sources::DirName::Fixed(_));
            let queued = self.queue.iter().any(|(_, q)| q.tag_name == r.tag_name)
                || self.busy.iter().any(|b| b.tag == r.tag_name);
            let action: Element<Msg> = if has {
                text("installed").size(SMALL_SIZE).color(PURPLE).into()
            } else if queued {
                text("queued").size(SMALL_SIZE).color(AMBER).into()
            } else {
                button(text("Install").size(TEXT_SIZE))
                    .padding(PAD)
                    .on_press(Msg::Install(r.tag_name.clone()))
                    .style(accent_button())
                    .into()
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
            let label = if l.loading {
                "Loading..."
            } else {
                "Load older releases"
            };
            let b = button(text(label).size(TEXT_SIZE))
                .padding(PAD)
                .style(quiet_button());
            col = col.push(if l.loading {
                b
            } else {
                b.on_press(Msg::LoadMore)
            });
        }
        column![
            bar,
            scrollable(col.padding(iced::Padding {
                right: 16.0,
                ..iced::Padding::ZERO
            }))
            .height(Fill)
        ]
        .spacing(SPACING)
        .into()
    }
}

/// Runs the blocking install on its own thread and turns its progress callbacks into a stream.
fn install_stream(
    src: &'static Source,
    rel: Release,
    dir: PathBuf,
) -> impl iced::futures::Stream<Item = Event> {
    iced::stream::channel(
        64,
        async move |tx: iced::futures::channel::mpsc::Sender<Event>| {
            let mut tx_progress = tx.clone();
            let mut tx_done = tx;
            let tag = rel.tag_name.clone();
            let tag_progress = tag.clone();
            let handle = std::thread::spawn(move || {
                install::install(src, &rel, &dir, &mut |p| {
                    let _ = iced::futures::executor::block_on(iced::futures::SinkExt::send(
                        &mut tx_progress,
                        Event::Progress(tag_progress.clone(), p),
                    ));
                })
            });
            let res = loop {
                if handle.is_finished() {
                    break handle
                        .join()
                        .unwrap_or_else(|_| Err("install thread crashed".into()));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            };
            let _ = iced::futures::SinkExt::send(&mut tx_done, Event::Finished(tag, res)).await;
        },
    )
}
