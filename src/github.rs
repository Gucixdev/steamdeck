//! Releases from the GitHub API. The API root can be redirected with PROTONUP_API, which is how the
//! tests and offline mirrors serve a local copy. GH_TOKEN / GITHUB_TOKEN raise the rate limit.
use crate::sources::Source;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub published_at: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    /// The archive of this release for `src`: the asset with the source's suffix, minus excluded ones.
    pub fn archive(&self, src: &Source) -> Option<&Asset> {
        self.assets.iter().find(|a| {
            a.name.ends_with(src.suffix)
                && !src
                    .exclude
                    .iter()
                    .any(|x| a.name.to_lowercase().contains(x))
        })
    }

    /// The SHA-512 file published next to `archive`, under any of the names projects use.
    pub fn checksum(&self, archive: &Asset) -> Option<&Asset> {
        let stem = archive
            .name
            .strip_suffix(".tar.gz")
            .or_else(|| archive.name.strip_suffix(".tar.xz"))
            .unwrap_or(&archive.name)
            .to_string();
        let names = [
            format!("{}.sha512sum", archive.name),
            format!("{}.sha512", archive.name),
            format!("{stem}.sha512sum"),
            format!("{stem}.sha512"),
            // some projects publish SHA-256 instead (Proton-EM)
            format!("{}.sha256sum", archive.name),
            format!("{}.sha256", archive.name),
            format!("{stem}.sha256sum"),
            format!("{stem}.sha256"),
        ];
        names
            .iter()
            .find_map(|n| self.assets.iter().find(|a| &a.name == n))
    }

    /// `2025-02-14T10:11:12Z` -> `2025-02-14`
    pub fn date(&self) -> &str {
        self.published_at.split('T').next().unwrap_or("")
    }

    /// The directory this release installs to.
    pub fn dir_name(&self, src: &Source) -> String {
        use crate::sources::DirName;
        match src.dir {
            DirName::Tag => self.tag_name.clone(),
            DirName::Fixed(n) => n.to_string(),
            DirName::ArchiveStem => self
                .archive(src)
                .map(|a| {
                    a.name
                        .strip_suffix(".tar.gz")
                        .or_else(|| a.name.strip_suffix(".tar.xz"))
                        .unwrap_or(&a.name)
                        .to_string()
                })
                .unwrap_or_else(|| self.tag_name.clone()),
        }
    }
}

pub fn api_root() -> String {
    std::env::var("PROTONUP_API").unwrap_or_else(|_| "https://api.github.com".to_string())
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60 * 60)))
        .timeout_connect(Some(Duration::from_secs(15)))
        .user_agent("protonup (github.com/Gucixdev/steamdeck)")
        .build()
        .into()
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut req = agent()
        .get(url)
        .header("Accept", "application/vnd.github+json");
    if let Ok(tok) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        req = req.header("Authorization", &format!("Bearer {tok}"));
    }
    req.call().map_err(|e| format!("{url}: {e}"))
}

/// One page (1-based) of a source's releases, as GitHub returns it: pre-releases and releases with no
/// matching archive are dropped, so a page can come back shorter than `per_page`.
/// The second value is whether GitHub had a full page, i.e. whether another page may exist.
pub fn releases_page(
    src: &Source,
    page: usize,
    per_page: usize,
) -> Result<(Vec<Release>, bool), String> {
    let per_page = per_page.clamp(1, 100);
    let url = format!(
        "{}/repos/{}/releases?per_page={}&page={}",
        api_root(),
        src.repo,
        per_page,
        page.max(1)
    );
    let mut resp = get(&url)?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let all: Vec<Release> =
        serde_json::from_str(&body).map_err(|e| format!("bad release list: {e}"))?;
    let full = all.len() >= per_page;
    let keep = all
        .into_iter()
        .filter(|r| !r.prerelease && r.archive(src).is_some())
        .collect();
    Ok((keep, full))
}

/// Up to `max` releases (newest first), reading as many pages as needed. `usize::MAX` means all.
pub fn releases(src: &Source, max: usize) -> Result<Vec<Release>, String> {
    let mut out = Vec::new();
    let mut page = 1;
    loop {
        let (mut v, more) = releases_page(src, page, 100)?;
        out.append(&mut v);
        if out.len() >= max || !more || page >= 20 {
            break;
        }
        page += 1;
    }
    out.truncate(max);
    Ok(out)
}

