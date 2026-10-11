//! A plain log file, so that when something breaks on the Deck the text can be fetched afterwards:
//! `~/.local/state/steamdeck/steamdeck.log` (also written by `setup`). Panics end up there too.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("steamdeck/steamdeck.log")
}

fn stamp() -> String {
    // seconds since the epoch is enough to order lines, and needs no time library
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{s}")
}

pub fn write(who: &str, msg: &str) {
    let p = path();
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    // keep the file from growing for ever: past 1 MB the old one becomes .1
    if fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = fs::rename(&p, p.with_extension("log.1"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        for line in msg.lines() {
            let _ = writeln!(f, "{} {who}: {line}", stamp());
        }
    }
}

/// The last `n` lines of the log.
pub fn tail(n: usize) -> String {
    let t = fs::read_to_string(path()).unwrap_or_default();
    let lines: Vec<&str> = t.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write("panic", &info.to_string());
        prev(info);
    }));
}
