{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "wl-mx-gesture";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../config
      ../packaging
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  postInstall = ''
    install -Dm644 packaging/70-wl-mx-gesture.rules -t $out/lib/udev/rules.d
  '';

  meta = {
    description = "Gestures and action wheel for the Logitech MX Master gesture button on Wayland";
    license = lib.licenses.mit;
    mainProgram = "wl-mx-gesture";
    platforms = lib.platforms.linux;
  };
}
