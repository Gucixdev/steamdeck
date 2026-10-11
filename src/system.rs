//! The System section: the same preparation the `setup` script does (OOM guard, memory settings, swap
//! file, ssh), run from the window with the output shown and logged. The script is embedded in the
//! program, so one binary is enough on the Deck; root comes from pkexec (or sudo in a terminal).
use crate::log;
use crate::ui::{heading, item, segment, DIM, SMALL_SIZE, SPACING, TEXT_SIZE};
use iced::widget::{column, row, scrollable, text, text_input};
use iced::{Element, Fill, Task};
use std::process::Command;

const SETUP: &str = include_str!("../setup");

#[derive(Debug, Clone)]
pub enum Msg {
    Run(Vec<String>),
    Done(String, String),
    Swappiness(String),
    SwapSize(String),
}

pub struct State {
    output: String,
    busy: bool,
    swappiness: String,
    swap: String,
}

impl State {
    pub fn new() -> (Self, Task<Msg>) {
        let s = State {
            output: String::new(),
            busy: true,
            swappiness: "100".into(),
            swap: "8G".into(),
        };
        (
            s,
            Task::perform(
                crate::ui::blocking(move || run_actions(vec!["status".into()])),
                |(a, o)| Msg::Done(a, o),
            ),
        )
    }

    pub fn last_error(&self) -> Option<&str> {
        None
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Swappiness(s) => self.swappiness = s,
            Msg::SwapSize(s) => self.swap = s,
            Msg::Run(a) => {
                self.busy = true;
                self.output = format!("running: {}\n", a.join(" "));
                return Task::perform(crate::ui::blocking(move || run_actions(a)), |(a, o)| {
                    Msg::Done(a, o)
                });
            }
            Msg::Done(a, o) => {
                self.busy = false;
                log::write("system", &format!("{a}\n{o}"));
                self.output = o;
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let go =
            |label: &'static str, args: Vec<String>| segment(label, false, false, Msg::Run(args));
        let ok_swappiness = self
            .swappiness
            .parse::<u32>()
            .map(|n| n <= 200)
            .unwrap_or(false);
        let ok_swap = {
            let t = self.swap.trim_end_matches(['G', 'g', 'M', 'm']);
            !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) && self.swap.len() > t.len()
        };
        let mut col = column![
            heading("System"),
            text("Changes only /etc, so they survive SteamOS updates. Asks for your password.")
                .size(SMALL_SIZE)
                .color(DIM),
            item(
                "OOM guard",
                "kill the game instead of freezing the Deck when memory runs out",
                row![
                    go("set up", vec!["oom".into()]),
                    go("remove", vec!["undo".into()])
                ]
                .spacing(8)
                .into()
            ),
            item(
                "Memory settings",
                "swappiness, cache pressure and page-cluster tuned for zram",
                row![
                    text_input("100", &self.swappiness)
                        .on_input(Msg::Swappiness)
                        .width(80)
                        .padding(8)
                        .size(SMALL_SIZE),
                    if ok_swappiness {
                        go(
                            "apply",
                            vec![
                                "--swappiness".into(),
                                self.swappiness.clone(),
                                "sysctl".into(),
                            ],
                        )
                    } else {
                        text("0-200").size(SMALL_SIZE).color(DIM).into()
                    },
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .into()
            ),
            item(
                "Swap file",
                "resize /home/swapfile (checks the free space first)",
                row![
                    text_input("8G", &self.swap)
                        .on_input(Msg::SwapSize)
                        .width(80)
                        .padding(8)
                        .size(SMALL_SIZE),
                    if ok_swap {
                        go("resize", vec!["swapfile".into(), self.swap.clone()])
                    } else {
                        text("e.g. 8G").size(SMALL_SIZE).color(DIM).into()
                    },
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .into()
            ),
            item(
                "SSH",
                "log in from another computer (set a password first: passwd)",
                go("enable", vec!["ssh".into()])
            ),
            item(
                "State",
                "what is configured now",
                go("refresh", vec!["status".into()])
            ),
        ]
        .spacing(SPACING);
        if self.busy {
            col = col.push(text("working...").size(TEXT_SIZE).color(DIM));
        }
        col = col.push(text(self.output.clone()).size(SMALL_SIZE));
        scrollable(col.padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }
}

/// Run `setup --system ARGS` and return (what was run, its output). `status` needs no root.
fn run_actions(args: Vec<String>) -> (String, String) {
    let label = args.join(" ");
    let script = std::env::temp_dir().join(format!("steamdeck-setup-{}.sh", std::process::id()));
    if let Err(e) = std::fs::write(&script, SETUP) {
        return (label, format!("could not write {}: {e}", script.display()));
    }
    let needs_root = args.iter().any(|a| a != "status");
    let mut cmd;
    if needs_root && std::env::var("USER").map(|u| u != "root").unwrap_or(true) {
        if which("pkexec") {
            cmd = Command::new("pkexec");
            cmd.arg("bash");
        } else {
            cmd = Command::new("sudo");
            cmd.arg("bash");
        }
    } else {
        cmd = Command::new("bash");
    }
    cmd.arg(&script).arg("--system").args(&args);
    let out = match cmd.output() {
        Ok(o) => {
            let mut t = String::from_utf8_lossy(&o.stdout).to_string();
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            if !o.status.success() {
                t.push_str(&format!("\n(exit status {})", o.status));
            }
            t
        }
        Err(e) => format!("could not start the script: {e}"),
    };
    let _ = std::fs::remove_file(&script);
    (label, out)
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
        .unwrap_or(false)
}
