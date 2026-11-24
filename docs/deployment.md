# Deploying the portal

The portal is one container: the server, which also serves the frontend. It needs:

- a Postgres database of its own;
- the Status List Server's admin API, on a private network;
- a GitHub OAuth app;
- a public HTTPS address, usually behind a reverse proxy or a Kubernetes ingress.

## The image

```sh
docker build --tag status-list-portal .
```

The image runs `portal server` on port 8080 as a user without privileges. It sets `APP_ENV=production`, `APP_LOG_FORMAT=json` and the frontend's location. The server is one static binary, so the image holds no C library and no shell, only the CA certificates for the HTTPS calls the server makes. CI builds it for every change.

It builds for `linux/amd64` and `linux/arm64`, such as with `docker buildx build --platform linux/amd64,linux/arm64`. The server is cross-compiled for each, not emulated.

Operators' commands run from the same image, in place of `server`:

```sh
docker run --rm --env-file migrate.env status-list-portal migrate
docker run --rm --env-file portal.env status-list-portal list-registrations
kubectl exec deploy/status-list-portal -- portal approve-registration 42
```

## The GitHub OAuth app

Create an [OAuth app](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app) under the organization that runs the portal:

- Homepage URL: the portal's address, such as `https://portal.example.com`.
- Authorization callback URL: that address followed by `/api/v1/session/github/callback`.

The portal asks for the `user:email` scope only. Its client ID goes in `APP_GITHUB__CLIENT_ID`, and its client secret is a secret.

## The admin API key

The portal signs its calls to the admin API with a key of its own. Make one, and keep the private half secret:

```sh
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out admin-key.pem
node -e 'console.log(JSON.stringify(require("node:crypto").createPublicKey(require("node:fs").readFileSync("admin-key.pem")).export({ format: "jwk" })))'
```

The second command prints the public half as a JWK. List the portal in the Status List Server's registry file, as its deployment runbook describes:

```yaml
admins:
  - id: portal
    scopes: [issuers:read, issuers:write]
    keys:
      - kid: portal-1
        alg: ES256
        jwk: {"kty": "EC", "crv": "P-256", "x": "...", "y": "..."}
```

Then `APP_ADMIN_API__PRINCIPAL` is `portal`, `APP_ADMIN_API__KEY_ID` is `portal-1`, and `APP_ADMIN_API__SIGNING_KEY_FILE` points to the mounted `admin-key.pem`.

## Settings and secrets

[`.env.template`](../.env.template) lists every setting with its default. These have none:

| Setting | Example |
| --- | --- |
| `APP_SERVER__ORIGIN` | `https://portal.example.com` |
| `APP_DATABASE__URL_FILE` | a file holding `postgres://portal:password@db:5432/portal` |
| `APP_SESSION__KEY_FILE` | a file holding the output of `openssl rand -hex 64` |
| `APP_GITHUB__CLIENT_ID` | from the OAuth app |
| `APP_GITHUB__CLIENT_SECRET_FILE` | a file holding the OAuth app's client secret |
| `APP_ADMIN_API__URL`, `__PRINCIPAL`, `__AUDIENCE`, `__KEY_ID`, `__SIGNING_KEY_FILE` | `http://status-list-server-admin:8002`, ... |
| `APP_MANAGEMENT_API__URL`, `__AUDIENCE` | the management API's public URL and audience |
| `APP_TERMS__VERSION`, `APP_TERMS__URL` | `2026-10-22`, `https://example.com/portal-terms` |

Keep the secrets out of the environment: the database URL, the session key and the GitHub client secret can each come from a file, named by the variable with `_FILE` appended, instead of from `APP_DATABASE__URL` and the like. With a Kubernetes Secret mounted at `/run/secrets/portal`:

```yaml
env:
  - name: APP_DATABASE__URL_FILE
    value: /run/secrets/portal/database-url
  - name: APP_SESSION__KEY_FILE
    value: /run/secrets/portal/session-key
  - name: APP_GITHUB__CLIENT_SECRET_FILE
    value: /run/secrets/portal/github-client-secret
  - name: APP_ADMIN_API__SIGNING_KEY_FILE
    value: /run/secrets/portal/admin-key.pem
```

A setting given both ways, or a file that is missing or empty, stops the server at startup. Changing the session key signs everyone out.

## Running in Kubernetes

The image needs no shell, no root and no writable file system. A pod spec along these lines keeps it that way, and lets the server read the Secret's files:

