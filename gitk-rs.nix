{
  lib,
  stdenv,
  autoPatchelfHook,
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
  revision ? "unknown",
  vulkan-loader,
  wayland,
  zlib,
}:

let
  baseVersion = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
  commonArgs = {
    pname = "gitk-rs";
    version = "${baseVersion}-${revision}";
    GITK_RS_REVISION = revision;
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        (lib.fileset.fromSource (craneLib.cleanCargoSource ./.))
        ./LICENSE
        ./resources
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
      # Keep dependency artifacts reusable across application revisions.
      version = baseVersion;
      GITK_RS_REVISION = "unknown";
      cargoTestExtraArgs = "--all-targets --no-run";
    }
  );
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    nativeBuildInputs =
      commonArgs.nativeBuildInputs
      ++ [ makeWrapper ]
      ++ lib.optionals stdenv.hostPlatform.isLinux [ autoPatchelfHook ];
    buildInputs =
      commonArgs.buildInputs ++ lib.optionals stdenv.hostPlatform.isLinux [ stdenv.cc.cc.lib ];
    # GPUI loads these by name with dlopen, so linking alone cannot retain them.
    runtimeDependencies = lib.optionals stdenv.hostPlatform.isLinux (
      map lib.getLib [
        vulkan-loader
        wayland
      ]
    );
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
