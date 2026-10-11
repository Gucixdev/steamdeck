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
    /// Steam root, appid, launch options ("" = none)
    SetLaunch(PathBuf, u64, String),
    /// the text being typed in a game's launch-options box
    LaunchEdit(PathBuf, u64, String),
    LaunchSet(Result<(), String>),
    /// ask protondb.com for the ratings of the listed games (on request only)
    CheckDb,
    DbDone(HashMap<u64, String>, HashMap<u64, bool>),
    /// Steam root, appid, tool name ("" = Steam default)
    SetTool(PathBuf, u64, String),
    ToolSet(Result<(), String>),
    Refresh,
    /// install the newest release of every kind of Proton that is installed (or Proton-GE if none)
    UpdateAll,
    /// what the cleanup of old versions did
    Cleaned(Vec<String>),
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
    /// Valve's own Protons from steamapps/common
    valve: Vec<String>,
    games: Vec<Game>,
    mapping: HashMap<u64, String>,
    launch: HashMap<u64, String>,
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
        valve: games::valve_tools(&root),
        games: games::games(&root),
        mapping: games::mapping(&root),
        launch: games::launch_options(&root),
        root,
        tools,
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Progress(String, Progress),
    Finished(String, Result<install::Outcome, String>),
}

/// Valve's own Protons by the name Steam knows them by. Steam downloads one of these by itself the
/// first time a game is set to use it, so they can be offered without knowing their app ids.
const VALVE_TOOLS: [(&str, &str); 11] = [
    ("proton_experimental", "Proton Experimental"),
    ("proton_hotfix", "Proton Hotfix"),
    ("proton_10", "Proton 10.0"),
    ("proton_9", "Proton 9.0"),
    ("proton_8", "Proton 8.0"),
    ("proton_7", "Proton 7.0"),
    ("proton_63", "Proton 6.3"),
    ("proton_513", "Proton 5.13"),
    ("proton_5", "Proton 5.0"),
    ("proton_411", "Proton 4.11"),
    ("proton_316", "Proton 3.16"),
];

const VALVE_CATEGORY: &str = "Valve Proton (via Steam)";

/// Downloads running at once, like `dspq -j 3`.
const JOBS: usize = 3;

