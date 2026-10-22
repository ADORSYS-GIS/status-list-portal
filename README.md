# Status List Portal

[![ci](https://github.com/ADORSYS-GIS/status-list-portal/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ADORSYS-GIS/status-list-portal/actions/workflows/ci.yml?query=branch%3Amain)
[![dependencies](https://deps.rs/repo/github/ADORSYS-GIS/status-list-portal/status.svg)](https://deps.rs/repo/github/ADORSYS-GIS/status-list-portal)
[![License: MIT / Apache 2.0](https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-blue.svg)](#license)

Developer portal for the [Status List Server](https://github.com/adorsys/status-list-server).
Developers sign in, register a sandbox issuer for their public key, and get
everything they need to publish status lists on the hosted sandbox.

## Development

You need a stable Rust toolchain (`rust-toolchain.toml` selects it).

```sh
cp .env.template .env   # then adjust as needed
cargo run               # serves on http://127.0.0.1:8080
cargo test              # unit and integration tests
```

`.env.template` documents every setting.

## Code layout

| Path | Contents |
|---|---|
| `src/router.rs` | Every route, with its OpenAPI description |
| `src/controllers/` | Request handlers, one module per area of the API |
| `src/middleware/` | Middleware applied to every request (security headers, timeouts, tracing) |
| `src/util/errors.rs` | Error types; anything not meant for the user becomes a logged `500` |
| `src/config.rs` | Settings, read from `APP_*` environment variables at startup |
| `src/tests/` | Integration tests against the full router |
| `src/snapshots/` | Snapshot of the generated OpenAPI document |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
