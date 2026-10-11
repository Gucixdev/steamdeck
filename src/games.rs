//! Per-game compatibility tools: which game runs with which Proton. Steam keeps the choice in
//! `config/config.vdf` under `CompatToolMapping`, so this reads and rewrites that Valve KeyValues file.
//! Steam rewrites config.vdf when it exits, so a change is only made while Steam is closed.
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A KeyValues node: a string or a list of named children (order and duplicates are kept).
#[derive(Debug, Clone, PartialEq)]
pub enum V {
    Str(String),
    Obj(Vec<(String, V)>),
}

fn tokens(src: &str) -> Result<Vec<Tok>, String> {
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        match b[i] {
            c if c.is_whitespace() => i += 1,
            '/' if b.get(i + 1) == Some(&'/') => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '{' => {
                out.push(Tok::Open);
                i += 1;
            }
            '}' => {
                out.push(Tok::Close);
                i += 1;
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                loop {
                    let Some(&c) = b.get(i) else {
                        return Err("unterminated string".into());
                    };
                    i += 1;
                    match c {
                        '"' => break,
                        '\\' => {
                            let Some(&n) = b.get(i) else {
                                return Err("unterminated string".into());
                            };
                            i += 1;
                            s.push(match n {
                                'n' => '\n',
                                't' => '\t',
                                other => other,
                            });
                        }
                        c => s.push(c),
                    }
                }
                out.push(Tok::Str(s));
            }
            _ => {
                let mut s = String::new();
                while i < b.len() && !b[i].is_whitespace() && !"{}\"".contains(b[i]) {
                    s.push(b[i]);
                    i += 1;
                }
                out.push(Tok::Str(s));
            }
        }
    }
    Ok(out)
}

enum Tok {
    Str(String),
    Open,
    Close,
}

fn block(t: &[Tok], i: &mut usize, top: bool) -> Result<Vec<(String, V)>, String> {
    let mut out = Vec::new();
    loop {
        match t.get(*i) {
            None if top => return Ok(out),
            None => return Err("missing }".into()),
            Some(Tok::Close) if !top => {
                *i += 1;
                return Ok(out);
            }
            Some(Tok::Close) => return Err("unexpected }".into()),
            Some(Tok::Open) => return Err("unexpected {".into()),
            Some(Tok::Str(k)) => {
                *i += 1;
                match t.get(*i) {
                    Some(Tok::Str(v)) => {
                        *i += 1;
                        out.push((k.clone(), V::Str(v.clone())));
                    }
                    Some(Tok::Open) => {
                        *i += 1;
                        let o = block(t, i, false)?;
                        out.push((k.clone(), V::Obj(o)));
                    }
                    _ => return Err(format!("no value for \"{k}\"")),
                }
            }
        }
    }
}

pub fn parse(src: &str) -> Result<Vec<(String, V)>, String> {
    let t = tokens(src)?;
    let mut i = 0;
    block(&t, &mut i, true)
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write(out: &mut String, items: &[(String, V)], depth: usize) {
    let ind = "\t".repeat(depth);
    for (k, v) in items {
        match v {
            V::Str(s) => out.push_str(&format!("{ind}\"{}\"\t\t\"{}\"\n", esc(k), esc(s))),
            V::Obj(o) => {
                out.push_str(&format!("{ind}\"{}\"\n{ind}{{\n", esc(k)));
                write(out, o, depth + 1);
                out.push_str(&format!("{ind}}}\n"));
            }
        }
    }
}

pub fn serialize(items: &[(String, V)]) -> String {
    let mut s = String::new();
    write(&mut s, items, 0);
    s
}

fn child<'a>(items: &'a [(String, V)], key: &str) -> Option<&'a V> {
    items
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn child_mut<'a>(items: &'a mut Vec<(String, V)>, key: &str) -> &'a mut Vec<(String, V)> {
    let i = match items
        .iter()
        .position(|(k, v)| k.eq_ignore_ascii_case(key) && matches!(v, V::Obj(_)))
    {
        Some(i) => i,
        None => {
            items.push((key.to_string(), V::Obj(Vec::new())));
            items.len() - 1
        }
    };
    match &mut items[i].1 {
        V::Obj(o) => o,
        V::Str(_) => unreachable!(),
    }
}