/// Parse `<hex>  <file>` lines of a checksum file and return the SHA-256 or SHA-512 for `file`. A file with a
/// single entry is trusted for the archive even when its name differs (projects rename on upload).
pub fn checksum_for(text: &str, file: &str) -> Option<String> {
    let valid =
        |h: &str| (h.len() == 128 || h.len() == 64) && h.chars().all(|c| c.is_ascii_hexdigit());
    let mut entries: Vec<(String, Option<String>)> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(hash) = it.next() else { continue };
        if !valid(hash) {
            continue;
        }
        entries.push((
            hash.to_ascii_lowercase(),
            it.next().map(|n| n.trim_start_matches('*').to_string()),
        ));
    }
    if let Some((h, _)) = entries.iter().find(|(_, n)| n.as_deref() == Some(file)) {
        return Some(h.clone());
    }
    if entries.len() == 1 {
        return Some(entries[0].0.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{find, SOURCES};

    fn asset(n: &str) -> Asset {
        Asset {
            name: n.into(),
            browser_download_url: format!("u/{n}"),
            size: 1,
        }
    }

    fn ge_release() -> Release {
        Release {
            tag_name: "GE-Proton11-7".into(),
            published_at: "2026-10-05T10:11:12Z".into(),
            prerelease: false,
            assets: vec![
                asset("GE-Proton11-7-aarch64.sha512sum"),
                asset("GE-Proton11-7-aarch64.tar.gz"),
                asset("GE-Proton11-7-x86_64.sha512sum"),
                asset("GE-Proton11-7-x86_64.tar.gz"),
            ],
        }
    }

    #[test]
    fn ge_picks_x86_64_not_aarch64() {
        let src = find("ge").unwrap();
        let r = ge_release();
        let a = r.archive(src).unwrap();
        assert_eq!(a.name, "GE-Proton11-7-x86_64.tar.gz");
        assert_eq!(
            r.checksum(a).unwrap().name,
            "GE-Proton11-7-x86_64.sha512sum"
        );
        assert_eq!(r.dir_name(src), "GE-Proton11-7");
        assert_eq!(r.date(), "2026-10-05");
    }

    #[test]
    fn cachyos_generic_and_v3_do_not_mix() {
        let r = Release {
            tag_name: "cachyos-11.0-20261005-slr".into(),
            published_at: String::new(),
            prerelease: false,
            assets: vec![
                asset("proton-cachyos-11.0-20261005-slr-arm64.tar.xz"),
                asset("proton-cachyos-11.0-20261005-slr-x86_64.sha512sum"),
                asset("proton-cachyos-11.0-20261005-slr-x86_64.tar.xz"),
                asset("proton-cachyos-11.0-20261005-slr-x86_64_v3.sha512sum"),
                asset("proton-cachyos-11.0-20261005-slr-x86_64_v3.tar.xz"),
            ],
        };
        let g = find("cachyos").unwrap();
        let v3 = find("cachyos-v3").unwrap();
        assert!(r.archive(g).unwrap().name.ends_with("-slr-x86_64.tar.xz"));
        assert!(r
            .archive(v3)
            .unwrap()
            .name
            .ends_with("-slr-x86_64_v3.tar.xz"));
        assert_eq!(r.dir_name(g), "proton-cachyos-11.0-20261005-slr-x86_64");
        assert_eq!(
            r.checksum(r.archive(v3).unwrap()).unwrap().name,
            "proton-cachyos-11.0-20261005-slr-x86_64_v3.sha512sum"
        );
    }

    #[test]
    fn luxtorpeda_checksum_naming_and_fixed_dir() {
        let r = Release {
            tag_name: "v76.3.0".into(),
            published_at: String::new(),
            prerelease: false,
            assets: vec![
                asset("luxtorpeda-v76.3.0.tar.xz"),
                asset("luxtorpeda-v76.3.0.tar.xz.sha512"),
            ],
        };
        let s = find("luxtorpeda").unwrap();
        assert_eq!(
            r.checksum(r.archive(s).unwrap()).unwrap().name,
            "luxtorpeda-v76.3.0.tar.xz.sha512"
        );
        assert_eq!(r.dir_name(s), "luxtorpeda");
    }

    #[test]
    fn sarek_skips_the_async_variant() {
        let r = Release {
            tag_name: "Proton-Sarek10-29".into(),
            published_at: String::new(),
            prerelease: false,
            assets: vec![
                asset("Proton-Sarek10-29-async.tar.gz"),
                asset("Proton-Sarek10-29.tar.gz"),
            ],
        };
        assert_eq!(
            r.archive(find("sarek").unwrap()).unwrap().name,
            "Proton-Sarek10-29.tar.gz"
        );
    }

    #[test]
    fn parses_checksum_lines() {
        let h = "a".repeat(128);
        let text = format!(
            "{h}  GE-Proton10-1.tar.gz\n{}  other.tar.gz\n",
            "b".repeat(128)
        );
        assert_eq!(checksum_for(&text, "GE-Proton10-1.tar.gz").unwrap(), h);
        assert_eq!(
            checksum_for(&text, "other.tar.gz").unwrap(),
            "b".repeat(128)
        );
        assert!(
            checksum_for(&text, "missing.tar.gz").is_none(),
            "two entries, none for this file"
        );
        assert!(
            checksum_for("zz  x.tar.gz", "x.tar.gz").is_none(),
            "short or non-hex hash is refused"
        );
        assert_eq!(
            checksum_for(&format!("{h}  renamed.tar.xz\n"), "real.tar.xz").unwrap(),
            h,
            "single entry is trusted"
        );
    }

    #[test]
    fn json_with_missing_fields_parses() {
        let j = r#"[{"tag_name":"x","assets":[{"name":"x.tar.gz","browser_download_url":"u"}]}]"#;
        let v: Vec<Release> = serde_json::from_str(j).unwrap();
        assert_eq!(v[0].assets[0].size, 0);
        assert!(!v[0].prerelease);
        assert!(SOURCES.len() >= 5);
    }
    #[test]
    fn test_builds_of_proton_em_are_not_offered() {
        let src = crate::sources::find("em").unwrap();
        let mk = |n: &str| Release {
            tag_name: n.into(),
            published_at: String::new(),
            prerelease: false,
            assets: vec![Asset {
                name: format!("proton-{n}.tar.xz"),
                browser_download_url: String::new(),
                size: 1,
            }],
        };
        assert!(mk("EM-10.0-37-HDR").archive(src).is_some());
        assert!(mk("EM-10.0-36-HDRTEST").archive(src).is_none());
    }
}
