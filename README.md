<p align="center">
  <img height="192" src="bins/wallet/packaging/icons/railoxide-icon.svg" alt="RailOxide">
</p>

<h1 align="center">
  <img width="420" src="bins/wallet/assets/icons/hero-wordmark.svg#gh-dark-mode-only" alt="RailOxide">
  <img width="420" src="bins/wallet/assets/icons/hero-wordmark-light.svg#gh-light-mode-only" alt="RailOxide">
</h1>

<p align="center">Desktop wallet for RAILGUN private transactions.</p>

<p align="center">
  <a href="https://github.com/triamazikamno/railoxide/releases">Releases</a> ·
  <a href="#install-from-source">Install</a> ·
  <a href="#build">Build</a> ·
  <a href="#privacy-model">Privacy Model</a>
</p>

---

## Status

RailOxide is under active development. APIs, wallet storage formats, and UI flows may change before a stable release.

## Features

- Fully open source
- Zero telemetry, zero home calls
- First-class integrated Tor support
- Indexed POI tree support, prevents UTXO spend intent leaking to the poi proxy operator
- Hardware-derived wallets:
  - public accounts have full hardware wallet support.
  - 0zk accounts are derived detirministically by signing a hash with a hardware device, private keys are **not** stored in app, but for signing they are briefly exposed in memory.
    Full on-device 0zk signing support is to be added as soon as hardware wallet vendors add railgun-specific cryptography functions.
- Aggressive batching to reduce rpc throttling
- Resilient public broadcaster network connection management
- Decenralized and leak-free pricing discovery via on-chain chainlink oracles. Used both for display and suspicious public broadcaster filtering

## Install From Source

RailOxide is alpha software and installs by building from source. The installer prompts for the source to build and defaults to the latest published GitHub release.

macOS/Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/triamazikamno/railoxide/main/scripts/install-wallet | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/triamazikamno/railoxide/main/scripts/install-wallet.ps1 | iex
```

To inspect the installer first:

macOS/Linux:

```bash
curl -fsSLO https://raw.githubusercontent.com/triamazikamno/railoxide/main/scripts/install-wallet
less install-wallet
bash install-wallet
```

Windows PowerShell:

```powershell
iwr https://raw.githubusercontent.com/triamazikamno/railoxide/main/scripts/install-wallet.ps1 -OutFile install-wallet.ps1
notepad .\install-wallet.ps1
powershell -ExecutionPolicy Bypass -File .\install-wallet.ps1
```

The installers support macOS, Ubuntu/Debian Linux, and Windows. See [`Install from source`](docs/install-from-source.md) for options and platform notes.

## Build

Native dependencies include Rust 1.91, `protoc`, and platform-specific C/C++ build dependencies.

```bash
cargo check -p wallet
cargo check -p wallet --features hardware
cargo build --release -p wallet --features hardware
```

For complete wallet build guides see

- [`Ubuntu`](docs/build-wallet-ubuntu.md)
- [`NixOS`](docs/nixos.md)
- [`macOS`](docs/build-wallet-macos.md)
- [`Windows`](docs/build-wallet-windows.md)

## Hardware Wallets

Ledger and Trezor support is available behind the `hardware` feature:

```bash
cargo run --bin wallet --features hardware
```

Current hardware-wallet support is hardware-derived software custody, not native RAILGUN hardware signing. The desktop app asks the device to derive profile material, then uses derived wallet material in desktop memory to prepare and sign RAILGUN spends. Treat this as hardware-assisted recovery/authorization for a software wallet, not as a guarantee that private transaction signing remains inside the hardware device.

## Privacy Model

RailOxide is privacy-oriented, but metadata privacy depends on mode and infrastructure choices.

By default, wallet HTTP/RPC traffic uses built-in Tor when no proxy or network mode is configured. Direct mode is explicit and sends outbound requests over the normal network. Proxy mode routes HTTP/RPC traffic through the configured proxy, but embedded Waku libp2p transports are disabled in proxy mode to avoid proxy bypass.

RPC providers, POI services, artifact gateways, public broadcasters, Waku peers, and token/fee data providers can observe metadata for the requests they receive. Self-broadcast and public-account actions may preflight or submit against multiple configured RPC providers for reliability, so each selected provider can observe the public transaction metadata it receives. Indexed POI artifacts avoid sending wallet blind commitments for normal POI reads, but the configured POI RPC URL is still used to live-tail recent public POI events. POI proxy mode is less private because it sends blind commitment hashes associated with UTXOs being received or prepared for spend.

The encrypted wallet vault protects wallet secrets and encrypted wallet cache records. App settings are stored outside the encrypted vault and may include proxy URLs, RPC endpoints, POI RPC URLs, Waku endpoints, and custom infrastructure settings. UI logs redact URL credentials, paths, query strings, and fragments where possible. Logs are intended for non-sensitive diagnostics, and users should still avoid putting credentials or API tokens in URLs where possible.

## Shared Crates

RailOxide depends on shared RAILGUN Rust crates from [`railgun-rust`](https://github.com/triamazikamno/railgun-rust).
