{
  lib,
  mkShell,
  cargo,
  rustc,
  rustfmt,
  clippy,
  git,
  python3,
  ruff,
  gitk-rs,
}:

mkShell {
  name = "gitk-rs-development";
  inputsFrom = [ gitk-rs ];
  packages = [
    cargo
    rustc
    rustfmt
    clippy
    git
    python3
    ruff
  ];
  LD_LIBRARY_PATH = lib.makeLibraryPath gitk-rs.buildInputs;
}
