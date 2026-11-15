# End-to-end test: run the installer headlessly in a VM against a blank disk,
# then check what it left behind. The VM has no network, so everything the
# install needs must already be in its store.
{
  pkgs,
  installer,
  disko,
  nixpkgsSrc,
  nixpkgsNarHash,
  encrypt ? false,
  tui ? false,
}:
let
  inherit (pkgs) lib;
  system = pkgs.stdenv.hostPlatform.system;
  diskoSrc = disko.outPath;

  encryptionPassword = "correct horse battery";

  answers = {
    schema_version = 1;
    disk = {
      device = "/dev/vda";
      swap_size_mib = 512;
      inherit encrypt;
    } // lib.optionalAttrs encrypt { encryption_password = encryptionPassword; };
    user = {
      name = "alice";
      # hunter2hunter2
      password_hash = "$6$nunsalt1$Hb.bACmNA/De96M6wpOm3Hh9FtswGIrqcRQUDXbB0vaLgZp5JiF4vpJRqz6mzaRcJuzfu7rWHcNj4ChlHLe/4/";
    };
    system = {
      hostname = "nun-test";
      locale = "en_US.UTF-8";
      timezone = "Europe/Berlin";
      keyboard_layout = "ch";
      keyboard_variant = "fr";
    };
  };
  answersFile = pkgs.writeText "answers.json" (builtins.toJSON answers);

  instrumentation = "${nixpkgsSrc}/nixos/modules/testing/test-instrumentation.nix";

  env = lib.concatStringsSep " " [
    "NUN_NIXPKGS_URL=path:${nixpkgsSrc}"
    "NUN_DISKO_URL=path:${diskoSrc}"
  ];

  # Mirrors what the installer generates. Nothing here is installed: building it
  # puts the closure of the target system and of the disko scripts into the
  # VM's store, which has no network to fetch anything.
  # Loaded the same way the VM loads it (as a path flake) so that documentation
  # derivations, which embed the revision, come out identical.
  nixpkgsFlake = builtins.getFlake (
    builtins.unsafeDiscardStringContext "path:${nixpkgsSrc}?narHash=${nixpkgsNarHash}"
  );
  target = nixpkgsFlake.lib.nixosSystem {
    inherit system;
    modules = [ targetModule ];
  };
  targetModule =
    { ... }:
    let
      mountOptions = [
        "compress=zstd"
        "noatime"
      ];
      btrfs = {
        type = "btrfs";
        extraArgs = [ "-f" ];
        subvolumes = {
          "/root" = {
            mountpoint = "/";
            inherit mountOptions;
          };
          "/home" = {
            mountpoint = "/home";
            inherit mountOptions;
          };
          "/nix" = {
            mountpoint = "/nix";
            inherit mountOptions;
          };
          "/swap" = {
            mountpoint = "/.swapvol";
            swap.swapfile.size = "512M";
          };
        };
      };
    in
    {
      imports = [ disko.nixosModules.disko ];

      disko.devices.disk.main = {
        type = "disk";
        device = "/dev/vda";
        content = {
          type = "gpt";
          partitions = {
            ESP = {
              size = "512M";
              type = "EF00";
              content = {
                type = "filesystem";
                format = "vfat";
                mountpoint = "/boot";
                mountOptions = [ "umask=0077" ];
              };
            };
            root = {
              size = "100%";
              content =
                if encrypt then
                  {
                    type = "luks";
                    name = "cryptroot";
                    passwordFile = "/run/nun-installer/luks-password";
                    settings.allowDiscards = true;
                    content = btrfs;
                  }
                else
                  btrfs;
            };
          };
        };
      };

      boot.loader.systemd-boot.enable = true;
      boot.loader.efi.canTouchEfiVariables = true;
      boot.initrd.availableKernelModules = [
        "virtio_pci"
        "virtio_blk"
        "ahci"
        "xhci_pci"
        "sd_mod"
      ];

      networking.hostName = "nun-test";
      i18n.defaultLocale = "en_US.UTF-8";
      time.timeZone = "Europe/Berlin";
      services.xserver.xkb = {
        layout = "ch";
        variant = "fr";
      };
      console.useXkbConfig = true;
      users.users.alice = {
        isNormalUser = true;
        extraGroups = [
          "wheel"
          "networkmanager"
        ];
      };
      programs.noctalia = {
        enable = true;
        systemd.enable = true;
        recommendedServices.enable = true;
      };
      programs.umbriel.enable = true;
      services.displayManager.noctalia-greeter.enable = true;
      nix.settings.experimental-features = [
        "nix-command"
        "flakes"
      ];
      system.stateVersion = lib.trivial.release;
    };
  targetBuild = target.config.system.build;
