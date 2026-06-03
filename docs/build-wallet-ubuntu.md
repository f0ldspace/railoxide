This guide builds the RailOxide desktop wallet binary from source on Ubuntu with Ledger and Trezor support enabled.

The commands below were verified on Ubuntu 24.04.

## Install System Dependencies

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  clang \
  cmake \
  curl \
  elfutils \
  gettext-base \
  git \
  jq \
  lld \
  llvm \
  pkg-config \
  protobuf-compiler \
  libasound2-dev \
  libfontconfig-dev \
  libglib2.0-dev \
  libsqlite3-dev \
  libssl-dev \
  libstdc++-14-dev \
  libudev-dev \
  libusb-1.0-0-dev \
  libva-dev \
  libvulkan1 \
  libwayland-dev \
  libx11-xcb-dev \
  libxkbcommon-x11-dev \
  libzstd-dev \
  pipewire \
  xdg-desktop-portal
```

These packages cover the Rust native build chain, `protoc`, OpenSSL, GPUI's Linux desktop dependencies, and the USB libraries used by hardware-wallet support.

## Install Rust 1.91

If Rust 1.91+ is not installed, install it with `rustup`:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
  sh -s -- -y --default-toolchain 1.91.0
. "$HOME/.cargo/env"
rustc --version
cargo --version
```

Both version commands should report `1.91.0` or higher.

## Clone The Repository

```bash
git clone https://github.com/triamazikamno/railoxide.git
cd railoxide
```

## Build With Hardware Wallet Support

Run a check build first:

```bash
cargo check -p wallet --features hardware
```

Build the optimized release binary:

```bash
cargo build --release -p wallet --features hardware
```

The wallet binary is written to:

```bash
target/release/wallet
```

Verify the binary starts far enough to parse command-line options:

```bash
./target/release/wallet --help
```

Run the wallet:

```bash
./target/release/wallet
```

To store wallet data in a custom location:

```bash
./target/release/wallet --db-path "$HOME/.local/share/RailOxide"
```

## Hardware Wallet USB Access

The `hardware` feature enables Ledger and Trezor integration through `coins-ledger`, `hidapi-rusb`, and `trezor-client`.

If the app builds but cannot open a connected device, install the official Ledger and Trezor Linux udev rules for your distribution, then reload udev and reconnect the device:

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
```

You may need to log out and back in if the rules add your user to a new group such as `plugdev`.

Current hardware-wallet support is hardware-derived software custody. The device is used to derive profile material, while RAILGUN transaction preparation and signing still happen in desktop memory.

## Troubleshooting

If `cargo` is not found after installing Rust, load rustup's environment:

```bash
. "$HOME/.cargo/env"
```

If the build cannot find `protoc`, install `protobuf-compiler`.

If `openssl-sys` fails, install `pkg-config` and `libssl-dev`.

If `hidapi-rusb`, `rusb`, Ledger, or Trezor crates fail to build, install `libusb-1.0-0-dev` and `libudev-dev`.

If GPUI/X11/Wayland dependencies fail to build, confirm the GUI packages from the dependency install command are present.

If running over SSH or in a headless environment, the GUI may fail to open even though the binary built correctly. Run it from a desktop session with X11 or Wayland available.
