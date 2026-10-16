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
    outputHashes = {
      "collections-0.1.0" = "sha256-iYYEO86G8XBzET58oEyreGG7RlVbPHHwoqVSzN7rj/k=";
      "gpui-component-0.5.2" = "sha256-yOXdgxQgfvGN2/+OdDnl1pYti0DoGFvS3Tyqvj3Bkng=";
      "zed-font-kit-0.14.1-zed" = "sha256-KXygi0olNQi5yM8eaJVykNDtbPMDjT+cWPBF8UrtXR4=";
      "zed-reqwest-0.12.15-zed" = "sha256-p4SiUrOrbTlk/3bBrzN/mq/t+1Gzy2ot4nso6w6S+F8=";
      "zed-scap-0.0.8-zed" = "sha256-BihiQHlal/eRsktyf0GI3aSWsUCW7WcICMsC2Xvb7kw=";
      "xim-ctext-0.3.0" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
    };
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
