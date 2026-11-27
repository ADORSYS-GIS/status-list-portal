import { createRawSnippet } from 'svelte';
import { expect, test, vi } from 'vitest';
import { render } from 'vitest-browser-svelte';

import ConfirmDialog from './ConfirmDialog.svelte';

function renderDialog(title: string) {
  return render(ConfirmDialog, {
    title,
    confirmLabel: 'Confirm',
    onconfirm: vi.fn(),
    children: createRawSnippet(() => ({ render: () => '<p>This cannot be undone.</p>' })),
  });
}

test('each dialog on a page is named by its own title', async () => {
  await renderDialog('Delete the issuer?');
  const second = await renderDialog('Revoke the key?');

  second.component.open();

  await expect.element(second.getByRole('dialog', { name: 'Revoke the key?' })).toBeVisible();
});

test('a dialog is described by its text', async () => {
  const screen = await renderDialog('Delete the issuer?');

  screen.component.open();

  await expect
    .element(screen.getByRole('dialog', { name: 'Delete the issuer?' }))
    .toHaveAccessibleDescription('This cannot be undone.');
});
