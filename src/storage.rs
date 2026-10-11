//! Game storage: the Proton prefix (`compatdata/<appid>`) and the shader cache (`shadercache/<appid>`)
//! of every game, how big they are, and three careful actions:
//!
//! * reset a prefix: the folder is MOVED aside (`<appid>.bak-<time>`), never deleted, so saves that
//!   Steam Cloud does not hold can be copied back;
//! * clear a shader cache: it is rebuilt by the game, nothing of the user's is in it;
//! * delete leftovers of games that are no longer installed: only when every Steam library is
//!   present (a game on an unmounted SD card would look uninstalled) and never for non-Steam
//!   shortcuts, whose prefix cannot be recreated by Steam.
use crate::games;
use std::fs;
use std::path::{Path, PathBuf};

/// Steam gives non-Steam shortcuts ids with the top bit set; Steam's own app ids are below it.
const SHORTCUT_MIN: u64 = 1 << 31;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub appid: u64,
    /// the game's name when it is installed
    pub name: Option<String>,
    /// `<library>/steamapps`
    pub steamapps: PathBuf,
    pub prefix_bytes: u64,
    pub shader_bytes: u64,
    pub has_prefix: bool,
    pub has_shader: bool,
}

impl Entry {
    pub fn is_shortcut(&self) -> bool {
        self.appid >= SHORTCUT_MIN
    }

    pub fn installed(&self) -> bool {
        self.name.is_some()
    }
}

fn dir_size(p: &Path) -> u64 {
    let mut n = 0;
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let Ok(md) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if md.is_dir() {
                n += dir_size(&e.path());
            } else {
                n += md.len();
            }
        }
    }
    n
}

/// Libraries named in libraryfolders.vdf that are not there right now (an SD card that is out).
pub fn missing_libraries(root: &Path) -> Vec<PathBuf> {
    games::libraries(root)
        .into_iter()
        .filter(|l| !l.join("steamapps").is_dir())
        .collect()
}

