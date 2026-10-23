<script lang="ts">
  import { resolve } from '$app/paths';
  import type { Registration } from '#lib/api/index.ts';
  import { formatDateTime } from '#lib/format.ts';

  import StateBadge from './StateBadge.svelte';

  let {
    registrations,
    onwithdraw,
  }: { registrations: Registration[]; onwithdraw: (registration: Registration) => void } = $props();
</script>

<ul>
  {#each registrations as registration (registration.id)}
    <li>
      <div class="summary">
        <strong>{registration.display_name}</strong>
        <StateBadge state={registration.state} />
      </div>
      <p class="muted">Asked for on {formatDateTime(registration.created_at)}</p>
      {#if registration.state === 'rejected' && registration.rejection_reason}
        <p>Reason: {registration.rejection_reason}</p>
      {/if}
      {#if registration.state === 'approved' && registration.issuer_id}
        <p>
          <a href={resolve('/issuers/[id]', { id: encodeURIComponent(registration.issuer_id) })}
            >Open the issuer</a
          >
        </p>
      {/if}
      {#if registration.state === 'pending'}
        <button type="button" class="button" onclick={() => onwithdraw(registration)}>
          Withdraw
        </button>
      {/if}
    </li>
  {/each}
</ul>

<style>
  ul {
    display: grid;
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    padding: var(--space-4);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
  }

  .summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-1);
  }

  p {
    margin-bottom: var(--space-2);
  }
</style>
