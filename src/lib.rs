//! steamdeck: ProtonUp and Flatseal in one crate, one `src`, one window layout (see `ui`).
//! The two programs are thin binaries (`protonup.rs`, `flatseal.rs`); everything else is shared here.
//! No Qt, no Python, no GTK: iced for the windows, ureq + rustls for HTTP, flate2/lzma-rust2 + tar for
//! archives.

pub mod cli_flatseal;
pub mod cli_protonup;
pub mod launcher;
pub mod log;
pub mod steaminstalls;
pub mod system;
pub mod ui;

// protonup: find, download, verify and install compatibility tools; set them per game
pub mod games;
pub mod github;
pub mod install;
pub mod proton_gui;
pub mod sources;
pub mod steam;

// flatseal: Flatpak permission overrides and portal permissions
pub mod flatapps;
pub mod flatpak;
pub mod health;
pub mod keyfile;
pub mod perms;
pub mod portals;
pub mod protondb;
pub mod seal_gui;
