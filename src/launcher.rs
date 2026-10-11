//! The one program: Home, ProtonUp, Flatseal and Settings (System + Log) as four tabs of one window.
//! Each tab has the same layout (`ui::shell`: a sidebar and a main area); this file only switches.
use crate::ui::{
    accent_button, card_busy, nav_item, page_title, quiet_button, shell, theme, warning_card,
    ACCENT, PAD, SMALL_SIZE, SPACING, TEXT_SIZE,
};
use crate::{flatapps, health, log, proton_gui, seal_gui, steaminstalls, storage_gui, system};
use iced::widget::{button, column, container, scrollable, text};
use iced::{Element, Fill, Task};
use std::path::PathBuf;

/// Which Home button started the running update, so only that one shows the spinner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Protons,
    Flatpaks,
    Everything,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Home,
    Games,
    Get,
    Installed,
    Storage,
    Perms,
    System,
    Steam,
    Apps,
    Health,
    Log,
}

/// Where the window opens.
#[derive(Debug, Clone, Default)]
pub struct Start {
    pub section: Section,
    /// pin ProtonUp to this compatibilitytools.d
    pub dir: Option<PathBuf>,
    /// ProtonUp tab: installed, available, games
    pub tab: Option<String>,
    /// Flatseal: the app to open
    pub app: Option<String>,
}

#[derive(Debug, Clone)]
enum Msg {
    Go(Section),
    UpdateProtons,
    UpdateFlatpaks,
    UpdateEverything,
    Proton(proton_gui::Msg),
    Seal(seal_gui::Msg),
    System(system::Msg),
    Steam(steaminstalls::Msg),
    Apps(flatapps::Msg),
    Health(health::Msg),
    Storage(storage_gui::Msg),
    RefreshLog,
    Key(iced::keyboard::Key, iced::keyboard::Modifiers),
    Tick,
}

struct App {
    section: Section,
    proton: proton_gui::App,
    seal: seal_gui::App,
    system: system::State,
    steam: steaminstalls::State,
    apps: flatapps::State,
    health: health::State,
    storage: storage_gui::State,
    seen: [Option<String>; 2],
    tick: usize,
    frames: usize,
    job: Option<Job>,
}

pub fn run(start: Start) -> iced::Result {
    log::install_panic_hook();
    log::write("launcher", "start");
    iced::application(move || App::new(start.clone()), App::update, App::view)
        .title("Steam Deck")
        .subscription(|app: &App| {
            let keys = iced::keyboard::listen().filter_map(|e| match e {
                iced::keyboard::Event::KeyPressed { key, modifiers, .. } => {
                    Some(Msg::Key(key, modifiers))
                }
                _ => None,
            });
            // the spinner and the clocks only tick while something runs
            if app.proton.is_busy() || app.apps.is_updating() {
                iced::Subscription::batch([keys, iced::window::frames().map(|_| Msg::Tick)])
            } else {
                keys
            }
        })
        .theme(|_: &App| theme())
        .window_size((1100.0, 700.0))
        .run()
}

impl App {
    fn new(start: Start) -> (Self, Task<Msg>) {
        let home = dirs::home_dir().unwrap_or_default();
        let pinned = start.dir.is_some();
        let dir = crate::steam::default_tool_dir(&home, start.dir.as_deref());
        let (proton, pt) = proton_gui::App::new(dir, pinned, start.tab.as_deref());
        let (seal, st) = seal_gui::App::new(start.app.clone());
        let (system, yt) = system::State::new();
        let (steam, wt) = steaminstalls::State::new();
        let (apps, at) = flatapps::State::new();
        let (health, ht) = health::State::new();
        let (storage, gt) = storage_gui::State::new();
        let section = match (start.section, start.tab.as_deref()) {
            (Section::Games, Some("available")) => Section::Get,
            (Section::Games, Some("installed")) => Section::Installed,
            (s, _) => s,
        };
        let app = App {
            section,
            proton,
            seal,
            system,
            steam,
            apps,
            health,
            storage,
            seen: [None, None],
            tick: 0,
            frames: 0,
            job: None,
        };
        (
            app,
            Task::batch([
                pt.map(Msg::Proton),
                st.map(Msg::Seal),
                yt.map(Msg::System),
                at.map(Msg::Apps),
                ht.map(Msg::Health),
                gt.map(Msg::Storage),
                wt.map(Msg::Steam),
            ]),
        )
    }

