/**
 * The portal end to end: the built frontend, served by the real server on a database of its
 * own, with stand-ins for GitHub and the admin API (`fakes.ts`).
 */

import { createPrivateKey, createPublicKey, sign, verify } from 'node:crypto';
import { readFile } from 'node:fs/promises';

import { expect, test, type Page, type TestInfo } from '@playwright/test';

import { ADMIN_API, GITHUB, GITHUB_USER_COOKIE, PORTAL } from './settings.ts';

/** The sandbox's issuer lifetime, in seconds. */
const LIFETIME = 30 * 24 * 60 * 60;

interface HeldIssuer {
  display_name: string;
  owner: string;
  state: string;
  expires_at: number;
  version: number;
  keys: { kid: string; state: string; public_key: JsonWebKey }[];
  usage: { keys: number; keys_limit: number };
}

/** A GitHub account of the test's own, so tests share nothing. */
function accountFor(testInfo: TestInfo): string {
  return `developer-${testInfo.workerIndex}-${testInfo.testId}`;
}

/** Signs in with GitHub as `login`, an account the stand-in creates on first use. */
async function signIn(page: Page, login: string) {
  await page.context().addCookies([{ name: GITHUB_USER_COOKIE, value: login, url: GITHUB.url }]);
  await page.goto('/');
  await page.getByRole('banner').getByRole('link', { name: 'Sign in with GitHub' }).click();

  await expect(page.getByRole('heading', { name: 'Your issuers' })).toBeVisible();
  const account = page.getByRole('navigation', { name: 'Account' });
  await expect(account).toContainText(`Signed in as ${login}`);
}

/** The public half of a new key pair, as a JSON Web Key. */
async function publicKey(): Promise<string> {
  const pair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, [
    'sign',
    'verify',
  ]);
  const { kty, crv, x, y } = await crypto.subtle.exportKey('jwk', pair.publicKey);
  return JSON.stringify({ kty, crv, x, y });
}

/**
 * Fills in the registration form, from the list of issuers, and sends it. The form asks for
 * the terms until the account has accepted them.
 */
async function register(page: Page, name: string, key: string, { termsAccepted = false } = {}) {
  await page.getByRole('link', { name: 'Register an issuer' }).click();
  await page.getByLabel('Display name').fill(name);
  await page.getByLabel('Public key').fill(key);
  await submitRegistration(page, { termsAccepted });
}

/** Sends the registration form, accepting the terms unless the account already has. */
async function submitRegistration(page: Page, { termsAccepted = false } = {}) {
  const terms = page.getByLabel('I have read and accept the terms of use');
  if (termsAccepted) {
    await expect(terms).toBeHidden();
  } else {
    await terms.check();
  }
  await page.getByRole('button', { name: 'Register issuer' }).click();
}

/** What the download that `button` starts holds. */
async function download(page: Page, button: string): Promise<string> {
  const [file] = await Promise.all([
    page.waitForEvent('download'),
    page.getByRole('button', { name: button }).click(),
  ]);
  return readFile(await file.path(), 'utf8');
}

/** The issuer named `name`, as the stand-in for the admin API holds it. */
async function heldIssuer(name: string): Promise<HeldIssuer> {
  const response = await fetch(`${ADMIN_API.url}/e2e/issuers`);
  const issuers: HeldIssuer[] = await response.json();
  const issuer = issuers.find((held) => held.display_name === name);
  expect(issuer, `the admin API holds no issuer named ${name}`).toBeDefined();
  return issuer as HeldIssuer;
}

/** Whether `expiresAt` is one sandbox lifetime from now, give or take a few minutes. */
function oneLifetimeAway(expiresAt: number): boolean {
  const fromNow = expiresAt - Date.now() / 1000;
  return fromNow > LIFETIME - 10 * 60 && fromNow <= LIFETIME;
}

