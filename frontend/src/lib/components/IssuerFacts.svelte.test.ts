import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

import type { Issuer } from '#lib/api/index.ts';

import IssuerFacts from './IssuerFacts.svelte';

const NOW = new Date('2026-10-23T12:00:00Z');
const DAY = 24 * 60 * 60 * 1000;

function inDays(days: number): string {
  return new Date(NOW.getTime() + days * DAY).toISOString();
}

/** An expiry `days` from now, warned about from a week before it. */
function expiry(days: number): Pick<Issuer, 'expires_at' | 'expires_soon_from'> {
  return { expires_at: inDays(days), expires_soon_from: inDays(days - 7) };
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

test('an issuer far from its expiry is not warned about', async () => {
  const screen = await render(IssuerFacts, { issuer: issuer() });

  expect(fact(screen, 'Expires')?.textContent).toMatch(/in 20 days/);
  await expect.element(screen.getByText(/Expires soon/)).not.toBeInTheDocument();
});

test('an open page warns once the warning starts', async () => {
  vi.useRealTimers();
  const soon = (milliseconds: number) => new Date(Date.now() + milliseconds).toISOString();
  const screen = await render(IssuerFacts, {
    issuer: issuer({ expires_at: soon(3 * DAY), expires_soon_from: soon(200) }),
  });
  await expect.element(screen.getByText(/Expires soon/)).not.toBeInTheDocument();

  await expect.element(screen.getByText(/Expires soon/)).toBeVisible();
});

test('a tab shown again reads the time again', async () => {
  const screen = await render(IssuerFacts, { issuer: issuer() });
  await expect.element(screen.getByText(/Expires soon/)).not.toBeInTheDocument();

  // The clock moves past the warning's start while no timer fires, as in a sleeping laptop.
  vi.setSystemTime(new Date(NOW.getTime() + 14 * DAY));
  document.dispatchEvent(new Event('visibilitychange'));

  await expect.element(screen.getByText(/Expires soon/)).toBeVisible();
  expect(fact(screen, 'Expires')?.textContent).toMatch(/in 6 days/);
});

test('an issuer changed on an open page is read against the time now', async () => {
  const screen = await render(IssuerFacts, { issuer: issuer() });

  // Half a day goes by with no timer firing, then a renewal brings a new expiry.
  const later = new Date(NOW.getTime() + DAY / 2);
  vi.setSystemTime(later);
  await screen.rerender({
    issuer: issuer({ expires_at: new Date(later.getTime() + 30 * DAY).toISOString() }),
  });

  await expect.element(screen.getByText(/in 30 days/)).toBeVisible();
});
