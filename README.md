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
cargo run -- server     # http://localhost:8080
```

Signing in needs a [GitHub OAuth app](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app) with the callback URL `http://localhost:8080/api/v1/session/github/callback`. Put its client ID and secret in `.env`, next to a session key.

`cargo run -- help` lists the operators' commands, such as approving registrations when `APP_SANDBOX__MANUAL_APPROVAL` is on.

### Frontend

The frontend lives in `frontend/` and needs [Bun](https://bun.com). Its commands run from the repository's root. With the backend running, `bun install && bun run dev` serves it at <http://localhost:5173>, passing API calls to the backend. Set `APP_SERVER__ORIGIN=http://localhost:5173` so that signing in returns there. For the backend to serve it instead, run `bun run build` and set `APP_SERVER__FRONTEND_DIR=frontend/build`.

## Kubernetes probes

Set `APP_SERVER__BIND_ADDRESS=0.0.0.0:8080` in the container. The HTTP listener
starts after database migrations finish. Give startup enough time for the longest
migration and for waiting on another replica's migration lock. For example, these
container probes allow up to five minutes of startup before liveness takes effect:

```yaml
startupProbe:
  httpGet:
    path: /health/live
    port: 8080
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 60
livenessProbe:
  httpGet:
    path: /health/live
    port: 8080
  periodSeconds: 10
  timeoutSeconds: 3
readinessProbe:
  httpGet:
    path: /health/ready
    port: 8080
  periodSeconds: 5
  timeoutSeconds: 3
```

Readiness begins as unavailable. A separate one-connection pool checks PostgreSQL every
five seconds, with a two-second deadline. One failed check is tolerated; two consecutive
failures make the instance unready. Results older than 15 seconds are unready. Size the
database connection limit for one extra connection per portal replica.

### Reverse proxy timeouts

Put a reverse proxy in front of the portal and configure a header read timeout there.
`axum::serve` does not impose one, so a client can otherwise hold a connection open while
sending request headers slowly. For example, set `client_header_timeout 10s;` in an NGINX
server block. The portal's 30-second request timeout starts once the request headers
have arrived and also covers reading the body. It does not replace the proxy's header
read timeout.

## Test

```sh
cargo test
```

Tests use the Postgres from `docker compose`, with a fresh database per test. `bun run test` runs the frontend's tests, some of them in Chromium (`bun run playwright install chromium`). `bun test` would run Bun's own test runner instead.

### Updating snapshots

When an intentional API change makes its snapshot test fail, run
`INSTA_UPDATE=always cargo test --lib openapi_snapshot`, review the snapshot
diff, and run `cargo test --all-targets --locked` before committing it. Other snapshot
assertions in the integration tests are inline; update those intentionally in their
test source when behavior changes.

### Adding database migrations

Install Diesel CLI with PostgreSQL support at the version pinned in CI. Then create
the migration:

```sh
diesel migration generate <name>
```

Implement the forward and rollback SQL. Point `DATABASE_URL` at a local development
database, then apply the migrations:

```sh
diesel migration run
```

Run `diesel print-schema` and replace the checked-in Diesel schema with its exact output,
including the generated header and declaration order. Then run the tests. CI compares
that schema byte for byte against a fresh migrated database.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
