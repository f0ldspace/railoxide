# Install RailOxide From Source

RailOxide is alpha software. The source installer builds the latest `main` branch from GitHub instead of installing prebuilt signed binaries.

## Quick Install

```bash
curl -fsSL https://raw.githubusercontent.com/triamazikamno/railoxide/main/scripts/install-wallet | bash
```

## Inspect First

```bash
curl -fsSLO https://raw.githubusercontent.com/triamazikamno/railoxide/main/scripts/install-wallet
less install-wallet
bash install-wallet
```

## What It Does

- Clones or updates `https://github.com/triamazikamno/railoxide.git` from `main`.
- Stores the managed checkout at `~/.local/src/railoxide` by default.
- Installs Rust `1.91.0` through `rustup` if needed.
- Builds the wallet with hardware-wallet support enabled by default.
- Prints the exact source commit before building.
- Refuses to run as root.
- Uses `sudo` only for Ubuntu/Debian system package installation.

## macOS

The installer checks for Apple Command Line Tools, `protoc`, Rust, and Git.

If full Xcode and the Metal tools are available, the installer builds with build-time Metal shader compilation. If full Xcode is installed but the Metal tools are missing, it asks before running:

```bash
xcodebuild -downloadComponent MetalToolchain
```

If Metal is unavailable or the download is skipped, it falls back to `gpui/runtime_shaders`, which works with Command Line Tools-only installs.

The installed app is written to:

```text
~/Applications/RailOxide.app
```

The installer also writes a command wrapper to:

```text
~/.local/bin/railoxide-wallet
```

Useful macOS options:

```bash
bash install-wallet --metal
bash install-wallet --runtime-shaders
```

## Ubuntu/Debian Linux

The installer uses `apt-get` to install the build dependencies listed in [`build-wallet-ubuntu.md`](build-wallet-ubuntu.md), then builds the release binary.

The installed command is written to:

```text
~/.local/bin/railoxide-wallet
```

It also installs a desktop entry and icon under `~/.local/share`.

## Options

```text
--prefix PATH          Install under PATH instead of ~/.local
--source-dir PATH      Use PATH for the managed source checkout
--no-deps              Do not install missing system dependencies
--no-hardware          Build without hardware-wallet support
--metal                On macOS, require build-time Metal shader compilation
--runtime-shaders      On macOS, force runtime shaders instead of Metal
-y, --yes              Do not prompt before dependency installs/downloads
--dry-run              Print what would happen without changing anything
--verbose              Print commands as they run
-h, --help             Show help
```

## Updating

Rerun the installer. It updates the managed checkout to the latest `main` commit and rebuilds.

## Troubleshooting

If `railoxide-wallet` is not found after installation, add `~/.local/bin` to your shell `PATH`.

If the installer reports that `~/.local/src/railoxide` is not installer-managed, either move that existing checkout or pass a different `--source-dir`.

For manual platform-specific build steps, see:

- [`Ubuntu`](build-wallet-ubuntu.md)
- [`macOS`](build-wallet-macos.md)
- [`Windows`](build-wallet-windows.md)
