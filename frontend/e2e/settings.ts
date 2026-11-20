/** Where the end-to-end tests run the portal and the stand-ins for its services. */

import { fileURLToPath } from 'node:url';

/** The portal, as the browser reaches it. Session cookies need HTTPS or `localhost`. */
export const PORTAL = { url: 'http://localhost:18100', bindAddress: '127.0.0.1:18100' };

/** The stand-in for GitHub's sign-in pages and REST API. */
export const GITHUB = {
  port: 18090,
  url: 'http://127.0.0.1:18090/',
  clientId: 'e2e-client',
  clientSecret: 'e2e-client-secret',
};

/** The browser cookie on the stand-in for GitHub that names the account signing in. */
export const GITHUB_USER_COOKIE = 'github_user';

/** The stand-in for the Status List Server's admin API. */
export const ADMIN_API = {
  port: 18091,
  url: 'http://127.0.0.1:18091',
  principal: 'portal',
  audience: 'e2e-admin',
  keyId: 'e2e-admin-key',
  /** The private key of the portal's admin tokens: the test key, never a real one. */
  signingKeyFile: fileURLToPath(new URL('../../test_data/admin-keys/es256.pem', import.meta.url)),
};

/** The database the tests run the portal on. It is emptied before each run. */
export const DATABASE_URL =
  process.env.E2E_DATABASE_URL ?? 'postgres://postgres:postgres@localhost:5432/portal_e2e';

/** The browser state that keeps a real GitHub session between runs. Never commit it. */
export const GITHUB_STATE = fileURLToPath(new URL('.auth/github.json', import.meta.url));
