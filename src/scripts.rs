//! Settings > Scripts: run small scripts from one place and see their output. Your own scripts are
//! the executable files in `~/.config/steamdeck/scripts/`; a few read-only ones come with the program.
//! Nothing runs without a confirmation, scripts run as you (never as root), and their output goes
//! to the log as well.
use crate::log;
use crate::ui::{danger_button, primary_button, quiet_button};
use crate::ui::{heading, item, DIM, SMALL_SIZE, SPACING, TEXT_SIZE};
use iced::widget::{button, column, row, scrollable, text};
use iced::{Element, Fill, Task};
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub name: String,
    pub about: String,
    /// either a file to execute or a built-in shell snippet
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    File(PathBuf),
    Builtin(&'static str),
}

pub fn scripts_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("steamdeck/scripts")
}

const BUILTINS: [(&str, &str, &str); 3] = [
    (
        "Disk usage of Steam and Flatpak",
        "How much space the games, prefixes, shader caches and Flatpak data take (read only).",
        "for d in ~/.local/share/Steam ~/.var/app/com.valvesoftware.Steam/data/Steam ~/.var/app; do [ -d \"$d\" ] && du -sh \"$d\"/* 2>/dev/null | sort -h | tail -12 && echo; done; df -h / /home 2>/dev/null",
    ),
    (
        "System information",
        "Kernel, memory, swap, GPU and the Steam Deck model if there is one (read only).",
        "uname -srm; echo; free -h; echo; swapon --show 2>/dev/null; echo; lspci 2>/dev/null | grep -i -E 'vga|3d' ; cat /sys/devices/virtual/dmi/id/product_name 2>/dev/null",
    ),
    (
        "Failed services",
        "systemd units that are in a failed state (read only).",
        "systemctl --failed --no-pager 2>&1; echo; systemctl --user --failed --no-pager 2>&1",
    ),
];

/// Built-ins first, then every executable regular file in the scripts directory, sorted by name.
pub fn list(dir: &std::path::Path) -> Vec<Script> {
    use std::os::unix::fs::PermissionsExt;
    let mut v: Vec<Script> = BUILTINS
        .iter()
        .map(|(n, a, c)| Script {
            name: n.to_string(),
            about: a.to_string(),
            source: Source::Builtin(c),
        })
        .collect();
    let mut mine: Vec<Script> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file() && !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| {
            e.metadata()
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            // the comment line after the shebang, if there is one, is the description
            let about = std::fs::read_to_string(e.path())
                .ok()
                .and_then(|t| {
                    t.lines()
                        .skip(1)
                        .find(|l| l.starts_with('#'))
                        .map(|l| l.trim_start_matches('#').trim().to_string())
                })
                .unwrap_or_else(|| "your script".to_string());
            Script {
                name,
                about,
                source: Source::File(e.path()),
            }
        })
        .collect();
    mine.sort_by(|a, b| a.name.cmp(&b.name));
    v.extend(mine);
    v
}

#[derive(Debug, Clone)]
pub enum Msg {
    Ask(usize),
    Cancel,
    Run(usize),
    Done(String, String),
}

pub struct State {
    scripts: Vec<Script>,
    asking: Option<usize>,
    running: Option<String>,
    title: String,
    output: String,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        State {
            scripts: list(&scripts_dir()),
            asking: None,
            running: None,
            title: String::new(),
            output: String::new(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Ask(i) => self.asking = Some(i),
            Msg::Cancel => self.asking = None,
            Msg::Run(i) => {
                // only the one that was asked about, and never two at once
                if self.asking != Some(i) || self.running.is_some() {
                    return Task::none();
                }
                self.asking = None;
                let Some(sc) = self.scripts.get(i).cloned() else {
                    return Task::none();
                };
                self.running = Some(sc.name.clone());
                self.output.clear();
                return Task::perform(crate::ui::blocking(move || run(&sc)), |(n, o)| {
                    Msg::Done(n, o)
                });
            }
            Msg::Done(name, out) => {
                log::write("script", &format!("{name}\n{out}"));
                self.title = name;
                self.output = out;
                self.running = None;
                self.scripts = list(&scripts_dir());
            }
        }
        Task::none()
    }

