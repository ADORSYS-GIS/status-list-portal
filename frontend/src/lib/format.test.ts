import { expect, test } from 'vitest';

import { expiryText } from './format';

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
