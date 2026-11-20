# Deployment

## Database roles and migrations

Use separate database roles so a compromised server cannot change the schema or
Diesel's migration history. The migration role owns portal tables; the server role
has DML access only. Never mount the migration credential in the server Pod.

Run `portal migrate` as a Kubernetes Job before rolling out `portal server`
(for example, with Helm `pre-install` and `pre-upgrade` hooks). The job needs
`APP_DATABASE__URL` and, optionally, `APP_ENV` (production by default),
`APP_LOG_FORMAT`, and `APP_DATABASE__TLS_CA_FILE`. Always use the same migration
role named in the default-privilege grants below. An advisory lock lets
overlapping jobs apply each pending migration once.

For a new database, provision both roles and run this SQL as the administrator
before the first migration (substitute your database, role, and schema names):

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

After the first migration, remove the app role's write access to Diesel's
migration history while retaining `SELECT` for readiness:

```sql
REVOKE INSERT, UPDATE, DELETE ON __diesel_schema_migrations FROM portal_app;
```

For an existing installation, use administrator credentials in the portal
database and follow this order:

1. Create `portal_app` and grant it access to existing objects, including
   `SELECT` on the migration history for readiness:

   ```sql
   GRANT CONNECT ON DATABASE portal TO portal_app;
   GRANT USAGE ON SCHEMA public TO portal_app;
   GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO portal_app;
   GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO portal_app;
   REVOKE INSERT, UPDATE, DELETE ON __diesel_schema_migrations FROM portal_app;
   ```

2. Roll out `portal server` with the new app credential. Confirm readiness and
   normal requests.

3. Inspect everything owned by `portal_old`: `REASSIGN OWNED` affects all its
   objects in this database, including the database itself if it owns it. Then
   transfer ownership to the migration role before the next migration:

   ```sql
   REASSIGN OWNED BY portal_old TO portal_migrator;
   ```

4. Apply the full new-database grant block above for `portal_migrator`, including
   `GRANT USAGE, CREATE ON SCHEMA public`. It must be able to create new tables
   and grant the app role access to them. Keep `CREATE` on the schema and database
   ownership away from the server role.

## Database TLS

- Production requires TLS with CA and hostname verification. Use
  `sslmode=require` for verified TLS; this connector does not accept libpq's
  `verify-full`. `prefer` is upgraded to `require` in production.
- Set `APP_DATABASE__TLS_CA_FILE` in both workloads to trust only the PEM CAs
  in that file. Without it, the system CAs are trusted.
- An explicit `sslmode=disable` in production is allowed only when every host
  and host address is loopback or a Unix socket, for a local proxy that encrypts
  the onward connection. Remote plaintext connections are refused.

## Backtraces

Set `RUST_BACKTRACE=1` to include stack traces in panic logs. Release builds keep
function names, but strip debug information, so file names and line numbers may
be unavailable. Rust's default panic hook writes the trace to stderr as plain text;
it is separate from JSON logs written to stdout when `APP_LOG_FORMAT=json`.

## Kubernetes probes

Set `APP_SERVER__BIND_ADDRESS=0.0.0.0:8080` in the server container. The listener
starts without waiting for migrations, because the migration Job ran before the
rollout. The startup probe below allows up to one minute for process startup;
database readiness is checked separately and remains unavailable while a
migration is pending or the migration history cannot be read:

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

Readiness begins as unavailable. A separate one-connection pool checks PostgreSQL and
the migration history every
five seconds, with a two-second deadline. One failed check is tolerated; two consecutive
failures make the instance unready. Results older than 15 seconds are unready. Size the
database connection limit for one extra connection per portal replica.

Restrict access to the container port to the ingress controller or reverse proxy,
for example with a Kubernetes NetworkPolicy. Otherwise, other in-cluster clients
can connect directly and bypass the proxy's timeouts.

## Reverse proxy

Put a reverse proxy in front of the portal and configure a header read timeout there.
`axum::serve` does not impose one, so a client can otherwise hold a connection open while
sending request headers slowly. For example, set `client_header_timeout 10s;` in an NGINX
server block, or use your proxy's equivalent header read timeout. Configure an idle
connection timeout too, such as NGINX's `keepalive_timeout`. The portal's 30-second
request timeout starts once the request headers have arrived and also covers reading
the body. It does not replace the proxy's header read timeout.
