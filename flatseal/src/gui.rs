//! The window: apps on the left, the permissions of the selected one on the right. Every change is
//! written to the override file at once, like the GTK original does.
use deck_common::{accent_button, danger_button, quiet_button, theme, AMBER, DIM, GREEN, PAD, PURPLE, RED, SMALL_SIZE, SPACING, TEXT_SIZE, TITLE_SIZE};
use flatseal::flatpak::{self, App as FpApp, GLOBAL};
use flatseal::perms::{self, Layers, SECTIONS};
use iced::widget::{button, column, container, pick_list, row, scrollable, space, text, text_input, toggler, Column};
use iced::{Element, Fill, Length};

pub fn run() -> iced::Result {
    iced::application(App::new, App::update, App::view).title("Flatseal").theme(|_: &App| theme()).window_size((1100.0, 700.0)).run()
}

#[derive(Debug, Clone)]
enum Msg {
    Select(String),
    Search(String),
    Toggle(&'static str, &'static str, bool),
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
}

impl App {
    fn new() -> Self {
        let (apps, error) = match flatpak::list_apps() {
            Ok(a) => (a, None),
            Err(e) => (vec![], Some(e)),
        };
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

    fn update(&mut self, m: Msg) {
        self.saved = false;
        match m {
            Msg::Select(id) => {
                self.selected = id;
                self.layers = flatpak::load_layers(&self.selected);
                self.confirm_reset = false;
            }
            Msg::Search(s) => self.search = s,
            Msg::Toggle(key, name, on) => {
                self.layers.set(key, name, on);
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
            }
        }
    }

    fn title(&self) -> String {
        if self.selected == GLOBAL {
            return "All apps (global overrides)".into();
        }
        self.apps.iter().find(|a| a.id == self.selected).map(|a| a.name.clone()).unwrap_or_else(|| self.selected.clone())
    }

    fn view(&self) -> Element<'_, Msg> {
        // left: the app list
        let q = self.search.to_lowercase();
        let mut items: Vec<Element<Msg>> = Vec::new();
        let mut entry = |id: String, label: String| {
            let b = button(text(label).size(SMALL_SIZE)).padding(10).width(Fill).on_press(Msg::Select(id.clone()));
            items.push(if id == self.selected { b.style(accent_button()).into() } else { b.style(quiet_button()).into() });
        };
        entry(GLOBAL.to_string(), "All apps (global)".to_string());
        for a in &self.apps {
            if q.is_empty() || a.name.to_lowercase().contains(&q) || a.id.to_lowercase().contains(&q) {
                entry(a.id.clone(), a.name.clone());
            }
        }
        let list = Column::with_children(items).spacing(4);
        let left = column![
            text("Flatseal").size(TITLE_SIZE).color(GREEN),
            text_input("Search", &self.search).on_input(Msg::Search).padding(10).size(SMALL_SIZE),
            scrollable(list).height(Fill),
        ]
        .spacing(SPACING)
        .width(Length::Fixed(300.0));

        // right: the permissions
        let mut right = Column::new().spacing(SPACING);
        let mut head = row![text(self.title()).size(TITLE_SIZE)].spacing(SPACING).align_y(iced::Alignment::Center);
        if self.selected != GLOBAL {
            head = head.push(text(self.selected.clone()).size(SMALL_SIZE).color(DIM));
        }
        head = head.push(space::horizontal());
        if self.saved {
            head = head.push(text("saved").size(SMALL_SIZE).color(GREEN));
        }
        if self.confirm_reset {
            head = head
                .push(text("Drop all overrides?").size(SMALL_SIZE).color(AMBER))
                .push(button(text("Yes").size(SMALL_SIZE)).padding(10).on_press(Msg::Reset).style(danger_button()))
                .push(button(text("No").size(SMALL_SIZE)).padding(10).on_press(Msg::CancelReset).style(quiet_button()));
        } else {
            head = head.push(button(text("Reset").size(SMALL_SIZE)).padding(10).on_press(Msg::AskReset).style(quiet_button()));
        }
        right = right.push(head);
        if let Some(e) = &self.error {
            right = right.push(text(e.clone()).size(SMALL_SIZE).color(RED));
        }

        for s in SECTIONS {
            right = right.push(text(s.title).size(TEXT_SIZE).color(PURPLE));
            for (name, desc) in s.items {
                let on = self.layers.is_on(s.key, name);
                let over = self.layers.is_overridden(s.key, name);
                let key = s.key;
                let nm: &'static str = name;
                right = right.push(
                    row![
                        toggler(on).on_toggle(move |v| Msg::Toggle(key, nm, v)).size(26),
                        text(name.to_string()).size(SMALL_SIZE),
                        text(*desc).size(SMALL_SIZE).color(DIM),
                        space::horizontal(),
                        text(if over { "overridden" } else { "" }).size(SMALL_SIZE).color(AMBER),
                    ]
                    .spacing(SPACING)
                    .align_y(iced::Alignment::Center),
                );
            }
            if s.key == "filesystems" {
                for e in self.layers.custom_filesystems() {
                    right = right.push(
                        row![
                            text(e.name.clone()).size(SMALL_SIZE),
                            space::horizontal(),
                            text(if e.overridden { "overridden" } else { "" }).size(SMALL_SIZE).color(AMBER),
                            button(text("Remove").size(SMALL_SIZE)).padding(8).on_press(Msg::FsRemove(e.name.clone())).style(quiet_button()),
                        ]
                        .spacing(SPACING)
                        .align_y(iced::Alignment::Center),
                    );
                }
                right = right.push(
                    row![
                        text_input("~/Games  or  /mnt/sdcard:ro", &self.fs_input).on_input(Msg::FsInput).on_submit(Msg::FsAdd).padding(10).size(SMALL_SIZE),
                        button(text("Add path").size(SMALL_SIZE)).padding(10).on_press(Msg::FsAdd).style(accent_button()),
                    ]
                    .spacing(SPACING),
                );
            }
        }

        for (title, group) in [("Session bus", perms::SESSION_BUS), ("System bus", perms::SYSTEM_BUS)] {
            right = right.push(text(title).size(TEXT_SIZE).color(PURPLE));
            for e in self.layers.bus(group) {
                let mut r = row![text(e.name.clone()).size(SMALL_SIZE), space::horizontal()].spacing(8).align_y(iced::Alignment::Center);
                for p in POLICIES {
                    let b = button(text(p).size(SMALL_SIZE)).padding(8).on_press(Msg::BusSet(group, e.name.clone(), p));
                    r = r.push(if e.value == p { b.style(accent_button()) } else { b.style(quiet_button()) });
                }
                if e.overridden {
                    r = r.push(button(text("Reset").size(SMALL_SIZE)).padding(8).on_press(Msg::BusClear(group, e.name.clone())).style(quiet_button()));
                }
                right = right.push(r);
            }
            right = right.push(
                row![
                    text_input("org.example.Service", &self.bus_name).on_input(Msg::BusName).on_submit(Msg::BusAdd(group)).padding(10).size(SMALL_SIZE),
                    pick_list(POLICIES, Some(self.bus_policy), |p| Msg::BusPolicy(POLICIES.iter().find(|x| **x == p).copied().unwrap_or("talk"))),
                    button(text("Add").size(SMALL_SIZE)).padding(10).on_press(Msg::BusAdd(group)).style(accent_button()),
                ]
                .spacing(SPACING)
                .align_y(iced::Alignment::Center),
            );
        }

        right = right.push(text("Environment").size(TEXT_SIZE).color(PURPLE));
        for e in self.layers.env() {
            let mut r = row![text(format!("{}={}", e.name, e.value)).size(SMALL_SIZE), space::horizontal()].spacing(8).align_y(iced::Alignment::Center);
            if e.overridden {
                r = r.push(button(text("Remove").size(SMALL_SIZE)).padding(8).on_press(Msg::EnvClear(e.name.clone())).style(quiet_button()));
            }
            right = right.push(r);
        }
        right = right.push(
            row![
                text_input("NAME", &self.env_name).on_input(Msg::EnvName).padding(10).size(SMALL_SIZE),
                text_input("value", &self.env_value).on_input(Msg::EnvValue).on_submit(Msg::EnvAdd).padding(10).size(SMALL_SIZE),
                button(text("Set").size(SMALL_SIZE)).padding(10).on_press(Msg::EnvAdd).style(accent_button()),
            ]
            .spacing(SPACING),
        );

        container(row![left, scrollable(right.padding(iced::Padding { right: 16.0, ..iced::Padding::ZERO })).width(Fill)].spacing(SPACING * 2.0))
            .padding(PAD)
            .width(Fill)
            .height(Fill)
            .into()
    }
}
