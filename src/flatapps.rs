//! Settings > Flatpak apps: what is installed and how much space the programs and their data take,
//! with uninstall (optionally with the data), delete-data-only and "remove unused runtimes". Uses the
//! `flatpak` command, so it does the same as `flatpak uninstall` would.
use crate::log;
use crate::ui::{danger_button, quiet_button};
use crate::ui::{heading, table_header, table_row, DIM, SMALL_SIZE, SPACING, TEXT_SIZE};
use iced::widget::{button, column, row, scrollable, text};
use iced::{Element, Fill, Task};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub size: String,
    pub installation: String,
    pub data_bytes: u64,
}

/// `flatpak list --columns=application,name,size,installation` (tab separated).
pub fn parse_list(out: &str) -> Vec<Item> {
    let mut v: Vec<Item> = out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 4 || f[0].trim().is_empty() {
                return None;
            }
            Some(Item {
                id: f[0].trim().to_string(),
                name: f[1].trim().to_string(),
                size: f[2].trim().to_string(),
                installation: f[3].trim().to_string(),
                data_bytes: 0,
            })
        })
        .collect();
    v.sort_by_key(|i| i.name.to_lowercase());
    v
}

fn data_dir(id: &str) -> PathBuf {
    data_dir_in(&dirs::home_dir().unwrap_or_default(), id)
}

fn data_dir_in(home: &std::path::Path, id: &str) -> PathBuf {
    home.join(".var/app").join(id)
}

fn dir_size(p: &std::path::Path) -> u64 {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            n += if md.is_dir() {
                dir_size(&e.path())
            } else {
                md.len()
            };
        }
    }
    n
}

fn valid_id(id: &str) -> bool {
    crate::flatpak::valid_id(id)
}

