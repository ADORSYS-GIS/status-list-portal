<script lang="ts">
  import type { Issuer } from '#lib/api/index.ts';
  import { expiresSoon, expiryText, nextExpiryChange } from '#lib/format.ts';

  let { issuer }: { issuer: Pick<Issuer, 'state' | 'expires_at' | 'expires_soon_from'> } = $props();

  /** The longest delay `setTimeout` holds; a later change is checked again then. */
  const MAX_DELAY = 2 ** 31 - 1;

  let now = $state(new Date());

  // A page left open reads again when the warning starts, a day goes by, or the issuer
  // expires.
  $effect(() => {
    const next = nextExpiryChange(issuer, now);
    if (next === null) {
      return;
    }
    const delay = Math.min(Math.max(next - now.getTime(), 0), MAX_DELAY);
    const timer = setTimeout(() => (now = new Date()), delay);
    return () => clearTimeout(timer);
  });

  /** Timers can fire late in a hidden tab or after sleep, so a tab shown again reads again. */
  function onvisibilitychange() {
    if (document.visibilityState === 'visible') {
      now = new Date();
    }
  }
</script>

<svelte:document {onvisibilitychange} />

<!-- One root, so the warning goes under the date whatever lays out the component. -->
<div>
  <span>{expiryText(issuer.expires_at, now)}</span>
  {#if expiresSoon(issuer, now)}
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