struct Busy {
    tag: String,
    phase: String,
    done: u64,
    total: u64,
    /// when the download started moving, and how much was there then (for the speed and the time left)
    since: Option<(std::time::Instant, u64)>,
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
    protondb: HashMap<u64, String>,
    db_busy: bool,
    /// (release tag, family, directory name) of installs whose older versions go once they are in
    pending_cleanup: Vec<(String, String, String)>,
    /// what the last update did, line by line (shown on Home and on the Proton pages)
    report: Vec<String>,
    /// answers still awaited from the "newest release" checks of an update
    checking: usize,
    /// the category dropdown shows Valve's own Protons instead of a GitHub project
    valve_view: bool,
    /// appid -> Steam lists a Linux build
    native: HashMap<u64, bool>,
    launch_edit: HashMap<(PathBuf, u64), String>,
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
            protondb: crate::protondb::cached(),
            db_busy: false,
            valve_view: false,
            pending_cleanup: Vec::new(),
            report: Vec::new(),
            checking: 0,
            native: crate::protondb::native_cached(),
            launch_edit: HashMap::new(),
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
                since: None,
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
                self.report.clear();
                self.error = None;
                self.notice = Some("Checking for newer Protons...".into());
                let mut ids: Vec<&'static str> = Vec::new();
                for i in &self.installed {
                    if let Some(id) = sources::family_of(&i.name) {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                }
                if ids.is_empty() {
                    ids.push("ge");
                }
                self.checking = ids.len();
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
            Msg::UpdateLoaded(id, r) => {
                self.checking = self.checking.saturating_sub(1);
                match (sources::find(id), r) {
                    (Some(src), Ok((rels, _))) => {
                        let have: Vec<String> =
                            self.installed.iter().map(|i| i.name.clone()).collect();
                        // the newest release only (the list is newest first)
                        if let Some(r) = rels.into_iter().find(|r| r.archive(src).is_some()) {
                            let dirname = r.dir_name(src);
                            if have.contains(&dirname) {
                                // already the newest: only the older ones may need to go
                                self.report.push(format!(
                                    "{}: {} is the newest and is installed",
                                    src.name, r.tag_name
                                ));
                                let dir = self.dir.clone();
                                let fam = src.id.to_string();
                                return Task::perform(
                                    async move {
                                        crate::update::cleanup_old(&dir, &fam, &dirname, true)
                                    },
                                    Msg::Cleaned,
                                );
                            }
                            self.pending_cleanup.push((
                                r.tag_name.clone(),
                                src.id.to_string(),
                                dirname,
                            ));
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
                }
            }
            Msg::GameFilter(q) => self.game_filter = q,
            Msg::SourceName(n) if n == VALVE_CATEGORY => {
                self.valve_view = true;
                self.tab = Tab::Available;
            }
            Msg::SourceName(n) => {
                self.valve_view = false;
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
            Msg::LaunchEdit(root, appid, t) => {
                self.launch_edit.insert((root, appid), t);
            }
            Msg::SetLaunch(root, appid, opts) => {
                self.launch_edit.remove(&(root.clone(), appid));
                // show it at once: a second click before the file is re-read must build on this one,
                // not on the old value (that would undo the first checkbox)
                if let Some(r) = self.all.iter_mut().find(|r| r.root == root) {
                    if opts.is_empty() {
                        r.launch.remove(&appid);
                    } else {
                        r.launch.insert(appid, opts.clone());
                    }
                }
                self.error = None;
                self.notice = None;
                return Task::perform(
                    async move {
                        let o = if opts.is_empty() {
                            None
                        } else {
                            Some(opts.as_str())
                        };
                        games::set_launch_options(&root, appid, o)
                    },
                    Msg::LaunchSet,
                );
            }
            Msg::LaunchSet(r) => match r {
                Ok(()) => {
                    self.notice = Some(
                        "Saved. The launch options apply the next time the game starts.".into(),
                    );
                    return self.load_all();
                }
                Err(e) => self.error = Some(e),
            },
            Msg::CheckDb => {
                if self.db_busy {
                    return Task::none();
                }
                self.db_busy = true;
                let ids: Vec<u64> = self
                    .all
                    .iter()
                    .flat_map(|r| r.games.iter().map(|g| g.appid))
                    .collect();
                return Task::perform(
                    async move {
                        (
                            crate::protondb::fetch_missing(&ids),
                            crate::protondb::fetch_native_missing(&ids),
                        )
                    },
                    |(t, n)| Msg::DbDone(t, n),
                );
            }
            Msg::DbDone(m, n) => {
                self.db_busy = false;
                self.protondb = m;
                self.native = n;
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
                            if b.since.is_none() {
                                b.since = Some((std::time::Instant::now(), done));
                            }
                            b.done = done;
                            b.total = total;
                        }
                    }
                }
            }
            Msg::Event(Event::Finished(tag, r)) => {
                self.busy.retain(|b| b.tag != tag);
                let reload = self.reload();
                let mut extra: Vec<Task<Msg>> = Vec::new();
                if r.is_ok() {
                    if let Some(i) = self.pending_cleanup.iter().position(|(t, _, _)| *t == tag) {
                        let (_, fam, dirname) = self.pending_cleanup.remove(i);
                        let dir = self.dir.clone();
                        extra.push(Task::perform(
                            async move { crate::update::cleanup_old(&dir, &fam, &dirname, true) },
                            Msg::Cleaned,
                        ));
                    }
                } else {
                    self.pending_cleanup.retain(|(t, _, _)| *t != tag);
                }
                match r {
                    Ok(o) => {
                        self.report.push(format!("installed {tag}"));
                        let note = if o.verified {
                            ""
                        } else {
                            " (the project publishes no checksum, not verified)"
                        };
                        self.notice = Some(format!("Installed {tag}{note}. Restart Steam, then pick it in the game's Compatibility settings."));
                    }
                    // one failure does not stop the rest; the partial download stays and resumes next time
                    Err(e) => {
                        self.report.push(format!("{tag} failed: {e}"));
                        self.error = Some(format!("{tag}: {e}"));
                    }
                }
                extra.push(reload);
                extra.push(self.start_next());
                return Task::batch(extra);
            }
            Msg::Cleaned(rep) => {
                if !rep.is_empty() {
                    crate::log::write("update", &rep.join("\n"));
                    self.report.extend(rep);
                }
                return self.reload();
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

    /// Progress of the running installs and the last notice or error; also shown on Home.
    /// True while an update or an install is running, so the window keeps its spinner turning.
    pub fn is_busy(&self) -> bool {
        !self.busy.is_empty() || !self.queue.is_empty() || self.checking > 0
    }

    /// Only the outcome (what was installed and removed, errors), no progress; for Home.
    pub fn report_view(&self) -> Element<'_, Msg> {
        let mut c = Column::new().spacing(4);
        if !self.is_busy() {
            for l in self.report.iter().rev().take(12).rev() {
                c = c.push(text(l.clone()).size(SMALL_SIZE).color(GREEN));
            }
            if let Some(e) = &self.error {
                c = c.push(text(e.clone()).size(SMALL_SIZE).color(RED));
            }
        }
        c.into()
    }

    pub fn status_view(&self, tick: usize) -> Element<'_, Msg> {
        let spin = crate::ui::spinner(tick);
        let mut status = Column::new().spacing(6);
        if self.checking > 0 {
            status =
                status.push(text(format!("{spin} checking for newer versions...")).size(TEXT_SIZE));
        }
        let mut speed_sum = 0.0f64;
        for b in &self.busy {
            let pct = if b.total > 0 {
                b.done as f32 / b.total as f32
            } else {
                0.0
            };
            let (speed, eta) = match b.since {
                Some((t0, d0)) if b.phase == "downloading" => {
                    let el = t0.elapsed().as_secs_f64();
                    let sp = if el > 0.5 {
                        (b.done.saturating_sub(d0)) as f64 / el
                    } else {
                        0.0
                    };
                    speed_sum += sp;
                    let left = b.total.saturating_sub(b.done) as f64;
                    (
                        sp,
                        if sp > 1.0 {
                            Some((left / sp) as u64)
                        } else {
                            None
                        },
                    )
                }
                _ => (0.0, None),
            };
            let detail = if b.phase == "downloading" {
                let mut d = format!("{} / {}", human_bytes(b.done), human_bytes(b.total));
                if speed > 0.0 {
                    d.push_str(&format!("  {}/s", human_bytes(speed as u64)));
                }
                if let Some(e) = eta {
                    d.push_str(&format!("  {} left", crate::ui::human_duration(e)));
                }
                d
            } else {
                String::new()
            };
            status = status.push(
                row![
                    text(format!("{spin} {}: {}", b.tag, b.phase)).size(TEXT_SIZE),
                    progress_bar(0.0..=1.0, if b.phase == "downloading" { pct } else { 1.0 })
                        .length(Length::Fill),
                    text(detail).size(SMALL_SIZE).color(DIM),
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center),
            );
        }
        // the whole job: what is left in the running downloads plus the ones still waiting
        if self.busy.len() + self.queue.len() > 1 && speed_sum > 1.0 {
            let mut left: u64 = self
                .busy
                .iter()
                .map(|b| b.total.saturating_sub(b.done))
                .sum();
            left += self
                .queue
                .iter()
                .filter_map(|(src, r)| r.archive(src))
                .map(|a| a.size)
                .sum::<u64>();
            status = status.push(
                text(format!(
                    "{} item(s) in all, about {} left",
                    self.busy.len() + self.queue.len(),
                    crate::ui::human_duration((left as f64 / speed_sum) as u64)
                ))
                .size(SMALL_SIZE)
                .color(ACCENT),
            );
        }
        if self.queue_total > 1 {
            status = status.push(
                text(format!("{} waiting in the queue", self.queue.len()))
                    .size(SMALL_SIZE)
                    .color(DIM),
            );
        }
        for l in self.report.iter().rev().take(12).rev() {
            status = status.push(text(l.clone()).size(SMALL_SIZE).color(GREEN));
        }
        if let Some(n) = &self.notice {
            status = status.push(text(n.clone()).size(SMALL_SIZE).color(GREEN));
        }
        if let Some(e) = &self.error {
            status = status.push(text(e.clone()).size(SMALL_SIZE).color(RED));
        }
        status.into()
    }