/// Every prefix and shader cache under the libraries of `root`, with names from the manifests.
pub fn scan(root: &Path, sizes: bool) -> Vec<Entry> {
    let names: std::collections::HashMap<u64, String> = games::games(root)
        .into_iter()
        .map(|g| (g.appid, g.name))
        .collect();
    let mut out: Vec<Entry> = Vec::new();
    for lib in games::libraries(root) {
        let sa = lib.join("steamapps");
        let mut ids: Vec<u64> = Vec::new();
        for sub in ["compatdata", "shadercache"] {
            if let Ok(rd) = fs::read_dir(sa.join(sub)) {
                for e in rd.flatten() {
                    if let Ok(id) = e.file_name().to_string_lossy().parse::<u64>() {
                        if e.path().is_dir() && !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                }
            }
        }
        for id in ids {
            let p = sa.join("compatdata").join(id.to_string());
            let s = sa.join("shadercache").join(id.to_string());
            out.push(Entry {
                appid: id,
                name: names.get(&id).cloned(),
                steamapps: sa.clone(),
                prefix_bytes: if sizes { dir_size(&p) } else { 0 },
                shader_bytes: if sizes { dir_size(&s) } else { 0 },
                has_prefix: p.is_dir(),
                has_shader: s.is_dir(),
            });
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.prefix_bytes + e.shader_bytes));
    out
}

fn check(steamapps: &Path, appid: u64, check_steam: bool) -> Result<(), String> {
    if appid == 0 {
        return Err("no app id".into());
    }
    if steamapps
        .file_name()
        .map(|n| n != "steamapps")
        .unwrap_or(true)
    {
        return Err(format!("{} is not a steamapps folder", steamapps.display()));
    }
    if check_steam && games::steam_running() {
        return Err(
            "Steam is running (a game may be using it): close Steam completely first".into(),
        );
    }
    Ok(())
}

fn stamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

/// Move the prefix aside. Returns where it went.
pub fn reset_prefix(steamapps: &Path, appid: u64, check_steam: bool) -> Result<PathBuf, String> {
    check(steamapps, appid, check_steam)?;
    let from = steamapps.join("compatdata").join(appid.to_string());
    if !from.is_dir() {
        return Err(format!("{} does not exist", from.display()));
    }
    let to = steamapps
        .join("compatdata")
        .join(format!("{appid}.bak-{}", stamp()));
    if to.exists() {
        return Err(format!("{} already exists", to.display()));
    }
    fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
    Ok(to)
}

pub fn clear_shader(steamapps: &Path, appid: u64, check_steam: bool) -> Result<u64, String> {
    check(steamapps, appid, check_steam)?;
    let d = steamapps.join("shadercache").join(appid.to_string());
    if !d.is_dir() {
        return Err(format!("{} does not exist", d.display()));
    }
    let n = dir_size(&d);
    fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    Ok(n)
}

/// Delete the prefix and the shader cache of a game that is not installed any more.
pub fn delete_leftovers(root: &Path, e: &Entry, check_steam: bool) -> Result<u64, String> {
    check(&e.steamapps, e.appid, check_steam)?;
    if e.installed() {
        return Err("the game is installed".into());
    }
    if e.is_shortcut() {
        return Err(
            "this is a non-Steam shortcut: its prefix cannot be recreated, so it is kept".into(),
        );
    }
    let missing = missing_libraries(root);
    if !missing.is_empty() {
        return Err(format!(
            "a Steam library is not available ({}): the game may be on it, so nothing is deleted",
            missing[0].display()
        ));
    }
    // look again right now: the entry may be old
    if games::games(root).iter().any(|g| g.appid == e.appid) {
        return Err("the game is installed".into());
    }
    let mut freed = 0;
    for sub in ["compatdata", "shadercache"] {
        let d = e.steamapps.join(sub).join(e.appid.to_string());
        if d.is_dir() {
            freed += dir_size(&d);
            fs::remove_dir_all(&d).map_err(|er| format!("{}: {er}", d.display()))?;
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("stor-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("steamapps")).unwrap();
        p
    }

    fn game(root: &Path, id: u64, name: &str) {
        fs::write(
            root.join(format!("steamapps/appmanifest_{id}.acf")),
            format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n}}\n"),
        )
        .unwrap();
    }

    fn prefix(root: &Path, id: u64, file: &str) {
        let d = root
            .join("steamapps/compatdata")
            .join(id.to_string())
            .join("pfx");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(file), "save data").unwrap();
    }

    fn shader(root: &Path, id: u64) {
        let d = root.join("steamapps/shadercache").join(id.to_string());
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("cache"), vec![0u8; 1000]).unwrap();
    }

    #[test]
    fn the_scan_names_installed_games_and_sizes_the_rest() {
        let r = lib("scan");
        game(&r, 570, "Dota 2");
        prefix(&r, 570, "a");
        shader(&r, 570);
        prefix(&r, 999, "b");
        let v = scan(&r, true);
        let dota = v.iter().find(|e| e.appid == 570).unwrap();
        assert_eq!(dota.name.as_deref(), Some("Dota 2"));
        assert!(dota.shader_bytes >= 1000 && dota.prefix_bytes > 0);
        assert!(!v.iter().find(|e| e.appid == 999).unwrap().installed());
    }

    #[test]
    fn a_reset_moves_the_prefix_aside_and_loses_nothing() {
        let r = lib("reset");
        prefix(&r, 570, "save.dat");
        let sa = r.join("steamapps");
        let to = reset_prefix(&sa, 570, false).unwrap();
        assert!(!sa.join("compatdata/570").exists());
        assert_eq!(
            fs::read_to_string(to.join("pfx/save.dat")).unwrap(),
            "save data"
        );
        assert!(
            reset_prefix(&sa, 570, false).is_err(),
            "nothing left to reset"
        );
    }

    #[test]
    fn clearing_the_shader_cache_touches_only_that_games_cache() {
        let r = lib("shader");
        shader(&r, 1);
        shader(&r, 2);
        prefix(&r, 1, "keep");
        let sa = r.join("steamapps");
        assert!(clear_shader(&sa, 1, false).unwrap() >= 1000);
        assert!(!sa.join("shadercache/1").exists());
        assert!(sa.join("shadercache/2/cache").is_file());
        assert!(
            sa.join("compatdata/1/pfx/keep").is_file(),
            "the prefix is not the shader cache"
        );
    }

    #[test]
    fn odd_arguments_are_refused() {
        let r = lib("odd");
        prefix(&r, 5, "x");
        assert!(reset_prefix(&r.join("steamapps"), 0, false).is_err());
        assert!(
            reset_prefix(&r.join("steamapps/compatdata"), 5, false).is_err(),
            "not a steamapps folder"
        );
        assert!(r.join("steamapps/compatdata/5/pfx/x").is_file());
    }

    #[test]
    fn leftovers_are_deleted_only_for_games_that_are_really_gone() {
        let r = lib("left");
        game(&r, 570, "Dota 2");
        prefix(&r, 570, "keep");
        prefix(&r, 999, "old");
        shader(&r, 999);
        let v = scan(&r, false);
        let installed = v.iter().find(|e| e.appid == 570).unwrap();
        let gone = v.iter().find(|e| e.appid == 999).unwrap();
        assert!(delete_leftovers(&r, installed, false).is_err());
        assert!(r.join("steamapps/compatdata/570/pfx/keep").is_file());
        let freed = delete_leftovers(&r, gone, false).unwrap();
        assert!(freed > 0);
        assert!(!r.join("steamapps/compatdata/999").exists());
        assert!(!r.join("steamapps/shadercache/999").exists());
        assert!(r.join("steamapps/compatdata/570").is_dir());
    }

    #[test]
    fn non_steam_shortcuts_are_never_deleted() {
        let r = lib("shortcut");
        let id = 3_000_000_000u64;
        prefix(&r, id, "precious");
        let e = scan(&r, false).into_iter().find(|e| e.appid == id).unwrap();
        assert!(e.is_shortcut());
        assert!(delete_leftovers(&r, &e, false)
            .unwrap_err()
            .contains("non-Steam"));
        assert!(r
            .join(format!("steamapps/compatdata/{id}/pfx/precious"))
            .is_file());
    }

    #[test]
    fn nothing_is_deleted_while_a_library_is_missing() {
        let r = lib("unmounted");
        let gone_card = std::env::temp_dir().join(format!("stor-card-{}", std::process::id()));
        let _ = fs::remove_dir_all(&gone_card); // the card is out
        fs::write(
            r.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                gone_card.display()
            ),
        )
        .unwrap();
        // the game lives on the card, its prefix on the main drive: it looks uninstalled
        prefix(&r, 4242, "saves of a game on the card");
        let e = scan(&r, false)
            .into_iter()
            .find(|e| e.appid == 4242)
            .unwrap();
        assert!(!e.installed());
        assert_eq!(missing_libraries(&r), vec![gone_card]);
        let err = delete_leftovers(&r, &e, false).unwrap_err();
        assert!(err.contains("not available"), "{err}");
        assert!(r.join("steamapps/compatdata/4242/pfx").is_dir());
    }
}
