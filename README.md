# Status List Portal

[![ci](https://github.com/ADORSYS-GIS/status-list-portal/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ADORSYS-GIS/status-list-portal/actions/workflows/ci.yml?query=branch%3Amain)
[![dependencies](https://deps.rs/repo/github/ADORSYS-GIS/status-list-portal/status.svg)](https://deps.rs/repo/github/ADORSYS-GIS/status-list-portal)
[![License: MIT / Apache 2.0](https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-blue.svg)](#license)

Developer portal for the [Status List Server](https://github.com/adorsys/status-list-server): developers sign in with GitHub and register sandbox issuers.

## Run locally

Requires [Rust](https://rustup.rs) and [Docker](https://docs.docker.com/get-started/get-docker/).

```sh
cp .env.template .env   # every setting, documented
docker compose up -d    # Postgres
cargo run               # http://127.0.0.1:8080
```

## Test

```sh
cargo test
```

Tests use the Postgres from `docker compose`, with a fresh database per test.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
