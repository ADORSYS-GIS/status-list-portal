import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

import type { Issuer } from '#lib/api/index.ts';

import IssuerTable from './IssuerTable.svelte';

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

function issuer(id: string, overrides: Partial<Issuer> = {}): Issuer {
  return {
    id,
    display_name: id,
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

type Screen = Awaited<ReturnType<typeof render>>;

/** The text in row `row` (the header row is 0) of the column headed `header`. */
function cell(screen: Screen, row: number, header: string): string {
  const headers = screen
    .getByRole('columnheader')
    .elements()
    .map((element) => element.textContent.trim());
  const column = headers.indexOf(header);
  expect(column, `no column headed ${header}`).not.toBe(-1);
  return screen
    .getByRole('row')
    .nth(row)
    .getByRole('cell')
    .nth(column)
    .element()
    .textContent.trim();
}

test('each issuer shows its status lists and keys against their limits', async () => {
  const screen = await render(IssuerTable, {
    issuers: [
      issuer('no-key-limit'),
      issuer('key-limit', { usage: { lists: 0, lists_limit: 5, keys: 2, keys_limit: 4 } }),
      issuer('full', { usage: { lists: 10, lists_limit: 10, keys: 1, keys_limit: null } }),
    ],
  });

  expect([cell(screen, 1, 'Status lists'), cell(screen, 1, 'Keys in use')]).toEqual([
    '3 of 10',
    '1',
  ]);
  expect([cell(screen, 2, 'Status lists'), cell(screen, 2, 'Keys in use')]).toEqual([
    '0 of 5',
    '2 of 4',
  ]);
  expect(cell(screen, 3, 'Status lists')).toBe('10 of 10');
});

test('an issuer close to its expiry is warned about', async () => {
  const screen = await render(IssuerTable, {
    issuers: [issuer('soon', { ...expiry(3) })],
  });

  expect(cell(screen, 1, 'Expires')).toMatch(/in 3 days/);
  await expect.element(screen.getByText(/Expires soon/)).toBeVisible();
});

test('an issuer far from its expiry is not warned about', async () => {
  const screen = await render(IssuerTable, { issuers: [issuer('later')] });

  expect(cell(screen, 1, 'Expires')).toMatch(/in 20 days$/);
  await expect.element(screen.getByText(/Expires soon/)).not.toBeInTheDocument();
});
