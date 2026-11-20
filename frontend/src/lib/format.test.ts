import { expect, test } from 'vitest';

import { expiresSoon, expiryText, usageText } from './format';

const NOW = new Date('2026-10-23T12:00:00Z');

test('issuers that never expire say so', () => {
  expect(expiryText(null, NOW)).toBe('Never');
});

test('a coming expiry counts the days left', () => {
  expect(expiryText('2026-11-22T12:00:00Z', NOW)).toMatch(/, in 30 days$/);
  expect(expiryText('2026-10-24T11:00:00Z', NOW)).toMatch(/, in 1 day$/);
});

test('a past expiry reads as expired', () => {
  expect(expiryText('2026-10-01T12:00:00Z', NOW)).toMatch(/^Expired on /);
});

function inDays(days: number): string {
  return new Date(NOW.getTime() + days * 24 * 60 * 60 * 1000).toISOString();
}

/** An active issuer that expires in `days` and is warned about from `expiresSoonFrom`. */
function active(days: number, expiresSoonFrom: number) {
  return {
    state: 'active' as const,
    expires_at: inDays(days),
    expires_soon_from: inDays(expiresSoonFrom),
  };
}

test('an active issuer expires soon from its expires_soon_from until its expiry', () => {
  expect(expiresSoon(active(7, 0), NOW)).toBe(true);
  expect(expiresSoon(active(0.5, -6.5), NOW)).toBe(true);
  expect(expiresSoon(active(8, 1), NOW)).toBe(false);
});

test('the warning follows expires_soon_from, not a fixed number of days', () => {
  // A short sandbox lifetime: three days left, but warned about only on the last day.
  expect(expiresSoon(active(3, 2), NOW)).toBe(false);
  expect(expiresSoon(active(1, 0), NOW)).toBe(true);
});

test('only an active issuer that will expire is warned about', () => {
  expect(expiresSoon({ state: 'active', expires_at: null, expires_soon_from: null }, NOW)).toBe(
    false,
  );
  expect(expiresSoon(active(-1, -8), NOW)).toBe(false);
  expect(expiresSoon({ ...active(-1, -8), state: 'expired' }, NOW)).toBe(false);
  expect(expiresSoon({ ...active(3, -4), state: 'suspended' }, NOW)).toBe(false);
});

test('usage reads against its limit, or alone when there is none', () => {
  expect(usageText(3, 10)).toBe('3 of 10');
  expect(usageText(0, 10)).toBe('0 of 10');
  expect(usageText(1, null)).toBe('1');
});
