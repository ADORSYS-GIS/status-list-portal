import { expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

import { makeKeyPair } from '#lib/key-pair.ts';

import KeyPairGenerator from './KeyPairGenerator.svelte';

vi.mock('#lib/key-pair.ts', () => ({ makeKeyPair: vi.fn(() => new Promise(() => {})) }));

test('one key pair is made at a time', async () => {
  const screen = await render(KeyPairGenerator, { onkey: vi.fn() });
  const make = screen.getByRole('button', { name: 'Make a key pair in this browser' });

  await make.click();

  await expect.element(make).toHaveAttribute('aria-disabled', 'true');
  await expect.element(make).toHaveFocus();
  // Forced: the click must reach a button that only looks disabled.
  await make.click({ force: true });
  expect(makeKeyPair).toHaveBeenCalledTimes(1);
});

test('a key pair that could not be made can be tried again', async () => {
  vi.mocked(makeKeyPair).mockRejectedValueOnce(new Error('no ECDSA'));
  const screen = await render(KeyPairGenerator, { onkey: vi.fn() });
  const make = screen.getByRole('button', { name: 'Make a key pair in this browser' });

  await make.click();

  await expect
    .element(screen.getByText(/could not make a key pair/))
    .toHaveAttribute('role', 'alert');
  await expect.element(make).toBeEnabled();
});
