{ pkgs, gitk-rs }:

let
  smoke = pkgs.writeShellApplication {
    name = "gitk-rs-gui-smoke";
    runtimeInputs = with pkgs; [
      git
      imagemagick
      python3
      xdotool
      xwininfo
    ];
    text = ''
      export DISPLAY=:0
      export XAUTHORITY=/home/smoke/.Xauthority
      export XDG_RUNTIME_DIR=/run/user/1000
      export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [ pkgs.libx11 ]}
      export LIBGL_ALWAYS_SOFTWARE=true
      drivers=( ${pkgs.mesa}/share/vulkan/icd.d/lvp_icd*.json )
      export VK_DRIVER_FILES="''${drivers[0]}"
      test -f "$VK_DRIVER_FILES"
      exec python -B ${./scripts}/gui_smoke.py ${pkgs.lib.getExe gitk-rs} /tmp/gui-smoke
    '';
  };
in
pkgs.testers.runNixOSTest {
  name = "gitk-rs-gui-smoke";

  nodes.machine = {
    imports = [ (pkgs.path + "/nixos/tests/common/x11.nix") ];

    # The NixOS test session uses IceWM and LightDM, without a desktop environment.
    test-support.displayManager.auto.user = "smoke";
    services.xserver.desktopManager.xterm.enable = false;
    users.users.smoke = {
      isNormalUser = true;
      uid = 1000;
      password = "";
    };

    hardware.graphics.enable = true;
    fonts.packages = [ pkgs.dejavu_fonts ];
    environment.systemPackages = [ smoke ];
    virtualisation = {
      memorySize = 4096;
      cores = 2;
      resolution = {
        x = 1440;
        y = 1000;
      };
    };
  };

  testScript = ''
    import datetime
    import shlex

    machine.wait_for_x()
    machine.wait_until_succeeds("pgrep -u smoke -x icewm")
    machine.wait_for_file("/home/smoke/.Xauthority")
    machine.succeed("install -d -o smoke -g users /tmp/gui-smoke")
    try:
        machine.succeed(
            "su - smoke -c " + shlex.quote("${smoke}/bin/gitk-rs-gui-smoke")
            + " > /tmp/gui-smoke/driver.log 2>&1",
            timeout=datetime.timedelta(seconds=180),
        )
    finally:
        machine.log(machine.succeed("cat /tmp/gui-smoke/driver.log"))
        machine.copy_from_machine("/tmp/gui-smoke", "artifacts")
  '';
}
