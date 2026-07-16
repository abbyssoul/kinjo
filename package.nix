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
    # The default mdns-sd backend is pure-Rust multicast (no C deps), so this
    # would build on Darwin too; kept Linux-only because that's what's tested.
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
