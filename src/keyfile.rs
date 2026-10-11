//! A GLib keyfile (the INI dialect Flatpak uses for metadata and overrides). Order, comments and keys
//! this program does not understand are kept, so a round trip does not lose the user's other settings.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyFile {
    /// comment lines above the first group, kept verbatim
    head: Vec<String>,
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
            } else if cur.is_none() {
                if !t.is_empty() {
                    kf.head.push(line.to_string());
                }
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
        for h in &self.head {
            out.push_str(h);
            out.push('\n');
        }
        let mut wrote_group = false;
        for (name, lines) in &self.groups {
            // a group left empty by an edit is not written at all
            if lines
                .iter()
                .all(|l| matches!(l, Line::Other(s) if s.trim().is_empty()))
            {
                continue;
            }
            if wrote_group {
                out.push('\n');
            }
            wrote_group = true;
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
        // a newline in a key, a value or a group name would start a new line of the file: the way to
        // slip another key or group in through a text box. Keys that cannot be written are refused,
        // newlines in values are written the way GLib escapes them.
        if key.is_empty()
            || key.contains(['\n', '\r', '='])
            || key.starts_with('[')
            || group.contains(['\n', '\r', ']'])
        {
            return;
        }
        let escaped;
        let value = if value.contains(['\n', '\r']) {
            escaped = value.replace('\n', "\\n").replace('\r', "\\r");
            escaped.as_str()
        } else {
            value
        };
        // `get` reads the last copy of a key, so `set` must change that one, not the first
        let mut hit: Option<(usize, usize)> = None;
        for (gi, (g, lines)) in self.groups.iter().enumerate() {
            if g != group {
                continue;
            }
            for (li, l) in lines.iter().enumerate() {
                if matches!(l, Line::Pair(k, _) if k == key) {
                    hit = Some((gi, li));
                }
            }
        }
        if let Some((gi, li)) = hit {
            self.groups[gi].1[li] = Line::Pair(key.to_string(), value.to_string());
            return;
        }
        let gi = match self.groups.iter().rposition(|(g, _)| g == group) {
            Some(i) => i,
            None => {
                self.groups.push((group.to_string(), Vec::new()));
                self.groups.len() - 1
            }
        };
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

#[cfg(test)]
mod edge_tests {
    use super::*;

    #[test]
    fn a_comment_above_the_first_group_survives_an_edit() {
        let mut k = KeyFile::parse("# written by hand\n[Context]\nshared=network;\n");
        k.set("Context", "sockets", "x11;");
        let t = k.to_text();
        assert!(t.starts_with("# written by hand\n[Context]\n"), "{t}");
    }

    #[test]
    fn a_value_may_contain_equals_signs_and_spaces() {
        let k = KeyFile::parse("[Environment]\nOPTS = a=b=c d\n");
        assert_eq!(k.get("Environment", "OPTS"), Some("a=b=c d"));
        assert_eq!(k.to_text(), "[Environment]\nOPTS=a=b=c d\n");
    }

    #[test]
    fn windows_line_endings_do_not_end_up_in_values() {
        let k = KeyFile::parse("[Context]\r\nshared=network;\r\n");
        assert_eq!(k.list("Context", "shared"), ["network"]);
        assert!(!k.to_text().contains('\r'));
    }

    #[test]
    fn set_changes_the_copy_get_reads_when_a_key_is_duplicated() {
        let mut k = KeyFile::parse("[Context]\nshared=network;\nshared=ipc;\n");
        k.set("Context", "shared", "x;");
        assert_eq!(k.get("Context", "shared"), Some("x;"));
    }

    #[test]
    fn set_changes_the_copy_get_reads_when_a_group_is_repeated() {
        let mut k = KeyFile::parse("[Context]\na=1\n[Other]\nx=y\n[Context]\na=2\n");
        k.set("Context", "a", "3");
        assert_eq!(k.get("Context", "a"), Some("3"));
        k.set("Context", "b", "4");
        assert_eq!(k.get("Context", "b"), Some("4"));
    }

    #[test]
    fn remove_takes_away_every_copy() {
        let mut k = KeyFile::parse("[Context]\na=1\n[Context]\na=2\n");
        k.remove("Context", "a");
        assert_eq!(k.get("Context", "a"), None);
        assert!(k.is_empty());
    }

    #[test]
    fn empty_and_garbage_input_do_not_panic() {
        for t in [
            "",
            "\n\n",
            "no group here\n",
            "[unterminated\nkey=1\n",
            "[]\na=b\n",
            "=\n",
        ] {
            let k = KeyFile::parse(t);
            let _ = k.to_text();
            let _ = k.list("Context", "shared");
        }
        assert!(KeyFile::parse("").is_empty());
    }

    #[test]
    fn a_list_with_empty_items_and_spaces_is_cleaned() {
        let k = KeyFile::parse("[Context]\nshared= network ;;ipc;\n");
        assert_eq!(k.list("Context", "shared"), ["network", "ipc"]);
    }

    #[test]
    fn a_text_box_cannot_slip_a_new_key_or_group_into_the_file() {
        let mut k = KeyFile::default();
        k.set("Environment", "A", "x\n[Context]\nsockets=!x11");
        k.set("Session Bus Policy", "org.a\nshared=network", "talk");
        k.set("Session Bus Policy", "org.b=c", "talk");
        k.set("Session Bus Policy", "", "talk");
        k.set("Bad]\n[Context", "k", "v");
        let again = KeyFile::parse(&k.to_text());
        assert_eq!(
            again.list("Context", "sockets"),
            Vec::<String>::new(),
            "{}",
            k.to_text()
        );
        assert_eq!(again.get("Context", "shared"), None);
        assert_eq!(
            again.pairs("Session Bus Policy"),
            Vec::<(String, String)>::new()
        );
        assert_eq!(
            again.get("Environment", "A"),
            Some("x\\n[Context]\\nsockets=!x11")
        );
    }
}
