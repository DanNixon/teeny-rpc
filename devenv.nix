{pkgs, ...}: {
  packages = with pkgs; [
    # Code formatting tools
    treefmt
    alejandra
    mdl
    rustfmt

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
}
