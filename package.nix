{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "kinjo";
  version = (lib.importTOML ./Cargo.toml).package.version;

  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    description = "kinjo — TUI browser and command launcher for DNS-SD services";
    homepage = "https://github.com/abbyssoul/kinjo";
    license = lib.licenses.mit;
    mainProgram = "kinjo";
    # The default mdns-sd backend is pure-Rust multicast (no C deps), and CI
    # builds and tests the crate on macOS, which also ships as a Homebrew
    # formula. The restriction is about this flake, not the crate: the Nix
    # build has only ever been exercised on Linux. Extending it to Darwin is a
    # follow-up.
    platforms = lib.platforms.linux;
    maintainers = [
      {
        name = "pliski";
        github = "pliski";
        githubId = 6731247;
      }
    ];
  };
}