    pub fn view(&self, tick: usize) -> Element<'_, Msg> {
        let mut col = column![
            heading("Scripts"),
            text(format!(
                "Your own: executable files in {}. They run as you.",
                scripts_dir().display()
            ))
            .size(SMALL_SIZE)
            .color(DIM),
        ]
        .spacing(SPACING);
        for (i, s) in self.scripts.iter().enumerate() {
            let action: Element<Msg> = if self.running.as_deref() == Some(s.name.as_str()) {
                text(format!("{}  running", crate::ui::spinner(tick)))
                    .size(SMALL_SIZE)
                    .into()
            } else if self.asking == Some(i) {
                row![
                    text("Run it?").size(SMALL_SIZE),
                    button(text("Yes").size(SMALL_SIZE))
                        .padding([6, 12])
                        .on_press(Msg::Run(i))
                        .style(danger_button()),
                    button(text("No").size(SMALL_SIZE))
                        .padding([6, 12])
                        .on_press(Msg::Cancel)
                        .style(quiet_button()),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center)
                .into()
            } else {
                button(text("Run").size(SMALL_SIZE))
                    .padding([6, 14])
                    .on_press(Msg::Ask(i))
                    .style(primary_button())
                    .into()
            };
            col = col.push(item(s.name.clone(), s.about.clone(), action));
        }
        if !self.output.is_empty() {
            col = col.push(
                text(format!("Output of {}", self.title))
                    .size(TEXT_SIZE)
                    .color(crate::ui::ACCENT),
            );
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

fn run(sc: &Script) -> (String, String) {
    let mut cmd = match &sc.source {
        Source::Builtin(c) => {
            let mut x = Command::new("sh");
            x.args(["-c", c]);
            x
        }
        Source::File(p) => Command::new(p),
    };
    let out = cmd.stdin(Stdio::null()).output();
    let text = match out {
        Ok(o) => {
            let mut t = String::from_utf8_lossy(&o.stdout).to_string();
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            if !o.status.success() {
                t.push_str(&format!("\n(exit status {})", o.status));
            }
            // a runaway script must not fill the window
            if t.len() > 20_000 {
                t.truncate(20_000);
                t.push_str("\n... (cut)");
            }
            t
        }
        Err(e) => format!("could not start it: {e}"),
    };
    (sc.name.clone(), text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn built_ins_come_first_then_your_executable_files() {
        let d = std::env::temp_dir().join(format!("scr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mk = |n: &str, body: &str, mode: u32| {
            std::fs::write(d.join(n), body).unwrap();
            std::fs::set_permissions(d.join(n), std::fs::Permissions::from_mode(mode)).unwrap();
        };
        mk(
            "b-clean.sh",
            "#!/bin/sh\n# clears my caches\necho x\n",
            0o755,
        );
        mk("a-first", "#!/bin/sh\necho y\n", 0o700);
        mk("notes.txt", "not a script", 0o644);
        mk(".hidden", "#!/bin/sh\n", 0o755);
        let v = list(&d);
        let names: Vec<&str> = v.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names[..3],
            [
                "Disk usage of Steam and Flatpak",
                "System information",
                "Failed services"
            ]
        );
        assert_eq!(names[3..], ["a-first", "b-clean.sh"]);
        assert_eq!(v[4].about, "clears my caches");
    }

    #[test]
    fn a_script_runs_and_its_output_comes_back() {
        let d = std::env::temp_dir().join(format!("scr2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("hi"),
            "#!/bin/sh\necho hello; echo oops >&2; exit 3\n",
        )
        .unwrap();
        std::fs::set_permissions(d.join("hi"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let sc = Script {
            name: "hi".into(),
            about: String::new(),
            source: Source::File(d.join("hi")),
        };
        let (n, out) = run(&sc);
        assert_eq!(n, "hi");
        assert!(out.contains("hello") && out.contains("oops") && out.contains("exit status"));
    }

    #[test]
    fn nothing_runs_without_being_asked_first() {
        let mut s = State::new();
        s.scripts = vec![Script {
            name: "x".into(),
            about: String::new(),
            source: Source::Builtin("true"),
        }];
        let _ = s.update(Msg::Run(0));
        assert!(!s.is_running());
        let _ = s.update(Msg::Ask(0));
        let _ = s.update(Msg::Run(0));
        assert!(s.is_running());
        let _ = s.update(Msg::Ask(0));
        let _ = s.update(Msg::Run(0));
        assert!(s.is_running(), "a second one is ignored while one runs");
    }
}
