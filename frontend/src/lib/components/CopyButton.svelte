<script lang="ts">
  import Icon from './Icon.svelte';

  /** How long "Copied" shows before the button reads "Copy" again. */
  const CONFIRMATION_MS = 2000;

  let { text, label }: { text: string; label: string } = $props();

  let copied = $state(false);
  let failed = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => () => clearTimeout(timer));

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      failed = false;
      clearTimeout(timer);
      timer = setTimeout(() => (copied = false), CONFIRMATION_MS);
    } catch {
      failed = true;
    }
  }
</script>

<button type="button" class="button copy" onclick={copy} aria-label="Copy {label}">
  <Icon name={copied ? 'check' : 'clipboard-document'} />
  <span aria-hidden="true">{copied ? 'Copied' : 'Copy'}</span>
</button>
<span class="visually-hidden" role="status">
  {#if copied}Copied {label}.{:else if failed}Copying failed: select the text and copy it.{/if}
</span>

<style>
  .copy {
    min-height: 2.25rem;
    padding: var(--space-1) var(--space-3);
    font-size: 0.875rem;
  }
</style>
