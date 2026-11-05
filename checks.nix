{
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
}
