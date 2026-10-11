//! Talking to Flatpak: which apps exist, their metadata, and the override files on disk.
use crate::keyfile::KeyFile;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub installation: String,
}

/// `$FLATPAK_USER_DIR`, else `~/.local/share/flatpak`.
pub fn user_dir() -> PathBuf {
    if let Ok(d) = std::env::var("FLATPAK_USER_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("flatpak")
}

pub const GLOBAL: &str = "global";

/// An application id or table/object name that is safe to put in a path or a command: letters,
/// digits, `.`, `_`, `-`; it starts with a letter or digit (so never `.`, `..` or an option) and has no
/// `..` inside.
pub fn valid_id(id: &str) -> bool {
    id.len() < 256
        && id
            .chars()
            .next()
            .map(|c| c.is_ascii_alphanumeric())
            .unwrap_or(false)
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

pub fn override_path(id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err(format!("not an application id: '{id}'"));
    }
    let base = match std::env::var("FLATSEAL_OVERRIDES_DIR") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => user_dir().join("overrides"),
    };
    Ok(base.join(id))
}

pub fn load_override(id: &str) -> KeyFile {
    override_path(id)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|t| KeyFile::parse(&t))
        .unwrap_or_default()
}

/// Write the override (atomically), or delete the file when nothing is left in it.
pub fn save_override(id: &str, kf: &KeyFile) -> Result<(), String> {
    let path = override_path(id)?;
    let text = kf.to_text();
    if text.trim().is_empty() {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        };
    }
    let dir = path.parent().unwrap().to_path_buf();
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(format!(".{id}.tmp"));
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Parse `flatpak list --columns=application,name,installation` output (tab separated, no header).
pub fn parse_list(out: &str) -> Vec<App> {
    let mut v: Vec<App> = out
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let id = f.next()?.trim();
            if !valid_id(id) {
                return None;
            }
            Some(App {
                id: id.to_string(),
                name: f.next().unwrap_or(id).trim().to_string(),
                installation: f.next().unwrap_or("").trim().to_string(),
            })
        })
        .collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v.dedup_by(|a, b| a.id == b.id);
    v
}

pub fn list_apps() -> Result<Vec<App>, String> {
    let out = Command::new("flatpak")
        .args(["list", "--app", "--columns=application,name,installation"])
        .output()
        .map_err(|e| format!("flatpak: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "flatpak list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse_list(&String::from_utf8_lossy(&out.stdout)))
}

/// The app's metadata, read straight from the installation (spawning `flatpak info` costs 100+ ms
/// each time); `flatpak info` stays as the fallback for unusual installations. Cached per run.
pub fn metadata(id: &str) -> KeyFile {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, KeyFile>>> = OnceLock::new();
    if !valid_id(id) {
        return KeyFile::default();
    }
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(k) = cache.lock().ok().and_then(|c| c.get(id).cloned()) {
        return k;
    }
    let mut dirs = vec![user_dir()];
    if let Ok(d) = std::env::var("FLATPAK_SYSTEM_DIR") {
        dirs.push(PathBuf::from(d));
    }
    dirs.push(PathBuf::from("/var/lib/flatpak"));
    let kf = dirs
        .iter()
        .find_map(|d| {
            std::fs::read_to_string(d.join("app").join(id).join("current/active/metadata")).ok()
        })
        .map(|t| KeyFile::parse(&t))
        .unwrap_or_else(|| {
            Command::new("flatpak")
                .args(["info", "--show-metadata", id])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| KeyFile::parse(&String::from_utf8_lossy(&o.stdout)))
                .unwrap_or_default()
        });
    if let Ok(mut c) = cache.lock() {
        c.insert(id.to_string(), kf.clone());
    }
    kf
}

pub fn load_layers(id: &str) -> crate::perms::Layers {
    crate::perms::Layers {
        meta: if id == GLOBAL {
            KeyFile::default()
        } else {
            metadata(id)
        },
        global: if id == GLOBAL {
            KeyFile::default()
        } else {
            load_override(GLOBAL)
        },
        app: load_override(id),
        is_global: id == GLOBAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_output_is_parsed_sorted_and_filtered() {
        let out = "org.mozilla.firefox\tFirefox\tuser\ncom.valvesoftware.Steam\tSteam\tuser\nbad id\tx\tuser\n\n";
        let v = parse_list(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "Firefox");
        assert_eq!(v[1].id, "com.valvesoftware.Steam");
    }

    #[test]
    fn overrides_round_trip_and_an_empty_one_removes_the_file() {
        let dir = std::env::temp_dir().join(format!("flatseal-t-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("FLATPAK_USER_DIR", &dir);
        assert!(load_override("org.example.App").is_empty());
        let mut kf = KeyFile::default();
        kf.set_list("Context", "sockets", &["!x11".into(), "wayland".into()]);
        save_override("org.example.App", &kf).unwrap();
        let p = dir.join("overrides/org.example.App");
        assert!(p.is_file());
        assert_eq!(
            fs::read_to_string(&p).unwrap(),
            "[Context]\nsockets=!x11;wayland;\n"
        );
        assert_eq!(
            load_override("org.example.App").list("Context", "sockets"),
            ["!x11", "wayland"]
        );
        save_override("org.example.App", &KeyFile::default()).unwrap();
        assert!(!p.exists());
        assert!(
            save_override("../evil", &kf).is_err(),
            "ids that are paths are refused"
        );
        assert!(save_override("a/b", &kf).is_err());
    }
    #[test]
    fn ids_that_could_climb_or_act_as_options_are_refused() {
        for bad in [
            "", ".", "..", "../x", "a/b", ".hidden", "-rf", "a..b", "x y", "x;y", "a\0b", "é",
        ] {
            assert!(!valid_id(bad), "'{bad}' must not be valid");
        }
        for good in [
            "org.mozilla.firefox",
            "com.valvesoftware.Steam",
            "global",
            "io.github.X-Y_z",
        ] {
            assert!(valid_id(good), "'{good}'");
        }
        assert!(override_path("..").is_err());
        assert!(override_path("a/b").is_err());
    }
}
