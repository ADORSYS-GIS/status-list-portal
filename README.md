# Status List Portal

[![ci](https://github.com/ADORSYS-GIS/status-list-portal/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ADORSYS-GIS/status-list-portal/actions/workflows/ci.yml?query=branch%3Amain)
[![dependencies](https://deps.rs/repo/github/ADORSYS-GIS/status-list-portal/status.svg)](https://deps.rs/repo/github/ADORSYS-GIS/status-list-portal)
[![License: MIT / Apache 2.0](https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-blue.svg)](#license)

Developer portal for the [Status List Server](https://github.com/adorsys/status-list-server): developers sign in with GitHub and register sandbox issuers.

## Run locally

Requires [Rust](https://rustup.rs) 1.99 or later (`rustup update`) and [Docker](https://docs.docker.com/get-started/get-docker/). Runs on Linux, macOS and Windows.

```sh
cp .env.template .env   # every setting, documented
docker compose up -d    # Postgres
cargo run               # http://localhost:8080
```

Signing in needs a [GitHub OAuth app](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app) with the callback URL `http://localhost:8080/api/v1/session/github/callback`. Put its client ID and secret in `.env`, next to a session key. The portal also signs its calls to the server's [admin API](https://github.com/adorsys/status-list-server/blob/develop/docs/openapi-admin.yaml) with a key of its own; `.env.template` shows how to make one.

## Test

```sh
cargo test
```

Tests use the Postgres from `docker compose`, with a fresh database per test.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
