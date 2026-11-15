{
  description = "Installer for the nun stack (noctalia, noctalia-greeter, umbriel) on NixOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    devenv = {
      url = "github:cachix/devenv";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  nixConfig = {
    extra-substituters = [ "https://devenv.cachix.org" ];
    extra-trusted-public-keys = [
      "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw="
    ];
  };

  outputs =
    inputs@{
      self,
      nixpkgs,
      crane,
      devenv,
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
              || (lib.hasInfix "/tests/fixtures/" path);
            name = "source";
          };
          commonArgs = {
            inherit src;
            strictDeps = true;
            pname = "nun-installer";
            version = "0.1.0";
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
                  --set-default NUN_NIXPKGS_PATH ${pkgs.path}
              '';
              meta.mainProgram = "nun-installer";
            }
          );
        in
        {
          inherit installer;
          default = installer;
        }
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
          vm-install-plain = import ./nix/tests/install.nix {
            inherit pkgs;
            inherit (self.packages.${system}) installer;
            inherit disko;
            nixpkgsSrc = nixpkgs.outPath;
            nixpkgsNarHash = nixpkgs.narHash;
          };
          vm-install-encrypted = import ./nix/tests/install.nix {
            inherit pkgs;
            inherit (self.packages.${system}) installer;
            inherit disko;
            nixpkgsSrc = nixpkgs.outPath;
            nixpkgsNarHash = nixpkgs.narHash;
            encrypt = true;
          };
        }
      );

      formatter = forEachSystem (_: pkgs: pkgs.nixfmt-rfc-style);

      devShells = forEachSystem (
        system: pkgs: {
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [ ./devenv.nix ];
          };
        }
      );
    };
}
