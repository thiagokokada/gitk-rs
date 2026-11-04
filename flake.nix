{
  description = "gitk-rs: a Git history explorer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
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

      devShells = eachSystem (system: {
        default = nixpkgs.legacyPackages.${system}.callPackage ./shell.nix {
          gitk-rs = self.packages.${system}.gitk-rs;
        };
      });

      checks = eachSystem (
        system:
        import ./checks.nix {
          pkgs = nixpkgs.legacyPackages.${system};
          gitk-rs = self.packages.${system}.gitk-rs;
        }
      );
    };
}
