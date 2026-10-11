//! flatseal: read and edit the permission overrides of Flatpak apps without GTK, Python or a portal.
//! The overrides are plain keyfiles under $FLATPAK_USER_DIR/overrides, the same ones `flatpak override`
//! writes, so the two tools can be mixed freely.

pub mod flatpak;
pub mod keyfile;
pub mod perms;