in
pkgs.testers.runNixOSTest {
  name = "nun-installer-${if tui then "tui-" else ""}${if encrypt then "encrypted" else "plain"}";

  passthru = { inherit target; };

  # A hung boot should fail, not run for hours.
  globalTimeout = 1200;

  nodes.installer =
    { ... }:
    {
      virtualisation = {
        # Builds and evaluates inside the VM, so it needs room.
        cores = 8;
        memorySize = 8192;
        diskSize = 24 * 1024;
        # The target disk is /dev/vda; the installer itself runs from a small /dev/vdb.
        diskImage = "./target.qcow2";
        emptyDiskImages = [ 1024 ];
        rootDevice = "/dev/vdb";
        fileSystems."/".autoFormat = true;
        # The installer picks the bootloader from how it was booted.
        useEFIBoot = true;
      };

      nix = {
        settings = {
          experimental-features = [
            "nix-command"
            "flakes"
          ];
          substituters = lib.mkForce [ ];
          hashed-mirrors = null;
          connect-timeout = 1;
        };
      };

      environment.systemPackages = [
        installer
        disko.packages.${system}.disko
        pkgs.cryptsetup
        pkgs.tmux
        pkgs.kbd
      ];

      # No network: everything nixos-install and disko build must already be here.
      system.extraDependencies =
        [
          targetBuild.toplevel
          targetBuild.destroyFormatMount
          targetBuild.formatScript
          targetBuild.mountScript
        ]
        ++ (with pkgs; [
          stdenv
          stdenvNoCC
          bintools
          brotli
          brotli.dev
          brotli.lib
          desktop-file-utils
          docbook5
          docbook_xsl_ns
          hello
          kbd.dev
          kmod.dev
          libarchive.dev
          libcap-text-verifier
          libxml2.bin
          libxslt.bin
          nixos-artwork.wallpapers.simple-dark-gray-bottom
          perlPackages.ConfigIniFiles
          perlPackages.FileSlurp
          perlPackages.JSON
          perlPackages.ListCompare
          perlPackages.XMLLibXML
          (python3.withPackages (p: [ p.mistune ]))
          shared-mime-info
          sudo
          switch-to-configuration-ng
          texinfo
          unionfs-fuse
          lndir
          shellcheck-minimal
          systemd.out
          curl
          zstd.bin
          mypy
        ]);
    };

  nodes.target = {
    virtualisation = {
      cores = 4;
      memorySize = 4096;
      diskSize = 24 * 1024;
      # Boot the disk the installer just wrote.
      diskImage = "./target.qcow2";
      useBootLoader = true;
      useEFIBoot = true;
      useDefaultFilesystems = false;
      efi.keepVariables = false;
      fileSystems."/" = {
        device = "/dev/disk/by-label/this-is-not-real-and-will-never-be-used";
        fsType = "ext4";
      };
    };
  };

  testScript = ''
    encrypted = ${if encrypt then "True" else "False"}
    tui = ${if tui then "True" else "False"}

    def send(*keys):
        installer.succeed("tmux send-keys -t nun " + " ".join(keys))

    def type_text(text):
        installer.succeed(f"tmux send-keys -t nun -l '{text}'")

    def wait_for(text):
        installer.wait_until_succeeds(f"tmux capture-pane -pt nun | grep -qF '{text}'", timeout=300)

    installer.start()
    installer.wait_for_unit("multi-user.target")

    installer.succeed("test -d /sys/firmware/efi")
    installer.succeed("lsblk >&2")

    if tui:
        with subtest("interactive install"):
            # No network in the VM, so say the install medium carries everything.
            installer.succeed("tmux new-session -d -x 120 -y 40 -s nun 'NUN_OFFLINE=1 ${env} nun-installer'")
            wait_for("Press Enter to continue.")
            send("Enter")

            # The keyboard comes first, so the searches below use the chosen layout.
            wait_for("Layout —")
            type_text("switzerland")
            send("Enter")
            # The layout has variants, so Enter moved on to the variant list.
            wait_for("Variant of \"German (Switzerland)\"")
            type_text("french")
            send("Enter")

            wait_for("Timezone —")
            type_text("Europe/Berlin")
            send("Enter")

            wait_for("Language —")
            send("Enter")

            wait_for("Disk — choose")
            installer.succeed("tmux capture-pane -pt nun | grep -q 'unavailable'")  # the installer's own disk
            installer.succeed("tmux capture-pane -pt nun >&2")
            send("Enter")

            # Disk, swap and encryption are one screen; Enter moves field to field
            # and the last one submits.
            wait_for("Swap size")
            type_text("512")
            send("Enter")  # swap -> encryption
            if encrypted:
                send("Space", "Enter")  # toggle, then to the passphrase
                type_text("${encryptionPassword}")
                send("Enter")
                type_text("${encryptionPassword}")
            send("Enter")

            wait_for("Confirm password")
            type_text("alice")
            send("Tab")
            type_text("hunter2hunter2")
            send("Tab")
            type_text("hunter2hunter2")
            send("Enter")

            wait_for("Hostname —")
            send("BSpace BSpace BSpace")
            type_text("nun-test")
            send("Enter")

            wait_for("EVERYTHING ON /dev/vda WILL BE ERASED")
            type_text("vda")
            send("Enter")

            wait_for("Installation complete")
            send("q")

        with subtest("the chosen keyboard layout was loaded into the console"):
            # Swiss French puts é where German has ö; the default US map has neither.
            installer.succeed("dumpkeys | grep -E '^keycode +39 =' >&2")
            installer.succeed("dumpkeys | grep -E '^keycode +39 =' | grep -q eacute")
    else:
        with subtest("dry run touches nothing"):
            installer.succeed("${env} nun-installer --config ${answersFile} --dry-run >&2")
            installer.fail("test -e /mnt/etc")

        with subtest("refuses to wipe without --yes"):
            installer.fail("${env} nun-installer --config ${answersFile}")

        with subtest("install"):
            installer.succeed("${env} nun-installer --config ${answersFile} --yes >&2")

    with subtest("generated config is in place"):
        for f in ["flake.nix", "configuration.nix", "disko.nix", "hardware-configuration.nix"]:
            installer.succeed(f"test -s /mnt/etc/nixos/{f}")
        installer.succeed("grep -q 'programs.umbriel.enable = true' /mnt/etc/nixos/configuration.nix")

    with subtest("password is set but not in the config"):
        installer.succeed("grep -Eq '^alice:[$][y6][$]' /mnt/etc/shadow")
        installer.fail("grep -rqE '[$][y6][$]' /mnt/etc/nixos")
        installer.succeed("nixos-enter --root /mnt -c 'id -nG alice' | grep -qw wheel")

    with subtest("secrets are cleaned up"):
        installer.fail("test -e /run/nun-installer/luks-password")
        installer.fail("test -e /run/nun-installer")

    with subtest("bootloader is installed"):
        installer.succeed("test -e /mnt/boot/EFI/BOOT/BOOTX64.EFI")
        installer.succeed("test -L /mnt/nix/var/nix/profiles/system")

    if encrypted:
        with subtest("disk is really encrypted"):
            installer.succeed("lsblk -f /dev/vda | grep -q crypto_LUKS")
            installer.fail("grep -rq '${encryptionPassword}' /mnt/etc/nixos")

    # The installed system has no way to talk to the test driver, so add the
    # instrumentation module to the generated flake and install once more.
    with subtest("add test instrumentation"):
        installer.succeed(
            r"""sed -i '/^ *\.\/configuration\.nix$/a\          "${instrumentation}"' /mnt/etc/nixos/flake.nix"""
        )
        installer.succeed("grep -q test-instrumentation /mnt/etc/nixos/flake.nix")
        installer.succeed("nixos-install --root /mnt --no-root-passwd --flake /mnt/etc/nixos#nun-test >&2")

    installer.succeed("sync; umount -R /mnt")
    if encrypted:
        installer.succeed("cryptsetup close cryptroot")
    installer.shutdown()

    with subtest("installed system boots"):
        # Same disk image as the installer wrote to.
        target.state_dir = installer.state_dir
        target.start()
        if encrypted:
            target.wait_for_console_text("passphrase for")
            target.send_console("${encryptionPassword}\n")
        target.wait_for_unit("multi-user.target", timeout=300)

    with subtest("installed system is what we asked for"):
        target.succeed("hostname | grep -qx nun-test")
        target.succeed("id -nG alice | grep -qw wheel")
        target.succeed("su alice -c 'echo hunter2hunter2 | sudo -S -k true'")
        target.succeed("findmnt -no FSTYPE / | grep -qx btrfs")
        target.succeed("swapon --show=NAME --noheadings | grep -q swap")
        target.succeed("timedatectl show -p Timezone --value | grep -qx Europe/Berlin")
        target.succeed("systemctl --failed >&2")
  '';
}