fn list() -> Vec<Item> {
    let out = Command::new("flatpak")
        .args([
            "list",
            "--app",
            "--columns=application,name,size,installation",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mut v = parse_list(&out);
    for i in &mut v {
        i.data_bytes = dir_size(&data_dir(&i.id));
    }
    v
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Uninstall(bool),
    DeleteData,
    Unused,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(Vec<Item>),
    Ask(usize, Action),
    Cancel,
    Do(usize, Action),
    Done(String),
}

pub struct State {
    items: Option<Vec<Item>>,
    asking: Option<(usize, Action)>,
    output: String,
    busy: bool,
}

fn load() -> Task<Msg> {
    Task::perform(async { list() }, Msg::Loaded)
}

impl State {
    pub fn new() -> (Self, Task<Msg>) {
        (
            State {
                items: None,
                asking: None,
                output: String::new(),
                busy: true,
            },
            load(),
        )
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Loaded(v) => {
                self.items = Some(v);
                self.busy = false;
            }
            Msg::Ask(i, a) => self.asking = Some((i, a)),
            Msg::Cancel => self.asking = None,
            Msg::Do(i, a) => {
                // only the action the user was just asked to confirm
                if self.asking.as_ref() != Some(&(i, a.clone())) {
                    return Task::none();
                }
                self.asking = None;
                let item = self.items.as_ref().and_then(|v| v.get(i)).cloned();
                self.busy = true;
                self.output = "working...".into();
                return Task::perform(async move { run(item, a) }, Msg::Done);
            }
            Msg::Done(o) => {
                log::write("flatapps", &o);
                self.output = o;
                return load();
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let mut col = column![
            heading("Flatpak apps"),
            row![
                text("Program size and the data each app keeps under ~/.var/app.")
                    .size(SMALL_SIZE)
                    .color(DIM)
                    .width(Fill),
                if self.asking == Some((usize::MAX, Action::Unused)) {
                    row![
                        text("Remove runtimes nothing uses?").size(SMALL_SIZE),
                        button(text("Yes").size(SMALL_SIZE))
                            .padding([6, 12])
                            .on_press(Msg::Do(usize::MAX, Action::Unused))
                            .style(danger_button()),
                        button(text("No").size(SMALL_SIZE))
                            .padding([6, 12])
                            .on_press(Msg::Cancel)
                            .style(quiet_button()),
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center)
                } else {
                    row![button(text("Remove unused runtimes").size(SMALL_SIZE))
                        .padding([6, 12])
                        .on_press(Msg::Ask(usize::MAX, Action::Unused))
                        .style(quiet_button())]
                },
            ]
            .align_y(iced::Alignment::Center),
        ]
        .spacing(SPACING);
        match &self.items {
            None => col = col.push(text("Looking...").size(TEXT_SIZE).color(DIM)),
            Some(v) if v.is_empty() => {
                col = col.push(
                    text("No Flatpak apps installed.")
                        .size(TEXT_SIZE)
                        .color(DIM),
                )
            }
            Some(v) => {
                col = col.push(table_header(&[
                    ("App", 4),
                    ("Program", 2),
                    ("Data", 2),
                    ("", 5),
                ]));
                for (n, it) in v.iter().enumerate() {
                    let actions: Element<Msg> = match &self.asking {
                        Some((i, a)) if *i == n => {
                            let q = match a {
                                Action::Uninstall(true) => "Uninstall and delete its data?",
                                Action::Uninstall(false) => "Uninstall (keep the data)?",
                                Action::DeleteData => "Delete its data?",
                                Action::Unused => "",
                            };
                            row![
                                text(q).size(SMALL_SIZE),
                                button(text("Yes").size(SMALL_SIZE))
                                    .padding([6, 12])
                                    .on_press(Msg::Do(n, a.clone()))
                                    .style(danger_button()),
                                button(text("No").size(SMALL_SIZE))
                                    .padding([6, 12])
                                    .on_press(Msg::Cancel)
                                    .style(quiet_button()),
                            ]
                            .spacing(6)
                            .align_y(iced::Alignment::Center)
                            .into()
                        }
                        _ => row![
                            button(text("Uninstall").size(SMALL_SIZE))
                                .padding([6, 10])
                                .on_press(Msg::Ask(n, Action::Uninstall(false)))
                                .style(quiet_button()),
                            button(text("+ data").size(SMALL_SIZE))
                                .padding([6, 10])
                                .on_press(Msg::Ask(n, Action::Uninstall(true)))
                                .style(quiet_button()),
                            button(text("Clear data").size(SMALL_SIZE))
                                .padding([6, 10])
                                .on_press(Msg::Ask(n, Action::DeleteData))
                                .style(quiet_button()),
                        ]
                        .spacing(6)
                        .into(),
                    };
                    col = col.push(table_row(
                        vec![
                            (text(it.name.clone()).size(TEXT_SIZE).into(), 4),
                            (text(it.size.clone()).size(SMALL_SIZE).color(DIM).into(), 2),
                            (
                                text(crate::ui::human_bytes(it.data_bytes))
                                    .size(SMALL_SIZE)
                                    .color(DIM)
                                    .into(),
                                2,
                            ),
                            (actions, 5),
                        ],
                        n % 2 == 1,
                    ));
                }
            }
        }
        if self.busy && self.items.is_some() {
            col = col.push(text("working...").size(TEXT_SIZE).color(DIM));
        }
        if !self.output.is_empty() {
            col = col.push(text(self.output.clone()).size(SMALL_SIZE));
        }
        scrollable(col.padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }
}

fn run(item: Option<Item>, a: Action) -> String {
    run_in(&dirs::home_dir().unwrap_or_default(), item, a)
}

fn run_in(home: &std::path::Path, item: Option<Item>, a: Action) -> String {
    let mut args: Vec<String> = Vec::new();
    let mut root = false;
    match (&a, &item) {
        (Action::Unused, _) => {
            args.extend(["uninstall".into(), "--unused".into(), "-y".into()]);
        }
        (Action::DeleteData, Some(it)) => {
            if !valid_id(&it.id) {
                return format!("odd app id '{}'", it.id);
            }
            let d = data_dir_in(home, &it.id);
            return match std::fs::remove_dir_all(&d) {
                Ok(()) => format!("deleted {}", d.display()),
                Err(e) => format!("could not delete {}: {e}", d.display()),
            };
        }
        (Action::Uninstall(with_data), Some(it)) => {
            if !valid_id(&it.id) {
                return format!("odd app id '{}'", it.id);
            }
            root = it.installation != "user";
            args.extend([
                "uninstall".into(),
                "-y".into(),
                format!("--{}", if root { "system" } else { "user" }),
            ]);
            if *with_data {
                args.push("--delete-data".into());
            }
            args.push(it.id.clone());
        }
        _ => return "nothing to do".into(),
    }
    let (prog, full): (&str, Vec<String>) = if root {
        let mut v = vec!["flatpak".to_string()];
        v.extend(args);
        ("pkexec", v)
    } else {
        ("flatpak", args)
    };
    match Command::new(prog).args(&full).output() {
        Ok(o) => {
            let mut t = format!("{prog} {}\n", full.join(" "));
            t.push_str(&String::from_utf8_lossy(&o.stdout));
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            t.push_str(if o.status.success() {
                "\ndone."
            } else {
                "\nfailed."
            });
            t
        }
        Err(e) => format!("could not start {prog}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn the_list_is_parsed_and_sorted_by_name() {
        let v = parse_list("org.b.B\tZed\t1.2 GB\tuser\norg.a.A\talpha\t30 MB\tsystem\n\n");
        assert_eq!(
            v.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["alpha", "Zed"]
        );
        assert_eq!(v[0].installation, "system");
    }

    #[test]
    fn odd_ids_are_not_acted_on() {
        let it = Item {
            id: "../x".into(),
            name: "x".into(),
            size: String::new(),
            installation: "user".into(),
            data_bytes: 0,
        };
        assert!(run(Some(it), Action::DeleteData).contains("odd app id"));
    }
    fn item(id: &str) -> Item {
        Item {
            id: id.into(),
            name: id.into(),
            size: String::new(),
            installation: "user".into(),
            data_bytes: 0,
        }
    }

    #[test]
    fn clearing_data_removes_only_that_apps_directory() {
        let home = std::env::temp_dir().join(format!("fa-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        for app in ["org.a.A", "org.b.B"] {
            fs::create_dir_all(home.join(".var/app").join(app).join("config")).unwrap();
            fs::write(home.join(".var/app").join(app).join("config/file"), app).unwrap();
        }
        fs::write(home.join("Documents"), "keep").unwrap();
        let msg = run_in(&home, Some(item("org.a.A")), Action::DeleteData);
        assert!(msg.starts_with("deleted"), "{msg}");
        assert!(!home.join(".var/app/org.a.A").exists());
        assert_eq!(
            fs::read_to_string(home.join(".var/app/org.b.B/config/file")).unwrap(),
            "org.b.B"
        );
        assert_eq!(fs::read_to_string(home.join("Documents")).unwrap(), "keep");
        // ids that could climb out are refused and nothing moves
        for bad in ["..", "../..", "org.a/../org.b.B", "", "a b", "x;y"] {
            let m = run_in(&home, Some(item(bad)), Action::DeleteData);
            assert!(m.contains("odd app id"), "'{bad}': {m}");
        }
        assert!(home.join(".var/app/org.b.B").is_dir());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn an_action_runs_only_when_it_was_asked_about() {
        let (mut s, _) = State::new();
        s.items = Some(vec![item("org.a.A")]);
        s.output.clear();
        let _ = s.update(Msg::Do(0, Action::DeleteData));
        assert!(s.output.is_empty(), "nothing was asked, nothing runs");
        let _ = s.update(Msg::Ask(0, Action::Uninstall(false)));
        let _ = s.update(Msg::Do(0, Action::DeleteData));
        assert!(
            s.output.is_empty(),
            "a different action than the one asked is ignored"
        );
    }
}
