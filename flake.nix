{
  description = "kinjo — TUI browser and command launcher for DNS-SD services";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems
        (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forAllSystems (pkgs: rec {
        kinjo = pkgs.callPackage ./package.nix { };
        default = kinjo;
      });

      # An overlay is a single function (final: prev: { ... }) — NOT per-system —
      # so it lives at the top level, a sibling of packages/apps/devShells.
      # Build via final.callPackage so overlay users get kinjo built against
      # *their* nixpkgs, not this flake's locked copy.
      overlays.default = final: _prev: {
        kinjo = final.callPackage ./package.nix { };
      };

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.kinjo;
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          # rust-toolchain.toml pins Rust 1.94, but that's a rustup convention —
          # nixpkgs' cargo/rustc don't read it, so this shell (and the package
          # build) intentionally track nixpkgs' Rust to keep nixpkgs as the only
          # flake input. `nix develop` may hand you a newer Rust than CI's 1.94.
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer ];
        };
      });
    };
}
