{ pkgs, ... }:
{
  languages.rust.enable = true;

  packages = with pkgs; [
    disko
    nixfmt-rfc-style
    cargo-insta
  ];
}
