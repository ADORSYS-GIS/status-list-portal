<script lang="ts">
  import type { Snippet } from 'svelte';

  import Icon, { type IconName } from './Icon.svelte';

  type Kind = 'info' | 'success' | 'warning' | 'error';

  let {
    kind = 'info',
    announce = true,
    children,
  }: { kind?: Kind; announce?: boolean; children: Snippet } = $props();

  const ICONS: Record<Kind, IconName> = {
    info: 'information-circle',
    success: 'check-circle',
    warning: 'exclamation-triangle',
    error: 'exclamation-triangle',
  };
</script>

<div class="notice {kind}" role={announce ? (kind === 'error' ? 'alert' : 'status') : undefined}>
  <Icon name={ICONS[kind]} />
  <div>{@render children()}</div>
</div>

<style>
  .notice {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
    padding: var(--space-3) var(--space-4);
    margin-bottom: var(--space-6);
    border: 1px solid var(--color-border);
    border-left-width: 4px;
    border-radius: var(--radius);
    background: var(--color-surface);
  }

  .notice :global(.icon) {
    width: 1.25rem;
    height: 1.25rem;
    margin-top: 0.15rem;
  }

  .notice :global(p:last-child) {
    margin-bottom: 0;
  }

  .success {
    border-left-color: var(--color-success);
  }

  .warning {
    border-left-color: var(--color-warning);
  }

  .error {
    border-left-color: var(--color-danger);
  }

  .info {
    border-left-color: var(--color-accent);
  }
</style>
