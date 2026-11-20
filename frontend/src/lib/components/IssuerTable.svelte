<script lang="ts">
  import { resolve } from '$app/paths';
  import type { Issuer } from '#lib/api/index.ts';
  import { usageText } from '#lib/format.ts';

  import Expiry from './Expiry.svelte';
  import StateBadge from './StateBadge.svelte';

  let { issuers }: { issuers: Issuer[] } = $props();
</script>

<!-- Scrolls sideways on its own on narrow screens, so the page does not. Only the first
     column has links, so keyboard users need to focus the region to scroll it. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="scroll" tabindex="0" role="region" aria-label="Issuers">
  <table>
    <thead>
      <tr>
        <th scope="col">Issuer</th>
        <th scope="col">State</th>
        <th scope="col">Expires</th>
        <th scope="col">Status lists</th>
        <th scope="col">Keys in use</th>
      </tr>
    </thead>
    <tbody>
      {#each issuers as issuer (issuer.id)}
        <tr>
          <td>
            <a href={resolve('/issuers/[id]', { id: encodeURIComponent(issuer.id) })}
              >{issuer.display_name ?? issuer.id}</a
            >
            <span class="mono id">{issuer.id}</span>
          </td>
          <td><StateBadge state={issuer.state} /></td>
          <td><Expiry {issuer} /></td>
          <td>{usageText(issuer.usage.lists, issuer.usage.lists_limit)}</td>
          <td>{usageText(issuer.usage.keys, issuer.usage.keys_limit)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .scroll {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  th {
    text-align: left;
    font-weight: 500;
    color: var(--color-muted);
    font-size: 0.875rem;
  }

  th,
  td {
    padding: var(--space-3) var(--space-3) var(--space-3) 0;
    border-bottom: 1px solid var(--color-border);
    vertical-align: top;
  }

  td a {
    font-weight: 500;
  }

  .id {
    display: block;
    color: var(--color-muted);
    font-size: 0.8125rem;
    overflow-wrap: anywhere;
  }
</style>
