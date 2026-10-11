//! The window: a row of sources, two tabs (installed / available), big touch targets, a progress bar
//! for the running install and a queue for "install all".
use crate::games::{self, Game};
use crate::github::Release;
use crate::install::{self, Installed, Progress};
use crate::sources::{self, Source, SOURCES};
use crate::ui::{
    danger_button, human_bytes, quiet_button, ACCENT, AMBER, DIM, GREEN, PAD, RED, SMALL_SIZE,
    SPACING, TEXT_SIZE,
};
use crate::ui::{heading, nav_item, primary_button, table_header, table_row};
use crate::{github, steam};
use iced::widget::{
    button, column, pick_list, progress_bar, row, scrollable, space, text, text_input, Column,
};
use iced::{Element, Fill, Length, Task};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tab {
    Installed,
    Available,
    Games,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Tab(Tab),
    Source(&'static str),
    SourceName(&'static str),
    GameFilter(String),
    /// every Steam install found, with its tools, games and per-game choices
    All(Vec<RootInfo>),
    /// Steam root, appid, tool name ("" = Steam default)
    SetTool(PathBuf, u64, String),
    ToolSet(Result<(), String>),
    Refresh,
    /// install the newest release of every kind of Proton that is installed (or Proton-GE if none)
    UpdateAll,
    UpdateLoaded(&'static str, Result<(Vec<Release>, bool), String>),
    LoadMore,
    Loaded(&'static str, usize, Result<(Vec<Release>, bool), String>),
    Install(String),
    AskAll,
    CancelAll,
    InstallAll,
    Event(Event),
    AskRemove(PathBuf, String),
    CancelRemove,
    Remove(PathBuf, String),
    UseDir(PathBuf),
    UseLabel(String),
}

/// One Steam install on this machine: where it is, what is installed in it, its games.
#[derive(Debug, Clone)]
pub struct RootInfo {
    label: String,
    root: PathBuf,
    tools: PathBuf,
    installed: Vec<Installed>,
    games: Vec<Game>,
    mapping: HashMap<u64, String>,
}

/// Which source a directory name belongs to, for "update all".
fn family(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    if n.starts_with("ge-proton") {
        Some("ge")
    } else if n.starts_with("proton-cachyos") {
        Some(if n.contains("v3") {
            "cachyos-v3"
        } else {
            "cachyos"
        })
    } else if n.contains("sarek") {
        Some("sarek")
    } else if n.contains("northstar") {
        Some("northstar")
    } else if n.contains("luxtorpeda") {
        Some("luxtorpeda")
    } else {
        None
    }
}

fn root_label(root: &std::path::Path) -> String {
    if root.to_string_lossy().contains("com.valvesoftware.Steam") {
        "Steam (Flatpak)".to_string()
    } else if root.to_string_lossy().contains("snap/steam") {
        "Steam (Snap)".to_string()
    } else {
        "Steam (native)".to_string()
    }
}

fn scan_root(tools: PathBuf) -> RootInfo {
    let root = tools.parent().map(PathBuf::from).unwrap_or_default();
    RootInfo {
        label: root_label(&root),
        installed: install::list_installed(&tools),
        games: games::games(&root),
        mapping: games::mapping(&root),
        root,
        tools,
    }
}

#[derive(Debug, Clone)]
pub enum Event {
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

pub struct App {
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
    confirm_remove: Option<(PathBuf, String)>,
    confirm_all: bool,
    all: Vec<RootInfo>,
    game_filter: String,
}

impl App {
    /// The error currently shown, for the launcher's log.
    pub fn last_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn new(dir: PathBuf, pinned: bool, start: Option<&str>) -> (Self, Task<Msg>) {
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
            all: Vec::new(),
            game_filter: String::new(),
        };
        let t = Task::batch([app.fetch_page(1), app.load_all()]);
        (app, t)
    }

    /// Names right away for the target directory; every install is scanned (sizes too) in the background.
    fn reload(&mut self) -> Task<Msg> {
        self.installed = install::list_installed_with(&self.dir, false);
        self.load_all()
    }

    fn load_all(&self) -> Task<Msg> {
        let dirs = self.dirs.clone();
        Task::perform(
            async move { dirs.into_iter().map(scan_root).collect::<Vec<_>>() },
            Msg::All,
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

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Tab(t) => self.tab = t,
            Msg::UpdateAll => {
                self.error = None;
                self.notice = Some("Checking for newer Protons...".into());
                let mut ids: Vec<&'static str> = Vec::new();
                for i in &self.installed {
                    if let Some(id) = family(&i.name) {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                }
                if ids.is_empty() {
                    ids.push("ge");
                }
                let tasks: Vec<Task<Msg>> = ids
                    .into_iter()
                    .filter_map(sources::find)
                    .map(|src| {
                        Task::perform(async move { github::releases_page(src, 1, 5) }, move |r| {
                            Msg::UpdateLoaded(src.id, r)
                        })
                    })
                    .collect();
                return Task::batch(tasks);
            }
            Msg::UpdateLoaded(id, r) => match (sources::find(id), r) {
                (Some(src), Ok((rels, _))) => {
                    let have: Vec<String> = self.installed.iter().map(|i| i.name.clone()).collect();
                    // the newest release only (the list is newest first)
                    if let Some(r) = rels.into_iter().find(|r| r.archive(src).is_some()) {
                        if !have.contains(&r.dir_name(src))
                            && !self.queue.iter().any(|(_, q)| q.tag_name == r.tag_name)
                        {
                            self.notice = Some(format!("Installing {} ...", r.tag_name));
                            self.queue.push_back((src, r));
                            self.queue_total += 1;
                            return self.start_next();
                        }
                    }
                    if self.busy.is_empty() && self.queue.is_empty() {
                        self.notice = Some("Everything is up to date.".into());
                    }
                }
                (_, Err(e)) => self.error = Some(e),
                _ => {}
            },
            Msg::GameFilter(q) => self.game_filter = q,
            Msg::SourceName(n) => {
                if let Some(s) = SOURCES.iter().find(|s| s.name == n) {
                    return self.update(Msg::Source(s.id));
                }
            }
            Msg::All(v) => {
                if let Some(r) = v.iter().find(|r| r.tools == self.dir) {
                    self.installed = r.installed.clone();
                }
                self.all = v;
            }
            Msg::SetTool(root, appid, tool) => {
                self.error = None;
                self.notice = None;
                return Task::perform(
                    async move {
                        let t = if tool.is_empty() {
                            None
                        } else {
                            Some(tool.as_str())
                        };
                        games::set_tool(&root, appid, t)
                    },
                    Msg::ToolSet,
                );
            }
            Msg::ToolSet(r) => {
                match r {
                    Ok(()) => {
                        self.notice = Some("Saved. Start Steam again; the game uses this tool from its next launch.".into());
                        return self.load_all();
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
            Msg::UseLabel(l) => {
                if let Some(d) = self
                    .dirs
                    .iter()
                    .find(|d| root_label(d.parent().unwrap_or(d)) == l)
                    .cloned()
                {
                    return self.update(Msg::UseDir(d));
                }
            }
            Msg::UseDir(d) => {
                self.dir = d;
                return self.reload();
            }
            Msg::AskRemove(d, n) => self.confirm_remove = Some((d, n)),
            Msg::CancelRemove => self.confirm_remove = None,
            Msg::Remove(d, n) => {
                self.confirm_remove = None;
                match install::remove(&d, &n) {
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

    /// The navigation entries of this section, for the launcher's single sidebar.
    pub fn side(&self, active: bool) -> Element<'_, Msg> {
        column![
            nav_item(
                "Installed",
                active && self.tab == Tab::Installed,
                Msg::Tab(Tab::Installed)
            ),
            nav_item(
                "Available",
                active && self.tab == Tab::Available,
                Msg::Tab(Tab::Available)
            ),
            nav_item(
                "Games",
                active && self.tab == Tab::Games,
                Msg::Tab(Tab::Games)
            ),
        ]
        .spacing(8)
        .into()
    }

    pub fn main(&self) -> Element<'_, Msg> {
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
        let mut top = column![status].spacing(SPACING);
        if self.tab == Tab::Available {
            let names: Vec<&'static str> = SOURCES.iter().map(|s| s.name).collect();
            let mut r = row![
                text("Category").size(SMALL_SIZE).color(DIM),
                pick_list(names, Some(self.source.name), Msg::SourceName)
                    .text_size(SMALL_SIZE)
                    .padding(10),
            ]
            .spacing(SPACING)
            .align_y(iced::Alignment::Center);
            if self.dirs.len() > 1 {
                let labels: Vec<String> = self
                    .dirs
                    .iter()
                    .map(|d| root_label(d.parent().unwrap_or(d)))
                    .collect();
                let cur = root_label(self.dir.parent().unwrap_or(&self.dir));
                r = r.push(text("Install into").size(SMALL_SIZE).color(DIM));
                r = r.push(
                    pick_list(labels, Some(cur), Msg::UseLabel)
                        .text_size(SMALL_SIZE)
                        .padding(10),
                );
            }
            top = top.push(r);
        }
        column![top, body].spacing(SPACING).into()
    }

    /// A table of every game in every Steam install with the Proton it runs with.
    fn games_view(&self) -> Element<'_, Msg> {
        let mut col = Column::new().spacing(4);
        if games::steam_running() {
            col = col.push(
                text("Steam is running: close it completely (Steam > Exit) before changing a game, or Steam undoes the change.")
                    .size(SMALL_SIZE)
                    .color(AMBER),
            );
        }
        col = col.push(
            text_input("Filter games", &self.game_filter)
                .on_input(Msg::GameFilter)
                .padding(10)
                .size(SMALL_SIZE),
        );
        col = col.push(table_header(&[("Game", 4), ("Steam", 2), ("Proton", 3)]));
        const DEFAULT: &str = "Steam default";
        let q = self.game_filter.to_lowercase();
        let mut n = 0usize;
        for r in &self.all {
            let short = r
                .label
                .trim_start_matches("Steam ")
                .trim_matches(|c| c == '(' || c == ')')
                .to_string();
            let mut rows: Vec<(u64, String)> = Vec::new();
            if q.is_empty() {
                rows.push((0, "All games (default)".to_string()));
            }
            rows.extend(
                r.games
                    .iter()
                    .filter(|g| q.is_empty() || g.name.to_lowercase().contains(&q))
                    .map(|g| (g.appid, g.name.clone())),
            );
            for (appid, name) in rows {
                let cur = r.mapping.get(&appid).cloned();
                let mut options: Vec<String> = vec![DEFAULT.to_string()];
                options.extend(r.installed.iter().map(|i| i.tool.clone()));
                if let Some(c) = &cur {
                    if !options.contains(c) {
                        options.push(c.clone());
                    }
                }
                let selected = cur.unwrap_or_else(|| DEFAULT.to_string());
                let root = r.root.clone();
                let dropdown = pick_list(options, Some(selected), move |t| {
                    Msg::SetTool(
                        root.clone(),
                        appid,
                        if t == DEFAULT { String::new() } else { t },
                    )
                })
                .text_size(SMALL_SIZE)
                .padding(8)
                .width(Fill);
                col = col.push(table_row(
                    vec![
                        (text(name).size(TEXT_SIZE).into(), 4),
                        (text(short.clone()).size(SMALL_SIZE).color(DIM).into(), 2),
                        (dropdown.into(), 3),
                    ],
                    n % 2 == 1,
                ));
                n += 1;
            }
        }
        if self.all.is_empty() {
            col = col.push(text("Looking for Steam...").size(TEXT_SIZE).color(DIM));
        } else if n == 0 {
            col = col.push(text("No installed games found.").size(TEXT_SIZE).color(DIM));
        }
        scrollable(col.padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }

    /// What is installed, in every Steam install, on one list.
    fn installed_view(&self) -> Element<'_, Msg> {
        let mut col = Column::new().spacing(8);
        let mut any = false;
        for r in &self.all {
            col = col.push(heading(format!("{}  {}", r.label, r.tools.display())));
            if r.installed.is_empty() {
                col = col.push(text("nothing installed").size(SMALL_SIZE).color(DIM));
            }
            for i in &r.installed {
                any = true;
                let confirm =
                    self.confirm_remove.as_ref() == Some(&(r.tools.clone(), i.name.clone()));
                let actions: Element<Msg> = if confirm {
                    row![
                        text("Remove?").size(TEXT_SIZE).color(AMBER),
                        button(text("Yes, remove").size(TEXT_SIZE))
                            .padding(PAD)
                            .on_press(Msg::Remove(r.tools.clone(), i.name.clone()))
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
                        .on_press(Msg::AskRemove(r.tools.clone(), i.name.clone()))
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
        }
        if self.all.is_empty() {
            col = col.push(text("Looking for Steam...").size(TEXT_SIZE).color(DIM));
        } else if !any {
            col = col.push(
                text("Nothing installed yet. Open Available and install a tool.")
                    .size(TEXT_SIZE)
                    .color(DIM),
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

        let mut col = Column::new().spacing(4);
        col = col.push(table_header(&[
            ("Version", 4),
            ("Released", 2),
            ("Size", 2),
            ("", 3),
        ]));
        for (n, r) in l.releases.iter().enumerate() {
            let size = r
                .archive(src)
                .map(|a| human_bytes(a.size))
                .unwrap_or_default();
            let dirname = r.dir_name(src);
            let has =
                have.contains(&dirname.as_str()) && !matches!(src.dir, sources::DirName::Fixed(_));
            let queued = self.queue.iter().any(|(_, q)| q.tag_name == r.tag_name)
                || self.busy.iter().any(|b| b.tag == r.tag_name);
            let action: Element<Msg> = if has {
                if self.confirm_remove.as_ref() == Some(&(self.dir.clone(), dirname.clone())) {
                    row![
                        button(text("Remove").size(SMALL_SIZE))
                            .padding([6, 12])
                            .on_press(Msg::Remove(self.dir.clone(), dirname.clone()))
                            .style(danger_button()),
                        button(text("No").size(SMALL_SIZE))
                            .padding([6, 12])
                            .on_press(Msg::CancelRemove)
                            .style(quiet_button()),
                    ]
                    .spacing(6)
                    .into()
                } else {
                    row![
                        text("installed").size(SMALL_SIZE).color(ACCENT),
                        button(text("remove").size(SMALL_SIZE))
                            .padding([4, 10])
                            .on_press(Msg::AskRemove(self.dir.clone(), dirname.clone()))
                            .style(quiet_button()),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                    .into()
                }
            } else if queued {
                text("queued").size(SMALL_SIZE).color(AMBER).into()
            } else {
                button(text("Install").size(SMALL_SIZE))
                    .padding([6, 16])
                    .on_press(Msg::Install(r.tag_name.clone()))
                    .style(primary_button())
                    .into()
            };
            col = col.push(table_row(
                vec![
                    (text(r.tag_name.clone()).size(TEXT_SIZE).into(), 4),
                    (
                        text(r.date().to_string())
                            .size(SMALL_SIZE)
                            .color(DIM)
                            .into(),
                        2,
                    ),
                    (text(size).size(SMALL_SIZE).color(DIM).into(), 2),
                    (action, 3),
                ],
                n % 2 == 1,
            ));
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
