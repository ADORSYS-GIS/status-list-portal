/**
 * The portal end to end: the built frontend, served by the real server on a database of its
 * own, with stand-ins for GitHub and the admin API (`fakes.ts`).
 */

import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test';

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

/** The rules of WCAG 2.2 at levels A and AA, as axe tags them. */
const WCAG_AA = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'];

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
  const terms = page.getByLabel('I have read and accept the terms of use');
  if (termsAccepted) {
    await expect(terms).toBeHidden();
  } else {
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

/** Checks the page as it is now against WCAG 2.2 AA, naming each failure and where it is. */
async function expectAccessible(page: Page) {
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG_AA).analyze();
  const failures = violations.map(
    ({ id, help, nodes }) => `${id}: ${help} (${nodes.map((node) => node.target).join(', ')})`,
  );
  expect(failures).toEqual([]);
}

/** The relative luminance of a color the browser computed, such as `rgb(100, 116, 139)`. */
function luminance(color: string): number {
  const [red, green, blue] = (color.match(/[\d.]+/g) ?? []).slice(0, 3).map((channel) => {
    const value = Number(channel) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/**
 * The contrast of a field's border against the background around it (WCAG 1.4.11), which
 * axe does not check.
 */
async function borderContrast(field: Locator): Promise<number> {
  const [border, background] = await field.evaluate((element) => {
    let around = element.parentElement;
    while (around && getComputedStyle(around).backgroundColor === 'rgba(0, 0, 0, 0)') {
      around = around.parentElement;
    }
    return [
      getComputedStyle(element).borderTopColor,
      getComputedStyle(around ?? document.documentElement).backgroundColor,
    ];
  });
  const [lighter, darker] = [luminance(border), luminance(background)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
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

  await register(page, `Second ${testInfo.testId}`, key, { termsAccepted: true });

  await expect(
    page.getByText('This public key is already registered, maybe for an issuer you deleted'),
  ).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Register an issuer' })).toBeVisible();
  // Nothing typed has to be typed again (WCAG 3.3.7).
  await expect(page.getByLabel('Display name')).toHaveValue(`Second ${testInfo.testId}`);
  await expect(page.getByLabel('Public key')).toHaveValue(key);
  await expectAccessible(page);
});

test('the keyboard skips to the content and works the delete dialog', async ({
  page,
}, testInfo) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /^Try the Status List Server/ })).toBeVisible();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to content' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('main')).toBeFocused();

  const name = `Wallet ${testInfo.testId}`;
  await signIn(page, accountFor(testInfo));
  await register(page, name, await publicKey());
  const remove = page.getByRole('button', { name: 'Delete issuer' });
  await remove.focus();
  await page.keyboard.press('Enter');

  const confirm = page.getByRole('dialog', { name: `Delete ${name}?` });
  await expect(confirm.getByRole('button', { name: 'Keep issuer' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(confirm).toBeHidden();
  await expect(remove).toBeFocused();
});

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(`in the ${colorScheme} theme`, () => {
    test.use({ colorScheme });

    test('the home page meets WCAG 2.2 AA', async ({ page }) => {
      await page.goto('/');
      await expect(
        page.getByRole('heading', { name: /^Try the Status List Server/ }),
      ).toBeVisible();
      await expectAccessible(page);

      await page.goto('/?sign_in_error=denied');
      await expect(page.getByRole('alert')).toContainText('Signing in was cancelled on GitHub.');
      await expectAccessible(page);
    });

    test('the registration page meets WCAG 2.2 AA', async ({ page }, testInfo) => {
      await signIn(page, accountFor(testInfo));
      await page.getByRole('link', { name: 'Register an issuer' }).click();
      await expect(page.getByRole('heading', { name: 'Register an issuer' })).toBeVisible();
      await expectAccessible(page);
      for (const label of ['Display name', 'Public key']) {
        expect(await borderContrast(page.getByLabel(label))).toBeGreaterThanOrEqual(3);
      }

      await page.getByRole('button', { name: 'Register issuer' }).click();
      await expect(page.getByLabel('Display name')).toBeFocused();
      await expectAccessible(page);
    });

    test("an issuer's page meets WCAG 2.2 AA", async ({ page }, testInfo) => {
      const name = `Wallet ${testInfo.testId}`;
      await signIn(page, accountFor(testInfo));
      await register(page, name, await publicKey());
      await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
      await expectAccessible(page);

      await page.getByRole('button', { name: 'Delete issuer' }).click();
      await expect(page.getByRole('dialog', { name: `Delete ${name}?` })).toBeVisible();
      await expectAccessible(page);
    });
  });
}
