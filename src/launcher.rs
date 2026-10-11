//! The one program: Home, ProtonUp, Flatseal and Settings (System + Log) as four tabs of one window.
//! Each tab has the same layout (`ui::shell`: a sidebar and a main area); this file only switches.
use crate::ui::{accent_button, card, nav_item, page_title, quiet_button, shell, theme, ACCENT, PAD, SMALL_SIZE, SPACING, TEXT_SIZE};
use crate::{log, proton_gui, seal_gui, steaminstalls, system};
use iced::widget::{button, column, container, scrollable, text};
use iced::{Element, Fill, Task};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Home,
    Games,
    Get,
    Installed,
    Perms,
    System,
    Steam,
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
    Proton(proton_gui::Msg),
    Seal(seal_gui::Msg),
    System(system::Msg),
    Steam(steaminstalls::Msg),
    RefreshLog,
}

struct App {
    section: Section,
    proton: proton_gui::App,
    seal: seal_gui::App,
    system: system::State,
    steam: steaminstalls::State,
    seen: [Option<String>; 2],
}

pub fn run(start: Start) -> iced::Result {
    log::install_panic_hook();
    log::write("launcher", "start");
    iced::application(move || App::new(start.clone()), App::update, App::view)
        .title("Steam Deck")
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
        let section = match (start.section, start.tab.as_deref()) {
            (Section::Games, Some("available")) => Section::Get,
            (Section::Games, Some("installed")) => Section::Installed,
            (s, _) => s,
        };
        let app = App { section, proton, seal, system, steam, seen: [None, None] };
        (app, Task::batch([pt.map(Msg::Proton), st.map(Msg::Seal), yt.map(Msg::System), wt.map(Msg::Steam)]))
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
            Msg::Proton(m) => self.proton.update(m).map(Msg::Proton),
            Msg::Seal(m) => self.seal.update(m).map(Msg::Seal),
            Msg::System(m) => self.system.update(m).map(Msg::System),
            Msg::Steam(m) => self.steam.update(m).map(Msg::Steam),
            Msg::RefreshLog => Task::none(),
        };
        // errors shown in a section also go to the log file, once each
        for (i, (who, e)) in [("protonup", self.proton.last_error()), ("flatseal", self.seal.last_error())].into_iter().enumerate() {
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
        let nav = |label: &'static str, s: Section| nav_item(label, self.section == s, Msg::Go(s));
        let in_proton = matches!(self.section, Section::Games | Section::Get | Section::Installed);
        let in_seal = self.section == Section::Perms;
        let in_settings = matches!(self.section, Section::System | Section::Steam | Section::Log);
        let top_tab = |label: &'static str, goto: Section, on: bool| {
            let b = button(text(label).size(TEXT_SIZE)).padding([8, 18]).on_press(Msg::Go(goto));
            if on { b.style(accent_button()) } else { b.style(quiet_button()) }
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
                    nav("Get a new Proton", Section::Get),
                    nav("Installed", Section::Installed),
                ]
                .spacing(8)
                .into(),
            )
        } else if in_seal {
            Some(self.seal.side(true).map(Msg::Seal))
        } else if in_settings {
            Some(column![nav("System tweaks", Section::System), nav("Steam installs", Section::Steam), nav("Log", Section::Log)].spacing(8).into())
        } else {
            None
        };

        let proton_page = |title: &str, about: &str| -> Element<'_, Msg> {
            column![page_title(title, about), self.proton.main().map(Msg::Proton)].spacing(SPACING).into()
        };
        let main: Element<Msg> = match self.section {
            Section::Home => column![
                page_title("Steam Deck", "Pick what you want to do."),
                if self.steam.count() > 1 {
                    card("Two Steam installs found", "Flatpak and system Steam are both here. Pick which one to uninstall.", "Choose", Msg::Go(Section::Steam))
                } else {
                    column![].into()
                },
                card("Choose Proton for a game", "Windows games run through Proton. Pick which version each game uses.", "Open", Msg::Go(Section::Games)),
                card("Get a newer Proton", "Download Proton-GE, CachyOS and others. A newer one often fixes a game that does not start.", "Open", Msg::Go(Section::Get)),
                card("Flatseal: what apps may touch", "Flatpak apps: allow or block network, sound, files and more, per app or for all.", "Open", Msg::Go(Section::Perms)),
                card("Settings: make the Deck steadier", "Memory guard, swap size, ssh. Changes only /etc and can be undone.", "Open", Msg::Go(Section::System)),
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
