//! The whole window, headless: real `view` rendered by iced's test simulator, real clicks on the
//! buttons by their text, every message and every background task run to completion, against a fake
//! `flatpak`, a fake Steam folder and a local GitHub. Nothing here touches the real system.
//!
//! This is what replaces clicking through the interface by hand.
use iced::futures::StreamExt;
use iced::Task;
use iced_test::Simulator;
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use steamdeck::launcher::{App, Msg, Section, Start};

// ---- the fake world ------------------------------------------------------------------------------

fn base() -> &'static PathBuf {
    static B: OnceLock<PathBuf> = OnceLock::new();
    B.get_or_init(|| {
        let b = std::env::temp_dir().join(format!("sd-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&b);
        fs::create_dir_all(&b).unwrap();
        b
    })
}

const FAKE_FLATPAK: &str = r#"#!/bin/sh
# a stand-in for flatpak: state lives in $FAKE_FP
S="$FAKE_FP"
echo "$*" >> "$S/calls"
case "$1" in
  list) cat "$S/apps" 2>/dev/null ;;
  uninstall)
    shift
    del=0; id=
    for a in "$@"; do case "$a" in --delete-data) del=1 ;; -*) ;; *) id=$a ;; esac; done
    case " $* " in *" --unused "*) echo "Nothing unused" ; exit 0 ;; esac
    grep -v "^$id	" "$S/apps" > "$S/apps.new"; mv "$S/apps.new" "$S/apps"
    [ "$del" = 1 ] && rm -rf "$HOME/.var/app/$id"
    echo "Uninstall complete." ;;
  update)
    echo "Updating app/org.a.A/x86_64/stable"
    echo "Updating runtime/org.freedesktop.Platform/x86_64/24.08" ;;
  remote-ls) : ;;
  info) printf '[Context]\nshared=network;\n' ;;
  permission-show) grep "	$2	" "$S/store" 2>/dev/null | awk -F'\t' '{print $1"\t"$2"\t"$3"\t"$4"\t"}' | awk -v app="$2" 'BEGIN{FS=OFS="\t"} {print}' ;;
  permission-set) grep -v "^$2	$3	$4	" "$S/store" > "$S/store.new" 2>/dev/null; mv "$S/store.new" "$S/store"; printf '%s\t%s\t%s\t%s\n' "$2" "$3" "$4" "$5" >> "$S/store" ;;
  permission-remove) grep -v "^$2	$3	$4	" "$S/store" > "$S/store.new" 2>/dev/null; mv "$S/store.new" "$S/store" ;;
  permission-reset) grep -v "	$2	" "$S/store" > "$S/store.new" 2>/dev/null; mv "$S/store.new" "$S/store" ;;
  *) echo "fake flatpak: $*" >&2 ;;
esac
"#;

struct World {
    _guard: MutexGuard<'static, ()>,
    home: PathBuf,
    fp: PathBuf,
    api: String,
}

fn serve_github() -> String {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        // GE-Proton10-20, a real tiny archive with its checksum
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        let mut add = |path: &str, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, path, data).unwrap();
        };
        add("GE-Proton10-20/compatibilitytool.vdf", b"\"compatibilitytools\"\n{\n\t\"compat_tools\"\n\t{\n\t\t\"GE-Proton10-20\"\n\t\t{\n\t\t\t\"from_oslist\"\t\"windows\"\n\t\t}\n\t}\n}\n");
        add("GE-Proton10-20/proton", b"#!/bin/sh\n");
        let tar = b.into_inner().unwrap().finish().unwrap();
        use sha2::{Digest, Sha512};
        let sum: String = Sha512::digest(&tar).iter().map(|x| format!("{x:02x}")).collect();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let release = format!(
            "[{{\"tag_name\":\"GE-Proton10-20\",\"published_at\":\"2026-09-01T00:00:00Z\",\"prerelease\":false,\"assets\":[\
             {{\"name\":\"GE-Proton10-20.tar.gz\",\"browser_download_url\":\"{base}/GE-Proton10-20.tar.gz\",\"size\":{}}},\
             {{\"name\":\"GE-Proton10-20.sha512sum\",\"browser_download_url\":\"{base}/GE-Proton10-20.sha512sum\",\"size\":150}}]}}]",
            tar.len()
        );
        let files: Arc<Vec<(String, Vec<u8>)>> = Arc::new(vec![
            ("/GE-Proton10-20.tar.gz".into(), tar),
            ("/GE-Proton10-20.sha512sum".into(), format!("{sum}  GE-Proton10-20.tar.gz\n").into_bytes()),
        ]);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let files = files.clone();
                let release = release.clone();
                std::thread::spawn(move || {
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
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let body: Option<Vec<u8>> = if path.contains("/releases") {
                        Some(release.into_bytes())
                    } else {
                        files.iter().find(|(p, _)| *p == path).map(|(_, d)| d.clone())
                    };
                    match body {
                        Some(d) => {
                            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", d.len());
                            let _ = s.write_all(&d);
                        }
                        None => {
                            let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        }
                    }
                });
            }
        });
        base
    })
    .clone()
}

