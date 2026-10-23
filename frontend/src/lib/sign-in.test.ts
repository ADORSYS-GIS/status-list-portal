import { expect, test } from 'vitest';

import { signInError } from './sign-in';

test('known reasons are sign-in errors', () => {
  expect(signInError('no_verified_email')).toBe('no_verified_email');
  expect(signInError('unavailable')).toBe('unavailable');
});

test('anything else is not', () => {
  expect(signInError(null)).toBeNull();
  expect(signInError('toString')).toBeNull();
  expect(signInError('made_up')).toBeNull();
});
