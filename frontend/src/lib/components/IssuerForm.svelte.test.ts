import { expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

import IssuerForm from './IssuerForm.svelte';

const TERMS = { version: '2026-10-22', url: 'https://portal.example.com/terms' };
const PUBLIC_KEY = '{"kty":"EC","crv":"P-256","x":"AAAA","y":"BBBB"}';

async function renderForm(termsAccepted = true) {
  const onsubmit = vi.fn();
  const screen = await render(IssuerForm, {
    terms: TERMS,
    termsAccepted,
    submitting: false,
    onsubmit,
  });
  return { screen, onsubmit };
}

test('a filled-in form is submitted with the trimmed name', async () => {
  const { screen, onsubmit } = await renderForm();

  await screen.getByLabelText('Display name').fill('  Acme Wallet Issuer ');
  await screen.getByLabelText('Public key').fill(PUBLIC_KEY);
  await screen.getByRole('button', { name: 'Register issuer' }).click();

  expect(onsubmit).toHaveBeenCalledWith({
    displayName: 'Acme Wallet Issuer',
    alg: 'ES256',
    publicKey: PUBLIC_KEY,
  });
});

test('a private key never leaves the form', async () => {
  const { screen, onsubmit } = await renderForm();

  await screen.getByLabelText('Display name').fill('Acme Wallet Issuer');
  await screen
    .getByLabelText('Public key')
    .fill('{"kty":"EC","crv":"P-256","x":"AAAA","y":"BBBB","d":"c2Vj"}');
  await screen.getByRole('button', { name: 'Register issuer' }).click();

  expect(onsubmit).not.toHaveBeenCalled();
  await expect.element(screen.getByText(/This is a private key/)).toBeVisible();
  await expect.element(screen.getByLabelText('Public key')).toHaveFocus();
});

test('an empty form points at the first field to fix', async () => {
  const { screen, onsubmit } = await renderForm();

  await screen.getByRole('button', { name: 'Register issuer' }).click();

  expect(onsubmit).not.toHaveBeenCalled();
  await expect.element(screen.getByLabelText('Display name')).toHaveFocus();
  await expect
    .element(screen.getByLabelText('Display name'))
    .toHaveAccessibleDescription(/Give your issuer a name/);
});

test('the terms must be accepted the first time', async () => {
  const { screen, onsubmit } = await renderForm(false);

  await screen.getByLabelText('Display name').fill('Acme Wallet Issuer');
  await screen.getByLabelText('Public key').fill(PUBLIC_KEY);
  await screen.getByRole('button', { name: 'Register issuer' }).click();
  expect(onsubmit).not.toHaveBeenCalled();

  await screen.getByLabelText(/I have read and accept the terms of use/).click();
  await screen.getByRole('button', { name: 'Register issuer' }).click();

  expect(onsubmit).toHaveBeenCalledOnce();
});
