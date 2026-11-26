/**
 * The end-to-end tests: `bun run test:e2e`, with Postgres running (`docker compose up -d`).
 *
 * They run the built frontend, served by the real server, with stand-ins for GitHub and the
 * admin API. With `E2E_REAL_GITHUB=1`, they sign in through GitHub itself instead, in a
 * visible browser, with a GitHub OAuth app for development: `E2E_GITHUB_CLIENT_ID` names it,
 * and `E2E_GITHUB_CLIENT_SECRET_FILE` the file that holds its secret, absolute or relative to
 * the repository's root. Never in CI.
 *
 * The servers start afresh for every run, on fixed ports: a server left running from an
 * earlier run makes this one fail rather than test stale code.
 */

import { randomBytes } from 'node:crypto';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { defineConfig, devices } from '@playwright/test';

import { ADMIN_API, GITHUB, GITHUB_STATE, PORTAL } from './e2e/settings.ts';

/** Where the commands run from, whatever the working directory. */
const REPOSITORY = new URL('..', import.meta.url);

const ci = Boolean(process.env.CI);
const realGitHub = process.env.E2E_REAL_GITHUB === '1';
if (ci && realGitHub) {
  throw new Error('signing in through GitHub itself needs a person, so CI never does');
}

function required(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is needed to sign in with GitHub itself`);
  }
  return value;
}

/** How the portal signs developers in: through the stand-in, or through GitHub. */
function gitHubSettings(): Record<string, string> {
  if (realGitHub) {
    return {
      APP_GITHUB__CLIENT_ID: required('E2E_GITHUB_CLIENT_ID'),
      // The secret stays in its file, which the portal reads. The portal runs elsewhere, so
      // the path must not be relative.
      APP_GITHUB__CLIENT_SECRET_FILE: resolve(
        fileURLToPath(REPOSITORY),
        required('E2E_GITHUB_CLIENT_SECRET_FILE'),
      ),
    };
  }
  return {
    APP_GITHUB__CLIENT_ID: GITHUB.clientId,
    APP_GITHUB__CLIENT_SECRET: GITHUB.clientSecret,
    APP_GITHUB__WEB_URL: GITHUB.url,
    APP_GITHUB__API_URL: GITHUB.url,
  };
}

/** Every setting the portal gets, besides its database: `start-portal.ts` passes no other. */
const portalSettings = {
  APP_ENV: 'development',
  APP_SERVER__ORIGIN: PORTAL.url,
  APP_SERVER__BIND_ADDRESS: PORTAL.bindAddress,
  APP_SERVER__FRONTEND_DIR: fileURLToPath(new URL('build', import.meta.url)),
  APP_SESSION__KEY: randomBytes(64).toString('hex'),
  ...gitHubSettings(),
  APP_ADMIN_API__URL: ADMIN_API.url,
  APP_ADMIN_API__PRINCIPAL: ADMIN_API.principal,
  APP_ADMIN_API__AUDIENCE: ADMIN_API.audience,
  APP_ADMIN_API__KEY_ID: ADMIN_API.keyId,
  APP_ADMIN_API__SIGNING_KEY_FILE: ADMIN_API.signingKeyFile,
  APP_MANAGEMENT_API__URL: 'https://status.example.com',
  APP_MANAGEMENT_API__AUDIENCE: 'https://status.example.com',
  APP_TERMS__VERSION: '2026-10-22',
  APP_TERMS__URL: `${PORTAL.url}/terms`,
  APP_PRIVACY_NOTICE__URL: `${PORTAL.url}/privacy`,
  APP_IMPRINT__URL: `${PORTAL.url}/imprint`,
  // Every test signs in from the same address.
  APP_RATE_LIMITS__SIGN_IN__BURST: '1000',
};

export default defineConfig({
  testDir: 'e2e',
  testMatch: realGitHub ? 'real-github.spec.ts' : 'portal.spec.ts',
  forbidOnly: ci,
  reporter: ci ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    ...devices['Desktop Chrome'],
    baseURL: PORTAL.url,
    headless: !realGitHub,
    storageState: realGitHub && existsSync(GITHUB_STATE) ? GITHUB_STATE : undefined,
    trace: 'retain-on-failure',
  },
  webServer: [
    {
      command: 'bun e2e/fakes.ts',
      url: `${ADMIN_API.url}/health`,
      reuseExistingServer: false,
    },
    {
      command: 'bun e2e/start-portal.ts',
      url: `http://${PORTAL.bindAddress}/health/ready`,
      env: {
        ...(process.env as Record<string, string>),
        E2E_PORTAL_SETTINGS: JSON.stringify(portalSettings),
      },
      // The first run builds the server.
      timeout: 10 * 60 * 1000,
      reuseExistingServer: false,
    },
  ],
});
