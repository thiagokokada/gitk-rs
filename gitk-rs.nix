{
  lib,
  clang,
  cmake,
  fontconfig,
  freetype,
  git,
  libx11,
  libxcb,
  libxcursor,
  libxi,
  libxkbcommon,
  libxrandr,
  makeWrapper,
  openssl,
  pkg-config,
  rustPlatform,
  vulkan-loader,
  wayland,
  zlib,
}:

rustPlatform.buildRustPackage rec {
  pname = "gitk-rs";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.lock
      ./Cargo.toml
      ./LICENSE
      ./resources
      ./src
      ./tests
      ./vendor
    ];
  };

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    clang
    cmake
    makeWrapper
    pkg-config
    rustPlatform.bindgenHook
  ];
  nativeCheckInputs = [ git ];
  buildInputs = [
    fontconfig
    freetype
    libx11
    libxcb
    libxcursor
    libxi
    libxkbcommon
    libxrandr
    openssl
    vulkan-loader
    wayland
    zlib
  ];

  postInstall = ''
    mkdir -p $out/share/doc/gitk-rs
    cp LICENSE $out/share/doc/gitk-rs/
    install -Dm644 resources/icons/gitk-rs.svg $out/share/icons/hicolor/scalable/apps/gitk-rs.svg
    install -Dm644 resources/gitk-rs.desktop $out/share/applications/gitk-rs.desktop
    substituteInPlace $out/share/applications/gitk-rs.desktop \
      --replace-fail "Exec=gitk-rs" "Exec=$out/bin/gitk-rs"
  '';

  postFixup = ''
    wrapProgram $out/bin/gitk-rs \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath buildInputs} \
      --prefix PATH : ${lib.makeBinPath [ git ]}
  '';

  meta = {
    description = "Git history explorer built with GPUI";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "gitk-rs";
  };
}
