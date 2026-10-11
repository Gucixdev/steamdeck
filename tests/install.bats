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
	grep -q "^ExecStart=$T/p/bin/steamdeck protonup update$" "$T/home/.config/systemd/user/protonup-update.service"
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

# ---- safety: nothing that is not ours is overwritten or removed ----------------------------------

@test "a protonup of the user's own in bin/ is not replaced and not removed" {
	mkdir -p "$T/p/bin"
	printf '#!/bin/sh\necho mine\n' > "$T/p/bin/protonup"; chmod +x "$T/p/bin/protonup"
	run "$ROOT/setup" --programs --from "$DIST" --prefix "$T/p"
	[ "$status" -eq 0 ]
	[[ "$output" == *"skip $T/p/bin/protonup"* ]]
	[ "$("$T/p/bin/protonup")" = mine ]
	run "$ROOT/setup" --programs --uninstall --prefix "$T/p"
	[ "$("$T/p/bin/protonup")" = mine ]
	[ ! -e "$T/p/bin/steamdeck" ]
}

@test "uninstall leaves every unrelated file in bin, applications and icons" {
	"$ROOT/setup" --programs --from "$DIST" --prefix "$T/p" >/dev/null
	for f in bin/other share/applications/other.desktop share/icons/hicolor/scalable/apps/other.svg; do
		mkdir -p "$(dirname "$T/p/$f")"; echo keep > "$T/p/$f"
	done
	run "$ROOT/setup" --programs --uninstall --prefix "$T/p"
	[ "$status" -eq 0 ]
	for f in bin/other share/applications/other.desktop share/icons/hicolor/scalable/apps/other.svg; do
		[ "$(cat "$T/p/$f")" = keep ]
	done
}

@test "a second install over the first keeps the user's other files and is idempotent" {
	mkdir -p "$T/p/bin"; echo keep > "$T/p/bin/something"
	"$ROOT/setup" --programs --from "$DIST" --prefix "$T/p" >/dev/null
	run "$ROOT/setup" --programs --from "$DIST" --prefix "$T/p"
	[ "$status" -eq 0 ]
	[ "$(cat "$T/p/bin/something")" = keep ]
	[ "$(readlink "$T/p/bin/protonup")" = steamdeck ]
}

@test "the firefox step touches only the chosen profile and keeps its other files" {
	A=$T/ffa; B=$T/ffb; mkdir -p "$A" "$B"
	printf 'user_pref("b.own", 2);\n' > "$B/user.js"; echo cookies > "$A/cookies.sqlite"
	cp "$B/user.js" "$T/b.before"
	FIREFOX_PROFILE="$A" "$ROOT/setup" --firefox >/dev/null
	cmp "$B/user.js" "$T/b.before"
	[ "$(cat "$A/cookies.sqlite")" = cookies ]
}

@test "the firefox step keeps a backup of the original user.js and never loses the user's lines" {
	P=$T/ff; mkdir -p "$P"
	printf '// my own\nuser_pref("my.own", 1);\n' > "$P/user.js"
	cp "$P/user.js" "$T/orig"
	FIREFOX_PROFILE="$P" "$ROOT/setup" --firefox >/dev/null
	cmp "$P/user.js.steamdeck-bak" "$T/orig"
	FIREFOX_PROFILE="$P" "$ROOT/setup" --firefox >/dev/null
	cmp "$P/user.js.steamdeck-bak" "$T/orig"   # the backup is the original, not the second run
	grep -q '^// my own$' "$P/user.js"
	grep -q 'user_pref("my.own", 1);' "$P/user.js"
}
