#!/usr/bin/env bats
# install: from a local dist directory (no network), into a temporary prefix.

setup() {
	ROOT=$BATS_TEST_DIRNAME/..
	T=$(mktemp -d)
	DIST=$T/dist
	mkdir -p "$DIST" "$T/home"
	printf '#!/bin/sh\necho protonup-fake\n' > "$DIST/protonup-x86_64-linux"
	printf '#!/bin/sh\necho flatseal-fake\n' > "$DIST/flatseal-x86_64-linux"
	chmod +x "$DIST"/*
	(cd "$DIST" && sha256sum -- *-x86_64-linux > SHA256SUMS)
	export HOME=$T/home XDG_CONFIG_HOME=$T/home/.config STEAMDECK_NO_SYSTEMD=1
}

teardown() { rm -rf "$T"; }

@test "installs both binaries, menu entries and icons under the prefix" {
	run "$ROOT/scripts/install" --from "$DIST" --prefix "$T/p"
	[ "$status" -eq 0 ]
	[ -x "$T/p/bin/protonup" ]
	[ -x "$T/p/bin/flatseal" ]
	[ "$("$T/p/bin/protonup")" = protonup-fake ]
	grep -q "^Exec=$T/p/bin/protonup$" "$T/p/share/applications/protonup.desktop"
	grep -q '^Name=Flatseal$' "$T/p/share/applications/flatseal.desktop"
	grep -q '<svg' "$T/p/share/icons/hicolor/scalable/apps/protonup.svg"
}

@test "--only installs just one tool" {
	run "$ROOT/scripts/install" --from "$DIST" --prefix "$T/p" --only flatseal
	[ "$status" -eq 0 ]
	[ -x "$T/p/bin/flatseal" ]
	[ ! -e "$T/p/bin/protonup" ]
}

@test "a tampered binary is refused and nothing is installed" {
	echo tampered >> "$DIST/protonup-x86_64-linux"
	run "$ROOT/scripts/install" --from "$DIST" --prefix "$T/p"
	[ "$status" -ne 0 ]
	[[ "$output" == *"checksum mismatch"* ]]
	[ ! -e "$T/p/bin/protonup" ]
	[ ! -e "$T/p/bin/flatseal" ]
}

@test "a binary missing from SHA256SUMS is refused" {
	grep -v flatseal "$DIST/SHA256SUMS" > "$T/s" && mv "$T/s" "$DIST/SHA256SUMS"
	run "$ROOT/scripts/install" --from "$DIST" --prefix "$T/p"
	[ "$status" -ne 0 ]
	[[ "$output" == *"not listed in SHA256SUMS"* ]]
}

@test "--dry-run changes nothing" {
	run "$ROOT/scripts/install" --from "$DIST" --prefix "$T/p" --dry-run
	[ "$status" -eq 0 ]
	[ ! -e "$T/p" ]
	[[ "$output" == *"+ install"* ]]
}

@test "--no-desktop writes only the binaries" {
	run "$ROOT/scripts/install" --from "$DIST" --prefix "$T/p" --no-desktop
	[ "$status" -eq 0 ]
	[ -x "$T/p/bin/protonup" ]
	[ ! -e "$T/p/share" ]
}

@test "--timer writes the update service and timer" {
	run "$ROOT/scripts/install" --from "$DIST" --prefix "$T/p" --timer
	[ "$status" -eq 0 ]
	grep -q "^ExecStart=$T/p/bin/protonup install latest$" "$T/home/.config/systemd/user/protonup-update.service"
	grep -q '^OnCalendar=weekly$' "$T/home/.config/systemd/user/protonup-update.timer"
}

@test "--uninstall removes what was installed" {
	"$ROOT/scripts/install" --from "$DIST" --prefix "$T/p" --timer
	run "$ROOT/scripts/install" --uninstall --prefix "$T/p"
	[ "$status" -eq 0 ]
	[ ! -e "$T/p/bin/protonup" ]
	[ ! -e "$T/p/share/applications/flatseal.desktop" ]
	[ ! -e "$T/home/.config/systemd/user/protonup-update.timer" ]
}

@test "installing twice is fine (idempotent)" {
	"$ROOT/scripts/install" --from "$DIST" --prefix "$T/p"
	run "$ROOT/scripts/install" --from "$DIST" --prefix "$T/p"
	[ "$status" -eq 0 ]
	[ -x "$T/p/bin/protonup" ]
}

@test "an unknown tool or option is an error" {
	run "$ROOT/scripts/install" --only nothing --from "$DIST"
	[ "$status" -ne 0 ]
	run "$ROOT/scripts/install" --bogus
	[ "$status" -ne 0 ]
}

@test "downloads from a release URL and verifies the checksum" {
	command -v python3 >/dev/null || skip "python3 not available"
	port=$((20000 + RANDOM % 20000))
	(cd "$DIST" && exec python3 -m http.server "$port" --bind 127.0.0.1 >/dev/null 2>&1) &
	srv=$!
	for _ in 1 2 3 4 5 6 7 8 9 10; do curl -fsS "http://127.0.0.1:$port/SHA256SUMS" >/dev/null 2>&1 && break; sleep 0.3; done
	run env STEAMDECK_RELEASE_URL="http://127.0.0.1:$port" "$ROOT/scripts/install" --prefix "$T/p"
	kill "$srv" 2>/dev/null || true
	[ "$status" -eq 0 ]
	[ "$("$T/p/bin/flatseal")" = flatseal-fake ]
}
