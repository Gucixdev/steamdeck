//! steamdeck: one program. No arguments opens the window; `protonup ...` and `flatseal ...` (or the
//! program started under one of those names, as the symlinks `setup` makes) are the command lines.
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    let name = args
        .first()
        .map(|a| a.rsplit('/').next().unwrap_or(a).to_string())
        .unwrap_or_default();
    args.remove(0);
    let (tool, rest) = match name.as_str() {
        "protonup" => ("protonup".to_string(), args),
        "flatseal" => ("flatseal".to_string(), args),
        _ => match args.first().map(String::as_str) {
            Some("protonup") | Some("flatseal") => (args[0].clone(), args[1..].to_vec()),
            Some("--help") | Some("-h") | Some("help") => {
                println!("steamdeck: ProtonUp, Flatseal and system setup in one window\n\n  steamdeck [storage|steam|apps|health|settings|log]   open the window (on that page)\n  steamdeck protonup ...     the ProtonUp command line (try: steamdeck protonup --help)\n  steamdeck flatseal ...     the Flatseal command line\n\nLog: {}", steamdeck::log::path().display());
                return ExitCode::SUCCESS;
            }
            Some("--version") | Some("-V") => {
                println!("steamdeck {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other => {
                const PAGES: [&str; 8] = [
                    "home", "steam", "apps", "scripts", "storage", "health", "system", "log",
                ];
                if let Some(o) = other {
                    if o != "settings" && !PAGES.contains(&o) {
                        eprintln!("steamdeck: unknown command '{o}' (try --help)");
                        return ExitCode::from(2);
                    }
                }
                let section = match other {
                    Some("steam") => steamdeck::launcher::Section::Steam,
                    Some("apps") => steamdeck::launcher::Section::Apps,
                    Some("scripts") => steamdeck::launcher::Section::Scripts,
                    Some("storage") => steamdeck::launcher::Section::Storage,
                    Some("health") => steamdeck::launcher::Section::Health,
                    Some("system") | Some("settings") => steamdeck::launcher::Section::System,
                    Some("log") => steamdeck::launcher::Section::Log,
                    _ => steamdeck::launcher::Section::Home,
                };
                return match steamdeck::launcher::run(steamdeck::launcher::Start {
                    section,
                    ..Default::default()
                }) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        steamdeck::log::write("launcher", &format!("window failed: {e}"));
                        eprintln!("steamdeck: {e}");
                        ExitCode::FAILURE
                    }
                };
            }
        },
    };
    steamdeck::log::install_panic_hook();
    match tool.as_str() {
        "protonup" => steamdeck::cli_protonup::run(rest),
        _ => steamdeck::cli_flatseal::run(rest),
    }
}
