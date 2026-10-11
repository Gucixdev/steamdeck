//! ProtonUp > Game storage: the table of prefixes and shader caches from `storage`, with the three
//! careful actions. Every action asks first and runs only the one that was asked about.
use crate::storage::{self, Entry};
use crate::ui::{danger_button, human_bytes, primary_button, quiet_button};
use crate::ui::{table_header, table_row, ACCENT, AMBER, DIM, SMALL_SIZE, SPACING, TEXT_SIZE};
use iced::widget::{button, column, row, scrollable, text};
use iced::{Element, Fill, Task};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Row {
    pub label: String,
    pub root: PathBuf,
    pub entry: Entry,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Act {
    Reset,
    Shader,
    Delete,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(Vec<Row>),
    Ask(usize, Act),
    Cancel,
    Do(usize, Act),
    Done(String),
    Reload,
}

pub struct State {
    rows: Option<Vec<Row>>,
    asking: Option<(usize, Act)>,
    output: String,
}

fn load() -> Task<Msg> {
    Task::perform(
        async {
            let home = dirs::home_dir().unwrap_or_default();
            let mut out = Vec::new();
            for tools in crate::steam::tool_dirs(&home) {
                let Some(root) = tools.parent().map(PathBuf::from) else {
                    continue;
                };
                let label = if root.to_string_lossy().contains("com.valvesoftware.Steam") {
                    "Flatpak"
                } else {
                    "native"
                };
                for e in storage::scan(&root, true) {
                    out.push(Row {
                        label: label.to_string(),
                        root: root.clone(),
                        entry: e,
                    });
                }
            }
            out.sort_by(|a, b| {
                (b.entry.prefix_bytes + b.entry.shader_bytes)
                    .cmp(&(a.entry.prefix_bytes + a.entry.shader_bytes))
            });
            out
        },
        Msg::Loaded,
    )
}

impl State {
    pub fn new() -> (Self, Task<Msg>) {
        (
            State {
                rows: None,
                asking: None,
                output: String::new(),
            },
            load(),
        )
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Loaded(v) => self.rows = Some(v),
            Msg::Reload => {
                self.rows = None;
                return load();
            }
            Msg::Ask(i, a) => self.asking = Some((i, a)),
            Msg::Cancel => self.asking = None,
            Msg::Do(i, a) => {
                if self.asking != Some((i, a)) {
                    return Task::none();
                }
                self.asking = None;
                let Some(row) = self.rows.as_ref().and_then(|v| v.get(i)).cloned() else {
                    return Task::none();
                };
                self.output = "working...".into();
                return Task::perform(async move { run(&row, a) }, Msg::Done);
            }
            Msg::Done(o) => {
                crate::log::write("storage", &o);
                self.output = o;
                self.rows = None;
                return load();
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let mut col = column![row![
            text("Prefix = the game's Windows folder (saves may be in it). Reset moves it aside; nothing is deleted.")
                .size(SMALL_SIZE)
                .color(DIM)
                .width(Fill),
            button(text("Rescan").size(SMALL_SIZE)).padding([6, 12]).on_press(Msg::Reload).style(quiet_button()),
        ]
        .align_y(iced::Alignment::Center)]
        .spacing(4);
        match &self.rows {
            None => col = col.push(text("Measuring...").size(TEXT_SIZE).color(DIM)),
            Some(v) if v.is_empty() => {
                col = col.push(
                    text("No prefixes or shader caches found.")
                        .size(TEXT_SIZE)
                        .color(DIM),
                )
            }
            Some(v) => {
                let total: u64 = v
                    .iter()
                    .map(|r| r.entry.prefix_bytes + r.entry.shader_bytes)
                    .sum();
                col = col.push(
                    text(format!(
                        "{} in prefixes and shader caches",
                        human_bytes(total)
                    ))
                    .size(SMALL_SIZE)
                    .color(ACCENT),
                );
                col = col.push(table_header(&[
                    ("Game", 4),
                    ("Steam", 1),
                    ("Prefix", 1),
                    ("Shaders", 1),
                    ("", 5),
                ]));
                for (n, r) in v.iter().enumerate() {
                    let e = &r.entry;
                    let name = match (&e.name, e.is_shortcut()) {
                        (Some(nm), _) => text(nm.clone()).size(TEXT_SIZE),
                        (None, true) => {
                            text(format!("non-Steam shortcut {}", e.appid)).size(TEXT_SIZE)
                        }
                        (None, false) => text(format!("not installed ({})", e.appid))
                            .size(TEXT_SIZE)
                            .color(AMBER),
                    };
                    let q = |a: Act| match a {
                        Act::Reset => "Move the prefix aside?",
                        Act::Shader => "Clear the shader cache?",
                        Act::Delete => "Delete prefix and shader cache for good?",
                    };
                    let actions: Element<Msg> = match self.asking {
                        Some((i, a)) if i == n => row![
                            text(q(a)).size(SMALL_SIZE),
                            button(text("Yes").size(SMALL_SIZE))
                                .padding([6, 12])
                                .on_press(Msg::Do(n, a))
                                .style(danger_button()),
                            button(text("No").size(SMALL_SIZE))
                                .padding([6, 12])
                                .on_press(Msg::Cancel)
                                .style(quiet_button()),
                        ]
                        .spacing(6)
                        .align_y(iced::Alignment::Center)
                        .into(),
                        _ => {
                            let mut r2 = row![].spacing(6);
                            if e.has_prefix {
                                r2 = r2.push(
                                    button(text("Reset prefix").size(SMALL_SIZE))
                                        .padding([6, 10])
                                        .on_press(Msg::Ask(n, Act::Reset))
                                        .style(quiet_button()),
                                );
                            }
                            if e.has_shader {
                                r2 = r2.push(
                                    button(text("Clear shaders").size(SMALL_SIZE))
                                        .padding([6, 10])
                                        .on_press(Msg::Ask(n, Act::Shader))
                                        .style(quiet_button()),
                                );
                            }
                            if !e.installed() && !e.is_shortcut() {
                                r2 = r2.push(
                                    button(text("Delete").size(SMALL_SIZE))
                                        .padding([6, 10])
                                        .on_press(Msg::Ask(n, Act::Delete))
                                        .style(primary_button()),
                                );
                            }
                            r2.into()
                        }
                    };
                    col = col.push(table_row(
                        vec![
                            (name.into(), 4),
                            (text(r.label.clone()).size(SMALL_SIZE).color(DIM).into(), 1),
                            (
                                text(human_bytes(e.prefix_bytes))
                                    .size(SMALL_SIZE)
                                    .color(DIM)
                                    .into(),
                                1,
                            ),
                            (
                                text(human_bytes(e.shader_bytes))
                                    .size(SMALL_SIZE)
                                    .color(DIM)
                                    .into(),
                                1,
                            ),
                            (actions, 5),
                        ],
                        n % 2 == 1,
                    ));
                }
            }
        }
        if !self.output.is_empty() {
            col = col.push(text(self.output.clone()).size(SMALL_SIZE));
        }
        scrollable(col.spacing(SPACING).padding(iced::Padding {
            right: 16.0,
            ..iced::Padding::ZERO
        }))
        .height(Fill)
        .into()
    }
}

fn run(r: &Row, a: Act) -> String {
    match a {
        Act::Reset => match storage::reset_prefix(&r.entry.steamapps, r.entry.appid, true) {
            Ok(to) => format!("moved the prefix to {}", to.display()),
            Err(e) => e,
        },
        Act::Shader => match storage::clear_shader(&r.entry.steamapps, r.entry.appid, true) {
            Ok(n) => format!("cleared the shader cache, freed {}", human_bytes(n)),
            Err(e) => e,
        },
        Act::Delete => match storage::delete_leftovers(&r.root, &r.entry, true) {
            Ok(n) => format!("deleted the leftovers, freed {}", human_bytes(n)),
            Err(e) => e,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_action_runs_only_after_it_was_asked() {
        let e = Entry {
            appid: 5,
            name: None,
            steamapps: PathBuf::from("/nonexistent/steamapps"),
            prefix_bytes: 0,
            shader_bytes: 0,
            has_prefix: true,
            has_shader: true,
        };
        let mut s = State {
            rows: Some(vec![Row {
                label: "x".into(),
                root: PathBuf::from("/nonexistent"),
                entry: e,
            }]),
            asking: None,
            output: String::new(),
        };
        let _ = s.update(Msg::Do(0, Act::Delete));
        assert!(s.output.is_empty());
        let _ = s.update(Msg::Ask(0, Act::Reset));
        let _ = s.update(Msg::Do(0, Act::Delete));
        assert!(
            s.output.is_empty(),
            "another action than the one asked is ignored"
        );
    }
}
