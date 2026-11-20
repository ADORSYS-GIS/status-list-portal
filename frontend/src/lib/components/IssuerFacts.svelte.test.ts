import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

import type { Issuer } from '#lib/api/index.ts';

import IssuerFacts from './IssuerFacts.svelte';

const NOW = new Date('2026-10-23T12:00:00Z');
const DAY = 24 * 60 * 60 * 1000;

function inDays(days: number): string {
  return new Date(NOW.getTime() + days * DAY).toISOString();
}

/** An expiry `days` from now, warned about from `windowDays` before it. */
function expiry(days: number, windowDays = 7): Pick<Issuer, 'expires_at' | 'renew_from'> {
  return { expires_at: inDays(days), renew_from: inDays(days - windowDays) };
}

beforeEach(() => {
  // Only the clock: the browser assertions poll with real timers.
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

function issuer(overrides: Partial<Issuer> = {}): Issuer {
  return {
    id: '6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00',
    display_name: 'Acme Wallet Issuer',
    state: 'active',
    created_at: '2026-10-01T00:00:00Z',
    ...expiry(20),
    keys: [{ kid: 'the-kid', alg: 'ES256', state: 'active' }],
    usage: { lists: 3, lists_limit: 10, keys: 1, keys_limit: null },
    management_url: 'https://status.example.com',
    audience: 'https://status.example.com',
    ...overrides,
  };
}

/** The value of the fact labelled `label`. */
function fact(screen: Awaited<ReturnType<typeof render>>, label: string) {
  return screen.getByText(label, { exact: true }).element().nextElementSibling;
}

test('the status lists and keys show against their limits', async () => {
  const screen = await render(IssuerFacts, { issuer: issuer() });

  expect(fact(screen, 'Status lists')?.textContent).toBe('3 of 10');
  expect(fact(screen, 'Keys in use')?.textContent).toBe('1');
});

test('a limit on keys is shown when there is one', async () => {
  const screen = await render(IssuerFacts, {
    issuer: issuer({ usage: { lists: 0, lists_limit: 10, keys: 2, keys_limit: 5 } }),
  });

  expect(fact(screen, 'Keys in use')?.textContent).toBe('2 of 5');
});

test('an issuer close to its expiry is warned about', async () => {
  const screen = await render(IssuerFacts, { issuer: issuer({ ...expiry(2) }) });

  expect(fact(screen, 'Expires')?.textContent).toMatch(/in 2 days/);
  await expect.element(screen.getByText(/Expires soon/)).toBeVisible();
});

test('an expired issuer is not warned about', async () => {
  const screen = await render(IssuerFacts, {
    issuer: issuer({ state: 'expired', ...expiry(-1) }),
  });

  await expect.element(screen.getByText(/^Expired on /)).toBeVisible();
  await expect.element(screen.getByText(/Expires soon/)).not.toBeInTheDocument();
});
