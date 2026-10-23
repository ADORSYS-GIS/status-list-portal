<script lang="ts">
  import CopyButton from './CopyButton.svelte';

  let { code, title }: { code: string; title: string } = $props();
</script>

<figure>
  <figcaption>
    <span>{title}</span>
    <CopyButton text={code} label={title} />
  </figcaption>
  <!-- Long lines scroll, so keyboard users need to focus the block to scroll it. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <pre tabindex="0" aria-label={title}><code>{code}</code></pre>
</figure>

<style>
  figure {
    margin: 0 0 var(--space-6);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  figcaption {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--color-border);
    background: var(--color-surface);
    font-size: 0.9375rem;
    font-weight: 500;
  }

  pre {
    margin: 0;
    padding: var(--space-4);
    overflow-x: auto;
    background: var(--color-code-bg);
    line-height: 1.5;
  }
</style>
