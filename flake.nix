{
  description = "Retina RTSP client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
            pkg-config
            openssl
            cmake
            
            # For testing
            gst_all_1.gstreamer
            gst_all_1.gst-plugins-base
            gst_all_1.gst-plugins-good
            gst_all_1.gst-plugins-bad
            gst_all_1.gst-plugins-ugly
            gst_all_1.gst-libav
            
            # Development tools
            cargo-watch
            cargo-nextest
            rust-analyzer
          ];

          shellHook = ''
            echo "Retina development environment"
            echo "Run 'cargo test' to run tests"
            echo "Run 'cargo test test_fu_a_boundary_bug -- --nocapture' to run the specific bug test"
          '';

          RUST_BACKTRACE = 1;
          RUST_LOG = "debug";
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "retina";
          version = "0.4.13";
          
          src = ./.;
          
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
          
          buildInputs = with pkgs; [
            openssl
          ];
        };
      });
}