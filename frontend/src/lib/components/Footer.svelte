<script lang="ts">
  import type { Terms } from '#lib/api/index.ts';

  let { terms }: { terms: Terms | null } = $props();
</script>

<footer>
  <div class="bar">
    <span>A sandbox for the Status List Server, run by adorsys.</span>
    <ul>
      {#if terms}
        <li><a href={terms.url} rel="external">Terms of use</a></li>
      {/if}
      <li>
        <a href="https://github.com/adorsys/status-list-server" rel="external">Status List Server</a
        >
      </li>
    </ul>
  </div>
</footer>

<style>
  footer {
    margin-top: auto;
    border-top: 1px solid var(--color-border);
    color: var(--color-muted);
    font-size: 0.9375rem;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: var(--space-3);
    max-width: var(--content-width);
    margin: 0 auto;
    padding: var(--space-6) var(--space-4);
  }

  ul {
    display: flex;
    gap: var(--space-4);
    margin: 0;
    padding: 0;
    list-style: none;
  }
</style>
