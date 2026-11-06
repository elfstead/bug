{ pkgs ? import <nixpkgs> {}, withTests ? false }:
let
  # Reuse the distribution's engine dependencies, but not its Qt frontend.
  engineInputs = builtins.filter (p: !(pkgs.lib.hasPrefix "qt" (pkgs.lib.getName p))) pkgs.ladybird.buildInputs;
in pkgs.mkShell {
  packages = (with pkgs; [ cargo rustc rustfmt clippy cmake ninja pkg-config python3 git curl patch glslang nasm ])
    ++ pkgs.lib.optionals withTests [ pkgs.xvfb-run pkgs.xdotool pkgs.weston pkgs.xclip ];
  buildInputs = engineInputs ++ (with pkgs; [ dbus wayland libxkbcommon libX11 libXcursor libXi libXrandr libxcb ]);
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [ wayland libxkbcommon libX11 libXcursor libXi libXrandr libxcb vulkan-loader ]);
  shellHook = ''
    export BUG_ICU_ROOT=${pkgs.icu78.dev}
  '';
}
