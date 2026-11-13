# Testing the portal against a live Status List Server

This guide starts the portal against a real Status List Server on your machine, so you can register and manage sandbox issuers end to end. It also runs the automated tests that check the portal against that server (issue #20).

The admin API is the Status List Server's own, built from its source. Nothing in the portal replaces or imitates it here. The portal only adds a launch script, a registry file that lists the portal as an admin, and the tests.

| Process | Address | What it is |
| --- | --- | --- |
| Portal | <http://localhost:8080> | Backend plus the built frontend |
| Status List Server, admin API | <http://127.0.0.1:8002> | Where the portal creates and changes issuers |
| Status List Server, management API | <http://localhost:8001> | Where developers' issuers publish status lists |
| Status List Server, public API | <http://127.0.0.1:8000> | Where anyone reads status lists |

The server runs `develop` at commit `6f53ece6e7eb5eb60b6a2776d93d8ff38f70092e`, because no release ships the admin API yet. Once one does, use that release instead.

## Prerequisites

- **Rust 1.99 or later.** Run `rustup update stable`; the portal does not build on older versions.
- **Docker**, for the portal's Postgres: `docker compose up -d` in the portal checkout. It listens on `POSTGRES_PORT`, 5432 by default. Use your own port wherever this guide says 5432.
- **cmake and Go**, which the server's crypto library needs to build (`sudo apt-get install -y cmake golang-go` on Ubuntu).
- **Bun**, to build the portal's frontend, and **Node.js 22 or later**, to make an issuer's key pair.
- **The server checkout** at the pinned commit, next to the portal:

  ```sh
  git clone https://github.com/adorsys/status-list-server.git ../status-list-server
  git -C ../status-list-server checkout 6f53ece6e7eb5eb60b6a2776d93d8ff38f70092e
  ```

- **A GitHub account**, to create the OAuth app that signs you in. The next section shows how.

## Configure the portal's `.env`

The portal refuses to start until three values in `.env` are set. `.env` is ignored by git; never commit it.

| Variable | What it is | Where it comes from |
| --- | --- | --- |
| `APP_GITHUB__CLIENT_ID` | Identifies the portal to GitHub when someone signs in | Your GitHub OAuth app |
| `APP_GITHUB__CLIENT_SECRET` | Proves to GitHub that the request comes from the portal | Your GitHub OAuth app, shown once |
| `APP_SESSION__KEY` | Encrypts session cookies: 64 random bytes in hex | `openssl rand -hex 64` |

1. In the portal checkout, create `.env` from the template, if you don't have one yet:

   ```sh
   cp .env.template .env
   ```

2. Generate the session key and write it into `.env`. Changing it later signs everyone out:

   ```sh
   sed -i "s/^APP_SESSION__KEY=.*/APP_SESSION__KEY=$(openssl rand -hex 64)/" .env
   ```

3. Create the GitHub OAuth app. Open <https://github.com/settings/applications/new> (GitHub → Settings → Developer settings → OAuth Apps → New OAuth App) and fill in:

   | Field | Value |
   | --- | --- |
   | Application name | Anything, such as `Status List Portal (local)` |
   | Homepage URL | `http://localhost:8080` |
   | Authorization callback URL | `http://localhost:8080/api/v1/session/github/callback` |

4. Click **Register application**. Copy the **Client ID**, then click **Generate a new client secret** and copy the secret right away: GitHub shows it only once.

5. Put both into `.env`:

   ```sh
   APP_GITHUB__CLIENT_ID=<the client ID>
   APP_GITHUB__CLIENT_SECRET=<the client secret>
   ```

6. Check the other values the local setup relies on:

   ```sh
   APP_SERVER__ORIGIN=http://localhost:8080
   APP_SERVER__FRONTEND_DIR=frontend/build
   APP_DATABASE__URL=postgres://postgres:postgres@localhost:5432/portal
   ```

The callback URL must be exactly `APP_SERVER__ORIGIN` followed by `/api/v1/session/github/callback`, or GitHub refuses the sign-in. Leave the `APP_ADMIN_API__*` and `APP_MANAGEMENT_API__*` values as they are; step 2 overrides them on the command line.

## Step 1: start the Status List Server

The server gets a database of its own on the same Postgres, and runs the hosted profile so it accepts portal issuers.

1. Create the server's database:

   ```sh
   PGPASSWORD=postgres psql -h localhost -p 5432 -U postgres -c 'CREATE DATABASE status_list'
   ```

2. Build the server with Postgres support. The first build takes several minutes:

   ```sh
   (cd ../status-list-server && cargo build --features postgres)
   ```

3. Start it from the portal checkout, and leave this terminal open:

   ```sh
   scripts/live-status-list-server.sh ../status-list-server postgres://postgres:postgres@localhost:5432/status_list
   ```

4. In another terminal, check that it is ready. It should print `200`:

   ```sh
   curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8002/health/ready
   ```

The script sets everything the server needs:

| Setting | Value | Why |
| --- | --- | --- |
| `APP_SERVER__PROFILE` | `hosted` | Portal issuers exist only in the hosted profile |
| `APP_MANAGEMENT_AUTH__ISSUERS_FILE` | `test_data/status-list-server/issuers.yaml` | Lists the portal as admin `portal`, key `test-admin-key`, scopes `issuers:read` and `issuers:write` |
| `APP_ADMIN_AUTH__AUDIENCES` | `test-admin-audience` | The audience the portal's admin tokens must carry |
| `APP_MANAGEMENT_AUTH__AUDIENCES` | `test-management-audience` | The audience issuers' management tokens must carry |
| `APP_RATE_LIMIT__ADMIN_BURST_SIZE` | `100000` | The tests make more admin calls than the default burst of 100 |
| Certificate paths | the server's `test_data/ed25519_*.pem` | The key the server signs status lists with |

The registry holds only the public half of `test_data/admin-keys/es256.pem`, a throwaway test key. Never use it outside a local or CI server.

## Step 2: start the portal

The portal must sign its admin tokens with the key and audience the server's registry expects. Pass those settings on the command line; they take precedence over `.env`, so `.env` stays as it is.

1. Build the frontend once, so the backend can serve it:

   ```sh
   bun install && bun run build
   ```

2. Start the portal, and leave this terminal open:

   ```sh
   APP_ADMIN_API__URL=http://127.0.0.1:8002 \
   APP_ADMIN_API__PRINCIPAL=portal \
   APP_ADMIN_API__AUDIENCE=test-admin-audience \
   APP_ADMIN_API__KEY_ID=test-admin-key \
   APP_ADMIN_API__SIGNING_KEY_FILE=test_data/admin-keys/es256.pem \
   APP_MANAGEMENT_API__URL=http://localhost:8001 \
   APP_MANAGEMENT_API__AUDIENCE=test-management-audience \
   cargo run -- server
   ```

3. Check that it is ready. It should print `200`, after about a minute on the first build:

   ```sh
   curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/health/ready
   ```

| Portal setting | Must match on the server |
| --- | --- |
| `APP_ADMIN_API__PRINCIPAL=portal` | The admin's `id` in `issuers.yaml` |
| `APP_ADMIN_API__KEY_ID=test-admin-key` | That admin's key `kid` |
| `APP_ADMIN_API__SIGNING_KEY_FILE` | The private half of that key's `jwk` |
| `APP_ADMIN_API__AUDIENCE=test-admin-audience` | `APP_ADMIN_AUTH__AUDIENCES` |
| `APP_MANAGEMENT_API__AUDIENCE=test-management-audience` | `APP_MANAGEMENT_AUTH__AUDIENCES` |

If any of them differs, the portal's calls fail with `invalid_token`, and the portal answers 502.

## Step 3: test it in the browser

Open <http://localhost:8080> and walk through a sandbox issuer's whole life. Each step calls the real admin API.

### Sign in

The home page explains the sandbox. Click **Sign in with GitHub** and authorize the OAuth app.

![The portal's home page, signed out, with Sign in with GitHub](images/live-testing/home.png)

### Register an issuer

Accept the terms of use if asked, then open **Register an issuer**. Enter a display name, keep **ES256**, and paste the public key as a JSON Web Key. **Register issuer** makes the portal call the server's `createIssuer`.

![The Register an issuer form: display name, algorithm and public key](images/live-testing/register.png)

#### Generate the key pair

The form needs the **public** key as a JSON Web Key. This is the same script the portal shows under **Need a key pair?**, for ES256.

1. Make an empty folder for the issuer's keys, outside both repositories:

   ```sh
   mkdir -p ~/sandbox-issuer && cd ~/sandbox-issuer
   ```

2. Save the script as `make-key.mjs`:

   ```sh
   cat > make-key.mjs <<'EOF'
   // make-key.mjs: run `node make-key.mjs` (Node.js 22 or later).
   import { writeFile } from 'node:fs/promises';

   const { privateKey, publicKey } = await crypto.subtle.generateKey(
     { name: 'ECDSA', namedCurve: 'P-256' },
     true,
     ['sign', 'verify'],
   );
   // Keep private-key.json to yourself: it signs your issuer's tokens. The script refuses to
   // overwrite one you already have.
   await writeFile('private-key.json', JSON.stringify(await crypto.subtle.exportKey('jwk', privateKey)), {
     mode: 0o600,
     flag: 'wx',
   });
   console.log(JSON.stringify(await crypto.subtle.exportKey('jwk', publicKey)));
   EOF
   ```

3. Run it. It saves the private key in `private-key.json` and prints the public key:

   ```sh
   node make-key.mjs
   ```

4. Paste the printed line, which looks like `{"key_ops":["verify"],"ext":true,"kty":"EC","x":"…","y":"…","crv":"P-256"}`, into **Public key**.

Never paste `private-key.json`: the portal refuses a private key. Each issuer needs its own key pair; the server answers 409 to a key it has seen. For a second issuer, run the script in another folder.

### Read the issuer's details

The portal confirms that the issuer is ready and shows what its tokens need:

- **Issuer id (iss)**: assigned by the server.
- **Key id (kid)**: the key's thumbprint, computed by the server.
- **Audience (aud)**: `test-management-audience`.
- **Management API**: `http://localhost:8001`.
- **Expires**: 30 days from now.

![A new issuer's page: its id, kid, audience, management API and expiry](images/live-testing/issuer.png)

### Publish a status list

In the folder with `private-key.json`, save the page's `mint-token.mjs` script. Then run the page's **Publish a status list** commands. The token it signs is checked by the real management API, so a `2xx` answer proves the issuer works end to end.

![The mint-token.mjs script and the commands that publish a status list, above the Renew and Delete buttons](images/live-testing/publish.png)

### Manage the issuer

**Your issuers** lists every issuer of the account, with its state and expiry.

![Your issuers: one active issuer, expiring in 30 days](images/live-testing/issuers.png)

On an issuer's page:

- **Renew for 30 days** sends a conditional change with `If-Issuer-Version`; the expiry moves forward.
- **Delete issuer** suspends it; it stays listed as **Suspended**, and its tokens stop working.

## Run the automated live tests

With the server from step 1 running, the six tests in `src/tests/live_admin_api.rs` pass in under a second. The portal does not need to run; the tests start their own.

```sh
LIVE_ADMIN_API_URL=http://127.0.0.1:8002 cargo test --test integration live_admin_api -- --ignored
```

The portal's other tests talk to a stand-in for the admin API, which stays the same when the server changes. These tests run the portal's real code against the real server instead:

| Test | What it checks |
| --- | --- |
| `created_issuers_are_sandbox_issuers_of_their_owner` | A created issuer's fields, its key and its 30-day expiry |
| `an_issuer_is_read_by_its_owner_only` | Another account and an unknown id both see nothing |
| `issuers_are_listed_by_owner_state_tier_and_source` | The list filters and counts that the sandbox limits rely on |
| `changes_apply_to_the_version_they_were_read_at` | Renew, suspend and resume with `If-Issuer-Version`; a stale version is refused |
| `refusals_carry_the_codes_the_portal_branches_on` | `key_material_in_use`, `invalid_public_jwk`, `invalid_expiry`, `invalid_token` |
| `developers_register_renew_and_delete_issuers` | Register, list, view, renew and delete through the portal's own routes |

Each test generates its own keys and GitHub account ids, and suspends what it creates, so the tests can run again against the same server. A test that fails stops before that, and leaves an active issuer behind: drop the server's database (see below) if failed runs pile up. `cargo test` reports the tests as ignored; `--ignored` runs them, and then they fail unless `LIVE_ADMIN_API_URL` is set, so a run meant to check the server never passes without reaching it.

The portal sends the version from the `Issuer-Version` header back as `If-Issuer-Version`, the header adorsys/status-list-server#739 introduced. The server still accepts `If-Match` with the quoted `ETag` too, so these tests would not catch a portal that sent that instead; they do catch one that sends the bare version as `If-Match`, or no version at all.

In CI, the `live-admin-api` job in `.github/workflows/ci.yml` builds the server at `STATUS_LIST_SERVER_REF`, starts it with the same script, and runs these tests. A nightly run does the same against the head of the server's `develop`, so a change to the admin API fails there before the pin moves to it.

## Stop and clean up

Press Ctrl+C in the portal's and the server's terminals. To start the next run with no issuers, drop the server's database, then create it again in step 1:

```sh
PGPASSWORD=postgres psql -h localhost -p 5432 -U postgres -c 'DROP DATABASE status_list'
```

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `rustc 1.97.0 is not supported` | Rust is older than 1.99 | `rustup update stable` |
| Script: `status-list-server: No such file or directory` | The server was built without the `memory` feature, so it has no binary | Build with `cargo build --features postgres`, not `--no-default-features` |
| The portal answers 502 on every issuer page | The portal's admin settings don't match the registry | Compare against the table in step 2 |
| `DROP DATABASE` fails: database is being accessed | The server is still running | Stop it first |
| Registering answers 409 | That public key is registered already | Generate a new key pair |
| Publishing a status list answers 401 | The token's `aud` or `kid` is wrong, or it expired after two minutes | Copy the script again from the issuer's page, and mint a fresh token |
