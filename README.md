# nun-installer

TUI installer for the nun stack on NixOS: [noctalia](https://github.com/noctalia-dev), noctalia-greeter, umbriel.

Status: early scaffolding.

## Development

```sh
nix develop --impure   # devenv shell with rust toolchain
cargo run -p installer-tui
nix flake check
```
