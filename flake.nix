{
  description = "Installer for the nun stack (noctalia, noctalia-greeter, umbriel) on NixOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      disko,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system: f system nixpkgs.legacyPackages.${system});

      mkCommon =
        pkgs:
        let
          craneLib = crane.mkLib pkgs;
          lib = pkgs.lib;
          src = lib.cleanSourceWith {
            src = ./.;
            filter =
              path: type:
              (craneLib.filterCargoSources path type) || (lib.hasSuffix ".j2" path) || (lib.hasSuffix ".snap" path)
              || (lib.hasInfix "/tests/fixtures/" path)
              # A test checks that the repo's own lock reduces cleanly.
              || (baseNameOf path == "flake.lock");
            name = "source";
          };
          commonArgs = {
            inherit src;
            strictDeps = true;
            pname = "nun-installer";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          inherit craneLib commonArgs cargoArtifacts;
        };
    in
    {
      packages = forEachSystem (
        system: pkgs:
        let
          c = mkCommon pkgs;
          installer = c.craneLib.buildPackage (
            c.commonArgs
            // {
              inherit (c) cargoArtifacts;
              cargoExtraArgs = "-p installer-tui";
              nativeBuildInputs = [ pkgs.makeWrapper ];
              postInstall = ''
                wrapProgram $out/bin/nun-installer \
                  --set-default NUN_ZONE_TAB ${pkgs.tzdata}/share/zoneinfo/zone.tab \
                  --set-default NUN_LOCALE_SUPPORTED ${pkgs.glibcLocales}/share/i18n/SUPPORTED \
                  --set-default NUN_XKB_BASE_LST ${pkgs.xkeyboard-config}/share/X11/xkb/rules/base.lst \
                  --set-default NUN_STATE_VERSION ${pkgs.lib.trivial.release} \
                  --set-default NUN_NIXPKGS_PATH ${nixpkgs.outPath} \
                  --set-default NUN_DISKO_PATH ${disko.outPath} \
                  --set-default NUN_CKBCOMP ${pkgs.ckbcomp}/bin/ckbcomp \
                  --set-default NUN_LOADKEYS ${pkgs.kbd}/bin/loadkeys \
                  --set-default NUN_LOCK_FILE ${./flake.lock} \
                  --suffix PATH : ${pkgs.lib.makeBinPath [ disko.packages.${system}.disko ]}
              '';
              meta.mainProgram = "nun-installer";
            }
          );
        in
        {
          inherit installer;
          default = installer;
          # Nothing in the ISO is specific to one architecture; only its boot test is.
          iso-online =
            (import ./nix/iso.nix { inherit nixpkgs installer system; }).config.system.build.isoImage;
        }
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") (
          let
            vm = import ./nix/vm.nix {
              inherit pkgs nixpkgs installer system;
            };
          in
          {
            vm-install = vm.install;
            vm-boot = vm.boot;
          }
        )
      );

      apps = forEachSystem (
        system: _: {
          default = {
            type = "app";
            program = "${self.packages.${system}.installer}/bin/nun-installer";
          };
        }
      );

      checks = forEachSystem (
        system: pkgs:
        let
          c = mkCommon pkgs;
          vmTest =
            flags:
            import ./nix/tests/install.nix (
              {
                inherit pkgs disko;
                inherit (self.packages.${system}) installer;
                nixpkgsSrc = nixpkgs.outPath;
                nixpkgsNarHash = nixpkgs.narHash;
                nixpkgsRev = nixpkgs.rev;
              }
              // flags
            );
        in
        {
          inherit (self.packages.${system}) installer;
          clippy = c.craneLib.cargoClippy (
            c.commonArgs
            // {
              inherit (c) cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );
          fmt = c.craneLib.cargoFmt { inherit (c.commonArgs) src; };
          tests = c.craneLib.cargoTest (c.commonArgs // { inherit (c) cargoArtifacts; });
        }
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          iso-boot = import ./nix/tests/iso.nix {
            inherit pkgs nixpkgs system;
            inherit (self.packages.${system}) installer;
          };
          vm-install-plain = vmTest { };
          vm-install-encrypted = vmTest { encrypt = true; };
          vm-install-pinned = vmTest { pinned = true; };
          vm-install-tui = vmTest {
            encrypt = true;
            tui = true;
          };
          vm-install-tui-user = vmTest {
            tui = true;
            asUser = true;
          };
        }
      );

      formatter = forEachSystem (_: pkgs: pkgs.nixfmt-rfc-style);
    };
}
