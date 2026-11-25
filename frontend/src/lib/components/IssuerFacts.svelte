<script lang="ts">
  import type { Issuer } from '#lib/api/index.ts';
  import { formatDateTime, usageText } from '#lib/format.ts';

  import CopyButton from './CopyButton.svelte';
  import Expiry from './Expiry.svelte';

  let { issuer, key }: { issuer: Issuer; key?: Issuer['keys'][number] } = $props();
  /** What a developer copies into their tokens and requests. */
  let copyable = $derived([
    { label: 'Issuer id (iss)', value: issuer.id },
    { label: 'Audience (aud)', value: issuer.audience },
    { label: 'Management API', value: issuer.management_url },
  ]);
</script>

<dl>
  {#each copyable as fact (fact.label)}
    <div class="fact">
      <dt>{fact.label}</dt>
      <dd>
        <span class="mono value">{fact.value}</span>
        <CopyButton text={fact.value} label={fact.label} />
      </dd>
    </div>
  {/each}
  <div class="fact">
    <dt>Key id (kid)</dt>
    <dd>
      {#if key}
        <span class="mono value">{key.kid}</span>
        <CopyButton text={key.kid} label="Key id (kid)" />
      {:else}
        No active key
      {/if}
    </dd>
  </div>
  <div class="fact">
    <dt>Algorithm (alg)</dt>
    <dd><span class="mono">{key?.alg ?? 'No active key'}</span></dd>
  </div>
  <div class="fact">
    <dt>Expires</dt>
    <dd><Expiry {issuer} /></dd>
  </div>
  <div class="fact">
    <dt>Status lists</dt>
    <dd>{usageText(issuer.usage.lists, issuer.usage.lists_limit)}</dd>
  </div>
  <div class="fact">
    <dt>Keys in use</dt>
    <dd>{usageText(issuer.usage.keys, issuer.usage.keys_limit)}</dd>
  </div>
  {#if issuer.created_at}
    <div class="fact">
      <dt>Registered</dt>
      <dd>{formatDateTime(issuer.created_at)}</dd>
    </div>
  {/if}
</dl>

<style>
  dl {
    margin: 0 0 var(--space-8);
    border-top: 1px solid var(--color-border);
  }

  .fact {
    display: grid;
    grid-template-columns: minmax(10rem, 14rem) 1fr;
    gap: var(--space-2) var(--space-4);
    padding: var(--space-3) 0;
    border-bottom: 1px solid var(--color-border);
  }

  dt {
    color: var(--color-muted);
  }

  dd {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin: 0;
    min-width: 0;
  }

  .value {
    overflow-wrap: anywhere;
  }

  @media (max-width: 40rem) {
    .fact {
      grid-template-columns: 1fr;
    }
  }
</style>
