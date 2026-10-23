import { expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

import Header from './Header.svelte';

const USER = {
  login: 'octocat',
  name: 'The Octocat',
  email: 'octocat@example.com',
  terms_accepted: true,
};

test('visitors are offered to sign in with GitHub', async () => {
  const screen = await render(Header, { user: null, onsignout: vi.fn() });

  await expect
    .element(screen.getByRole('link', { name: 'Sign in with GitHub' }))
    .toHaveAttribute('href', '/api/v1/session/github');
  await expect.element(screen.getByRole('img', { name: 'adorsys' })).toBeVisible();
});

test('signed-in users see who they are and can sign out', async () => {
  const onsignout = vi.fn();
  const screen = await render(Header, { user: USER, onsignout });

  await expect.element(screen.getByText('octocat')).toBeInTheDocument();
  await screen.getByRole('button', { name: 'Sign out' }).click();

  expect(onsignout).toHaveBeenCalledOnce();
});
