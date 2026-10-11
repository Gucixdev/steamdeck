//! The two command lines run as real processes, in a throw-away HOME with no `flatpak` on PATH and the
//! GitHub API pointed at a closed port: argument handling, exit codes, and what ends up on disk.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Env {
    home: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let home =
            std::env::temp_dir().join(format!("steamdeck-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("bin")).unwrap();
        Env { home }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_steamdeck"))
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", self.home.join("bin"))
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env("XDG_STATE_HOME", self.home.join(".local/state"))
            .env("FLATPAK_USER_DIR", self.home.join("fp"))
            .env("PROTONUP_API", "http://127.0.0.1:9")
            .env("STEAMDECK_STEAM_RUNNING", "0")
            .output()
            .unwrap()
    }

    fn overrides(&self, id: &str) -> Option<String> {
        std::fs::read_to_string(self.home.join("fp/overrides").join(id)).ok()
    }

    fn tools(&self) -> PathBuf {
        self.home.join("steam/compatibilitytools.d")
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}
fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

#[test]
fn help_and_version_and_unknown_commands() {
    let e = Env::new("help");
    let o = e.run(&["--help"]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).contains("protonup"));
    let o = e.run(&["--version"]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).starts_with("steamdeck 0."), "{}", out(&o));
    let o = e.run(&["bogus"]);
    assert_eq!(code(&o), 2, "an unknown word must not open the window");
    assert!(err(&o).contains("unknown command"));
    assert_eq!(code(&e.run(&["protonup", "bogus"])), 2);
    assert_eq!(code(&e.run(&["flatseal", "bogus"])), 2);
    assert_eq!(code(&e.run(&["protonup", "--bogus"])), 2);
}

#[test]
fn protonup_option_errors_are_reported_before_any_work() {
    let e = Env::new("popts");
    for args in [
        vec!["protonup", "path", "--dir"],
        vec!["protonup", "path", "--dir="],
        vec!["protonup", "path", "--source"],
        vec!["protonup", "path", "--source", "nope"],
        vec!["protonup", "path", "-j", "0"],
        vec!["protonup", "path", "-j", "x"],
        vec!["protonup", "path", "--jobs", "-3"],
        vec!["protonup", "list", "-n", "abc"],
        vec!["protonup", "list", "-n", "0"],
        vec!["protonup", "list", "-n"],
        vec!["protonup", "use"],
        vec!["protonup", "use", "570"],
        vec!["protonup", "use", "abc", "default"],
    ] {
        let o = e.run(&args);
        assert_eq!(code(&o), 1, "{args:?}: {} {}", out(&o), err(&o));
        assert!(!err(&o).is_empty(), "{args:?} gave no reason");
    }
}

