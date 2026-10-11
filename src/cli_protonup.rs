//! protonup: Proton-GE manager for the Steam Deck. One binary: no arguments opens the window, with
//! arguments it is a command line tool.

use std::path::PathBuf;
use std::process::ExitCode;
use crate::{github, install, sources, steam};

macro_rules! say {
    ($($a:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($a)*);
    }};
}

const HELP: &str = "\
protonup - install and remove Proton-GE and other compatibility tools for Steam

usage:
  protonup [gui [TAB]]           open the window (TAB: installed, available, games)
  protonup sources               the tools it can fetch
  protonup list [-n N | --all]   releases of a source (default 15), installed ones marked
  protonup installed             what is in the compatibility tools directory
  protonup install [TAG]         install TAG, or the newest release when omitted or 'latest'
  protonup install --all [--yes] every release of the source that is not installed yet
  protonup remove NAME           remove an installed tool (its directory name)
  protonup path                  the directory used for installs
  protonup games                 installed games and the tool each one uses
  protonup use APPID TOOL        run a game with TOOL (a name from `installed`), or `default`; Steam must be closed

options:
  --source ID                    ge (default), cachyos, cachyos-v3, rtsp, sarek, northstar, luxtorpeda
  --dir PATH                     use this compatibilitytools.d instead of the detected Steam
  -y, --yes                      do not ask before `install --all`
  -j, --jobs N                   downloads at once (default 3, at most 8); interrupted ones resume
  -h, --help                     this text

Restart Steam after an install or removal; then pick the tool in a game's Properties > Compatibility.
Environment: PROTONUP_API redirects the GitHub API (testing, mirrors); GH_TOKEN raises the rate limit.";

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn take_opt(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    if let Some(i) = args.iter().position(|a| a == name) {
        if i + 1 >= args.len() {
            return Err(format!("{name} needs a value"));
        }
        let v = args.remove(i + 1);
        args.remove(i);
        return Ok(Some(v));
    }
    Ok(None)
}

fn take_flag(args: &mut Vec<String>, names: &[&str]) -> bool {
    let before = args.len();
    args.retain(|a| !names.contains(&a.as_str()));
    args.len() != before
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("protonup: {msg}");
    ExitCode::FAILURE
}

