//! After an update: remove the older versions of a kind of Proton, without breaking any game.
//!
//! * only the same kind (Proton-GE with Proton-GE, CachyOS with CachyOS...) and only versions that are
//!   clearly older: names without a version number (a rolling `proton-cachyos-slr`, a package's own
//!   directory) and symlinks are never touched;
//! * a game that was set to an old version is moved to the new one first (Steam must be closed for
//!   that); when that is not possible the old version stays and is reported;
//! * the newest one is never removed.
use crate::games;
use crate::install::{self, Installed};
use crate::sources::family_of;
use std::path::Path;

/// What happened, one line per old version, for the log and the window.
pub fn cleanup_old(dir: &Path, family: &str, newest_dir: &str, check_steam: bool) -> Vec<String> {
    let mut report = Vec::new();
    let installed = install::list_installed_with(dir, false);
    let Some(newest) = installed.iter().find(|i| i.name == newest_dir) else {
        return vec![format!(
            "{newest_dir} is not installed: nothing was removed"
        )];
    };
    let newest_key = install::version_key(&newest.name);
    let old: Vec<&Installed> = installed
        .iter()
        .filter(|i| i.name != newest.name)
        .filter(|i| family_of(&i.name) == Some(family))
        .filter(|i| {
            let k = install::version_key(&i.name);
            !k.is_empty() && k < newest_key && !i.path.is_symlink()
        })
        .collect();
    // partial downloads of older versions of this kind (left by an interrupted install) are dead weight
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(stem) = n.strip_prefix('.').filter(|_| n.ends_with(".part")) {
                let k = install::version_key(stem);
                if family_of(stem) == Some(family)
                    && !k.is_empty()
                    && k < newest_key
                    && e.path().is_file()
                    && std::fs::remove_file(e.path()).is_ok()
                {
                    report.push(format!("removed the partial download {n}"));
                }
            }
        }
    }
    if old.is_empty() {
        return report;
    }
    let root = dir.parent().map(Path::to_path_buf).unwrap_or_default();
    let mapping = match games::mapping_checked(&root) {
        Ok(m) => m,
        Err(e) => {
            return vec![format!(
                "kept the old versions: Steam's config cannot be read ({e})"
            )];
        }
    };
    for o in old {
        let users: Vec<u64> = mapping
            .iter()
            .filter(|(_, t)| **t == o.tool)
            .map(|(id, _)| *id)
            .collect();
        if !users.is_empty() {
            if check_steam && games::steam_running() {
                report.push(format!(
                    "kept {}: {} game(s) use it and Steam is running (close Steam, then update again)",
                    o.name,
                    users.len()
                ));
                continue;
            }
            let mut failed = None;
            for id in &users {
                if let Err(e) = games::set_tool_checked(&root, *id, Some(&newest.tool), check_steam)
                {
                    failed = Some(e);
                    break;
                }
            }
            if let Some(e) = failed {
                report.push(format!(
                    "kept {}: could not move its games to {} ({e})",
                    o.name, newest.name
                ));
                continue;
            }
        }
        match install::remove(dir, &o.name) {
            Ok(()) => report.push(format!(
                "removed {}{}",
                o.name,
                if users.is_empty() {
                    String::new()
                } else {
                    format!(" ({} game(s) now use {})", users.len(), newest.name)
                }
            )),
            Err(e) => report.push(format!("kept {}: {e}", o.name)),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn steam_root(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("upd-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("compatibilitytools.d")).unwrap();
        fs::create_dir_all(p.join("config")).unwrap();
        fs::write(
            p.join("config/config.vdf"),
            "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t}\n\t\t}\n\t}\n}\n",
        )
        .unwrap();
        p
    }

    fn tool(root: &Path, name: &str) {
        let d = root.join("compatibilitytools.d").join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("compatibilitytool.vdf"),
            format!("\"compatibilitytools\"\n{{\n\t\"compat_tools\"\n\t{{\n\t\t\"{name}\"\n\t\t{{\n\t\t\t\"from_oslist\"\t\"windows\"\n\t\t}}\n\t}}\n}}\n"),
        )
        .unwrap();
    }

    #[test]
    fn older_versions_of_the_same_kind_go_and_everything_else_stays() {
        let r = steam_root("basic");
        for n in [
            "GE-Proton9-27",
            "GE-Proton10-19",
            "GE-Proton10-20",
            "proton-cachyos-10.0-20260920-slr-x86_64",
            "proton-cachyos-slr",
        ] {
            tool(&r, n);
        }
        let dir = r.join("compatibilitytools.d");
        let rep = cleanup_old(&dir, "ge", "GE-Proton10-20", false);
        assert_eq!(rep.len(), 2, "{rep:?}");
        assert!(dir.join("GE-Proton10-20").is_dir(), "the newest stays");
        assert!(!dir.join("GE-Proton9-27").exists() && !dir.join("GE-Proton10-19").exists());
        assert!(
            dir.join("proton-cachyos-10.0-20260920-slr-x86_64").is_dir(),
            "another kind is untouched"
        );
        assert!(
            dir.join("proton-cachyos-slr").is_dir(),
            "an unversioned directory is untouched"
        );
    }

    #[test]
    fn a_game_on_an_old_version_is_moved_to_the_new_one_before_it_is_removed() {
        let r = steam_root("move");
        tool(&r, "GE-Proton10-19");
        tool(&r, "GE-Proton10-20");
        games::set_tool_checked(&r, 570, Some("GE-Proton10-19"), false).unwrap();
        games::set_tool_checked(&r, 0, Some("GE-Proton10-19"), false).unwrap();
        let dir = r.join("compatibilitytools.d");
        let rep = cleanup_old(&dir, "ge", "GE-Proton10-20", false);
        assert!(rep[0].starts_with("removed GE-Proton10-19"), "{rep:?}");
        let m = games::mapping(&r);
        assert_eq!(m.get(&570).map(String::as_str), Some("GE-Proton10-20"));
        assert_eq!(m.get(&0).map(String::as_str), Some("GE-Proton10-20"));
        assert!(!dir.join("GE-Proton10-19").exists());
    }

    #[test]
    fn an_old_version_stays_when_its_games_cannot_be_moved() {
        let r = steam_root("stay");
        tool(&r, "GE-Proton10-19");
        tool(&r, "GE-Proton10-20");
        games::set_tool_checked(&r, 570, Some("GE-Proton10-19"), false).unwrap();
        // break the config so the move fails
        fs::write(r.join("config/config.vdf"), "\"InstallConfigStore\" {").unwrap();
        let dir = r.join("compatibilitytools.d");
        let rep = cleanup_old(&dir, "ge", "GE-Proton10-20", false);
        assert!(rep[0].starts_with("kept"), "{rep:?}");
        assert!(dir.join("GE-Proton10-19").is_dir());
    }

    #[test]
    fn partial_downloads_of_older_versions_are_cleared_but_not_newer_or_other_ones() {
        let r = steam_root("part");
        tool(&r, "GE-Proton10-20");
        let dir = r.join("compatibilitytools.d");
        fs::write(dir.join(".GE-Proton10-5-x86_64.tar.gz.part"), "x").unwrap();
        fs::write(dir.join(".GE-Proton11-1-x86_64.tar.gz.part"), "x").unwrap();
        fs::write(dir.join(".proton-cachyos-9.0-x.tar.xz.part"), "x").unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        let rep = cleanup_old(&dir, "ge", "GE-Proton10-20", false);
        assert_eq!(rep.len(), 1, "{rep:?}");
        assert!(!dir.join(".GE-Proton10-5-x86_64.tar.gz.part").exists());
        assert!(
            dir.join(".GE-Proton11-1-x86_64.tar.gz.part").exists(),
            "a newer one may be in progress"
        );
        assert!(dir.join(".proton-cachyos-9.0-x.tar.xz.part").exists());
        assert!(dir.join("notes.txt").exists());
    }

    #[test]
    fn nothing_is_removed_when_the_newest_is_not_there() {
        let r = steam_root("nonew");
        tool(&r, "GE-Proton10-19");
        let dir = r.join("compatibilitytools.d");
        let rep = cleanup_old(&dir, "ge", "GE-Proton10-20", false);
        assert!(rep[0].contains("nothing was removed"));
        assert!(dir.join("GE-Proton10-19").is_dir());
    }

    #[test]
    fn a_symlinked_version_is_never_removed() {
        let r = steam_root("link");
        tool(&r, "GE-Proton10-20");
        let real = r.join("elsewhere/GE-Proton9-1");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("compatibilitytool.vdf"), "x").unwrap();
        std::os::unix::fs::symlink(&real, r.join("compatibilitytools.d/GE-Proton9-1")).unwrap();
        let dir = r.join("compatibilitytools.d");
        let rep = cleanup_old(&dir, "ge", "GE-Proton10-20", false);
        assert!(rep.is_empty(), "{rep:?}");
        assert!(real.join("compatibilitytool.vdf").is_file());
    }
}
