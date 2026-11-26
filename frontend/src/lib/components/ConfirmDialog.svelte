<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    confirmLabel,
    cancelLabel = 'Cancel',
    onconfirm,
    children,
  }: {
    title: string;
    confirmLabel: string;
    cancelLabel?: string;
    onconfirm: () => void;
    /** What confirming does. Screen readers read all of it as the dialog's description. */
    children: Snippet;
  } = $props();

  const id = $props.id();

  let dialog: HTMLDialogElement;

  /** Opens the dialog. It keeps focus inside until it closes, then gives focus back. */
  export function open() {
    dialog.showModal();
  }

  function confirm() {
    dialog.close();
    onconfirm();
  }
</script>

<dialog bind:this={dialog} aria-labelledby="{id}-title" aria-describedby="{id}-body">
  <h2 id="{id}-title">{title}</h2>
  <div id="{id}-body">{@render children()}</div>
  <div class="actions">
    <button type="button" class="button" onclick={() => dialog.close()}>{cancelLabel}</button>
    <button type="button" class="button danger" onclick={confirm}>{confirmLabel}</button>
  </div>
</dialog>

<style>
  dialog {
    max-width: min(32rem, calc(100vw - 2rem));
    padding: var(--space-6);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
    background: var(--color-bg);
    color: var(--color-text);
  }

  dialog::backdrop {
    background: rgb(15 23 42 / 0.6);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-6);
  }
</style>
