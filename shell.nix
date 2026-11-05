{
  lib,
  craneLib,
  checks,
  stdenv,
  git,
  python3,
  ruff,
  gitk-rs,
}:

craneLib.devShell (
  {
    name = "gitk-rs-development";
    inherit checks;
    inputsFrom = [ gitk-rs ];
    packages = [
      git
      python3
      ruff
    ];
  }
  // lib.optionalAttrs stdenv.hostPlatform.isLinux {
    LD_LIBRARY_PATH = lib.makeLibraryPath gitk-rs.buildInputs;
  }
)
