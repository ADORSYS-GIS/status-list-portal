#!/usr/bin/env bash
# Runs a Status List Server for the live admin API tests (src/tests/live_admin_api.rs):
# the hosted profile, with the portal in its registry as test_data/status-list-server says.
#
# Usage: scripts/live-status-list-server.sh <server checkout> <Postgres URL>
#
# The checkout must be built first with `cargo build --features postgres`; keep the default
# `memory` feature, without which the server has no binary. The admin API then listens on
# http://127.0.0.1:8002: pass that to the tests as LIVE_ADMIN_API_URL.
set -euo pipefail

server_dir=$(cd "${1:?the server checkout}" && pwd)
database_url=${2:?the Postgres URL of the server}
portal_dir=$(cd "$(dirname "$0")/.." && pwd)

export APP_ENV=development
export APP_SERVER__PROFILE=hosted
export APP_SERVER__HOST=127.0.0.1
export APP_SERVER__MANAGEMENT__HOST=127.0.0.1
export APP_SERVER__ADMIN__HOST=127.0.0.1
export APP_SERVER__ADMIN__PORT=8002
export APP_DATABASE__URL=$database_url
export APP_MANAGEMENT_AUTH__ISSUERS_FILE=$portal_dir/test_data/status-list-server/issuers.yaml
# The tests' ADMIN_AUDIENCE; the management audience must differ from it.
export APP_ADMIN_AUTH__AUDIENCES=test-admin-audience
export APP_MANAGEMENT_AUTH__AUDIENCES=test-management-audience
# The tests make more admin calls than the default budget of a burst of 100.
export APP_RATE_LIMIT__ADMIN_BURST_SIZE=100000
export APP_SERVER__CERT__PROVISIONING_STRATEGY=store
export APP_SERVER__CERT__STORE__CERTIFICATE_PATH=$server_dir/test_data/ed25519_cert.pem
export APP_SERVER__CERT__STORE__SIGNING_KEY_PATH=$server_dir/test_data/ed25519_key.pem

exec "$server_dir/target/debug/status-list-server"
