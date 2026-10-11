//! The permission model: what an app asks for in its metadata, what the global and per-app overrides
//! change, and the toggles built on top. An override entry `x` grants, `!x` revokes, absence leaves
//! the layer below alone; a toggle writes an override only when the wanted state differs from what the
//! layers below already give, so overrides stay minimal.
use crate::keyfile::KeyFile;
use std::collections::BTreeSet;

pub const CONTEXT: &str = "Context";
pub const SESSION_BUS: &str = "Session Bus Policy";
pub const SYSTEM_BUS: &str = "System Bus Policy";
pub const ENV: &str = "Environment";

pub struct Section {
    pub title: &'static str,
    pub key: &'static str,
    pub items: &'static [(&'static str, &'static str)],
}

pub const SECTIONS: &[Section] = &[
    Section {
        title: "Share",
        key: "shared",
        items: &[
            ("network", "Network access"),
            ("ipc", "Inter-process communication (X11 shared memory)"),
        ],
    },
    Section {
        title: "Sockets",
        key: "sockets",
        items: &[
            ("x11", "X11 windowing"),
            ("fallback-x11", "X11 only when Wayland is missing"),
            ("wayland", "Wayland windowing"),
            (
                "pulseaudio",
                "Sound: the PulseAudio socket, which PipeWire serves too (key: pulseaudio)",
            ),
            ("session-bus", "Session message bus, unfiltered"),
            ("system-bus", "System message bus, unfiltered"),
            ("ssh-auth", "Remote login (SSH agent)"),
            ("pcsc", "Smart cards"),
            ("cups", "Printing"),
            ("gpg-agent", "GPG agent"),
        ],
    },
    Section {
        title: "Devices",
        key: "devices",
        items: &[
            ("dri", "Graphics acceleration"),
            ("kvm", "Virtualization"),
            ("shm", "Shared memory"),
            ("all", "All devices (webcam, controllers, ...)"),
        ],
    },
    Section {
        title: "Features",
        key: "features",
        items: &[
            ("devel", "Development syscalls (ptrace, perf)"),
            ("multiarch", "Run 32 bit code"),
            ("bluetooth", "Bluetooth"),
            ("canbus", "CAN bus"),
            ("per-app-dev-shm", "Private /dev/shm"),
        ],
    },
    Section {
        title: "Filesystem",
        key: "filesystems",
        items: &[
            ("host", "All files"),
            ("host-os", "Operating system files (/usr, read only)"),
            ("host-etc", "System configuration (/etc, read only)"),
            ("home", "Home folder"),
            ("xdg-desktop", "Desktop"),
            ("xdg-documents", "Documents"),
            ("xdg-download", "Downloads"),
            ("xdg-music", "Music"),
            ("xdg-pictures", "Pictures"),
            ("xdg-videos", "Videos"),
            ("xdg-public-share", "Public"),
            ("xdg-templates", "Templates"),
            ("xdg-config", "User configuration"),
            ("xdg-cache", "User cache"),
            ("xdg-data", "User data"),
            ("xdg-run/pipewire-0", "PipeWire (sound and screen capture)"),
        ],
    },
];

/// Names a filesystem entry is toggled under: the path without its `:ro` / `:rw` / `:create` mode.
fn fs_path(entry: &str) -> &str {
    entry
        .rsplit_once(':')
        .filter(|(_, m)| matches!(*m, "ro" | "rw" | "create"))
        .map(|(p, _)| p)
        .unwrap_or(entry)
}

fn apply(set: &mut Vec<String>, tokens: &[String], key: &str) {
    for t in tokens {
        let (neg, name) = match t.strip_prefix('!') {
            Some(n) => (true, n),
            None => (false, t.as_str()),
        };
        let same = |e: &String| {
            if key == "filesystems" {
                fs_path(e) == fs_path(name)
            } else {
                e == name
            }
        };
        set.retain(|e| !same(e));
        if !neg {
            set.push(t.clone());
        }
    }
}

/// A permission as one layer sees it: untouched, explicitly allowed or explicitly denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri {
    Default,
    Allow,
    Deny,
}

/// One app: its metadata, the global overrides and its own overrides, lowest layer first.
#[derive(Debug, Clone, Default)]
pub struct Layers {
    pub meta: KeyFile,
    pub global: KeyFile,
    pub app: KeyFile,
    /// editing the global overrides themselves: there is no layer below to compare with, so a toggle
    /// always writes an explicit `name` or `!name`
    pub is_global: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub value: String,
    /// set by this app's own override (as opposed to coming from metadata or the global override)
    pub overridden: bool,
}

impl Layers {
    /// What the layers below the app's own override give.
    pub fn base_list(&self, key: &str) -> Vec<String> {
        let mut v = Vec::new();
        apply(&mut v, &self.meta.list(CONTEXT, key), key);
        apply(&mut v, &self.global.list(CONTEXT, key), key);
        v
    }

    pub fn effective_list(&self, key: &str) -> Vec<String> {
        let mut v = self.base_list(key);
        apply(&mut v, &self.app.list(CONTEXT, key), key);
        v.sort();
        v
    }

