<script lang="ts">
  import { tick } from 'svelte';

  import { makeKeyPair, type KeyPair } from '#lib/key-pair.ts';

  import Icon from './Icon.svelte';
  import Notice from './Notice.svelte';

  /**
   * Gets the public key once the private key is saved: an issuer registered with a key
   * whose private half was never saved could never sign a token.
   */
  let { onkey }: { onkey: (publicJwk: string) => void } = $props();

  let pair = $state<KeyPair | null>(null);
  let making = $state(false);
  let failed = $state(false);
  let saved = $state(false);
  let firstDownload = $state<HTMLButtonElement>();

  async function make() {
    // A second pair would replace the first after its private key may have been saved.
    making = true;
    try {
      pair = await makeKeyPair();
    } catch {
      failed = true;
      return;
    } finally {
      making = false;
    }
    // The button that had focus is gone; the next step is saving the key.
    await tick();
    firstDownload?.focus();
  }

  function save(name: string, text: string, type: string) {
    const url = URL.createObjectURL(new Blob([text], { type }));
    const link = document.createElement('a');
    link.href = url;
    link.download = name;
    link.click();
    // Revoked once the click has started the download, which needs the URL until then.
    setTimeout(() => URL.revokeObjectURL(url));
    if (pair && !saved) {
      saved = true;
      onkey(pair.publicJwk);
    }
  }
</script>

<div class="generator">
  {#if pair}
    {@const { privatePem, privateJwk } = pair}
    <Notice kind="warning">
      <p>
        <strong>Your private key exists only in the files you download here.</strong> The portal never
        receives it and cannot recover it. Keep it safe: it signs your issuer's tokens.
      </p>
    </Notice>
    <!-- The JSON first: the token script on the issuer page reads it; the PEM is for other tools. -->
    <div class="downloads">
      <button
        bind:this={firstDownload}
        type="button"
        class="button"
        onclick={() => save('private-key.json', privateJwk, 'application/json')}
      >
        <Icon name="arrow-down-tray" /> Download private-key.json
      </button>
      <button
        type="button"
        class="button"
        onclick={() => save('private-key.pem', privatePem, 'application/x-pem-file')}
      >
        <Icon name="arrow-down-tray" /> Download private-key.pem
      </button>
    </div>
    <p class="hint" role="status">
      {saved
        ? 'The public key is in the form.'
        : 'The public key goes into the form once you have saved the private key.'}
    </p>
  {:else}
    <button type="button" class="button" onclick={make} disabled={making}>
      <Icon name="key" /> Make an ES256 key pair in this browser
    </button>
    {#if failed}
      <p class="field-error" role="alert">
        This browser could not make a key pair. Use the Node.js script below instead.
      </p>
    {/if}
  {/if}
</div>

<style>
  .generator {
    margin-bottom: var(--space-4);
  }

  .downloads {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
</style>
