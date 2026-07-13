{
  description = "kinjo — TUI browser and command launcher for DNS-SD services";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems
        (system: f nixpkgs.legacyPackages.${system});
      version = (nixpkgs.lib.importTOML ./Cargo.toml).package.version;
    in {
      packages = forAllSystems (pkgs: rec {
        kinjo = pkgs.rustPlatform.buildRustPackage {
          pname = "kinjo";
          inherit version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          meta = {
            description = "kinjo — TUI browser and command launcher for DNS-SD services";
            homepage = "https://github.com/abbyssoul/kinjo";
            license = pkgs.lib.licenses.mit;
            mainProgram = "kinjo";
            platforms = pkgs.lib.platforms.linux;
          };
        };
        default = kinjo;
      });

      # An overlay is a single function (final: prev: { ... }) — NOT per-system —
      # so it lives at the top level, a sibling of packages/apps/devShells.
      overlays.default = final: prev: {
        kinjo = self.packages.${final.stdenv.hostPlatform.system}.kinjo;
      };

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.kinjo;
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer ];
        };
      });
    };
}
