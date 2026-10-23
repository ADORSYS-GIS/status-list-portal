<script lang="ts">
  import type { SigningAlgorithm } from '#lib/api/index.ts';
  import { keyPairScript } from '#lib/keys.ts';

  import CodeBlock from './CodeBlock.svelte';

  let { alg }: { alg: SigningAlgorithm } = $props();
</script>

<details>
  <summary>Need a key pair?</summary>
  <ol>
    <li>Save this script as <code>make-key.mjs</code> in an empty folder.</li>
    <li>
      Run <code>node make-key.mjs</code> there (Node.js 22 or later). It saves the private key in
      <code>private-key.json</code>, which you keep, and prints the public key.
    </li>
    <li>Paste the printed public key into the field above.</li>
  </ol>
  <CodeBlock code={keyPairScript(alg)} title="make-key.mjs ({alg})" />
</details>

<style>
  details {
    margin-bottom: var(--space-6);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
  }

  summary {
    cursor: pointer;
    font-weight: 500;
    min-height: 1.5rem;
  }

  ol {
    margin: var(--space-3) 0;
    padding-left: var(--space-6);
  }
</style>
