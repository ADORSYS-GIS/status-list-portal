#!/usr/bin/env bash
# Checks that an image of the portal works: its command line, its user, and the portal it
# serves once migrated against a fresh Postgres. CI runs it on the image it builds, and the
# release workflow on the published one, on each platform. Run it from the repository's root.
#
#   bash scripts/check-image.sh status-list-portal
#   bash scripts/check-image.sh ghcr.io/adorsys-gis/status-list-portal@sha256:...
set -euo pipefail

readonly POSTGRES_IMAGE=postgres:18.6
# The nonroot user of the distroless base image.
readonly EXPECTED_USER=65532
readonly READY_ATTEMPTS=30

image=${1:?usage: check-image.sh <image>}

# Fixed names would clash with another run on the same Docker daemon.
suffix="$$-${RANDOM}"
database="portal-check-db-${suffix}"
portal="portal-check-${suffix}"
work=$(mktemp -d)

cleanup() {
    local status=$?
    if [ "$status" -ne 0 ] && docker container inspect "$portal" > /dev/null 2>&1; then
        echo "The portal's log:"
        docker logs "$portal" || true
    fi
    docker rm --force "$portal" "$database" > /dev/null 2>&1 || true
    rm -rf "$work"
    exit "$status"
}
trap cleanup EXIT

# Outputs are read whole rather than piped into grep -q, which can exit before the writer
# finishes and fail the pipe.
echo "Checking that ${image} prints its usage"
usage=$(docker run --rm "$image" --help)
if [[ "$usage" != *"Usage: portal"* ]]; then
    echo "The image's --help printed no usage of portal:"
    echo "$usage"
    exit 1
fi

echo "Checking that it runs as user ${EXPECTED_USER}"
user=$(docker image inspect --format '{{.Config.User}}' "$image")
if [ "$user" != "$EXPECTED_USER" ]; then
    echo "The image runs as '${user}', not ${EXPECTED_USER}."
    exit 1
fi

# The portal joins the database's network namespace, so the database is on localhost: in
# production, the portal refuses plaintext to any other host. Port 8080 is the portal's,
# published by the namespace's first container.
docker run --detach --name "$database" --env POSTGRES_PASSWORD=postgres \
    --publish 127.0.0.1::8080 "$POSTGRES_IMAGE" > /dev/null
# Over TCP: while it initializes, Postgres listens on its socket only.
for _ in $(seq "$READY_ATTEMPTS"); do
    docker exec "$database" pg_isready --host 127.0.0.1 --quiet && break
    sleep 1
done
docker exec "$database" pg_isready --host 127.0.0.1

# The server runs as the image's user, so it must be able to read these.
secrets="${work}/secrets"
mkdir -m 755 "$secrets"
echo 'postgres://postgres:postgres@localhost:5432/postgres?sslmode=disable' > "${secrets}/database-url"
openssl rand -hex 64 > "${secrets}/session-key"
cp test_data/admin-keys/es256.pem "${secrets}/admin-key.pem"
chmod 644 "$secrets"/*
cat > "${work}/portal.env" << EOF
APP_SERVER__ORIGIN=http://localhost:8080
APP_DATABASE__URL_FILE=/run/secrets/portal/database-url
APP_SESSION__KEY_FILE=/run/secrets/portal/session-key
APP_GITHUB__CLIENT_ID=ci
APP_GITHUB__CLIENT_SECRET=ci
APP_ADMIN_API__URL=http://localhost:8002
APP_ADMIN_API__PRINCIPAL=portal
APP_ADMIN_API__AUDIENCE=status-list-server
APP_ADMIN_API__KEY_ID=ci
APP_ADMIN_API__SIGNING_KEY_FILE=/run/secrets/portal/admin-key.pem
APP_MANAGEMENT_API__URL=https://status.example.com
APP_MANAGEMENT_API__AUDIENCE=https://status.example.com
APP_TERMS__VERSION=ci
APP_TERMS__URL=https://example.com/terms
EOF

echo "Checking that it migrates the database"
# The migration job gets only its database URL, as docs/deployment.md has it.
docker run --rm --network "container:${database}" --volume "${secrets}:/run/secrets/portal:ro" \
    --env APP_DATABASE__URL_FILE=/run/secrets/portal/database-url "$image" migrate

echo "Checking that it serves the portal"
docker run --detach --name "$portal" --network "container:${database}" \
    --env-file "${work}/portal.env" --volume "${secrets}:/run/secrets/portal:ro" "$image" > /dev/null
# Docker may list an IPv6 address after the IPv4 one.
addresses=$(docker port "$database" 8080/tcp)
address=${addresses%%$'\n'*}
for _ in $(seq "$READY_ATTEMPTS"); do
    curl --fail --silent --output /dev/null "http://${address}/health/ready" && break
    sleep 1
done
curl --fail --silent --show-error --output /dev/null "http://${address}/health/ready"
# The image's own frontend, with the policy the backend reads from it.
page=$(curl --fail --silent --show-error --header 'Accept: text/html' "http://${address}/")
if [[ "$page" != *'<meta http-equiv="content-security-policy"'* ]]; then
    echo "The portal's page has no content security policy."
    exit 1
fi

echo "${image} works"
