# The installer as a bootable ISO: it starts the installer on the first console.
# The online ISO fetches what it installs; the nixpkgs and disko the installed
# system pins come from the installer's own lock.
{
  nixpkgs,
  installer,
  system,
  extraModules ? [ ],
}:
nixpkgs.lib.nixosSystem {
  inherit system;
  modules = [
    "${nixpkgs}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
  ]
  ++ extraModules
  ++ [
    (
      { lib, ... }:
      {
        networking.hostName = "nun-installer";
        image.baseName = lib.mkForce "nun-installer-${system}";
        isoImage.volumeID = "NUN_INSTALLER";

        nix.settings.experimental-features = [
          "nix-command"
          "flakes"
        ];
      }
    )
    # The profile logs `nixos` in on the first console and the installer starts
    # there. It escalates through the profile's passwordless sudo.
    (import ./session.nix { inherit installer; })
  ];
}
