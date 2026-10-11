#!/usr/bin/env bats
# setup: the wizard. Answers come from stdin; --dry-run and --root keep it away from the real system.

setup() {
	ROOT=$BATS_TEST_DIRNAME/..
	T=$(mktemp -d)
}

teardown() { rm -rf "$T"; }

@test "the defaults install the programs and set the OOM guard and sysctl, but not ssh" {
	mkdir -p "$T/dist"
	printf '#!/bin/sh\n' > "$T/dist/steamdeck-x86_64-linux"
	(cd "$T/dist" && sha256sum steamdeck-x86_64-linux > SHA256SUMS)
	run bash -c "printf '\n\n\n\n\n\n\n\n\n\n' | '$ROOT/setup' --dry-run --root '$T' --from '$T/dist'"
	[ "$status" -eq 0 ]
	[[ "$output" == *"install the Steam Deck program"* ]]
	[[ "$output" == *"== oom"* ]]
	[[ "$output" == *"== sysctl"* ]]
	[[ "$output" != *"== ssh"* ]]
	[[ "$output" != *"== swapfile"* ]]
}

@test "answering no to everything changes nothing" {
	run bash -c "printf '1\nn\nn\nn\nkeep\nn\nn\n' | '$ROOT/setup' --dry-run --root '$T'"
	[ "$status" -eq 0 ]
	[[ "$output" == *"(nothing)"* ]]
	[ -z "$(ls -A "$T")" ]
}

@test "a swap size and ssh are passed on, and 'no' at the summary cancels" {
	run bash -c "printf '1\nn\ny\nn\n100\n16G\ny\nn\ny\n' | '$ROOT/setup' --dry-run --root '$T'"
	[ "$status" -eq 0 ]
	[[ "$output" == *"== swapfile 16G"* ]]
	[[ "$output" == *"== ssh"* ]]
	run bash -c "printf '1\nn\ny\nn\n100\nkeep\nn\nn\nn\n' | '$ROOT/setup' --dry-run --root '$T'"
	[[ "$output" == *"cancelled"* ]]
}

@test "bad answers are asked again" {
	run bash -c "printf '1\nn\nn\nn\nhuge\n8G\nn\nn\ny\n' | '$ROOT/setup' --dry-run --root '$T'"
	[[ "$output" == *"not a valid value"* ]]
	[[ "$output" == *"== swapfile 8G"* ]]
}

@test "option 2 shows the state and option 3 removes what was installed" {
	export HOME=$T/home XDG_CONFIG_HOME=$T/home/.config STEAMDECK_NO_SYSTEMD=1
	mkdir -p "$HOME/.local/bin"
	: > "$HOME/.local/bin/protonup"; chmod +x "$HOME/.local/bin/protonup"
	run bash -c "printf '2\n' | '$ROOT/setup' --dry-run --root '$T/r'"
	[ "$status" -eq 0 ]
	[[ "$output" == *"installed  $HOME/.local/bin/protonup"* ]]
	[[ "$output" == *"missing    $HOME/.local/bin/steamdeck"* ]]
	run bash -c "printf '3\ny\nn\n' | '$ROOT/setup' --root '$T/r'"
	[ "$status" -eq 0 ]
	[ ! -e "$HOME/.local/bin/protonup" ]
}
