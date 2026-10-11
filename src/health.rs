//! Settings > Health: what the system is set to now against what games like (read only), and the
//! recent crashes of Steam, Proton and Wine from the system journal. Nothing here changes anything;
//! the System page has the buttons for that.
use crate::ui::{
    heading, table_header, table_row, ACCENT, AMBER, DIM, SMALL_SIZE, SPACING, TEXT_SIZE,
};
use iced::widget::{column, scrollable, text};
use iced::{Element, Fill, Task};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub current: String,
    pub wanted: &'static str,
    pub ok: bool,
    pub why: &'static str,
}

fn read(p: &str) -> String {
    std::fs::read_to_string(p)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".into())
}

/// The checklist from values already read, so it can be tested.
pub fn checks_from(
    swappiness: &str,
    max_map: &str,
    governor: &str,
    zram: bool,
    swap_kb: u64,
) -> Vec<Check> {
    let sw: i64 = swappiness.parse().unwrap_or(-1);
    let mm: i64 = max_map.parse().unwrap_or(-1);
    vec![
        Check {
            name: "vm.max_map_count",
            current: max_map.to_string(),
            wanted: "at least 1048576",
            ok: mm >= 1_048_576,
            why: "many games (and Proton) map more memory areas than the old default of 65530 allows and crash",
        },
        Check {
            name: "vm.swappiness",
            current: swappiness.to_string(),
            wanted: "100 with zram, 10-60 without",
            ok: if zram { sw >= 60 } else { (0..=60).contains(&sw) },
            why: "zram is fast, so swapping early keeps the file cache; without it the disk is slow",
        },
        Check {
            name: "zram swap",
            current: if zram { "yes".into() } else { "no".into() },
            wanted: "yes",
            ok: zram,
            why: "compressed swap in memory is what makes 16 GB last on a Deck",
        },
        Check {
            name: "swap file",
            current: format!("{} MB total swap", swap_kb / 1024),
            wanted: "some disk swap behind zram",
            ok: swap_kb > 0,
            why: "without any swap an out-of-memory game is killed outright",
        },
        Check {
            name: "CPU governor",
            current: governor.to_string(),
            wanted: "schedutil, powersave or performance",
            ok: matches!(governor, "schedutil" | "powersave" | "performance" | "ondemand"),
            why: "an odd governor usually means a leftover tweak from another tool",
        },
    ]
}

pub fn checks() -> Vec<Check> {
    let swaps = read("/proc/swaps");
    let zram = swaps.contains("zram");
    let swap_kb: u64 = swaps
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().nth(2)?.parse::<u64>().ok())
        .sum();
    checks_from(
        &read("/proc/sys/vm/swappiness"),
        &read("/proc/sys/vm/max_map_count"),
        &read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        zram,
        swap_kb,
    )
}

/// Recent crash lines about Steam, Proton and Wine, newest first; empty when there are none.
pub fn crashes() -> String {
    let mut out = String::new();
    if let Ok(o) = Command::new("coredumpctl")
        .args(["list", "--no-pager", "--no-legend", "--since=-14d"])
        .output()
    {
        let t = String::from_utf8_lossy(&o.stdout);
        let hits: Vec<&str> = t
            .lines()
            .filter(|l| {
                let l = l.to_lowercase();
                [
                    "steam",
                    "wine",
                    "proton",
                    "pressure-vessel",
                    "gamescope",
                    "reaper",
                ]
                .iter()
                .any(|k| l.contains(k))
            })
            .collect();
        for l in hits.iter().rev().take(15) {
            out.push_str(l);
            out.push('\n');
        }
    }
    if let Ok(o) = Command::new("journalctl")
        .args([
            "--user",
            "-b",
            "-p",
            "err",
            "--no-pager",
            "-n",
            "200",
            "-o",
            "short",
        ])
        .output()
    {
        let t = String::from_utf8_lossy(&o.stdout);
        let hits: Vec<&str> = t
            .lines()
            .filter(|l| {
                let l = l.to_lowercase();
                ["wine", "proton", "steam", "vkd3d", "dxvk"]
                    .iter()
                    .any(|k| l.contains(k))
            })
            .collect();
        for l in hits.iter().rev().take(15) {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(Vec<Check>, String),
}

pub struct State {
    checks: Vec<Check>,
    crashes: Option<String>,
}

impl State {
    pub fn new() -> (Self, Task<Msg>) {
        (
            State {
                checks: Vec::new(),
                crashes: None,
            },
            Task::perform(
                crate::ui::blocking(move || (checks(), crashes())),
                |(c, k)| Msg::Loaded(c, k),
            ),
        )
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        let Msg::Loaded(c, k) = m;
        self.checks = c;
        self.crashes = Some(k);
        Task::none()
    }

    pub fn problems(&self) -> usize {
        self.checks.iter().filter(|c| !c.ok).count()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let mut col = column![heading("Checklist: now against what games like")].spacing(4);
        col = col.push(table_header(&[("Setting", 3), ("Now", 3), ("Wanted", 4)]));
        for (n, c) in self.checks.iter().enumerate() {
            let tick = if c.ok {
                text("ok").size(SMALL_SIZE).color(ACCENT)
            } else {
                text("check").size(SMALL_SIZE).color(AMBER)
            };
            col = col.push(table_row(
                vec![
                    (text(c.name).size(TEXT_SIZE).into(), 3),
                    (
                        column![text(c.current.clone()).size(SMALL_SIZE), tick].into(),
                        3,
                    ),
                    (
                        column![
                            text(c.wanted).size(SMALL_SIZE),
                            text(c.why).size(SMALL_SIZE).color(DIM)
                        ]
                        .into(),
                        4,
                    ),
                ],
                n % 2 == 1,
            ));
        }
        col = col.push(heading("Recent crashes (Steam, Proton, Wine)"));
        let body = match &self.crashes {
            None => "Looking...".to_string(),
            Some(k) if k.trim().is_empty() => "none in the last two weeks".to_string(),
            Some(k) => k.clone(),
        };
        col = col.push(text(body).size(SMALL_SIZE));
        scrollable(col.spacing(SPACING).padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checklist_flags_what_games_dislike() {
        let bad = checks_from("60", "65530", "powersave", false, 0);
        assert!(!bad[0].ok, "old max_map_count");
        assert!(!bad[2].ok, "no zram");
        assert!(!bad[3].ok, "no swap at all");
        let good = checks_from("100", "2147483642", "schedutil", true, 8_000_000);
        assert!(good.iter().all(|c| c.ok));
    }
}
