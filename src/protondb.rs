//! ProtonDB ratings (platinum, gold, ...) and Steam's own "has a Linux build" flag for the games in the
//! table. Nothing is sent unless you press
//! the button; the request is the public summary endpoint with only the app id in it, and the answers
//! are cached in `~/.cache/steamdeck/protondb.json` so each game is asked once.
use std::collections::HashMap;
use std::path::PathBuf;

fn cache_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".cache")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("steamdeck/protondb.json")
}

pub fn cached() -> HashMap<u64, String> {
    std::fs::read_to_string(cache_path())
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
        .map(|m| {
            m.into_iter()
                .filter_map(|(k, v)| k.parse().ok().map(|k| (k, v)))
                .collect()
        })
        .unwrap_or_default()
}

fn save(m: &HashMap<u64, String>) {
    let p = cache_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let s: HashMap<String, &String> = m.iter().map(|(k, v)| (k.to_string(), v)).collect();
    if let Ok(t) = serde_json::to_string(&s) {
        let _ = std::fs::write(p, t);
    }
}

/// The tier of one game: "platinum", "gold", "silver", "bronze", "borked" or "pending".
pub fn tier(appid: u64) -> Result<String, String> {
    let root = std::env::var("PROTONDB_API").unwrap_or_else(|_| "https://www.protondb.com".into());
    let url = format!("{root}/api/v1/reports/summaries/{appid}.json");
    let mut r = crate::github::agent()
        .get(&url)
        .call()
        .map_err(|e| format!("protondb {appid}: {e}"))?;
    let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    v.get("tier")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| "no tier in the answer".to_string())
}

fn native_cache_path() -> PathBuf {
    cache_path().with_file_name("native.json")
}

/// appid -> the game has a native Linux build (from Steam's store data), as cached.
pub fn native_cached() -> HashMap<u64, bool> {
    std::fs::read_to_string(native_cache_path())
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, bool>>(&t).ok())
        .map(|m| {
            m.into_iter()
                .filter_map(|(k, v)| k.parse().ok().map(|k| (k, v)))
                .collect()
        })
        .unwrap_or_default()
}

/// Does Steam list a Linux build for this game? Uses the public store endpoint with only the app id.
pub fn has_linux_build(appid: u64) -> Result<bool, String> {
    let root = std::env::var("STEAM_STORE_API")
        .unwrap_or_else(|_| "https://store.steampowered.com".into());
    let url = format!("{root}/api/appdetails?appids={appid}&filters=platforms");
    let mut r = crate::github::agent()
        .get(&url)
        .call()
        .map_err(|e| format!("store {appid}: {e}"))?;
    let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    v.get(appid.to_string())
        .and_then(|a| a.get("data"))
        .and_then(|d| d.get("platforms"))
        .and_then(|p| p.get("linux"))
        .and_then(|l| l.as_bool())
        .ok_or_else(|| "no platform data in the answer".to_string())
}

/// Same as `fetch_missing` for the native-Linux flag.
pub fn fetch_native_missing(ids: &[u64]) -> HashMap<u64, bool> {
    let mut m = native_cached();
    for id in ids {
        if *id == 0 || m.contains_key(id) {
            continue;
        }
        match has_linux_build(*id) {
            Ok(b) => {
                m.insert(*id, b);
            }
            Err(e) => crate::log::write("store", &e),
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let p = native_cache_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let s: HashMap<String, bool> = m.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    if let Ok(t) = serde_json::to_string(&s) {
        let _ = std::fs::write(p, t);
    }
    m
}

/// Ask for every game not cached yet (politely, one at a time) and return everything known.
pub fn fetch_missing(ids: &[u64]) -> HashMap<u64, String> {
    let mut m = cached();
    for id in ids {
        if *id == 0 || m.contains_key(id) {
            continue;
        }
        match tier(*id) {
            Ok(t) => {
                m.insert(*id, t);
            }
            Err(e) => crate::log::write("protondb", &e),
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    save(&m);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn a_tier_is_read_from_the_summary_and_cached() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let body = if line.contains("/570.json") {
                    "{\"tier\":\"platinum\",\"total\":99}"
                } else {
                    "{}"
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let cache = std::env::temp_dir().join(format!("pdb-{}", std::process::id()));
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("PROTONDB_API", format!("http://127.0.0.1:{port}"));
        assert_eq!(tier(570).unwrap(), "platinum");
        assert!(tier(1).is_err(), "an answer without a tier is an error");
        let l2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port2 = l2.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l2.incoming().flatten() {
                let mut s = s;
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let body = if line.contains("appids=570") {
                    "{\"570\":{\"success\":true,\"data\":{\"platforms\":{\"windows\":true,\"mac\":true,\"linux\":true}}}}"
                } else {
                    "{\"9\":{\"success\":true,\"data\":{\"platforms\":{\"windows\":true,\"mac\":false,\"linux\":false}}}}"
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        std::env::set_var("STEAM_STORE_API", format!("http://127.0.0.1:{port2}"));
        assert_eq!(has_linux_build(570), Ok(true));
        assert!(
            has_linux_build(77).is_err(),
            "an answer for another app id is not trusted"
        );
        assert_eq!(fetch_native_missing(&[570]).get(&570), Some(&true));
        assert_eq!(native_cached().get(&570), Some(&true));
        let m = fetch_missing(&[570, 0]);
        assert_eq!(m.get(&570).map(String::as_str), Some("platinum"));
        assert_eq!(cached().get(&570).map(String::as_str), Some("platinum"));
        let _ = std::fs::remove_dir_all(cache);
    }
}
