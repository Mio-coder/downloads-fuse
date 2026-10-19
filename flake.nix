{
  description = "Live categorized FUSE views of a Downloads directory";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        rec {
          downloads-fuse = pkgs.rustPlatform.buildRustPackage {
            pname = "downloads-fuse";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.makeWrapper ];
            postInstall = ''
              wrapProgram $out/bin/downloads-fuse \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.fuse3 ]}
            '';

            meta = {
              description = "Live categorized FUSE views of a Downloads directory";
              license = pkgs.lib.licenses.mit;
              mainProgram = "downloads-fuse";
              platforms = pkgs.lib.platforms.linux;
            };
          };
          default = downloads-fuse;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/downloads-fuse";
        };
      });

      checks = forAllSystems (system: {
        inherit (self.packages.${system}) downloads-fuse;
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              fuse3
              rustc
              rustfmt
            ];
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