    pub fn main(&self) -> Element<'_, Msg> {
        let status = self.status_view(0);
        let body: Element<Msg> = match self.tab {
            Tab::Installed => self.installed_view(),
            Tab::Available => self.available_view(),
            Tab::Games => self.games_view(),
        };
        let mut top = column![status].spacing(SPACING);
        if self.tab == Tab::Available {
            let mut names: Vec<&'static str> = SOURCES.iter().map(|s| s.name).collect();
            names.push(VALVE_CATEGORY);
            let selected = if self.valve_view {
                VALVE_CATEGORY
            } else {
                self.source.name
            };
            let mut r = row![
                text("Category").size(SMALL_SIZE).color(DIM),
                pick_list(names, Some(selected), Msg::SourceName)
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
        if games::steam_running_cached() {
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
        let db_label = if self.db_busy {
            "Asking protondb.com..."
        } else {
            "Check native / ProtonDB"
        };
        let db_btn = button(text(db_label).size(SMALL_SIZE))
            .padding([6, 12])
            .style(quiet_button());
        col = col.push(
            row![
                text("Pick the Proton and the launch options per game.")
                    .size(SMALL_SIZE)
                    .color(DIM)
                    .width(Fill),
                if self.db_busy {
                    db_btn
                } else {
                    db_btn.on_press(Msg::CheckDb)
                },
            ]
            .align_y(iced::Alignment::Center),
        );
        col = col.push(table_header(&[
            ("Game", 3),
            ("Steam", 1),
            ("ProtonDB", 2),
            ("Proton", 4),
            ("Launch options", 3),
        ]));
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
                for (t, _) in VALVE_TOOLS {
                    if !r.valve.iter().any(|v| v == t) {
                        options.push(t.to_string());
                    }
                }
                options.extend(
                    r.valve
                        .iter()
                        .filter(|v| !options.contains(v))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
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
                // launch options: checkboxes for the usual things plus the command itself
                let key = (r.root.clone(), appid);
                let saved = r.launch.get(&appid).cloned().unwrap_or_default();
                let shown = self
                    .launch_edit
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| saved.clone());
                let mut line_a = row![].spacing(10);
                let mut line_b = row![].spacing(10);
                for (i, f) in crate::launch::FLAGS.into_iter().enumerate() {
                    let root = r.root.clone();
                    let base = saved.clone();
                    let target = if i < 2 { &mut line_a } else { &mut line_b };
                    *target = std::mem::replace(target, row![]).push(
                        iced::widget::checkbox(crate::launch::has(&saved, f))
                            .label(f.label())
                            .size(18)
                            .text_size(SMALL_SIZE)
                            .on_toggle(move |on| {
                                Msg::SetLaunch(
                                    root.clone(),
                                    appid,
                                    crate::launch::toggle(&base, f, on),
                                )
                            }),
                    );
                }
                let root_e = r.root.clone();
                let root_s = r.root.clone();
                let shown_s = shown.clone();
                let mut input_row =
                    row![
                        iced::widget::text_input("command, e.g. gamemoderun %command%", &shown)
                            .on_input(move |t| Msg::LaunchEdit(root_e.clone(), appid, t))
                            .on_submit(Msg::SetLaunch(root_s, appid, shown_s))
                            .padding(6)
                            .size(SMALL_SIZE)
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center);
                if shown != saved {
                    input_row = input_row.push(
                        button(text("Save").size(SMALL_SIZE))
                            .padding([4, 10])
                            .on_press(Msg::SetLaunch(r.root.clone(), appid, shown.clone()))
                            .style(primary_button()),
                    );
                }
                let launch_dd: Element<Msg> = if appid == 0 {
                    text("per game below").size(SMALL_SIZE).color(DIM).into()
                } else {
                    column![line_a, line_b, input_row].spacing(4).into()
                };
                let tier = self.protondb.get(&appid).cloned();
                // a suggestion with its reason, and one button to take it
                let installed_names: Vec<String> =
                    r.installed.iter().map(|i| i.tool.clone()).collect();
                let cur_tool = r.mapping.get(&appid).map(String::as_str);
                let sug = if appid == 0 {
                    None
                } else {
                    crate::recommend::suggest_with(
                        tier.as_deref(),
                        self.native.get(&appid).copied().unwrap_or(false),
                        cur_tool,
                        &installed_names,
                        &r.valve,
                    )
                };
                let name_cell: Element<Msg> = match sug {
                    Some(sg) => {
                        let label = if sg.tool.is_empty() {
                            "Steam default".to_string()
                        } else {
                            sg.tool.clone()
                        };
                        column![
                            text(name.clone()).size(TEXT_SIZE),
                            row![
                                text(format!("Suggested: {label}"))
                                    .size(SMALL_SIZE)
                                    .color(ACCENT),
                                button(text("Use").size(SMALL_SIZE))
                                    .padding([2, 10])
                                    .on_press(Msg::SetTool(r.root.clone(), appid, sg.tool.clone()))
                                    .style(primary_button()),
                            ]
                            .spacing(6)
                            .align_y(iced::Alignment::Center),
                            text(sg.why.clone()).size(SMALL_SIZE).color(DIM),
                        ]
                        .spacing(2)
                        .into()
                    }
                    None => text(name.clone()).size(TEXT_SIZE).into(),
                };
                let is_native = self.native.get(&appid).copied().unwrap_or(false);
                let tier_text: Element<Msg> = if is_native {
                    // a Linux build exists: it runs without Proton, so the Proton rating is beside the point
                    column![
                        text("native").size(SMALL_SIZE).color(GREEN),
                        text(match tier.as_deref() {
                            Some(t) => format!("Windows build: {t}"),
                            None => String::new(),
                        })
                        .size(SMALL_SIZE)
                        .color(DIM),
                    ]
                    .into()
                } else {
                    match tier.as_deref() {
                        Some(t) => {
                            let c = match t {
                                "platinum" => ACCENT,
                                "gold" => AMBER,
                                "borked" => RED,
                                _ => DIM,
                            };
                            text(t.to_string()).size(SMALL_SIZE).color(c).into()
                        }
                        None => text(if appid == 0 { "" } else { "-" })
                            .size(SMALL_SIZE)
                            .color(DIM)
                            .into(),
                    }
                };
                col = col.push(table_row(
                    vec![
                        (name_cell, 4),
                        (text(short.clone()).size(SMALL_SIZE).color(DIM).into(), 1),
                        (tier_text, 2),
                        (dropdown.into(), 3),
                        (launch_dd, 6),
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

    /// Valve's own Protons: which are installed in each Steam, and one click to use one for all games.
    fn valve_view(&self) -> Element<'_, Msg> {
        let mut col = Column::new().spacing(4);
        col = col.push(
            text("Valve's Protons are part of Steam. Steam downloads one by itself the first time a game is set to use it; this list sets it for you.")
                .size(SMALL_SIZE)
                .color(DIM),
        );
        col = col.push(table_header(&[
            ("Version", 3),
            ("Steam", 2),
            ("State", 3),
            ("", 3),
        ]));
        let mut n = 0usize;
        for r in &self.all {
            for (tool, label) in VALVE_TOOLS {
                let installed = r.valve.iter().any(|v| v == tool);
                let using_default = r.mapping.get(&0).map(String::as_str) == Some(tool);
                let state: Element<Msg> = if using_default {
                    text("default for all games")
                        .size(SMALL_SIZE)
                        .color(ACCENT)
                        .into()
                } else if installed {
                    text("installed").size(SMALL_SIZE).color(GREEN).into()
                } else {
                    text("downloaded by Steam on first use")
                        .size(SMALL_SIZE)
                        .color(DIM)
                        .into()
                };
                let action: Element<Msg> = if using_default {
                    text("").into()
                } else {
                    button(text("Use for all games").size(SMALL_SIZE))
                        .padding([6, 12])
                        .on_press(Msg::SetTool(r.root.clone(), 0, tool.to_string()))
                        .style(primary_button())
                        .into()
                };
                let short = r
                    .label
                    .trim_start_matches("Steam ")
                    .trim_matches(|c| c == '(' || c == ')')
                    .to_string();
                col = col.push(table_row(
                    vec![
                        (text(label).size(TEXT_SIZE).into(), 3),
                        (text(short).size(SMALL_SIZE).color(DIM).into(), 2),
                        (state, 3),
                        (action, 3),
                    ],
                    n % 2 == 1,
                ));
                n += 1;
            }
        }
        if self.all.is_empty() {
            col = col.push(text("Looking for Steam...").size(TEXT_SIZE).color(DIM));
        }
        scrollable(col.padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }

    fn available_view(&self) -> Element<'_, Msg> {
        if self.valve_view {
            return self.valve_view();
        }
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

/// Runs the blocking install on its own thread and turns its progress into a stream.
///
/// The thread reports through an unbounded channel and this side forwards each report with `.await`,
/// so the stream is polled the whole time. (An earlier version waited in a plain loop with no
/// `.await`: nothing drained the bounded channel, the install thread blocked on it after 64 reports
/// and the whole update hung on any download bigger than a few tens of megabytes.)
fn install_stream(
    src: &'static Source,
    rel: Release,
    dir: PathBuf,
) -> impl iced::futures::Stream<Item = Event> {
    use iced::futures::{SinkExt, StreamExt};
    iced::stream::channel(
        64,
        async move |mut tx: iced::futures::channel::mpsc::Sender<Event>| {
            let tag = rel.tag_name.clone();
            let (ptx, mut prx) = iced::futures::channel::mpsc::unbounded::<Progress>();
            let handle = std::thread::spawn(move || {
                install::install(src, &rel, &dir, &mut |p| {
                    let _ = ptx.unbounded_send(p);
                })
            });
            // ends when the thread is done and drops its sender
            while let Some(p) = prx.next().await {
                if tx.send(Event::Progress(tag.clone(), p)).await.is_err() {
                    break;
                }
            }
            let res = handle
                .join()
                .unwrap_or_else(|_| Err("install thread crashed".into()));
            let _ = tx.send(Event::Finished(tag, res)).await;
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Asset;
    use iced::futures::StreamExt;
    use std::io::{BufRead, BufReader, Write};

    /// A big archive of incompressible data, so the download reports many times.
    fn big_archive(tag: &str, mb: usize) -> Vec<u8> {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        let mut add = |path: String, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, data).unwrap();
        };
        add(
            format!("{tag}/compatibilitytool.vdf"),
            b"\"compatibilitytools\" {}",
        );
        let mut x: u64 = 0x2545F4914F6CDD1D;
        let data: Vec<u8> = (0..mb * 1024 * 1024)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        add(format!("{tag}/files/blob"), &data);
        b.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn a_big_install_streams_all_its_progress_and_finishes() {
        let tag = "GE-Proton99-1";
        let data = big_archive(tag, 48);
        let sum: String = {
            use sha2::{Digest, Sha512};
            Sha512::digest(&data)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let sums = format!("{sum}  {tag}.tar.gz\n");
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let files = std::sync::Arc::new(vec![
            (format!("/{tag}.tar.gz"), data),
            (format!("/{tag}.sha512sum"), sums.into_bytes()),
        ]);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut s = s;
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    match files.iter().find(|(p, _)| *p == path) {
                        Some((_, d)) => {
                            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", d.len());
                            let _ = s.write_all(d);
                        }
                        None => {
                            let _ =
                                write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                        }
                    }
                });
            }
        });
        let base = format!("http://127.0.0.1:{port}");
        let rel = Release {
            tag_name: tag.into(),
            published_at: String::new(),
            prerelease: false,
            assets: vec![
                Asset {
                    name: format!("{tag}.tar.gz"),
                    browser_download_url: format!("{base}/{tag}.tar.gz"),
                    size: 0,
                },
                Asset {
                    name: format!("{tag}.sha512sum"),
                    browser_download_url: format!("{base}/{tag}.sha512sum"),
                    size: 0,
                },
            ],
        };
        let dir = std::env::temp_dir().join(format!("gui-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = sources::find("ge").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let d2 = dir.clone();
        std::thread::spawn(move || {
            let stream = install_stream(src, rel, d2);
            let events: Vec<Event> = iced::futures::executor::block_on(stream.collect());
            let _ = tx.send(events);
        });
        let events = rx
            .recv_timeout(std::time::Duration::from_secs(120))
            .expect("the install stream hung (the progress channel deadlocked)");
        let progress = events
            .iter()
            .filter(|e| matches!(e, Event::Progress(..)))
            .count();
        assert!(
            progress > 64,
            "{progress} progress reports: enough to fill a 64-slot channel"
        );
        assert!(
            matches!(events.last(), Some(Event::Finished(_, Ok(_)))),
            "{:?}",
            events.last()
        );
        assert!(dir.join(tag).join("files/blob").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
