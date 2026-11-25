import { beforeEach, expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

const post = vi.hoisted(() => vi.fn());

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/navigation', () => ({ goto: vi.fn(), invalidateAll: vi.fn() }));
vi.mock('$app/state', () => ({ page: { url: new URL('http://portal.test/issuers/issuer-1') } }));
vi.mock('#lib/api/index.ts', () => ({
  createApi: () => ({ POST: post }),
  errorMessage: () => 'Request failed',
  SIGNING_ALGORITHMS: ['ES256', 'ES384', 'EdDSA', 'PS256'],
}));
vi.mock('#lib/notices.svelte.ts', () => ({ flash: vi.fn() }));

import Page from './+page.svelte';

function issuer(keys = 1, limit: number | null = 2) {
  return {
    id: 'issuer-1',
    display_name: 'Test issuer',
    state: 'active' as const,
    created_at: null,
    expires_at: null,
    expires_soon_from: null,
    keys: Array.from({ length: keys }, (_, index) => ({
      kid: `key-${index + 1}`,
      alg: 'ES256' as const,
      state: 'active' as const,
    })),
    usage: { lists: 0, lists_limit: 10, keys, keys_limit: limit },
    management_url: 'https://status.example.com',
    audience: 'https://status.example.com',
  };
}

function pageData(keys = 1, limit: number | null = 2) {
  return { issuer: issuer(keys, limit), issuerVersion: '7', user: null, terms: null };
}

beforeEach(() => post.mockReset());

test('a pasted public key can be added while the old key stays available', async () => {
  post.mockResolvedValue({ data: { kid: 'key-2', alg: 'ES256', state: 'active' } });
  const screen = await render(Page, { data: pageData() });
  await screen
    .getByLabelText('Public key (JWK)')
    .fill('{"kty":"EC","crv":"P-256","x":"AAAA","y":"BBBB"}');
  await screen.getByRole('button', { name: 'Add public key' }).click();
  expect(post).toHaveBeenCalledWith(
    '/api/v1/issuers/{id}/keys',
    expect.objectContaining({
      params: { path: { id: 'issuer-1' }, header: { 'if-issuer-version': '7' } },
      body: { alg: 'ES256', public_key: { kty: 'EC', crv: 'P-256', x: 'AAAA', y: 'BBBB' } },
    }),
  );
  await expect.element(screen.getByLabelText('Public key (JWK)')).toHaveValue('');
});

test('the tier limit blocks adding a key', async () => {
  const screen = await render(Page, { data: pageData(2, 2) });
  await expect.element(screen.getByText(/The key limit is reached/)).toBeVisible();
  await expect
    .element(screen.getByRole('button', { name: 'Add public key' }))
    .not.toBeInTheDocument();
});

test('revoking the last active key requires confirmation', async () => {
  post.mockResolvedValue({ data: { kid: 'key-1', alg: 'ES256', state: 'revoked' } });
  const screen = await render(Page, { data: pageData(1) });
  await screen.getByRole('button', { name: 'Revoke key key-1' }).click();
  expect(post).not.toHaveBeenCalled();
  await expect.element(screen.getByText(/will no longer be able to write/)).toBeVisible();
  await screen.getByRole('button', { name: 'Revoke key' }).click();
  expect(post).toHaveBeenCalledWith('/api/v1/issuers/{id}/keys/{kid}/revoke', {
    params: { path: { id: 'issuer-1', kid: 'key-1' }, header: { 'if-issuer-version': '7' } },
  });
});

test('revoking one of two active keys also requires confirmation', async () => {
  post.mockResolvedValue({ data: { kid: 'key-1', alg: 'ES256', state: 'revoked' } });
  const screen = await render(Page, { data: pageData(2) });
  await screen.getByRole('button', { name: 'Revoke key key-1' }).click();
  expect(post).not.toHaveBeenCalled();
  await expect.element(screen.getByRole('dialog', { name: 'Revoke key key-1?' })).toBeVisible();
});

test('changing the algorithm keeps a pasted public key', async () => {
  const screen = await render(Page, { data: pageData() });
  await screen.getByLabelText('Public key (JWK)').fill('{"kty":"EC"}');
  await screen.getByLabelText('Algorithm').selectOptions('ES384');
  await expect.element(screen.getByLabelText('Public key (JWK)')).toHaveValue('{"kty":"EC"}');
});

test('changing the algorithm clears a generated public key', async () => {
  const screen = await render(Page, { data: pageData() });
  await screen.getByRole('button', { name: 'Make an ES256 key pair in this browser' }).click();
  await expect.element(screen.getByLabelText('Public key (JWK)')).toHaveValue('');
  await screen.getByRole('button', { name: 'Download private-key.json' }).click();
  await expect.element(screen.getByLabelText('Public key (JWK)')).not.toHaveValue('');
  await screen.getByLabelText('Algorithm').selectOptions('ES384');
  await expect.element(screen.getByLabelText('Public key (JWK)')).toHaveValue('');
});

test('invalid key input is described and focused', async () => {
  const screen = await render(Page, { data: pageData() });
  const field = screen.getByLabelText('Public key (JWK)');
  await field.fill('not JSON');
  await screen.getByRole('button', { name: 'Add public key' }).click();
  await expect.element(field).toHaveAttribute('aria-invalid', 'true');
  await expect
    .element(field)
    .toHaveAttribute('aria-describedby', 'new-public-key-hint new-public-key-error');
  expect(document.activeElement?.id).toBe('new-public-key');
});
