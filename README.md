# nun-installer

TUI installer for the nun stack on NixOS: noctalia, noctalia-greeter and umbriel.

Status: early. The core, headless mode and VM tests work; the TUI is not built yet.

## What it does

Takes a handful of answers (disk, user, hostname, locale, timezone, keyboard),
generates a flake (`flake.nix`, `configuration.nix`, `disko.nix`) that enables
the three nun modules, then partitions the disk with
[disko](https://github.com/nix-community/disko) and runs `nixos-install`.

The disk layout is fixed: GPT, an ESP (UEFI) or BIOS boot partition, and btrfs
(`/root`, `/home`, `/nix`) with an optional swapfile and optional LUKS.

## Usage

```sh
nun-installer --config answers.json --dry-run   # print the plan and generated files
nun-installer --config answers.json --yes       # install, wipes the target disk
```

Without `--config` the interactive installer starts (not implemented yet).
See `crates/installer-tui/tests/fixtures/answers.json` for the answers format.

## Development

```sh
nix develop --impure       # devenv shell with the rust toolchain
cargo test
nix flake check --impure   # build, clippy, fmt, tests and the VM tests
```

The VM tests (`vm-install-plain`, `vm-install-encrypted`) install onto a blank
disk inside a NixOS test VM and then boot the result. They need KVM. Run one
with:

```sh
nix build .#checks.x86_64-linux.vm-install-encrypted --impure -L
```

`--impure` is needed by the devenv shell.

## Layout

- `crates/installer-core`: answers, validation, config generation, install plan, executors, safety checks. No UI.
- `crates/installer-tui`: the `nun-installer` binary.
- `nix/tests`: the VM tests.

## Environment

The Nix package sets these; outside Nix (or in the devenv shell) they fall back
to system paths or must be set by hand:

| Variable | Purpose |
|---|---|
| `NUN_ZONE_TAB`, `NUN_LOCALE_SUPPORTED`, `NUN_XKB_BASE_LST` | timezone, locale and keyboard lists |
| `NUN_STATE_VERSION` | `system.stateVersion` for the new system (required) |
| `NUN_NIXPKGS_PATH` | nixpkgs used to evaluate disko |
| `NUN_NIXPKGS_URL`, `NUN_DISKO_URL` | flake inputs written into the generated flake |