```yaml
spec:
  terminationGracePeriodSeconds: 45
  automountServiceAccountToken: false
  securityContext:
    runAsNonRoot: true
    runAsUser: 65532
    runAsGroup: 65532
    fsGroup: 65532
    seccompProfile:
      type: RuntimeDefault
  containers:
    - name: portal
      image: status-list-portal
      ports:
        - containerPort: 8080
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        capabilities:
          drop: [ALL]
      volumeMounts:
        - name: secrets
          mountPath: /run/secrets/portal
          readOnly: true
      startupProbe:
        httpGet: { path: /health/live, port: 8080 }
        periodSeconds: 5
        timeoutSeconds: 3
        failureThreshold: 12
      livenessProbe:
        httpGet: { path: /health/live, port: 8080 }
        periodSeconds: 10
        timeoutSeconds: 3
      readinessProbe:
        httpGet: { path: /health/ready, port: 8080 }
        periodSeconds: 5
        timeoutSeconds: 3
  volumes:
    - name: secrets
      secret:
        secretName: status-list-portal
        defaultMode: 0440
```

Restrict the network too, such as with a NetworkPolicy: only the ingress controller needs to reach the portal, since a client that connects directly bypasses the proxy's timeouts. The portal needs only its database, the Status List Server's admin API, GitHub, and the OpenTelemetry Collector if it exports telemetry. The reminder job needs the database, the admin API and the SMTP server.

## Behind a proxy

The proxy terminates HTTPS: the session cookies need it, and `APP_SERVER__ORIGIN` must be the address browsers use. List the proxy's addresses in `APP_SERVER__TRUSTED_PROXIES`, as narrowly as you can, such as the addresses the ingress controller's pods get. The server then reads the client's address from `X-Forwarded-For`, not `Forwarded`, but only from those proxies; anything else that sends the header is ignored. Every listed address can name any client, so list no other workload, and never `0.0.0.0/0`, which the server refuses.

Without the list, every request seems to come from the proxy, and all clients share one sign-in rate limit. A production server whose list is empty says so in its log when it starts.

The proxy itself must see the client's address. In Kubernetes, a load balancer in front of the ingress controller replaces it unless the controller's Service sets `externalTrafficPolicy: Local`, or the load balancer passes it on with the PROXY protocol.

The proxy must also time out slow clients: the server has no limit on how long request headers take to arrive, and its 30-second request timeout starts only once they have. Set a header read timeout and an idle connection timeout, such as NGINX's `client_header_timeout 10s` and `keepalive_timeout`.

## Health checks

- `GET /health/live` answers while the process runs: use it as the startup and liveness probe.
- `GET /health/ready` answers while the database does: use it as the readiness probe.