    fn update(&mut self, m: Msg) -> Task<Msg> {
        let t = match m {
            Msg::Go(sec) => {
                self.section = sec;
                let tab = match sec {
                    Section::Games => Some(proton_gui::Tab::Games),
                    Section::Get => Some(proton_gui::Tab::Available),
                    Section::Installed => Some(proton_gui::Tab::Installed),
                    _ => None,
                };
                match tab {
                    Some(t) => self.proton.update(proton_gui::Msg::Tab(t)).map(Msg::Proton),
                    None => Task::none(),
                }
            }
            Msg::UpdateFlatpaks => {
                self.job = Some(Job::Flatpaks);
                self.apps.update(flatapps::Msg::UpdateAll).map(Msg::Apps)
            }
            Msg::UpdateEverything => {
                self.job = Some(Job::Everything);
                Task::batch([
                    self.proton
                        .update(proton_gui::Msg::UpdateAll)
                        .map(Msg::Proton),
                    self.apps.update(flatapps::Msg::UpdateAll).map(Msg::Apps),
                ])
            }
            // stays on Home: the button itself shows that it is working
            Msg::UpdateProtons => {
                self.job = Some(Job::Protons);
                self.proton
                    .update(proton_gui::Msg::UpdateAll)
                    .map(Msg::Proton)
            }
            Msg::Proton(m) => self.proton.update(m).map(Msg::Proton),
            Msg::Seal(m) => self.seal.update(m).map(Msg::Seal),
            Msg::System(m) => self.system.update(m).map(Msg::System),
            Msg::Steam(m) => self.steam.update(m).map(Msg::Steam),
            Msg::Apps(m) => self.apps.update(m).map(Msg::Apps),
            Msg::Health(m) => self.health.update(m).map(Msg::Health),
            Msg::Storage(m) => self.storage.update(m).map(Msg::Storage),
            Msg::RefreshLog => Task::none(),
            Msg::Tick => {
                // frames come at the display rate; the spinner turns about eight times a second
                self.frames = self.frames.wrapping_add(1);
                self.tick = self.frames / 8;
                Task::none()
            }
            // Alt+1..4 switch the tabs (plain digits would land in the text boxes)
            Msg::Key(key, mods) => {
                if mods.alt() {
                    let go = match key.as_ref() {
                        iced::keyboard::Key::Character("1") => Some(Section::Home),
                        iced::keyboard::Key::Character("2") => Some(Section::Games),
                        iced::keyboard::Key::Character("3") => Some(Section::Perms),
                        iced::keyboard::Key::Character("4") => Some(Section::System),
                        _ => None,
                    };
                    if let Some(s) = go {
                        return self.update(Msg::Go(s));
                    }
                }
                Task::none()
            }
        };
        // errors shown in a section also go to the log file, once each
        for (i, (who, e)) in [
            ("protonup", self.proton.last_error()),
            ("flatseal", self.seal.last_error()),
        ]
        .into_iter()
        .enumerate()
        {
            let e = e.map(str::to_string);
            if e != self.seen[i] {
                if let Some(e) = &e {
                    log::write(who, e);
                }
                self.seen[i] = e;
            }
        }
        t
    }