test('every page links the legal documents', async ({ page }, testInfo) => {
  const links = [
    { name: 'Terms of use', path: '/terms' },
    { name: 'Privacy notice', path: '/privacy' },
    { name: 'Imprint', path: '/imprint' },
  ];
  async function checkFooter() {
    const footer = page.getByRole('contentinfo');
    for (const { name, path } of links) {
      await expect(footer.getByRole('link', { name })).toHaveAttribute(
        'href',
        `${PORTAL.url}${path}`,
      );
    }
  }

  await page.goto('/');
  await checkFooter();
  await page.goto('/missing');
  await checkFooter();
  await signIn(page, accountFor(testInfo));
  await checkFooter();
  await page.goto('/issuers/new');
  await checkFooter();
});

test('a developer signs in with GitHub and out again', async ({ page }, testInfo) => {
  await signIn(page, accountFor(testInfo));

  await page.getByRole('button', { name: 'Sign out' }).click();

  await expect(page.getByRole('heading', { name: /^Try the Status List Server/ })).toBeVisible();
  await page.goto('/issuers');
  await expect(page).toHaveURL('/');
});

test('a developer registers an issuer, renews it and deletes it', async ({ page }, testInfo) => {
  const name = `Wallet ${testInfo.testId}`;
  await signIn(page, accountFor(testInfo));

  await register(page, name, await publicKey());

  await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
  await expect(page.getByText('Your issuer is ready.')).toBeVisible();
  await expect(page.getByRole('region', { name: 'mint-token.mjs' })).toContainText(
    'statuslist-management+jwt',
  );
  const registered = await heldIssuer(name);
  expect(registered.owner).toMatch(/^github\|\d+$/);
  expect(oneLifetimeAway(registered.expires_at)).toBe(true);

  await page.getByRole('button', { name: 'Renew for 30 days' }).click();

  await expect(page.getByText('Renewed. The new expiry date is below.')).toBeVisible();
  const renewed = await heldIssuer(name);
  expect(renewed.version).toBe(registered.version + 1);
  expect(renewed.expires_at).toBeGreaterThanOrEqual(registered.expires_at);
  expect(oneLifetimeAway(renewed.expires_at)).toBe(true);

  await page.getByRole('button', { name: 'Delete issuer' }).click();
  const confirm = page.getByRole('dialog', { name: `Delete ${name}?` });
  await confirm.getByRole('button', { name: 'Delete issuer' }).click();

  await expect(page).toHaveURL('/issuers');
  await expect(page.getByText(`${name} is deleted.`)).toBeVisible();
  await expect(page.getByRole('row', { name })).toContainText('Deleted');
  expect((await heldIssuer(name)).state).toBe('suspended');
});

test('a key registers one issuer only', async ({ page }, testInfo) => {
  const key = await publicKey();
  await signIn(page, accountFor(testInfo));
  await register(page, `First ${testInfo.testId}`, key);
  await expect(page.getByText('Your issuer is ready.')).toBeVisible();
  await page
    .getByRole('navigation', { name: 'Account' })
    .getByRole('link', { name: 'Issuers' })
    .click();

  await register(page, `Second ${testInfo.testId}`, key, { termsAccepted: true });

  await expect(
    page.getByText('This public key is already registered, maybe for an issuer you deleted'),
  ).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Register an issuer' })).toBeVisible();
});

