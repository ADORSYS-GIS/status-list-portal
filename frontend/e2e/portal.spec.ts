/**
 * The portal end to end: the built frontend, served by the real server on a database of its
 * own, with stand-ins for GitHub and the admin API (`fakes.ts`).
 */

import { expect, test, type Page, type TestInfo } from '@playwright/test';

import { ADMIN_API, GITHUB, GITHUB_USER_COOKIE } from './settings.ts';

/** The sandbox's issuer lifetime, in seconds. */
const LIFETIME = 30 * 24 * 60 * 60;

interface HeldIssuer {
  display_name: string;
  owner: string;
  state: string;
  expires_at: number;
  version: number;
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

/** Fills in the registration form, from the list of issuers, and sends it. */
async function register(page: Page, name: string, key: string) {
  await page.getByRole('link', { name: 'Register an issuer' }).click();
  await page.getByLabel('Display name').fill(name);
  await page.getByLabel('Public key').fill(key);
  const terms = page.getByLabel('I have read and accept the terms of use');
  if (await terms.isVisible()) {
    await terms.check();
  }
  await page.getByRole('button', { name: 'Register issuer' }).click();
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

  await register(page, `Second ${testInfo.testId}`, key);

  await expect(
    page.getByText('This public key is already registered, maybe for an issuer you deleted'),
  ).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Register an issuer' })).toBeVisible();
});
