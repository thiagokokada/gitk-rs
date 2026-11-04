{
  description = "gitk-rs: a Git history explorer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      eachSystem = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = eachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = self.packages.${system}.gitk-rs;
          gitk-rs = pkgs.callPackage ./gitk-rs.nix { };
        }
      );

      # TODO: restore devShell and checks
    };
}
