{ pkgs, lib, config, inputs, ... }:

{
  name = "bongbong";
  packages = [
    pkgs.git
    pkgs.cmake
    pkgs.SDL2
    # Used by tools/gen_*.py (asset generators) and tools/setup_emscripten.sh.
    pkgs.python3
    # Runs the web build/serve recipes in `justfile`.
    pkgs.just
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

  env = {
    RUST_BACKTRACE = "full";
    NIX_ENFORCE_PURITY = 0;
  };

  enterShell = ''
    export PATH="$DEVENV_ROOT/target/release:$PATH"
  '';
}
