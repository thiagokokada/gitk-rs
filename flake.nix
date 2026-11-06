{
  description = "gitk-rs: a Git history explorer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.crane.url = "github:ipetkov/crane";

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      eachSystem = nixpkgs.lib.genAttrs systems;
      revision = builtins.substring 0 7 (self.rev or (self.dirtyRev or "unknown"));
    in
    {
      packages = eachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = self.packages.${system}.gitk-rs;
          gitk-rs = pkgs.callPackage ./gitk-rs.nix {
            inherit revision;
            craneLib = crane.mkLib pkgs;
          };
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          gui-smoke = import ./gui-smoke.nix {
            inherit pkgs;
            gitk-rs = self.packages.${system}.gitk-rs;
          };
        }
      );

      devShells = eachSystem (system: {
        default = nixpkgs.legacyPackages.${system}.callPackage ./shell.nix {
          craneLib = crane.mkLib nixpkgs.legacyPackages.${system};
          checks = self.checks.${system};
          gitk-rs = self.packages.${system}.gitk-rs;
        };
      });

      checks = eachSystem (
        system:
        import ./checks.nix {
          pkgs = nixpkgs.legacyPackages.${system};
          craneLib = crane.mkLib nixpkgs.legacyPackages.${system};
          gitk-rs = self.packages.${system}.gitk-rs;
        }
      );
    };
}
