# The system the installer would install for some answers, built by NixOS from the
# files the installer itself renders, so tests cannot drift from the templates.
# Nothing here is installed: building it puts the closure of the target system and
# of the disko scripts into a VM's store, which has no network to fetch anything.
{
  pkgs,
  installer,
  disko,
  nixpkgsSrc,
  nixpkgsNarHash,
  answersFile,
}:
let
  inherit (pkgs) lib;
  system = pkgs.stdenv.hostPlatform.system;

  # The VM boots with UEFI; the build sandbox has no /sys/firmware/efi to tell.
  rendered =
    pkgs.runCommand "nun-rendered"
      {
        NUN_BOOT = "uefi";
      }
      ''
        ${installer}/bin/nun-installer --config ${answersFile} --render $out
      '';

  # Loaded the same way the VM loads it (as a path flake) so that documentation
  # derivations, which embed the revision, come out identical.
  nixpkgsFlake = builtins.getFlake (
    builtins.unsafeDiscardStringContext "path:${nixpkgsSrc}?narHash=${nixpkgsNarHash}"
  );
in
{
  inherit rendered;
  target = nixpkgsFlake.lib.nixosSystem {
    inherit system;
    modules = [
      disko.nixosModules.disko
      "${rendered}/disko.nix"
      "${rendered}/configuration.nix"
      # What nixos-generate-config finds on the machine; here, the test VM's.
      {
        boot.initrd.availableKernelModules = [
          "virtio_pci"
          "virtio_blk"
          "ahci"
          "xhci_pci"
          "sd_mod"
        ];
      }
    ];
  };
}
