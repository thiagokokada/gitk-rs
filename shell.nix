{
  lib,
  mkShell,
  cargo,
  rustc,
  rustfmt,
  clippy,
  git,
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
  ];
  LD_LIBRARY_PATH = lib.makeLibraryPath gitk-rs.buildInputs;
}
