{ pkgs, ... }:
{
  languages.rust.enable = true;

  env = {
    NUN_ZONE_TAB = "${pkgs.tzdata}/share/zoneinfo/zone.tab";
    NUN_LOCALE_SUPPORTED = "${pkgs.glibcLocales}/share/i18n/SUPPORTED";
    NUN_XKB_BASE_LST = "${pkgs.xkeyboard-config}/share/X11/xkb/rules/base.lst";
    NUN_STATE_VERSION = pkgs.lib.trivial.release;
    # Not the flake's pins: devenv has its own lock. Good enough for trying the
    # interactive screens; `nix run` or the VM tests are the representative runs.
    NUN_NIXPKGS_PATH = "${pkgs.path}";
    NUN_CKBCOMP = "${pkgs.ckbcomp}/bin/ckbcomp";
    NUN_LOADKEYS = "${pkgs.kbd}/bin/loadkeys";
  };

  packages = with pkgs; [
    disko
    nixfmt-rfc-style
    cargo-insta
  ];
}
