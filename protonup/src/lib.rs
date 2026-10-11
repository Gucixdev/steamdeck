//! protonup: find, download, verify and install Proton-GE builds into Steam's compatibilitytools.d.
//! Everything here is plain std + ureq/flate2/tar/sha2; the GUI and the CLI both sit on top of it.

pub mod github;
pub mod install;
pub mod sources;
pub mod steam;

pub use github::{Asset, Release};
pub use sources::Source;
pub use install::{Installed, Progress};
