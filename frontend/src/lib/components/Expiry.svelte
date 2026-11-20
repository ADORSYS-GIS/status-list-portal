<script lang="ts">
  import type { Issuer } from '#lib/api/index.ts';
  import { expiresSoon, expiryText } from '#lib/format.ts';

  let { issuer }: { issuer: Pick<Issuer, 'state' | 'expires_at' | 'renew_from'> } = $props();
</script>

<!-- One root, so the warning goes under the date whatever lays out the component. -->
<div>
  <span>{expiryText(issuer.expires_at)}</span>
  {#if expiresSoon(issuer)}
    <!-- In words, so that color is never the only signal. -->
    <span class="soon">Expires soon: renew it to keep it active</span>
  {/if}
</div>

<style>
  .soon {
    display: block;
    color: var(--color-warning);
    font-size: 0.875rem;
    font-weight: 500;
  }
</style>
