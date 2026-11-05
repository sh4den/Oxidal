{
  lib,
  rustPlatform,
  pkg-config,
  cmake,
  perl,
  dbus,
  fontconfig,
  freetype,
  libxkbcommon,
  libxcb,
  systemd,
  vulkan-loader,
  wayland,
  zstd,
}:

let
  manifest = (lib.importTOML ../../Cargo.toml).package;
in
rustPlatform.buildRustPackage {
  pname = "oxidal";
  inherit (manifest) version;

  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../Cargo.toml
      ../../Cargo.lock
      ../../build.rs
      ../../src
      ../../assets
      ../../packaging/linux
    ];
  };

  cargoLock = {
    lockFile = ../../Cargo.lock;
  };

  nativeBuildInputs = [
    pkg-config
    cmake
    perl
    rustPlatform.bindgenHook
  ];

  buildInputs = [
    dbus
    fontconfig
    freetype
    libxkbcommon
    libxcb
    systemd
    wayland
    zstd
  ];

  env.ZSTD_SYS_USE_PKG_CONFIG = true;

  postInstall = ''
    mv $out/bin/Oxidal $out/bin/oxidal
    install -Dm644 packaging/linux/oxidal.desktop -t $out/share/applications
    for size in 16 32 48 64 128 256 512; do
      install -Dm644 packaging/linux/icons/$size.png \
        $out/share/icons/hicolor/''${size}x''${size}/apps/oxidal.png
    done
  '';

  postFixup = ''
    patchelf --add-rpath ${
      lib.makeLibraryPath [
        fontconfig
        vulkan-loader
        wayland
        libxkbcommon
      ]
    } $out/bin/oxidal
  '';

  meta = {
    description = "Cross-platform SSH, SFTP and serial terminal client";
    homepage = "https://github.com/sh4den/Oxidal";
    license = lib.licenses.gpl3Only;
    mainProgram = "oxidal";
    platforms = lib.platforms.linux;
  };
}
