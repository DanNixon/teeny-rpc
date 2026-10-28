{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = {nixpkgs, ...}: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {inherit system;};
  in {
    devShells.${system}.default = pkgs.mkShell {
      buildInputs = with pkgs; [
        # General build tools
        just

        # Code formatting tools
        alejandra
        typos

        # Rust toolchain
        rustup

        # Release tools
        release-plz

        # Embedded example
        probe-rs-tools

        # Linux example
        pkg-config
        systemd
      ];
    };
  };
}
