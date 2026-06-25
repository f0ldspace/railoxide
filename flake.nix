{
  description = "RailOxide — Desktop wallet for RAILGUN private transactions";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        isLinux = pkgs.stdenv.isLinux;
        isDarwin = pkgs.stdenv.isDarwin;

        walletCargoToml = builtins.fromTOML (builtins.readFile ./bins/wallet/Cargo.toml);

        linuxBuildInputs = with pkgs; [
          openssl
          sqlite
          fontconfig
          libxkbcommon
          libx11
          libxcursor
          libxi
          libxrandr
          libxcb
          wayland
          vulkan-loader
          alsa-lib
          libGL
          zstd
          libusb1
          eudev
          hidapi
        ];

        darwinBuildInputs = with pkgs; [
          darwin.apple_sdk.frameworks.Security
          darwin.apple_sdk.frameworks.SystemConfiguration
          darwin.apple_sdk.frameworks.ApplicationServices
          darwin.apple_sdk.frameworks.CoreFoundation
          darwin.apple_sdk.frameworks.Cocoa
          darwin.apple_sdk.frameworks.Metal
          darwin.apple_sdk.frameworks.QuartzCore
          darwin.apple_sdk.frameworks.IOKit
        ];

        nativeBuildInputs = with pkgs; [
          pkg-config
          cmake
          clang
          libclang.lib
          rustPlatform.bindgenHook
          makeWrapper
        ] ++ (if isLinux then [
          wayland-protocols
          libxkbcommon
        ] else []);

      in
      {
        packages.default = rustPlatform.buildRustPackage {
          pname = "railoxide";
          version = walletCargoToml.package.version;

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };

          cargoBuildFlags = [ "-p" "wallet" ];

          buildFeatures = [ "hardware" ];

          inherit nativeBuildInputs;

          buildInputs =
            (if isLinux then linuxBuildInputs else []) ++
            (if isDarwin then darwinBuildInputs else []);

          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

          postInstall = ''
            mv $out/bin/wallet $out/bin/railoxide
          '';

          postFixup = pkgs.lib.optionalString isLinux ''
            wrapProgram $out/bin/railoxide \
              --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath (linuxBuildInputs ++ [ pkgs.vulkan-loader ])}"
          '';

          doCheck = false;

          meta = with pkgs.lib; {
            description = "Desktop wallet for RAILGUN private transactions";
            homepage = "https://github.com/triamazikamno/railoxide";
            license = licenses.mit;
            mainProgram = "railoxide";
            platforms = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
          };
        };

        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.pkg-config
            pkgs.cmake
            pkgs.clang
            pkgs.libclang.lib
          ] ++ (if isLinux then (with pkgs; [
            openssl
            sqlite
            fontconfig
            libxkbcommon
            libx11
            libxcursor
            libxi
            libxrandr
            libxcb
            wayland
            wayland-protocols
            vulkan-loader
            alsa-lib
            libGL
            zstd
            libusb1
            eudev
            hidapi
          ]) else if isDarwin then darwinBuildInputs else []);

          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

          shellHook = ''
            echo "RailOxide dev shell"
            echo "Rust: $(rustc --version)"
            echo "Cargo: $(cargo --version)"
          '';
        };
      }
    );
}