The server listens at once, since the migration job runs before the rollout, as [below](#database-roles-and-migrations). The startup probe in the pod spec above gives it a minute before the liveness probe takes over.

The readiness probe does not wait on the database. In the background, a pool of one connection checks the database and its migration history every five seconds, and gives each check two seconds. The server starts unready, stays unready while a migration it knows of is pending, and becomes unready after two failed checks in a row, or when no check has finished for 15 seconds. Allow the database one more connection per replica for it.

## Database roles and migrations

Use separate database roles so a compromised server cannot change the schema or
Diesel's migration history. The migration role owns portal tables; the server role
has DML access only. Never mount the migration credential in the server Pod.

Run `portal migrate` as a Kubernetes Job before rolling out `portal server`
(for example, with Helm `pre-install` and `pre-upgrade` hooks). The job needs
`APP_DATABASE__URL` or `APP_DATABASE__URL_FILE` and, optionally, `APP_ENV` (production by default),
`APP_LOG_FORMAT`, and `APP_DATABASE__TLS_CA_FILE`. Always use the same migration
role named in the default-privilege grants below. An advisory lock lets
overlapping jobs apply each pending migration once.

Give the Job its own environment rather than the server's: it reads no other secret, so
only the migrator's database URL needs mounting.

```yaml
containers:
  - name: migrate
    image: status-list-portal
    args: [migrate]
    env:
      - name: APP_DATABASE__URL_FILE
        value: /run/secrets/portal-migrator/database-url
    volumeMounts:
      - name: migrator
        mountPath: /run/secrets/portal-migrator
        readOnly: true
```

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

During a rolling update, the old release runs against the new schema, so each release's migrations must leave the previous release working:

- Add before you use: a new column is nullable or has a default, and a new table is used only by code released with it or after.
- Remove after you stop using: drop a column or a table one release after the code that used it.
- Rename in steps: add the new name, use both, then drop the old one.
- Never change a migration that was released; add another.

Back the database up. It holds the developers' accounts, which version of the terms each accepted, and when, and the registrations that wait for approval.

## Database TLS

- Production requires TLS with CA and hostname verification. Use
  `sslmode=require` for verified TLS; this connector does not accept libpq's
  `verify-full`. `prefer` is upgraded to `require` in production.
- Set `APP_DATABASE__TLS_CA_FILE` in both workloads to trust only the PEM CAs
  in that file. Without it, the system CAs are trusted.
- An explicit `sslmode=disable` in production is allowed only when every host
  and host address is loopback or a Unix socket, for a local proxy that encrypts
  the onward connection. Remote plaintext connections are refused.

## Rate limits

Registering, renewing and deleting issuers are limited per developer, in the database, so the limits hold across replicas. A registration or a renewal counts once the developer may make it, whether the sandbox then has room or not. The database keeps up to three rows per developer for this, deleted with the account.

Finishing a sign-in is limited per client network: an IPv4 address, or an IPv6 /64. Only sign-ins that started on the portal count. Each replica counts on its own, so with three replicas a network may sign in up to three times as often. A replica counts up to 10,000 networks at once; to count another, it forgets the one it counted longest ago, which only gives that network its full allowance back. No limit counts all networks together: anyone could use it up and lock everyone out. A client with many networks shows up as many `sign_in` refusals in `portal.rate_limited`: alert on those.

`.env.template` lists the defaults and their settings. The limits slow down one developer or one network. Someone with many GitHub accounts could still take the sandbox's places one account at a time; if that happens, `APP_SANDBOX__MANUAL_APPROVAL` puts every registration in front of an operator.

## Expiry reminders

`portal send-reminders` emails developers whose sandbox issuers expire within 7 days, and again within a day. Run it every hour as a CronJob; the server sends no email. A reminder goes out once per issuer, window and expiry date, so a rerun or an overlapping run sends nothing twice, and a renewed issuer is reminded again before its new expiry. A delivery that fails for a reason that may pass, a connection error, a timeout or an SMTP 4xx answer, is tried again by the next runs, an hour apart, 4 tries in all. An SMTP 5xx answer, such as for an unknown mailbox, gives it up at once. A run exits with an error when it gives a reminder up, or cannot reach the database, the admin API or the SMTP server. The `reminders` table keeps each reminder's tries and the SMTP server's last answer, which logs leave out since it can name the recipient.

Besides the database and admin API settings, and `APP_SERVER__ORIGIN`, which the emails link to, the job reads:

| Setting | Example |
| --- | --- |
| `APP_EMAIL__SMTP_URL_FILE` | a file holding `smtp://user:password@smtp.eu.mailgun.org:587?tls=required` |
| `APP_EMAIL__FROM` | `Status List Portal <noreply@portal.example.com>` |
| `APP_EMAIL__REPLY_TO` | optional |

`smtps://` connects over TLS, and `?tls=required` upgrades the connection with STARTTLS. Production refuses SMTP in the clear unless the server listens on loopback. Any provider that takes SMTP works, and changing provider changes these settings only. With Mailgun, use the sending domain's SMTP credentials and turn off its open and click tracking, which would add a tracking pixel and rewrite every link. Publish the SPF, DKIM and DMARC records the provider gives for the sender's domain, or mailboxes may take the emails for spam. The emails show the wordmark at `APP_SERVER__ORIGIN/email/adorsys-logo.png`, which the server serves with the frontend.

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: status-list-portal-reminders
spec:
  schedule: "17 * * * *"
  concurrencyPolicy: Forbid
  startingDeadlineSeconds: 600
  jobTemplate:
    spec:
      # The next run retries what failed; restarting at once would only spend tries.
      backoffLimit: 0
      activeDeadlineSeconds: 1800
      template:
        spec:
          restartPolicy: Never
          automountServiceAccountToken: false
          # The server's pod and container security contexts apply here too.
          containers:
            - name: send-reminders
              image: status-list-portal
              args: [send-reminders]
              env:
                - name: APP_SERVER__ORIGIN
                  value: https://portal.example.com
                - name: APP_DATABASE__URL_FILE
                  value: /run/secrets/portal-reminders/database-url
                - name: APP_ADMIN_API__URL
                  value: http://status-list-server-admin:8002
                - name: APP_ADMIN_API__SIGNING_KEY_FILE
                  value: /run/secrets/portal-reminders/admin-key.pem
                # ... the other APP_ADMIN_API__* settings, as the server has them.
                - name: APP_EMAIL__SMTP_URL_FILE
                  value: /run/secrets/portal-reminders/smtp-url
                - name: APP_EMAIL__FROM
                  value: Status List Portal <noreply@portal.example.com>
              volumeMounts:
                - name: secrets
                  mountPath: /run/secrets/portal-reminders
                  readOnly: true
          volumes:
            # Only what the job reads: the server's database URL, its admin key and the
            # SMTP URL. The session key and the GitHub secret stay with the server.
            - name: secrets
              secret:
                secretName: status-list-portal-reminders
                defaultMode: 0440
```

## Telemetry

[telemetry.md](telemetry.md) describes the logs, traces and metrics, and how to send them to an OpenTelemetry Collector.

## Backtraces

`RUST_BACKTRACE=1` adds a stack trace to a panic's message on stderr. Release builds keep function names but no line numbers.

## Stopping

On `SIGTERM`, the server stops taking connections and finishes the requests it is answering, which takes up to 30 seconds. It then sends the telemetry it still holds, for up to the longest export timeout: 10 seconds, unless `OTEL_EXPORTER_OTLP_TIMEOUT` says otherwise. Kubernetes kills a pod 30 seconds after `SIGTERM` by default, so the pod spec above gives it 45.
