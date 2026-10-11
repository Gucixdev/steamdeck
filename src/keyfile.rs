//! A GLib keyfile (the INI dialect Flatpak uses for metadata and overrides). Order, comments and keys
//! this program does not understand are kept, so a round trip does not lose the user's other settings.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyFile {
    /// (group name, lines); a line is either `key=value` or a verbatim comment / blank line
    groups: Vec<(String, Vec<Line>)>,
}

#[derive(Debug, Clone, PartialEq)]
enum Line {
    Pair(String, String),
    Other(String),
}

impl KeyFile {
    pub fn parse(text: &str) -> KeyFile {
        let mut kf = KeyFile::default();
        let mut cur: Option<usize> = None;
        for raw in text.lines() {
            let line = raw.trim_end();
            let t = line.trim_start();
            if t.starts_with('[') && t.ends_with(']') {
                kf.groups.push((t[1..t.len() - 1].to_string(), Vec::new()));
                cur = Some(kf.groups.len() - 1);
            } else if let Some(g) = cur {
                if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
                    kf.groups[g].1.push(Line::Other(line.to_string()));
                } else if let Some((k, v)) = t.split_once('=') {
                    kf.groups[g]
                        .1
                        .push(Line::Pair(k.trim().to_string(), v.trim().to_string()));
                } else {
                    kf.groups[g].1.push(Line::Other(line.to_string()));
                }
            }
        }
        kf
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (name, lines) in &self.groups {
            // a group left empty by an edit is not written at all
            if lines
                .iter()
                .all(|l| matches!(l, Line::Other(s) if s.trim().is_empty()))
            {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{name}]\n"));
            for l in lines {
                match l {
                    Line::Pair(k, v) => out.push_str(&format!("{k}={v}\n")),
                    Line::Other(s) => {
                        if !s.trim().is_empty() {
                            out.push_str(s);
                            out.push('\n');
                        }
                    }
                }
            }
        }
        out
    }

    pub fn get(&self, group: &str, key: &str) -> Option<&str> {
        self.groups
            .iter()
            .filter(|(g, _)| g == group)
            .flat_map(|(_, l)| l.iter())
            .rev()
            .find_map(|l| match l {
                Line::Pair(k, v) if k == key => Some(v.as_str()),
                _ => None,
            })
    }

    pub fn set(&mut self, group: &str, key: &str, value: &str) {
        let gi = match self.groups.iter().position(|(g, _)| g == group) {
            Some(i) => i,
            None => {
                self.groups.push((group.to_string(), Vec::new()));
                self.groups.len() - 1
            }
        };
        for l in &mut self.groups[gi].1 {
            if let Line::Pair(k, v) = l {
                if k == key {
                    *v = value.to_string();
                    return;
                }
            }
        }
        self.groups[gi]
            .1
            .push(Line::Pair(key.to_string(), value.to_string()));
    }

    pub fn remove(&mut self, group: &str, key: &str) {
        for (g, lines) in &mut self.groups {
            if g == group {
                lines.retain(|l| !matches!(l, Line::Pair(k, _) if k == key));
            }
        }
    }

    /// Every `key=value` of a group, in file order.
    pub fn pairs(&self, group: &str) -> Vec<(String, String)> {
        self.groups
            .iter()
            .filter(|(g, _)| g == group)
            .flat_map(|(_, l)| l.iter())
            .filter_map(|l| match l {
                Line::Pair(k, v) => Some((k.clone(), v.clone())),
                _ => None,
            })
            .collect()
    }

    /// A `;`-separated list value: `a;b;c;` -> [a, b, c]
    pub fn list(&self, group: &str, key: &str) -> Vec<String> {
        self.get(group, key)
            .map(|v| {
                v.split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_list(&mut self, group: &str, key: &str, items: &[String]) {
        if items.is_empty() {
            self.remove(group, key);
        } else {
            self.set(group, key, &format!("{};", items.join(";")));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.to_text().trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[Application]
name=org.example.App
runtime=runtime/org.freedesktop.Platform/x86_64/24.08

[Context]
shared=network;ipc;
sockets=x11;wayland;
# a comment the user wrote
filesystems=xdg-download;~/Games:rw;

[Session Bus Policy]
org.freedesktop.Notifications=talk
";

    #[test]
    fn parses_lists_and_pairs() {
        let k = KeyFile::parse(SAMPLE);
        assert_eq!(k.list("Context", "shared"), ["network", "ipc"]);
        assert_eq!(
            k.list("Context", "filesystems"),
            ["xdg-download", "~/Games:rw"]
        );
        assert_eq!(k.get("Application", "name"), Some("org.example.App"));
        assert_eq!(
            k.pairs("Session Bus Policy"),
            [("org.freedesktop.Notifications".into(), "talk".into())]
        );
        assert!(k.list("Context", "missing").is_empty());
    }

    #[test]
    fn round_trip_keeps_everything() {
        let k = KeyFile::parse(SAMPLE);
        let again = KeyFile::parse(&k.to_text());
        assert_eq!(k, again);
        assert!(k.to_text().contains("# a comment the user wrote"));
    }

    #[test]
    fn edits_replace_add_and_drop_empty_groups() {
        let mut k = KeyFile::parse(SAMPLE);
        k.set_list("Context", "sockets", &["x11".into()]);
        k.set("Environment", "FOO", "bar");
        k.set_list("Context", "shared", &[]);
        k.remove("Session Bus Policy", "org.freedesktop.Notifications");
        let t = k.to_text();
        assert!(t.contains("sockets=x11;\n"));
        assert!(!t.contains("shared="));
        assert!(t.contains("[Environment]\nFOO=bar"));
        assert!(
            !t.contains("Session Bus Policy"),
            "an emptied group disappears: {t}"
        );
    }
}
