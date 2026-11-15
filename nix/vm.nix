# Try the installer by hand: a VM that boots straight into it with a blank disk,
# and a second one that boots the system the installer produced.
{
  pkgs,
  nixpkgs,
  installer,
  system,
}:
let
  installerSystem = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      (
        { lib, ... }:
        {
          imports = [ "${nixpkgs}/nixos/modules/virtualisation/qemu-vm.nix" ];

          networking.hostName = "nun-installer";
          system.stateVersion = lib.trivial.release;

          virtualisation = {
            # nixos-install evaluates nixpkgs, which needs room.
            memorySize = 8192;
            cores = 4;
            diskSize = 8192;
            writableStoreUseTmpfs = false;
            # UEFI, like most real machines.
            useEFIBoot = true;
          };

          nix.settings.experimental-features = [
            "nix-command"
            "flakes"
          ];
          # Log in as root on the first console; the installer starts there.
          services.getty.autologinUser = "root";
        }
      )
      (import ./session.nix { inherit installer; })
    ];
  };
  runVm = "${installerSystem.config.system.build.vm}/bin/run-nun-installer-vm";
in
{
  install = pkgs.writeShellApplication {
    name = "nun-vm-install";
    runtimeInputs = [ pkgs.qemu_kvm ];
    text = ''
      disk="''${NUN_VM_DISK:-$PWD/nun-target.qcow2}"
      if [ ! -e /dev/kvm ]; then
        echo "error: /dev/kvm not found, the VM needs hardware virtualisation" >&2
        exit 1
      fi
      if [ ! -e "$disk" ]; then
        qemu-img create -f qcow2 "$disk" 20G >/dev/null
        echo "created blank target disk $disk"
      else
        echo "reusing target disk $disk (delete it to start from a blank disk)"
      fi

      # The installer's own disk is throwaway; only the target disk is kept.
      root="$(mktemp -u /tmp/nun-installer-root.XXXXXX.qcow2)"
      trap 'rm -f "$root"' EXIT
      export NIX_DISK_IMAGE="$root"
      export QEMU_OPTS="-drive file=$disk,if=virtio,format=qcow2,cache=writeback ''${QEMU_OPTS:-}"
      ${runVm}
    '';
  };

  boot = pkgs.writeShellApplication {
    name = "nun-vm-boot";
    runtimeInputs = [ pkgs.qemu_kvm ];
    text = ''
      disk="''${NUN_VM_DISK:-$PWD/nun-target.qcow2}"
      if [ ! -e "$disk" ]; then
        echo "error: no $disk yet, run nun-vm-install first" >&2
        exit 1
      fi
      exec qemu-kvm \
        -m 4096 -smp 4 \
        -bios ${pkgs.OVMF.fd}/FV/OVMF.fd \
        -drive file="$disk",if=virtio,format=qcow2 \
        -nic user,model=virtio-net-pci \
        -device virtio-vga-gl -display gtk,gl=on
    '';
  };
}
