//! The compatibility tools protonup knows how to fetch. Every one is a GitHub project that publishes
//! a tarball per release; adding another is one more entry below.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DirName {
    /// the release tag is the directory name (GE-Proton10-3)
    Tag,
    /// the archive name without .tar.gz / .tar.xz
    ArchiveStem,
    /// always this directory: an install replaces the previous one (Luxtorpeda updates in place)
    Fixed(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Source {
    pub id: &'static str,
    pub name: &'static str,
    pub repo: &'static str,
    /// archive file name suffix, e.g. ".tar.gz" or "-x86_64.tar.xz"
    pub suffix: &'static str,
    /// archives containing any of these are skipped (other architectures, "async" variants)
    pub exclude: &'static [&'static str],
    pub dir: DirName,
    /// does the project publish a SHA-512 next to the archive? When not, only HTTPS vouches for it.
    pub checksum: bool,
    pub about: &'static str,
}

pub const SOURCES: &[Source] = &[
    Source {
        id: "ge",
        name: "Proton-GE",
        repo: "GloriousEggroll/proton-ge-custom",
        suffix: ".tar.gz",
        exclude: &["aarch64", "arm64"],
        dir: DirName::Tag,
        checksum: true,
        about: "GloriousEggroll's Proton with extra patches and media codecs. The usual pick.",
    },
    Source {
        id: "cachyos",
        name: "Proton-CachyOS",
        repo: "CachyOS/proton-cachyos",
        suffix: "-x86_64.tar.xz",
        exclude: &[],
        dir: DirName::ArchiveStem,
        checksum: true,
        about: "CachyOS build with scheduler and performance patches (generic x86-64).",
    },
    Source {
        id: "cachyos-v3",
        name: "Proton-CachyOS v3",
        repo: "CachyOS/proton-cachyos",
        suffix: "-x86_64_v3.tar.xz",
        exclude: &[],
        dir: DirName::ArchiveStem,
        checksum: true,
        about: "The same, built for x86-64-v3 (AVX2). The Deck's Zen 2 CPU runs it.",
    },
    Source {
        id: "rtsp",
        name: "Proton-GE RTSP",
        repo: "SpookySkeletons/proton-ge-rtsp",
        suffix: ".tar.gz",
        exclude: &[],
        dir: DirName::ArchiveStem,
        checksum: true,
        about: "Proton-GE with fixes for games that play video through GStreamer RTSP.",
    },
    Source {
        id: "sarek",
        name: "Proton-Sarek",
        repo: "pythonlover02/Proton-Sarek",
        suffix: ".tar.gz",
        exclude: &["async"],
        dir: DirName::Tag,
        checksum: false,
        about: "Proton built for older GPUs and Vulkan drivers.",
    },
    Source {
        id: "northstar",
        name: "NorthstarProton",
        repo: "R2NorthstarTools/NorthstarProton",
        suffix: ".tar.gz",
        exclude: &[],
        dir: DirName::ArchiveStem,
        checksum: false,
        about: "Proton for the Northstar client of Titanfall 2.",
    },
    Source {
        id: "luxtorpeda",
        name: "Luxtorpeda",
        repo: "luxtorpeda-dev/luxtorpeda",
        suffix: ".tar.xz",
        exclude: &[],
        dir: DirName::Fixed("luxtorpeda"),
        checksum: true,
        about: "Runs the native Linux engine of a game (Luxtorpeda packages) instead of the Windows build.",
    },
];

pub fn find(id: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.id == id)
}

pub fn default_source() -> &'static Source {
    &SOURCES[0]
}

/// Which source a directory name belongs to, for "update all".
pub fn family_of(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    // the RTSP builds are named like GE builds but are their own line: never mix them
    if n.contains("rtsp") {
        Some("rtsp")
    } else if n.starts_with("ge-proton") {
        Some("ge")
    } else if n.starts_with("proton-cachyos") {
        Some(if n.contains("v3") {
            "cachyos-v3"
        } else {
            "cachyos"
        })
    } else if n.contains("sarek") {
        Some("sarek")
    } else if n.contains("northstar") {
        Some("northstar")
    } else if n.contains("luxtorpeda") {
        Some("luxtorpeda")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_findable() {
        for (i, s) in SOURCES.iter().enumerate() {
            assert_eq!(find(s.id).unwrap().name, s.name);
            assert!(
                SOURCES[i + 1..].iter().all(|o| o.id != s.id),
                "duplicate id {}",
                s.id
            );
            assert!(s.repo.contains('/'));
        }
        assert!(find("nope").is_none());
    }
    #[test]
    fn rtsp_builds_are_not_taken_for_ge() {
        assert_eq!(family_of("GE-Proton9-27-rtsp15"), Some("rtsp"));
        assert_eq!(family_of("GE-Proton10-20"), Some("ge"));
    }
}
