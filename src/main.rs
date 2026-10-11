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
                println!("steamdeck: ProtonUp, Flatseal and system setup in one window\n\n  steamdeck                  open the window\n  steamdeck protonup ...     the ProtonUp command line (try: steamdeck protonup --help)\n  steamdeck flatseal ...     the Flatseal command line\n\nLog: {}", steamdeck::log::path().display());
                return ExitCode::SUCCESS;
            }
            _ => {
                return match steamdeck::launcher::run(steamdeck::launcher::Start::default()) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        steamdeck::log::write("launcher", &format!("window failed: {e}"));
                        eprintln!("steamdeck: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
        },
    };
    steamdeck::log::install_panic_hook();
    match tool.as_str() {
        "protonup" => steamdeck::cli_protonup::run(rest),
        _ => steamdeck::cli_flatseal::run(rest),
    }
}
