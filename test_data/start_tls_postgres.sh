#!/usr/bin/env bash
set -euo pipefail

cert_dir="$(mktemp -d)"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -sha256 \
  -subj '/CN=Portal test CA' -keyout "$cert_dir/ca.key" -out "$cert_dir/ca.crt" >/dev/null 2>&1
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -sha256 \
  -subj '/CN=Wrong test CA' -keyout "$cert_dir/wrong-ca.key" -out "$cert_dir/wrong-ca.crt" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -sha256 -subj '/CN=localhost' \
  -keyout "$cert_dir/server.key" -out "$cert_dir/server.csr" >/dev/null 2>&1
printf 'subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n' > "$cert_dir/server.ext"
openssl x509 -req -in "$cert_dir/server.csr" -CA "$cert_dir/ca.crt" \
  -CAkey "$cert_dir/ca.key" -CAcreateserial -out "$cert_dir/server.crt" \
  -days 1 -sha256 -extfile "$cert_dir/server.ext" >/dev/null 2>&1

container="portal-pg-tls-$$"
docker run -d --name "$container" -p 127.0.0.1::5432 \
  -e POSTGRES_PASSWORD=postgres -v "$cert_dir:/certs:ro" postgres:18.6 \
  bash -c 'cp /certs/server.crt /tmp/server.crt; cp /certs/server.key /tmp/server.key; chown postgres:postgres /tmp/server.*; chmod 600 /tmp/server.key; exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tmp/server.crt -c ssl_key_file=/tmp/server.key' >/dev/null

for _ in {1..30}; do
  if docker exec "$container" pg_isready -h localhost -p 5432 -U postgres >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
docker exec "$container" pg_isready -h localhost -p 5432 -U postgres >/dev/null
port="$(docker port "$container" 5432/tcp)"
port="${port##*:}"
printf 'TEST_TLS_DATABASE_URL=postgres://postgres:postgres@localhost:%s/postgres\n' "$port"
printf 'TEST_TLS_CA_FILE=%s/ca.crt\n' "$cert_dir"
printf 'TEST_TLS_WRONG_CA_FILE=%s/wrong-ca.crt\n' "$cert_dir"
printf 'TEST_TLS_CONTAINER=%s\n' "$container"
