{
  description = "system-monitor: Raspberry Pi Pico display for host system stats";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        # Stable Rust plus the cross targets for both Pico boards:
        #   thumbv6m-none-eabi        -> RP2040 (Pico / Pico W, Cortex-M0+)
        #   thumbv8m.main-none-eabihf -> RP2350 (Pico 2 / Pico 2 W, Cortex-M33)
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "llvm-tools-preview" ];
          targets = [ "thumbv6m-none-eabi" "thumbv8m.main-none-eabihf" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustToolchain
            pkg-config

            # Firmware: flashing, debugging, linking
            probe-rs-tools # `probe-rs run` / `cargo embed` via a debug probe
            elf2uf2-rs     # flash over USB by holding BOOTSEL (no probe needed)
            picotool       # inspect / load / reboot Pico boards
            flip-link      # stack-overflow protection linker wrapper
            cargo-binutils # `cargo size`, `cargo objdump`, ...

            # Host <-> Pico serial debugging
            tio
            usbutils       # lsusb
          ];

          buildInputs = with pkgs; [
            udev # needed by probe-rs / serialport crates on the host side
          ];

          shellHook = ''
            echo "system-monitor dev shell: $(rustc --version)"
          '';
        };
        packages.host = pkgs.rustPlatform.buildRustPackage {
          pname = "system-monitor-host";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          cargoBuildFlags = ["-p" "host"];
          doCheck = false;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.udev ];
        };
        packages.default = self.packages.${system}.host;
      }
  );
}
