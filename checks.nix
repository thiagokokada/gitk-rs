{
  pkgs,
  gitk-rs,
  craneLib,
}:
{
  cargoFmt = craneLib.cargoFmt { inherit (gitk-rs) src; };

  clippy = craneLib.cargoClippy (
    gitk-rs.commonArgs
    // {
      inherit (gitk-rs) cargoArtifacts;
      doInstallCargoArtifacts = false;
      cargoClippyExtraArgs = "--all-targets -- -D warnings";
    }
  );

  scripts =
    pkgs.runCommand "gitk-rs-scripts"
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
        python scripts/update_screenshot.py --help > /dev/null
        python -B -m unittest discover -s scripts -v
        touch $out
      '';

  nixfmt =
    pkgs.runCommand "gitk-rs-nixfmt"
      {
        src = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.fileFilter (file: file.hasExt "nix") ./.;
        };
        nativeBuildInputs = [
          pkgs.findutils
          pkgs.nixfmt
        ];
      }
      ''
        find "$src" -type f -name '*.nix' -exec nixfmt --check {} +
        touch $out
      '';
}
