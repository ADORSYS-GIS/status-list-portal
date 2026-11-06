<script lang="ts">
  import { resolve } from '$app/paths';
  import type { Issuer } from '#lib/api/index.ts';
  import { expiryText } from '#lib/format.ts';

  import StateBadge from './StateBadge.svelte';

  let { issuers }: { issuers: Issuer[] } = $props();
</script>

<table>
  <thead>
    <tr>
      <th scope="col">Issuer</th>
      <th scope="col">State</th>
      <th scope="col">Expires</th>
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
        <td>{expiryText(issuer.expires_at)}</td>
      </tr>
    {/each}
  </tbody>
</table>

<style>
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
