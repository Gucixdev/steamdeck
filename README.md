# steamdeck

Small tools for the Steam Deck that do not drag Qt, Python or GTK along. Two single-binary apps
written in Rust, and three bash scripts that put them on the Deck and tune the system.

| | what | form |
|---|---|---|
| [`protonup`](protonup) | install and remove Proton-GE, Proton-CachyOS, Luxtorpeda and more for Steam | one binary, window or command line |
| [`flatseal`](flatseal) | edit Flatpak permission overrides | one binary, window or command line |
| [`install`](install) | put the two binaries and menu entries on the Deck | bash, no root |
| [`setup`](setup) | OOM guard, swap file, sysctl, ssh | bash, asks sudo |
| [`build`](build) | build the release binaries | bash |

Both apps are one executable each (about 7-8 MB): the window is [iced](https://iced.rs) drawn on the GPU
with a software fallback, plain Wayland/X11, rustls for HTTPS, pure-Rust gzip and xz. Nothing is
installed next to them.

## Install

On the Deck, in Desktop Mode, in a terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/Gucixdev/steamdeck/main/install | bash
```

That downloads the binaries of the latest release, verifies their SHA-256 against the release's
`SHA256SUMS`, puts them in `~/.local/bin` and adds ProtonUp and Flatseal to the application menu.
Nothing needs root. Options (`bash -s -- OPTION`): `--only protonup`, `--timer` (a weekly
`protonup install latest`), `--prefix DIR`, `--version TAG`, `--uninstall`, `--dry-run`.

For Gaming Mode: Desktop Mode, Steam, Games, **Add a Non-Steam Game**, pick ProtonUp or Flatseal.
The windows are laid out for the Deck's screen and touch.

Then, if you want the system tuning:

```sh
git clone https://github.com/Gucixdev/steamdeck && cd steamdeck
passwd                 # once: the deck user has no password and sudo needs one
./setup all            # oom guard + sysctl + ssh
./setup swapfile 16G   # optional: a bigger swap file
```

## protonup

```
protonup                       open the window
protonup sources               the tools it can fetch
protonup list [-n N | --all]   releases of a source, installed ones marked
protonup install [TAG]         one release, or the newest
protonup install --all [--yes] every release of the source that is not installed yet
-j N                           downloads at once (default 3, at most 8)
protonup installed | remove NAME | path
--source ID                    ge (default), cachyos, cachyos-v3, rtsp, sarek, northstar, luxtorpeda
```

| source | what |
|---|---|
| `ge` | Proton-GE, GloriousEggroll's build with extra patches and codecs |
| `cachyos`, `cachyos-v3` | Proton-CachyOS; the v3 build needs AVX2, which the Deck's Zen 2 has |
| `rtsp` | Proton-GE RTSP, for games that play video through GStreamer RTSP |
| `sarek` | Proton-Sarek, for older GPUs and Vulkan drivers |
| `northstar` | NorthstarProton, for the Titanfall 2 Northstar client |
| `luxtorpeda` | runs the native Linux engine of a game instead of its Windows build |

Releases come from GitHub's API. Downloads are checked against the SHA-512 the project publishes; a
project that publishes none (Sarek, Northstar) is installed on HTTPS alone and the window and the
command line say so. Nothing appears under its final name unless the checksum matched and the archive
unpacked completely. Install directories: the native Steam
(`~/.local/share/Steam/compatibilitytools.d`) and the Flatpak Steam, chosen in the window or with
`--dir`. **Install all** shows the number and size first; a full Proton-GE history is tens of
gigabytes, mind the Deck's SSD. The queue works like `dspq` from the bash repo: a FIFO list, `-j N`
downloads at once, each into a `.part` file that becomes the real file only when it is complete, and an
interrupted download continues from the bytes already on disk (HTTP `Range`) when you run it again. After an install restart Steam and pick the tool under a game's
Properties, Compatibility.

## flatseal

```
flatseal                          open the window
flatseal list                     installed Flatpak apps
flatseal show APP                 effective permissions; * marks the app's own overrides
flatseal allow|deny|default APP KEY NAME   KEY: shared, sockets, devices, features, filesystems
flatseal portals APP              portal permissions and files the app was granted
flatseal portal APP NAME yes|no|default    background, notification, camera, microphone, speakers
flatseal revoke APP TABLE ID      take back one stored entry
flatseal fs-add APP PATH[:ro|:rw|:create]   |  fs-remove APP PATH
flatseal reset APP                drop the app's overrides
```

`APP` is an application id or `global` (applies to every app). It edits the files `flatpak override
--user` writes, under `$FLATPAK_USER_DIR/overrides`, so the two can be mixed. Every permission has
three states: **default** (no entry, inherit), **on** (`name`) and **off** (`!name`). On and off are
always written, so you can switch networking off for all apps explicitly instead of just not enabling it;
default removes the entry again. **Portals** (background, notifications, camera, microphone, speakers)
and the files an app was given through the file chooser are read and changed through `flatpak
permission-*`, the same permission store the desktop portals use.
Network, sockets, devices, features, filesystems (the standard folders plus your own paths), session
and system bus policies and environment variables are covered.

## setup

| action | what it writes (all under `/etc`, which survives SteamOS updates) |
|---|---|
| `oom` | systemd-oomd settings: kill the worst offender in the user slice (the game) at 60% memory pressure or 90% swap, keep the Gaming Mode compositor out of reach, enable the daemon |
| `sysctl [--swappiness N]` | `vm.swappiness`, `vm.vfs_cache_pressure = 50`, `vm.page-cluster = 0`, suited to zram |
| `swapfile SIZE` | resize `/home/swapfile` (checks free space first) |
| `ssh` | enable `sshd` |
| `status`, `undo` | what is set; remove exactly the files this script wrote |

Every file starts with a `managed by steamdeck/setup` line; a file without it is never overwritten or
removed. `--root DIR` writes into a test tree and only prints the commands, `--dry-run` changes nothing.

## Build

```sh
./build                 # host toolchain, dist/protonup-x86_64-linux, dist/flatseal-x86_64-linux, dist/SHA256SUMS
./build --portable      # links against glibc 2.36 with zig (cargo-zigbuild): runs on any SteamOS
./build --publish v0.1.0   # also create the GitHub release with those files
cargo test              # 30 unit and end-to-end tests (a local HTTP server stands in for GitHub)
bats tests/             # the install and setup scripts, 20 tests
```

Use `--portable` for releases: a plain build needs the glibc of the machine it was built on.

## Status

Tested on an x86_64 desktop: all unit tests, the whole install pipeline against a local mock of the
GitHub API and against the real one (a real xz-compressed Luxtorpeda release with its published
checksum), the windows rendered and inspected, both scripts under bats. **Not yet run on an actual
Steam Deck**: the systemd-oomd units and the swap file steps follow what SteamOS ships but have not
been exercised on one. `./setup status` and `./setup undo` are safe to try first.

Not done: the apps cannot be driven by the Deck's gamepad directly (they take touch and the trackpads;
as a non-Steam game Steam Input maps the pads to the mouse), no DXVK/VKD3D downloads for Lutris/Heroic,
no Proton-EM and DW-Proton (not on GitHub).

## Licence

MIT, see [`LICENSE`](LICENSE).