fn vdf_tool(root: &Path, name: &str) {
    let d = root.join("compatibilitytools.d").join(name);
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("compatibilitytool.vdf"),
        format!("\"compatibilitytools\"\n{{\n\t\"compat_tools\"\n\t{{\n\t\t\"{name}\"\n\t\t{{\n\t\t\t\"from_oslist\"\t\"windows\"\n\t\t}}\n\t}}\n}}\n"),
    )
    .unwrap();
}

/// A clean fake machine for one test. Tests run one at a time: they share the process environment.
fn world() -> World {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let b = base().clone();
    let home = b.join("home");
    let fp = b.join("fp");
    let bin = b.join("bin");
    for d in [&home, &fp] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).unwrap();
    }
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("flatpak"), FAKE_FLATPAK).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin.join("flatpak"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    // two apps with data, one leftover directory
    fs::write(
        fp.join("apps"),
        "org.a.A\tAlpha App\t10 MB\tuser\norg.b.B\tBeta App\t20 MB\tuser\n",
    )
    .unwrap();
    for id in ["org.a.A", "org.b.B", "org.gone.Gone"] {
        let d = home.join(".var/app").join(id).join("config");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("data"), id).unwrap();
    }
    // a Steam with one game, two Proton-GE versions, the game set to the older one
    let steam = home.join(".local/share/Steam");
    fs::create_dir_all(steam.join("steamapps")).unwrap();
    fs::create_dir_all(steam.join("config")).unwrap();
    fs::write(
        steam.join("steamapps/appmanifest_570.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"570\"\n\t\"name\"\t\t\"Dota 2\"\n}\n",
    )
    .unwrap();
    vdf_tool(&steam, "GE-Proton9-27");
    vdf_tool(&steam, "GE-Proton10-19");
    fs::write(
        steam.join("config/config.vdf"),
        "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"570\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"GE-Proton9-27\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"250\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n",
    )
    .unwrap();
    let api = serve_github();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // set once per test; every test sets the same values
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
    std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));
    std::env::set_var("XDG_STATE_HOME", home.join(".local/state"));
    std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
    std::env::set_var("FLATPAK_USER_DIR", home.join(".local/share/flatpak"));
    std::env::set_var("PATH", path);
    std::env::set_var("FAKE_FP", &fp);
    std::env::set_var("PROTONUP_API", &api);
    std::env::set_var("STEAMDECK_STEAM_RUNNING", "0");
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    World {
        _guard: guard,
        home,
        fp,
        api,
    }
}

// ---- driving the window ------------------------------------------------------------------------------

/// Run a task to its end and return the messages it produced.
fn collect(task: Task<Msg>) -> Vec<Msg> {
    let Some(mut stream) = iced_runtime::task::into_stream(task) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    iced::futures::executor::block_on(async {
        while let Some(a) = stream.next().await {
            if let iced_runtime::Action::Output(m) = a {
                out.push(m);
            }
        }
    });
    out
}

/// A window as big as the page is long, so that nothing is below the fold for the simulated mouse.
fn tall<'a>(view: iced::Element<'a, Msg>) -> Simulator<'a, Msg> {
    Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(1280.0, 4000.0),
        view,
    )
}

struct Window {
    app: App,
}

impl Window {
    fn open(start: Start) -> Window {
        let (app, task) = App::new(start);
        let mut w = Window { app };
        w.feed(collect(task));
        w
    }

    /// Deliver messages and everything they cause, until nothing is left.
    fn feed(&mut self, msgs: Vec<Msg>) {
        let mut q: VecDeque<Msg> = msgs.into();
        let mut guard = 0;
        while let Some(m) = q.pop_front() {
            guard += 1;
            assert!(guard < 2000, "the window keeps producing messages");
            let t = self.app.update(m);
            q.extend(collect(t));
        }
    }

    /// Click the button or text with this label, like a user would.
    fn click(&mut self, label: &str) {
        let msgs: Vec<Msg> = {
            let mut sim = tall(self.app.view());
            sim.click(label)
                .unwrap_or_else(|e| panic!("no '{label}' on screen: {e:?}"));
            sim.into_messages().collect()
        };
        assert!(!msgs.is_empty(), "clicking '{label}' did nothing");
        self.feed(msgs);
    }

