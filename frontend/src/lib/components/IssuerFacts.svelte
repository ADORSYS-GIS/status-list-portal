<script lang="ts">
  import type { Issuer } from '#lib/api/index.ts';
  import { expiryText, formatDateTime } from '#lib/format.ts';
  import { signingKey } from '#lib/keys.ts';

  import CopyButton from './CopyButton.svelte';

  let { issuer }: { issuer: Issuer } = $props();

  let key = $derived(signingKey(issuer));
  /** What a developer copies into their tokens and requests. */
  let copyable = $derived([
    { label: 'Issuer id (iss)', value: issuer.id },
    { label: 'Key id (kid)', value: key?.kid ?? '' },
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
    <dt>Algorithm (alg)</dt>
    <dd><span class="mono">{key?.alg}</span></dd>
  </div>
  <div class="fact">
    <dt>Expires</dt>
    <dd>{expiryText(issuer.expires_at)}</dd>
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
