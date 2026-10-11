//! Where Steam keeps compatibility tools. A Deck has the native Steam; a desktop may have the flatpak.
use std::path::{Path, PathBuf};

/// Every Steam root that exists on this machine, in the order Steam itself prefers them.
pub fn steam_roots(home: &Path) -> Vec<PathBuf> {
    let candidates = [
        home.join(".local/share/Steam"),
        home.join(".steam/root"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        home.join("snap/steam/common/.local/share/Steam"),
    ];
    let mut out: Vec<PathBuf> = Vec::new();
    for c in candidates {
        if !c.is_dir() {
            continue;
        }
        // ~/.steam/root and ~/.steam/steam are symlinks to the real root: keep one copy of each real dir
        let real = std::fs::canonicalize(&c).unwrap_or(c.clone());
        if !out
            .iter()
            .any(|o| std::fs::canonicalize(o).unwrap_or(o.clone()) == real)
        {
            out.push(c);
        }
    }
    out
}

/// `<root>/compatibilitytools.d` for each Steam root; the directory itself may not exist yet.
pub fn tool_dirs(home: &Path) -> Vec<PathBuf> {
    steam_roots(home)
        .into_iter()
        .map(|r| r.join("compatibilitytools.d"))
        .collect()
}

/// The directory to install into: the one the user asked for, else the first Steam found, else the
/// default location of the native Steam (created on install).
pub fn default_tool_dir(home: &Path, requested: Option<&Path>) -> PathBuf {
    if let Some(p) = requested {
        return p.to_path_buf();
    }
    tool_dirs(home)
        .into_iter()
        .next()
        .unwrap_or_else(|| home.join(".local/share/Steam/compatibilitytools.d"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("protonup-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn no_steam_gives_the_native_default() {
        let h = tmp("none");
        assert_eq!(
            default_tool_dir(&h, None),
            h.join(".local/share/Steam/compatibilitytools.d")
        );
    }

    #[test]
    fn native_steam_is_preferred_and_symlinks_are_deduplicated() {
        let h = tmp("native");
        std::fs::create_dir_all(h.join(".local/share/Steam")).unwrap();
        std::fs::create_dir_all(h.join(".steam")).unwrap();
        std::os::unix::fs::symlink(h.join(".local/share/Steam"), h.join(".steam/root")).unwrap();
        std::os::unix::fs::symlink(h.join(".local/share/Steam"), h.join(".steam/steam")).unwrap();
        let roots = steam_roots(&h);
        assert_eq!(roots.len(), 1, "{roots:?}");
        assert_eq!(
            default_tool_dir(&h, None),
            h.join(".local/share/Steam/compatibilitytools.d")
        );
    }

    #[test]
    fn flatpak_steam_is_found() {
        let h = tmp("flatpak");
        std::fs::create_dir_all(h.join(".var/app/com.valvesoftware.Steam/data/Steam")).unwrap();
        let d = tool_dirs(&h);
        assert_eq!(d.len(), 1);
        assert!(d[0].to_string_lossy().contains("com.valvesoftware.Steam"));
    }

    #[test]
    fn an_explicit_directory_wins() {
        let h = tmp("explicit");
        let want = h.join("somewhere/tools");
        assert_eq!(default_tool_dir(&h, Some(&want)), want);
    }
}
