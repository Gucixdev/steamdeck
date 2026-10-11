//! Download, verify (SHA-512), unpack and remove compatibility tools.
use crate::github::{agent, checksum_for, Release};
use crate::sources::{DirName, Source};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha512};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// A step without a measurable length ("checksum", "extracting", "done").
    Phase(String),
    /// Bytes of the archive received so far, and the total when the server said it.
    Download { done: u64, total: u64 },
}

fn dir_size(p: &Path) -> u64 {
    let mut n = 0;
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                n += dir_size(&e.path());
            } else {
                n += md.len();
            }
        }
    }
    n
}

/// A compatibility tool is a directory with a compatibilitytool.vdf in it.
pub fn list_installed(dir: &Path) -> Vec<Installed> {
    list_installed_with(dir, true)
}

/// Same list; `sizes: false` skips the recursive size walk (a few thousand files per tool) and
/// leaves `size` at 0, for interfaces that show the names first and fill the sizes in later.
pub fn list_installed_with(dir: &Path, sizes: bool) -> Vec<Installed> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !path.is_dir() || !path.join("compatibilitytool.vdf").is_file()
        {
            continue;
        }
        out.push(Installed {
            size: if sizes { dir_size(&path) } else { 0 },
            name,
            path,
        });
    }
    out.sort_by(|a, b| {
        version_key(&b.name)
            .cmp(&version_key(&a.name))
            .then(b.name.cmp(&a.name))
    });
    out
}

/// `GE-Proton10-12` sorts after `GE-Proton10-2` and after `GE-Proton9-27`: compare the numbers, not the text.
pub fn version_key(name: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(cur.parse().unwrap_or(0));
            cur.clear();
        }
    }
    if !cur.is_empty() {
        out.push(cur.parse().unwrap_or(0));
    }
    out
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
        && !name.starts_with('.')
}

pub fn remove(dir: &Path, name: &str) -> Result<(), String> {
    if !safe_name(name) {
        return Err(format!("refusing to remove '{name}'"));
    }
    let target = dir.join(name);
    if !target.join("compatibilitytool.vdf").is_file() {
        return Err(format!("{} is not a compatibility tool", target.display()));
    }
    fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))
}

struct Cleanup(Vec<PathBuf>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            if p.is_dir() {
                let _ = fs::remove_dir_all(p);
            } else {
                let _ = fs::remove_file(p);
            }
        }
    }
}

/// Download `url` into `part`, continuing what is already there when the server honours `Range`.
/// Returns the SHA-512 (hex) of the complete file.
fn pull(
    url: &str,
    part: &Path,
    size_hint: u64,
    progress: &mut dyn FnMut(Progress),
) -> Result<String, String> {
    let mut have = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let mut hasher = Sha512::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut resp = None;
    if have > 0 {
        match agent()
            .get(url)
            .header("Range", format!("bytes={have}-"))
            .call()
        {
            Ok(r) if r.status().as_u16() == 206 => resp = Some(r),
            _ => have = 0, // no ranges, or the part is not a prefix of this file: start over
        }
    }
    if have > 0 {
        let mut f = File::open(part).map_err(|e| format!("{}: {e}", part.display()))?;
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    let resp = match resp {
        Some(r) => r,
        None => agent()
            .get(url)
            .call()
            .map_err(|e| format!("download: {e}"))?,
    };
    let len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let total = len.map(|l| l + have).unwrap_or(size_hint);
    let mut out = if have > 0 {
        fs::OpenOptions::new().append(true).open(part)
    } else {
        File::create(part)
    }
    .map_err(|e| format!("{}: {e}", part.display()))?;
    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    let mut done = have;
    let mut last_report = have;
    progress(Progress::Download { done, total });
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])
            .map_err(|e| format!("write: {e}"))?;
        done += n as u64;
        if done - last_report >= 512 * 1024 {
            last_report = done;
            progress(Progress::Download { done, total });
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    progress(Progress::Download {
        done,
        total: total.max(done),
    });
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// What happened to the checksum of an install, for the interface to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub path: PathBuf,
    /// false when the project publishes no checksum and only HTTPS vouched for the download
    pub verified: bool,
}

