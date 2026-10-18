{
  description = "Mimick — unofficial Immich desktop client and auto-sync agent";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Splits the Rust build into a deps-only derivation plus the crate itself,
    # so `nix build` after a src edit recompiles Mimick but not its ~400 deps.
    crane.url = "github:ipetkov/crane";

    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-compat.url = "https://git.lix.systems/lix-project/flake-compat/archive/main.tar.gz";
    flake-compat.flake = false;
    treefmt-nix.url = "github:numtide/treefmt-nix";
  };

  outputs =
    {
      self,
      treefmt-nix,
      flake-parts,
      ...
    }@inputs:
    flake-parts.lib.mkFlake { inherit inputs self; } {
      imports = [
        flake-parts.flakeModules.easyOverlay
        treefmt-nix.flakeModule
      ];

      # Mimick is a GTK4 / libadwaita app that talks to the StatusNotifier and
      # XDG portal D-Bus APIs, so it is Linux only.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        { config, pkgs, ... }:
        {
          packages = {
            mimick = pkgs.callPackage ./package.nix {
              craneLib = inputs.crane.mkLib pkgs;
            };
            default = config.packages.mimick;
          };

          overlayAttrs = {
            inherit (config.packages) mimick;
          };

          # The developer shell (entered by .envrc → shell.nix → this): the
          # package's build inputs plus the Rust toolchain and the treefmt wrapper.
          devShells.default = pkgs.mkShell {
            inputsFrom = [ config.packages.mimick ];
            packages = [
              pkgs.cargo
              pkgs.clippy
              pkgs.git
              pkgs.rust-analyzer
              pkgs.rustc
              pkgs.rustfmt
              config.treefmt.build.wrapper
            ];
            env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };

          treefmt = {
            programs.nixfmt.enable = true;
            programs.deadnix.enable = true;
            programs.statix.enable = true;
            # Keep the `{ ... }:` set pattern in default.nix: `nix-build -A` only
            # auto-calls set-pattern lambdas, but statix's empty_pattern lint
            # would rewrite it to `_:`, which nix-build refuses to auto-call.
            programs.statix.disabled-lints = [ "empty_pattern" ];
            # Rust is formatted with rustfmt via `cargo fmt`, not treefmt.
          };
        };
    };
}
