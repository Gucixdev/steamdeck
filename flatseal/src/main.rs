//! flatseal: Flatpak permission editor for the Steam Deck. No arguments opens the window; the same
//! operations are available on the command line.
mod gui;

use flatseal::flatpak::{self, GLOBAL};
use flatseal::perms::{self, SECTIONS};
use std::process::ExitCode;

macro_rules! say {
    ($($a:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($a)*);
    }};
}

const HELP: &str = "\
flatseal - edit Flatpak permission overrides

usage:
  flatseal [gui [APP]]              open the window, optionally on APP
  flatseal list                     installed apps
  flatseal show APP                 effective permissions; '*' marks what this app overrides
  flatseal allow APP KEY NAME       KEY is shared, sockets, devices, features or filesystems
  flatseal deny APP KEY NAME       explicit !NAME, even if nothing allows it now
  flatseal default APP KEY NAME    drop this app's own entry for NAME (inherit)
  flatseal portals APP              what the portal permission store holds for APP
  flatseal portal APP NAME yes|no|default   NAME: background, notification, camera, microphone, speakers
  flatseal revoke APP TABLE ID      take back one stored entry (see `portals`)
  flatseal reset APP                drop the app's own overrides
  flatseal fs-add APP PATH[:ro|:rw|:create]
  flatseal fs-remove APP PATH

APP is an application id, or 'global' for the overrides that apply to every app.
Overrides are the files under $FLATPAK_USER_DIR/overrides (default ~/.local/share/flatpak), the same
ones `flatpak override --user` writes.";

fn fail(m: impl std::fmt::Display) -> ExitCode {
    eprintln!("flatseal: {m}");
    ExitCode::FAILURE
}

fn save(id: &str, l: &perms::Layers) -> Result<(), String> {
    flatpak::save_override(id, &l.app)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = |i: usize| args.get(i).map(String::as_str);
    match a(0).unwrap_or("gui") {
        "-h" | "--help" | "help" => {
            say!("{HELP}");
            ExitCode::SUCCESS
        }
        "gui" => gui::run(a(1).map(str::to_string))
            .map(|_| ExitCode::SUCCESS)
            .unwrap_or_else(fail),
        "list" => match flatpak::list_apps() {
            Ok(v) => {
                for app in v {
                    say!("{:<44} {}", app.id, app.name);
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        "show" => {
            let Some(id) = a(1) else {
                return fail("show needs an app id");
            };
            let l = flatpak::load_layers(id);
            for s in SECTIONS {
                say!("[{}]", s.title);
                for (name, _) in s.items {
                    let on = l.is_on(s.key, name);
                    let star = if l.is_overridden(s.key, name) {
                        "*"
                    } else {
                        " "
                    };
                    say!("  {star} {:<3} {name}", if on { "on" } else { "off" });
                }
                if s.key == "filesystems" {
                    for e in l.custom_filesystems() {
                        say!("  {} on  {}", if e.overridden { "*" } else { " " }, e.name);
                    }
                }
            }
            for (title, group) in [
                ("Session bus", perms::SESSION_BUS),
                ("System bus", perms::SYSTEM_BUS),
            ] {
                let b = l.bus(group);
                if !b.is_empty() {
                    say!("[{title}]");
                    for e in b {
                        say!(
                            "  {} {:<6} {}",
                            if e.overridden { "*" } else { " " },
                            e.value,
                            e.name
                        );
                    }
                }
            }
            let env = l.env();
            if !env.is_empty() {
                say!("[Environment]");
                for e in env {
                    say!(
                        "  {} {}={}",
                        if e.overridden { "*" } else { " " },
                        e.name,
                        e.value
                    );
                }
            }
            ExitCode::SUCCESS
        }
        "portals" => {
            let Some(id) = a(1) else {
                return fail("portals needs an app id");
            };
            let g = flatseal::portals::grants(id);
            for p in flatseal::portals::PORTALS {
                let v = match flatseal::portals::answer(&g, p) {
                    flatseal::portals::Answer::Yes => "yes",
                    flatseal::portals::Answer::No => "no",
                    flatseal::portals::Answer::Default => "default",
                };
                say!("  {:<13} {:<8} {}", p.name, v, p.about);
            }
            for e in g.iter().filter(|e| e.table == "documents") {
                say!(
                    "  file          {}  {}",
                    e.id,
                    flatseal::portals::document_path(&e.data).unwrap_or_default()
                );
            }
            ExitCode::SUCCESS
        }
        "portal" => {
            let (Some(id), Some(name), Some(v)) = (a(1), a(2), a(3)) else {
                return fail("portal needs APP NAME yes|no|default");
            };
            let Some(p) = flatseal::portals::PORTALS.iter().find(|p| p.name == name) else {
                return fail(format!("unknown portal '{name}' (background, notification, camera, microphone, speakers)"));
            };
            let ans = match v {
                "yes" => flatseal::portals::Answer::Yes,
                "no" => flatseal::portals::Answer::No,
                "default" => flatseal::portals::Answer::Default,
                _ => return fail("the value is yes, no or default"),
            };
            match flatseal::portals::set(id, p, ans) {
                Ok(()) => {
                    say!("{id}: {name} {v}");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        "revoke" => {
            let (Some(id), Some(table), Some(obj)) = (a(1), a(2), a(3)) else {
                return fail("revoke needs APP TABLE ID");
            };
            match flatseal::portals::revoke(id, table, obj) {
                Ok(()) => {
                    say!("{id}: revoked {table} {obj}");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        cmd @ ("allow" | "deny" | "default") => {
            let (Some(id), Some(key), Some(name)) = (a(1), a(2), a(3)) else {
                return fail(format!("{cmd} needs APP KEY NAME"));
            };
            if !SECTIONS.iter().any(|s| s.key == key) {
                return fail(format!(
                    "unknown key '{key}' (shared, sockets, devices, features, filesystems)"
                ));
            }
            let mut l = flatpak::load_layers(id);
            l.set_state(
                key,
                name,
                match cmd {
                    "allow" => perms::Tri::Allow,
                    "deny" => perms::Tri::Deny,
                    _ => perms::Tri::Default,
                },
            );
            match save(id, &l) {
                Ok(()) => {
                    say!(
                        "{id}: {key} {name} {}",
                        match cmd {
                            "allow" => "allowed",
                            "deny" => "denied",
                            _ => "back to default",
                        }
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        "reset" => {
            let Some(id) = a(1) else {
                return fail("reset needs an app id");
            };
            match flatpak::save_override(id, &Default::default()) {
                Ok(()) => {
                    say!("{id}: overrides removed");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        cmd @ ("fs-add" | "fs-remove") => {
            let (Some(id), Some(path)) = (a(1), a(2)) else {
                return fail(format!("{cmd} needs APP PATH"));
            };
            let mut l = flatpak::load_layers(id);
            if cmd == "fs-add" {
                l.add_filesystem(path);
            } else {
                l.remove_filesystem(path);
            }
            match save(id, &l) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(e),
            }
        }
        other => {
            eprintln!("flatseal: unknown command '{other}'\n\n{HELP}");
            let _ = GLOBAL;
            ExitCode::from(2)
        }
    }
}
