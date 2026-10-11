//! The window: apps on the left, the permissions of the selected one on the right. Every change is
//! written to the override file at once, like the GTK original does.
use deck_common::{
    accent_button, danger_button, quiet_button, theme, AMBER, DIM, GREEN, PAD, PURPLE, RED,
    SMALL_SIZE, SPACING, TEXT_SIZE,
};
use flatseal::flatpak::{self, App as FpApp, GLOBAL};
use flatseal::perms::{self, Layers, Tri, SECTIONS};
use flatseal::portals::{self, Answer, Grant, PORTALS};
use iced::widget::{
    button, column, container, pick_list, row, scrollable, space, text, text_input, Column,
};
use iced::{Element, Fill, Length, Task};

pub fn run(select: Option<String>) -> iced::Result {
    iced::application(move || App::new(select.clone()), App::update, App::view)
        .title("Flatseal")
        .theme(|_: &App| theme())
        .window_size((1100.0, 700.0))
        .run()
}

#[derive(Debug, Clone)]
enum Msg {
    Apps(Result<Vec<FpApp>, String>),
    Select(String),
    /// the permission-store entries of an app (app id, entries or the error)
    Grants(String, Result<Vec<Grant>, String>),
    Portal(usize, Answer),
    Revoke(String, String),
    Search(String),
    Toggle(&'static str, &'static str, Tri),
    FsInput(String),
    FsAdd,
    FsRemove(String),
    BusName(String),
    BusPolicy(&'static str),
    BusAdd(&'static str),
    BusSet(&'static str, String, &'static str),
    BusClear(&'static str, String),
    EnvName(String),
    EnvValue(String),
    EnvAdd,
    EnvClear(String),
    Reset,
    AskReset,
    CancelReset,
}

const POLICIES: [&str; 4] = ["none", "see", "talk", "own"];

struct App {
    apps: Vec<FpApp>,
    error: Option<String>,
    search: String,
    selected: String,
    layers: Layers,
    saved: bool,
    fs_input: String,
    bus_name: String,
    bus_policy: &'static str,
    env_name: String,
    env_value: String,
    confirm_reset: bool,
    loading: bool,
    grants: Vec<Grant>,
}

impl App {
    fn new(select: Option<String>) -> (Self, Task<Msg>) {
        // `flatpak list` is slow: the window opens first, the app list arrives when it is done
        let (apps, error) = (vec![], None);
        let load = Task::perform(async { flatpak::list_apps() }, Msg::Apps);
        let mut app = Self::blank(apps, error);
        app.loading = true;
        if let Some(id) = select.filter(|s| !s.is_empty()) {
            app.selected = id;
            app.layers = flatpak::load_layers(&app.selected);
            let grants = app.load_grants();
            return (app, Task::batch([load, grants]));
        }
        (app, load)
    }

    fn blank(apps: Vec<FpApp>, error: Option<String>) -> Self {
        let selected = GLOBAL.to_string();
        let layers = flatpak::load_layers(&selected);
        App {
            apps,
            error,
            search: String::new(),
            selected,
            layers,
            saved: false,
            fs_input: String::new(),
            bus_name: String::new(),
            bus_policy: "talk",
            env_name: String::new(),
            env_value: String::new(),
            confirm_reset: false,
            loading: false,
            grants: Vec::new(),
        }
    }

    fn persist(&mut self) {
        match flatpak::save_override(&self.selected, &self.layers.app) {
            Ok(()) => {
                self.saved = true;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn load_grants(&self) -> Task<Msg> {
        if self.selected == GLOBAL {
            return Task::none();
        }
        let app = self.selected.clone();
        Task::perform(
            async move { (app.clone(), portals::grants(&app)) },
            |(a, g)| Msg::Grants(a, Ok(g)),
        )
    }

    fn update(&mut self, m: Msg) -> Task<Msg> {
        self.saved = false;
        match m {
            Msg::Apps(r) => {
                self.loading = false;
                match r {
                    Ok(a) => self.apps = a,
                    Err(e) => self.error = Some(e),
                }
            }
            Msg::Select(id) => {
                self.selected = id;
                self.layers = flatpak::load_layers(&self.selected);
                self.confirm_reset = false;
                self.grants.clear();
                return self.load_grants();
            }
            Msg::Grants(app, r) => {
                if app == self.selected {
                    match r {
                        Ok(g) => self.grants = g,
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            Msg::Portal(i, a) => {
                let app = self.selected.clone();
                return Task::perform(
                    async move {
                        let r = portals::set(&app, &PORTALS[i], a);
                        let g = portals::grants(&app);
                        (app, r.map(|_| g))
                    },
                    |(a, r)| Msg::Grants(a, r),
                );
            }
            Msg::Revoke(table, id) => {
                let app = self.selected.clone();
                return Task::perform(
                    async move {
                        let r = portals::revoke(&app, &table, &id);
                        let g = portals::grants(&app);
                        (app, r.map(|_| g))
                    },
                    |(a, r)| Msg::Grants(a, r),
                );
            }
            Msg::Search(s) => self.search = s,
            Msg::Toggle(key, name, st) => {
                self.layers.set_state(key, name, st);
                self.persist();
            }
            Msg::FsInput(s) => self.fs_input = s,
            Msg::FsAdd => {
                let v = std::mem::take(&mut self.fs_input);
                self.layers.add_filesystem(&v);
                self.persist();
            }
            Msg::FsRemove(e) => {
                self.layers.remove_filesystem(&e);
                self.persist();
            }
            Msg::BusName(s) => self.bus_name = s,
            Msg::BusPolicy(p) => self.bus_policy = p,
            Msg::BusAdd(group) => {
                let n = std::mem::take(&mut self.bus_name);
                self.layers.set_bus(group, &n, self.bus_policy);
                self.persist();
            }
            Msg::BusSet(group, name, p) => {
                self.layers.set_bus(group, &name, p);
                self.persist();
            }
            Msg::BusClear(group, name) => {
                self.layers.clear_bus(group, &name);
                self.persist();
            }
            Msg::EnvName(s) => self.env_name = s,
            Msg::EnvValue(s) => self.env_value = s,
            Msg::EnvAdd => {
                let n = std::mem::take(&mut self.env_name);
                let v = std::mem::take(&mut self.env_value);
                self.layers.set_env(&n, &v);
                self.persist();
            }
            Msg::EnvClear(n) => {
                self.layers.clear_env(&n);
                self.persist();
            }
            Msg::AskReset => self.confirm_reset = true,
            Msg::CancelReset => self.confirm_reset = false,
            Msg::Reset => {
                self.confirm_reset = false;
                self.layers.reset();
                self.persist();
                if self.selected != GLOBAL {
                    let app = self.selected.clone();
                    return Task::perform(
                        async move {
                            let r = portals::reset(&app);
                            let g = portals::grants(&app);
                            (app, r.map(|_| g))
                        },
                        |(a, r)| Msg::Grants(a, r),
                    );
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Msg> {
        // left: the app list
        let q = self.search.to_lowercase();
        let mut items: Vec<Element<Msg>> = Vec::new();
        let mut entry = |id: String, label: String| {
            let b = button(text(label).size(SMALL_SIZE))
                .padding(10)
                .width(Fill)
                .on_press(Msg::Select(id.clone()));
            items.push(if id == self.selected {
                b.style(accent_button()).into()
            } else {
                b.style(quiet_button()).into()
            });
        };
        entry(GLOBAL.to_string(), "All apps (global)".to_string());
        for a in &self.apps {
            if q.is_empty()
                || a.name.to_lowercase().contains(&q)
                || a.id.to_lowercase().contains(&q)
            {
                entry(a.id.clone(), a.name.clone());
            }
        }
        let list = Column::with_children(items).spacing(4);
        let left = column![
            text_input("Search", &self.search)
                .on_input(Msg::Search)
                .padding(10)
                .size(SMALL_SIZE),
            scrollable(list).height(Fill),
        ]
        .spacing(SPACING)
        .width(Length::Fixed(190.0));

        // right: the permissions
        let mut right = Column::new().spacing(SPACING);
        let mut head = row![space::horizontal()]
            .spacing(SPACING)
            .align_y(iced::Alignment::Center);
        if self.saved {
            head = head.push(text("saved").size(SMALL_SIZE).color(GREEN));
        }
        if self.confirm_reset {
            head = head
                .push(text("Drop all overrides?").size(SMALL_SIZE).color(AMBER))
                .push(
                    button(text("Yes").size(SMALL_SIZE))
                        .padding(10)
                        .on_press(Msg::Reset)
                        .style(danger_button()),
                )
                .push(
                    button(text("No").size(SMALL_SIZE))
                        .padding(10)
                        .on_press(Msg::CancelReset)
                        .style(quiet_button()),
                );
        } else {
            head = head.push(
                button(text("Reset").size(SMALL_SIZE))
                    .padding(10)
                    .on_press(Msg::AskReset)
                    .style(quiet_button()),
            );
        }
        right = right.push(head);
        if let Some(e) = &self.error {
            right = right.push(text(e.clone()).size(SMALL_SIZE).color(RED));
        }

        for s in SECTIONS {
            right = right.push(text(s.title).size(TEXT_SIZE).color(PURPLE));
            for (name, desc) in s.items {
                let on = self.layers.is_on(s.key, name);
                let own = self.layers.own_state(s.key, name);
                let key = s.key;
                let nm: &'static str = name;
                let pick = |label: &'static str, st: Tri| {
                    let b = button(text(label).size(SMALL_SIZE))
                        .padding([6, 10])
                        .on_press(Msg::Toggle(key, nm, st));
                    if own != st {
                        b.style(quiet_button())
                    } else if st == Tri::Deny {
                        b.style(danger_button())
                    } else {
                        b.style(accent_button())
                    }
                };
                right = right.push(
                    column![
                        row![
                            text(name.to_string()).size(TEXT_SIZE),
                            space::horizontal(),
                            pick("default", Tri::Default),
                            pick("on", Tri::Allow),
                            pick("off", Tri::Deny),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        text(format!("{desc}  ·  now {}", if on { "on" } else { "off" }))
                            .size(SMALL_SIZE)
                            .color(DIM),
                    ]
                    .spacing(2),
                );
            }
            if s.key == "filesystems" {
                for e in self.layers.custom_filesystems() {
                    right = right.push(
                        row![
                            text(e.name.clone()).size(SMALL_SIZE),
                            space::horizontal(),
                            text(if e.overridden { "overridden" } else { "" })
                                .size(SMALL_SIZE)
                                .color(AMBER),
                            button(text("Remove").size(SMALL_SIZE))
                                .padding(8)
                                .on_press(Msg::FsRemove(e.name.clone()))
                                .style(quiet_button()),
                        ]
                        .spacing(SPACING)
                        .align_y(iced::Alignment::Center),
                    );
                }
                right = right.push(
                    row![
                        text_input("~/Games  or  /mnt/sdcard:ro", &self.fs_input)
                            .on_input(Msg::FsInput)
                            .on_submit(Msg::FsAdd)
                            .padding(10)
                            .size(SMALL_SIZE),
                        button(text("Add path").size(SMALL_SIZE))
                            .padding(10)
                            .on_press(Msg::FsAdd)
                            .style(accent_button()),
                    ]
                    .spacing(SPACING),
                );
            }
        }

        if self.selected != GLOBAL {
            right = right.push(text("Portals").size(TEXT_SIZE).color(PURPLE));
            for (i, p) in PORTALS.iter().enumerate() {
                let a = portals::answer(&self.grants, p);
                let pick = |label: &'static str, v: Answer| {
                    let b = button(text(label).size(SMALL_SIZE))
                        .padding([6, 10])
                        .on_press(Msg::Portal(i, v));
                    if a != v {
                        b.style(quiet_button())
                    } else if v == Answer::No {
                        b.style(danger_button())
                    } else {
                        b.style(accent_button())
                    }
                };
                right = right.push(
                    column![
                        row![
                            text(p.name).size(TEXT_SIZE),
                            space::horizontal(),
                            pick("default", Answer::Default),
                            pick("yes", Answer::Yes),
                            pick("no", Answer::No),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        text(p.about).size(SMALL_SIZE).color(DIM),
                    ]
                    .spacing(2),
                );
            }
            let files: Vec<&Grant> = self
                .grants
                .iter()
                .filter(|g| g.table == "documents")
                .collect();
            if !files.is_empty() {
                right = right.push(
                    text(format!(
                        "Files granted through the file chooser ({})",
                        files.len()
                    ))
                    .size(SMALL_SIZE)
                    .color(DIM),
                );
                for g in files {
                    let label = portals::document_path(&g.data).unwrap_or_else(|| g.id.clone());
                    right = right.push(
                        row![
                            text(label)
                                .size(SMALL_SIZE)
                                .width(Length::Fill)
                                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                            button(text("Revoke").size(SMALL_SIZE))
                                .padding(8)
                                .on_press(Msg::Revoke(g.table.clone(), g.id.clone()))
                                .style(quiet_button()),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                    );
                }
            }
        }

        for (title, group) in [
            ("Session bus", perms::SESSION_BUS),
            ("System bus", perms::SYSTEM_BUS),
        ] {
            right = right.push(text(title).size(TEXT_SIZE).color(PURPLE));
            for e in self.layers.bus(group) {
                let mut r = row![text(e.name.clone()).size(SMALL_SIZE), space::horizontal()]
                    .spacing(8)
                    .align_y(iced::Alignment::Center);
                for p in POLICIES {
                    let b = button(text(p).size(SMALL_SIZE))
                        .padding(8)
                        .on_press(Msg::BusSet(group, e.name.clone(), p));
                    r = r.push(if e.value == p {
                        b.style(accent_button())
                    } else {
                        b.style(quiet_button())
                    });
                }
                if e.overridden {
                    r = r.push(
                        button(text("Reset").size(SMALL_SIZE))
                            .padding(8)
                            .on_press(Msg::BusClear(group, e.name.clone()))
                            .style(quiet_button()),
                    );
                }
                right = right.push(r);
            }
            right = right.push(
                row![
                    text_input("org.example.Service", &self.bus_name)
                        .on_input(Msg::BusName)
                        .on_submit(Msg::BusAdd(group))
                        .padding(10)
                        .size(SMALL_SIZE),
                    pick_list(POLICIES, Some(self.bus_policy), |p| Msg::BusPolicy(
                        POLICIES
                            .iter()
                            .find(|x| **x == p)
                            .copied()
                            .unwrap_or("talk")
                    )),
                    button(text("Add").size(SMALL_SIZE))
                        .padding(10)
                        .on_press(Msg::BusAdd(group))
                        .style(accent_button()),
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center),
            );
        }

        right = right.push(text("Environment").size(TEXT_SIZE).color(PURPLE));
        for e in self.layers.env() {
            let mut r = row![
                text(e.name.clone())
                    .size(SMALL_SIZE)
                    .color(GREEN)
                    .width(Length::FillPortion(2)),
                text(if e.value.is_empty() {
                    "(empty)".to_string()
                } else {
                    e.value.clone()
                })
                .size(SMALL_SIZE)
                .width(Length::FillPortion(3))
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);
            if e.overridden {
                r = r.push(
                    button(text("Remove").size(SMALL_SIZE))
                        .padding(8)
                        .on_press(Msg::EnvClear(e.name.clone()))
                        .style(quiet_button()),
                );
            }
            right = right.push(r);
        }
        right = right.push(
            row![
                text_input("NAME", &self.env_name)
                    .on_input(Msg::EnvName)
                    .padding(10)
                    .size(SMALL_SIZE),
                text_input("value", &self.env_value)
                    .on_input(Msg::EnvValue)
                    .on_submit(Msg::EnvAdd)
                    .padding(10)
                    .size(SMALL_SIZE),
                button(text("Set").size(SMALL_SIZE))
                    .padding(10)
                    .on_press(Msg::EnvAdd)
                    .style(accent_button()),
            ]
            .spacing(SPACING),
        );

        container(
            row![
                left,
                scrollable(right.padding(iced::Padding {
                    right: 16.0,
                    ..iced::Padding::ZERO
                }))
                .width(Fill)
            ]
            .spacing(SPACING * 2.0),
        )
        .padding(PAD)
        .width(Fill)
        .height(Fill)
        .into()
    }
}
