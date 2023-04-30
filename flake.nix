{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
  };
  outputs = inputs @ {
    self,
    nixpkgs,
    parts,
    ...
  }: let
    inherit (builtins) attrValues;
  in parts.lib.mkFlake { inherit inputs; } {
    systems = ["x86_64-linux" "x86-linux" "aarch64-linux"];

    perSystem = { pkgs, system, ... }: {
      packages.system76-scheduler = pkgs.rustPlatform.buildRustPackage {
        name = "system76-scheduler";
        src = ./.;
        cargoLock = {
          lockFile = ./Cargo.lock;
          # Allow dependencies to be fetched from git and avoid having to set the outputHashes manually
          allowBuiltinFetchGit = true;
        };

        nativeBuildInputs = attrValues {
          inherit (pkgs)
            pkg-config
            llvm
            clang
          ;
        };
        buildInputs = attrValues {
          inherit (pkgs)
            dbus
            pipewire
          ;
        };

        LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
        EXECSNOOP_PATH = "${pkgs.bcc}/bin/execsnoop";

        # tests don't build
        doCheck = false;

        postInstall = ''
          mkdir -p $out/data
          install -D -m 0644 data/com.system76.Scheduler.conf $out/etc/dbus-1/system.d/com.system76.Scheduler.conf
          install -D -m 0644 data/*.kdl $out/data/
        '';
      };

      packages.default = self.outputs.packages.${system}.system76-scheduler;

      devShells.default = self.packages.${system}.default.overrideAttrs (super: {
        nativeBuildInputs = super.nativeBuildInputs
                            ++ (attrValues {
                              inherit (pkgs)
                                cargo-edit
                                clippy
                                rustfmt
                              ;
                            });
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      });
    };
  };
}
