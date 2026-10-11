# steamdeck

One small program for the Steam Deck that does not drag Qt, Python or GTK along: **ProtonUp**, **Flatseal**
and **System** setup in a single window, in Steam's colours. Rust and iced, one binary (about 15 MB), plus
two bash files: `setup` (a wizard) and `run` (build, install, start).

| | what |
|---|---|
| Home | tiles saying what each tab is for, Update Protons, and a warning when two Steam installs are found |
| ProtonUp | install and remove Proton-GE, Proton-CachyOS, Luxtorpeda and more; pick the Proton per game; every Steam install (native and Flatpak) on one list |
| Flatseal | Flatpak permissions as default / on / off, portal permissions (also for all apps at once), PipeWire, filesystems, buses, environment |
| Settings | Health check (what games like against what is set, recent crashes), Flatpak apps (uninstall, clear data, unused runtimes), Steam installs (remove one), System (OOM guard, memory settings for zram, swap file size, ssh: the same actions as `setup`) and Log (`~/.local/state/steamdeck/steamdeck.log`: what the window and `setup` did, every error and panic) |

The window is [iced](https://iced.rs) drawn on the GPU with a software fallback, plain Wayland/X11, rustls
for HTTPS, pure-Rust gzip and xz.

## Install

On the Deck, in Desktop Mode, in a terminal:

```sh
git clone https://github.com/Gucixdev/steamdeck && cd steamdeck && ./setup
```

or only the program, straight from the latest release (SHA-256 verified against `SHA256SUMS`):

```sh
curl -fsSL https://raw.githubusercontent.com/Gucixdev/steamdeck/main/setup | bash
```

`setup` asks what you want and shows the list before it changes anything: the program (menu entry and the
`protonup` / `flatseal` command names), a weekly Proton-GE update, the OOM guard, memory settings, swap
size, ssh, and preparing Firefox (telemetry and sponsored content off, WebRTC leak guard, uBlock Origin;
Mullvad VPN is a system app, not a Firefox add-on). Run `./setup` again to see the state or remove it.

For Gaming Mode: Desktop Mode, Steam, Games, **Add a Non-Steam Game**, pick Steam Deck.

## Layout

```
setup                     the wizard: programs, system, Firefox (asks, confirms, applies)
run                       build when src changed, install, start; --publish TAG makes the release
src/main.rs               steamdeck [protonup|flatseal ...] command lines, no arguments = the window
src/launcher.rs           the window: ProtonUp, Flatseal, System, Log
src/ui.rs                 theme (Steam colours) and the layout every section uses: sidebar + main area
src/proton_gui.rs  cli_protonup.rs  github.rs install.rs sources.rs steam.rs games.rs
src/seal_gui.rs    cli_flatseal.rs  flatpak.rs keyfile.rs perms.rs portals.rs
src/system.rs log.rs      System section (runs the embedded setup script), the log file
```

One Cargo package, one `src`, one binary. `protonup` and `flatseal` are symlinks to it.

## protonup

```
protonup [gui [TAB]]           open the window (TAB: installed, available, games)
protonup games                 installed games and the tool each one runs with
protonup use APPID TOOL|default   run one game with TOOL (Steam must be closed)
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

In the window, two pages: **Download Proton** (a category dropdown, then a table of that category's
releases with Install, and remove for the installed ones; **Update Protons** on Home installs the newest
of every kind you have) and **Proton for my games** (a table with a dropdown per game, like
ProtonUp-Qt, across every Steam install, or one choice for all games; Valve's own Protons are in the list, launch options per game: GameMode, MangoHud, FSR; ProtonDB ratings on request). `protonup update` (what the weekly timer runs) installs the newest of every kind you have and sends a desktop notification. Alt+1..4 switch the tabs. The per-game choice is written to Steam's `config/config.vdf` (`CompatToolMapping`); Steam
rewrites that file when it exits, so protonup refuses to change it while Steam is running and keeps a
`config.vdf.steamdeck-bak` of the previous file.

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

`./setup` is a wizard: it asks whether to install the programs (and the weekly Proton-GE update), the OOM
guard, the memory settings (and the swappiness value), a new swap file size, and ssh; it shows the list
and applies it only after you confirm. The actions below are what the System section and `setup --system` do: `setup --system oom`, `setup --system swapfile 16G`, `setup --system undo`.

| action | what it writes (all under `/etc`, which survives SteamOS updates) |
|---|---|
| `oom` | systemd-oomd settings: kill the worst offender in the user slice (the game) at 60% memory pressure or 90% swap, keep the Gaming Mode compositor out of reach, enable the daemon |
| `sysctl [--swappiness N]` | `vm.swappiness`, `vm.vfs_cache_pressure = 50`, `vm.page-cluster = 0`, suited to zram |
| `swapfile SIZE` | resize `/home/swapfile` (checks free space first) |
| `ssh` | enable `sshd` |
| `status`, `undo` | what is set; remove exactly the files this script wrote |

Every file starts with a `managed by steamdeck/setup` line; a file without it is never overwritten or
removed. `--root DIR` writes into a test tree and only prints the commands, `--dry-run` changes nothing.

## Development

```sh
./run                   # builds when src/ changed, installs into ~/.local/bin, opens the window
./run protonup          # the same, opened on ProtonUp (flatseal, system, log)
./run --release         # optimised build
./run --publish v0.2.0  # portable build (glibc 2.36 via zig) and a GitHub release for ./setup to download
cargo test              # unit and end-to-end tests (a local HTTP server stands in for GitHub)
bats tests/             # setup: programs, system actions, firefox, the wizard
```

`run` passes no arguments on; for those use `steamdeck protonup ...` / `steamdeck flatseal ...`.

## Status

Tested on an x86_64 desktop: all unit tests, the whole install pipeline against a local mock of the
GitHub API and against the real one (a real xz-compressed Luxtorpeda release with its published
checksum), the windows rendered and inspected, both scripts under bats. **Not yet run on an actual
Steam Deck**: the systemd-oomd units and the swap file steps follow what SteamOS ships but have not
been exercised on one. `setup --system status` and `setup --system undo` are safe to try first.

Not done: the apps cannot be driven by the Deck's gamepad directly (they take touch and the trackpads;
as a non-Steam game Steam Input maps the pads to the mouse), no DXVK/VKD3D downloads for Lutris/Heroic,
no Proton-EM and DW-Proton (not on GitHub).

## Licence

MIT, see [`LICENSE`](LICENSE).
