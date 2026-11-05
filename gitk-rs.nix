{
  lib,
  stdenv,
  clang,
  cmake,
  craneLib,
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

let
  commonArgs = {
    pname = "gitk-rs";
    version = "0.1.0";
    strictDeps = true;
    # bindgenHook copies NIX_CFLAGS_COMPILE into a tracked environment variable.
    # Keep its random seed identical across dependency and application builds.
    NIX_OUTPATH_USED_AS_RANDOM_SEED = builtins.hashFile "sha256" ./Cargo.lock;
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

    nativeBuildInputs = [
      clang
      cmake
      makeWrapper
      pkg-config
      rustPlatform.bindgenHook
    ];

    nativeCheckInputs = [ git ];

    buildInputs = [
      openssl
      zlib
    ]
    ++ lib.optionals stdenv.hostPlatform.isLinux [
      fontconfig
      freetype
      libx11
      libxcb
      libxcursor
      libxi
      libxkbcommon
      libxrandr
      vulkan-loader
      wayland
    ];

    preCheck = ''
      unset DISPLAY WAYLAND_DISPLAY
    '';
  };

  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      # Registry crates need the real API of our patched local dependency.
      # Keep its source in the dummy tree without depending on app sources.
      extraDummyScript = ''
        rm -r "$out/vendor/xattr"
        cp -r ${./vendor/xattr} "$out/vendor/xattr"
      '';
    }
  );
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    postInstall = ''
      mkdir -p $out/share/doc/gitk-rs
      cp LICENSE $out/share/doc/gitk-rs/
    ''
    + lib.optionalString stdenv.hostPlatform.isLinux ''
      install -Dm644 resources/icons/gitk-rs.svg $out/share/icons/hicolor/scalable/apps/gitk-rs.svg
      install -Dm644 resources/gitk-rs.desktop $out/share/applications/gitk-rs.desktop
      substituteInPlace $out/share/applications/gitk-rs.desktop \
        --replace-fail "Exec=gitk-rs" "Exec=$out/bin/gitk-rs"
    '';

    postFixup = ''
      wrapProgram $out/bin/gitk-rs \
        ${lib.optionalString stdenv.hostPlatform.isLinux "--prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath commonArgs.buildInputs}"} \
        --prefix PATH : ${lib.makeBinPath [ git ]}
    '';

    meta = {
      description = "Git history explorer built with GPUI";
      license = lib.licenses.mit;
      platforms = lib.platforms.linux ++ lib.platforms.darwin;
      mainProgram = "gitk-rs";
    };
  }
)
