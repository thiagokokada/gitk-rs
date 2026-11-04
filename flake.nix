{
  description = "gitk-rs: a GPUI Git history explorer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      eachSystem = nixpkgs.lib.genAttrs systems;
      source = nixpkgs.lib.cleanSourceWith {
        src = ./.;
        filter = path: type:
          nixpkgs.lib.cleanSourceFilter path type
          && !(type == "directory" && builtins.elem (baseNameOf path) [ "target" ".cache" ]);
      };
      environment = pkgs: rec {
        native = with pkgs; [ pkg-config cmake clang rustPlatform.bindgenHook ];
        libraries = with pkgs; [
          fontconfig freetype openssl zlib libxkbcommon wayland libxcb
          libx11 libxcursor libxi libxrandr vulkan-loader
        ];
        runtimePath = pkgs.lib.makeLibraryPath libraries;
      };
    in {
      devShells = eachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          deps = environment pkgs;
        in {
          default = pkgs.mkShell {
            nativeBuildInputs = deps.native ++ (with pkgs; [ cargo rustc rustfmt clippy git ]);
            buildInputs = deps.libraries;
            LD_LIBRARY_PATH = deps.runtimePath;
          };
        });

      packages = eachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          deps = environment pkgs;
        in {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "gitk-rs";
            version = "0.1.0";
            src = source;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = deps.native ++ [ pkgs.makeWrapper ];
            nativeCheckInputs = [ pkgs.git ];
            buildInputs = deps.libraries;
            postInstall = ''
              mkdir -p $out/share/doc/gitk-rs
              cp -r assets/licenses $out/share/doc/gitk-rs/
              cp LICENSE $out/share/doc/gitk-rs/
            '';
            postFixup = ''
              wrapProgram $out/bin/gitk-rs \
                --prefix LD_LIBRARY_PATH : ${deps.runtimePath} \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.git ]}
            '';
            meta = {
              description = "Git history explorer built with GPUI";
              license = pkgs.lib.licenses.mit;
              platforms = systems;
              mainProgram = "gitk-rs";
            };
          };
        });

      apps = eachSystem (system: {
        default = { type = "app"; program = "${self.packages.${system}.default}/bin/gitk-rs"; };
      });

      checks = eachSystem (system:
        let pkgs = nixpkgs.legacyPackages.${system}; in {
          package = self.packages.${system}.default;
          format = pkgs.runCommand "gitk-rs-format" { nativeBuildInputs = [ pkgs.rustfmt ]; } ''
            cd ${source}
            find src tests -name '*.rs' -print0 | xargs -0 rustfmt --edition 2024 --check
            touch $out
          '';
        });
    };
}
