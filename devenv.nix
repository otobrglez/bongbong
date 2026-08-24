{ 
  pkgs, lib, config, inputs, ... 
}: let

  unstable = import inputs.unstable-nixpkgs {
    inherit (pkgs.stdenv) system;
       config.allowUnfree = true;
     };
in {
  name = "bongbong";
  packages = [
    pkgs.git
    pkgs.cmake
    pkgs.SDL2
    # Used by tools/gen_*.py (asset generators) and tools/setup_emscripten.sh.
    pkgs.python3
    # Runs the web build/serve recipes in `justfile`.
    pkgs.just
    # CLI tool for managing CoWork Skills (https://crates.io/crates/cowork).
    # Built from crates.io rather than cargo-install'd, so it's reproducible
    # like the rest of `packages`. Uses `unstable`'s rustPlatform (not the
    # devenv-provided languages.rust toolchain) because cowork's Cargo.toml
    # requires edition2024, unsupported by nixpkgs-rolling's older cargo.
    (unstable.rustPlatform.buildRustPackage {
      pname = "cowork";
      version = "0.1.5";
      src = unstable.fetchCrate {
        pname = "cowork";
        version = "0.1.5";
        hash = "sha256-AhiOmhWxMkOcEPGboJAbrMm0EvbAP6rmXIIZNM7CsGU=";
      };
      cargoHash = "sha256-Wt5Gel+xa2eDNswirB1JNLZ1w6NrfGRT1Ji3QScBSYw=";
    })
  ];

  languages.rust = {
    enable = true;
    channel = "stable";
    version = "1.97.1";
    lsp.enable = true;
    # wasm32-unknown-emscripten: the web build target. Emscripten itself is
    # not a nix package here - see tools/setup_emscripten.sh (pinned emsdk
    # version, documented in CLAUDE.md's web build section).
    targets = [ "wasm32-unknown-emscripten" ];
  };

  languages.javascript = {
    enable = true;
    package = unstable.nodejs_24;
    nodejs.enable = true;
    yarn.enable = true;
    # yarn.install.enable = false;
    yarn.package = unstable.yarn-berry;
  };

  env = {
    RUST_BACKTRACE = "full";
    NIX_ENFORCE_PURITY = 0;
  };

  enterShell = ''
    export PATH="$DEVENV_ROOT/target/release:$PATH"
  '';
}
