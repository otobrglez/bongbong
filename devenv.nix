{ pkgs, lib, config, inputs, ... }:

{
  name = "bongbong";
  packages = [ 
    pkgs.git 
    pkgs.cmake
    pkgs.SDL2
  ];
  
  languages.rust = {
    enable = true;
    channel = "stable";
    version = "1.97.1";
    lsp.enable = true;
  };

  env = {
    RUST_BACKTRACE = "full";
    NIX_ENFORCE_PURITY = 0;
  };

  enterShell = ''
    export PATH="$DEVENV_ROOT/target/release:$PATH"
  '';
}
