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
use std::sync::{Arc, Mutex};

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
    /// `flatpak update` for everything
    UpdateAll,
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
    started: Option<std::time::Instant>,
    /// what the running operation is, for the line at the top of the page
    doing: String,
    /// operations seen so far, expected in all, and the one being worked on (written by the update thread)
    progress: Arc<Mutex<(usize, usize, String)>>,
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
                started: None,
                doing: String::new(),
                progress: Arc::new(Mutex::new((0, 0, String::new()))),
            },
            load(),
        )
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::UpdateAll => {
                self.doing = "Updating Flatpak apps".to_string();
                self.busy = true;
                self.started = Some(std::time::Instant::now());
                self.output = "updating Flatpak apps...".into();
                if let Ok(mut p) = self.progress.lock() {
                    *p = (0, 0, String::new());
                }
                let progress = self.progress.clone();
                return Task::perform(async move { update_all_streaming(progress) }, Msg::Done);
            }
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
                self.started = Some(std::time::Instant::now());
                let name = item.as_ref().map(|it| it.name.clone()).unwrap_or_default();
                self.doing = match &a {
                    Action::Uninstall(true) => format!("Uninstalling {name} and its data"),
                    Action::Uninstall(false) => format!("Uninstalling {name}"),
                    Action::DeleteData => format!("Deleting the data of {name}"),
                    Action::Unused => "Removing unused runtimes".to_string(),
                };
                self.output.clear();
                return Task::perform(async move { run(item, a) }, Msg::Done);
            }
            Msg::Done(o) => {
                self.started = None;
                self.doing.clear();
                log::write("flatapps", &o);
                self.output = o;
                return load();
            }
        }
        Task::none()
    }

    /// "2/5 org.gnome.Platform" while an update runs, empty before the first operation.
    pub fn progress_label(&self) -> String {
        match self.progress.lock() {
            Ok(p) if p.0 > 0 => format!("{}/{}  {}", p.0, p.1.max(p.0), p.2),
            _ => String::new(),
        }
    }

    /// True while an update runs (the window keeps its spinner turning).
    pub fn is_updating(&self) -> bool {
        self.started.is_some()
    }

    /// The last result only, for Home (nothing while an update runs: the button spins instead).
    pub fn report_view<'a, M: 'a>(&'a self) -> Element<'a, M> {
        let t = if self.started.is_some() {
            String::new()
        } else {
            self.output.clone()
        };
        text(t).size(SMALL_SIZE).into()
    }

    /// The last result, for Home; while an update runs, a spinner and the time it has taken.
    pub fn status_view<'a, M: 'a>(&'a self, tick: usize) -> Element<'a, M> {
        let t = match self.started {
            Some(t0) => format!(
                "{} updating Flatpak apps...  {}",
                crate::ui::spinner(tick),
                crate::ui::human_duration(t0.elapsed().as_secs())
            ),
            None => self.output.clone(),
        };
        text(t).size(SMALL_SIZE).into()
    }

    pub fn view(&self, tick: usize) -> Element<'_, Msg> {
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
        // what is running now, with a spinner, at the top where it is seen
        if let Some(t0) = self.started {
            let prog = self.progress_label();
            let mut line = format!(
                "{} {}...  {}",
                crate::ui::spinner(tick),
                self.doing,
                crate::ui::human_duration(t0.elapsed().as_secs())
            );
            if !prog.is_empty() && self.doing.starts_with("Updating") {
                line.push_str(&format!("   {prog}"));
            }
            col = col.push(text(line).size(TEXT_SIZE).color(crate::ui::ACCENT));
        }
        // the result goes at the top: at the bottom of a long list it was never seen
        if !self.output.is_empty() {
            col = col.push(text(self.output.clone()).size(SMALL_SIZE));
        }
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
        scrollable(col.padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }
}

/// `runtime/org.gnome.Platform/x86_64/51` -> `org.gnome.Platform`
fn pretty_ref(r: &str) -> String {
    let r = r.split_whitespace().next().unwrap_or(r);
    let mut parts = r.split('/');
    match (parts.next(), parts.next()) {
        (Some(_), Some(id)) if !id.is_empty() => id.to_string(),
        _ => r.to_string(),
    }
}

/// Run a shell command line, send every output line to the log while it runs, and return
/// (success, all the lines). Its stdin is closed, so nothing can sit waiting for an answer.
fn stream_cmd(
    cmd: &str,
    progress: &Arc<Mutex<(usize, usize, String)>>,
    changed: &mut Vec<String>,
) -> (bool, String) {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let child = Command::new("sh")
        .args(["-c", &format!("exec {cmd} 2>&1")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (false, format!("could not start `{cmd}`: {e}")),
    };
    let mut all = String::new();
    if let Some(out) = child.stdout.take() {
        // flatpak redraws progress with carriage returns: split on those as well
        for chunk in BufReader::new(out).split(b'\n') {
            let Ok(chunk) = chunk else { break };
            for part in String::from_utf8_lossy(&chunk).split('\r') {
                let t = part.trim();
                if !t.is_empty() {
                    log::write("flatpak", t);
                    // non-terminal output has no percentages, only one "Installing/Updating ref" line
                    // per operation: count those
                    if let Some(r) = t
                        .strip_prefix("Installing ")
                        .or_else(|| t.strip_prefix("Updating "))
                    {
                        let r = r.split_whitespace().next().unwrap_or(r).to_string();
                        if let Ok(mut p) = progress.lock() {
                            p.0 += 1;
                            if p.0 > p.1 {
                                p.1 = p.0;
                            }
                            p.2 = pretty_ref(&r);
                        }
                        if !changed.contains(&r) {
                            changed.push(r);
                        }
                    }
                    all.push_str(t);
                    all.push('\n');
                }
            }
        }
    }
    (child.wait().map(|s| s.success()).unwrap_or(false), all)
}

/// Update the Flatpak apps. The user installation first, with no questions. The system installation
/// only when it has updates, and through pkexec: a plain `flatpak update` on it waits for a password
/// that a window without a terminal can never give, and looks hung (it did).
fn update_all_streaming(progress: Arc<Mutex<(usize, usize, String)>>) -> String {
    let mut report = Vec::new();
    // how many things the user installation is going to update, for "2 of 5"
    let expected = Command::new("flatpak")
        .args(["remote-ls", "--user", "--updates", "--columns=ref"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count()
        })
        .unwrap_or(0);
    if let Ok(mut p) = progress.lock() {
        p.1 = expected;
    }
    let mut changed: Vec<String> = Vec::new();
    let (ok, out) = stream_cmd(
        "timeout 1800 flatpak update --user -y --noninteractive",
        &progress,
        &mut changed,
    );
    report.push(format!(
        "your apps: {}",
        if ok { "up to date" } else { "update failed" }
    ));
    let tail = |s: &str| {
        s.lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    };
    if !ok {
        report.push(tail(&out));
    }
    let pending = Command::new("flatpak")
        .args([
            "remote-ls",
            "--system",
            "--updates",
            "--columns=application",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count()
        })
        .unwrap_or(0);
    if pending > 0 {
        let (ok2, out2) = stream_cmd(
            "timeout 1800 pkexec flatpak update --system -y --noninteractive",
            &progress,
            &mut changed,
        );
        report.push(format!(
            "system apps ({pending} update(s)): {}",
            if ok2 {
                "up to date"
            } else {
                "update failed (needs your password)"
            }
        ));
        if !ok2 {
            report.push(tail(&out2));
        }
    }
    // which ones changed: apps first, runtimes (and their locale/extension parts) after
    let apps: Vec<String> = changed
        .iter()
        .filter(|r| r.starts_with("app/"))
        .map(|r| pretty_ref(r))
        .collect();
    let runtimes: Vec<String> = changed
        .iter()
        .filter(|r| r.starts_with("runtime/"))
        .map(|r| pretty_ref(r))
        .collect();
    if apps.is_empty() && runtimes.is_empty() {
        report.push("nothing needed updating".to_string());
    }
    if !apps.is_empty() {
        report.push(format!(
            "updated apps ({}): {}",
            apps.len(),
            apps.join(", ")
        ));
    }
    if !runtimes.is_empty() {
        let mut u = runtimes.clone();
        u.sort();
        u.dedup();
        report.push(format!("runtimes ({}): {}", u.len(), u.join(", ")));
    }
    report.join("\n")
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
    fn a_confirmed_action_starts() {
        let (mut s, _) = State::new();
        s.items = Some(vec![item("org.a.A")]);
        for a in [
            Action::Uninstall(false),
            Action::Uninstall(true),
            Action::DeleteData,
        ] {
            s.started = None;
            let _ = s.update(Msg::Ask(0, a.clone()));
            let _ = s.update(Msg::Do(0, a.clone()));
            assert!(
                s.started.is_some(),
                "{a:?} must start after its question was confirmed"
            );
            assert!(
                s.doing.contains("org.a.A"),
                "the page says what it is doing: {}",
                s.doing
            );
        }
        s.started = None;
        let _ = s.update(Msg::Ask(usize::MAX, Action::Unused));
        let _ = s.update(Msg::Do(usize::MAX, Action::Unused));
        assert!(s.started.is_some());
    }

    #[test]
    fn an_action_runs_only_when_it_was_asked_about() {
        let (mut s, _) = State::new();
        s.items = Some(vec![item("org.a.A")]);
        s.output.clear();
        let _ = s.update(Msg::Do(0, Action::DeleteData));
        assert!(s.started.is_none(), "nothing was asked, nothing runs");
        let _ = s.update(Msg::Ask(0, Action::Uninstall(false)));
        let _ = s.update(Msg::Do(0, Action::DeleteData));
        assert!(
            s.started.is_none(),
            "a different action than the one asked is ignored"
        );
    }
    #[test]
    fn refs_are_shown_by_their_id() {
        assert_eq!(
            pretty_ref("runtime/org.gnome.Platform/x86_64/51"),
            "org.gnome.Platform"
        );
        assert_eq!(
            pretty_ref("app/org.gimp.GIMP/x86_64/stable"),
            "org.gimp.GIMP"
        );
        assert_eq!(pretty_ref("weird"), "weird");
    }
    #[test]
    fn the_changed_refs_are_collected_from_the_output_lines() {
        let progress = Arc::new(Mutex::new((0usize, 3usize, String::new())));
        let mut changed = Vec::new();
        let script = "printf 'Info: x is end-of-life\\nInstalling runtime/org.gnome.Platform/x86_64/51\\nUpdating app/org.gimp.GIMP/x86_64/stable\\nUpdating app/org.gimp.GIMP/x86_64/stable\\n'";
        let (ok, out) = stream_cmd(script, &progress, &mut changed);
        assert!(ok);
        assert!(out.contains("Info: x is end-of-life"));
        assert_eq!(
            changed,
            [
                "runtime/org.gnome.Platform/x86_64/51",
                "app/org.gimp.GIMP/x86_64/stable"
            ]
        );
        let p = progress.lock().unwrap();
        assert_eq!(p.0, 3, "every Installing/Updating line is one step");
        assert_eq!(p.2, "org.gimp.GIMP");
    }
}