    fn has(&mut self, label: &str) -> bool {
        let mut sim = tall(self.app.view());
        sim.find(label).is_ok()
    }
}

fn calls(w: &World) -> String {
    fs::read_to_string(w.fp.join("calls")).unwrap_or_default()
}

// ---- the tests ---------------------------------------------------------------------------------------

#[test]
fn flatpak_apps_uninstall_after_the_question_and_only_that_app() {
    let w = world();
    let mut win = Window::open(Start {
        section: Section::Apps,
        ..Start::default()
    });
    assert!(win.has("Alpha App") && win.has("Beta App"));
    win.click("Uninstall"); // the first row's button
    assert!(
        calls(&w).is_empty() || !calls(&w).contains("uninstall"),
        "asking is not doing"
    );
    win.click("Yes");
    let c = calls(&w);
    assert!(c.contains("uninstall -y --user org.a.A"), "{c}");
    assert!(!c.contains("org.b.B"), "{c}");
    assert!(!win.has("Alpha App"), "the list reloads without it");
    assert!(win.has("Beta App"));
    assert!(
        w.home.join(".var/app/org.a.A").exists(),
        "uninstall alone keeps the data"
    );
}

#[test]
fn clear_data_removes_only_that_apps_data_and_leftovers_can_be_cleared() {
    let w = world();
    let mut win = Window::open(Start {
        section: Section::Apps,
        ..Start::default()
    });
    win.click("Clear data");
    win.click("Yes");
    assert!(!w.home.join(".var/app/org.a.A").exists());
    assert!(
        w.home.join(".var/app/org.b.B/config/data").is_file(),
        "the other app keeps its data"
    );
    assert!(
        calls(&w).is_empty() || !calls(&w).contains("uninstall"),
        "no uninstall happened"
    );
    // the data of an app that is gone is listed and can be cleared too
    assert!(win.has("org.gone.Gone"));
}

#[test]
fn saying_no_changes_nothing() {
    let w = world();
    let mut win = Window::open(Start {
        section: Section::Apps,
        ..Start::default()
    });
    win.click("Uninstall");
    win.click("No");
    assert!(!calls(&w).contains("uninstall"));
    assert!(w.home.join(".var/app/org.a.A/config/data").is_file());
}

