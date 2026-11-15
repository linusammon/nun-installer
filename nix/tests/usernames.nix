# Every account NixOS defines for the generated configuration must be refused as
# the login user's name: such a name makes the configuration fail to evaluate, or
# merges the user into a daemon account, after the disk has been wiped. This
# keeps the list in the installer from drifting when nixpkgs changes.
{
  pkgs,
  installer,
  disko,
  nixpkgsSrc,
  nixpkgsNarHash,
}:
let
  inherit (pkgs) lib;
  answersFile = pkgs.writeText "answers.json" (
    builtins.toJSON {
      schema_version = 1;
      disk = {
        device = "/dev/vda";
        swap_size_mib = 0;
        encrypt = false;
      };
      user = {
        name = "alice";
        password_hash = "$6$nunsalt1$Hb.bACmNA/De96M6wpOm3Hh9FtswGIrqcRQUDXbB0vaLgZp5JiF4vpJRqz6mzaRcJuzfu7rWHcNj4ChlHLe/4/";
      };
      system = {
        hostname = "nun";
        locale = "en_US.UTF-8";
        timezone = "Europe/Berlin";
        keyboard_layout = "us";
      };
    }
  );
  inherit
    (import ../target.nix {
      inherit
        pkgs
        installer
        disko
        nixpkgsSrc
        nixpkgsNarHash
        answersFile
        ;
    })
    target
    ;
  # Our own login user is the one that is meant to be there.
  taken = lib.attrNames (lib.filterAttrs (_: u: !u.isNormalUser) target.config.users.users);
in
pkgs.runCommand "nun-usernames"
  {
    nativeBuildInputs = [ pkgs.gnused ];
    NUN_BOOT = "uefi";
  }
  ''
    try() {
      sed "s/\"name\":\"alice\"/\"name\":\"$1\"/" ${answersFile} > answers.json
      ${installer}/bin/nun-installer --config answers.json --render out 2>err
    }

    try alice || { echo "alice should be accepted"; cat err; exit 1; }

    bad=0
    for name in ${lib.escapeShellArgs taken}; do
      if try "$name"; then
        echo "NixOS defines the account '$name' but the installer accepts it as a username"
        bad=1
      fi
    done
    [ "$bad" = 0 ] || exit 1
    touch $out
  ''