    pub fn is_on(&self, key: &str, name: &str) -> bool {
        self.effective_list(key).iter().any(|e| {
            if key == "filesystems" {
                fs_path(e) == name
            } else {
                e == name
            }
        })
    }

    pub fn is_overridden(&self, key: &str, name: &str) -> bool {
        self.app.list(CONTEXT, key).iter().any(|t| {
            let n = t.strip_prefix('!').unwrap_or(t);
            if key == "filesystems" {
                fs_path(n) == name
            } else {
                n == name
            }
        })
    }

    /// What this layer says about `name`: nothing, an explicit allow or an explicit deny.
    pub fn own_state(&self, key: &str, name: &str) -> Tri {
        let mut st = Tri::Default;
        for t in self.app.list(CONTEXT, key) {
            let (neg, n) = match t.strip_prefix('!') {
                Some(n) => (true, n),
                None => (false, t.as_str()),
            };
            if if key == "filesystems" {
                fs_path(n) == name
            } else {
                n == name
            } {
                st = if neg { Tri::Deny } else { Tri::Allow };
            }
        }
        st
    }

    /// Set `name` to default (no entry, inherit), allowed (`name`) or denied (`!name`). The two
    /// explicit states are always written, even when they equal what is inherited, so a later
    /// change below does not silently flip them.
    pub fn set_state(&mut self, key: &str, name: &str, st: Tri) {
        let mut own = self.app.list(CONTEXT, key);
        own.retain(|t| {
            let n = t.strip_prefix('!').unwrap_or(t);
            !(if key == "filesystems" {
                fs_path(n) == name
            } else {
                n == name
            })
        });
        match st {
            Tri::Default => {}
            Tri::Allow => own.push(name.to_string()),
            Tri::Deny => own.push(format!("!{name}")),
        }
        self.app.set_list(CONTEXT, key, &own);
    }

    /// Turn `name` explicitly on or off for this app.
    pub fn set(&mut self, key: &str, name: &str, on: bool) {
        self.set_state(key, name, if on { Tri::Allow } else { Tri::Deny });
    }

    /// Back to metadata + global: drop the app's own overrides.
    pub fn reset(&mut self) {
        self.app = KeyFile::default();
    }

    /// Filesystem entries that are not one of the standard toggles: `~/Games:rw`, `/mnt/sd`, ...
    pub fn custom_filesystems(&self) -> Vec<Entry> {
        let std: BTreeSet<&str> = SECTIONS
            .iter()
            .find(|s| s.key == "filesystems")
            .unwrap()
            .items
            .iter()
            .map(|(n, _)| *n)
            .collect();
        let own = self.app.list(CONTEXT, "filesystems");
        self.effective_list("filesystems")
            .into_iter()
            .filter(|e| !std.contains(fs_path(e)))
            .map(|e| Entry {
                overridden: own.iter().any(|o| o == &e),
                name: e,
                value: String::new(),
            })
            .collect()
    }

    pub fn add_filesystem(&mut self, entry: &str) {
        let entry = entry.trim();
        if entry.is_empty() {
            return;
        }
        let mut own = self.app.list(CONTEXT, "filesystems");
        let p = fs_path(entry).to_string();
        own.retain(|t| fs_path(t.strip_prefix('!').unwrap_or(t)) != p);
        own.push(entry.to_string());
        self.app.set_list(CONTEXT, "filesystems", &own);
    }

    /// Remove a custom path: drop our own entry, or revoke it when it comes from a lower layer.
    pub fn remove_filesystem(&mut self, entry: &str) {
        let p = fs_path(entry).to_string();
        let mut own = self.app.list(CONTEXT, "filesystems");
        own.retain(|t| fs_path(t.strip_prefix('!').unwrap_or(t)) != p);
        let still = {
            let mut v = self.base_list("filesystems");
            apply(&mut v, &own, "filesystems");
            v.iter().any(|e| fs_path(e) == p)
        };
        if still {
            own.push(format!("!{p}"));
        }
        self.app.set_list(CONTEXT, "filesystems", &own);
    }