/// Install `rel` from `src` into `dir`. Nothing appears under its final name unless the checksum
/// matched (when the project publishes one) and the archive unpacked completely; a failure leaves no
/// partial files behind.
pub fn install(
    src: &Source,
    rel: &Release,
    dir: &Path,
    progress: &mut dyn FnMut(Progress),
) -> Result<Outcome, String> {
    if !safe_name(&rel.tag_name) {
        return Err(format!("odd release name '{}'", rel.tag_name));
    }
    let archive = rel
        .archive(src)
        .ok_or("release has no archive for this source")?;
    let want_dir = rel.dir_name(src);
    if !safe_name(&want_dir) {
        return Err(format!("odd directory name '{want_dir}'"));
    }
    let replace = matches!(src.dir, DirName::Fixed(_));
    let final_dir = dir.join(&want_dir);
    if final_dir.exists() && !replace {
        return Err(format!("{want_dir} is already installed"));
    }
    let sums = rel.checksum(archive);
    if src.checksum && sums.is_none() {
        return Err("this release publishes no checksum: refusing to install it unverified".into());
    }
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let part = dir.join(format!(".{}.part", archive.name));
    let tmp = dir.join(format!(".{}.tmp", want_dir));
    let mut guard = Cleanup(vec![part.clone(), tmp.clone()]);
    let _ = fs::remove_dir_all(&tmp);

    let expected = match sums {
        Some(sums) => {
            progress(Progress::Phase("checksum".into()));
            let mut r = agent()
                .get(&sums.browser_download_url)
                .call()
                .map_err(|e| format!("checksum: {e}"))?;
            let text = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
            Some(
                checksum_for(&text, &archive.name)
                    .ok_or("no valid SHA-512 for the archive in the checksum file")?,
            )
        }
        None => None,
    };

    // like dspq: sequence of attempts, each continuing the same .part; only a bad checksum or a
    // finished install throws the part away, a network failure keeps it so a re-run resumes
    let mut attempt = 0;
    let got = loop {
        match pull(&archive.browser_download_url, &part, archive.size, progress) {
            Ok(d) => break d,
            Err(e) => {
                attempt += 1;
                if attempt >= 3 {
                    guard.0.retain(|p| p != &part);
                    return Err(format!(
                        "{e} (the partial download is kept, run it again to resume)"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_secs(attempt));
            }
        }
    };

    if let Some(expected) = &expected {
        progress(Progress::Phase("verifying".into()));
        if &got != expected {
            return Err(format!(
                "checksum mismatch: expected {}..., got {}...",
                &expected[..16],
                &got[..16]
            ));
        }
    }

    progress(Progress::Phase("extracting".into()));
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let file = File::open(&part).map_err(|e| e.to_string())?;
    if archive.name.ends_with(".tar.xz") {
        tar::Archive::new(lzma_rust2::XzReader::new(
            std::io::BufReader::new(file),
            true,
        ))
        .unpack(&tmp)
        .map_err(|e| format!("extract: {e}"))?;
    } else {
        tar::Archive::new(GzDecoder::new(file))
            .unpack(&tmp)
            .map_err(|e| format!("extract: {e}"))?;
    }
    // the directory with the compatibilitytool.vdf, wherever the archive put it (top level in practice)
    let mut found = None;
    if let Ok(rd) = fs::read_dir(&tmp) {
        for e in rd.flatten() {
            if e.path().join("compatibilitytool.vdf").is_file() {
                found = Some(e.path());
            }
        }
    }
    let src_dir =
        found.ok_or("archive does not contain a compatibility tool (no compatibilitytool.vdf)")?;
    if replace && final_dir.exists() {
        fs::remove_dir_all(&final_dir).map_err(|e| format!("{}: {e}", final_dir.display()))?;
    }
    fs::rename(&src_dir, &final_dir).map_err(|e| format!("{}: {e}", final_dir.display()))?;
    guard.0.retain(|p| p != &final_dir);
    progress(Progress::Phase("done".into()));
    Ok(Outcome {
        path: final_dir,
        verified: expected.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Asset;
    use crate::sources::{find, Source};
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("protonup-i-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn make_archive(tag: &str, with_vdf: bool) -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut b = tar::Builder::new(enc);
        let mut add = |path: String, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, data).unwrap();
        };
        if with_vdf {
            add(
                format!("{tag}/compatibilitytool.vdf"),
                b"\"compatibilitytools\" {}",
            );
        }
        add(format!("{tag}/proton"), b"#!/bin/sh\n");
        add(format!("{tag}/files/bin/wine"), &vec![7u8; 100_000]);
        b.into_inner().unwrap().finish().unwrap()
    }

    fn sha512_hex(d: &[u8]) -> String {
        Sha512::digest(d)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Serves `files` (path -> bytes) on 127.0.0.1 until the process ends; returns the base URL.
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let files = Arc::new(files);
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    let mut from = 0usize;
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            from = v.trim().trim_end_matches('-').parse().unwrap_or(0);
                        }
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    match files.iter().find(|(p, _)| *p == path) {
                        Some((_, data)) if from > 0 && from < data.len() => {
                            let _ = write!(s, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n", data.len() - from, from, data.len() - 1, data.len());
                            let _ = s.write_all(&data[from..]);
                        }
                        Some((_, data)) => {
                            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len());
                            let _ = s.write_all(data);
                        }
                        None => {
                            let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        }
                    }
                });
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn ge() -> &'static Source {
        find("ge").unwrap()
    }

    fn release(base: &str, tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            published_at: "2025-01-01T00:00:00Z".into(),
            prerelease: false,
            assets: vec![
                Asset {
                    name: format!("{tag}.tar.gz"),
                    browser_download_url: format!("{base}/{tag}.tar.gz"),
                    size: 0,
                },
                Asset {
                    name: format!("{tag}.sha512sum"),
                    browser_download_url: format!("{base}/{tag}.sha512sum"),
                    size: 0,
                },
            ],
        }
    }

    fn make_xz_archive(top: &str) -> Vec<u8> {
        let enc =
            lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
        let mut b = tar::Builder::new(enc);
        let mut add = |path: String, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, data).unwrap();
        };
        add(
            format!("{top}/compatibilitytool.vdf"),
            b"\"compatibilitytools\" {}",
        );
        add(format!("{top}/proton"), b"#!/bin/sh\n");
        add(format!("{top}/files/data"), &vec![3u8; 200_000]);
        let mut w = b.into_inner().unwrap();
        w.flush().unwrap();
        w.finish().unwrap()
    }

    #[test]
    fn installs_verifies_lists_and_removes() {
        let tag = "GE-Proton10-9";
        let data = make_archive(tag, true);
        let sums = format!("{}  {tag}.tar.gz\n", sha512_hex(&data));
        let base = serve(vec![
            (format!("/{tag}.tar.gz"), data),
            (format!("/{tag}.sha512sum"), sums.into_bytes()),
        ]);
        let dir = tmp("ok");
        let mut phases = Vec::new();
        let mut max_done = 0;
        let p = install(ge(), &release(&base, tag), &dir, &mut |pr| match pr {
            Progress::Phase(s) => phases.push(s),
            Progress::Download { done, .. } => max_done = max_done.max(done),
        })
        .unwrap();
        assert!(p.verified);
        let p = p.path;
        assert_eq!(p, dir.join(tag));
        assert!(p.join("compatibilitytool.vdf").is_file());
        assert!(p.join("files/bin/wine").is_file());
        assert!(max_done > 0);
        assert_eq!(phases, ["checksum", "verifying", "extracting", "done"]);
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, [tag], "no temporary files are left: {names:?}");

        let l = list_installed(&dir);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].name, tag);
        assert!(l[0].size > 100_000);

        assert!(install(ge(), &release(&base, tag), &dir, &mut |_| {})
            .unwrap_err()
            .contains("already installed"));
        remove(&dir, tag).unwrap();
        assert!(list_installed(&dir).is_empty());
    }

    #[test]
    fn a_wrong_checksum_installs_nothing() {
        let tag = "GE-Proton10-8";
        let data = make_archive(tag, true);
        let sums = format!("{}  {tag}.tar.gz\n", "0".repeat(128));
        let base = serve(vec![
            (format!("/{tag}.tar.gz"), data),
            (format!("/{tag}.sha512sum"), sums.into_bytes()),
        ]);
        let dir = tmp("badsum");
        let e = install(ge(), &release(&base, tag), &dir, &mut |_| {}).unwrap_err();
        assert!(e.contains("checksum mismatch"), "{e}");
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "partial files must be cleaned up"
        );
    }

    #[test]
    fn an_interrupted_download_resumes_from_the_part_file() {
        let tag = "GE-Proton10-6";
        let data = make_archive(tag, true);
        let sums = format!("{}  {tag}.tar.gz\n", sha512_hex(&data));
        let half = data.len() / 2;
        let base = serve(vec![
            (format!("/{tag}.tar.gz"), data.clone()),
            (format!("/{tag}.sha512sum"), sums.into_bytes()),
        ]);
        let dir = tmp("resume");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!(".{tag}.tar.gz.part")), &data[..half]).unwrap();
        let mut first = None;
        let o = install(ge(), &release(&base, tag), &dir, &mut |p| {
            if let (None, Progress::Download { done, .. }) = (&first, &p) {
                first = Some(*done);
            }
        })
        .unwrap();
        assert!(o.verified);
        assert_eq!(
            first,
            Some(half as u64),
            "the first report must start at the bytes already on disk"
        );
        assert!(dir.join(tag).join("compatibilitytool.vdf").is_file());
        assert!(!dir.join(format!(".{tag}.tar.gz.part")).exists());
    }

    #[test]
    fn a_garbage_part_file_is_caught_by_the_checksum() {
        let tag = "GE-Proton10-5";
        let data = make_archive(tag, true);
        let sums = format!("{}  {tag}.tar.gz\n", sha512_hex(&data));
        let base = serve(vec![
            (format!("/{tag}.tar.gz"), data),
            (format!("/{tag}.sha512sum"), sums.into_bytes()),
        ]);
        let dir = tmp("garbage");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!(".{tag}.tar.gz.part")),
            b"not the start of the archive",
        )
        .unwrap();
        let e = install(ge(), &release(&base, tag), &dir, &mut |_| {}).unwrap_err();
        assert!(e.contains("checksum mismatch"), "{e}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn an_archive_without_a_tool_is_refused() {
        let tag = "GE-Proton10-7";
        let data = make_archive(tag, false);
        let sums = format!("{}  {tag}.tar.gz\n", sha512_hex(&data));
        let base = serve(vec![
            (format!("/{tag}.tar.gz"), data),
            (format!("/{tag}.sha512sum"), sums.into_bytes()),
        ]);
        let dir = tmp("novdf");
        let e = install(ge(), &release(&base, tag), &dir, &mut |_| {}).unwrap_err();
        assert!(e.contains("no compatibilitytool.vdf"), "{e}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn a_release_without_a_checksum_is_refused() {
        let dir = tmp("nosum");
        let mut r = release("http://127.0.0.1:1", "GE-Proton1-1");
        r.assets.truncate(1);
        assert!(install(ge(), &r, &dir, &mut |_| {})
            .unwrap_err()
            .contains("unverified"));
    }

    #[test]
    fn remove_is_careful() {
        let dir = tmp("rm");
        fs::create_dir_all(dir.join("keep")).unwrap();
        fs::write(dir.join("keep/file"), "x").unwrap();
        assert!(remove(&dir, "keep")
            .unwrap_err()
            .contains("not a compatibility tool"));
        assert!(remove(&dir, "../etc").is_err());
        assert!(remove(&dir, "a/b").is_err());
        assert!(remove(&dir, "..").is_err());
        assert!(dir.join("keep/file").exists());
    }

    #[test]
    fn versions_sort_by_number() {
        let dir = tmp("sort");
        for n in ["GE-Proton9-27", "GE-Proton10-2", "GE-Proton10-12"] {
            fs::create_dir_all(dir.join(n)).unwrap();
            fs::write(dir.join(n).join("compatibilitytool.vdf"), "").unwrap();
        }
        fs::create_dir_all(dir.join("not-a-tool")).unwrap();
        let names: Vec<String> = list_installed(&dir).into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["GE-Proton10-12", "GE-Proton10-2", "GE-Proton9-27"]);
    }

    #[test]
    fn an_xz_archive_installs_and_a_fixed_directory_is_replaced() {
        let src = find("luxtorpeda").unwrap();
        let data = make_xz_archive("luxtorpeda");
        let sums = format!("{}  luxtorpeda-v1.tar.xz\n", sha512_hex(&data));
        let base = serve(vec![
            ("/luxtorpeda-v1.tar.xz".into(), data),
            ("/luxtorpeda-v1.tar.xz.sha512".into(), sums.into_bytes()),
        ]);
        let rel = Release {
            tag_name: "v1".into(),
            published_at: String::new(),
            prerelease: false,
            assets: vec![
                Asset {
                    name: "luxtorpeda-v1.tar.xz".into(),
                    browser_download_url: format!("{base}/luxtorpeda-v1.tar.xz"),
                    size: 0,
                },
                Asset {
                    name: "luxtorpeda-v1.tar.xz.sha512".into(),
                    browser_download_url: format!("{base}/luxtorpeda-v1.tar.xz.sha512"),
                    size: 0,
                },
            ],
        };
        let dir = tmp("xz");
        let o = install(src, &rel, &dir, &mut |_| {}).unwrap();
        assert_eq!(o.path, dir.join("luxtorpeda"));
        assert!(o.path.join("files/data").is_file());
        fs::write(o.path.join("stale"), "old").unwrap();
        let o2 = install(src, &rel, &dir, &mut |_| {}).unwrap();
        assert!(
            !o2.path.join("stale").exists(),
            "the previous install is replaced, not merged"
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn a_source_without_published_checksums_installs_unverified_and_says_so() {
        let src = find("sarek").unwrap();
        let tag = "Proton-Sarek10-29";
        let data = make_archive(tag, true);
        let base = serve(vec![(format!("/{tag}.tar.gz"), data)]);
        let rel = Release {
            tag_name: tag.into(),
            published_at: String::new(),
            prerelease: false,
            assets: vec![Asset {
                name: format!("{tag}.tar.gz"),
                browser_download_url: format!("{base}/{tag}.tar.gz"),
                size: 0,
            }],
        };
        let dir = tmp("nosum2");
        let o = install(src, &rel, &dir, &mut |_| {}).unwrap();
        assert!(!o.verified);
        assert!(o.path.join("compatibilitytool.vdf").is_file());
    }
}
