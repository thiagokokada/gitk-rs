{
  pkgs,
  gitk-rs,
}:
let
  inherit (pkgs)
    runCommand
    rustfmt
    nixfmt
    desktop-file-utils
    librsvg
    ;
in
{
  package = gitk-rs;

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
        python scripts/release_probe.py --help > /dev/null
        python -B -m unittest discover -s scripts -v
        touch $out
      '';

  format =
    runCommand "gitk-rs-format"
      {
        nativeBuildInputs = [
          rustfmt
          nixfmt
        ];
      }
      ''
        cd ${gitk-rs.src}
        find src tests -name '*.rs' -print0 | xargs -0 rustfmt --edition 2024 --check
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
