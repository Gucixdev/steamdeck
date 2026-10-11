//! protonup: Proton-GE manager for the Steam Deck. One binary: no arguments opens the window, with
//! arguments it is a command line tool.
mod gui;

use protonup::{github, install, sources, steam};
use std::path::PathBuf;
use std::process::ExitCode;

macro_rules! say {
    ($($a:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($a)*);
    }};
}

const HELP: &str = "\
protonup - install and remove Proton-GE and other compatibility tools for Steam

usage:
  protonup                       open the window
  protonup sources               the tools it can fetch
  protonup list [-n N | --all]   releases of a source (default 15), installed ones marked
  protonup installed             what is in the compatibility tools directory
  protonup install [TAG]         install TAG, or the newest release when omitted or 'latest'
  protonup install --all [--yes] every release of the source that is not installed yet
  protonup remove NAME           remove an installed tool (its directory name)
  protonup path                  the directory used for installs

options:
  --source ID                    ge (default), cachyos, cachyos-v3, rtsp, sarek, northstar, luxtorpeda
  --dir PATH                     use this compatibilitytools.d instead of the detected Steam
  -y, --yes                      do not ask before `install --all`
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

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dir = match take_opt(&mut args, "--dir") {
        Ok(d) => d.map(PathBuf::from),
        Err(e) => return fail(e),
    };
    let source_id = match take_opt(&mut args, "--source") {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let all = take_flag(&mut args, &["--all"]);
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
        "gui" => match gui::run(dir_path, dir.is_some()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
        "installed" => {
            let l = install::list_installed(&dir_path);
            if l.is_empty() {
                say!("nothing installed in {}", dir_path.display());
            }
            for i in l {
                say!("{:<44} {}", i.name, deck_common::human_bytes(i.size));
            }
            ExitCode::SUCCESS
        }
        "list" => {
            let n = if all {
                usize::MAX
            } else {
                args.iter().position(|a| a == "-n").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(15)
            };
            match github::releases(src, n) {
                Ok(rs) => {
                    let have: Vec<String> = install::list_installed(&dir_path).into_iter().map(|i| i.name).collect();
                    for r in rs {
                        let size = r.archive(src).map(|a| deck_common::human_bytes(a.size)).unwrap_or_default();
                        let mark = if have.contains(&r.dir_name(src)) { "installed" } else { "" };
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
            let have: Vec<String> = install::list_installed(&dir_path).into_iter().map(|i| i.name).collect();
            let todo: Vec<&github::Release> = if all {
                rs.iter().filter(|r| !have.contains(&r.dir_name(src)) || matches!(src.dir, sources::DirName::Fixed(_))).collect()
            } else {
                let one = if want == "latest" { rs.first() } else { rs.iter().find(|r| r.tag_name == want || r.dir_name(src) == want) };
                match one {
                    Some(r) => vec![r],
                    None => return fail(format!("no {} release '{want}' (see `protonup list`)", src.name)),
                }
            };
            if todo.is_empty() {
                say!("nothing to install: all {} releases are already there", src.name);
                return ExitCode::SUCCESS;
            }
            if all {
                let total: u64 = todo.iter().filter_map(|r| r.archive(src)).map(|a| a.size).sum();
                say!("{} releases of {} not installed yet, {} to download (more once unpacked)", todo.len(), src.name, deck_common::human_bytes(total));
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
            let mut failed = 0;
            for rel in todo {
                say!("installing {} into {}", rel.tag_name, dir_path.display());
                let mut last = 0u64;
                let res = install::install(src, rel, &dir_path, &mut |p| match p {
                    install::Progress::Phase(s) => {
                        if last > 0 {
                            say!();
                            last = 0;
                        }
                        say!("  {s}");
                    }
                    install::Progress::Download { done, total } => {
                        last = done;
                        let pct = (done * 100).checked_div(total).unwrap_or(0);
                        print!("\r  download {:>3}%  {} / {}", pct, deck_common::human_bytes(done), deck_common::human_bytes(total));
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                });
                match res {
                    Ok(o) => say!("installed {}{}", o.path.display(), if o.verified { "" } else { "  (the project publishes no checksum: not verified)" }),
                    Err(e) => {
                        failed += 1;
                        eprintln!("\nprotonup: {}: {e}", rel.tag_name);
                    }
                }
            }
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
