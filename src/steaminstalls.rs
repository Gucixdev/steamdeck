//! Settings > Steam installs: which Steam clients are on this machine (the Flatpak one and the system
//! package) and a guarded way to remove one of them. Only the program is removed; the games and
//! settings in the Steam folder stay on disk unless you delete that folder yourself.
use crate::log;
use crate::ui::{heading, item, segment, DIM, SMALL_SIZE, SPACING, TEXT_SIZE};
use iced::widget::{column, row, scrollable, text};
use iced::{Element, Fill, Task};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// com.valvesoftware.Steam; the string is the installation, "user" or "system"
    Flatpak(String),
    /// the distribution's own package (`steam`)
    Package,
    /// a Steam folder with no program behind it (left over from a removed install)
    DataOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    pub kind: Kind,
    pub label: String,
    /// the Steam folder (games, settings) that stays after removing the program
    pub data: Option<PathBuf>,
    pub games: usize,
    /// why it cannot be removed from here, if it cannot
    pub locked: Option<String>,
}

/// `flatpak list --columns=application,installation` -> the installation of Steam, if listed.
pub fn flatpak_installation(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let mut f = l.split('\t');
        let app = f.next()?.trim();
        let inst = f.next().unwrap_or("system").trim();
        (app == "com.valvesoftware.Steam").then(|| if inst.is_empty() { "system".into() } else { inst.to_string() })
    })
}

fn is_steamos() -> bool {
    std::fs::read_to_string("/etc/os-release").map(|t| t.lines().any(|l| l == "ID=steamos")).unwrap_or(false)
}

pub fn detect() -> Vec<Install> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut out = Vec::new();
    let fp = Command::new("flatpak")
        .args(["list", "--app", "--columns=application,installation"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| flatpak_installation(&String::from_utf8_lossy(&o.stdout)));
    if let Some(inst) = fp {
        let data = home.join(".var/app/com.valvesoftware.Steam/data/Steam");
        let games = crate::games::games(&data).len();
        out.push(Install { kind: Kind::Flatpak(inst), label: "Steam (Flatpak)".into(), data: Some(data), games, locked: None });
    }
    let pkg = Command::new("pacman").args(["-Qq", "steam"]).output().map(|o| o.status.success()).unwrap_or(false);
    let native = home.join(".local/share/Steam");
    if !pkg && (native.join("steamapps").is_dir() || native.join("config").is_dir()) {
        let games = crate::games::games(&native).len();
        out.push(Install {
            kind: Kind::DataOnly,
            label: "Steam folder without a program".into(),
            data: Some(native),
            games,
            locked: Some("no Steam program is installed for this folder; delete it yourself if you do not need its games".into()),
        });
    }
    if pkg {
        let data = home.join(".local/share/Steam");
        let games = crate::games::games(&data).len();
        let locked = is_steamos().then(|| "this is SteamOS: its Steam is part of the system and stays".to_string());
        out.push(Install { kind: Kind::Package, label: "Steam (system package)".into(), data: Some(data), games, locked });
    }
    out
}

#[derive(Debug, Clone)]
pub enum Msg {
    Found(Vec<Install>),
    Ask(usize),
    Cancel,
    Remove(usize),
    Done(String),
}

pub struct State {
    found: Option<Vec<Install>>,
    asking: Option<usize>,
    output: String,
}

fn scan() -> Task<Msg> {
    Task::perform(async { detect() }, Msg::Found)
}

impl State {
    pub fn new() -> (Self, Task<Msg>) {
        (State { found: None, asking: None, output: String::new() }, scan())
    }

    pub fn count(&self) -> usize {
        self.found.as_ref().map(Vec::len).unwrap_or(0)
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Found(v) => self.found = Some(v),
            Msg::Ask(i) => self.asking = Some(i),
            Msg::Cancel => self.asking = None,
            Msg::Remove(i) => {
                self.asking = None;
                let Some(inst) = self.found.as_ref().and_then(|f| f.get(i)).cloned() else { return Task::none() };
                if crate::games::steam_running() {
                    self.output = "Steam is running: close it completely first.".into();
                    return Task::none();
                }
                self.output = format!("removing {}...", inst.label);
                return Task::perform(async move { remove(&inst) }, Msg::Done);
            }
            Msg::Done(o) => {
                log::write("steam-remove", &o);
                self.output = o;
                return scan();
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let mut col = column![heading("Steam installs")].spacing(SPACING);
        match &self.found {
            None => col = col.push(text("Looking...").size(TEXT_SIZE).color(DIM)),
            Some(v) if v.is_empty() => col = col.push(text("No Flatpak or system Steam found.").size(TEXT_SIZE).color(DIM)),
            Some(v) => {
                if v.len() > 1 {
                    col = col.push(
                        text("Two Steam installs were found. Which one do you want to uninstall? The games and settings in its folder stay on disk.")
                            .size(TEXT_SIZE),
                    );
                }
                for (i, s) in v.iter().enumerate() {
                    let detail = format!(
                        "{} games in {}",
                        s.games,
                        s.data.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
                    );
                    let action: Element<Msg> = if let Some(why) = &s.locked {
                        text(why.clone()).size(SMALL_SIZE).color(DIM).into()
                    } else if self.asking == Some(i) {
                        row![
                            text("Uninstall the program?").size(SMALL_SIZE),
                            segment("Yes, uninstall", true, true, Msg::Remove(i)),
                            segment("Cancel", false, false, Msg::Cancel),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center)
                        .into()
                    } else {
                        segment("Uninstall", false, false, Msg::Ask(i))
                    };
                    col = col.push(item(s.label.clone(), detail, action));
                }
            }
        }
        if !self.output.is_empty() {
            col = col.push(text(self.output.clone()).size(SMALL_SIZE));
        }
        scrollable(col.padding(iced::Padding { right: 16.0, ..iced::Padding::ZERO })).height(Fill).into()
    }
}

/// Runs the removal; root through pkexec where needed. Returns what to show.
fn remove(i: &Install) -> String {
    let (prog, args): (&str, Vec<String>) = match &i.kind {
        Kind::Flatpak(inst) if inst == "user" => (
            "flatpak",
            vec!["uninstall".into(), "-y".into(), "--user".into(), "com.valvesoftware.Steam".into()],
        ),
        Kind::Flatpak(_) => (
            "pkexec",
            vec!["flatpak".into(), "uninstall".into(), "-y".into(), "--system".into(), "com.valvesoftware.Steam".into()],
        ),
        Kind::Package => ("pkexec", vec!["pacman".into(), "-R".into(), "--noconfirm".into(), "steam".into()]),
        Kind::DataOnly => return "nothing to uninstall: only a folder is left".into(),
    };
    match Command::new(prog).args(&args).output() {
        Ok(o) => {
            let mut t = format!("{prog} {}\n", args.join(" "));
            t.push_str(&String::from_utf8_lossy(&o.stdout));
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            t.push_str(if o.status.success() { "\ndone." } else { "\nfailed." });
            t
        }
        Err(e) => format!("could not start {prog}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flatpak_installation_is_read_from_the_list() {
        assert_eq!(flatpak_installation("org.a.B\tuser\ncom.valvesoftware.Steam\tsystem\n").as_deref(), Some("system"));
        assert_eq!(flatpak_installation("com.valvesoftware.Steam\tuser\n").as_deref(), Some("user"));
        assert_eq!(flatpak_installation("org.a.B\tuser\n"), None);
    }
}
