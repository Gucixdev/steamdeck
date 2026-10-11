//! Portal permissions: what the xdg-desktop-portal permission store remembers per app (background,
//! notifications, camera, microphone, speakers) and the files an app was granted through the file
//! chooser. Uses the `flatpak permission-*` commands, which talk to the same store Flatseal and the
//! portals use; nothing here edits its database directly.
use std::process::Command;

/// A portal permission the interface offers: label, store table, object id, what it controls.
pub struct Portal {
    pub name: &'static str,
    pub table: &'static str,
    pub id: &'static str,
    pub about: &'static str,
}

pub const PORTALS: &[Portal] = &[
    Portal {
        name: "background",
        table: "background",
        id: "background",
        about: "run in the background and autostart",
    },
    Portal {
        name: "notification",
        table: "notifications",
        id: "notification",
        about: "show desktop notifications",
    },
    Portal {
        name: "camera",
        table: "devices",
        id: "camera",
        about: "use the camera",
    },
    Portal {
        name: "microphone",
        table: "devices",
        id: "microphone",
        about: "record sound",
    },
    Portal {
        name: "speakers",
        table: "devices",
        id: "speakers",
        about: "play sound",
    },
];

/// One entry of the permission store for an app.
#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub table: String,
    pub id: String,
    pub perms: Vec<String>,
    pub data: String,
}

/// A portal permission as the store has it: no entry, yes or no.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Default,
    Yes,
    No,
}

/// `flatpak permission-show` prints tab-separated `table id app permissions data`.
pub fn parse_show(out: &str) -> Vec<Grant> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 4 {
                return None;
            }
            Some(Grant {
                table: f[0].trim().to_string(),
                id: f[1].trim().to_string(),
                perms: f[3]
                    .split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect(),
                data: f.get(4).map(|d| d.trim().to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

/// The path out of a documents entry's data, `(b'/home/me/file.png', 56, ...)`.
pub fn document_path(data: &str) -> Option<String> {
    // GVariant prints a byte string as b'...' with \' and \\ escaped inside
    let rest = data.strip_prefix("(b'")?;
    let mut out = String::new();
    let mut it = rest.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => out.push(it.next()?),
            '\'' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

fn valid(s: &str) -> bool {
    crate::flatpak::valid_id(s)
}

pub fn grants(app: &str) -> Vec<Grant> {
    if !valid(app) {
        return Vec::new();
    }
    Command::new("flatpak")
        .args(["permission-show", app])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_show(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// What the store says for one of [`PORTALS`].
pub fn answer(grants: &[Grant], p: &Portal) -> Answer {
    match grants
        .iter()
        .find(|g| g.table == p.table && g.id == p.id)
        .and_then(|g| g.perms.first())
    {
        Some(v) if v == "yes" => Answer::Yes,
        Some(v) if v == "no" => Answer::No,
        _ => Answer::Default,
    }
}

fn run(args: &[&str]) -> Result<(), String> {
    let o = Command::new("flatpak")
        .args(args)
        .output()
        .map_err(|e| format!("flatpak: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

pub fn set(app: &str, p: &Portal, a: Answer) -> Result<(), String> {
    if !valid(app) {
        return Err(format!("odd app id '{app}'"));
    }
    match a {
        Answer::Yes => run(&["permission-set", p.table, p.id, app, "yes"]),
        Answer::No => run(&["permission-set", p.table, p.id, app, "no"]),
        Answer::Default => run(&["permission-remove", p.table, p.id, app]),
    }
}

/// Take back one granted entry (a file the app was given through the portal, for instance).
pub fn revoke(app: &str, table: &str, id: &str) -> Result<(), String> {
    if !valid(app) || !valid(table) || !valid(id) {
        return Err("odd name".into());
    }
    run(&["permission-remove", table, id, app])
}

/// Forget everything the store holds for the app.
pub fn reset(app: &str) -> Result<(), String> {
    if !valid(app) {
        return Err(format!("odd app id '{app}'"));
    }
    run(&["permission-reset", app])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_output_is_parsed() {
        let out = "documents\tWRMj7\torg.a.B\tread,write,grant-permissions\t(b'/home/me/a.png', 56, 510, 0, [0xfe])\nbackground\tbackground\torg.a.B\tyes\t\n";
        let g = parse_show(out);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].perms, ["read", "write", "grant-permissions"]);
        assert_eq!(document_path(&g[0].data).as_deref(), Some("/home/me/a.png"));
        assert_eq!(answer(&g, &PORTALS[0]), Answer::Yes);
        assert_eq!(answer(&g, &PORTALS[1]), Answer::Default);
    }

    #[test]
    fn names_with_odd_characters_are_refused() {
        assert!(revoke("org.a.B", "documents", "x;rm").is_err());
        assert!(set("../x", &PORTALS[0], Answer::Yes).is_err());
    }

    #[test]
    fn a_file_name_with_an_apostrophe_or_backslash_is_read_whole() {
        assert_eq!(
            document_path(r"(b'/home/me/Bob\'s file.png', 56)").as_deref(),
            Some("/home/me/Bob's file.png")
        );
        assert_eq!(
            document_path(r"(b'/home/me/a\\b.png', 56)").as_deref(),
            Some(r"/home/me/a\b.png")
        );
        assert_eq!(document_path("(b'/unterminated"), None);
        assert_eq!(document_path("garbage"), None);
    }

    #[test]
    fn short_or_empty_store_lines_are_skipped_and_unknown_answers_are_default() {
        let g = parse_show("\n\nonly\ttwo\ndevices\tcamera\torg.a.B\task\t\n");
        assert_eq!(g.len(), 1);
        assert_eq!(
            answer(&g, &PORTALS[2]),
            Answer::Default,
            "'ask' is neither yes nor no"
        );
        assert!(parse_show("").is_empty());
    }
}
