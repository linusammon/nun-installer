# nun-installer

TUI installer for the nun stack on NixOS: noctalia, noctalia-greeter and umbriel.

Status: early. The interactive installer, headless mode and VM tests work.

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

Without `--config` the interactive installer starts (it needs a terminal). When not
root it re-runs itself through `sudo`. Welcome checks for internet access and a
usable disk, and warns about a wrong clock or running on battery power.
See `crates/installer-tui/tests/fixtures/answers.json` for the answers format.

## Development

```sh
devenv shell             # rust toolchain and dev tools (needs devenv)
cargo test
nix flake check         # build, clippy, fmt, tests and the VM tests
```

The VM tests (`vm-install-plain`, `vm-install-encrypted`, `vm-install-tui`) install
onto a blank disk inside a NixOS test VM and then boot the result. The last one
drives the interactive installer through tmux. They need KVM. Run one
with:

```sh
nix build .#checks.x86_64-linux.vm-install-encrypted -L
```

## Try it in a VM

Your own machine will not get past the Disk step, because its disks are in use.
A VM with a blank disk is the safe way to walk through the whole installer:

```sh
nix run .#vm-install   # boots a VM straight into the installer
nix run .#vm-boot      # boots the system the installer produced
```

`vm-install` opens a QEMU window running the real installer on a Linux console,
with UEFI, user-mode networking, and a blank 20 GiB disk at `./nun-target.qcow2`.
The VM's own disk shows up as unavailable, as it should. The install is online,
so it downloads the system. Both need KVM.

- When it finishes, close the window. Enter on the last screen reboots the
  installer VM, not the new system. Run `vm-boot` for that.
- Delete `nun-target.qcow2` to start from a blank disk again, or point
  `NUN_VM_DISK` somewhere else.
- Ctrl+Alt+G releases the mouse from the QEMU window.

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
| `NUN_CKBCOMP`, `NUN_LOADKEYS` | tools that load the chosen keyboard layout into the console |
| `NUN_OFFLINE=1` | the medium carries everything needed, so skip the internet check |
