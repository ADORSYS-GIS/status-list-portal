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
cargo run -- migrate    # apply pending migrations
cargo run -- server     # http://localhost:8080
```

Signing in needs a [GitHub OAuth app](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app) with the callback URL `http://localhost:8080/api/v1/session/github/callback`. Put its client ID and secret in `.env`, next to a session key.

`cargo run -- help` lists the operators' commands, such as approving registrations when `APP_SANDBOX__MANUAL_APPROVAL` is on.

### Frontend

The frontend lives in `frontend/` and needs [Bun](https://bun.com). Its commands run from the repository's root. With the backend running, `bun install && bun run dev` serves it at <http://localhost:5173>, passing API calls to the backend. Set `APP_SERVER__ORIGIN=http://localhost:5173` so that signing in returns there. For the backend to serve it instead, run `bun run build` and set `APP_SERVER__FRONTEND_DIR=frontend/build`.

## Database deployment

Run `portal migrate` as a Kubernetes Job before rolling out `portal server`
(for example, Helm `pre-install` and `pre-upgrade` hooks). The job needs only `APP_DATABASE__URL`,
`APP_ENV`, and optionally `APP_DATABASE__TLS_CA_FILE`. Give it a migration role
that owns the portal tables. Give the server a separate role with DML privileges;
do not mount the migration credential in the server Pod. Two migration jobs may
overlap: an advisory lock ensures that each pending migration runs once.

For a new database, provision distinct roles before the first migration. As the
database administrator, establish privileges before running migrations
(substitute your database, role, and schema names):

```sql
GRANT CONNECT ON DATABASE portal TO portal_migrator, portal_app;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE, CREATE ON SCHEMA public TO portal_migrator;
GRANT USAGE ON SCHEMA public TO portal_app;
ALTER DEFAULT PRIVILEGES FOR ROLE portal_migrator IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO portal_app;
ALTER DEFAULT PRIVILEGES FOR ROLE portal_migrator IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO portal_app;
```

Existing tables and sequences also need matching `GRANT ... ON ALL TABLES` and
`GRANT ... ON ALL SEQUENCES`. When migrating an existing installation, transfer
ownership of existing tables to the migration role before applying future
schema changes. Keep `CREATE` on the schema and database ownership away from
the server role. Production connections require TLS and verify the server's CA
chain and hostname. Mount an extra PEM CA bundle and set
`APP_DATABASE__TLS_CA_FILE` in both workloads when the system store lacks that CA.

## Kubernetes probes

Set `APP_SERVER__BIND_ADDRESS=0.0.0.0:8080` in the server container. The listener
starts without waiting for migrations, because the migration Job ran before the
rollout. The startup probe below allows up to one minute for process startup;
database readiness is checked separately:

```yaml
startupProbe:
  httpGet:
    path: /health/live
    port: 8080
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 12
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

## Test

```sh
cargo test
```

Tests use the Postgres from `docker compose`, with a fresh database per test. `bun run test` runs the frontend's tests, some of them in Chromium (`bun run playwright install chromium`). `bun test` would run Bun's own test runner instead.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
