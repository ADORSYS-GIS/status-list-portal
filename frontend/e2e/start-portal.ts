/**
 * Starts the portal for the end-to-end tests: `bun e2e/start-portal.ts`, with the settings
 * that `playwright.config.ts` passes in `E2E_PORTAL_SETTINGS`.
 *
 * It empties the test database first, so every run starts from the same state, then builds
 * the portal, migrates the database and runs the server. The server gets only the tests'
 * settings: none of the portal's or OpenTelemetry's variables from the shell, and no `.env`
 * file, since it runs outside the repository.
 */

import { spawn, type ChildProcess, type SpawnOptions } from 'node:child_process';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import pg from 'pg';

import { DATABASE_URL } from './settings.ts';

const REPOSITORY = fileURLToPath(new URL('../..', import.meta.url));
/** A database the tests may drop: one named for them, so never a development database. */
const TEST_DATABASE = /^[a-z0-9_]+_e2e$/;
/** The hosts whose databases the tests may drop: this machine's. */
const LOCAL_HOSTS = ['localhost', '127.0.0.1', '[::1]'];
/** Variables of the shell that must not reach the server. */
const SETTINGS = /^(APP|OTEL)_/;

let running: ChildProcess | undefined;
for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => (running ? running.kill(signal) : process.exit(1)));
}

/** Runs `command` and resolves to its exit code. A signal to this script stops it too. */
function run(command: string, args: string[], options: SpawnOptions): Promise<number> {
  return new Promise((resolve, reject) => {
    running = spawn(command, args, { stdio: 'inherit', ...options });
    running.on('error', reject);
    running.on('exit', (code) => {
      running = undefined;
      resolve(code ?? 1);
    });
  });
}

/** Drops the database named in `url` and creates it again, empty. */
async function emptyDatabase(url: string) {
  const database = new URL(url);
  const name = database.pathname.slice(1);
  if (!TEST_DATABASE.test(name) || !LOCAL_HOSTS.includes(database.hostname)) {
    throw new Error(
      `the tests empty their database, so it must be on this machine and named *_e2e, not ${database.hostname}/${name}`,
    );
  }
  database.pathname = '/postgres';
  const client = new pg.Client({ connectionString: database.href });
  await client.connect();
  try {
    const identifier = client.escapeIdentifier(name);
    await client.query(`DROP DATABASE IF EXISTS ${identifier} WITH (FORCE)`);
    await client.query(`CREATE DATABASE ${identifier}`);
  } finally {
    await client.end();
  }
}

/** This script's environment, without the shell's settings, with the tests' instead. */
function portalEnvironment(): NodeJS.ProcessEnv {
  const settings: Record<string, string> = JSON.parse(process.env.E2E_PORTAL_SETTINGS ?? '{}');
  const inherited = Object.entries(process.env).filter(([name]) => !SETTINGS.test(name));
  return { ...Object.fromEntries(inherited), ...settings, APP_DATABASE__URL: DATABASE_URL };
}

await emptyDatabase(DATABASE_URL);
const built = await run('cargo', ['build', '--locked', '--bin', 'portal'], { cwd: REPOSITORY });
if (built !== 0) {
  process.exit(built);
}
// Where cargo put it. Cargo ran in the repository, so a relative CARGO_TARGET_DIR starts there.
const portal = resolve(REPOSITORY, process.env.CARGO_TARGET_DIR ?? 'target', 'debug', 'portal');
const options = { cwd: tmpdir(), env: portalEnvironment() };
const migrated = await run(portal, ['migrate'], options);
if (migrated !== 0) {
  process.exit(migrated);
}
process.exit(await run(portal, ['server'], options));
