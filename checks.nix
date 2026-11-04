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
        test "$(env -u DISPLAY -u WAYLAND_DISPLAY ${gitk-rs}/bin/gitk-rs --version)" = "gitk-rs ${gitk-rs.version}"
        touch $out
      '';
}