#[test]
fn update_installs_the_newest_proton_removes_the_old_and_moves_the_game() {
    let w = world();
    let mut win = Window::open(Start::default());
    win.click("Update"); // the first tile: Update Protons
    let steam = w.home.join(".local/share/Steam");
    assert!(
        steam
            .join("compatibilitytools.d/GE-Proton10-20/compatibilitytool.vdf")
            .is_file(),
        "the newest is installed"
    );
    assert!(
        !steam.join("compatibilitytools.d/GE-Proton9-27").exists(),
        "the old ones are removed"
    );
    assert!(!steam.join("compatibilitytools.d/GE-Proton10-19").exists());
    let cfg = fs::read_to_string(steam.join("config/config.vdf")).unwrap();
    assert!(
        cfg.contains("GE-Proton10-20") && !cfg.contains("GE-Proton9-27"),
        "the game moved to the new version:\n{cfg}"
    );
    // no partial files left
    let left: Vec<String> = fs::read_dir(steam.join("compatibilitytools.d"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(left, ["GE-Proton10-20"], "{left:?}");
}

#[test]
fn update_flatpaks_runs_flatpak_update_for_the_user_installation() {
    let w = world();
    let mut win = Window::open(Start::default());
    win.feed(vec![Msg::UpdateFlatpaks]);
    let c = calls(&w);
    assert!(c.contains("update --user -y --noninteractive"), "{c}");
    assert!(
        !c.contains("update --system"),
        "the system installation is not updated when it has nothing: {c}"
    );
}

#[test]
fn a_permission_is_written_as_an_override_for_that_app_only() {
    let w = world();
    let mut win = Window::open(Start {
        section: Section::Perms,
        app: Some("org.a.A".into()),
        ..Start::default()
    });
    assert!(win.has("network"));
    // the first "off" button on the page is the network permission
    win.click("off");
    let ov = w.home.join(".local/share/flatpak/overrides/org.a.A");
    let text = fs::read_to_string(&ov).unwrap_or_default();
    assert!(text.contains("!network"), "{text}");
    assert!(!w
        .home
        .join(".local/share/flatpak/overrides/org.b.B")
        .exists());
}

#[test]
fn portals_for_one_app_go_to_the_permission_store() {
    let w = world();
    let mut win = Window::open(Start {
        section: Section::Perms,
        app: Some("org.a.A".into()),
        ..Start::default()
    });
    assert!(win.has("background"));
    win.click("no"); // the first "no" is the background portal
    let store = fs::read_to_string(w.fp.join("store")).unwrap_or_default();
    assert!(
        store.contains("background\tbackground\torg.a.A\tno"),
        "{store}"
    );
    assert!(!store.contains("org.b.B"));
}

#[test]
fn the_game_list_shows_the_game_and_its_proton_and_a_change_is_saved() {
    let w = world();
    let mut win = Window::open(Start {
        section: Section::Games,
        ..Start::default()
    });
    assert!(win.has("Dota 2"));
    // the dropdown itself is a pick_list; the message it sends is what is checked here
    let root = w.home.join(".local/share/Steam");
    win.feed(vec![Msg::Proton(steamdeck::proton_gui::Msg::SetTool(
        root.clone(),
        570,
        "GE-Proton10-19".into(),
    ))]);
    let cfg = fs::read_to_string(root.join("config/config.vdf")).unwrap();
    assert!(
        cfg.contains("GE-Proton10-19") && !cfg.contains("GE-Proton9-27"),
        "{cfg}"
    );
    assert!(
        root.join("config/config.vdf.steamdeck-bak").is_file(),
        "a backup of the previous file"
    );
}

#[test]
fn the_other_pages_are_not_offered() {
    let _w = world();
    let mut win = Window::open(Start::default());
    for gone in ["Settings", "Scripts", "System tweaks", "Health check"] {
        assert!(!win.has(gone), "{gone} must not be on the screen");
    }
    for here in ["Home", "ProtonUp", "Flatseal", "Log"] {
        assert!(win.has(here), "{here}");
    }
    let _ = &_w.api;
}

#[test]
fn the_data_of_a_removed_app_can_be_cleared_and_nothing_else_goes() {
    let w = world();
    let mut win = Window::open(Start {
        section: Section::Apps,
        ..Start::default()
    });
    assert!(win.has("data of a removed app"));
    // the leftover is the third row (two installed apps come first); asking for it is what its button does
    win.feed(vec![Msg::Apps(steamdeck::flatapps::Msg::Ask(
        2,
        steamdeck::flatapps::Action::DeleteData,
    ))]);
    win.click("Yes");
    assert!(!w.home.join(".var/app/org.gone.Gone").exists());
    assert!(w.home.join(".var/app/org.a.A/config/data").is_file());
    assert!(w.home.join(".var/app/org.b.B/config/data").is_file());
}

#[test]
fn big_pages_draw_fast_enough_not_to_stutter() {
    let w = world();
    // 250 apps and 250 games: far more than a real Deck has
    let mut apps = String::new();
    for i in 0..250 {
        apps.push_str(&format!("org.big.App{i}\tBig App {i}\t{i} MB\tuser\n"));
        let d = w
            .home
            .join(".var/app")
            .join(format!("org.big.App{i}/config"));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("f"), "x").unwrap();
    }
    fs::write(w.fp.join("apps"), apps).unwrap();
    let steam = w.home.join(".local/share/Steam");
    for i in 0..250 {
        fs::write(
            steam.join(format!("steamapps/appmanifest_{}.acf", 1000 + i)),
            format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{}\"\n\t\"name\"\t\t\"Game {i}\"\n}}\n",
                1000 + i
            ),
        )
        .unwrap();
    }
    let time = |win: &mut Window| {
        let t = std::time::Instant::now();
        for _ in 0..3 {
            let sim = tall(win.app.view());
            drop(sim);
        }
        t.elapsed().as_millis() / 3
    };
    let mut apps_page = Window::open(Start {
        section: Section::Apps,
        ..Start::default()
    });
    let mut games_page = Window::open(Start {
        section: Section::Games,
        ..Start::default()
    });
    let mut get_page = Window::open(Start {
        section: Section::Get,
        ..Start::default()
    });
    // the first render of a process loads fonts and shapes text: not what a user feels page to page
    drop(tall(apps_page.app.view()));
    let a = time(&mut apps_page);
    let g = time(&mut games_page);
    let d = time(&mut get_page);
    eprintln!("draw time (ms): apps {a}, games {g}, download {d}");
    for (name, ms) in [("Flatpak apps", a), ("Games", g), ("Download", d)] {
        assert!(
            ms < 400,
            "{name} page takes {ms} ms to build and lay out: it would stutter"
        );
    }
}
