# Boot the ISO like a machine would, from a CD under UEFI, and check that the
# installer comes up on the first console. The ISO under test has the test
# backdoor added; it is otherwise the shipped system.
{
  pkgs,
  nixpkgs,
  installer,
  system,
}:
let
  testing = import "${nixpkgs}/nixos/lib/testing-python.nix" { inherit system pkgs; };
  iso =
    (import ../iso.nix {
      inherit nixpkgs installer system;
      extraModules = [
        "${nixpkgs}/nixos/modules/testing/test-instrumentation.nix"
        { nixpkgs.pkgs = pkgs; }
      ];
    }).config.system.build.isoImage;
  qemu = "${pkgs.qemu_test}/bin/qemu-system-x86_64";
in
testing.makeTest {
  meta.timeout = 900;
  name = "nun-iso-boot";
  nodes = { };
  testScript = ''
    machine = create_machine(
        "${qemu} -m 4096 -smp 2"
        " -netdev user,id=net0 -device virtio-net-pci,netdev=net0"
        " -cdrom ${iso}/iso/${iso.isoName}"
        " -drive if=pflash,format=raw,unit=0,readonly=on,file=${pkgs.OVMF.firmware}"
        " -drive if=pflash,format=raw,unit=1,readonly=on,file=${pkgs.OVMF.variables}"
    )
    machine.start()
    machine.wait_for_unit("multi-user.target")

    with subtest("the installer runs as root on the first console"):
        try:
            with machine.nested("waiting for the installer"):
                for _ in range(60):
                    status, _out = machine.execute("pgrep -u root -f nun-installer")
                    if status == 0:
                        break
                    machine.sleep(2)
                else:
                    raise Exception("the installer never started")
        except Exception:
            print(machine.execute("systemctl status getty@tty1 --no-pager; ps -eo user,args | head -40; who; ls -l /dev/vcs1")[1])
            raise
        machine.wait_until_succeeds("grep -q 'Welcome to the NUN installer' /dev/vcs1", timeout=60)

    machine.shutdown()
  '';
}
