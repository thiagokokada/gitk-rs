{ pkgs, gitk-rs }:
pkgs.writeShellApplication {
  name = "gitk-rs-update-screenshot";
  runtimeInputs = with pkgs; [
    git
    imagemagick
    python3
    xdotool
    xwininfo
  ];
  text = ''
    export LD_LIBRARY_PATH=${
      pkgs.lib.makeLibraryPath [ pkgs.libx11 ]
    }''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
    exec python -B ${./scripts}/update_screenshot.py --binary ${pkgs.lib.getExe gitk-rs} "$@"
  '';
}
