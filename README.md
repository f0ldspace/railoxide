# RailOxide

Desktop wallet for RAILGUN private transactions.

## Status

RailOxide is under active development. APIs, wallet storage formats, and UI flows may change before a stable release.

## Build

Native dependencies include Rust 1.91, `protoc`, OpenSSL development libraries, and `pkg-config`.

```bash
cargo check -p wallet
cargo check -p wallet --features hardware
```

## Hardware Wallets

Ledger and Trezor support is available behind the `hardware` feature:

```bash
cargo run --bin wallet --features hardware
```

## Shared Crates

RailOxide depends on shared RAILGUN Rust crates from [`railgun-rust`](https://github.com/triamazikamno/railgun-rust).
