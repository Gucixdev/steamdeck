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
    /// the distribution's own package: package manager and package name
    Package(String, String),
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
        (app == "com.valvesoftware.Steam").then(|| {
            if inst.is_empty() {
                "system".into()
            } else {
                inst.to_string()
            }
        })
    })
}

fn is_steamos() -> bool {
    std::fs::read_to_string("/etc/os-release")
        .map(|t| t.lines().any(|l| l == "ID=steamos"))
        .unwrap_or(false)
}

fn ok(prog: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(prog).args(args).output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// The package manager and name of the system Steam, whichever distribution this is: first the
/// package that owns the `steam` command, then a package literally called steam.
pub fn detect_package() -> Option<(String, String)> {
    let bin =
        ok("sh", &["-c", "command -v steam"]).filter(|p| !p.is_empty() && !p.contains("flatpak"));
    let real = bin
        .as_deref()
        .and_then(|b| std::fs::canonicalize(b).ok())
        .map(|p| p.display().to_string());
    if let Some(path) = real.as_deref() {
        let owners: [(&str, &str, Vec<&str>); 5] = [
            ("pacman", "pacman", vec!["-Qoq", path]),
            ("apt", "dpkg-query", vec!["-S", path]),
            ("dnf", "rpm", vec!["-qf", "--qf", "%{NAME}", path]),
            ("xbps", "xbps-query", vec!["-o", path]),
            ("apk", "apk", vec!["info", "-W", path]),
        ];
        for (mgr, prog, args) in owners {
            if let Some(out) = ok(prog, &args) {
                let first = out
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split([':', ' '])
                    .next()
                    .unwrap_or("")
                    .trim();
                let first = first.rsplit('/').next().unwrap_or(first);
                let pkg = match mgr {
                    "xbps" | "apk" => first.split(['-']).next().unwrap_or(first),
                    _ => first,
                };
                if !pkg.is_empty() {
                    return Some((mgr.to_string(), pkg.to_string()));
                }
            }
        }
    }
    let named: [(&str, &str, Vec<&str>); 3] = [
        ("pacman", "pacman", vec!["-Qq", "steam"]),
        ("apt", "dpkg-query", vec!["-W", "steam", "steam-launcher"]),
        ("dnf", "rpm", vec!["-q", "steam"]),
    ];
    for (mgr, prog, args) in named {
        if ok(prog, &args).map(|o| !o.is_empty()).unwrap_or(false) {
            let pkg = if mgr == "apt" && ok("dpkg-query", &["-W", "steam-launcher"]).is_some() {
                "steam-launcher"
            } else {
                "steam"
            };
            return Some((mgr.to_string(), pkg.to_string()));
        }
    }
    None
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
        out.push(Install {
            kind: Kind::Flatpak(inst),
            label: "Steam (Flatpak)".into(),
            data: Some(data),
            games,
            locked: None,
        });
    }
    let pkg = detect_package();
    let native = home.join(".local/share/Steam");
    if pkg.is_none() && (native.join("steamapps").is_dir() || native.join("config").is_dir()) {
        let games = crate::games::games(&native).len();
        out.push(Install {
            kind: Kind::DataOnly,
            label: "Steam folder without a program".into(),
            data: Some(native),
            games,
            locked: None,
        });
    }
    if let Some((mgr, name)) = pkg {
        let data = home.join(".local/share/Steam");
        let games = crate::games::games(&data).len();
        let locked = is_steamos()
            .then(|| "this is SteamOS: its Steam is part of the system and stays".to_string());
        out.push(Install {
            kind: Kind::Package(mgr.clone(), name.clone()),
            label: format!("Steam (system package: {name} via {mgr})"),
            data: Some(data),
            games,
            locked,
        });
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
        (
            State {
                found: None,
                asking: None,
                output: String::new(),
            },
            scan(),
        )
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
                let Some(inst) = self.found.as_ref().and_then(|f| f.get(i)).cloned() else {
                    return Task::none();
                };
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
            Some(v) if v.is_empty() => {
                col = col.push(
                    text("No Flatpak or system Steam found.")
                        .size(TEXT_SIZE)
                        .color(DIM),
                )
            }
            Some(v) => {
                if v.len() > 1 {
                    col = col.push(
                        text("More than one Steam was found. Which one do you want to remove? Uninstalling a program keeps its games and settings folder; deleting a folder removes its games.")
                            .size(TEXT_SIZE),
                    );
                }
                for (i, s) in v.iter().enumerate() {
                    let detail = format!(
                        "{} games in {}",
                        s.games,
                        s.data
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    );
                    let action: Element<Msg> = if let Some(why) = &s.locked {
                        text(why.clone()).size(SMALL_SIZE).color(DIM).into()
                    } else if self.asking == Some(i) {
                        row![
                            text(if s.kind == Kind::DataOnly {
                                "Delete this folder for good?"
                            } else {
                                "Uninstall the program?"
                            })
                            .size(SMALL_SIZE),
                            segment(
                                if s.kind == Kind::DataOnly {
                                    "Yes, delete the folder and its games"
                                } else {
                                    "Yes, uninstall"
                                },
                                true,
                                true,
                                Msg::Remove(i)
                            ),
                            segment("Cancel", false, false, Msg::Cancel),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center)
                        .into()
                    } else {
                        segment(
                            if s.kind == Kind::DataOnly {
                                "Delete folder"
                            } else {
                                "Uninstall"
                            },
                            false,
                            false,
                            Msg::Ask(i),
                        )
                    };
                    col = col.push(item(s.label.clone(), detail, action));
                }
            }
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

/// Delete a Steam folder that has no program behind it. Refuses anything that does not look like one.
fn delete_folder(path: &std::path::Path) -> String {
    let real = match std::fs::canonicalize(path) {
        Ok(r) => r,
        Err(e) => return format!("{}: {e}", path.display()),
    };
    let home = dirs::home_dir().unwrap_or_default();
    let name_ok = real.file_name().map(|n| n == "Steam").unwrap_or(false);
    let has_steam_files = real.join("steamapps").is_dir()
        || real.join("config").is_dir()
        || real.join("steam.sh").is_file();
    if !name_ok || !has_steam_files || real == home || real.parent().is_none() {
        return format!(
            "refusing to delete {}: it does not look like a Steam folder",
            real.display()
        );
    }
    if let Err(e) = std::fs::remove_dir_all(&real) {
        return format!("could not delete {}: {e}", real.display());
    }
    // the usual ~/.local/share/Steam is a symlink to the real folder: drop it if it now dangles
    if path != real
        && std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        && !path.exists()
    {
        let _ = std::fs::remove_file(path);
    }
    format!("deleted {}", real.display())
}

/// Runs the removal; root through pkexec where needed. Returns what to show.
fn remove(i: &Install) -> String {
    let (prog, args): (&str, Vec<String>) = match &i.kind {
        Kind::Flatpak(inst) if inst == "user" => (
            "flatpak",
            vec![
                "uninstall".into(),
                "-y".into(),
                "--user".into(),
                "com.valvesoftware.Steam".into(),
            ],
        ),
        Kind::Flatpak(_) => (
            "pkexec",
            vec![
                "flatpak".into(),
                "uninstall".into(),
                "-y".into(),
                "--system".into(),
                "com.valvesoftware.Steam".into(),
            ],
        ),
        Kind::Package(mgr, pkg) => {
            let a: Vec<&str> = match mgr.as_str() {
                "pacman" => vec!["pacman", "-R", "--noconfirm", pkg],
                "apt" => vec!["apt-get", "remove", "-y", pkg],
                "dnf" => vec!["dnf", "remove", "-y", pkg],
                "xbps" => vec!["xbps-remove", "-y", pkg],
                "apk" => vec!["apk", "del", pkg],
                other => return format!("do not know how to remove a package with {other}"),
            };
            ("pkexec", a.into_iter().map(String::from).collect())
        }
        Kind::DataOnly => {
            return i
                .data
                .as_ref()
                .map(|p| delete_folder(p))
                .unwrap_or_default()
        }
    };
    match Command::new(prog).args(&args).output() {
        Ok(o) => {
            let mut t = format!("{prog} {}\n", args.join(" "));
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

    #[test]
    fn a_leftover_steam_folder_is_deleted_with_its_symlink_and_other_folders_are_refused() {
        let base = std::env::temp_dir().join(format!("sdel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let real = base.join("data/Steam");
        std::fs::create_dir_all(real.join("steamapps")).unwrap();
        let link = base.join("share-Steam");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let other = base.join("Documents");
        std::fs::create_dir_all(&other).unwrap();
        assert!(delete_folder(&other).contains("refusing"));
        assert!(other.exists());
        assert!(delete_folder(&link).starts_with("deleted"));
        assert!(!real.exists());
        assert!(
            std::fs::symlink_metadata(&link).is_err(),
            "the dangling link is removed too"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_flatpak_installation_is_read_from_the_list() {
        assert_eq!(
            flatpak_installation("org.a.B\tuser\ncom.valvesoftware.Steam\tsystem\n").as_deref(),
            Some("system")
        );
        assert_eq!(
            flatpak_installation("com.valvesoftware.Steam\tuser\n").as_deref(),
            Some("user")
        );
        assert_eq!(flatpak_installation("org.a.B\tuser\n"), None);
    }
}
