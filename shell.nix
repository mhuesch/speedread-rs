{ pkgs ? import <unstable> {} }:
pkgs.mkShell {
  buildInputs = with pkgs; [
    rustc
    cargo
    rustfmt
  ];
}