#[test]
fn dir_and_source_accept_the_equals_form() {
    let e = Env::new("eq");
    let o = e.run(&["protonup", "path", "--dir=/x/compatibilitytools.d"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(out(&o).trim(), "/x/compatibilitytools.d");
    assert_eq!(code(&e.run(&["protonup", "path", "--source=cachyos"])), 0);
    assert_eq!(code(&e.run(&["protonup", "path", "--source=nope"])), 1);
}

#[test]
fn installed_and_remove_work_on_a_directory_and_refuse_odd_names() {
    let e = Env::new("inst");
    let t = e.tools();
    std::fs::create_dir_all(t.join("GE-Proton10-1")).unwrap();
    std::fs::write(t.join("GE-Proton10-1/compatibilitytool.vdf"), "").unwrap();
    std::fs::create_dir_all(t.join("precious")).unwrap();
    std::fs::write(t.join("precious/file"), "keep").unwrap();
    let d = t.to_string_lossy().to_string();

    let o = e.run(&["protonup", "installed", "--dir", &d]);
    assert_eq!(code(&o), 0);
    assert!(
        out(&o).contains("GE-Proton10-1") && !out(&o).contains("precious"),
        "{}",
        out(&o)
    );

    for bad in [
        "precious",
        "..",
        "../steam",
        "GE-Proton10-1/..",
        "",
        ".hidden",
        "missing",
    ] {
        let o = e.run(&["protonup", "remove", bad, "--dir", &d]);
        assert_ne!(code(&o), 0, "'{bad}' must be refused");
    }
    assert!(
        t.join("precious/file").is_file(),
        "a folder that is not a tool survives"
    );
    assert!(t.join("GE-Proton10-1").is_dir());

    assert_eq!(
        code(&e.run(&["protonup", "remove", "--dir", &d])),
        2,
        "remove without a name"
    );
    let o = e.run(&["protonup", "remove", "GE-Proton10-1", "--dir", &d]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!t.join("GE-Proton10-1").exists());
}

#[test]
fn installed_on_a_missing_directory_is_not_an_error() {
    let e = Env::new("nodir");
    let d = e.tools().to_string_lossy().to_string();
    let o = e.run(&["protonup", "installed", "--dir", &d]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).contains("nothing installed"));
}

#[test]
fn network_failures_are_an_error_with_a_reason_not_a_hang_or_a_panic() {
    let e = Env::new("net");
    let d = e.tools().to_string_lossy().to_string();
    for args in [
        vec!["protonup", "list", "--dir", &d],
        vec!["protonup", "install", "--dir", &d],
        vec!["protonup", "install", "GE-Proton10-1", "--dir", &d],
        vec!["protonup", "install", "--all", "--yes", "--dir", &d],
    ] {
        let o = e.run(&args);
        assert_eq!(code(&o), 1, "{args:?}: {}", err(&o));
        assert!(err(&o).starts_with("protonup:"), "{}", err(&o));
        assert!(!err(&o).contains("panicked"));
    }
    assert!(!e.tools().exists(), "a failed install creates nothing");
}

#[test]
fn use_checks_the_tool_and_games_lists_the_default() {
    let e = Env::new("use");
    let root = e.home.join("steam");
    let t = root.join("compatibilitytools.d");
    std::fs::create_dir_all(t.join("GE-Proton10-1")).unwrap();
    std::fs::write(
        t.join("GE-Proton10-1/compatibilitytool.vdf"),
        "\"compatibilitytools\"\n{\n\"compat_tools\"\n{\n\"GE-Proton10-1\"\n{\n\"install_path\" \".\"\n}\n}\n}\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("config")).unwrap();
    let cfg = root.join("config/config.vdf");
    std::fs::write(&cfg, "\"InstallConfigStore\"\n{\n}\n").unwrap();
    let d = t.to_string_lossy().to_string();

    let o = e.run(&["protonup", "use", "570", "Not-Installed", "--dir", &d]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("not installed"), "{}", err(&o));
    assert_eq!(
        std::fs::read_to_string(&cfg).unwrap(),
        "\"InstallConfigStore\"\n{\n}\n"
    );

    let o = e.run(&["protonup", "use", "570", "GE-Proton10-1", "--dir", &d]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let o = e.run(&["protonup", "games", "--dir", &d]);
    assert!(out(&o).contains("all games"), "{}", out(&o));
    let after = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        after.contains("\"570\"") && after.contains("GE-Proton10-1"),
        "{after}"
    );

    let o = e.run(&["protonup", "use", "570", "default", "--dir", &d]);
    assert_eq!(code(&o), 0);
    assert!(!std::fs::read_to_string(&cfg).unwrap().contains("\"570\""));
}

#[test]
fn use_while_steam_runs_changes_nothing() {
    let e = Env::new("running");
    let root = e.home.join("steam");
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::create_dir_all(root.join("compatibilitytools.d")).unwrap();
    let cfg = root.join("config/config.vdf");
    std::fs::write(&cfg, "\"InstallConfigStore\"\n{\n}\n").unwrap();
    let d = root
        .join("compatibilitytools.d")
        .to_string_lossy()
        .to_string();
    let o = Command::new(env!("CARGO_BIN_EXE_steamdeck"))
        .args(["protonup", "use", "570", "default", "--dir", &d])
        .env_clear()
        .env("HOME", &e.home)
        .env("PATH", e.home.join("bin"))
        .env("STEAMDECK_STEAM_RUNNING", "1")
        .output()
        .unwrap();
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("Steam is running"), "{}", err(&o));
    assert_eq!(
        std::fs::read_to_string(&cfg).unwrap(),
        "\"InstallConfigStore\"\n{\n}\n"
    );
}

#[test]
fn flatseal_allow_deny_default_write_the_expected_file() {
    let e = Env::new("fs");
    let id = "org.test.App";
    assert_eq!(
        code(&e.run(&["flatseal", "allow", id, "sockets", "x11"])),
        0
    );
    assert_eq!(e.overrides(id).unwrap(), "[Context]\nsockets=x11;\n");
    assert_eq!(
        code(&e.run(&["flatseal", "deny", id, "shared", "network"])),
        0
    );
    assert_eq!(
        e.overrides(id).unwrap(),
        "[Context]\nsockets=x11;\nshared=!network;\n"
    );
    assert_eq!(
        code(&e.run(&["flatseal", "default", id, "sockets", "x11"])),
        0
    );
    assert_eq!(e.overrides(id).unwrap(), "[Context]\nshared=!network;\n");
    assert_eq!(code(&e.run(&["flatseal", "reset", id])), 0);
    assert!(
        e.overrides(id).is_none(),
        "an empty override is removed, not left as an empty file"
    );
}

#[test]
fn flatseal_refuses_names_that_would_corrupt_the_override_file() {
    let e = Env::new("fsbad");
    let id = "org.test.App";
    for args in [
        vec!["flatseal", "allow", id, "sockets", "x11;wayland"],
        vec!["flatseal", "allow", id, "sockets", "!x11"],
        vec!["flatseal", "allow", id, "sockets", ""],
        vec!["flatseal", "allow", id, "sockets", " x11"],
        vec!["flatseal", "allow", id, "bogus", "x11"],
        vec!["flatseal", "allow", "../evil", "sockets", "x11"],
        vec!["flatseal", "allow", "a/b", "sockets", "x11"],
        vec!["flatseal", "allow", "-rf", "sockets", "x11"],
        vec!["flatseal", "fs-add", id, "/mnt/a;/mnt/b"],
        vec!["flatseal", "fs-add", id, "!/mnt/a"],
        vec!["flatseal", "fs-add", id, ""],
        vec!["flatseal", "show", "../x"],
        vec!["flatseal", "portals", "../x"],
        vec!["flatseal", "portal", id, "nope", "yes"],
        vec!["flatseal", "portal", id, "camera", "maybe"],
        vec!["flatseal", "reset", "../x"],
        vec!["flatseal", "allow", id],
        vec!["flatseal", "allow"],
    ] {
        let o = e.run(&args);
        assert_eq!(code(&o), 1, "{args:?} -> {} {}", out(&o), err(&o));
        assert!(!err(&o).is_empty(), "{args:?} gave no reason");
    }
    assert!(!e.home.join("fp").exists(), "nothing was written anywhere");
    assert!(!e.home.join("evil").exists());
}

#[test]
fn flatseal_filesystem_paths_keep_their_mode_and_can_be_removed() {
    let e = Env::new("fspath");
    let id = "org.test.App";
    assert_eq!(
        code(&e.run(&["flatseal", "fs-add", id, "~/My Games:rw"])),
        0
    );
    assert_eq!(code(&e.run(&["flatseal", "fs-add", id, "/mnt/sd:ro"])), 0);
    let f = e.overrides(id).unwrap();
    assert!(
        f.contains("~/My Games:rw;") && f.contains("/mnt/sd:ro;"),
        "{f}"
    );
    let o = e.run(&["flatseal", "show", id]);
    assert!(
        out(&o).contains("~/My Games:rw") && out(&o).contains("/mnt/sd:ro"),
        "{}",
        out(&o)
    );
    assert_eq!(
        code(&e.run(&["flatseal", "fs-remove", id, "~/My Games:rw"])),
        0
    );
    assert_eq!(
        code(&e.run(&["flatseal", "fs-remove", id, "/mnt/sd:ro"])),
        0
    );
    assert!(e.overrides(id).is_none());
}

#[test]
fn flatseal_keeps_what_the_user_wrote_by_hand() {
    let e = Env::new("hand");
    let id = "org.test.App";
    let dir = e.home.join("fp/overrides");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(id),
        "# my notes\n[Context]\nsockets=wayland;\n\n[Environment]\nFOO=a=b\n\n[Session Bus Policy]\norg.x.Y=talk\n",
    )
    .unwrap();
    assert_eq!(
        code(&e.run(&["flatseal", "allow", id, "devices", "dri"])),
        0
    );
    let f = e.overrides(id).unwrap();
    for keep in [
        "# my notes",
        "sockets=wayland;",
        "FOO=a=b",
        "org.x.Y=talk",
        "devices=dri;",
    ] {
        assert!(f.contains(keep), "'{keep}' lost:\n{f}");
    }
}

#[test]
fn a_symlinked_override_is_not_followed_out_of_the_overrides_folder() {
    let e = Env::new("link");
    let id = "org.test.App";
    let dir = e.home.join("fp/overrides");
    std::fs::create_dir_all(&dir).unwrap();
    let victim: &Path = &e.home.join("victim");
    std::fs::write(victim, "important").unwrap();
    std::os::unix::fs::symlink(victim, dir.join(id)).unwrap();
    let _ = e.run(&["flatseal", "allow", id, "sockets", "x11"]);
    assert_eq!(
        std::fs::read_to_string(victim).unwrap(),
        "important",
        "the file the symlink pointed at is not overwritten"
    );
}
