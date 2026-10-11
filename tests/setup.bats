#!/usr/bin/env bats
# setup: the wizard. Answers come from stdin; --dry-run and --root keep it away from the real system.

setup() {
	ROOT=$BATS_TEST_DIRNAME/..
	T=$(mktemp -d)
}

teardown() { rm -rf "$T"; }

@test "the defaults install the programs and set the OOM guard and sysctl, but not ssh" {
	run bash -c "printf '\n\n\n\n\n\n\n\n' | '$ROOT/setup' --dry-run --root '$T'"
	[ "$status" -eq 0 ]
	[[ "$output" == *"install ProtonUp and Flatseal"* ]]
	[[ "$output" == *"== oom"* ]]
	[[ "$output" == *"== sysctl"* ]]
	[[ "$output" != *"== ssh"* ]]
	[[ "$output" != *"== swapfile"* ]]
}

@test "answering no to everything changes nothing" {
	run bash -c "printf 'n\nn\nn\nkeep\nn\n' | '$ROOT/setup' --dry-run --root '$T'"
	[ "$status" -eq 0 ]
	[[ "$output" == *"(nothing)"* ]]
	[ -z "$(ls -A "$T")" ]
}

@test "a swap size and ssh are passed on, and 'no' at the summary cancels" {
	run bash -c "printf 'n\ny\nn\n100\n16G\ny\ny\n' | '$ROOT/setup' --dry-run --root '$T'"
	[ "$status" -eq 0 ]
	[[ "$output" == *"== swapfile 16G"* ]]
	[[ "$output" == *"== ssh"* ]]
	run bash -c "printf 'n\ny\nn\n100\nkeep\nn\nn\n' | '$ROOT/setup' --dry-run --root '$T'"
	[[ "$output" == *"cancelled"* ]]
}

@test "bad answers are asked again" {
	run bash -c "printf 'n\nn\nn\nhuge\n8G\nn\ny\n' | '$ROOT/setup' --dry-run --root '$T'"
	[[ "$output" == *"not a valid value"* ]]
	[[ "$output" == *"== swapfile 8G"* ]]
}