    /// Four tabs on top (Home, ProtonUp, Flatseal, Settings); the sidebar belongs to the tab.
    fn view(&self) -> Element<'_, Msg> {
        let running = self.proton.is_busy() || self.apps.is_updating();
        let nav = |label: &'static str, s: Section| nav_item(label, self.section == s, Msg::Go(s));
        let in_proton = matches!(
            self.section,
            Section::Games | Section::Get | Section::Installed
        );
        let in_seal = self.section == Section::Perms;
        let in_settings = matches!(
            self.section,
            Section::System | Section::Steam | Section::Apps | Section::Health | Section::Log
        );
        let top_tab = |label: &'static str, goto: Section, on: bool| {
            let b = button(text(label).size(TEXT_SIZE))
                .padding([8, 18])
                .on_press(Msg::Go(goto));
            if on {
                b.style(accent_button())
            } else {
                b.style(quiet_button())
            }
        };
        let bar = iced::widget::row![
            text("Steam Deck").size(TEXT_SIZE).color(ACCENT),
            iced::widget::space::horizontal(),
            top_tab("Home", Section::Home, self.section == Section::Home),
            top_tab("ProtonUp", Section::Games, in_proton),
            top_tab("Flatseal", Section::Perms, in_seal),
            top_tab("Settings", Section::System, in_settings),
        ]
        .spacing(SPACING)
        .align_y(iced::Alignment::Center);

        let side: Option<Element<Msg>> = if in_proton {
            Some(
                column![
                    nav("Proton for my games", Section::Games),
                    nav("Download Proton", Section::Get),
                    nav("Game storage", Section::Storage),
                ]
                .spacing(8)
                .into(),
            )
        } else if in_seal {
            Some(self.seal.side(true).map(Msg::Seal))
        } else if in_settings {
            Some(
                column![
                    nav("System tweaks", Section::System),
                    nav("Health check", Section::Health),
                    nav("Flatpak apps", Section::Apps),
                    nav("Steam installs", Section::Steam),
                    nav("Log", Section::Log)
                ]
                .spacing(8)
                .into(),
            )
        } else {
            None
        };

        let proton_page = |title: &str, about: &str| -> Element<'_, Msg> {
            column![
                page_title(title, about),
                self.proton.main().map(Msg::Proton)
            ]
            .spacing(SPACING)
            .into()
        };
        let main: Element<Msg> = match self.section {
            Section::Home => column![
                page_title("Steam Deck", "One click each. Results appear below."),
                if self.steam.count() > 1 {
                    warning_card(
                        "Two Steam installs found",
                        "Flatpak and system Steam are both here, and games can be split between them. Pick which one to uninstall.",
                        "Choose",
                        Msg::Go(Section::Steam),
                    )
                } else {
                    column![].into()
                },
                card_busy(
                    "Update Protons",
                    "Installs the newest version of every Proton you have and removes the older ones. Games on an old one are moved to the new one.",
                    "Update",
                    running && self.job == Some(Job::Protons),
                    self.tick,
                    Msg::UpdateProtons
                ),
                card_busy(
                    "Update Flatpak apps",
                    "flatpak update for every installed app and runtime.",
                    "Update",
                    running && self.job == Some(Job::Flatpaks),
                    self.tick,
                    Msg::UpdateFlatpaks
                ),
                card_busy(
                    "Update everything",
                    "Both of the above.",
                    "Update all",
                    running && self.job == Some(Job::Everything),
                    self.tick,
                    Msg::UpdateEverything
                ),
                self.proton.report_view().map(Msg::Proton),
                self.apps.report_view::<Msg>(),
            ]
            .spacing(SPACING)
            .into(),
            Section::Games => proton_page("Proton for my games", "Pick the Proton each game runs with. Close Steam first, or Steam undoes the change."),
            Section::Get => proton_page("Get a new Proton", "Choose a category, then press Install on a version. It appears under Installed."),
            Section::Installed => proton_page("Installed", "Everything installed in your Steam installs. Remove what you do not use."),
            Section::Perms => column![
                page_title("Flatseal", "Pick an app on the left, then choose for each item: default, allow (on) or block (off)."),
                self.seal.main().map(Msg::Seal),
            ]
            .spacing(SPACING)
            .into(),
            Section::System => column![
                page_title("Settings: system tweaks", "Each button changes one thing in /etc and asks for your password."),
                self.system.view().map(Msg::System),
            ]
            .spacing(SPACING)
            .into(),
            Section::Steam => column![
                page_title("Settings: Steam installs", "Flatpak and system Steam side by side; remove the one you do not use."),
                self.steam.view().map(Msg::Steam),
            ]
            .spacing(SPACING)
            .into(),
            Section::Apps => column![
                page_title("Settings: Flatpak apps", "Uninstall apps, delete what they keep, and free unused runtimes."),
                self.apps.view().map(Msg::Apps),
            ]
            .spacing(SPACING)
            .into(),
            Section::Storage => column![
                page_title("Game storage", "Prefixes and shader caches. A game that will not start often needs its prefix reset; leftovers of removed games can go."),
                self.storage.view().map(Msg::Storage),
            ]
            .spacing(SPACING)
            .into(),
            Section::Health => column![
                page_title("Settings: health check", "Read only. Fix what is marked with System tweaks."),
                self.health.view().map(Msg::Health),
            ]
            .spacing(SPACING)
            .into(),
            Section::Log => column![
                page_title("Settings: log", "What this program did and any errors. Send this file if something breaks."),
                text(log::path().display().to_string()).size(SMALL_SIZE),
                button(text("Refresh").size(SMALL_SIZE)).padding(8).on_press(Msg::RefreshLog).style(quiet_button()),
                scrollable(text(log::tail(300)).size(SMALL_SIZE)).height(Fill).width(Fill),
            ]
            .spacing(SPACING)
            .into(),
        };
        let body: Element<Msg> = match side {
            Some(side) => shell(side, main),
            None => container(main).padding(PAD).into(),
        };
        column![container(bar).padding(PAD), body].into()
    }
}
