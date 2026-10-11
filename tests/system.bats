#!/usr/bin/env bats
# scripts/system: files are written under --root, system commands are only printed.

setup() {
	ROOT=$BATS_TEST_DIRNAME/..
	T=$(mktemp -d)
}

teardown() { rm -rf "$T"; }

@test "oom writes the oomd configuration and enables the daemon" {
	run "$ROOT/setup" --system --root "$T" oom
	[ "$status" -eq 0 ]
	grep -q '^SwapUsedLimit=90%$' "$T/etc/systemd/oomd.conf.d/10-steamdeck.conf"
	grep -q '^ManagedOOMSwap=kill$' "$T/etc/systemd/system/-.slice.d/10-steamdeck-oomd.conf"
	grep -q '^ManagedOOMMemoryPressure=kill$' "$T/etc/systemd/system/user@.service.d/10-steamdeck-oomd.conf"
	grep -q '^ManagedOOMPreference=avoid$' "$T/etc/systemd/user/gamescope-session-plus@.service.d/10-steamdeck-oomd.conf"
	[[ "$output" == *"+ systemctl enable --now systemd-oomd.service"* ]]
}

@test "every written file carries the managed marker on its first line" {
	"$ROOT/setup" --system --root "$T" all >/dev/null
	n=0
	while IFS= read -r f; do
		head -1 "$f" | grep -q '^# managed by steamdeck/setup$'
		n=$((n + 1))
	done < <(find "$T" -type f)
	[ "$n" -eq 5 ]
}

@test "sysctl takes the swappiness" {
	run "$ROOT/setup" --system --root "$T" sysctl --swappiness 60
	[ "$status" -eq 0 ]
	grep -q '^vm.swappiness = 60$' "$T/etc/sysctl.d/99-steamdeck-setup.conf"
	grep -q '^vm.page-cluster = 0$' "$T/etc/sysctl.d/99-steamdeck-setup.conf"
}

@test "a file this script did not write is left alone" {
	mkdir -p "$T/etc/sysctl.d"
	printf 'vm.swappiness = 10\n' > "$T/etc/sysctl.d/99-steamdeck-setup.conf"
	run "$ROOT/setup" --system --root "$T" sysctl
	[ "$status" -eq 0 ]
	[[ "$output" == *"skip /etc/sysctl.d/99-steamdeck-setup.conf"* ]]
	[ "$(cat "$T/etc/sysctl.d/99-steamdeck-setup.conf")" = 'vm.swappiness = 10' ]
}

@test "undo removes exactly the managed files" {
	"$ROOT/setup" --system --root "$T" all >/dev/null
	printf 'keep\n' > "$T/etc/sysctl.d/50-mine.conf"
	run "$ROOT/setup" --system --root "$T" undo
	[ "$status" -eq 0 ]
	[ ! -e "$T/etc/systemd/oomd.conf.d/10-steamdeck.conf" ]
	[ ! -e "$T/etc/sysctl.d/99-steamdeck-setup.conf" ]
	[ -f "$T/etc/sysctl.d/50-mine.conf" ]
}

@test "--dry-run writes nothing" {
	run "$ROOT/setup" --system --root "$T" --dry-run all
	[ "$status" -eq 0 ]
	[[ "$output" == *"would write /etc/systemd/oomd.conf.d/10-steamdeck.conf"* ]]
	[ -z "$(find "$T" -type f)" ]
}

@test "swapfile checks the size and prints the steps" {
	run "$ROOT/setup" --system --root "$T" swapfile 8G
	[ "$status" -eq 0 ]
	[[ "$output" == *"+ fallocate -l 8G /home/swapfile"* ]]
	[[ "$output" == *"+ swapon /home/swapfile"* ]]
	run "$ROOT/setup" --system --root "$T" swapfile lots
	[ "$status" -ne 0 ]
	run "$ROOT/setup" --system --root "$T" swapfile
	[ "$status" -ne 0 ]
}

@test "unknown actions and options are rejected before any privilege escalation" {
	run "$ROOT/setup" --system bogus
	[ "$status" -eq 1 ]
	[[ "$output" == *"unknown action"* ]]
	run "$ROOT/setup" --system --nope oom
	[ "$status" -eq 1 ]
	run "$ROOT/setup" --system --swappiness abc oom
	[ "$status" -ne 0 ]
}

@test "status reports what is configured" {
	"$ROOT/setup" --system --root "$T" oom >/dev/null
	run "$ROOT/setup" --system --root "$T" status
	[ "$status" -eq 0 ]
	[[ "$output" == *"present  /etc/systemd/oomd.conf.d/10-steamdeck.conf"* ]]
	[[ "$output" == *"absent   /etc/sysctl.d/99-steamdeck-setup.conf"* ]]
}

# ---- safety ---------------------------------------------------------------------------------------

@test "with --root nothing outside the root is written" {
	before=$(ls -la /etc/sysctl.d /etc/systemd 2>/dev/null | md5sum)
	run "$ROOT/setup" --system --root "$T" all
	[ "$status" -eq 0 ]
	after=$(ls -la /etc/sysctl.d /etc/systemd 2>/dev/null | md5sum)
	[ "$before" = "$after" ]
	[ -e "$T/etc/sysctl.d/99-steamdeck-setup.conf" ]
}

@test "an existing managed file is replaced as a whole, an unmanaged one in the same directory is not touched" {
	mkdir -p "$T/etc/sysctl.d"
	printf 'vm.swappiness = 5\n' > "$T/etc/sysctl.d/10-mine.conf"
	"$ROOT/setup" --system --root "$T" sysctl >/dev/null
	"$ROOT/setup" --system --root "$T" --swappiness 70 sysctl >/dev/null
	[ "$(cat "$T/etc/sysctl.d/10-mine.conf")" = 'vm.swappiness = 5' ]
	grep -q '^vm.swappiness = 70$' "$T/etc/sysctl.d/99-steamdeck-setup.conf"
	[ "$(grep -c swappiness "$T/etc/sysctl.d/99-steamdeck-setup.conf")" -eq 1 ]
}

@test "undo twice and undo with nothing written are harmless" {
	run "$ROOT/setup" --system --root "$T" undo
	[ "$status" -eq 0 ]
	"$ROOT/setup" --system --root "$T" oom >/dev/null
	"$ROOT/setup" --system --root "$T" undo >/dev/null
	run "$ROOT/setup" --system --root "$T" undo
	[ "$status" -eq 0 ]
}

@test "a swap size that is not a size never reaches rm or fallocate" {
	for bad in '8G; rm -rf /' '$(id)' '-1G' 'G' ''; do
		run "$ROOT/setup" --system --root "$T" swapfile "$bad"
		[ "$status" -ne 0 ]
		[[ "$output" != *"+ rm"* ]]
	done
}
