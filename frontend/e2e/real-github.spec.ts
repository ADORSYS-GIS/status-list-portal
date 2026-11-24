/**
 * Signing in with a real GitHub account, through a GitHub OAuth app for development. A person
 * signs in on github.com the first time; the browser then keeps the GitHub session in
 * `.auth/github.json`, so later runs go straight through. Runs only with `E2E_REAL_GITHUB`
 * set: see `playwright.config.ts`.
 */

import { mkdir } from 'node:fs/promises';
import { dirname } from 'node:path';

import { expect, test } from '@playwright/test';

import { GITHUB_STATE } from './settings.ts';

/** Time for a person to sign in on GitHub and approve the app. */
const SIGN_IN_TIMEOUT = 5 * 60 * 1000;

test('a developer signs in with their GitHub account', async ({ page }) => {
  test.setTimeout(SIGN_IN_TIMEOUT + 30 * 1000);
  await page.goto('/');

  await page.getByRole('banner').getByRole('link', { name: 'Sign in with GitHub' }).click();

  // On github.com, sign in and approve the app, unless the saved session lets GitHub send
  // the browser straight back.
  const issuers = page.getByRole('heading', { name: 'Your issuers' });
  await expect(issuers).toBeVisible({ timeout: SIGN_IN_TIMEOUT });
  await expect(page.getByRole('navigation', { name: 'Account' })).toContainText('Signed in as');
  await mkdir(dirname(GITHUB_STATE), { recursive: true });
  await page.context().storageState({ path: GITHUB_STATE });
});
