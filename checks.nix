{
  pkgs,
  gitk-rs,
  craneLib,
}:
let
  inherit (pkgs)
    runCommand
    rustfmt
    nixfmt
    desktop-file-utils
    librsvg
    ;
  # Share compiled dependencies between sandboxed development checks. The
  # dependency derivation stays reusable when only application source changes.
  cargoArgs = {
    inherit (gitk-rs) src version buildInputs;
    pname = "gitk-rs-checks";
    nativeBuildInputs = with pkgs; [
      clang
      cmake
      pkg-config
      rustPlatform.bindgenHook
      git
    ];
    CARGO_PROFILE = "dev";
    CARGO_PROFILE_DEV_DEBUG = "0";
    CARGO_PROFILE_TEST_DEBUG = "0";
    preBuild = ''
      unset DISPLAY WAYLAND_DISPLAY
    '';
  };
  cargoArtifacts = craneLib.buildDepsOnly (
    cargoArgs
    // {
      cargoTestExtraArgs = "--all-targets --no-run";
      # Registry crates consume this patched local dependency's real API.
      # Preserve it instead of Crane's stub, without depending on app sources.
      extraDummyScript = ''
        rm -r "$out/vendor/xattr"
        cp -r ${./vendor/xattr} "$out/vendor/xattr"
      '';
    }
  );
in
{
  package = gitk-rs;

  clippy = craneLib.cargoClippy (
    cargoArgs
    // {
      inherit cargoArtifacts;
      doInstallCargoArtifacts = false;
      cargoClippyExtraArgs = "--all-targets -- -D warnings";
    }
  );

  tests = craneLib.cargoTest (
    cargoArgs
    // {
      inherit cargoArtifacts;
      doInstallCargoArtifacts = false;
      cargoTestExtraArgs = "--all-targets";
    }
  );

  scripts =
    runCommand "gitk-rs-scripts"
      {
        nativeBuildInputs = [
          pkgs.python3
          pkgs.ruff
        ];
      }
      ''
        cp -r ${./scripts} scripts
        cp ${./pyproject.toml} pyproject.toml
        ruff check scripts
        ruff format --check scripts
        python scripts/gui_smoke.py --help > /dev/null
        python -B -m unittest discover -s scripts -v
        touch $out
      '';

  format =
    runCommand "gitk-rs-format"
      {
        nativeBuildInputs = [
          pkgs.cargo
          rustfmt
          nixfmt
        ];
      }
      ''
        cd ${gitk-rs.src}
        cargo fmt --all -- --check
        nixfmt --check ${./flake.nix} ${./gitk-rs.nix} ${./shell.nix} ${./checks.nix}
        touch $out
      '';

  executable = runCommand "gitk-rs-executable" { } ''
    test "$(env -u DISPLAY -u WAYLAND_DISPLAY ${gitk-rs}/bin/gitk-rs --version)" = "gitk-rs ${gitk-rs.version}"
    touch $out
  '';
}
// pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
  desktop =
    runCommand "gitk-rs-desktop"
      {
        nativeBuildInputs = [
          desktop-file-utils
          librsvg
        ];
      }
      ''
        desktop-file-validate ${gitk-rs}/share/applications/gitk-rs.desktop
        rsvg-convert --width 32 --height 32 \
          ${gitk-rs}/share/icons/hicolor/scalable/apps/gitk-rs.svg > icon.png
        test -s icon.png
        touch $out
      '';
}