    fn merged_pairs(&self, group: &str) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        for (kf, own) in [
            (&self.meta, false),
            (&self.global, false),
            (&self.app, true),
        ] {
            for (k, v) in kf.pairs(group) {
                match out.iter_mut().find(|e| e.name == k) {
                    Some(e) => {
                        e.value = v;
                        e.overridden = own;
                    }
                    None => out.push(Entry {
                        name: k,
                        value: v,
                        overridden: own,
                    }),
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn bus(&self, group: &str) -> Vec<Entry> {
        self.merged_pairs(group)
    }

    pub fn set_bus(&mut self, group: &str, name: &str, policy: &str) {
        if name.trim().is_empty() {
            return;
        }
        self.app.set(group, name.trim(), policy);
    }

    pub fn clear_bus(&mut self, group: &str, name: &str) {
        self.app.remove(group, name);
    }

    pub fn env(&self) -> Vec<Entry> {
        self.merged_pairs(ENV)
    }

    pub fn set_env(&mut self, name: &str, value: &str) {
        let n = name.trim();
        if n.is_empty() || n.contains('=') {
            return;
        }
        self.app.set(ENV, n, value);
    }

    pub fn clear_env(&mut self, name: &str) {
        self.app.remove(ENV, name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layers() -> Layers {
        Layers {
            meta: KeyFile::parse("[Context]\nshared=network;ipc;\nsockets=x11;wayland;pulseaudio;\nfilesystems=xdg-download;~/Games:rw;\n[Session Bus Policy]\norg.a.B=talk\n[Environment]\nLANG=C\n"),
            global: KeyFile::parse("[Context]\nsockets=!pulseaudio;ssh-auth;\n"),
            app: KeyFile::default(),
            is_global: false,
        }
    }

    #[test]
    fn global_toggles_are_always_explicit() {
        let mut g = Layers {
            is_global: true,
            ..Default::default()
        };
        g.set("sockets", "ssh-auth", false);
        assert_eq!(g.app.list(CONTEXT, "sockets"), ["!ssh-auth"]);
        g.set("sockets", "x11", true);
        assert_eq!(g.app.list(CONTEXT, "sockets"), ["!ssh-auth", "x11"]);
        g.set("sockets", "ssh-auth", true);
        assert_eq!(g.app.list(CONTEXT, "sockets"), ["x11", "ssh-auth"]);
    }

    #[test]
    fn layers_stack_in_order() {
        let l = layers();
        assert!(l.is_on("sockets", "x11"));
        assert!(
            !l.is_on("sockets", "pulseaudio"),
            "the global override revokes it"
        );
        assert!(l.is_on("sockets", "ssh-auth"));
        assert!(l.is_on("shared", "network"));
    }

    #[test]
    fn a_permission_has_three_states_and_the_explicit_ones_are_always_written() {
        let mut l = layers();
        assert_eq!(l.own_state("sockets", "x11"), Tri::Default);
        l.set("sockets", "x11", true); // already on, but asked for explicitly
        assert_eq!(l.app.list(CONTEXT, "sockets"), ["x11"]);
        assert_eq!(l.own_state("sockets", "x11"), Tri::Allow);
        l.set("shared", "network", false);
        assert_eq!(l.app.list(CONTEXT, "shared"), ["!network"]);
        assert_eq!(l.own_state("shared", "network"), Tri::Deny);
        assert!(!l.is_on("shared", "network"));
        assert!(l.is_overridden("shared", "network"));
        l.set_state("shared", "network", Tri::Default); // inherit again: the entry disappears
        assert!(l.app.list(CONTEXT, "shared").is_empty());
        assert!(l.is_on("shared", "network"));
        l.set("sockets", "pulseaudio", true); // the global override took it away, the app grants it back
        assert!(l.is_on("sockets", "pulseaudio"));
    }

    #[test]
    fn global_can_deny_explicitly_and_return_to_default() {
        let mut l = Layers {
            is_global: true,
            ..Layers::default()
        };
        l.set("shared", "network", false);
        assert_eq!(l.app.list(CONTEXT, "shared"), ["!network"]);
        l.set_state("shared", "network", Tri::Default);
        assert!(l.app.list(CONTEXT, "shared").is_empty());
    }

    #[test]
    fn filesystems_keep_their_mode_and_custom_paths_are_listed() {
        let mut l = layers();
        let c = l.custom_filesystems();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "~/Games:rw");
        l.add_filesystem("/mnt/sdcard:ro");
        assert!(l.is_on("filesystems", "/mnt/sdcard"));
        assert_eq!(l.custom_filesystems().len(), 2);
        l.remove_filesystem("/mnt/sdcard:ro");
        assert!(!l.is_on("filesystems", "/mnt/sdcard"));
        assert!(l.app.is_empty());
        l.remove_filesystem("~/Games:rw"); // comes from metadata: revoke
        assert_eq!(l.app.list(CONTEXT, "filesystems"), ["!~/Games"]);
        assert!(!l.is_on("filesystems", "~/Games"));
        l.set("filesystems", "home", true);
        assert!(l.is_on("filesystems", "home"));
    }

    #[test]
    fn bus_and_env_merge_with_the_apps_own_values_on_top() {
        let mut l = layers();
        assert_eq!(l.bus(SESSION_BUS)[0].value, "talk");
        l.set_bus(SESSION_BUS, "org.a.B", "none");
        l.set_bus(SESSION_BUS, "org.c.D", "own");
        let b = l.bus(SESSION_BUS);
        assert_eq!(
            b.iter().find(|e| e.name == "org.a.B").unwrap().value,
            "none"
        );
        assert!(b.iter().find(|e| e.name == "org.c.D").unwrap().overridden);
        l.clear_bus(SESSION_BUS, "org.a.B");
        assert_eq!(
            l.bus(SESSION_BUS)
                .iter()
                .find(|e| e.name == "org.a.B")
                .unwrap()
                .value,
            "talk"
        );
        l.set_env("FOO", "1");
        l.set_env("bad=name", "x");
        assert_eq!(l.env().len(), 2);
        l.clear_env("FOO");
        assert_eq!(l.env().len(), 1);
        l.reset();
        assert!(l.app.is_empty());
    }
}
