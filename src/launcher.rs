//! The one program: ProtonUp, Flatseal, System and Log as sections of a single window. Each section
//! keeps its own sidebar layout (`ui::shell`); this file only switches between them.
use crate::ui::{heading, nav_item, quiet_button, shell, theme, SMALL_SIZE, SPACING};
use crate::{log, proton_gui, seal_gui, system};
use iced::widget::{button, column, container, scrollable, text};
use iced::{Element, Fill, Task};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Proton,
    Seal,
    System,
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
    Section(Section),
    Proton(proton_gui::Msg),
    Seal(seal_gui::Msg),
    System(system::Msg),
    RefreshLog,
}

struct App {
    section: Section,
    proton: proton_gui::App,
    seal: seal_gui::App,
    system: system::State,
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
        let app = App { section: start.section, proton, seal, system, seen: [None, None] };
        (app, Task::batch([pt.map(Msg::Proton), st.map(Msg::Seal), yt.map(Msg::System)]))
    }

    fn update(&mut self, m: Msg) -> Task<Msg> {
        let t = match m {
            Msg::Section(s) => {
                self.section = s;
                Task::none()
            }
            Msg::Proton(m) => {
                if matches!(m, proton_gui::Msg::Tab(_)) {
                    self.section = Section::Proton;
                }
                self.proton.update(m).map(Msg::Proton)
            }
            Msg::Seal(m) => {
                if matches!(m, seal_gui::Msg::Select(_)) {
                    self.section = Section::Seal;
                }
                self.seal.update(m).map(Msg::Seal)
            }
            Msg::System(m) => self.system.update(m).map(Msg::System),
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

    /// One sidebar for everything: ProtonUp's pages, Flatseal's apps, System and Log.
    fn view(&self) -> Element<'_, Msg> {
        let nav = |label: &'static str, s: Section| nav_item(label, self.section == s, Msg::Section(s));
        let side = column![
            heading("ProtonUp"),
            self.proton.side(self.section == Section::Proton).map(Msg::Proton),
            heading("Flatseal"),
            container(self.seal.side(self.section == Section::Seal).map(Msg::Seal)).height(Fill),
            nav("System", Section::System),
            nav("Log", Section::Log),
        ]
        .spacing(8)
        .height(Fill);
        let main: Element<Msg> = match self.section {
            Section::Proton => self.proton.main().map(Msg::Proton),
            Section::Seal => self.seal.main().map(Msg::Seal),
            Section::System => self.system.view().map(Msg::System),
            Section::Log => column![
                text(log::path().display().to_string()).size(SMALL_SIZE),
                button(text("Refresh").size(SMALL_SIZE)).padding(8).on_press(Msg::RefreshLog).style(quiet_button()),
                scrollable(text(log::tail(300)).size(SMALL_SIZE)).height(Fill).width(Fill),
            ]
            .spacing(SPACING)
            .into(),
        };
        shell(side.into(), main)
    }
}
