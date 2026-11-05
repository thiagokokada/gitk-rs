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
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        (lib.fileset.fromSource (craneLib.cleanCargoSource ./.))
        ./LICENSE
        ./resources
        ./vendor
      ];
    };
    strictDeps = true;
    # bindgenHook forwards the compiler's random seed to bindgen. Keep it
    # identical across derivations sharing artifacts to avoid rebuilds.
    # https://crane.dev/faq/rebuilds-bindgen.html
    NIX_OUTPATH_USED_AS_RANDOM_SEED = "gitk-rs000";

    nativeBuildInputs = [
      clang
      cmake
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

    cargoTestExtraArgs = "--all-targets";
    preCheck = ''
      unset DISPLAY WAYLAND_DISPLAY
    '';
  };
  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      cargoTestExtraArgs = "--all-targets --no-run";
      # Registry crates need the real API of this patched local dependency.
      # Keep it intact while Crane stubs the application's sources.
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
    nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ makeWrapper ];
    passthru = { inherit commonArgs cargoArtifacts; };

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