pub fn run(mut args: Vec<String>) -> ExitCode {
    let dir = match take_opt(&mut args, "--dir") {
        Ok(d) => d.map(PathBuf::from),
        Err(e) => return fail(e),
    };
    let source_id = match take_opt(&mut args, "--source") {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let all = take_flag(&mut args, &["--all"]);
    let mut jobs = 3usize;
    for name in ["-j", "--jobs"] {
        match take_opt(&mut args, name) {
            Ok(Some(v)) => match v.parse::<usize>() {
                Ok(n) if n >= 1 => jobs = n,
                _ => return fail(format!("{name} needs a number of at least 1")),
            },
            Ok(None) => {}
            Err(e) => return fail(e),
        }
    }
    let yes = take_flag(&mut args, &["-y", "--yes"]);
    let src = match source_id.as_deref() {
        None => sources::default_source(),
        Some(id) => match sources::find(id) {
            Some(s) => s,
            None => return fail(format!("unknown source '{id}' (see `protonup sources`)")),
        },
    };
    let dir_path = steam::default_tool_dir(&home(), dir.as_deref());

    let cmd = args.first().map(String::as_str).unwrap_or("gui");
    match cmd {
        "-h" | "--help" | "help" => {
            say!("{HELP}");
            ExitCode::SUCCESS
        }
        "path" => {
            say!("{}", dir_path.display());
            ExitCode::SUCCESS
        }
        "sources" => {
            for s in sources::SOURCES {
                say!("{:<11} {:<19} {}", s.id, s.name, s.about);
            }
            ExitCode::SUCCESS
        }
        "gui" => match crate::launcher::run(crate::launcher::Start { section: crate::launcher::Section::Games, dir: dir.is_some().then(|| dir_path.clone()), tab: args.get(1).cloned(), app: None }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
        "games" => {
            let Some(root) = dir_path.parent() else {
                return fail("no Steam root");
            };
            let map = crate::games::mapping(root);
            let tool = |id: u64| {
                map.get(&id)
                    .map(String::as_str)
                    .unwrap_or("(Steam default)")
                    .to_string()
            };
            say!("{:>9}  {:<40} {}", 0, "all games", tool(0));
            for g in crate::games::games(root) {
                say!("{:>9}  {:<40} {}", g.appid, g.name, tool(g.appid));
            }
            ExitCode::SUCCESS
        }
        "use" => {
            let (Some(id), Some(tool)) = (args.get(1), args.get(2)) else {
                return fail("use needs APPID TOOL|default");
            };
            let Ok(appid) = id.parse::<u64>() else {
                return fail("APPID is a number (see `protonup games`)");
            };
            let Some(root) = dir_path.parent() else {
                return fail("no Steam root");
            };
            let chosen = if tool == "default" {
                None
            } else {
                let have = install::list_installed(&dir_path);
                match have.iter().find(|i| &i.name == tool || &i.tool == tool) {
                    Some(i) => Some(i.tool.clone()),
                    None => {
                        return fail(format!(
                            "'{tool}' is not installed here (see `protonup installed`)"
                        ))
                    }
                }
            };
            match crate::games::set_tool(root, appid, chosen.as_deref()) {
                Ok(()) => {
                    say!("{appid}: {}", chosen.as_deref().unwrap_or("Steam default"));
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        "installed" => {
            let l = install::list_installed(&dir_path);
            if l.is_empty() {
                say!("nothing installed in {}", dir_path.display());
            }
            for i in l {
                say!("{:<44} {}", i.name, crate::ui::human_bytes(i.size));
            }
            ExitCode::SUCCESS
        }
        "list" => {
            let n = if all {
                usize::MAX
            } else {
                args.iter()
                    .position(|a| a == "-n")
                    .and_then(|i| args.get(i + 1))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(15)
            };
            match github::releases(src, n) {
                Ok(rs) => {
                    let have: Vec<String> = install::list_installed(&dir_path)
                        .into_iter()
                        .map(|i| i.name)
                        .collect();
                    for r in rs {
                        let size = r
                            .archive(src)
                            .map(|a| crate::ui::human_bytes(a.size))
                            .unwrap_or_default();
                        let mark = if have.contains(&r.dir_name(src)) {
                            "installed"
                        } else {
                            ""
                        };
                        say!("{:<44} {}  {:>9}  {}", r.tag_name, r.date(), size, mark);
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        "install" => {
            let want = args.get(1).map(String::as_str).unwrap_or("latest");
            let rs = match github::releases(src, if all { usize::MAX } else { 100 }) {
                Ok(r) => r,
                Err(e) => return fail(e),
            };
            let have: Vec<String> = install::list_installed(&dir_path)
                .into_iter()
                .map(|i| i.name)
                .collect();
            let todo: Vec<&github::Release> = if all {
                rs.iter()
                    .filter(|r| {
                        !have.contains(&r.dir_name(src))
                            || matches!(src.dir, sources::DirName::Fixed(_))
                    })
                    .collect()
            } else {
                let one = if want == "latest" {
                    rs.first()
                } else {
                    rs.iter()
                        .find(|r| r.tag_name == want || r.dir_name(src) == want)
                };
                match one {
                    Some(r) => vec![r],
                    None => {
                        return fail(format!(
                            "no {} release '{want}' (see `protonup list`)",
                            src.name
                        ))
                    }
                }
            };
            if todo.is_empty() {
                say!(
                    "nothing to install: all {} releases are already there",
                    src.name
                );
                return ExitCode::SUCCESS;
            }
            if all {
                let total: u64 = todo
                    .iter()
                    .filter_map(|r| r.archive(src))
                    .map(|a| a.size)
                    .sum();
                say!(
                    "{} releases of {} not installed yet, {} to download (more once unpacked)",
                    todo.len(),
                    src.name,
                    crate::ui::human_bytes(total)
                );
                if !yes {
                    print!("continue? [y/N] ");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    let mut line = String::new();
                    let _ = std::io::stdin().read_line(&mut line);
                    if !line.trim().eq_ignore_ascii_case("y") {
                        say!("cancelled");
                        return ExitCode::SUCCESS;
                    }
                }
            }
            // dspq style: the list is a FIFO, `jobs` workers pull from it, one aggregate progress line
            let jobs = jobs.clamp(1, 8).min(todo.len());
            let next = std::sync::atomic::AtomicUsize::new(0);
            let finished = std::sync::atomic::AtomicUsize::new(0);
            let failed = std::sync::atomic::AtomicUsize::new(0);
            let have_bytes: Vec<std::sync::atomic::AtomicU64> = todo
                .iter()
                .map(|_| std::sync::atomic::AtomicU64::new(0))
                .collect();
            let sizes: Vec<u64> = todo
                .iter()
                .map(|r| r.archive(src).map(|a| a.size).unwrap_or(0))
                .collect();
            let out = std::sync::Mutex::new(());
            let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
            let ord = std::sync::atomic::Ordering::SeqCst;
            say!(
                "installing {} release(s) into {}, {} at a time",
                todo.len(),
                dir_path.display(),
                jobs
            );
            std::thread::scope(|sc| {
                for _ in 0..jobs {
                    sc.spawn(|| loop {
                        let i = next.fetch_add(1, ord);
                        let Some(rel) = todo.get(i) else { break };
                        let res = install::install(src, rel, &dir_path, &mut |p| {
                            if let install::Progress::Download { done, .. } = p {
                                have_bytes[i].store(done, ord);
                            }
                        });
                        let _g = out.lock();
                        if tty {
                            eprint!("\r{:78}\r", "");
                        }
                        match res {
                            Ok(o) => say!(
                                "installed {}{}",
                                o.path.display(),
                                if o.verified {
                                    ""
                                } else {
                                    "  (the project publishes no checksum: not verified)"
                                }
                            ),
                            Err(e) => {
                                failed.fetch_add(1, ord);
                                eprintln!("protonup: {}: {e}", rel.tag_name);
                            }
                        }
                        finished.fetch_add(1, ord);
                    });
                }
                while finished.load(ord) < todo.len() {
                    let have: u64 = have_bytes.iter().map(|b| b.load(ord)).sum();
                    let want: u64 = sizes.iter().sum();
                    let _g = out.lock();
                    if tty {
                        eprint!(
                            "\r  queue: {}/{} done   {} / {}   -j{}  ",
                            finished.load(ord),
                            todo.len(),
                            crate::ui::human_bytes(have),
                            crate::ui::human_bytes(want),
                            jobs
                        );
                    }
                    drop(_g);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            });
            if tty {
                eprint!("\r{:78}\r", "");
            }
            let failed = failed.load(ord);
            if failed > 0 {
                return ExitCode::FAILURE;
            }
            say!("restart Steam to use it");
            ExitCode::SUCCESS
        }
        "remove" => {
            let Some(name) = args.get(1) else {
                eprintln!("protonup: remove needs a name (see `protonup installed`)");
                return ExitCode::from(2);
            };
            match install::remove(&dir_path, name) {
                Ok(()) => {
                    say!("removed {name}; restart Steam");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        other => {
            eprintln!("protonup: unknown command '{other}'\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}
