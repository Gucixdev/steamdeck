#!/usr/bin/env bats
# programs step of setup: from a local dist directory (no network), into a temporary prefix.

setup() {
	ROOT=$BATS_TEST_DIRNAME/..
	T=$(mktemp -d)
	DIST=$T/dist
	mkdir -p "$DIST" "$T/home"
	printf '#!/bin/sh\necho steamdeck-fake "$@"\n' > "$DIST/steamdeck-x86_64-linux"
	chmod +x "$DIST"/*
	(cd "$DIST" && sha256sum -- *-x86_64-linux > SHA256SUMS)
	export HOME=$T/home XDG_CONFIG_HOME=$T/home/.config XDG_STATE_HOME=$T/home/state STEAMDECK_NO_SYSTEMD=1
}

teardown() { rm -rf "$T"; }

@test "installs the program, the protonup and flatseal names, the menu entry and the icon" {
	run "$ROOT/setup" --programs --from "$DIST" --prefix "$T/p"
	[ "$status" -eq 0 ]
	[ -x "$T/p/bin/steamdeck" ]
	[ "$("$T/p/bin/steamdeck")" = steamdeck-fake ]
	[ "$(readlink "$T/p/bin/protonup")" = steamdeck ]
	[ "$(readlink "$T/p/bin/flatseal")" = steamdeck ]
	grep -q "^Exec=$T/p/bin/steamdeck$" "$T/p/share/applications/steamdeck.desktop"
	grep -q '^Name=Steam Deck$' "$T/p/share/applications/steamdeck.desktop"
	grep -q '<svg' "$T/p/share/icons/hicolor/scalable/apps/steamdeck.svg"
}

@test "a tampered binary is refused and nothing is installed" {
	echo tampered >> "$DIST/steamdeck-x86_64-linux"
	run "$ROOT/setup" --programs --from "$DIST" --prefix "$T/p"
	[ "$status" -ne 0 ]
	[[ "$output" == *"checksum mismatch"* ]]
	[ ! -e "$T/p/bin/steamdeck" ]
}

@test "a binary missing from SHA256SUMS is refused" {
	: > "$DIST/SHA256SUMS"
	run "$ROOT/setup" --programs --from "$DIST" --prefix "$T/p"
	[ "$status" -ne 0 ]
	[[ "$output" == *"not listed in SHA256SUMS"* ]]
}

@test "--dry-run changes nothing" {
	run "$ROOT/setup" --programs --from "$DIST" --prefix "$T/p" --dry-run
	[ "$status" -eq 0 ]
	[ ! -e "$T/p" ]
}

@test "--no-desktop writes only the binary and the names" {
	run "$ROOT/setup" --programs --from "$DIST" --prefix "$T/p" --no-desktop
	[ "$status" -eq 0 ]
	[ -x "$T/p/bin/steamdeck" ]
	[ ! -e "$T/p/share" ]
}

@test "--local copies your own build and skips it when unchanged" {
	mkdir -p "$T/build"
	printf '#!/bin/sh\necho local\n' > "$T/build/steamdeck"; chmod +x "$T/build/steamdeck"
	run "$ROOT/setup" --programs --local "$T/build" --prefix "$T/p"
	[ "$status" -eq 0 ]
	[ "$("$T/p/bin/steamdeck")" = local ]
	run "$ROOT/setup" --programs --local "$T/build" --prefix "$T/p"
	[[ "$output" == *"is up to date"* ]]
}

@test "--timer writes the weekly update unit" {
	run "$ROOT/setup" --programs --from "$DIST" --prefix "$T/p" --timer
	[ "$status" -eq 0 ]
	grep -q "^ExecStart=$T/p/bin/steamdeck protonup install latest$" "$T/home/.config/systemd/user/protonup-update.service"
	grep -q '^OnCalendar=weekly$' "$T/home/.config/systemd/user/protonup-update.timer"
}

@test "--uninstall removes everything it installed" {
	"$ROOT/setup" --programs --from "$DIST" --prefix "$T/p" --timer >/dev/null
	run "$ROOT/setup" --programs --uninstall --prefix "$T/p"
	[ "$status" -eq 0 ]
	[ ! -e "$T/p/bin/steamdeck" ]
	[ ! -e "$T/p/bin/protonup" ]
	[ ! -e "$T/p/share/applications/steamdeck.desktop" ]
	[ ! -e "$T/home/.config/systemd/user/protonup-update.timer" ]
}

@test "the firefox step writes prefs between markers and keeps the user's own lines" {
	P=$T/ff; mkdir -p "$P"
	printf 'user_pref("my.own", 1);\n' > "$P/user.js"
	run env FIREFOX_PROFILE="$P" "$ROOT/setup" --firefox --dry-run
	[ "$status" -eq 0 ]
	run env FIREFOX_PROFILE="$P" PATH="$T/nobin:/usr/bin:/bin" "$ROOT/setup" --firefox
	[ "$status" -eq 0 ]
	grep -q 'my.own' "$P/user.js"
	grep -q 'toolkit.telemetry.enabled", false' "$P/user.js"
	FIREFOX_PROFILE="$P" "$ROOT/setup" --firefox >/dev/null
	[ "$(grep -c '>>> steamdeck setup' "$P/user.js")" -eq 1 ]
}