fn text(items: &[(String, V)], key: &str) -> Option<String> {
    match child(items, key) {
        Some(V::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub appid: u64,
    pub name: String,
}

/// The Steam root itself plus every library folder listed in libraryfolders.vdf.
pub fn libraries(root: &Path) -> Vec<PathBuf> {
    // the same folder can be named twice (a symlinked ~/.local/share/Steam and the path in
    // libraryfolders.vdf), so compare real paths and keep one
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let mut out = vec![root.to_path_buf()];
    let mut seen = vec![real(root)];
    if let Ok(t) = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        if let Ok(items) = parse(&t) {
            if let Some(V::Obj(lf)) = child(&items, "libraryfolders") {
                for (_, v) in lf {
                    if let V::Obj(o) = v {
                        if let Some(p) = text(o, "path") {
                            let p = PathBuf::from(p);
                            let r = real(&p);
                            if !seen.contains(&r) {
                                seen.push(r);
                                out.push(p);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn is_tool(name: &str) -> bool {
    name.starts_with("Proton ")
        || name.starts_with("Steam Linux Runtime")
        || name.starts_with("Steamworks Common")
}

/// Installed games, by name. Proton versions and Steam runtimes are not games and are left out.
pub fn games(root: &Path) -> Vec<Game> {
    let mut out: Vec<Game> = Vec::new();
    for lib in libraries(root) {
        let Ok(rd) = fs::read_dir(lib.join("steamapps")) else {
            continue;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !(n.starts_with("appmanifest_") && n.ends_with(".acf")) {
                continue;
            }
            let Ok(t) = fs::read_to_string(e.path()) else {
                continue;
            };
            let Ok(items) = parse(&t) else { continue };
            let Some(V::Obj(st)) = child(&items, "AppState") else {
                continue;
            };
            let (Some(id), Some(name)) = (
                text(st, "appid").and_then(|s| s.parse().ok()),
                text(st, "name"),
            ) else {
                continue;
            };
            if !is_tool(&name) && !out.iter().any(|g| g.appid == id) {
                out.push(Game { appid: id, name });
            }
        }
    }
    out.sort_by_key(|g| g.name.to_lowercase());
    out
}

pub fn config_path(root: &Path) -> PathBuf {
    root.join("config/config.vdf")
}

fn mapping_of(items: &[(String, V)]) -> Option<&[(String, V)]> {
    let mut cur = items;
    for k in [
        "InstallConfigStore",
        "Software",
        "Valve",
        "Steam",
        "CompatToolMapping",
    ] {
        match child(cur, k) {
            Some(V::Obj(o)) => cur = o,
            _ => return None,
        }
    }
    Some(cur)
}

/// appid -> tool name Steam is told to use. appid 0 is the default for every game.
pub fn mapping(root: &Path) -> HashMap<u64, String> {
    let mut out = HashMap::new();
    let Ok(t) = fs::read_to_string(config_path(root)) else {
        return out;
    };
    let Ok(items) = parse(&t) else { return out };
    if let Some(m) = mapping_of(&items) {
        for (k, v) in m {
            if let (Ok(id), V::Obj(o)) = (k.parse::<u64>(), v) {
                if let Some(n) = text(o, "name").filter(|n| !n.is_empty()) {
                    out.insert(id, n);
                }
            }
        }
    }
    out
}

/// Like `mapping`, but a config that exists and cannot be read is an error, so that callers who
/// decide something from "nobody uses this tool" do not take a broken file for an empty one.
pub fn mapping_checked(root: &Path) -> Result<HashMap<u64, String>, String> {
    let p = config_path(root);
    match fs::read_to_string(&p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(format!("{}: {e}", p.display())),
        Ok(t) => {
            parse(&t).map_err(|e| format!("{}: {e}", p.display()))?;
            Ok(mapping(root))
        }
    }
}

/// True when a Steam client process is running (it would overwrite config.vdf on exit).
pub fn steam_running() -> bool {
    let Ok(rd) = fs::read_dir("/proc") else {
        return false;
    };
    rd.flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_digit())
            && fs::read_to_string(e.path().join("comm"))
                .map(|c| c.trim() == "steam")
                .unwrap_or(false)
    })
}

/// The name Steam knows a tool by: the key under `compat_tools` in its compatibilitytool.vdf (usually
/// the directory name, not always).
pub fn internal_name(tool_dir: &Path) -> Option<String> {
    let t = fs::read_to_string(tool_dir.join("compatibilitytool.vdf")).ok()?;
    let items = parse(&t).ok()?;
    let Some(V::Obj(ct)) = child(&items, "compatibilitytools") else {
        return None;
    };
    let Some(V::Obj(tools)) = child(ct, "compat_tools") else {
        return None;
    };
    tools.first().map(|(k, _)| k.clone())
}

/// False for Steam's own Linux runtimes (from_oslist "linux", e.g. LegacyRuntime): they are not a
/// Proton and are not offered as one. A tool that does not say is taken to be a Proton.
pub fn runs_windows_games(tool_dir: &Path) -> bool {
    let Ok(t) = fs::read_to_string(tool_dir.join("compatibilitytool.vdf")) else {
        return true;
    };
    let Ok(items) = parse(&t) else { return true };
    let Some(V::Obj(ct)) = child(&items, "compatibilitytools") else {
        return true;
    };
    let Some(V::Obj(tools)) = child(ct, "compat_tools") else {
        return true;
    };
    match tools.first() {
        Some((_, V::Obj(o))) => text(o, "from_oslist").map(|v| v != "linux").unwrap_or(true),
        _ => true,
    }
}

/// `userdata/<id>/config/localconfig.vdf` of every Steam account that logged in here.
pub fn localconfig_paths(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(root.join("userdata")) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path().join("config/localconfig.vdf"))
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v
}

fn apps_node(items: &[(String, V)]) -> Option<&[(String, V)]> {
    let mut cur = items;
    for k in ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"] {
        match child(cur, k) {
            Some(V::Obj(o)) => cur = o,
            _ => return None,
        }
    }
    Some(cur)
}

/// appid -> the launch options string set in Steam (first account that has any).
pub fn launch_options(root: &Path) -> HashMap<u64, String> {
    let mut out = HashMap::new();
    for p in localconfig_paths(root) {
        let Ok(t) = fs::read_to_string(&p) else {
            continue;
        };
        let Ok(items) = parse(&t) else { continue };
        if let Some(apps) = apps_node(&items) {
            for (k, v) in apps {
                if let (Ok(id), V::Obj(o)) = (k.parse::<u64>(), v) {
                    if let Some(l) = text(o, "LaunchOptions").filter(|l| !l.is_empty()) {
                        out.entry(id).or_insert(l);
                    }
                }
            }
        }
    }
    out
}

/// Set (or clear with `None`) the launch options of a game in every account's localconfig.vdf.
pub fn set_launch_options(root: &Path, appid: u64, opts: Option<&str>) -> Result<(), String> {
    set_launch_options_checked(root, appid, opts, true)
}

pub fn set_launch_options_checked(
    root: &Path,
    appid: u64,
    opts: Option<&str>,
    check_steam: bool,
) -> Result<(), String> {
    if check_steam && steam_running() {
        return Err(
            "Steam is running and would undo the change: close Steam completely first".into(),
        );
    }
    let paths = localconfig_paths(root);
    if paths.is_empty() {
        return Err("no Steam account has logged in here yet (no localconfig.vdf)".into());
    }
    // two phases: every file is parsed, changed and verified first, so a broken second account can
    // not leave the first one changed and the second not
    let mut staged: Vec<(PathBuf, String)> = Vec::new();
    for path in paths {
        let t = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut items =
            parse(&t).map_err(|e| format!("{}: {e}; left untouched", path.display()))?;
        {
            let a = child_mut(&mut items, "UserLocalConfigStore");
            let a = child_mut(a, "Software");
            let a = child_mut(a, "Valve");
            let a = child_mut(a, "Steam");
            let a = child_mut(a, "apps");
            let app = child_mut(a, &appid.to_string());
            app.retain(|(k, _)| !k.eq_ignore_ascii_case("LaunchOptions"));
            if let Some(o) = opts {
                app.push(("LaunchOptions".into(), V::Str(o.into())));
            }
        }
        let out = serialize(&items);
        // verify before replacing: what we wrote must parse back to the same tree
        if parse(&out).map(|x| x != items).unwrap_or(true) {
            return Err("internal check failed, nothing written".into());
        }
        staged.push((path, out));
    }
    for (path, out) in staged {
        let _ = fs::copy(&path, path.with_extension("vdf.steamdeck-bak"));
        let tmp = path.with_extension("vdf.steamdeck-tmp");
        fs::write(&tmp, out).map_err(|e| format!("{}: {e}", tmp.display()))?;
        fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Valve's own Protons (Proton 9.x, Experimental, Hotfix...) are ordinary Steam games in
/// steamapps/common; the name Steam knows them by is in their compatibilitytool.vdf.
pub fn valve_tools(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for lib in libraries(root) {
        let Ok(rd) = fs::read_dir(lib.join("steamapps/common")) else {
            continue;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.starts_with("Proton") {
                continue;
            }
            if let Some(t) = internal_name(&e.path()) {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out.sort();
    out
}

/// Use `tool` for `appid` (or the Steam default again with `None`). Writes config.vdf atomically and
/// keeps a copy of the previous one next to it.
pub fn set_tool(root: &Path, appid: u64, tool: Option<&str>) -> Result<(), String> {
    set_tool_checked(root, appid, tool, true)
}

pub fn set_tool_checked(
    root: &Path,
    appid: u64,
    tool: Option<&str>,
    check_steam: bool,
) -> Result<(), String> {
    if check_steam && steam_running() {
        return Err(
            "Steam is running and would undo the change: close Steam completely first".into(),
        );
    }
    let path = config_path(root);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut items = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    {
        let steam = {
            let a = child_mut(&mut items, "InstallConfigStore");
            let a = child_mut(a, "Software");
            let a = child_mut(a, "Valve");
            child_mut(a, "Steam")
        };
        let map = child_mut(steam, "CompatToolMapping");
        let key = appid.to_string();
        map.retain(|(k, _)| k != &key);
        if let Some(t) = tool {
            map.push((
                key,
                V::Obj(vec![
                    ("name".into(), V::Str(t.into())),
                    ("config".into(), V::Str(String::new())),
                    ("priority".into(), V::Str("250".into())),
                ]),
            ));
        }
    }
    let backup = path.with_extension("vdf.steamdeck-bak");
    let _ = fs::copy(&path, &backup);
    let tmp = path.with_extension("vdf.steamdeck-tmp");
    fs::write(&tmp, serialize(&items)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"path\"\t\t\"C:\\\\x\"\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"570\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"proton_experimental\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"250\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("games-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("config")).unwrap();
        p
    }

    #[test]
    fn vdf_round_trips_byte_for_byte() {
        let t = parse(CONFIG).unwrap();
        assert_eq!(serialize(&t), CONFIG);
    }

    #[test]
    fn mapping_is_read_set_changed_and_removed() {
        let root = tmp("map");
        fs::write(config_path(&root), CONFIG).unwrap();
        assert_eq!(
            mapping(&root).get(&570).map(String::as_str),
            Some("proton_experimental")
        );
        set_tool_checked(&root, 1245620, Some("GE-Proton10-20"), false).unwrap();
        set_tool_checked(&root, 570, Some("GE-Proton9-27"), false).unwrap();
        let m = mapping(&root);
        assert_eq!(m.get(&1245620).map(String::as_str), Some("GE-Proton10-20"));
        assert_eq!(m.get(&570).map(String::as_str), Some("GE-Proton9-27"));
        set_tool_checked(&root, 570, None, false).unwrap();
        assert!(!mapping(&root).contains_key(&570));
        // unrelated content survives and a backup of the previous file exists
        assert!(fs::read_to_string(config_path(&root))
            .unwrap()
            .contains("C:\\\\x"));
        assert!(root.join("config/config.vdf.steamdeck-bak").is_file());
    }

    #[test]
    fn a_config_without_a_mapping_gets_one() {
        let root = tmp("fresh");
        fs::write(config_path(&root), "\"InstallConfigStore\"\n{\n}\n").unwrap();
        set_tool_checked(&root, 0, Some("proton_9"), false).unwrap();
        assert_eq!(mapping(&root).get(&0).map(String::as_str), Some("proton_9"));
    }

    #[test]
    fn games_come_from_every_library_without_the_tools() {
        let root = tmp("lib");
        let lib2 = tmp("lib2");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::create_dir_all(lib2.join("steamapps")).unwrap();
        fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                lib2.display()
            ),
        )
        .unwrap();
        let acf = |id: u64, n: &str| {
            format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{n}\"\n}}\n")
        };
        fs::write(
            root.join("steamapps/appmanifest_570.acf"),
            acf(570, "Dota 2"),
        )
        .unwrap();
        fs::write(
            root.join("steamapps/appmanifest_1493710.acf"),
            acf(1493710, "Proton Experimental"),
        )
        .unwrap();
        fs::write(lib2.join("steamapps/appmanifest_1.acf"), acf(1, "Aaa Game")).unwrap();
        let g = games(&root);
        assert_eq!(
            g.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            ["Aaa Game", "Dota 2"]
        );
    }

    #[test]
    fn launch_options_are_set_read_and_cleared_in_localconfig() {
        let root = tmp("launch");
        let cfg = root.join("userdata/1234/config");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(
            cfg.join("localconfig.vdf"),
            "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"570\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"old %command%\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n",
        )
        .unwrap();
        assert_eq!(
            launch_options(&root).get(&570).map(String::as_str),
            Some("old %command%")
        );
        set_launch_options_checked(&root, 570, Some("gamemoderun %command%"), false).unwrap();
        set_launch_options_checked(&root, 4800, Some("mangohud %command%"), false).unwrap();
        let m = launch_options(&root);
        assert_eq!(
            m.get(&570).map(String::as_str),
            Some("gamemoderun %command%")
        );
        assert_eq!(m.get(&4800).map(String::as_str), Some("mangohud %command%"));
        set_launch_options_checked(&root, 570, None, false).unwrap();
        assert!(!launch_options(&root).contains_key(&570));
        assert!(cfg.join("localconfig.vdf.steamdeck-bak").is_file());
    }

    #[test]
    fn a_library_named_twice_through_a_symlink_is_listed_once() {
        let root = tmp("dup");
        let link = std::env::temp_dir().join(format!("games-duplink-{}", std::process::id()));
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&root, &link).unwrap();
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                link.display()
            ),
        )
        .unwrap();
        assert_eq!(libraries(&root).len(), 1);
        let _ = fs::remove_file(&link);
    }

    #[test]
    fn valve_protons_are_found_in_steamapps_common() {
        let root = tmp("valve");
        let d = root.join("steamapps/common/Proton Experimental");
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("compatibilitytool.vdf"),
            "\"compatibilitytools\"\n{\n\t\"compat_tools\"\n\t{\n\t\t\"proton_experimental\"\n\t\t{\n\t\t}\n\t}\n}\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("steamapps/common/Half-Life")).unwrap();
        assert_eq!(valve_tools(&root), ["proton_experimental"]);
    }

    #[test]
    fn the_internal_name_is_the_compat_tools_key() {
        let d = tmp("vdf");
        fs::write(
            d.join("compatibilitytool.vdf"),
            "\"compatibilitytools\"\n{\n\t\"compat_tools\"\n\t{\n\t\t\"GE-Proton10-20\"\n\t\t{\n\t\t\t\"install_path\"\t\".\"\n\t\t}\n\t}\n}\n",
        )
        .unwrap();
        assert_eq!(internal_name(&d).as_deref(), Some("GE-Proton10-20"));
    }
    // ---- safety: config files are never corrupted, truncated or silently overwritten ------------

    const LOCALCFG: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"570\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"keep me %command%\"\n\t\t\t\t\t\t\"playtime\"\t\t\"42\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    #[test]
    fn a_broken_config_is_left_exactly_as_it_was() {
        let root = tmp("broken");
        let p = config_path(&root);
        let bad = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n"; // never closed
        fs::write(&p, bad).unwrap();
        assert!(set_tool_checked(&root, 570, Some("x"), false).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), bad);
        assert!(
            !root.join("config/config.vdf.steamdeck-bak").exists(),
            "no backup of a file we did not change"
        );
        assert!(!root.join("config/config.vdf.steamdeck-tmp").exists());
    }

    #[test]
    fn a_missing_config_is_not_invented() {
        let root = tmp("missing");
        assert!(set_tool_checked(&root, 570, Some("x"), false).is_err());
        assert!(!config_path(&root).exists());
    }

    #[test]
    fn setting_one_game_changes_that_game_and_nothing_else() {
        let root = tmp("onlyone");
        let p = config_path(&root);
        fs::write(&p, CONFIG).unwrap();
        set_tool_checked(&root, 570, Some("GE-Proton10-20"), false).unwrap();
        set_tool_checked(&root, 730, Some("other"), false).unwrap();
        let before = parse(CONFIG).unwrap();
        let after = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        // everything outside CompatToolMapping is identical
        fn strip(items: &[(String, V)]) -> Vec<(String, V)> {
            items
                .iter()
                .map(|(k, v)| match v {
                    V::Obj(o) if k == "CompatToolMapping" => (k.clone(), V::Obj(vec![])),
                    V::Obj(o) => (k.clone(), V::Obj(strip(o))),
                    s => (k.clone(), s.clone()),
                })
                .collect()
        }
        assert_eq!(strip(&before), strip(&after));
        // clearing 730 leaves 570 alone
        set_tool_checked(&root, 730, None, false).unwrap();
        let m = mapping(&root);
        assert_eq!(m.get(&570).map(String::as_str), Some("GE-Proton10-20"));
        assert!(!m.contains_key(&730));
    }

    #[test]
    fn the_backup_is_the_previous_file_and_no_temporary_file_is_left() {
        let root = tmp("backup");
        let p = config_path(&root);
        fs::write(&p, CONFIG).unwrap();
        set_tool_checked(&root, 1, Some("a"), false).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("config/config.vdf.steamdeck-bak")).unwrap(),
            CONFIG
        );
        let names: Vec<String> = fs::read_dir(root.join("config"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().all(|n| !n.contains("tmp")), "{names:?}");
    }

    #[test]
    fn launch_options_of_other_games_and_other_keys_survive() {
        let root = tmp("lo-other");
        let cfg = root.join("userdata/1/config");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join("localconfig.vdf"), LOCALCFG).unwrap();
        set_launch_options_checked(&root, 4800, Some("mangohud %command%"), false).unwrap();
        let m = launch_options(&root);
        assert_eq!(m.get(&570).map(String::as_str), Some("keep me %command%"));
        assert_eq!(m.get(&4800).map(String::as_str), Some("mangohud %command%"));
        assert!(fs::read_to_string(cfg.join("localconfig.vdf"))
            .unwrap()
            .contains("\"playtime\"\t\t\"42\""));
        // changing 570 keeps its playtime
        set_launch_options_checked(&root, 570, Some("gamemoderun %command%"), false).unwrap();
        assert!(fs::read_to_string(cfg.join("localconfig.vdf"))
            .unwrap()
            .contains("\"playtime\"\t\t\"42\""));
    }

    #[test]
    fn a_broken_second_account_leaves_the_first_one_unchanged() {
        let root = tmp("lo-two");
        let a = root.join("userdata/1/config");
        let b = root.join("userdata/2/config");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("localconfig.vdf"), LOCALCFG).unwrap();
        fs::write(b.join("localconfig.vdf"), "\"UserLocalConfigStore\" {").unwrap();
        assert!(set_launch_options_checked(&root, 570, Some("x %command%"), false).is_err());
        assert_eq!(
            fs::read_to_string(a.join("localconfig.vdf")).unwrap(),
            LOCALCFG
        );
        assert_eq!(
            fs::read_to_string(b.join("localconfig.vdf")).unwrap(),
            "\"UserLocalConfigStore\" {"
        );
    }

    #[test]
    fn strings_with_quotes_and_backslashes_round_trip() {
        let root = tmp("lo-esc");
        let cfg = root.join("userdata/1/config");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join("localconfig.vdf"), LOCALCFG).unwrap();
        let opts = "FOO=\"a b\" C:\\Games\\x %command% --name \"Zoë\"";
        set_launch_options_checked(&root, 570, Some(opts), false).unwrap();
        assert_eq!(
            launch_options(&root).get(&570).map(String::as_str),
            Some(opts)
        );
    }

    #[test]
    fn a_game_without_a_manifest_name_is_skipped_not_guessed() {
        let root = tmp("acf-bad");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::write(
            root.join("steamapps/appmanifest_9.acf"),
            "\"AppState\" { \"appid\" \"9\" }",
        )
        .unwrap();
        fs::write(root.join("steamapps/appmanifest_x.acf"), "not vdf {").unwrap();
        assert!(games(&root).is_empty());
    }
}
