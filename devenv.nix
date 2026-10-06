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
    pkgs.python3
    pkgs.just
 ];

  languages.rust = {
    enable = true;
    channel = "stable";
    version = "1.98.1";
    lsp.enable = true;
    # wasm32-unknown-emscripten: the web build target. Emscripten itself is
    # not a nix package here - see tools/setup_emscripten.sh (pinned emsdk
    # version, documented in CLAUDE.md's web build section).
    # aarch64-apple-ios / -sim: the iOS port (docs/ios-native-port-prd.md).
    # Only the Rust std for those targets comes from nix; the SDK, linker and
    # simulators come from /Applications/Xcode.app, which the devenv shell
    # hides behind DEVELOPER_DIR/SDKROOT - iOS builds set DEVELOPER_DIR back
    # to Xcode's, see the PRD.
    # aarch64-linux-android: the Android port (docs/android-port-prd.md);
    # the NDK, SDK and emulator come from tools/setup_android.sh.
    # x86_64-apple-darwin: the Intel half of the macOS app
    # (tools/macos/bundle.sh), cross-compiled on Apple Silicon.
    targets = [
      "wasm32-unknown-emscripten"
      "aarch64-apple-ios"
      "aarch64-apple-ios-sim"
      "aarch64-linux-android"
      "x86_64-apple-darwin"
    ];
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
