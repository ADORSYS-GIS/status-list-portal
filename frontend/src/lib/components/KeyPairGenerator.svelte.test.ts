import { expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

import KeyPairGenerator from './KeyPairGenerator.svelte';

vi.mock('#lib/key-pair.ts', () => ({ makeKeyPair: vi.fn(() => new Promise(() => {})) }));

test('one key pair is made at a time', async () => {
  const screen = await render(KeyPairGenerator, { onkey: vi.fn() });
  const make = screen.getByRole('button', { name: 'Make an ES256 key pair in this browser' });

  await make.click();

  await expect.element(make).toBeDisabled();
});