test('a developer adds a key, reaches the limit, and revokes the old key', async ({
  page,
}, testInfo) => {
  const name = `Rotation ${testInfo.testId}`;
  await signIn(page, accountFor(testInfo));
  await register(page, name, await publicKey());
  await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
  await expect(page.getByText('Your issuer is ready.')).toBeVisible();
  const before = await heldIssuer(name);
  const oldKid = before.keys[0].kid;

  await page.getByRole('button', { name: 'Make a key pair in this browser' }).click();
  const privateJwk: JsonWebKey = JSON.parse(await download(page, 'Download private-key.json'));
  const { kty, crv, x, y } = privateJwk;
  await expect(page.getByLabel('Public key (JWK)')).toHaveValue(JSON.stringify({ kty, crv, x, y }));
  await page.getByRole('button', { name: 'Add public key' }).click();
  await expect(page.getByText('Key added. Move your signers')).toBeVisible();
  const added = await heldIssuer(name);
  expect(added.version).toBe(before.version + 1);
  expect(added.usage.keys).toBe(2);
  expect(added.keys.filter((key) => key.state === 'active')).toHaveLength(2);
  await expect(page.getByText('The key limit is reached.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add public key' })).toBeHidden();

  await page.getByRole('button', { name: `Revoke key ${oldKid}` }).click();
  const confirm = page.getByRole('dialog', { name: `Revoke key ${oldKid}?` });
  await confirm.getByRole('button', { name: 'Revoke key' }).click();
  await expect(page.getByText('Key revoked. Tokens signed with it')).toBeVisible();
  const revoked = await heldIssuer(name);
  expect(revoked.version).toBe(added.version + 1);
  expect(revoked.keys.find((key) => key.kid === oldKid)?.state).toBe('revoked');
  expect(revoked.keys.filter((key) => key.state === 'active')).toHaveLength(1);
  expect(revoked.usage.keys).toBe(1);
});

test('a key pair made in the browser never leaves it', async ({ page }, testInfo) => {
  const name = `Browser key ${testInfo.testId}`;
  // Capture every request without waiting for headers on requests that may stay open.
  const requests: { url: string; headers: object; body: string | null }[] = [];
  page.context().on('request', (request) => {
    requests.push({ url: request.url(), headers: request.headers(), body: request.postData() });
  });
  await signIn(page, accountFor(testInfo));
  await page.getByRole('link', { name: 'Register an issuer' }).click();
  await page.getByLabel('Display name').fill(name);

  await page.getByRole('button', { name: 'Make a key pair in this browser' }).click();
  const pem = await download(page, 'Download private-key.pem');
  const jwk: JsonWebKey = JSON.parse(await download(page, 'Download private-key.json'));
  await submitRegistration(page);

  await expect(page.getByText('Your issuer is ready.')).toBeVisible();
  const sent = requests;
  // Exactly what the page means to send, so that no encoding of the private key gets by.
  const { kty, crv, x, y } = jwk;
  expect(
    sent
      .filter((request) => request.body !== null)
      .map(({ url, body }) => ({ path: new URL(url).pathname, body: JSON.parse(body ?? '') })),
  ).toEqual([
    { path: '/api/v1/me/terms', body: { version: expect.any(String) } },
    {
      path: '/api/v1/issuers',
      body: { display_name: name, alg: 'ES256', public_key: { kty, crv, x, y } },
    },
  ]);
  expect(jwk.d, 'the JWK holds a P-256 private key').toMatch(/^[\w-]{43}$/);
  // The PEM's full lines, which hold the private key; its last line is too short to tell.
  const secrets = [jwk.d, ...pem.split('\n').filter((line) => line.length === 64)];
  for (const request of sent) {
    for (const secret of secrets) {
      expect(JSON.stringify(request), 'a request carried the private key').not.toContain(secret);
    }
  }
  const cookies = await page.context().cookies();
  for (const secret of secrets) {
    expect(JSON.stringify(cookies), 'a browser cookie carried the private key').not.toContain(
      secret,
    );
  }
  // Signing tests the private halves: the public point each file also holds would match anyway.
  const registered = createPublicKey({
    key: (await heldIssuer(name)).keys[0].public_key,
    format: 'jwk',
  });
  const data = Buffer.from('a management token');
  for (const key of [createPrivateKey(pem), createPrivateKey({ key: jwk, format: 'jwk' })]) {
    expect(verify('sha256', data, registered, sign('sha256', data, key))).toBe(true);
  }
});
