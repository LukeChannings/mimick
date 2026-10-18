# Developer shell, entered by `.envrc` (`use nix`). Delegates to the flake's
# `devShells.default` via flake-compat so there is a single source of truth.
(import ./. { }).devShells.${builtins.currentSystem}.default
